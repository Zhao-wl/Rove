//! `workbench.list.*` / `workbench.tree.*` settings (#18): registration and typed snapshot.
use super::matcher::{FindMode, MatchType};
use rove_base::{Error, Result, Value};
use rove_config::{Scope, Setting, SettingRegistry, Settings};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiSelectModifier {
    CtrlCmd,
    Alt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    SingleClick,
    DoubleClick,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeNavigationMode {
    Automatic,
    Trigger,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentGuides {
    None,
    OnHover,
    Always,
}

/// Immutable snapshot read once per settings revision; the list never reads settings while painting.
#[derive(Clone, Debug, PartialEq)]
pub struct ListOptions {
    pub find_match_type: MatchType,
    pub find_mode: FindMode,
    pub fast_scroll_sensitivity: f32,
    pub horizontal_scrolling: bool,
    pub mouse_wheel_scroll_sensitivity: f32,
    pub multi_select_modifier: MultiSelectModifier,
    pub open_mode: OpenMode,
    pub scroll_by_page: bool,
    pub smooth_scrolling: bool,
    pub type_navigation_mode: TypeNavigationMode,
    pub sticky_scroll: bool,
    pub sticky_scroll_max_items: usize,
    pub expand_mode: OpenMode,
    pub indent: f32,
    pub indent_guides: IndentGuides,
}

impl Default for ListOptions {
    /// VS Code 1.145 defaults (docs/compat/SETTINGS.md).
    fn default() -> Self {
        Self {
            find_match_type: MatchType::Fuzzy,
            find_mode: FindMode::Highlight,
            fast_scroll_sensitivity: 5.0,
            horizontal_scrolling: false,
            mouse_wheel_scroll_sensitivity: 1.0,
            multi_select_modifier: MultiSelectModifier::CtrlCmd,
            open_mode: OpenMode::SingleClick,
            scroll_by_page: false,
            smooth_scrolling: false,
            type_navigation_mode: TypeNavigationMode::Automatic,
            sticky_scroll: true,
            sticky_scroll_max_items: 7,
            expand_mode: OpenMode::SingleClick,
            indent: 8.0,
            indent_guides: IndentGuides::OnHover,
        }
    }
}

fn s(v: &str) -> Value {
    Value::String(Arc::from(v))
}

/// All P0 keys this component consumes, with VS Code defaults. `keyboardNavigation` is the
/// deprecated fallback for find mode / type navigation, kept for `settings.json` compatibility.
pub fn setting_defaults() -> Vec<(&'static str, Value)> {
    vec![
        ("workbench.list.defaultFindMatchType", s("fuzzy")),
        ("workbench.list.defaultFindMode", s("highlight")),
        ("workbench.list.fastScrollSensitivity", Value::Number(5.0)),
        ("workbench.list.horizontalScrolling", Value::Bool(false)),
        ("workbench.list.keyboardNavigation", s("highlight")),
        (
            "workbench.list.mouseWheelScrollSensitivity",
            Value::Number(1.0),
        ),
        ("workbench.list.multiSelectModifier", s("ctrlCmd")),
        ("workbench.list.openMode", s("singleClick")),
        ("workbench.list.scrollByPage", Value::Bool(false)),
        ("workbench.list.smoothScrolling", Value::Bool(false)),
        ("workbench.list.typeNavigationMode", s("automatic")),
        ("workbench.tree.enableStickyScroll", Value::Bool(true)),
        ("workbench.tree.expandMode", s("singleClick")),
        ("workbench.tree.indent", Value::Number(8.0)),
        ("workbench.tree.renderIndentGuides", s("onHover")),
        (
            "workbench.tree.stickyScrollMaxItemCount",
            Value::Number(7.0),
        ),
    ]
}

/// Registers the settings above. A key already registered by its central owner (#13) keeps the
/// existing entry: defaults are the same VS Code values, so the duplicate is not an error.
pub fn register_settings(registry: &mut SettingRegistry) -> Result<()> {
    for (key, default) in setting_defaults() {
        match registry.register(
            key,
            Setting {
                default,
                rove_default: None,
                language_overridable: false,
            },
        ) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

impl ListOptions {
    /// Reads a snapshot. Invalid or missing values fall back to the VS Code default for that key
    /// (VS Code also ignores invalid values), so one bad entry never disables the list.
    pub fn read(settings: &dyn Settings) -> Self {
        let d = Self::default();
        let scope = Scope::default();
        let get = |k: &str| settings.read(k, &scope).ok();
        let string = |k: &str| match get(k) {
            Some(Value::String(v)) => Some(v),
            _ => None,
        };
        let number = |k: &str| match get(k) {
            Some(Value::Number(v)) if v.is_finite() => Some(v as f32),
            _ => None,
        };
        let boolean = |k: &str| match get(k) {
            Some(Value::Bool(v)) => Some(v),
            _ => None,
        };
        let legacy = string("workbench.list.keyboardNavigation");
        let find_mode = match string("workbench.list.defaultFindMode").as_deref() {
            Some("highlight") => FindMode::Highlight,
            Some("filter") => FindMode::Filter,
            _ => match legacy.as_deref() {
                Some("filter") => FindMode::Filter,
                _ => d.find_mode,
            },
        };
        let type_navigation_mode = match string("workbench.list.typeNavigationMode").as_deref() {
            Some("automatic") => TypeNavigationMode::Automatic,
            Some("trigger") => TypeNavigationMode::Trigger,
            _ => d.type_navigation_mode,
        };
        let click = |k: &str, def: OpenMode| match string(k).as_deref() {
            Some("singleClick") => OpenMode::SingleClick,
            Some("doubleClick") => OpenMode::DoubleClick,
            _ => def,
        };
        Self {
            find_match_type: match string("workbench.list.defaultFindMatchType").as_deref() {
                Some("fuzzy") => MatchType::Fuzzy,
                Some("contiguous") => MatchType::Contiguous,
                _ => d.find_match_type,
            },
            find_mode,
            fast_scroll_sensitivity: number("workbench.list.fastScrollSensitivity")
                .filter(|v| *v > 0.0)
                .unwrap_or(d.fast_scroll_sensitivity),
            horizontal_scrolling: boolean("workbench.list.horizontalScrolling")
                .unwrap_or(d.horizontal_scrolling),
            mouse_wheel_scroll_sensitivity: number("workbench.list.mouseWheelScrollSensitivity")
                .filter(|v| *v > 0.0)
                .unwrap_or(d.mouse_wheel_scroll_sensitivity),
            multi_select_modifier: match string("workbench.list.multiSelectModifier").as_deref() {
                Some("alt") => MultiSelectModifier::Alt,
                Some("ctrlCmd") => MultiSelectModifier::CtrlCmd,
                _ => d.multi_select_modifier,
            },
            open_mode: click("workbench.list.openMode", d.open_mode),
            scroll_by_page: boolean("workbench.list.scrollByPage").unwrap_or(d.scroll_by_page),
            smooth_scrolling: boolean("workbench.list.smoothScrolling")
                .unwrap_or(d.smooth_scrolling),
            type_navigation_mode,
            sticky_scroll: boolean("workbench.tree.enableStickyScroll").unwrap_or(d.sticky_scroll),
            sticky_scroll_max_items: number("workbench.tree.stickyScrollMaxItemCount")
                .map(|v| v.clamp(1.0, 20.0) as usize)
                .unwrap_or(d.sticky_scroll_max_items),
            expand_mode: click("workbench.tree.expandMode", d.expand_mode),
            // VS Code clamps the indent to [4, 40] pixels.
            indent: number("workbench.tree.indent")
                .map(|v| v.clamp(4.0, 40.0))
                .unwrap_or(d.indent),
            indent_guides: match string("workbench.tree.renderIndentGuides").as_deref() {
                Some("none") => IndentGuides::None,
                Some("onHover") => IndentGuides::OnHover,
                Some("always") => IndentGuides::Always,
                _ => d.indent_guides,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_config::DefaultSettings;

    #[test]
    fn registered_defaults_read_back_as_vscode_defaults() {
        let mut r = SettingRegistry::default();
        register_settings(&mut r).unwrap();
        // Idempotent when the central settings owner registered first.
        register_settings(&mut r).unwrap();
        assert_eq!(
            ListOptions::read(&DefaultSettings(r)),
            ListOptions::default()
        );
    }

    #[test]
    fn invalid_values_fall_back_per_key() {
        let mut r = SettingRegistry::default();
        for (k, v) in setting_defaults() {
            let v = match k {
                "workbench.tree.indent" => Value::Number(100.0),
                "workbench.list.openMode" => Value::String("tripleClick".into()),
                "workbench.list.defaultFindMode" => Value::Bool(true),
                "workbench.list.keyboardNavigation" => Value::String("filter".into()),
                _ => v,
            };
            r.register(
                k,
                Setting {
                    default: v,
                    rove_default: None,
                    language_overridable: false,
                },
            )
            .unwrap();
        }
        let o = ListOptions::read(&DefaultSettings(r));
        assert_eq!(o.indent, 40.0);
        assert_eq!(o.open_mode, OpenMode::SingleClick);
        // Invalid new key => deprecated keyboardNavigation decides.
        assert_eq!(o.find_mode, FindMode::Filter);
    }
}
