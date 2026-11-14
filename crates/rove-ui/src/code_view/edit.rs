//! 代码视图的编辑交互与输入法（#47）。
//!
//! - **编辑入口**：视图本身不持有文本模型。宿主用 [`CodeView::set_edit_host`] 交给它一个
//!   [`EditHost`]（借出 #46 的 [`EditSession`]）；没有宿主（超大文件、元数据视图、尚在建立）时
//!   视图只读。每条编辑命令先把视图的选区写进会话、执行会话的事务，再用新快照增量更新本视图
//!   （行缓存、装饰按行平移，只重建被改的行），并发出 [`CodeViewEvent::Edited`]：宿主据此同步
//!   同一文档的其他视图（[`CodeView::apply_edits`]）、#94 的增量解析、#38 的 `BufferStates` 与
//!   标签脏标记。
//! - **输入法**：实现 GPUI 的 [`EntityInputHandler`]。组合串只在视图里内嵌渲染（[`Composition`]，
//!   带 `editor.compositionBorder` 下划线），**不写进缓冲区**：组合期间没有事务、不进撤销栈、不触发
//!   重解析；提交时作为一个独立的撤销单元写入。偏移量是 UTF-16，坐标系是“选区（或组合串）所在的
//!   那几行”，换算见 [`ImeRegion`]。点击别处、失焦时先在原位置提交组合串，再通知输入法取消它自己的
//!   组合状态（Windows `ImmNotifyIME(CPS_CANCEL)`），不留残片、不重复提交。
//! - **只读**：只读文档上的编辑命令发出 [`CodeViewEvent::EditBlocked`]（#20 的光标处提示），
//!   输入法不进入组合态。单词导航与 `cursorLineStart/End` 不要求可编辑。

use super::autoedit;
use super::columns::{grapheme_boundaries, read_line, DisplayLine, TextLines};
use super::scroll::Reveal;
use super::selection::{pos, Selection};
use super::view::{CodeView, CodeViewEvent, LineEntry, SelectionSource};
use gpui::{
    point, px, size, Bounds, ClipboardItem, Context, EntityInputHandler, Pixels, UTF16Selection,
    Window,
};
use rove_base::{ByteOffset, Error, Position, Span, TextEdit};
use rove_buffer::edit::{
    self as ed, AppliedEdit, ClipboardData, CursorEdit, DeleteWord, EditKind, EditSession,
    LineChanges, MoveWord, Replacement, Selections, Transaction, WordClassifier,
};
use rove_buffer::TextSource;
use std::cell::{RefCell, RefMut};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

/// 剪贴板元数据：无选区复制整行（`editor.emptySelectionClipboard`），粘贴时插到当前行之上。
pub const CLIPBOARD_WHOLE_LINE: &str = "rove.wholeLine";

/// 宿主借出编辑会话（窗口拥有 #49 的 `TextFile`，同一文档的所有视图共用一个会话）。
pub trait EditHost {
    /// 借出会话；文档当前不能编辑（已关闭、正在建立）时为 `None`。借用只在一条命令内持有。
    fn session(&self) -> Option<RefMut<'_, EditSession>>;
}

impl EditHost for RefCell<EditSession> {
    fn session(&self) -> Option<RefMut<'_, EditSession>> {
        self.try_borrow_mut().ok()
    }
}

/// 一次编辑（含撤销 / 重做）的结果。
pub struct TextEdited {
    /// 编辑前视图显示的快照。
    pub before: Arc<dyn TextSource>,
    /// 编辑后的快照（与会话的快照相同）。
    pub after: Arc<dyn TextSource>,
    /// 按应用顺序的事务结果（撤销一个单元可能有多步）。
    pub edits: Arc<[AppliedEdit]>,
    /// 合成一个替换（相对 `before`）：交给 #94 的增量解析与 #38 的 `BufferStates::edited`。
    pub combined: Option<TextEdit>,
}

/// [`CodeViewEvent::Edited`] 的载荷（按指针比较）。
#[derive(Clone)]
pub struct EditNotice(pub Arc<TextEdited>);

impl PartialEq for EditNotice {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for EditNotice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditNotice")
            .field("before", &self.0.before.version())
            .field("after", &self.0.after.version())
            .field("combined", &self.0.combined)
            .finish()
    }
}

/// 输入法组合串：只在视图里显示，不在缓冲区里。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composition {
    /// 组合串的插入点（组合开始时选区的起点）。
    pub start: Position,
    /// 提交时被替换区间的终点（组合开始时选区的终点）。
    pub end: Position,
    pub text: String,
    /// 组合串内的光标（UTF-8 字节，`GCS_CURSORPOS`）。
    pub cursor: usize,
}

/// 一条编辑命令的结果：事务与（可选的）显式选区。
#[derive(Default)]
pub(crate) struct EditOutcome {
    pub edits: Vec<AppliedEdit>,
    pub selection: Option<Selection>,
}

impl From<Option<AppliedEdit>> for EditOutcome {
    fn from(e: Option<AppliedEdit>) -> Self {
        Self {
            edits: e.into_iter().collect(),
            selection: None,
        }
    }
}

// ------------------------------------------------------------------ 行号映射（编辑后的缓存平移）

/// 旧行号 → 新行号；落在被修改的行里时为 `None`。
pub fn old_to_new(changes: &LineChanges, line: u64) -> Option<u64> {
    let mut delta: i64 = 0;
    for c in &changes.ranges {
        if line < c.old.start().0 {
            break;
        }
        if line < c.old.end().0 {
            return None;
        }
        delta = c.new.end().0 as i64 - c.old.end().0 as i64;
    }
    Some((line as i64 + delta).max(0) as u64)
}

/// 新行号 → 旧行号（被修改的行映射到旧区间里同一偏移处，越界取旧区间末行）。
pub fn new_to_old(changes: &LineChanges, line: u64) -> u64 {
    let mut delta: i64 = 0;
    for c in &changes.ranges {
        if line < c.new.start().0 {
            break;
        }
        if line < c.new.end().0 {
            let offset = line - c.new.start().0;
            let last = c.old.end().0.saturating_sub(1).max(c.old.start().0);
            return (c.old.start().0 + offset).min(last);
        }
        delta = c.old.end().0 as i64 - c.new.end().0 as i64;
    }
    (line as i64 + delta).max(0) as u64
}

/// 把依次应用的若干事务合成一个相对第一个快照的替换（区间覆盖所有改动，中间未改的文本也算在内）。
pub fn combine_edits(edits: &[AppliedEdit], after: &dyn TextSource) -> Option<TextEdit> {
    let spans = edits.iter().flat_map(|e| {
        e.input_edits.iter().map(|ie| {
            (
                ie.start_byte.0 as i64,
                ie.old_end_byte.0 as i64,
                ie.new_end_byte.0 as i64,
            )
        })
    });
    combine_spans(spans, edits.first()?.before, after)
}

/// 依次发生的替换（各自坐标系里的 `(起点, 旧终点, 新终点)`）合成一个相对 `version` 的替换。
pub fn combine_spans(
    spans: impl IntoIterator<Item = (i64, i64, i64)>,
    version: rove_base::Version,
    after: &dyn TextSource,
) -> Option<TextEdit> {
    // (起点, 旧终点, 当前终点)：[起点, 当前终点) 在当前坐标里覆盖全部改动，[起点, 旧终点) 在最初坐标里。
    let mut acc: Option<(i64, i64, i64)> = None;
    for (a, b, l) in spans {
        acc = Some(match acc {
            None => (a, b, l),
            Some((s, oe, ne)) => {
                let start = s.min(a);
                let old_end = if b > ne { b + oe - ne } else { oe };
                let new_end = ne.max(b) + (l - b);
                (start, old_end, new_end)
            }
        });
    }
    let (s, oe, ne) = acc?;
    let span = Span::new(ByteOffset(s as u64), ByteOffset(ne as u64)).ok()?;
    let mut inserted = String::with_capacity((ne - s).max(0) as usize);
    after
        .visit_chunks(span, &mut |c| inserted.push_str(c))
        .ok()?;
    Some(TextEdit {
        version,
        range: Span::new(ByteOffset(s as u64), ByteOffset(oe as u64)).ok()?,
        inserted: inserted.into(),
    })
}

/// #52：对会话里的**每个**选区各算一次替换，合成一个事务（一次撤销）。
fn for_each_selection(
    session: &EditSession,
    text: &dyn TextSource,
    mut f: impl FnMut(Selection) -> rove_base::Result<Option<CursorEdit>>,
) -> rove_base::Result<Vec<Option<CursorEdit>>> {
    let mut out = Vec::with_capacity(session.selections().len());
    for s in session.selections().all() {
        out.push(f(Selection::new(
            text.position(s.anchor)?,
            text.position(s.active)?,
        ))?);
    }
    Ok(out)
}

/// #52：事务之后把光标显式放到一组偏移上（`lineBreakInsert`、`insertLine*`）。
fn place_cursors(session: &mut EditSession, offsets: Vec<ByteOffset>) {
    let placed: Vec<ed::Selection> = offsets.into_iter().map(ed::Selection::cursor).collect();
    if let Ok(selections) = Selections::new(placed) {
        session.set_selections(selections);
    }
}

/// #52：把会话读回来的选区列表与命令显式指定的主选区合并成视图的选区列表。
fn selections_with_primary(all: &[Selection], primary: Option<Selection>) -> Vec<Selection> {
    let mut out = all.to_vec();
    match (primary, out.first_mut()) {
        (Some(p), Some(first)) => *first = p,
        (Some(p), None) => out.push(p),
        _ => {}
    }
    out
}

fn to_core(text: &dyn TextSource, s: Selection) -> Option<ed::Selection> {
    Some(ed::Selection::new(
        text.offset(s.anchor).ok()?,
        text.offset(s.active).ok()?,
    ))
}

fn from_core(text: &dyn TextSource, s: &ed::Selection) -> Option<Selection> {
    Some(Selection::new(
        text.position(s.anchor).ok()?,
        text.position(s.active).ok()?,
    ))
}

fn line_text(text: &dyn TextSource, line: u64) -> String {
    read_line(text, line).unwrap_or_default()
}

/// 两个位置之间的文本（#53 的自动包围判定用）。
fn range_text(text: &dyn TextSource, from: Position, to: Position) -> String {
    let (Ok(a), Ok(b)) = (text.offset(from), text.offset(to)) else {
        return String::new();
    };
    let Ok(span) = Span::new(a, b) else {
        return String::new();
    };
    let mut out = String::new();
    let _ = text.visit_chunks(span, &mut |s| out.push_str(s));
    out
}

/// #53：抽象改动（行 + 字节列）换算成事务的替换项。
fn to_replacements(
    text: &dyn TextSource,
    changes: &[autoedit::Change],
) -> rove_base::Result<Vec<Replacement>> {
    changes
        .iter()
        .map(|c| {
            let start = text.offset(pos(c.start.0, c.start.1 as u64))?;
            let end = text.offset(pos(c.end.0, c.end.1 as u64))?;
            Ok(Replacement::new(Span::new(start, end)?, c.text.as_str()))
        })
        .collect()
}

fn leading_whitespace(line: &str) -> &str {
    let n = line.len() - line.trim_start_matches([' ', '\t']).len();
    &line[..n]
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// UTF-16 偏移 → UTF-8 字节（落在代理对中间时取后一个字符边界，超出取末尾）。
fn utf16_to_byte(s: &str, target: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in s.char_indices() {
        if units >= target {
            return byte;
        }
        units += ch.len_utf16();
    }
    s.len()
}

/// 输入法看到的“文档”：从首行行首到末行行尾（含组合串）；偏移是 UTF-16。
pub(crate) struct ImeRegion {
    /// 首行（文档行号）。
    pub first: u64,
    pub text: String,
    /// 每行在 `text` 里的起始字节（下标对应 `first + i`）。
    pub line_starts: Vec<usize>,
    /// 组合串在 `text` 里的字节区间。
    pub marked: Option<Range<usize>>,
}

impl ImeRegion {
    fn byte_of(&self, p: Position) -> Option<usize> {
        let i = p.line.0.checked_sub(self.first)? as usize;
        Some(self.line_starts.get(i)? + p.column.0 as usize)
    }
    fn utf16_of_byte(&self, byte: usize) -> usize {
        utf16_len(&self.text[..byte.min(self.text.len())])
    }
    fn byte_of_utf16(&self, units: usize) -> usize {
        utf16_to_byte(&self.text, units)
    }
    /// 区域内字节 → 文档位置（组合串内的字节映射到组合串起点）。
    fn position_of(&self, byte: usize, composition: Option<&Composition>) -> Position {
        if let (Some(m), Some(c)) = (&self.marked, composition) {
            if byte >= m.start && byte <= m.end {
                return c.start;
            }
        }
        let i = self.line_starts.partition_point(|s| *s <= byte).max(1) - 1;
        let mut col = byte - self.line_starts[i];
        if let (Some(m), Some(c)) = (&self.marked, composition) {
            // 组合行上组合串之后的部分：减去组合串长度，加回被替换的部分。
            if self.first + i as u64 == c.start.line.0 && byte > m.end {
                col = col - (m.end - m.start) + (c.end.column.0 - c.start.column.0) as usize;
                return pos(c.end.line.0, col as u64);
            }
        }
        pos(self.first + i as u64, col as u64)
    }
}

#[cfg(windows)]
fn cancel_platform_composition() {
    use windows_sys::Win32::UI::Input::Ime::{
        ImmGetContext, ImmNotifyIME, ImmReleaseContext, CPS_CANCEL, NI_COMPOSITIONSTR,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    // SAFETY：只在 UI 线程调用；句柄为空时直接返回，取到的输入法上下文总是释放。
    unsafe {
        let hwnd = GetFocus();
        if hwnd.is_null() {
            return;
        }
        let himc = ImmGetContext(hwnd);
        if himc.is_null() {
            return;
        }
        ImmNotifyIME(himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0);
        ImmReleaseContext(hwnd, himc);
    }
}

#[cfg(not(windows))]
fn cancel_platform_composition() {}

impl CodeView {
    // ------------------------------------------------------------------ 宿主接口

    /// 设置（或撤掉）编辑入口。撤掉时视图回到只读阅读，未提交的组合串丢弃。
    pub fn set_edit_host(&mut self, host: Option<Rc<dyn EditHost>>, cx: &mut Context<Self>) {
        self.edit_host = host;
        self.composition = None;
        self.composition_entry = None;
        cx.notify();
    }

    pub fn edit_host(&self) -> Option<&Rc<dyn EditHost>> {
        self.edit_host.as_ref()
    }

    /// 可以编辑：有编辑入口且没有只读原因。
    pub fn is_editable(&self) -> bool {
        self.edit_host.is_some() && !self.is_read_only()
    }

    /// 当前的输入法组合串。
    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }

    /// 改写模式（`editor.action.toggleOvertypeInsertMode`）。
    pub fn overtype(&self) -> bool {
        self.overtype
    }

    /// 选择定位点（`editor.action.setSelectionAnchor`）。
    pub fn selection_anchor(&self) -> Option<Position> {
        self.selection_anchor
    }

    /// 同一文档在别的视图里被编辑了（或宿主做了编辑）：换成新快照，行缓存与装饰按行平移，
    /// 本视图的选区与组合串按事务映射到新文本，滚动位置不变。
    pub fn apply_edits(
        &mut self,
        after: Arc<dyn TextSource>,
        edits: &[AppliedEdit],
        cx: &mut Context<Self>,
    ) {
        let Some(before) = self.text.clone() else {
            self.set_text(Some(after), true, cx);
            return;
        };
        if edits.first().is_some_and(|e| e.before != before.version()) {
            // 本视图的快照不是事务的起点（漏掉了中间的版本）：整体换文本，保持视图状态。
            self.set_text(Some(after), true, cx);
            return;
        }
        let map = |p: Position| -> Option<Position> {
            let mut o = before.offset(p).ok()?;
            for e in edits {
                o = e.map_offset(o, ed::Bias::After);
            }
            after.position(o).ok()
        };
        let s = self.cursor.selection;
        let mapped = map(s.anchor).zip(map(s.active));
        if let Some(c) = self.composition.as_mut() {
            match map(c.start).zip(map(c.end)) {
                Some((a, b)) => {
                    c.start = a;
                    c.end = b;
                }
                None => self.composition = None,
            }
        }
        self.composition_entry = None;
        self.apply_text(after, edits, cx);
        if let Some((a, b)) = mapped {
            self.cursor.set(Selection::new(a, b));
        }
        cx.notify();
    }

    /// 换上编辑后的快照：行缓存、装饰、语法旧结果按行平移，只有被改的行重建。
    fn apply_text(
        &mut self,
        after: Arc<dyn TextSource>,
        edits: &[AppliedEdit],
        cx: &mut Context<Self>,
    ) {
        self.text = Some(after);
        // #53：任何编辑都让“这对括号是编辑器补的”失效（自动闭合自己在编辑之后重新记）。
        self.auto_closed.clear();
        let mut lines_moved = false;
        let first = self.doc_line(self.visible.start);
        let keep = first.saturating_sub(2048)..first + 4096;
        for e in edits {
            let changes = &e.changes;
            lines_moved |= changes.line_delta != 0;
            if changes.line_delta == 0 {
                for c in &changes.ranges {
                    for l in c.old.start().0..c.old.end().0 {
                        self.lines.remove(&l);
                    }
                }
            } else {
                self.lines
                    .rekey(|l| old_to_new(changes, *l).filter(|_| keep.contains(l)));
            }
            self.decorations.remap(|l| old_to_new(changes, l));
            self.syntax.borrow_mut().pending.push(changes.clone());
        }
        if lines_moved {
            self.sync_fold_map();
            self.recompute_folds(cx);
        }
        self.update_highlights(cx);
        // 查找条重新搜索、#40 的同名高亮按新文本重查（它们按快照工作）。
        cx.emit(CodeViewEvent::TextChanged);
        cx.notify();
    }

    /// 当前行（被改的可见行）立即重建并整形，下一帧不会出现占位。
    fn rebuild_lines_now(&mut self, edits: &[AppliedEdit], window: &mut Window) {
        let visible = self.doc_line(self.visible.start)
            ..self.doc_line(self.visible.end.saturating_sub(1)) + 1;
        let mut lines: Vec<u64> = edits
            .last()
            .map(|e| {
                e.changes
                    .ranges
                    .iter()
                    .flat_map(|c| c.new.start().0..c.new.end().0)
                    .collect()
            })
            .unwrap_or_default();
        lines.push(self.cursor.selection.active.line.0);
        for line in lines {
            if visible.contains(&line) || line == self.cursor.selection.active.line.0 {
                self.ensure_line_now(line, window);
            }
        }
    }

    // ------------------------------------------------------------------ 编辑执行

    /// 只读时在光标处提示原因（#20 的光标处提示）。
    pub(crate) fn edit_blocked(&mut self, cx: &mut Context<Self>) {
        cx.emit(CodeViewEvent::EditBlocked {
            position: self.cursor.selection.active,
            reasons: self.read_only.clone(),
        });
    }

    /// 执行一条编辑：把视图选区写进会话 → `f` → 增量更新视图并发出 [`CodeViewEvent::Edited`]。
    /// 只读时提示原因并返回 `false`；没有编辑入口时什么也不做。
    pub(crate) fn run_edit(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
        f: impl FnOnce(
            &mut EditSession,
            &Arc<rove_buffer::RopeText>,
            Instant,
        ) -> rove_base::Result<EditOutcome>,
    ) -> bool {
        if self.is_read_only() {
            self.edit_blocked(cx);
            return false;
        }
        let Some(host) = self.edit_host.clone() else {
            return false;
        };
        // 视图显示的快照必须就是会话的快照（宿主刚换上会话时可能还不一样）。
        let snapshot = match host.session() {
            Some(s) => s.snapshot(),
            None => return false,
        };
        if self.text.as_ref().map(|t| t.version()) != Some(snapshot.version()) {
            self.set_text(Some(snapshot.clone() as Arc<dyn TextSource>), true, cx);
        }
        let before: Arc<dyn TextSource> = snapshot.clone();
        let view_selections = self.selections();
        let result = {
            let Some(mut session) = host.session() else {
                return false;
            };
            // #52：主选区在前，次光标跟在后面；整个命令对所有光标生效。
            let core: Vec<ed::Selection> = view_selections
                .iter()
                .filter_map(|s| to_core(&*snapshot, *s))
                .collect();
            if let Ok(selections) = Selections::new(core) {
                session.set_selections(selections);
            }
            match f(&mut session, &snapshot, Instant::now()) {
                Ok(outcome) => {
                    let after = session.snapshot();
                    let all: Vec<Selection> = session
                        .selections()
                        .all()
                        .iter()
                        .filter_map(|s| from_core(&*after, s))
                        .collect();
                    Ok((outcome, after, all))
                }
                Err(e) => Err(e),
            }
        };
        let (outcome, after, all) = match result {
            Ok(r) => r,
            Err(Error::Unsupported) => {
                self.edit_blocked(cx);
                return false;
            }
            Err(_) => return false,
        };
        let primary = all.first().copied();
        let selection = outcome.selection.or(primary);
        if outcome.edits.is_empty() {
            let changed = selection.is_some_and(|s| s != self.cursor.selection)
                || all.len() != view_selections.len();
            if changed {
                self.apply_selections(selections_with_primary(&all, selection));
                self.reveal_caret_nearest(cx);
                self.selection_changed(SelectionSource::Keyboard, cx);
            }
            return false;
        }
        let after: Arc<dyn TextSource> = after;
        self.apply_text(after.clone(), &outcome.edits, cx);
        let mut placed = selections_with_primary(&all, selection);
        for s in &mut placed {
            *s = self.clamp_selection(*s);
        }
        if !placed.is_empty() {
            self.apply_selections(placed);
        }
        if let Some(window) = window {
            self.rebuild_lines_now(&outcome.edits, window);
            window.invalidate_character_coordinates();
        }
        let combined = combine_edits(&outcome.edits, &*after);
        let notice = EditNotice(Arc::new(TextEdited {
            before,
            after,
            edits: outcome.edits.into(),
            combined,
        }));
        cx.emit(CodeViewEvent::Edited(notice));
        self.reveal_caret_nearest(cx);
        self.selection_changed(SelectionSource::Keyboard, cx);
        true
    }

    fn reveal_caret_nearest(&mut self, cx: &mut Context<Self>) {
        let p = self.cursor.selection.active;
        self.reveal_position(p, Reveal::Nearest, cx);
    }

    /// 视图选区变化后同步给会话（光标撤销栈、撤销分组的断开都以会话为准）。
    pub(crate) fn push_selection_to_session(&mut self) {
        let (Some(host), Some(text)) = (self.edit_host.clone(), self.text.clone()) else {
            return;
        };
        let Some(mut session) = host.session() else {
            return;
        };
        if session.snapshot().version() != text.version() {
            return;
        }
        // #52：全部光标（主选区在前）。
        let core: Vec<ed::Selection> = self
            .selections()
            .iter()
            .filter_map(|s| to_core(&*text, *s))
            .collect();
        if let Ok(selections) = Selections::new(core) {
            session.set_selections(selections);
        }
    }

    /// 输入文本（`type` 命令、键盘字符、输入法提交）。改写模式下覆盖其后同样多的字素簇（不越过行尾）。
    pub fn type_text(
        &mut self,
        text: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        if text.is_empty() {
            return false;
        }
        // #53：单个字符先走自动行为（包围 / 越过 / 闭合 / 重新缩进）。改写模式下不自动闭合。
        let mut window = window;
        if !self.overtype
            && text.chars().count() == 1
            && !text.contains(['\n', '\r'])
            && self.auto_type(text, window.as_deref_mut(), cx)
        {
            return true;
        }
        self.type_text_plain(text, window, cx)
    }

    fn type_text_plain(
        &mut self,
        text: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        let overtype = self.overtype && !text.contains(['\n', '\r']);
        let text = text.to_string();
        self.run_edit(window, cx, move |s, snap, now| {
            if overtype {
                extend_for_overtype(s, &**snap, &text)?;
            }
            Ok(s.type_text(&text, now)?.into())
        })
    }

    /// 回车：`onEnterRules`、括号规则与 `indentationRules` 的自动缩进（#53），
    /// 五种 `editor.autoIndent` 取值的差异都在 [`autoedit::on_enter::enter`] 里。
    /// `editor.trimAutoWhitespace` 时只有空白的当前行先清空，自动插入的空白在光标离行时清除。
    pub(crate) fn enter(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) -> bool {
        let trim = self.options.trim_auto_whitespace;
        if !self.secondary.is_empty() {
            // #52：多光标时每个光标各自换行、各自保持本行缩进，合成一个事务；
            // #53 的缩进规则引擎目前只作用于单选区。
            return self.run_edit(window, cx, move |s, snap, now| {
                let text: &dyn TextSource = &**snap;
                // #52：每个光标各自换行、各自保持本行缩进，合成一个事务。
                let edits = for_each_selection(s, text, |sel| {
                    let start = sel.start();
                    let line = line_text(text, start.line.0);
                    let indent =
                        leading_whitespace(&line[..(start.column.0 as usize).min(line.len())]);
                    let only_ws = sel.is_empty()
                        && trim
                        && !line.is_empty()
                        && line.trim().is_empty()
                        && start.column.0 as usize == line.len();
                    let from = if only_ws { pos(start.line.0, 0) } else { start };
                    let range = Span::new(text.offset(from)?, text.offset(sel.end())?)?;
                    Ok(Some(CursorEdit::replace(range, format!("\n{indent}"))))
                })?;
                let kind = EditKind::typing("\n", s.previous_kind());
                Ok(s.edit_each(edits, kind, now)?.into())
            });
        }
        let sel = self.cursor.selection;
        let converter = self.options.indent_converter();
        let auto_indent = if self.auto_edit_allowed() {
            self.options.auto_indent
        } else {
            autoedit::indent::AutoIndent::Keep
        };
        let config = self
            .auto_edit_allowed()
            .then(|| self.language_config())
            .flatten();
        let pending_trim = self.take_trim_line(sel.start().line.0);
        let edited = self.run_edit(window, cx, move |s, snap, now| {
            let text: &dyn TextSource = &**snap;
            let start = sel.start();
            let line = line_text(text, start.line.0);
            // 当前行只剩自动插入的空白：连同它一起换行（VS Code 的 trimAutoWhitespace）。
            let only_ws = sel.is_empty()
                && trim
                && !line.is_empty()
                && line.trim().is_empty()
                && start.column.0 as usize == line.len();
            let lines = TextLines(text);
            let mut auto = autoedit::on_enter::enter(
                auto_indent,
                &lines,
                config,
                (start.line.0, start.column.0 as usize),
                (sel.end().line.0, sel.end().column.0 as usize),
                &converter,
            );
            if only_ws {
                // 当前行只剩自动插入的空白：连同它一起替换掉（新行的缩进照常算）。
                if let Some(first) = auto.changes.first_mut() {
                    if first.start == (start.line.0, start.column.0 as usize) {
                        first.start = (start.line.0, 0);
                    }
                }
            }
            let mut replacements = to_replacements(text, &auto.changes)?;
            if let Some(trim) = &pending_trim {
                replacements.extend(to_replacements(text, std::slice::from_ref(trim))?);
            }
            if replacements.is_empty() {
                return Ok(EditOutcome::default());
            }
            let kind = EditKind::typing("\n", s.previous_kind());
            let applied = if replacements.len() == 1 && pending_trim.is_none() {
                let r = &replacements[0];
                s.edit_each(
                    vec![Some(CursorEdit::replace(
                        r.range.clone(),
                        r.text.to_string(),
                    ))],
                    kind,
                    now,
                )?
                .into_iter()
                .collect::<Vec<_>>()
            } else {
                vec![s.edit(Transaction::new(replacements)?, now)?]
            };
            Ok(EditOutcome {
                edits: applied,
                selection: auto.caret.map(|(l, c)| Selection::caret(pos(l, c as u64))),
            })
        });
        if edited && trim {
            self.remember_auto_whitespace();
        }
        edited
    }

    // ------------------------------------------------------------------ #53：自动行为

    /// #53 的自动行为是否生效：超大文件上一律不触发（它本来也是只读的，这里再挡一道）。
    pub fn auto_edit_allowed(&self) -> bool {
        self.document_level() != rove_buffer::SizeClass::Huge
    }

    /// 本次编辑要顺带清掉的“自动插入的空白行”（光标已经离开那一行）。
    fn take_trim_line(&mut self, editing: u64) -> Option<autoedit::Change> {
        let line = self.auto_whitespace_line.take()?;
        if line == editing {
            return None;
        }
        let text = self.text.as_ref()?;
        let source = line_text(&**text, line);
        if source.is_empty() || !source.trim().is_empty() {
            return None;
        }
        Some(autoedit::Change::delete((line, 0), (line, source.len())))
    }

    /// 编辑后：当前行只剩空白时记下来，光标离开时清掉（`editor.trimAutoWhitespace`）。
    fn remember_auto_whitespace(&mut self) {
        let line = self.cursor.selection.active.line.0;
        self.auto_whitespace_line = match self.text.as_ref() {
            Some(text) => {
                let source = line_text(&**text, line);
                (!source.is_empty() && source.trim().is_empty()).then_some(line)
            }
            None => None,
        };
    }

    /// 输入一个字符时的自动行为：自动包围、越过自己补的闭合符、自动闭合、
    /// `decreaseIndentPattern` 触发的重新缩进。处理了返回 `true`（调用方就不再普通输入）。
    pub(crate) fn auto_type(
        &mut self,
        ch: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.auto_edit_allowed() || self.is_read_only() || self.edit_host.is_none() {
            return false;
        }
        let Some(config) = self.language_config() else {
            return false;
        };
        let Some(text) = self.text.clone() else {
            return false;
        };
        let data = config.data().clone();
        let sel = self.cursor.selection;
        let options = self.options.auto_close;
        let converter = self.options.indent_converter();
        let auto_indent = self.options.auto_indent;
        let separators = self.options.word_separators.clone();

        // 1. 有选区：自动包围（每段各自包围；本视图当前是单选区，#52 合并后按选区逐段展开）。
        if !sel.is_empty() {
            let selected = range_text(&*text, sel.start(), sel.end());
            let Some(pair) = autoedit::autoclose::should_surround(&options, &data, ch, &selected)
            else {
                return false;
            };
            let start = (sel.start().line.0, sel.start().column.0 as usize);
            let end = (sel.end().line.0, sel.end().column.0 as usize);
            let changes = autoedit::autoclose::surround_changes(pair, start, end);
            // 包围后选区仍然是原来那段文本：起点与（同一行时的）终点都右移一个 `open`。
            let open_len = pair.open.len();
            let moved = Selection::new(
                pos(sel.start().line.0, (start.1 + open_len) as u64),
                pos(
                    sel.end().line.0,
                    (end.1
                        + if sel.start().line == sel.end().line {
                            open_len
                        } else {
                            0
                        }) as u64,
                ),
            );
            return self.run_edit(window, cx, move |s, snap, now| {
                let replacements = to_replacements(&**snap, &changes)?;
                Ok(EditOutcome {
                    edits: vec![s.edit(Transaction::new(replacements)?, now)?],
                    selection: Some(moved),
                })
            });
        }

        // 2. 空选区：越过 / 自动闭合 / 重新缩进。
        let caret = sel.active;
        let line = caret.line.0;
        let column = caret.column.0 as usize;
        let source = line_text(&*text, line);
        let before = source.get(..column.min(source.len())).unwrap_or("");
        let after = source.get(column.min(source.len())..).unwrap_or("");
        if autoedit::autoclose::should_overtype(
            &options,
            &data,
            &self.auto_closed,
            line,
            column,
            before,
            after,
            ch,
        ) {
            // 越过：把后面那个闭合符原样替换掉（和 VS Code 一样是一次可撤销的编辑）。
            let typed = ch.to_string();
            return self.run_edit(window, cx, move |s, snap, now| {
                let from = snap.offset(pos(line, column as u64))?;
                let to = snap.offset(pos(line, (column + typed.len()) as u64))?;
                let kind = EditKind::typing(&typed, s.previous_kind());
                Ok(s.edit_each(
                    vec![Some(CursorEdit::replace(Span::new(from, to)?, typed))],
                    kind,
                    now,
                )?
                .into())
            });
        }
        // 语法 token 只在真的有一对可闭合时才查：普通字符不走这一步，按键路径上没有额外开销。
        let tokens = if autoedit::autoclose::find_open_pair(&data, before, ch).is_some() {
            let (in_string, in_comment) = self.syntax.borrow().scope_at(line, column as u32);
            autoedit::autoclose::TokenContext {
                in_string,
                in_comment,
                in_regex: false,
            }
        } else {
            autoedit::autoclose::TokenContext::default()
        };
        if let Some(pair) = autoedit::autoclose::should_auto_close(
            &options,
            &data,
            before,
            after,
            ch,
            tokens,
            &separators,
        ) {
            let (open_tail, close) = (ch.to_string(), pair.close.to_string());
            let caret_at = column + open_tail.len();
            let closed = autoedit::autoclose::AutoClosed {
                line,
                column: caret_at,
                close: close.clone(),
            };
            let inserted = format!("{open_tail}{close}");
            let handled = self.run_edit(window, cx, move |s, snap, now| {
                let at = snap.offset(pos(line, column as u64))?;
                let kind = EditKind::typing(&open_tail, s.previous_kind());
                Ok(EditOutcome {
                    edits: s
                        .edit_each(
                            vec![Some(CursorEdit::replace(Span::new(at, at)?, inserted))],
                            kind,
                            now,
                        )?
                        .into_iter()
                        .collect(),
                    selection: Some(Selection::caret(pos(line, caret_at as u64))),
                })
            });
            if handled {
                self.auto_closed.record(closed);
            }
            return handled;
        }
        // 3. `decreaseIndentPattern`：输入 `}` 等字符后这一行要重新对齐。
        let lines = TextLines(&*text);
        let Some(indent) = autoedit::indent::indent_action_for_type(
            auto_indent,
            &lines,
            config,
            (line, column),
            (line, column),
            ch,
            &converter,
        ) else {
            return false;
        };
        let current = leading_whitespace(&source).to_string();
        if current == indent || column < current.len() {
            return false;
        }
        let typed = ch.to_string();
        let caret_at = indent.len() + (column - current.len()) + typed.len();
        self.run_edit(window, cx, move |s, snap, now| {
            let text: &dyn TextSource = &**snap;
            let changes = vec![
                autoedit::Change::replace((line, 0), (line, current.len()), indent),
                autoedit::Change::insert((line, column), typed),
            ];
            let replacements = to_replacements(text, &changes)?;
            Ok(EditOutcome {
                edits: vec![s.edit(Transaction::new(replacements)?, now)?],
                selection: Some(Selection::caret(pos(line, caret_at as u64))),
            })
        })
    }

    /// 退格：左右正好是一对自动补上的括号 / 引号时一起删除（`editor.autoClosingDelete`）。
    pub(crate) fn auto_delete_pair(&self) -> bool {
        if !self.auto_edit_allowed() {
            return false;
        }
        let (Some(config), Some(text)) = (self.language_config(), self.text.as_ref()) else {
            return false;
        };
        let sel = self.cursor.selection;
        if !sel.is_empty() {
            return false;
        }
        let line = sel.active.line.0;
        let column = sel.active.column.0 as usize;
        let source = line_text(&**text, line);
        let before = source.get(..column.min(source.len())).unwrap_or("");
        let after = source.get(column.min(source.len())..).unwrap_or("");
        autoedit::autoclose::should_delete_pair(
            &self.options.auto_close,
            config.data(),
            &self.auto_closed,
            line,
            column,
            before,
            after,
        )
    }

    /// 注释切换（`editor.action.commentLine` / `addCommentLine` / `removeCommentLine`）。
    pub(crate) fn toggle_comment(
        &mut self,
        kind: autoedit::comments::CommentType,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.auto_edit_allowed() {
            return false;
        }
        let Some(config) = self.language_config() else {
            return false;
        };
        let data = config.data().clone();
        if data.line_comment.is_none() {
            // 没有行注释的语言退化为块注释（VS Code 的做法）。
            return self.block_comment(window, cx);
        }
        let options = self.options.comments;
        let sel = self.cursor.selection;
        let first = sel.start().line.0;
        let mut last = sel.end().line.0;
        if last > first && sel.end().column.0 == 0 {
            last -= 1;
        }
        let trim = self.take_trim_line(first);
        self.run_edit(window, cx, move |s, snap, now| {
            let text: &dyn TextSource = &**snap;
            let lines = TextLines(text);
            let Some(mut changes) =
                autoedit::comments::toggle_line_comment(&lines, &data, first, last, kind, options)
            else {
                return Ok(EditOutcome::default());
            };
            changes.extend(trim);
            if changes.is_empty() {
                return Ok(EditOutcome::default());
            }
            let replacements = to_replacements(text, &changes)?;
            Ok(EditOutcome {
                edits: vec![s.edit(Transaction::new(replacements)?, now)?],
                selection: None,
            })
        })
    }

    /// 块注释切换（`editor.action.blockComment`）。
    pub(crate) fn block_comment(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.auto_edit_allowed() {
            return false;
        }
        let Some(config) = self.language_config() else {
            return false;
        };
        let data = config.data().clone();
        let insert_space = self.options.comments.insert_space;
        let sel = self.cursor.selection;
        let start = (sel.start().line.0, sel.start().column.0 as usize);
        let end = (sel.end().line.0, sel.end().column.0 as usize);
        self.run_edit(window, cx, move |s, snap, now| {
            let text: &dyn TextSource = &**snap;
            let lines = TextLines(text);
            let Some(auto) =
                autoedit::comments::toggle_block_comment(&lines, &data, start, end, insert_space)
            else {
                return Ok(EditOutcome::default());
            };
            let replacements = to_replacements(text, &auto.changes)?;
            if replacements.is_empty() {
                return Ok(EditOutcome::default());
            }
            Ok(EditOutcome {
                edits: vec![s.edit(Transaction::new(replacements)?, now)?],
                selection: auto.caret.map(|(l, c)| Selection::caret(pos(l, c as u64))),
            })
        })
    }

    /// 编辑类命令（#47）。不是编辑命令时返回 `None`。
    pub(crate) fn execute_edit(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let word_move = |m: MoveWord, select: bool| Some((m, select));
        let movement = match id {
            "cursorWordLeft" => word_move(MoveWord::Left, false),
            "cursorWordLeftSelect" => word_move(MoveWord::Left, true),
            "cursorWordRight" => word_move(MoveWord::Right, false),
            "cursorWordRightSelect" => word_move(MoveWord::Right, true),
            "cursorWordStartLeft" => word_move(MoveWord::StartLeft, false),
            "cursorWordStartLeftSelect" => word_move(MoveWord::StartLeft, true),
            "cursorWordStartRight" => word_move(MoveWord::StartRight, false),
            "cursorWordStartRightSelect" => word_move(MoveWord::StartRight, true),
            "cursorWordEndLeft" => word_move(MoveWord::EndLeft, false),
            "cursorWordEndLeftSelect" => word_move(MoveWord::EndLeft, true),
            "cursorWordEndRight" => word_move(MoveWord::EndRight, false),
            "cursorWordEndRightSelect" => word_move(MoveWord::EndRight, true),
            "cursorWordPartLeft" | "cursorWordPartStartLeft" => {
                word_move(MoveWord::PartLeft, false)
            }
            "cursorWordPartLeftSelect" | "cursorWordPartStartLeftSelect" => {
                word_move(MoveWord::PartLeft, true)
            }
            "cursorWordPartRight" => word_move(MoveWord::PartRight, false),
            "cursorWordPartRightSelect" => word_move(MoveWord::PartRight, true),
            "cursorWordAccessibilityLeft" => word_move(MoveWord::AccessibilityLeft, false),
            "cursorWordAccessibilityLeftSelect" => word_move(MoveWord::AccessibilityLeft, true),
            "cursorWordAccessibilityRight" => word_move(MoveWord::AccessibilityRight, false),
            "cursorWordAccessibilityRightSelect" => word_move(MoveWord::AccessibilityRight, true),
            _ => None,
        };
        if let Some((m, select)) = movement {
            self.move_word(m, select, cx);
            return Some(true);
        }
        let delete_word = |d: DeleteWord| Some(d);
        let deletion = match id {
            "deleteWordLeft" => delete_word(DeleteWord::Left),
            "deleteWordRight" => delete_word(DeleteWord::Right),
            "deleteWordStartLeft" => delete_word(DeleteWord::StartLeft),
            "deleteWordStartRight" => delete_word(DeleteWord::StartRight),
            "deleteWordEndLeft" => delete_word(DeleteWord::EndLeft),
            "deleteWordEndRight" => delete_word(DeleteWord::EndRight),
            "deleteWordPartLeft" => delete_word(DeleteWord::PartLeft),
            "deleteWordPartRight" => delete_word(DeleteWord::PartRight),
            _ => None,
        };
        if let Some(d) = deletion {
            self.run_edit(Some(window), cx, move |s, _, now| {
                Ok(s.delete_word(d, now)?.into())
            });
            return Some(true);
        }
        let w = Some(window);
        match id {
            "cursorLineStart"
            | "cursorLineStartSelect"
            | "cursorLineEnd"
            | "cursorLineEndSelect" => {
                let text = self.text.clone()?;
                let s = self.cursor.selection;
                let line = s.active.line.0;
                let col = if id.starts_with("cursorLineStart") {
                    0
                } else {
                    line_text(&*text, line).len() as u64
                };
                let target = pos(line, col);
                let anchor = if id.ends_with("Select") {
                    s.anchor
                } else {
                    target
                };
                self.cursor.set(Selection::new(anchor, target));
                self.reveal_caret_nearest(cx);
                self.selection_changed(SelectionSource::Keyboard, cx);
            }
            "deleteLeft" => {
                let (tab_stops, size) = (self.options.use_tab_stops, self.options.tab_size as u64);
                // #53：左右正好是一对自己补上的括号 / 引号时一起删除。
                let pair_sel = self.cursor.selection;
                let delete_pair = self.secondary.is_empty() && self.auto_delete_pair();
                self.run_edit(w, cx, move |s, snap, now| {
                    let text: &dyn TextSource = &**snap;
                    if delete_pair {
                        let p = pair_sel.active;
                        let line = line_text(text, p.line.0);
                        let col = p.column.0 as usize;
                        let open = line[..col].chars().next_back().map_or(0, char::len_utf8);
                        let close = line[col..].chars().next().map_or(0, char::len_utf8);
                        let range = Span::new(
                            text.offset(pos(p.line.0, (col - open) as u64))?,
                            text.offset(pos(p.line.0, (col + close) as u64))?,
                        )?;
                        return Ok(s
                            .edit_each(
                                vec![Some(CursorEdit::replace(range, ""))],
                                EditKind::DeletingLeft,
                                now,
                            )?
                            .into());
                    }
                    // 缩进里的空格：退到上一个制表位（VS Code `useTabStops`）。
                    let on_tab_stop = |sel: &Selection| {
                        if !tab_stops || !sel.is_empty() || sel.active.column.0 == 0 {
                            return false;
                        }
                        let line = line_text(text, sel.active.line.0);
                        let end = (sel.active.column.0 as usize).min(line.len());
                        !line[..end].is_empty() && line[..end].bytes().all(|b| b == b' ')
                    };
                    let any_tab_stop = s
                        .selections()
                        .all()
                        .iter()
                        .filter_map(|c| {
                            Some(Selection::new(
                                text.position(c.anchor).ok()?,
                                text.position(c.active).ok()?,
                            ))
                        })
                        .any(|sel| on_tab_stop(&sel));
                    if !any_tab_stop {
                        return Ok(s.delete_left(now)?.into());
                    }
                    let edits = for_each_selection(s, text, |sel| {
                        if on_tab_stop(&sel) {
                            let col = sel.active.column.0;
                            let target = (col - 1) / size * size;
                            let range = Span::new(
                                text.offset(pos(sel.active.line.0, target))?,
                                text.offset(sel.active)?,
                            )?;
                            return Ok(Some(CursorEdit::replace(range, "")));
                        }
                        if !sel.is_empty() {
                            let range =
                                Span::new(text.offset(sel.start())?, text.offset(sel.end())?)?;
                            return Ok(Some(CursorEdit::replace(range, "")));
                        }
                        let at = text.offset(sel.active)?;
                        let left = ed::selection::grapheme_left(text, at)?;
                        Ok((left != at)
                            .then(|| CursorEdit::replace(Span::new(left, at).expect("有序"), "")))
                    })?;
                    Ok(s.edit_each(edits, EditKind::DeletingLeft, now)?.into())
                });
            }
            "deleteRight" => {
                self.run_edit(w, cx, |s, _, now| Ok(s.delete_right(now)?.into()));
            }
            "deleteAllLeft" | "deleteAllRight" => {
                let left = id == "deleteAllLeft";
                self.run_edit(w, cx, move |s, snap, now| {
                    let text: &dyn TextSource = &**snap;
                    let edits = for_each_selection(s, text, |sel| {
                        let range = if !sel.is_empty() {
                            Span::new(text.offset(sel.start())?, text.offset(sel.end())?)?
                        } else {
                            let p = sel.active;
                            let line = line_text(text, p.line.0);
                            let (a, b) = if left {
                                (pos(p.line.0, 0), p)
                            } else if (p.column.0 as usize) < line.len() {
                                (p, pos(p.line.0, line.len() as u64))
                            } else {
                                // 行尾：删除换行（VS Code `deleteAllRight`）。
                                let next = (p.line.0 + 1).min(text.lines().known.saturating_sub(1));
                                (p, pos(next, 0))
                            };
                            Span::new(text.offset(a)?, text.offset(b)?)?
                        };
                        Ok((range.start() != range.end()).then(|| CursorEdit::replace(range, "")))
                    })?;
                    if edits.iter().all(Option::is_none) {
                        return Ok(EditOutcome::default());
                    }
                    Ok(s.edit_each(edits, EditKind::Other, now)?.into())
                });
            }
            "tab" => {
                let sel = self.cursor.selection;
                if sel.start().line != sel.end().line {
                    self.shift_lines(false, w, cx);
                } else {
                    let o = &self.options;
                    let (spaces, tab, sticky) =
                        (o.insert_spaces, o.tab_size as u64, o.use_tab_stops);
                    self.run_edit(w, cx, move |s, snap, now| {
                        let indent = if spaces {
                            let line = line_text(&**snap, sel.start().line.0);
                            let col = ed::selection::visible_column(&line, sel.start().column, tab);
                            let n = if sticky { tab - col % tab } else { tab };
                            " ".repeat(n as usize)
                        } else {
                            "\t".to_string()
                        };
                        Ok(s.type_text(&indent, now)?.into())
                    });
                }
            }
            "outdent" | "editor.action.outdentLines" => self.shift_lines(true, w, cx),
            "editor.action.indentLines" => self.shift_lines(false, w, cx),
            "lineBreakInsert" => {
                self.run_edit(w, cx, move |s, _, now| {
                    // 换行插在光标之后：每个光标都停在原位（VS Code `lineBreakInsert`）。
                    let starts: Vec<ByteOffset> =
                        s.selections().all().iter().map(|c| c.start()).collect();
                    let edits = s
                        .selections()
                        .all()
                        .iter()
                        .map(|c| Some(CursorEdit::replace(c.range(), "\n")))
                        .collect();
                    let out = s.edit_each(edits, EditKind::Other, now)?;
                    if let Some(applied) = &out {
                        let placed = starts
                            .into_iter()
                            .map(|o| applied.map_offset(o, ed::Bias::Before))
                            .collect();
                        place_cursors(s, placed);
                    }
                    Ok(out.into())
                });
            }
            "editor.action.insertLineAfter" | "editor.action.insertLineBefore" => {
                let after = id.ends_with("After");
                self.run_edit(w, cx, move |s, snap, now| {
                    let text: &dyn TextSource = &**snap;
                    // #52：每个光标各插一行；插入点按偏移排序后与 `inserted_ranges()` 一一对应。
                    let mut indents: Vec<(ByteOffset, usize)> = Vec::new();
                    let edits = for_each_selection(s, text, |sel| {
                        let line = sel.active.line.0;
                        let content = line_text(text, line);
                        let indent = leading_whitespace(&content).to_string();
                        let (at, inserted) = if after {
                            (pos(line, content.len() as u64), format!("\n{indent}"))
                        } else {
                            (pos(line, 0), format!("{indent}\n"))
                        };
                        let o = text.offset(at)?;
                        indents.push((o, indent.len()));
                        Ok(Some(CursorEdit::replace(Span::new(o, o)?, inserted)))
                    })?;
                    let out = s.edit_each(edits, EditKind::Other, now)?;
                    if let Some(applied) = &out {
                        indents.sort_by_key(|(o, _)| *o);
                        let placed = indents
                            .iter()
                            .zip(applied.inserted_ranges())
                            .map(|((_, indent), range)| {
                                if after {
                                    range.end()
                                } else {
                                    ByteOffset(range.start().0 + *indent as u64)
                                }
                            })
                            .collect();
                        place_cursors(s, placed);
                    }
                    Ok(out.into())
                });
            }
            "undo" | "redo" => {
                if self.composition.is_some() {
                    // 组合中的撤销由输入法消费；万一到达这里，只取消组合，不撤销上一次提交。
                    self.cancel_composition(cx);
                    return Some(true);
                }
                let undo = id == "undo";
                self.run_edit(w, cx, move |s, _, _| {
                    let edits = if undo { s.undo()? } else { s.redo()? };
                    Ok(EditOutcome {
                        edits,
                        selection: None,
                    })
                });
            }
            "cursorUndo" | "cursorRedo" => {
                let undo = id == "cursorUndo";
                // 不要求可编辑；没有会话时没有光标历史。
                if let (Some(host), Some(text)) = (self.edit_host.clone(), self.text.clone()) {
                    let moved = host.session().and_then(|mut s| {
                        let ok = if undo {
                            s.cursor_undo()
                        } else {
                            s.cursor_redo()
                        };
                        ok.then(|| from_core(&*text, s.selections().primary()))
                            .flatten()
                    });
                    if let Some(sel) = moved {
                        self.cursor.set(sel);
                        self.reveal_caret_nearest(cx);
                        self.selection_changed(SelectionSource::Keyboard, cx);
                    }
                }
            }
            "editor.action.clipboardCutAction" => {
                let mut copied = None;
                let whole_line = self.options.empty_selection_clipboard;
                self.run_edit(w, cx, |s, snap, now| {
                    let Some(data) = s.copy()? else {
                        return Ok(EditOutcome::default());
                    };
                    let ranges = ed::clipboard::cut_ranges(&**snap, s.selections(), whole_line)?;
                    copied = Some(data);
                    if ranges.is_empty() {
                        return Ok(EditOutcome::default());
                    }
                    let t =
                        Transaction::new(ranges.into_iter().map(Replacement::delete).collect())?;
                    Ok(EditOutcome {
                        edits: vec![s.edit(t, now)?],
                        selection: None,
                    })
                });
                if let Some(data) = copied {
                    self.write_clipboard(&data, cx);
                }
            }
            "editor.action.clipboardPasteAction" => {
                let Some(item) = cx.read_from_clipboard() else {
                    return Some(true);
                };
                let Some(text) = item.text() else {
                    return Some(true);
                };
                let data = ClipboardData {
                    from_empty_selection: item
                        .metadata()
                        .is_some_and(|m| m == CLIPBOARD_WHOLE_LINE),
                    ..ClipboardData::plain(text)
                };
                let overtype = self.overtype && self.options.overtype_on_paste;
                // #53：`editor.autoIndentOnPaste`（默认关）。在字符串里粘贴另看
                // `editor.autoIndentOnPasteWithinString`（默认开）。
                let caret = self.cursor.selection.start();
                let in_string = self
                    .syntax
                    .borrow()
                    .scope_at(caret.line.0, caret.column.0 as u32)
                    .0;
                let reindent = self.auto_edit_allowed()
                    && self.options.auto_indent_on_paste
                    && (!in_string || self.options.auto_indent_on_paste_within_string);
                let config = reindent.then(|| self.language_config()).flatten();
                let converter = self.options.indent_converter();
                let auto_indent = self.options.auto_indent;
                self.run_edit(w, cx, move |s, snap, now| {
                    if overtype && !data.text.contains(['\n', '\r']) {
                        extend_for_overtype(s, &**snap, &data.text)?;
                    }
                    let Some(applied) = s.paste(&data, now)? else {
                        return Ok(EditOutcome::default());
                    };
                    let mut edits = vec![applied];
                    if let Some(config) = config {
                        // 粘贴进来的那几行逐行重新缩进，并入同一个撤销单元。
                        let pasted = edits[0]
                            .changes
                            .ranges
                            .iter()
                            .map(|c| (c.new.start().0, c.new.end().0))
                            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
                        if let Some((first, end)) = pasted {
                            let after = s.snapshot();
                            let lines = TextLines(&*after);
                            let changes = autoedit::indent::reindent_lines(
                                auto_indent,
                                &lines,
                                config,
                                first,
                                end.saturating_sub(1).max(first),
                                &converter,
                            );
                            if !changes.is_empty() {
                                let replacements = to_replacements(&*after, &changes)?;
                                edits.push(s.edit_joined(Transaction::new(replacements)?, now)?);
                            }
                        }
                    }
                    Ok(EditOutcome {
                        edits,
                        selection: None,
                    })
                });
            }
            "editor.action.toggleOvertypeInsertMode" => {
                self.overtype = !self.overtype;
                cx.notify();
            }
            "editor.action.indentationToSpaces" | "editor.action.indentationToTabs" => {
                let spaces = id.ends_with("Spaces");
                self.run_edit(w, cx, move |s, _, now| {
                    Ok(s.convert_indentation(spaces, now)?.into())
                });
            }
            "editor.action.commentLine"
            | "editor.action.addCommentLine"
            | "editor.action.removeCommentLine" => {
                let kind = match id {
                    "editor.action.addCommentLine" => autoedit::comments::CommentType::ForceAdd,
                    "editor.action.removeCommentLine" => {
                        autoedit::comments::CommentType::ForceRemove
                    }
                    _ => autoedit::comments::CommentType::Toggle,
                };
                if self.is_read_only() {
                    self.edit_blocked(cx);
                } else {
                    self.toggle_comment(kind, w, cx);
                }
            }
            "editor.action.blockComment" => {
                if self.is_read_only() {
                    self.edit_blocked(cx);
                } else {
                    self.block_comment(w, cx);
                }
            }
            "editor.action.setSelectionAnchor" => {
                self.selection_anchor = Some(self.cursor.selection.active);
                cx.notify();
            }
            "editor.action.selectFromAnchorToCursor" => {
                if let Some(anchor) = self.selection_anchor.take() {
                    let s = Selection::new(anchor, self.cursor.selection.active);
                    self.cursor.set(self.clamp_selection(s));
                    self.selection_changed(SelectionSource::Keyboard, cx);
                }
            }
            "editor.action.goToSelectionAnchor" => {
                if let Some(anchor) = self.selection_anchor {
                    self.cursor
                        .set(self.clamp_selection(Selection::caret(anchor)));
                    self.reveal_caret_nearest(cx);
                    self.selection_changed(SelectionSource::Keyboard, cx);
                }
            }
            "editor.action.cancelSelectionAnchor" => {
                self.selection_anchor = None;
                cx.notify();
            }
            _ => return None,
        }
        Some(true)
    }

    fn write_clipboard(&self, data: &ClipboardData, cx: &mut Context<Self>) {
        let item = if data.from_empty_selection {
            ClipboardItem::new_string_with_metadata(data.text.clone(), CLIPBOARD_WHOLE_LINE.into())
        } else {
            ClipboardItem::new_string(data.text.clone())
        };
        cx.write_to_clipboard(item);
    }

    /// 选区所在各行增加 / 减少一级缩进（一次事务）。多行选区止于下一行行首时不含那一行。
    fn shift_lines(&mut self, outdent: bool, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let sel = self.cursor.selection;
        let (spaces, tab) = (self.options.insert_spaces, self.options.tab_size as usize);
        self.run_edit(window, cx, move |s, snap, now| {
            let first = sel.start().line.0;
            let mut last = sel.end().line.0;
            if last > first && sel.end().column.0 == 0 {
                last -= 1;
            }
            let unit = if spaces { " ".repeat(tab) } else { "\t".into() };
            let mut edits = Vec::new();
            for line in first..=last {
                let text = line_text(&**snap, line);
                let start = snap.offset(pos(line, 0))?;
                if outdent {
                    let n = if text.starts_with('\t') {
                        1
                    } else {
                        text.bytes().take(tab).take_while(|b| *b == b' ').count()
                    };
                    if n > 0 {
                        edits.push(Replacement::delete(Span::new(
                            start,
                            ByteOffset(start.0 + n as u64),
                        )?));
                    }
                } else if !(text.is_empty() && last > first) {
                    edits.push(Replacement::insert(start, unit.as_str()));
                }
            }
            if edits.is_empty() {
                return Ok(EditOutcome::default());
            }
            Ok(EditOutcome {
                edits: vec![s.edit(Transaction::new(edits)?, now)?],
                selection: None,
            })
        });
    }

    fn move_word(&mut self, m: MoveWord, select: bool, cx: &mut Context<Self>) {
        let Some(text) = self.text.clone() else {
            return;
        };
        let classifier = WordClassifier::new(&self.options.word_separators);
        let s = self.cursor.selection;
        let Ok(target) = ed::word::move_word(&*text, &classifier, s.active, m, false) else {
            return;
        };
        let anchor = if select { s.anchor } else { target };
        self.cursor.set(Selection::new(anchor, target));
        self.reveal_caret_nearest(cx);
        self.selection_changed(SelectionSource::Keyboard, cx);
    }

    /// `editor.stickyTabStops`：在只有空格的缩进里左右移动时按制表位跳。处理了返回 `true`。
    pub(crate) fn sticky_tab_move(&mut self, left: bool, cx: &mut Context<Self>) -> bool {
        if !self.options.sticky_tab_stops || !self.cursor.selection.is_empty() {
            return false;
        }
        let Some(text) = self.text.clone() else {
            return false;
        };
        let p = self.cursor.selection.active;
        let line = line_text(&*text, p.line.0);
        let indent = line.len() - line.trim_start_matches(' ').len();
        let col = p.column.0 as usize;
        let tab = self.options.tab_size as usize;
        if col > indent || (left && col == 0) || (!left && col >= indent) {
            return false;
        }
        let target = if left {
            (col - 1) / tab * tab
        } else {
            ((col / tab + 1) * tab).min(indent)
        };
        self.cursor
            .set(Selection::caret(pos(p.line.0, target as u64)));
        self.reveal_caret_nearest(cx);
        self.selection_changed(SelectionSource::Keyboard, cx);
        true
    }

    // ------------------------------------------------------------------ 输入法

    /// 取消组合串（`Esc`、切换输入法发来空串）：丢弃，不写入。
    pub fn cancel_composition(&mut self, cx: &mut Context<Self>) {
        if self.composition.take().is_some() {
            self.composition_entry = None;
            cx.notify();
        }
    }

    /// 点击别处、失焦：组合串在原位置提交（一个撤销单元），再让输入法丢掉它自己的组合状态。
    pub fn commit_composition(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        let Some(c) = self.composition.take() else {
            return;
        };
        self.composition_entry = None;
        cancel_platform_composition();
        if !c.text.is_empty() {
            self.commit_text(&c, &c.text, window, cx);
        }
        cx.notify();
    }

    /// 组合串提交：替换组合开始时的选区，作为一个独立的撤销单元。改写模式下按整串覆盖。
    fn commit_text(
        &mut self,
        c: &Composition,
        text: &str,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.cursor
            .set(self.clamp_selection(Selection::new(c.start, c.end)));
        let overtype = self.overtype && !text.contains(['\n', '\r']);
        let text = text.to_string();
        self.run_edit(window, cx, move |s, snap, now| {
            s.push_undo_stop();
            if overtype {
                extend_for_overtype(s, &**snap, &text)?;
            }
            let out = s.edit_each(
                s.selections()
                    .all()
                    .iter()
                    .map(|sel| Some(CursorEdit::replace(sel.range(), text.as_str())))
                    .collect(),
                EditKind::Other,
                now,
            )?;
            s.push_undo_stop();
            Ok(out.into())
        });
    }

    /// 输入法看到的区域：组合串（或选区）所在的行。
    pub(crate) fn ime_region(&self) -> Option<ImeRegion> {
        let text = self.text.as_deref()?;
        let (first, last) = match &self.composition {
            Some(c) => (c.start.line.0, c.start.line.0),
            None => {
                let s = self.cursor.selection;
                (s.start().line.0, s.end().line.0)
            }
        };
        let mut out = String::new();
        let mut line_starts = Vec::new();
        let mut marked = None;
        for line in first..=last {
            line_starts.push(out.len());
            let t = line_text(text, line);
            match &self.composition {
                Some(c) => {
                    let a = (c.start.column.0 as usize).min(t.len());
                    out.push_str(&t[..a]);
                    marked = Some(out.len()..out.len() + c.text.len());
                    out.push_str(&c.text);
                    let tail = if c.end.line == c.start.line {
                        t.get(c.end.column.0 as usize..).unwrap_or("").to_string()
                    } else {
                        line_text(text, c.end.line.0)
                            .get(c.end.column.0 as usize..)
                            .unwrap_or("")
                            .to_string()
                    };
                    out.push_str(&tail);
                }
                None => out.push_str(&t),
            }
            if line < last {
                out.push('\n');
            }
        }
        Some(ImeRegion {
            first,
            text: out,
            line_starts,
            marked,
        })
    }

    /// 组合行的显示条目（组合串内嵌在原行里，没有装饰）。
    pub(crate) fn composition_line(&mut self) -> Option<(u64, Rc<LineEntry>)> {
        let c = self.composition.clone()?;
        if let Some((key, entry)) = &self.composition_entry {
            if *key == c {
                return Some((c.start.line.0, entry.clone()));
            }
        }
        let region = self.ime_region()?;
        let source: Arc<str> = region.text.as_str().into();
        let display = DisplayLine::build(&source, self.options.tab_size, self.render_limit());
        let entry = Rc::new(self.plain_entry(display, source));
        self.composition_entry = Some((c.clone(), entry.clone()));
        Some((c.start.line.0, entry))
    }

    /// 组合串在组合行上的源列区间（UTF-8 字节）与组合内光标列。
    pub(crate) fn composition_columns(&self) -> Option<(Range<u32>, u32)> {
        let c = self.composition.as_ref()?;
        let a = c.start.column.0 as u32;
        Some((a..a + c.text.len() as u32, a + c.cursor as u32))
    }

    fn ime_bounds(&mut self, p: Position, composed_col: Option<u32>) -> Option<Bounds<Pixels>> {
        let m = self.metrics()?;
        if self.folding.map().is_hidden(p.line.0) {
            return None;
        }
        let view = self.view_line(p.line.0);
        let top = (m.line_top(view, &self.zones) - self.current_top()) * m.line_height;
        let x = match composed_col {
            Some(col) => {
                let (_, entry) = self.composition_line()?;
                let shaped = self.shapes.peek(&self.shape_key(&entry)).cloned();
                let xs = self.xs_for(&entry, shaped.as_deref());
                entry.display.x_for_source(&xs, col) as f64
            }
            None => {
                let (entry, shaped) = self.line_geometry(p.line.0)?;
                let xs = self.xs_for(&entry, shaped.as_deref());
                entry.display.x_for_source(&xs, p.column.0 as u32) as f64
            }
        };
        let text = self.layout.text;
        Some(Bounds::new(
            point(
                text.origin.x + px((x - self.scroll.x) as f32),
                text.origin.y + px(top as f32),
            ),
            size(px(self.font.cell_width.max(1.0)), px(self.font.line_height)),
        ))
    }
}

/// 改写模式：空选区向右扩展覆盖与 `text` 同样多的字素簇（不越过行尾）。
fn extend_for_overtype(
    s: &mut EditSession,
    snap: &dyn TextSource,
    text: &str,
) -> rove_base::Result<()> {
    let primary = *s.selections().primary();
    if !primary.is_empty() {
        return Ok(());
    }
    let p = snap.position(primary.active)?;
    let line = line_text(snap, p.line.0);
    let n = grapheme_boundaries(text).len().saturating_sub(1);
    let bounds = grapheme_boundaries(&line);
    let i = bounds.partition_point(|b| (*b as u64) < p.column.0);
    let end = bounds.get(i + n).copied().unwrap_or(line.len() as u32);
    let end = snap.offset(pos(p.line.0, end as u64))?;
    s.set_selections(Selections::single(ed::Selection::new(primary.active, end)));
    Ok(())
}

impl EntityInputHandler for CodeView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.ime_region()?;
        let (a, b) = (r.byte_of_utf16(range.start), r.byte_of_utf16(range.end));
        *adjusted = Some(r.utf16_of_byte(a)..r.utf16_of_byte(b));
        r.text.get(a..b).map(str::to_string)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let r = self.ime_region()?;
        if let (Some(c), Some(m)) = (&self.composition, &r.marked) {
            let caret = r.utf16_of_byte(m.start + c.cursor.min(c.text.len()));
            return Some(UTF16Selection {
                range: caret..caret,
                reversed: false,
            });
        }
        let s = self.cursor.selection;
        let (a, b) = (r.byte_of(s.start())?, r.byte_of(s.end())?);
        Some(UTF16Selection {
            range: r.utf16_of_byte(a)..r.utf16_of_byte(b),
            reversed: s.is_reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let r = self.ime_region()?;
        let m = r.marked.clone()?;
        Some(r.utf16_of_byte(m.start)..r.utf16_of_byte(m.end))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 平台“按现状确认组合串”：提交它。
        if let Some(c) = self.composition.take() {
            self.composition_entry = None;
            if !c.text.is_empty() {
                self.commit_text(&c, &c.text, Some(window), cx);
            }
        }
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(c) = self.composition.take() {
            // 组合串提交（或被清空）。
            self.composition_entry = None;
            if !text.is_empty() {
                self.commit_text(&c, text, Some(window), cx);
            }
            cx.notify();
            return;
        }
        if text.is_empty() {
            // 输入法取消后发来的空串：不能删掉选区。
            return;
        }
        if !self.is_editable() {
            if self.is_read_only() {
                self.edit_blocked(cx);
            }
            return;
        }
        if let Some(range) = range {
            // 平台指定了区间（重新转换等）：换算成文档选区再输入。
            if let Some(r) = self.ime_region() {
                let a = r.position_of(r.byte_of_utf16(range.start), None);
                let b = r.position_of(r.byte_of_utf16(range.end), None);
                self.cursor.set(self.clamp_selection(Selection::new(a, b)));
            }
        }
        self.type_text(text, Some(window), cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_editable() {
            // 只读文档不进入组合态（只在组合开始时提示一次）。
            if self.is_read_only() && !new_text.is_empty() && self.composition.is_none() {
                self.edit_blocked(cx);
            }
            return;
        }
        if new_text.is_empty() {
            self.cancel_composition(cx);
            return;
        }
        let s = self.cursor.selection;
        let (start, end) = match &self.composition {
            Some(c) => (c.start, c.end),
            None => (s.start(), s.end()),
        };
        // 组合串内的光标：相对组合串本身的 UTF-16 偏移（不能按整篇文档换算，见 ROVE_PATCHES）。
        let cursor = match new_selected_range {
            Some(r) => utf16_to_byte(new_text, r.end),
            None => new_text.len(),
        };
        self.composition = Some(Composition {
            start,
            end,
            text: new_text.to_string(),
            cursor,
        });
        self.composition_entry = None;
        // 组合行在这里立即整形（事件处理，不在绘制里），下一帧直接画正式字形。
        if let Some((_, entry)) = self.composition_line() {
            self.shape_entry_now(&entry, window);
        }
        self.caret_visible = true;
        self.reveal_position(start, Reveal::Nearest, cx);
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let r = self.ime_region()?;
        let byte = r.byte_of_utf16(range_utf16.start);
        if let (Some(m), Some(c)) = (&r.marked, &self.composition) {
            if byte >= m.start && byte <= m.end {
                let col = c.start.column.0 as u32 + (byte - m.start) as u32;
                let start = c.start;
                return self.ime_bounds(start, Some(col));
            }
        }
        let p = r.position_of(byte, self.composition.as_ref());
        let composed = self.composition.as_ref().and_then(|c| {
            (p.line == c.start.line)
                .then(|| (byte - r.line_starts[(p.line.0 - r.first) as usize]) as u32)
        });
        self.ime_bounds(p, composed)
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let p = self.position_for_point(point)?;
        let r = self.ime_region()?;
        let byte = r.byte_of(p)?;
        Some(r.utf16_of_byte(byte))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::{Line, LineRange};
    use rove_buffer::edit::LineChange;

    fn change(old: (u64, u64), new: (u64, u64), delta: i64) -> LineChanges {
        LineChanges {
            ranges: vec![LineChange {
                old: LineRange::new(Line(old.0), Line(old.1)).unwrap(),
                new: LineRange::new(Line(new.0), Line(new.1)).unwrap(),
            }],
            line_delta: delta,
        }
    }

    #[test]
    fn line_maps_shift_rows_after_an_inserted_newline() {
        // 第 5 行里插入一个换行：旧 5 → 新 5..7。
        let c = change((5, 6), (5, 7), 1);
        assert_eq!(old_to_new(&c, 4), Some(4));
        assert_eq!(old_to_new(&c, 5), None);
        assert_eq!(old_to_new(&c, 6), Some(7));
        assert_eq!(new_to_old(&c, 4), 4);
        assert_eq!(new_to_old(&c, 5), 5);
        assert_eq!(new_to_old(&c, 6), 5);
        assert_eq!(new_to_old(&c, 7), 6);
    }

    #[test]
    fn combined_edit_covers_consecutive_keystrokes() {
        let mut session = EditSession::new(
            ed::EditBuffer::new(
                "abc\n",
                rove_buffer::LineEndings::Uniform(rove_buffer::Eol::Lf),
            ),
            ed::EditOptions::default(),
        );
        let before = session.snapshot();
        session.set_selections(Selections::single(ed::Selection::cursor(ByteOffset(1))));
        let now = Instant::now();
        let a = session.type_text("X", now).unwrap().unwrap();
        let b = session.type_text("Y", now).unwrap().unwrap();
        let c = session.delete_left(now).unwrap().unwrap();
        let after = session.snapshot();
        let e = combine_edits(&[a, b, c], &*after).unwrap();
        assert_eq!(e.version, before.version());
        assert_eq!((e.range.start().0, e.range.end().0), (1, 1));
        assert_eq!(&*e.inserted, "X");
    }

    #[test]
    fn ime_region_maps_utf16_around_composition() {
        let r = ImeRegion {
            first: 3,
            text: "a😀你好b".into(),
            line_starts: vec![0],
            marked: Some(5..11),
        };
        let c = Composition {
            start: pos(3, 5),
            end: pos(3, 5),
            text: "你好".into(),
            cursor: 6,
        };
        assert_eq!(r.utf16_of_byte(5), 3);
        assert_eq!(r.byte_of_utf16(3), 5);
        assert_eq!(r.position_of(12, Some(&c)), pos(3, 6));
        assert_eq!(r.position_of(1, Some(&c)), pos(3, 1));
    }
}
