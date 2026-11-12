//! 编辑会话（#46）：把缓冲、选区、撤销栈与光标撤销栈绑在一起，是 R46（编辑交互）、R47（当前
//! 文件替换）、R48（保存）、R62（全局替换）改文本的唯一入口。
//!
//! 时间由调用方传入（`now`），模型本身不读时钟，便于测试与回放。

use std::time::Instant;

use rove_base::*;

use super::clipboard::{self, ClipboardData};
use super::history::{CursorHistory, EditKind, Step, UndoHistory};
use super::rewrite;
use super::selection::{grapheme_left, grapheme_right, CursorPlacement, Selection, Selections};
use super::settings::EditOptions;
use super::transaction::{AppliedEdit, Bias, EditBuffer, Replacement, Transaction};
use super::word::{self, DeleteWord, DeleteWordContext, MoveWord, WordClassifier};
use crate::contracts::{Eol, LineEndings, TextSource};

/// 某个选区上的一次替换（下标与 [`Selections::all`] 对应）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorEdit {
    pub range: ByteRange,
    pub text: String,
    pub placement: CursorPlacement,
}

impl CursorEdit {
    pub fn replace(range: ByteRange, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
            placement: CursorPlacement::AfterInserted,
        }
    }
}

/// 可编辑文档的会话状态。
#[derive(Clone, Debug)]
pub struct EditSession {
    buffer: EditBuffer,
    selections: Selections,
    history: UndoHistory,
    cursors: CursorHistory,
    options: EditOptions,
    classifier: WordClassifier,
    previous_kind: EditKind,
}

impl EditSession {
    /// 光标初始在文档开头。
    pub fn new(buffer: EditBuffer, options: EditOptions) -> Self {
        Self {
            buffer,
            selections: Selections::single(Selection::cursor(ByteOffset(0))),
            history: UndoHistory::default(),
            cursors: CursorHistory::default(),
            classifier: WordClassifier::with_segmenter_locales(
                &options.word_separators,
                &options.word_segmenter_locales,
            ),
            options,
            previous_kind: EditKind::Other,
        }
    }

    pub fn buffer(&self) -> &EditBuffer {
        &self.buffer
    }

    /// 当前快照（O(1)）。
    pub fn snapshot(&self) -> std::sync::Arc<crate::RopeText> {
        self.buffer.snapshot()
    }

    /// 解除或设置只读原因（不进入撤销栈）。
    pub fn set_read_only(&mut self, reasons: std::sync::Arc<[crate::ReadOnlyReason]>) {
        self.buffer.set_read_only(reasons);
    }

    pub fn options(&self) -> &EditOptions {
        &self.options
    }

    /// 设置变化时更新（`editor.wordSeparators` 等）。
    pub fn set_options(&mut self, options: EditOptions) {
        self.classifier = WordClassifier::with_segmenter_locales(
            &options.word_separators,
            &options.word_segmenter_locales,
        );
        self.options = options;
    }

    pub fn classifier(&self) -> &WordClassifier {
        &self.classifier
    }

    pub fn selections(&self) -> &Selections {
        &self.selections
    }

    /// 上一次编辑的类型。
    pub fn previous_kind(&self) -> EditKind {
        self.previous_kind
    }

    /// 显式改变选区（点击、方向键、按单词移动……）：先按设置截断与合并；位置真的变了时记入
    /// 光标撤销栈，并断开文本撤销单元（下一次输入开始新单元）。
    pub fn set_selections(&mut self, mut selections: Selections) {
        selections.normalize(
            self.options.multi_cursor_merge_overlapping,
            self.options.multi_cursor_limit,
        );
        if !selections.same_ranges(&self.selections) {
            self.cursors.record(&self.selections);
            self.history.close();
        }
        self.selections = selections;
    }

    /// `pushUndoStop`：之后的编辑开始新的撤销单元。
    pub fn push_undo_stop(&mut self) {
        self.history.close();
    }

    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }

    /// 执行一个任意事务（R47 的“全部替换”、R62 的跨文件批量编辑）：永远是一个撤销单元，
    /// 选区按偏移映射到新文本。
    pub fn edit(&mut self, transaction: Transaction, now: Instant) -> Result<AppliedEdit> {
        let before = self.selections.clone();
        self.commit(&transaction, EditKind::Other, now, false, None, |applied| {
            before.map(|s| Ok(map_selection(applied, s)))
        })
    }

    /// 每个选区各自做一次替换（`None` 表示该选区不动），合成一个事务。区间重叠时
    /// [`Error::InvalidInput`]。没有任何替换时返回 `Ok(None)`。
    pub fn edit_each(
        &mut self,
        edits: Vec<Option<CursorEdit>>,
        kind: EditKind,
        now: Instant,
    ) -> Result<Option<AppliedEdit>> {
        self.edit_each_inner(edits, kind, now, false)
    }

    fn edit_each_inner(
        &mut self,
        edits: Vec<Option<CursorEdit>>,
        kind: EditKind,
        now: Instant,
        force_break: bool,
    ) -> Result<Option<AppliedEdit>> {
        if edits.len() != self.selections.len() {
            return Err(Error::InvalidInput);
        }
        let mut indexed: Vec<(usize, CursorEdit)> = edits
            .into_iter()
            .enumerate()
            .filter_map(|(index, edit)| edit.map(|edit| (index, edit)))
            .collect();
        if indexed.is_empty() {
            return Ok(None);
        }
        // 与 Transaction::new 相同的稳定排序，第 k 个替换对应 inserted_ranges()[k]。
        indexed.sort_by_key(|(_, edit)| (edit.range.start(), edit.range.end()));
        let transaction = Transaction::new(
            indexed
                .iter()
                .map(|(_, edit)| Replacement::new(edit.range.clone(), edit.text.as_str()))
                .collect(),
        )?;
        let before = self.selections.clone();
        let applied = self.commit(&transaction, kind, now, force_break, None, |applied| {
            let mut placed: Vec<Option<Selection>> = vec![None; before.len()];
            for (slot, (index, edit)) in indexed.iter().enumerate() {
                placed[*index] = Some(match edit.placement {
                    CursorPlacement::AfterInserted => {
                        Selection::cursor(applied.inserted_ranges()[slot].end())
                    }
                    CursorPlacement::Preserve => map_selection(applied, &before.all()[*index]),
                });
            }
            Selections::new(
                placed
                    .into_iter()
                    .zip(before.all())
                    .map(|(placed, old)| placed.unwrap_or_else(|| map_selection(applied, old)))
                    .collect(),
            )
        })?;
        Ok(Some(applied))
    }

    /// 应用事务并记入撤销栈。
    fn commit(
        &mut self,
        transaction: &Transaction,
        kind: EditKind,
        now: Instant,
        force_break: bool,
        line_endings_after: Option<LineEndings>,
        selections_after: impl FnOnce(&AppliedEdit) -> Result<Selections>,
    ) -> Result<AppliedEdit> {
        let start_new = force_break
            || !self.history.open
            || kind.breaks_after(self.previous_kind)
            || self.history.timed_out(now, self.options.undo_group_timeout);
        self.commit_grouped(
            transaction,
            kind,
            now,
            start_new,
            line_endings_after,
            selections_after,
        )
    }

    fn commit_grouped(
        &mut self,
        transaction: &Transaction,
        kind: EditKind,
        now: Instant,
        start_new: bool,
        line_endings_after: Option<LineEndings>,
        selections_after: impl FnOnce(&AppliedEdit) -> Result<Selections>,
    ) -> Result<AppliedEdit> {
        let line_endings_before = self.buffer.line_endings().clone();
        let applied = self.buffer.apply(transaction)?;
        if let Some(line_endings) = line_endings_after {
            self.buffer.set_line_endings(line_endings);
        }
        // 选区映射只依赖已经算好的区间，不会失败；失败时保底放在原光标的映射位置。
        let mut after = selections_after(&applied).unwrap_or_else(|_| {
            Selections::single(Selection::cursor(
                applied.map_offset(self.selections.primary().active, Bias::After),
            ))
        });
        after.normalize(
            self.options.multi_cursor_merge_overlapping,
            self.options.multi_cursor_limit,
        );
        self.history.record(
            Step {
                forward: applied.forward().clone(),
                inverse: applied.inverse().clone(),
                line_endings_before,
                line_endings_after: self.buffer.line_endings().clone(),
            },
            start_new,
            &self.selections,
            &after,
            now,
        );
        if kind == EditKind::Other {
            self.history.close();
        }
        self.previous_kind = kind;
        self.selections = after;
        self.cursors.clear();
        Ok(applied)
    }

    /// 输入文本：替换每个选区，光标停在输入之后。撤销分组按 VS Code 的输入规则。
    pub fn type_text(&mut self, text: &str, now: Instant) -> Result<Option<AppliedEdit>> {
        let kind = EditKind::typing(text, self.previous_kind);
        let edits = self
            .selections
            .all()
            .iter()
            .map(|s| Some(CursorEdit::replace(s.range(), text)))
            .collect();
        self.edit_each_inner(edits, kind, now, false)
    }

    /// 退格：有选区删选区，否则删光标左边一个字素簇（行首删除上一行的换行）。
    pub fn delete_left(&mut self, now: Instant) -> Result<Option<AppliedEdit>> {
        self.delete_char(true, now)
    }

    /// Delete：有选区删选区，否则删光标右边一个字素簇。
    pub fn delete_right(&mut self, now: Instant) -> Result<Option<AppliedEdit>> {
        self.delete_char(false, now)
    }

    fn delete_char(&mut self, left: bool, now: Instant) -> Result<Option<AppliedEdit>> {
        let text = self.buffer.snapshot();
        // 有非空选区时总是断开（VS Code `deleteLeft` / `deleteRight`）。
        let force_break = self.selections.all().iter().any(|s| !s.is_empty());
        let mut edits = Vec::with_capacity(self.selections.len());
        for selection in self.selections.all() {
            edits.push(if !selection.is_empty() {
                Some(CursorEdit::replace(selection.range(), ""))
            } else {
                let other = if left {
                    grapheme_left(&*text, selection.active)?
                } else {
                    grapheme_right(&*text, selection.active)?
                };
                (other != selection.active).then(|| {
                    CursorEdit::replace(
                        Span::new(other.min(selection.active), other.max(selection.active))
                            .expect("有序"),
                        "",
                    )
                })
            });
        }
        let kind = if left {
            EditKind::DeletingLeft
        } else {
            EditKind::DeletingRight
        };
        self.edit_each_inner(edits, kind, now, force_break)
    }

    /// 按单词移动光标（`select` 为真时只移动 active，扩展选区）。
    pub fn move_word(&mut self, command: MoveWord, select: bool) -> Result<()> {
        let text = self.buffer.snapshot();
        let has_multicursor = self.selections.len() > 1;
        let classifier = &self.classifier;
        let moved = self.selections.map(|selection| {
            let position = text.position(selection.active)?;
            let target = word::move_word(&*text, classifier, position, command, has_multicursor)?;
            let target = text.offset(target)?;
            Ok(Selection::new(
                if select { selection.anchor } else { target },
                target,
            ))
        })?;
        self.set_selections(moved);
        Ok(())
    }

    /// 按单词删除（`deleteWord*` / `deleteWordPart*`）：前后都断开撤销单元。
    pub fn delete_word(
        &mut self,
        command: DeleteWord,
        now: Instant,
    ) -> Result<Option<AppliedEdit>> {
        let text = self.buffer.snapshot();
        let mut edits = Vec::with_capacity(self.selections.len());
        for selection in self.selections.all() {
            let context = DeleteWordContext {
                text: &*text,
                classifier: &self.classifier,
                anchor: text.position(selection.anchor)?,
                active: text.position(selection.active)?,
                auto_closing_pair: false,
            };
            edits.push(word::delete_word(&context, command)?.and_then(|range| {
                let range = Span::new(
                    text.offset(range.start()).ok()?,
                    text.offset(range.end()).ok()?,
                )
                .ok()?;
                (range.start() != range.end()).then(|| CursorEdit::replace(range, ""))
            }));
        }
        self.edit_each_inner(edits, EditKind::Other, now, true)
    }

    /// `deleteInsideWord`。
    pub fn delete_inside_word(
        &mut self,
        only_word: bool,
        now: Instant,
    ) -> Result<Option<AppliedEdit>> {
        let text = self.buffer.snapshot();
        let mut edits = Vec::with_capacity(self.selections.len());
        for selection in self.selections.all() {
            let range = word::delete_inside_word(
                &*text,
                &self.classifier,
                text.position(selection.anchor)?,
                text.position(selection.active)?,
                only_word,
            )?;
            let range = Span::new(text.offset(range.start())?, text.offset(range.end())?)?;
            edits.push((range.start() != range.end()).then(|| CursorEdit::replace(range, "")));
        }
        self.edit_each_inner(edits, EditKind::Other, now, true)
    }

    /// 撤销一个单元：文本逐字节回到编辑之前，选区回到**编辑前**的位置。没有可撤销的内容时
    /// 返回空列表。返回的每个 [`AppliedEdit`] 都要像普通编辑一样交给语法与视图。
    pub fn undo(&mut self) -> Result<Vec<AppliedEdit>> {
        self.history.close();
        if !self.buffer.read_only().is_empty() {
            return Err(Error::Unsupported);
        }
        let Some(group) = self.history.undo.pop() else {
            return Ok(Vec::new());
        };
        let mut applied = Vec::with_capacity(group.steps.len());
        for step in group.steps.iter().rev() {
            applied.push(self.buffer.apply(&step.inverse)?);
            self.buffer
                .set_line_endings(step.line_endings_before.clone());
        }
        self.selections = group.selections_before.clone();
        self.history.redo.push(group);
        self.previous_kind = EditKind::Other;
        self.cursors.clear();
        Ok(applied)
    }

    /// 重做一个单元，选区回到该单元编辑之后的位置。
    pub fn redo(&mut self) -> Result<Vec<AppliedEdit>> {
        self.history.close();
        if !self.buffer.read_only().is_empty() {
            return Err(Error::Unsupported);
        }
        let Some(group) = self.history.redo.pop() else {
            return Ok(Vec::new());
        };
        let mut applied = Vec::with_capacity(group.steps.len());
        for step in &group.steps {
            applied.push(self.buffer.apply(&step.forward)?);
            self.buffer
                .set_line_endings(step.line_endings_after.clone());
        }
        self.selections = group.selections_after.clone();
        self.history.undo.push(group);
        self.previous_kind = EditKind::Other;
        self.cursors.clear();
        Ok(applied)
    }

    /// `cursorUndo`：只回退选区，不动文本。没有可回退的选区时返回 `false`。
    pub fn cursor_undo(&mut self) -> bool {
        match self.cursors.undo(&self.selections) {
            Some(selections) => {
                self.selections = selections;
                self.history.close();
                true
            }
            None => false,
        }
    }

    /// `cursorRedo`。
    pub fn cursor_redo(&mut self) -> bool {
        match self.cursors.redo(&self.selections) {
            Some(selections) => {
                self.selections = selections;
                self.history.close();
                true
            }
            None => false,
        }
    }

    /// 复制当前选区（`editor.emptySelectionClipboard` 决定无选区时是否复制整行）。
    pub fn copy(&self) -> Result<Option<ClipboardData>> {
        clipboard::copy(
            &*self.buffer.snapshot(),
            &self.selections,
            self.options.empty_selection_clipboard,
            self.buffer.eol(),
        )
    }

    /// 剪切：返回写进剪贴板的内容，并删除对应区间（一个撤销单元）。
    pub fn cut(&mut self, now: Instant) -> Result<Option<ClipboardData>> {
        let Some(data) = self.copy()? else {
            return Ok(None);
        };
        let text = self.buffer.snapshot();
        let ranges = clipboard::cut_ranges(
            &*text,
            &self.selections,
            self.options.empty_selection_clipboard,
        )?;
        if !ranges.is_empty() {
            let transaction =
                Transaction::new(ranges.into_iter().map(Replacement::delete).collect())?;
            let before = self.selections.clone();
            self.commit(&transaction, EditKind::Other, now, true, None, |applied| {
                before.map(|s| {
                    Ok(Selection::cursor(
                        applied.map_offset(s.active, Bias::Before),
                    ))
                })
            })?;
        }
        Ok(Some(data))
    }

    /// 粘贴（`editor.multiCursorPaste` 决定多光标时如何分配）。
    pub fn paste(&mut self, data: &ClipboardData, now: Instant) -> Result<Option<AppliedEdit>> {
        let edits = clipboard::paste_edits(
            &*self.buffer.snapshot(),
            &self.selections,
            data,
            self.options.empty_selection_clipboard,
            self.options.multi_cursor_paste,
        )?;
        let edits = edits
            .into_iter()
            .map(|edit| {
                Some(CursorEdit {
                    range: edit.range,
                    text: edit.text,
                    placement: edit.placement,
                })
            })
            .collect();
        self.edit_each_inner(edits, EditKind::Other, now, true)
    }

    /// 缩进转换（`editor.action.indentationToSpaces` / `indentationToTabs`），用 `editor.tabSize`。
    pub fn convert_indentation(
        &mut self,
        to_spaces: bool,
        now: Instant,
    ) -> Result<Option<AppliedEdit>> {
        let transaction = rewrite::indentation_transaction(
            &*self.buffer.snapshot(),
            self.options.tab_size,
            to_spaces,
        )?;
        if transaction.is_empty() {
            return Ok(None);
        }
        let before = self.selections.clone();
        self.commit(&transaction, EditKind::Other, now, true, None, |applied| {
            before.map(|s| Ok(map_selection(applied, s)))
        })
        .map(Some)
    }

    /// 把全部行尾统一为 `eol`（一个撤销单元），之后新输入的换行也用它。
    pub fn change_eol(&mut self, eol: Eol, now: Instant) -> Result<Option<AppliedEdit>> {
        if !self.buffer.read_only().is_empty() {
            return Err(Error::Unsupported);
        }
        let transaction = rewrite::eol_transaction(&*self.buffer.snapshot(), eol)?;
        if transaction.is_empty() {
            self.buffer.set_line_endings(LineEndings::Uniform(eol));
            return Ok(None);
        }
        let before = self.selections.clone();
        self.commit(
            &transaction,
            EditKind::Other,
            now,
            true,
            Some(LineEndings::Uniform(eol)),
            |applied| before.map(|s| Ok(map_selection(applied, s))),
        )
        .map(Some)
    }

    /// 便于测试与调试：当前全文。
    /// 撤销位置标记（#49）：文本回到某个标记时的内容，标记就相同。保存时记下标记，
    /// “当前标记 ≠ 保存时的标记”即为有未保存的修改（撤销回保存点后重新变为未修改）。
    pub fn history_marker(&self) -> u64 {
        self.history.marker()
    }

    /// 把事务并入最后一个撤销单元（#49：保存时的文本变换与用户的改动是同一个撤销单元）。
    /// 撤销栈为空时开启新单元。之后的编辑开始新单元。
    pub fn edit_joined(&mut self, transaction: Transaction, now: Instant) -> Result<AppliedEdit> {
        let before = self.selections.clone();
        let start_new = self.history.undo.is_empty();
        // 栈顶单元即使已关闭（间隔超时、上一次是“其他编辑”）也照样并入。
        self.history.open = true;
        self.commit_grouped(
            &transaction,
            EditKind::Other,
            now,
            start_new,
            None,
            |applied| before.map(|s| Ok(map_selection(applied, s))),
        )
    }

    /// 用磁盘上的新内容替换全文（#49：外部修改后的重新加载、还原）。只替换首尾公共部分
    /// 之间的区间，所以未变的部分里的选区原样保留；是一个可撤销的单元。文档带只读原因时同样
    /// 生效（重新加载不是用户编辑）。内容相同时返回 `None`，只更新换行符信息。
    pub fn replace_from_disk(
        &mut self,
        text: &str,
        line_endings: LineEndings,
        now: Instant,
    ) -> Result<Option<AppliedEdit>> {
        let current = self.buffer.contents();
        let old = current.as_bytes();
        let new = text.as_bytes();
        let mut prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        while !current.is_char_boundary(prefix) || !text.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let max_suffix = old.len().min(new.len()) - prefix;
        let mut suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        while !current.is_char_boundary(old.len() - suffix)
            || !text.is_char_boundary(new.len() - suffix)
        {
            suffix -= 1;
        }
        let range = Span::new(
            ByteOffset(prefix as u64),
            ByteOffset((old.len() - suffix) as u64),
        )?;
        if range.start() == range.end() && prefix + suffix == new.len() {
            self.buffer.set_line_endings(line_endings);
            return Ok(None);
        }
        let inserted = &text[prefix..new.len() - suffix];
        let transaction = Transaction::single(range, inserted).verbatim();
        let reasons: std::sync::Arc<[crate::ReadOnlyReason]> =
            std::sync::Arc::from(self.buffer.read_only());
        self.buffer.set_read_only(std::sync::Arc::from([]));
        self.history.close();
        let before = self.selections.clone();
        let result = self.commit(
            &transaction,
            EditKind::Other,
            now,
            true,
            Some(line_endings),
            |applied| before.map(|s| Ok(map_selection(applied, s))),
        );
        self.buffer.set_read_only(reasons);
        result.map(Some)
    }

    pub fn contents(&self) -> String {
        self.buffer.contents()
    }

    /// 便于调用方：主光标的行列。
    pub fn primary_position(&self) -> Result<Position> {
        self.buffer
            .text()
            .position(self.selections.primary().active)
    }
}

fn map_selection(applied: &AppliedEdit, selection: &Selection) -> Selection {
    Selection::new(
        applied.map_offset(selection.anchor, Bias::After),
        applied.map_offset(selection.active, Bias::After),
    )
}
