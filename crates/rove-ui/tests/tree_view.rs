//! #18 integration: a lazily loaded tree in a real GPUI window — keyboard expand/collapse,
//! twistie clicks, asynchronous children and a 10,000-child expansion.
mod list_support;

use gpui::{point, px, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use rove_ui::{
    list_widget::{
        options::ListOptions,
        tree::{ChildCount, Children, TreeItem, TreeModel, TreeSource},
        view::ListView,
        widget::{ListWidget, RowHeight},
    },
    lists::ItemId,
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

/// Folder "big" has `big_children` files; folder "slow" answers asynchronously.
struct Source {
    big_children: usize,
    requests: Mutex<Vec<String>>,
}

fn item(id: String, label: String, children: ChildCount) -> TreeItem {
    TreeItem {
        id: ItemId(id.into()),
        label: label.into(),
        detail: None,
        children,
    }
}

impl TreeSource for Source {
    fn children(&self, parent: Option<&ItemId>) -> Children {
        let p = parent.map(|p| p.0.to_string());
        self.requests
            .lock()
            .unwrap()
            .push(p.clone().unwrap_or_default());
        match p.as_deref() {
            None => Children::Ready(Arc::from(vec![
                item(
                    "big".into(),
                    "big".into(),
                    ChildCount::Known(self.big_children),
                ),
                item("slow".into(), "slow".into(), ChildCount::Unknown),
                item("readme.md".into(), "readme.md".into(), ChildCount::None),
            ])),
            Some("big") => Children::Ready(
                (0..self.big_children)
                    .map(|i| item(format!("big/f{i}.cs"), format!("f{i}.cs"), ChildCount::None))
                    .collect(),
            ),
            Some("slow") => Children::Pending,
            _ => Children::Ready(Arc::from([])),
        }
    }
}

fn open(
    cx: &mut TestAppContext,
    big_children: usize,
) -> (Entity<ListView>, Arc<Source>, &mut VisualTestContext) {
    cx.update(rove_ui::keys::init);
    let (theme, strings) = list_support::theme_and_strings();
    let source = Arc::new(Source {
        big_children,
        requests: Mutex::new(vec![]),
    });
    let s = source.clone();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let widget = ListWidget::tree(
            TreeModel::new(s),
            RowHeight::Fixed(22.0),
            ListOptions::default(),
        );
        ListView::new(widget, theme, strings, cx)
    });
    cx.update(|window, cx| {
        let f = view.focus_handle(cx);
        window.focus(&f);
    });
    cx.run_until_parked();
    (view, source, cx)
}

fn labels(view: &Entity<ListView>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.read(|cx| {
        let w = &view.read(cx).widget;
        w.display_rows(0..w.len().min(8))
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), r.label))
            .collect()
    })
}

#[gpui::test]
fn keyboard_tree_navigation_and_lazy_loading(cx: &mut TestAppContext) {
    let (view, source, cx) = open(cx, 3);
    assert_eq!(
        *source.requests.lock().unwrap(),
        [""],
        "only roots requested"
    );
    cx.simulate_keystrokes("down right");
    assert_eq!(
        labels(&view, cx),
        ["big", "  f0.cs", "  f1.cs", "  f2.cs", "slow", "readme.md"]
    );
    cx.simulate_keystrokes("right");
    cx.read(|cx| assert_eq!(view.read(cx).widget.focus(), Some(1), "right again enters"));
    cx.simulate_keystrokes("left");
    cx.read(|cx| {
        assert_eq!(
            view.read(cx).widget.focus(),
            Some(0),
            "left on leaf => parent"
        )
    });
    cx.simulate_keystrokes("left");
    assert_eq!(labels(&view, cx), ["big", "slow", "readme.md"]);
    cx.simulate_keystrokes("space");
    assert_eq!(labels(&view, cx).len(), 6, "space toggles");
    cx.simulate_keystrokes("ctrl-left");
    assert_eq!(labels(&view, cx), ["big", "slow", "readme.md"]);

    // Asynchronous children: loading row, then delivered data.
    cx.simulate_keystrokes("down right");
    assert_eq!(labels(&view, cx), ["big", "slow", "  ", "readme.md"]);
    view.update(cx, |v, cx| {
        v.widget.update_tree(|t| {
            t.set_children(
                Some(&ItemId("slow".into())),
                Arc::from(vec![item(
                    "slow/a.cs".into(),
                    "a.cs".into(),
                    ChildCount::None,
                )]),
            )
        });
        cx.notify();
    });
    assert_eq!(labels(&view, cx), ["big", "slow", "  a.cs", "readme.md"]);
    cx.read(|cx| assert_eq!(view.read(cx).widget.focus(), Some(1), "focus kept by id"));
}

#[gpui::test]
fn twistie_click_and_expand_mode(cx: &mut TestAppContext) {
    let (view, _, cx) = open(cx, 2);
    // Clicking the label of a folder toggles it (expandMode = singleClick).
    cx.simulate_click(point(px(80.), px(11.)), Modifiers::default());
    assert_eq!(labels(&view, cx).len(), 5);
    view.update(cx, |v, cx| {
        v.set_options(
            ListOptions {
                expand_mode: rove_ui::list_widget::options::OpenMode::DoubleClick,
                ..Default::default()
            },
            cx,
        )
    });
    cx.simulate_click(point(px(80.), px(11.)), Modifiers::default());
    assert_eq!(
        labels(&view, cx).len(),
        5,
        "doubleClick mode: single click only selects"
    );
    cx.simulate_click(point(px(10.), px(11.)), Modifiers::default());
    assert_eq!(labels(&view, cx).len(), 3, "twistie always toggles");
}

#[gpui::test]
fn expanding_ten_thousand_children_is_fast(cx: &mut TestAppContext) {
    let (view, _, cx) = open(cx, 10_000);
    let start = Instant::now();
    view.update(cx, |v, cx| {
        v.widget.expand_id(&ItemId("big".into()), true).unwrap();
        cx.notify();
    });
    let elapsed = start.elapsed();
    cx.run_until_parked();
    cx.read(|cx| assert_eq!(view.read(cx).widget.len(), 10_003));
    eprintln!("TREE_EXPAND_10000_MS {:.3}", elapsed.as_secs_f64() * 1000.0);
    // The < 50 ms acceptance number is measured in release (examples/list_bench.rs); debug
    // builds only guard against accidental O(n²).
    assert!(elapsed.as_millis() < 1000, "{elapsed:?}");
    let rows = cx.read(|cx| view.read(cx).widget.visible_range().len());
    assert!(rows < 60, "only visible rows are built: {rows}");
}
