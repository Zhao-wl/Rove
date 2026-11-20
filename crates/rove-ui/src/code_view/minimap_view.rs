//! 缩略图的布局、后台分块与输入处理。

#[cfg(test)]
#[path = "minimap_view_tests.rs"]
mod tests;

use super::{
    columns::{self, DisplayLine},
    decorations::{merge, InlineDecoration, LineDecorations},
    highlight::SyntaxLayer,
    minimap::{
        self, Autohide, MarkHeaders, Marker, MarkerKind, Sampling, SectionHeader, Tile, TileCache,
        TILE_ROWS,
    },
    scroll::{self, Metrics, ScrollPosition, Thumb, ViewZones},
    semantic::SemanticLayer,
    view::{hsla, Drag},
    wrap::{self, LineWrap, WrapOptions},
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
    slider: Option<Thumb>,
    max_texture_top: f64,
    slider_travel: f64,
}

impl Track {
    fn new(
        sampling: Sampling,
        height: f64,
        metrics: &Metrics,
        zones: &ViewZones,
        top: f64,
    ) -> Self {
        let max_top = metrics.max_top(zones);
        let capacity = sampling.raster_rows as f64 * sampling.row_height;
        let project = |top| {
            sampling.raster_at_visual(visual_at_scroll(top, metrics, zones)) * sampling.row_height
        };
        let texture_top = project(top);
        let max_texture_top = project(max_top);
        let max_offset = (capacity - height).max(0.0);
        let progress = if max_texture_top > 0.0 {
            (texture_top / max_texture_top).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let offset = max_offset * progress;
        let slider_travel = (max_texture_top - max_offset).max(0.0);
        let slider = (max_top > 0.0 && capacity > 0.0).then(|| {
            let texture_end = project(top + metrics.viewport_lines());
            let min_height = height.min(1.0);
            let start = (texture_top - offset).clamp(0.0, height - min_height);
            let len = (texture_end - texture_top).clamp(min_height, height - start);
            Thumb { start, len }
        });
        Self {
            sampling,
            height,
            offset,
            slider,
            max_texture_top,
            slider_travel,
        }
    }

    fn top_for_thumb(self, slider_y: f64, metrics: &Metrics, zones: &ViewZones) -> f64 {
        if slider_y <= 0.0 || self.slider_travel <= 0.0 {
            return 0.0;
        }
        // 只有尾部不足一个像素的滑块需要钳制。拖到可达的最后一行时保留编辑器的留白语义。
        if slider_y >= self.slider_travel.min((self.height - 1.0).max(0.0)) {
            return metrics.max_top(zones);
        }
        let raster =
            self.max_texture_top * slider_y / self.slider_travel / self.sampling.row_height;
        let visual = self.sampling.visual_at_raster(raster);
        let row = visual.floor() as u64;
        metrics.line_top(row, zones) + visual.fract() * (1.0 + zones.get(row) / metrics.line_height)
    }

    fn tiles(self) -> Range<u64> {
        let per_row = self.sampling.row_height;
        let start = (self.offset / per_row).floor() as u64;
        let end =
            (((self.offset + self.height) / per_row).ceil() as u64).min(self.sampling.raster_rows);
        minimap::tile_range(start..end)
    }

    fn raster_columns(self, requested: u32) -> u32 {
        let tile_rows = u64::from(TILE_ROWS);
        // 任意滚动偏移最多跨过一个额外的分块边界。计入整块纹理，防止可见块相互淘汰。
        let visible = ((self.height / self.sampling.row_height).ceil() as u64).saturating_add(1);
        let resident_rows = visible
            .saturating_add(tile_rows - 1)
            .div_ceil(tile_rows)
            .saturating_mul(tile_rows)
            .min(self.sampling.raster_rows);
        TileCache::fit_columns(requested, resident_rows)
    }
}

fn visual_at_scroll(top: f64, metrics: &Metrics, zones: &ViewZones) -> f64 {
    let (line, y) = ScrollPosition { top, x: 0.0 }.first_visible(metrics, zones);
    // 嵌入区域随其上一行一起插值，保证到下一行前没有逆向跳动。
    let interval = metrics.line_height + zones.get(line);
    line as f64 + (-y / interval).clamp(0.0, 1.0)
}

#[derive(Clone, Copy)]
struct Panel {
    bounds: Bounds<Pixels>,
    track: Track,
    columns: u32,
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

fn release_images(cx: &mut App, images: Vec<Arc<gpui::RenderImage>>) {
    if !images.is_empty() {
        // 布局过程中当前窗口暂时不在 App 的窗口表中。
        cx.defer(move |app| {
            for image in images {
                app.drop_image(image, None);
            }
        });
    }
}

impl MinimapView {
    pub(crate) fn clear(&mut self, cx: &mut App) {
        self.task = None;
        self.headers.clear();
        release_images(cx, self.cache.clear().collect());
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
        release_images(cx, self.cache.invalidate(rows));
    }
}

struct RasterRow {
    line: u64,
    segment: u32,
    wrap: Option<LineWrap>,
}

impl RasterRow {
    fn first_header_line(&self) -> u64 {
        self.line + u64::from(self.segment > 0)
    }
}

struct RasterLine {
    source: String,
    wrap: Option<(LineWrap, u32)>,
    opacity: u8,
}

struct TileJob {
    text: Arc<dyn TextSource>,
    syntax: SyntaxLayer,
    semantic: SemanticLayer,
    columns: u32,
    row_info: Vec<RasterRow>,
    header_end: u64,
    hidden_rows: Vec<Range<u64>>,
    region_rows: Vec<u64>,
    marker_pattern: Option<Arc<str>>,
    region_start: Option<rove_syntax::language_config::CompiledRegex>,
    wrap: Option<(u32, WrapOptions)>,
    tab_size: u32,
    render_limit: Option<usize>,
    foreground: Color,
    opacity: u8,
}

impl TileJob {
    fn section_headers(&self) -> Vec<(u64, SectionHeader)> {
        let matcher = self.marker_pattern.clone().and_then(MarkHeaders::compile);
        let mut headers = Vec::new();
        if matcher.is_none() && self.region_rows.is_empty() {
            return headers;
        }

        let mut comments = Vec::new();
        for (index, row) in self.row_info.iter().enumerate() {
            let next = self
                .row_info
                .get(index + 1)
                .map_or(self.header_end, RasterRow::first_header_line);
            for line in row.first_header_line()..next {
                let hidden = self.hidden_rows.partition_point(|range| range.end <= line);
                if self
                    .hidden_rows
                    .get(hidden)
                    .is_some_and(|range| range.start <= line)
                {
                    continue;
                }
                let region = self
                    .region_start
                    .as_ref()
                    .filter(|_| self.region_rows.binary_search(&line).is_ok());
                comments.clear();
                if matcher.is_some() {
                    self.syntax.comment_ranges(line, &mut comments);
                }
                if region.is_none() && comments.is_empty() {
                    continue;
                }
                let Some(source) = columns::read_line(&*self.text, line) else {
                    continue;
                };
                if let Some(title) = region
                    .and_then(|start| minimap::region_header(&source, start))
                    .or_else(|| matcher.as_ref()?.find(&source, comments.iter().cloned()))
                {
                    headers.push((index as u64, title));
                }
            }
        }
        headers
    }

    fn raster_line(
        &self,
        row: &RasterRow,
        lexical: &mut LineDecorations,
        semantic: &mut LineDecorations,
    ) -> Option<RasterLine> {
        let source = columns::read_line(&*self.text, row.line)?;
        let wrap = self.wrap.as_ref().map(|(limit, options)| {
            let display = DisplayLine::build(&source, self.tab_size, self.render_limit);
            let wraps = row.wrap.clone().unwrap_or_else(|| {
                wrap::wrap_line(
                    &source,
                    &display,
                    wrap::Metric::Columns { limit: *limit },
                    options,
                )
            });
            (wraps, display.rendered_source_len())
        });
        lexical.inline.clear();
        semantic.inline.clear();
        semantic.line.opacity = None;
        self.syntax.styles(row.line, &source, &mut lexical.inline);
        self.semantic.decorate(row.line, &source, semantic);
        if !semantic.inline.is_empty() {
            let merged = merge(&[lexical, semantic], source.len() as u32);
            lexical.inline.clear();
            lexical
                .inline
                .extend(merged.runs.into_iter().map(|run| InlineDecoration {
                    range: run.range,
                    style: run.style,
                }));
        }
        let opacity =
            (u16::from(self.opacity) * u16::from(semantic.line.opacity.unwrap_or(255)) / 255) as u8;
        Some(RasterLine {
            source,
            wrap,
            opacity,
        })
    }

    fn run(self) -> (Arc<gpui::RenderImage>, Vec<(u64, SectionHeader)>) {
        let headers = self.section_headers();
        let mut tile = Tile::new(self.columns, self.row_info.len() as u32);
        let mut lexical = LineDecorations::default();
        let mut semantic = LineDecorations::default();
        let mut previous_line = None;
        let mut raster = None;
        // 采样的视觉行按文档顺序排列，同一逻辑行的各段共享源文本、断行与样式。
        for (index, row) in self.row_info.iter().enumerate() {
            if previous_line.replace(row.line) != Some(row.line) {
                raster = self.raster_line(row, &mut lexical, &mut semantic);
            }
            let Some(raster) = &raster else { continue };
            let (range, indent) = match &raster.wrap {
                Some((wrap, end)) if row.segment < wrap.count() => (
                    wrap.segment_range(row.segment, *end),
                    wrap.indent_columns(row.segment),
                ),
                Some(_) => continue,
                None => (0..raster.source.len() as u32, 0),
            };
            let Some(segment_text) = raster.source.get(range.start as usize..range.end as usize)
            else {
                continue;
            };
            let spans = lexical.inline.iter().filter_map(|part| {
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
                raster.opacity,
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
        let marks_changed = self.minimap.marks_dirty;
        if marks_changed && (bounds.is_some() || self.minimap.panel.is_some()) {
            self.update_minimap_marks();
        }
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
        if !marks_changed {
            if let Some(overview) = self.minimap.overview.as_mut().filter(|o| o.key == key) {
                overview.bounds = bounds;
                return;
            }
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

    pub(super) fn overview_contains(&self, position: Point<Pixels>) -> bool {
        self.minimap
            .overview
            .as_ref()
            .is_some_and(|overview| overview.bounds.contains(&position))
    }

    pub(super) fn paint_overview(&self, window: &mut Window) {
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
        self.last_frame.minimap_cache_hit = false;
        let panel = bounds.zip(self.metrics()).and_then(|(bounds, metrics)| {
            let height = bounds.size.height.to_f64();
            let o = &self.options.minimap;
            let sampling = Sampling::new(o.size, o.scale, height, self.visual_map().visual_count());
            let track = Track::new(sampling, height, &metrics, &self.zones, self.current_top());
            let columns = track.raster_columns(o.columns);
            (columns > 0).then_some(Panel {
                bounds,
                track,
                columns,
            })
        });
        if std::mem::take(&mut self.minimap.mapping_dirty)
            || (panel.is_none() && self.minimap.panel.is_some())
            || self.minimap.panel.zip(panel).is_some_and(|(old, new)| {
                let a = old.track.sampling;
                let b = new.track.sampling;
                (a.visual_rows, a.raster_rows, old.columns)
                    != (b.visual_rows, b.raster_rows, new.columns)
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
        self.last_frame.minimap_cache_hit = missing.is_none();
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
        let visual_map = self.visual_map();
        let mut previous_line = None;
        let row_info: Vec<_> = (first..end)
            .map(|row| {
                let (line, segment) = visual_map.to_doc(sampling.visual_row(row));
                let wrap = if previous_line.replace(line) == Some(line) {
                    None
                } else {
                    self.cached_minimap_wrap(line)
                };
                RasterRow {
                    line,
                    segment,
                    wrap,
                }
            })
            .collect();
        // 标题归属由逻辑行的首个视觉段决定，分块的最后一行需要下一块的边界。
        let header_end = if end < sampling.raster_rows {
            let (line, segment) = visual_map.to_doc(sampling.visual_row(end));
            line + u64::from(segment > 0)
        } else {
            visual_map.doc_lines()
        };
        let header_start = row_info
            .first()
            .map_or(header_end, RasterRow::first_header_line);
        let marker_pattern = (self.options.minimap.mark_headers
            && self.syntax.borrow().current.is_some())
        .then(|| self.options.minimap.header_pattern.clone());
        let region_start = self
            .language_config()
            .and_then(|config| config.folding_markers())
            .filter(|_| self.options.minimap.region_headers)
            .map(|markers| markers.start().clone());
        let mut region_rows = Vec::new();
        if region_start.is_some() {
            let folds = self.folding.ranges();
            let first = folds.partition_point(|fold| fold.start < header_start);
            region_rows.extend(
                folds[first..]
                    .iter()
                    .take_while(|fold| fold.start < header_end)
                    .filter(|fold| fold.source == super::folding::FoldSource::Region)
                    .map(|fold| fold.start),
            );
        }
        let mut hidden_rows = Vec::new();
        if marker_pattern.is_some() || !region_rows.is_empty() {
            let hidden = visual_map.folds().hidden_ranges();
            let first = hidden.partition_point(|range| range.end <= header_start);
            let end = hidden.partition_point(|range| range.start < header_end);
            hidden_rows.extend_from_slice(&hidden[first..end]);
        }
        let job = TileJob {
            text,
            syntax: self.syntax.borrow().clone(),
            semantic: self
                .semantic
                .borrow()
                .snapshot_for_rows(row_info.iter().map(|row| row.line)),
            columns: panel.columns,
            row_info,
            header_end,
            hidden_rows,
            region_rows,
            marker_pattern,
            region_start,
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
                    release_images(cx, view.minimap.cache.insert(index, image));
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
        let options = &self.options.minimap;
        let headers_enabled = options.mark_headers || options.region_headers;
        let sampling = panel.track.sampling;
        for lines in changes.iter().filter(|r| !r.is_empty()) {
            let last = self.visual_map().visual_range(lines.end - 1);
            let visual = self.view_line(lines.start)..last.end;
            let raster = if headers_enabled {
                sampling.covering_rows(visual)
            } else {
                sampling.sampled_rows(visual)
            };
            self.minimap.invalidate(raster, cx);
        }
    }

    fn paint_minimap_headers(
        &mut self,
        panel: Panel,
        index: u64,
        next_label_y: &mut f32,
        background: Color,
        window: &mut Window,
    ) {
        let Some(headers) = self.minimap.headers.get_mut(&index) else {
            return;
        };
        let options = &self.options.minimap;
        let font = &self.font.fonts[1];
        let font_size = px(options.header_font_size);
        let color = hsla(self.palette.foreground);
        let scale = window.scale_factor();
        let bounds = panel.bounds;
        let band_height = options.header_font_size * 1.5;
        for header in headers {
            let line_y =
                (header.row as f64 * panel.track.sampling.row_height - panel.track.offset) as f32;
            let has_label = !header.title.label.is_empty();
            let y = if has_label {
                line_y.max(*next_label_y)
            } else {
                line_y
            };
            if y + band_height < 0.0 || px(y) >= bounds.size.height {
                continue;
            }
            let origin = point(bounds.origin.x, bounds.origin.y + px(y));
            if has_label {
                *next_label_y = y + band_height;
                window.paint_quad(fill(
                    Bounds::new(origin, size(bounds.size.width, px(band_height))),
                    hsla(background),
                ));
            }
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
            if !has_label {
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
            let mut next_label_y = f32::NEG_INFINITY;
            let mut label_background = background;
            label_background.3 = (u16::from(background.3) * 179 / 255) as u8;
            for index in panel.track.tiles() {
                self.paint_minimap_headers(
                    panel,
                    index,
                    &mut next_label_y,
                    label_background,
                    window,
                );
            }
            if slider_visible {
                if let Some(thumb) = panel.track.slider {
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
        let y = (position.y - panel.bounds.origin.y).to_f64();
        if let Some(thumb) = panel.track.slider {
            if (thumb.start..thumb.start + thumb.len).contains(&y) {
                self.drag = Some(Drag::MinimapThumb {
                    grab: y - thumb.start,
                });
                cx.notify();
                return true;
            }
        }
        let sampling = panel.track.sampling;
        let row = ((y + panel.track.offset) / sampling.row_height).floor() as u64;
        let visual = sampling.visual_row(row);
        self.reveal_visual_line(visual, Reveal::Center, cx);
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
        let start = if y >= panel.track.height - 1.0 {
            panel.track.height
        } else if y <= 0.0 {
            0.0
        } else {
            y - grab
        };
        let top = panel.track.top_for_thumb(start, &metrics, &self.zones);
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
