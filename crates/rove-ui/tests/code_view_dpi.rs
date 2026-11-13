//! #30：高 DPI 与多显示器的回归保护（缩放参数化）。
//!
//! 运行：`cargo test -p rove-ui --test code_view_dpi`
//!
//! 在 GPUI 测试窗口里把窗口缩放系数依次设为 100% / 125% / 150% / 200%（`VisualTestContext::
//! simulate_scale_factor_change`，见 `vendor/gpui/ROVE_PATCHES.md` 的 R30 条目），断言：
//!
//! 1. 布局尺寸（行高、字号、行号栏、视口行数）是逻辑像素量，不随缩放变化；
//! 2. 「像素坐标 → 行列」在四种缩放下都精确往返，含 CJK 全角、Tab 制表位、emoji（含 ZWJ 序列）；
//! 3. 一次缩放变更事件（跨显示器拖动）**当场**清空整形缓存，重新整形后的坐标与直接在新缩放下打开
//!    的结果完全一致——也就是不会复用旧缩放下的字形尺寸；
//! 4. 跨屏来回拖动后滚动位置与选区不变；
//! 5. 字体缩放（`editor.fontZoom*`、`editor.fontSize`、`window.zoomLevel` 的缩放系数）与系统缩放
//!    叠加时行高与命中测试仍然正确。
//!
//! 截图类检查（文字清晰度、插入符、浮层、图标、Markdown 预览）与真正的双显示器混合 DPI 必须人工
//! 执行，清单见 `docs/DPI_CHECKLIST.md`。

use gpui::{
    point, px, Context, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, Point, TestAppContext, VisualTestContext, Window,
};
use rove_base::*;
use rove_buffer::{RopeText, TextSource};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings, SettingsChange};
use rove_theme::{ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{
    self, columns::DisplayLine, selection::pos, CodeView, CodeViewServices, Reveal, Selection,
    Viewport,
};
use std::sync::Arc;

/// 验收要求的四档系统缩放。
const SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

// ------------------------------------------------------------------ 夹具

struct Overrides(DefaultSettings, Vec<(&'static str, Value)>);
impl Settings for Overrides {
    fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
        match self.1.iter().find(|(k, _)| *k == key) {
            Some((_, v)) => Ok(v.clone()),
            None => self.0.read(key, scope),
        }
    }
    fn write(&self, k: Arc<str>, s: Scope, v: Value, c: Cancellation) -> Task<()> {
        self.0.write(k, s, v, c)
    }
    fn changes(&self) -> &dyn Events<SettingsChange> {
        self.0.changes()
    }
}

fn services(overrides: Vec<(&'static str, Value)>) -> CodeViewServices {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    CodeViewServices {
        settings: Arc::new(Overrides(DefaultSettings(settings), overrides)),
        theme: Arc::new(DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        }),
        theme_kind: ThemeKind::Dark,
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    overrides: Vec<(&'static str, Value)>,
) -> (Entity<CodeView>, &'a mut VisualTestContext) {
    let source: Arc<dyn TextSource> = Arc::new(RopeText::from_text(text));
    let s = services(overrides);
    let (view, cx) = cx.add_window_view(move |_: &mut Window, cx: &mut Context<CodeView>| {
        let mut v = CodeView::new(s, cx);
        v.set_text(Some(source), false, cx);
        v
    });
    cx.update(|window, cx| {
        rove_ui::keys::init(cx);
        window.focus(&view.read(cx).focus_handle(cx));
    });
    settle(cx);
    (view, cx)
}

/// 让空闲准备切片全部跑完并重绘。
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

/// 把窗口搬到另一块缩放比不同的显示器上（Windows 的 `WM_DPICHANGED`）。
fn move_to_display(cx: &mut VisualTestContext, scale: f32) {
    cx.simulate_scale_factor_change(scale);
    settle(cx);
    assert_eq!(cx.update(|window, _| window.scale_factor()), scale);
}

fn point_of(view: &Entity<CodeView>, cx: &mut VisualTestContext, p: Position) -> Point<Pixels> {
    view.update(cx, |v, _| v.point_for_position(p))
        .expect("position visible")
}

/// 行内某位置垂直居中的点（命中测试用）。
fn mid(view: &Entity<CodeView>, cx: &mut VisualTestContext, p: Position) -> Point<Pixels> {
    let lh = view.read_with(cx, |v, _| v.line_height());
    let at = point_of(view, cx, p);
    point(at.x, at.y + px(lh / 2.0))
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
}

/// 覆盖验收矩阵要求的混排：ASCII、CJK 全角、emoji（含 ZWJ 与肤色修饰）、Tab 缩进、宽窄混排。
fn mixed_patterns() -> Vec<String> {
    vec![
        "// ASCII only: fn main() { let x = 42; }".into(),
        "// 中文注释：这是一个用于测试 CJK 字形渲染与字体回退的示例行。".into(),
        "// Mixed 混合 text with emoji 🚀🔥✅ and symbols ±≈→∞ ok".into(),
        "let emoji_table = [\"😀\", \"🇨🇳\", \"👨‍👩‍👧‍👦\", \"🧑🏽‍💻\"]; // ZWJ + skin tone".into(),
        "\tpublic void Foo(int bar) { /* tab indented, 宽窄混排 */ }".into(),
    ]
}

fn sample_text() -> String {
    let mut lines = mixed_patterns();
    for i in 0..200 {
        lines.push(format!("line {i:04} 内容 content\ttail"));
    }
    lines.join("\n") + "\n"
}

/// 每一行每一个字素簇边界的 x（逻辑像素，相对视口左上角）。
fn boundary_xs(view: &Entity<CodeView>, cx: &mut VisualTestContext, lines: &[String]) -> Vec<f32> {
    let mut out = Vec::new();
    for (line, src) in lines.iter().enumerate() {
        let display = DisplayLine::build(src, 4, None);
        for stop in display.stops() {
            if stop.col > 120 {
                break; // 超出视口宽度的部分不在可见区。
            }
            out.push(f32::from(
                point_of(view, cx, pos(line as u64, stop.src as u64)).x,
            ));
        }
    }
    out
}

/// 逐个字素簇边界断言「坐标 → 行列」精确往返（误差 0 个字符，强于验收要求的 ≤ 1 个字符）。
fn assert_hit_testing_round_trips(
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
    lines: &[String],
    what: &str,
) {
    for (line, src) in lines.iter().enumerate() {
        let line = line as u64;
        let display = DisplayLine::build(src, 4, None);
        let mut last_x = f32::MIN;
        for stop in display.stops() {
            if stop.col > 120 {
                break;
            }
            let p = pos(line, stop.src as u64);
            let at = mid(view, cx, p);
            assert!(
                f32::from(at.x) >= last_x,
                "{what}：x 必须单调 {src:?} @ {}",
                stop.src
            );
            last_x = at.x.into();
            assert_eq!(
                view.update(cx, |v, _| v.position_for_point(at)),
                Some(p),
                "{what}：{src:?} @ 字节 {}",
                stop.src
            );
        }
        // 簇内部（Tab 中段、ZWJ 序列内部）的点吸附到最近的边界，不会落进簇里。
        for w in display.stops().windows(2) {
            if w[1].col > 120 {
                break;
            }
            let a = mid(view, cx, pos(line, w[0].src as u64));
            let b = mid(view, cx, pos(line, w[1].src as u64));
            for (f, expect) in [(0.25f32, w[0].src), (0.75, w[1].src)] {
                let x = a.x + (b.x - a.x) * f;
                let hit = view
                    .update(cx, |v, _| v.position_for_point(point(x, a.y)))
                    .unwrap();
                assert_eq!(
                    hit.column.0 as u32, expect,
                    "{what}：{src:?} 段 {}..{}",
                    w[0].src, w[1].src
                );
            }
        }
    }
}

// ------------------------------------------------------------------ 1. 布局尺寸与缩放无关

/// GPUI 的几何量是逻辑像素：四档缩放下行高、字号、行号栏宽度、视口行数与每个字素簇边界的 x
/// 必须逐一相同。任何一项随缩放漂移，都意味着某处把物理像素当成了逻辑像素。
#[gpui::test]
fn layout_is_identical_at_every_scale(cx: &mut TestAppContext) {
    let patterns = mixed_patterns();
    let (view, cx) = open(cx, &sample_text(), vec![]);

    let mut baseline: Option<(f32, f32, Viewport, Vec<f32>)> = None;
    for scale in SCALES {
        move_to_display(cx, scale);
        let (lh, fs, viewport) = view.read_with(cx, |v, _| {
            (v.line_height(), v.font_size(), v.viewport().unwrap())
        });
        let xs = boundary_xs(&view, cx, &patterns);
        match &baseline {
            None => baseline = Some((lh, fs, viewport, xs)),
            Some((blh, bfs, bvp, bxs)) => {
                assert_eq!(lh, *blh, "{scale}×：行高变了");
                assert_eq!(fs, *bfs, "{scale}×：字号变了");
                assert_eq!(
                    (viewport.first_line, viewport.last_line),
                    (bvp.first_line, bvp.last_line),
                    "{scale}×：可见行范围变了"
                );
                assert_eq!(viewport.height, bvp.height, "{scale}×：视口高度变了");
                assert_eq!(xs.len(), bxs.len(), "{scale}×：可见边界数量变了");
                for (i, (a, b)) in xs.iter().zip(bxs).enumerate() {
                    assert!(
                        (a - b).abs() < 0.01,
                        "{scale}×：第 {i} 个字素簇边界 {a} ≠ {b}"
                    );
                }
            }
        }
    }
}

// ------------------------------------------------------------------ 2. 命中测试

/// 四档缩放下逐个字素簇边界往返（CJK 全角、Tab 制表位、emoji / ZWJ / 肤色修饰）。
#[gpui::test]
fn hit_testing_round_trips_at_every_scale(cx: &mut TestAppContext) {
    let patterns = mixed_patterns();
    let (view, cx) = open(cx, &sample_text(), vec![]);
    for scale in SCALES {
        move_to_display(cx, scale);
        assert_hit_testing_round_trips(&view, cx, &patterns, &format!("{scale}×"));
        // Tab 展开到制表位：tab 行的 'p' 落在第 4 列，和普通行第 4 个字符同一个 x。
        let tab = mid(&view, cx, pos(4, 0));
        let p = mid(&view, cx, pos(4, 1));
        let a = mid(&view, cx, pos(0, 0));
        let four = mid(&view, cx, pos(0, 4));
        assert_eq!(p.x - tab.x, four.x - a.x, "{scale}×：Tab 制表位");
    }
}

/// 鼠标点击（走完整的事件链路，而不是只调换算函数）在各缩放下落到同一个字符上。
#[gpui::test]
fn clicking_lands_on_the_same_character_at_every_scale(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, &sample_text(), vec![]);
    let targets = [pos(1, 9), pos(2, 30), pos(3, 20), pos(4, 8)];
    for scale in SCALES {
        move_to_display(cx, scale);
        for p in targets {
            let at = mid(&view, cx, p);
            click(cx, at);
            let got = view.read_with(cx, |v, _| v.selection().active);
            assert_eq!(got, p, "{scale}×：点击 {p:?} 落到了 {got:?}");
        }
    }
}

// ------------------------------------------------------------------ 3. 缩放变更事件与整形缓存

/// 跨显示器拖动：缩放变更事件到达时整形缓存当场清空（键里带字体代次，代次覆盖缩放），
/// 重新整形后的几何与「一开始就在这块屏上打开」完全一致。
#[gpui::test]
fn scale_change_clears_the_shaping_cache(cx: &mut TestAppContext) {
    let patterns = mixed_patterns();
    let (view, cx) = open(cx, &sample_text(), vec![]);
    move_to_display(cx, 1.0);
    assert!(
        view.read_with(cx, |v, _| v.shape_cache_stats().entries) > 0,
        "稳定后应当有整形结果"
    );

    // 事件到达，还没有重绘：缓存必须已经空了（旧缩放的字形尺寸不会被复用）。
    cx.simulate_scale_factor_change(1.5);
    assert_eq!(
        view.read_with(cx, |v, _| v.shape_cache_stats().entries),
        0,
        "缩放变更事件应当当场清空整形缓存"
    );

    settle(cx);
    assert!(
        view.read_with(cx, |v, _| v.shape_cache_stats().entries) > 0,
        "重新整形后缓存应当被重新填上"
    );
    let dragged = boundary_xs(&view, cx, &patterns);
    assert_hit_testing_round_trips(&view, cx, &patterns, "拖到 150% 之后");

    // 再拖回 100%：又一次当场清空，且几何回到拖走之前的值（整形结果与缩放无关的那部分不漂移）。
    cx.simulate_scale_factor_change(1.0);
    assert_eq!(
        view.read_with(cx, |v, _| v.shape_cache_stats().entries),
        0,
        "拖回去也应当当场清空"
    );
    settle(cx);
    let back = boundary_xs(&view, cx, &patterns);
    assert_eq!(dragged.len(), back.len());
    for (i, (a, b)) in dragged.iter().zip(&back).enumerate() {
        assert!(
            (a - b).abs() < 0.01,
            "第 {i} 个边界：150% {a} ≠ 回到 100% {b}"
        );
    }
}

// ------------------------------------------------------------------ 4. 跨屏来回拖动

/// 100% ↔ 150% 来回拖动后：滚动位置、选区、命中测试都不变。
#[gpui::test]
fn scroll_and_selection_survive_mixed_dpi_drags(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, &sample_text(), vec![]);
    move_to_display(cx, 1.0);
    view.update(cx, |v, cx| {
        v.reveal_line(120, Reveal::Center, cx);
        v.set_selection(Selection::new(pos(120, 2), pos(122, 7)), None, cx);
    });
    settle(cx);
    let (top, left, selection, viewport) = view.read_with(cx, |v, _| {
        (
            v.scroll_position().top,
            v.scroll_position().x,
            v.selection(),
            v.viewport().unwrap(),
        )
    });
    let anchor = pos(120, 5);
    let at_100 = mid(&view, cx, anchor);

    for scale in [1.5f32, 1.0, 2.0, 1.25, 1.0] {
        move_to_display(cx, scale);
        let s = view.read_with(cx, |v, _| v.scroll_position());
        assert_eq!(s.top, top, "{scale}×：滚动位置变了");
        assert_eq!(s.x, left, "{scale}×：水平滚动变了");
        assert_eq!(
            view.read_with(cx, |v, _| v.selection()),
            selection,
            "{scale}×：选区变了"
        );
        let vp = view.read_with(cx, |v, _| v.viewport().unwrap());
        assert_eq!(
            (vp.first_line, vp.last_line),
            (viewport.first_line, viewport.last_line),
            "{scale}×：可见行范围变了"
        );
        // 同一个逻辑坐标仍然命中同一个字符。
        assert_eq!(
            view.update(cx, |v, _| v.position_for_point(at_100)),
            Some(anchor),
            "{scale}×：命中测试漂移"
        );
    }
}

// ------------------------------------------------------------------ 5. 字体缩放与系统缩放叠加

/// `editor.fontZoomIn` / `Out` / `Reset`、`editor.fontSize` 与窗口缩放（`window.zoomLevel` 的
/// 缩放系数经 `set_font_scale` 传进来）叠加系统缩放后，行高与命中测试仍然正确。
#[gpui::test]
fn font_zoom_combines_with_system_scale(cx: &mut TestAppContext) {
    let patterns = mixed_patterns();
    let (view, cx) = open(
        cx,
        &sample_text(),
        vec![("editor.fontSize", Value::Number(18.0))],
    );
    for scale in SCALES {
        move_to_display(cx, scale);
        let base = view.read_with(cx, |v, _| (v.font_size(), v.line_height()));
        assert!(
            (base.0 - 18.0).abs() < 0.01,
            "{scale}×：基准字号应当是设置值"
        );

        for (id, steps) in [
            ("editor.action.fontZoomIn", 3i32),
            ("editor.action.fontZoomOut", -2),
        ] {
            for _ in 0..steps.unsigned_abs() {
                cx.update(|window, cx| {
                    view.update(cx, |v, cx| {
                        assert!(v.execute(id, window, cx), "命令 {id} 应当被处理");
                    })
                });
            }
        }
        settle(cx);
        let zoomed = view.read_with(cx, |v, _| (v.font_size(), v.line_height(), v.zoom_level()));
        assert_eq!(zoomed.2, 1, "缩放级别应当是 +3 -2 = +1");
        assert!(zoomed.0 > base.0, "{scale}×：放大后字号应当变大");
        assert!(zoomed.1 > base.1, "{scale}×：放大后行高应当变大");
        // 叠加后命中测试依然精确。
        assert_hit_testing_round_trips(&view, cx, &patterns, &format!("{scale}× + 字体缩放"));

        cx.update(|window, cx| {
            view.update(cx, |v, cx| {
                assert!(v.execute("editor.action.fontZoomReset", window, cx));
            })
        });
        settle(cx);
        let reset = view.read_with(cx, |v, _| (v.font_size(), v.line_height(), v.zoom_level()));
        assert_eq!(reset.2, 0);
        assert!(
            (reset.0 - base.0).abs() < 0.01,
            "{scale}×：重置后字号应当回到基准"
        );
        assert!(
            (reset.1 - base.1).abs() < 0.01,
            "{scale}×：重置后行高应当回到基准"
        );
    }
}

// ------------------------------------------------------------------ 6. 物理像素对齐

/// 细线与插入符的几何：四档缩放下都落在物理像素网格上，宽度恰好是整数个物理像素。
/// （绘制出来的像素要靠人工看截图，这里锁住换算本身，见 `docs/DPI_CHECKLIST.md`。）
#[gpui::test]
fn hairlines_and_caret_land_on_whole_device_pixels(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, &sample_text(), vec![]);
    for scale in SCALES {
        move_to_display(cx, scale);
        // 缩进参考线：1 物理像素宽，x 对齐到物理像素网格。
        let guide = rove_ui::dpi::vline(
            37.3,
            11.4,
            view.read_with(cx, |v, _| v.line_height()),
            scale,
        );
        let w = f32::from(guide.size.width) * scale;
        assert!((w - 1.0).abs() < 1e-3, "{scale}×：参考线宽 {w} 物理像素");
        let x = f32::from(guide.origin.x) * scale;
        assert!((x - x.round()).abs() < 1e-3, "{scale}×：参考线 x 未对齐");

        // 插入符：竖线宽度取整数个物理像素，至少 1 个。
        let caret = rove_ui::dpi::device_thickness(2.0, scale);
        let phys = caret * scale;
        assert!(
            (phys - phys.round()).abs() < 1e-3,
            "{scale}×：插入符宽 {phys}"
        );
        assert!(phys >= 1.0, "{scale}×：插入符不能消失");
    }
}
