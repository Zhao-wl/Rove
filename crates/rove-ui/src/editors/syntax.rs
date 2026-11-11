//! Syntax highlighting of opened documents (#94).
//!
//! Small documents are highlighted before their tab becomes ready, so they never show
//! uncoloured text. Larger ones become readable first; highlighting then runs in the
//! background, visible lines first, and each result is applied only to tabs that still
//! show that document at that version. Releasing the last tab of a document cancels its
//! running analysis and drops its baseline in the [`DocumentHighlighter`].

use rove_base::{Cancellation, DocumentId, Line, LineRange, Span, Task, Version};
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
