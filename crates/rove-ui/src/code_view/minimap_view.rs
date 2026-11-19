//! 缩略图的布局、后台分块与输入处理。

#[cfg(test)]
#[path = "minimap_view_tests.rs"]
mod tests;

use super::{
    columns::{self, DisplayLine},
    highlight::SyntaxLayer,
    minimap::{
        self, Autohide, MarkHeaders, Marker, MarkerKind, Sampling, SectionHeader, Tile, TileCache,
        TILE_ROWS,
    },
    scroll::{self, Metrics, ScrollPosition, Thumb, ViewZones},
    view::{hsla, Drag},
    wrap::{self, WrapOptions},
    CodeView, Reveal,
};
use gpui::{
    fill, point, px, size, App, Bounds, ContentMask, Context, Corners, Pixels, Point, Task, Window,
};
use rove_buffer::{SizeClass, TextSource};
use rove_theme::Color;
use std::{collections::BTreeMap, ops::Range, sync::Arc, time::Duration};

#[derive(Clone, Copy)]
struct Track {
    sampling: Sampling,
    height: f64,
    offset: f64,
}

impl Track {
    fn new(sampling: Sampling, height: f64, top: f64, max_top: f64) -> Self {
        let capacity = sampling.raster_rows as f64 * sampling.row_height;
        let progress = if max_top > 0.0 {
            (top / max_top).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Self {
            sampling,
            height,
            offset: (capacity - height).max(0.0) * progress,
        }
    }

    fn tiles(self) -> Range<u64> {
        let per_row = self.sampling.row_height;
        let start = (self.offset / per_row).floor() as u64;
        let end =
            (((self.offset + self.height) / per_row).ceil() as u64).min(self.sampling.raster_rows);
        minimap::tile_range(start..end)
    }

    fn thumb(self, metrics: &Metrics, zones: &ViewZones, top: f64) -> Option<Thumb> {
        let total = metrics.total_lines(zones);
        scroll::thumb(self.height, metrics.viewport_lines(), total, top)
    }
}

#[derive(Clone, Copy)]
struct Panel {
    bounds: Bounds<Pixels>,
    track: Track,
}

struct Overview {
    key: (usize, u64, u64),
    bounds: Bounds<Pixels>,
    bands: Vec<(usize, Range<usize>, Color)>,
}

struct CancelOnDrop(rove_base::Cancellation);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct Header {
    row: u64,
    title: SectionHeader,
    shape: Option<gpui::ShapedLine>,
}

#[derive(Default)]
pub(crate) struct MinimapView {
    headers: BTreeMap<u64, Vec<Header>>,
    external_marks: BTreeMap<&'static str, Vec<Marker>>,
    visual_marks: BTreeMap<MarkerKind, Vec<Range<u64>>>,
    marks_dirty: bool,
    mapping_dirty: bool,
    occurrence_key: Option<(u64, rove_base::Version, bool)>,
    occurrence_task: Option<(CancelOnDrop, Task<()>)>,
    overview: Option<Overview>,
    panel: Option<Panel>,
    cache: TileCache,
    task: Option<(u64, Task<()>)>,
    hovered: bool,
    last_top: f64,
    revealed_on_scroll: bool,
    fade: Option<Task<()>>,
}

impl MinimapView {
    pub(crate) fn clear(&mut self, cx: &mut App) {
        self.task = None;
        self.headers.clear();
        for image in self.cache.clear() {
            cx.drop_image(image, None);
        }
    }

    fn invalidate(&mut self, rows: Range<u64>, cx: &mut App) {
        let tiles = minimap::tile_range(rows.clone());
        if self
            .task
            .as_ref()
            .is_some_and(|(index, _)| tiles.contains(index))
        {
            self.task = None;
        }
        self.headers.retain(|index, _| !tiles.contains(index));
        for image in self.cache.invalidate(rows) {
            cx.drop_image(image, None);
        }
    }
}

struct TileJob {
    text: Arc<dyn TextSource>,
    syntax: SyntaxLayer,
    columns: u32,
    row_info: Vec<(u64, u32, bool)>,
    marker_pattern: Option<Arc<str>>,
    region_start: Option<rove_syntax::language_config::CompiledRegex>,
    wrap: Option<(u32, WrapOptions)>,
    tab_size: u32,
    render_limit: Option<usize>,
    foreground: Color,
    opacity: u8,
}

impl TileJob {
    fn run(self) -> (Arc<gpui::RenderImage>, Vec<(u64, SectionHeader)>) {
        let mut tile = Tile::new(self.columns, self.row_info.len() as u32);
        let mut decorations = Vec::new();
        let mut comments = Vec::new();
        let mut headers = Vec::new();
        let matcher = self.marker_pattern.and_then(MarkHeaders::compile);
        for (index, (line, segment, region)) in self.row_info.into_iter().enumerate() {
            let Some(source) = columns::read_line(&*self.text, line) else {
                continue;
            };
            if segment == 0 {
                let region_title = self
                    .region_start
                    .as_ref()
                    .filter(|_| region)
                    .and_then(|start| minimap::region_header(&source, start));
                if let Some(title) = region_title.or_else(|| {
                    let matcher = matcher.as_ref()?;
                    comments.clear();
                    self.syntax.comment_ranges(line, &mut comments);
                    matcher.find(&source, comments.iter().cloned())
                }) {
                    headers.push((index as u64, title));
                }
            }
            let mut range = 0..source.len() as u32;
            let mut indent = 0;
            if let Some((limit, options)) = &self.wrap {
                let display = DisplayLine::build(&source, self.tab_size, self.render_limit);
                let wraps = wrap::wrap_line(
                    &source,
                    &display,
                    wrap::Metric::Columns { limit: *limit },
                    options,
                );
                let Some(actual_segment) = (segment < wraps.count()).then_some(segment) else {
                    continue;
                };
                range = wraps.segment_range(actual_segment, display.rendered_source_len());
                indent = wraps.indent_columns(actual_segment);
            }
            let Some(segment_text) = source.get(range.start as usize..range.end as usize) else {
                continue;
            };
            decorations.clear();
            self.syntax.styles(line, &source, &mut decorations);
            let spans = decorations.iter().filter_map(|part| {
                let start = part.range.start.max(range.start);
                let end = part.range.end.min(range.end);
                (start < end)
                    .then(|| {
                        (
                            start - range.start + indent..end - range.start + indent,
                            part.style.foreground,
                        )
                    })
                    .and_then(|(range, color)| color.map(|color| (range, color)))
            });
            let text = if indent == 0 {
                std::borrow::Cow::Borrowed(segment_text)
            } else {
                std::borrow::Cow::Owned(" ".repeat(indent as usize) + segment_text)
            };
            tile.paint_row(
                index as u32,
                &text,
                spans,
                self.foreground,
                self.opacity,
                self.tab_size,
            );
        }
        (tile.into_image(), headers)
    }
}

impl CodeView {
    /// 各装饰来源提交文档行区间，缩略图与概览标尺共享这份位置数据。
    pub fn set_minimap_markers(
        &mut self,
        source: &'static str,
        markers: Vec<Marker>,
        cx: &mut Context<Self>,
    ) {
        if self.minimap.external_marks.get(source) == Some(&markers) {
            return;
        }
        if markers.is_empty() {
            self.minimap.external_marks.remove(source);
        } else {
            self.minimap.external_marks.insert(source, markers);
        }
        self.minimap.marks_dirty = true;
        cx.notify();
    }

    pub(crate) fn invalidate_minimap_mapping(&mut self) {
        self.minimap.mapping_dirty = true;
        self.invalidate_minimap_marks();
    }

    pub(crate) fn invalidate_minimap_marks(&mut self) {
        self.minimap.marks_dirty = true;
    }

    pub(crate) fn clear_minimap_document_marks(&mut self) {
        self.minimap.external_marks.clear();
        self.minimap.occurrence_task = None;
        self.minimap.occurrence_key = None;
        self.minimap.marks_dirty = true;
    }

    fn update_minimap_marks(&mut self) {
        let selections = self.selections();
        let mut local = Vec::new();
        for selection in selections {
            if !self.options.minimap.ruler_hide_cursor {
                let line = selection.active.line.0;
                local.push(Marker {
                    lines: line..line.saturating_add(1),
                    kind: MarkerKind::Cursor,
                });
            }
            if !selection.is_empty() {
                let first = selection.start();
                let last = selection.end();
                let end = last.line.0 + u64::from(last.column.0 != 0 || first.line == last.line);
                local.push(Marker {
                    lines: first.line.0..end,
                    kind: MarkerKind::Selection,
                });
            }
        }
        if let Some(brackets) = self.bracket_match() {
            for line in [brackets.open.0, brackets.close.0] {
                local.push(Marker {
                    lines: line..line.saturating_add(1),
                    kind: MarkerKind::Bracket,
                });
            }
        }
        let mut visual: BTreeMap<MarkerKind, Vec<Range<u64>>> = BTreeMap::new();
        for marker in local
            .iter()
            .chain(self.minimap.external_marks.values().flatten())
        {
            if marker.lines.is_empty() || marker.lines.start >= self.doc_line_count() {
                continue;
            }
            let end = (marker.lines.end - 1).min(self.doc_line_count() - 1);
            let last = self.visual_map().visual_range(end);
            let start = self.view_line(marker.lines.start);
            visual
                .entry(marker.kind)
                .or_default()
                .push(start..last.end.max(start + 1));
        }
        for ranges in visual.values_mut() {
            ranges.sort_unstable_by_key(|range| range.start);
            let mut write = 0;
            for read in 0..ranges.len() {
                let range = ranges[read].clone();
                if write > 0 && ranges[write - 1].end >= range.start {
                    ranges[write - 1].end = ranges[write - 1].end.max(range.end);
                } else {
                    ranges[write] = range;
                    write += 1;
                }
            }
            ranges.truncate(write);
        }
        self.minimap.visual_marks = visual;
        self.minimap.marks_dirty = false;
    }

    pub(crate) fn update_overview(&mut self, bounds: Option<Bounds<Pixels>>, scale: f32) {
        let Some((bounds, metrics)) = bounds.zip(self.metrics()) else {
            self.minimap.overview = None;
            return;
        };
        let scale = f64::from(scale);
        let height = (bounds.size.height.to_f64() * scale).round() as usize;
        let key = (
            height,
            scale.to_bits(),
            metrics.total_lines(&self.zones).to_bits(),
        );
        if !self.minimap.marks_dirty {
            if let Some(overview) = self.minimap.overview.as_mut().filter(|o| o.key == key) {
                overview.bounds = bounds;
                return;
            }
        } else {
            self.update_minimap_marks();
        }
        let mut raster = minimap::RulerRaster::new(height);
        for (&kind, ranges) in &self.minimap.visual_marks {
            if let Some(color) = self.minimap_color(kind.style().ruler) {
                for range in ranges {
                    let y = |line| {
                        scroll::track_y_for_line(height as f64, line, &metrics, &self.zones).round()
                            as usize
                    };
                    let start = y(range.start).min(height.saturating_sub(2));
                    let end = y(range.end).max(start.saturating_add(2));
                    raster.mark(start..end, kind, color);
                }
            }
        }
        let mut bands = Vec::new();
        for lane in 0..3 {
            let mut start = 0;
            let mut previous = None;
            for pixel in 0..=height {
                let color = raster.row(pixel).and_then(|row| row[lane]);
                if color != previous {
                    if let Some(color) = previous {
                        bands.push((lane, start..pixel, color));
                    }
                    start = pixel;
                    previous = color;
                }
            }
        }
        self.minimap.overview = Some(Overview { key, bounds, bands });
    }

    pub(crate) fn paint_overview(&self, window: &mut Window) {
        let Some(overview) = &self.minimap.overview else {
            return;
        };
        let bounds = overview.bounds;
        let scale = window.scale_factor();
        if let Some(color) = self.minimap_color("editorOverviewRuler.background") {
            window.paint_quad(fill(bounds, hsla(color)));
        }
        let width = bounds.size.width / 3.0;
        for (lane, rows, color) in &overview.bands {
            let start = rows.start as f32 / scale;
            let length = (rows.end - rows.start) as f32 / scale;
            let rect = Bounds::new(
                point(
                    bounds.origin.x + width * *lane as f32,
                    bounds.origin.y + px(start),
                ),
                size(width, px(length)),
            );
            window.paint_quad(fill(rect, hsla(*color)));
        }
        if self.options.minimap.ruler_border {
            if let Some(color) = self.minimap_color("editorOverviewRuler.border") {
                let line = crate::dpi::vline(
                    f32::from(bounds.origin.x),
                    f32::from(bounds.origin.y),
                    f32::from(bounds.size.height),
                    scale,
                );
                window.paint_quad(fill(line, hsla(color)));
            }
        }
    }

    fn paint_minimap_marks(&self, panel: Panel, window: &mut Window) {
        let row_height = panel.track.sampling.row_height;
        let visible_start = (panel.track.offset / row_height).floor() as u64;
        let visible_end = ((panel.track.offset + panel.track.height) / row_height).ceil() as u64;
        let first = panel.track.sampling.visual_row(visible_start);
        let last = panel.track.sampling.visual_row(visible_end);
        for (&kind, ranges) in &self.minimap.visual_marks {
            let Some(color) = kind.style().minimap.and_then(|key| self.minimap_color(key)) else {
                continue;
            };
            let start = ranges.partition_point(|range| range.end <= first);
            for range in ranges[start..]
                .iter()
                .take_while(|range| range.start <= last)
            {
                let rows = panel.track.sampling.sampled_rows(range.clone());
                let y = (rows.start as f64 * row_height - panel.track.offset) as f32;
                let h = (((rows.end - rows.start) as f64 * row_height) as f32).max(2.0);
                let rect = Bounds::new(
                    point(panel.bounds.origin.x, panel.bounds.origin.y + px(y)),
                    size(panel.bounds.size.width, px(h)),
                );
                window.paint_quad(fill(rect, hsla(color)));
            }
        }
    }

    pub(crate) fn refresh_minimap_occurrences(&mut self, epoch: u64, cx: &mut Context<Self>) {
        let version = self.version();
        let target = self.occurrences.borrow().target.clone();
        let active = target.is_some() && self.effective_level() == SizeClass::Normal;
        let key = (epoch, version, active);
        if self.minimap.occurrence_key == Some(key) {
            return;
        }
        self.minimap.occurrence_key = Some(key);
        self.minimap.occurrence_task = None;
        let Some((target, text)) = target.zip(self.text.clone()).filter(|_| active) else {
            self.set_minimap_markers("textOccurrences", Vec::new(), cx);
            return;
        };
        let cancel = rove_base::Cancellation::default();
        let worker_cancel = cancel.clone();
        let work = cx.background_executor().spawn(async move {
            let occurrences = super::occurrences::Occurrences {
                target: Some(target),
                ..Default::default()
            };
            let mut marks = Vec::new();
            for line in 0..text.lines().known {
                if worker_cancel.check().is_err() {
                    return Vec::new();
                }
                if let Some(source) = columns::read_line(&*text, line) {
                    if !occurrences.matches(line, &source).is_empty() {
                        marks.push(Marker {
                            lines: line..line + 1,
                            kind: MarkerKind::Occurrence,
                        });
                    }
                }
            }
            marks
        });
        self.minimap.occurrence_task = Some((
            CancelOnDrop(cancel),
            cx.spawn(async move |view, cx| {
                let marks = work.await;
                let _ = view.update(cx, |view, cx| {
                    if view.minimap.occurrence_key == Some(key) && view.version() == version {
                        view.set_minimap_markers("textOccurrences", marks, cx);
                        view.minimap.occurrence_task = None;
                    }
                });
            }),
        ));
    }

    pub(crate) fn minimap_width(&self, width: f32) -> f32 {
        let o = &self.options.minimap;
        if !o.enabled || self.text.is_none() || self.effective_level() != SizeClass::Normal {
            return 0.0;
        }
        (o.columns as f32).min((width * 0.23).floor())
    }

    fn minimap_color(&self, key: &str) -> Option<Color> {
        self.services.theme.color(key).ok().flatten()
    }

    pub(crate) fn update_minimap(
        &mut self,
        bounds: Option<Bounds<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let panel = bounds.zip(self.metrics()).map(|(bounds, metrics)| {
            let height = bounds.size.height.to_f64();
            let o = &self.options.minimap;
            let sampling = Sampling::new(o.size, o.scale, height, self.visual_map().visual_count());
            let track = Track::new(
                sampling,
                height,
                self.current_top(),
                metrics.max_top(&self.zones),
            );
            Panel { bounds, track }
        });
        if std::mem::take(&mut self.minimap.mapping_dirty)
            || (panel.is_none() && self.minimap.panel.is_some())
            || self.minimap.panel.zip(panel).is_some_and(|(old, new)| {
                let a = old.track.sampling;
                let b = new.track.sampling;
                (a.visual_rows, a.raster_rows) != (b.visual_rows, b.raster_rows)
            })
        {
            self.minimap.clear(cx);
        }
        self.minimap.panel = panel;
        let top = self.current_top();
        if self.options.minimap.autohide == Autohide::Scroll && top != self.minimap.last_top {
            self.minimap.revealed_on_scroll = true;
            let timer = cx.background_executor().timer(Duration::from_millis(700));
            self.minimap.fade = Some(cx.spawn(async move |view, cx| {
                timer.await;
                let _ = view.update(cx, |v, cx| {
                    v.minimap.revealed_on_scroll = false;
                    cx.notify();
                });
            }));
        }
        self.minimap.last_top = top;
        self.prepare_minimap(cx);
    }

    fn prepare_minimap(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.minimap.panel else {
            return;
        };
        let Some(text) = self.text.clone() else {
            return;
        };
        let tiles = panel.track.tiles();
        let mut missing = None;
        for index in tiles.clone() {
            if self.minimap.cache.get(index).is_none() && missing.is_none() {
                missing = Some(index);
            }
        }
        if self
            .minimap
            .task
            .as_ref()
            .is_some_and(|(index, _)| tiles.contains(index))
        {
            return;
        }
        self.minimap.task = None;
        let Some(index) = missing else { return };
        let first = index * u64::from(TILE_ROWS);
        let sampling = panel.track.sampling;
        let end = (first + u64::from(TILE_ROWS)).min(sampling.raster_rows);
        let row_info = (first..end)
            .map(|row| {
                let visual = sampling.visual_row(row);
                let logical = self.doc_line(visual);
                let folds = self.folding.ranges();
                let begin = folds.partition_point(|f| f.start < logical);
                let region = folds[begin..]
                    .iter()
                    .take_while(|f| f.start == logical)
                    .any(|f| f.source == super::folding::FoldSource::Region);
                (
                    logical,
                    visual.saturating_sub(self.view_line(logical)) as u32,
                    region,
                )
            })
            .collect();
        let job = TileJob {
            text,
            syntax: self.syntax.borrow().clone(),
            columns: self.options.minimap.columns,
            row_info,
            marker_pattern: self
                .options
                .minimap
                .mark_headers
                .then(|| self.options.minimap.header_pattern.clone()),
            region_start: self
                .language_config()
                .and_then(|config| config.folding_markers())
                .filter(|_| self.options.minimap.region_headers)
                .map(|markers| markers.start().clone()),
            wrap: (self.wrap.columns > 0).then(|| (self.wrap.columns, self.wrap_options())),
            tab_size: self.options.tab_size,
            render_limit: self.options.stop_rendering_line_after,
            foreground: self.palette.foreground,
            opacity: self
                .minimap_color("minimap.foregroundOpacity")
                .map_or(255, |c| c.3),
        };
        let background = cx.background_executor().spawn(async move { job.run() });
        self.minimap.task = Some((
            index,
            cx.spawn(async move |view, cx| {
                let (image, headers) = background.await;
                let _ = view.update(cx, |view, cx| {
                    view.minimap.task = None;
                    for old in view.minimap.cache.insert(index, image) {
                        cx.drop_image(old, None);
                    }
                    view.minimap.headers.insert(
                        index,
                        headers
                            .into_iter()
                            .map(|(row, title)| Header {
                                row: first + row,
                                title,
                                shape: None,
                            })
                            .collect(),
                    );
                    let cache = &view.minimap.cache;
                    view.minimap
                        .headers
                        .retain(|index, _| cache.contains(*index));
                    cx.notify();
                });
            }),
        ));
    }

    pub(crate) fn invalidate_minimap_rows(&mut self, changes: Option<&[Range<u64>]>, cx: &mut App) {
        let Some(changes) = changes else {
            self.minimap.clear(cx);
            return;
        };
        let Some(panel) = self.minimap.panel else {
            return;
        };
        for lines in changes.iter().filter(|r| !r.is_empty()) {
            let last = self.visual_map().visual_range(lines.end - 1);
            let visual = self.view_line(lines.start)..last.end;
            let raster = panel.track.sampling.sampled_rows(visual);
            self.minimap.invalidate(raster, cx);
        }
    }

    fn paint_minimap_headers(&mut self, panel: Panel, index: u64, window: &mut Window) {
        let Some(headers) = self.minimap.headers.get_mut(&index) else {
            return;
        };
        let options = &self.options.minimap;
        let font = &self.font.fonts[1];
        let font_size = px(options.header_font_size);
        let color = hsla(self.palette.foreground);
        let scale = window.scale_factor();
        let bounds = panel.bounds;
        for header in headers {
            let y =
                (header.row as f64 * panel.track.sampling.row_height - panel.track.offset) as f32;
            if y + options.header_font_size < 0.0 || px(y) >= bounds.size.height {
                continue;
            }
            let origin = point(bounds.origin.x, bounds.origin.y + px(y));
            if header.title.separator {
                let line = crate::dpi::hline(
                    f32::from(origin.x),
                    f32::from(origin.y),
                    f32::from(bounds.size.width),
                    1.0,
                    scale,
                );
                window.paint_quad(fill(line, color));
            }
            if header.title.label.is_empty() {
                continue;
            }
            let shaped = header.shape.get_or_insert_with(|| {
                let text = gpui::SharedString::from(
                    header.title.label.chars().take(120).collect::<String>(),
                );
                let run = gpui::TextRun {
                    len: text.len(),
                    font: font.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                window
                    .text_system()
                    .shape_line(text, font_size, &[run], None)
            });
            for run in &shaped.runs {
                for glyph in &run.glyphs {
                    let count = shaped
                        .text
                        .get(..glyph.index)
                        .map_or(0, |s| s.chars().count());
                    let x =
                        px(2.0 + count as f32 * options.header_letter_spacing) + glyph.position.x;
                    let at = origin + point(x, shaped.ascent + glyph.position.y);
                    if glyph.is_emoji {
                        let _ = window.paint_emoji(at, run.font_id, glyph.id, font_size);
                    } else {
                        let _ = window.paint_glyph(at, run.font_id, glyph.id, font_size, color);
                    }
                }
            }
        }
    }

    pub(crate) fn paint_minimap(&mut self, window: &mut Window) {
        let Some(panel) = self.minimap.panel else {
            return;
        };
        let Some(metrics) = self.metrics() else {
            return;
        };
        let dragging = matches!(self.drag, Some(Drag::MinimapThumb { .. }));
        let o = &self.options.minimap;
        let show = match o.autohide {
            Autohide::None => true,
            Autohide::Mouseover => self.minimap.hovered || dragging,
            Autohide::Scroll => self.minimap.hovered || dragging || self.minimap.revealed_on_scroll,
        };
        if !show {
            return;
        }
        let slider_visible = o.slider_always || self.minimap.hovered || dragging;
        let bounds = panel.bounds;
        let background = self
            .minimap_color("minimap.background")
            .unwrap_or(self.palette.background);
        window.paint_quad(fill(bounds, hsla(background)));
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            let row_height = panel.track.sampling.row_height;
            for index in panel.track.tiles() {
                let Some(image) = self.minimap.cache.get(index).cloned() else {
                    continue;
                };
                let first = index * u64::from(TILE_ROWS);
                let rows = (panel.track.sampling.raster_rows - first).min(u64::from(TILE_ROWS));
                let y = (first as f64 * row_height - panel.track.offset) as f32;
                let rect = Bounds::new(
                    point(bounds.origin.x, bounds.origin.y + px(y)),
                    size(bounds.size.width, px((rows as f64 * row_height) as f32)),
                );
                let _ = window.paint_image(rect, Corners::default(), image, 0, false);
            }
            self.paint_minimap_marks(panel, window);
            for index in panel.track.tiles() {
                self.paint_minimap_headers(panel, index, window);
            }
            if slider_visible {
                if let Some(thumb) = panel.track.thumb(&metrics, &self.zones, self.current_top()) {
                    let color = if dragging {
                        "minimapSlider.activeBackground"
                    } else if self.minimap.hovered {
                        "minimapSlider.hoverBackground"
                    } else {
                        "minimapSlider.background"
                    };
                    if let Some(color) = self.minimap_color(color) {
                        let rect = Bounds::new(
                            point(bounds.origin.x, bounds.origin.y + px(thumb.start as f32)),
                            size(bounds.size.width, px(thumb.len as f32)),
                        );
                        window.paint_quad(fill(rect, hsla(color)));
                    }
                }
            }
        });
    }

    pub(crate) fn minimap_hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let hovered = self
            .minimap
            .panel
            .is_some_and(|panel| panel.bounds.contains(&position));
        if self.minimap.hovered != hovered {
            self.minimap.hovered = hovered;
            cx.notify();
        }
    }

    pub(crate) fn minimap_down(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(panel) = self
            .minimap
            .panel
            .filter(|panel| panel.bounds.contains(&position))
        else {
            return false;
        };
        let Some(metrics) = self.metrics() else {
            return false;
        };
        let y = (position.y - panel.bounds.origin.y).to_f64();
        if let Some(thumb) = panel.track.thumb(&metrics, &self.zones, self.current_top()) {
            if (thumb.start..thumb.start + thumb.len).contains(&y) {
                self.drag = Some(Drag::MinimapThumb {
                    grab: y - thumb.start,
                });
                cx.notify();
                return true;
            }
        }
        let content_height =
            panel.track.sampling.raster_rows as f64 * panel.track.sampling.row_height;
        let visual = scroll::line_for_track_y(
            content_height,
            y + panel.track.offset,
            &metrics,
            &self.zones,
        );
        self.reveal_line(self.doc_line(visual), Reveal::Center, cx);
        true
    }

    pub(crate) fn minimap_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(Drag::MinimapThumb { grab }) = self.drag else {
            return;
        };
        let Some(panel) = self.minimap.panel else {
            return;
        };
        let Some(metrics) = self.metrics() else {
            return;
        };
        let y = (position.y - panel.bounds.origin.y).to_f64();
        let top = scroll::pos_for_thumb(
            panel.track.height,
            metrics.viewport_lines(),
            metrics.total_lines(&self.zones),
            y - grab,
        );
        self.set_scroll_position(
            ScrollPosition {
                top,
                x: self.scroll.x,
            },
            false,
            cx,
        );
    }
}
