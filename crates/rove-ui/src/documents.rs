//! UI-thread document presentation contract (#17). Disk/model loading is rove-buffer::Documents.
use rove_base::*;
use rove_buffer::Document;
use std::{fmt, sync::Arc};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EditorGroupId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenRequestId(pub u64);
#[derive(Clone, Debug)]
pub enum Presentation {
    Default,
    Source,
    Preview,
}
/// Resolves the group when the open is submitted; completion cannot retarget the request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenTarget {
    #[default]
    Active,
    /// Reuse the side group or create one if necessary.
    Beside,
    /// Target an existing group; a missing group fails with NotFound.
    Group(EditorGroupId),
}

/// The initiating UI surface, used for preview policy and navigation history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenSource {
    #[default]
    Other,
    Explorer,
    QuickOpen,
    Search,
    CodeNavigation,
    GoToLine,
    Outline,
    Breadcrumb,
}

#[derive(Clone, Debug)]
pub struct OpenOptions {
    pub target: OpenTarget,
    pub source: OpenSource,
    /// Allow an ephemeral tab, subject to settings for the originating surface.
    pub preview_tab: bool,
    /// Leave keyboard focus where it was when the request was submitted.
    pub preserve_focus: bool,
    pub presentation: Presentation,
    /// Zero-based UTF-8 position to reveal. An explicit position forces source presentation.
    pub location: Option<Position>,
    /// Half-open UTF-8 selection. Forces source presentation and reveals its start.
    /// When both fields are supplied, the selection takes precedence over location.
    pub selection: Option<TextRange>,
}
impl OpenOptions {
    /// The selection takes precedence, even when it is an empty range (a caret).
    pub fn reveal_position(&self) -> Option<Position> {
        self.selection
            .as_ref()
            .map(TextRange::start)
            .or(self.location)
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            target: OpenTarget::Active,
            source: OpenSource::Other,
            preview_tab: true,
            preserve_focus: false,
            presentation: Presentation::Default,
            location: None,
            selection: None,
        }
    }
}
/// Stable across reordering and moves between groups, until the tab is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EditorTabId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorTabKind {
    Text,
    Custom(Arc<str>),
}

/// What the tab can display. A tab with a committed view can remain Ready while a later
/// request runs; only OpenEvent reports whether that request completed.
#[derive(Clone)]
pub enum EditorTabState {
    Loading,
    Ready(Arc<Document>),
    Failed(Error),
}

impl fmt::Debug for EditorTabState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loading => f.write_str("Loading"),
            Self::Ready(document) => f.debug_tuple("Ready").field(&document.id).finish(),
            Self::Failed(error) => f.debug_tuple("Failed").field(error).finish(),
        }
    }
}

/// One entry per visible tab, including different presentations of the same document.
#[derive(Clone, Debug)]
pub struct EditorTabSnapshot {
    pub id: EditorTabId,
    pub resource: Resource,
    pub kind: EditorTabKind,
    pub preview: bool,
    pub pinned: bool,
    pub dirty: bool,
    /// Selected within this group, independent of which group has keyboard focus.
    pub active: bool,
    pub state: EditorTabState,
}

/// The group is the tab's group at completion, or its last group on cancellation.
/// A window that has been disposed drops its service and subscriptions.
#[derive(Clone, Debug)]
pub enum OpenEvent {
    Ready {
        request: OpenRequestId,
        group: EditorGroupId,
        document: DocumentId,
    },
    Failed {
        request: OpenRequestId,
        group: EditorGroupId,
        error: Error,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Closed,
    NeedsUserDecision,
    NotOpen,
}
/// UI-thread only. `open` returns after accepting the request. The installed service owns its
/// scheduling bridge: run disk/model work off the UI thread, then deliver results to the owning
/// window on the UI thread. Callers subscribe before opening and need no polling or host callbacks.
/// Implementations observe caller cancellation without cancelling the caller's scope.
/// While the service lives, each accepted request emits exactly one terminal event. Closing or
/// replacing a pending tab emits Failed(Cancelled); a stale worker emits nothing. Caller
/// cancellation removes a tab still Loading, or preserves its committed view if already Ready.
/// Ready is emitted only after reveal and selection have been applied.
/// Find is cached O(log n), close is O(groups+tabs). Closing dirty documents
/// returns NeedsUserDecision, never discards edits. Aliases share one model, possibly in several
/// groups. Cancelled/superseded opens cannot steal focus. Ready means location applied, including
/// deferred line indexing. Errors reported by return value or Failed event, never empty tabs.
pub trait OpenDocuments {
    fn open(
        &mut self,
        resource: Resource,
        options: OpenOptions,
        cancel: Cancellation,
    ) -> Result<OpenRequestId>;
    fn find(&self, resource: &Resource) -> Option<Arc<Document>>;
    fn close(&mut self, document: DocumentId, group: Option<EditorGroupId>)
        -> Result<CloseOutcome>;
    /// Closes a tab by its window-scoped ID, even if loading, failed, or moved to another group.
    /// A dirty tab reports NeedsUserDecision without changing its state.
    fn close_tab(&mut self, tab: EditorTabId) -> Result<CloseOutcome>;
    /// Returns every tab in display order. A nonexistent group returns an empty vector.
    fn in_group(&self, group: EditorGroupId) -> Vec<EditorTabSnapshot>;
    fn events(&self) -> &dyn Events<OpenEvent>;
}
pub struct EmptyOpenDocuments;
impl OpenDocuments for EmptyOpenDocuments {
    fn open(&mut self, _: Resource, _: OpenOptions, c: Cancellation) -> Result<OpenRequestId> {
        c.check()?;
        Err(Error::Unsupported)
    }
    fn find(&self, _: &Resource) -> Option<Arc<Document>> {
        None
    }
    fn close(&mut self, _: DocumentId, _: Option<EditorGroupId>) -> Result<CloseOutcome> {
        Ok(CloseOutcome::NotOpen)
    }
    fn close_tab(&mut self, _: EditorTabId) -> Result<CloseOutcome> {
        Ok(CloseOutcome::NotOpen)
    }
    fn in_group(&self, _: EditorGroupId) -> Vec<EditorTabSnapshot> {
        Vec::new()
    }
    fn events(&self) -> &dyn Events<OpenEvent> {
        &NoEvents
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_uses_open_service() {
        let mut svc = EmptyOpenDocuments;
        let _sub = svc.events().subscribe(Arc::new(|_| {}));
        let opts = OpenOptions {
            location: Some(Position {
                line: Line(12),
                column: Utf8Column(0),
            }),
            ..Default::default()
        };
        assert!(svc
            .open(
                Resource::Virtual("metadata:A".into()),
                opts,
                Cancellation::default()
            )
            .is_err());
        assert!(svc.in_group(EditorGroupId(1)).is_empty());
        assert_eq!(
            svc.close(DocumentId(1), None).unwrap(),
            CloseOutcome::NotOpen
        );
    }
    #[test]
    fn selection_anchor_takes_precedence_even_for_a_caret() {
        let location = Position {
            line: Line(50),
            column: Utf8Column(2),
        };
        let anchor = Position {
            line: Line(8),
            column: Utf8Column(4),
        };
        let mut options = OpenOptions {
            location: Some(location),
            ..Default::default()
        };
        assert_eq!(options.reveal_position(), Some(location));
        options.selection = Some(TextRange::new(anchor, anchor).unwrap());
        assert_eq!(options.reveal_position(), Some(anchor));
    }
}
