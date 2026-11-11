//! #18 integration: the quick input driven entirely from the keyboard in a real GPUI window,
//! plus a consumer example (an encoding picker, as #27 will build) showing the intended usage.
mod list_support;

use gpui::{div, prelude::*, Entity, Focusable, TestAppContext, VisualTestContext};
use rove_base::Version;
use rove_i18n::Message;
use rove_ui::{
    list_widget::{
        plain_row,
        quick_input::{QuickInputService, QuickPickConfig, QuickPickEvent},
        quick_input_view::{QuickInputView, QuickInputViewEvent},
        StaticList,
    },
    lists::{ItemId, QuickInput, QuickInputEvent, QuickInputOptions},
};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------- consumer example

/// How a feature builds a picker on the generic component: provide rows, groups and a title,
/// then react to `Accepted`. No list/keyboard code lives in the consumer.
struct EncodingPicker {
    quick: Entity<QuickInputView>,
    chosen: Arc<Mutex<Option<String>>>,
    _subscription: rove_base::Subscription,
}

impl EncodingPicker {
    fn new(quick: Entity<QuickInputView>, cx: &mut gpui::App) -> Self {
        let chosen = Arc::new(Mutex::new(None));
        let c = chosen.clone();
        let sub = quick
            .read(cx)
            .service
            .events()
            .subscribe(Arc::new(move |e| {
                if let QuickInputEvent::Accepted { items, .. } = e {
                    *c.lock().unwrap() = items.first().map(|i| i.0.to_string());
                }
            }));
        Self {
            quick,
            chosen,
            _subscription: sub,
        }
    }

    fn open(&self, window: &mut gpui::Window, cx: &mut gpui::App) {
        let rows: Vec<_> = [
            ("utf8", "UTF-8"),
            ("utf8bom", "UTF-8 with BOM"),
            ("gbk", "Simplified Chinese (GBK)"),
            ("gb18030", "Simplified Chinese (GB 18030)"),
            ("big5", "Traditional Chinese (Big5)"),
            ("shiftjis", "Japanese (Shift JIS)"),
        ]
        .iter()
        .map(|(id, label)| plain_row(id, label))
        .collect();
        self.quick.update(cx, |q, cx| {
            q.show(
                QuickInputOptions {
                    placeholder: Message::key("test.pick"),
                    initial: "".into(),
                    items: Arc::new(StaticList::new(rows, Version(0))),
                    multi_select: false,
                },
                QuickPickConfig {
                    title: Some(Message::key("test.pick")),
                    separators: vec![
                        (0, Some("Unicode".into())),
                        (2, Some("中文".into())),
                        (5, None),
                    ],
                    ..Default::default()
                },
                window,
                cx,
            )
            .unwrap();
        });
    }
}

struct Host {
    quick: Entity<QuickInputView>,
    closed: usize,
}
impl Render for Host {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(self.quick.clone())
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<Host>, &mut VisualTestContext) {
    cx.update(rove_ui::keys::init);
    let (theme, strings) = list_support::theme_and_strings();
    cx.add_window_view(|_, cx| {
        let quick =
            cx.new(|cx| QuickInputView::new(QuickInputService::default(), theme, strings, cx));
        cx.subscribe(&quick, |h: &mut Host, _, _: &QuickInputViewEvent, _| {
            h.closed += 1
        })
        .detach();
        Host { quick, closed: 0 }
    })
}

fn focused(host: &Entity<Host>, cx: &mut VisualTestContext) -> Option<String> {
    cx.read(|cx| {
        let q = host.read(cx).quick.read(cx);
        q.service
            .active()
            .and_then(|a| a.list.focused_id())
            .map(|i| i.0.to_string())
    })
}

#[gpui::test]
fn encoding_picker_consumer_example(cx: &mut TestAppContext) {
    let (host, cx) = open(cx);
    let quick = cx.read(|cx| host.read(cx).quick.clone());
    let picker = cx.update(|_, cx| EncodingPicker::new(quick, cx));
    cx.update(|window, cx| picker.open(window, cx));
    cx.run_until_parked();
    assert_eq!(focused(&host, cx).as_deref(), Some("utf8"));
    // Type to filter, then accept with Enter.
    cx.simulate_input("gb");
    assert_eq!(focused(&host, cx).as_deref(), Some("gbk"));
    cx.simulate_keystrokes("down enter");
    assert_eq!(picker.chosen.lock().unwrap().as_deref(), Some("gb18030"));
    assert_eq!(cx.read(|cx| host.read(cx).closed), 1);
}

#[gpui::test]
fn every_quick_input_operation_from_the_keyboard(cx: &mut TestAppContext) {
    let (host, cx) = open(cx);
    let quick = cx.read(|cx| host.read(cx).quick.clone());
    let log = Arc::new(Mutex::new(vec![]));
    let l = log.clone();
    let _sub = cx.read(|cx| {
        quick
            .read(cx)
            .service
            .pick_events()
            .subscribe(Arc::new(move |e| l.lock().unwrap().push(e)))
    });
    let labels: Vec<String> = (0..60).map(|i| format!("file{i:02}.cs")).collect();
    let rows: Vec<_> = labels.iter().map(|l| plain_row(l, l)).collect();
    cx.update(|window, cx| {
        quick.update(cx, |q, cx| {
            q.show(
                QuickInputOptions {
                    placeholder: Message::key("test.pick"),
                    initial: "".into(),
                    items: Arc::new(StaticList::new(rows, Version(0))),
                    multi_select: false,
                },
                QuickPickConfig {
                    separators: vec![(0, None), (20, Some("最近打开".into())), (40, None)],
                    loading_tail: Some(Message::key("test.scanning")),
                    ..Default::default()
                },
                window,
                cx,
            )
            .unwrap();
        })
    });
    cx.run_until_parked();
    // Navigate: next / previous (wrap) / page / first / last / groups.
    cx.simulate_keystrokes("up");
    assert_eq!(
        focused(&host, cx).as_deref(),
        Some("file59.cs"),
        "previous wraps"
    );
    cx.simulate_keystrokes("down");
    assert_eq!(focused(&host, cx).as_deref(), Some("file00.cs"));
    cx.simulate_keystrokes("pagedown");
    assert_eq!(
        focused(&host, cx).as_deref(),
        Some("file15.cs"),
        "last row of the 16-row viewport"
    );
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(focused(&host, cx).as_deref(), Some("file59.cs"));
    cx.simulate_keystrokes("ctrl-home ctrl-alt-down");
    assert_eq!(
        focused(&host, cx).as_deref(),
        Some("file20.cs"),
        "next group"
    );
    cx.simulate_keystrokes("alt-down");
    assert_eq!(focused(&host, cx).as_deref(), Some("file40.cs"));
    cx.simulate_keystrokes("alt-up");
    assert_eq!(
        focused(&host, cx).as_deref(),
        Some("file20.cs"),
        "previous group"
    );
    cx.simulate_keystrokes("ctrl-alt-up");
    assert_eq!(focused(&host, cx).as_deref(), Some("file00.cs"));
    // Typing narrows the list; right at the end of the box accepts in the background.
    cx.simulate_input("file3");
    assert_eq!(focused(&host, cx).as_deref(), Some("file03.cs"));
    cx.simulate_keystrokes("right");
    assert!(cx.read(|cx| host.read(cx).quick.read(cx).service.active().is_some()));
    cx.simulate_keystrokes("down");
    // Real keyboards report the held modifier before the key; accept reads it from the window.
    cx.simulate_modifiers_change(gpui::Modifiers::control());
    cx.simulate_keystrokes("ctrl-enter");
    cx.simulate_modifiers_change(gpui::Modifiers::default());
    assert!(cx.read(|cx| host.read(cx).quick.read(cx).service.active().is_none()));
    let log = log.lock().unwrap();
    let accepted: Vec<(String, bool, bool)> = log
        .iter()
        .filter_map(|e| match e {
            QuickPickEvent::Accepted {
                items,
                in_background,
                modifiers,
                ..
            } => Some((items[0].0.to_string(), *in_background, modifiers.ctrl)),
            _ => None,
        })
        .collect();
    assert_eq!(
        accepted,
        [
            ("file03.cs".to_string(), true, false),
            ("file13.cs".to_string(), false, true)
        ]
    );
}

#[gpui::test]
fn escape_hides_without_accepting_and_checkboxes_toggle(cx: &mut TestAppContext) {
    let (host, cx) = open(cx);
    let quick = cx.read(|cx| host.read(cx).quick.clone());
    let log = Arc::new(Mutex::new(vec![]));
    let l = log.clone();
    let _sub = cx.read(|cx| {
        quick
            .read(cx)
            .service
            .events()
            .subscribe(Arc::new(move |e| l.lock().unwrap().push(e)))
    });
    cx.update(|window, cx| {
        quick.update(cx, |q, cx| {
            q.show(
                QuickInputOptions {
                    placeholder: Message::key("test.pick"),
                    initial: "".into(),
                    items: Arc::new(StaticList::from_labels(["a", "b", "c"])),
                    multi_select: true,
                },
                QuickPickConfig::default(),
                window,
                cx,
            )
            .unwrap();
            // Checkboxes are toggled from the list (input not focused), as in VS Code.
            let f = q.focus_handle(cx);
            window.focus(&f);
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("space down down space ctrl-space");
    cx.read(|cx| {
        let a = host.read(cx).quick.read(cx).service.active().unwrap();
        assert_eq!(a.checked, [ItemId("a".into()), ItemId("c".into())]);
        assert!(a.hover_visible);
    });
    cx.simulate_keystrokes("escape");
    let log = log.lock().unwrap();
    assert!(matches!(log.last(), Some(QuickInputEvent::Cancelled(_))));
    assert!(!log
        .iter()
        .any(|e| matches!(e, QuickInputEvent::Accepted { .. })));
}
