//! Cross-crate consumer examples: task ownership, errors, cancellation and registration seams.
use rove_base::*;
use rove_buffer::{Documents, EmptyDocuments, EmptyText};
use rove_commands::{Command, CommandExecutor, CommandRegistry, EmptyCommands, Invocation};
use rove_config::{DefaultSettings, Scope, Settings};
use rove_i18n::{chinese, tr, Message};
use rove_syntax::{EmptySyntaxService, SyntaxService};
use rove_ui::contributions::{Contribution, ContributionRegistry, Placement};
use std::{
    sync::Arc,
    task::{Context, Poll, Waker},
};

/// Fixed providers are immediately ready. This is not an executor for production tasks.
fn ready<T>(mut task: Task<T>) -> Result<T> {
    let waker = Waker::noop();
    match task.as_mut().poll(&mut Context::from_waker(waker)) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("fixed provider unexpectedly pending"),
    }
}
#[test]
fn async_consumers_never_mistake_unimplemented_work_for_success() {
    let resource = Resource::Virtual("fixture".into());
    let docs: Arc<dyn Documents> = Arc::new(EmptyDocuments);
    assert!(matches!(
        ready(docs.open(resource, Cancellation::default())),
        Err(Error::Unsupported)
    ));
    let syntax: Arc<dyn SyntaxService> = Arc::new(EmptySyntaxService);
    assert!(matches!(
        ready(syntax.analyze(
            Arc::new(EmptyText),
            "csharp".into(),
            Cancellation::default()
        )),
        Err(Error::Unsupported)
    ));
    let commands: &dyn CommandExecutor = &EmptyCommands;
    assert_eq!(
        ready(commands.execute(
            Invocation {
                id: "workbench.action.quickOpen".into(),
                args: Value::Null
            },
            Cancellation::default()
        )),
        Err(Error::NotFound)
    );
    let settings: &dyn Settings = &DefaultSettings(Registry::default());
    assert_eq!(
        ready(settings.write(
            "editor.fontSize".into(),
            Scope::default(),
            Value::Number(14.0),
            Cancellation::default()
        )),
        Err(Error::Unsupported)
    );
}
#[test]
fn cancellation_reaches_all_background_seams() {
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        ready(EmptyDocuments.open(Resource::Virtual("fixture".into()), cancel.clone())),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        ready(EmptySyntaxService.analyze(Arc::new(EmptyText), "csharp".into(), cancel.clone())),
        Err(Error::Cancelled)
    ));
    assert_eq!(
        ready(EmptyCommands.execute(
            Invocation {
                id: "test".into(),
                args: Value::Null
            },
            cancel.clone()
        )),
        Err(Error::Cancelled)
    );
    assert_eq!(
        ready(DefaultSettings(Registry::default()).write(
            "test".into(),
            Scope::default(),
            Value::Null,
            cancel
        )),
        Err(Error::Cancelled)
    );
}
#[test]
fn built_in_view_resolves_its_command_and_chinese_title() {
    let mut strings = chinese().unwrap();
    strings
        .register("commands.quickOpen", Arc::from("快速打开"))
        .unwrap();
    let mut commands = CommandRegistry::default();
    commands
        .register(
            "workbench.action.quickOpen",
            Command {
                title_key: "commands.quickOpen".into(),
                category_key: None,
            },
        )
        .unwrap();
    let mut views = ContributionRegistry::default();
    views
        .register(
            "rove.commandCenter",
            Contribution {
                title: Message::key("commands.quickOpen"),
                placement: Placement::EditorTitle,
                order: 0,
                icon: None,
                command: Some("workbench.action.quickOpen".into()),
                when: None,
                factory_key: "fixture.commandCenter".into(),
            },
        )
        .unwrap();
    let view = views.get("rove.commandCenter").unwrap();
    let command = commands.get(view.command.as_deref().unwrap()).unwrap();
    assert_eq!(
        tr!(&strings, &Message::key(command.title_key.clone())).unwrap(),
        "快速打开"
    );
    assert_eq!(tr!(&strings, &view.title).unwrap(), "快速打开");
}
#[test]
fn core_objects_can_cross_worker_boundaries() {
    fn shareable<T: Send + Sync>() {}
    shareable::<Arc<dyn rove_buffer::TextSource>>();
    shareable::<Arc<dyn rove_syntax::SyntaxSnapshot>>();
    shareable::<Arc<dyn rove_lang_cs::query::CsQueries>>();
    shareable::<Arc<dyn rove_workspace::Workspace>>();
    shareable::<Arc<dyn rove_config::Settings>>();
    shareable::<Arc<dyn rove_theme::Theme>>();
    shareable::<Arc<dyn Documents>>();
    shareable::<Arc<dyn SyntaxService>>();
    shareable::<Arc<dyn CommandExecutor>>();
    shareable::<Cancellation>();
    shareable::<Subscription>();
}

#[gpui::test]
fn document_caller_opens_a_position_and_closes_a_pending_tab(cx: &mut gpui::TestAppContext) {
    use rove_ui::{
        documents::{CloseOutcome, EditorTabState, OpenEvent, OpenOptions, OpenSource},
        skeleton::Workbench,
    };
    use std::{sync::Mutex, time::Instant};

    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("caller.cs");
    let pending_path = folder.path().join("pending.cs");
    std::fs::write(&path, "// first\n// 中文\n").unwrap();
    std::fs::write(&pending_path, "// pending\n").unwrap();
    let (view, cx) =
        cx.add_window_view(|_, cx| Workbench::new(folder.path().into(), Instant::now(), cx));
    let events = Arc::new(Mutex::new(Vec::new()));
    let position = Position {
        line: Line(1),
        column: Utf8Column(3),
    };
    let resource = Resource::File(path);
    let pending_resource = Resource::File(pending_path);
    let caller_token = Cancellation::default();
    let (request, _subscription) = cx.update(|_, app| {
        view.update(app, |view, cx| {
            let documents = view.open_documents();
            let captured = events.clone();
            let subscription = documents.events().subscribe(Arc::new(move |event| {
                captured.lock().unwrap().push(event);
            }));
            let request = documents
                .open(
                    resource.clone(),
                    OpenOptions {
                        location: Some(position),
                        source: OpenSource::GoToLine,
                        preserve_focus: true,
                        ..Default::default()
                    },
                    caller_token.clone(),
                )
                .unwrap();
            cx.notify();
            (request, subscription)
        })
    });
    cx.run_until_parked();
    let group = match events.lock().unwrap().as_slice() {
        [OpenEvent::Ready {
            request: completed,
            group,
            ..
        }] if *completed == request => *group,
        other => panic!("expected one Ready event, got {other:?}"),
    };
    let (pending, replaced) = cx.update(|_, app| {
        view.update(app, |view, cx| {
            let documents = view.open_documents();
            let tabs = documents.in_group(group);
            assert_eq!(tabs.len(), 1);
            assert!(tabs[0].active);
            assert!(!tabs[0].dirty);
            let EditorTabState::Ready(document) = &tabs[0].state else {
                panic!("expected a committed document, got {:?}", tabs[0]);
            };
            assert_eq!(document.id, documents.find(&resource).unwrap().id);
            assert_eq!(
                document
                    .text
                    .position(document.text.offset(position).unwrap())
                    .unwrap(),
                position
            );

            let pending = documents
                .open(
                    pending_resource.clone(),
                    OpenOptions::default(),
                    caller_token.clone(),
                )
                .unwrap();
            let loading = documents
                .in_group(group)
                .into_iter()
                .find(|tab| tab.resource == pending_resource)
                .unwrap();
            assert!(matches!(loading.state, EditorTabState::Loading));
            let replaced = documents
                .open(
                    pending_resource.clone(),
                    OpenOptions::default(),
                    caller_token.clone(),
                )
                .unwrap();
            let same_tab = documents
                .in_group(group)
                .into_iter()
                .find(|tab| tab.resource == pending_resource)
                .unwrap();
            assert_eq!(same_tab.id, loading.id);
            assert_eq!(
                documents.close_tab(same_tab.id).unwrap(),
                CloseOutcome::Closed
            );
            assert_eq!(
                documents.close_tab(same_tab.id).unwrap(),
                CloseOutcome::NotOpen
            );
            assert_eq!(caller_token.check(), Ok(()));
            cx.notify();
            (pending, replaced)
        })
    });
    cx.run_until_parked();
    assert!(matches!(
        events.lock().unwrap().as_slice(),
        [
            OpenEvent::Ready { .. },
            OpenEvent::Failed { request: a, group: a_group, error: Error::Cancelled },
            OpenEvent::Failed { request: b, group: b_group, error: Error::Cancelled },
        ] if (*a, *b, *a_group, *b_group) == (pending, replaced, group, group)
    ));
}
