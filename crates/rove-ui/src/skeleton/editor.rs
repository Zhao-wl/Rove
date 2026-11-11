//! SKELETON(editors): single editor/open service, replaced by #17.
use super::events::EventBus;
use crate::documents::*;
use rove_base::*;
use rove_buffer::{Document, Documents};
use rove_syntax::{SyntaxService, SyntaxSnapshot};
use std::{rc::Rc, sync::Arc};
pub struct Opened {
    pub document: Arc<Document>,
    pub syntax: Arc<dyn SyntaxSnapshot>,
    pub position: Option<Position>,
}
pub struct Pending {
    pub id: OpenRequestId,
    pub resource: Resource,
    pub task: Task<Opened>,
}
const GROUP: EditorGroupId = EditorGroupId(0);

pub struct SingleEditor {
    pub current: Option<Opened>,
    pub loading: bool,
    loader: Arc<dyn Documents>,
    syntax: Arc<dyn SyntaxService>,
    schedule: Rc<dyn Fn(Pending)>,
    cancel: Cancellation,
    serial: u64,
    next_tab: u64,
    active_request: Option<OpenRequestId>,
    tab: Option<EditorTabSnapshot>,
    events: EventBus<OpenEvent>,
}
impl SingleEditor {
    pub fn new(
        loader: Arc<dyn Documents>,
        syntax: Arc<dyn SyntaxService>,
        schedule: impl Fn(Pending) + 'static,
    ) -> Self {
        Self {
            current: None,
            loading: false,
            loader,
            syntax,
            schedule: Rc::new(schedule),
            cancel: Default::default(),
            serial: 0,
            next_tab: 0,
            active_request: None,
            tab: None,
            events: EventBus::empty(),
        }
    }
    /// Commit the snapshot after the host applies the requested viewport.
    pub fn ready(&mut self, id: OpenRequestId) {
        if self.active_request != Some(id) || self.loading {
            return;
        }
        if let (Some(tab), Some(opened)) = (&mut self.tab, &self.current) {
            tab.state = EditorTabState::Ready(opened.document.clone());
            self.active_request = None;
            self.events.emit(OpenEvent::Ready {
                request: id,
                group: GROUP,
                document: opened.document.id,
            });
        }
    }

    fn cancel_pending(&mut self) {
        // The skeleton can invalidate a request immediately. The installed worker
        // observes caller cancellation without mutating that caller's token.
        if let Some(request) = self.active_request.take() {
            self.events.emit(OpenEvent::Failed {
                request,
                group: GROUP,
                error: Error::Cancelled,
            });
        }
    }

    pub fn complete(&mut self, id: OpenRequestId, result: Result<Opened>) -> Result<bool> {
        if self.active_request != Some(id) || !self.loading {
            return Ok(false);
        }
        let Some(tab) = self.tab.as_mut() else {
            return Ok(false);
        };
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
                self.active_request = None;
                if !matches!(tab.state, EditorTabState::Ready(_)) {
                    self.current = None;
                    if error == Error::Cancelled {
                        self.tab = None;
                    } else {
                        tab.state = EditorTabState::Failed(error.clone());
                    }
                }
                self.events.emit(OpenEvent::Failed {
                    request: id,
                    group: GROUP,
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
        match options.target {
            OpenTarget::Active | OpenTarget::Group(GROUP) => {}
            OpenTarget::Group(_) => return Err(Error::NotFound),
            OpenTarget::Beside => return Err(Error::Unsupported),
        }
        if options.selection.is_some()
            || (options.reveal_position().is_none()
                && matches!(options.presentation, Presentation::Preview))
        {
            return Err(Error::Unsupported);
        }
        self.cancel_pending();
        self.cancel = cancel.clone();
        self.serial += 1;
        self.loading = true;
        let id = OpenRequestId(self.serial);
        self.active_request = Some(id);
        if let Some(tab) = self.tab.as_mut().filter(|tab| tab.resource == r) {
            if !matches!(tab.state, EditorTabState::Ready(_)) {
                tab.state = EditorTabState::Loading;
            }
        } else {
            self.current = None;
            self.next_tab += 1;
            self.tab = Some(EditorTabSnapshot {
                id: EditorTabId(self.next_tab),
                resource: r.clone(),
                kind: EditorTabKind::Text,
                preview: false,
                pinned: false,
                dirty: false,
                active: true,
                state: EditorTabState::Loading,
            });
        }
        let task = self.loader.open(r.clone(), cancel.clone());
        let syntax = self.syntax.clone();
        (self.schedule)(Pending {
            id,
            resource: r,
            task: Box::pin(async move {
                let document = task.await?;
                cancel.check()?;
                if let Some(p) = options.location {
                    document.text.offset(p)?;
                }
                let update = syntax
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
                    position: options.location,
                })
            }),
        });
        Ok(id)
    }
    fn find(&self, r: &Resource) -> Option<Arc<Document>> {
        self.loader.find(r)
    }
    fn close(&mut self, id: DocumentId, group: Option<EditorGroupId>) -> Result<CloseOutcome> {
        if group.is_some_and(|group| group != GROUP) {
            return Ok(CloseOutcome::NotOpen);
        }
        let Some(tab) = self.tab.as_ref().filter(
            |tab| matches!(&tab.state, EditorTabState::Ready(document) if document.id == id),
        ) else {
            return Ok(CloseOutcome::NotOpen);
        };
        self.close_tab(tab.id)
    }
    fn close_tab(&mut self, id: EditorTabId) -> Result<CloseOutcome> {
        let Some(tab) = self.tab.as_ref().filter(|tab| tab.id == id) else {
            return Ok(CloseOutcome::NotOpen);
        };
        if tab.dirty {
            return Ok(CloseOutcome::NeedsUserDecision);
        }
        self.tab = None;
        self.loading = false;
        self.current = None;
        self.cancel_pending();
        Ok(CloseOutcome::Closed)
    }
    fn in_group(&self, group: EditorGroupId) -> Vec<EditorTabSnapshot> {
        if group != GROUP {
            return Vec::new();
        }
        self.tab.iter().cloned().collect()
    }
    fn events(&self) -> &dyn Events<OpenEvent> {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::{
        syntax::CSharp,
        text::{default_settings, FileDocuments},
    };
    use std::{cell::RefCell, collections::VecDeque, sync::Mutex};

    #[gpui::test]
    async fn stale_completion_and_failed_file_do_not_replace_latest_document() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.cs"), "public class A {}\n").unwrap();
        std::fs::write(root.path().join("b.cs"), "public class B {}\n").unwrap();
        let loader = Arc::new(FileDocuments::new(root.path().into(), default_settings()));
        let jobs = Rc::new(RefCell::new(VecDeque::new()));
        let queue = jobs.clone();
        let mut editor = SingleEditor::new(loader.clone(), Arc::new(CSharp), move |job| {
            queue.borrow_mut().push_back(job);
        });
        for (options, expected) in [
            (
                OpenOptions {
                    target: OpenTarget::Beside,
                    ..Default::default()
                },
                Error::Unsupported,
            ),
            (
                OpenOptions {
                    target: OpenTarget::Group(EditorGroupId(99)),
                    ..Default::default()
                },
                Error::NotFound,
            ),
            (
                OpenOptions {
                    selection: Some(
                        TextRange::new(Position::default(), Position::default()).unwrap(),
                    ),
                    ..Default::default()
                },
                Error::Unsupported,
            ),
            (
                OpenOptions {
                    presentation: Presentation::Preview,
                    ..Default::default()
                },
                Error::Unsupported,
            ),
        ] {
            assert_eq!(
                editor.open(
                    Resource::Virtual("unused".into()),
                    options,
                    Cancellation::default()
                ),
                Err(expected)
            );
        }
        assert!(jobs.borrow().is_empty());
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
        assert!(matches!(
            editor.in_group(GROUP)[0].state,
            EditorTabState::Loading
        ));
        assert!(editor.in_group(EditorGroupId(999)).is_empty());
        let old = jobs.borrow_mut().pop_front().unwrap();
        let old_result = old.task.await;
        editor
            .open(
                resource("b.cs"),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        let latest = jobs.borrow_mut().pop_front().unwrap();
        let result = latest.task.await;
        assert!(editor.complete(latest.id, result).unwrap());
        assert!(
            events.lock().unwrap().len() == 1,
            "Ready waits for host location application"
        );
        assert!(matches!(
            editor.in_group(GROUP)[0].state,
            EditorTabState::Loading
        ));
        editor.ready(latest.id);
        assert!(!editor.complete(old.id, old_result).unwrap());
        assert_eq!(
            editor.current.as_ref().unwrap().document.resource,
            resource("b.cs")
        );
        assert!(
            matches!(events.lock().unwrap().as_slice(), [OpenEvent::Failed { request: previous, group: GROUP, error: Error::Cancelled }, OpenEvent::Ready { request, group: GROUP, .. }] if *previous == old.id && *request == latest.id)
        );
        assert!(matches!(
            editor.in_group(GROUP)[0].state,
            EditorTabState::Ready(_)
        ));
        let alias = Resource::File(root.path().join("b.cs"));
        let alias_doc = loader
            .open(alias.clone(), Cancellation::default())
            .await
            .unwrap();
        assert_eq!(loader.find(&alias).unwrap().id, alias_doc.id);
        editor
            .open(
                resource("bad.cs"),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        let bad = jobs.borrow_mut().pop_front().unwrap();
        assert!(matches!(
            editor.complete(bad.id, bad.task.await),
            Err(Error::Io {
                kind: std::io::ErrorKind::NotFound,
                ..
            })
        ));
        assert!(!editor.loading);
        assert!(editor.current.is_none());
        assert!(matches!(
            editor.in_group(GROUP)[0].state,
            EditorTabState::Failed(Error::Io {
                kind: std::io::ErrorKind::NotFound,
                ..
            })
        ));
        let failed = editor.in_group(GROUP)[0].id;
        assert_eq!(editor.close_tab(failed).unwrap(), CloseOutcome::Closed);
        assert!(editor.in_group(GROUP).is_empty());
        drop(subscription);
        editor
            .open(
                resource("a.cs"),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        let recovery = jobs.borrow_mut().pop_front().unwrap();
        assert!(editor.complete(recovery.id, recovery.task.await).unwrap());
        editor.ready(recovery.id);
        assert_eq!(
            events.lock().unwrap().len(),
            3,
            "dropped subscriber is detached"
        );
    }
}
