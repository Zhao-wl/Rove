//! #59 的主题侧入口测试：语义 token（类型 + 修饰符）→ 最终样式。
//!
//! 两条路径各自有用例：主题写了 `semanticTokenColors` 时按主题，没写时走回退表 →
//! `tokenColors`；再加上 `editor.semanticTokenColorCustomizations` 的合并语义
//! （含按主题的形式）与 `semanticHighlighting` 开关的来源。

use rove_base::{Error, Result, Value};
use rove_theme::file::{load, ThemeSource};
use rove_theme::resolved::{Customizations, ResolvedTheme};
use rove_theme::semantic::SemanticStyle;
use rove_theme::service::p0_registry;
use rove_theme::{parse_hex, Color, ThemeKind};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

struct OneFile(&'static str, String);

impl ThemeSource for OneFile {
    fn read(&self, path: &str) -> Result<Cow<'static, str>> {
        if path == self.0 {
            Ok(Cow::Owned(self.1.clone()))
        } else {
            Err(Error::NotFound)
        }
    }
}

/// 只写了 `tokenColors` 的主题：语义着色只能靠回退表。
const TEXTMATE_ONLY: &str = r##"{
  "name": "Probe", "type": "dark",
  "colors": { "editor.foreground": "#cccccc", "editor.background": "#101010" },
  "tokenColors": [
    { "scope": "entity.name.type.class", "settings": { "foreground": "#4ec9b0" } },
    { "scope": "entity.name.type.struct", "settings": { "foreground": "#86c691" } },
    { "scope": "entity.name.type.interface", "settings": { "foreground": "#b8d7a3" } },
    { "scope": "entity.name.type.enum", "settings": { "foreground": "#b8d7a4" } },
    { "scope": "entity.name.namespace", "settings": { "foreground": "#4ec9b1" } },
    { "scope": "variable.other.constant", "settings": { "foreground": "#4fc1ff", "fontStyle": "bold" } },
    { "scope": "variable.other.readwrite", "settings": { "foreground": "#9cdcfe" } },
    { "scope": "variable.other.property", "settings": { "foreground": "#9cdcff" } },
    { "scope": "variable.other.enummember", "settings": { "foreground": "#b5cea8" } },
    { "scope": "variable.parameter", "settings": { "foreground": "#9a9a9a" } },
    { "scope": "entity.name.function.member", "settings": { "foreground": "#dcdcaa" } },
    { "scope": "support.class", "settings": { "foreground": "#ff00ff" } }
  ]
}"##;

/// 同一份主题再加上 `semanticTokenColors`。
const WITH_SEMANTIC: &str = r##"{
  "name": "Probe", "type": "dark",
  "semanticHighlighting": true,
  "colors": { "editor.foreground": "#cccccc", "editor.background": "#101010" },
  "tokenColors": [
    { "scope": "entity.name.type.class", "settings": { "foreground": "#4ec9b0" } },
    { "scope": "variable.other.readwrite", "settings": { "foreground": "#9cdcfe" } }
  ],
  "semanticTokenColors": {
    "class": "#010203",
    "variable.readonly": { "foreground": "#040506", "italic": true },
    "*.deprecated": { "strikethrough": true },
    "property:csharp": "#070809"
  }
}"##;

fn c(hex: &str) -> Color {
    parse_hex(hex).unwrap()
}

fn theme(json: &str, custom: Customizations) -> ResolvedTheme {
    let source = OneFile("t.json", json.to_owned());
    let doc = load(&source, "t.json").unwrap();
    ResolvedTheme::build(
        "Probe",
        ThemeKind::Dark,
        doc,
        &custom,
        Arc::new(p0_registry()),
    )
}

fn style(theme: &ResolvedTheme, ty: &str, mods: &[&str]) -> SemanticStyle {
    theme.semantic_token_style(ty, mods, "csharp")
}

fn obj(entries: &[(&str, Value)]) -> Value {
    Value::Object(Arc::new(
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
    ))
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}

#[test]
fn theme_without_semantic_rules_falls_back_to_textmate_scopes() {
    let theme = theme(TEXTMATE_ONLY, Customizations::default());
    // 回退表里的每一条代表性映射都走通。
    for (ty, mods, hex) in [
        ("class", &[][..], "#4ec9b0"),
        ("struct", &[][..], "#86c691"),
        ("interface", &[][..], "#b8d7a3"),
        ("enum", &[][..], "#b8d7a4"),
        ("namespace", &[][..], "#4ec9b1"),
        ("variable", &[][..], "#9cdcfe"),
        ("variable", &["readonly"][..], "#4fc1ff"),
        ("property", &[][..], "#9cdcff"),
        ("enumMember", &[][..], "#b5cea8"),
        ("parameter", &[][..], "#9a9a9a"),
        ("method", &[][..], "#dcdcaa"),
    ] {
        assert_eq!(
            style(&theme, ty, mods).foreground,
            Some(c(hex)),
            "{ty}{mods:?} 应走回退表"
        );
    }
    // `variable.other.constant` 带 fontStyle，粗体也跟着回退表过来。
    assert_eq!(style(&theme, "variable", &["readonly"]).bold, Some(true));
    // 命中的规则没写 `fontStyle` 时粗斜体保持「未设置」，不覆盖语法层。
    assert_eq!(style(&theme, "variable", &[]).bold, None);
    // 第一组 scope 没命中时试第二组：`class.defaultLibrary` → support.class。
    assert_eq!(
        style(&theme, "class", &["defaultLibrary"]).foreground,
        Some(c("#ff00ff"))
    );
    // 回退表里没有的类型、主题里也没有对应 scope：不着色，保持语法高亮。
    assert_eq!(
        style(&theme, "typeParameter", &[]),
        SemanticStyle::default()
    );
}

#[test]
fn theme_semantic_rules_win_over_the_fallback_table() {
    let theme = theme(WITH_SEMANTIC, Customizations::default());
    assert!(theme.semantic_highlighting());
    assert_eq!(style(&theme, "class", &[]).foreground, Some(c("#010203")));
    // 修饰符更具体的规则分数更高。
    let ro = style(&theme, "variable", &["readonly"]);
    assert_eq!(ro.foreground, Some(c("#040506")));
    assert_eq!(ro.italic, Some(true));
    // 通配符规则只给删除线，前景色仍由回退表补齐。
    let dep = style(&theme, "variable", &["deprecated"]);
    assert_eq!(dep.strikethrough, Some(true));
    assert_eq!(dep.foreground, Some(c("#9cdcfe")));
    // 语言限定：`property:csharp` 命中 C#，不命中其他语言。
    assert_eq!(
        theme
            .semantic_token_style("property", &[], "csharp")
            .foreground,
        Some(c("#070809"))
    );
    assert_eq!(
        theme
            .semantic_token_style("property", &[], "java")
            .foreground,
        None
    );
}

#[test]
fn semantic_token_color_customizations_merge_global_then_theme_specific() {
    let custom = Customizations {
        semantic_token_colors: Some(obj(&[
            (
                "rules",
                obj(&[("class", s("#111111")), ("method", s("#222222"))]),
            ),
            (
                "[Probe]",
                obj(&[("rules", obj(&[("class", s("#333333"))]))]),
            ),
            (
                "[Other]",
                obj(&[("rules", obj(&[("method", s("#444444"))]))]),
            ),
        ])),
        ..Customizations::default()
    };
    let theme = theme(WITH_SEMANTIC, custom);
    // 用户覆盖高于主题。
    assert_eq!(style(&theme, "method", &[]).foreground, Some(c("#222222")));
    // 按主题的形式高于全局形式（同分时后读入的赢）。
    assert_eq!(style(&theme, "class", &[]).foreground, Some(c("#333333")));
}

#[test]
fn the_enabled_flag_comes_from_theme_then_customizations() {
    // 主题没写 `semanticHighlighting` → false。
    assert!(!theme(TEXTMATE_ONLY, Customizations::default()).semantic_highlighting());
    // 主题写了 → true。
    assert!(theme(WITH_SEMANTIC, Customizations::default()).semantic_highlighting());
    // `editor.tokenColorCustomizations.semanticHighlighting`（旧写法）可以关掉它。
    let deprecated = Customizations {
        token_colors: Some(obj(&[("semanticHighlighting", Value::Bool(false))])),
        ..Customizations::default()
    };
    assert!(!theme(WITH_SEMANTIC, deprecated).semantic_highlighting());
    // `editor.semanticTokenColorCustomizations.enabled` 优先级最高。
    let enabled = Customizations {
        token_colors: Some(obj(&[("semanticHighlighting", Value::Bool(false))])),
        semantic_token_colors: Some(obj(&[("enabled", Value::Bool(true))])),
        ..Customizations::default()
    };
    assert!(theme(WITH_SEMANTIC, enabled).semantic_highlighting());
    // 按主题的形式再覆盖一次。
    let scoped = Customizations {
        semantic_token_colors: Some(obj(&[
            ("enabled", Value::Bool(true)),
            ("[Probe]", obj(&[("enabled", Value::Bool(false))])),
        ])),
        ..Customizations::default()
    };
    assert!(!theme(WITH_SEMANTIC, scoped).semantic_highlighting());
}

#[test]
fn semantic_styles_are_cached_and_agree_with_the_uncached_path() {
    let theme = theme(WITH_SEMANTIC, Customizations::default());
    for (ty, mods) in [
        ("class", &[][..]),
        ("variable", &["readonly"][..]),
        ("method", &["static"][..]),
    ] {
        let cached = theme.semantic_token_style(ty, mods, "csharp");
        let again = theme.semantic_token_style(ty, mods, "csharp");
        assert_eq!(cached, again);
        assert_eq!(
            cached,
            theme.semantic_token_style_uncached(ty, mods, "csharp")
        );
    }
}
