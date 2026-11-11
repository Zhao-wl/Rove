//! TextMate 主题匹配：移植自 `vscode-textmate` 9.3.2 的 `src/theme.ts`（MIT，见 THIRD_PARTY_NOTICES.md）。
//!
//! 与原实现保持同样的语义：
//! - 选择器按空格分成「父 scope … 目标 scope」，逗号或数组表示多个选择器；`>` 是子代组合符；
//! - 规则先按目标 scope 字典序、父 scope 数组、定义顺序排序后插入前缀树，
//!   同一节点的同名规则合并（后定义者覆盖已设置的字段，即「同分时后定义者胜」）；
//! - 匹配时取目标 scope 在前缀树上最深的节点，候选规则按特异性排序
//!   （目标 scope 段数 → 父 scope 由深到浅逐个比较长度 → 父 scope 个数），取第一个父 scope 也匹配的；
//! - `fontStyle` 为字符串时先清空再按词设置，所以 `""` 表示显式清空（区别于未设置）。
//!
//! 不移植 `fontFamily`/`fontSize`/`lineHeight`（Rove 的 token 样式没有这些字段）以及颜色表 ID：
//! 直接保存颜色值，语义等价。

use crate::color::{is_valid_hex, parse_hex};
use crate::Color;
use std::collections::HashMap;
use std::sync::Arc;

/// 字体样式位，与 vscode-textmate 的 `FontStyle` 相同；`None` 表示未设置（`NotSet = -1`）。
pub mod font_style {
    pub const ITALIC: u8 = 1;
    pub const BOLD: u8 = 2;
    pub const UNDERLINE: u8 = 4;
    pub const STRIKETHROUGH: u8 = 8;
}

/// 主题中的一条 TextMate 规则（`tokenColors` 的元素），尚未解析选择器。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawRule {
    /// `None` 表示没有 `scope` 字段（默认规则）。
    pub scope: Option<RawScope>,
    pub font_style: Option<String>,
    pub foreground: Option<String>,
    pub background: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RawScope {
    /// 逗号分隔的选择器字符串。
    Pattern(String),
    /// 选择器数组。
    List(Vec<String>),
}

/// `parseTheme` 的输出：一条规则对应一个选择器。
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedRule {
    pub scope: Box<str>,
    /// 父 scope，已经反转为「由深到浅」；`None` 表示没有父 scope。
    pub parent_scopes: Option<Vec<Box<str>>>,
    pub index: usize,
    pub font_style: Option<u8>,
    pub foreground: Option<Color>,
    pub background: Option<Color>,
}

/// vscode-textmate `parseTheme`。非法颜色与非字符串字段忽略，与原实现一致。
pub fn parse_rules(rules: &[RawRule]) -> Vec<ParsedRule> {
    let mut out = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        let scopes: Vec<String> = match &rule.scope {
            Some(RawScope::Pattern(p)) => p
                .trim_start_matches(',')
                .trim_end_matches(',')
                .split(',')
                .map(str::to_owned)
                .collect(),
            Some(RawScope::List(l)) => l.clone(),
            None => vec![String::new()],
        };
        let font_style = rule.font_style.as_ref().map(|s| {
            s.split(' ').fold(0u8, |acc, seg| {
                acc | match seg {
                    "italic" => font_style::ITALIC,
                    "bold" => font_style::BOLD,
                    "underline" => font_style::UNDERLINE,
                    "strikethrough" => font_style::STRIKETHROUGH,
                    _ => 0,
                }
            })
        });
        let color =
            |c: &Option<String>| c.as_deref().filter(|c| is_valid_hex(c)).and_then(parse_hex);
        let (foreground, background) = (color(&rule.foreground), color(&rule.background));
        for scope in scopes {
            let segments: Vec<&str> = scope.trim().split(' ').collect();
            let (last, parents) = segments.split_last().expect("split yields one item");
            let parent_scopes = (!parents.is_empty())
                .then(|| parents.iter().rev().map(|s| Box::from(*s)).collect());
            out.push(ParsedRule {
                scope: Box::from(*last),
                parent_scopes,
                index,
                font_style,
                foreground,
                background,
            });
        }
    }
    out
}

/// 样式属性：`font_style` 为 `None` 表示未设置，颜色为 `None` 表示未设置。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleAttributes {
    pub font_style: Option<u8>,
    pub foreground: Option<Color>,
    pub background: Option<Color>,
}

#[derive(Clone, Debug, PartialEq)]
struct TrieRule {
    scope_depth: usize,
    parent_scopes: Arc<[Box<str>]>,
    font_style: Option<u8>,
    foreground: Option<Color>,
    background: Option<Color>,
}

impl TrieRule {
    fn accept_overwrite(
        &mut self,
        scope_depth: usize,
        font_style: Option<u8>,
        fg: Option<Color>,
        bg: Option<Color>,
    ) {
        self.scope_depth = self.scope_depth.max(scope_depth);
        if font_style.is_some() {
            self.font_style = font_style;
        }
        if fg.is_some() {
            self.foreground = fg;
        }
        if bg.is_some() {
            self.background = bg;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct TrieElement {
    main: TrieRule,
    with_parents: Vec<TrieRule>,
    children: HashMap<Box<str>, TrieElement>,
    /// `with_parents` 加 `main` 按特异性排好序；构建结束后由 `finalize` 填充，匹配时不再排序。
    candidates: Vec<TrieRule>,
}

fn split_head(scope: &str) -> (&str, &str) {
    scope.split_once('.').unwrap_or((scope, ""))
}

/// `strArrCmp`：`None` 小于任何数组；逐项字典序，短者在前。
fn str_arr_cmp(a: Option<&[Box<str>]>, b: Option<&[Box<str>]>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, _) => std::cmp::Ordering::Less,
        (_, None) => std::cmp::Ordering::Greater,
        (Some(a), Some(b)) => a.cmp(b),
    }
}

/// `_cmpBySpecificity`：越特异越靠前。
fn cmp_by_specificity(a: &TrieRule, b: &TrieRule) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if a.scope_depth != b.scope_depth {
        return b.scope_depth.cmp(&a.scope_depth);
    }
    let (mut ai, mut bi) = (0, 0);
    loop {
        if a.parent_scopes.get(ai).is_some_and(|s| &**s == ">") {
            ai += 1;
        }
        if b.parent_scopes.get(bi).is_some_and(|s| &**s == ">") {
            bi += 1;
        }
        if ai >= a.parent_scopes.len() || bi >= b.parent_scopes.len() {
            break;
        }
        let diff = b.parent_scopes[bi].len().cmp(&a.parent_scopes[ai].len());
        if diff != Equal {
            return diff;
        }
        ai += 1;
        bi += 1;
    }
    b.parent_scopes.len().cmp(&a.parent_scopes.len())
}

impl TrieElement {
    fn new(main: TrieRule, with_parents: Vec<TrieRule>) -> Self {
        Self {
            main,
            with_parents,
            children: HashMap::new(),
            candidates: Vec::new(),
        }
    }

    fn finalize(&mut self) {
        let mut rules = self.with_parents.clone();
        rules.push(self.main.clone());
        rules.sort_by(cmp_by_specificity);
        self.candidates = rules;
        for child in self.children.values_mut() {
            child.finalize();
        }
    }

    fn find(&self, scope: &str) -> &Self {
        let mut node = self;
        let mut rest = scope;
        while !rest.is_empty() {
            let (head, tail) = split_head(rest);
            match node.children.get(head) {
                Some(child) => node = child,
                None => break,
            }
            rest = tail;
        }
        node
    }

    fn insert(&mut self, depth: usize, scope: &str, rule: &ParsedRule) {
        if scope.is_empty() {
            return self.insert_here(depth, rule);
        }
        let (head, tail) = split_head(scope);
        if !self.children.contains_key(head) {
            let child = TrieElement::new(self.main.clone(), self.with_parents.clone());
            self.children.insert(Box::from(head), child);
        }
        self.children
            .get_mut(head)
            .expect("inserted above")
            .insert(depth + 1, tail, rule);
    }

    fn insert_here(&mut self, depth: usize, rule: &ParsedRule) {
        let (fs, fg, bg) = (rule.font_style, rule.foreground, rule.background);
        let Some(parents) = &rule.parent_scopes else {
            return self.main.accept_overwrite(depth, fs, fg, bg);
        };
        if let Some(existing) = self
            .with_parents
            .iter_mut()
            .find(|r| *r.parent_scopes == **parents)
        {
            return existing.accept_overwrite(depth, fs, fg, bg);
        }
        self.with_parents.push(TrieRule {
            scope_depth: depth,
            parent_scopes: parents.clone().into(),
            font_style: fs.or(self.main.font_style),
            foreground: fg.or(self.main.foreground),
            background: bg.or(self.main.background),
        });
    }
}

fn matches_scope(scope: &str, pattern: &str) -> bool {
    scope == pattern
        || (scope.starts_with(pattern) && scope.as_bytes().get(pattern.len()) == Some(&b'.'))
}

/// `_scopePathMatchesParentScopes`：`path` 为父 scope 路径（由浅到深），`parents` 由深到浅。
fn path_matches_parents<S: AsRef<str>>(path: &[S], parents: &[Box<str>]) -> bool {
    let mut pos = path.len(); // path[..pos] 是尚未消费的部分
    let mut i = 0;
    while i < parents.len() {
        let mut pattern = &*parents[i];
        let mut must_match = false;
        if pattern == ">" {
            if i == parents.len() - 1 {
                return false;
            }
            i += 1;
            pattern = &parents[i];
            must_match = true;
        }
        loop {
            if pos == 0 {
                return false;
            }
            if matches_scope(path[pos - 1].as_ref(), pattern) {
                break;
            }
            if must_match {
                return false;
            }
            pos -= 1;
        }
        pos -= 1;
        i += 1;
    }
    true
}

/// 解析后的 TextMate 主题（不可变，`Send + Sync`）。
#[derive(Clone, Debug, PartialEq)]
pub struct TextMateTheme {
    defaults: StyleAttributes,
    root: TrieElement,
}

impl TextMateTheme {
    pub fn from_rules(rules: &[RawRule]) -> Self {
        Self::from_parsed(parse_rules(rules))
    }

    /// `resolveParsedThemeRules`。
    pub fn from_parsed(mut rules: Vec<ParsedRule>) -> Self {
        rules.sort_by(|a, b| {
            a.scope
                .cmp(&b.scope)
                .then_with(|| str_arr_cmp(a.parent_scopes.as_deref(), b.parent_scopes.as_deref()))
                .then_with(|| a.index.cmp(&b.index))
        });
        let mut defaults = StyleAttributes {
            font_style: Some(0),
            foreground: Some(Color(0, 0, 0, 255)),
            background: Some(Color(255, 255, 255, 255)),
        };
        let mut start = 0;
        while start < rules.len() && rules[start].scope.is_empty() {
            let d = &rules[start];
            defaults.font_style = d.font_style.or(defaults.font_style);
            defaults.foreground = d.foreground.or(defaults.foreground);
            defaults.background = d.background.or(defaults.background);
            start += 1;
        }
        let mut root = TrieElement::new(
            TrieRule {
                scope_depth: 0,
                parent_scopes: Arc::from([]),
                font_style: None,
                foreground: None,
                background: None,
            },
            vec![],
        );
        for rule in &rules[start..] {
            root.insert(0, &rule.scope, rule);
        }
        root.finalize();
        Self { defaults, root }
    }

    pub fn defaults(&self) -> StyleAttributes {
        self.defaults
    }

    /// `Theme.match`：按完整 scope 路径（由外到内）匹配最内层 scope；无匹配返回 `None`。
    pub fn match_path<S: AsRef<str>>(&self, path: &[S]) -> Option<StyleAttributes> {
        let Some((name, parents)) = path.split_last() else {
            return Some(self.defaults);
        };
        let node = self.root.find(name.as_ref());
        let rule = node
            .candidates
            .iter()
            .find(|r| path_matches_parents(parents, &r.parent_scopes))?;
        Some(StyleAttributes {
            font_style: rule.font_style,
            foreground: rule.foreground,
            background: rule.background,
        })
    }

    /// 与 vscode-textmate 分词器相同的逐层合并：从默认样式开始，依次用每一层前缀路径的匹配结果
    /// 覆盖已设置的字段。返回值的 `font_style`/`foreground`/`background` 总是已设置。
    pub fn resolve<S: AsRef<str>>(&self, scopes: &[S]) -> StyleAttributes {
        let mut style = self.defaults;
        for depth in 1..=scopes.len() {
            if let Some(m) = self.match_path(&scopes[..depth]) {
                style.font_style = m.font_style.or(style.font_style);
                style.foreground = m.foreground.or(style.foreground);
                style.background = m.background.or(style.background);
            }
        }
        style
    }
}

#[cfg(test)]
mod tests {
    //! 逐条移植自 vscode-textmate `src/tests/themes.test.ts` 中不依赖语法文件的用例。
    use super::*;

    fn rule(scope: Option<&str>, fs: Option<&str>, fg: Option<&str>, bg: Option<&str>) -> RawRule {
        RawRule {
            scope: scope.map(|s| RawScope::Pattern(s.into())),
            font_style: fs.map(Into::into),
            foreground: fg.map(Into::into),
            background: bg.map(Into::into),
        }
    }
    fn fg(scope: &str, color: &str) -> RawRule {
        rule(Some(scope), None, Some(color), None)
    }
    fn c(s: &str) -> Color {
        parse_hex(s).unwrap()
    }
    fn fg_of(theme: &TextMateTheme, path: &[&str]) -> Option<Color> {
        theme.match_path(path).and_then(|s| s.foreground)
    }

    #[test]
    fn gives_higher_priority_to_deeper_matches() {
        let t = TextMateTheme::from_rules(&[
            rule(None, None, Some("#100000"), Some("#200000")),
            fg("punctuation.definition.string.begin.html", "#300000"),
            fg("meta.tag punctuation.definition.string", "#400000"),
        ]);
        assert_eq!(
            fg_of(&t, &["punctuation.definition.string.begin.html"]),
            Some(c("#300000"))
        );
    }

    #[test]
    fn gives_higher_priority_to_parent_matches_1() {
        let t = TextMateTheme::from_rules(&[
            rule(None, None, Some("#100000"), Some("#200000")),
            fg("c a", "#300000"),
            fg("d a.b", "#400000"),
            fg("a", "#500000"),
        ]);
        assert_eq!(fg_of(&t, &["d", "a.b"]), Some(c("#400000")));
    }

    #[test]
    fn gives_higher_priority_to_parent_matches_2() {
        let t = TextMateTheme::from_rules(&[
            rule(None, None, Some("#100000"), Some("#200000")),
            fg("meta.tag entity", "#300000"),
            fg("meta.selector.css entity.name.tag", "#400000"),
            fg("entity", "#500000"),
        ]);
        let path = [
            "text.html.cshtml",
            "meta.tag.structure.any.html",
            "entity.name.tag.structure.any.html",
        ];
        assert_eq!(fg_of(&t, &path), Some(c("#300000")));
    }

    fn describe(
        t: &TextMateTheme,
        path: &[&str],
    ) -> Option<(String, Option<Color>, Option<Color>)> {
        let r = t.match_path(path)?;
        let fs = match r.font_style {
            None => "not set".to_string(),
            Some(0) => "none".to_string(),
            Some(bits) => [
                (font_style::ITALIC, "italic"),
                (font_style::BOLD, "bold"),
                (font_style::UNDERLINE, "underline"),
                (font_style::STRIKETHROUGH, "strikethrough"),
            ]
            .iter()
            .filter(|(b, _)| bits & b != 0)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(" "),
        };
        Some((fs, r.foreground, r.background))
    }

    #[test]
    fn can_match() {
        let t = TextMateTheme::from_rules(&[
            rule(None, None, Some("#F8F8F2"), Some("#272822")),
            rule(Some("source, something"), None, None, Some("#100000")),
            RawRule {
                scope: Some(RawScope::List(vec!["bar".into(), "baz".into()])),
                background: Some("#200000".into()),
                ..Default::default()
            },
            rule(Some("source.css selector bar"), Some("bold"), None, None),
            rule(Some("constant"), Some("italic"), Some("#300000"), None),
            fg("constant.numeric", "#400000"),
            rule(Some("constant.numeric.hex"), Some("bold"), None, None),
            rule(
                Some("constant.numeric.oct"),
                Some("bold italic underline"),
                None,
                None,
            ),
            rule(
                Some("constant.numeric.dec"),
                Some(""),
                Some("#500000"),
                None,
            ),
            rule(Some("storage.object.bar"), Some(""), Some("#600000"), None),
        ]);
        let bg = |s: &str| (String::from("not set"), None, Some(c(s)));
        let fgs = |fs: &str, s: &str| (String::from(fs), Some(c(s)), None);
        let unset = (String::from("not set"), None, None);
        for (path, want) in [
            (&["source"][..], bg("#100000")),
            (&["source.ts"], bg("#100000")),
            (&["source.tss"], bg("#100000")),
            (&["something"], bg("#100000")),
            (&["something.ts"], bg("#100000")),
            (&["something.tss"], bg("#100000")),
            (&["baz"], bg("#200000")),
            (&["baz.ts"], bg("#200000")),
            (&["baz.tss"], bg("#200000")),
            (&["constant"], fgs("italic", "#300000")),
            (&["constant.string"], fgs("italic", "#300000")),
            (&["constant.hex"], fgs("italic", "#300000")),
            (&["constant.numeric"], fgs("italic", "#400000")),
            (&["constant.numeric.baz"], fgs("italic", "#400000")),
            (&["constant.numeric.hex"], fgs("bold", "#400000")),
            (&["constant.numeric.hex.baz"], fgs("bold", "#400000")),
            (
                &["constant.numeric.oct"],
                fgs("italic bold underline", "#400000"),
            ),
            (
                &["constant.numeric.oct.baz"],
                fgs("italic bold underline", "#400000"),
            ),
            (&["constant.numeric.dec"], fgs("none", "#500000")),
            (&["constant.numeric.dec.baz"], fgs("none", "#500000")),
            (&["storage.object.bar"], fgs("none", "#600000")),
            (&["storage.object.bar.baz"], fgs("none", "#600000")),
            (&["storage.object.bart"], unset.clone()),
            (&["storage.object"], unset.clone()),
            (&["storage"], unset.clone()),
            (&[""], unset.clone()),
            (&["bazz"], unset.clone()),
            (&["asdfg"], unset.clone()),
            (&["bar"], bg("#200000")),
            (
                &["source.css", "selector", "bar"],
                (String::from("bold"), None, Some(c("#200000"))),
            ),
        ] {
            assert_eq!(describe(&t, path), Some(want), "{path:?}");
        }
    }

    #[test]
    fn microsoft_vscode_23460() {
        let t = TextMateTheme::from_rules(&[
            rule(None, None, Some("#aec2e0"), Some("#14191f")),
            fg(
                "meta.structure.dictionary.json string.quoted.double.json",
                "#FF410D",
            ),
            fg(
                "meta.structure.dictionary.json string.quoted.double.json",
                "#ffffff",
            ),
            fg(
                "meta.structure.dictionary.value.json string.quoted.double.json",
                "#FF410D",
            ),
        ]);
        let path = [
            "source.json",
            "meta.structure.dictionary.json",
            "meta.structure.dictionary.value.json",
            "string.quoted.double.json",
        ];
        assert_eq!(fg_of(&t, &path), Some(c("#FF410D")));
    }

    fn p(
        scope: &str,
        parents: Option<&[&str]>,
        index: usize,
        fs: Option<u8>,
        fg: Option<&str>,
        bg: Option<&str>,
    ) -> ParsedRule {
        ParsedRule {
            scope: scope.into(),
            parent_scopes: parents.map(|p| p.iter().map(|s| Box::from(*s)).collect()),
            index,
            font_style: fs,
            foreground: fg.map(c),
            background: bg.map(c),
        }
    }

    #[test]
    fn parsing_can_parse() {
        use font_style::*;
        let actual = parse_rules(&[
            rule(None, None, Some("#F8F8F2"), Some("#272822")),
            rule(Some("source, something"), None, None, Some("#100000")),
            RawRule {
                scope: Some(RawScope::List(vec!["bar".into(), "baz".into()])),
                background: Some("#010000".into()),
                ..Default::default()
            },
            rule(Some("source.css selector bar"), Some("bold"), None, None),
            rule(Some("constant"), Some("italic"), Some("#ff0000"), None),
            fg("constant.numeric", "#00ff00"),
            rule(Some("constant.numeric.hex"), Some("bold"), None, None),
            rule(
                Some("constant.numeric.oct"),
                Some("bold italic underline"),
                None,
                None,
            ),
            rule(
                Some("constant.numeric.bin"),
                Some("bold strikethrough"),
                None,
                None,
            ),
            rule(
                Some("constant.numeric.dec"),
                Some(""),
                Some("#0000ff"),
                None,
            ),
            rule(Some("foo"), Some(""), Some("#CFA"), None),
        ]);
        let expected = vec![
            p("", None, 0, None, Some("#F8F8F2"), Some("#272822")),
            p("source", None, 1, None, None, Some("#100000")),
            p("something", None, 1, None, None, Some("#100000")),
            p("bar", None, 2, None, None, Some("#010000")),
            p("baz", None, 2, None, None, Some("#010000")),
            p(
                "bar",
                Some(&["selector", "source.css"]),
                3,
                Some(BOLD),
                None,
                None,
            ),
            p("constant", None, 4, Some(ITALIC), Some("#ff0000"), None),
            p("constant.numeric", None, 5, None, Some("#00ff00"), None),
            p("constant.numeric.hex", None, 6, Some(BOLD), None, None),
            p(
                "constant.numeric.oct",
                None,
                7,
                Some(BOLD | ITALIC | UNDERLINE),
                None,
                None,
            ),
            p(
                "constant.numeric.bin",
                None,
                8,
                Some(BOLD | STRIKETHROUGH),
                None,
                None,
            ),
            p(
                "constant.numeric.dec",
                None,
                9,
                Some(0),
                Some("#0000ff"),
                None,
            ),
            p("foo", None, 10, Some(0), Some("#CFA"), None),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn resolving_str_arr_cmp_works() {
        use std::cmp::Ordering::*;
        let v = |s: &[&str]| s.iter().map(|x| Box::from(*x)).collect::<Vec<Box<str>>>();
        type Case = (
            Option<Vec<Box<str>>>,
            Option<Vec<Box<str>>>,
            std::cmp::Ordering,
        );
        let cases: [Case; 14] = [
            (None, None, Equal),
            (None, Some(v(&[])), Less),
            (None, Some(v(&["a"])), Less),
            (Some(v(&[])), None, Greater),
            (Some(v(&["a"])), None, Greater),
            (Some(v(&[])), Some(v(&[])), Equal),
            (Some(v(&[])), Some(v(&["a"])), Less),
            (Some(v(&["a"])), Some(v(&[])), Greater),
            (Some(v(&["a"])), Some(v(&["a"])), Equal),
            (Some(v(&["a", "b"])), Some(v(&["a"])), Greater),
            (Some(v(&["a"])), Some(v(&["a", "b"])), Less),
            (Some(v(&["a", "b"])), Some(v(&["a", "b"])), Equal),
            (Some(v(&["a", "b"])), Some(v(&["a", "c"])), Less),
            (Some(v(&["a", "c"])), Some(v(&["a", "b"])), Greater),
        ];
        for (i, (a, b, want)) in cases.into_iter().enumerate() {
            assert_eq!(
                str_arr_cmp(a.as_deref(), b.as_deref()),
                want,
                "case {:03}",
                i + 1
            );
        }
        let mut s = vec!["bar", "z", "zu", "a", "ab", ""];
        s.sort();
        assert_eq!(s, ["", "a", "ab", "bar", "z", "zu"]);
    }

    fn defaults_of(rules: Vec<ParsedRule>) -> StyleAttributes {
        let t = TextMateTheme::from_parsed(rules);
        assert!(t.root.children.is_empty());
        t.defaults()
    }

    #[test]
    fn resolving_always_has_and_respects_incoming_defaults() {
        let style = |fs, fg: &str, bg: &str| StyleAttributes {
            font_style: Some(fs),
            foreground: Some(c(fg)),
            background: Some(c(bg)),
        };
        assert_eq!(defaults_of(vec![]), style(0, "#000000", "#ffffff"));
        assert_eq!(
            defaults_of(vec![p("", None, 0, None, None, None)]),
            style(0, "#000000", "#ffffff")
        );
        assert_eq!(
            defaults_of(vec![p("", None, 0, Some(0), None, None)]),
            style(0, "#000000", "#ffffff")
        );
        assert_eq!(
            defaults_of(vec![p("", None, 0, Some(font_style::BOLD), None, None)]),
            style(font_style::BOLD, "#000000", "#ffffff")
        );
        assert_eq!(
            defaults_of(vec![p("", None, 0, None, Some("#ff0000"), None)]),
            style(0, "#ff0000", "#ffffff")
        );
        assert_eq!(
            defaults_of(vec![p("", None, 0, None, None, Some("#ff0000"))]),
            style(0, "#000000", "#ff0000")
        );
        // can merge incoming defaults
        assert_eq!(
            defaults_of(vec![
                p("", None, 0, None, None, Some("#ff0000")),
                p("", None, 0, None, Some("#00ff00"), None),
                p("", None, 0, Some(font_style::BOLD), None, None),
            ]),
            style(font_style::BOLD, "#00ff00", "#ff0000")
        );
    }

    fn node<'a>(t: &'a TextMateTheme, path: &[&str]) -> &'a TrieElement {
        path.iter().fold(&t.root, |n, seg| &n.children[*seg])
    }

    fn main_rule(depth: usize, fs: Option<u8>, fg: &str) -> TrieRule {
        TrieRule {
            scope_depth: depth,
            parent_scopes: Arc::from([]),
            font_style: fs,
            foreground: Some(c(fg)),
            background: None,
        }
    }

    #[test]
    fn resolving_defaults_are_inherited_and_same_rules_merged() {
        let t = TextMateTheme::from_parsed(vec![
            p("", None, 0, None, Some("#F8F8F2"), Some("#272822")),
            p("var", None, 1, Some(font_style::BOLD), None, None),
            p("var", None, 0, None, Some("#ff0000"), None),
        ]);
        assert_eq!(
            node(&t, &["var"]).main,
            main_rule(1, Some(font_style::BOLD), "#ff0000")
        );
    }

    #[test]
    fn resolving_rules_are_inherited() {
        use font_style::*;
        let t = TextMateTheme::from_parsed(vec![
            p("", None, 0, None, Some("#F8F8F2"), Some("#272822")),
            p("var", None, 0, Some(BOLD), Some("#ff0000"), None),
            p("var.identifier", None, 0, None, Some("#00ff00"), None),
            p("constant", None, 4, Some(ITALIC), Some("#100000"), None),
            p("constant.numeric", None, 5, None, Some("#200000"), None),
            p("constant.numeric.hex", None, 6, Some(BOLD), None, None),
            p(
                "constant.numeric.oct",
                None,
                7,
                Some(BOLD | ITALIC | UNDERLINE),
                None,
                None,
            ),
            p(
                "constant.numeric.dec",
                None,
                8,
                Some(0),
                Some("#300000"),
                None,
            ),
        ]);
        assert_eq!(node(&t, &["var"]).main, main_rule(1, Some(BOLD), "#ff0000"));
        assert_eq!(
            node(&t, &["var", "identifier"]).main,
            main_rule(2, Some(BOLD), "#00ff00")
        );
        assert_eq!(
            node(&t, &["constant"]).main,
            main_rule(1, Some(ITALIC), "#100000")
        );
        assert_eq!(
            node(&t, &["constant", "numeric"]).main,
            main_rule(2, Some(ITALIC), "#200000")
        );
        let num = ["constant", "numeric"];
        let at = |leaf: &str| &node(&t, &[num[0], num[1], leaf]).main;
        assert_eq!(*at("hex"), main_rule(3, Some(BOLD), "#200000"));
        assert_eq!(
            *at("oct"),
            main_rule(3, Some(BOLD | ITALIC | UNDERLINE), "#200000")
        );
        assert_eq!(*at("dec"), main_rule(3, Some(0), "#300000"));
    }

    #[test]
    fn resolving_rules_with_parent_scopes() {
        use font_style::*;
        let t = TextMateTheme::from_parsed(vec![
            p("", None, 0, None, Some("#F8F8F2"), Some("#272822")),
            p("var", None, 0, Some(BOLD), Some("#100000"), None),
            p("var.identifier", None, 0, None, Some("#200000"), None),
            p(
                "var",
                Some(&["source.css"]),
                1,
                Some(ITALIC),
                Some("#300000"),
                None,
            ),
            p("var", Some(&["source.css"]), 2, Some(UNDERLINE), None, None),
        ]);
        let with_parent = TrieRule {
            scope_depth: 1,
            parent_scopes: Arc::from([Box::from("source.css")]),
            font_style: Some(UNDERLINE),
            foreground: Some(c("#300000")),
            background: None,
        };
        let var = node(&t, &["var"]);
        assert_eq!(var.main, main_rule(1, Some(BOLD), "#100000"));
        assert_eq!(var.with_parents, vec![with_parent.clone()]);
        let ident = node(&t, &["var", "identifier"]);
        assert_eq!(ident.main, main_rule(2, Some(BOLD), "#200000"));
        assert_eq!(ident.with_parents, vec![with_parent]);
    }

    #[test]
    fn resolving_a_rule_with_child_combinator() {
        let t = TextMateTheme::from_rules(&[
            rule(None, None, Some("#100000"), None),
            fg("b a", "#200000"),
            fg("b > a", "#300000"),
            fg("c > b > a", "#400000"),
            fg("a", "#500000"),
        ]);
        assert_eq!(fg_of(&t, &["b", "a"]), Some(c("#300000")), "b a");
        assert_eq!(fg_of(&t, &["b", "c", "a"]), Some(c("#200000")), "b c a");
        assert_eq!(fg_of(&t, &["c", "b", "a"]), Some(c("#400000")), "c b a");
        assert_eq!(
            fg_of(&t, &["c", "b", "d", "a"]),
            Some(c("#200000")),
            "c b d a"
        );
    }

    #[test]
    fn resolving_gives_deeper_scopes_higher_specificity_233() {
        let t = TextMateTheme::from_rules(&[
            rule(None, None, Some("#100000"), None),
            fg("y.z a.b", "#200000"),
            fg("x y a.b", "#300000"),
        ]);
        assert_eq!(fg_of(&t, &["x", "a.b"]), None);
        assert_eq!(fg_of(&t, &["y", "a.b"]), None);
        assert_eq!(fg_of(&t, &["y.z", "a"]), None);
        assert_eq!(fg_of(&t, &["x", "y", "a.b"]), Some(c("#300000")));
        assert_eq!(fg_of(&t, &["x", "y.z", "a.b"]), Some(c("#200000")));
    }

    #[test]
    fn resolving_issue_38_ignores_rules_with_invalid_colors() {
        use font_style::*;
        let actual = parse_rules(&[
            rule(None, None, Some("#cccccc"), Some("#222222")),
            rule(Some("variable"), Some(""), None, None),
            rule(Some("variable.parameter"), Some("italic"), Some(""), None),
            rule(Some("support.other.variable"), Some(""), None, None),
            rule(Some("variable.other"), Some("normal"), Some(""), None),
            rule(
                Some("variable.parameter.function.coffee"),
                Some("italic"),
                Some("#F9D423"),
                None,
            ),
        ]);
        let expected = vec![
            p("", None, 0, None, Some("#cccccc"), Some("#222222")),
            p("variable", None, 1, Some(0), None, None),
            p("variable.parameter", None, 2, Some(ITALIC), None, None),
            p("support.other.variable", None, 3, Some(0), None, None),
            p("variable.other", None, 4, Some(0), None, None),
            p(
                "variable.parameter.function.coffee",
                None,
                5,
                Some(ITALIC),
                Some("#F9D423"),
                None,
            ),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn resolving_issue_35_trailing_comma_in_tmtheme_scope_selector() {
        let scope = [
            "meta.at-rule.return.scss,",
            "meta.at-rule.return.scss punctuation.definition,",
            "meta.at-rule.else.scss,",
            "meta.at-rule.else.scss punctuation.definition,",
            "meta.at-rule.if.scss,",
            "meta.at-rule.if.scss punctuation.definition,",
        ]
        .join("\n");
        let actual = parse_rules(&[
            rule(None, None, Some("#EFEFEF"), Some("#25292C")),
            fg(&scope, "#CC7832"),
        ]);
        let f = Some("#CC7832");
        let expected = vec![
            p("", None, 0, None, Some("#EFEFEF"), Some("#25292C")),
            p("meta.at-rule.return.scss", None, 1, None, f, None),
            p(
                "punctuation.definition",
                Some(&["meta.at-rule.return.scss"]),
                1,
                None,
                f,
                None,
            ),
            p("meta.at-rule.else.scss", None, 1, None, f, None),
            p(
                "punctuation.definition",
                Some(&["meta.at-rule.else.scss"]),
                1,
                None,
                f,
                None,
            ),
            p("meta.at-rule.if.scss", None, 1, None, f, None),
            p(
                "punctuation.definition",
                Some(&["meta.at-rule.if.scss"]),
                1,
                None,
                f,
                None,
            ),
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn resolve_merges_each_scope_level_like_the_tokenizer() {
        use font_style::*;
        let t = TextMateTheme::from_rules(&[
            rule(None, None, Some("#111111"), Some("#222222")),
            rule(Some("comment"), Some("italic"), Some("#00aa00"), None),
            rule(Some("comment.line.todo"), Some(""), None, None),
            fg("meta.function entity.name", "#dcdcaa"),
        ]);
        let s = t.resolve(&["source.cs", "comment.line.cs"]);
        assert_eq!(
            (s.font_style, s.foreground),
            (Some(ITALIC), Some(c("#00aa00")))
        );
        // fontStyle "" 显式清空斜体，但颜色继承外层。
        let s = t.resolve(&["source.cs", "comment.line.cs", "comment.line.todo"]);
        assert_eq!((s.font_style, s.foreground), (Some(0), Some(c("#00aa00"))));
        // 父 scope 选择器
        let s = t.resolve(&["source.cs", "meta.function.cs", "entity.name.function.cs"]);
        assert_eq!(s.foreground, Some(c("#dcdcaa")));
        let s = t.resolve(&["source.cs", "entity.name.function.cs"]);
        assert_eq!(s.foreground, Some(c("#111111")));
    }
}
