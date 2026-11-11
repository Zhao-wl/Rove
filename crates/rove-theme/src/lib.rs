//! Theme contracts (#10). UI converts these values to GPUI colors at the boundary.
use rove_base::*;
use std::sync::Arc;
/// Unpremultiplied sRGB RGBA channels, no platform type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);
#[derive(Clone, Copy, Debug)]
pub enum ThemeKind {
    Dark,
    Light,
    HighContrastDark,
    HighContrastLight,
}
/// None is a deliberately unset/transparent semantic color, not a missing registration.
#[derive(Clone, Debug)]
pub struct ColorDefaults {
    pub dark: Option<Color>,
    pub light: Option<Color>,
    pub high_contrast_dark: Option<Color>,
    pub high_contrast_light: Option<Color>,
}
impl ColorDefaults {
    pub fn for_kind(&self, kind: ThemeKind) -> Option<Color> {
        match kind {
            ThemeKind::Dark => self.dark,
            ThemeKind::Light => self.light,
            ThemeKind::HighContrastDark => self.high_contrast_dark,
            ThemeKind::HighContrastLight => self.high_contrast_light,
        }
    }
}
pub type ColorRegistry = Registry<ColorDefaults>;
#[derive(Clone, Debug, Default)]
pub struct TokenStyle {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}
/// Cached immutable theme; Send + Sync. UI-safe O(log keys) color read; scope matching must be
/// cached/off-thread when costly (O(scope depth * rules) worst case). Missing color ID => NotFound.
/// Subscribers receive revision invalidations, never GPUI handles. Parsing errors preserve old theme.
pub trait Theme: Send + Sync {
    fn color(&self, id: &str) -> Result<Option<Color>>;
    fn token_style(&self, scopes: &[Arc<str>]) -> TokenStyle;
    fn changes(&self) -> &dyn Events<Version>;
}
/// Fixed registry defaults; does not load or watch theme files and emits no events.
pub struct DefaultTheme {
    pub colors: ColorRegistry,
    pub kind: ThemeKind,
}
impl Theme for DefaultTheme {
    fn color(&self, id: &str) -> Result<Option<Color>> {
        self.colors
            .get(id)
            .map(|d| d.for_kind(self.kind))
            .ok_or(Error::NotFound)
    }
    fn token_style(&self, _: &[Arc<str>]) -> TokenStyle {
        TokenStyle::default()
    }
    fn changes(&self) -> &dyn Events<Version> {
        &NoEvents
    }
}

// #10 主题引擎的正式实现（契约类型见上方，未改动）。
mod color;
pub mod defaults;
pub mod file;
pub mod resolved;
pub mod service;
pub mod textmate;
pub use color::{format_hex, parse_hex};
pub use resolved::{Customizations, ResolvedTheme};
pub use service::{ThemeCatalog, ThemeEntry, ThemeService, DEFAULT_THEME};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn view_registers_and_resolves_color() {
        let mut colors = ColorRegistry::default();
        colors
            .register(
                "editor.background",
                ColorDefaults {
                    dark: Some(Color(0, 0, 0, 255)),
                    light: None,
                    high_contrast_dark: None,
                    high_contrast_light: None,
                },
            )
            .unwrap();
        let theme = DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        };
        assert_eq!(
            theme.color("editor.background").unwrap(),
            Some(Color(0, 0, 0, 255))
        );
        assert!(theme.color("missing").is_err());
        let _sub = theme.changes().subscribe(Arc::new(|_| {}));
        assert!(!theme.token_style(&[]).bold);
    }
}
