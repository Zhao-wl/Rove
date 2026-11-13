//! 代码视图的 GPUI `Element`：布局可见行、生成绘制指令、登记鼠标处理。
//!
//! 帧内只读缓存（行显示形式、整形结果、装饰）；任何未命中都画占位并在绘制后调度空闲准备切片。

use super::columns::DisplayLine;
use super::guides;
use super::options::{
    CursorStyle, FinalNewline, FoldingControls, LineHighlight, LineNumbers, ScrollbarVisibility,
};
use super::scroll::{self, Metrics, ScrollPosition};
use super::view::{
    hsla, CodeView, CodeViewEvent, Drag, FrameLayout, FrameStats, LineEntry, Shaped, Viewport,
};
use gpui::{
    fill, outline, point, px, quad, size, App, BorderStyle, Bounds, ContentMask,
    CursorStyle as MouseCursor, DispatchPhase, Element, ElementId, Entity, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, ScrollWheelEvent, Style, Window,
};
use rove_theme::Color;
use std::rc::Rc;
use std::time::Instant;

pub struct CodeElement {
    pub view: Entity<CodeView>,
}

impl IntoElement for CodeElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// 一条可见行的绘制计划。
pub(crate) struct PaintLine {
    pub line: u64,
    /// 行顶边相对文本区顶边的 y（像素）。
    pub y: f32,
    pub entry: Option<Rc<LineEntry>>,
    pub shaped: Option<Rc<Shaped>>,
    /// 首行为本行的折叠区间：(是否已折叠)。
    pub fold: Option<bool>,
}

pub struct Frame {
    lines: Vec<PaintLine>,
    hitbox: Hitbox,
    text_hitbox: Hitbox,
    layout: FrameLayout,
    gutter_widths: (f32, f32, f32),
    focused: bool,
}

impl Element for CodeElement {
    type RequestLayoutState = ();
    type PrepaintState = Frame;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.0).into();
        style.size.height = gpui::relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Frame {
        let view = self.view.clone();
        self.view
            .update(cx, |v, cx| v.layout_frame(bounds, view, window, cx))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        frame: &mut Frame,
        window: &mut Window,
        cx: &mut App,
    ) {
        let view = self.view.clone();
        let focus = self.view.read(cx).focus.clone();
        // #47：输入法与键盘字符经 `EntityInputHandler` 进入代码视图（只在聚焦时登记）。
        window.handle_input(
            &focus,
            gpui::ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        self.view.update(cx, |v, cx| {
            let caret = window.with_content_mask(Some(ContentMask { bounds }), |window| {
                v.paint_frame(frame, window)
            });
            // 插入符区域变了（输入、滚动、缩放、换行）：让候选框按新布局重新定位（ROVE_PATCHES 的要求）。
            if caret != v.last_caret {
                v.last_caret = caret;
                window.invalidate_character_coordinates();
            }
            register_mouse(&view, frame, window, v.link_cursor);
            v.finish_frame(window, cx);
        });
    }
}

fn digits(mut n: u64, buf: &mut [u8; 20]) -> &[u8] {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    &buf[i..]
}

fn alpha(c: Color, a: f32) -> Hsla {
    let mut h = hsla(c);
    h.a *= a;
    h
}

impl CodeView {
    pub(crate) fn layout_frame(
        &mut self,
        bounds: Bounds<Pixels>,
        view: Entity<CodeView>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Frame {
        self.frame_started = Some(Instant::now());
        let focused = self.focus.is_focused(window);
        self.focus_changed(focused, window, cx);
        self.line_number_ready(window);
        let lh = self.font.line_height;
        let digit_w = self
            .font
            .digits
            .as_ref()
            .map_or(self.font.cell_width, |d| d.advance);
        let doc_count = self.text.as_ref().map_or(1, |t| t.lines().known.max(1));
        // 行数随 #8 的索引增长时，折叠映射跟着更新。
        self.folding.set_doc_lines(doc_count);
        let line_count = self.line_count();
        let o = &self.options;

        // 行号栏：字形边距 | 行号 | 行装饰区（含折叠控件）。
        let glyph_w = if o.glyph_margin { 18.0 } else { 0.0 };
        let ln_w = if o.line_numbers == LineNumbers::Off {
            0.0
        } else {
            let n = (doc_count.max(1) as f64).log10().floor() as u32 + 1;
            (n.max(LINE_NUMBERS_MIN_CHARS) as f32 * digit_w).round()
        };
        let controls = o.folding
            && o.show_folding_controls != FoldingControls::Never
            && self.folding_enabled();
        let deco_w = if controls {
            (self.font.cell_width * 1.8).round().max(10.0)
        } else {
            10.0
        };
        let fold_column = if controls {
            (glyph_w + ln_w)..(glyph_w + ln_w + deco_w)
        } else {
            0.0..0.0
        };
        let gutter_w = glyph_w + ln_w + deco_w;

        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let vs = o.vertical_scrollbar_size as f32;
        let hs = o.horizontal_scrollbar_size as f32;
        let beyond = o.scroll_beyond_last_column * self.font.cell_width as f64;
        // 先按“不显示垂直滚动条”估算水平条，再确定垂直条（两者相互影响，取保守值）。
        let text_w_guess = (width - gutter_w - vs).max(0.0) as f64;
        let show_h = match o.horizontal_scrollbar {
            ScrollbarVisibility::Visible => true,
            ScrollbarVisibility::Hidden => false,
            ScrollbarVisibility::Auto => self.content_width + beyond > text_w_guess,
        };
        let hbar_h = if show_h { hs } else { 0.0 };
        let viewport_h = if o.ignore_horizontal_scrollbar_in_content_height {
            height
        } else {
            (height - hbar_h).max(0.0)
        };
        let mut metrics = Metrics {
            line_height: lh as f64,
            viewport_height: viewport_h as f64,
            viewport_width: text_w_guess,
            line_count,
            padding_top: o.padding_top + self.top_inset as f64,
            padding_bottom: o.padding_bottom,
            scroll_beyond_last_line: o.scroll_beyond_last_line,
            content_width: self.content_width,
            beyond_last_column: beyond,
        };
        let show_v = match o.vertical_scrollbar {
            ScrollbarVisibility::Visible => true,
            ScrollbarVisibility::Hidden => false,
            ScrollbarVisibility::Auto => metrics.max_top(&self.zones) > 0.0,
        };
        let vbar_w = if show_v { vs } else { 0.0 };
        let text_w = (width - gutter_w - vbar_w).max(0.0);
        metrics.viewport_width = text_w as f64;

        let gutter = Bounds::new(bounds.origin, size(px(gutter_w), bounds.size.height));
        let text = Bounds::new(
            point(bounds.origin.x + px(gutter_w), bounds.origin.y),
            size(px(text_w), px(viewport_h)),
        );
        let vbar = show_v.then(|| {
            Bounds::new(
                point(bounds.origin.x + px(width - vbar_w), bounds.origin.y),
                size(px(vbar_w), px(viewport_h)),
            )
        });
        let hbar = show_h.then(|| {
            Bounds::new(
                point(text.origin.x, bounds.origin.y + px(height - hbar_h)),
                size(px(text_w), px(hbar_h)),
            )
        });

        // 钳制滚动位置（行数随 #8 的索引增长时也在这里自然更新）。
        self.scroll = self.scroll.clamped(&metrics, &self.zones);
        if self.animating() {
            let view = view.clone();
            window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify()));
        }
        let top = self.current_top().clamp(0.0, metrics.max_top(&self.zones));
        let pos = ScrollPosition {
            top,
            x: self.scroll.x,
        };
        let (first, y0) = pos.first_visible(&metrics, &self.zones);

        // #47：输入法组合行用内嵌了组合串的显示条目。
        let composed = self.composition_line();
        let mut lines = Vec::with_capacity((viewport_h / lh) as usize + 2);
        let mut y = y0 as f32;
        // `view` 是屏幕行（滚动坐标），`line` 是文档行（缓存与绘制内容）。
        let mut view_line = first;
        let mut stats = FrameStats::default();
        while view_line < line_count && y < viewport_h {
            let line = self.doc_line(view_line);
            let entry = match &composed {
                Some((l, e)) if *l == line => Some(e.clone()),
                _ => self.lines.get(&line).cloned(),
            };
            let shaped = entry
                .as_ref()
                .and_then(|e| self.shapes.get(&self.shape_key(e)).cloned());
            match (&entry, &shaped) {
                (None, _) => stats.missing += 1,
                (Some(_), None) => stats.placeholders += 1,
                (Some(_), Some(_)) => stats.shaped += 1,
            }
            let fold = self.folding.header(line).map(|(_, collapsed)| collapsed);
            lines.push(PaintLine {
                line,
                y,
                entry,
                shaped,
                fold,
            });
            y += lh + self.zones.get(view_line) as f32;
            view_line += 1;
        }
        let first_doc = self.doc_line(first);
        stats.visible_lines = lines.len() as u32;
        stats.first_line = first_doc;
        stats.first_line_y = y0 as f32;
        self.last_frame = stats;
        let last = lines.last().map_or(first_doc, |l| l.line);
        self.note_visible(first..view_line.max(first + 1), top);

        let vthumb = vbar.and_then(|b| {
            let t = scroll::thumb(
                f64::from(f32::from(b.size.height)),
                metrics.viewport_lines(),
                metrics.total_lines(&self.zones),
                top,
            )?;
            Some(Bounds::new(
                point(b.origin.x, b.origin.y + px(t.start as f32)),
                size(b.size.width, px(t.len as f32)),
            ))
        });
        let hthumb = hbar.and_then(|b| {
            let t = scroll::thumb(
                f64::from(f32::from(b.size.width)),
                metrics.viewport_width,
                metrics.content_width + metrics.beyond_last_column,
                self.scroll.x,
            )?;
            Some(Bounds::new(
                point(b.origin.x + px(t.start as f32), b.origin.y),
                size(px(t.len as f32), b.size.height),
            ))
        });

        let viewport = Viewport {
            first_line: first_doc,
            last_line: last,
            top,
            scroll_x: self.scroll.x,
            width: text_w,
            height: viewport_h,
        };
        if self.last_viewport != Some(viewport) {
            self.last_viewport = Some(viewport);
            cx.emit(CodeViewEvent::ViewportChanged(viewport));
        }
        self.layout = FrameLayout {
            bounds,
            gutter,
            text,
            vbar,
            hbar,
            vthumb,
            hthumb,
            metrics: Some(metrics),
            fold_column,
        };
        if let Some(start) = self.frame_started {
            self.last_frame.layout = start.elapsed();
        }
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let text_hitbox = window.insert_hitbox(text, HitboxBehavior::Normal);
        Frame {
            lines,
            hitbox,
            text_hitbox,
            layout: self.layout.clone(),
            gutter_widths: (glyph_w, ln_w, deco_w),
            focused,
        }
    }

    /// 某行字素簇边界的 x：有整形结果时借用缓存，占位行按等宽近似现算。
    /// 某行字素簇边界的 x（按本行自己的 stops 计算，见 [`CodeView::xs_for`]）。
    fn line_xs(&self, l: &PaintLine) -> Option<Rc<[f32]>> {
        let entry = l.entry.as_ref()?;
        Some(self.xs_for(entry, l.shaped.as_deref()))
    }

    /// 绘制一帧；返回插入符的窗口坐标（输入法候选框定位用）。
    pub(crate) fn paint_frame(&self, frame: &Frame, window: &mut Window) -> Option<Bounds<Pixels>> {
        let p = &self.palette;
        let o = &self.options;
        let lay = &frame.layout;
        let lh = self.font.line_height;
        let text = lay.text;
        let left = text.origin.x - px(self.scroll.x as f32);
        let sel = self.cursor.selection;
        // #47：组合中插入符在组合串里（组合行是内嵌了组合串的显示条目）。
        let composing = self.composition_columns();
        let comp_line = self.composition.as_ref().map(|c| c.start.line.0);
        let caret_line = comp_line.unwrap_or(sel.active.line.0);
        let mut caret_bounds = None;

        window.paint_quad(fill(lay.bounds, hsla(p.background)));
        window.paint_quad(fill(lay.gutter, hsla(p.gutter_background)));

        // 当前行高亮（选区非空时不画，与 VS Code 一致）。
        let highlight = sel.is_empty()
            && o.render_line_highlight != LineHighlight::None
            && (frame.focused || !o.line_highlight_only_when_focus);
        if highlight {
            if let Some(l) = frame.lines.iter().find(|l| l.line == caret_line) {
                let bg = if frame.focused {
                    p.line_highlight_background
                } else {
                    p.inactive_line_highlight_background
                };
                let mut regions = vec![];
                if matches!(
                    o.render_line_highlight,
                    LineHighlight::Line | LineHighlight::All
                ) {
                    regions.push((text.origin.x, text.size.width));
                }
                if matches!(
                    o.render_line_highlight,
                    LineHighlight::Gutter | LineHighlight::All
                ) {
                    regions.push((lay.gutter.origin.x, lay.gutter.size.width));
                }
                for (x, w) in regions {
                    let b = Bounds::new(point(x, text.origin.y + px(l.y)), size(w, px(lh)));
                    if let Some(bg) = bg {
                        window.paint_quad(fill(b, hsla(bg)));
                    } else if let Some(border) = p.line_highlight_border {
                        window.paint_quad(quad(
                            b,
                            px(0.),
                            gpui::transparent_black(),
                            px(2.),
                            hsla(border),
                            BorderStyle::Solid,
                        ));
                    }
                }
            }
        }

        window.with_content_mask(Some(ContentMask { bounds: text }), |window| {
            // 整个文本区放在一个绘制层里：层内图元共用同一个绘制次序，GPUI 不再为每个字形
            // 做 BoundsTree 插入（逐字形开销从约 0.7 µs 降到约 0.1 µs）。同层内四边形先于
            // 字形绘制，所以背景、选区、占位条都在文字下面；插入符在层外，位于文字之上。
            window.paint_layer(text, |window| {
                for l in &frame.lines {
                    let y = text.origin.y + px(l.y);
                    let Some(entry) = &l.entry else { continue };
                    let Some(xs) = self.line_xs(l) else { continue };
                    // 整行装饰。
                    if let Some(m) = &entry.merged {
                        let b = Bounds::new(point(text.origin.x, y), size(text.size.width, px(lh)));
                        if let Some(bg) = m.line.background {
                            window.paint_quad(fill(b, hsla(bg)));
                        }
                        if let Some(border) = m.line.border {
                            window.paint_quad(outline(b, hsla(border), BorderStyle::Solid));
                        }
                    }
                    // 已折叠的首行：`editor.foldBackground`（`editor.foldingHighlight`）。
                    if l.fold == Some(true) && o.folding_highlight {
                        if let Some(bg) = p.fold_background {
                            let b =
                                Bounds::new(point(text.origin.x, y), size(text.size.width, px(lh)));
                            window.paint_quad(fill(b, hsla(bg)));
                        }
                    }
                    if o.guides_indentation {
                        self.paint_indent_guides(l, entry, &xs, left, y, frame, window);
                    }
                    // 选区。
                    if !sel.is_empty()
                        && composing.is_none()
                        && l.line >= sel.start().line.0
                        && l.line <= sel.end().line.0
                    {
                        let a = if l.line == sel.start().line.0 {
                            entry.display.x_for_source(&xs, sel.start().column.0 as u32)
                        } else {
                            0.0
                        };
                        let mut b = if l.line == sel.end().line.0 {
                            entry.display.x_for_source(&xs, sel.end().column.0 as u32)
                        } else {
                            *xs.last().unwrap_or(&0.0) + self.font.cell_width * 0.5
                        };
                        if b <= a && l.line != sel.end().line.0 {
                            b = a + self.font.cell_width * 0.5;
                        }
                        if b > a {
                            let color = if frame.focused {
                                p.selection
                            } else {
                                p.inactive_selection
                            };
                            let r = if o.rounded_selection { px(3.) } else { px(0.) };
                            window.paint_quad(quad(
                                Bounds::new(point(left + px(a), y), size(px(b - a), px(lh))),
                                r,
                                hsla(color),
                                px(0.),
                                gpui::transparent_black(),
                                BorderStyle::Solid,
                            ));
                        }
                    }
                    // 装饰区间的背景、外框。
                    for (range, style) in &entry.paint_runs {
                        if style.background.is_none() && style.border.is_none() {
                            continue;
                        }
                        let a = DisplayLine::x_at(
                            &xs,
                            entry.display.stop_index_for_display(range.start),
                        );
                        let b =
                            DisplayLine::x_at(&xs, entry.display.stop_index_for_display(range.end));
                        let r =
                            Bounds::new(point(left + px(a), y), size(px((b - a).max(1.0)), px(lh)));
                        if let Some(bg) = style.background {
                            window.paint_quad(fill(r, hsla(bg)));
                        }
                        if let Some(border) = style.border {
                            window.paint_quad(outline(r, hsla(border), BorderStyle::Solid));
                        }
                    }
                    match &l.shaped {
                        Some(s) => self.paint_glyphs(l, entry, s, left, y, &xs, window),
                        None => self.paint_placeholder(entry, left, y, &xs, text, window),
                    }
                    self.paint_whitespace(l, entry, &xs, left, y, window);
                    if l.fold == Some(true) {
                        self.paint_fold_placeholder(&xs, left, y, window);
                    }
                    // 下划线、删除线。
                    for (range, style) in &entry.paint_runs {
                        let a = DisplayLine::x_at(
                            &xs,
                            entry.display.stop_index_for_display(range.start),
                        );
                        let b =
                            DisplayLine::x_at(&xs, entry.display.stop_index_for_display(range.end));
                        if let Some(u) = style.underline {
                            let r = Bounds::new(
                                point(left + px(a), y + px(lh - 2.0)),
                                size(px(b - a), px(1.)),
                            );
                            window.paint_quad(fill(r, hsla(u)));
                        }
                        if let Some(s) = style.strikethrough {
                            let r = Bounds::new(
                                point(left + px(a), y + px(lh / 2.0)),
                                size(px(b - a), px(1.)),
                            );
                            window.paint_quad(fill(r, hsla(s)));
                        }
                    }
                    // #47：组合串下划线（`editor.compositionBorder`）。
                    if let (Some((cols, _)), Some(cl)) = (&composing, comp_line) {
                        if cl == l.line {
                            let a = entry.display.x_for_source(&xs, cols.start);
                            let b = entry.display.x_for_source(&xs, cols.end);
                            let color = p.composition_border.unwrap_or(p.foreground);
                            let r = Bounds::new(
                                point(left + px(a), y + px(lh - 2.0)),
                                size(px((b - a).max(1.0)), px(1.)),
                            );
                            window.paint_quad(fill(r, hsla(color)));
                        }
                    }
                    // 行内覆盖物。
                    if let Some(m) = &entry.merged {
                        for ov in &m.overlays {
                            let key = self.overlay_key(&ov.text);
                            if let Some(shaped) = self.shapes.peek(&key).cloned() {
                                let x = entry.display.x_for_source(&xs, ov.column);
                                let color = ov.style.foreground.unwrap_or(p.foreground);
                                self.paint_layout(
                                    &shaped.layout,
                                    left + px(x),
                                    y,
                                    hsla(color),
                                    window,
                                );
                            }
                        }
                    }
                }
            });
            // 插入符（组合期间不闪烁；改写模式用 `editor.overtypeCursorStyle`）。
            if frame.focused && (self.caret_visible || composing.is_some()) {
                if let Some(l) = frame.lines.iter().find(|l| l.line == caret_line) {
                    if let (Some(entry), Some(xs)) = (&l.entry, self.line_xs(l)) {
                        let column = composing
                            .as_ref()
                            .map_or(sel.active.column.0 as u32, |(_, c)| *c);
                        let x = entry.display.x_for_source(&xs, column);
                        let next = {
                            let i = entry.display.stop_index_for_source(column);
                            xs.get(i + 1).copied().unwrap_or(x + self.font.cell_width)
                        };
                        let cell = (next - x).max(self.font.cell_width * 0.5);
                        let y = text.origin.y + px(l.y);
                        let caret = hsla(p.cursor);
                        let style = if self.overtype && composing.is_none() {
                            o.overtype_cursor_style
                        } else {
                            o.cursor_style
                        };
                        let r = match style {
                            CursorStyle::Line | CursorStyle::LineThin => {
                                let h = if o.cursor_height > 0.0 {
                                    (o.cursor_height as f32).min(lh)
                                } else {
                                    lh
                                };
                                Bounds::new(
                                    point(left + px(x), y + px((lh - h) / 2.0)),
                                    size(px(o.caret_width() as f32), px(h)),
                                )
                            }
                            CursorStyle::Block | CursorStyle::BlockOutline => {
                                Bounds::new(point(left + px(x), y), size(px(cell), px(lh)))
                            }
                            CursorStyle::Underline => Bounds::new(
                                point(left + px(x), y + px(lh - 2.0)),
                                size(px(cell), px(2.)),
                            ),
                            CursorStyle::UnderlineThin => Bounds::new(
                                point(left + px(x), y + px(lh - 1.0)),
                                size(px(cell), px(1.)),
                            ),
                        };
                        if style == CursorStyle::BlockOutline {
                            window.paint_quad(outline(r, caret, BorderStyle::Solid));
                        } else {
                            window.paint_quad(fill(r, caret));
                        }
                        caret_bounds =
                            Some(Bounds::new(point(left + px(x), y), size(px(cell), px(lh))));
                    }
                }
            }
        });

        self.paint_gutter(frame, window);
        self.paint_scrollbars(frame, window);
        caret_bounds
    }

    /// 缩进参考线：每个缩进单位一条 1 px 竖线，颜色按层级取 `editorIndentGuide.background1`–`6`。
    #[allow(clippy::too_many_arguments)]
    fn paint_indent_guides(
        &self,
        l: &PaintLine,
        entry: &LineEntry,
        xs: &[f32],
        left: Pixels,
        y: Pixels,
        frame: &Frame,
        window: &mut Window,
    ) {
        let tab = self.options.tab_size;
        let levels = match guides::indent_columns(&entry.source, tab) {
            Some(cols) => guides::levels(cols, tab),
            None => {
                // 空行：看上下最近的有内容行（只看已缓存的行，不在帧内读文本）。
                let indent_of = |line: u64| {
                    self.lines
                        .peek(&line)
                        .and_then(|e| guides::indent_columns(&e.source, tab))
                        .map(|c| guides::levels(c, tab))
                };
                let doc_count = self.doc_line_count();
                let above = (l.line.saturating_sub(GUIDE_SEARCH)..l.line)
                    .rev()
                    .find_map(indent_of);
                let below =
                    (l.line + 1..(l.line + GUIDE_SEARCH).min(doc_count)).find_map(indent_of);
                let off_side = false;
                guides::blank_line_levels(above, below, off_side)
            }
        };
        let _ = frame;
        let stops = entry.display.stops();
        for level in 0..levels {
            let Some(color) = self.palette.indent_guides[level as usize % 6] else {
                continue;
            };
            let col = level * tab;
            let x = stops
                .iter()
                .position(|s| s.col == col)
                .filter(|&i| i < xs.len())
                .map_or(col as f32 * self.font.cell_width, |i| xs[i]);
            let r = Bounds::new(
                point(left + px(x.round()), y),
                size(px(1.0), px(self.font.line_height)),
            );
            window.paint_quad(fill(r, hsla(color)));
        }
    }

    /// `editor.renderWhitespace`：空格画居中的点，Tab 画一条横线。
    fn paint_whitespace(
        &self,
        l: &PaintLine,
        entry: &LineEntry,
        xs: &[f32],
        left: Pixels,
        y: Pixels,
        window: &mut Window,
    ) {
        let mode = self.options.render_whitespace;
        if mode == guides::RenderWhitespace::None {
            return;
        }
        let Some(color) = self.palette.whitespace else {
            return;
        };
        let sel = self.cursor.selection;
        let selection = (!sel.is_empty()
            && l.line >= sel.start().line.0
            && l.line <= sel.end().line.0)
            .then(|| {
                let a = if l.line == sel.start().line.0 {
                    sel.start().column.0 as u32
                } else {
                    0
                };
                let b = if l.line == sel.end().line.0 {
                    sel.end().column.0 as u32
                } else {
                    u32::MAX
                };
                a..b
            });
        let lh = self.font.line_height;
        let c = hsla(color);
        for (i, tab) in guides::whitespace_marks(&entry.source, &entry.display, mode, selection) {
            let (a, b) = (DisplayLine::x_at(xs, i), DisplayLine::x_at(xs, i + 1));
            let r = if tab {
                Bounds::new(
                    point(left + px(a + 2.0), y + px((lh / 2.0).round())),
                    size(px((b - a - 4.0).max(1.0)), px(1.0)),
                )
            } else {
                let d = 2.0;
                Bounds::new(
                    point(
                        left + px(((a + b) / 2.0 - d / 2.0).round()),
                        y + px((lh - d) / 2.0),
                    ),
                    size(px(d), px(d)),
                )
            };
            window.paint_quad(fill(r, c));
        }
    }

    /// 已折叠首行行尾的“⋯”占位标记（`editor.foldPlaceholderForeground`）。
    fn paint_fold_placeholder(&self, xs: &[f32], left: Pixels, y: Pixels, window: &mut Window) {
        let p = &self.palette;
        let color = hsla(p.fold_placeholder_foreground.unwrap_or(p.foreground));
        let end = *xs.last().unwrap_or(&0.0);
        let (a, b) = self.placeholder_x(end);
        let lh = self.font.line_height;
        let d = (self.font.cell_width * 0.22).max(2.0);
        let step = (b - a) / 3.0;
        for k in 0..3 {
            let cx = a + step * (k as f32 + 0.5);
            let r = Bounds::new(
                point(left + px(cx - d / 2.0), y + px((lh - d) / 2.0)),
                size(px(d), px(d)),
            );
            window.paint_quad(fill(r, color));
        }
    }

    /// 行号栏的折叠控件：折叠的显示 ›，展开的显示 ⌄（`mouseover` 时只在鼠标悬停行号栏时显示）。
    fn paint_fold_control(
        &self,
        lay: &FrameLayout,
        y: Pixels,
        collapsed: bool,
        window: &mut Window,
    ) {
        if lay.fold_column.is_empty() {
            return;
        }
        let show = collapsed
            || match self.options.show_folding_controls {
                FoldingControls::Always => true,
                FoldingControls::Mouseover => self.hover_gutter,
                FoldingControls::Never => false,
            };
        if !show {
            return;
        }
        let lh = self.font.line_height;
        let cx = lay.gutter.origin.x + px((lay.fold_column.start + lay.fold_column.end) / 2.0);
        let cy = y + px(lh / 2.0);
        let r = (lh * 0.18).max(2.5);
        let mut path = PathBuilder::stroke(px(1.2));
        if collapsed {
            path.move_to(point(cx - px(r * 0.5), cy - px(r)));
            path.line_to(point(cx + px(r * 0.5), cy));
            path.line_to(point(cx - px(r * 0.5), cy + px(r)));
        } else {
            path.move_to(point(cx - px(r), cy - px(r * 0.5)));
            path.line_to(point(cx, cy + px(r * 0.5)));
            path.line_to(point(cx + px(r), cy - px(r * 0.5)));
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, hsla(self.palette.folding_control_foreground));
        }
    }

    fn baseline(&self, y: Pixels) -> Pixels {
        let lh = self.font.line_height;
        let (ascent, descent) = self.font.digits.as_ref().map_or(
            (self.font.cell_width * 1.6, self.font.cell_width * 0.4),
            |d| (d.ascent, d.descent),
        );
        y + px((lh - ascent - descent) / 2.0 + ascent)
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_glyphs(
        &self,
        l: &PaintLine,
        entry: &LineEntry,
        s: &Shaped,
        left: Pixels,
        y: Pixels,
        xs: &[f32],
        window: &mut Window,
    ) {
        let p = &self.palette;
        let baseline = self.baseline(y);
        let text = self.layout.text;
        let min_x = f32::from(text.origin.x - left) - self.font.cell_width * 4.0;
        let max_x = f32::from(text.origin.x + text.size.width - left);
        let spacing = self.font.letter_spacing;
        let sel = self.cursor.selection;
        let sel_disp = (p.selection_foreground.is_some()
            && !sel.is_empty()
            && l.line >= sel.start().line.0
            && l.line <= sel.end().line.0)
            .then(|| {
                let a = if l.line == sel.start().line.0 {
                    entry
                        .display
                        .display_for_source(sel.start().column.0 as u32)
                } else {
                    0
                };
                let b = if l.line == sel.end().line.0 {
                    entry.display.display_for_source(sel.end().column.0 as u32)
                } else {
                    u32::MAX
                };
                a..b
            });
        let _ = xs;
        let size = self.font.size;
        let runs = &entry.paint_runs;
        let bytes = entry.display.text.as_bytes();
        // 相邻字形几乎总是同色：记住上一次的颜色换算结果。
        let mut memo = (p.foreground, hsla(p.foreground));
        for run in &s.layout.runs {
            let start = run
                .glyphs
                .partition_point(|g| f32::from(g.position.x) < min_x);
            let mut ri = 0usize;
            for g in &run.glyphs[start..] {
                let gx = f32::from(g.position.x);
                if gx > max_x {
                    break;
                }
                let idx = g.index as u32;
                // 空格（含 Tab 展开出的空格）没有可见字形，跳过以省下 GPUI 的逐字形开销。
                if bytes.get(g.index) == Some(&b' ') {
                    continue;
                }
                while ri < runs.len() && runs[ri].0.end <= idx {
                    ri += 1;
                }
                let mut color = match runs.get(ri) {
                    Some((r, st)) if r.start <= idx => st.foreground.unwrap_or(p.foreground),
                    _ => p.foreground,
                };
                if let Some(r) = &sel_disp {
                    if r.contains(&idx) {
                        color = p.selection_foreground.unwrap_or(color);
                    }
                }
                let dx = if spacing != 0.0 {
                    spacing * entry.display.stop_index_for_display(idx) as f32
                } else {
                    0.0
                };
                self.glyphs_painted.set(self.glyphs_painted.get() + 1);
                let origin = point(left + px(gx + dx), baseline + g.position.y);
                if memo.0 != color {
                    memo = (color, hsla(color));
                }
                let _ = if g.is_emoji {
                    window.paint_emoji(origin, run.font_id, g.id, size)
                } else {
                    window.paint_glyph(origin, run.font_id, g.id, size, memo.1)
                };
            }
        }
    }

    fn paint_layout(
        &self,
        layout: &gpui::LineLayout,
        x: Pixels,
        y: Pixels,
        color: Hsla,
        window: &mut Window,
    ) {
        let baseline = self.baseline(y);
        for run in &layout.runs {
            for g in &run.glyphs {
                let origin = point(x + g.position.x, baseline + g.position.y);
                let _ = if g.is_emoji {
                    window.paint_emoji(origin, run.font_id, g.id, self.font.size)
                } else {
                    window.paint_glyph(origin, run.font_id, g.id, self.font.size, color)
                };
            }
        }
    }

    /// 等宽近似占位：每段连续的非空白字素画一条淡色条，宽度按可见列估算。
    fn paint_placeholder(
        &self,
        entry: &LineEntry,
        left: Pixels,
        y: Pixels,
        xs: &[f32],
        text: Bounds<Pixels>,
        window: &mut Window,
    ) {
        let lh = self.font.line_height;
        let color = alpha(self.palette.foreground, 0.25);
        let stops = entry.display.stops();
        let bytes = entry.display.text.as_bytes();
        let max_x = f32::from(text.origin.x + text.size.width - left);
        let mut run_start: Option<usize> = None;
        for i in 0..stops.len() {
            let blank = i + 1 >= stops.len() || bytes[stops[i].disp as usize] == b' ';
            match (run_start, blank) {
                (None, false) => run_start = Some(i),
                (Some(s), true) => {
                    let (a, b) = (DisplayLine::x_at(xs, s), DisplayLine::x_at(xs, i));
                    if a > max_x {
                        break;
                    }
                    let r = Bounds::new(
                        point(left + px(a), y + px(lh * 0.3)),
                        size(px((b - a).max(1.0)), px(lh * 0.4)),
                    );
                    window.paint_quad(fill(r, color));
                    run_start = None;
                }
                _ => {}
            }
        }
    }

    fn paint_gutter(&self, frame: &Frame, window: &mut Window) {
        let o = &self.options;
        let p = &self.palette;
        let lay = &frame.layout;
        let Some(d) = self.font.digits.clone() else {
            return;
        };
        let (glyph_w, ln_w, _) = frame.gutter_widths;
        let caret_line = self.cursor.selection.active.line.0;
        let line_count = self.text.as_ref().map_or(1, |t| t.lines().known.max(1));
        let complete = self.text.as_ref().is_none_or(|t| t.lines().complete);
        window.with_content_mask(Some(ContentMask { bounds: lay.gutter }), |window| {
            window.paint_layer(lay.gutter, |window| {
                let mut buf = [0u8; 20];
                for l in &frame.lines {
                    let y = lay.text.origin.y + px(l.y);
                    // 字形边距图标：以首字母画一个徽记（图标资源由提供方 #40 等接入后替换）。
                    if glyph_w > 0.0 {
                        if let Some(icon) = l
                            .entry
                            .as_ref()
                            .and_then(|e| e.merged.as_ref())
                            .and_then(|m| m.line.glyph.clone())
                        {
                            let c = icon.color.unwrap_or(p.foreground);
                            let r = Bounds::new(
                                point(
                                    lay.gutter.origin.x + px(4.0),
                                    y + px(self.font.line_height / 2.0 - 5.0),
                                ),
                                size(px(10.0), px(10.0)),
                            );
                            window.paint_quad(quad(
                                r,
                                px(5.0),
                                hsla(c),
                                px(0.),
                                gpui::transparent_black(),
                                BorderStyle::Solid,
                            ));
                        }
                    }
                    if let Some(collapsed) = l.fold {
                        self.paint_fold_control(lay, y, collapsed, window);
                    }
                    if ln_w <= 0.0 {
                        continue;
                    }
                    let final_newline = complete
                        && line_count > 1
                        && l.line + 1 == line_count
                        && l.entry.as_ref().is_some_and(|e| e.source.is_empty());
                    if final_newline && o.render_final_newline == FinalNewline::Off {
                        continue;
                    }
                    let number = match o.line_numbers {
                        LineNumbers::Relative if l.line != caret_line => {
                            l.line.abs_diff(caret_line)
                        }
                        LineNumbers::Interval if l.line != caret_line && (l.line + 1) % 10 != 0 => {
                            continue
                        }
                        _ => l.line + 1,
                    };
                    let color = if final_newline && o.render_final_newline == FinalNewline::Dimmed {
                        p.line_number_dimmed
                    } else if l.line == caret_line {
                        p.line_number_active
                    } else {
                        p.line_number
                    };
                    let ds = digits(number, &mut buf);
                    let right = lay.gutter.origin.x + px(glyph_w + ln_w);
                    let baseline = self.baseline(y);
                    for (k, digit) in ds.iter().enumerate() {
                        let Some(glyph) = d.glyphs[*digit as usize] else {
                            continue;
                        };
                        let x = right - px((ds.len() - k) as f32 * d.advance);
                        let _ = window.paint_glyph(
                            point(x, baseline),
                            d.font_id,
                            glyph,
                            self.font.size,
                            hsla(color),
                        );
                    }
                }
            });
        });
    }

    fn paint_scrollbars(&self, frame: &Frame, window: &mut Window) {
        let p = &self.palette;
        let lay = &frame.layout;
        if let (Some(shadow), true) = (p.scrollbar_shadow, self.current_top() > 0.0) {
            let r = Bounds::new(lay.text.origin, size(lay.text.size.width, px(3.0)));
            window.paint_quad(fill(r, alpha(shadow, 0.6)));
        }
        let slider = |active: bool, hover: bool| {
            if active {
                p.slider_active
            } else if hover {
                p.slider_hover
            } else {
                p.slider
            }
        };
        if let Some(t) = lay.vthumb {
            let c = slider(
                matches!(self.drag, Some(Drag::VThumb { .. })),
                self.hover_vthumb,
            );
            window.paint_quad(fill(t, hsla(c)));
        }
        if let Some(t) = lay.hthumb {
            let c = slider(
                matches!(self.drag, Some(Drag::HThumb { .. })),
                self.hover_hthumb,
            );
            window.paint_quad(fill(t, hsla(c)));
        }
    }

    pub(crate) fn finish_frame(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if let Some(start) = self.frame_started.take() {
            self.last_frame.cpu = start.elapsed();
            self.last_frame.glyphs = self.glyphs_painted.replace(0);
            if let Some(rec) = self.recording.as_mut() {
                rec.push(self.last_frame);
            }
        }
        if self.needs_prepare() {
            self.schedule_prepare(window, cx);
        }
    }
}

fn register_mouse(view: &Entity<CodeView>, frame: &Frame, window: &mut Window, link: bool) {
    let lay = frame.layout.clone();
    // #40：`Ctrl+悬停` 在可跳转的标识符上时显示手形光标。
    let cursor = if link {
        MouseCursor::PointingHand
    } else {
        MouseCursor::IBeam
    };
    window.set_cursor_style(cursor, &frame.text_hitbox);
    let hitbox = frame.hitbox.clone();
    let v = view.clone();
    window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble
            || e.button != MouseButton::Left
            || !hitbox.is_hovered(window)
        {
            return;
        }
        v.update(cx, |v, cx| {
            window.focus(&v.focus);
            let p = e.position;
            if lay.vbar.is_some_and(|b| b.contains(&p)) {
                v.scrollbar_down(p, true, cx);
            } else if lay.hbar.is_some_and(|b| b.contains(&p)) {
                v.scrollbar_down(p, false, cx);
            } else if lay.gutter.contains(&p) {
                v.mouse_down_gutter(p, e.modifiers.shift, cx);
            } else if e.modifiers.control && !e.modifiers.shift && e.click_count == 1 {
                // #40：Ctrl+点击转到定义（Ctrl+Alt+点击在旁边打开）。
                v.mouse_down_link(p, e.modifiers.alt, cx);
            } else {
                v.mouse_down_text(p, e.click_count, e.modifiers.shift, cx);
            }
        });
        cx.stop_propagation();
    });
    let v = view.clone();
    let lay = frame.layout.clone();
    let text_hitbox = frame.text_hitbox.clone();
    window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        // #40：悬停与 `Ctrl+悬停` 的指针位置（拖动时不报）。
        if e.pressed_button.is_none() {
            let inside = text_hitbox.is_hovered(window).then_some(e.position);
            v.update(cx, |v, cx| v.pointer_moved(inside, e.modifiers.control, cx));
        }
        v.update(cx, |v, cx| match v.drag {
            Some(Drag::Text { .. }) if e.pressed_button == Some(MouseButton::Left) => {
                v.mouse_drag_text(e.position, cx)
            }
            Some(Drag::VThumb { .. } | Drag::HThumb { .. })
                if e.pressed_button == Some(MouseButton::Left) =>
            {
                v.scrollbar_drag(e.position, cx)
            }
            _ => {
                let hv = lay.vthumb.is_some_and(|b| b.contains(&e.position));
                let hh = lay.hthumb.is_some_and(|b| b.contains(&e.position));
                let hg = lay.gutter.contains(&e.position);
                if hv != v.hover_vthumb || hh != v.hover_hthumb || hg != v.hover_gutter {
                    v.hover_vthumb = hv;
                    v.hover_hthumb = hh;
                    v.hover_gutter = hg;
                    cx.notify();
                }
            }
        });
    });
    let v = view.clone();
    window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
            v.update(cx, |v, cx| v.mouse_up(cx));
        }
    });
    let v = view.clone();
    let hitbox = frame.hitbox.clone();
    window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
            return;
        }
        v.update(cx, |v, cx| v.wheel(e.delta, e.modifiers, cx));
        cx.stop_propagation();
    });
}

const LINE_NUMBERS_MIN_CHARS: u32 = 5;
/// 空行的缩进参考线向上下各找多少行有内容的行。
const GUIDE_SEARCH: u64 = 64;
