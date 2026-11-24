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
        assert!(view.basket_command(reg::COPY_MARKDOWN, cx))
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
