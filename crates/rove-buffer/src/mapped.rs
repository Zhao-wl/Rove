//! 超大文件的文本源（#8）：内存映射 + 分块行索引 + 追加内容的增量索引。
//!
//! - 打开时只映射文件，不把内容读进内存；同步索引头部 1 MiB（首屏立即可用），
//!   其余由后台线程按 32 MiB 分块用 `memchr` 扫描，[`TextSource::lines`] 报告
//!   “已知行数 + 已索引字节”。未索引到的行返回 [`Error::NotReady`]，与越界
//!   （[`Error::OutOfBounds`]）区分开：`complete == false` 时最终行数未知。
//! - 编码：显式指定 → BOM → `files.encoding`，不做全文猜测。UTF-8（含 BOM）直接
//!   映射；其他编码只能整体解码进内存（代价与文件大小成正比，PR 中写明了这一取舍）。
//! - 映射内容可能在外部被改写：所有读取都做边界检查，不合法的 UTF-8 字节逐字节
//!   显示成 `?`（字节长度不变，坐标仍然对应）。任何情况下都不 panic。
//! - [`MappedText::reload`] 通过“文件大小 + 修改时间 + 头部与旧末尾的校验”判断
//!   追加还是改写：追加时新快照共享旧索引的冻结段，只扫描新增部分；否则整体重建。
//!
//! 平台说明：Windows 上文件被映射期间，其他进程不能截断它（`SetEndOfFile` 返回
//! `ERROR_USER_MAPPED_FILE`），但可以改写内容、在末尾追加、重命名或删除。

use std::collections::BTreeMap;
use std::fs::File;
use std::hash::{DefaultHasher, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockReadGuard, Weak};
use std::time::{Duration, Instant, SystemTime};

use encoding_rs::{Encoding as RsEncoding, UTF_8};
use memmap2::Mmap;
use rove_base::*;

use crate::contracts::{LineCount, TextSource};
use crate::line_index::{scan_chunk, LineIndex};

/// 打开时同步索引的字节数：保证首屏立即可用。
pub const SYNC_INDEX_BYTES: u64 = 1024 * 1024;
/// 后台索引每次扫描并发布的字节数。
pub const INDEX_CHUNK_BYTES: u64 = 32 * 1024 * 1024;
/// 后台索引的工作线程上限（每个线程同时持有一块 32 MiB 的读缓冲）。
const INDEX_WORKERS: usize = 3;
/// 追加/改写判断时校验的头部与旧末尾长度。
const STAMP_SAMPLE: u64 = 4096;
/// 不合法 UTF-8 字节的替代显示：每个字节一个 `?`，字节长度不变。
const SUBSTITUTE: &str = "????????????????????????????????????????????????????????????????";

/// 超大文件的编码决定（打开时一次确定，重新加载沿用）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct MappedEncoding {
    pub encoding: &'static RsEncoding,
    pub bom_len: usize,
}

/// 文本的存放方式：UTF-8 直接映射；其他编码解码后放在堆上。
enum Storage {
    /// `file` 留给后台索引用普通读取扫描：不经映射触页，索引过程不把整个文件拉进
    /// 本进程的工作集，也避开逐页缺页的开销（1 GB 约 0.6 s）。
    Mapped {
        map: Mmap,
        skip: usize,
        file: File,
    },
    Owned(Box<[u8]>),
}

impl Storage {
    fn bytes(&self) -> &[u8] {
        match self {
            Storage::Mapped { map, skip, .. } => map.get(*skip..).unwrap_or_default(),
            Storage::Owned(bytes) => bytes,
        }
    }

    /// 读取内容 `[from, to)`，返回实际读到的字节（文件在外部变短时可能更少）。
    fn read<'a>(&'a self, from: u64, to: u64, buffer: &'a mut Vec<u8>) -> &'a [u8] {
        match self {
            Storage::Owned(bytes) => {
                let end = (to as usize).min(bytes.len());
                bytes.get(from as usize..end).unwrap_or_default()
            }
            Storage::Mapped { skip, file, .. } => {
                buffer.resize((to - from) as usize, 0);
                let filled = read_at(file, buffer, from + *skip as u64);
                &buffer[..filled]
            }
        }
    }
}

/// 定位读取，尽量读满；出错或到达文件末尾时返回已读字节数。
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> usize {
    let mut filled = 0;
    while filled < buffer.len() {
        #[cfg(windows)]
        let read = std::os::windows::fs::FileExt::seek_read(
            file,
            &mut buffer[filled..],
            offset + filled as u64,
        );
        #[cfg(unix)]
        let read = std::os::unix::fs::FileExt::read_at(
            file,
            &mut buffer[filled..],
            offset + filled as u64,
        );
        match read {
            Ok(0) | Err(_) => break,
            Ok(read) => filled += read,
        }
    }
    filled
}

/// 打开时记下的文件状态，用来判断后来是追加还是改写。
#[derive(Clone, Debug)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    head: u64,
    tail: u64,
}

impl FileStamp {
    fn read(file: &mut File) -> std::io::Result<Self> {
        let metadata = file.metadata()?;
        let len = metadata.len();
        Ok(Self {
            len,
            modified: metadata.modified().ok(),
            head: sample_hash(file, 0, len.min(STAMP_SAMPLE))?,
            tail: sample_hash(file, len.saturating_sub(STAMP_SAMPLE), len)?,
        })
    }
}

fn sample_hash(file: &mut File, from: u64, to: u64) -> std::io::Result<u64> {
    let mut buffer = vec![0u8; (to - from) as usize];
    file.seek(SeekFrom::Start(from))?;
    file.read_exact(&mut buffer)?;
    let mut hasher = DefaultHasher::new();
    hasher.write(&buffer);
    Ok(hasher.finish())
}

/// 索引与等待通知。后台线程只持有弱引用：快照全部释放后索引线程随之退出。
struct Shared {
    index: RwLock<LineIndex>,
    signal: Mutex<()>,
    changed: Condvar,
}

impl Shared {
    fn read(&self) -> RwLockReadGuard<'_, LineIndex> {
        self.index
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn notify(&self) {
        let _guard = self.signal.lock().unwrap_or_else(|p| p.into_inner());
        self.changed.notify_all();
    }
}

/// 追加/改写判断的结果。
pub enum Reload {
    /// 文件没有变化（大小、修改时间与头部都相同）。
    Unchanged,
    /// 文件只在末尾增长：新快照沿用已有行号，只索引新增部分。
    Appended(Arc<MappedText>),
    /// 文件被截断或中间被改写：整体重建（行号从头计算）。
    Replaced(Arc<MappedText>),
}

impl std::fmt::Debug for Reload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Reload::Unchanged => "Unchanged",
            Reload::Appended(_) => "Appended",
            Reload::Replaced(_) => "Replaced",
        })
    }
}

/// 超大文件的不可变文本快照。内容与版本不变；行索引在后台逐步补全。
pub struct MappedText {
    version: Version,
    storage: Arc<Storage>,
    len: u64,
    shared: Arc<Shared>,
    path: PathBuf,
    encoding: MappedEncoding,
    stamp: FileStamp,
}

impl std::fmt::Debug for MappedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedText")
            .field("version", &self.version)
            .field("path", &self.path)
            .field("len", &self.len)
            .field("lines", &self.lines())
            .finish()
    }
}

impl MappedText {
    /// 映射（或按非 UTF-8 编码解码）并同步索引头部 1 MiB，然后在后台索引其余部分。
    pub(crate) fn open(path: &Path, encoding: MappedEncoding) -> Result<Arc<Self>> {
        let mut file = File::open(path)?;
        let stamp = FileStamp::read(&mut file)?;
        let storage = Arc::new(load_storage(&file, stamp.len, encoding)?);
        let len = storage.bytes().len() as u64;
        Ok(Self::start(
            path.to_path_buf(),
            encoding,
            stamp,
            storage,
            len,
            LineIndex::new(),
            0,
        ))
    }

    fn start(
        path: PathBuf,
        encoding: MappedEncoding,
        stamp: FileStamp,
        storage: Arc<Storage>,
        len: u64,
        mut index: LineIndex,
        from: u64,
    ) -> Arc<Self> {
        let sync_end = from.saturating_add(SYNC_INDEX_BYTES).min(len);
        if from < sync_end {
            index.scan(storage.bytes(), from, sync_end);
        }
        if sync_end >= len {
            index.complete = true;
        }
        let complete = index.complete;
        let shared = Arc::new(Shared {
            index: RwLock::new(index),
            signal: Mutex::new(()),
            changed: Condvar::new(),
        });
        if !complete {
            let job = Arc::new(IndexJob {
                storage: storage.clone(),
                shared: Arc::downgrade(&shared),
                from: sync_end,
                len,
                claimed: AtomicU64::new(0),
                pending: Mutex::new(Pending::default()),
            });
            let workers = std::thread::available_parallelism()
                .map_or(1, |n| n.get() / 2)
                .clamp(1, INDEX_WORKERS);
            let chunks = (len - sync_end).div_ceil(INDEX_CHUNK_BYTES) as usize;
            for _ in 0..workers.min(chunks) {
                let job = job.clone();
                std::thread::Builder::new()
                    .name("rove-line-index".into())
                    .spawn(move || job.run())
                    .expect("启动行索引线程");
            }
        }
        Arc::new(Self {
            version: crate::text::next_version(),
            storage,
            len,
            shared,
            path,
            encoding,
            stamp,
        })
    }

    /// 文件路径（重新加载时使用）。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 内容是否直接来自内存映射（UTF-8）；否则是解码后放在堆上的副本。
    pub fn is_memory_mapped(&self) -> bool {
        matches!(*self.storage, Storage::Mapped { .. })
    }

    /// 行索引本身占用的堆内存（字节）。不含映射的文件页：那些页由系统按需换入、可回收。
    pub fn index_heap_bytes(&self) -> usize {
        self.shared.read().heap_bytes()
    }

    /// 等到已知行数达到 `lines`、索引完成或超时，返回当时的进度。
    /// “转到行”在索引未完成时用它等待（#22 负责界面）。
    pub fn wait_for_lines(&self, lines: u64, timeout: Duration) -> LineCount {
        self.wait_until(timeout, |count| count.complete || count.known >= lines)
    }

    /// 等到索引完成或超时，返回当时的进度。
    pub fn wait_complete(&self, timeout: Duration) -> LineCount {
        self.wait_until(timeout, |count| count.complete)
    }

    fn wait_until(&self, timeout: Duration, done: impl Fn(&LineCount) -> bool) -> LineCount {
        let deadline = Instant::now() + timeout;
        let mut guard = self.shared.signal.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            let count = self.lines();
            let now = Instant::now();
            if done(&count) || now >= deadline {
                return count;
            }
            guard = self
                .shared
                .changed
                .wait_timeout(guard, (deadline - now).min(Duration::from_millis(50)))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// 重新检查磁盘上的文件：没变、只在末尾追加、还是被截断/改写。
    ///
    /// 追加时新快照复用本快照已有的索引（冻结段共享），只扫描新增部分；本快照不受影响。
    /// 非 UTF-8 编码的超大文件（内容是解码副本）不做增量，按改写处理。
    pub fn reload(&self) -> Result<Reload> {
        let mut file = File::open(&self.path)?;
        let metadata = file.metadata()?;
        let len = metadata.len();
        let old = &self.stamp;
        let head_len = old.len.min(STAMP_SAMPLE);
        let head_same = len >= head_len && sample_hash(&mut file, 0, head_len)? == old.head;
        if len == old.len && metadata.modified().ok() == old.modified && head_same {
            return Ok(Reload::Unchanged);
        }
        let appended = len > old.len
            && head_same
            && sample_hash(&mut file, old.len.saturating_sub(STAMP_SAMPLE), old.len)? == old.tail
            && self.is_memory_mapped();
        let stamp = FileStamp::read(&mut file)?;
        let storage = Arc::new(load_storage(&file, stamp.len, self.encoding)?);
        let new_len = storage.bytes().len() as u64;
        if appended && new_len >= self.len {
            let mut index = self.shared.read().clone();
            let from = if index.complete {
                index.resume_after_append(storage.bytes())
            } else {
                index.scanned
            };
            index.complete = false;
            return Ok(Reload::Appended(Self::start(
                self.path.clone(),
                self.encoding,
                stamp,
                storage,
                new_len,
                index,
                from,
            )));
        }
        Ok(Reload::Replaced(Self::start(
            self.path.clone(),
            self.encoding,
            stamp,
            storage,
            new_len,
            LineIndex::new(),
            0,
        )))
    }

    /// 内容开头至多 `len` 个字节（打开时的二进制判定与换行符统计用）。
    pub(crate) fn head(&self, len: usize) -> &[u8] {
        let bytes = self.bytes();
        &bytes[..len.min(bytes.len())]
    }

    fn bytes(&self) -> &[u8] {
        let bytes = self.storage.bytes();
        &bytes[..(self.len as usize).min(bytes.len())]
    }

    fn is_boundary(&self, offset: usize) -> bool {
        let bytes = self.bytes();
        offset == bytes.len() || bytes.get(offset).is_some_and(|&b| (b & 0xC0) != 0x80)
    }

    /// 第 `line` 行内容的字节区间（不含终止符）。
    fn line_bytes(&self, line: u64) -> Result<(usize, usize)> {
        let index = self.shared.read();
        let starts = index.starts();
        let total = starts + u64::from(index.trailing_cr);
        if index.complete {
            if line >= total {
                return Err(Error::OutOfBounds);
            }
        } else if line + 1 >= starts {
            return Err(Error::NotReady);
        }
        let len = self.len;
        if line == starts {
            // 末尾单独的 CR 之后的空行。
            return Ok((len as usize, len as usize));
        }
        let start = index.start(line).min(len);
        let end = if line + 1 < starts {
            let next = index.start(line + 1).min(len);
            next - self.terminator_len(start, next)
        } else if index.trailing_cr {
            len - 1
        } else {
            len
        };
        drop(index);
        Ok((start as usize, end.max(start) as usize))
    }

    /// `next` 之前的行终止符长度。内容被外部改写时可能为 0，调用方不能假定 > 0。
    fn terminator_len(&self, start: u64, next: u64) -> u64 {
        let bytes = self.bytes();
        let at = |offset: u64| bytes.get(offset as usize).copied();
        match next.checked_sub(1).and_then(at) {
            Some(b'\n') if next >= start + 2 && at(next - 2) == Some(b'\r') => 2,
            Some(b'\n' | b'\r') => 1,
            _ => 0,
        }
        .min(next - start)
    }
}

fn load_storage(file: &File, file_len: u64, encoding: MappedEncoding) -> Result<Storage> {
    let skip = encoding.bom_len.min(file_len as usize);
    if file_len == 0 {
        return Ok(Storage::Owned(Box::default()));
    }
    // SAFETY: 映射是只读的；文件在外部被改写时内容可能变化，所有读取都做边界检查，
    // 不依赖内容是合法 UTF-8，也不依赖索引与内容一致。Windows 上被映射的文件不能被截断。
    let map = unsafe { Mmap::map(file)? };
    if encoding.encoding == UTF_8 {
        return Ok(Storage::Mapped {
            map,
            skip,
            file: file.try_clone()?,
        });
    }
    let (text, _) = encoding
        .encoding
        .decode_without_bom_handling(map.get(skip..).unwrap_or_default());
    Ok(Storage::Owned(
        text.into_owned().into_bytes().into_boxed_slice(),
    ))
}

/// 已扫描、等待按顺序发布的块。
#[derive(Default)]
struct Pending {
    next: u64,
    done: BTreeMap<u64, (Vec<u64>, bool)>,
}

/// 一次后台索引：若干工作线程按顺序领取 32 MiB 的块并行扫描，扫完的块按顺序发布。
/// 工作线程只持有索引的弱引用：快照全部释放后，正在扫描的块结束即退出。
struct IndexJob {
    storage: Arc<Storage>,
    shared: Weak<Shared>,
    from: u64,
    len: u64,
    claimed: AtomicU64,
    pending: Mutex<Pending>,
}

impl IndexJob {
    fn run(&self) {
        let mut buffer = Vec::new();
        loop {
            let chunk = self.claimed.fetch_add(1, Ordering::Relaxed);
            let from = self
                .from
                .saturating_add(chunk.saturating_mul(INDEX_CHUNK_BYTES));
            if from >= self.len || self.shared.strong_count() == 0 {
                return;
            }
            let to = from.saturating_add(INDEX_CHUNK_BYTES).min(self.len);
            // 窗口多带前后各一个字节，判断跨块的 CRLF。
            let window_start = from.saturating_sub(1);
            let window_end = to.saturating_add(1).min(self.len);
            let window = self.storage.read(window_start, window_end, &mut buffer);
            let mut starts = Vec::new();
            let trailing_cr = scan_chunk(window, window_start, from, to, self.len, &mut starts);
            if !self.publish(chunk, starts, trailing_cr) {
                return;
            }
            // 机械硬盘上整读一遍 1 GB 要几秒：每块之后让出一次，避免长期占住 I/O 与 CPU。
            std::thread::yield_now();
        }
    }

    /// 放入扫描结果，并把从 `next` 开始连续完成的块写进索引。快照已释放时返回假。
    fn publish(&self, chunk: u64, starts: Vec<u64>, trailing_cr: bool) -> bool {
        let Some(shared) = self.shared.upgrade() else {
            return false;
        };
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.done.insert(chunk, (starts, trailing_cr));
        let mut published = false;
        loop {
            let next = pending.next;
            let Some((starts, trailing_cr)) = pending.done.remove(&next) else {
                break;
            };
            let to = self
                .from
                .saturating_add((next + 1).saturating_mul(INDEX_CHUNK_BYTES))
                .min(self.len);
            let mut index = shared.index.write().unwrap_or_else(|p| p.into_inner());
            index.publish(&starts, to, trailing_cr);
            index.complete = to >= self.len;
            drop(index);
            pending.next = next + 1;
            published = true;
        }
        drop(pending);
        if published {
            shared.notify();
        }
        true
    }
}

impl TextSource for MappedText {
    fn version(&self) -> Version {
        self.version
    }

    fn byte_len(&self) -> u64 {
        self.len
    }

    fn lines(&self) -> LineCount {
        let index = self.shared.read();
        let starts = index.starts();
        LineCount {
            known: if index.complete {
                starts + u64::from(index.trailing_cr)
            } else {
                starts - 1
            },
            indexed_bytes: ByteOffset(index.scanned.min(self.len)),
            total_bytes: self.len,
            complete: index.complete,
        }
    }

    fn line_range(&self, line: Line) -> Result<ByteRange> {
        let (start, end) = self.line_bytes(line.0)?;
        Span::new(ByteOffset(start as u64), ByteOffset(end as u64))
    }

    fn visit_chunks(&self, range: ByteRange, visitor: &mut dyn FnMut(&str)) -> Result<()> {
        let start = usize::try_from(range.start().0).map_err(|_| Error::OutOfBounds)?;
        let end = usize::try_from(range.end().0).map_err(|_| Error::OutOfBounds)?;
        let bytes = self.bytes();
        if start > end || end > bytes.len() || !self.is_boundary(start) || !self.is_boundary(end) {
            return Err(Error::OutOfBounds);
        }
        for chunk in bytes[start..end].utf8_chunks() {
            if !chunk.valid().is_empty() {
                visitor(chunk.valid());
            }
            let mut invalid = chunk.invalid().len();
            while invalid > 0 {
                let take = invalid.min(SUBSTITUTE.len());
                visitor(&SUBSTITUTE[..take]);
                invalid -= take;
            }
        }
        Ok(())
    }

    fn offset(&self, position: Position) -> Result<ByteOffset> {
        let (start, end) = self.line_bytes(position.line.0)?;
        let column = usize::try_from(position.column.0).map_err(|_| Error::OutOfBounds)?;
        let offset = start.checked_add(column).ok_or(Error::OutOfBounds)?;
        if offset > end || !self.is_boundary(offset) {
            return Err(Error::OutOfBounds);
        }
        Ok(ByteOffset(offset as u64))
    }

    fn position(&self, offset: ByteOffset) -> Result<Position> {
        let byte = offset.0;
        if byte > self.len || !self.is_boundary(byte as usize) {
            return Err(Error::OutOfBounds);
        }
        let line = {
            let index = self.shared.read();
            if index.complete && index.trailing_cr && byte == self.len {
                index.starts()
            } else {
                index.line_at(byte)
            }
        };
        let (start, end) = self.line_bytes(line)?;
        let byte = byte as usize;
        if byte < start || byte > end {
            // 行终止符内部。
            return Err(Error::OutOfBounds);
        }
        Ok(Position {
            line: Line(line),
            column: Utf8Column((byte - start) as u64),
        })
    }

    fn utf16_to_utf8(&self, line: Line, column: Utf16Column) -> Result<Utf8Column> {
        let (start, end) = self.line_bytes(line.0)?;
        let target = column.0;
        let mut units = 0u64;
        let mut bytes = 0usize;
        if target == 0 {
            return Ok(Utf8Column(0));
        }
        for chunk in self.bytes()[start..end].utf8_chunks() {
            for character in chunk.valid().chars() {
                units += character.len_utf16() as u64;
                bytes += character.len_utf8();
                if units >= target {
                    return if units == target {
                        Ok(Utf8Column(bytes as u64))
                    } else {
                        Err(Error::OutOfBounds)
                    };
                }
            }
            let invalid = chunk.invalid().len() as u64;
            if units + invalid >= target {
                return Ok(Utf8Column(bytes as u64 + (target - units)));
            }
            units += invalid;
            bytes += invalid as usize;
        }
        Err(Error::OutOfBounds)
    }

    fn utf8_to_utf16(&self, line: Line, column: Utf8Column) -> Result<Utf16Column> {
        let (start, end) = self.line_bytes(line.0)?;
        let column = usize::try_from(column.0).map_err(|_| Error::OutOfBounds)?;
        let byte = start.checked_add(column).ok_or(Error::OutOfBounds)?;
        if byte > end || !self.is_boundary(byte) {
            return Err(Error::OutOfBounds);
        }
        let mut units = 0u64;
        for chunk in self.bytes()[start..byte].utf8_chunks() {
            // 合法 UTF-8：每个非续字节是一个字符，4 字节字符（首字节 ≥ 0xF0）占两个码元。
            units += chunk
                .valid()
                .bytes()
                .map(|b| u64::from((b & 0xC0) != 0x80) + u64::from(b >= 0xF0))
                .sum::<u64>();
            units += chunk.invalid().len() as u64;
        }
        Ok(Utf16Column(units))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(bytes: &[u8]) -> (tempfile::TempDir, Arc<MappedText>) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("huge.txt");
        std::fs::write(&path, bytes).unwrap();
        let text = MappedText::open(
            &path,
            MappedEncoding {
                encoding: UTF_8,
                bom_len: 0,
            },
        )
        .unwrap();
        text.wait_complete(Duration::from_secs(30));
        (directory, text)
    }

    fn collect(text: &dyn TextSource, range: ByteRange) -> String {
        let mut out = String::new();
        text.visit_chunks(range, &mut |chunk| out.push_str(chunk))
            .unwrap();
        out
    }

    fn span(start: u64, end: u64) -> ByteRange {
        Span::new(ByteOffset(start), ByteOffset(end)).unwrap()
    }

    #[test]
    fn matches_rope_text_on_mixed_terminators() {
        for sample in [
            "",
            "a",
            "a\n",
            "a\r",
            "a\r\n",
            "\n\n\n",
            "\r\n\r\n",
            "\r\r\n\n\r",
            "a\r\nb\nc\rd",
            "中文\r\n😀x\ry\n",
        ] {
            let rope = crate::RopeText::from_text(sample);
            let (_dir, mapped) = open(sample.as_bytes());
            assert_eq!(mapped.lines().known, rope.lines().known, "{sample:?}");
            assert!(mapped.lines().complete);
            for line in 0..=rope.lines().known {
                assert_eq!(
                    mapped.line_range(Line(line)),
                    rope.line_range(Line(line)),
                    "{sample:?} 第 {line} 行"
                );
            }
            for offset in 0..=sample.len() as u64 + 1 {
                assert_eq!(
                    mapped.position(ByteOffset(offset)),
                    rope.position(ByteOffset(offset)),
                    "{sample:?} 偏移 {offset}"
                );
            }
            for line in 0..rope.lines().known {
                for column in 0..12 {
                    assert_eq!(
                        mapped.utf16_to_utf8(Line(line), Utf16Column(column)),
                        rope.utf16_to_utf8(Line(line), Utf16Column(column)),
                        "{sample:?} {line}:{column}"
                    );
                    assert_eq!(
                        mapped.utf8_to_utf16(Line(line), Utf8Column(column)),
                        rope.utf8_to_utf16(Line(line), Utf8Column(column)),
                        "{sample:?} {line}:{column}"
                    );
                }
            }
            assert_eq!(
                collect(mapped.as_ref(), span(0, sample.len() as u64)),
                sample
            );
        }
    }

    #[test]
    fn invalid_utf8_is_shown_byte_for_byte() {
        let (_dir, text) = open(b"a\xFFb\xE4\n");
        assert_eq!(collect(text.as_ref(), span(0, 5)), "a?b?\n");
        assert_eq!(text.line_range(Line(0)).unwrap(), span(0, 4));
        assert_eq!(
            text.utf8_to_utf16(Line(0), Utf8Column(4)).unwrap(),
            Utf16Column(4)
        );
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(2)).unwrap(),
            Utf8Column(2)
        );
    }

    #[test]
    fn rejects_scalar_interiors() {
        let (_dir, text) = open("a😀b".as_bytes());
        assert_eq!(
            text.visit_chunks(span(0, 2), &mut |_| {}),
            Err(Error::OutOfBounds)
        );
        assert_eq!(text.position(ByteOffset(3)), Err(Error::OutOfBounds));
        assert_eq!(collect(text.as_ref(), span(1, 5)), "😀");
    }
}
