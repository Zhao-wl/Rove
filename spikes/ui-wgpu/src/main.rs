//! Rove M0 UI spike - Route B: self-built stack (winit + wgpu + parley + swash).
//!
//! Same feature set as the GPUI prototype: read-only viewer over mmap + a
//! background line index, only the visible lines are laid out and rendered,
//! mouse-wheel scrolling, caret placement by click, and a one-line editable
//! buffer wired to winit's IME events.
//!
//! Usage: ui-wgpu <file> [--bench-scroll N] [--bench-step N]

mod doc;
mod gfx;

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use doc::Doc;
use gfx::{Gfx, Inst};
use parley::editing::Cursor;
use parley::layout::{Affinity, PositionedLayoutItem};
use parley::style::{FontFamily, StyleProperty};
use parley::{FontContext, Layout, LayoutContext};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const FAMILY: &str = "Consolas, \"Microsoft YaHei\", \"Segoe UI Emoji\", monospace";
const FONT_SIZE: f32 = 14.0;
const LINE_HEIGHT: f32 = 20.0; // logical px
const GUTTER_W: f32 = 96.0;
const STATUS_H: f32 = 22.0;
const INPUT_H: f32 = 30.0;
const MAX_LINE_BYTES: usize = 2048;

const FG: [f32; 4] = [0.831, 0.831, 0.831, 1.0];
const DIM: [f32; 4] = [0.521, 0.521, 0.521, 1.0];
const ACCENT: [f32; 4] = [0.306, 0.788, 0.690, 1.0];

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

struct Bench {
    target: usize,
    step: f64,
    done: usize,
    intervals: Vec<f64>,
    cpu_us: Vec<f64>,
}

struct App {
    doc: Arc<Doc>,
    window: Option<Arc<Window>>,
    gfx: Option<Gfx>,
    font_cx: FontContext,
    layout_cx: LayoutContext<()>,
    /// Shaped-line cache: text -> glyph quads relative to the line origin.
    /// GPUI keeps an equivalent cache inside its TextSystem, so both prototypes
    /// pay shaping only on a cache miss.
    line_cache: HashMap<String, Vec<Inst>>,
    scale: f32,
    scroll_line: f64,
    caret: (usize, usize),
    // IME probe buffer
    edit: String,
    edit_caret: usize,
    preedit: Option<(String, Option<(usize, usize)>)>,
    last_cursor: (f32, f32),
    // measurement
    t_main: Instant,
    first_frame_ms: Option<f64>,
    last_frame: Option<Instant>,
    bench: Option<Bench>,
    index_reported: bool,
}

/// Lay out `text` with parley and append its glyph quads to `insts`.
/// `y_top` is the top of the line box; all units are physical pixels.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    cache: &mut HashMap<String, Vec<Inst>>,
    g: &mut Gfx,
    insts: &mut Vec<Inst>,
    text: &str,
    x: f32,
    y_top: f32,
    scale: f32,
    color: [f32; 4],
    clip_w: f32,
) {
    if text.is_empty() {
        return;
    }
    if let Some(quads) = cache.get(text) {
        for q in quads {
            if q.rect[0] > clip_w {
                break;
            }
            let mut q = *q;
            q.rect[0] += x;
            q.rect[1] += y_top;
            q.color = color;
            insts.push(q);
        }
        return;
    }
    let mut local: Vec<Inst> = Vec::new();
    let mut b = layout_cx.ranged_builder(font_cx, text, scale, true);
    b.push_default(StyleProperty::FontFamily(FontFamily::Source(FAMILY.into())));
    b.push_default(StyleProperty::FontSize(FONT_SIZE));
    let mut layout: Layout<()> = b.build(text);
    layout.break_all_lines(None);

    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let frun = run.run();
            let font = frun.font();
            let font_size = frun.font_size();
            let font_id = font.data.id();
            let data = font.data.data();
            let mut gx = run.offset();
            let gy = run.baseline();
            for glyph in run.glyphs() {
                let px = gx + glyph.x;
                let py = gy - glyph.y;
                gx += glyph.advance;
                if let Some(e) = g.glyph(data, font.index, font_id, glyph.id as u16, font_size) {
                    local.push(Inst {
                        rect: [px + e.left as f32, py - e.top as f32, e.w as f32, e.h as f32],
                        uv: e.uv,
                        color,
                        flags: u32::from(e.color),
                        _pad: [0; 3],
                    });
                }
            }
        }
    }
    for q in local.iter() {
        if q.rect[0] > clip_w {
            break;
        }
        let mut q = *q;
        q.rect[0] += x;
        q.rect[1] += y_top;
        insts.push(q);
    }
    if cache.len() > 8192 {
        cache.clear();
    }
    cache.insert(text.to_string(), local);
}

fn solid(insts: &mut Vec<Inst>, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
    let c = 0.5 / gfx::ATLAS as f32; // centre of the reserved opaque-white block
    insts.push(Inst {
        rect: [x, y, w, h],
        uv: [c, c, c, c],
        color,
        flags: 0,
        _pad: [0; 3],
    });
}

impl App {
    fn layout_line(&mut self, text: &str) -> Layout<()> {
        let mut b = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, self.scale, true);
        b.push_default(StyleProperty::FontFamily(FontFamily::Source(FAMILY.into())));
        b.push_default(StyleProperty::FontSize(FONT_SIZE));
        let mut layout: Layout<()> = b.build(text);
        layout.break_all_lines(None);
        layout
    }

    fn x_for_index(&mut self, text: &str, byte: usize) -> f32 {
        if text.is_empty() || byte == 0 {
            return 0.0;
        }
        let byte = byte.min(text.len());
        let layout = self.layout_line(text);
        Cursor::from_byte_index(&layout, byte, Affinity::Downstream)
            .geometry(&layout, 1.0)
            .x0 as f32
    }

    fn index_for_x(&mut self, text: &str, x: f32) -> usize {
        if text.is_empty() {
            return 0;
        }
        let layout = self.layout_line(text);
        Cursor::from_point(&layout, x, 1.0).index()
    }

    fn max_scroll(&self) -> f64 {
        (self.doc.line_count() as f64 - 1.0).max(0.0)
    }

    fn redraw(&mut self) {
        let Some(mut g) = self.gfx.take() else { return };
        let t0 = Instant::now();
        let s = self.scale;
        let (vw, vh) = (g.config.width as f32, g.config.height as f32);
        let line_h = LINE_HEIGHT * s;
        let gutter = GUTTER_W * s;
        let status_h = STATUS_H * s;
        let input_h = INPUT_H * s;
        let area_top = status_h;
        let area_h = (vh - status_h - input_h).max(0.0);

        let visible = (area_h / line_h).ceil() as usize + 1;
        let first = self.scroll_line.floor() as usize;
        let count = self.doc.line_count();
        let last = (first + visible).min(count);
        let doc = self.doc.clone();

        // caret x needs a layout pass of its own line (borrowck: before `g` use)
        let caret_px = if self.caret.0 >= first && self.caret.0 < last {
            let t = doc.line(self.caret.0, MAX_LINE_BYTES).into_owned();
            Some(self.x_for_index(&t, self.caret.1))
        } else {
            None
        };
        // IME caret x inside the edit line
        let ime_x = {
            let mut shown = self.edit.clone();
            let at = self.edit_caret.min(shown.len());
            if let Some((pre, _)) = self.preedit.as_ref() {
                shown.insert_str(at, pre);
            }
            let upto = at + self.preedit.as_ref().map_or(0, |(p, _)| p.len());
            self.x_for_index(&shown, upto)
        };

        let mut insts: Vec<Inst> = Vec::with_capacity(8192);
        solid(&mut insts, 0.0, 0.0, vw, status_h, [0.145, 0.145, 0.149, 1.0]);
        solid(&mut insts, 0.0, area_top, gutter, area_h, [0.0, 0.0, 0.0, 0.3]);
        solid(
            &mut insts,
            0.0,
            vh - input_h,
            vw,
            input_h,
            [0.176, 0.176, 0.188, 1.0],
        );

        for i in first..last {
            let y = area_top + ((i as f64 - self.scroll_line) as f32) * line_h;
            if y > vh - input_h {
                break;
            }
            let num = format!("{:>7} ", i + 1);
            draw_text(
                &mut self.font_cx,
                &mut self.layout_cx,
                &mut self.line_cache,
                &mut g,
                &mut insts,
                &num,
                0.0,
                y,
                s,
                DIM,
                gutter,
            );
            let text = doc.line(i, MAX_LINE_BYTES);
            draw_text(
                &mut self.font_cx,
                &mut self.layout_cx,
                &mut self.line_cache,
                &mut g,
                &mut insts,
                &text,
                gutter,
                y,
                s,
                FG,
                vw - gutter,
            );
        }
        if let Some(cx) = caret_px {
            let y = area_top + ((self.caret.0 as f64 - self.scroll_line) as f32) * line_h;
            solid(&mut insts, gutter + cx, y, 2.0 * s, line_h, ACCENT);
        }

        let idx = &self.doc.index;
        let status = format!(
            "{}  |  {} bytes  |  lines indexed: {}{}  |  caret L{}:{}  |  dpi x{:.2}  |  first frame {:.1} ms  |  {}",
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
            s,
            self.first_frame_ms.unwrap_or(0.0),
            g.adapter_info,
        );
        draw_text(
            &mut self.font_cx,
            &mut self.layout_cx,
            &mut self.line_cache,
            &mut g,
            &mut insts,
            &status,
            4.0 * s,
            1.0 * s,
            s,
            FG,
            vw,
        );

        // --- IME probe line -------------------------------------------------
        let mut shown = self.edit.clone();
        let at = self.edit_caret.min(shown.len());
        let mut mark: Option<(usize, usize)> = None;
        if let Some((pre, _)) = self.preedit.as_ref() {
            shown.insert_str(at, pre);
            mark = Some((at, at + pre.len()));
        }
        let empty = shown.is_empty();
        let display = if empty {
            "IME test: type Chinese here (Microsoft Pinyin) ...".to_string()
        } else {
            shown.clone()
        };
        let iy = vh - input_h + 5.0 * s;
        let ix = 8.0 * s;
        draw_text(
            &mut self.font_cx,
            &mut self.layout_cx,
            &mut self.line_cache,
            &mut g,
            &mut insts,
            &display,
            ix,
            iy,
            s,
            if empty { DIM } else { FG },
            vw,
        );
        if let Some((a, b)) = mark {
            // underline the preedit span
            let x0 = self.x_for_index(&shown, a);
            let x1 = self.x_for_index(&shown, b);
            solid(
                &mut insts,
                ix + x0,
                iy + line_h - 2.0 * s,
                (x1 - x0).max(1.0),
                2.0 * s,
                ACCENT,
            );
        }
        solid(&mut insts, ix + ime_x, iy, 2.0 * s, line_h, ACCENT);

        let quads = insts.len();
        // CPU cost of laying out + emitting the frame, excluding swapchain
        // acquire/present (which blocks on vsync and would mask the real cost).
        let cpu_us = t0.elapsed().as_secs_f64() * 1e6;
        let res = g.render(&insts, wgpu::Color {
            r: 0.118,
            g: 0.118,
            b: 0.118,
            a: 1.0,
        });
        if let Err(e) = res {
            eprintln!("surface error: {e:?}");
        }
        self.gfx = Some(g);

        // tell the IME where the caret is, so the candidate window follows it
        if let Some(w) = self.window.as_ref() {
            w.set_ime_cursor_area(
                PhysicalPosition::new(ix + ime_x, iy),
                PhysicalSize::new(2.0 * s, line_h),
            );
        }

        self.frame_done(cpu_us, quads);
    }

    fn frame_done(&mut self, cpu_us: f64, quads: usize) {
        let now = Instant::now();
        if self.first_frame_ms.is_none() {
            let ms = self.t_main.elapsed().as_secs_f64() * 1000.0;
            self.first_frame_ms = Some(ms);
            println!("FIRSTFRAME_SINCE_MAIN_MS {ms:.1}");
            println!("FIRSTFRAME_EPOCH_MS {}", epoch_ms());
            println!("SCALE_FACTOR {}", self.scale);
            println!("OPEN_MS {:.2}", self.doc.open_ms);
            println!("FILE_BYTES {}", self.doc.len());
            println!("QUADS_FIRST_FRAME {quads}");
            if let Some(g) = self.gfx.as_ref() {
                println!("ADAPTER {}", g.adapter_info);
            }
        }
        if let Some(prev) = self.last_frame {
            let dt = (now - prev).as_secs_f64() * 1000.0;
            if let Some(b) = self.bench.as_mut() {
                b.intervals.push(dt);
                b.cpu_us.push(cpu_us);
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

        let mut finished = false;
        if let Some(b) = self.bench.as_mut() {
            b.done += 1;
            finished = b.done >= b.target;
        }
        if finished {
            let mut b = self.bench.take().unwrap();
            let (avg, p99, max) = stats(&mut b.intervals);
            let (cavg, cp99, cmax) = stats(&mut b.cpu_us);
            println!("SCROLL_FRAMES {}", b.intervals.len());
            println!("SCROLL_INTERVAL_MS avg={avg:.2} p99={p99:.2} max={max:.2}");
            println!("SCROLL_CPU_US avg={cavg:.0} p99={cp99:.0} max={cmax:.0}");
            println!(
                "INDEX_COMPLETE {} LINES {} FULL_INDEX_MS {}",
                self.doc.index.complete.load(Ordering::Acquire),
                self.doc.line_count(),
                self.doc.index.full_index_ms.load(Ordering::Relaxed)
            );
            if let Some(g) = self.gfx.as_ref() {
                println!("ATLAS_FULL {}", g.atlas_full);
            }
            std::process::exit(0);
        } else if let Some(b) = self.bench.as_ref() {
            let next = self.scroll_line + b.step;
            self.scroll_line = if next > self.max_scroll() { 0.0 } else { next };
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Rove spike - winit + wgpu + parley")
            .with_inner_size(LogicalSize::new(1200.0, 800.0));
        let window = Arc::new(el.create_window(attrs).expect("window"));
        self.scale = window.scale_factor() as f32;
        window.set_ime_allowed(true);
        let g = pollster::block_on(Gfx::new(window.clone()));
        self.gfx = Some(g);
        self.window = Some(window.clone());
        window.request_redraw();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gfx.as_mut() {
                    g.resize(size.width, size.height);
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.line_cache.clear();
                println!("SCALE_FACTOR_CHANGED {scale_factor}");
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64 * 3.0,
                    MouseScrollDelta::PixelDelta(p) => p.y / (LINE_HEIGHT * self.scale) as f64,
                };
                self.scroll_line = (self.scroll_line - lines).clamp(0.0, self.max_scroll());
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.last_cursor = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput { state, .. } if state == ElementState::Pressed => {
                let (mx, my) = self.last_cursor;
                let s = self.scale;
                let area_top = STATUS_H * s;
                let line_h = LINE_HEIGHT * s;
                let row = ((my - area_top) / line_h).floor().max(0.0) as usize;
                let line_ix = self.scroll_line.floor() as usize + row;
                if line_ix < self.doc.line_count() {
                    let text = self.doc.clone().line(line_ix, MAX_LINE_BYTES).into_owned();
                    let col = self.index_for_x(&text, mx - GUTTER_W * s);
                    self.caret = (line_ix, col);
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::Ime(ime) => {
                match &ime {
                    Ime::Enabled => println!("IME enabled"),
                    Ime::Disabled => println!("IME disabled"),
                    Ime::Preedit(text, cursor) => {
                        println!("IME preedit: {text:?} cursor={cursor:?}");
                        self.preedit = if text.is_empty() {
                            None
                        } else {
                            Some((text.clone(), *cursor))
                        };
                    }
                    Ime::Commit(text) => {
                        println!("IME commit: {text:?}");
                        let at = self.edit_caret.min(self.edit.len());
                        self.edit.insert_str(at, text);
                        self.edit_caret = at + text.len();
                        self.preedit = None;
                        println!("  buffer={:?}", self.edit);
                    }
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.preedit.is_none() {
                    match &event.logical_key {
                        Key::Named(NamedKey::Backspace) => {
                            if self.edit_caret > 0 {
                                let prev = self.edit[..self.edit_caret]
                                    .char_indices()
                                    .next_back()
                                    .map(|(i, _)| i)
                                    .unwrap_or(0);
                                self.edit.replace_range(prev..self.edit_caret, "");
                                self.edit_caret = prev;
                            }
                        }
                        Key::Named(NamedKey::Escape) => el.exit(),
                        _ => {
                            if let Some(t) = &event.text {
                                let at = self.edit_caret.min(self.edit.len());
                                self.edit.insert_str(at, t);
                                self.edit_caret = at + t.len();
                            }
                        }
                    }
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
}

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
        eprintln!("usage: ui-wgpu <file> [--bench-scroll N] [--bench-step N]");
        std::process::exit(2)
    });
    let doc = Arc::new(Doc::open(std::path::Path::new(&path)).expect("open file"));
    println!("DOC_OPEN_MS {:.2}", doc.open_ms);

    let mut app = App {
        doc,
        window: None,
        gfx: None,
        font_cx: FontContext::new(),
        layout_cx: LayoutContext::new(),
        line_cache: HashMap::new(),
        scale: 1.0,
        scroll_line: 0.0,
        caret: (0, 0),
        edit: String::new(),
        edit_caret: 0,
        preedit: None,
        last_cursor: (0.0, 0.0),
        t_main,
        first_frame_ms: None,
        last_frame: None,
        bench: (bench > 0).then(|| Bench {
            target: bench,
            step,
            done: 0,
            intervals: Vec::with_capacity(bench),
            cpu_us: Vec::with_capacity(bench),
        }),
        index_reported: false,
    };

    let el = EventLoop::new().unwrap();
    el.set_control_flow(ControlFlow::Wait);
    el.run_app(&mut app).unwrap();
}
