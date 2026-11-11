//! 颜色默认值表：VS Code `registerColor` 的四套默认值及其派生写法（#10）。
//!
//! 数据在 [`generated`]（由 `scripts/theme/gen_color_defaults.py` 从 VS Code 源码生成）。
//! 求值语义移植自 VS Code `platform/theme/common/colorUtils.ts` 的 `resolveColorValue` /
//! `executeTransform`：引用另一个键时按当前主题解析（主题值优先，其次该键的默认值）。

#[rustfmt::skip]
mod generated;

pub use generated::{P0_KEYS, VSCODE_COMMIT, VSCODE_VERSION};

use crate::color::VsColor;
use crate::{ColorDefaults, ColorRegistry, ThemeKind};
use rove_base::Result;

/// 一个颜色默认值表达式（VS Code `ColorValue`）。
#[derive(Debug)]
pub enum V {
    /// `null`：该主题类型下明确不设色。
    None,
    Hex(&'static str),
    Rgba(u8, u8, u8, f64),
    /// 另一个颜色键。
    Ref(&'static str),
    Darken(&'static V, f64),
    Lighten(&'static V, f64),
    Transparent(&'static V, f64),
    /// `withOpacity`：直接设定 alpha。
    Opacity(&'static V, f64),
    Opaque(&'static V, &'static V),
    OneOf(&'static [V]),
    /// `ifDefinedThenElse(key, then, else)`：主题是否显式定义了 `key`。
    IfDefined(&'static str, &'static V, &'static V),
    LessProminent(&'static V, &'static V, f64, f64),
    Mix(&'static V, &'static V, Option<f64>),
}

pub(crate) fn kind_index(kind: ThemeKind) -> usize {
    match kind {
        ThemeKind::Dark => 0,
        ThemeKind::Light => 1,
        ThemeKind::HighContrastDark => 2,
        ThemeKind::HighContrastLight => 3,
    }
}

/// 查默认值表达式；`None` 表示 VS Code 没有登记这个键。
pub fn expression(id: &str, kind: ThemeKind) -> Option<&'static V> {
    let table = generated::COLOR_DEFAULTS;
    let i = table.binary_search_by(|(k, _)| (*k).cmp(id)).ok()?;
    Some(&table[i].1[kind_index(kind)])
}

/// VS Code 是否登记了这个颜色键（包括 P1/P2 与不支持的键）。
pub fn is_vscode_color(id: &str) -> bool {
    expression(id, ThemeKind::Dark).is_some()
}

/// 解析默认值时需要的主题视图（VS Code `IColorTheme` 的 `getColor`/`defines`）。
pub(crate) trait ColorScope {
    /// 主题或用户覆盖显式给出的值（即 VS Code `defines(id)` 为真时的颜色）；没有则为 `None`。
    fn explicit(&self, id: &str) -> Option<VsColor>;
    fn kind(&self) -> ThemeKind;
}

/// 解析深度上限：默认值之间的引用链很短，超过即视为环并返回 `None`。
const MAX_DEPTH: usize = 64;

/// VS Code `IColorTheme.getColor(id)`：显式值优先，否则按默认值表求值。
pub(crate) fn color(scope: &dyn ColorScope, id: &str, depth: usize) -> Option<VsColor> {
    if let Some(c) = scope.explicit(id) {
        return Some(c);
    }
    default_color(scope, id, depth)
}

/// VS Code `resolveDefaultColor`。
pub(crate) fn default_color(scope: &dyn ColorScope, id: &str, depth: usize) -> Option<VsColor> {
    if depth > MAX_DEPTH {
        return None;
    }
    eval(scope, expression(id, scope.kind())?, depth + 1)
}

fn eval(scope: &dyn ColorScope, v: &V, depth: usize) -> Option<VsColor> {
    if depth > MAX_DEPTH {
        return None;
    }
    let e = |v: &V| eval(scope, v, depth + 1);
    match v {
        V::None => None,
        V::Hex(h) => VsColor::from_hex(h),
        V::Rgba(r, g, b, a) => Some(VsColor::rgba(*r as f64, *g as f64, *b as f64, *a)),
        V::Ref(id) => color(scope, id, depth + 1),
        V::Darken(v, f) => e(v).map(|c| c.darken(*f)),
        V::Lighten(v, f) => e(v).map(|c| c.lighten(*f)),
        V::Transparent(v, f) => e(v).map(|c| c.transparent(*f)),
        V::Opacity(v, o) => e(v).map(|c| c.with_opacity(*o)),
        V::Opaque(v, bg) => {
            let c = e(v)?;
            Some(match e(bg) {
                Some(bg) => c.make_opaque(bg),
                None => c,
            })
        }
        V::OneOf(vs) => vs.iter().find_map(e),
        V::IfDefined(id, then, other) => {
            if scope.explicit(id).is_some() {
                e(then)
            } else {
                e(other)
            }
        }
        V::LessProminent(v, bg, factor, transparency) => {
            Some(e(v)?.less_prominent(e(bg), *factor, *transparency))
        }
        V::Mix(a, b, ratio) => {
            let a = e(a).unwrap_or(VsColor::TRANSPARENT);
            let b = e(b).unwrap_or(VsColor::TRANSPARENT);
            Some(a.mix(b, ratio.unwrap_or(0.5)))
        }
    }
}

/// 没有任何主题值时的默认值（契约登记表里的静态默认值）。
struct Bare(ThemeKind);
impl ColorScope for Bare {
    fn explicit(&self, _: &str) -> Option<VsColor> {
        None
    }
    fn kind(&self) -> ThemeKind {
        self.0
    }
}

/// 按 VS Code 默认值表计算某个键的四套静态默认值；未登记的键返回 `None`。
pub fn static_defaults(id: &str) -> Option<ColorDefaults> {
    is_vscode_color(id).then(|| {
        let at = |k| default_color(&Bare(k), id, 0).map(VsColor::to_color);
        ColorDefaults {
            dark: at(ThemeKind::Dark),
            light: at(ThemeKind::Light),
            high_contrast_dark: at(ThemeKind::HighContrastDark),
            high_contrast_light: at(ThemeKind::HighContrastLight),
        }
    })
}

/// 通过 #1 的颜色登记机制登记 VS Code 颜色键（键名 + 四套默认值）。后续 issue 登记
/// P1/P2 键时也用它；VS Code 未登记的键返回 `NotFound`，重复登记返回 `Duplicate`。
pub fn register_vscode_colors(registry: &mut ColorRegistry, ids: &[&str]) -> Result<()> {
    for id in ids {
        let defaults = static_defaults(id).ok_or(rove_base::Error::NotFound)?;
        registry.register(*id, defaults)?;
    }
    Ok(())
}

/// 登记 `THEME_COLORS.md` 中全部 P0 键。
pub fn register_p0_colors(registry: &mut ColorRegistry) -> Result<()> {
    register_vscode_colors(registry, P0_KEYS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;
    use std::collections::HashMap;

    struct Map(ThemeKind, HashMap<&'static str, VsColor>);
    impl ColorScope for Map {
        fn explicit(&self, id: &str) -> Option<VsColor> {
            self.1.get(id).copied()
        }
        fn kind(&self) -> ThemeKind {
            self.0
        }
    }
    fn hex(s: &str) -> VsColor {
        VsColor::from_hex(s).unwrap()
    }
    fn get(scope: &Map, id: &str) -> Option<Color> {
        color(scope, id, 0).map(VsColor::to_color)
    }

    #[test]
    fn table_is_sorted_and_versioned() {
        let t = generated::COLOR_DEFAULTS;
        assert!(t.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(VSCODE_VERSION, "1.145.2");
    }

    #[test]
    fn literal_defaults_per_kind() {
        let d = static_defaults("editor.background").unwrap();
        assert_eq!(d.dark, Some(Color(0x1e, 0x1e, 0x1e, 255)));
        assert_eq!(d.light, Some(Color(0xff, 0xff, 0xff, 255)));
        assert_eq!(d.high_contrast_dark, Some(Color(0, 0, 0, 255)));
        assert_eq!(d.high_contrast_light, Some(Color(0xff, 0xff, 0xff, 255)));
        assert!(static_defaults("no.such.color").is_none());
    }

    #[test]
    fn reference_follows_the_theme_value() {
        // editorGroupHeader.tabsBackground 默认引用 editor.background。
        let mut theme = Map(ThemeKind::Dark, HashMap::new());
        assert_eq!(
            get(&theme, "editorGroup.emptyBackground"),
            None,
            "null 默认值保持未设置"
        );
        theme.1.insert("editor.background", hex("#123456"));
        assert_eq!(
            get(&theme, "editorPane.background"),
            Some(Color(0x12, 0x34, 0x56, 255))
        );
        assert!(matches!(
            expression("editorPane.background", ThemeKind::Dark),
            Some(V::Ref("editor.background"))
        ));
    }

    #[test]
    fn transparent_derivation() {
        // editor.foldBackground = transparent(editor.selectionBackground, 0.3)（深色/浅色），高对比为 null。
        assert!(matches!(
            expression("editor.foldBackground", ThemeKind::Dark),
            Some(V::Transparent(V::Ref("editor.selectionBackground"), f)) if *f == 0.3
        ));
        let mut theme = Map(ThemeKind::Dark, HashMap::new());
        theme.1.insert("editor.selectionBackground", hex("#ff0000"));
        // 1 * 0.3 = 0.3 → 76.5 → 77
        assert_eq!(
            get(&theme, "editor.foldBackground"),
            Some(Color(255, 0, 0, 77))
        );
        theme.0 = ThemeKind::HighContrastDark;
        assert_eq!(get(&theme, "editor.foldBackground"), None);
    }

    #[test]
    fn lighten_and_darken_derivations() {
        // 在默认值表中找一条 lighten 与一条 darken 派生，验证求值走的是派生而不是字面量。
        let find = |pred: fn(&V) -> bool| {
            generated::COLOR_DEFAULTS
                .iter()
                .find_map(|(id, vs)| {
                    vs.iter()
                        .enumerate()
                        .find(|(_, v)| pred(v))
                        .map(|(k, _)| (*id, k))
                })
                .expect("table contains derivation")
        };
        for (pred, op) in [
            (
                (|v: &V| matches!(v, V::Lighten(V::Ref(_), _))) as fn(&V) -> bool,
                VsColor::lighten as fn(VsColor, f64) -> VsColor,
            ),
            (
                |v: &V| matches!(v, V::Darken(V::Ref(_), _)),
                VsColor::darken,
            ),
        ] {
            let (id, k) = find(pred);
            let kind = [
                ThemeKind::Dark,
                ThemeKind::Light,
                ThemeKind::HighContrastDark,
                ThemeKind::HighContrastLight,
            ][k];
            let (V::Lighten(V::Ref(base), f) | V::Darken(V::Ref(base), f)) =
                expression(id, kind).unwrap()
            else {
                unreachable!()
            };
            let mut theme = Map(kind, HashMap::new());
            theme.1.insert(base, hex("#3794ff"));
            assert_eq!(
                get(&theme, id),
                Some(op(hex("#3794ff"), *f).to_color()),
                "{id}"
            );
            assert_ne!(get(&theme, id), Some(hex("#3794ff").to_color()), "{id}");
        }
    }

    #[test]
    fn if_defined_and_mix_and_one_of() {
        // checkbox.disabled.background = mix(checkbox.background, checkbox.foreground, 0.33)
        let mut theme = Map(ThemeKind::Dark, HashMap::new());
        theme.1.insert("checkbox.background", hex("#000000"));
        theme.1.insert("checkbox.foreground", hex("#ffffff"));
        assert_eq!(
            get(&theme, "checkbox.disabled.background"),
            Some(Color(84, 84, 84, 255))
        );
        let any_if = generated::COLOR_DEFAULTS
            .iter()
            .any(|(_, vs)| vs.iter().any(|v| matches!(v, V::IfDefined(..))));
        let any_one_of = generated::COLOR_DEFAULTS
            .iter()
            .any(|(_, vs)| vs.iter().any(|v| matches!(v, V::OneOf(..))));
        assert!(any_if || any_one_of);
    }

    #[test]
    fn every_p0_key_has_four_registered_defaults() {
        let mut registry = ColorRegistry::default();
        register_p0_colors(&mut registry).unwrap();
        assert_eq!(registry.entries().count(), P0_KEYS.len());
        assert!(register_p0_colors(&mut registry).is_err(), "重复登记被拒绝");
        // 每个 P0 键在四种主题类型下都有一条默认值表达式（可以是明确的 null）。
        for id in P0_KEYS {
            for kind in [
                ThemeKind::Dark,
                ThemeKind::Light,
                ThemeKind::HighContrastDark,
                ThemeKind::HighContrastLight,
            ] {
                assert!(expression(id, kind).is_some(), "{id}");
            }
        }
        let d = registry.get("statusBarItem.hoverBackground").unwrap();
        assert_eq!(d.dark, Some(Color(255, 255, 255, 31)));
        assert_eq!(d.high_contrast_dark, Some(Color(0, 0, 0, 255)));
    }

    #[test]
    fn cyclic_references_terminate() {
        struct Cyclic;
        impl ColorScope for Cyclic {
            fn explicit(&self, _: &str) -> Option<VsColor> {
                None
            }
            fn kind(&self) -> ThemeKind {
                ThemeKind::Dark
            }
        }
        // 表中不存在环；直接构造深度超限也必须返回而不是栈溢出。
        assert_eq!(
            default_color(&Cyclic, "editor.background", MAX_DEPTH + 1),
            None
        );
    }
}
