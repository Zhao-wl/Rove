//! 选区与光标模型（#46）。
//!
//! - [`Selection`]：anchor / active 两个字节偏移 + “期望列”（上下移动跨过短行后回到原来的
//!   可见列，即 VS Code 的 `leftoverVisibleColumns`）。
//! - [`Selections`]：主选区 + 次选区，按创建顺序保存（下标 0 是主选区，与 VS Code 的
//!   `CursorCollection` 一致）；[`Selections::normalize`] 按 `editor.multiCursorLimit` 截断、
//!   按 `editor.multiCursorMergeOverlapping` 合并（移植 `_mergeCursorsIfNecessary`）。
//! - 左右移动按字素簇（CJK、emoji、组合字符都是一个“字符”），上下移动按可见列（Tab 展开到
//!   下一个制表位、全角字符占两列，移植 `CursorColumns`）。
//!
//! 多光标手势与行操作的界面在 R51；这里只实现模型语义。

use rove_base::*;
use unicode_segmentation::UnicodeSegmentation;

use super::lines::{line_count, line_text};
use crate::contracts::TextSource;

/// 一个选区。`anchor == active` 时是光标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: ByteOffset,
    pub active: ByteOffset,
    /// 上下移动时希望回到的可见列；左右移动、编辑后清空。
    pub desired_column: Option<u64>,
}

impl Selection {
    pub fn cursor(at: ByteOffset) -> Self {
        Self::new(at, at)
    }

    pub fn new(anchor: ByteOffset, active: ByteOffset) -> Self {
        Self {
            anchor,
            active,
            desired_column: None,
        }
    }

    pub fn start(&self) -> ByteOffset {
        self.anchor.min(self.active)
    }

    pub fn end(&self) -> ByteOffset {
        self.anchor.max(self.active)
    }

    pub fn range(&self) -> ByteRange {
        Span::new(self.start(), self.end()).expect("start <= end")
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.active
    }

    /// active 在 anchor 之前（从右向左选）。
    pub fn is_reversed(&self) -> bool {
        self.active < self.anchor
    }

    /// 只比较位置，不比较期望列。
    pub fn same_range(&self, other: &Selection) -> bool {
        self.anchor == other.anchor && self.active == other.active
    }
}

/// 一次编辑之后，被编辑的选区变成什么。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorPlacement {
    /// 光标移到插入文本之后（`ReplaceCommand`：输入、粘贴、删除都是这种）。
    AfterInserted,
    /// 选区跟着原来的文字走（`ReplaceCommandThatPreservesSelection`：整行粘贴到当前行之前）。
    Preserve,
}

/// 主选区 + 次选区。永远至少有一个选区。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selections {
    items: Vec<Selection>,
}

impl Selections {
    pub fn single(selection: Selection) -> Self {
        Self {
            items: vec![selection],
        }
    }

    /// 按创建顺序给出，第一个是主选区；空列表返回 [`Error::InvalidInput`]。
    pub fn new(items: Vec<Selection>) -> Result<Self> {
        if items.is_empty() {
            return Err(Error::InvalidInput);
        }
        Ok(Self { items })
    }

    pub fn primary(&self) -> &Selection {
        &self.items[0]
    }

    /// 创建顺序（主选区在前）。
    pub fn all(&self) -> &[Selection] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// 按起点排序的副本（编辑与复制按这个顺序处理）。
    pub fn sorted(&self) -> Vec<Selection> {
        let mut sorted = self.items.clone();
        sorted.sort_by_key(|s| (s.start(), s.end()));
        sorted
    }

    /// 只比较位置。
    pub fn same_ranges(&self, other: &Selections) -> bool {
        self.items.len() == other.items.len()
            && self
                .items
                .iter()
                .zip(&other.items)
                .all(|(a, b)| a.same_range(b))
    }

    /// 截断到 `limit`（`editor.multiCursorLimit`，保留创建顺序靠前的），再按需合并重叠选区。
    ///
    /// 合并规则（VS Code `_mergeCursorsIfNecessary`）：按起点排序后相邻两个选区，任一为空时
    /// “接触”即合并，都不为空时只有真正重叠才合并（首尾相接的两个选区保留）。合并后的选区取
    /// 并集，保留创建顺序更早的那个（主选区永远保留）；方向跟随它，除非被合并掉的是最后创建的
    /// 选区（例如 Ctrl+拖动碰到别的光标），那时方向跟随最后创建的那个。
    pub fn normalize(&mut self, merge_overlapping: bool, limit: usize) {
        if self.items.len() > limit.max(1) {
            self.items.truncate(limit.max(1));
        }
        if !merge_overlapping || self.items.len() < 2 {
            return;
        }
        let last_index = self.items.len() - 1;
        // (创建顺序, 选区)
        let mut sorted: Vec<(usize, Selection)> = self.items.iter().copied().enumerate().collect();
        sorted.sort_by_key(|(_, s)| (s.start(), s.end()));
        let mut i = 0;
        while i + 1 < sorted.len() {
            let (current_index, current) = sorted[i];
            let (next_index, next) = sorted[i + 1];
            let should_merge = if current.is_empty() || next.is_empty() {
                next.start() <= current.end()
            } else {
                next.start() < current.end()
            };
            if !should_merge {
                i += 1;
                continue;
            }
            let (winner_slot, loser_slot) = if current_index < next_index {
                (i, i + 1)
            } else {
                (i + 1, i)
            };
            let (loser_index, loser) = sorted[loser_slot];
            let (_, winner) = sorted[winner_slot];
            if !loser.same_range(&winner) {
                let start = loser.start().min(winner.start());
                let end = loser.end().max(winner.end());
                let left_to_right = if loser_index == last_index {
                    !loser.is_reversed()
                } else {
                    !winner.is_reversed()
                };
                sorted[winner_slot].1 = if left_to_right {
                    Selection::new(start, end)
                } else {
                    Selection::new(end, start)
                };
            }
            sorted.remove(loser_slot);
            // 合并后可能与前一个选区重叠，回退一步重新检查。
            i = i.saturating_sub(1);
        }
        sorted.sort_by_key(|(index, _)| *index);
        self.items = sorted.into_iter().map(|(_, s)| s).collect();
    }

    /// 对每个选区做同样的变换（保持创建顺序），变换不会失败时用它。
    pub(crate) fn map_each(&self, f: impl FnMut(&Selection) -> Selection) -> Self {
        Self {
            items: self.items.iter().map(f).collect(),
        }
    }

    /// 对每个选区做同样的变换（保持创建顺序）。
    pub fn map(&self, mut f: impl FnMut(&Selection) -> Result<Selection>) -> Result<Self> {
        Ok(Self {
            items: self.items.iter().map(&mut f).collect::<Result<_>>()?,
        })
    }
}

/// VS Code `strings.isFullWidthCharacter`：可见列按两列计。
pub fn is_full_width(ch: char) -> bool {
    let code = ch as u32;
    (0x2E80..=0xD7AF).contains(&code)
        || (0xF900..=0xFAFF).contains(&code)
        || (0xFF01..=0xFF5E).contains(&code)
}

fn next_visible_column(visible: u64, grapheme: &str, tab_size: u64) -> u64 {
    let first = grapheme.chars().next().unwrap_or(' ');
    if first == '\t' {
        let tab = tab_size.max(1);
        visible + tab - visible % tab
    } else if is_full_width(first) {
        visible + 2
    } else {
        visible + 1
    }
}

/// UTF-8 列 → 可见列（`CursorColumns.visibleColumnFromColumn`）。
pub fn visible_column(line: &str, column: Utf8Column, tab_size: u64) -> u64 {
    let column = column.0 as usize;
    let mut visible = 0;
    for (at, grapheme) in line.grapheme_indices(true) {
        if at >= column {
            break;
        }
        visible = next_visible_column(visible, grapheme, tab_size);
    }
    visible
}

/// 可见列 → 最接近的 UTF-8 列（`CursorColumns.columnFromVisibleColumn`，距离相等取左边）。
pub fn column_from_visible(line: &str, target: u64, tab_size: u64) -> Utf8Column {
    let mut before = 0;
    for (at, grapheme) in line.grapheme_indices(true) {
        let after = next_visible_column(before, grapheme, tab_size);
        if after >= target {
            let before_delta = target.saturating_sub(before);
            let after_delta = after - target;
            return Utf8Column(if after_delta < before_delta {
                (at + grapheme.len()) as u64
            } else {
                at as u64
            });
        }
        before = after;
    }
    Utf8Column(line.len() as u64)
}

/// 光标向左移动一个字素簇；在行首时移到上一行行尾（CRLF 算一个换行）。
pub fn grapheme_left(text: &dyn TextSource, offset: ByteOffset) -> Result<ByteOffset> {
    let position = text.position(offset)?;
    if position.column.0 == 0 {
        if position.line.0 == 0 {
            return Ok(offset);
        }
        return Ok(text.line_range(Line(position.line.0 - 1))?.end());
    }
    let line = line_text(text, position.line)?;
    let column = position.column.0 as usize;
    let start = line[..column]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(at, _)| at);
    Ok(ByteOffset(offset.0 - (column - start) as u64))
}

/// 光标向右移动一个字素簇；在行尾时移到下一行行首。
pub fn grapheme_right(text: &dyn TextSource, offset: ByteOffset) -> Result<ByteOffset> {
    let position = text.position(offset)?;
    let line = line_text(text, position.line)?;
    let column = position.column.0 as usize;
    if column >= line.len() {
        if position.line.0 + 1 >= line_count(text)? {
            return Ok(offset);
        }
        return Ok(text.line_range(Line(position.line.0 + 1))?.start());
    }
    let length = line[column..].graphemes(true).next().map_or(0, str::len);
    Ok(ByteOffset(offset.0 + length as u64))
}

/// 光标上下移动 `lines` 行（负数向上），保持期望列。
///
/// 第一行再向上移到行首、最后一行再向下移到行尾（VS Code 的行为）；返回的选区带着期望列，
/// 下一次上下移动从它出发。`select` 为真时只移动 active。
pub fn move_vertical(
    text: &dyn TextSource,
    selection: &Selection,
    lines: i64,
    tab_size: u64,
    select: bool,
) -> Result<Selection> {
    let position = text.position(selection.active)?;
    let current_line = line_text(text, position.line)?;
    let desired = selection
        .desired_column
        .unwrap_or_else(|| visible_column(&current_line, position.column, tab_size));
    let count = line_count(text)?;
    let target_line = position.line.0 as i64 + lines;
    let (active, desired_column) = if target_line < 0 {
        (text.line_range(Line(0))?.start(), None)
    } else if target_line as u64 >= count {
        (text.line_range(Line(count - 1))?.end(), None)
    } else {
        let line = Line(target_line as u64);
        let content = line_text(text, line)?;
        let column = column_from_visible(&content, desired, tab_size);
        (text.offset(Position { line, column })?, Some(desired))
    };
    Ok(Selection {
        anchor: if select { selection.anchor } else { active },
        active,
        desired_column,
    })
}
