//! 加载 VS Code 颜色主题 JSON（含 `include` 链），语义移植自 VS Code
//! `workbench/services/themes/common/colorThemeData.ts` 的 `_loadColorTheme`（MIT）。
//!
//! 与 VS Code 相同：先加载 `include` 指向的文件（相对当前文件所在目录），再叠加本文件，
//! 所以本文件的 `colors` 覆盖被包含文件，`tokenColors` 追加在被包含文件之后（后定义者胜）；
//! `colors` 中值为 `"default"` 的键删除已有值；旧式 `settings` 数组按 `convertSettings` 处理。
//!
//! 与 VS Code 不同之处（更宽容）：非法颜色、错误类型的字段、无法读取或解析的被包含文件、
//! 循环 include 都只跳过出问题的部分并记录一条带文件与字段位置的诊断，而不是整份主题失败。
//! 只有主题入口文件本身无法读取或不是 JSON 对象时才返回错误。

use crate::color::parse_hex;
use crate::textmate::{RawRule, RawScope};
use crate::{Color, ThemeKind};
use rove_base::{Error, Result};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

/// 主题文件来源。路径一律用 `/` 分隔的相对路径（相对来源的根），`include` 在其上解析。
pub trait ThemeSource: Send + Sync {
    fn read(&self, path: &str) -> Result<Cow<'static, str>>;
}

/// 磁盘目录中的主题文件（供 R68 加载扩展主题，以及测试）。
pub struct DirSource(pub PathBuf);

impl ThemeSource for DirSource {
    fn read(&self, path: &str) -> Result<Cow<'static, str>> {
        Ok(Cow::Owned(std::fs::read_to_string(self.0.join(path))?))
    }
}

/// 一条可定位的加载诊断：哪个文件、哪个字段、出了什么问题。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub file: String,
    /// JSON 字段位置，例如 `colors["editor.background"]`、`tokenColors[3].settings.foreground`。
    pub field: String,
    pub message: String,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.field.is_empty() {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(f, "{} {}: {}", self.file, self.field, self.message)
        }
    }
}

/// `semanticTokenColors` 的一条规则；选择器原样保存，由 R58 解释。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticTokenRule {
    pub selector: String,
    pub foreground: Option<Color>,
    pub font_style: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
}

/// 展开 include 链后的主题内容。
#[derive(Clone, Debug, Default)]
pub struct ThemeDocument {
    /// 第一个出现 `type` 字段的文件（入口文件优先）给出的主题类型。
    pub kind: Option<ThemeKind>,
    pub name: Option<String>,
    pub colors: BTreeMap<String, Color>,
    pub token_colors: Vec<RawRule>,
    pub semantic_token_colors: Vec<SemanticTokenRule>,
    pub semantic_highlighting: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// 解析主题的 `type` 字段（`dark`/`light`/`hc`/`hcLight`，也接受 `uiTheme` 写法）。
pub fn parse_kind(s: &str) -> Option<ThemeKind> {
    match s {
        "dark" | "vs-dark" => Some(ThemeKind::Dark),
        "light" | "vs" => Some(ThemeKind::Light),
        "hc" | "hcDark" | "hc-black" => Some(ThemeKind::HighContrastDark),
        "hcLight" | "hc-light" => Some(ThemeKind::HighContrastLight),
        _ => None,
    }
}

/// 加载 `path` 及其 include 链。
pub fn load(source: &dyn ThemeSource, path: &str) -> Result<ThemeDocument> {
    let mut doc = ThemeDocument::default();
    let mut stack = Vec::new();
    load_into(source, &normalize(path), &mut doc, &mut stack, true)?;
    Ok(doc)
}

/// 规范化 `/` 路径：去掉 `.`，折叠 `..`（越过根时保留，读取时自然失败）。
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn join(base_file: &str, rel: &str) -> String {
    let dir = base_file.rsplit_once('/').map_or("", |(d, _)| d);
    normalize(&format!("{dir}/{rel}"))
}

fn load_into(
    source: &dyn ThemeSource,
    path: &str,
    doc: &mut ThemeDocument,
    stack: &mut Vec<String>,
    root: bool,
) -> Result<()> {
    let diag = |doc: &mut ThemeDocument, field: &str, message: String| {
        doc.diagnostics.push(Diagnostic {
            file: path.to_owned(),
            field: field.to_owned(),
            message,
        })
    };
    if stack.iter().any(|p| p == path) {
        let chain = stack.join(" → ");
        diag(
            doc,
            "include",
            format!("循环 include（{chain} → {path}），已忽略"),
        );
        return Ok(());
    }
    let text = match source.read(path) {
        Ok(t) => t,
        Err(e) if root => return Err(e),
        Err(e) => {
            diag(doc, "", format!("无法读取被包含的主题文件：{e}"));
            return Ok(());
        }
    };
    let value = match jsonc_parser::parse_to_serde_value::<Value>(&text, &Default::default()) {
        Ok(Value::Object(map)) => map,
        Ok(_) if root => return Err(Error::InvalidInput),
        Err(_) if root => return Err(Error::InvalidInput),
        Ok(_) => {
            diag(doc, "", "主题文件必须是 JSON 对象".into());
            return Ok(());
        }
        Err(e) => {
            diag(doc, "", format!("JSON 解析失败：{e}"));
            return Ok(());
        }
    };
    if root {
        doc.name = value.get("name").and_then(Value::as_str).map(str::to_owned);
    }
    if doc.kind.is_none() {
        if let Some(t) = value.get("type") {
            match t.as_str().and_then(parse_kind) {
                Some(k) => doc.kind = Some(k),
                None => diag(doc, "type", format!("未知的主题类型 {t}，已忽略")),
            }
        }
    }
    stack.push(path.to_owned());
    if let Some(include) = value.get("include") {
        match include.as_str() {
            Some(rel) => load_into(source, &join(path, rel), doc, stack, false)?,
            None => diag(doc, "include", "应为字符串，已忽略".into()),
        }
    }
    stack.pop();

    if let Some(settings) = value.get("settings").and_then(Value::as_array) {
        convert_settings(path, settings, doc);
        return Ok(());
    }
    if let Some(sh) = value.get("semanticHighlighting") {
        match sh.as_bool() {
            Some(b) => doc.semantic_highlighting |= b,
            None => diag(doc, "semanticHighlighting", "应为布尔值，已忽略".into()),
        }
    }
    match value.get("colors") {
        None => {}
        Some(Value::Object(colors)) => {
            for (id, v) in colors {
                let field = format!("colors[{id:?}]");
                match v.as_str() {
                    Some("default") => {
                        doc.colors.remove(id);
                    }
                    Some(s) => match parse_hex(s) {
                        Some(c) => {
                            doc.colors.insert(id.clone(), c);
                        }
                        None => diag(doc, &field, format!("非法颜色值 {s:?}，已跳过该键")),
                    },
                    None => diag(
                        doc,
                        &field,
                        format!("颜色值应为字符串，实际为 {v}，已跳过该键"),
                    ),
                }
            }
        }
        Some(_) => diag(doc, "colors", "应为对象，已忽略".into()),
    }
    match value.get("tokenColors") {
        None => {}
        Some(Value::Array(rules)) => {
            for (i, rule) in rules.iter().enumerate() {
                let field = format!("tokenColors[{i}]");
                if let Some(r) = parse_rule(rule, &field, path, &mut doc.diagnostics) {
                    doc.token_colors.push(r);
                }
            }
        }
        Some(Value::String(tm)) => diag(
            doc,
            "tokenColors",
            format!("引用的 tmTheme 文件 {tm:?} 暂不支持（R68），已忽略"),
        ),
        Some(_) => diag(doc, "tokenColors", "应为数组，已忽略".into()),
    }
    match value.get("semanticTokenColors") {
        None => {}
        Some(Value::Object(rules)) => {
            for (selector, v) in rules {
                let field = format!("semanticTokenColors[{selector:?}]");
                if let Some(r) =
                    parse_semantic_rule(selector, v, &field, path, &mut doc.diagnostics)
                {
                    doc.semantic_token_colors.push(r);
                }
            }
        }
        Some(_) => diag(doc, "semanticTokenColors", "应为对象，已忽略".into()),
    }
    Ok(())
}

fn push(diags: &mut Vec<Diagnostic>, file: &str, field: String, message: String) {
    diags.push(Diagnostic {
        file: file.to_owned(),
        field,
        message,
    });
}

/// 解析一条 TextMate 规则（`tokenColors` 元素或 `textMateRules` 元素）。
pub(crate) fn parse_rule(
    rule: &Value,
    field: &str,
    file: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<RawRule> {
    let Some(obj) = rule.as_object() else {
        push(diags, file, field.into(), "规则应为对象，已跳过".into());
        return None;
    };
    let scope = match obj.get("scope") {
        None => None,
        Some(Value::String(s)) => Some(RawScope::Pattern(s.clone())),
        Some(Value::Array(items)) => Some(RawScope::List(
            items
                .iter()
                .filter_map(|s| {
                    let s = s.as_str().map(str::to_owned);
                    if s.is_none() {
                        push(
                            diags,
                            file,
                            format!("{field}.scope"),
                            "选择器应为字符串，已跳过".into(),
                        );
                    }
                    s
                })
                .collect(),
        )),
        Some(v) => {
            push(
                diags,
                file,
                format!("{field}.scope"),
                format!("应为字符串或数组，实际为 {v}，已跳过该规则"),
            );
            return None;
        }
    };
    let Some(settings) = obj.get("settings").and_then(Value::as_object) else {
        push(
            diags,
            file,
            format!("{field}.settings"),
            "缺少 settings 对象，已跳过该规则".into(),
        );
        return None;
    };
    Some(parse_settings(
        scope,
        settings,
        &format!("{field}.settings"),
        file,
        diags,
    ))
}

pub(crate) fn parse_settings(
    scope: Option<RawScope>,
    settings: &Map<String, Value>,
    field: &str,
    file: &str,
    diags: &mut Vec<Diagnostic>,
) -> RawRule {
    let mut color = |key: &str| -> Option<String> {
        let v = settings.get(key)?;
        match v.as_str() {
            Some(s) if parse_hex(s).is_some() => Some(s.to_owned()),
            // 与 vscode-textmate 相同：空串视为未设置，不算错误。
            Some("") => None,
            _ => {
                push(
                    diags,
                    file,
                    format!("{field}.{key}"),
                    format!("非法颜色值 {v}，已忽略"),
                );
                None
            }
        }
    };
    let foreground = color("foreground");
    let background = color("background");
    RawRule {
        scope,
        font_style: settings
            .get("fontStyle")
            .and_then(Value::as_str)
            .map(str::to_owned),
        foreground,
        background,
    }
}

fn parse_semantic_rule(
    selector: &str,
    v: &Value,
    field: &str,
    file: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<SemanticTokenRule> {
    let mut rule = SemanticTokenRule {
        selector: selector.to_owned(),
        foreground: None,
        font_style: None,
        bold: None,
        italic: None,
        underline: None,
        strikethrough: None,
    };
    let mut color = |s: &str| {
        let c = parse_hex(s);
        if c.is_none() {
            push(
                diags,
                file,
                field.into(),
                format!("非法颜色值 {s:?}，已忽略"),
            );
        }
        c
    };
    match v {
        Value::String(s) => rule.foreground = color(s),
        Value::Object(o) => {
            rule.foreground = o
                .get("foreground")
                .and_then(Value::as_str)
                .and_then(&mut color);
            rule.font_style = o
                .get("fontStyle")
                .and_then(Value::as_str)
                .map(str::to_owned);
            rule.bold = o.get("bold").and_then(Value::as_bool);
            rule.italic = o.get("italic").and_then(Value::as_bool);
            rule.underline = o.get("underline").and_then(Value::as_bool);
            rule.strikethrough = o.get("strikethrough").and_then(Value::as_bool);
        }
        _ => {
            push(
                diags,
                file,
                field.into(),
                format!("应为颜色字符串或对象，实际为 {v}，已跳过"),
            );
            return None;
        }
    }
    Some(rule)
}

/// VS Code `themeCompatibility.convertSettings`：旧式 tmTheme `settings` 数组，
/// 无 scope 的规则中的编辑器颜色映射到颜色键。
fn convert_settings(file: &str, settings: &[Value], doc: &mut ThemeDocument) {
    const MAPPING: &[(&str, &[&str])] = &[
        ("background", &["editor.background"]),
        ("foreground", &["editor.foreground"]),
        ("selection", &["editor.selectionBackground"]),
        ("inactiveSelection", &["editor.inactiveSelectionBackground"]),
        (
            "selectionHighlightColor",
            &["editor.selectionHighlightBackground"],
        ),
        (
            "findMatchHighlight",
            &[
                "editor.findMatchHighlightBackground",
                "peekViewResult.matchHighlightBackground",
            ],
        ),
        ("currentFindMatchHighlight", &["editor.findMatchBackground"]),
        ("hoverHighlight", &["editor.hoverHighlightBackground"]),
        ("wordHighlight", &["editor.wordHighlightBackground"]),
        (
            "wordHighlightStrong",
            &["editor.wordHighlightStrongBackground"],
        ),
        (
            "findRangeHighlight",
            &["editor.findRangeHighlightBackground"],
        ),
        (
            "referenceHighlight",
            &["peekViewEditor.matchHighlightBackground"],
        ),
        ("lineHighlight", &["editor.lineHighlightBackground"]),
        ("rangeHighlight", &["editor.rangeHighlightBackground"]),
        ("caret", &["editorCursor.foreground"]),
        ("invisibles", &["editorWhitespace.foreground"]),
        ("guide", &["editorIndentGuide.background1"]),
        ("activeGuide", &["editorIndentGuide.activeBackground1"]),
    ];
    for (i, rule) in settings.iter().enumerate() {
        let field = format!("settings[{i}]");
        let Some(r) = parse_rule(rule, &field, file, &mut doc.diagnostics) else {
            continue;
        };
        if r.scope.is_none() {
            if let Some(s) = rule.get("settings").and_then(Value::as_object) {
                for (key, ids) in MAPPING {
                    if let Some(c) = s.get(*key).and_then(Value::as_str).and_then(parse_hex) {
                        for id in *ids {
                            doc.colors.insert((*id).to_owned(), c);
                        }
                    }
                }
            }
        }
        doc.token_colors.push(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Mem(HashMap<&'static str, &'static str>);
    impl ThemeSource for Mem {
        fn read(&self, path: &str) -> Result<Cow<'static, str>> {
            self.0
                .get(path)
                .map(|s| Cow::Borrowed(*s))
                .ok_or(Error::NotFound)
        }
    }

    #[test]
    fn include_chain_merges_in_vscode_order() {
        let src = Mem(HashMap::from([
            (
                "themes/child.json",
                r##"{ "include": "./base/parent.json", "type": "dark",
                   "colors": { "a": "#111111", "b": "default" },
                   "tokenColors": [{ "scope": "x", "settings": { "foreground": "#222222" } }] }"##,
            ),
            (
                "themes/base/parent.json",
                r##"{ "include": "../../grand.json", "colors": { "a": "#999999", "b": "#888888", "c": "#777777" },
                   "tokenColors": [{ "scope": "x", "settings": { "foreground": "#333333" } }],
                   "semanticHighlighting": true }"##,
            ),
            (
                "grand.json",
                r##"{ "type": "light", "colors": { "d": "#666666" } }"##,
            ),
        ]));
        let doc = load(&src, "themes/child.json").unwrap();
        assert!(
            matches!(doc.kind, Some(ThemeKind::Dark)),
            "入口文件的 type 优先"
        );
        assert_eq!(doc.colors["a"], Color(0x11, 0x11, 0x11, 255));
        assert!(
            !doc.colors.contains_key("b"),
            "\"default\" 删除被包含文件的值"
        );
        assert_eq!(doc.colors["c"], Color(0x77, 0x77, 0x77, 255));
        assert_eq!(doc.colors["d"], Color(0x66, 0x66, 0x66, 255));
        let fg: Vec<_> = doc
            .token_colors
            .iter()
            .map(|r| r.foreground.clone().unwrap())
            .collect();
        assert_eq!(fg, ["#333333", "#222222"], "被包含文件的规则在前");
        assert!(doc.semantic_highlighting);
        assert!(doc.diagnostics.is_empty(), "{:?}", doc.diagnostics);
    }

    #[test]
    fn include_cycles_and_missing_files_are_diagnosed_not_fatal() {
        let src = Mem(HashMap::from([
            (
                "a.json",
                r##"{ "include": "b.json", "colors": { "x": "#010101" } }"##,
            ),
            (
                "b.json",
                r##"{ "include": "./a.json", "colors": { "y": "#020202" } }"##,
            ),
            (
                "c.json",
                r##"{ "include": "missing.json", "colors": { "z": "#030303" } }"##,
            ),
        ]));
        let doc = load(&src, "a.json").unwrap();
        assert_eq!(doc.colors.len(), 2);
        assert_eq!(doc.diagnostics.len(), 1);
        assert!(
            doc.diagnostics[0].message.contains("循环"),
            "{}",
            doc.diagnostics[0]
        );
        let doc = load(&src, "c.json").unwrap();
        assert_eq!(doc.colors.len(), 1);
        assert_eq!(doc.diagnostics[0].file, "missing.json");
        assert!(load(&src, "nope.json").is_err());
    }

    #[test]
    fn unknown_fields_and_bad_values_skip_only_the_bad_part() {
        let src = Mem(HashMap::from([(
            "t.json",
            r##"{ "$schema": "vscode://schemas/color-theme", "futureField": { "x": 1 },
                 // JSONC 注释与尾逗号
                 "colors": { "editor.background": "#zzzzzz", "editor.foreground": 12, "ok": "#abc", },
                 "tokenColors": [
                    "not an object",
                    { "scope": "a", "settings": { "foreground": "red", "fontStyle": "bold" } },
                    { "scope": "b" }
                 ],
                 "semanticTokenColors": { "class": "#nothex", "enum": { "foreground": "#123456", "bold": true } } }"##,
        )]));
        let doc = load(&src, "t.json").unwrap();
        assert_eq!(doc.colors.len(), 1);
        assert_eq!(doc.colors["ok"], Color(0xaa, 0xbb, 0xcc, 255));
        assert_eq!(doc.token_colors.len(), 1);
        assert_eq!(doc.token_colors[0].foreground, None);
        assert_eq!(doc.token_colors[0].font_style.as_deref(), Some("bold"));
        assert_eq!(doc.semantic_token_colors.len(), 2);
        let fields: Vec<_> = doc.diagnostics.iter().map(|d| d.to_string()).collect();
        for want in [
            r##"t.json colors["editor.background"]: 非法颜色值 "#zzzzzz""##,
            r##"t.json colors["editor.foreground"]: 颜色值应为字符串"##,
            "t.json tokenColors[0]: 规则应为对象",
            "t.json tokenColors[1].settings.foreground: 非法颜色值",
            "t.json tokenColors[2].settings: 缺少 settings",
            r##"t.json semanticTokenColors["class"]: 非法颜色值"##,
        ] {
            assert!(
                fields.iter().any(|f| f.starts_with(want)),
                "缺少诊断 {want}：{fields:#?}"
            );
        }
    }

    #[test]
    fn legacy_settings_array_maps_editor_colors() {
        let src = Mem(HashMap::from([(
            "old.json",
            r##"{ "settings": [ { "settings": { "background": "#101010", "foreground": "#eeeeee", "caret": "#ff0000" } },
                                { "scope": "comment", "settings": { "foreground": "#00ff00" } } ] }"##,
        )]));
        let doc = load(&src, "old.json").unwrap();
        assert_eq!(
            doc.colors["editor.background"],
            Color(0x10, 0x10, 0x10, 255)
        );
        assert_eq!(doc.colors["editorCursor.foreground"], Color(255, 0, 0, 255));
        assert_eq!(doc.token_colors.len(), 2);
    }

    #[test]
    fn paths_normalize() {
        assert_eq!(join("a/b/c.json", "./d.json"), "a/b/d.json");
        assert_eq!(join("a/b/c.json", "../d.json"), "a/d.json");
        assert_eq!(join("c.json", "./d.json"), "d.json");
    }
}
