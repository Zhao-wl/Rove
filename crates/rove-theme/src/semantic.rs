//! 语义 token 的分类、选择器与回退表（#59）。
//!
//! 这一层回答的是「一个语义 token（类型 + 修饰符）该用什么颜色」，与它从哪来无关：
//!
//! 1. 主题的 `semanticTokenColors` 与用户的 `editor.semanticTokenColorCustomizations`
//!    按选择器匹配，逐属性取分数最高的一条（VS Code `ColorThemeData.getTokenStyle`）；
//! 2. 还有属性没定下来时，按**回退表**把语义 token 映射到一组 TextMate scope，
//!    再走 #10 已有的 `tokenColors` 匹配（VS Code `ColorThemeData.resolveScopes`）。
//!
//! 类型集合、修饰符集合、选择器语法、匹配打分与回退表**逐条移植自** VS Code
//! `src/vs/platform/theme/common/tokenClassificationRegistry.ts`（MIT，来源见
//! `THIRD_PARTY_NOTICES.md`）。不要凭直觉改这里的数据：主题作者写的规则是按 VS Code 的
//! 语义生效的，改一条就会让某些主题对不上。

use crate::Color;

/// 选择器里表示「任意类型」的通配符。
pub const TOKEN_TYPE_WILDCARD: &str = "*";

/// 标准语义 token 类型（`createDefaultTokenClassificationRegistry` 的登记顺序）。
/// `member` 已被 VS Code 标为废弃（用 `method` 代替），保留是为了认识老主题里的规则。
pub const TOKEN_TYPES: &[&str] = &[
    "comment",
    "string",
    "keyword",
    "number",
    "regexp",
    "operator",
    "namespace",
    "type",
    "struct",
    "class",
    "interface",
    "enum",
    "typeParameter",
    "function",
    "member",
    "method",
    "macro",
    "variable",
    "parameter",
    "property",
    "enumMember",
    "event",
    "decorator",
    "label",
];

/// 标准语义 token 修饰符。前八个由 `tokenClassificationRegistry` 登记，
/// `defaultLibrary` 与 `definition` 来自 LSP 的标准集合（VS Code 在语义 token 的
/// 扩展点上登记，匹配时一视同仁地按名字比较）。
pub const TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "documentation",
    "static",
    "abstract",
    "deprecated",
    "modification",
    "async",
    "readonly",
    "definition",
    "defaultLibrary",
];

/// 类型的父类型。VS Code 的默认登记里只有一条：`member` 的父类型是 `method`。
pub fn super_type(id: &str) -> Option<&'static str> {
    (id == "member").then_some("method")
}

/// `getTypeHierarchy`：`[自身, 父, 祖父, …]`。
pub fn type_hierarchy(id: &str) -> Vec<&str> {
    let mut out = vec![id];
    let mut cur = super_type(id);
    while let Some(t) = cur {
        out.push(t);
        cur = super_type(t);
    }
    out
}

/// 一条语义选择器：`[类型|*](.修饰符)*(:语言)?`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    pub token_type: String,
    /// 全部修饰符都必须出现，token 才算命中。
    pub modifiers: Vec<String>,
    pub language: Option<String>,
}

/// `parseClassifierString`：从尾部往前扫，`:` 之后是语言，`.` 之后是修饰符。
pub fn parse_classifier(s: &str) -> (String, Vec<String>, Option<String>) {
    let bytes = s.as_bytes();
    let mut k = s.len();
    let mut language = None;
    let mut modifiers = Vec::new();
    for i in (0..s.len()).rev() {
        let ch = bytes[i];
        if ch == b':' || ch == b'.' {
            let segment = &s[i + 1..k];
            k = i;
            if ch == b':' {
                language = Some(segment.to_owned());
            } else {
                modifiers.push(segment.to_owned());
            }
        }
    }
    (s[..k].to_owned(), modifiers, language)
}

impl Selector {
    /// 解析一条选择器。类型为空（`".readonly"` 这种）时 VS Code 得到 `$invalid`，
    /// 永远不匹配；这里用 `None` 表示。
    pub fn parse(s: &str) -> Option<Self> {
        let (token_type, modifiers, language) = parse_classifier(s);
        (!token_type.is_empty()).then_some(Self {
            token_type,
            modifiers,
            language,
        })
    }

    /// `TokenSelector.match`：不匹配返回 `None`，匹配返回分数（越大越优先）。
    pub fn score(&self, token_type: &str, modifiers: &[&str], language: &str) -> Option<i32> {
        let mut score = 0;
        if let Some(l) = &self.language {
            if l != language {
                return None;
            }
            score += 10;
        }
        if self.token_type != TOKEN_TYPE_WILDCARD {
            let hierarchy = type_hierarchy(token_type);
            let level = hierarchy.iter().position(|t| *t == self.token_type)?;
            score += 100 - level as i32;
        }
        for m in &self.modifiers {
            if !modifiers.iter().any(|x| x == m) {
                return None;
            }
        }
        Some(score + self.modifiers.len() as i32 * 100)
    }

    /// `TokenSelector.id`：规范化后的选择器文本（修饰符排序）。
    pub fn id(&self) -> String {
        let mut mods = self.modifiers.clone();
        mods.sort();
        let mut out = self.token_type.clone();
        for m in mods {
            out.push('.');
            out.push_str(&m);
        }
        if let Some(l) = &self.language {
            out.push(':');
            out.push_str(l);
        }
        out
    }
}

/// 一条语义样式。全部字段都是「未设置」可表达的，叠加时只覆盖已设置的属性。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SemanticStyle {
    pub foreground: Option<Color>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
}

impl SemanticStyle {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// 回退表的一条：选择器 + 若干组待探测的 scope 路径（按顺序试，第一组命中就用它）。
pub struct FallbackRule {
    pub selector: &'static str,
    pub probes: &'static [&'static [&'static str]],
}

/// **回退表**：语义 token → TextMate scope。逐条移植自
/// `createDefaultTokenClassificationRegistry()`，顺序也与那里一致
/// （同分时后登记的覆盖先登记的，所以顺序有意义）。
pub const FALLBACK_RULES: &[FallbackRule] = &[
    // 类型登记时顺带登记的默认 scope（`registerTokenType` 的 `scopesToProbe`）。
    FallbackRule {
        selector: "comment",
        probes: &[&["comment"]],
    },
    FallbackRule {
        selector: "string",
        probes: &[&["string"]],
    },
    FallbackRule {
        selector: "keyword",
        probes: &[&["keyword.control"]],
    },
    FallbackRule {
        selector: "number",
        probes: &[&["constant.numeric"]],
    },
    FallbackRule {
        selector: "regexp",
        probes: &[&["constant.regexp"]],
    },
    FallbackRule {
        selector: "operator",
        probes: &[&["keyword.operator"]],
    },
    FallbackRule {
        selector: "namespace",
        probes: &[&["entity.name.namespace"]],
    },
    FallbackRule {
        selector: "type",
        probes: &[&["entity.name.type"], &["support.type"]],
    },
    FallbackRule {
        selector: "struct",
        probes: &[&["entity.name.type.struct"]],
    },
    FallbackRule {
        selector: "class",
        probes: &[&["entity.name.type.class"], &["support.class"]],
    },
    FallbackRule {
        selector: "interface",
        probes: &[&["entity.name.type.interface"]],
    },
    FallbackRule {
        selector: "enum",
        probes: &[&["entity.name.type.enum"]],
    },
    FallbackRule {
        selector: "typeParameter",
        probes: &[&["entity.name.type.parameter"]],
    },
    FallbackRule {
        selector: "function",
        probes: &[&["entity.name.function"], &["support.function"]],
    },
    // `member` 登记时给的是空数组：规则存在但探测不到任何 scope（VS Code 行为一致）。
    FallbackRule {
        selector: "member",
        probes: &[],
    },
    FallbackRule {
        selector: "method",
        probes: &[&["entity.name.function.member"], &["support.function"]],
    },
    FallbackRule {
        selector: "macro",
        probes: &[&["entity.name.function.preprocessor"]],
    },
    FallbackRule {
        selector: "variable",
        probes: &[&["variable.other.readwrite"], &["entity.name.variable"]],
    },
    FallbackRule {
        selector: "parameter",
        probes: &[&["variable.parameter"]],
    },
    FallbackRule {
        selector: "property",
        probes: &[&["variable.other.property"]],
    },
    FallbackRule {
        selector: "enumMember",
        probes: &[&["variable.other.enummember"]],
    },
    FallbackRule {
        selector: "event",
        probes: &[&["variable.other.event"]],
    },
    FallbackRule {
        selector: "decorator",
        probes: &[&["entity.name.decorator"], &["entity.name.function"]],
    },
    FallbackRule {
        selector: "label",
        probes: &[],
    },
    // 带修饰符的默认规则（`registerTokenStyleDefault` 的那一组）。
    FallbackRule {
        selector: "variable.readonly",
        probes: &[&["variable.other.constant"]],
    },
    FallbackRule {
        selector: "property.readonly",
        probes: &[&["variable.other.constant.property"]],
    },
    FallbackRule {
        selector: "type.defaultLibrary",
        probes: &[&["support.type"]],
    },
    FallbackRule {
        selector: "class.defaultLibrary",
        probes: &[&["support.class"]],
    },
    FallbackRule {
        selector: "interface.defaultLibrary",
        probes: &[&["support.class"]],
    },
    FallbackRule {
        selector: "variable.defaultLibrary",
        probes: &[&["support.variable"], &["support.other.variable"]],
    },
    FallbackRule {
        selector: "variable.defaultLibrary.readonly",
        probes: &[&["support.constant"]],
    },
    FallbackRule {
        selector: "property.defaultLibrary",
        probes: &[&["support.variable.property"]],
    },
    FallbackRule {
        selector: "property.defaultLibrary.readonly",
        probes: &[&["support.constant.property"]],
    },
    FallbackRule {
        selector: "function.defaultLibrary",
        probes: &[&["support.function"]],
    },
    FallbackRule {
        selector: "member.defaultLibrary",
        probes: &[&["support.function"]],
    },
];

/// 逐属性记分的叠加器（`getTokenStyle` 里的 `_processStyle`）。
#[derive(Default)]
pub(crate) struct StyleAccumulator {
    pub(crate) style: SemanticStyle,
    scores: [i32; 5],
    seen: [bool; 5],
}

impl StyleAccumulator {
    pub(crate) fn new() -> Self {
        Self {
            style: SemanticStyle::default(),
            scores: [-1; 5],
            seen: [false; 5],
        }
    }

    pub(crate) fn process(&mut self, score: i32, style: &SemanticStyle) {
        if style.foreground.is_some() && self.scores[0] <= score {
            self.scores[0] = score;
            self.seen[0] = true;
            self.style.foreground = style.foreground;
        }
        let props: [(usize, Option<bool>); 4] = [
            (1, style.bold),
            (2, style.underline),
            (3, style.strikethrough),
            (4, style.italic),
        ];
        for (i, value) in props {
            if let Some(v) = value {
                if self.scores[i] <= score {
                    self.scores[i] = score;
                    self.seen[i] = true;
                    match i {
                        1 => self.style.bold = Some(v),
                        2 => self.style.underline = Some(v),
                        3 => self.style.strikethrough = Some(v),
                        _ => self.style.italic = Some(v),
                    }
                }
            }
        }
    }

    /// 还有属性没定下来吗（决定要不要走回退表）。
    pub(crate) fn incomplete(&self) -> bool {
        self.seen.iter().any(|s| !s)
    }

    /// 已定下来的属性锁死（`score = MAX`），回退表不能再覆盖它们。
    pub(crate) fn lock(&mut self) {
        for i in 0..5 {
            if self.seen[i] {
                self.scores[i] = i32::MAX;
            }
        }
    }
}

/// `TokenStyle.fromSettings`：给了 `fontStyle` 字符串时，四个布尔属性全部由它决定
/// （显式的 `bold` / `italic` 等字段被覆盖）。
pub fn style_from_settings(
    foreground: Option<Color>,
    font_style: Option<&str>,
    bold: Option<bool>,
    underline: Option<bool>,
    strikethrough: Option<bool>,
    italic: Option<bool>,
) -> SemanticStyle {
    match font_style {
        Some(fs) => {
            let has = |name: &str| fs.split_whitespace().any(|w| w == name);
            SemanticStyle {
                foreground,
                bold: Some(has("bold")),
                italic: Some(has("italic")),
                underline: Some(has("underline")),
                strikethrough: Some(has("strikethrough")),
            }
        }
        None => SemanticStyle {
            foreground,
            bold,
            italic,
            underline,
            strikethrough,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_strings_parse_like_vscode() {
        assert_eq!(
            parse_classifier("variable.readonly:csharp"),
            (
                "variable".into(),
                vec!["readonly".to_owned()],
                Some("csharp".into())
            )
        );
        assert_eq!(
            parse_classifier("*.declaration"),
            ("*".into(), vec!["declaration".to_owned()], None)
        );
        assert_eq!(parse_classifier("class"), ("class".into(), vec![], None));
        // 类型为空的选择器是非法的，永远不匹配。
        assert!(Selector::parse(".readonly").is_none());
    }

    #[test]
    fn selector_scores_follow_vscode() {
        let exact = Selector::parse("variable").unwrap();
        let with_mod = Selector::parse("variable.readonly").unwrap();
        let wildcard = Selector::parse("*.readonly").unwrap();
        let language = Selector::parse("variable:csharp").unwrap();
        assert_eq!(exact.score("variable", &["readonly"], "csharp"), Some(100));
        assert_eq!(
            with_mod.score("variable", &["readonly"], "csharp"),
            Some(200)
        );
        assert_eq!(
            wildcard.score("variable", &["readonly"], "csharp"),
            Some(100)
        );
        assert_eq!(language.score("variable", &[], "csharp"), Some(110));
        assert_eq!(language.score("variable", &[], "java"), None);
        // 修饰符没全出现就不匹配。
        assert_eq!(with_mod.score("variable", &[], "csharp"), None);
        // 类型不同也不匹配。
        assert_eq!(exact.score("property", &[], "csharp"), None);
    }

    #[test]
    fn member_inherits_method_through_the_type_hierarchy() {
        assert_eq!(type_hierarchy("member"), vec!["member", "method"]);
        let method = Selector::parse("method").unwrap();
        // `method` 选中 `member`，但分数低一级（level 1）。
        assert_eq!(method.score("member", &[], "csharp"), Some(99));
        assert_eq!(method.score("method", &[], "csharp"), Some(100));
        // 反过来不成立。
        assert_eq!(
            Selector::parse("member").unwrap().score("method", &[], "x"),
            None
        );
    }

    #[test]
    fn selector_id_sorts_modifiers() {
        assert_eq!(
            Selector::parse("variable.static.readonly:csharp")
                .unwrap()
                .id(),
            "variable.readonly.static:csharp"
        );
    }

    #[test]
    fn font_style_string_overrides_explicit_booleans() {
        let s = style_from_settings(None, Some("italic"), Some(true), None, None, None);
        assert_eq!(s.bold, Some(false));
        assert_eq!(s.italic, Some(true));
        let s = style_from_settings(None, None, Some(true), None, None, None);
        assert_eq!(s.bold, Some(true));
        assert_eq!(s.italic, None);
    }

    #[test]
    fn fallback_table_matches_the_vscode_registry() {
        // 条数与 VS Code 的 `tokenStylingDefaultRules` 一致：24 个类型 + 11 条带修饰符的规则。
        assert_eq!(FALLBACK_RULES.len(), TOKEN_TYPES.len() + 11);
        for rule in FALLBACK_RULES {
            assert!(
                Selector::parse(rule.selector).is_some(),
                "{} 应能解析",
                rule.selector
            );
        }
        // 抽查几条关键映射（PR 里有完整对照表）。
        let find = |s: &str| {
            FALLBACK_RULES
                .iter()
                .find(|r| r.selector == s)
                .map(|r| r.probes)
        };
        assert_eq!(
            find("class"),
            Some(&[&["entity.name.type.class"][..], &["support.class"][..]][..])
        );
        assert_eq!(
            find("variable.readonly"),
            Some(&[&["variable.other.constant"][..]][..])
        );
        assert_eq!(
            find("enumMember"),
            Some(&[&["variable.other.enummember"][..]][..])
        );
    }
}
