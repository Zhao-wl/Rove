//! 代码视图消费的 `editor.*` 设置项：登记（VS Code 默认值）与强类型读取。
//!
//! 只登记键与默认值，不读写设置文件（#13）。读取时对非法值回退到默认值并按 VS Code 的范围钳制。

use rove_base::{Result, Value};
use rove_config::{Scope, Setting, SettingRegistry, Settings};
use std::sync::Arc;

/// 本视图登记的全部设置项及 VS Code 默认值（`docs/compat/SETTINGS.md`）。
pub const SETTINGS: &[(&str, DefaultValue)] = &[
    (
        "editor.fontFamily",
        DefaultValue::Str("Consolas, 'Courier New', monospace"),
    ),
    ("editor.fontSize", DefaultValue::Num(14.0)),
    ("editor.fontWeight", DefaultValue::Str("normal")),
    ("editor.fontLigatures", DefaultValue::Bool(false)),
    ("editor.fontVariations", DefaultValue::Bool(false)),
    ("editor.lineHeight", DefaultValue::Num(0.0)),
    ("editor.letterSpacing", DefaultValue::Num(0.0)),
    ("editor.fullwidthCharacterWidth", DefaultValue::Str("font")),
    ("editor.wordBreak", DefaultValue::Str("normal")),
    ("editor.tabSize", DefaultValue::Num(4.0)),
    ("editor.stopRenderingLineAfter", DefaultValue::Num(10000.0)),
    ("editor.smoothScrolling", DefaultValue::Bool(false)),
    ("editor.mouseWheelScrollSensitivity", DefaultValue::Num(1.0)),
    ("editor.fastScrollSensitivity", DefaultValue::Num(5.0)),
    ("editor.scrollPredominantAxis", DefaultValue::Bool(true)),
    ("editor.inertialScroll", DefaultValue::Bool(false)),
    ("editor.scrollBeyondLastLine", DefaultValue::Bool(true)),
    ("editor.scrollBeyondLastColumn", DefaultValue::Num(4.0)),
    ("editor.cursorSurroundingLines", DefaultValue::Num(0.0)),
    (
        "editor.cursorSurroundingLinesStyle",
        DefaultValue::Str("default"),
    ),
    ("editor.scrollbar.vertical", DefaultValue::Str("auto")),
    ("editor.scrollbar.horizontal", DefaultValue::Str("auto")),
    (
        "editor.scrollbar.verticalScrollbarSize",
        DefaultValue::Num(14.0),
    ),
    (
        "editor.scrollbar.horizontalScrollbarSize",
        DefaultValue::Num(12.0),
    ),
    ("editor.scrollbar.scrollByPage", DefaultValue::Bool(false)),
    (
        "editor.scrollbar.ignoreHorizontalScrollbarInContentHeight",
        DefaultValue::Bool(false),
    ),
    ("editor.lineNumbers", DefaultValue::Str("on")),
    ("editor.renderFinalNewline", DefaultValue::Str("on")),
    ("editor.glyphMargin", DefaultValue::Bool(true)),
    ("editor.padding.top", DefaultValue::Num(0.0)),
    ("editor.padding.bottom", DefaultValue::Num(0.0)),
    ("editor.renderLineHighlight", DefaultValue::Str("line")),
    (
        "editor.renderLineHighlightOnlyWhenFocus",
        DefaultValue::Bool(false),
    ),
    (
        "editor.wordSeparators",
        DefaultValue::Str("`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?"),
    ),
    ("editor.cursorStyle", DefaultValue::Str("line")),
    ("editor.cursorWidth", DefaultValue::Num(0.0)),
    ("editor.cursorHeight", DefaultValue::Num(0.0)),
    ("editor.cursorBlinking", DefaultValue::Str("blink")),
    (
        "editor.cursorSmoothCaretAnimation",
        DefaultValue::Str("off"),
    ),
    ("editor.roundedSelection", DefaultValue::Bool(true)),
    ("editor.emptySelectionClipboard", DefaultValue::Bool(true)),
    ("editor.mouseWheelZoom", DefaultValue::Bool(false)),
    // #20：高亮、括号、同名高亮、折叠、参考线、空白字符。
    (
        "editor.maxTokenizationLineLength",
        DefaultValue::Num(20000.0),
    ),
    ("editor.matchBrackets", DefaultValue::Str("always")),
    (
        "editor.occurrencesHighlight",
        DefaultValue::Str("singleFile"),
    ),
    ("editor.occurrencesHighlightDelay", DefaultValue::Num(0.0)),
    ("editor.selectionHighlight", DefaultValue::Bool(true)),
    (
        "editor.selectionHighlightMaxLength",
        DefaultValue::Num(200.0),
    ),
    (
        "editor.selectionHighlightMultiline",
        DefaultValue::Bool(false),
    ),
    (
        "editor.selectedTextMatchMode",
        DefaultValue::Str("findOptions"),
    ),
    ("editor.folding", DefaultValue::Bool(true)),
    ("editor.foldingStrategy", DefaultValue::Str("auto")),
    ("editor.foldingMaximumRegions", DefaultValue::Num(5000.0)),
    ("editor.showFoldingControls", DefaultValue::Str("mouseover")),
    ("editor.foldingHighlight", DefaultValue::Bool(true)),
    (
        "editor.unfoldOnClickAfterEndOfLine",
        DefaultValue::Bool(false),
    ),
    ("editor.guides.indentation", DefaultValue::Bool(true)),
    ("editor.renderWhitespace", DefaultValue::Str("selection")),
    ("editor.largeFileOptimizations", DefaultValue::Bool(true)),
    // #47：编辑交互（制表位、改写模式、自动空白）。
    ("editor.insertSpaces", DefaultValue::Bool(true)),
    ("editor.useTabStops", DefaultValue::Bool(true)),
    ("editor.stickyTabStops", DefaultValue::Bool(false)),
    ("editor.trimAutoWhitespace", DefaultValue::Bool(true)),
    ("editor.overtypeCursorStyle", DefaultValue::Str("block")),
    ("editor.overtypeOnPaste", DefaultValue::Bool(true)),
    // #52：多光标与列选择。
    ("editor.multiCursorModifier", DefaultValue::Str("alt")),
    (
        "editor.multiCursorMergeOverlapping",
        DefaultValue::Bool(true),
    ),
    ("editor.multiCursorLimit", DefaultValue::Num(10000.0)),
    ("editor.multiCursorPaste", DefaultValue::Str("spread")),
    ("editor.columnSelection", DefaultValue::Bool(false)),
    // #53：注释切换、自动缩进、自动闭合与自动包围。
    ("editor.indentSize", DefaultValue::Str("tabSize")),
    ("editor.comments.insertSpace", DefaultValue::Bool(true)),
    ("editor.comments.ignoreEmptyLines", DefaultValue::Bool(true)),
    ("editor.autoIndent", DefaultValue::Str("full")),
    ("editor.autoIndentOnPaste", DefaultValue::Bool(false)),
    (
        "editor.autoIndentOnPasteWithinString",
        DefaultValue::Bool(true),
    ),
    (
        "editor.autoClosingBrackets",
        DefaultValue::Str("languageDefined"),
    ),
    (
        "editor.autoClosingQuotes",
        DefaultValue::Str("languageDefined"),
    ),
    (
        "editor.autoClosingComments",
        DefaultValue::Str("languageDefined"),
    ),
    ("editor.autoClosingDelete", DefaultValue::Str("auto")),
    ("editor.autoClosingOvertype", DefaultValue::Str("auto")),
    ("editor.autoSurround", DefaultValue::Str("languageDefined")),
];

/// 设置项默认值（编译期表）。
#[derive(Clone, Copy, Debug)]
pub enum DefaultValue {
    Bool(bool),
    Num(f64),
    Str(&'static str),
}

impl DefaultValue {
    fn value(self) -> Value {
        match self {
            DefaultValue::Bool(b) => Value::Bool(b),
            DefaultValue::Num(n) => Value::Number(n),
            DefaultValue::Str(s) => Value::String(s.into()),
        }
    }
}

/// 把上表登记进设置表。已被其他模块登记的键（例如 `editor.tabSize`）跳过而不是报 Duplicate：
/// 同一个 VS Code 设置项可能有多个消费方，默认值都取 VS Code 的。
pub fn register(registry: &mut SettingRegistry) -> Result<()> {
    for (key, default) in SETTINGS {
        if registry.get(key).is_some() {
            continue;
        }
        registry.register(
            *key,
            Setting {
                default: default.value(),
                rove_default: None,
                language_overridable: true,
            },
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineNumbers {
    On,
    Off,
    Relative,
    Interval,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinalNewline {
    On,
    Off,
    Dimmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineHighlight {
    None,
    Gutter,
    Line,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Line,
    Block,
    Underline,
    LineThin,
    BlockOutline,
    UnderlineThin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorBlinking {
    Blink,
    Smooth,
    Phase,
    Expand,
    Solid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarVisibility {
    Auto,
    Visible,
    Hidden,
}

/// 字体连字：关闭、开启（`liga`+`calt`），或 CSS `font-feature-settings` 字符串解析出的特性表。
#[derive(Clone, Debug, PartialEq)]
pub enum Ligatures {
    Off,
    On,
    Features(Vec<(String, u32)>),
}

/// `editor.showFoldingControls`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldingControls {
    Always,
    Never,
    Mouseover,
}

/// 强类型的编辑器选项快照。设置变更时整体重读；字体相关字段变化会让整形缓存整体失效。
#[derive(Clone, Debug, PartialEq)]
pub struct EditorOptions {
    pub font_family: Arc<str>,
    pub font_fallbacks: Vec<String>,
    pub font_size: f64,
    pub font_weight: f32,
    pub ligatures: Ligatures,
    pub font_variations: bool,
    /// 0 表示按字号计算（VS Code：`round(1.35 × fontSize)`，Windows）。
    pub line_height_setting: f64,
    pub letter_spacing: f64,
    pub fullwidth_font_width: bool,
    pub word_break_keep_all: bool,
    pub tab_size: u32,
    /// `None` 表示不限（设置为 -1）。
    pub stop_rendering_line_after: Option<usize>,
    pub smooth_scrolling: bool,
    pub wheel_sensitivity: f64,
    pub fast_scroll_sensitivity: f64,
    pub scroll_predominant_axis: bool,
    pub inertial_scroll: bool,
    pub scroll_beyond_last_line: bool,
    pub scroll_beyond_last_column: f64,
    pub cursor_surrounding_lines: u64,
    pub cursor_surrounding_all: bool,
    pub vertical_scrollbar: ScrollbarVisibility,
    pub horizontal_scrollbar: ScrollbarVisibility,
    pub vertical_scrollbar_size: f64,
    pub horizontal_scrollbar_size: f64,
    pub scroll_by_page: bool,
    pub ignore_horizontal_scrollbar_in_content_height: bool,
    pub line_numbers: LineNumbers,
    pub render_final_newline: FinalNewline,
    pub glyph_margin: bool,
    pub padding_top: f64,
    pub padding_bottom: f64,
    pub render_line_highlight: LineHighlight,
    pub line_highlight_only_when_focus: bool,
    pub word_separators: Arc<str>,
    pub cursor_style: CursorStyle,
    pub cursor_width: f64,
    pub cursor_height: f64,
    pub cursor_blinking: CursorBlinking,
    pub smooth_caret: bool,
    pub rounded_selection: bool,
    pub empty_selection_clipboard: bool,
    pub mouse_wheel_zoom: bool,
    /// `editor.maxTokenizationLineLength`（`None` 为不限）。
    pub max_tokenization_line_length: Option<usize>,
    pub match_brackets: super::brackets::MatchBrackets,
    /// `editor.occurrencesHighlight` 不为 `off`。
    pub occurrences_highlight: bool,
    pub occurrences_highlight_delay_ms: u64,
    pub selection_highlight: bool,
    pub selection_highlight_max_length: usize,
    pub selection_highlight_multiline: bool,
    /// `editor.selectedTextMatchMode`：`caseSensitive` 为真；`findOptions` 按查找条的默认（不区分大小写）。
    pub selected_text_case_sensitive: bool,
    pub folding: bool,
    pub folding_strategy: super::folding::FoldingStrategy,
    pub folding_maximum_regions: usize,
    pub show_folding_controls: FoldingControls,
    pub folding_highlight: bool,
    pub unfold_on_click_after_end_of_line: bool,
    pub guides_indentation: bool,
    pub render_whitespace: super::guides::RenderWhitespace,
    pub large_file_optimizations: bool,
    // #47
    pub insert_spaces: bool,
    pub use_tab_stops: bool,
    pub sticky_tab_stops: bool,
    pub trim_auto_whitespace: bool,
    /// 改写模式下的插入符样式（`editor.overtypeCursorStyle`）。
    pub overtype_cursor_style: CursorStyle,
    pub overtype_on_paste: bool,
    // #52
    /// `editor.multiCursorModifier`：加光标与 #40 的跳转手势共用的修饰键来源。
    pub multi_cursor_modifier: super::multicursor::MultiCursorModifier,
    pub multi_cursor_merge_overlapping: bool,
    pub multi_cursor_limit: usize,
    /// `editor.columnSelection`：`shift+方向键` 直接进入列选择。
    pub column_selection: bool,
    // #53
    /// `editor.indentSize`：`"tabSize"` 时等于 `tab_size`。
    pub indent_size: u32,
    pub auto_indent: super::autoedit::indent::AutoIndent,
    pub auto_indent_on_paste: bool,
    pub auto_indent_on_paste_within_string: bool,
    pub comments: super::autoedit::comments::CommentOptions,
    pub auto_close: super::autoedit::autoclose::AutoCloseOptions,
}

impl EditorOptions {
    /// `editor.insertSpaces` / `tabSize` / `indentSize` 的换算句柄（#53）。
    pub fn indent_converter(&self) -> super::autoedit::indent::IndentConverter {
        super::autoedit::indent::IndentConverter {
            insert_spaces: self.insert_spaces,
            tab_size: self.tab_size.max(1) as usize,
            indent_size: self.indent_size.max(1) as usize,
        }
    }
}

impl Default for EditorOptions {
    fn default() -> Self {
        let mut registry = SettingRegistry::default();
        register(&mut registry).expect("内置设置表");
        Self::read(&rove_config::DefaultSettings(registry), None)
    }
}

struct Reader<'a> {
    settings: &'a dyn Settings,
    scope: Scope,
}

impl Reader<'_> {
    fn raw(&self, key: &str) -> Value {
        let fallback = SETTINGS
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, d)| d.value())
            .unwrap_or(Value::Null);
        self.settings.read(key, &self.scope).unwrap_or(fallback)
    }
    fn num(&self, key: &str, lo: f64, hi: f64) -> f64 {
        let fallback = match SETTINGS.iter().find(|(k, _)| *k == key) {
            Some((_, DefaultValue::Num(n))) => *n,
            _ => 0.0,
        };
        match self.raw(key) {
            Value::Number(n) if n.is_finite() => n.clamp(lo, hi),
            Value::String(s) => s
                .trim()
                .parse::<f64>()
                .map_or(fallback, |n| n.clamp(lo, hi)),
            _ => fallback,
        }
    }
    fn bool(&self, key: &str) -> bool {
        match self.raw(key) {
            Value::Bool(b) => b,
            _ => matches!(
                SETTINGS.iter().find(|(k, _)| *k == key),
                Some((_, DefaultValue::Bool(true)))
            ),
        }
    }
    fn str(&self, key: &str) -> String {
        match self.raw(key) {
            Value::String(s) => s.to_string(),
            _ => match SETTINGS.iter().find(|(k, _)| *k == key) {
                Some((_, DefaultValue::Str(s))) => s.to_string(),
                _ => String::new(),
            },
        }
    }
}

/// 解析 `editor.fontFamily`（逗号分隔、可带引号）。泛型族名（monospace 等）不交给 DirectWrite。
pub fn parse_font_family(s: &str) -> (String, Vec<String>) {
    let mut names: Vec<String> = s
        .split(',')
        .map(|p| {
            p.trim()
                .trim_matches(|c| c == '\'' || c == '"')
                .trim()
                .to_string()
        })
        .filter(|p| !p.is_empty())
        .filter(|p| {
            !matches!(
                p.as_str(),
                "monospace" | "serif" | "sans-serif" | "system-ui"
            )
        })
        .collect();
    if names.is_empty() {
        names.push("Consolas".into());
    }
    let first = names.remove(0);
    (first, names)
}

/// 解析 CSS `font-feature-settings`（`"'ss01', 'zero' 1, 'liga' off"`）。
pub fn parse_font_features(s: &str) -> Vec<(String, u32)> {
    s.split(',')
        .filter_map(|item| {
            let mut parts = item.split_whitespace();
            let tag = parts.next()?.trim_matches(|c| c == '\'' || c == '"');
            if tag.len() != 4 {
                return None;
            }
            let value = match parts.next() {
                None | Some("on") => 1,
                Some("off") => 0,
                Some(n) => n.parse().ok()?,
            };
            Some((tag.to_string(), value))
        })
        .collect()
}

fn cursor_style(s: &str) -> CursorStyle {
    match s {
        "block" => CursorStyle::Block,
        "underline" => CursorStyle::Underline,
        "line-thin" => CursorStyle::LineThin,
        "block-outline" => CursorStyle::BlockOutline,
        "underline-thin" => CursorStyle::UnderlineThin,
        _ => CursorStyle::Line,
    }
}

fn parse_weight(s: &str) -> f32 {
    match s.trim() {
        "normal" => 400.0,
        "bold" => 700.0,
        n => n.parse::<f32>().map_or(400.0, |w| w.clamp(1.0, 1000.0)),
    }
}

impl EditorOptions {
    /// 读取当前快照。`language` 为语言 ID（语言级覆盖，如 `"[csharp]"`）。O(键数 × log 键数)，不做 I/O。
    pub fn read(settings: &dyn Settings, language: Option<&str>) -> Self {
        let r = Reader {
            settings,
            scope: Scope {
                language: language.map(Arc::from),
            },
        };
        let (font_family, font_fallbacks) = parse_font_family(&r.str("editor.fontFamily"));
        let ligatures = match r.raw("editor.fontLigatures") {
            Value::Bool(true) => Ligatures::On,
            Value::String(s) if s.trim() == "true" => Ligatures::On,
            Value::String(s) if s.trim() == "false" || s.trim().is_empty() => Ligatures::Off,
            Value::String(s) => Ligatures::Features(parse_font_features(&s)),
            _ => Ligatures::Off,
        };
        let font_variations = match r.raw("editor.fontVariations") {
            Value::Bool(b) => b,
            Value::String(s) => !s.trim().is_empty() && s.trim() != "false",
            _ => false,
        };
        let weight = match r.raw("editor.fontWeight") {
            Value::Number(n) => (n as f32).clamp(1.0, 1000.0),
            Value::String(s) => parse_weight(&s),
            _ => 400.0,
        };
        let visibility = |k: &str| match r.str(k).as_str() {
            "visible" => ScrollbarVisibility::Visible,
            "hidden" => ScrollbarVisibility::Hidden,
            _ => ScrollbarVisibility::Auto,
        };
        let stop = r.num("editor.stopRenderingLineAfter", -1.0, 1e9);
        Self {
            font_family: font_family.into(),
            font_fallbacks,
            font_size: r.num("editor.fontSize", 6.0, 100.0),
            font_weight: weight,
            ligatures,
            font_variations,
            line_height_setting: r.num("editor.lineHeight", 0.0, 150.0),
            letter_spacing: r.num("editor.letterSpacing", -5.0, 20.0),
            fullwidth_font_width: r.str("editor.fullwidthCharacterWidth") != "fixed",
            word_break_keep_all: r.str("editor.wordBreak") == "keepAll",
            tab_size: r.num("editor.tabSize", 1.0, 100.0) as u32,
            stop_rendering_line_after: if stop < 0.0 {
                None
            } else {
                Some(stop as usize)
            },
            smooth_scrolling: r.bool("editor.smoothScrolling"),
            wheel_sensitivity: r.num("editor.mouseWheelScrollSensitivity", 0.0, 1e3),
            fast_scroll_sensitivity: r.num("editor.fastScrollSensitivity", 0.0, 1e3),
            scroll_predominant_axis: r.bool("editor.scrollPredominantAxis"),
            inertial_scroll: r.bool("editor.inertialScroll"),
            scroll_beyond_last_line: r.bool("editor.scrollBeyondLastLine"),
            scroll_beyond_last_column: r.num("editor.scrollBeyondLastColumn", 0.0, 1e6),
            cursor_surrounding_lines: r.num("editor.cursorSurroundingLines", 0.0, 1e6) as u64,
            cursor_surrounding_all: r.str("editor.cursorSurroundingLinesStyle") == "all",
            vertical_scrollbar: visibility("editor.scrollbar.vertical"),
            horizontal_scrollbar: visibility("editor.scrollbar.horizontal"),
            vertical_scrollbar_size: r.num("editor.scrollbar.verticalScrollbarSize", 0.0, 1000.0),
            horizontal_scrollbar_size: r.num(
                "editor.scrollbar.horizontalScrollbarSize",
                0.0,
                1000.0,
            ),
            scroll_by_page: r.bool("editor.scrollbar.scrollByPage"),
            ignore_horizontal_scrollbar_in_content_height: r
                .bool("editor.scrollbar.ignoreHorizontalScrollbarInContentHeight"),
            line_numbers: match r.str("editor.lineNumbers").as_str() {
                "off" => LineNumbers::Off,
                "relative" => LineNumbers::Relative,
                "interval" => LineNumbers::Interval,
                _ => LineNumbers::On,
            },
            render_final_newline: match r.str("editor.renderFinalNewline").as_str() {
                "off" => FinalNewline::Off,
                "dimmed" => FinalNewline::Dimmed,
                _ => FinalNewline::On,
            },
            glyph_margin: r.bool("editor.glyphMargin"),
            padding_top: r.num("editor.padding.top", 0.0, 1000.0),
            padding_bottom: r.num("editor.padding.bottom", 0.0, 1000.0),
            render_line_highlight: match r.str("editor.renderLineHighlight").as_str() {
                "none" => LineHighlight::None,
                "gutter" => LineHighlight::Gutter,
                "all" => LineHighlight::All,
                _ => LineHighlight::Line,
            },
            line_highlight_only_when_focus: r.bool("editor.renderLineHighlightOnlyWhenFocus"),
            word_separators: r.str("editor.wordSeparators").into(),
            cursor_style: cursor_style(&r.str("editor.cursorStyle")),
            insert_spaces: r.bool("editor.insertSpaces"),
            use_tab_stops: r.bool("editor.useTabStops"),
            sticky_tab_stops: r.bool("editor.stickyTabStops"),
            trim_auto_whitespace: r.bool("editor.trimAutoWhitespace"),
            overtype_cursor_style: cursor_style(&r.str("editor.overtypeCursorStyle")),
            overtype_on_paste: r.bool("editor.overtypeOnPaste"),
            multi_cursor_modifier: super::multicursor::MultiCursorModifier::parse(
                &r.str("editor.multiCursorModifier"),
            ),
            multi_cursor_merge_overlapping: r.bool("editor.multiCursorMergeOverlapping"),
            multi_cursor_limit: r.num("editor.multiCursorLimit", 1.0, 1e6) as usize,
            column_selection: r.bool("editor.columnSelection"),
            indent_size: match r.raw("editor.indentSize") {
                Value::Number(n) if n >= 1.0 => n.min(100.0) as u32,
                _ => r.num("editor.tabSize", 1.0, 100.0) as u32,
            },
            auto_indent: super::autoedit::indent::AutoIndent::parse(&r.str("editor.autoIndent")),
            auto_indent_on_paste: r.bool("editor.autoIndentOnPaste"),
            auto_indent_on_paste_within_string: r.bool("editor.autoIndentOnPasteWithinString"),
            comments: super::autoedit::comments::CommentOptions {
                insert_space: r.bool("editor.comments.insertSpace"),
                ignore_empty_lines: r.bool("editor.comments.ignoreEmptyLines"),
                tab_size: r.num("editor.tabSize", 1.0, 100.0) as usize,
            },
            auto_close: super::autoedit::autoclose::AutoCloseOptions {
                brackets: super::autoedit::autoclose::AutoClosing::parse(
                    &r.str("editor.autoClosingBrackets"),
                ),
                quotes: super::autoedit::autoclose::AutoClosing::parse(
                    &r.str("editor.autoClosingQuotes"),
                ),
                comments: super::autoedit::autoclose::AutoClosing::parse(
                    &r.str("editor.autoClosingComments"),
                ),
                overtype: super::autoedit::autoclose::AutoClosingEdit::parse(
                    &r.str("editor.autoClosingOvertype"),
                ),
                delete: super::autoedit::autoclose::AutoClosingEdit::parse(
                    &r.str("editor.autoClosingDelete"),
                ),
                surround: super::autoedit::autoclose::AutoSurround::parse(
                    &r.str("editor.autoSurround"),
                ),
            },
            cursor_width: r.num("editor.cursorWidth", 0.0, 100.0),
            cursor_height: r.num("editor.cursorHeight", 0.0, 1000.0),
            cursor_blinking: match r.str("editor.cursorBlinking").as_str() {
                "smooth" => CursorBlinking::Smooth,
                "phase" => CursorBlinking::Phase,
                "expand" => CursorBlinking::Expand,
                "solid" => CursorBlinking::Solid,
                _ => CursorBlinking::Blink,
            },
            smooth_caret: r.str("editor.cursorSmoothCaretAnimation") != "off",
            rounded_selection: r.bool("editor.roundedSelection"),
            empty_selection_clipboard: r.bool("editor.emptySelectionClipboard"),
            mouse_wheel_zoom: r.bool("editor.mouseWheelZoom"),
            max_tokenization_line_length: {
                let n = r.num("editor.maxTokenizationLineLength", -1.0, 1e9);
                (n >= 0.0).then_some(n as usize)
            },
            match_brackets: match r.str("editor.matchBrackets").as_str() {
                "never" => super::brackets::MatchBrackets::Never,
                "near" => super::brackets::MatchBrackets::Near,
                _ => super::brackets::MatchBrackets::Always,
            },
            occurrences_highlight: match r.raw("editor.occurrencesHighlight") {
                Value::Bool(b) => b,
                Value::String(s) => &*s != "off",
                _ => true,
            },
            occurrences_highlight_delay_ms: r.num("editor.occurrencesHighlightDelay", 0.0, 2000.0)
                as u64,
            selection_highlight: r.bool("editor.selectionHighlight"),
            selection_highlight_max_length: r.num("editor.selectionHighlightMaxLength", 0.0, 1e9)
                as usize,
            selection_highlight_multiline: r.bool("editor.selectionHighlightMultiline"),
            selected_text_case_sensitive: r.str("editor.selectedTextMatchMode") == "caseSensitive",
            folding: r.bool("editor.folding"),
            folding_strategy: if r.str("editor.foldingStrategy") == "indentation" {
                super::folding::FoldingStrategy::Indentation
            } else {
                super::folding::FoldingStrategy::Auto
            },
            folding_maximum_regions: r.num("editor.foldingMaximumRegions", 10.0, 65000.0) as usize,
            show_folding_controls: match r.str("editor.showFoldingControls").as_str() {
                "always" => FoldingControls::Always,
                "never" => FoldingControls::Never,
                _ => FoldingControls::Mouseover,
            },
            folding_highlight: r.bool("editor.foldingHighlight"),
            unfold_on_click_after_end_of_line: r.bool("editor.unfoldOnClickAfterEndOfLine"),
            guides_indentation: r.bool("editor.guides.indentation"),
            render_whitespace: match r.str("editor.renderWhitespace").as_str() {
                "none" => super::guides::RenderWhitespace::None,
                "boundary" => super::guides::RenderWhitespace::Boundary,
                "trailing" => super::guides::RenderWhitespace::Trailing,
                "all" => super::guides::RenderWhitespace::All,
                _ => super::guides::RenderWhitespace::Selection,
            },
            large_file_optimizations: r.bool("editor.largeFileOptimizations"),
        }
    }

    /// 缩放后的字号（VS Code `EditorZoom`：每级 10%）。
    pub fn zoomed_font_size(&self, zoom_level: i32) -> f64 {
        (self.font_size * zoom_multiplier(zoom_level)).max(1.0)
    }

    /// 缩放后的行高（像素，取整）。VS Code：0 → `round(1.35 × fontSize)`；< 8 视为字号倍数；最小 8。
    pub fn zoomed_line_height(&self, zoom_level: i32) -> f64 {
        let font = self.font_size;
        let lh = if self.line_height_setting == 0.0 {
            (1.35 * font).round()
        } else if self.line_height_setting < 8.0 {
            (self.line_height_setting * font).round()
        } else {
            self.line_height_setting
        };
        (lh.max(8.0) * zoom_multiplier(zoom_level)).round()
    }

    /// 插入符宽度（`line` 样式下 0 表示 2 px，`line-thin` 1 px）。
    pub fn caret_width(&self) -> f64 {
        match self.cursor_style {
            CursorStyle::LineThin => 1.0,
            CursorStyle::Line if self.cursor_width > 0.0 => self.cursor_width,
            CursorStyle::Line => 2.0,
            _ => 0.0,
        }
    }
}

/// VS Code 字体缩放级别范围。
pub const MIN_ZOOM: i32 = -5;
pub const MAX_ZOOM: i32 = 20;

pub fn zoom_multiplier(level: i32) -> f64 {
    1.0 + level.clamp(MIN_ZOOM, MAX_ZOOM) as f64 * 0.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_config::DefaultSettings;

    /// 只覆盖若干键的测试设置。
    pub struct Overrides(pub DefaultSettings, pub Vec<(&'static str, Value)>);
    impl Settings for Overrides {
        fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
            match self.1.iter().find(|(k, _)| *k == key) {
                Some((_, v)) => Ok(v.clone()),
                None => self.0.read(key, scope),
            }
        }
        fn write(
            &self,
            k: Arc<str>,
            s: Scope,
            v: Value,
            c: rove_base::Cancellation,
        ) -> rove_base::Task<()> {
            self.0.write(k, s, v, c)
        }
        fn changes(&self) -> &dyn rove_base::Events<rove_config::SettingsChange> {
            self.0.changes()
        }
    }

    fn with(overrides: Vec<(&'static str, Value)>) -> EditorOptions {
        let mut r = SettingRegistry::default();
        register(&mut r).unwrap();
        EditorOptions::read(&Overrides(DefaultSettings(r), overrides), None)
    }

    #[test]
    fn registration_is_idempotent_and_uses_vscode_defaults() {
        let mut r = SettingRegistry::default();
        r.register(
            "editor.tabSize",
            Setting {
                default: Value::Number(4.0),
                rove_default: None,
                language_overridable: true,
            },
        )
        .unwrap();
        register(&mut r).unwrap();
        register(&mut r).unwrap();
        assert_eq!(r.entries().count(), SETTINGS.len());
        let o = EditorOptions::default();
        assert_eq!(&*o.font_family, "Consolas");
        assert_eq!(o.font_fallbacks, vec!["Courier New".to_string()]);
        assert_eq!(o.font_size, 14.0);
        assert_eq!(o.zoomed_line_height(0), 19.0);
        assert_eq!(o.tab_size, 4);
        assert_eq!(o.stop_rendering_line_after, Some(10000));
        assert!(o.scroll_beyond_last_line && o.glyph_margin && o.empty_selection_clipboard);
        assert_eq!(o.caret_width(), 2.0);
    }

    #[test]
    fn values_are_parsed_and_clamped_like_vscode() {
        let o = with(vec![
            ("editor.fontSize", Value::Number(1000.0)),
            ("editor.fontWeight", Value::String("bold".into())),
            ("editor.lineHeight", Value::Number(1.5)),
            (
                "editor.fontLigatures",
                Value::String("'ss01', 'zero' 1, 'liga' off".into()),
            ),
            ("editor.stopRenderingLineAfter", Value::Number(-1.0)),
            ("editor.lineNumbers", Value::String("relative".into())),
            ("editor.cursorStyle", Value::String("block-outline".into())),
            ("editor.scrollbar.vertical", Value::String("hidden".into())),
            (
                "editor.mouseWheelScrollSensitivity",
                Value::String("bogus".into()),
            ),
        ]);
        assert_eq!(o.font_size, 100.0);
        assert_eq!(o.font_weight, 700.0);
        assert_eq!(o.zoomed_line_height(0), 150.0);
        assert_eq!(
            o.ligatures,
            Ligatures::Features(vec![
                ("ss01".into(), 1),
                ("zero".into(), 1),
                ("liga".into(), 0)
            ])
        );
        assert_eq!(o.stop_rendering_line_after, None);
        assert_eq!(o.line_numbers, LineNumbers::Relative);
        assert_eq!(o.cursor_style, CursorStyle::BlockOutline);
        assert_eq!(o.vertical_scrollbar, ScrollbarVisibility::Hidden);
        assert_eq!(o.wheel_sensitivity, 1.0);
        assert_eq!(o.zoomed_font_size(2), 100.0 * 1.2);
        assert_eq!(zoom_multiplier(99), 3.0);
    }
}
