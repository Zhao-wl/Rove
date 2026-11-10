//! SKELETON(editors): single editor/open service, replaced by #17.
use super::events::EventBus;
use crate::documents::*;
use rove_base::*;
use rove_buffer::{Document, Documents};
use rove_syntax::{
    highlight::{DocumentSyntax, TreeSitterSyntax},
    SyntaxService, SyntaxSnapshot,
};
use std::sync::Arc;
pub struct Opened {
    pub document: Arc<Document>,
    pub syntax: Arc<dyn SyntaxSnapshot>,
    syntax_session: DocumentSyntax,
    pub position: Option<Position>,
}
pub struct Pending {
    pub id: OpenRequestId,
    pub task: Task<Opened>,
}
pub struct SingleEditor {
    pub current: Option<Opened>,
    pub loading: bool,
    loader: Arc<dyn Documents>,
    syntax: TreeSitterSyntax,
    pending: Option<Pending>,
    cancel: Cancellation,
    serial: u64,
    events: EventBus<OpenEvent>,
}
impl SingleEditor {
    pub fn new(loader: Arc<dyn Documents>, syntax: TreeSitterSyntax) -> Self {
        Self {
            current: None,
            loading: false,
            loader,
            syntax,
            pending: None,
            cancel: Default::default(),
            serial: 0,
            events: EventBus::empty(),
        }
    }
    /// The host acknowledges after applying the requested location to its viewport.
    pub fn ready(&self, id: OpenRequestId) {
        if id.0 == self.serial {
            if let Some(opened) = &self.current {
                self.events.emit(OpenEvent::Ready {
                    request: id,
                    document: opened.document.id,
                });
            }
        }
    }
    pub fn take_pending(&mut self) -> Option<Pending> {
        self.pending.take()
    }
    pub fn complete(&mut self, id: OpenRequestId, result: Result<Opened>) -> Result<bool> {
        if id.0 != self.serial {
            return Ok(false);
        }
        self.loading = false;
        let result = result.and_then(|opened| {
            self.cancel.check()?;
            Ok(opened)
        });
        match result {
            Ok(opened) => {
                self.current = Some(opened);
                Ok(true)
            }
            Err(error) => {
                self.current = None;
                self.events.emit(OpenEvent::Failed {
                    request: id,
                    error: error.clone(),
                });
                Err(error)
            }
        }
    }
}
impl OpenDocuments for SingleEditor {
    fn open(
        &mut self,
        r: Resource,
        options: OpenOptions,
        cancel: Cancellation,
    ) -> Result<OpenRequestId> {
        cancel.check()?;
        if options.group.is_some() || matches!(options.presentation, Presentation::Preview) {
            return Err(Error::Unsupported);
        }
        self.cancel.cancel();
        self.cancel = cancel.clone();
        self.serial += 1;
        self.loading = true;
        let id = OpenRequestId(self.serial);
        let task = self.loader.open(r, cancel.clone());
        let syntax = self.syntax.clone();
        let active = self
            .current
            .as_ref()
            .map(|opened| (opened.document.id, opened.syntax_session.clone()));
        self.pending = Some(Pending {
            id,
            task: Box::pin(async move {
                let document = task.await?;
                cancel.check()?;
                if let Some(p) = options.location {
                    document.text.offset(p)?;
                }
                let session = active
                    .filter(|(id, _)| *id == document.id)
                    .map_or_else(|| syntax.document(), |(_, session)| session);
                let update = session
                    .analyze(
                        document.text.clone(),
                        document.metadata.language.clone(),
                        cancel.clone(),
                    )
                    .await?;
                cancel.check()?;
                Ok(Opened {
                    document,
                    syntax: update.snapshot,
                    syntax_session: session,
                    position: options.location,
                })
            }),
        });
        Ok(id)
    }
    fn find(&self, r: &Resource) -> Option<Arc<Document>> {
        self.loader.find(r)
    }
    fn close(&mut self, id: DocumentId, _: Option<EditorGroupId>) -> Result<CloseOutcome> {
        if self.current.as_ref().is_some_and(|c| c.document.id == id) {
            self.cancel.cancel();
            self.serial += 1;
            self.loading = false;
            self.pending = None;
            self.current = None;
            Ok(CloseOutcome::Closed)
        } else {
            Ok(CloseOutcome::NotOpen)
        }
    }
    fn events(&self) -> &dyn Events<OpenEvent> {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::text::{default_settings, FileDocuments};
    use std::sync::Mutex;

    #[gpui::test]
    async fn stale_completion_and_failed_file_do_not_replace_latest_document() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.cs"), "public class A {}\n").unwrap();
        std::fs::write(root.path().join("b.cs"), "public class B {}\n").unwrap();
        std::fs::write(root.path().join("bad.cs"), [0xff]).unwrap();
        let loader = Arc::new(FileDocuments::new(root.path().into(), default_settings()));
        let mut editor = SingleEditor::new(loader.clone(), TreeSitterSyntax::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let received = events.clone();
        let subscription = editor
            .events()
            .subscribe(Arc::new(move |event| received.lock().unwrap().push(event)));
        let resource = |name: &str| Resource::Workspace(WorkspacePath::new(name).unwrap());
        editor
            .open(
                resource("a.cs"),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        let old = editor.take_pending().unwrap();
        let old_result = old.task.await;
        editor
            .open(
                resource("b.cs"),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        let latest = editor.take_pending().unwrap();
        let result = latest.task.await;
        assert!(editor.complete(latest.id, result).unwrap());
        assert!(
            events.lock().unwrap().is_empty(),
            "Ready waits for host location application"
        );
        editor.ready(latest.id);
        assert!(!editor.complete(old.id, old_result).unwrap());
        assert_eq!(
            editor.current.as_ref().unwrap().document.resource,
            resource("b.cs")
        );
        assert!(
            matches!(events.lock().unwrap().as_slice(), [OpenEvent::Ready { request, .. }] if *request == latest.id)
        );
        let alias = Resource::File(root.path().join("b.cs"));
        let alias_doc = loader
            .open(alias.clone(), Cancellation::default())
            .await
            .unwrap();
        assert_eq!(loader.find(&alias).unwrap().id, alias_doc.id);
        let previous = editor.current.as_ref().unwrap().syntax.clone();
        editor
            .open(alias, OpenOptions::default(), Cancellation::default())
            .unwrap();
        let reopen = editor.take_pending().unwrap();
        assert!(editor.complete(reopen.id, reopen.task.await).unwrap());
        assert!(Arc::ptr_eq(
            &previous,
            &editor.current.as_ref().unwrap().syntax
        ));
        editor
            .open(
                resource("bad.cs"),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        let bad = editor.take_pending().unwrap();
        assert_eq!(
            editor.complete(bad.id, bad.task.await),
            Err(Error::Unsupported)
        );
        assert!(!editor.loading);
        assert!(editor.current.is_none());
        drop(subscription);
        editor
            .open(
                resource("a.cs"),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        let recovery = editor.take_pending().unwrap();
        assert!(editor.complete(recovery.id, recovery.task.await).unwrap());
        assert!(!Arc::ptr_eq(
            &previous,
            &editor.current.as_ref().unwrap().syntax
        ));
        editor.ready(recovery.id);
        assert_eq!(
            events.lock().unwrap().len(),
            2,
            "dropped subscriber is detached"
        );
    }
}
