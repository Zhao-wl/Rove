//! 代码视图使用的颜色键：登记（VS Code 内置默认值）与解析。
//!
//! 主题（#10）若已登记同名键则跳过；视图解析颜色时先问当前主题，主题未登记或明确未设色时
//! 用这里的默认值，确保在 #10 合并前后都能正确显示。

use rove_base::Result;
use rove_theme::{Color, ColorDefaults, ColorRegistry, Theme, ThemeKind};

const fn rgba(v: u32) -> Option<Color> {
    Some(Color(
        (v >> 24) as u8,
        (v >> 16) as u8,
        (v >> 8) as u8,
        v as u8,
    ))
}

/// (键, 深色, 浅色, 深色高对比, 浅色高对比)。`None` 表示 VS Code 明确不设色。
/// 数值取自 VS Code 1.145 的颜色登记默认值（`editorColorRegistry.ts`、`colorRegistry.ts`）；
/// `editorGutter.background` 跟随 `editor.background`，在解析时处理。
/// 一个颜色键的默认值：(键, 深色, 浅色, 深色高对比, 浅色高对比)。
pub type ColorDefault = (
    &'static str,
    Option<Color>,
    Option<Color>,
    Option<Color>,
    Option<Color>,
);

pub const COLORS: &[ColorDefault] = &[
    (
        "editor.background",
        rgba(0x1E1E1EFF),
        rgba(0xFFFFFFFF),
        rgba(0x000000FF),
        rgba(0xFFFFFFFF),
    ),
    (
        "editor.foreground",
        rgba(0xD4D4D4FF),
        rgba(0x000000FF),
        rgba(0xFFFFFFFF),
        rgba(0x292929FF),
    ),
    (
        "editorLineNumber.foreground",
        rgba(0x858585FF),
        rgba(0x237893FF),
        rgba(0xFFFFFFFF),
        rgba(0x292929FF),
    ),
    (
        "editorLineNumber.activeForeground",
        rgba(0xC6C6C6FF),
        rgba(0x0B216FFF),
        rgba(0xF38518FF),
        rgba(0x0F4A85FF),
    ),
    ("editorLineNumber.dimmedForeground", None, None, None, None),
    ("editorGutter.background", None, None, None, None),
    ("editor.lineHighlightBackground", None, None, None, None),
    (
        "editor.lineHighlightBorder",
        rgba(0x282828FF),
        rgba(0xEEEEEEFF),
        rgba(0xF38518FF),
        rgba(0x0F4A85FF),
    ),
    (
        "editor.inactiveLineHighlightBackground",
        None,
        None,
        None,
        None,
    ),
    (
        "editorCursor.foreground",
        rgba(0xAEAFADFF),
        rgba(0x000000FF),
        rgba(0xFFFFFFFF),
        rgba(0x0F4A85FF),
    ),
    ("editorCursor.background", None, None, None, None),
    (
        "editor.selectionBackground",
        rgba(0x264F78FF),
        rgba(0xADD6FFFF),
        rgba(0xFFFFFFFF),
        rgba(0x0F4A85FF),
    ),
    (
        "editor.selectionForeground",
        None,
        None,
        rgba(0x000000FF),
        rgba(0xFFFFFFFF),
    ),
    (
        "editor.inactiveSelectionBackground",
        rgba(0x3A3D41FF),
        rgba(0xE5EBF1FF),
        rgba(0xFFFFFF80),
        rgba(0x0F4A8580),
    ),
    (
        "scrollbar.shadow",
        rgba(0x000000FF),
        rgba(0xDDDDDDFF),
        None,
        None,
    ),
    (
        "scrollbarSlider.background",
        rgba(0x79797966),
        rgba(0x64646466),
        rgba(0x6FC3DF99),
        rgba(0x0F4A8566),
    ),
    (
        "scrollbarSlider.hoverBackground",
        rgba(0x646464B3),
        rgba(0x646464B3),
        rgba(0x6FC3DF80),
        rgba(0x0F4A8580),
    ),
    (
        "scrollbarSlider.activeBackground",
        rgba(0xBFBFBF66),
        rgba(0x00000099),
        rgba(0x6FC3DFFF),
        rgba(0x0F4A85FF),
    ),
];

/// #20 使用的颜色键。默认值不在本模块重复，取 #10 生成的 VS Code 默认值表
/// （`rove_theme::defaults::static_defaults`，含派生）；其中 `editor.wordHighlight*` 是 P1 键，
/// 主题服务的 P0 登记表里没有，需经 [`theme_registry`] 一并登记，用户主题才能覆盖它们。
pub const FEATURE_COLORS: &[&str] = &[
    "editorBracketMatch.background",
    "editorBracketMatch.border",
    "editorBracketMatch.foreground",
    "editor.wordHighlightBackground",
    "editor.wordHighlightBorder",
    "editor.wordHighlightStrongBackground",
    "editor.wordHighlightStrongBorder",
    "editor.wordHighlightTextBackground",
    "editor.wordHighlightTextBorder",
    "editor.selectionHighlightBackground",
    "editor.selectionHighlightBorder",
    "editor.foldBackground",
    "editor.foldPlaceholderForeground",
    "editorGutter.foldingControlForeground",
    "editorIndentGuide.background1",
    "editorIndentGuide.background2",
    "editorIndentGuide.background3",
    "editorIndentGuide.background4",
    "editorIndentGuide.background5",
    "editorIndentGuide.background6",
    "editorWhitespace.foreground",
];

/// 主题服务用的颜色登记表：全部 P0 键，加上代码视图用到的其余键（[`COLORS`]、[`FEATURE_COLORS`]）。
pub fn theme_registry() -> ColorRegistry {
    let mut registry = rove_theme::service::p0_registry();
    register(&mut registry).expect("code view colors");
    crate::find::registry::register_colors(&mut registry).expect("find colors");
    // #25：搜索视图。
    crate::search::registry::register_colors(&mut registry).expect("search colors");
    registry
}

/// 登记颜色键；已存在的键跳过（主题或其他视图已登记）。
pub fn register(registry: &mut ColorRegistry) -> Result<()> {
    for key in FEATURE_COLORS {
        if registry.get(key).is_some() {
            continue;
        }
        if let Some(defaults) = rove_theme::defaults::static_defaults(key) {
            registry.register(*key, defaults)?;
        }
    }
    for &(key, dark, light, hcd, hcl) in COLORS {
        if registry.get(key).is_some() {
            continue;
        }
        registry.register(
            key,
            ColorDefaults {
                dark,
                light,
                high_contrast_dark: hcd,
                high_contrast_light: hcl,
            },
        )?;
    }
    Ok(())
}

/// 解析后的调色板；主题变化时整体重建，绘制只读这里。
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub line_number: Color,
    pub line_number_active: Color,
    pub line_number_dimmed: Color,
    pub gutter_background: Color,
    pub line_highlight_background: Option<Color>,
    pub line_highlight_border: Option<Color>,
    pub inactive_line_highlight_background: Option<Color>,
    pub cursor: Color,
    pub cursor_text: Option<Color>,
    pub selection: Color,
    pub selection_foreground: Option<Color>,
    pub inactive_selection: Color,
    pub scrollbar_shadow: Option<Color>,
    pub slider: Color,
    pub slider_hover: Color,
    pub slider_active: Color,
    // #20
    pub bracket_match_background: Option<Color>,
    pub bracket_match_border: Option<Color>,
    pub bracket_match_foreground: Option<Color>,
    pub word_highlight_text_background: Option<Color>,
    pub word_highlight_text_border: Option<Color>,
    pub selection_highlight_background: Option<Color>,
    pub selection_highlight_border: Option<Color>,
    pub fold_background: Option<Color>,
    pub fold_placeholder_foreground: Option<Color>,
    pub folding_control_foreground: Color,
    /// `editorIndentGuide.background1`–`background6`，按缩进层级循环取。
    pub indent_guides: [Option<Color>; 6],
    pub whitespace: Option<Color>,
}

fn fallback(key: &str, kind: ThemeKind) -> Option<Color> {
    if FEATURE_COLORS.contains(&key) {
        return rove_theme::defaults::static_defaults(key).and_then(|d| d.for_kind(kind));
    }
    COLORS
        .iter()
        .find(|c| c.0 == key)
        .and_then(|&(_, d, l, hcd, hcl)| match kind {
            ThemeKind::Dark => d,
            ThemeKind::Light => l,
            ThemeKind::HighContrastDark => hcd,
            ThemeKind::HighContrastLight => hcl,
        })
}

impl Palette {
    /// 先问主题；主题没有这个键（`NotFound`）时用本模块的默认值。主题明确返回 `None` 表示不设色。
    pub fn resolve(theme: &dyn Theme, kind: ThemeKind) -> Self {
        let get = |key: &str| match theme.color(key) {
            Ok(c) => c,
            Err(_) => fallback(key, kind),
        };
        let background = get("editor.background").unwrap_or(Color(0x1E, 0x1E, 0x1E, 255));
        let foreground = get("editor.foreground").unwrap_or(Color(0xD4, 0xD4, 0xD4, 255));
        let line_number = get("editorLineNumber.foreground").unwrap_or(foreground);
        let selection = get("editor.selectionBackground").unwrap_or(Color(0x26, 0x4F, 0x78, 255));
        let line_highlight_background = get("editor.lineHighlightBackground");
        Self {
            background,
            foreground,
            line_number,
            line_number_active: get("editorLineNumber.activeForeground").unwrap_or(line_number),
            // VS Code：未设色时为前景色的半透明。
            line_number_dimmed: get("editorLineNumber.dimmedForeground").unwrap_or(Color(
                line_number.0,
                line_number.1,
                line_number.2,
                line_number.3 / 2,
            )),
            gutter_background: get("editorGutter.background").unwrap_or(background),
            line_highlight_background,
            line_highlight_border: get("editor.lineHighlightBorder"),
            inactive_line_highlight_background: get("editor.inactiveLineHighlightBackground")
                .or(line_highlight_background),
            cursor: get("editorCursor.foreground").unwrap_or(foreground),
            cursor_text: get("editorCursor.background"),
            selection,
            selection_foreground: get("editor.selectionForeground"),
            inactive_selection: get("editor.inactiveSelectionBackground").unwrap_or(Color(
                selection.0,
                selection.1,
                selection.2,
                selection.3 / 2,
            )),
            scrollbar_shadow: get("scrollbar.shadow"),
            slider: get("scrollbarSlider.background").unwrap_or(Color(0x79, 0x79, 0x79, 0x66)),
            slider_hover: get("scrollbarSlider.hoverBackground")
                .unwrap_or(Color(0x64, 0x64, 0x64, 0xB3)),
            slider_active: get("scrollbarSlider.activeBackground")
                .unwrap_or(Color(0xBF, 0xBF, 0xBF, 0x66)),
            bracket_match_background: get("editorBracketMatch.background"),
            bracket_match_border: get("editorBracketMatch.border"),
            bracket_match_foreground: get("editorBracketMatch.foreground"),
            word_highlight_text_background: get("editor.wordHighlightTextBackground"),
            word_highlight_text_border: get("editor.wordHighlightTextBorder"),
            selection_highlight_background: get("editor.selectionHighlightBackground"),
            selection_highlight_border: get("editor.selectionHighlightBorder"),
            fold_background: get("editor.foldBackground"),
            fold_placeholder_foreground: get("editor.foldPlaceholderForeground"),
            folding_control_foreground: get("editorGutter.foldingControlForeground")
                .unwrap_or(foreground),
            indent_guides: [
                get("editorIndentGuide.background1"),
                get("editorIndentGuide.background2"),
                get("editorIndentGuide.background3"),
                get("editorIndentGuide.background4"),
                get("editorIndentGuide.background5"),
                get("editorIndentGuide.background6"),
            ],
            whitespace: get("editorWhitespace.foreground"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_theme::DefaultTheme;

    #[test]
    fn theme_values_win_and_missing_keys_fall_back() {
        let mut colors = ColorRegistry::default();
        let red = Some(Color(255, 0, 0, 255));
        colors
            .register(
                "editor.background",
                ColorDefaults {
                    dark: red,
                    light: red,
                    high_contrast_dark: red,
                    high_contrast_light: red,
                },
            )
            .unwrap();
        register(&mut colors).unwrap();
        register(&mut colors).unwrap();
        assert_eq!(
            colors.entries().count(),
            COLORS.len() + FEATURE_COLORS.len()
        );
        let theme = DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        };
        let p = Palette::resolve(&theme, ThemeKind::Dark);
        assert_eq!(Some(p.background), red);
        assert_eq!(p.gutter_background, p.background);
        assert_eq!(p.selection, Color(0x26, 0x4F, 0x78, 255));
        assert_eq!(p.line_highlight_background, None);
        assert_eq!(p.line_highlight_border, Some(Color(0x28, 0x28, 0x28, 255)));
        // 主题完全没有登记时同样可用。
        let empty = DefaultTheme {
            colors: ColorRegistry::default(),
            kind: ThemeKind::Light,
        };
        let p = Palette::resolve(&empty, ThemeKind::Light);
        assert_eq!(p.background, Color(255, 255, 255, 255));
    }
}
