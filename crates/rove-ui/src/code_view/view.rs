//! `CodeView`：代码视图实体（状态 + 对外接口）。绘制由 [`super::element::CodeElement`] 完成。
//!
//! 帧内只做三件事：按 f64 滚动位置算出可见行；从缓存取“行显示形式 + 整形结果 + 装饰”；
//! 生成绘制指令。读文本、调用装饰提供方、整形都放在帧之间的空闲切片里（[`CodeView::prepare_slice`]），
//! 每个切片有时间预算，沿滚动方向预取；缓存未命中的行先按等宽近似画占位，下一帧换成正式结果，
//! 行高与 y 坐标只由行号和 f64 滚动位置决定，所以替换时不会跳动。

use super::brackets::{self, BracketMatch};
use super::cache::{CacheStats, Lru};
use super::colors::Palette;
use super::columns::{read_line, DisplayLine, LineSource, TextLines};
use super::decorations::{
    priority, DecorationProvider, DecorationStore, InlineDecoration, LayerId, LineDecorations,
    MergedLine, TextStyle,
};
use super::folding::{FoldMap, FoldRange, Folding};
use super::highlight::{SyntaxLayer, SyntaxProvider};
use super::occurrences::{OccurrenceProvider, Occurrences, Target, WordRule};
use super::options::{EditorOptions, Ligatures, MAX_ZOOM, MIN_ZOOM};
use super::scroll::{self, Metrics, Reveal, ScrollPosition, ViewZones, WheelOptions};
use super::selection::{
    self, extend_by_unit, line_unit, pos, Cursor, Granularity, Move, Selection,
};
use gpui::{
    px, App, Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, Font,
    FontFallbacks, FontFeatures, FontId, FontStyle, FontWeight, GlyphId, Hsla, IntoElement,
    LineLayout, Pixels, Point, Render, TextRun, Window,
};
use rove_base::{Line, LineRange, Position, Version};
use rove_buffer::{ReadOnlyReason, SizeClass, TextSource};
use rove_config::Settings;
use rove_syntax::SyntaxSnapshot;
use rove_theme::{Color, Theme, ThemeKind};
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 默认缓存容量（字节估算）。整形缓存约 600 字节/行（80 字符），16 MB ≈ 2.7 万行。
pub const DEFAULT_SHAPE_CACHE_BYTES: usize = 16 << 20;
pub const DEFAULT_LINE_CACHE_BYTES: usize = 8 << 20;
/// 每个空闲准备切片的时间预算。帧 CPU（< 1 ms）+ 切片 ≤ 半个 120 Hz 刷新间隔。
pub const PREPARE_SLICE: Duration = Duration::from_millis(3);
/// 插入符闪烁间隔（VS Code `BLINK_INTERVAL`）。
pub const BLINK_INTERVAL: Duration = Duration::from_millis(500);
pub(crate) fn hsla(c: Color) -> Hsla {
    gpui::rgba(((c.0 as u32) << 24) | ((c.1 as u32) << 16) | ((c.2 as u32) << 8) | c.3 as u32)
        .into()
}

/// 视图依赖的服务。
#[derive(Clone)]
pub struct CodeViewServices {
    pub settings: Arc<dyn Settings>,
    pub theme: Arc<dyn Theme>,
    pub theme_kind: ThemeKind,
}

/// 选区变化的来源（VS Code `CursorChangeReason` 的子集）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionSource {
    Api,
    Keyboard,
    Mouse,
}

/// 视口：首末可见行、f64 滚动位置与文本区尺寸。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub first_line: u64,
    /// 最后一个（部分）可见的行，含。
    pub last_line: u64,
    /// 视口顶边的虚拟行坐标（f64）。
    pub top: f64,
    pub scroll_x: f64,
    pub width: f32,
    pub height: f32,
}

/// 视图事件（GPUI `cx.subscribe` 订阅）。
#[derive(Clone, Debug, PartialEq)]
pub enum CodeViewEvent {
    SelectionChanged {
        selection: Selection,
        source: SelectionSource,
    },
    ViewportChanged(Viewport),
    /// 文本源被替换（新文档或新快照）。
    TextChanged,
    /// 折叠状态变了（#20）；宿主把它写回标签的 `ViewState::folds`。
    FoldsChanged(Arc<[LineRange]>),
    /// 只读编辑器里尝试输入（#20）；宿主在光标处提示原因（UX_SPEC §6.1 第 2 级反馈）。
    EditBlocked {
        position: Position,
        reasons: Arc<[ReadOnlyReason]>,
    },
    /// “在会话中切换只读模式”（#20）：只读状态按文档保存，由宿主决定并回填 [`CodeView::set_read_only`]。
    ToggleReadonlyRequested,
}

/// 按文档级别（UX_SPEC §3.1）关闭的功能（#20 提供查询，#22 组装状态栏悬停文案）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    OccurrencesHighlight,
    SyntaxFolding,
    /// 语法高亮仍然有，但改为后台完成（完成前显示纯文本）。
    BackgroundHighlighting,
    SyntaxHighlighting,
    Folding,
}

impl Feature {
    /// 界面字符串 key。
    pub fn label_key(self) -> &'static str {
        match self {
            Feature::OccurrencesHighlight => "codeView.feature.occurrencesHighlight",
            Feature::SyntaxFolding => "codeView.feature.syntaxFolding",
            Feature::BackgroundHighlighting => "codeView.feature.backgroundHighlighting",
            Feature::SyntaxHighlighting => "codeView.feature.syntaxHighlighting",
            Feature::Folding => "codeView.feature.folding",
        }
    }
}

/// 只读原因的提示文字 key（`Session` 只在用户用命令设为只读时出现）。
pub fn read_only_reason_key(reason: &ReadOnlyReason) -> &'static str {
    match reason {
        ReadOnlyReason::HugeFile => "codeView.readonly.reason.hugeFile",
        ReadOnlyReason::Metadata => "codeView.readonly.reason.metadata",
        ReadOnlyReason::Permissions => "codeView.readonly.reason.permissions",
        ReadOnlyReason::Rule(r) if &**r == rove_workspace::REASON_READONLY_INCLUDE => {
            "codeView.readonly.reason.readonlyInclude"
        }
        ReadOnlyReason::Rule(_) => "codeView.readonly.reason.extraReadonlyInclude",
        ReadOnlyReason::Session => "codeView.readonly.reason.session",
    }
}

/// 文档元数据里需要在界面上表现为只读的原因：超大文件、元数据视图、磁盘只读属性、只读规则。
/// `Session` 是加载器的默认值（M1 尚无编辑），不显示锁。
pub fn presented_read_only(reasons: &[ReadOnlyReason]) -> Vec<ReadOnlyReason> {
    reasons
        .iter()
        .filter(|r| !matches!(r, ReadOnlyReason::Session))
        .cloned()
        .collect()
}

impl EventEmitter<CodeViewEvent> for CodeView {}

/// 一帧的统计（基准与测试使用）。`cpu` 口径同 M0：prepaint 开始到 paint 结束。
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub cpu: Duration,
    /// 其中布局（prepaint）部分。
    pub layout: Duration,
    /// 本帧绘制的字形数（已按水平可见范围裁剪）。
    pub glyphs: u32,
    pub visible_lines: u32,
    /// 用正式整形结果绘制的行数。
    pub shaped: u32,
    /// 用等宽近似占位的行数。
    pub placeholders: u32,
    /// 文本尚未读到（空白）的行数。
    pub missing: u32,
    /// 首个可见行相对文本区顶边的 y（像素）。占位与正式结果替换前后必须相同。
    pub first_line_y: f32,
    pub first_line: u64,
}

/// 一行的显示与装饰（行缓存条目，随文本版本失效）。
pub(crate) struct LineEntry {
    pub display: DisplayLine,
    pub source: Arc<str>,
    pub hash: u64,
    pub merged: Option<Rc<MergedLine>>,
    /// 显示偏移上的样式段（由装饰的源列换算而来），绘制时直接用。
    pub paint_runs: Vec<(Range<u32>, TextStyle)>,
    /// 整形用的字体段：(显示字节长度, 样式位：1 粗体 / 2 斜体)。
    pub font_runs: Arc<[(u32, u8)]>,
    /// 本行字素簇边界 x 的备忘（只缓存正式整形的结果）：(字体代次, x 表)。x 表必须按本行自己的
    /// stops 计算——整形缓存按显示文本共享，Tab 开头的 `foo` 与四个空格开头的 `foo` 显示相同但边界不同。
    pub xs: RefCell<Option<(u64, Rc<[f32]>)>>,
}

#[derive(Clone)]
pub(crate) struct ShapeKey {
    hash: u64,
    text: Arc<str>,
    runs: Arc<[(u32, u8)]>,
    font_generation: u64,
}
impl PartialEq for ShapeKey {
    fn eq(&self, o: &Self) -> bool {
        self.hash == o.hash
            && self.font_generation == o.font_generation
            && (Arc::ptr_eq(&self.text, &o.text) || self.text == o.text)
            && self.runs == o.runs
    }
}
impl Eq for ShapeKey {}
impl Hash for ShapeKey {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.hash.hash(h);
        self.font_generation.hash(h);
    }
}

/// 整形结果：只含与显示文本对应的纯几何（GPUI 行布局 + 按显示字节偏移排序的字形 x）。
/// 不含任何依赖源行的信息，因此可以在显示文本相同的不同源行之间共享。
pub(crate) struct Shaped {
    pub layout: Arc<LineLayout>,
    pub glyphs: Vec<(u32, f32)>,
}

/// 行号用的数字字形（每次字体变化整形一次 "0123456789"）。
pub(crate) struct Digits {
    pub font_id: FontId,
    pub glyphs: [Option<GlyphId>; 10],
    pub advance: f32,
    pub ascent: f32,
    pub descent: f32,
}

/// 字体状态；`generation` 进入整形缓存键，字体/字号/缩放/DPI 变化时递增并清空缓存。
pub(crate) struct FontState {
    pub fonts: [Font; 4],
    pub size: Pixels,
    pub line_height: f32,
    pub cell_width: f32,
    pub letter_spacing: f32,
    pub generation: u64,
    pub digits: Option<Rc<Digits>>,
    pub scale_factor: f32,
}

/// 上一帧的布局（事件处理用）。
#[derive(Clone, Debug, Default)]
pub(crate) struct FrameLayout {
    pub bounds: Bounds<Pixels>,
    pub gutter: Bounds<Pixels>,
    pub text: Bounds<Pixels>,
    pub vbar: Option<Bounds<Pixels>>,
    pub hbar: Option<Bounds<Pixels>>,
    pub vthumb: Option<Bounds<Pixels>>,
    pub hthumb: Option<Bounds<Pixels>>,
    pub metrics: Option<Metrics>,
    /// 行号栏里折叠控件所在的 x 范围（相对行号栏左边，像素）；没有控件时为空区间。
    pub fold_column: Range<f32>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Drag {
    Text {
        granularity: Granularity,
        origin: (Position, Position),
    },
    VThumb {
        grab: f64,
    },
    HThumb {
        grab: f64,
    },
}

struct Animation {
    from: f64,
    to: f64,
    start: Instant,
}

/// 代码视图。只读：选区、复制、滚动、缩放；编辑交互由 R46 在此基础上扩展。
pub struct CodeView {
    pub(crate) focus: FocusHandle,
    pub(crate) text: Option<Arc<dyn TextSource>>,
    pub(crate) services: CodeViewServices,
    pub(crate) language: Option<Arc<str>>,
    pub(crate) options: EditorOptions,
    pub(crate) palette: Palette,
    pub(crate) zoom: i32,
    pub(crate) scroll: ScrollPosition,
    animation: Option<Animation>,
    /// 嵌入区域（按屏幕行，滚动计算用）；由 `doc_zones` 经折叠映射得到。
    pub(crate) zones: ViewZones,
    /// 嵌入区域（按文档行，对外接口的数据）。
    pub(crate) doc_zones: ViewZones,
    pub(crate) folding: Folding,
    fold_task: u64,
    pub(crate) syntax: Rc<RefCell<SyntaxLayer>>,
    syntax_layer: LayerId,
    last_syntax_invalidation: Option<Option<Vec<Range<u64>>>>,
    brackets: Rc<RefCell<(Option<BracketMatch>, TextStyle)>>,
    bracket_layer: LayerId,
    pub(crate) occurrences: Rc<RefCell<Occurrences>>,
    occurrence_layer: LayerId,
    occurrence_epoch: Rc<Cell<u64>>,
    pub(crate) level: SizeClass,
    /// 当前滚动坐标所基于的映射（折叠变化时据此保持视口首行稳定）。
    applied_map: FoldMap,
    pub(crate) read_only: Arc<[ReadOnlyReason]>,
    pub(crate) hover_gutter: bool,
    pub(crate) cursor: Cursor,
    pub(crate) drag: Option<Drag>,
    pub(crate) decorations: DecorationStore,
    pub(crate) lines: Lru<u64, Rc<LineEntry>>,
    pub(crate) shapes: Lru<ShapeKey, Rc<Shaped>>,
    pub(crate) font: FontState,
    pub(crate) layout: FrameLayout,
    pub(crate) render_limit_override: Option<Option<usize>>,
    pub(crate) content_width: f64,
    pub(crate) visible: Range<u64>,
    last_top: f64,
    pub(crate) direction: i8,
    prepare_scheduled: bool,
    pub(crate) last_viewport: Option<Viewport>,
    pub(crate) caret_visible: bool,
    blink_epoch: Rc<Cell<u64>>,
    blinking: bool,
    pub(crate) focused: bool,
    pub(crate) hover_vthumb: bool,
    pub(crate) hover_hthumb: bool,
    pub(crate) frame_started: Option<Instant>,
    pub(crate) last_frame: FrameStats,
    pub(crate) recording: Option<Vec<FrameStats>>,
    pub(crate) slice_budget: Duration,
    pub(crate) glyphs_painted: Cell<u32>,
}

impl Focusable for CodeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CodeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui::{InteractiveElement, ParentElement, Styled};
        let mut context = gpui::KeyContext::default();
        context.add(super::commands::KEY_CONTEXT);
        for key in super::commands::FOCUS_CONTEXT_KEYS {
            context.add(*key);
        }
        if !self.cursor.selection.is_empty() {
            context.add("editorHasSelection");
        }
        if self.is_read_only() {
            context.add(super::commands::READONLY_CONTEXT_KEY);
        }
        if self.folding_enabled() {
            context.add(super::commands::FOLDING_CONTEXT_KEY);
        }
        if self.has_word_highlights() {
            context.add(super::commands::WORD_HIGHLIGHTS_CONTEXT_KEY);
        }
        let el = gpui::div()
            .key_context(context)
            .track_focus(&self.focus)
            .size_full()
            // 可打印字符：只读时在光标处提示原因（编辑本身由 R46 实现）。
            .on_key_down(cx.listener(|v, e: &gpui::KeyDownEvent, _, cx| {
                let m = &e.keystroke.modifiers;
                if e.keystroke.key_char.is_some() && !m.control && !m.alt && !m.platform {
                    v.typed(cx);
                }
            }))
            .child(super::element::CodeElement { view: cx.entity() });
        super::commands::on_actions(el, cx)
    }
}

fn content_hash(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn make_fonts(o: &EditorOptions) -> [Font; 4] {
    let features = match &o.ligatures {
        Ligatures::Off => FontFeatures(Arc::new(vec![("liga".into(), 0), ("calt".into(), 0)])),
        Ligatures::On => FontFeatures(Arc::new(vec![("liga".into(), 1), ("calt".into(), 1)])),
        Ligatures::Features(f) => FontFeatures(Arc::new(f.clone())),
    };
    let fallbacks =
        (!o.font_fallbacks.is_empty()).then(|| FontFallbacks::from_fonts(o.font_fallbacks.clone()));
    let base = Font {
        family: o.font_family.to_string().into(),
        features,
        fallbacks,
        weight: FontWeight(o.font_weight),
        style: FontStyle::Normal,
    };
    let bold = Font {
        weight: FontWeight::BOLD,
        ..base.clone()
    };
    let italic = Font {
        style: FontStyle::Italic,
        ..base.clone()
    };
    let bold_italic = Font {
        style: FontStyle::Italic,
        ..bold.clone()
    };
    [base, bold, italic, bold_italic]
}

impl CodeView {
    pub fn new(services: CodeViewServices, cx: &mut Context<Self>) -> Self {
        let options = EditorOptions::read(&*services.settings, None);
        let palette = Palette::resolve(&*services.theme, services.theme_kind);
        let font = FontState {
            fonts: make_fonts(&options),
            size: px(options.zoomed_font_size(0) as f32),
            line_height: options.zoomed_line_height(0) as f32,
            cell_width: options.zoomed_font_size(0) as f32 * 0.55,
            letter_spacing: options.letter_spacing as f32,
            generation: 1,
            digits: None,
            scale_factor: 0.0,
        };
        let focus = cx.focus_handle();
        let mut decorations = DecorationStore::default();
        let syntax = Rc::new(RefCell::new(SyntaxLayer::new(services.theme.clone())));
        let syntax_layer = decorations
            .add(
                "rove.syntax",
                priority::SYNTAX,
                Rc::new(SyntaxProvider(syntax.clone())),
            )
            .expect("built-in layer");
        let occurrences = Rc::new(RefCell::new(Occurrences::default()));
        let occurrence_layer = decorations
            .add(
                "rove.occurrences",
                priority::WORD_HIGHLIGHT,
                Rc::new(OccurrenceProvider(occurrences.clone())),
            )
            .expect("built-in layer");
        let brackets: Rc<RefCell<(Option<BracketMatch>, TextStyle)>> = Rc::default();
        let state = brackets.clone();
        let bracket_layer = decorations
            .add(
                "rove.bracketMatch",
                priority::BRACKET_MATCH,
                Rc::new(move |line: u64, _: &str, out: &mut LineDecorations| {
                    let (m, style) = &*state.borrow();
                    if let Some(m) = m {
                        for (l, r) in [&m.open, &m.close] {
                            if *l == line {
                                out.inline.push(InlineDecoration {
                                    range: r.clone(),
                                    style: *style,
                                });
                            }
                        }
                    }
                }),
            )
            .expect("built-in layer");
        let mut view = Self {
            focus,
            text: None,
            services,
            language: None,
            options,
            palette,
            zoom: 0,
            scroll: ScrollPosition::default(),
            animation: None,
            zones: ViewZones::default(),
            doc_zones: ViewZones::default(),
            folding: Folding::default(),
            fold_task: 0,
            syntax,
            syntax_layer,
            last_syntax_invalidation: None,
            brackets,
            bracket_layer,
            occurrences,
            occurrence_layer,
            occurrence_epoch: Rc::new(Cell::new(0)),
            level: SizeClass::Normal,
            applied_map: FoldMap::default(),
            read_only: Arc::from([]),
            hover_gutter: false,
            cursor: Cursor::default(),
            drag: None,
            decorations,
            lines: Lru::new(DEFAULT_LINE_CACHE_BYTES),
            shapes: Lru::new(DEFAULT_SHAPE_CACHE_BYTES),
            font,
            layout: FrameLayout::default(),
            render_limit_override: None,
            content_width: 0.0,
            visible: 0..0,
            last_top: 0.0,
            direction: 1,
            prepare_scheduled: false,
            last_viewport: None,
            caret_visible: true,
            blink_epoch: Rc::new(Cell::new(0)),
            blinking: false,
            focused: false,
            hover_vthumb: false,
            hover_hthumb: false,
            frame_started: None,
            last_frame: FrameStats::default(),
            recording: None,
            slice_budget: PREPARE_SLICE,
            glyphs_painted: Cell::new(0),
        };
        view.apply_feature_options();
        view
    }

    // ------------------------------------------------------------------ 文本源

    /// 设置或更换文本源。`keep_view_state` 为真时（同一文档的新快照）保留滚动与选区并钳制到新范围；
    /// 否则回到文首。整形缓存按内容键控，跨文档复用；行缓存与装饰缓存清空。
    pub fn set_text(
        &mut self,
        text: Option<Arc<dyn TextSource>>,
        keep_view_state: bool,
        cx: &mut Context<Self>,
    ) {
        self.text = text;
        self.lines.clear();
        self.content_width = 0.0;
        if keep_view_state {
            // 同一文档的新快照：装饰重算前沿用旧结果，折叠保留（区间在后台按新文本重算）。
            self.decorations.mark_all_stale();
            let s = self.clamp_selection(self.cursor.selection);
            self.cursor.set(s);
        } else {
            self.decorations.reset();
            self.scroll = ScrollPosition::default();
            self.animation = None;
            self.cursor.set(Selection::default());
            self.doc_zones = ViewZones::default();
            self.syntax.borrow_mut().current = None;
            self.folding.clear();
        }
        self.sync_fold_map();
        self.recompute_folds(cx);
        self.update_highlights(cx);
        cx.emit(CodeViewEvent::TextChanged);
        cx.notify();
    }

    pub fn text(&self) -> Option<&Arc<dyn TextSource>> {
        self.text.as_ref()
    }

    /// 文本快照版本（无文本时为 0）。
    pub fn version(&self) -> Version {
        self.text.as_ref().map_or(Version(0), |t| t.version())
    }

    /// 语言 ID（语言级设置覆盖，如 `"[csharp]"`）；变化时重读设置。
    pub fn set_language(&mut self, language: Option<Arc<str>>, cx: &mut Context<Self>) {
        if self.language != language {
            self.language = language;
            self.reload_options(cx);
            self.recompute_folds(cx);
            self.update_highlights(cx);
        }
    }

    /// 文档行数（已知部分，至少 1）。
    pub(crate) fn doc_line_count(&self) -> u64 {
        self.text.as_ref().map_or(1, |t| t.lines().known.max(1))
    }

    /// 屏幕行数（折叠隐藏的行不算）。
    pub(crate) fn line_count(&self) -> u64 {
        self.folding.map().view_count(self.doc_line_count()).max(1)
    }

    /// 文档行 → 屏幕行（隐藏行映射到折叠首行）。
    pub(crate) fn view_line(&self, doc: u64) -> u64 {
        self.folding.map().to_view(doc)
    }

    /// 屏幕行 → 文档行。
    pub(crate) fn doc_line(&self, view: u64) -> u64 {
        self.folding.map().to_doc(view)
    }

    // ------------------------------------------------------------------ 设置、主题、字体

    /// 重新读取设置（#13 发出设置变更事件后调用）。字体相关项变化会让整形缓存整体失效。
    pub fn reload_options(&mut self, cx: &mut Context<Self>) {
        let options = EditorOptions::read(&*self.services.settings, self.language.as_deref());
        self.set_options(options, cx);
    }

    /// 直接设置选项快照（测试与嵌入方使用）。
    pub fn set_options(&mut self, options: EditorOptions, cx: &mut Context<Self>) {
        let layout_changed = options.tab_size != self.options.tab_size
            || options.stop_rendering_line_after != self.options.stop_rendering_line_after;
        let folds_changed = options.tab_size != self.options.tab_size
            || options.folding != self.options.folding
            || options.folding_strategy != self.options.folding_strategy
            || options.folding_maximum_regions != self.options.folding_maximum_regions
            || options.large_file_optimizations != self.options.large_file_optimizations;
        let tokens_changed =
            options.max_tokenization_line_length != self.options.max_tokenization_line_length;
        let highlights_changed = options.match_brackets != self.options.match_brackets
            || options.occurrences_highlight != self.options.occurrences_highlight
            || options.selection_highlight != self.options.selection_highlight
            || options.selection_highlight_max_length
                != self.options.selection_highlight_max_length
            || options.selection_highlight_multiline != self.options.selection_highlight_multiline
            || options.selected_text_case_sensitive != self.options.selected_text_case_sensitive
            || options.word_separators != self.options.word_separators;
        self.options = options;
        if layout_changed {
            self.lines.clear();
            self.content_width = 0.0;
        }
        self.apply_feature_options();
        if tokens_changed {
            self.decorations.invalidate(&self.syntax_layer, None);
        }
        if folds_changed {
            self.recompute_folds(cx);
        }
        if highlights_changed {
            self.update_highlights(cx);
        }
        self.refresh_font();
        cx.notify();
    }

    pub fn options(&self) -> &EditorOptions {
        &self.options
    }

    /// 更换主题（#10 主题切换）。颜色在绘制时上色，不影响整形缓存。
    pub fn set_theme(&mut self, theme: Arc<dyn Theme>, kind: ThemeKind, cx: &mut Context<Self>) {
        self.services.theme = theme;
        self.services.theme_kind = kind;
        self.palette = Palette::resolve(&*self.services.theme, kind);
        // 主题切换按 issue 要求视为字体/度量变化：整形结果整体失效（字体可能随主题变化）。
        self.invalidate_shapes();
        // token 颜色与 fontStyle 整体重算（旧结果在重算前继续绘制）；同名高亮与括号换新配色。
        self.syntax.borrow_mut().theme = self.services.theme.clone();
        self.apply_feature_options();
        for layer in [
            &self.syntax_layer,
            &self.occurrence_layer,
            &self.bracket_layer,
        ] {
            self.decorations.invalidate(layer, None);
        }
        cx.notify();
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// 字体或度量变化：整形缓存整体失效。DPI 变化在绘制时检测（#30 协调清空时机）。
    fn refresh_font(&mut self) {
        let o = &self.options;
        let fonts = make_fonts(o);
        let size = px(o.zoomed_font_size(self.zoom) as f32);
        let line_height = o.zoomed_line_height(self.zoom) as f32;
        let letter_spacing = o.letter_spacing as f32;
        if fonts != self.font.fonts
            || size != self.font.size
            || line_height != self.font.line_height
            || letter_spacing != self.font.letter_spacing
        {
            self.font.fonts = fonts;
            self.font.size = size;
            self.font.line_height = line_height;
            self.font.letter_spacing = letter_spacing;
            self.font.cell_width = f32::from(size) * 0.55;
            self.invalidate_shapes();
        }
    }

    pub(crate) fn invalidate_shapes(&mut self) {
        self.font.generation += 1;
        self.font.digits = None;
        self.shapes.clear();
        self.content_width = 0.0;
    }

    /// 当前缩放级别（VS Code `EditorZoom`，-5..=20，每级 10%）。
    pub fn zoom_level(&self) -> i32 {
        self.zoom
    }

    pub fn set_zoom_level(&mut self, level: i32, cx: &mut Context<Self>) {
        let level = level.clamp(MIN_ZOOM, MAX_ZOOM);
        if level != self.zoom {
            // 缩放前记住顶部行，缩放后保持同一行在顶部。
            let (first, _) = self.first_visible();
            self.zoom = level;
            self.refresh_font();
            if let Some(m) = self.metrics() {
                self.scroll.top = m.line_top(first, &self.zones);
            }
            cx.notify();
        }
    }

    /// 实际字号（像素，含缩放）。
    pub fn font_size(&self) -> f32 {
        self.font.size.into()
    }

    /// 实际行高（像素，含缩放）。
    pub fn line_height(&self) -> f32 {
        self.font.line_height
    }

    // ------------------------------------------------------------------ 装饰

    /// 登记一层装饰（多个提供方叠加；数值大的优先级覆盖小的）。重复 ID 返回 `Duplicate`。
    pub fn add_decorations(
        &mut self,
        id: &str,
        priority: i32,
        provider: Rc<dyn DecorationProvider>,
        cx: &mut Context<Self>,
    ) -> rove_base::Result<LayerId> {
        let id = self.decorations.add(id, priority, provider)?;
        cx.notify();
        Ok(id)
    }

    pub fn remove_decorations(&mut self, id: &LayerId, cx: &mut Context<Self>) -> bool {
        let removed = self.decorations.remove(id);
        if removed {
            cx.notify();
        }
        removed
    }

    /// 让某层的若干行（`None` 为全部）失效；下一个空闲切片重新调用提供方。
    pub fn invalidate_decorations(
        &mut self,
        id: &LayerId,
        lines: Option<Range<u64>>,
        cx: &mut Context<Self>,
    ) {
        self.decorations.invalidate(id, lines);
        cx.notify();
    }

    /// 已合并的装饰（只读缓存；行尚未准备时为 `None`）。
    pub fn merged_decorations(&self, line: u64) -> Option<Rc<MergedLine>> {
        self.lines.peek(&line).and_then(|e| e.merged.clone())
    }

    // ------------------------------------------------------------------ 行下方嵌入区域、渲染上限

    /// 在 `after` 行下方保留 `height` 像素（0 删除）。供 #41 速览窗口、#22 截断提示等使用。
    pub fn set_view_zone(&mut self, after: u64, height: f32, cx: &mut Context<Self>) {
        self.doc_zones.set(after, height as f64);
        self.sync_fold_map();
        cx.notify();
    }

    /// 嵌入区域在窗口中的位置（行已滚出视口时为 `None`）。消费方据此摆放自己的元素。
    pub fn view_zone_bounds(&self, after: u64) -> Option<Bounds<Pixels>> {
        let h = self.doc_zones.get(after);
        let m = self.metrics()?;
        let m = &m;
        if h <= 0.0 || self.folding.map().is_hidden(after) {
            return None;
        }
        let after = self.view_line(after);
        // 嵌入区域紧接在 `after` 行之后。
        let y =
            ((m.line_top(after, &self.zones) + 1.0 - self.current_top()) * m.line_height) as f32;
        let text = self.layout.text;
        let top = text.origin.y + px(y);
        if y + h as f32 <= 0.0 || top > text.origin.y + text.size.height {
            return None;
        }
        Some(Bounds::new(
            gpui::point(text.origin.x, top),
            gpui::size(text.size.width, px(h as f32)),
        ))
    }

    /// 单行渲染上限挂钩（字符数）：`Some(None)` 为不限，`Some(Some(n))` 为 n 个字符，
    /// `None` 恢复按 `editor.stopRenderingLineAfter`。由 #22 接入。
    pub fn set_line_render_limit(&mut self, limit: Option<Option<usize>>, cx: &mut Context<Self>) {
        if self.render_limit_override != limit {
            self.render_limit_override = limit;
            self.lines.clear();
            cx.notify();
        }
    }

    pub(crate) fn render_limit(&self) -> Option<usize> {
        self.render_limit_override
            .unwrap_or(self.options.stop_rendering_line_after)
    }

    /// 已准备（可绘制）的某行源文本；行尚未进入缓存时为 `None`。供测试与 #20 断言使用。
    pub fn painted_text(&self, line: u64) -> Option<Arc<str>> {
        self.lines.peek(&line).map(|e| e.source.clone())
    }

    /// 某行是否已有正式整形结果（预取的行也算）。
    pub fn is_line_shaped(&self, line: u64) -> bool {
        self.lines
            .peek(&line)
            .is_some_and(|e| self.shapes.contains(&self.shape_key(e)))
    }

    /// 平滑滚动动画是否进行中。
    pub fn is_animating(&self) -> bool {
        self.animation.is_some()
    }

    /// 某行是否因渲染上限被截断（行尚未准备时为 `None`）。
    pub fn is_line_truncated(&self, line: u64) -> Option<bool> {
        self.lines.peek(&line).map(|e| e.display.truncated)
    }

    // ------------------------------------------------------------------ 选区

    pub fn selection(&self) -> Selection {
        self.cursor.selection
    }

    /// 设置选区（坐标会钳制到合法范围并对齐字符边界）。`reveal` 为 `Some` 时滚动使插入符可见。
    pub fn set_selection(
        &mut self,
        selection: Selection,
        reveal: Option<Reveal>,
        cx: &mut Context<Self>,
    ) {
        let s = self.clamp_selection(selection);
        // 光标落进折叠：展开（VS Code 行为）。
        self.unfold_to_reveal(s.active.line.0, cx);
        self.cursor.set(s);
        if let Some(r) = reveal {
            self.reveal_position(s.active, r, cx);
        }
        self.selection_changed(SelectionSource::Api, cx);
    }

    fn clamp_selection(&self, s: Selection) -> Selection {
        Selection::new(self.clamp_position(s.anchor), self.clamp_position(s.active))
    }

    fn clamp_position(&self, p: Position) -> Position {
        let Some(text) = self.text.as_deref() else {
            return Position::default();
        };
        let text = TextLines(text);
        let last = text.line_count().saturating_sub(1);
        let line = p.line.0.min(last);
        let t = text.line_text(line).unwrap_or_default();
        let mut col = (p.column.0 as usize).min(t.len());
        while !t.is_char_boundary(col) {
            col -= 1;
        }
        pos(line, col as u64)
    }

    /// 选区文本（保持原始换行符，含 CRLF）。
    pub fn selected_text(&self) -> Option<String> {
        let text = self.text.as_deref()?;
        let s = self.cursor.selection;
        let start = text.offset(s.start()).ok()?;
        let end = text.offset(s.end()).ok()?;
        let mut out = String::new();
        text.visit_chunks(rove_base::Span::new(start, end).ok()?, &mut |c| {
            out.push_str(c)
        })
        .ok()?;
        Some(out)
    }

    /// 复制时的文本：有选区复制选区；无选区且 `emptySelectionClipboard` 时复制当前行（含行终止符）。
    pub fn copy_text(&self) -> Option<String> {
        let text = self.text.as_deref()?;
        let s = self.cursor.selection;
        if !s.is_empty() {
            return self.selected_text();
        }
        if !self.options.empty_selection_clipboard {
            return None;
        }
        let line = s.active.line;
        let range = text.line_range(line).ok()?;
        let last = TextLines(text).line_count().saturating_sub(1);
        let end = if line.0 < last {
            text.line_range(Line(line.0 + 1)).ok()?.start()
        } else {
            range.end()
        };
        let mut out = String::new();
        text.visit_chunks(rove_base::Span::new(range.start(), end).ok()?, &mut |c| {
            out.push_str(c)
        })
        .ok()?;
        if line.0 >= last {
            // 末行没有终止符：VS Code 整行复制时补上文档的换行符。
            out.push_str(&self.document_eol());
        }
        Some(out)
    }

    /// 文档使用的换行符（取第一行的终止符；单行文件为 `\n`）。
    fn document_eol(&self) -> String {
        let Some(text) = self.text.as_deref() else {
            return "\n".into();
        };
        if TextLines(text).line_count() < 2 {
            return "\n".into();
        }
        let (Ok(a), Ok(b)) = (text.line_range(Line(0)), text.line_range(Line(1))) else {
            return "\n".into();
        };
        let mut out = String::new();
        if let Ok(span) = rove_base::Span::new(a.end(), b.start()) {
            let _ = text.visit_chunks(span, &mut |c| out.push_str(c));
        }
        if out.is_empty() {
            "\n".into()
        } else {
            out
        }
    }

    fn selection_changed(&mut self, source: SelectionSource, cx: &mut Context<Self>) {
        self.update_highlights(cx);
        self.caret_visible = true;
        self.blink_epoch.set(self.blink_epoch.get() + 1);
        self.blinking = false;
        cx.emit(CodeViewEvent::SelectionChanged {
            selection: self.cursor.selection,
            source,
        });
        cx.notify();
    }

    // ------------------------------------------------------------------ 滚动定位

    pub fn scroll_position(&self) -> ScrollPosition {
        self.scroll
    }

    /// 视口（上一帧布局的结果）。
    pub fn viewport(&self) -> Option<Viewport> {
        self.last_viewport
    }

    pub(crate) fn metrics(&self) -> Option<Metrics> {
        let mut m = self.layout.metrics.clone()?;
        m.line_count = self.line_count();
        m.line_height = self.font.line_height as f64;
        Some(m)
    }

    pub(crate) fn first_visible(&self) -> (u64, f64) {
        match self.metrics() {
            Some(m) => self.scroll.first_visible(&m, &self.zones),
            None => (0, 0.0),
        }
    }

    /// 设置滚动位置（f64 虚拟行与水平像素）。`animate` 时遵循 `editor.smoothScrolling`。
    pub fn set_scroll_position(
        &mut self,
        p: ScrollPosition,
        animate: bool,
        cx: &mut Context<Self>,
    ) {
        let target = match self.metrics() {
            Some(m) => p.clamped(&m, &self.zones),
            None => ScrollPosition {
                top: p.top.max(0.0),
                x: p.x.max(0.0),
            },
        };
        if animate && self.options.smooth_scrolling && (target.top - self.scroll.top).abs() > 1e-9 {
            self.animation = Some(Animation {
                from: self.current_top(),
                to: target.top,
                start: Instant::now(),
            });
            self.scroll.x = target.x;
            self.scroll.top = target.top;
        } else {
            self.animation = None;
            self.scroll = target;
        }
        cx.notify();
    }

    /// 动画中的实际顶边位置（`scroll.top` 始终是目标位置）。
    pub(crate) fn current_top(&self) -> f64 {
        match &self.animation {
            Some(a) => {
                let t = a.start.elapsed().as_secs_f64() * 1000.0 / scroll::SMOOTH_SCROLL_MS;
                a.from + (a.to - a.from) * scroll::ease_out_cubic(t)
            }
            None => self.scroll.top,
        }
    }

    pub(crate) fn animating(&mut self) -> bool {
        if let Some(a) = &self.animation {
            if a.start.elapsed().as_secs_f64() * 1000.0 >= scroll::SMOOTH_SCROLL_MS {
                self.animation = None;
            }
        }
        self.animation.is_some()
    }

    /// 跳转到任意行（`workbench.action.gotoLine` 的落点接口）。行号超出已知范围时钳制到最后一个已知行。
    pub fn reveal_line(&mut self, line: u64, reveal: Reveal, cx: &mut Context<Self>) {
        let line = line.min(self.doc_line_count().saturating_sub(1));
        self.unfold_to_reveal(line, cx);
        let line = self.view_line(line);
        match self.metrics() {
            Some(m) => {
                let p = self.scroll.reveal_line(line, reveal, 0, &m, &self.zones);
                self.set_scroll_position(p, true, cx);
            }
            None => {
                // 尚未布局：记下目标，首帧布局时按 Top 落位。
                self.scroll.top = line as f64;
                cx.notify();
            }
        }
    }

    /// 让某个位置（行 + 列）可见。
    pub fn reveal_position(&mut self, p: Position, reveal: Reveal, cx: &mut Context<Self>) {
        let p = self.clamp_position(p);
        let surrounding = self.options.cursor_surrounding_lines;
        self.unfold_to_reveal(p.line.0, cx);
        if let Some(m) = self.metrics() {
            let line = self.view_line(p.line.0);
            let mut s = self
                .scroll
                .reveal_line(line, reveal, surrounding, &m, &self.zones);
            if let Some((x0, x1)) = self.caret_x_range(p) {
                s = s.reveal_x(x0, x1 + self.font.cell_width as f64, &m);
            }
            self.set_scroll_position(s, true, cx);
        } else {
            self.reveal_line(p.line.0, reveal, cx);
        }
    }

    /// 某位置在文本坐标系中的 x 范围（像素，相对文本起点）。
    fn caret_x_range(&mut self, p: Position) -> Option<(f64, f64)> {
        let (entry, shaped) = self.line_geometry(p.line.0)?;
        let xs = self.xs_for(&entry, shaped.as_deref());
        let x = entry.display.x_for_source(&xs, p.column.0 as u32) as f64;
        Some((x, x))
    }

    // ------------------------------------------------------------------ 坐标换算（命中测试）

    /// 窗口坐标 → 文档位置（行 + UTF-8 列），以上一帧布局为准。点在文本区之外时按最近位置钳制。
    pub fn position_for_point(&mut self, point: Point<Pixels>) -> Option<Position> {
        let m = self.metrics()?;
        let text = self.layout.text;
        let y = (point.y - text.origin.y).to_f64();
        let coord = self.current_top() + y / m.line_height;
        let line = self.doc_line(line_at_coord(coord, &m, &self.zones));
        let x = (point.x - text.origin.x).to_f64() + self.scroll.x;
        let (entry, shaped) = self.line_geometry(line)?;
        let xs = self.xs_for(&entry, shaped.as_deref());
        let col = entry.display.source_for_x(&xs, x as f32);
        Some(self.clamp_position(pos(line, col as u64)))
    }

    /// 文档位置 → 窗口坐标（该位置左上角）。行不在视口内时为 `None`。
    pub fn point_for_position(&mut self, p: Position) -> Option<Point<Pixels>> {
        let m = self.metrics()?;
        if self.folding.map().is_hidden(p.line.0) {
            return None;
        }
        let view = self.view_line(p.line.0);
        let top = (m.line_top(view, &self.zones) - self.current_top()) * m.line_height;
        if top + m.line_height < 0.0 || top > m.viewport_height {
            return None;
        }
        let (x, _) = self.caret_x_range(p)?;
        let text = self.layout.text;
        Some(gpui::point(
            text.origin.x + px((x - self.scroll.x) as f32),
            text.origin.y + px(top as f32),
        ))
    }

    /// 取某行的显示形式与（若已缓存的）整形结果；行缓存未命中时同步读这一行（只用于事件处理）。
    pub(crate) fn line_geometry(
        &mut self,
        line: u64,
    ) -> Option<(Rc<LineEntry>, Option<Rc<Shaped>>)> {
        let entry = match self.lines.peek(&line) {
            Some(e) => e.clone(),
            None => self.build_entry(line)?,
        };
        let shaped = self.shapes.peek(&self.shape_key(&entry)).cloned();
        Some((entry, shaped))
    }

    // ------------------------------------------------------------------ 命令

    /// 按 VS Code 命令 ID 执行（#14 的命令桥调用这里）。未知 ID 返回 `false`。
    pub fn execute(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let _ = window;
        let page = self.metrics().map_or(1, |m| m.page_lines());
        let mv = |m: Move, extend: bool| Some((m, extend));
        let movement = match id {
            "cursorUp" => mv(Move::Up, false),
            "cursorUpSelect" => mv(Move::Up, true),
            "cursorDown" => mv(Move::Down, false),
            "cursorDownSelect" => mv(Move::Down, true),
            "cursorLeft" => mv(Move::Left, false),
            "cursorLeftSelect" => mv(Move::Left, true),
            "cursorRight" => mv(Move::Right, false),
            "cursorRightSelect" => mv(Move::Right, true),
            "cursorHome" => mv(Move::Home, false),
            "cursorHomeSelect" => mv(Move::Home, true),
            "cursorEnd" => mv(Move::End, false),
            "cursorEndSelect" => mv(Move::End, true),
            "cursorTop" => mv(Move::Top, false),
            "cursorTopSelect" => mv(Move::Top, true),
            "cursorBottom" => mv(Move::Bottom, false),
            "cursorBottomSelect" => mv(Move::Bottom, true),
            "cursorPageUp" => mv(Move::PageUp, false),
            "cursorPageUpSelect" => mv(Move::PageUp, true),
            "cursorPageDown" => mv(Move::PageDown, false),
            "cursorPageDownSelect" => mv(Move::PageDown, true),
            _ => None,
        };
        if let Some((m, extend)) = movement {
            let Some(text) = self.text.clone() else {
                return true;
            };
            let tab = self.options.tab_size;
            let before = self.cursor.selection.active.line.0;
            self.cursor
                .move_caret(&TextLines(&*text), m, extend, tab, page);
            self.skip_folded_lines(before);
            self.reveal_caret(cx);
            self.selection_changed(SelectionSource::Keyboard, cx);
            return true;
        }
        match id {
            "scrollLineUp" | "scrollLineDown" | "scrollPageUp" | "scrollPageDown" => {
                let n = if id.contains("Page") {
                    page as f64
                } else {
                    1.0
                };
                let d = if id.ends_with("Up") { -n } else { n };
                let p = ScrollPosition {
                    top: self.scroll.top + d,
                    x: self.scroll.x,
                };
                self.set_scroll_position(p, true, cx);
            }
            "editor.action.selectAll" => {
                if let Some(text) = self.text.clone() {
                    self.cursor.select_all(&TextLines(&*text));
                    self.selection_changed(SelectionSource::Keyboard, cx);
                }
            }
            "expandLineSelection" => {
                if let Some(text) = self.text.clone() {
                    self.cursor.expand_line_selection(&TextLines(&*text));
                    self.reveal_caret(cx);
                    self.selection_changed(SelectionSource::Keyboard, cx);
                }
            }
            "cancelSelection" => {
                self.cursor.cancel_selection();
                self.selection_changed(SelectionSource::Keyboard, cx);
            }
            "editor.action.clipboardCopyAction" => {
                if let Some(s) = self.copy_text() {
                    cx.write_to_clipboard(ClipboardItem::new_string(s));
                }
            }
            "workbench.action.files.toggleActiveEditorReadonlyInSession" => {
                cx.emit(CodeViewEvent::ToggleReadonlyRequested);
            }
            _ if super::folding::FOLD_COMMANDS.contains(&id) => {
                if self.folding_enabled() {
                    let s = self.cursor.selection;
                    let lines = [s.active.line.0];
                    let mut end = s.end().line.0;
                    // VS Code：选区止于下一行行首时不含那一行。
                    if end > s.start().line.0 && s.end().column.0 == 0 {
                        end -= 1;
                    }
                    let before = self.folding.generation;
                    self.folding.execute(id, &lines, (s.start().line.0, end));
                    if self.folding.generation != before {
                        self.folds_changed(cx);
                    }
                }
            }
            "editor.action.fontZoomIn" => self.set_zoom_level(self.zoom + 1, cx),
            "editor.action.fontZoomOut" => self.set_zoom_level(self.zoom - 1, cx),
            "editor.action.fontZoomReset" => self.set_zoom_level(0, cx),
            _ => return false,
        }
        true
    }

    fn reveal_caret(&mut self, cx: &mut Context<Self>) {
        let p = self.cursor.selection.active;
        self.reveal_position(p, Reveal::Nearest, cx);
    }

    // ------------------------------------------------------------------ 鼠标

    pub(crate) fn mouse_down_text(
        &mut self,
        point: Point<Pixels>,
        click_count: usize,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = self.position_for_point(point) else {
            return;
        };
        let Some(text) = self.text.clone() else {
            return;
        };
        if click_count == 1 && !shift && self.click_unfolds(point, p.line.0) {
            if self.folding.toggle_header(p.line.0) {
                self.folds_changed(cx);
            }
            return;
        }
        let separators = self.options.word_separators.clone();
        let (granularity, unit) = match click_count {
            1 => (Granularity::Character, (p, p)),
            2 => {
                let t = TextLines(&*text).line_text(p.line.0).unwrap_or_default();
                let (a, b) = selection::word_range_at(&t, p.column.0 as usize, &separators);
                (
                    Granularity::Word,
                    (pos(p.line.0, a as u64), pos(p.line.0, b as u64)),
                )
            }
            _ => (Granularity::Line, line_unit(&TextLines(&*text), p.line.0)),
        };
        let sel = if shift && click_count == 1 {
            Selection::new(self.cursor.selection.anchor, p)
        } else if click_count == 1 {
            Selection::caret(p)
        } else {
            Selection::new(unit.0, unit.1)
        };
        let origin = if shift && click_count == 1 {
            let a = self.cursor.selection.anchor;
            (a, a)
        } else {
            unit
        };
        self.cursor.set(sel);
        self.drag = Some(Drag::Text {
            granularity,
            origin,
        });
        if self.options.cursor_surrounding_all {
            self.reveal_caret(cx);
        }
        self.selection_changed(SelectionSource::Mouse, cx);
    }

    /// 点击行号选中整行（VS Code 行为）。
    pub(crate) fn mouse_down_gutter(
        &mut self,
        point: Point<Pixels>,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(m) = self.metrics() else { return };
        let Some(text) = self.text.clone() else {
            return;
        };
        let y = (point.y - self.layout.text.origin.y).to_f64();
        let view = line_at_coord(self.current_top() + y / m.line_height, &m, &self.zones);
        let line = self.doc_line(view);
        // 折叠控件列：点击切换该行开始的折叠。
        let x = f32::from(point.x - self.layout.gutter.origin.x);
        if self.layout.fold_column.contains(&x) && self.folding.header(line).is_some() {
            if self.folding.toggle_header(line) {
                self.folds_changed(cx);
            }
            return;
        }
        let unit = line_unit(&TextLines(&*text), line);
        let sel = if shift {
            let a = self.cursor.selection.anchor;
            extend_by_unit((a, a), unit)
        } else {
            Selection::new(unit.0, unit.1)
        };
        self.cursor.set(sel);
        self.drag = Some(Drag::Text {
            granularity: Granularity::Line,
            origin: if shift {
                (self.cursor.selection.anchor, self.cursor.selection.anchor)
            } else {
                unit
            },
        });
        self.selection_changed(SelectionSource::Mouse, cx);
    }

    pub(crate) fn mouse_drag_text(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(Drag::Text {
            granularity,
            origin,
        }) = self.drag
        else {
            return;
        };
        let Some(p) = self.position_for_point(point) else {
            return;
        };
        let Some(text) = self.text.clone() else {
            return;
        };
        let current = match granularity {
            Granularity::Character => (p, p),
            Granularity::Word => {
                let t = TextLines(&*text).line_text(p.line.0).unwrap_or_default();
                let (a, b) = selection::word_range_for_drag(
                    &t,
                    p.column.0 as usize,
                    &self.options.word_separators,
                );
                (pos(p.line.0, a as u64), pos(p.line.0, b as u64))
            }
            Granularity::Line => line_unit(&TextLines(&*text), p.line.0),
        };
        let sel = if granularity == Granularity::Character {
            Selection::new(origin.0, p)
        } else {
            extend_by_unit(origin, current)
        };
        if sel != self.cursor.selection {
            self.cursor.set(sel);
            // 拖出视口时自动滚动（不受 cursorSurroundingLines 影响）。
            if let Some(m) = self.metrics() {
                let view = self.view_line(sel.active.line.0);
                let s = self
                    .scroll
                    .reveal_line(view, Reveal::Nearest, 0, &m, &self.zones);
                if s != self.scroll {
                    self.set_scroll_position(s, false, cx);
                }
            }
            self.selection_changed(SelectionSource::Mouse, cx);
        }
    }

    pub(crate) fn mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    /// 滚动条轨道点击：`scrollByPage` 时翻页，否则把滑块中心移到点击处（VS Code 行为）。
    pub(crate) fn scrollbar_down(
        &mut self,
        point: Point<Pixels>,
        vertical: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(m) = self.metrics() else { return };
        let (bar, thumb) = if vertical {
            (self.layout.vbar, self.layout.vthumb)
        } else {
            (self.layout.hbar, self.layout.hthumb)
        };
        let (Some(bar), Some(thumb)) = (bar, thumb) else {
            return;
        };
        let along = |p: Point<Pixels>, b: Bounds<Pixels>| -> f64 {
            if vertical {
                (p.y - b.origin.y).to_f64()
            } else {
                (p.x - b.origin.x).to_f64()
            }
        };
        let (track, thumb_start, thumb_len) = if vertical {
            (
                bar.size.height.to_f64(),
                along(thumb.origin, bar),
                thumb.size.height.to_f64(),
            )
        } else {
            (
                bar.size.width.to_f64(),
                along(thumb.origin, bar),
                thumb.size.width.to_f64(),
            )
        };
        let at = along(point, bar);
        if at >= thumb_start && at <= thumb_start + thumb_len {
            let grab = at - thumb_start;
            self.drag = Some(if vertical {
                Drag::VThumb { grab }
            } else {
                Drag::HThumb { grab }
            });
            cx.notify();
            return;
        }
        let mut p = self.scroll;
        if self.options.scroll_by_page {
            let dir = if at < thumb_start { -1.0 } else { 1.0 };
            if vertical {
                p.top += dir * m.page_lines() as f64;
            } else {
                p.x += dir * m.viewport_width;
            }
        } else if vertical {
            let total = m.total_lines(&self.zones);
            p.top = scroll::pos_for_thumb(track, m.viewport_lines(), total, at - thumb_len / 2.0);
        } else {
            let total = m.content_width + m.beyond_last_column;
            p.x = scroll::pos_for_thumb(track, m.viewport_width, total, at - thumb_len / 2.0);
        }
        self.set_scroll_position(p, false, cx);
    }

    pub(crate) fn scrollbar_drag(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(m) = self.metrics() else { return };
        match self.drag {
            Some(Drag::VThumb { grab }) => {
                let Some(bar) = self.layout.vbar else { return };
                let at = (point.y - bar.origin.y).to_f64() - grab;
                let total = m.total_lines(&self.zones);
                let top =
                    scroll::pos_for_thumb(bar.size.height.to_f64(), m.viewport_lines(), total, at);
                self.set_scroll_position(
                    ScrollPosition {
                        top,
                        x: self.scroll.x,
                    },
                    false,
                    cx,
                );
            }
            Some(Drag::HThumb { grab }) => {
                let Some(bar) = self.layout.hbar else { return };
                let at = (point.x - bar.origin.x).to_f64() - grab;
                let total = m.content_width + m.beyond_last_column;
                let x = scroll::pos_for_thumb(bar.size.width.to_f64(), m.viewport_width, total, at);
                self.set_scroll_position(
                    ScrollPosition {
                        top: self.scroll.top,
                        x,
                    },
                    false,
                    cx,
                );
            }
            _ => {}
        }
    }

    /// 滚轮：`Ctrl` + 滚轮在 `mouseWheelZoom` 时缩放；`Alt` 加速；`Shift` 水平滚动。
    pub(crate) fn wheel(
        &mut self,
        delta: gpui::ScrollDelta,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        let lh = self.font.line_height as f64;
        let (dx, dy) = match delta {
            gpui::ScrollDelta::Lines(p) => (p.x as f64 * lh, p.y as f64 * lh),
            gpui::ScrollDelta::Pixels(p) => (p.x.to_f64(), p.y.to_f64()),
        };
        if modifiers.control {
            if self.options.mouse_wheel_zoom && dy != 0.0 {
                let level = self.zoom + if dy > 0.0 { 1 } else { -1 };
                self.set_zoom_level(level, cx);
            }
            return;
        }
        let o = WheelOptions {
            sensitivity: self.options.wheel_sensitivity,
            fast_sensitivity: self.options.fast_scroll_sensitivity,
            predominant_axis: self.options.scroll_predominant_axis,
        };
        let (dx, dy) = scroll::wheel_delta(dx, dy, modifiers.alt, modifiers.shift, &o);
        // 内容随手势移动：向下滚（dy < 0）时视口向下。
        let p = ScrollPosition {
            top: self.scroll.top - dy / lh,
            x: self.scroll.x - dx,
        };
        let pixel_precise = matches!(delta, gpui::ScrollDelta::Pixels(_));
        self.set_scroll_position(p, !pixel_precise, cx);
    }

    // ------------------------------------------------------------------ 焦点与插入符闪烁

    pub(crate) fn focus_changed(
        &mut self,
        focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if focused == self.focused {
            return;
        }
        self.focused = focused;
        self.caret_visible = true;
        self.blinking = false;
        self.blink_epoch.set(self.blink_epoch.get() + 1);
        if focused {
            self.start_blink(window, cx);
        }
    }

    pub(crate) fn start_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use super::options::CursorBlinking;
        if self.blinking || !self.focused || self.options.cursor_blinking == CursorBlinking::Solid {
            return;
        }
        self.blinking = true;
        let epoch = self.blink_epoch.clone();
        let mine = epoch.get();
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(BLINK_INTERVAL).await;
            if epoch.get() != mine {
                break;
            }
            let alive = this.update(cx, |v, cx| {
                v.caret_visible = !v.caret_visible;
                cx.notify();
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }

    // ------------------------------------------------------------------ 帧外准备（读文本、装饰、整形、预取）

    pub(crate) fn shape_key(&self, e: &LineEntry) -> ShapeKey {
        ShapeKey {
            hash: e.hash,
            text: e.display.text.clone(),
            runs: e.font_runs.clone(),
            font_generation: self.font.generation,
        }
    }

    /// 读一行并构建显示形式与装饰（不整形）。
    fn build_entry(&mut self, line: u64) -> Option<Rc<LineEntry>> {
        let text = self.text.clone()?;
        if line >= text.lines().known {
            return None;
        }
        let source: Arc<str> = read_line(&*text, line)?.into();
        let display = DisplayLine::build(&source, self.options.tab_size, self.render_limit());
        let entry = self.decorate_entry(line, display, source);
        Some(self.store_entry(line, entry))
    }

    fn decorate_entry(&mut self, line: u64, display: DisplayLine, source: Arc<str>) -> LineEntry {
        self.decorations.prepare(line, &source);
        let merged = self.decorations.cached(line).cloned();
        let mut paint_runs = Vec::new();
        let mut font_runs: Vec<(u32, u8)> = Vec::new();
        let total = display.text.len() as u32;
        let mut cursor = 0u32;
        if let Some(m) = &merged {
            for r in &m.runs {
                let a = display.display_for_source(r.range.start);
                let b = display.display_for_source(r.range.end.min(display.rendered_source_len()));
                if a >= b {
                    continue;
                }
                paint_runs.push((a..b, r.style));
                let bits = r.style.bold.unwrap_or(false) as u8
                    | (r.style.italic.unwrap_or(false) as u8) << 1;
                if bits != 0 {
                    if a > cursor {
                        font_runs.push((a - cursor, 0));
                    }
                    font_runs.push((b - a, bits));
                    cursor = b;
                }
            }
        }
        if cursor < total || font_runs.is_empty() {
            font_runs.push((total - cursor, 0));
        }
        LineEntry {
            hash: content_hash(&display.text),
            display,
            source,
            merged,
            paint_runs,
            font_runs: font_runs.into(),
            xs: RefCell::new(None),
        }
    }

    fn store_entry(&mut self, line: u64, entry: LineEntry) -> Rc<LineEntry> {
        let cost =
            entry.source.len() + entry.display.text.len() + entry.display.stops().len() * 12 + 128;
        let e = Rc::new(entry);
        self.lines.insert(line, e.clone(), cost);
        e
    }

    /// 准备一行：保证显示形式与装饰最新。返回是否有变化。
    fn ensure_entry(&mut self, line: u64) -> bool {
        match self.lines.peek(&line).cloned() {
            Some(e) => {
                if self.decorations.needs(line) {
                    let entry = self.decorate_entry(line, e.display.clone(), e.source.clone());
                    let changed = entry.paint_runs != e.paint_runs || entry.merged != e.merged;
                    self.store_entry(line, entry);
                    changed
                } else {
                    false
                }
            }
            None => self.build_entry(line).is_some(),
        }
    }

    /// 整形一行（必须在帧外调用）。返回是否新整形了。
    fn ensure_shaped(&mut self, line: u64, window: &mut Window) -> bool {
        let Some(entry) = self.lines.peek(&line).cloned() else {
            return false;
        };
        let key = self.shape_key(&entry);
        if self.shapes.contains(&key) {
            return false;
        }
        let shaped = self.shape(&entry, window);
        let cost = shaped
            .layout
            .runs
            .iter()
            .map(|r| r.glyphs.len())
            .sum::<usize>()
            * 40
            + shaped.glyphs.len() * 8
            + entry.display.text.len()
            + 96;
        let xs = self.xs_for(&entry, Some(&shaped));
        self.content_width = self.content_width.max(*xs.last().unwrap_or(&0.0) as f64);
        self.shapes.insert(key, Rc::new(shaped), cost);
        for o in &entry.merged.as_ref().map_or(vec![], |m| m.overlays.clone()) {
            self.ensure_overlay_shaped(&o.text, window);
        }
        true
    }

    pub(crate) fn overlay_key(&self, text: &Arc<str>) -> ShapeKey {
        ShapeKey {
            hash: content_hash(text),
            text: text.clone(),
            runs: Arc::from([(text.len() as u32, 0u8)]),
            font_generation: self.font.generation,
        }
    }

    fn ensure_overlay_shaped(&mut self, text: &Arc<str>, window: &mut Window) {
        let key = self.overlay_key(text);
        if self.shapes.contains(&key) {
            return;
        }
        let runs = [TextRun {
            len: text.len(),
            font: self.font.fonts[0].clone(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let layout = window
            .text_system()
            .layout_line(text, self.font.size, &runs, None);
        let shaped = Shaped {
            glyphs: Vec::new(),
            layout,
        };
        self.shapes
            .insert(key, Rc::new(shaped), text.len() * 48 + 96);
    }

    fn shape(&self, entry: &LineEntry, window: &mut Window) -> Shaped {
        let runs: Vec<TextRun> = entry
            .font_runs
            .iter()
            .filter(|(len, _)| *len > 0)
            .map(|&(len, bits)| TextRun {
                len: len as usize,
                font: self.font.fonts[bits as usize & 3].clone(),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            })
            .collect();
        let layout =
            window
                .text_system()
                .layout_line(&entry.display.text, self.font.size, &runs, None);
        let glyphs: Vec<(u32, f32)> = layout
            .runs
            .iter()
            .flat_map(|r| {
                r.glyphs
                    .iter()
                    .map(|g| (g.index as u32, f32::from(g.position.x)))
            })
            .collect();
        let mut glyphs = glyphs;
        glyphs.sort_by_key(|g| g.0);
        Shaped { layout, glyphs }
    }

    /// 本行字素簇边界的 x 表：有整形结果时由共享的字形几何按本行 stops 现算（按字体代次备忘），
    /// 否则用等宽近似（占位是短暂状态，不备忘）。长度恒等于本行 stops 数。
    pub(crate) fn xs_for(&self, entry: &LineEntry, shaped: Option<&Shaped>) -> Rc<[f32]> {
        let Some(s) = shaped else {
            return entry
                .display
                .monospace_xs(self.font.cell_width, self.font.letter_spacing)
                .into();
        };
        let generation = self.font.generation;
        if let Some((g, xs)) = &*entry.xs.borrow() {
            if *g == generation {
                return xs.clone();
            }
        }
        let xs: Rc<[f32]> = entry
            .display
            .shaped_xs(
                &s.glyphs,
                f32::from(s.layout.width),
                self.font.letter_spacing,
            )
            .into();
        *entry.xs.borrow_mut() = Some((generation, xs.clone()));
        xs
    }

    /// 绘制后调度空闲准备切片（已调度则跳过）。
    pub(crate) fn schedule_prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.prepare_scheduled {
            return;
        }
        self.prepare_scheduled = true;
        cx.spawn_in(window, async move |this, cx| {
            let _ = this.update_in(cx, |v, window, cx| v.prepare_slice(window, cx));
        })
        .detach();
    }

    /// 是否还有可见行或预取行需要准备。
    pub(crate) fn needs_prepare(&self) -> bool {
        self.pending_lines().next().is_some()
    }

    fn prefetch_range(&self) -> Range<u64> {
        let n = self.visible.end.saturating_sub(self.visible.start).max(1);
        let count = self.line_count();
        if self.direction >= 0 {
            self.visible.end..(self.visible.end + n).min(count)
        } else {
            self.visible.start.saturating_sub(n)..self.visible.start
        }
    }

    /// 需要准备的行（文档行）：先可见行，再沿滚动方向预取一屏，再反方向半屏（按屏幕行计）。
    fn pending_lines(&self) -> impl Iterator<Item = u64> + '_ {
        let count = self.line_count();
        let n = self.visible.end.saturating_sub(self.visible.start).max(1);
        let ahead = self.prefetch_range();
        let behind = if self.direction >= 0 {
            self.visible.start.saturating_sub(n / 2)..self.visible.start
        } else {
            self.visible.end..(self.visible.end + n / 2).min(count)
        };
        let known = self.text.as_ref().map_or(0, |t| t.lines().known);
        let map = self.folding.map();
        self.visible
            .clone()
            .chain(ahead)
            .chain(behind)
            .map(move |v| map.to_doc(v))
            .filter(move |l| *l < known)
            .filter(move |l| match self.lines.peek(l) {
                None => true,
                Some(e) => self.decorations.needs(*l) || !self.shapes.contains(&self.shape_key(e)),
            })
    }

    /// 一个空闲切片：在预算内读文本、算装饰、整形，可见行优先，然后预取。
    pub fn prepare_slice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_scheduled = false;
        if self.text.is_none() {
            return;
        }
        let deadline = Instant::now() + self.slice_budget;
        let mut changed = false;
        // 可见行的文本与装饰先全部到位（很快），保证下一帧至少能画占位。
        let visible: Vec<u64> = self.visible.clone().map(|v| self.doc_line(v)).collect();
        for &line in &visible {
            changed |= self.ensure_entry(line);
        }
        let pending: Vec<u64> = self.pending_lines().collect();
        let mut progress = changed;
        let mut out_of_time = false;
        for (i, line) in pending.into_iter().enumerate() {
            // 每片至少推进一行，预算再小也不会原地重排。
            if i > 0 && Instant::now() >= deadline {
                out_of_time = true;
                break;
            }
            let visible = visible.contains(&line);
            let e = self.ensure_entry(line);
            let s = self.ensure_shaped(line, window);
            progress |= e | s;
            if visible {
                changed |= e | s;
            }
        }
        if changed {
            cx.notify();
        }
        // 读不到的行（例如 #8 尚未索引）不会无限重排：没有进展就等下一帧再试。
        if out_of_time && progress {
            self.schedule_prepare(window, cx);
        }
    }

    /// 视口位置变化后记录方向与可见范围（prepaint 调用）。
    pub(crate) fn note_visible(&mut self, range: Range<u64>, top: f64) {
        if top > self.last_top {
            self.direction = 1;
        } else if top < self.last_top {
            self.direction = -1;
        }
        self.last_top = top;
        self.visible = range;
        let first = self.doc_line(self.visible.start);
        let last = self.doc_line(self.visible.end.saturating_sub(1));
        let keep = first.saturating_sub(4096)..last + 4096;
        self.decorations.retain_near(keep);
    }

    pub(crate) fn line_number_ready(&mut self, window: &mut Window) {
        if self.font.digits.is_some() && self.font.scale_factor == window.scale_factor() {
            return;
        }
        if self.font.scale_factor != window.scale_factor() && self.font.scale_factor != 0.0 {
            // DPI 变化：字形度量随之变化，整形结果整体失效（与 #30 协调）。
            self.invalidate_shapes();
        }
        self.font.scale_factor = window.scale_factor();
        let ts = window.text_system();
        let runs = [TextRun {
            len: 10,
            font: self.font.fonts[0].clone(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let layout = ts.layout_line("0123456789", self.font.size, &runs, None);
        let mut glyphs: [Option<GlyphId>; 10] = [None; 10];
        let mut font_id = FontId(0);
        for run in &layout.runs {
            for g in &run.glyphs {
                if g.index < 10 {
                    glyphs[g.index] = Some(g.id);
                    font_id = run.font_id;
                }
            }
        }
        let advance = f32::from(layout.width) / 10.0;
        self.font.cell_width = advance.max(1.0);
        self.font.digits = Some(Rc::new(Digits {
            font_id,
            glyphs,
            advance,
            ascent: layout.ascent.into(),
            descent: layout.descent.into(),
        }));
    }

    // ------------------------------------------------------------------ 语法着色（#20）

    /// 换语法快照（#9/#94 的分析结果，与当前文本同一版本）。`changed` 来自
    /// `SyntaxUpdate::changed_lines`：`None` 让整层失效（首次解析、丢失基线），`Some(ranges)` 只让这些行
    /// 失效。重算前继续用旧样式绘制；首次结果到达前显示纯文本，到达后一次性替换。
    pub fn set_syntax(
        &mut self,
        snapshot: Option<Arc<dyn SyntaxSnapshot>>,
        changed: Option<&[LineRange]>,
        cx: &mut Context<Self>,
    ) {
        let folds_of = |s: &Option<super::highlight::SyntaxPair>| {
            s.as_ref().is_some_and(|(s, _)| !s.folds().is_empty())
        };
        let had_folds = folds_of(&self.syntax.borrow().current);
        let text = self.text.clone();
        let invalid = self.syntax.borrow_mut().update(snapshot, text, changed);
        match &invalid {
            None => self.decorations.invalidate(&self.syntax_layer, None),
            Some(ranges) => {
                for r in ranges {
                    self.decorations
                        .invalidate(&self.syntax_layer, Some(r.clone()));
                }
            }
        }
        self.last_syntax_invalidation = Some(invalid);
        if had_folds || folds_of(&self.syntax.borrow().current) {
            self.recompute_folds(cx);
        }
        self.update_brackets();
        cx.notify();
    }

    /// 当前的语法快照。
    pub fn syntax_snapshot(&self) -> Option<Arc<dyn SyntaxSnapshot>> {
        self.syntax
            .borrow()
            .current
            .as_ref()
            .map(|(s, _)| s.clone())
    }

    /// 上一次 [`CodeView::set_syntax`] 让哪些行失效（`Some(None)` 为整层；测试用）。
    pub fn take_syntax_invalidation(&mut self) -> Option<Option<Vec<Range<u64>>>> {
        self.last_syntax_invalidation.take()
    }

    /// 选项、配色与级别变化后，更新各功能层的参数。
    fn apply_feature_options(&mut self) {
        let p = &self.palette;
        {
            let mut syntax = self.syntax.borrow_mut();
            syntax.max_line_length = self.options.max_tokenization_line_length;
            syntax.enabled = self.effective_level() != SizeClass::Huge;
        }
        self.brackets.borrow_mut().1 = TextStyle {
            foreground: p.bracket_match_foreground,
            background: p.bracket_match_background,
            border: p.bracket_match_border,
            ..Default::default()
        };
        let mut o = self.occurrences.borrow_mut();
        o.word = TextStyle {
            background: p.word_highlight_text_background,
            border: p.word_highlight_text_border,
            ..Default::default()
        };
        o.selection = TextStyle {
            background: p.selection_highlight_background,
            border: p.selection_highlight_border,
            ..Default::default()
        };
    }

    // ------------------------------------------------------------------ 文档级别与只读（#20）

    /// 文档级别（UX_SPEC §3.1，由 #17 的打开文档服务给出）。大文件关闭同名高亮与基于语法的折叠，
    /// 超大文件再关闭语法高亮与全部折叠。
    pub fn set_document_level(&mut self, level: SizeClass, cx: &mut Context<Self>) {
        if self.level == level {
            return;
        }
        self.level = level;
        self.apply_feature_options();
        self.decorations.invalidate(&self.syntax_layer, None);
        self.recompute_folds(cx);
        self.update_highlights(cx);
        cx.notify();
    }

    pub fn document_level(&self) -> SizeClass {
        self.level
    }

    /// `editor.largeFileOptimizations` 为假时不启用“大文件”这一级。
    fn effective_level(&self) -> SizeClass {
        match self.level {
            SizeClass::Large if !self.options.large_file_optimizations => SizeClass::Normal,
            l => l,
        }
    }

    /// 当前因文档级别被关闭的功能（#22 据此组装状态栏悬停提示）。
    pub fn disabled_features(&self) -> Vec<Feature> {
        match self.effective_level() {
            SizeClass::Normal => Vec::new(),
            SizeClass::Large => vec![
                Feature::OccurrencesHighlight,
                Feature::SyntaxFolding,
                Feature::BackgroundHighlighting,
            ],
            SizeClass::Huge => vec![
                Feature::OccurrencesHighlight,
                Feature::SyntaxHighlighting,
                Feature::Folding,
            ],
        }
    }

    /// 只读原因（空为可编辑）。宿主按文档计算：见 [`presented_read_only`] 与“在会话中切换只读模式”。
    pub fn set_read_only(&mut self, reasons: Arc<[ReadOnlyReason]>, cx: &mut Context<Self>) {
        if self.read_only != reasons {
            self.read_only = reasons;
            cx.notify();
        }
    }

    pub fn read_only(&self) -> &[ReadOnlyReason] {
        &self.read_only
    }

    pub fn is_read_only(&self) -> bool {
        !self.read_only.is_empty()
    }

    /// 用户在视图里输入了字符。只读时发出 [`CodeViewEvent::EditBlocked`]（编辑由 R46 实现）。
    pub fn typed(&mut self, cx: &mut Context<Self>) {
        if self.is_read_only() {
            cx.emit(CodeViewEvent::EditBlocked {
                position: self.cursor.selection.active,
                reasons: self.read_only.clone(),
            });
        }
    }

    // ------------------------------------------------------------------ 折叠（#20）

    /// 折叠是否可用（`foldingEnabled` 上下文键）。
    pub fn folding_enabled(&self) -> bool {
        self.options.folding && self.effective_level() != SizeClass::Huge && self.text.is_some()
    }

    /// 当前折叠（写回标签的 `ViewState::folds`）。
    pub fn folds(&self) -> Vec<LineRange> {
        self.folding.collapsed()
    }

    /// 恢复保存的折叠（切换标签时，在恢复滚动位置之前调用）。不发事件。
    pub fn set_folds(&mut self, folds: &[LineRange], cx: &mut Context<Self>) {
        if self.folding_enabled() {
            self.folding.restore(folds);
            self.sync_fold_map();
            cx.notify();
        }
    }

    /// 可折叠区间（语法、标记、缩进、注释、手动）。
    pub fn fold_ranges(&self) -> &[FoldRange] {
        self.folding.ranges()
    }

    /// 区间是否已经算出来（后台计算）。
    pub fn folding_ready(&self) -> bool {
        self.folding.is_ready()
    }

    /// 文档行 ↔ 屏幕行映射（R54/R55/R57 共用）。
    pub fn fold_map(&self) -> &FoldMap {
        self.folding.map()
    }

    /// 折叠变化后：重建屏幕行映射与嵌入区域，保持视口首行稳定。
    fn sync_fold_map(&mut self) {
        let doc_lines = self.doc_line_count();
        self.folding.set_doc_lines(doc_lines);
        let map = self.folding.map().clone();
        if map == self.applied_map {
            self.rebuild_view_zones();
            return;
        }
        let anchor = self.metrics().map(|m| {
            let (first, y) = self.scroll.first_visible(&m, &self.zones);
            (m, self.applied_map.to_doc(first), y)
        });
        self.applied_map = map.clone();
        self.rebuild_view_zones();
        if let Some((mut m, doc, y)) = anchor {
            m.line_count = self.line_count();
            let top = m.line_top(map.to_view(doc), &self.zones) - y / m.line_height;
            self.animation = None;
            self.scroll = ScrollPosition {
                top,
                x: self.scroll.x,
            }
            .clamped(&m, &self.zones);
        }
    }

    fn rebuild_view_zones(&mut self) {
        let map = self.folding.map();
        let mut zones = ViewZones::default();
        for (after, h) in self.doc_zones.iter() {
            if !map.is_hidden(after) {
                zones.set(map.to_view(after), h);
            }
        }
        self.zones = zones;
    }

    fn folds_changed(&mut self, cx: &mut Context<Self>) {
        self.sync_fold_map();
        // 折叠住了光标：光标回到折叠首行（VS Code 行为）。
        let s = self.cursor.selection;
        let map = self.folding.map();
        if map.is_hidden(s.active.line.0) || map.is_hidden(s.anchor.line.0) {
            let line = map.prev_visible(s.active.line.0);
            let p = self.clamp_position(pos(line, s.active.column.0));
            self.cursor.set(Selection::caret(p));
            self.selection_changed(SelectionSource::Api, cx);
        }
        cx.emit(CodeViewEvent::FoldsChanged(self.folding.collapsed().into()));
        cx.notify();
    }

    /// 目标行在折叠里：展开它（跳转、选区接口）。
    fn unfold_to_reveal(&mut self, line: u64, cx: &mut Context<Self>) {
        if self.folding.reveal(line) {
            self.folds_changed(cx);
        }
    }

    /// 上下移动后落进折叠：跳过整段折叠。
    fn skip_folded_lines(&mut self, before: u64) {
        let s = self.cursor.selection;
        let line = s.active.line.0;
        let map = self.folding.map();
        if !map.is_hidden(line) {
            return;
        }
        let target = if line > before {
            map.next_visible(line, self.doc_line_count())
                .unwrap_or_else(|| map.prev_visible(line))
        } else {
            map.prev_visible(line)
        };
        let p = self.clamp_position(pos(target, s.active.column.0));
        let anchor = if s.is_empty() { p } else { s.anchor };
        self.cursor.set(Selection::new(anchor, p));
    }

    /// 折叠首行行尾“⋯”占位标记的 x 范围（相对文本起点）。
    pub(crate) fn placeholder_x(&self, line_end: f32) -> (f32, f32) {
        let cw = self.font.cell_width;
        let start = line_end + cw * 0.6;
        (start, start + cw * 2.6)
    }

    /// 点击折叠首行行尾之后：点在占位标记上，或开启了 `unfoldOnClickAfterEndOfLine`，就展开。
    fn click_unfolds(&mut self, point: Point<Pixels>, line: u64) -> bool {
        if !matches!(self.folding.header(line), Some((_, true))) {
            return false;
        }
        let Some((entry, shaped)) = self.line_geometry(line) else {
            return false;
        };
        let xs = self.xs_for(&entry, shaped.as_deref());
        let end = *xs.last().unwrap_or(&0.0);
        let x = ((point.x - self.layout.text.origin.x).to_f64() + self.scroll.x) as f32;
        let (a, b) = self.placeholder_x(end);
        (a..=b).contains(&x) || (self.options.unfold_on_click_after_end_of_line && x > end)
    }

    /// 后台重算可折叠区间（文本、语言、语法快照、相关设置或级别变化时）。
    fn recompute_folds(&mut self, cx: &mut Context<Self>) {
        self.fold_task += 1;
        if !self.folding_enabled() {
            if !self.folding.ranges().is_empty() || !self.folding.collapsed_keys().is_empty() {
                self.folding.clear();
                self.folds_changed(cx);
            }
            return;
        }
        let Some(text) = self.text.clone() else {
            return;
        };
        let task = self.fold_task;
        let language = self.language.clone();
        let syntax: Arc<[rove_syntax::Fold]> = self
            .syntax
            .borrow()
            .current
            .as_ref()
            .map_or_else(|| Arc::from([]), |(s, _)| s.folds());
        let strategy = self.options.folding_strategy;
        let tab = self.options.tab_size as usize;
        let syntax_allowed = self.effective_level() == SizeClass::Normal;
        let max = self.options.folding_maximum_regions;
        let job = cx.background_executor().spawn(async move {
            let len = text.byte_len();
            let mut source = String::with_capacity(len as usize);
            let span =
                rove_base::Span::new(rove_base::ByteOffset(0), rove_base::ByteOffset(len)).ok()?;
            text.visit_chunks(span, &mut |c| source.push_str(c)).ok()?;
            Some(super::folding::compute_ranges(
                &source,
                language.as_deref(),
                &syntax,
                strategy,
                tab,
                syntax_allowed,
            ))
        });
        cx.spawn(async move |this, cx| {
            let Some(ranges) = job.await else {
                return;
            };
            let _ = this.update(cx, |v, cx| {
                if v.fold_task == task {
                    v.folding.set_computed(ranges, max);
                    v.folds_changed(cx);
                }
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------ 括号匹配与同名高亮（#20）

    /// 选区变化后更新括号匹配与同名高亮。
    pub(crate) fn update_highlights(&mut self, cx: &mut Context<Self>) {
        self.update_brackets();
        self.update_occurrences(cx);
    }

    /// 当前高亮的括号对。
    pub fn bracket_match(&self) -> Option<BracketMatch> {
        self.brackets.borrow().0.clone()
    }

    fn update_brackets(&mut self) {
        let new = self.find_brackets();
        let old = self.brackets.borrow().0.clone();
        if old != new {
            let lines: Vec<u64> = old
                .iter()
                .chain(new.iter())
                .flat_map(|m| m.lines())
                .collect();
            self.brackets.borrow_mut().0 = new;
            for l in lines {
                self.decorations
                    .invalidate(&self.bracket_layer, Some(l..l + 1));
            }
        }
    }

    fn language_config(
        &self,
    ) -> Option<&'static rove_syntax::language_config::CompiledLanguageConfiguration> {
        let language = self.language.as_deref()?;
        rove_syntax::language_config::BuiltinLanguages::shared()
            .0
            .compiled(language)
    }

    fn find_brackets(&self) -> Option<BracketMatch> {
        let text = self.text.as_ref()?;
        let config = self.language_config()?;
        let a = self.cursor.selection.active;
        let lines = TextLines(&**text);
        let read = |l: u64| -> Option<Arc<str>> {
            self.lines
                .peek(&l)
                .map(|e| e.source.clone())
                .or_else(|| lines.line_text(l).map(Arc::from))
        };
        let syntax = self.syntax.borrow();
        let skip = |l: u64, c: u32| syntax.in_string_or_comment(l, c);
        brackets::find(
            &config.data().brackets,
            a.line.0,
            a.column.0 as u32,
            self.options.match_brackets,
            &read,
            &skip,
        )
    }

    /// `hasWordHighlights` 上下文键的取值。
    pub fn has_word_highlights(&self) -> bool {
        self.occurrences.borrow().has_word_highlights()
    }

    /// 当前同名高亮的文本（单词或选中文本）。
    pub fn occurrence_text(&self) -> Option<Arc<str>> {
        match &self.occurrences.borrow().target {
            Some(Target::Word { text, .. }) | Some(Target::Selection { text, .. }) => {
                Some(text.clone())
            }
            None => None,
        }
    }

    fn word_rule(&self) -> WordRule {
        self.language_config()
            .map(WordRule::Language)
            .unwrap_or_else(|| WordRule::Separators(self.options.word_separators.clone()))
    }

    fn occurrence_target(&self) -> Option<Target> {
        let text = self.text.as_ref()?;
        let s = self.cursor.selection;
        let lines = TextLines(&**text);
        if !s.is_empty() {
            // 跨行选区的相似匹配需要跨行查找，本视图按行匹配，暂不支持（selectionHighlightMultiline）。
            if !self.options.selection_highlight || s.start().line != s.end().line {
                return None;
            }
            let line = lines.line_text(s.start().line.0)?;
            let (a, b) = (s.start().column.0 as usize, s.end().column.0 as usize);
            let t = line.get(a..b)?;
            if t.trim().is_empty()
                || t.chars().count() > self.options.selection_highlight_max_length
            {
                return None;
            }
            return Some(Target::Selection {
                text: t.into(),
                case_sensitive: self.options.selected_text_case_sensitive,
                exclude: (s.start().line.0, a),
            });
        }
        if !self.options.occurrences_highlight || self.effective_level() != SizeClass::Normal {
            return None;
        }
        let line = lines.line_text(s.active.line.0)?;
        let rule = self.word_rule();
        let (a, b) = rule.word_at(&line, s.active.column.0 as usize)?;
        Some(Target::Word {
            text: line[a..b].into(),
            rule,
        })
    }

    fn update_occurrences(&mut self, cx: &mut Context<Self>) {
        let target = self.occurrence_target();
        let same = match (&self.occurrences.borrow().target, &target) {
            (None, None) => true,
            (Some(Target::Word { text: a, .. }), Some(Target::Word { text: b, .. })) => a == b,
            (
                Some(Target::Selection {
                    text: a,
                    exclude: x,
                    case_sensitive: c,
                }),
                Some(Target::Selection {
                    text: b,
                    exclude: y,
                    case_sensitive: d,
                }),
            ) => a == b && x == y && c == d,
            _ => false,
        };
        if same {
            return;
        }
        self.occurrence_epoch.set(self.occurrence_epoch.get() + 1);
        let delay = self.options.occurrences_highlight_delay_ms;
        let deferred = delay > 0 && matches!(target, Some(Target::Word { .. }));
        self.occurrences.borrow_mut().target = if deferred { None } else { target.clone() };
        self.decorations.invalidate(&self.occurrence_layer, None);
        if deferred {
            let epoch = self.occurrence_epoch.clone();
            let mine = epoch.get();
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(delay))
                    .await;
                if epoch.get() != mine {
                    return;
                }
                let _ = this.update(cx, |v, cx| {
                    v.occurrences.borrow_mut().target = target;
                    v.decorations.invalidate(&v.occurrence_layer, None);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    // ------------------------------------------------------------------ 统计（基准、测试）

    /// 开始/停止记录每帧统计。
    pub fn record_frames(&mut self, on: bool) {
        self.recording = on.then(Vec::new);
    }

    pub fn take_frame_samples(&mut self) -> Vec<FrameStats> {
        self.recording
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    pub fn last_frame(&self) -> FrameStats {
        self.last_frame
    }

    pub fn shape_cache_stats(&self) -> CacheStats {
        self.shapes.stats()
    }

    pub fn line_cache_stats(&self) -> CacheStats {
        self.lines.stats()
    }

    /// 调整缓存容量（字节估算，计入内存预算）。
    pub fn set_cache_capacity(&mut self, shape_bytes: usize, line_bytes: usize) {
        self.shapes.set_capacity(shape_bytes);
        self.lines.set_capacity(line_bytes);
    }

    /// 调整空闲准备切片的时间预算（基准用）。
    pub fn set_prepare_budget(&mut self, budget: Duration) {
        self.slice_budget = budget;
    }
}

/// 虚拟行坐标 → 行号（考虑上内边距与嵌入区域）。
pub(crate) fn line_at_coord(coord: f64, m: &Metrics, zones: &ViewZones) -> u64 {
    let last = m.line_count.saturating_sub(1);
    let mut l = ((coord - (m.padding_top + zones.total()) / m.line_height)
        .floor()
        .max(0.0) as u64)
        .min(last);
    while l < last && m.line_top(l + 1, zones) <= coord {
        l += 1;
    }
    l
}
