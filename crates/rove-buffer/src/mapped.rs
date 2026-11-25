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

mod backing;
use backing::{Storage, PAGE_BYTES};

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

/// 打开时记下的文件状态，用来判断后来是追加还是改写。
#[derive(Clone, Debug)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    identity: Option<(u64, u64)>,
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
            identity: crate::save::tail::file_identity(file, &metadata),
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
        Self::open_with_shared_reads(path, encoding, false)
    }

    pub(crate) fn open_with_shared_reads(
        path: &Path,
        encoding: MappedEncoding,
        shared_reads: bool,
    ) -> Result<Arc<Self>> {
        let mut file = File::open(path)?;
        let stamp = FileStamp::read(&mut file)?;
        let storage = Arc::new(load_storage(path, &file, &stamp, encoding, shared_reads)?);
        let len = storage.len();
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
            let window_start = from.saturating_sub(1);
            let window_end = sync_end.saturating_add(1).min(len);
            let mut starts = Vec::new();
            let trailing_cr = storage.with_range(window_start..window_end, |window| {
                scan_chunk(window, window_start, from, sync_end, len, &mut starts)
            });
            index.publish(&starts, sync_end, trailing_cr);
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

    /// 内容是否仍由内存映射提供。其余快照使用文件读取或堆上的解码副本。
    pub fn is_memory_mapped(&self) -> bool {
        self.storage.is_mapped()
    }

    /// The physical file length captured for this snapshot, including a BOM if present.
    pub fn raw_byte_len(&self) -> u64 {
        self.stamp.len
    }

    /// Stop keeping a file mapping open, so a writer can truncate it even while this
    /// snapshot is held by an editor. An existing read can briefly defer unmapping.
    pub fn enable_shared_reads(&self) -> bool {
        self.storage.enable_shared_reads()
    }

    /// Warm the last two shared-read pages on a worker before publishing a followed snapshot.
    pub fn warm_tail(&self) {
        if self.len != 0 && self.storage.has_disk_bytes() && !self.storage.is_mapped() {
            self.storage.byte(self.len - 1);
            self.storage.byte(self.len.saturating_sub(PAGE_BYTES));
        }
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
        self.reload_with_replacement(true, true)?
            .ok_or(Error::NotReady)
    }

    /// Check the append fast path without creating a replacement snapshot. The caller can
    /// run the normal open pipeline and detect encoding on the new file when this is `None`.
    pub fn reload_append(&self, track_truncation: bool) -> Result<Option<Reload>> {
        self.reload_with_replacement(false, track_truncation)
    }

    fn reload_with_replacement(&self, replace: bool, append: bool) -> Result<Option<Reload>> {
        let mut file = File::open(&self.path)?;
        let metadata = file.metadata()?;
        let len = metadata.len();
        let old = &self.stamp;
        let head_len = old.len.min(STAMP_SAMPLE);
        let same_file = old
            .identity
            .zip(crate::save::tail::file_identity(&file, &metadata))
            .is_none_or(|(old, new)| old == new);
        let head_same =
            same_file && len >= head_len && sample_hash(&mut file, 0, head_len)? == old.head;
        if len == old.len && metadata.modified().ok() == old.modified && head_same {
            return Ok(Some(Reload::Unchanged));
        }
        let appended = append
            && len > old.len
            && head_same
            && sample_hash(&mut file, old.len.saturating_sub(STAMP_SAMPLE), old.len)? == old.tail
            && self.storage.has_disk_bytes();
        if !appended && !replace {
            return Ok(None);
        }
        let stamp = FileStamp::read(&mut file)?;
        let storage = Arc::new(load_storage(
            &self.path,
            &file,
            &stamp,
            self.encoding,
            self.storage.uses_shared_reads(),
        )?);
        let new_len = storage.len();
        if appended && new_len >= self.len {
            let mut index = self.shared.read().clone();
            let from = if index.complete {
                let next = index
                    .trailing_cr
                    .then(|| storage.byte(index.scanned))
                    .flatten();
                index.resume_after_append(next)
            } else {
                index.scanned
            };
            index.complete = false;
            return Ok(Some(Reload::Appended(Self::start(
                self.path.clone(),
                self.encoding,
                stamp,
                storage,
                new_len,
                index,
                from,
            ))));
        }
        if !replace {
            return Ok(None);
        }
        Ok(Some(Reload::Replaced(Self::start(
            self.path.clone(),
            self.encoding,
            stamp,
            storage,
            new_len,
            LineIndex::new(),
            0,
        ))))
    }

    /// 内容开头至多 `len` 个字节（打开时的二进制判定与换行符统计用）。
    pub(crate) fn with_head<T>(&self, len: u64, use_bytes: impl FnOnce(&[u8]) -> T) -> T {
        self.storage.with_range(0..len.min(self.len), use_bytes)
    }

    fn is_boundary(&self, offset: usize) -> bool {
        let offset = offset as u64;
        offset == self.len
            || self
                .storage
                .byte(offset)
                .is_some_and(|b| (b & 0xC0) != 0x80)
    }

    /// 第 `line` 行内容的字节区间（不含终止符）。
    fn visit_decoded(
        &self,
        start: usize,
        end: usize,
        mut visitor: impl FnMut(&str) -> bool,
    ) -> Result<()> {
        let mut from = start as u64;
        let end = end as u64;
        while from < end {
            let boundary = (from / PAGE_BYTES + 1) * PAGE_BYTES;
            let mut to = end.min(boundary);
            if to < end {
                let limit = from.max(to.saturating_sub(3));
                while to > limit && !self.is_boundary(to as usize) {
                    to -= 1;
                }
                if to == from {
                    to = end.min(from + 4);
                }
            }
            let keep_going = self.storage.with_range(from..to, |bytes| {
                if bytes.len() as u64 != to - from {
                    return Err(Error::NotReady);
                }
                for chunk in bytes.utf8_chunks() {
                    if !chunk.valid().is_empty() && !visitor(chunk.valid()) {
                        return Ok(false);
                    }
                    let mut invalid = chunk.invalid().len();
                    while invalid > 0 {
                        let count = invalid.min(SUBSTITUTE.len());
                        if !visitor(&SUBSTITUTE[..count]) {
                            return Ok(false);
                        }
                        invalid -= count;
                    }
                }
                Ok(true)
            })?;
            if !keep_going {
                break;
            }
            from = to;
        }
        Ok(())
    }

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
        let at = |offset: u64| self.storage.byte(offset);
        match next.checked_sub(1).and_then(at) {
            Some(b'\n') if next >= start + 2 && at(next - 2) == Some(b'\r') => 2,
            Some(b'\n' | b'\r') => 1,
            _ => 0,
        }
        .min(next - start)
    }
}

fn load_storage(
    path: &Path,
    file: &File,
    stamp: &FileStamp,
    encoding: MappedEncoding,
    shared_reads: bool,
) -> Result<Storage> {
    let file_len = stamp.len;
    let skip = encoding.bom_len.min(file_len as usize);
    if file_len == 0 {
        return Ok(Storage::owned(Box::default(), shared_reads));
    }
    if shared_reads && encoding.encoding == UTF_8 {
        return Ok(Storage::disk(
            path.to_owned(),
            file.try_clone()?,
            stamp.identity,
            skip as u64,
            file_len - skip as u64,
            None,
        ));
    }
    if shared_reads {
        let mut input = Vec::new();
        let mut file = file.try_clone()?;
        file.seek(SeekFrom::Start(skip as u64))?;
        file.read_to_end(&mut input)?;
        let (text, _) = encoding.encoding.decode_without_bom_handling(&input);
        return Ok(Storage::owned(
            text.into_owned().into_bytes().into_boxed_slice(),
            true,
        ));
    }
    // SAFETY: the mapping is read-only. Readers check byte boundaries and tolerate
    // invalid text. A following editor switches to shared reads before allowing truncation.
    let map = unsafe { Mmap::map(file)? };
    if encoding.encoding == UTF_8 {
        let len = map.len().saturating_sub(skip) as u64;
        return Ok(Storage::disk(
            path.to_owned(),
            file.try_clone()?,
            stamp.identity,
            skip as u64,
            len,
            Some(map),
        ));
    }
    let (text, _) = encoding
        .encoding
        .decode_without_bom_handling(map.get(skip..).unwrap_or_default());
    Ok(Storage::owned(
        text.into_owned().into_bytes().into_boxed_slice(),
        false,
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
        if start > end
            || end as u64 > self.len
            || !self.is_boundary(start)
            || !self.is_boundary(end)
        {
            return Err(Error::OutOfBounds);
        }
        self.visit_decoded(start, end, |chunk| {
            visitor(chunk);
            true
        })
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
        let mut result = Err(Error::OutOfBounds);
        self.visit_decoded(start, end, |chunk| {
            for character in chunk.chars() {
                units += character.len_utf16() as u64;
                bytes += character.len_utf8();
                if units >= target {
                    if units == target {
                        result = Ok(Utf8Column(bytes as u64));
                    }
                    return false;
                }
            }
            true
        })?;
        result
    }

    fn utf8_to_utf16(&self, line: Line, column: Utf8Column) -> Result<Utf16Column> {
        let (start, end) = self.line_bytes(line.0)?;
        let column = usize::try_from(column.0).map_err(|_| Error::OutOfBounds)?;
        let byte = start.checked_add(column).ok_or(Error::OutOfBounds)?;
        if byte > end || !self.is_boundary(byte) {
            return Err(Error::OutOfBounds);
        }
        let mut units = 0u64;
        self.visit_decoded(start, byte, |chunk| {
            // 合法 UTF-8：每个非续字节是一个字符，4 字节字符占两个码元。
            units += chunk
                .bytes()
                .map(|b| u64::from((b & 0xC0) != 0x80) + u64::from(b >= 0xF0))
                .sum::<u64>();
            true
        })?;
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
    fn shared_reads_release_the_original_file_mapping_before_truncation() {
        use std::io::Write as _;
        let original = "before rotation\r\n".repeat(128);
        let (_dir, text) = open(original.as_bytes());
        let still_open = text.clone();
        assert!(text.is_memory_mapped());
        assert!(text.enable_shared_reads());
        assert!(!still_open.is_memory_mapped());
        let mut writer = std::fs::OpenOptions::new()
            .write(true)
            .open(text.path())
            .unwrap();
        writer.set_len(0).unwrap();
        writer.write_all(b"replacement\n").unwrap();
        writer.flush().unwrap();
        let Reload::Replaced(next) = text.reload().unwrap() else {
            panic!("the new generation must rebuild");
        };
        assert!(!next.is_memory_mapped());
        assert_eq!(collect(&*next, span(0, next.byte_len())), "replacement\n");
        assert_eq!(next.lines().known, 2);
    }

    #[test]
    fn decoded_snapshots_keep_their_open_policy_across_reload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("utf16.log");
        std::fs::write(&path, [b'a', 0, b'\n', 0]).unwrap();
        let ordinary = MappedText::open(
            &path,
            MappedEncoding {
                encoding: encoding_rs::UTF_16LE,
                bom_len: 0,
            },
        )
        .unwrap();
        assert!(!ordinary.storage.uses_shared_reads());
        std::fs::write(&path, [b'b', 0, b'\n', 0, b'c', 0]).unwrap();
        let Reload::Replaced(reloaded) = ordinary.reload().unwrap() else {
            panic!("the decoded input needs to be reloaded");
        };
        assert!(!reloaded.storage.uses_shared_reads());
        assert!(reloaded.enable_shared_reads());
        std::fs::write(&path, [b'd', 0, b'\n', 0]).unwrap();
        let Reload::Replaced(followed) = reloaded.reload().unwrap() else {
            panic!("the followed input needs to be reloaded");
        };
        assert!(followed.storage.uses_shared_reads());
    }

    #[test]
    fn shared_reader_preserves_a_scalar_across_cached_page_boundaries() {
        let prefix = "a".repeat(PAGE_BYTES as usize - 1);
        let text = format!("{prefix}😀in the next page\r\n雪");
        let rope = crate::RopeText::from_text(&text);
        let (_dir, shared) = open(text.as_bytes());
        assert!(shared.enable_shared_reads());
        assert_eq!(collect(&*shared, span(0, shared.byte_len())), text);
        assert_eq!(shared.line_range(Line(1)), rope.line_range(Line(1)));
        for delta in 0..=4 {
            let column = Utf16Column(PAGE_BYTES - 2 + delta);
            assert_eq!(
                shared.utf16_to_utf8(Line(0), column),
                rope.utf16_to_utf8(Line(0), column)
            );
        }
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
