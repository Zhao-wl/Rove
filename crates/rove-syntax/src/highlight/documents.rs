//! Syntax baselines keyed by [`DocumentId`] (#94).
//!
//! Versions are only ordered within one document, so each open document keeps its own
//! [`DocumentSyntax`]. Switching between documents (A → B → A) reuses each one's latest
//! snapshot, equal version numbers of different documents never meet, and closing a
//! document drops its baseline: a result that finishes after the close is discarded.

use super::{DocumentSyntax, TreeSitterSyntax};
use crate::{SyntaxService, SyntaxUpdate};
use rove_base::{Cancellation, DocumentId, Error, LineRange, Task, TextEdit};
use rove_buffer::TextSource;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Syntax analysis of open documents. Implementations are Send + Sync and run the returned
/// task on a background executor, like [`SyntaxService`]. This is the seam the editor holds,
/// so tests can substitute any service through [`Stateless`].
pub trait DocumentHighlighter: Send + Sync {
    /// Analyze `text` of `document`. `edit` is the single replacement that turned the
    /// previously analyzed version into `text`; without it (or when it does not match)
    /// the result is a full invalidation. `visible` asks for a partial, visible-first
    /// result when there is no baseline yet; analyzing the same text again completes it.
    fn analyze(
        &self,
        document: DocumentId,
        text: Arc<dyn TextSource>,
        language: Arc<str>,
        edit: Option<TextEdit>,
        visible: Option<LineRange>,
        cancel: Cancellation,
    ) -> Task<SyntaxUpdate>;

    /// Forget the document's baseline. Analyses still running for it return `Cancelled`.
    fn close(&self, document: DocumentId);
}

/// Adapts a stateless [`SyntaxService`] (for example `EmptySyntaxService` in tests):
/// every analysis is a full one and there is nothing to close.
pub struct Stateless(pub Arc<dyn SyntaxService>);

impl DocumentHighlighter for Stateless {
    fn analyze(
        &self,
        _: DocumentId,
        text: Arc<dyn TextSource>,
        language: Arc<str>,
        _: Option<TextEdit>,
        _: Option<LineRange>,
        cancel: Cancellation,
    ) -> Task<SyntaxUpdate> {
        self.0.analyze(text, language, cancel)
    }

    fn close(&self, _: DocumentId) {}
}

/// Tree-sitter baselines for every open document. Clones share the same baselines.
#[derive(Clone)]
pub struct SyntaxDocuments(Arc<Inner>);

struct Inner {
    syntax: TreeSitterSyntax,
    open: Mutex<HashMap<DocumentId, Entry>>,
    generation: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
struct Entry {
    syntax: DocumentSyntax,
    /// Distinguishes a reopened document from the one an older task started for.
    generation: u64,
}

impl SyntaxDocuments {
    pub fn new(syntax: TreeSitterSyntax) -> Self {
        Self(Arc::new(Inner {
            syntax,
            open: Mutex::default(),
            generation: Default::default(),
        }))
    }

    /// Number of documents with a baseline.
    pub fn len(&self) -> usize {
        self.0.open.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn entry(&self, document: DocumentId) -> Entry {
        let inner = &self.0;
        inner
            .open
            .lock()
            .unwrap()
            .entry(document)
            .or_insert_with(|| Entry {
                syntax: inner.syntax.document(),
                generation: inner
                    .generation
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            })
            .clone()
    }

    fn is_current(&self, document: DocumentId, generation: u64) -> bool {
        self.0
            .open
            .lock()
            .unwrap()
            .get(&document)
            .is_some_and(|entry| entry.generation == generation)
    }
}

impl DocumentHighlighter for SyntaxDocuments {
    fn analyze(
        &self,
        document: DocumentId,
        text: Arc<dyn TextSource>,
        language: Arc<str>,
        edit: Option<TextEdit>,
        visible: Option<LineRange>,
        cancel: Cancellation,
    ) -> Task<SyntaxUpdate> {
        let Entry { syntax, generation } = self.entry(document);
        let mut service = syntax;
        if let Some(edit) = edit {
            service = service.with_edit(edit);
        }
        if let Some(lines) = visible {
            service = service.with_visible(lines);
        }
        let documents = self.clone();
        Box::pin(async move {
            let update = service.analyze(text, language, cancel).await?;
            if !documents.is_current(document, generation) {
                return Err(Error::Cancelled);
            }
            Ok(update)
        })
    }

    fn close(&self, document: DocumentId) {
        let removed = self.0.open.lock().unwrap().remove(&document);

        drop(removed);
    }
}
