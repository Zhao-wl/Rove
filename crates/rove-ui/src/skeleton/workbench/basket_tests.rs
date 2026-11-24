use super::{LayoutStore, Workbench};
use crate::{
    code_view::Selection,
    context_basket::{model::LineRange, registry as reg},
    documents::OpenOptions,
};
use gpui::{Focusable, TestAppContext};
use rove_base::{Cancellation, Line, Position, Resource, Utf8Column, WorkspacePath};
use rove_config::{Settings, SettingsOptions, SettingsService};
use std::{sync::Arc, time::Instant};

#[gpui::test]
fn context_basket_editor_collects_two_ranges_and_copies_after_verification(
    cx: &mut TestAppContext,
) {
    let root = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("notes.txt"), "one\nignore\nignore\nfour\n").unwrap();
    let service: Arc<dyn Settings> = Arc::new(SettingsService::open(
        SettingsOptions::new(user.path(), user.path().join("backups")),
        crate::app_settings::registry().unwrap(),
    ));
    let (workbench, cx) = cx.add_window_view(|_, cx| {
        Workbench::with_settings(
            root.path().into(),
            Instant::now(),
            LayoutStore::disabled(),
            service,
            cx,
        )
        .unwrap()
    });
    cx.run_until_parked();
    workbench.update(cx, |view, _| {
        view.basket.use_test_state();
        view.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new("notes.txt").unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
    });
    cx.run_until_parked();
    let code = workbench.read_with(cx, |view, _| view.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    code.update(cx, |view, cx| {
        let selected_line = |line: u64| {
            Selection::new(
                Position {
                    line: Line(line),
                    column: Utf8Column(0),
                },
                Position {
                    line: Line(line),
                    column: Utf8Column(2),
                },
            )
        };
        view.set_selections(vec![selected_line(0), selected_line(3)], None, cx);
    });
    let original_sidebar =
        workbench.read_with(cx, |view, _| view.shell.layout.active_container.clone());
    cx.simulate_keystrokes("ctrl-k b");
    cx.run_until_parked();
    workbench.read_with(cx, |view, _| {
        let items = view.basket.contents.items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].lines, LineRange::new(1, 1).unwrap());
        assert_eq!(items[1].lines, LineRange::new(4, 4).unwrap());
        assert_eq!(view.shell.layout.active_container, original_sidebar);
        assert_eq!(
            view.status(),
            view.label("contextBasket.added", &[("count", "2".into())])
        );
    });
    workbench.update(cx, |view, cx| {
        assert!(view.basket_command(reg::ADD, cx));
        assert!(!view.basket.pending_anchors.is_empty());
        assert!(view.basket_command(reg::COPY_MARKDOWN, cx));
    });
    cx.run_until_parked();
    let clipboard = cx
        .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .unwrap();
    assert!(clipboard.contains("notes.txt:1-1"));
    assert!(clipboard.contains("notes.txt:4-4"));
    assert!(clipboard.contains("\none\n"));
    assert!(clipboard.contains("\nfour\n"));
    assert!(!clipboard.contains("ignore"));
}

#[gpui::test]
fn context_basket_ignores_superseded_copy_completions(cx: &mut TestAppContext) {
    use crate::context_basket::export::Export;

    let root = tempfile::tempdir().unwrap();
    let (view, cx) =
        cx.add_window_view(|_, cx| Workbench::new(root.path().into(), Instant::now(), cx));
    cx.run_until_parked();
    view.update(cx, |view, cx| {
        view.basket.use_test_state();
        let revision = view.basket.revision;
        let result = |text: &str| Export {
            text: text.to_owned(),
            entries: 1,
            ..Export::default()
        };
        assert!(view.basket_command(reg::COPY_MARKDOWN, cx));
        assert!(view.basket_command(reg::COPY_REFERENCES, cx));
        view.finish_basket_copy(2, revision, false, result("latest references"), cx);
        view.finish_basket_copy(1, revision, false, result("old markdown"), cx);
        assert!(view.basket_command(reg::CLEAR, cx));
        view.finish_basket_copy(2, revision, false, result("cleared contents"), cx);
    });
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("latest references".to_owned())
    );
}

#[gpui::test]
fn context_basket_syncs_same_workspace_windows_and_keeps_other_roots_independent(
    cx: &mut TestAppContext,
) {
    use super::basket_host::BasketTestStateRoot;
    use crate::context_basket::{model::Entry, persistence::BasketStore};

    let root = tempfile::tempdir().unwrap();
    let other_root = tempfile::tempdir().unwrap();
    let app_data = tempfile::tempdir().unwrap();
    cx.update(|app| app.set_global(BasketTestStateRoot(app_data.path().into())));
    let first = cx
        .add_window_view(|_, cx| Workbench::new(root.path().into(), Instant::now(), cx))
        .0;
    let second = cx
        .add_window_view(|_, cx| Workbench::new(root.path().into(), Instant::now(), cx))
        .0;
    let unrelated = cx
        .add_window_view(|_, cx| Workbench::new(other_root.path().into(), Instant::now(), cx))
        .0;
    cx.run_until_parked();

    let insert = |view: &gpui::Entity<Workbench>, name: &str, cx: &mut TestAppContext| {
        cx.update(|app| {
            view.update(app, |view, cx| {
                assert!(view.basket.loaded);
                view.basket
                    .contents
                    .insert(Entry::new(
                        WorkspacePath::new(name).unwrap(),
                        LineRange::new(1, 1).unwrap(),
                        None,
                    ))
                    .unwrap();
                view.basket_changed(cx);
            })
        });
        cx.run_until_parked();
    };
    let names = |view: &gpui::Entity<Workbench>, cx: &TestAppContext| {
        cx.read(|app| {
            view.read(app)
                .basket
                .contents
                .items()
                .iter()
                .map(|entry| entry.path.as_str().to_owned())
                .collect::<Vec<_>>()
        })
    };
    insert(&first, "one.cs", cx);
    assert_eq!(names(&second, cx), ["one.cs"]);
    insert(&second, "two.cs", cx);
    assert_eq!(names(&first, cx), ["one.cs", "two.cs"]);
    assert!(names(&unrelated, cx).is_empty());
    let store = BasketStore::with_base(Some(app_data.path().into()), root.path());
    assert_eq!(store.load().items().len(), 2);

    // Deliver two operations before either workbench receives the observer notification.
    cx.update(|app| {
        first.update(app, |view, cx| {
            view.basket.contents.introduction = "updated".into();
            assert!(view.basket_changed(cx));
        });
        second.update(app, |view, cx| {
            assert!(view.basket_command(reg::CLEAR, cx));
            assert_eq!(
                view.status(),
                view.label("contextBasket.changedInOtherWindow", &[])
            );
        });
    });
    assert_eq!(names(&second, cx), ["one.cs", "two.cs"]);

    cx.update(|app| {
        first.update(app, |view, cx| assert!(view.basket_command(reg::CLEAR, cx)));
    });
    cx.run_until_parked();
    assert!(names(&second, cx).is_empty());
    assert!(store.load().items().is_empty());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(other_root.path()).unwrap().count(), 0);
}

#[gpui::test]
fn context_basket_can_copy_from_another_window_while_anchors_are_prepared(cx: &mut TestAppContext) {
    let fixture = start_shared_capture(cx);
    cx.update(|app| {
        fixture.second.update(app, |view, cx| {
            assert_eq!(view.basket.contents.items().len(), 1);
            assert!(view.basket.contents.items()[0].anchor.is_none());
            view.basket_workspace_files_changed(
                &[super::FileEvent::Changed(
                    WorkspacePath::new("example.txt").unwrap(),
                )],
                cx,
            );
            assert!(view.basket.pending_anchors.is_empty());
            assert!(view.basket_command(reg::COPY_MARKDOWN, cx));
        });
    });
    assert_shared_capture_copies(cx);
}

#[gpui::test]
fn context_basket_commits_pending_anchor_after_the_source_window_closes(cx: &mut TestAppContext) {
    let SharedCaptureFixture {
        root,
        state,
        first,
        first_window,
        second,
    } = start_shared_capture(cx);
    let source = first.downgrade();
    drop(first);
    let mut first_view = gpui::VisualTestContext::from_window(first_window, cx);
    first_view.update(|window, _| window.remove_window());
    assert!(source.upgrade().is_none());
    cx.update(|app| {
        second.update(app, |view, cx| {
            assert!(view.basket.contents.items()[0].anchor.is_none());
            assert!(view.basket_command(reg::COPY_MARKDOWN, cx));
        });
    });
    assert_shared_capture_copies(cx);
    let stored = crate::context_basket::persistence::BasketStore::with_base(
        Some(state.path().into()),
        root.path(),
    )
    .load();
    assert!(stored.items()[0].anchor.is_some());
}

struct SharedCaptureFixture {
    root: tempfile::TempDir,
    state: tempfile::TempDir,
    first: gpui::Entity<Workbench>,
    first_window: gpui::AnyWindowHandle,
    second: gpui::Entity<Workbench>,
}

fn start_shared_capture(cx: &mut TestAppContext) -> SharedCaptureFixture {
    use super::basket_host::BasketTestStateRoot;
    use crate::context_basket::model::Entry;

    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let content = "first\nselected\nlast\n";
    std::fs::write(root.path().join("example.txt"), content).unwrap();
    cx.update(|app| app.set_global(BasketTestStateRoot(state.path().into())));
    let (first, first_cx) =
        cx.add_window_view(|_, cx| Workbench::new(root.path().into(), Instant::now(), cx));
    let first_window = first_cx.update(|window, _| window.window_handle());
    let second = cx
        .add_window_view(|_, cx| Workbench::new(root.path().into(), Instant::now(), cx))
        .0;
    cx.run_until_parked();
    cx.update(|app| {
        first.update(app, |view, cx| {
            let range = LineRange::new(2, 2).unwrap();
            let id = view
                .basket
                .contents
                .insert(Entry::new(
                    WorkspacePath::new("example.txt").unwrap(),
                    range,
                    None,
                ))
                .unwrap();
            assert!(view.basket_changed(cx));
            view.capture_basket_anchors(
                vec![(id, range)],
                Arc::new(rove_buffer::RopeText::from_text(content)),
                cx,
            );
        });
    });
    SharedCaptureFixture {
        root,
        state,
        first,
        first_window,
        second,
    }
}

fn assert_shared_capture_copies(cx: &mut TestAppContext) {
    cx.run_until_parked();
    let copied = cx
        .update(|app| app.read_from_clipboard().and_then(|item| item.text()))
        .expect("the pending copy completes when the other window publishes the anchor");
    assert!(copied.contains("example.txt:2-2"));
    assert!(copied.contains("\nselected\n"));
}

pub(super) fn click_basket_menu(cx: &mut gpui::VisualTestContext, index: usize) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
    let selector = [
        "context-basket-source-action-0",
        "context-basket-source-action-1",
    ][index];
    let bounds = cx
        .debug_bounds(selector)
        .expect("basket menu action is visible");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}
