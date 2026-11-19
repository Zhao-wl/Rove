//! 打开中的 C# 缓冲区相对“索引时版本”的状态：基线内容哈希 + 累积的编辑。
//!
//! 由装配处在文档生命周期的几个节点上调用（全部在后台线程，`reloaded` 会扫描新旧文本）：
//!
//! | 节点 | 调用 |
//! |---|---|
//! | 打开文档（#7 读盘完成） | [`BufferStates::opened`]（磁盘原始字节的 xxHash64，与索引文件表同一算法，见 [`hash_bytes`]） |
//! | 编辑（M3 的编辑事务） | [`BufferStates::edited`] |
//! | 外部修改后重新载入 | [`BufferStates::reloaded`]：新旧文本的公共前后缀之间算作一次编辑 |
//! | 保存 | [`BufferStates::saved`] |
//! | 关闭最后一个视图 | [`BufferStates::closed`] |
//!
//! 基线哈希是“编辑记录的起点”的内容哈希；索引里该文件的哈希等于它时，编辑记录就是索引版本到
//! 当前缓冲区的完整差异，可以做位置映射。两者不等时看**磁盘版本检查点**（见下），都对不上才
//! 无法映射、查询降级为近似。
//!
//! # 磁盘版本检查点（#216）
//!
//! 「当前内容 = 磁盘上的某个哈希」这件事只在保存 / 载入的那一刻成立：下一次按键就把
//! [`BufferState::current_hash`] 置为 `None`，信息随之丢失。索引是异步重建的，等它换代过来时
//! 用户往往已经继续在打字，于是“基线哈希对不上、当前哈希又是 `None`”——整份文件被判为无法
//! 映射，哪怕只多敲了一个字符。
//!
//! 所以在信息还在的那一刻就把它记下来：每次 [`BufferStates::saved`] / [`BufferStates::reloaded`]
//! 登记一个检查点（那一刻的磁盘哈希 + 一张空的编辑表），之后每次编辑都并入**所有**检查点的编辑
//! 表。索引换代到哈希 `h` 时，只要还留着 `h` 的检查点，就能把基线推进到 `h`、把编辑记录换成
//! “那一刻之后的编辑”，于是只有新敲的那几行降级，其余照旧精确。检查点最多留
//! [`MAX_CHECKPOINTS`] 个（索引可能落在中间某次保存上），超出丢最旧的；刚打开、还没保存过的
//! 文档一个都不需要（基线哈希就是那一刻的磁盘哈希），所以这条路上没有额外开销。
use super::mapping::{EditMap, LineEdit};
use crate::lifecycle::hash::xxh64;
use rove_base::{ByteOffset, ByteRange, Error, Position, Result, TextEdit, Version, WorkspacePath};
use rove_buffer::TextSource;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
};

/// 与 C# 索引文件表相同的内容哈希（原始字节的 xxHash64，种子 0）。
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    xxh64(bytes, 0)
}

/// 一个打开中的缓冲区。克隆是廉价的（编辑表与文本都是 `Arc`）。
#[derive(Clone)]
pub struct BufferState {
    pub version: Version,
    /// 编辑记录起点的内容哈希。
    pub base_hash: u64,
    /// 当前内容的磁盘哈希；有未保存的编辑时为 `None`。
    pub current_hash: Option<u64>,
    pub edits: Arc<EditMap>,
    pub text: Arc<dyn TextSource>,
}

/// 最多保留的磁盘版本检查点数：索引不一定追到最新一次保存，可能落在中间某一次上。
const MAX_CHECKPOINTS: usize = 4;

/// 一个磁盘版本检查点：某一刻“缓冲区内容 = 磁盘内容”的哈希，以及从那一刻到当前内容的编辑记录。
#[derive(Clone)]
struct Checkpoint {
    hash: u64,
    edits: Arc<EditMap>,
}

/// 一个文档的登记项：对外可见的状态 + 内部的检查点。
struct Entry {
    state: BufferState,
    /// 按时间从旧到新，最多 [`MAX_CHECKPOINTS`] 个。
    checkpoints: Vec<Checkpoint>,
}

impl Entry {
    /// 刚打开的文档不需要检查点：基线哈希就是那一刻的磁盘哈希。
    fn new(state: BufferState) -> Self {
        Self {
            state,
            checkpoints: Vec::new(),
        }
    }

    /// 记下“当前内容就是磁盘上的 `hash`”这一刻。之后的编辑会并进它的编辑表。
    fn checkpoint(&mut self, hash: u64) {
        if self
            .checkpoints
            .last()
            .is_some_and(|c| c.hash == hash && c.edits.is_empty())
        {
            return;
        }
        self.checkpoints.push(Checkpoint {
            hash,
            edits: Arc::new(EditMap::new()),
        });
        if self.checkpoints.len() > MAX_CHECKPOINTS {
            self.checkpoints.remove(0);
        }
    }

    /// 并入一次编辑：当前编辑记录与每个检查点的编辑表都要跟上。
    fn push_edit(&mut self, edit: LineEdit) {
        Arc::make_mut(&mut self.state.edits).push(edit);
        for c in &mut self.checkpoints {
            Arc::make_mut(&mut c.edits).push(edit);
        }
    }
}

/// 按项目相对路径登记的缓冲区状态。`Send + Sync`，内部一把短锁。
#[derive(Default)]
pub struct BufferStates {
    docs: Mutex<HashMap<Arc<str>, Entry>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl BufferStates {
    pub fn new() -> Self {
        Self::default()
    }

    /// 打开文档：`disk_hash` 是读盘时原始字节的哈希（[`hash_bytes`]）。已登记的文档被重置。
    pub fn opened(&self, path: &WorkspacePath, text: Arc<dyn TextSource>, disk_hash: u64) {
        let state = BufferState {
            version: text.version(),
            base_hash: disk_hash,
            current_hash: Some(disk_hash),
            edits: Arc::new(EditMap::new()),
            text,
        };
        lock(&self.docs).insert(path.as_str().into(), Entry::new(state));
    }

    /// 一次编辑：`before` 是编辑前的快照（`edit.version` 必须等于它的版本），`after` 是编辑后的快照。
    /// 版本对不上返回 `Stale`，未登记返回 `NotFound`。
    pub fn edited(
        &self,
        path: &WorkspacePath,
        before: &dyn TextSource,
        edit: &TextEdit,
        after: Arc<dyn TextSource>,
    ) -> Result<()> {
        let key = path.as_str();
        let registered_version = {
            let docs = lock(&self.docs);
            docs.get(key).ok_or(Error::NotFound)?.state.version
        };
        if registered_version != edit.version || before.version() != edit.version {
            return Err(Error::Stale);
        }
        // 插入文本可能很大。计算换行范围时让并发导航查询继续访问已登记快照。
        let start = before.position(edit.range.start())?;
        let end = before.position(edit.range.end())?;
        let line_edit = indexed_line_edit(start, end, edit, &*after)
            .unwrap_or_else(|| LineEdit::new(start, end, &edit.inserted));
        let mut docs = lock(&self.docs);
        let entry = docs.get_mut(key).ok_or(Error::NotFound)?;
        if entry.state.version != registered_version {
            return Err(Error::Stale);
        }
        entry.push_edit(line_edit);
        entry.state.version = after.version();
        entry.state.current_hash = None;
        entry.state.text = after;
        Ok(())
    }

    /// 外部修改后重新载入：新旧文本的公共前缀与公共后缀之外算作一次替换，并入编辑记录。
    /// 扫描两份文本一次（O(n)），只在载入时发生，不在查询路径上。
    pub fn reloaded(
        &self,
        path: &WorkspacePath,
        after: Arc<dyn TextSource>,
        disk_hash: u64,
    ) -> Result<()> {
        let before = lock(&self.docs)
            .get(path.as_str())
            .map(|e| e.state.text.clone())
            .ok_or(Error::NotFound)?;
        let (old, new) = (read_all(&*before)?, read_all(&*after)?);
        let (start, old_end, new_end) = common_affixes(old.as_bytes(), new.as_bytes());
        let line_edit = LineEdit::new(
            before.position(ByteOffset(start as u64))?,
            before.position(ByteOffset(old_end as u64))?,
            &new[start..new_end],
        );
        let mut docs = lock(&self.docs);
        let entry = docs.get_mut(path.as_str()).ok_or(Error::NotFound)?;
        if !Arc::ptr_eq(&entry.state.text, &before) {
            // 扫描期间又有编辑：编辑记录已经不是这两份文本之间的差异。
            return Err(Error::Stale);
        }
        if start != old_end || start != new_end {
            entry.push_edit(line_edit);
        }
        entry.state.version = after.version();
        entry.state.current_hash = Some(disk_hash);
        entry.state.text = after;
        entry.checkpoint(disk_hash);
        Ok(())
    }

    /// 保存：磁盘内容变成当前缓冲区（`disk_hash` 是写出的原始字节的哈希）。编辑记录保留——
    /// 索引重建之前，它仍然是索引版本到当前内容的差异；同时登记一个磁盘版本检查点（#216），
    /// 这样索引重建完成时即使用户已经继续输入，基线也能推进到这一版。
    pub fn saved(&self, path: &WorkspacePath, disk_hash: u64) -> Result<()> {
        let mut docs = lock(&self.docs);
        let entry = docs.get_mut(path.as_str()).ok_or(Error::NotFound)?;
        entry.state.current_hash = Some(disk_hash);
        entry.checkpoint(disk_hash);
        Ok(())
    }

    pub fn closed(&self, path: &WorkspacePath) {
        lock(&self.docs).remove(path.as_str());
    }

    pub fn is_empty(&self) -> bool {
        lock(&self.docs).is_empty()
    }

    pub fn get(&self, path: &str) -> Option<BufferState> {
        lock(&self.docs).get(path).map(|e| e.state.clone())
    }

    /// 索引换代到了哈希 `hash`：把基线推进到那一版，返回**那一版到当前内容**的编辑记录
    /// （空表示文件此刻是干净的）。
    ///
    /// 只有留着 `hash` 的磁盘版本检查点时才能推进（见模块注释的 #216 一节）；对不上返回
    /// `None`，调用方按“无法映射”处理。`version` 与登记的版本对不上时同样返回 `None`：
    /// 调用方手里的快照比登记的旧，返回的编辑记录对它不成立。
    pub(crate) fn rebase(&self, path: &str, version: Version, hash: u64) -> Option<Arc<EditMap>> {
        let mut docs = lock(&self.docs);
        let entry = docs.get_mut(path)?;
        if entry.state.version != version {
            return None;
        }
        if entry.state.base_hash == hash {
            return Some(entry.state.edits.clone());
        }
        // 同一哈希有多个检查点（改了又改回去）时取最近的那个：编辑表最短。
        let i = entry.checkpoints.iter().rposition(|c| c.hash == hash)?;
        let edits = entry.checkpoints.drain(..=i).next_back()?.edits;
        entry.state.base_hash = hash;
        entry.state.edits = edits.clone();
        Some(edits)
    }
}

/// 整份文本（查询路径不用；载入与 L0 回退时用）。
pub(crate) fn read_all(text: &dyn TextSource) -> Result<String> {
    let range = ByteRange::new(ByteOffset(0), ByteOffset(text.byte_len()))?;
    let mut out = String::with_capacity(text.byte_len() as usize);
    text.visit_chunks(range, &mut |chunk| out.push_str(chunk))?;
    Ok(out)
}

/// 公共前缀长度与两侧公共后缀的起点（不切开 UTF-8 字符，也不切开 `\r\n`）。
fn common_affixes(old: &[u8], new: &[u8]) -> (usize, usize, usize) {
    let mut start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    while start > 0 && (is_continuation(old, start) || splits_crlf(old, start)) {
        start -= 1;
    }
    let max_suffix = (old.len() - start).min(new.len() - start);
    let mut suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    while suffix > 0
        && (is_continuation(old, old.len() - suffix) || splits_crlf(old, old.len() - suffix))
    {
        suffix -= 1;
    }
    (start, old.len() - suffix, new.len() - suffix)
}

/// 大段插入通过新快照的行索引求换行数和尾行长度。
/// 跨编辑边界拼成 CRLF 时，边界位置无效，调用者沿用插入文本本身的统计。
fn indexed_line_edit(
    start: Position,
    end: Position,
    edit: &TextEdit,
    after: &dyn TextSource,
) -> Option<LineEdit> {
    let inserted_len = edit.inserted.len() as u64;
    let offset = edit.range.start();
    if inserted_len < 8 * 1024 || after.position(offset).ok()? != start {
        return None;
    }
    let inserted_end = ByteOffset(offset.0.checked_add(inserted_len)?);
    let inserted_end = after.position(inserted_end).ok()?;
    let inserted_breaks = inserted_end.line.0.checked_sub(start.line.0)?;
    Some(LineEdit {
        start,
        end,
        inserted_breaks,
        inserted_tail: if inserted_breaks == 0 {
            inserted_len
        } else {
            inserted_end.column.0
        },
    })
}

fn is_continuation(bytes: &[u8], at: usize) -> bool {
    bytes.get(at).is_some_and(|b| b & 0xc0 == 0x80)
}

fn splits_crlf(bytes: &[u8], at: usize) -> bool {
    at > 0 && bytes.get(at - 1) == Some(&b'\r') && bytes.get(at) == Some(&b'\n')
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_buffer::RopeText;

    /// 在缓冲区开头插入一段文本（`text` 跟着改），返回编辑后的快照。
    fn insert(
        states: &BufferStates,
        path: &WorkspacePath,
        before: &Arc<RopeText>,
        text: &mut String,
        ins: &str,
    ) -> Arc<RopeText> {
        text.insert_str(0, ins);
        let after = Arc::new(RopeText::new(text.clone()));
        let edit = TextEdit {
            version: before.version(),
            range: ByteRange::new(ByteOffset(0), ByteOffset(0)).unwrap(),
            inserted: ins.into(),
        };
        states
            .edited(path, &**before, &edit, after.clone())
            .unwrap();
        after
    }

    #[test]
    fn large_edit_uses_snapshot_coordinates_with_precise_crlf_boundaries() {
        let mixed = "界\r\nα\nβ\r中😀".repeat(1300);
        let same_line = "字😀".repeat(1500);
        for (text, offset, inserted, indexed) in [
            ("ab", 1, mixed.clone(), true),
            ("ab", 1, same_line, true),
            ("a\rb", 2, format!("\n{mixed}"), false),
            ("a\nb", 1, format!("{mixed}\r"), false),
        ] {
            let before = RopeText::from_text(text);
            let mut changed = text.to_owned();
            changed.insert_str(offset, &inserted);
            let after = Arc::new(RopeText::new(changed));
            let byte = ByteOffset(offset as u64);
            let start = before.position(byte).unwrap();
            let expected = LineEdit::new(start, start, &inserted);
            let edit = TextEdit {
                version: before.version(),
                range: ByteRange::new(byte, byte).unwrap(),
                inserted: inserted.into(),
            };
            assert_eq!(
                indexed_line_edit(start, start, &edit, &*after),
                indexed.then_some(expected)
            );

            let path = WorkspacePath::new("Assets/A.cs").unwrap();
            let before = Arc::new(before);
            let buffers = BufferStates::new();
            buffers.opened(&path, before.clone(), 0);
            buffers.edited(&path, &*before, &edit, after).unwrap();
            let mut reference = EditMap::new();
            reference.push(expected);
            assert_eq!(*buffers.get(path.as_str()).unwrap().edits, reference);
        }
    }

    #[test]
    fn unregistered_buffer_edit_is_rejected_before_resolving_the_range() {
        let states = BufferStates::new();
        let path = WorkspacePath::new("Notes.txt").unwrap();
        let before = Arc::new(RopeText::from_text("text"));
        let edit = TextEdit {
            version: before.version(),
            range: ByteRange::new(ByteOffset(100), ByteOffset(100)).unwrap(),
            inserted: "replacement".into(),
        };
        assert!(matches!(
            states.edited(&path, &*before, &edit, before.clone()),
            Err(Error::NotFound)
        ));

        states.opened(&path, before.clone(), 0);
        assert!(matches!(
            states.edited(&path, &*before, &edit, before.clone()),
            Err(Error::OutOfBounds)
        ));
    }

    /// #216：检查点按保存登记、超出上限丢最旧的；`rebase` 推进基线并交回那一版之后的编辑。
    #[test]
    fn rebase_advances_to_a_save_point_and_drops_the_oldest() {
        let states = BufferStates::new();
        let path = WorkspacePath::new("Assets/A.cs").unwrap();
        let mut text = "class A {}\n".to_owned();
        let mut v: Arc<RopeText> = Arc::new(RopeText::from_text(&text));
        states.opened(&path, v.clone(), hash_bytes(text.as_bytes()));
        // 保存 MAX_CHECKPOINTS + 1 次，每次之前都改一点。
        let mut saved = Vec::new();
        for i in 0..=MAX_CHECKPOINTS {
            v = insert(&states, &path, &v, &mut text, &format!("// {i}\n"));
            states.saved(&path, hash_bytes(text.as_bytes())).unwrap();
            saved.push(hash_bytes(text.as_bytes()));
        }
        // 再敲一个字符：当前哈希作废，只剩检查点能把基线接上。
        v = insert(&states, &path, &v, &mut text, "// typing\n");
        let version = v.version();
        assert_eq!(states.get("Assets/A.cs").unwrap().current_hash, None);
        // 最旧的那次保存已经被挤掉；版本对不上也不推进。
        assert!(states.rebase("Assets/A.cs", version, saved[0]).is_none());
        assert!(states
            .rebase("Assets/A.cs", Version(version.0 + 1), saved[1])
            .is_none());
        // 还留着的那次：基线推进过去，交回的是那之后的编辑。
        let edits = states.rebase("Assets/A.cs", version, saved[1]).unwrap();
        assert!(!edits.is_empty());
        assert_eq!(states.get("Assets/A.cs").unwrap().base_hash, saved[1]);
        // 再换一代到最后一次保存：只剩最后那次输入。
        let edits = states
            .rebase("Assets/A.cs", version, saved[MAX_CHECKPOINTS])
            .unwrap();
        assert_eq!(edits.hunks().len(), 1);
        assert_eq!(
            states.get("Assets/A.cs").unwrap().base_hash,
            saved[MAX_CHECKPOINTS]
        );
        // 已经推进过的那一版不再认（检查点已丢弃），但基线自己仍然成立。
        assert!(states.rebase("Assets/A.cs", version, saved[1]).is_none());
        assert!(states
            .rebase("Assets/A.cs", version, saved[MAX_CHECKPOINTS])
            .is_some());
    }

    #[test]
    fn affixes_respect_char_and_crlf_boundaries() {
        assert_eq!(common_affixes(b"abc", b"abc"), (3, 3, 3));
        assert_eq!(common_affixes(b"abXc", b"abYc"), (2, 3, 3));
        assert_eq!(common_affixes(b"x\r\ny", b"x\r\n\r\ny"), (3, 3, 5));
        let (old, new) = ("aé", "aè");
        assert_eq!(common_affixes(old.as_bytes(), new.as_bytes()), (1, 3, 3));
        // 前缀与后缀不重叠。
        assert_eq!(common_affixes(b"aa", b"aaa"), (2, 2, 3));
    }
}
