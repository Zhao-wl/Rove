//! #18 integration: a real GPUI window hosting `ListView`, driven only through key bindings and
//! mouse events. Covers the `listScrollAtBoundary` when-clause pass-through of ctrl+up/down.
mod list_support;

use gpui::{
    actions, div, point, prelude::*, px, Context, Entity, FocusHandle, Focusable, KeyBinding,
    Modifiers, TestAppContext, VisualTestContext, Window,
};
use rove_i18n::Message;
use rove_ui::list_widget::{
    keys::bind_default_keys,
    options::ListOptions,
    view::{EmptyState, ListView},
    widget::{ListEvent, ListWidget, RowHeight, ScrollBoundary},
    StaticList,
};
use std::sync::Arc;

actions!(test_host, [HostCtrlUp, HostCtrlDown]);

struct Host {
    list: Entity<ListView>,
    passed: Vec<&'static str>,
    events: Vec<ListEvent>,
    focus: FocusHandle,
}

impl Focusable for Host {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Host")
            .track_focus(&self.focus)
            .size_full()
            .on_action(cx.listener(|h, _: &HostCtrlUp, _, _| h.passed.push("ctrl-up")))
            .on_action(cx.listener(|h, _: &HostCtrlDown, _, _| h.passed.push("ctrl-down")))
            .child(div().w(px(300.)).h(px(220.)).child(self.list.clone()))
    }
}

fn open(cx: &mut TestAppContext, rows: usize) -> (Entity<Host>, &mut VisualTestContext) {
    cx.update(|cx| {
        bind_default_keys(cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-up", HostCtrlUp, Some("Host")),
            KeyBinding::new("ctrl-down", HostCtrlDown, Some("Host")),
        ]);
    });
    let (theme, strings) = list_support::theme_and_strings();
    let labels: Vec<String> = (0..rows).map(|i| format!("item{i}")).collect();
    let (host, cx) = cx.add_window_view(|_, cx| {
        let mut widget = ListWidget::new(
            Arc::new(StaticList::from_labels(labels.iter().map(|s| s.as_str()))),
            RowHeight::Fixed(22.0),
            ListOptions::default(),
        );
        widget.multi_select = true;
        let list = cx.new(|cx| ListView::new(widget, theme, strings, cx));
        cx.subscribe(&list, |h: &mut Host, _, e: &ListEvent, _| {
            h.events.push(e.clone())
        })
        .detach();
        Host {
            list,
            passed: vec![],
            events: vec![],
            focus: cx.focus_handle(),
        }
    });
    cx.update(|window, cx| {
        let f = host.read(cx).list.focus_handle(cx);
        window.focus(&f);
    });
    cx.run_until_parked();
    (host, cx)
}

fn with_list<R>(
    host: &Entity<Host>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&ListView) -> R,
) -> R {
    cx.read(|cx| f(host.read(cx).list.read(cx)))
}

#[gpui::test]
fn keyboard_navigation_and_scroll_boundary_pass_through(cx: &mut TestAppContext) {
    let (host, cx) = open(cx, 1000);
    with_list(&host, cx, |l| {
        assert_eq!(l.widget.viewport(), 220.0, "viewport measured from layout");
        assert!(
            l.widget.visible_range().len() <= 11,
            "only the viewport is laid out"
        );
    });
    cx.simulate_keystrokes("down down");
    with_list(&host, cx, |l| assert_eq!(l.widget.focus(), Some(1)));

    // At the top: ctrl+up is not the list's (listScrollAtBoundary == top) and reaches the host.
    cx.simulate_keystrokes("ctrl-up");
    cx.simulate_keystrokes("ctrl-down");
    let passed = cx.read(|cx| host.read(cx).passed.clone());
    assert_eq!(passed, ["ctrl-up"]);
    with_list(&host, cx, |l| assert_eq!(l.widget.scroll_top(), 10.0));

    // At the bottom: ctrl+down passes through, ctrl+up scrolls the list.
    cx.simulate_keystrokes("end");
    with_list(&host, cx, |l| {
        assert_eq!(l.widget.focus(), Some(999));
        assert_eq!(l.widget.scroll_boundary(), ScrollBoundary::Bottom);
    });
    cx.simulate_keystrokes("ctrl-down ctrl-up");
    let passed = cx.read(|cx| host.read(cx).passed.clone());
    assert_eq!(passed, ["ctrl-up", "ctrl-down"]);
    with_list(&host, cx, |l| {
        assert_eq!(l.widget.scroll_boundary(), ScrollBoundary::None)
    });

    cx.simulate_keystrokes("home pagedown");
    with_list(&host, cx, |l| assert_eq!(l.widget.focus(), Some(9)));
    cx.simulate_keystrokes("pagedown");
    with_list(&host, cx, |l| assert_eq!(l.widget.focus(), Some(19)));
    cx.simulate_keystrokes("pageup");
    with_list(&host, cx, |l| assert_eq!(l.widget.focus(), Some(10)));
}

#[gpui::test]
fn short_list_reports_both_and_passes_both_keys(cx: &mut TestAppContext) {
    let (host, cx) = open(cx, 3);
    with_list(&host, cx, |l| {
        assert_eq!(l.widget.scroll_boundary(), ScrollBoundary::Both)
    });
    cx.simulate_keystrokes("ctrl-down ctrl-up");
    let passed = cx.read(|cx| host.read(cx).passed.clone());
    assert_eq!(passed, ["ctrl-down", "ctrl-up"]);
}

#[gpui::test]
fn multiselect_select_find_and_type_navigation(cx: &mut TestAppContext) {
    let (host, cx) = open(cx, 50);
    cx.simulate_keystrokes("down shift-down shift-down");
    with_list(&host, cx, |l| {
        assert_eq!(l.widget.selection_indices(), [0, 1, 2])
    });
    cx.simulate_keystrokes("ctrl-shift-enter");
    with_list(&host, cx, |l| {
        assert_eq!(l.widget.selection_indices(), [0, 1])
    });
    cx.simulate_keystrokes("ctrl-a");
    with_list(&host, cx, |l| {
        assert_eq!(l.widget.selection_indices().len(), 50)
    });
    cx.simulate_keystrokes("escape");
    with_list(&host, cx, |l| {
        assert!(l.widget.selection_indices().is_empty())
    });
    cx.simulate_keystrokes("enter");
    let opened = cx.read(|cx| {
        host.read(cx)
            .events
            .iter()
            .any(|e| matches!(e, ListEvent::Open { pinned: true, .. }))
    });
    assert!(opened, "enter emits Open");

    // Type navigation (find closed).
    cx.simulate_input("item4");
    with_list(&host, cx, |l| assert_eq!(l.widget.focus(), Some(4)));

    // Find widget: ctrl+alt+f focuses its input; typing highlights; escape closes.
    cx.simulate_keystrokes("ctrl-alt-f");
    with_list(&host, cx, |l| assert!(l.widget.find_state().open));
    cx.simulate_input("m49");
    with_list(&host, cx, |l| {
        assert_eq!(l.widget.find_state().query, "m49");
        assert_eq!(l.widget.focus(), Some(49), "first match focused");
        assert_eq!(l.widget.highlights(49), vec![3..6]);
    });
    cx.simulate_keystrokes("escape");
    with_list(&host, cx, |l| assert!(!l.widget.find_state().open));
}

#[gpui::test]
fn mouse_click_wheel_and_empty_state(cx: &mut TestAppContext) {
    let (host, cx) = open(cx, 100);
    cx.simulate_click(point(px(50.), px(22. * 3. + 5.)), Modifiers::default());
    with_list(&host, cx, |l| {
        assert_eq!(
            (l.widget.focus(), l.widget.selection_indices()),
            (Some(3), &[3][..])
        )
    });
    let opened = cx.read(|cx| {
        host.read(cx).events.iter().any(|e| {
            matches!(
                e,
                ListEvent::Open {
                    preserve_focus: true,
                    ..
                }
            )
        })
    });
    assert!(
        opened,
        "single click opens a preview (openMode = singleClick)"
    );
    cx.simulate_click(point(px(50.), px(22. * 5. + 5.)), Modifiers::control());
    with_list(&host, cx, |l| {
        assert_eq!(l.widget.selection_indices(), [3, 5])
    });
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: point(px(50.), px(50.)),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-44.))),
        modifiers: Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    with_list(&host, cx, |l| assert_eq!(l.widget.scroll_top(), 44.0));

    // Empty state slot with an action button.
    let list = cx.read(|cx| host.read(cx).list.clone());
    list.update(cx, |l, cx| {
        l.widget.set_model(Arc::new(StaticList::from_labels([])));
        l.set_empty_state(Some(EmptyState {
            message: Message::key("test.empty"),
            actions: vec![(
                Message::key("test.open"),
                "workbench.action.files.openFolder".into(),
            )],
        }));
        cx.notify();
    });
    cx.run_until_parked();
    with_list(&host, cx, |l| assert!(l.widget.is_empty()));
}
