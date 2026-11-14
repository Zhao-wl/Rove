//! 代码视图入口的集成测试（#19）。只用 `rove_ui::code_view` 的公开接口，在 GPUI 测试窗口里驱动。
//!
//! 运行：`cargo test -p rove-ui --test code_view`
//!
//! 测试窗口使用 GPUI 的测试文本系统：每个 BMP 字符宽 0.6 em，非 BMP 字符（emoji 等）宽 1.2 em，
//! 因此命中测试的期望值是确定的。真实 DirectWrite 下的命中测试走同一份“字素簇边界 x 表”。

use gpui::{
    point, px, Context, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext,
    Window,
};
use rove_base::*;
use rove_buffer::{LineCount, RopeText, TextSource};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings, SettingsChange};
use rove_theme::{Color, ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{
    self, columns::DisplayLine, priority, scroll, selection::pos, CodeView, CodeViewEvent,
    CodeViewServices, InlineDecoration, LineDecorations, LineStyle, Reveal, ScrollPosition,
    Selection, SelectionSource, TextStyle, Viewport,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

// ------------------------------------------------------------------ 测试夹具

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
    open_source(cx, source, overrides)
}

fn open_source<'a>(
    cx: &'a mut TestAppContext,
    source: Arc<dyn TextSource>,
    overrides: Vec<(&'static str, Value)>,
) -> (Entity<CodeView>, &'a mut VisualTestContext) {
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

fn point_of(view: &Entity<CodeView>, cx: &mut VisualTestContext, p: Position) -> Point<Pixels> {
    view.update(cx, |v, _| v.point_for_position(p))
        .expect("position visible")
}

fn mouse_down(cx: &mut VisualTestContext, at: Point<Pixels>, clicks: usize, shift: bool) {
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers {
            shift,
            ..Default::default()
        },
        click_count: clicks,
        first_mouse: false,
    });
}

fn mouse_up(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
}

fn wheel(cx: &mut VisualTestContext, at: Point<Pixels>, lines: f32, modifiers: Modifiers) {
    cx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Lines(point(0.0, lines)),
        modifiers,
        touch_phase: gpui::TouchPhase::Moved,
    });
}

/// 行内某位置垂直居中的点（命中测试用）。
fn mid(view: &Entity<CodeView>, cx: &mut VisualTestContext, p: Position) -> Point<Pixels> {
    let lh = view.read_with(cx, |v, _| v.line_height());
    let at = point_of(view, cx, p);
    point(at.x, at.y + px(lh / 2.0))
}

/// 按行号生成内容的超大文本源：14,290,000 行，不分配全文。
struct Synthetic {
    lines: u64,
}
impl Synthetic {
    const WIDTH: u64 = 13; // "line 00000000"
    fn text(line: u64) -> String {
        format!("line {line:08}")
    }
}
impl TextSource for Synthetic {
    fn version(&self) -> Version {
        Version(1)
    }
    fn byte_len(&self) -> u64 {
        self.lines * (Self::WIDTH + 1) - 1
    }
    fn lines(&self) -> LineCount {
        LineCount {
            known: self.lines,
            indexed_bytes: ByteOffset(self.byte_len()),
            total_bytes: self.byte_len(),
            complete: true,
        }
    }
    fn line_range(&self, line: Line) -> Result<ByteRange> {
        if line.0 >= self.lines {
            return Err(Error::OutOfBounds);
        }
        let start = line.0 * (Self::WIDTH + 1);
        Span::new(ByteOffset(start), ByteOffset(start + Self::WIDTH))
    }
    fn visit_chunks(&self, range: ByteRange, visitor: &mut dyn FnMut(&str)) -> Result<()> {
        let mut at = range.start().0;
        while at < range.end().0 {
            let line = at / (Self::WIDTH + 1);
            let mut full = Self::text(line);
            full.push('\n');
            let off = (at % (Self::WIDTH + 1)) as usize;
            let take = ((range.end().0 - at) as usize).min(full.len() - off);
            visitor(&full[off..off + take]);
            at += take as u64;
        }
        Ok(())
    }
    fn offset(&self, p: Position) -> Result<ByteOffset> {
        if p.line.0 >= self.lines || p.column.0 > Self::WIDTH {
            return Err(Error::OutOfBounds);
        }
        Ok(ByteOffset(p.line.0 * (Self::WIDTH + 1) + p.column.0))
    }
    fn position(&self, o: ByteOffset) -> Result<Position> {
        Ok(pos(o.0 / (Self::WIDTH + 1), o.0 % (Self::WIDTH + 1)))
    }
    fn utf16_to_utf8(&self, _: Line, c: Utf16Column) -> Result<Utf8Column> {
        Ok(Utf8Column(c.0))
    }
    fn utf8_to_utf16(&self, _: Line, c: Utf8Column) -> Result<Utf16Column> {
        Ok(Utf16Column(c.0))
    }
}

/// `spikes/gen_testdata.py` 的 7 种行模式。
fn mixed_patterns() -> Vec<String> {
    vec![
        "// ASCII only: fn main() { let x = 42; }".into(),
        "// 中文注释：这是一个用于测试 CJK 字形渲染与字体回退的示例行。".into(),
        "// Mixed 混合 text with emoji 🚀🔥✅ and symbols ±≈→∞ ok".into(),
        "// 日本語テキスト・한국어 텍스트 mixed with English".into(),
        "let emoji_table = [\"😀\", \"🇨🇳\", \"👨‍👩‍👧‍👦\", \"🧑🏽‍💻\"]; // ZWJ + skin tone".into(),
        "\tpublic void Foo(int bar) { /* tab indented, 宽窄混排 */ }".into(),
        format!(
            "const LONG: &str = \"{}\"; // very long line",
            "x".repeat(300)
        ),
    ]
}

// ------------------------------------------------------------------ 入口与消费方示例

/// 入口集成测试：打开文本 → 布局可见行 → 正式整形 → 首行内容与视口正确。
#[gpui::test]
fn code_view_entry_lays_out_only_visible_lines(cx: &mut TestAppContext) {
    let text = (0..10_000)
        .map(|i| format!("line {i}\n"))
        .collect::<String>();
    let (view, cx) = open(cx, &text, vec![]);
    view.read_with(cx, |v, _| {
        let f = v.last_frame();
        assert_eq!(f.first_line, 0);
        assert!(f.visible_lines > 10 && f.visible_lines < 200, "{f:?}");
        assert_eq!(f.shaped, f.visible_lines, "空闲切片完成后全部是正式结果");
        assert_eq!(&*v.painted_text(0).unwrap(), "line 0");
        // 只准备了可见行与预取行，远处的行不在缓存里。
        assert!(v.painted_text(9_000).is_none());
        assert!(v.line_cache_stats().entries < 1000);
    });
}

/// 消费方示例：视口变化事件（#20 高亮按可见范围请求、R55 缩略图同步）。
#[gpui::test]
fn consumer_example_viewport_events(cx: &mut TestAppContext) {
    let text = "x\n".repeat(5000);
    let (view, cx) = open(cx, &text, vec![]);
    let seen: Rc<RefCell<Vec<Viewport>>> = Rc::default();
    let sink = seen.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &CodeViewEvent, _| {
            if let CodeViewEvent::ViewportChanged(v) = e {
                sink.borrow_mut().push(*v);
            }
        })
        .detach();
    });
    view.update(cx, |v, cx| v.reveal_line(1000, Reveal::Top, cx));
    settle(cx);
    let last = *seen.borrow().last().expect("viewport event");
    assert_eq!(last.first_line, 1000);
    assert!(last.last_line > 1000);
    assert!(last.height > 0.0 && last.width > 0.0);
    assert_eq!(Some(last), view.read_with(cx, |v, _| v.viewport()));
}

/// 消费方示例：装饰（多提供方按优先级叠加、按行失效、整行样式）。
#[gpui::test]
fn consumer_example_decorations(cx: &mut TestAppContext) {
    const RED: Color = Color(255, 0, 0, 255);
    const GREEN: Color = Color(0, 255, 0, 255);
    const YELLOW: Color = Color(255, 255, 0, 255);
    let (view, cx) = open(cx, "let alpha = beta;\nsecond\n", vec![]);
    let calls = Rc::new(RefCell::new(0));
    let counter = calls.clone();
    let colour = Rc::new(RefCell::new(RED));
    let c2 = colour.clone();
    let syntax = view
        .update(cx, |v, cx| {
            v.add_decorations(
                "test.syntax",
                priority::SYNTAX,
                Rc::new(move |line: u64, _: &str, out: &mut LineDecorations| {
                    *counter.borrow_mut() += 1;
                    if line == 0 {
                        out.inline.push(InlineDecoration {
                            range: 0..3,
                            style: TextStyle {
                                foreground: Some(*c2.borrow()),
                                ..Default::default()
                            },
                        });
                    }
                }),
                cx,
            )
        })
        .unwrap();
    view.update(cx, |v, cx| {
        v.add_decorations(
            "test.find",
            priority::FIND_MATCH,
            Rc::new(|line: u64, text: &str, out: &mut LineDecorations| {
                if let Some(i) = text.find("alpha") {
                    out.inline.push(InlineDecoration {
                        range: i as u32..(i + 5) as u32,
                        style: TextStyle {
                            background: Some(YELLOW),
                            foreground: Some(GREEN),
                            ..Default::default()
                        },
                    });
                }
                if line == 1 {
                    out.line = LineStyle {
                        background: Some(YELLOW),
                        ..Default::default()
                    };
                }
            }),
            cx,
        )
        .unwrap();
    });
    settle(cx);
    let merged = view.read_with(cx, |v, _| v.merged_decorations(0).unwrap());
    assert_eq!(merged.runs[0].range, 0..3);
    assert_eq!(merged.runs[0].style.foreground, Some(RED));
    let alpha = merged.runs.iter().find(|r| r.range == (4..9)).unwrap();
    assert_eq!(alpha.style.foreground, Some(GREEN), "高优先级覆盖前景色");
    assert_eq!(alpha.style.background, Some(YELLOW));
    let line1 = view.read_with(cx, |v, _| v.merged_decorations(1).unwrap());
    assert_eq!(line1.line.background, Some(YELLOW));

    // 不再调用提供方：滚动、重绘都只读缓存。
    let before = *calls.borrow();
    settle(cx);
    assert_eq!(*calls.borrow(), before);
    // 按行失效后只重算该行。
    *colour.borrow_mut() = GREEN;
    view.update(cx, |v, cx| {
        v.invalidate_decorations(&syntax, Some(0..1), cx)
    });
    settle(cx);
    assert_eq!(*calls.borrow(), before + 1);
    let merged = view.read_with(cx, |v, _| v.merged_decorations(0).unwrap());
    assert_eq!(merged.runs[0].style.foreground, Some(GREEN));
    // 颜色变化不让整形缓存失效。
    view.read_with(cx, |v, _| assert!(v.is_line_shaped(0)));
}

/// 消费方示例：选区读写与事件（R65 上下文篮子读取选区、#40 跳转后设置选区）。
#[gpui::test]
fn consumer_example_selection(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "alpha beta\r\ngamma\r\n", vec![]);
    let events: Rc<RefCell<Vec<(Selection, SelectionSource)>>> = Rc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &CodeViewEvent, _| {
            if let CodeViewEvent::SelectionChanged { selection, source } = e {
                sink.borrow_mut().push((*selection, *source));
            }
        })
        .detach();
    });
    view.update(cx, |v, cx| {
        v.set_selection(
            Selection::new(pos(0, 6), pos(1, 2)),
            Some(Reveal::Nearest),
            cx,
        )
    });
    let (sel, source) = *events.borrow().last().unwrap();
    assert_eq!(sel, Selection::new(pos(0, 6), pos(1, 2)));
    assert_eq!(source, SelectionSource::Api);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selected_text().unwrap(), "beta\r\nga")
    });
    // 越界与字符中部坐标被钳制。
    view.update(cx, |v, cx| {
        v.set_selection(Selection::caret(pos(99, 99)), None, cx)
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::caret(pos(2, 0)))
    });
}

/// 消费方示例：滚动定位（#24 跳转到行、#40 定位到行列、#21 居中策略）。
#[gpui::test]
fn consumer_example_scroll_positioning(cx: &mut TestAppContext) {
    let text = "0123456789\n".repeat(2000);
    let (view, cx) = open(cx, &text, vec![]);
    view.update(cx, |v, cx| v.reveal_line(500, Reveal::Center, cx));
    settle(cx);
    let vp = view.read_with(cx, |v, _| v.viewport().unwrap());
    let middle = (vp.first_line + vp.last_line) / 2;
    assert!(middle.abs_diff(500) <= 1, "{vp:?}");
    // 已可见时 CenterIfOutside 不动。
    view.update(cx, |v, cx| v.reveal_line(502, Reveal::CenterIfOutside, cx));
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.viewport().unwrap().first_line),
        vp.first_line
    );
    view.update(cx, |v, cx| v.reveal_position(pos(1500, 5), Reveal::Top, cx));
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.viewport().unwrap().first_line),
        1500
    );
    // 落位后的命中测试与行列一致。
    let p = mid(&view, cx, pos(1500, 5));
    assert_eq!(
        view.update(cx, |v, _| v.position_for_point(p)),
        Some(pos(1500, 5))
    );
}

// ------------------------------------------------------------------ 验收：精度、占位、预取

/// 1429 万行：跳转到任意行准确落位，滚动条拖到底/中间与行号一致。
#[gpui::test]
fn gigantic_text_scrolls_without_precision_loss(cx: &mut TestAppContext) {
    let lines = 14_290_000;
    let (view, cx) = open_source(cx, Arc::new(Synthetic { lines }), vec![]);
    for target in [1u64, 7_145_000, 9_999_999, 14_289_000, lines - 1] {
        view.update(cx, |v, cx| v.reveal_line(target, Reveal::Top, cx));
        settle(cx);
        let (first, text) = view.read_with(cx, |v, _| {
            (v.viewport().unwrap().first_line, v.painted_text(target))
        });
        assert_eq!(first, target);
        assert_eq!(text.as_deref(), Some(Synthetic::text(target).as_str()));
        // 首行顶边与视口顶边重合（无 f32 像素误差）。
        view.read_with(cx, |v, _| assert_eq!(v.last_frame().first_line_y, 0.0));
        let at = mid(&view, cx, pos(target, 3));
        assert_eq!(
            view.update(cx, |v, _| v.position_for_point(at)),
            Some(pos(target, 3))
        );
    }
    // 滚动条：拖到底就是最后一行在顶端；轨道中点对应中间行。
    let (vp, top) = view.read_with(cx, |v, _| (v.viewport().unwrap(), v.scroll_position().top));
    let track = vp.height as f64;
    let m = scroll::Metrics {
        line_height: view.read_with(cx, |v, _| v.line_height()) as f64,
        viewport_height: vp.height as f64,
        viewport_width: vp.width as f64,
        line_count: lines,
        padding_top: 0.0,
        padding_bottom: 0.0,
        scroll_beyond_last_line: true,
        content_width: 0.0,
        beyond_last_column: 0.0,
        sticky_height: 0.0,
    };
    let zones = scroll::ViewZones::default();
    let total = m.total_lines(&zones);
    let t = scroll::thumb(track, m.viewport_lines(), total, top).unwrap();
    assert!(
        (t.start + t.len - track).abs() < 1e-6,
        "最后一行在顶端时滑块到底"
    );
    let line = scroll::line_for_track_y(track, track / 2.0, &m, &zones);
    assert!(line.abs_diff(lines / 2 + (m.viewport_lines() as u64) / 2) <= 1);
    view.update(cx, |v, cx| {
        v.set_scroll_position(
            ScrollPosition {
                top: scroll::pos_for_thumb(track, m.viewport_lines(), total, (track - t.len) / 2.0),
                x: 0.0,
            },
            false,
            cx,
        )
    });
    settle(cx);
    let first = view.read_with(cx, |v, _| v.viewport().unwrap().first_line);
    assert!(first.abs_diff(((total - m.viewport_lines()) / 2.0) as u64) <= 1);
}

/// 缓存未命中先占位、下一帧换成正式结果，首行 y 与行高不变。
#[gpui::test]
fn placeholders_are_replaced_without_jumping(cx: &mut TestAppContext) {
    let text = (0..3000)
        .map(|i| format!("unique line {i} {}\n", i * 7919))
        .collect::<String>();
    let (view, cx) = open(cx, &text, vec![]);
    view.update(cx, |v, cx| {
        // 极小预算：每片只整形一行，制造多帧占位。
        v.set_prepare_budget(std::time::Duration::ZERO);
        v.record_frames(true);
        v.reveal_line(2000, Reveal::Top, cx);
    });
    settle(cx);
    let frames = view.update(cx, |v, _| v.take_frame_samples());
    let at_target: Vec<_> = frames.iter().filter(|f| f.first_line == 2000).collect();
    assert!(
        at_target
            .iter()
            .any(|f| f.placeholders > 0 || f.missing > 0),
        "{frames:?}"
    );
    let last = at_target.last().unwrap();
    assert_eq!(last.placeholders + last.missing, 0);
    assert!(at_target
        .iter()
        .all(|f| f.first_line_y == 0.0 && f.visible_lines == last.visible_lines));
}

/// #132：正常预算下跳转一帧到位——目标帧就是正式字形，没有占位行也没有空白行。
#[gpui::test]
fn a_jump_paints_the_target_without_placeholders(cx: &mut TestAppContext) {
    let text = (0..3000)
        .map(|i| format!("unique line {i} {}\n", i * 7919))
        .collect::<String>();
    let (view, cx) = open(cx, &text, vec![]);
    settle(cx);
    view.update(cx, |v, cx| {
        v.record_frames(true);
        v.take_frame_samples();
        v.reveal_line(2000, Reveal::Top, cx);
    });
    settle(cx);
    let frames = view.update(cx, |v, _| v.take_frame_samples());
    let at_target: Vec<_> = frames.iter().filter(|f| f.first_line == 2000).collect();
    assert!(!at_target.is_empty(), "画了目标位置的帧：{frames:?}");
    assert!(
        at_target.iter().all(|f| f.placeholders + f.missing == 0),
        "目标帧没有占位行：{frames:?}"
    );
    assert_eq!(
        at_target[0].shaped, at_target[0].visible_lines,
        "第一帧的可见行全部是正式整形结果"
    );
}

/// 沿滚动方向预取：停下后下一屏已整形。
#[gpui::test]
fn prefetches_along_scroll_direction(cx: &mut TestAppContext) {
    let text = (0..5000).map(|i| format!("row {i}\n")).collect::<String>();
    let (view, cx) = open(cx, &text, vec![]);
    let at = view.read_with(cx, |v, _| {
        let b = v.viewport().unwrap();
        point(px(200.0), px(b.height / 2.0))
    });
    wheel(cx, at, -30.0, Modifiers::default());
    settle(cx);
    view.read_with(cx, |v, _| {
        let vp = v.viewport().unwrap();
        assert_eq!(vp.first_line, 30);
        let n = vp.last_line - vp.first_line;
        assert!(v.is_line_shaped(vp.last_line + n / 2), "下一屏已预取");
    });
}

/// 单行 10 万字符：按渲染上限截断，不卡死；点击超出部分落在渲染末尾。
#[gpui::test]
fn very_long_line_is_capped(cx: &mut TestAppContext) {
    let text = format!("{}\nshort\n", "y".repeat(100_000));
    let (view, cx) = open(cx, &text, vec![]);
    view.read_with(cx, |v, _| {
        assert_eq!(v.is_line_truncated(0), Some(true));
        assert!(v.is_line_shaped(0));
    });
    view.update(cx, |v, cx| v.set_line_render_limit(Some(Some(50)), cx));
    settle(cx);
    let far = view.read_with(cx, |v, _| {
        let vp = v.viewport().unwrap();
        point(px(vp.width + 60.0), px(5.0))
    });
    assert_eq!(
        view.update(cx, |v, _| v.position_for_point(far)),
        Some(pos(0, 50))
    );
}

// ------------------------------------------------------------------ 验收：命中测试

/// 7 种模式逐一断言：每个字素簇边界的坐标 ↔ 行列互逆；落在簇内部（ZWJ、肤色修饰、组合字符、
/// Tab 中段）的点击吸附到最近的边界。
#[gpui::test]
fn hit_testing_round_trips_on_all_seven_patterns(cx: &mut TestAppContext) {
    let patterns = mixed_patterns();
    let text = patterns.join("\n") + "\n";
    let (view, cx) = open(cx, &text, vec![]);
    for (line, src) in patterns.iter().enumerate() {
        let line = line as u64;
        let display = DisplayLine::build(src, 4, None);
        let mut last_x = f32::MIN;
        for stop in display.stops() {
            if stop.col > 150 {
                break; // 超出视口宽度的部分不在可见区。
            }
            let p = pos(line, stop.src as u64);
            let at = mid(&view, cx, p);
            assert!(f32::from(at.x) >= last_x, "x 单调：{src:?} @ {}", stop.src);
            last_x = at.x.into();
            assert_eq!(
                view.update(cx, |v, _| v.position_for_point(at)),
                Some(p),
                "{src:?} @ 字节 {}",
                stop.src
            );
        }
        // 簇内部的每个字节都不会成为插入位置。
        let boundaries: Vec<u32> = display.stops().iter().map(|s| s.src).collect();
        for w in display.stops().windows(2) {
            if w[1].col > 150 {
                break;
            }
            let a = mid(&view, cx, pos(line, w[0].src as u64));
            let b = mid(&view, cx, pos(line, w[1].src as u64));
            for f in [0.25f32, 0.75] {
                let x = a.x + (b.x - a.x) * f;
                let hit = view
                    .update(cx, |v, _| v.position_for_point(point(x, a.y)))
                    .unwrap();
                assert!(boundaries.contains(&(hit.column.0 as u32)), "{src:?}");
                let expect = if f < 0.5 { w[0].src } else { w[1].src };
                assert_eq!(
                    hit.column.0 as u32, expect,
                    "{src:?} 段 {}..{}",
                    w[0].src, w[1].src
                );
            }
        }
    }
    // Tab 展开到制表位：tab 行 'p' 在第 4 列。
    let tab = mid(&view, cx, pos(5, 0));
    let p = mid(&view, cx, pos(5, 1));
    let a = mid(&view, cx, pos(0, 0));
    let four = mid(&view, cx, pos(0, 4));
    assert_eq!(p.x - tab.x, four.x - a.x);
}

// ------------------------------------------------------------------ 验收：选区手势与命令

#[gpui::test]
fn selection_gestures_match_vscode(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "foo.bar  baz\nsecond line\nthird\n", vec![]);
    // 双击选词
    let at = mid(&view, cx, pos(0, 5));
    mouse_down(cx, at, 2, false);
    mouse_up(cx, at);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(0, 4), pos(0, 7)))
    });
    // 三击选行（含行终止符）
    mouse_down(cx, at, 3, false);
    mouse_up(cx, at);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(0, 0), pos(1, 0)))
    });
    // 单击 + Shift 点击扩展
    let a = mid(&view, cx, pos(0, 1));
    mouse_down(cx, a, 1, false);
    mouse_up(cx, a);
    let b = mid(&view, cx, pos(1, 3));
    mouse_down(cx, b, 1, true);
    mouse_up(cx, b);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(0, 1), pos(1, 3)))
    });
    // 拖选
    let c = mid(&view, cx, pos(2, 1));
    let d = mid(&view, cx, pos(1, 7));
    mouse_down(cx, c, 1, false);
    cx.simulate_event(MouseMoveEvent {
        position: d,
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::default(),
    });
    mouse_up(cx, d);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(2, 1), pos(1, 7)))
    });
    // 双击后拖动按词扩展
    let e = mid(&view, cx, pos(1, 2));
    let f = mid(&view, cx, pos(1, 9));
    mouse_down(cx, e, 2, false);
    cx.simulate_event(MouseMoveEvent {
        position: f,
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::default(),
    });
    mouse_up(cx, e);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(1, 0), pos(1, 11)))
    });
    // ctrl+a / ctrl+l / escape
    cx.simulate_keystrokes("ctrl-a");
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(0, 0), pos(3, 0)))
    });
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::caret(pos(3, 0)))
    });
    cx.simulate_keystrokes("ctrl-home ctrl-l");
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(0, 0), pos(1, 0)))
    });
    cx.simulate_keystrokes("ctrl-l");
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(0, 0), pos(2, 0)))
    });
    // 插入符移动与扩选
    cx.simulate_keystrokes("escape ctrl-home right right shift-down shift-end");
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(0, 2), pos(1, 11)))
    });
    cx.simulate_keystrokes("left");
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::caret(pos(0, 2)))
    });
    cx.simulate_keystrokes("ctrl-shift-end");
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(0, 2), pos(3, 0)))
    });
    // 点击行号选中整行
    let gutter = {
        let p = mid(&view, cx, pos(1, 0));
        point(px(20.0), p.y)
    };
    mouse_down(cx, gutter, 1, false);
    mouse_up(cx, gutter);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(1, 0), pos(2, 0)))
    });
}

#[gpui::test]
fn copy_preserves_crlf_and_empty_selection_copies_line(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "one\r\ntwo\r\nthree", vec![]);
    cx.simulate_keystrokes("ctrl-a ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        "one\r\ntwo\r\nthree"
    );
    cx.simulate_keystrokes("ctrl-home down ctrl-c");
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "two\r\n");
    cx.simulate_keystrokes("ctrl-end ctrl-insert");
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        "three\r\n"
    );
    cx.simulate_keystrokes("shift-left shift-left ctrl-c");
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "ee");
    view.read_with(cx, |v, _| assert_eq!(v.copy_text().as_deref(), Some("ee")));
}

#[gpui::test]
fn empty_selection_clipboard_off_copies_nothing(cx: &mut TestAppContext) {
    let (view2, cx2) = open(
        cx,
        "a\nb\n",
        vec![("editor.emptySelectionClipboard", Value::Bool(false))],
    );
    cx2.write_to_clipboard(gpui::ClipboardItem::new_string("keep".into()));
    cx2.simulate_keystrokes("ctrl-c");
    assert_eq!(cx2.read_from_clipboard().unwrap().text().unwrap(), "keep");
    view2.read_with(cx2, |v, _| assert!(v.copy_text().is_none()));
}

#[gpui::test]
fn font_zoom_commands_and_ctrl_wheel_invalidate_without_leaking(cx: &mut TestAppContext) {
    let text = (0..400).map(|i| format!("zoom {i}\n")).collect::<String>();
    let (view, cx) = open(
        cx,
        &text,
        vec![("editor.mouseWheelZoom", Value::Bool(true))],
    );
    let base = view.read_with(cx, |v, _| v.font_size());
    let run = |cx: &mut VisualTestContext, id: &str| {
        view.update_in(cx, |v, window, cx| assert!(v.execute(id, window, cx)));
        settle(cx);
    };
    run(cx, "editor.action.fontZoomIn");
    let (size, entries) = view.read_with(cx, |v, _| (v.font_size(), v.shape_cache_stats().entries));
    assert!((size - base * 1.1).abs() < 1e-3);
    // 旧字号的整形结果已清空，只有新字号下的可见行与预取行。
    let visible = view.read_with(cx, |v, _| v.last_frame().visible_lines as usize);
    assert!(entries <= visible * 3, "{entries} vs {visible}");
    run(cx, "editor.action.fontZoomOut");
    run(cx, "editor.action.fontZoomOut");
    assert!((view.read_with(cx, |v, _| v.font_size()) - base * 0.9).abs() < 1e-3);
    run(cx, "editor.action.fontZoomReset");
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), base);
    // Ctrl+滚轮
    let at = point(px(300.0), px(100.0));
    wheel(
        cx,
        at,
        1.0,
        Modifiers {
            control: true,
            ..Default::default()
        },
    );
    settle(cx);
    assert_eq!(view.read_with(cx, |v, _| v.zoom_level()), 1);
    // 反复缩放不泄漏：条目数有界。
    for _ in 0..20 {
        run(cx, "editor.action.fontZoomIn");
        run(cx, "editor.action.fontZoomOut");
    }
    let stats = view.read_with(cx, |v, _| v.shape_cache_stats());
    assert!(stats.entries <= visible * 3, "{stats:?}");
    assert!(stats.bytes <= stats.capacity);
}

#[gpui::test]
fn scroll_commands_and_wheel(cx: &mut TestAppContext) {
    let text = "z\n".repeat(1000);
    let (view, cx) = open(cx, &text, vec![]);
    cx.simulate_keystrokes("ctrl-down ctrl-down");
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.viewport().unwrap().first_line),
        2
    );
    cx.simulate_keystrokes("ctrl-up");
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.viewport().unwrap().first_line),
        1
    );
    let page = view.read_with(cx, |v, _| {
        let vp = v.viewport().unwrap();
        ((vp.height / v.line_height()).floor() as u64)
            .saturating_sub(2)
            .max(1)
    });
    cx.simulate_keystrokes("alt-pagedown");
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.viewport().unwrap().first_line),
        1 + page
    );
    cx.simulate_keystrokes("alt-pageup");
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.viewport().unwrap().first_line),
        1
    );
    // 光标保持不动（scrollLine* 不移动光标）。
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::caret(pos(0, 0)))
    });
}

// ------------------------------------------------------------------ 验收：editor.* 设置逐项生效

#[derive(Clone, Copy, Debug)]
struct Probe {
    text_x: f32,
    char_w: f32,
    line_h: f32,
    first_y: f32,
    tab_w: f32,
    max_top: f64,
    width: f32,
    height: f32,
    wheel_1: u64,
    wheel_alt: u64,
}

fn probe(overrides: Vec<(&'static str, Value)>) -> (Probe, CodeViewSnapshot) {
    let mut tc = TestAppContext::single();
    let text = format!("abcdef\n\tx\n{}", "line\n".repeat(400));
    let (view, cx) = open(&mut tc, &text, overrides);
    let o = mid(&view, cx, pos(0, 0));
    let c3 = mid(&view, cx, pos(0, 3));
    let l1 = mid(&view, cx, pos(1, 0));
    let tab = mid(&view, cx, pos(1, 1));
    let (vp, lh) = view.read_with(cx, |v, _| (v.viewport().unwrap(), v.line_height()));
    view.update(cx, |v, cx| {
        v.set_scroll_position(ScrollPosition { top: 1e12, x: 0.0 }, false, cx)
    });
    settle(cx);
    let max_top = view.read_with(cx, |v, _| v.scroll_position().top);
    view.update(cx, |v, cx| {
        v.set_scroll_position(ScrollPosition::default(), false, cx)
    });
    settle(cx);
    let at = point(px(300.0), px(50.0));
    wheel(cx, at, -1.0, Modifiers::default());
    settle(cx);
    let wheel_1 = view.read_with(cx, |v, _| v.viewport().unwrap().first_line);
    view.update(cx, |v, cx| {
        v.set_scroll_position(ScrollPosition::default(), false, cx)
    });
    settle(cx);
    wheel(
        cx,
        at,
        -1.0,
        Modifiers {
            alt: true,
            ..Default::default()
        },
    );
    settle(cx);
    let wheel_alt = view.read_with(cx, |v, _| v.viewport().unwrap().first_line);
    let snapshot = view.read_with(cx, |v, _| CodeViewSnapshot {
        truncated: v.is_line_truncated(0),
        options: v.options().clone(),
    });
    (
        Probe {
            text_x: o.x.into(),
            char_w: f32::from(c3.x - o.x) / 3.0,
            line_h: f32::from(l1.y - o.y),
            first_y: f32::from(o.y) - lh / 2.0,
            tab_w: f32::from(tab.x - l1.x),
            max_top,
            width: vp.width,
            height: vp.height,
            wheel_1,
            wheel_alt,
        },
        snapshot,
    )
}

struct CodeViewSnapshot {
    truncated: Option<bool>,
    options: code_view::options::EditorOptions,
}

#[test]
fn editor_settings_take_effect() {
    let (base, _) = probe(vec![]);
    let num = |n: f64| Value::Number(n);
    let s = |v: &str| Value::String(v.into());
    type Check = fn(&Probe, &Probe, &CodeViewSnapshot) -> bool;
    let cases: Vec<(&str, Value, Check)> = vec![
        ("editor.lineNumbers", s("off"), |b, p, _| {
            p.text_x < b.text_x
        }),
        ("editor.glyphMargin", Value::Bool(false), |b, p, _| {
            (b.text_x - p.text_x - 18.0).abs() < 0.01
        }),
        ("editor.padding.top", num(10.0), |b, p, _| {
            (p.first_y - b.first_y - 10.0).abs() < 0.01
        }),
        ("editor.lineHeight", num(30.0), |_, p, _| {
            (p.line_h - 30.0).abs() < 0.01
        }),
        ("editor.fontSize", num(20.0), |b, p, _| {
            p.char_w > b.char_w * 1.3
        }),
        (
            "editor.fontFamily",
            s("'Cascadia Code', monospace"),
            |_, _, s| &*s.options.font_family == "Cascadia Code",
        ),
        ("editor.fontWeight", s("bold"), |_, _, s| {
            s.options.font_weight == 700.0
        }),
        ("editor.fontLigatures", Value::Bool(true), |_, _, s| {
            s.options.ligatures == code_view::options::Ligatures::On
        }),
        ("editor.letterSpacing", num(2.0), |b, p, _| {
            (p.char_w - b.char_w - 2.0).abs() < 0.01
        }),
        ("editor.tabSize", num(2.0), |b, p, _| {
            (p.tab_w * 2.0 - b.tab_w).abs() < 0.01
        }),
        (
            "editor.scrollBeyondLastLine",
            Value::Bool(false),
            |b, p, _| p.max_top < b.max_top,
        ),
        ("editor.padding.bottom", num(100.0), |b, p, _| {
            p.max_top > b.max_top
        }),
        ("editor.mouseWheelScrollSensitivity", num(2.0), |b, p, _| {
            p.wheel_1 == b.wheel_1 * 2
        }),
        ("editor.fastScrollSensitivity", num(2.0), |b, p, _| {
            p.wheel_alt == b.wheel_1 * 2 && b.wheel_alt == b.wheel_1 * 5
        }),
        ("editor.scrollbar.vertical", s("hidden"), |b, p, _| {
            (p.width - b.width - 14.0).abs() < 0.01
        }),
        (
            "editor.scrollbar.verticalScrollbarSize",
            num(30.0),
            |b, p, _| (b.width - p.width - 16.0).abs() < 0.01,
        ),
        ("editor.stopRenderingLineAfter", num(3.0), |_, _, s| {
            s.truncated == Some(true)
        }),
        ("editor.cursorSurroundingLines", num(5.0), |_, _, s| {
            s.options.cursor_surrounding_lines == 5
        }),
        ("editor.cursorSurroundingLinesStyle", s("all"), |_, _, s| {
            s.options.cursor_surrounding_all
        }),
        ("editor.smoothScrolling", Value::Bool(true), |_, _, s| {
            s.options.smooth_scrolling
        }),
        (
            "editor.scrollPredominantAxis",
            Value::Bool(false),
            |_, _, s| !s.options.scroll_predominant_axis,
        ),
        ("editor.scrollBeyondLastColumn", num(10.0), |_, _, s| {
            s.options.scroll_beyond_last_column == 10.0
        }),
        ("editor.scrollbar.horizontal", s("visible"), |b, p, _| {
            (b.height - p.height - 12.0).abs() < 0.01
        }),
        (
            "editor.scrollbar.horizontalScrollbarSize",
            num(40.0),
            |_, _, s| s.options.horizontal_scrollbar_size == 40.0,
        ),
        (
            "editor.scrollbar.ignoreHorizontalScrollbarInContentHeight",
            Value::Bool(true),
            |_, _, s| s.options.ignore_horizontal_scrollbar_in_content_height,
        ),
        (
            "editor.scrollbar.scrollByPage",
            Value::Bool(true),
            |_, _, s| s.options.scroll_by_page,
        ),
        ("editor.renderFinalNewline", s("dimmed"), |_, _, s| {
            s.options.render_final_newline == code_view::options::FinalNewline::Dimmed
        }),
        ("editor.renderLineHighlight", s("all"), |_, _, s| {
            s.options.render_line_highlight == code_view::options::LineHighlight::All
        }),
        (
            "editor.renderLineHighlightOnlyWhenFocus",
            Value::Bool(true),
            |_, _, s| s.options.line_highlight_only_when_focus,
        ),
        ("editor.cursorStyle", s("block"), |_, _, s| {
            s.options.cursor_style == code_view::options::CursorStyle::Block
        }),
        ("editor.cursorWidth", num(5.0), |_, _, s| {
            s.options.caret_width() == 5.0
        }),
        ("editor.cursorBlinking", s("solid"), |_, _, s| {
            s.options.cursor_blinking == code_view::options::CursorBlinking::Solid
        }),
        ("editor.roundedSelection", Value::Bool(false), |_, _, s| {
            !s.options.rounded_selection
        }),
        ("editor.wordSeparators", s(""), |_, _, s| {
            s.options.word_separators.is_empty()
        }),
    ];
    let mut failures = vec![];
    for (key, value, check) in cases {
        let (p, snap) = probe(vec![(key, value.clone())]);
        if !check(&base, &p, &snap) {
            failures.push(format!("{key} = {value:?}: base {base:?} / got {p:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[gpui::test]
fn word_separators_and_surrounding_lines_drive_behaviour(cx: &mut TestAppContext) {
    let text = "foo.bar\n".repeat(300);
    let (view, cx) = open(
        cx,
        &text,
        vec![
            ("editor.wordSeparators", Value::String(" ".into())),
            ("editor.cursorSurroundingLines", Value::Number(3.0)),
        ],
    );
    let at = mid(&view, cx, pos(0, 1));
    mouse_down(cx, at, 2, false);
    mouse_up(cx, at);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection(), Selection::new(pos(0, 0), pos(0, 7)))
    });
    let rows = view.read_with(cx, |v, _| {
        (v.viewport().unwrap().height / v.line_height()) as u64
    });
    cx.simulate_keystrokes("escape ctrl-home");
    for _ in 0..rows {
        cx.simulate_keystrokes("down");
    }
    settle(cx);
    view.read_with(cx, |v, _| {
        let vp = v.viewport().unwrap();
        let caret = v.selection().active.line.0;
        assert!(
            vp.last_line >= caret + 3,
            "光标下方保留 3 行：{vp:?} caret {caret}"
        );
    });
}

/// 消费方示例：行下方嵌入区域（#41 速览窗口）——后续行整体下移，区域位置可查询。
#[gpui::test]
fn view_zones_push_following_lines_down(cx: &mut TestAppContext) {
    let text = "z\n".repeat(200);
    let (view, cx) = open(cx, &text, vec![]);
    let before = mid(&view, cx, pos(6, 0));
    view.update(cx, |v, cx| v.set_view_zone(5, 120.0, cx));
    settle(cx);
    let after = mid(&view, cx, pos(6, 0));
    assert_eq!(f32::from(after.y - before.y), 120.0);
    let line5 = point_of(&view, cx, pos(5, 0));
    let lh = view.read_with(cx, |v, _| v.line_height());
    let zone = view
        .read_with(cx, |v, _| v.view_zone_bounds(5))
        .expect("zone visible");
    assert_eq!(zone.origin.y, line5.y + px(lh));
    assert_eq!(f32::from(zone.size.height), 120.0);
    // 点击嵌入区域下方的行仍然命中正确的行。
    assert_eq!(
        view.update(cx, |v, _| v.position_for_point(after)),
        Some(pos(6, 0))
    );
}

/// 回归（PR #101 评审）：`\tfoo` 与 `    foo` 展开后的显示文本相同、整形结果共享，但两行的字素簇边界数
/// 不同（5 与 8）。边界 x 必须按各行自己的 stops 计算，光标放到空格行行尾时不能越界。
#[gpui::test]
fn shared_shape_for_tab_and_spaces_keeps_per_line_boundaries(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "\tfoo\n    foo\n", vec![]);
    view.read_with(cx, |v, _| {
        assert!(v.is_line_shaped(0) && v.is_line_shaped(1));
        assert_eq!(
            v.shape_cache_stats().entries,
            2,
            "前两行共享同一份整形结果，另一份是末尾空行"
        );
    });
    view.update(cx, |v, cx| {
        v.set_selection(Selection::caret(pos(1, 7)), Some(Reveal::Nearest), cx)
    });
    settle(cx); // 每帧绘制插入符
    let tab_end = mid(&view, cx, pos(0, 4));
    let space_end = mid(&view, cx, pos(1, 7));
    assert_eq!(tab_end.x, space_end.x, "两行行尾在同一列");
    for col in 0..=7 {
        let at = mid(&view, cx, pos(1, col));
        assert_eq!(
            view.update(cx, |v, _| v.position_for_point(at)),
            Some(pos(1, col))
        );
    }
    for col in 0..=4 {
        let at = mid(&view, cx, pos(0, col));
        assert_eq!(
            view.update(cx, |v, _| v.position_for_point(at)),
            Some(pos(0, col))
        );
    }
}
