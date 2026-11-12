//! 查询：搜索文本与选项 → 编译好的匹配器；“包含 / 排除的文件” → glob 规则（#25）。

use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use rove_base::{Error, Result, Value};
use rove_workspace::RuleSet;
use std::collections::BTreeMap;
use std::sync::Arc;

/// 搜索视图交给引擎的一次查询。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub pattern: String,
    pub is_regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// “包含的文件”输入框的原文（逗号分隔的 glob）。
    pub includes: String,
    /// “排除的文件”输入框的原文。
    pub excludes: String,
    /// “使用排除设置”开关：`files.exclude`、`search.exclude`、`rove.files.extraExclude`。
    pub use_exclude_settings: bool,
}

impl SearchQuery {
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            use_exclude_settings: true,
            ..Self::default()
        }
    }

    /// 与 VS Code 一样：正则里出现 `\n`（转义或字面换行）时按多行搜索。
    pub fn is_multiline(&self) -> bool {
        self.is_regex && (self.pattern.contains("\\n") || self.pattern.contains('\n'))
    }
}

/// 编译匹配器。`smart_case` 是 `search.smartCase`：区分大小写开关关闭、且查询全是小写时不区分大小写，
/// 否则区分。非法正则返回 `InvalidInput`（界面显示校验错误，不发起搜索）。
pub fn compile(query: &SearchQuery, smart_case: bool) -> Result<RegexMatcher> {
    if query.pattern.is_empty() {
        return Err(Error::InvalidInput);
    }
    let mut builder = RegexMatcherBuilder::new();
    builder
        .fixed_strings(!query.is_regex)
        .word(query.whole_word)
        // 大量字面量组成的正则也能编译；与 ripgrep 默认一致。
        .size_limit(100 * (1 << 20))
        .dfa_size_limit(1000 * (1 << 20));
    if query.case_sensitive {
        builder.case_insensitive(false);
    } else if smart_case {
        builder.case_smart(true);
    } else {
        builder.case_insensitive(true);
    }
    if query.is_multiline() {
        builder.multi_line(true);
    } else {
        builder.line_terminator(Some(b'\n'));
    }
    builder
        .build(&query.pattern)
        .map_err(|_| Error::InvalidInput)
}

/// 把输入框里逗号分隔的模式展开成 VS Code 的语义（`queryBuilder.expandGlobalGlob`）：
///
/// - `./src`、`.\src` 相对工作区根；
/// - 其他不以 `**` 开头的模式加 `**/` 前缀（`*.cs` → `**/*.cs`，`Assets` → `**/Assets`）；
/// - 结尾的 `/` 去掉；目录模式匹配其下全部文件由 [`RuleSet::matches`] 的祖先检查完成；
/// - 花括号里的逗号不分隔（`*.{cs,shader}`）。
pub fn patterns(input: &str) -> Vec<String> {
    split_outside_braces(input)
        .into_iter()
        .filter_map(|raw| {
            let mut p = raw.trim().replace('\\', "/");
            if p.is_empty() {
                return None;
            }
            while p.len() > 1 && p.ends_with('/') {
                p.pop();
            }
            if let Some(rest) = p.strip_prefix("./") {
                let rest = rest.trim_start_matches('/');
                return (!rest.is_empty()).then(|| rest.to_string());
            }
            let p = p.trim_start_matches('/').to_string();
            if p.is_empty() {
                None
            } else if p.starts_with("**") {
                Some(p)
            } else {
                Some(format!("**/{p}"))
            }
        })
        .collect()
}

fn split_outside_braces(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in input.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = (depth - 1).max(0),
            ',' if depth == 0 => {
                parts.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

/// 输入框原文 → 规则集；空输入返回 `None`。
pub fn rules(input: &str) -> Option<RuleSet> {
    let list = patterns(input);
    if list.is_empty() {
        return None;
    }
    let map: BTreeMap<String, Value> = list.into_iter().map(|p| (p, Value::Bool(true))).collect();
    RuleSet::from_value(&Value::Object(Arc::new(map))).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grep_matcher::Matcher;

    fn find_all(m: &RegexMatcher, text: &str) -> Vec<(usize, usize)> {
        let mut out = vec![];
        m.find_iter(text.as_bytes(), |r| {
            out.push((r.start(), r.end()));
            true
        })
        .unwrap();
        out
    }

    #[test]
    fn literal_queries_escape_regex_metacharacters() {
        let m = compile(&SearchQuery::new("a.b("), false).unwrap();
        assert_eq!(find_all(&m, "axb( a.b("), vec![(5, 9)]);
    }

    #[test]
    fn case_insensitive_by_default_and_sensitive_with_the_toggle() {
        let q = SearchQuery::new("player");
        assert_eq!(
            find_all(&compile(&q, false).unwrap(), "Player player").len(),
            2
        );
        let q = SearchQuery {
            case_sensitive: true,
            ..SearchQuery::new("player")
        };
        assert_eq!(
            find_all(&compile(&q, false).unwrap(), "Player player"),
            vec![(7, 13)]
        );
    }

    #[test]
    fn smart_case_is_insensitive_only_for_all_lowercase() {
        let lower = SearchQuery::new("player");
        assert_eq!(
            find_all(&compile(&lower, true).unwrap(), "Player player").len(),
            2
        );
        let mixed = SearchQuery::new("Player");
        assert_eq!(
            find_all(&compile(&mixed, true).unwrap(), "Player player"),
            vec![(0, 6)]
        );
        // smartCase 关闭时混合大小写仍不区分。
        assert_eq!(
            find_all(&compile(&mixed, false).unwrap(), "Player player").len(),
            2
        );
    }

    #[test]
    fn whole_word_rejects_partial_identifiers() {
        let q = SearchQuery {
            whole_word: true,
            ..SearchQuery::new("Move")
        };
        assert_eq!(
            find_all(&compile(&q, false).unwrap(), "Move MoveTo move(x) _move"),
            vec![(0, 4), (12, 16)]
        );
    }

    #[test]
    fn regex_and_invalid_regex() {
        let q = SearchQuery {
            is_regex: true,
            ..SearchQuery::new(r"Get\w+<\w+>")
        };
        assert_eq!(
            find_all(
                &compile(&q, false).unwrap(),
                "x = GetComponent<Rigidbody>();"
            ),
            vec![(4, 27)]
        );
        let bad = SearchQuery {
            is_regex: true,
            ..SearchQuery::new("(unclosed")
        };
        assert_eq!(compile(&bad, false).unwrap_err(), Error::InvalidInput);
        assert_eq!(
            compile(&SearchQuery::new(""), false).unwrap_err(),
            Error::InvalidInput
        );
    }

    #[test]
    fn include_patterns_follow_vscode_expansion() {
        assert_eq!(
            patterns("*.cs, ./Assets/Scripts/ ,Editor,{a,b}.txt,**/x"),
            vec![
                "**/*.cs",
                "Assets/Scripts",
                "**/Editor",
                "**/{a,b}.txt",
                "**/x"
            ]
        );
        assert!(patterns(" , ").is_empty());
        let r = rules("Scripts, *.shader").unwrap();
        assert!(r.is_excluded("Assets/Scripts/Player.cs"));
        assert!(r.is_excluded("Assets/A.shader"));
        assert!(!r.is_excluded("Assets/A.cs"));
        let anchored = rules("./Assets").unwrap();
        assert!(anchored.is_excluded("Assets/A.cs"));
        assert!(!anchored.is_excluded("Packages/Assets/A.cs"));
    }
}
