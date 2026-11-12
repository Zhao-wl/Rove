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
//! 当前缓冲区的完整差异，可以做位置映射。等于当前内容的哈希（保存后索引重建完成）时，编辑记录
//! 作废、文件视为未修改。两者都不等时无法映射，查询降级为近似。
use super::mapping::{EditMap, LineEdit};
use crate::lifecycle::hash::xxh64;
use rove_base::{ByteOffset, ByteRange, Error, Result, TextEdit, Version, WorkspacePath};
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

/// 按项目相对路径登记的缓冲区状态。`Send + Sync`，内部一把短锁。
#[derive(Default)]
pub struct BufferStates {
    docs: Mutex<HashMap<Arc<str>, BufferState>>,
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
        lock(&self.docs).insert(path.as_str().into(), state);
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
        let line_edit = LineEdit::new(
            before.position(edit.range.start())?,
            before.position(edit.range.end())?,
            &edit.inserted,
        );
        let mut docs = lock(&self.docs);
        let state = docs.get_mut(path.as_str()).ok_or(Error::NotFound)?;
        if state.version != edit.version || before.version() != edit.version {
            return Err(Error::Stale);
        }
        Arc::make_mut(&mut state.edits).push(line_edit);
        state.version = after.version();
        state.current_hash = None;
        state.text = after;
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
            .map(|s| s.text.clone())
            .ok_or(Error::NotFound)?;
        let (old, new) = (read_all(&*before)?, read_all(&*after)?);
        let (start, old_end, new_end) = common_affixes(old.as_bytes(), new.as_bytes());
        let line_edit = LineEdit::new(
            before.position(ByteOffset(start as u64))?,
            before.position(ByteOffset(old_end as u64))?,
            &new[start..new_end],
        );
        let mut docs = lock(&self.docs);
        let state = docs.get_mut(path.as_str()).ok_or(Error::NotFound)?;
        if !Arc::ptr_eq(&state.text, &before) {
            // 扫描期间又有编辑：编辑记录已经不是这两份文本之间的差异。
            return Err(Error::Stale);
        }
        if start != old_end || start != new_end {
            Arc::make_mut(&mut state.edits).push(line_edit);
        }
        state.version = after.version();
        state.current_hash = Some(disk_hash);
        state.text = after;
        Ok(())
    }

    /// 保存：磁盘内容变成当前缓冲区（`disk_hash` 是写出的原始字节的哈希）。编辑记录保留——
    /// 索引重建之前，它仍然是索引版本到当前内容的差异。
    pub fn saved(&self, path: &WorkspacePath, disk_hash: u64) -> Result<()> {
        let mut docs = lock(&self.docs);
        let state = docs.get_mut(path.as_str()).ok_or(Error::NotFound)?;
        state.current_hash = Some(disk_hash);
        Ok(())
    }

    pub fn closed(&self, path: &WorkspacePath) {
        lock(&self.docs).remove(path.as_str());
    }

    pub fn is_empty(&self) -> bool {
        lock(&self.docs).is_empty()
    }

    pub fn get(&self, path: &str) -> Option<BufferState> {
        lock(&self.docs).get(path).cloned()
    }

    /// 索引已经包含当前内容（重建完成）：丢弃编辑记录，以当前内容为新基线。
    pub(crate) fn rebase(&self, path: &str, version: Version, hash: u64) {
        if let Some(state) = lock(&self.docs).get_mut(path) {
            if state.version == version && state.current_hash == Some(hash) {
                state.base_hash = hash;
                state.edits = Arc::new(EditMap::new());
            }
        }
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

fn is_continuation(bytes: &[u8], at: usize) -> bool {
    bytes.get(at).is_some_and(|b| b & 0xc0 == 0x80)
}

fn splits_crlf(bytes: &[u8], at: usize) -> bool {
    at > 0 && bytes.get(at - 1) == Some(&b'\r') && bytes.get(at) == Some(&b'\n')
}

#[cfg(test)]
mod tests {
    use super::*;

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
