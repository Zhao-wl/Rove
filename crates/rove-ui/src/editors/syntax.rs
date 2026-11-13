//! Syntax highlighting of opened documents (#94).
//!
//! Small documents are highlighted before their tab becomes ready, so they never show
//! uncoloured text. Larger ones become readable first; highlighting then runs in the
//! background, visible lines first, and each result is applied only to tabs that still
//! show that document at that version. Releasing the last tab of a document cancels its
//! running analysis and drops its baseline in the [`DocumentHighlighter`].

use rove_base::{Cancellation, DocumentId, Line, LineRange, Span, Task, TextEdit, Version};
use rove_buffer::{Document, SizeClass};
use rove_syntax::{highlight::DocumentHighlighter, SyntaxUpdate};
use std::{collections::BTreeMap, sync::Arc};

/// Documents below this size are highlighted before they are shown.
pub(crate) const INLINE_BYTES: u64 = 256 * 1024;

/// A background analysis for the host to run off the UI thread; its result goes back to
/// [`super::EditorService::finish_syntax`] with the same `document` and `generation`.
pub(crate) struct SyntaxJob {
    pub(crate) document: DocumentId,
    pub(crate) generation: u64,
    pub(crate) task: Task<SyntaxUpdate>,
}

struct Running {
    cancel: Cancellation,
    generation: u64,
    version: Version,
    /// The visible-first pass; a full pass follows it.
    partial: bool,
}

pub(crate) struct Highlights {
    pub(crate) service: Arc<dyn DocumentHighlighter>,
    running: BTreeMap<DocumentId, Running>,
    serial: u64,
    /// #47：分析进行中又来了编辑：不取消它（它完成后基线前进到它的版本），先记下最新文档与
    /// 之后的编辑，完成时合成一个增量编辑再分析。`None` 表示中间有无法增量的变化（整篇重来）。
    pending: BTreeMap<DocumentId, (Arc<Document>, Option<Vec<TextEdit>>)>,
    /// #47：同一内容换了版本（可编辑文档换上会话的快照）：(旧版本, 新版本)。两个版本的分析结果与
    /// 增量编辑互认，不因为换快照丢掉进行中的着色、也不让第一次输入退化成整篇重解析。
    alias: BTreeMap<DocumentId, (Version, Version)>,
    /// #47：最近一次成功分析的版本（增量编辑的基线）。
    baseline: BTreeMap<DocumentId, Version>,
}

/// Highlighting is skipped for huge documents, which open read-only and unparsed.
pub(crate) fn highlighted(document: &Document) -> bool {
    document.metadata.size != SizeClass::Huge
}

pub(crate) fn inline(document: &Document) -> bool {
    document.metadata.size == SizeClass::Normal && document.text.byte_len() < INLINE_BYTES
}

/// Lines from `top` (in virtual lines) through one screen of `rows`.
pub(crate) fn visible(top: f64, rows: f64) -> Option<LineRange> {
    let start = top.max(0.0).floor() as u64;
    Span::new(Line(start), Line(start + rows.max(1.0).ceil() as u64 + 1)).ok()
}

impl Highlights {
    pub(crate) fn new(service: Arc<dyn DocumentHighlighter>) -> Self {
        Self {
            service,
            running: BTreeMap::new(),
            serial: 0,
            pending: BTreeMap::new(),
            alias: BTreeMap::new(),
            baseline: BTreeMap::new(),
        }
    }

    /// #47：`document` 的内容不变、版本从 `from` 换成 `to`。
    pub(crate) fn rebased(&mut self, document: DocumentId, from: Version, to: Version) {
        self.alias.insert(document, (from, to));
    }

    /// #47：两个版本是否是同一内容（相同，或互为别名）。
    pub(crate) fn same_content(&self, document: DocumentId, a: Version, b: Version) -> bool {
        a == b
            || self
                .alias
                .get(&document)
                .is_some_and(|(from, to)| (a == *from && b == *to) || (a == *to && b == *from))
    }

    /// #47：一次分析成功完成（基线前进到它的版本）。
    pub(crate) fn note_baseline(&mut self, document: DocumentId, version: Version) {
        self.baseline.insert(document, version);
    }

    /// #47：文档被编辑（新快照、相对上一版本的增量编辑）。返回要在后台运行的分析；正在分析时
    /// 记下编辑，等它完成后由 [`Self::resume`] 接着做。
    pub(crate) fn edited(
        &mut self,
        document: &Arc<Document>,
        edit: Option<TextEdit>,
    ) -> Option<SyntaxJob> {
        if !highlighted(document) {
            return None;
        }
        if self.running.contains_key(&document.id) {
            let entry = self
                .pending
                .entry(document.id)
                .or_insert_with(|| (document.clone(), Some(Vec::new())));
            entry.0 = document.clone();
            match (&mut entry.1, edit) {
                (Some(edits), Some(edit)) => edits.push(edit),
                (slot, _) => *slot = None,
            }
            return None;
        }
        Some(self.launch(document, edit, None))
    }

    /// #47：`finished` 版本的分析完成后，接着分析期间累积的编辑。
    pub(crate) fn resume(&mut self, document: DocumentId, finished: Version) -> Option<SyntaxJob> {
        let (latest, edits) = self.pending.remove(&document)?;
        let edit = edits
            .filter(|e| {
                e.first()
                    .is_some_and(|e| self.same_content(document, e.version, finished))
            })
            .and_then(|e| combine_text_edits(&e, &*latest.text));
        Some(self.launch(&latest, edit, None))
    }

    fn launch(
        &mut self,
        document: &Document,
        edit: Option<TextEdit>,
        visible: Option<LineRange>,
    ) -> SyntaxJob {
        if let Some(running) = self.running.get(&document.id) {
            running.cancel.cancel();
        }
        // 增量编辑以基线为准：基线与编辑的起点是同一内容的两个版本时改写成基线的版本。
        let edit = edit.map(|mut e| {
            if let Some(base) = self.baseline.get(&document.id).copied() {
                if self.same_content(document.id, e.version, base) {
                    e.version = base;
                }
            }
            e
        });
        self.serial += 1;
        let cancel = Cancellation::default();
        self.running.insert(
            document.id,
            Running {
                cancel: cancel.clone(),
                generation: self.serial,
                version: document.text.version(),
                partial: visible.is_some(),
            },
        );
        SyntaxJob {
            document: document.id,
            generation: self.serial,
            task: self.service.analyze(
                document.id,
                document.text.clone(),
                document.metadata.language.clone(),
                edit,
                visible,
                cancel,
            ),
        }
    }

    /// Start analysing `document` unless that version is already being analysed.
    /// `visible` requests a visible-first pass before the full one.
    pub(crate) fn start(
        &mut self,
        document: &Document,
        visible: Option<LineRange>,
    ) -> Option<SyntaxJob> {
        let version = document.text.version();
        if let Some(running) = self.running.get(&document.id) {
            if running.version == version {
                return None;
            }
            running.cancel.cancel();
        }
        self.serial += 1;
        let cancel = Cancellation::default();
        self.running.insert(
            document.id,
            Running {
                cancel: cancel.clone(),
                generation: self.serial,
                version,
                partial: visible.is_some(),
            },
        );
        Some(SyntaxJob {
            document: document.id,
            generation: self.serial,
            task: self.service.analyze(
                document.id,
                document.text.clone(),
                document.metadata.language.clone(),
                None,
                visible,
                cancel,
            ),
        })
    }

    /// Accept a finished job if it is still the document's current one. Returns whether
    /// a full pass must follow (the job was visible-first).
    pub(crate) fn finish(&mut self, document: DocumentId, generation: u64) -> Option<bool> {
        let running = self.running.get(&document)?;
        if running.generation != generation {
            return None;
        }
        let partial = running.partial;
        self.running.remove(&document);
        Some(partial)
    }

    /// The last tab of `document` closed: cancel its analysis and drop its baseline.
    pub(crate) fn release(&mut self, document: DocumentId) {
        self.pending.remove(&document);
        self.alias.remove(&document);
        self.baseline.remove(&document);
        if let Some(running) = self.running.remove(&document) {
            running.cancel.cancel();
        }
        self.service.close(document);
    }

    #[cfg(test)]
    pub(crate) fn is_running(&self, document: DocumentId) -> bool {
        self.running.contains_key(&document)
    }
}

/// #47：依次发生的若干替换合成一个相对第一个版本的替换（覆盖全部改动，`after` 是最后的文本）。
pub(crate) fn combine_text_edits(
    edits: &[TextEdit],
    after: &dyn rove_buffer::TextSource,
) -> Option<TextEdit> {
    let spans = edits.iter().map(|e| {
        let a = e.range.start().0 as i64;
        (a, e.range.end().0 as i64, a + e.inserted.len() as i64)
    });
    crate::code_view::edit::combine_spans(spans, edits.first()?.version, after)
}
