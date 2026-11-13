//! `onEnterRules` 与回车的完整行为。
//!
//! - [`on_enter`] 是 `supports/onEnter.ts` 的 `OnEnterSupport.onEnter`：先逐条试 `onEnterRules`
//!   （`beforeText` / `afterText` / `previousLineText` 三个条件全部满足才命中），再按 `brackets`
//!   判定 `indentOutdent` 与 `indent`，触发门槛分别是 `autoIndent >= advanced` 与 `>= brackets`。
//! - [`enter_action`] 是 `enterAction.ts` 的 `getEnterAction`：补上 `appendText` 的默认值与
//!   `removeText` 作用后的缩进。
//! - [`enter`] 是 `cursorTypeOperations.ts` 的 `TypeOperations._enter`：把上面两步变成实际的改动与
//!   光标位置，`none` / `keep` / `brackets` / `advanced` / `full` 五种取值在这里产生行为差异。

use super::super::columns::LineSource;
use super::indent::{enter_indent, Action, AutoIndent, IndentConverter};
use super::{first_non_whitespace, leading_whitespace, AutoEdit, Change};
use rove_syntax::language_config::CompiledLanguageConfiguration;
use rove_syntax::Pair;

/// `onEnter` 的结果（`EnterAction`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnterResult {
    pub indent: Action,
    pub append_text: Option<String>,
    pub remove_text: Option<u32>,
}

/// `getEnterAction` 的结果：动作 + 归一化后的 `appendText` + 当前行缩进。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnterAction {
    pub indent: Action,
    pub append_text: String,
    pub indentation: String,
}

fn matches(
    regex: Option<&rove_syntax::language_config::js_regex::CompiledRegex>,
    text: &str,
) -> bool {
    match regex {
        None => true,
        Some(r) => r.is_match(text).unwrap_or(false),
    }
}

/// `OnEnterSupport.onEnter`。
pub fn on_enter(
    auto_indent: AutoIndent,
    config: &CompiledLanguageConfiguration,
    previous_line: &str,
    before_enter: &str,
    after_enter: &str,
) -> Option<EnterResult> {
    // (1) onEnterRules
    if auto_indent >= AutoIndent::Advanced {
        for rule in config.on_enter() {
            if matches(Some(&rule.before), before_enter)
                && matches(rule.after.as_ref(), after_enter)
                && matches(rule.previous_line.as_ref(), previous_line)
            {
                return Some(EnterResult {
                    indent: Action::of(&rule.indent),
                    append_text: rule.append_text.as_ref().map(|s| s.to_string()),
                    remove_text: rule.remove_text,
                });
            }
        }
    }
    // (2) 光标夹在一对括号中间：`indentOutdent`
    if auto_indent >= AutoIndent::Brackets
        && !before_enter.is_empty()
        && !after_enter.is_empty()
        && config
            .data()
            .brackets
            .iter()
            .any(|b| open_before(b, before_enter) && close_after(b, after_enter))
    {
        return Some(EnterResult {
            indent: Action::IndentOutdent,
            append_text: None,
            remove_text: None,
        });
    }
    // (3) 行尾是左括号：`indent`
    if auto_indent >= AutoIndent::Brackets
        && !before_enter.is_empty()
        && config
            .data()
            .brackets
            .iter()
            .any(|b| open_before(b, before_enter))
    {
        return Some(EnterResult {
            indent: Action::Indent,
            append_text: None,
            remove_text: None,
        });
    }
    None
}

/// `_createOpenBracketRegExp`：左括号后面只剩空白（括号前是单词字符时要求单词边界）。
fn open_before(bracket: &Pair, before: &str) -> bool {
    let trimmed = before.trim_end_matches([' ', '\t']);
    let Some(head) = trimmed.strip_suffix(&*bracket.open) else {
        return false;
    };
    // `\b`：左括号以单词字符开头时，它前面不能也是单词字符。
    let first = bracket.open.chars().next();
    match first {
        Some(c) if is_word_char(c) => head.chars().next_back().is_none_or(|p| !is_word_char(p)),
        _ => true,
    }
}

/// `_createCloseBracketRegExp`：右括号前面只剩空白（括号后是单词字符时要求单词边界）。
fn close_after(bracket: &Pair, after: &str) -> bool {
    let trimmed = after.trim_start_matches([' ', '\t']);
    let Some(tail) = trimmed.strip_prefix(&*bracket.close) else {
        return false;
    };
    let last = bracket.close.chars().next_back();
    match last {
        Some(c) if is_word_char(c) => tail.chars().next().is_none_or(|n| !is_word_char(n)),
        _ => true,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `getEnterAction`：`appendText` 为空时按动作补 `\t`，`removeText` 从当前行缩进尾部扣掉。
pub fn enter_action(
    auto_indent: AutoIndent,
    lines: &dyn LineSource,
    config: &CompiledLanguageConfiguration,
    start: (u64, usize),
    end: (u64, usize),
) -> Option<EnterAction> {
    let (line, column) = start;
    let text = lines.line_text(line).unwrap_or_default();
    let before_enter = text.get(..column.min(text.len())).unwrap_or("").to_string();
    let end_text = lines.line_text(end.0).unwrap_or_default();
    let after_enter = end_text
        .get(end.1.min(end_text.len())..)
        .unwrap_or("")
        .to_string();
    let previous_line = if line > 0 {
        lines.line_text(line - 1).unwrap_or_default()
    } else {
        String::new()
    };
    let result = on_enter(
        auto_indent,
        config,
        &previous_line,
        &before_enter,
        &after_enter,
    )?;
    let append_text = match result.append_text {
        None => match result.indent {
            Action::Indent | Action::IndentOutdent => "\t".to_string(),
            _ => String::new(),
        },
        Some(t) if result.indent == Action::Indent => format!("\t{t}"),
        Some(t) => t,
    };
    // `getIndentationAtPosition`：光标所在行的缩进，但不越过光标。
    let indentation = {
        let head = text.get(..column.min(text.len())).unwrap_or("");
        let all = leading_whitespace(&text);
        if all.len() > head.len() {
            head.to_string()
        } else {
            all.to_string()
        }
    };
    let indentation = match result.remove_text {
        Some(n) => {
            let keep = indentation.len().saturating_sub(n as usize);
            indentation[..keep].to_string()
        }
        None => indentation,
    };
    Some(EnterAction {
        indent: result.indent,
        append_text,
        indentation,
    })
}

/// `TypeOperations._enter`：在 `start..end` 上回车的改动与光标。
///
/// `trim_auto_whitespace` 时只有空白的当前行先清空（VS Code 的 `trimAutoWhitespace` 走的是
/// 模型层，这里只处理“回车时清空自己这一行”，离行清除见 [`super::super::edit`]）。
pub fn enter(
    auto_indent: AutoIndent,
    lines: &dyn LineSource,
    config: Option<&CompiledLanguageConfiguration>,
    start: (u64, usize),
    end: (u64, usize),
    converter: &IndentConverter,
) -> AutoEdit {
    let text = lines.line_text(start.0).unwrap_or_default();
    let column = start.1.min(text.len());
    // `none`：只插入换行，不带任何缩进。
    if auto_indent == AutoIndent::None {
        return AutoEdit {
            changes: vec![Change::replace(start, end, "\n")],
            caret: None,
            anchor: None,
        };
    }
    // `keep`（以及没有语言配置时）：保持当前行缩进。
    let keep_indent = || {
        let all = leading_whitespace(&text);
        let indentation = &all[..all.len().min(column)];
        converter.normalize(indentation)
    };
    let Some(config) = config else {
        return AutoEdit {
            changes: vec![Change::replace(start, end, format!("\n{}", keep_indent()))],
            caret: None,
            anchor: None,
        };
    };
    if auto_indent == AutoIndent::Keep {
        return AutoEdit {
            changes: vec![Change::replace(start, end, format!("\n{}", keep_indent()))],
            caret: None,
            anchor: None,
        };
    }
    if let Some(action) = enter_action(auto_indent, lines, config, start, end) {
        let normal = converter.normalize(&action.indentation);
        let increased =
            converter.normalize(&format!("{}{}", action.indentation, action.append_text));
        return match action.indent {
            Action::None | Action::Indent => AutoEdit {
                changes: vec![Change::replace(start, end, format!("\n{increased}"))],
                caret: None,
                anchor: None,
            },
            Action::Outdent => {
                let outdented = converter.unshift_indent(&action.indentation);
                let text = converter.normalize(&format!("{outdented}{}", action.append_text));
                AutoEdit {
                    changes: vec![Change::replace(start, end, format!("\n{text}"))],
                    caret: None,
                    anchor: None,
                }
            }
            Action::IndentOutdent => AutoEdit {
                // `\n<缩进一级>\n<原缩进>`，光标停在中间那一行的行尾。
                changes: vec![Change::replace(
                    start,
                    end,
                    format!("\n{increased}\n{normal}"),
                )],
                caret: Some((start.0 + 1, increased.len())),
                anchor: None,
            },
        };
    }
    // 没有 onEnterRules / 括号规则命中：`full` 时按 indentationRules 推导，否则保持缩进。
    if auto_indent >= AutoIndent::Full {
        if let Some(ir) = enter_indent(auto_indent, lines, config, start, end, converter) {
            let all = leading_whitespace(&text);
            let indentation = &all[..all.len().min(column)];
            let before_normalized = converter.normalize(&ir.before_enter);
            let after_normalized = converter.normalize(&ir.after_enter);
            let mut from = start;
            let mut inserted = String::from("\n");
            if indentation != before_normalized {
                // 回车前这一行的缩进也要改：整行一起替换。
                from = (start.0, 0);
                inserted = format!(
                    "{before_normalized}{}\n",
                    text.get(indentation.len()..column).unwrap_or("")
                );
            }
            // 新行上原有的前导空白被新缩进取代（VS Code 把 range 扩到第一个非空白字符）。
            let end_text = lines.line_text(end.0).unwrap_or_default();
            let to = match first_non_whitespace(&end_text) {
                Some(i) => (end.0, end.1.max(i)),
                None => (end.0, end.1.max(end_text.len())),
            };
            inserted.push_str(&after_normalized);
            return AutoEdit {
                changes: vec![Change::replace(from, to, inserted)],
                caret: None,
                anchor: None,
            };
        }
    }
    AutoEdit {
        changes: vec![Change::replace(start, end, format!("\n{}", keep_indent()))],
        caret: None,
        anchor: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::lines;
    use super::*;
    use rove_syntax::language_config::BuiltinLanguages;
    use rove_syntax::{EnterRule, IndentAction, JsRegex, LanguageConfiguration};
    use std::sync::Arc;

    fn config(language: &str) -> &'static CompiledLanguageConfiguration {
        BuiltinLanguages::shared().0.compiled(language).unwrap()
    }

    fn spaces4() -> IndentConverter {
        IndentConverter {
            insert_spaces: true,
            tab_size: 4,
            indent_size: 4,
        }
    }

    fn js(source: &str) -> JsRegex {
        JsRegex {
            source: source.into(),
            flags: "".into(),
        }
    }

    fn compile(data: LanguageConfiguration) -> CompiledLanguageConfiguration {
        CompiledLanguageConfiguration::compile("fixture", "fixture", Arc::new(data)).unwrap()
    }

    // ---------------------------------------------------------------- VS Code `onEnter.test.ts`

    /// `onEnter.test.ts` 的 `uses brackets`：括号 `[['(' , ')'], ['begin', 'end']]`，逐例移植。
    #[test]
    fn uses_brackets() {
        let data = LanguageConfiguration {
            brackets: vec![
                Pair {
                    open: "(".into(),
                    close: ")".into(),
                },
                Pair {
                    open: "begin".into(),
                    close: "end".into(),
                },
            ]
            .into(),
            ..LanguageConfiguration::default()
        };
        let c = compile(data);
        let action = |before: &str, after: &str| {
            on_enter(AutoIndent::Full, &c, "", before, after).map(|r| r.indent)
        };
        assert_eq!(action("a", ""), None);
        assert_eq!(action("", "b"), None);
        assert_eq!(action("(", "b"), Some(Action::Indent));
        assert_eq!(action("a", ")"), None);
        assert_eq!(action("begin", "ending"), Some(Action::Indent));
        assert_eq!(action("abegin", "end"), None);
        assert_eq!(action("begin", ")"), Some(Action::Indent));
        assert_eq!(action("begin", "end"), Some(Action::IndentOutdent));
        assert_eq!(action("begin ", " end"), Some(Action::IndentOutdent));
        assert_eq!(action(" begin", "end//as"), Some(Action::IndentOutdent));
        assert_eq!(action("(", ")"), Some(Action::IndentOutdent));
        assert_eq!(action("( ", ")"), Some(Action::IndentOutdent));
        assert_eq!(action("a(", ")b"), Some(Action::IndentOutdent));
        assert_eq!(action("(", ""), Some(Action::Indent));
        assert_eq!(action("( ", ""), Some(Action::Indent));
        assert_eq!(action("a(", "b"), Some(Action::Indent));
    }

    /// `javascriptOnEnterRules.ts` 的五条规则：`/* */` 块注释里补 ` * ` / `* `，收尾的 `removeText`。
    fn javascript_on_enter_rules() -> CompiledLanguageConfiguration {
        let rule = |before: &str,
                    after: Option<&str>,
                    previous: Option<&str>,
                    indent: IndentAction,
                    append: Option<&str>,
                    remove: Option<u32>| EnterRule {
            before: js(before),
            after: after.map(js),
            previous_line: previous.map(js),
            indent,
            append_text: append.map(Arc::from),
            remove_text: remove,
        };
        let data = LanguageConfiguration {
            on_enter: vec![
                rule(
                    r"^\s*\/\*\*(?!\/)([^\*]|\*(?!\/))*$",
                    Some(r"^\s*\*\/$"),
                    None,
                    IndentAction::IndentOutdent,
                    Some(" * "),
                    None,
                ),
                rule(
                    r"^\s*\/\*\*(?!\/)([^\*]|\*(?!\/))*$",
                    None,
                    None,
                    IndentAction::None,
                    Some(" * "),
                    None,
                ),
                rule(
                    r"^(\t|[ ])*[ ]\*([ ]([^\*]|\*(?!\/))*)?$",
                    None,
                    Some(r"(?=^(\s*(\/\*\*|\*)).*)(?=(?!(\s*\*\/)))"),
                    IndentAction::None,
                    Some("* "),
                    None,
                ),
                rule(
                    r"^(\t|[ ])*[ ]\*\/\s*$",
                    None,
                    None,
                    IndentAction::None,
                    None,
                    Some(1),
                ),
                rule(
                    r"^(\t|[ ])*[ ]\*[^/]*\*\/\s*$",
                    None,
                    None,
                    IndentAction::None,
                    None,
                    Some(1),
                ),
            ]
            .into(),
            ..LanguageConfiguration::default()
        };
        compile(data)
    }

    /// `onEnter.test.ts` 的 `uses regExpRules`，逐例移植（24 例）。
    #[test]
    fn uses_regexp_rules() {
        let c = javascript_on_enter_rules();
        let act = |previous: &str, before: &str, after: &str| {
            on_enter(AutoIndent::Full, &c, previous, before, after)
                .map(|r| (r.indent, r.append_text, r.remove_text))
        };
        let indent_outdent =
            |append: &str| Some((Action::IndentOutdent, Some(append.to_string()), None));
        let none = |append: &str| Some((Action::None, Some(append.to_string()), None));
        assert_eq!(act("", "\t/**", " */"), indent_outdent(" * "));
        assert_eq!(act("", "\t/**", ""), none(" * "));
        assert_eq!(act("", "\t/** * / * / * /", ""), none(" * "));
        assert_eq!(act("", "\t/** /*", ""), none(" * "));
        assert_eq!(act("", "/**", ""), none(" * "));
        assert_eq!(act("", "\t/**/", ""), None);
        assert_eq!(act("", "\t/***/", ""), None);
        assert_eq!(act("", "\t/*******/", ""), None);
        assert_eq!(act("", "\t/** * * * * */", ""), None);
        assert_eq!(act("", "\t/** */", ""), None);
        assert_eq!(act("", "\t/** asdfg */", ""), None);
        assert_eq!(act("", "\t/* asdfg */", ""), None);
        assert_eq!(act("", "*/", ""), None);
        assert_eq!(act("", "\t/*", ""), None);
        assert_eq!(act("", "\t*", ""), None);
        assert_eq!(act("\t/**", "\t *", ""), none("* "));
        assert_eq!(act("\t * something", "\t *", ""), none("* "));
        assert_eq!(act("\t *", "\t *", ""), none("* "));
        assert_eq!(act("", "\t */", ""), Some((Action::None, None, Some(1))));
        assert_eq!(act("", "\t * */", ""), Some((Action::None, None, Some(1))));
        assert_eq!(act("", "\t * * / * / * / */", ""), None);
        assert_eq!(act("\t/**", "\t *", " */"), none("* "));
        assert_eq!(act("\t/**", "\t * something", " */"), none("* "));
        assert_eq!(act("/**", " * something", " */"), none("* "));
    }

    /// `removeText` 从当前行缩进的尾部扣掉（`getEnterAction`）。
    #[test]
    fn remove_text_trims_the_indentation() {
        let c = javascript_on_enter_rules();
        let l = lines(&["  */"]);
        let a = enter_action(AutoIndent::Full, &l, &c, (0, 4), (0, 4)).unwrap();
        assert_eq!(a.indentation, " ");
        assert_eq!(a.append_text, "");
    }

    /// `indentOutdent` 的 `appendText` 默认是一级缩进（`getEnterAction` 补 `\t`）。
    #[test]
    fn indent_outdent_appends_one_level_by_default() {
        let c = config("csharp");
        let l = lines(&["if (x) {}"]);
        let a = enter_action(AutoIndent::Full, &l, c, (0, 8), (0, 8)).unwrap();
        assert_eq!(a.indent, Action::IndentOutdent);
        assert_eq!(a.append_text, "\t");
    }

    // ---------------------------------------------------------------- 随仓库的语言配置

    #[test]
    fn csharp_doc_comment_continues() {
        let c = config("csharp");
        let r = on_enter(AutoIndent::Full, c, "", "    /// <summary>", "").unwrap();
        assert_eq!(r.indent, Action::None);
        assert_eq!(r.append_text.as_deref(), Some("/// "));
    }

    #[test]
    fn csharp_line_comment_continues_only_before_text() {
        let c = config("csharp");
        // afterText 是 `^(?!\s*$)`：后面还有内容才续写 `// `。
        assert!(on_enter(AutoIndent::Full, c, "", "    // note", "rest").is_some());
        assert!(on_enter(AutoIndent::Full, c, "", "    // note", "").is_none());
    }

    #[test]
    fn on_enter_rules_need_advanced() {
        let c = config("csharp");
        assert!(on_enter(AutoIndent::Brackets, c, "", "    /// <summary>", "").is_none());
        assert!(on_enter(AutoIndent::Advanced, c, "", "    /// <summary>", "").is_some());
    }

    #[test]
    fn brackets_give_indent_outdent_between_the_pair() {
        let c = config("csharp");
        assert_eq!(
            on_enter(AutoIndent::Full, c, "", "if (x) {", "}")
                .unwrap()
                .indent,
            Action::IndentOutdent
        );
        assert_eq!(
            on_enter(AutoIndent::Full, c, "", "if (x) {", "")
                .unwrap()
                .indent,
            Action::Indent
        );
        assert!(on_enter(AutoIndent::Keep, c, "", "if (x) {", "}").is_none());
    }

    // ---------------------------------------------------------------- 回车的实际改动

    #[test]
    fn enter_between_braces_puts_caret_on_the_blank_line() {
        let c = config("csharp");
        let l = lines(&["if (x) {}"]);
        let e = enter(AutoIndent::Full, &l, Some(c), (0, 8), (0, 8), &spaces4());
        assert_eq!(e.changes.len(), 1);
        assert_eq!(e.changes[0].text, "\n    \n");
        assert_eq!(e.caret, Some((1, 4)));
    }

    #[test]
    fn enter_covers_the_five_auto_indent_values() {
        let cs = config("csharp");
        let yaml = config("yaml");
        let l = lines(&["    if (x) {"]);
        // `none`：光秃秃的换行。
        assert_eq!(
            enter(AutoIndent::None, &l, Some(cs), (0, 12), (0, 12), &spaces4()).changes[0].text,
            "\n"
        );
        // `keep`：只保持当前缩进。
        assert_eq!(
            enter(AutoIndent::Keep, &l, Some(cs), (0, 12), (0, 12), &spaces4()).changes[0].text,
            "\n    "
        );
        // `brackets`：左括号后缩进一级。
        assert_eq!(
            enter(
                AutoIndent::Brackets,
                &l,
                Some(cs),
                (0, 12),
                (0, 12),
                &spaces4()
            )
            .changes[0]
                .text,
            "\n        "
        );
        // `advanced`：多了 onEnterRules（`brackets` 下同一处只保持缩进）。
        let doc = lines(&["    /// <summary>"]);
        assert_eq!(
            enter(
                AutoIndent::Advanced,
                &doc,
                Some(cs),
                (0, 17),
                (0, 17),
                &spaces4()
            )
            .changes[0]
                .text,
            "\n    /// "
        );
        assert_eq!(
            enter(
                AutoIndent::Brackets,
                &doc,
                Some(cs),
                (0, 17),
                (0, 17),
                &spaces4()
            )
            .changes[0]
                .text,
            "\n    "
        );
        // `full`：多了 indentationRules（YAML 的 `key:` 没有括号也缩进）。
        let y = lines(&["  key:"]);
        assert_eq!(
            enter(AutoIndent::Full, &y, Some(yaml), (0, 6), (0, 6), &spaces4()).changes[0].text,
            "\n    "
        );
        assert_eq!(
            enter(
                AutoIndent::Advanced,
                &y,
                Some(yaml),
                (0, 6),
                (0, 6),
                &spaces4()
            )
            .changes[0]
                .text,
            "\n  "
        );
    }

    #[test]
    fn enter_without_a_language_keeps_the_indent() {
        let l = lines(&["    foo"]);
        let e = enter(AutoIndent::Full, &l, None, (0, 7), (0, 7), &spaces4());
        assert_eq!(e.changes[0].text, "\n    ");
    }

    #[test]
    fn open_bracket_needs_a_word_boundary() {
        let begin = Pair {
            open: "begin".into(),
            close: "end".into(),
        };
        assert!(open_before(&begin, "begin"));
        assert!(open_before(&begin, "  begin  "));
        assert!(!open_before(&begin, "xbegin"));
        assert!(close_after(&begin, "end"));
        assert!(!close_after(&begin, "ending"));
    }
}
