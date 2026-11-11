//! 一份已经解析好的主题：主题文件 + 用户覆盖 + 默认值表，按 VS Code `ColorThemeData` 的语义合并。
//!
//! - 颜色：`workbench.colorCustomizations`（按主题形式覆盖全局形式）> 主题 `colors` > 默认值表；
//!   覆盖值 `"default"` 表示忽略主题值、回到默认值。
//! - token：首条规则是默认样式（`editor.foreground`/`editor.background`），然后依次是主题
//!   `tokenColors`、`editor.tokenColorCustomizations`（简写分组 → `textMateRules` → 按主题形式），
//!   最后在主题没有 `token.info-token` 时补上 VS Code 的四条日志 token 默认规则。

use crate::color::{parse_hex, VsColor};
use crate::defaults::{self, ColorScope};
use crate::file::{parse_rule, parse_settings, Diagnostic, SemanticTokenRule, ThemeDocument};
use crate::textmate::{font_style, RawRule, RawScope, TextMateTheme};
use crate::{Color, ColorRegistry, ThemeKind, TokenStyle};
use rove_base::{Error, Result, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

/// `workbench.colorCustomizations` 中的一个值。
#[derive(Clone, Copy, Debug, PartialEq)]
enum Custom {
    Color(Color),
    /// `"default"`：忽略主题值。
    Default,
}

/// 用户设置中的主题相关部分（来自 #13 的设置接口）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Customizations {
    /// `workbench.colorCustomizations`
    pub colors: Option<Value>,
    /// `editor.tokenColorCustomizations`
    pub token_colors: Option<Value>,
}

/// `[Theme]`/`[*Theme*]`/`[Theme*]`/`[*Theme]` 是否命中当前主题（VS Code `isThemeScopeMatch`）。
fn theme_scope_matches(settings_id: &str, pattern: &str) -> bool {
    let first = pattern.starts_with('*');
    let last = pattern.ends_with('*') && !pattern.is_empty();
    pattern == settings_id
        || (first
            && last
            && pattern.len() >= 2
            && settings_id.contains(&pattern[1..pattern.len() - 1]))
        || (last && settings_id.starts_with(&pattern[..pattern.len() - 1]))
        || (first && settings_id.ends_with(&pattern[1..]))
}

fn is_theme_scope(key: &str) -> bool {
    key.starts_with('[') && key.ends_with(']')
}

/// VS Code `getThemeSpecificColors`：合并所有命中当前主题的 `[...]` 段；数组拼接，其余后者覆盖。
fn theme_specific(settings_id: &str, value: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let mut out: BTreeMap<String, Value> = BTreeMap::new();
    for (key, scoped) in value {
        let Value::Object(scoped) = scoped else {
            continue;
        };
        if !is_theme_scope(key) {
            continue;
        }
        // 一个键可以列出多个主题：`[Dark+][Dark Modern]`。
        let hits = key
            .split('[')
            .filter_map(|s| {
                s.strip_suffix(']')
                    .or_else(|| s.split_once(']').map(|(a, _)| a))
            })
            .filter(|s| !s.is_empty())
            .filter(|id| theme_scope_matches(settings_id, id))
            .count();
        for _ in 0..hits {
            for (sub, v) in scoped.iter() {
                match (out.get(sub), v) {
                    (Some(Value::Array(a)), Value::Array(b)) => {
                        let joined: Vec<Value> = a.iter().chain(b.iter()).cloned().collect();
                        out.insert(sub.clone(), Value::Array(joined.into()));
                    }
                    (_, Value::Null) => {}
                    _ => {
                        out.insert(sub.clone(), v.clone());
                    }
                }
            }
        }
    }
    out
}

/// 把 `rove_base::Value` 转成 serde_json，复用主题文件的规则解析。
fn to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Number(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.to_string()),
        Value::Array(a) => serde_json::Value::Array(a.iter().map(to_json).collect()),
        Value::Object(o) => {
            serde_json::Value::Object(o.iter().map(|(k, v)| (k.clone(), to_json(v))).collect())
        }
    }
}

const SETTINGS_FILE: &str = "settings.json";

fn overwrite_custom_colors(
    obj: &BTreeMap<String, Value>,
    field: &str,
    out: &mut HashMap<String, Custom>,
    diags: &mut Vec<Diagnostic>,
) {
    for (id, v) in obj {
        if is_theme_scope(id) {
            continue;
        }
        match v {
            Value::String(s) if &**s == "default" => {
                out.insert(id.clone(), Custom::Default);
            }
            Value::String(s) => match parse_hex(s) {
                Some(c) => {
                    out.insert(id.clone(), Custom::Color(c));
                }
                None => diags.push(Diagnostic {
                    file: SETTINGS_FILE.into(),
                    field: format!("{field}[{id:?}]"),
                    message: format!("非法颜色值 {s:?}，已跳过该键"),
                }),
            },
            _ => {}
        }
    }
}

/// `tokenGroupToScopesMap`
const TOKEN_GROUPS: &[(&str, &[&str])] = &[
    ("comments", &["comment", "punctuation.definition.comment"]),
    ("strings", &["string", "meta.embedded.assembly"]),
    (
        "keywords",
        &[
            "keyword - keyword.operator",
            "keyword.control",
            "storage",
            "storage.type",
        ],
    ),
    ("numbers", &["constant.numeric"]),
    (
        "types",
        &[
            "entity.name.type",
            "entity.name.class",
            "support.type",
            "support.class",
        ],
    ),
    ("functions", &["entity.name.function", "support.function"]),
    ("variables", &["variable", "entity.name.variable"]),
];

/// VS Code `addCustomTokenColors`：先放简写分组，再放 `textMateRules`。
fn add_custom_token_colors(
    obj: &BTreeMap<String, Value>,
    field: &str,
    out: &mut Vec<RawRule>,
    diags: &mut Vec<Diagnostic>,
) {
    for (group, scopes) in TOKEN_GROUPS {
        let settings = match obj.get(*group) {
            None | Some(Value::Null) => continue,
            Some(Value::String(s)) if s.is_empty() => continue,
            Some(Value::String(s)) => serde_json::Map::from_iter([(
                "foreground".to_string(),
                serde_json::Value::String(s.to_string()),
            )]),
            Some(Value::Object(o)) => match to_json(&Value::Object(o.clone())) {
                serde_json::Value::Object(m) => m,
                _ => unreachable!(),
            },
            Some(_) => continue,
        };
        for scope in *scopes {
            let f = format!("{field}.{group}");
            out.push(parse_settings(
                Some(RawScope::Pattern((*scope).into())),
                &settings,
                &f,
                SETTINGS_FILE,
                diags,
            ));
        }
    }
    if let Some(Value::Array(rules)) = obj.get("textMateRules") {
        for (i, rule) in rules.iter().enumerate() {
            let f = format!("{field}.textMateRules[{i}]");
            if let Some(r) = parse_rule(&to_json(rule), &f, SETTINGS_FILE, diags) {
                if r.scope.is_some() {
                    out.push(r);
                }
            }
        }
    }
}

/// `defaultThemeColors`：日志类 token 的默认颜色。
fn default_token_rules(kind: ThemeKind) -> [RawRule; 4] {
    let (info, warn, error, debug) = match kind {
        ThemeKind::Light | ThemeKind::HighContrastLight => {
            ("#316bcd", "#cd9731", "#cd3131", "#800080")
        }
        ThemeKind::Dark => ("#6796e6", "#cd9731", "#f44747", "#b267e6"),
        ThemeKind::HighContrastDark => ("#6796e6", "#008000", "#FF0000", "#b267e6"),
    };
    [
        ("token.info-token", info),
        ("token.warn-token", warn),
        ("token.error-token", error),
        ("token.debug-token", debug),
    ]
    .map(|(scope, fg)| RawRule {
        scope: Some(RawScope::Pattern(scope.into())),
        foreground: Some(fg.into()),
        ..Default::default()
    })
}

/// 不可变的主题快照。`Send + Sync`；颜色查询 O(log 键数)，token 样式按 scope 路径缓存。
pub struct ResolvedTheme {
    id: Arc<str>,
    kind: ThemeKind,
    registry: Arc<ColorRegistry>,
    theme_colors: BTreeMap<String, Color>,
    custom: HashMap<String, Custom>,
    /// 已登记键的最终颜色（构建时一次算好，UI 线程只做查表）。
    resolved: BTreeMap<String, Option<Color>>,
    textmate: TextMateTheme,
    semantic_rules: Vec<SemanticTokenRule>,
    semantic_highlighting: bool,
    diagnostics: Vec<Diagnostic>,
    cache: RwLock<HashMap<Box<[Arc<str>]>, TokenStyle>>,
}

impl std::fmt::Debug for ResolvedTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedTheme")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// 缓存上限：超过后清空重来，防止异常输入让缓存无限增长。
const CACHE_LIMIT: usize = 16 * 1024;

impl ColorScope for ResolvedTheme {
    fn explicit(&self, id: &str) -> Option<VsColor> {
        match self.custom.get(id) {
            Some(Custom::Color(c)) => Some(VsColor::from_color(*c)),
            Some(Custom::Default) => None,
            None => self.theme_colors.get(id).copied().map(VsColor::from_color),
        }
    }
    fn kind(&self) -> ThemeKind {
        self.kind
    }
}

impl ResolvedTheme {
    /// 由主题文件内容与用户覆盖构建。`settings_id` 是 `workbench.colorTheme` 中的主题 ID，
    /// 用于匹配按主题形式的覆盖。
    pub fn build(
        settings_id: &str,
        kind: ThemeKind,
        doc: ThemeDocument,
        custom: &Customizations,
        registry: Arc<ColorRegistry>,
    ) -> Self {
        let mut diagnostics = doc.diagnostics;
        let mut custom_colors = HashMap::new();
        if let Some(Value::Object(obj)) = &custom.colors {
            overwrite_custom_colors(
                obj,
                "workbench.colorCustomizations",
                &mut custom_colors,
                &mut diagnostics,
            );
            let scoped = theme_specific(settings_id, obj);
            overwrite_custom_colors(
                &scoped,
                &format!("workbench.colorCustomizations[{settings_id:?}]"),
                &mut custom_colors,
                &mut diagnostics,
            );
        }
        let mut theme = Self {
            id: settings_id.into(),
            kind,
            registry,
            theme_colors: doc.colors,
            custom: custom_colors,
            resolved: BTreeMap::new(),
            textmate: TextMateTheme::from_rules(&[]),
            semantic_rules: doc.semantic_token_colors,
            semantic_highlighting: doc.semantic_highlighting,
            diagnostics: Vec::new(),
            cache: RwLock::new(HashMap::new()),
        };

        // token 规则
        let fg = defaults::color(&theme, "editor.foreground", 0).map(VsColor::to_color);
        let bg = defaults::color(&theme, "editor.background", 0).map(VsColor::to_color);
        let mut rules = vec![RawRule {
            scope: None,
            foreground: fg.map(crate::format_hex),
            background: bg.map(crate::format_hex),
            font_style: None,
        }];
        let has_scope = |r: &RawRule| match &r.scope {
            Some(RawScope::Pattern(p)) => !p.is_empty(),
            Some(RawScope::List(l)) => !l.is_empty(),
            None => false,
        };
        rules.extend(doc.token_colors.into_iter().filter(has_scope));
        let mut custom_rules = Vec::new();
        if let Some(Value::Object(obj)) = &custom.token_colors {
            add_custom_token_colors(
                obj,
                "editor.tokenColorCustomizations",
                &mut custom_rules,
                &mut diagnostics,
            );
            let scoped = theme_specific(settings_id, obj);
            add_custom_token_colors(
                &scoped,
                &format!("editor.tokenColorCustomizations[{settings_id:?}]"),
                &mut custom_rules,
                &mut diagnostics,
            );
            let sh = scoped
                .get("semanticHighlighting")
                .or_else(|| obj.get("semanticHighlighting"));
            if let Some(Value::Bool(b)) = sh {
                theme.semantic_highlighting = *b;
            }
        }
        rules.extend(custom_rules);
        let has_default_tokens = rules
            .iter()
            .any(|r| matches!(&r.scope, Some(RawScope::Pattern(p)) if p == "token.info-token"));
        if !has_default_tokens {
            rules.extend(default_token_rules(kind));
        }
        theme.textmate = TextMateTheme::from_rules(&rules);

        let resolved = theme
            .registry
            .entries()
            .map(|(id, _)| (id.to_owned(), theme.compute(id)))
            .collect();
        theme.resolved = resolved;
        theme.diagnostics = diagnostics;
        theme
    }

    fn compute(&self, id: &str) -> Option<Color> {
        if let Some(c) = self.explicit(id) {
            return Some(c.to_color());
        }
        if defaults::is_vscode_color(id) {
            return defaults::default_color(self, id, 0).map(VsColor::to_color);
        }
        // 不在 VS Code 默认值表中的键（例如 rove.* 自有键）：用登记时给出的默认值。
        self.registry.get(id).and_then(|d| d.for_kind(self.kind))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn kind(&self) -> ThemeKind {
        self.kind
    }

    /// 主题与用户设置加载过程中的诊断（文件、字段、原因）。
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn semantic_highlighting(&self) -> bool {
        self.semantic_highlighting
    }

    pub fn semantic_token_rules(&self) -> &[SemanticTokenRule] {
        &self.semantic_rules
    }

    /// 已登记颜色键的值；未登记返回 `NotFound`，明确未设色返回 `Ok(None)`。
    pub fn color(&self, id: &str) -> Result<Option<Color>> {
        self.resolved.get(id).copied().ok_or(Error::NotFound)
    }

    /// 主题（含用户覆盖）是否显式定义了这个键（VS Code `defines`）。
    pub fn defines(&self, id: &str) -> bool {
        self.explicit(id).is_some()
    }

    /// 不经缓存地解析 token 样式（基准与测试用）。
    pub fn token_style_uncached(&self, scopes: &[Arc<str>]) -> TokenStyle {
        let s = self.textmate.resolve(scopes);
        let bits = s.font_style.unwrap_or(0);
        TokenStyle {
            foreground: s.foreground,
            // 默认背景就是编辑器背景，不作为 token 背景返回，免得每个 token 都画背景。
            background: s
                .background
                .filter(|b| Some(*b) != self.textmate.defaults().background),
            bold: bits & font_style::BOLD != 0,
            italic: bits & font_style::ITALIC != 0,
            underline: bits & font_style::UNDERLINE != 0,
            strikethrough: bits & font_style::STRIKETHROUGH != 0,
        }
    }

    /// 按 scope 路径（由外到内）解析 token 样式。前景色总是已设置（无匹配时为默认前景色）。
    pub fn token_style(&self, scopes: &[Arc<str>]) -> TokenStyle {
        if let Some(s) = self.cache.read().expect("cache lock").get(scopes) {
            return s.clone();
        }
        let style = self.token_style_uncached(scopes);
        let mut cache = self.cache.write().expect("cache lock");
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(scopes.into(), style.clone());
        style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_scope_patterns() {
        for (pat, hit) in [
            ("Dark 2026", true),
            ("Dark", false),
            ("*2026", true),
            ("Dark*", true),
            ("*rk 20*", true),
            ("Light*", false),
        ] {
            assert_eq!(theme_scope_matches("Dark 2026", pat), hit, "{pat}");
        }
    }
}
