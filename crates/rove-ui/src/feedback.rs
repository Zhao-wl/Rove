//! Five feedback levels from UX_SPEC §6.1; implemented by #16.
use rove_base::*;
use rove_i18n::Message;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub enum Severity {
    Info,
    Warning,
    Error,
}
#[derive(Clone, Debug)]
pub struct FeedbackAction {
    pub id: Arc<str>,
    pub label: Message,
}
/// Targets/levels are explicit; no anonymous toast for document or cursor-specific feedback.
#[derive(Clone, Debug)]
pub enum FeedbackLevel {
    Status {
        duration_ms: u32,
    },
    Cursor {
        document: DocumentId,
        position: Position,
    },
    EditorBar {
        document: DocumentId,
        dismissible: bool,
        suppress_key: Option<Arc<str>>,
    },
    Notification {
        severity: Severity,
    },
    Modal,
}
#[derive(Clone, Debug)]
pub struct Feedback {
    pub key: Arc<str>,
    pub level: FeedbackLevel,
    pub message: Message,
    pub actions: Arc<[FeedbackAction]>,
}
#[derive(Clone, Debug)]
pub enum FeedbackEvent {
    Action { key: Arc<str>, action: Arc<str> },
    Dismissed { key: Arc<str> },
}
/// UI-thread-only enqueue, O(log active feedback); never block a thread for a modal result.
/// key deduplicates same-cause notifications (update count). DND hides non-error notifications;
/// normal background progress uses status. Modal is reserved for data-loss/security confirmation.
/// Unsupported means nothing shown, never implicit acceptance. Events complete interactive choices.
pub trait FeedbackService {
    fn show(&mut self, feedback: Feedback) -> Result<()>;
    fn dismiss(&mut self, key: &str) -> Result<()>;
    fn events(&self) -> &dyn Events<FeedbackEvent>;
}
pub struct EmptyFeedback;
impl FeedbackService for EmptyFeedback {
    fn show(&mut self, _: Feedback) -> Result<()> {
        Err(Error::Unsupported)
    }
    fn dismiss(&mut self, _: &str) -> Result<()> {
        Err(Error::NotFound)
    }
    fn events(&self) -> &dyn Events<FeedbackEvent> {
        &NoEvents
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operation_reports_localized_status() {
        let mut svc = EmptyFeedback;
        let _sub = svc.events().subscribe(Arc::new(|_| {}));
        assert_eq!(
            svc.show(Feedback {
                key: "search.status".into(),
                level: FeedbackLevel::Status { duration_ms: 3000 },
                message: Message::key("common.loading"),
                actions: Arc::from([])
            }),
            Err(Error::Unsupported)
        );
    }
}
