//! UI-thread document presentation contract (#17). Disk/model loading is rove-buffer::Documents.
use rove_base::*;
use rove_buffer::Document;
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorGroupId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenRequestId(pub u64);
#[derive(Clone, Debug)]
pub enum Presentation {
    Default,
    Source,
    Preview,
}
/// Explicit location forces source presentation (search/jump/reference), including Markdown.
#[derive(Clone, Debug)]
pub struct OpenOptions {
    pub group: Option<EditorGroupId>,
    pub preview_tab: bool,
    pub preserve_focus: bool,
    pub presentation: Presentation,
    pub location: Option<Position>,
}
impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            group: None,
            preview_tab: true,
            preserve_focus: false,
            presentation: Presentation::Default,
            location: None,
        }
    }
}
#[derive(Clone, Debug)]
pub enum OpenEvent {
    Ready {
        request: OpenRequestId,
        document: DocumentId,
    },
    Failed {
        request: OpenRequestId,
        error: Error,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Closed,
    NeedsUserDecision,
    NotOpen,
}
/// UI-thread only. open enqueues work and returns immediately; subscribe before opening to avoid
/// losing completion. Find is cached O(log n), close is O(groups+tabs). Closing dirty documents
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
        assert_eq!(
            svc.close(DocumentId(1), None).unwrap(),
            CloseOutcome::NotOpen
        );
    }
}
