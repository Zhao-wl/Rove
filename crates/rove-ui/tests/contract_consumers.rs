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
