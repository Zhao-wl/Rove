//! 缩略图的离屏色块和视觉行采样。

use gpui::RenderImage;
use image::RgbaImage;
use rove_config::{Key, Scope, Settings};
use rove_syntax::{
    language_config::{compile_regex, CompiledRegex, RegexOrigin},
    JsRegex,
};
use rove_theme::Color;
use std::{collections::BTreeMap, ops::Range, sync::Arc};

/// 一行像素对应一个采样后的视觉行。绘制时按布局缩放，同一行无需存储多份像素。
pub const TILE_ROWS: u32 = 256;
pub const ENABLED: Key<bool> = Key::new("editor.minimap.enabled");
pub const DEFAULT_SECTION_PATTERN: &str = r"\bMARK:\s*(?<separator>-?)\s*(?<label>.*)$";

pub const PANEL_COLOR_KEYS: &[&str] = &[
    "minimap.background",
    "minimap.foregroundOpacity",
    "minimapSlider.background",
    "minimapSlider.hoverBackground",
    "minimapSlider.activeBackground",
    "editorOverviewRuler.background",
    "editorOverviewRuler.border",
    // 兼容现有主题输入。没有 Git 数据提供方。
    "minimapGutter.addedBackground",
    "minimapGutter.modifiedBackground",
    "minimapGutter.deletedBackground",
];

/// 声明顺序同时决定同一通道内的覆盖优先级。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MarkerKind {
    Range,
    Selection,
    Occurrence,
    SymbolRead,
    SymbolWrite,
    Bracket,
    Find,
    Info,
    Warning,
    Error,
    Cursor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkerStyle {
    pub minimap: Option<&'static str>,
    pub ruler: &'static str,
    pub lanes: Lane,
}

/// 概览标尺由左、中、右三个相邻通道组成。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Center,
    Right,
    Full,
}

impl Lane {
    fn columns(self) -> Range<usize> {
        match self {
            Self::Center => 1..2,
            Self::Right => 2..3,
            Self::Full => 0..3,
        }
    }
}

impl MarkerKind {
    pub const ALL: [Self; 11] = [
        Self::Range,
        Self::Selection,
        Self::Occurrence,
        Self::SymbolRead,
        Self::SymbolWrite,
        Self::Bracket,
        Self::Find,
        Self::Info,
        Self::Warning,
        Self::Error,
        Self::Cursor,
    ];

    pub fn style(self) -> MarkerStyle {
        use Lane::*;
        let (minimap, ruler, lanes) = match self {
            Self::Range => (None, "editorOverviewRuler.rangeHighlightForeground", Full),
            Self::Selection => (
                Some("minimap.selectionHighlight"),
                "editorOverviewRuler.selectionHighlightForeground",
                Center,
            ),
            Self::Occurrence => (
                Some("minimap.selectionOccurrenceHighlight"),
                "editorOverviewRuler.wordHighlightTextForeground",
                Center,
            ),
            Self::SymbolRead => (
                Some("minimap.selectionOccurrenceHighlight"),
                "editorOverviewRuler.wordHighlightForeground",
                Center,
            ),
            Self::SymbolWrite => (
                Some("minimap.selectionOccurrenceHighlight"),
                "editorOverviewRuler.wordHighlightStrongForeground",
                Center,
            ),
            Self::Bracket => (None, "editorOverviewRuler.bracketMatchForeground", Center),
            Self::Find => (
                Some("minimap.findMatchHighlight"),
                "editorOverviewRuler.findMatchForeground",
                Center,
            ),
            Self::Info => (
                Some("minimap.infoHighlight"),
                "editorOverviewRuler.infoForeground",
                Right,
            ),
            Self::Warning => (
                Some("minimap.warningHighlight"),
                "editorOverviewRuler.warningForeground",
                Right,
            ),
            Self::Error => (
                Some("minimap.errorHighlight"),
                "editorOverviewRuler.errorForeground",
                Right,
            ),
            Self::Cursor => (None, "editorCursor.foreground", Full),
        };
        MarkerStyle {
            minimap,
            ruler,
            lanes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub lines: Range<u64>,
    pub kind: MarkerKind,
}

/// 将同像素位置的标记按通道叠加。缓存按屏幕高度开销分配，独立于文件总行数与标记数量。
pub struct RulerRaster(Vec<[Option<(MarkerKind, Color)>; 3]>);

impl RulerRaster {
    pub fn new(height: usize) -> Self {
        Self(vec![[None; 3]; height])
    }

    pub fn mark(&mut self, pixels: Range<usize>, kind: MarkerKind, color: Color) {
        let height = self.0.len();
        let start = pixels.start.min(height);
        let end = pixels.end.min(height).max(start);
        let lanes = kind.style().lanes.columns();
        for row in &mut self.0[start..end] {
            for cell in &mut row[lanes.clone()] {
                if cell.is_none_or(|(previous, _)| previous <= kind) {
                    *cell = Some((kind, color));
                }
            }
        }
    }

    pub fn row(&self, pixel: usize) -> Option<[Option<Color>; 3]> {
        self.0
            .get(pixel)
            .map(|row| row.map(|cell| cell.map(|(_, color)| color)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    #[default]
    Proportional,
    Fill,
    Fit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Autohide {
    #[default]
    None,
    Mouseover,
    Scroll,
}

/// 缩略图和标尺共用配置。设置目录已登记这些 VS Code 键。
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub enabled: bool,
    pub left: bool,
    pub size: Size,
    pub scale: u8,
    pub columns: u32,
    pub autohide: Autohide,
    pub slider_always: bool,
    pub mark_headers: bool,
    pub region_headers: bool,
    pub header_font_size: f32,
    pub header_letter_spacing: f32,
    pub header_pattern: Arc<str>,
    pub ruler_border: bool,
    pub ruler_hide_cursor: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            enabled: true,
            left: false,
            size: Size::Proportional,
            scale: 1,
            columns: 120,
            autohide: Autohide::None,
            slider_always: false,
            mark_headers: true,
            region_headers: true,
            header_font_size: 9.0,
            header_letter_spacing: 1.0,
            header_pattern: DEFAULT_SECTION_PATTERN.into(),
            ruler_border: true,
            ruler_hide_cursor: false,
        }
    }
}

impl Options {
    pub fn read(settings: &dyn Settings, scope: &Scope) -> Self {
        let mut options = Self::default();
        let flag = |key, default| {
            Key::<bool>::new(key)
                .read(settings, scope)
                .unwrap_or(default)
        };
        let number = |key, default| {
            Key::<f64>::new(key)
                .read(settings, scope)
                .ok()
                .filter(|n| n.is_finite())
                .unwrap_or(default)
        };
        let text = |key| Key::<Arc<str>>::new(key).read(settings, scope).ok();
        options.enabled = ENABLED.read(settings, scope).unwrap_or(options.enabled);
        options.left = text("editor.minimap.side").as_deref() == Some("left");
        options.size = match text("editor.minimap.size").as_deref() {
            Some("fit") => Size::Fit,
            Some("fill") => Size::Fill,
            _ => Size::Proportional,
        };
        options.scale = number("editor.minimap.scale", options.scale.into()).clamp(1.0, 3.0) as u8;
        options.columns =
            number("editor.minimap.maxColumn", options.columns.into()).clamp(1.0, 1_000.0) as u32;
        options.autohide = match text("editor.minimap.autohide").as_deref() {
            Some("mouseover") => Autohide::Mouseover,
            Some("scroll") => Autohide::Scroll,
            _ => Autohide::None,
        };
        options.slider_always = text("editor.minimap.showSlider").as_deref() == Some("always");
        options.mark_headers = flag(
            "editor.minimap.showMarkSectionHeaders",
            options.mark_headers,
        );
        options.region_headers = flag(
            "editor.minimap.showRegionSectionHeaders",
            options.region_headers,
        );
        options.header_font_size = number(
            "editor.minimap.sectionHeaderFontSize",
            options.header_font_size.into(),
        )
        .clamp(4.0, 32.0) as f32;
        options.header_letter_spacing = number(
            "editor.minimap.sectionHeaderLetterSpacing",
            options.header_letter_spacing.into(),
        )
        .clamp(0.0, 8.0) as f32;
        if let Some(pattern) = text("editor.minimap.markSectionHeaderRegex") {
            options.header_pattern = pattern;
        }
        options.ruler_border = flag("editor.overviewRulerBorder", options.ruler_border);
        options.ruler_hide_cursor = flag(
            "editor.hideCursorInOverviewRuler",
            options.ruler_hide_cursor,
        );
        options
    }
}

/// 视觉行到色块行的采样。fill 与 fit 把采样后的行数限制在面板容量内，
/// 每帧只搬运相交的分块。Proportional 使用原比例，在面板内独立滚动。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampling {
    pub visual_rows: u64,
    pub raster_rows: u64,
    pub row_height: f64,
}

impl Sampling {
    pub fn new(mode: Size, scale: u8, height: f64, visual_rows: u64) -> Self {
        let scale = f64::from(scale.clamp(1, 3));
        let available_rows = ((height / scale).floor() as u64).max(1);
        let raster_rows = match mode {
            Size::Proportional => visual_rows,
            Size::Fill | Size::Fit => visual_rows.min(available_rows),
        };
        let row_height = match mode {
            Size::Fill if raster_rows > 0 => height / raster_rows as f64,
            Size::Fit if raster_rows > 0 && raster_rows < visual_rows => {
                height / raster_rows as f64
            }
            _ => scale,
        };
        Self {
            visual_rows,
            raster_rows,
            row_height,
        }
    }

    pub fn visual_row(&self, raster_row: u64) -> u64 {
        if self.raster_rows <= 1 {
            return 0;
        }
        let numerator =
            u128::from(raster_row.min(self.raster_rows - 1)) * u128::from(self.visual_rows - 1);
        (numerator / u128::from(self.raster_rows - 1)) as u64
    }

    /// 所有真正采样到该半开区间内的行。未采样到的修改不作废图像。
    pub fn sampled_rows(&self, visible: Range<u64>) -> Range<u64> {
        let lower_bound = |visual_row| {
            if visual_row == 0 {
                return 0;
            }
            if self.raster_rows <= 1 || visual_row >= self.visual_rows {
                return self.raster_rows;
            }
            let numerator = u128::from(visual_row) * u128::from(self.raster_rows - 1);
            numerator.div_ceil(u128::from(self.visual_rows - 1)) as u64
        };
        lower_bound(visible.start)..lower_bound(visible.end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionHeader {
    pub label: Arc<str>,
    pub separator: bool,
}

pub struct MarkHeaders {
    matcher: CompiledRegex,
    label: usize,
    separator: Option<usize>,
}

impl MarkHeaders {
    pub fn compile(pattern: Arc<str>) -> Option<Self> {
        let origin = RegexOrigin::new(
            "*",
            "settings.json",
            "editor.minimap.markSectionHeaderRegex",
        );
        let matcher = compile_regex(
            &JsRegex {
                source: pattern,
                flags: "".into(),
            },
            &origin,
        )
        .ok()?;
        let label = matcher
            .capture_names()
            .position(|name| name == Some("label"))?;
        let separator = matcher
            .capture_names()
            .position(|name| name == Some("separator"));
        Some(Self {
            matcher,
            label,
            separator,
        })
    }

    /// 仅对语法提供的注释区间运行正则。候选不包括字符串或正文。
    pub fn find(
        &self,
        text: &str,
        comments: impl IntoIterator<Item = Range<u32>>,
    ) -> Option<SectionHeader> {
        comments.into_iter().find_map(|comment| {
            let source = text.get(comment.start as usize..comment.end as usize)?;
            let captures = self.matcher.capture_ranges(source, 0).ok()??;
            let (start, end) = captures.get(self.label).copied().flatten()?;
            let label = source[start..end].trim();
            let separator = self
                .separator
                .and_then(|slot| captures.get(slot).copied().flatten())
                .is_some_and(|(start, end)| start < end);
            (!label.is_empty() || separator).then(|| SectionHeader {
                label: label.into(),
                separator,
            })
        })
    }
}

/// 调用方先确认此行是折叠服务给出的区域起始行，再提取标记后的文字。
pub fn region_header(text: &str, start: &CompiledRegex) -> Option<SectionHeader> {
    let (_, end) = start.find(text).ok()??;
    let label = text[end..].trim();
    (!label.is_empty()).then(|| SectionHeader {
        label: label.into(),
        separator: false,
    })
}

pub fn tile_range(rows: Range<u64>) -> Range<u64> {
    let end = if rows.is_empty() {
        rows.start / u64::from(TILE_ROWS)
    } else {
        rows.end.div_ceil(u64::from(TILE_ROWS))
    };
    rows.start / u64::from(TILE_ROWS)..end
}

/// GPUI 使用 BGRA 格式。透明像素允许面板单独绘制背景与标记。
pub struct Tile {
    pixels: RgbaImage,
}

impl Tile {
    pub fn new(columns: u32, rows: u32) -> Self {
        Self {
            pixels: RgbaImage::new(columns, rows.min(TILE_ROWS)),
        }
    }

    pub fn into_pixels(self) -> RgbaImage {
        self.pixels
    }

    pub fn into_image(self) -> Arc<RenderImage> {
        Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
            self.into_pixels()
        )]))
    }

    /// 样式区间按源行的 UTF-8 字节列排序且不相交。Tab 前进到下一制表位，空白保留透明。
    pub fn paint_row(
        &mut self,
        row: u32,
        source: &str,
        spans: impl IntoIterator<Item = (Range<u32>, Color)>,
        foreground: Color,
        opacity: u8,
        tab_size: u32,
    ) {
        if row >= self.pixels.height() {
            return;
        }
        let width = self.pixels.width() as usize;
        let offset = row as usize * width * 4;
        let row = &mut self.pixels.as_flat_samples_mut().samples[offset..offset + width * 4];
        row.fill(0);
        let tab_size = tab_size.max(1) as usize;
        let mut spans = spans.into_iter().peekable();
        let mut characters = source.char_indices();
        let mut next_column = 0;
        for (column, pixel) in row.chunks_exact_mut(4).enumerate() {
            if column < next_column {
                continue;
            }
            let Some((byte, ch)) = characters.next() else {
                break;
            };
            if ch == '\t' {
                next_column = column + (tab_size - column % tab_size);
                continue;
            }
            if ch.is_whitespace() {
                continue;
            }
            let byte = byte as u32;
            while spans.peek().is_some_and(|(span, _)| span.end <= byte) {
                spans.next();
            }
            let color = spans
                .peek()
                .filter(|(span, _)| span.contains(&byte))
                .map_or(foreground, |(_, color)| *color);
            let alpha = (u16::from(color.3) * u16::from(opacity) + 127) / 255;
            pixel.copy_from_slice(&[color.2, color.1, color.0, alpha as u8]);
        }
    }
}

struct CachedTile {
    image: Arc<RenderImage>,
    used: u64,
}

/// 缓存体积设硬上限。删除或替换的图像交给窗口从 GPUI 的纹理图集中移除。
#[derive(Default)]
pub struct TileCache {
    tiles: BTreeMap<u64, CachedTile>,
    bytes: usize,
    clock: u64,
}

impl TileCache {
    const BYTE_LIMIT: usize = 4 * 1024 * 1024;

    pub fn get(&mut self, index: u64) -> Option<&Arc<RenderImage>> {
        let tile = self.tiles.get_mut(&index)?;
        self.clock += 1;
        tile.used = self.clock;
        Some(&tile.image)
    }

    pub fn insert(&mut self, index: u64, image: Arc<RenderImage>) -> Vec<Arc<RenderImage>> {
        let mut removed = self.remove(index).into_iter().collect::<Vec<_>>();
        self.clock += 1;
        self.bytes += image_bytes(&image);
        self.tiles.insert(
            index,
            CachedTile {
                image,
                used: self.clock,
            },
        );
        while self.bytes > Self::BYTE_LIMIT {
            let oldest = *self
                .tiles
                .iter()
                .min_by_key(|(_, tile)| tile.used)
                .unwrap()
                .0;
            removed.extend(self.remove(oldest));
        }
        removed
    }

    /// 失效给定栅格行中的分块，保留其他块及其使用顺序。
    pub fn invalidate(&mut self, rows: Range<u64>) -> Vec<Arc<RenderImage>> {
        let indices: Vec<_> = self
            .tiles
            .range(tile_range(rows))
            .map(|(&index, _)| index)
            .collect();
        indices
            .into_iter()
            .filter_map(|index| self.remove(index))
            .collect()
    }

    pub fn clear(&mut self) -> impl Iterator<Item = Arc<RenderImage>> {
        self.bytes = 0;
        std::mem::take(&mut self.tiles)
            .into_values()
            .map(|tile| tile.image)
    }

    fn remove(&mut self, index: u64) -> Option<Arc<RenderImage>> {
        let tile = self.tiles.remove(&index)?;
        self.bytes -= image_bytes(&tile.image);
        Some(tile.image)
    }
}

fn image_bytes(image: &RenderImage) -> usize {
    image.as_bytes(0).map_or(0, <[u8]>::len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn paints_clipped_color_blocks_using_byte_spans_and_tab_stops() {
        let mut tile = Tile::new(9, 2);
        let foreground = Color(100, 110, 120, 255);
        let token = Color(70, 80, 90, 128);
        tile.paint_row(
            1,
            " \t中xy z012345",
            [(2..5, token), (5..6, token)],
            foreground,
            128,
            4,
        );
        let image = tile.into_pixels();
        let clear = Rgba([0; 4]);
        let styled = Rgba([90, 80, 70, 64]);
        let plain = Rgba([120, 110, 100, 128]);
        assert!(image.rows().next().unwrap().all(|p| *p == clear));
        assert_eq!(
            image.rows().nth(1).unwrap().copied().collect::<Vec<_>>(),
            [clear, clear, clear, clear, styled, styled, plain, clear, plain],
        );
    }

    #[test]
    fn invalidates_only_tiles_intersecting_the_half_open_row_range() {
        assert_eq!(tile_range(0..0), 0..0);
        assert_eq!(tile_range(0..256), 0..1);
        assert_eq!(tile_range(255..257), 0..2);
        assert_eq!(tile_range(256..258), 1..2);
        assert_eq!(tile_range(700..700), 2..2);
    }

    #[test]
    fn repainting_a_shorter_row_clears_the_old_suffix() {
        let mut tile = Tile::new(4, 1);
        let foreground = Color(10, 20, 30, 255);
        tile.paint_row(0, "abcd", [], foreground, 255, 4);
        tile.paint_row(0, "a", [], foreground, 255, 4);
        assert_eq!(
            tile.into_pixels().into_raw(),
            [30, 20, 10, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn three_layouts_scale_and_sample_visual_rows() {
        let proportional = Sampling::new(Size::Proportional, 2, 20.0, 25);
        assert_eq!(
            (proportional.raster_rows, proportional.row_height),
            (25, 2.0)
        );
        let small = Sampling::new(Size::Fit, 2, 20.0, 5);
        assert_eq!((small.raster_rows, small.row_height), (5, 2.0));
        let fill = Sampling::new(Size::Fill, 2, 20.0, 5);
        assert_eq!((fill.raster_rows, fill.row_height), (5, 4.0));

        let sampled = Sampling::new(Size::Fit, 1, 1000.0, 300_000);
        assert_eq!(sampled.raster_rows, 1000);
        assert_eq!(sampled.visual_row(999), 299_999);
        for idx in 0..1000 {
            let visual = sampled.visual_row(idx);
            assert_eq!(sampled.sampled_rows(visual..visual + 1), idx..idx + 1);
        }
        assert!(sampled.sampled_rows(1..2).is_empty());
    }

    #[test]
    fn tile_cache_releases_old_images_and_invalidates_only_intersecting_tiles() {
        let mut cache = TileCache::default();
        for index in 0..4 {
            assert!(cache
                .insert(index, Tile::new(1000, TILE_ROWS).into_image())
                .is_empty());
        }
        let kept = cache.get(0).unwrap().clone();
        let oldest = cache.get(1).unwrap().clone();
        // Touch the other entries after reading the oldest ID for the assertion.
        for index in [2, 3, 0] {
            cache.get(index).unwrap();
        }
        assert_eq!(
            cache.insert(4, Tile::new(1000, TILE_ROWS).into_image()),
            [oldest]
        );
        assert_eq!(cache.bytes, 4 * 1000 * TILE_ROWS as usize * 4);
        assert_eq!(cache.invalidate(511..513).len(), 1);
        assert_eq!(cache.get(0), Some(&kept));
        assert_eq!(cache.clear().count(), 3);
        assert_eq!(cache.bytes, 0);
        assert!(cache.get(0).is_none());
    }

    #[test]
    fn section_labels_honor_comment_tokens_named_groups_and_region_markers() {
        let default = MarkHeaders::compile(DEFAULT_SECTION_PATTERN.into()).unwrap();
        let line = r#"var text = "MARK: wrong"; // MARK: - Foo"#;
        let comment = line.find("//").unwrap() as u32..line.len() as u32;
        assert!(default.find(line, []).is_none());
        assert_eq!(
            default.find(line, [comment]),
            Some(SectionHeader {
                label: "Foo".into(),
                separator: true
            })
        );
        let custom = MarkHeaders::compile(r"NOTE: (?<label>\w+)".into()).unwrap();
        assert_eq!(custom.find("// NOTE: 集合", std::iter::once(0..15)), None);
        assert_eq!(
            &*custom
                .find("// NOTE: Foo", std::iter::once(0..12))
                .unwrap()
                .label,
            "Foo"
        );

        let marker = compile_regex(
            &JsRegex {
                source: r"^\s*#region\b".into(),
                flags: "".into(),
            },
            &RegexOrigin::new(
                "csharp",
                "language-configuration.json",
                "folding.markers.start",
            ),
        )
        .unwrap();
        assert_eq!(
            &*region_header("    #region Foo", &marker).unwrap().label,
            "Foo"
        );
        assert!(region_header("#region", &marker).is_none());
    }

    #[test]
    fn overview_lanes_keep_overlapping_match_search_range_and_diagnostic_colors() {
        let mut raster = RulerRaster::new(12);
        let range = Color(10, 20, 30, 100);
        let find = Color(35, 45, 55, 190);
        let warning = Color(90, 80, 70, 220);
        let error = Color(255, 0, 0, 220);
        raster.mark(4..8, MarkerKind::Find, find);
        raster.mark(2..8, MarkerKind::Range, range);
        raster.mark(6..12, MarkerKind::Error, error);
        raster.mark(4..8, MarkerKind::Warning, warning);
        assert_eq!(raster.row(3), Some([Some(range); 3]));
        assert_eq!(
            raster.row(4),
            Some([Some(range), Some(find), Some(warning)])
        );
        assert_eq!(raster.row(6), Some([Some(range), Some(find), Some(error)]));
        assert_eq!(raster.row(11), Some([None, None, Some(error)]));
    }
}
