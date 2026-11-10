//! Rove M0 UI spike - Route A: GPUI (Zed's UI framework), crates.io `gpui` 0.2.2.
//!
//! Read-only viewer: mmap + background line index, virtualized rendering of the
//! visible lines only, mouse-wheel scrolling, caret placement by click, plus a
//! one-line editable buffer used to exercise the IME (preedit / candidate window).
//!
//! Usage: ime-gpui <file> [--bench-scroll N]

mod doc;
mod ime_selection;

use std::ops::Range;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use doc::Doc;
use gpui::{
    actions, div, fill, point, prelude::*, px, rgb, rgba, size, App, Application, Bounds,
    ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, Font, FontFallbacks, FontFeatures, FontStyle,
    FontWeight, GlobalElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent, PaintQuad,
    Pixels, Point, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, Style, TextRun,
    UTF16Selection, UnderlineStyle, Window, WindowBounds, WindowOptions,
};

const MAX_LINE_BYTES: usize = 2048;
const FONT_SIZE: f32 = 14.0;
const LINE_HEIGHT: f32 = 20.0;
const GUTTER_W: f32 = 96.0;

// ---------------------------------------------------------------- measurement

struct Bench {
    target: usize,
    step: f64,
    done: usize,
    intervals: Vec<f64>,
    paint_us: Vec<f64>,
}

fn epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn stats(v: &mut [f64]) -> (f64, f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg = v.iter().sum::<f64>() / v.len() as f64;
    let p99 = v[((v.len() as f64 * 0.99) as usize).min(v.len() - 1)];
    (avg, p99, *v.last().unwrap())
}

// ---------------------------------------------------------------- file viewer

struct Viewer {
    doc: Arc<Doc>,
    scroll_line: f64,
    caret: (usize, usize), // (line, byte offset within line)
    rows: Vec<(usize, ShapedLine)>,
    row_origin: Point<Pixels>,
    visible_rows: usize,
    input: Entity<TextInput>,
    focus_handle: FocusHandle,
    // instrumentation
    t_main: Instant,
    first_frame_ms: Option<f64>,
    last_frame: Option<Instant>,
    last_paint_us: f64,
    bench: Option<Bench>,
    scale_factor: f32,
    index_reported: bool,
}

impl Viewer {
    fn font(&self) -> Font {
        Font {
            family: "Consolas".into(),
            features: FontFeatures::default(),
            fallbacks: Some(FontFallbacks::from_fonts(vec![
                "Microsoft YaHei".into(),
                "Segoe UI Emoji".into(),
                "Segoe UI Symbol".into(),
            ])),
            weight: FontWeight::default(),
            style: FontStyle::default(),
        }
    }

    fn max_scroll(&self) -> f64 {
        (self.doc.line_count() as f64 - 1.0).max(0.0)
    }

    fn on_scroll(&mut self, e: &ScrollWheelEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let lines = match e.delta {
            ScrollDelta::Lines(p) => p.y as f64,
            ScrollDelta::Pixels(p) => p.y.to_f64() / LINE_HEIGHT as f64,
        };
        self.scroll_line = (self.scroll_line - lines).clamp(0.0, self.max_scroll());
        cx.notify();
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let rel_y = (e.position.y - self.row_origin.y).to_f64();
        let row = (rel_y / LINE_HEIGHT as f64).floor().max(0.0) as usize;
        if let Some((line_ix, shaped)) = self.rows.get(row) {
            let x = e.position.x - self.row_origin.x - px(GUTTER_W);
            self.caret = (*line_ix, shaped.closest_index_for_x(x));
            cx.notify();
        }
    }

    fn frame_presented(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.first_frame_ms.is_none() {
            let ms = self.t_main.elapsed().as_secs_f64() * 1000.0;
            self.first_frame_ms = Some(ms);
            println!("FIRSTFRAME_SINCE_MAIN_MS {:.1}", ms);
            println!("FIRSTFRAME_EPOCH_MS {}", epoch_ms());
            println!("SCALE_FACTOR {}", self.scale_factor);
            println!("OPEN_MS {:.2}", self.doc.open_ms);
            println!("FILE_BYTES {}", self.doc.len());
        }
        if let Some(prev) = self.last_frame {
            let dt = (now - prev).as_secs_f64() * 1000.0;
            if let Some(b) = self.bench.as_mut() {
                b.intervals.push(dt);
                b.paint_us.push(self.last_paint_us);
            }
        }
        self.last_frame = Some(now);
        if !self.index_reported && self.doc.index.complete.load(Ordering::Acquire) {
            self.index_reported = true;
            println!(
                "FULL_INDEX_MS {} LINES {}",
                self.doc.index.full_index_ms.load(Ordering::Relaxed),
                self.doc.line_count()
            );
        }

        let finish = if let Some(b) = self.bench.as_mut() {
            b.done += 1;
            b.done >= b.target
        } else {
            false
        };
        if self.bench.is_some() {
            if finish {
                let mut b = self.bench.take().unwrap();
                let (avg, p99, max) = stats(&mut b.intervals);
                let (pavg, pp99, pmax) = stats(&mut b.paint_us);
                println!("SCROLL_FRAMES {}", b.intervals.len());
                println!(
                    "SCROLL_INTERVAL_MS avg={:.2} p99={:.2} max={:.2}",
                    avg, p99, max
                );
                println!(
                    "SCROLL_CPU_US avg={:.0} p99={:.0} max={:.0}",
                    pavg, pp99, pmax
                );
                println!(
                    "INDEX_COMPLETE {} LINES {} FULL_INDEX_MS {}",
                    self.doc.index.complete.load(Ordering::Acquire),
                    self.doc.line_count(),
                    self.doc.index.full_index_ms.load(Ordering::Relaxed)
                );
                cx.quit();
            } else {
                // continuous scroll, 3 lines per frame, bounce at the end
                let next = self.scroll_line + self.bench.as_ref().map(|b| b.step).unwrap_or(3.0);
                self.scroll_line = if next > self.max_scroll() { 0.0 } else { next };
                cx.notify();
            }
        }
    }
}

impl Focusable for Viewer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.scale_factor = window.scale_factor();
        let me = cx.entity();
        window.on_next_frame(move |_window, cx| {
            me.update(cx, |this, cx| this.frame_presented(cx));
        });

        let idx = &self.doc.index;
        let status = format!(
            "{}  |  {} bytes  |  lines indexed: {}{}  |  caret L{}:{}  |  dpi x{:.2}  |  first frame {:.1} ms",
            self.doc.path.display(),
            self.doc.len(),
            self.doc.line_count(),
            if idx.complete.load(Ordering::Acquire) {
                " (complete)".to_string()
            } else {
                format!(" ({:.0}% ...)", idx.progress() * 100.0)
            },
            self.caret.0 + 1,
            self.caret.1,
            self.scale_factor,
            self.first_frame_ms.unwrap_or(0.0),
        );

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xd4d4d4))
            .font_family("Consolas")
            .text_size(px(FONT_SIZE))
            .line_height(px(LINE_HEIGHT))
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(22.0))
                    .px_2()
                    .bg(rgb(0x252526))
                    .child(status),
            )
            .child(
                div()
                    .id("code")
                    .flex_grow()
                    .overflow_hidden()
                    .cursor(CursorStyle::IBeam)
                    .on_scroll_wheel(cx.listener(Self::on_scroll))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .child(CodeElement {
                        viewer: cx.entity(),
                    }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(30.0))
                    .bg(rgb(0x2d2d30))
                    .child(self.input.clone()),
            )
    }
}

/// Custom element: lays out and paints only the visible lines.
struct CodeElement {
    viewer: Entity<Viewer>,
}

struct CodePrepaint {
    rows: Vec<(usize, ShapedLine, ShapedLine)>, // (line index, gutter, text)
    caret: Option<PaintQuad>,
    paint_start: Instant,
}

impl IntoElement for CodeElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CodeElement {
    type RequestLayoutState = ();
    type PrepaintState = CodePrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = gpui::relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let paint_start = Instant::now();
        let v = self.viewer.read(cx);
        let doc = v.doc.clone();
        let font = v.font();
        let scroll = v.scroll_line;
        let caret = v.caret;

        let visible = (f32::from(bounds.size.height) / LINE_HEIGHT).ceil() as usize + 1;
        let first = scroll.floor() as usize;
        let count = doc.line_count();
        let range: Range<usize> = first..(first + visible).min(count);

        let text_system = window.text_system().clone();
        let mut rows = Vec::with_capacity(range.len());
        let mut caret_quad = None;
        for i in range.clone() {
            let text: SharedString = doc.line(i, MAX_LINE_BYTES).into_owned().into();
            let num: SharedString = format!("{:>7} ", i + 1).into();
            let gutter = text_system.shape_line(
                num,
                px(FONT_SIZE),
                &[TextRun {
                    len: 8,
                    font: font.clone(),
                    color: rgb(0x858585).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            let runs = [TextRun {
                len: text.len(),
                font: font.clone(),
                color: rgb(0xd4d4d4).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let line = text_system.shape_line(text, px(FONT_SIZE), &runs, None);
            if i == caret.0 {
                let x = line.x_for_index(caret.1.min(line.len()));
                let y = bounds.top() + px((i as f64 - scroll) as f32 * LINE_HEIGHT);
                caret_quad = Some(fill(
                    Bounds::new(
                        point(bounds.left() + px(GUTTER_W) + x, y),
                        size(px(2.0), px(LINE_HEIGHT)),
                    ),
                    rgb(0x4ec9b0),
                ));
            }
            rows.push((i, gutter, line));
        }

        CodePrepaint {
            rows,
            caret: caret_quad,
            paint_start,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let scroll = self.viewer.read(cx).scroll_line;
        window.paint_quad(fill(
            Bounds::new(bounds.origin, size(px(GUTTER_W), bounds.size.height)),
            rgba(0x00000040),
        ));
        for (i, gutter, line) in prepaint.rows.iter() {
            let y = bounds.top() + px((*i as f64 - scroll) as f32 * LINE_HEIGHT);
            let _ = gutter.paint(point(bounds.left(), y), px(LINE_HEIGHT), window, cx);
            let _ = line.paint(
                point(bounds.left() + px(GUTTER_W), y),
                px(LINE_HEIGHT),
                window,
                cx,
            );
        }
        if let Some(c) = prepaint.caret.take() {
            window.paint_quad(c);
        }
        let elapsed = prepaint.paint_start.elapsed().as_secs_f64() * 1e6;
        let rows: Vec<(usize, ShapedLine)> = prepaint
            .rows
            .iter()
            .map(|(i, _, l)| (*i, l.clone()))
            .collect();
        self.viewer.update(cx, |v, _| {
            v.rows = rows;
            v.row_origin = bounds.origin;
            v.visible_rows = prepaint.rows.len();
            v.last_paint_us = elapsed;
        });
    }
}

// ---------------------------------------------------------------- IME probe
// Single-line editable buffer, condensed from gpui's own examples/input.rs.
// `EntityInputHandler` is what GPUI routes IME composition through.

actions!(
    spike_input,
    [Backspace, Left, Right, SelectAll, Paste, Copy, Quit]
);

struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    selected_range: Range<usize>,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    ime_events: usize,
}

impl TextInput {
    fn o2u16(&self, offset: usize) -> usize {
        let (mut u16c, mut u8c) = (0, 0);
        for ch in self.content.chars() {
            if u8c >= offset {
                break;
            }
            u8c += ch.len_utf8();
            u16c += ch.len_utf16();
        }
        u16c
    }
    fn u16_2o(&self, offset: usize) -> usize {
        let (mut u16c, mut u8c) = (0, 0);
        for ch in self.content.chars() {
            if u16c >= offset {
                break;
            }
            u16c += ch.len_utf16();
            u8c += ch.len_utf8();
        }
        u8c
    }
    fn r2u16(&self, r: &Range<usize>) -> Range<usize> {
        self.o2u16(r.start)..self.o2u16(r.end)
    }
    fn ru16(&self, r: &Range<usize>) -> Range<usize> {
        self.u16_2o(r.start)..self.u16_2o(r.end)
    }
    fn prev_char(&self, offset: usize) -> usize {
        self.content[..offset]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
    fn backspace(&mut self, _: &Backspace, w: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() && self.selected_range.start > 0 {
            self.selected_range =
                self.prev_char(self.selected_range.start)..self.selected_range.end;
        }
        self.replace_text_in_range(None, "", w, cx);
    }
    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let p = self.prev_char(self.selected_range.start);
        self.selected_range = p..p;
        cx.notify();
    }
    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.content[self.selected_range.end..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.selected_range.end + i)
            .unwrap_or(self.content.len());
        self.selected_range = n..n;
        cx.notify();
    }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        cx.notify();
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.content[self.selected_range.clone()].to_string(),
        ));
    }
    fn paste(&mut self, _: &Paste, w: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
            self.replace_text_in_range(None, &t.replace('\n', " "), w, cx);
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.ru16(&range_utf16);
        actual.replace(self.r2u16(&r));
        Some(self.content[r].to_string())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.r2u16(&self.selected_range),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|r| self.r2u16(r))
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked_range = None;
    }
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range_utf16
            .as_ref()
            .map(|r| self.ru16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            (self.content[..r.start].to_owned() + new_text + &self.content[r.end..]).into();
        self.selected_range = r.start + new_text.len()..r.start + new_text.len();
        self.marked_range = None;
        self.ime_events += 1;
        println!("IME commit: {:?}  buffer={:?}", new_text, self.content);
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_sel: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = range_utf16
            .as_ref()
            .map(|r| self.ru16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.content =
            (self.content[..r.start].to_owned() + new_text + &self.content[r.end..]).into();
        self.marked_range = (!new_text.is_empty()).then(|| r.start..r.start + new_text.len());
        self.selected_range = ime_selection::composition_selection(r.start, new_text, new_sel);
        self.ime_events += 1;
        println!(
            "IME preedit: {:?}  marked={:?}  sel={:?}",
            new_text, self.marked_range, self.selected_range
        );
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let r = self.ru16(&range_utf16);
        let b = Bounds::from_corners(
            point(bounds.left() + layout.x_for_index(r.start), bounds.top()),
            point(bounds.left() + layout.x_for_index(r.end), bounds.bottom()),
        );
        println!("IME bounds_for_range {:?} -> {:?}", range_utf16, b);
        Some(b)
    }
    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.last_bounds?.localize(&p)?;
        let i = self.last_layout.as_ref()?.index_for_x(local.x)?;
        Some(self.o2u16(i))
    }
}

struct InputElement {
    input: Entity<TextInput>,
}
struct InputPrepaint {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
}

impl IntoElement for InputElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for InputElement {
    type RequestLayoutState = ();
    type PrepaintState = InputPrepaint;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = px(LINE_HEIGHT).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let text = if input.content.is_empty() {
            SharedString::from("IME test: type Chinese here (Microsoft Pinyin) ...")
        } else {
            input.content.clone()
        };
        let font = Font {
            family: "Consolas".into(),
            features: FontFeatures::default(),
            fallbacks: Some(FontFallbacks::from_fonts(vec![
                "Microsoft YaHei".into(),
                "Segoe UI Emoji".into(),
            ])),
            weight: FontWeight::default(),
            style: FontStyle::default(),
        };
        let base = TextRun {
            len: text.len(),
            font,
            color: rgb(0xffffff).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(m) = input.marked_range.as_ref() {
            vec![
                TextRun {
                    len: m.start,
                    ..base.clone()
                },
                TextRun {
                    len: m.end - m.start,
                    underline: Some(UnderlineStyle {
                        color: Some(rgb(0x4ec9b0).into()),
                        thickness: px(1.5),
                        wavy: false,
                    }),
                    ..base.clone()
                },
                TextRun {
                    len: text.len() - m.end,
                    ..base
                },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect()
        } else {
            vec![base]
        };
        let line = window
            .text_system()
            .shape_line(text, px(FONT_SIZE), &runs, None);
        let x = line.x_for_index(input.selected_range.end.min(line.len()));
        let cursor = Some(fill(
            Bounds::new(
                point(bounds.left() + x, bounds.top()),
                size(px(2.0), px(LINE_HEIGHT)),
            ),
            rgb(0x4ec9b0),
        ));
        InputPrepaint {
            line: Some(line),
            cursor,
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let fh = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &fh,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let line = prepaint.line.take().unwrap();
        let _ = line.paint(bounds.origin, px(LINE_HEIGHT), window, cx);
        if fh.is_focused(window) {
            if let Some(c) = prepaint.cursor.take() {
                window.paint_quad(c);
            }
        }
        self.input.update(cx, |i, _| {
            i.last_layout = Some(line);
            i.last_bounds = Some(bounds);
        });
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.invalidate_character_coordinates();
        div()
            .key_context("SpikeInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .w_full()
            .h(px(30.0))
            .px_2()
            .py(px(5.0))
            .child(InputElement { input: cx.entity() })
    }
}

// ---------------------------------------------------------------- entry point

fn main() {
    let t_main = Instant::now();
    println!("MAIN_EPOCH_MS {}", epoch_ms());

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut path = None;
    let mut bench = 0usize;
    let mut step = 3.0f64;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bench-scroll" => {
                bench = args[i + 1].parse().unwrap_or(600);
                i += 2;
            }
            "--bench-step" => {
                step = args[i + 1].parse().unwrap_or(3.0);
                i += 2;
            }
            other => {
                path = Some(other.to_string());
                i += 1;
            }
        }
    }
    let path = path.unwrap_or_else(|| {
        eprintln!("usage: ime-gpui <file> [--bench-scroll N]");
        std::process::exit(2)
    });
    let doc = Arc::new(Doc::open(std::path::Path::new(&path)).expect("open file"));
    println!("DOC_OPEN_MS {:.2}", doc.open_ms);

    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, None),
            KeyBinding::new("left", Left, None),
            KeyBinding::new("right", Right, None),
            KeyBinding::new("ctrl-a", SelectAll, None),
            KeyBinding::new("ctrl-c", Copy, None),
            KeyBinding::new("ctrl-v", Paste, None),
            KeyBinding::new("ctrl-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| {
                    let input = cx.new(|cx| TextInput {
                        focus_handle: cx.focus_handle(),
                        content: "".into(),
                        selected_range: 0..0,
                        marked_range: None,
                        last_layout: None,
                        last_bounds: None,
                        ime_events: 0,
                    });
                    cx.new(|cx| Viewer {
                        doc,
                        scroll_line: 0.0,
                        caret: (0, 0),
                        rows: Vec::new(),
                        row_origin: point(px(0.0), px(0.0)),
                        visible_rows: 0,
                        input,
                        focus_handle: cx.focus_handle(),
                        t_main,
                        first_frame_ms: None,
                        last_frame: None,
                        last_paint_us: 0.0,
                        bench: (bench > 0).then(|| Bench {
                            target: bench,
                            step,
                            done: 0,
                            intervals: Vec::with_capacity(bench),
                            paint_us: Vec::with_capacity(bench),
                        }),
                        scale_factor: 1.0,
                        index_reported: false,
                    })
                },
            )
            .unwrap();
        window
            .update(cx, |v, window, cx| {
                window.focus(&v.input.focus_handle(cx));
                cx.activate(true);
            })
            .unwrap();
    });
}
