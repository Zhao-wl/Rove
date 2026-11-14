//! 多光标、列选择与行操作的界面接线（#52）。
//!
//! 模型都在别处：多选区语义（合并、上限、方向）在 #46 的
//! [`Selections`](rove_buffer::edit::Selections)，匹配查找在 #21 的
//! [`find::engine`](crate::find::engine)，行操作的事务在 [`super::lineops`]，保存时的尾随空白
//! 变换在 #49。本模块只负责：
//!
//! - **视图里的次光标**：[`CodeView`] 的主选区仍然是 `cursor`，次光标是 `secondary`；
//!   写进会话时按“主选区在前”的顺序交给 #46，读回来时同样顺序。
//! - **修饰键的唯一来源**：`editor.multiCursorModifier` 决定“加光标”和 #40 的“转到定义”
//!   各用哪个修饰键（[`MultiCursorModifier::adds_cursor`] / [`MultiCursorModifier::goes_to_definition`]），
//!   两边都读这里，不各读各的设置。
//! - **列选择**：[`ColumnSelect`] 是 VS Code `columnSelection.ts` 的移植；列对齐一律用 #19 的
//!   [`DisplayLine`](super::columns::DisplayLine)（Tab 到制表位、全角两列），不按字节数。
//!   超长行（`editor.stopRenderingLineAfter` 之后）没有渲染结果可依据，按 VS Code 的做法
//!   限制在已渲染范围内。
//! - **按匹配添加选区**：`Ctrl+D` 一族复用查找条的 [`Searcher`](crate::find::engine::Searcher)
//!   与 `editor.wordSeparators`；查找条可见时区分大小写等选项取它的当前状态
//!   （[`CodeView::set_find_match_options`]）。

use gpui::{Context, Window};
use rove_base::{Cancellation, Position};
use rove_buffer::edit as ed;
use rove_buffer::TextSource;

use super::columns::{DisplayLine, LineSource, TextLines};
use super::lineops::{self, CaseKind, LineOp};
use super::scroll::Reveal;
use super::selection::{pos, word_range_at, Cursor, Move, Selection};
use super::view::{CodeView, SelectionSource};
use crate::find::engine::{self, FindOptions, Query, Searcher, MATCHES_LIMIT};

/// `editor.multiCursorModifier`：加光标用哪个修饰键。
///
/// VS Code 的规则是“多光标修饰键与跳转修饰键互换”：默认 `alt` 时 `Alt+点击`加光标、
/// `Ctrl+点击`跳转；设为 `ctrlCmd` 时两者交换。#40 的 `Ctrl+点击`转到定义与这里共用同一个判定。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MultiCursorModifier {
    #[default]
    Alt,
    CtrlCmd,
}

impl MultiCursorModifier {
    pub fn parse(value: &str) -> Self {
        match value {
            "ctrlCmd" => Self::CtrlCmd,
            _ => Self::Alt,
        }
    }

    /// 这次点击是“加光标”（Windows 上 `ctrlCmd` 即 Ctrl）。
    pub fn adds_cursor(self, ctrl: bool, alt: bool) -> bool {
        match self {
            Self::Alt => alt && !ctrl,
            Self::CtrlCmd => ctrl && !alt,
        }
    }

    /// 这次点击是 #40 的“转到定义”（与加光标互换的那一个）。
    pub fn goes_to_definition(self, ctrl: bool, alt: bool) -> bool {
        match self {
            Self::Alt => ctrl,
            Self::CtrlCmd => alt,
        }
    }
}

/// 列选择状态（VS Code `IColumnSelectData`）：起止行与起止**可见列**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnSelect {
    pub from_line: u64,
    pub from_visual: u32,
    pub to_line: u64,
    pub to_visual: u32,
}

/// 按可见列在一行上取源列（Tab 展开到制表位、全角两列；超长行限制在已渲染范围内）。
fn source_column(line: &str, visual: u32, tab_size: u32, limit: Option<usize>) -> u32 {
    DisplayLine::build(line, tab_size, limit).source_for_visible_column(visual)
}

fn visual_column(line: &str, source: u32, tab_size: u32, limit: Option<usize>) -> u32 {
    DisplayLine::build(line, tab_size, limit).visible_column(source)
}

impl ColumnSelect {
    /// 从一对位置建立（`Shift+Alt+拖动`的起点与当前点）。
    pub fn between(
        text: &dyn LineSource,
        anchor: Position,
        active: Position,
        tab_size: u32,
        limit: Option<usize>,
    ) -> Self {
        let line_of = |p: Position| text.line_text(p.line.0).unwrap_or_default();
        Self {
            from_line: anchor.line.0,
            from_visual: visual_column(&line_of(anchor), anchor.column.0 as u32, tab_size, limit),
            to_line: active.line.0,
            to_visual: visual_column(&line_of(active), active.column.0 as u32, tab_size, limit),
        }
    }

    /// 命令移动终点（`cursorColumnSelect*`）。
    pub fn moved(self, m: Move, lines: u64, last_line: u64) -> Self {
        let mut next = self;
        match m {
            Move::Left => next.to_visual = next.to_visual.saturating_sub(1),
            Move::Right => next.to_visual += 1,
            Move::Up => next.to_line = next.to_line.saturating_sub(1),
            Move::Down => next.to_line = (next.to_line + 1).min(last_line),
            Move::PageUp => next.to_line = next.to_line.saturating_sub(lines.max(1)),
            Move::PageDown => next.to_line = (next.to_line + lines.max(1)).min(last_line),
            _ => {}
        }
        next
    }

    /// 展开成每行一个选区（VS Code `ColumnSelection.columnSelect`）。主选区是终点所在的行。
    pub fn selections(
        self,
        text: &dyn LineSource,
        tab_size: u32,
        limit: Option<usize>,
    ) -> Vec<Selection> {
        let reversed = self.from_line > self.to_line;
        let ltr = self.from_visual < self.to_visual;
        let rtl = self.from_visual > self.to_visual;
        let count = (self.from_line as i64 - self.to_line as i64).unsigned_abs() + 1;
        let mut out = Vec::with_capacity(count as usize);
        for step in 0..count {
            let line = if reversed {
                self.from_line - step
            } else {
                self.from_line + step
            };
            let content = text.line_text(line).unwrap_or_default();
            let start = source_column(&content, self.from_visual, tab_size, limit);
            let end = source_column(&content, self.to_visual, tab_size, limit);
            let visible_start = visual_column(&content, start, tab_size, limit);
            let visible_end = visual_column(&content, end, tab_size, limit);
            if ltr && (visible_start > self.to_visual || visible_end < self.from_visual) {
                continue;
            }
            if rtl && (visible_end > self.from_visual || visible_start < self.to_visual) {
                continue;
            }
            out.push(Selection::new(
                pos(line, start as u64),
                pos(line, end as u64),
            ));
        }
        // 终点所在的行是主选区（下标 0）。
        let primary = out.iter().position(|s| s.active.line.0 == self.to_line);
        if let Some(i) = primary {
            out.swap(0, i);
        }
        out
    }
}

/// `Ctrl+D` 会话：当前在找什么。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MatchSession {
    pub text: String,
    pub whole_word: bool,
}

/// 本 issue 新增的命令（VS Code ID）。
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (
        "editor.action.insertCursorAbove",
        "codeView.command.insertCursorAbove",
        "在上面添加光标",
    ),
    (
        "editor.action.insertCursorBelow",
        "codeView.command.insertCursorBelow",
        "在下面添加光标",
    ),
    (
        "editor.action.insertCursorAtEndOfEachLineSelected",
        "codeView.command.insertCursorAtEndOfEachLineSelected",
        "在行尾添加光标",
    ),
    (
        "removeSecondaryCursors",
        "codeView.command.removeSecondaryCursors",
        "删除次要光标",
    ),
    (
        "editor.action.addSelectionToNextFindMatch",
        "codeView.command.addSelectionToNextFindMatch",
        "将选择添加到下一个查找匹配项",
    ),
    (
        "editor.action.addSelectionToPreviousFindMatch",
        "codeView.command.addSelectionToPreviousFindMatch",
        "将选择添加到上一个查找匹配项",
    ),
    (
        "editor.action.moveSelectionToNextFindMatch",
        "codeView.command.moveSelectionToNextFindMatch",
        "将上次选择移到下一个查找匹配项",
    ),
    (
        "editor.action.moveSelectionToPreviousFindMatch",
        "codeView.command.moveSelectionToPreviousFindMatch",
        "将上次选择移到上一个查找匹配项",
    ),
    (
        "editor.action.selectHighlights",
        "codeView.command.selectHighlights",
        "选择所有出现的匹配项",
    ),
    (
        "editor.action.changeAll",
        "codeView.command.changeAll",
        "更改所有匹配项",
    ),
    (
        "cursorColumnSelectUp",
        "codeView.command.cursorColumnSelectUp",
        "列选择上移",
    ),
    (
        "cursorColumnSelectDown",
        "codeView.command.cursorColumnSelectDown",
        "列选择下移",
    ),
    (
        "cursorColumnSelectLeft",
        "codeView.command.cursorColumnSelectLeft",
        "列选择左移",
    ),
    (
        "cursorColumnSelectRight",
        "codeView.command.cursorColumnSelectRight",
        "列选择右移",
    ),
    (
        "cursorColumnSelectPageUp",
        "codeView.command.cursorColumnSelectPageUp",
        "列选择上移一页",
    ),
    (
        "cursorColumnSelectPageDown",
        "codeView.command.cursorColumnSelectPageDown",
        "列选择下移一页",
    ),
    (
        "editor.action.copyLinesUpAction",
        "codeView.command.copyLinesUp",
        "向上复制行",
    ),
    (
        "editor.action.copyLinesDownAction",
        "codeView.command.copyLinesDown",
        "向下复制行",
    ),
    (
        "editor.action.moveLinesUpAction",
        "codeView.command.moveLinesUp",
        "向上移动行",
    ),
    (
        "editor.action.moveLinesDownAction",
        "codeView.command.moveLinesDown",
        "向下移动行",
    ),
    (
        "editor.action.deleteLines",
        "codeView.command.deleteLines",
        "删除行",
    ),
    (
        "editor.action.trimTrailingWhitespace",
        "codeView.command.trimTrailingWhitespace",
        "删除尾随空格",
    ),
    (
        "editor.action.duplicateSelection",
        "codeView.command.duplicateSelection",
        "重复选定内容",
    ),
    (
        "editor.action.joinLines",
        "codeView.command.joinLines",
        "合并行",
    ),
    (
        "editor.action.sortLinesAscending",
        "codeView.command.sortLinesAscending",
        "按升序排列行",
    ),
    (
        "editor.action.sortLinesDescending",
        "codeView.command.sortLinesDescending",
        "按降序排列行",
    ),
    (
        "editor.action.transpose",
        "codeView.command.transpose",
        "转置光标处的字符",
    ),
    (
        "editor.action.transposeLetters",
        "codeView.command.transposeLetters",
        "转置字母",
    ),
    (
        "editor.action.transformToUppercase",
        "codeView.command.transformToUppercase",
        "转换为大写",
    ),
    (
        "editor.action.transformToLowercase",
        "codeView.command.transformToLowercase",
        "转换为小写",
    ),
    (
        "editor.action.transformToTitlecase",
        "codeView.command.transformToTitlecase",
        "转换为词首字母大写",
    ),
    (
        "editor.action.transformToCamelcase",
        "codeView.command.transformToCamelcase",
        "转换为驼峰拼写法",
    ),
    (
        "editor.action.transformToPascalcase",
        "codeView.command.transformToPascalcase",
        "转换为 Pascal 拼写法",
    ),
    (
        "editor.action.transformToSnakecase",
        "codeView.command.transformToSnakecase",
        "转换为蛇形拼写法",
    ),
    (
        "editor.action.transformToKebabcase",
        "codeView.command.transformToKebabcase",
        "转换为短横线拼写法",
    ),
];

/// 存在次光标时发布（`removeSecondaryCursors` 等的 when 子句）。
pub const HAS_MULTIPLE_SELECTIONS_KEY: &str = "editorHasMultipleSelections";
/// `editor.columnSelection` 为真时发布（`shift+方向键` 直接进入列选择）。
pub const COLUMN_SELECTION_KEY: &str = "editorColumnSelection";

/// 命令 ID → 行操作。
fn line_op(id: &str) -> Option<LineOp> {
    Some(match id {
        "editor.action.moveLinesUpAction" => LineOp::MoveUp,
        "editor.action.moveLinesDownAction" => LineOp::MoveDown,
        "editor.action.copyLinesUpAction" => LineOp::CopyUp,
        "editor.action.copyLinesDownAction" => LineOp::CopyDown,
        "editor.action.deleteLines" => LineOp::Delete,
        "editor.action.duplicateSelection" => LineOp::DuplicateSelection,
        "editor.action.joinLines" => LineOp::JoinLines,
        "editor.action.sortLinesAscending" => LineOp::SortAscending,
        "editor.action.sortLinesDescending" => LineOp::SortDescending,
        "editor.action.transpose" => LineOp::Transpose,
        "editor.action.transposeLetters" => LineOp::TransposeLetters,
        "editor.action.trimTrailingWhitespace" => LineOp::TrimTrailingWhitespace,
        "editor.action.transformToUppercase" => LineOp::Case(CaseKind::Upper),
        "editor.action.transformToLowercase" => LineOp::Case(CaseKind::Lower),
        "editor.action.transformToTitlecase" => LineOp::Case(CaseKind::Title),
        "editor.action.transformToCamelcase" => LineOp::Case(CaseKind::Camel),
        "editor.action.transformToPascalcase" => LineOp::Case(CaseKind::Pascal),
        "editor.action.transformToSnakecase" => LineOp::Case(CaseKind::Snake),
        "editor.action.transformToKebabcase" => LineOp::Case(CaseKind::Kebab),
        _ => return None,
    })
}

/// 命令 ID → 列选择方向。
fn column_move(id: &str) -> Option<Move> {
    Some(match id {
        "cursorColumnSelectUp" => Move::Up,
        "cursorColumnSelectDown" => Move::Down,
        "cursorColumnSelectLeft" => Move::Left,
        "cursorColumnSelectRight" => Move::Right,
        "cursorColumnSelectPageUp" => Move::PageUp,
        "cursorColumnSelectPageDown" => Move::PageDown,
        _ => return None,
    })
}

impl CodeView {
    /// `editor.multiCursorModifier`：**唯一**的修饰键来源（#40 的鼠标跳转也读这里）。
    pub fn multi_cursor_modifier(&self) -> MultiCursorModifier {
        self.options.multi_cursor_modifier
    }

    // ------------------------------------------------------------------ 多选区读写

    /// 全部选区，主选区在前（与 #46 的 [`Selections`](rove_buffer::edit::Selections) 同序）。
    pub fn selections(&self) -> Vec<Selection> {
        let mut out = Vec::with_capacity(self.secondary.len() + 1);
        out.push(self.cursor.selection);
        out.extend(self.secondary.iter().map(|c| c.selection));
        out
    }

    /// 有次光标。
    pub fn has_multiple_selections(&self) -> bool {
        !self.secondary.is_empty()
    }

    /// 设置全部选区（第一个是主选区），按 `editor.multiCursorLimit` 截断、按
    /// `editor.multiCursorMergeOverlapping` 合并，坐标钳制到合法范围。
    pub fn set_selections(
        &mut self,
        selections: Vec<Selection>,
        reveal: Option<Reveal>,
        cx: &mut Context<Self>,
    ) {
        let clamped: Vec<Selection> = selections
            .into_iter()
            .map(|s| self.clamp_selection(s))
            .collect();
        let merged = self.normalize_selections(clamped);
        let Some(primary) = merged.first().copied() else {
            return;
        };
        self.unfold_to_reveal(primary.active.line.0, cx);
        self.apply_selections(merged);
        if let Some(r) = reveal {
            self.reveal_position(primary.active, r, cx);
        }
        self.selection_changed(SelectionSource::Api, cx);
    }

    /// 不做合并直接写入（列选择：每行一个选区，本来就互不重叠）。
    pub(crate) fn apply_selections(&mut self, selections: Vec<Selection>) {
        let limit = self.options.multi_cursor_limit.max(1);
        let mut items = selections;
        items.truncate(limit);
        let Some(primary) = items.first().copied() else {
            return;
        };
        self.cursor.set(primary);
        self.secondary = items[1..].iter().map(|s| Cursor::new(*s)).collect();
    }

    /// 走 #46 的 [`Selections::normalize`](rove_buffer::edit::Selections::normalize)：截断与合并
    /// 的语义只有一份。文本还没准备好时只截断。
    pub(crate) fn normalize_selections(&self, selections: Vec<Selection>) -> Vec<Selection> {
        let limit = self.options.multi_cursor_limit.max(1);
        let Some(text) = self.text.clone() else {
            let mut items = selections;
            items.truncate(limit);
            return items;
        };
        let core: Vec<ed::Selection> = selections
            .iter()
            .filter_map(|s| {
                Some(ed::Selection::new(
                    text.offset(s.anchor).ok()?,
                    text.offset(s.active).ok()?,
                ))
            })
            .collect();
        let Ok(mut core) = ed::Selections::new(core) else {
            let mut items = selections;
            items.truncate(limit);
            return items;
        };
        core.normalize(self.options.multi_cursor_merge_overlapping, limit);
        core.all()
            .iter()
            .filter_map(|s| {
                Some(Selection::new(
                    text.position(s.anchor).ok()?,
                    text.position(s.active).ok()?,
                ))
            })
            .collect()
    }

    /// 去掉全部次光标；没有次光标时返回 `false`（`escape` 继续交给别人处理）。
    pub(crate) fn remove_secondary_cursors(&mut self, cx: &mut Context<Self>) -> bool {
        if self.secondary.is_empty() {
            return false;
        }
        self.secondary.clear();
        self.column_select = None;
        self.match_session = None;
        self.selection_changed(SelectionSource::Keyboard, cx);
        true
    }

    /// 查找条把自己的当前选项交给代码视图（`Ctrl+D` 是否区分大小写等取它的状态）；
    /// 查找条关闭时传 `None`。
    pub fn set_find_match_options(&mut self, options: Option<FindOptions>) {
        self.find_match_options = options;
    }

    // ------------------------------------------------------------------ 鼠标手势

    /// `Alt+点击`（或 `editor.multiCursorModifier = ctrlCmd` 时的 `Ctrl+点击`）加光标；
    /// 点在已有光标上时删掉它（VS Code 行为），只剩一个时不删。
    pub(crate) fn add_cursor_at(&mut self, p: Position, cx: &mut Context<Self>) {
        let p = self.clamp_position(p);
        let mut items = self.selections();
        let existing = items
            .iter()
            .position(|s| s.is_empty() && s.active == p)
            .filter(|_| items.len() > 1);
        if let Some(i) = existing {
            items.remove(i);
            self.apply_selections(items);
        } else {
            items.insert(0, Selection::caret(p));
            let merged = self.normalize_selections(items);
            self.apply_selections(merged);
        }
        self.column_select = None;
        self.match_session = None;
        self.selection_changed(SelectionSource::Mouse, cx);
    }

    /// `Shift+Alt+拖动`：从 `anchor` 到 `active` 的列选择。
    pub(crate) fn column_select_to(
        &mut self,
        anchor: Position,
        active: Position,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.text.clone() else {
            return;
        };
        let lines = TextLines(&*text);
        let tab = self.options.tab_size;
        let limit = self.options.stop_rendering_line_after;
        let state = ColumnSelect::between(&lines, anchor, active, tab, limit);
        let selections = state.selections(&lines, tab, limit);
        self.column_select = Some(state);
        let clamped: Vec<Selection> = selections
            .into_iter()
            .map(|s| self.clamp_selection(s))
            .collect();
        self.apply_selections(clamped);
        self.reveal_position(active, Reveal::Nearest, cx);
        self.selection_changed(SelectionSource::Mouse, cx);
    }

    // ------------------------------------------------------------------ 命令

    /// 多光标 / 列选择 / 行操作命令。不是本模块的命令时返回 `None`。
    pub(crate) fn execute_multicursor(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        if let Some(op) = line_op(id) {
            self.run_line_op(op, window, cx);
            return Some(true);
        }
        if let Some(m) = column_move(id) {
            self.column_select_by(m, cx);
            return Some(true);
        }
        match id {
            "editor.action.insertCursorAbove" | "editor.action.insertCursorBelow" => {
                self.insert_cursor(id.ends_with("Below"), cx);
            }
            "editor.action.insertCursorAtEndOfEachLineSelected" => {
                self.insert_cursor_at_end_of_lines(cx);
            }
            "removeSecondaryCursors" => return Some(self.remove_secondary_cursors(cx)),
            "editor.action.addSelectionToNextFindMatch" => {
                self.add_selection_to_match(true, false, cx)
            }
            "editor.action.addSelectionToPreviousFindMatch" => {
                self.add_selection_to_match(false, false, cx)
            }
            "editor.action.moveSelectionToNextFindMatch" => {
                self.add_selection_to_match(true, true, cx)
            }
            "editor.action.moveSelectionToPreviousFindMatch" => {
                self.add_selection_to_match(false, true, cx)
            }
            "editor.action.selectHighlights" | "editor.action.changeAll" => {
                self.select_all_occurrences(cx);
            }
            _ => return None,
        }
        Some(true)
    }

    /// `ctrl+alt+up` / `ctrl+alt+down`：每个光标各复制一个到上 / 下一行。
    fn insert_cursor(&mut self, below: bool, cx: &mut Context<Self>) {
        let Some(text) = self.text.clone() else {
            return;
        };
        let lines = TextLines(&*text);
        let tab = self.options.tab_size;
        let mut items = Vec::with_capacity(self.secondary.len() * 2 + 2);
        let mut added = None;
        for s in self.selections() {
            let mut c = Cursor::new(s);
            c.move_caret(
                &lines,
                if below { Move::Down } else { Move::Up },
                false,
                tab,
                1,
            );
            // VS Code `CursorMoveCommands.addCursorUp/Down`：先保留原光标，再加新的，
            // 所以主选区不变。
            items.push(s);
            if c.selection.active != s.active {
                added.get_or_insert(c.selection.active);
                items.push(c.selection);
            }
        }
        let merged = self.normalize_selections(items);
        self.apply_selections(merged);
        self.column_select = None;
        // 滚动到新加的那个光标（原光标本来就在视口里）。
        match added {
            Some(p) => self.reveal_position(p, Reveal::Nearest, cx),
            None => self.reveal_caret_multi(cx),
        }
        self.selection_changed(SelectionSource::Keyboard, cx);
    }

    /// `shift+alt+i`：在选区覆盖的每一行行尾放一个光标。
    fn insert_cursor_at_end_of_lines(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.text.clone() else {
            return;
        };
        let lines = TextLines(&*text);
        let mut items = Vec::new();
        for s in self.selections() {
            if s.is_empty() {
                items.push(s);
                continue;
            }
            let (start, end) = (s.start(), s.end());
            for line in start.line.0..end.line.0 {
                let len = lines.line_text(line).map_or(0, |t| t.len() as u64);
                items.push(Selection::caret(pos(line, len)));
            }
            if end.column.0 > 0 {
                items.push(Selection::caret(end));
            }
        }
        if items.is_empty() {
            return;
        }
        let merged = self.normalize_selections(items);
        self.apply_selections(merged);
        self.column_select = None;
        self.selection_changed(SelectionSource::Keyboard, cx);
    }

    /// `ctrl+shift+alt+方向键`，以及 `editor.columnSelection` 为真时的 `shift+方向键`。
    pub(crate) fn column_select_by(&mut self, m: Move, cx: &mut Context<Self>) {
        let Some(text) = self.text.clone() else {
            return;
        };
        let lines = TextLines(&*text);
        let tab = self.options.tab_size;
        let limit = self.options.stop_rendering_line_after;
        let last = lines.line_count().saturating_sub(1);
        let page = self.metrics().map_or(1, |m| m.page_lines());
        let state = self
            .column_select
            .unwrap_or_else(|| {
                let s = self.cursor.selection;
                ColumnSelect::between(&lines, s.anchor, s.active, tab, limit)
            })
            .moved(m, page, last);
        let selections = state.selections(&lines, tab, limit);
        if selections.is_empty() {
            return;
        }
        self.column_select = Some(state);
        let clamped: Vec<Selection> = selections
            .into_iter()
            .map(|s| self.clamp_selection(s))
            .collect();
        let active = clamped[0].active;
        self.apply_selections(clamped);
        self.reveal_position(active, Reveal::Nearest, cx);
        self.selection_changed(SelectionSource::Keyboard, cx);
    }

    fn reveal_caret_multi(&mut self, cx: &mut Context<Self>) {
        let p = self.cursor.selection.active;
        self.reveal_position(p, Reveal::Nearest, cx);
    }

    // ------------------------------------------------------------------ 按匹配添加选区

    /// 当前查找选项：查找条可见时取它的状态，否则按 VS Code 的 `Ctrl+D`（区分大小写）。
    fn match_options(&self, whole_word: bool) -> FindOptions {
        match self.find_match_options {
            Some(o) => FindOptions {
                case_sensitive: o.case_sensitive,
                whole_word,
                regex: false,
            },
            None => FindOptions {
                case_sensitive: true,
                whole_word,
                regex: false,
            },
        }
    }

    fn searcher_for(&self, text: &str, whole_word: bool) -> Option<Searcher> {
        Searcher::compile(&Query {
            text: text.to_string(),
            options: self.match_options(whole_word),
            word_separators: self.options.word_separators.clone(),
        })
        .ok()
        .flatten()
    }

    /// 主选区的文本；空选区时为 `None`。多行选区按 VS Code 的做法不开会话。
    fn primary_match_text(&self) -> Option<String> {
        let s = self.cursor.selection;
        if s.is_empty() || s.start().line != s.end().line {
            return None;
        }
        self.selected_text()
    }

    /// `ctrl+d` / `ctrl+shift+d` 一族。`move_last` 为真时替换最后一个选区而不是新增。
    fn add_selection_to_match(&mut self, forward: bool, move_last: bool, cx: &mut Context<Self>) {
        let Some(text) = self.text.clone() else {
            return;
        };
        // 没有选区：先选中光标处的单词（VS Code 的第一次 Ctrl+D）。
        let Some(search) = self.primary_match_text() else {
            let Some(word) = self.word_selection_at(self.cursor.selection.active) else {
                return;
            };
            self.match_session = Some(MatchSession {
                text: self.text_in(word).unwrap_or_default(),
                whole_word: true,
            });
            let mut items = self.selections();
            items[0] = word;
            self.apply_selections(items);
            self.reveal_caret_multi(cx);
            self.selection_changed(SelectionSource::Keyboard, cx);
            return;
        };
        let whole_word = self
            .match_session
            .as_ref()
            .is_some_and(|s| s.text == search && s.whole_word);
        let Some(searcher) = self.searcher_for(&search, whole_word) else {
            return;
        };
        let from = if forward {
            self.cursor.selection.end()
        } else {
            self.cursor.selection.start()
        };
        let cancel = Cancellation::default();
        let Ok(Some(step)) =
            engine::find_next(&*text, &searcher, None, from, forward, true, &cancel)
        else {
            return;
        };
        let found = Selection::new(step.found.start, step.found.end);
        self.match_session = Some(MatchSession {
            text: search,
            whole_word,
        });
        let mut items = self.selections();
        if move_last {
            items[0] = found;
        } else {
            items.insert(0, found);
        }
        let merged = self.normalize_selections(items);
        self.apply_selections(merged);
        self.column_select = None;
        self.reveal_caret_multi(cx);
        self.selection_changed(SelectionSource::Keyboard, cx);
    }

    /// `ctrl+shift+l` / `ctrl+f2`：选中本文件里所有同名出现。
    fn select_all_occurrences(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.text.clone() else {
            return;
        };
        let (search, whole_word) = match self.primary_match_text() {
            Some(s) => (s, false),
            None => {
                let Some(word) = self.word_selection_at(self.cursor.selection.active) else {
                    return;
                };
                (self.text_in(word).unwrap_or_default(), true)
            }
        };
        let Some(searcher) = self.searcher_for(&search, whole_word) else {
            return;
        };
        let cancel = Cancellation::default();
        let Ok(out) =
            engine::search_all(&*text, &searcher, None, MATCHES_LIMIT, &cancel, &mut |_, _| {})
        else {
            return;
        };
        if out.matches.is_empty() {
            return;
        }
        let caret = self.cursor.selection.active;
        let mut items: Vec<Selection> = out
            .matches
            .iter()
            .map(|m| Selection::new(m.start, m.end))
            .collect();
        // 光标所在的那一处作为主选区。
        let at = items
            .iter()
            .position(|s| s.start() <= caret && caret <= s.end());
        if let Some(i) = at {
            items.swap(0, i);
        }
        self.match_session = Some(MatchSession {
            text: search,
            whole_word,
        });
        let merged = self.normalize_selections(items);
        self.apply_selections(merged);
        self.column_select = None;
        self.reveal_caret_multi(cx);
        self.selection_changed(SelectionSource::Keyboard, cx);
    }

    /// 光标处的单词（按 `editor.wordSeparators`，与双击选词同一套规则）。
    pub(crate) fn word_selection_at(&self, p: Position) -> Option<Selection> {
        let text = self.text.as_deref()?;
        let line = super::columns::read_line(text, p.line.0)?;
        let (a, b) = word_range_at(&line, p.column.0 as usize, &self.options.word_separators);
        (a != b).then(|| Selection::new(pos(p.line.0, a as u64), pos(p.line.0, b as u64)))
    }

    fn text_in(&self, s: Selection) -> Option<String> {
        let text = self.text.as_deref()?;
        let start = text.offset(s.start()).ok()?;
        let end = text.offset(s.end()).ok()?;
        let mut out = String::new();
        text.visit_chunks(rove_base::Span::new(start, end).ok()?, &mut |c| {
            out.push_str(c)
        })
        .ok()?;
        Some(out)
    }

    // ------------------------------------------------------------------ 行操作

    /// 一条行操作：所有光标合成**一个**事务、一次撤销。
    pub(crate) fn run_line_op(&mut self, op: LineOp, window: &mut Window, cx: &mut Context<Self>) {
        let selections = self.selections();
        let ctx = lineops::LineOpContext {
            word_separators: self.options.word_separators.to_string(),
            eol: self.document_eol(),
        };
        self.run_edit(Some(window), cx, move |session, snapshot, now| {
            let Some(plan) = lineops::plan(op, &**snapshot, &selections, &ctx)? else {
                return Ok(Default::default());
            };
            let applied = session.edit(plan.transaction, now)?;
            if let Some(after) = plan.selections {
                let snapshot = session.snapshot();
                let mapped: Vec<ed::Selection> = after
                    .iter()
                    .filter_map(|s| {
                        let a = snapshot.offset(lineops::clamp(&*snapshot, s.anchor)).ok()?;
                        let b = snapshot.offset(lineops::clamp(&*snapshot, s.active)).ok()?;
                        Some(ed::Selection::new(a, b))
                    })
                    .collect();
                if let Ok(selections) = ed::Selections::new(mapped) {
                    session.set_selections(selections);
                }
            }
            Ok(super::edit::EditOutcome {
                edits: vec![applied],
                selection: None,
            })
        });
        self.column_select = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_view::selection::tests::lines;

    #[test]
    fn modifier_swaps_add_cursor_and_go_to_definition() {
        let alt = MultiCursorModifier::Alt;
        assert!(alt.adds_cursor(false, true) && !alt.adds_cursor(true, false));
        assert!(alt.goes_to_definition(true, false) && !alt.goes_to_definition(false, true));
        let ctrl = MultiCursorModifier::CtrlCmd;
        assert!(ctrl.adds_cursor(true, false) && !ctrl.adds_cursor(false, true));
        assert!(ctrl.goes_to_definition(false, true) && !ctrl.goes_to_definition(true, false));
        assert_eq!(MultiCursorModifier::parse("ctrlCmd"), ctrl);
        assert_eq!(MultiCursorModifier::parse("alt"), alt);
        assert_eq!(MultiCursorModifier::parse("什么都不是"), alt);
    }

    #[test]
    fn column_select_aligns_on_tabs_and_full_width_characters() {
        // 可见列：第 0 行 "\tab" → Tab 到第 4 列；第 1 行 "中文x"：中=0..2、文=2..4。
        let t = lines(&["\tab", "中文x", "y"]);
        let state = ColumnSelect {
            from_line: 0,
            from_visual: 4,
            to_line: 1,
            to_visual: 6,
        };
        let sels = state.selections(&t, 4, None);
        assert_eq!(sels.len(), 2);
        // 主选区是终点所在的行。
        assert_eq!(sels[0].start(), pos(1, 6)); // “中文”各 3 字节：可见列 4 → 字节 6
        assert_eq!(sels[0].end(), pos(1, 7));
        assert_eq!(sels[1], Selection::new(pos(0, 1), pos(0, 3)));
    }

    #[test]
    fn column_select_skips_lines_that_are_too_short() {
        let t = lines(&["aaaaaa", "bb", "cccccc"]);
        let state = ColumnSelect {
            from_line: 0,
            from_visual: 4,
            to_line: 2,
            to_visual: 6,
        };
        let sels = state.selections(&t, 4, None);
        // 第 1 行只有 2 列，整段都在 from_visual 之前：跳过。
        assert_eq!(sels.len(), 2);
        assert!(sels.iter().all(|s| s.active.line.0 != 1));
    }

    #[test]
    fn column_select_moves_by_command() {
        let t = lines(&["abcdef", "abcdef", "abcdef"]);
        let start = ColumnSelect {
            from_line: 0,
            from_visual: 1,
            to_line: 0,
            to_visual: 1,
        };
        let right = start.moved(Move::Right, 10, 2);
        assert_eq!(right.to_visual, 2);
        let down = right.moved(Move::Down, 10, 2);
        assert_eq!(down.to_line, 1);
        let sels = down.selections(&t, 4, None);
        assert_eq!(sels.len(), 2);
        assert!(sels
            .iter()
            .all(|s| s.start() == pos(s.start().line.0, 1) && s.end() == pos(s.end().line.0, 2)));
        // 末行之后不再下移。
        assert_eq!(down.moved(Move::Down, 10, 1).to_line, 1);
        assert_eq!(start.moved(Move::Left, 10, 2).to_visual, 0);
    }

    #[test]
    fn every_command_maps_to_an_operation() {
        for (id, _, _) in COMMANDS {
            let known = line_op(id).is_some()
                || column_move(id).is_some()
                || id.contains("Cursor")
                || id.contains("FindMatch")
                || id.contains("selectHighlights")
                || id.contains("changeAll");
            assert!(known, "{id} 没有接到任何实现");
        }
    }
}
