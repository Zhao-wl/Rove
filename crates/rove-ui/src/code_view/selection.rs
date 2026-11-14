//! 只读选区与插入符移动，行为对照 VS Code（`cursorMoveOperations.ts`、`cursorWordOperations.ts`、
//! `cursorMoveCommands.ts`）。坐标一律是源行 UTF-8 列，移动以字素簇为单位。

use super::columns::{grapheme_boundaries, DisplayLine, LineSource};
use rove_base::{Line, Position, Utf8Column};

/// 单选区：`anchor` 是起点（Shift 扩选时不动），`active` 是插入符所在端。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Position,
    pub active: Position,
}

impl Selection {
    pub fn caret(p: Position) -> Self {
        Self {
            anchor: p,
            active: p,
        }
    }
    pub fn new(anchor: Position, active: Position) -> Self {
        Self { anchor, active }
    }
    pub fn is_empty(&self) -> bool {
        self.anchor == self.active
    }
    pub fn start(&self) -> Position {
        self.anchor.min(self.active)
    }
    pub fn end(&self) -> Position {
        self.anchor.max(self.active)
    }
    /// 插入符是否在选区起点一侧（反向选择）。
    pub fn is_reversed(&self) -> bool {
        self.active < self.anchor
    }
}

pub fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

/// 插入符移动方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Top,
    Bottom,
    PageUp,
    PageDown,
}

/// 字符类别（`editor.wordSeparators`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Regular,
    Whitespace,
    Separator,
}

fn class(c: char, separators: &str) -> Class {
    if c == ' ' || c == '\t' {
        Class::Whitespace
    } else if separators.contains(c) {
        Class::Separator
    } else {
        Class::Regular
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Word {
    start: usize,
    end: usize,
    regular: bool,
}

/// 选区状态：一个选区 + 上下移动时记忆的可见列。
#[derive(Clone, Debug, Default)]
pub struct Cursor {
    pub selection: Selection,
    /// 上下移动保持的可见列（VS Code `leftoverVisibleColumns` 的等价物）。
    desired_column: Option<u32>,
}

/// 拖选的粒度（单击 / 双击 / 三击）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    Character,
    Word,
    Line,
}

impl Cursor {
    pub fn new(selection: Selection) -> Self {
        Self {
            selection,
            desired_column: None,
        }
    }

    pub fn set(&mut self, selection: Selection) {
        self.selection = selection;
        self.desired_column = None;
    }

    /// 软换行下的插入符移动（#58）：`cursorUp` / `cursorDown` / `Home` / `End` /
    /// `PageUp` / `PageDown` 及其 `*Select` 变体按**视觉行**走；`cursorLineStart` /
    /// `cursorLineEnd`（在 `edit.rs` 里）仍按缓冲行走。左右键与其余移动不受影响。
    ///
    /// `segs` 给出每条缓冲行的换行点；没开软换行时传 [`NoWrap`]，行为与
    /// [`move_caret`](Self::move_caret) 完全一致。
    pub fn move_caret_visual(
        &mut self,
        text: &dyn LineSource,
        segs: &dyn LineSegments,
        m: Move,
        extend: bool,
        tab_size: u32,
        page: u64,
    ) {
        let sel = self.selection;
        let vertical = matches!(
            m,
            Move::Up | Move::Down | Move::PageUp | Move::PageDown | Move::Home | Move::End
        );
        if !vertical {
            // 左右移动、文首文末不受软换行影响。
            self.move_caret(text, m, extend, tab_size, page);
            return;
        }
        let last = text.line_count().saturating_sub(1);
        let row = |line: u64| Row::new(text, segs, line, tab_size);
        let from = match m {
            Move::Up | Move::PageUp if !extend && !sel.is_empty() => sel.start(),
            Move::Down | Move::PageDown if !extend && !sel.is_empty() => sel.end(),
            _ => sel.active,
        };
        let here = row(from.line.0);
        let seg = here.segment_of(from.column.0 as u32);
        let target = match m {
            Move::Home => {
                let (start, first) = here.home(seg);
                pos(
                    from.line.0,
                    if from.column.0 != first { first } else { start },
                )
            }
            Move::End => pos(from.line.0, here.end(seg) as u64),
            _ => {
                let down = matches!(m, Move::Down | Move::PageDown);
                let count = if matches!(m, Move::Up | Move::Down) {
                    1
                } else {
                    page.max(1)
                };
                let want = self
                    .desired_column
                    .unwrap_or_else(|| here.column_in_row(seg, from.column.0 as u32));
                self.desired_column = Some(want);
                let mut at = (from.line.0, seg);
                let mut moved = 0;
                while moved < count {
                    let Some(next) = step(text, segs, at, down, last) else {
                        break;
                    };
                    at = next;
                    moved += 1;
                }
                if moved == 0 {
                    // 已经在首 / 末视觉行：VS Code 把插入符送到文首 / 文末。
                    if down {
                        pos(last, text.line_text(last).map_or(0, |t| t.len() as u64))
                    } else {
                        pos(0, 0)
                    }
                } else {
                    let r = row(at.0);
                    pos(at.0, r.column_at(at.1, want) as u64)
                }
            }
        };
        if !matches!(m, Move::Up | Move::Down | Move::PageUp | Move::PageDown) {
            self.desired_column = None;
        }
        self.selection = if extend {
            Selection::new(sel.anchor, target)
        } else {
            Selection::caret(target)
        };
    }

    /// 插入符移动；`extend` 为 `*Select` 变体。`tab_size` 用于可见列；`page` 为一页的行数。
    pub fn move_caret(
        &mut self,
        text: &dyn LineSource,
        m: Move,
        extend: bool,
        tab_size: u32,
        page: u64,
    ) {
        let sel = self.selection;
        let last = text.line_count().saturating_sub(1);
        let line_len = |l: u64| text.line_text(l).map_or(0, |t| t.len() as u64);
        let keep_column = matches!(m, Move::Up | Move::Down | Move::PageUp | Move::PageDown);
        let target = match m {
            Move::Left if !extend && !sel.is_empty() => sel.start(),
            Move::Right if !extend && !sel.is_empty() => sel.end(),
            Move::Left => {
                let p = sel.active;
                if p.column.0 == 0 {
                    if p.line.0 == 0 {
                        p
                    } else {
                        pos(p.line.0 - 1, line_len(p.line.0 - 1))
                    }
                } else {
                    let t = text.line_text(p.line.0).unwrap_or_default();
                    let b = grapheme_boundaries(&t);
                    let i = b.partition_point(|&x| (x as u64) < p.column.0);
                    pos(p.line.0, b[i.saturating_sub(1)] as u64)
                }
            }
            Move::Right => {
                let p = sel.active;
                let t = text.line_text(p.line.0).unwrap_or_default();
                if p.column.0 >= t.len() as u64 {
                    if p.line.0 >= last {
                        p
                    } else {
                        pos(p.line.0 + 1, 0)
                    }
                } else {
                    let b = grapheme_boundaries(&t);
                    let i = b.partition_point(|&x| (x as u64) <= p.column.0);
                    pos(p.line.0, b[i.min(b.len() - 1)] as u64)
                }
            }
            Move::Up | Move::Down | Move::PageUp | Move::PageDown => {
                let down = matches!(m, Move::Down | Move::PageDown);
                let count = if matches!(m, Move::Up | Move::Down) {
                    1
                } else {
                    page.max(1)
                };
                // VS Code：非扩选且有选区时，上移从选区起点、下移从选区终点出发。
                let from = if !extend && !sel.is_empty() {
                    if down {
                        sel.end()
                    } else {
                        sel.start()
                    }
                } else {
                    sel.active
                };
                let from_text = text.line_text(from.line.0).unwrap_or_default();
                let column = self.desired_column.unwrap_or_else(|| {
                    DisplayLine::build(&from_text, tab_size, None)
                        .visible_column(from.column.0 as u32)
                });
                self.desired_column = Some(column);
                if down {
                    if from.line.0 >= last {
                        pos(last, from_text.len() as u64)
                    } else {
                        let l = (from.line.0 + count).min(last);
                        column_on_line(text, l, column, tab_size)
                    }
                } else if from.line.0 == 0 {
                    pos(0, 0)
                } else {
                    let l = from.line.0.saturating_sub(count);
                    column_on_line(text, l, column, tab_size)
                }
            }
            Move::Home => {
                let p = sel.active;
                let t = text.line_text(p.line.0).unwrap_or_default();
                let first = t.find(|c: char| c != ' ' && c != '\t').unwrap_or(t.len()) as u64;
                // 不在首个非空白处 → 去首个非空白；已经在 → 去行首。
                pos(p.line.0, if p.column.0 != first { first } else { 0 })
            }
            Move::End => pos(sel.active.line.0, line_len(sel.active.line.0)),
            Move::Top => pos(0, 0),
            Move::Bottom => pos(last, line_len(last)),
        };
        if !keep_column {
            self.desired_column = None;
        }
        self.selection = if extend {
            Selection::new(sel.anchor, target)
        } else {
            Selection::caret(target)
        };
    }

    /// `editor.action.selectAll`。
    pub fn select_all(&mut self, text: &dyn LineSource) {
        let last = text.line_count().saturating_sub(1);
        let len = text.line_text(last).map_or(0, |t| t.len() as u64);
        self.set(Selection::new(pos(0, 0), pos(last, len)));
    }

    /// `expandLineSelection`（ctrl+l）：从选区首行行首选到末行的下一行行首；末行则到行尾。
    /// 重复执行时选区终点已在下一行行首，于是每次再向下扩一行（与 VS Code 相同）。
    pub fn expand_line_selection(&mut self, text: &dyn LineSource) {
        let sel = self.selection;
        let last = text.line_count().saturating_sub(1);
        let start = sel.start().line.0;
        let end = sel.end().line.0;
        let target = if end >= last {
            pos(last, text.line_text(last).map_or(0, |t| t.len() as u64))
        } else {
            pos(end + 1, 0)
        };
        self.set(Selection::new(pos(start, 0), target));
    }

    /// `cancelSelection`（escape）：折叠到插入符。
    pub fn cancel_selection(&mut self) {
        self.set(Selection::caret(self.selection.active));
    }
}

/// 软换行下每条缓冲行的分段（由 #58 的 `LineWrap` 提供）。关闭软换行时用 [`NoWrap`]。
pub trait LineSegments {
    /// 续行起点（源行 UTF-8 字节列，升序，不含 0）；空表示这一行不换行。
    fn breaks(&self, line: u64) -> Vec<u32>;
    /// 续行缩进（可见列数）。
    fn indent_columns(&self, line: u64) -> u32;
}

/// 不换行。
pub struct NoWrap;

impl LineSegments for NoWrap {
    fn breaks(&self, _: u64) -> Vec<u32> {
        Vec::new()
    }
    fn indent_columns(&self, _: u64) -> u32 {
        0
    }
}

/// 一条缓冲行在软换行下的分段几何（可见列 ↔ 源列，按段）。
struct Row {
    text: String,
    display: DisplayLine,
    breaks: Vec<u32>,
    indent: u32,
}

impl Row {
    fn new(text: &dyn LineSource, segs: &dyn LineSegments, line: u64, tab_size: u32) -> Self {
        let t = text.line_text(line).unwrap_or_default();
        Self {
            display: DisplayLine::build(&t, tab_size, None),
            breaks: segs.breaks(line),
            indent: segs.indent_columns(line),
            text: t,
        }
    }
    fn count(&self) -> u32 {
        self.breaks.len() as u32 + 1
    }
    fn segment_of(&self, column: u32) -> u32 {
        self.breaks.partition_point(|&b| b <= column) as u32
    }
    fn start(&self, seg: u32) -> u32 {
        if seg == 0 {
            0
        } else {
            self.breaks[(seg as usize - 1).min(self.breaks.len() - 1)]
        }
    }
    /// 本段最后一个可以停插入符的源列：末段是行尾，其余段是换行点之前的那个字素簇边界
    /// （换行点本身属于下一条视觉行）。
    fn end(&self, seg: u32) -> u32 {
        match self.breaks.get(seg as usize) {
            None => self.text.len() as u32,
            Some(&b) => {
                let g = grapheme_boundaries(&self.text);
                let i = g.partition_point(|&x| x < b);
                g[i.saturating_sub(1)].max(self.start(seg))
            }
        }
    }
    /// 本段的行首位置：(段首, 段内首个非空白)。
    fn home(&self, seg: u32) -> (u64, u64) {
        let (s, e) = (self.start(seg), self.end(seg));
        let slice = &self.text[s as usize..e as usize];
        let first = slice
            .find(|c: char| c != ' ' && c != '\t')
            .map_or(e, |i| s + i as u32);
        (s as u64, first as u64)
    }
    fn indent_of(&self, seg: u32) -> u32 {
        if seg == 0 {
            0
        } else {
            self.indent
        }
    }
    /// 源列 → 它在自己那条视觉行里的可见列（含续行缩进）。
    fn column_in_row(&self, seg: u32, column: u32) -> u32 {
        self.display.visible_column(column) - self.display.visible_column(self.start(seg))
            + self.indent_of(seg)
    }
    /// 视觉行内的可见列 → 源列（钳在本段内）。
    fn column_at(&self, seg: u32, want: u32) -> u32 {
        let seg = seg.min(self.count() - 1);
        let base = self.display.visible_column(self.start(seg));
        let visible = base + want.saturating_sub(self.indent_of(seg));
        self.display
            .source_for_visible_column(visible)
            .clamp(self.start(seg), self.end(seg))
    }
}

/// 上 / 下一条视觉行的 (缓冲行, 段)。
fn step(
    text: &dyn LineSource,
    segs: &dyn LineSegments,
    at: (u64, u32),
    down: bool,
    last: u64,
) -> Option<(u64, u32)> {
    let count = |l: u64| segs.breaks(l).len() as u32 + 1;
    let _ = text;
    let (line, seg) = at;
    if down {
        if seg + 1 < count(line) {
            Some((line, seg + 1))
        } else if line < last {
            Some((line + 1, 0))
        } else {
            None
        }
    } else if seg > 0 {
        Some((line, seg - 1))
    } else if line > 0 {
        Some((line - 1, count(line - 1) - 1))
    } else {
        None
    }
}

fn column_on_line(text: &dyn LineSource, line: u64, column: u32, tab_size: u32) -> Position {
    let t = text.line_text(line).unwrap_or_default();
    let d = DisplayLine::build(&t, tab_size, None);
    pos(line, d.source_for_visible_column(column) as u64)
}

/// VS Code `_findPreviousWordOnLine`：位置左侧字符所在的词（可向右延伸）。
fn previous_word(chars: &[(usize, char)], len: usize, at: usize, seps: &str) -> Option<Word> {
    // at 是字符下标（位置左侧字符为 at-1）。
    let mut kind: Option<Class> = None;
    let mut i = at;
    while i > 0 {
        let c = class(chars[i - 1].1, seps);
        match (c, kind) {
            (Class::Whitespace, Some(k)) => return Some(word_from(chars, len, i, k, seps)),
            (Class::Whitespace, None) => {}
            (c, Some(k)) if c != k => return Some(word_from(chars, len, i, k, seps)),
            (c, _) => kind = Some(c),
        }
        i -= 1;
    }
    kind.map(|k| word_from(chars, len, 0, k, seps))
}

/// VS Code `_findNextWordOnLine`：位置右侧字符所在的词（可向左延伸）。
fn next_word(chars: &[(usize, char)], len: usize, at: usize, seps: &str) -> Option<Word> {
    let mut kind: Option<Class> = None;
    let mut i = at;
    while i < chars.len() {
        let c = class(chars[i].1, seps);
        match (c, kind) {
            (Class::Whitespace, Some(k)) => return Some(word_to(chars, len, i, k, seps)),
            (Class::Whitespace, None) => {}
            (c, Some(k)) if c != k => return Some(word_to(chars, len, i, k, seps)),
            (c, _) => kind = Some(c),
        }
        i += 1;
    }
    kind.map(|k| word_to(chars, len, chars.len(), k, seps))
}

fn byte(chars: &[(usize, char)], len: usize, i: usize) -> usize {
    chars.get(i).map_or(len, |c| c.0)
}

/// 从字符下标 `start` 开始、类别为 `k` 的词（向右找终点）。
fn word_from(chars: &[(usize, char)], len: usize, start: usize, k: Class, seps: &str) -> Word {
    let mut end = start;
    while end < chars.len() && class(chars[end].1, seps) == k {
        end += 1;
    }
    Word {
        start: byte(chars, len, start),
        end: byte(chars, len, end),
        regular: k == Class::Regular,
    }
}

/// 在字符下标 `end` 结束、类别为 `k` 的词（向左找起点）。
fn word_to(chars: &[(usize, char)], len: usize, end: usize, k: Class, seps: &str) -> Word {
    let mut start = end;
    while start > 0 && class(chars[start - 1].1, seps) == k {
        start -= 1;
    }
    Word {
        start: byte(chars, len, start),
        end: byte(chars, len, end),
        regular: k == Class::Regular,
    }
}

/// 双击选词（VS Code `WordOperations.word`，首次进入词选择）：返回 `[start, end)` 字节列。
pub fn word_range_at(line: &str, column: usize, separators: &str) -> (usize, usize) {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let at = chars.partition_point(|c| c.0 < column);
    let prev = previous_word(&chars, line.len(), at, separators);
    let next = next_word(&chars, line.len(), at, separators);
    let touching =
        |w: &Option<Word>| w.filter(|w| w.regular && w.start <= column && column <= w.end);
    if let Some(w) = touching(&prev).or_else(|| touching(&next)) {
        return (w.start, w.end);
    }
    // VS Code 用 Range(prev.end, next.start) 构造，起止颠倒时 Range 会自动交换。
    let start = prev.map_or(0, |w| w.end);
    let end = next.map_or(line.len(), |w| w.start);
    (start.min(end), start.max(end))
}

/// 拖选时按词扩展：位置严格在某个普通词内部时扩到整个词，否则就是位置本身。
pub fn word_range_for_drag(line: &str, column: usize, separators: &str) -> (usize, usize) {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let at = chars.partition_point(|c| c.0 < column);
    let inside = |w: Option<Word>| w.filter(|w| w.regular && w.start < column && column < w.end);
    match inside(previous_word(&chars, line.len(), at, separators))
        .or_else(|| inside(next_word(&chars, line.len(), at, separators)))
    {
        Some(w) => (w.start, w.end),
        None => (column, column),
    }
}

/// 把按粒度扩展的拖选组合成选区：`origin` 是按下时的单位范围，`current` 是当前位置的单位范围。
pub fn extend_by_unit(origin: (Position, Position), current: (Position, Position)) -> Selection {
    if current.0 < origin.0 {
        Selection::new(origin.1, current.0)
    } else {
        Selection::new(origin.0, current.1.max(origin.1))
    }
}

/// 三击选行：整行加行终止符（末行到行尾）。
pub fn line_unit(text: &dyn LineSource, line: u64) -> (Position, Position) {
    let last = text.line_count().saturating_sub(1);
    if line >= last {
        (
            pos(last, 0),
            pos(last, text.line_text(last).map_or(0, |t| t.len() as u64)),
        )
    } else {
        (pos(line, 0), pos(line + 1, 0))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub struct Lines(pub Vec<String>);
    impl LineSource for Lines {
        fn line_count(&self) -> u64 {
            self.0.len() as u64
        }
        fn line_text(&self, line: u64) -> Option<String> {
            self.0.get(line as usize).cloned()
        }
    }
    pub fn lines(s: &[&str]) -> Lines {
        Lines(s.iter().map(|s| s.to_string()).collect())
    }

    const SEPS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

    #[test]
    fn double_click_selects_like_vscode() {
        let l = "foo.bar  baz 中文词 x";
        assert_eq!(word_range_at(l, 1, SEPS), (0, 3));
        assert_eq!(word_range_at(l, 3, SEPS), (0, 3)); // 紧贴前词
        assert_eq!(word_range_at(l, 4, SEPS), (4, 7)); // 紧贴后词
        assert_eq!(word_range_at(l, 8, SEPS), (7, 9)); // 空白区
        let cjk = l.find('中').unwrap();
        assert_eq!(word_range_at(l, cjk + 3, SEPS), (cjk, cjk + 9));
        // 落在分隔符段中间：选中整段分隔符
        assert_eq!(word_range_at("a..b", 2, SEPS), (1, 3));
        assert_eq!(word_range_at("(())", 2, SEPS), (0, 4));
        assert_eq!(word_range_at("", 0, SEPS), (0, 0));
    }

    #[test]
    fn arrows_move_by_grapheme_and_wrap_lines() {
        let family = "👨‍👩‍👧‍👦";
        let t = lines(&[&format!("a{family}b"), "xyz"]);
        let mut c = Cursor::new(Selection::caret(pos(0, 1)));
        c.move_caret(&t, Move::Right, false, 4, 10);
        assert_eq!(c.selection.active, pos(0, 1 + family.len() as u64));
        c.move_caret(&t, Move::Left, false, 4, 10);
        assert_eq!(c.selection.active, pos(0, 1));
        c.set(Selection::caret(pos(0, 2 + family.len() as u64)));
        c.move_caret(&t, Move::Right, false, 4, 10);
        assert_eq!(c.selection.active, pos(1, 0));
        c.move_caret(&t, Move::Left, true, 4, 10);
        assert_eq!(
            c.selection,
            Selection::new(pos(1, 0), pos(0, 2 + family.len() as u64))
        );
        // 有选区时不带 Shift 的左右键折叠到选区端点
        c.move_caret(&t, Move::Right, false, 4, 10);
        assert_eq!(c.selection, Selection::caret(pos(1, 0)));
    }

    #[test]
    fn vertical_moves_keep_visible_column_across_tabs_and_cjk() {
        let t = lines(&["\tabcdef", "中文字", "x", "0123456789"]);
        let mut c = Cursor::new(Selection::caret(pos(0, 3))); // 可见列 6
        c.move_caret(&t, Move::Down, false, 4, 10);
        assert_eq!(c.selection.active, pos(1, 9)); // 中文字 各 2 列：第 6 列在“字”之后
        c.move_caret(&t, Move::Down, false, 4, 10);
        assert_eq!(c.selection.active, pos(2, 1));
        c.move_caret(&t, Move::Down, false, 4, 10);
        assert_eq!(c.selection.active, pos(3, 6)); // 记忆列恢复
        c.move_caret(&t, Move::Down, false, 4, 10);
        assert_eq!(c.selection.active, pos(3, 10)); // 末行再下移到行尾
        c.move_caret(&t, Move::Top, false, 4, 10);
        c.move_caret(&t, Move::Up, false, 4, 10);
        assert_eq!(c.selection.active, pos(0, 0));
        c.move_caret(&t, Move::PageDown, true, 4, 2);
        assert_eq!(c.selection, Selection::new(pos(0, 0), pos(2, 0)));
    }

    #[test]
    fn home_toggles_first_non_whitespace_and_end_goes_to_eol() {
        let t = lines(&["    code();"]);
        let mut c = Cursor::new(Selection::caret(pos(0, 8)));
        c.move_caret(&t, Move::Home, false, 4, 1);
        assert_eq!(c.selection.active, pos(0, 4));
        c.move_caret(&t, Move::Home, false, 4, 1);
        assert_eq!(c.selection.active, pos(0, 0));
        c.move_caret(&t, Move::Home, false, 4, 1);
        assert_eq!(c.selection.active, pos(0, 4));
        c.move_caret(&t, Move::End, true, 4, 1);
        assert_eq!(c.selection, Selection::new(pos(0, 4), pos(0, 11)));
    }

    #[test]
    fn ctrl_a_ctrl_l_and_escape() {
        let t = lines(&["one", "two", "three"]);
        let mut c = Cursor::new(Selection::caret(pos(1, 1)));
        c.expand_line_selection(&t);
        assert_eq!(c.selection, Selection::new(pos(1, 0), pos(2, 0)));
        c.expand_line_selection(&t);
        assert_eq!(c.selection, Selection::new(pos(1, 0), pos(2, 5)));
        c.cancel_selection();
        assert_eq!(c.selection, Selection::caret(pos(2, 5)));
        c.select_all(&t);
        assert_eq!(c.selection, Selection::new(pos(0, 0), pos(2, 5)));
        let mut c = Cursor::new(Selection::caret(pos(0, 0)));
        c.expand_line_selection(&t);
        c.expand_line_selection(&t);
        assert_eq!(c.selection, Selection::new(pos(0, 0), pos(2, 0)));
    }

    #[test]
    fn drag_by_words_and_lines() {
        let l = "alpha beta gamma";
        let origin = (pos(0, 6), pos(0, 10));
        let w = word_range_for_drag(l, 13, SEPS);
        assert_eq!(w, (11, 16));
        assert_eq!(
            extend_by_unit(origin, (pos(0, w.0 as u64), pos(0, w.1 as u64))),
            Selection::new(pos(0, 6), pos(0, 16))
        );
        let w = word_range_for_drag(l, 2, SEPS);
        assert_eq!(
            extend_by_unit(origin, (pos(0, w.0 as u64), pos(0, w.1 as u64))),
            Selection::new(pos(0, 10), pos(0, 0))
        );
        let t = lines(&["a", "b", "c"]);
        assert_eq!(line_unit(&t, 1), (pos(1, 0), pos(2, 0)));
        assert_eq!(line_unit(&t, 2), (pos(2, 0), pos(2, 1)));
    }
}
