//! #18 performance evidence (AGENTS.md §8.1), measured the same way as M0_UI_SPIKE_REPORT §2.5:
//!
//! - frame interval: wall time between consecutive presented frames while scrolling continuously;
//! - per-frame CPU: list view render → layout → prepaint → paint (no vsync/present wait);
//! - tree expansion: expanding a folder with 10,000 children (lazy load + flatten + relayout).
//!
//! Run (release, real window; the GPU/vsync path cannot run on CI):
//!   cargo run -p rove-ui --release --example list_bench -- [frames] [step_px]
//! Output keys: SCROLL_FRAMES, SCROLL_INTERVAL_MS, SCROLL_CPU_US, TREE_EXPAND_10000_MS.
use gpui::{
    div, prelude::*, px, size, App, Application, Bounds, Context, Entity, Window, WindowBounds,
    WindowOptions,
};
use rove_i18n::{chinese, Strings};
use rove_theme::{ColorRegistry, DefaultTheme, Theme, ThemeKind};
use rove_ui::{
    list_widget::{
        keys::bind_default_keys,
        options::ListOptions,
        plain_row, registry,
        tree::{ChildCount, Children, TreeItem, TreeModel, TreeSource},
        view::{FrameProbe, ListView},
        widget::{ListWidget, RowHeight},
        StaticList,
    },
    lists::ItemId,
};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};

struct BigFolder(usize);
impl TreeSource for BigFolder {
    fn children(&self, parent: Option<&ItemId>) -> Children {
        Children::Ready(match parent {
            None => Arc::from(vec![TreeItem {
                id: ItemId("big".into()),
                label: "big".into(),
                detail: None,
                children: ChildCount::Known(self.0),
            }]),
            Some(_) => (0..self.0)
                .map(|i| TreeItem {
                    id: ItemId(format!("big/File{i:05}.cs").into()),
                    label: format!("File{i:05}.cs").into(),
                    detail: None,
                    children: ChildCount::None,
                })
                .collect(),
        })
    }
}

fn stats(mut v: Vec<f64>) -> (f64, f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let avg = v.iter().sum::<f64>() / v.len() as f64;
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p99 = v[((v.len() as f64 * 0.99).ceil() as usize)
        .saturating_sub(1)
        .min(v.len() - 1)];
    (avg, p99, *v.last().unwrap())
}

fn tree_expand_ms() -> f64 {
    let mut samples = vec![];
    for _ in 0..20 {
        let mut w = ListWidget::tree(
            TreeModel::new(Arc::new(BigFolder(10_000))),
            RowHeight::Fixed(22.0),
            ListOptions::default(),
        );
        w.set_viewport(800.0, 300.0);
        let start = Instant::now();
        w.expand_id(&ItemId("big".into()), true).unwrap();
        let rows = w.display_rows(w.visible_range());
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(w.len(), 10_001);
        assert!(!rows.is_empty());
    }
    stats(samples).1
}

struct Bench {
    list: Entity<ListView>,
    probe: Rc<RefCell<FrameProbe>>,
    presented: Rc<RefCell<Vec<Instant>>>,
    frame: usize,
    frames: usize,
    step: f32,
    warmup: usize,
}

impl Render for Bench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.frame < self.frames + self.warmup {
            let step = self.step;
            self.list.update(cx, |l, _| {
                let before = l.widget.scroll_top();
                l.widget.set_scroll_top(before + step);
                if l.widget.scroll_top() == before {
                    l.widget.set_scroll_top(0.0);
                }
            });
            if self.frame == self.warmup {
                self.probe.borrow_mut().cpu_us.clear();
                self.presented.borrow_mut().clear();
            }
            let presented = self.presented.clone();
            window.on_next_frame(move |_, _| presented.borrow_mut().push(Instant::now()));
            self.frame += 1;
            window.request_animation_frame();
        } else {
            let p = self.presented.borrow();
            let intervals: Vec<f64> = p
                .windows(2)
                .map(|w| (w[1] - w[0]).as_secs_f64() * 1000.0)
                .collect();
            let (ia, ip, im) = stats(intervals.clone());
            let (ca, cp, cm) = stats(self.probe.borrow().cpu_us.clone());
            println!("SCROLL_FRAMES {}", intervals.len());
            println!("SCROLL_INTERVAL_MS avg={ia:.2} p99={ip:.2} max={im:.2}");
            println!("SCROLL_CPU_US avg={ca:.0} p99={cp:.0} max={cm:.0}");
            println!("SCALE_FACTOR {}", window.scale_factor());
            cx.quit();
        }
        div().size_full().child(self.list.clone())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let frames: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(600);
    let step: f32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(66.0);
    println!("TREE_EXPAND_10000_MS p99={:.3}", tree_expand_ms());
    Application::new().run(move |cx: &mut App| {
        bind_default_keys(cx);
        let mut colors = ColorRegistry::default();
        registry::register_colors(&mut colors).unwrap();
        let mut table = chinese().unwrap();
        registry::register_strings(&mut table).unwrap();
        let theme: Arc<dyn Theme> = Arc::new(DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        });
        let strings: Arc<dyn Strings> = Arc::new(table);
        let rows: Vec<_> = (0..100_000)
            .map(|i| {
                plain_row(
                    &format!("item{i}"),
                    &format!("Assets/Scripts/Module{}/File{i:06}.cs", i % 97),
                )
            })
            .collect();
        let bounds = Bounds::centered(None, size(px(420.0), px(900.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_, cx| {
                let probe = Rc::new(RefCell::new(FrameProbe::default()));
                let p = probe.clone();
                let list = cx.new(move |cx| {
                    let mut widget = ListWidget::new(
                        Arc::new(StaticList::new(rows, rove_base::Version(0))),
                        RowHeight::Fixed(22.0),
                        ListOptions::default(),
                    );
                    widget.set_find_query("");
                    widget.close_find();
                    widget.set_focus(Some(0));
                    let mut v = ListView::new(widget, theme, strings, cx);
                    v.frame_probe = Some(p);
                    v
                });
                cx.new(|_| Bench {
                    list,
                    probe,
                    presented: Rc::new(RefCell::new(vec![])),
                    frame: 0,
                    frames,
                    step,
                    warmup: 30,
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
