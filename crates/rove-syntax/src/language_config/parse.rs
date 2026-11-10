//! JSONC to [`LanguageConfiguration`].
//!
//! The files are JSONC: VS Code's `csharp` and `markdown-basics` configurations
//! contain `//` comments and trailing commas. Parsing uses the `jsonc-parser`
//! crate, the same parser the settings layer (#13) should use; there is no
//! second hand-written JSONC reader in this crate. Offsets do not matter here,
//! so the whole file is parsed into a `serde_json::Value` before mapping it onto
//! the frozen contract types.
//!
//! Mapping rules follow VS Code's `languageConfigurationRegistry` and
//! `languageConfiguration.ts`:
//!
//! - `null` counts as "not set" for optional fields.
//! - Pairs may be written as `["open", "close"]` or `{"open": ..., "close": ...}`;
//!   `autoClosingPairs` entries may additionally carry `notIn`.
//! - `comments.lineComment` may be a string or `{"comment": ..., "noIndent": ...}`.
//! - `indentationRules.unIndentedLinePattern` (VS Code's spelling) maps to
//!   [`IndentationRules::unindented`].
//! - Every regular-expression property (`wordPattern`, `folding.markers`,
//!   `indentationRules.*`, `onEnterRules.*`) may be a bare source string or
//!   `{"pattern": ..., "flags": ...}`, as in VS Code.
//! - Unknown properties are ignored, as in VS Code, so a configuration update
//!   never breaks loading.
use super::{ConfigResult, LanguageConfigError};
use crate::{
    AutoClosingPair, EnterRule, IndentAction, IndentationRules, JsRegex, LanguageConfiguration,
    Pair,
};
use jsonc_parser::ParseOptions;
use serde_json::{Map, Value};
use std::sync::Arc;

/// Parse one `language-configuration.json`. `language` and `config` are only
/// used to attribute errors, e.g. `field brackets[2][1]: expected a string`.
pub fn parse_language_configuration(
    language: &str,
    config: &str,
    text: &str,
) -> ConfigResult<LanguageConfiguration> {
    let document = Doc { language, config };
    let value = jsonc_parser::parse_to_serde_value::<Value>(text, &ParseOptions::default())
        .map_err(|error| document.error("$", format!("invalid JSONC: {error}")))?;
    let root = document.object(&value, "$")?;
    let mut configuration = LanguageConfiguration::default();

    if let Some(comments) = member(root, "comments") {
        let comments = document.object(comments, "comments")?;
        if let Some(line) = member(comments, "lineComment") {
            configuration.line_comment = Some(match line {
                Value::String(token) => Arc::from(token.as_str()),
                Value::Object(_) => {
                    let line = document.object(line, "comments.lineComment")?;
                    let token = required(
                        member(line, "comment"),
                        &document,
                        "comments.lineComment.comment",
                    )?;
                    document.string(token, "comments.lineComment.comment")?
                }
                _ => {
                    return Err(document.error(
                        "comments.lineComment",
                        "expected a string or an object with a `comment` property",
                    ));
                }
            });
        }
        if let Some(block) = member(comments, "blockComment") {
            configuration.block_comment =
                Some(parse_pair(&document, block, "comments.blockComment")?);
        }
    }

    if let Some(brackets) = member(root, "brackets") {
        configuration.brackets = parse_pairs(&document, brackets, "brackets")?.into();
    }
    if let Some(colorized) = member(root, "colorizedBracketPairs") {
        configuration.colorized_brackets =
            Some(parse_pairs(&document, colorized, "colorizedBracketPairs")?.into());
    }
    if let Some(pairs) = member(root, "autoClosingPairs") {
        let entries = document.array(pairs, "autoClosingPairs")?;
        let mut parsed = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            parsed.push(parse_auto_closing(
                &document,
                entry,
                &format!("autoClosingPairs[{index}]"),
            )?);
        }
        configuration.auto_closing = parsed.into();
    }
    if let Some(pairs) = member(root, "surroundingPairs") {
        configuration.surrounding = parse_pairs(&document, pairs, "surroundingPairs")?.into();
    }
    if let Some(before) = member(root, "autoCloseBefore") {
        configuration.auto_close_before = Some(document.string(before, "autoCloseBefore")?);
    }

    if let Some(folding) = member(root, "folding") {
        let folding = document.object(folding, "folding")?;
        if let Some(off_side) = member(folding, "offSide") {
            configuration.folding_off_side = document.boolean(off_side, "folding.offSide")?;
        }
        if let Some(markers) = member(folding, "markers") {
            let markers = document.object(markers, "folding.markers")?;
            let start = required(member(markers, "start"), &document, "folding.markers.start")?;
            let end = required(member(markers, "end"), &document, "folding.markers.end")?;
            configuration.folding_markers = Some((
                parse_regex(&document, start, "folding.markers.start")?,
                parse_regex(&document, end, "folding.markers.end")?,
            ));
        }
    }

    if let Some(pattern) = member(root, "wordPattern") {
        configuration.word_pattern = Some(parse_regex(&document, pattern, "wordPattern")?);
    }

    if let Some(rules) = member(root, "indentationRules") {
        let rules = document.object(rules, "indentationRules")?;
        let increase = required(
            member(rules, "increaseIndentPattern"),
            &document,
            "indentationRules.increaseIndentPattern",
        )?;
        let decrease = required(
            member(rules, "decreaseIndentPattern"),
            &document,
            "indentationRules.decreaseIndentPattern",
        )?;
        configuration.indentation = Some(IndentationRules {
            increase: parse_regex(
                &document,
                increase,
                "indentationRules.increaseIndentPattern",
            )?,
            decrease: parse_regex(
                &document,
                decrease,
                "indentationRules.decreaseIndentPattern",
            )?,
            indent_next: match member(rules, "indentNextLinePattern") {
                Some(pattern) => Some(parse_regex(
                    &document,
                    pattern,
                    "indentationRules.indentNextLinePattern",
                )?),
                None => None,
            },
            unindented: match member(rules, "unIndentedLinePattern") {
                Some(pattern) => Some(parse_regex(
                    &document,
                    pattern,
                    "indentationRules.unIndentedLinePattern",
                )?),
                None => None,
            },
        });
    }

    if let Some(rules) = member(root, "onEnterRules") {
        let entries = document.array(rules, "onEnterRules")?;
        let mut parsed = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let field = format!("onEnterRules[{index}]");
            let rule = document.object(entry, &field)?;
            let before = required(
                member(rule, "beforeText"),
                &document,
                &format!("{field}.beforeText"),
            )?;
            let after = match member(rule, "afterText") {
                Some(pattern) => Some(parse_regex(
                    &document,
                    pattern,
                    &format!("{field}.afterText"),
                )?),
                None => None,
            };
            let previous_line = match member(rule, "previousLineText") {
                Some(pattern) => Some(parse_regex(
                    &document,
                    pattern,
                    &format!("{field}.previousLineText"),
                )?),
                None => None,
            };
            let action_field = format!("{field}.action");
            let action = required(member(rule, "action"), &document, &action_field)?;
            let action = document.object(action, &action_field)?;
            let indent = match member(action, "indent") {
                Some(action) => {
                    let field = format!("{action_field}.indent");
                    match document.string(action, &field)?.as_ref() {
                        "none" => IndentAction::None,
                        "indent" => IndentAction::Indent,
                        "outdent" => IndentAction::Outdent,
                        "indentOutdent" => IndentAction::IndentOutdent,
                        other => {
                            return Err(
                                document.error(field, format!("unknown indent action \"{other}\""))
                            );
                        }
                    }
                }
                None => IndentAction::None,
            };
            let append_text = match member(action, "appendText") {
                Some(text) => Some(document.string(text, &format!("{action_field}.appendText"))?),
                None => None,
            };
            let remove_text = match member(action, "removeText") {
                Some(text) => {
                    Some(document.unsigned_text(text, &format!("{action_field}.removeText"))?)
                }
                None => None,
            };
            parsed.push(EnterRule {
                before: parse_regex(&document, before, &format!("{field}.beforeText"))?,
                after,
                previous_line,
                indent,
                append_text,
                remove_text,
            });
        }
        configuration.on_enter = parsed.into();
    }

    Ok(configuration)
}

struct Doc<'a> {
    language: &'a str,
    config: &'a str,
}
impl Doc<'_> {
    fn error(&self, field: impl AsRef<str>, reason: impl AsRef<str>) -> LanguageConfigError {
        LanguageConfigError::new(self.language, self.config, field.as_ref(), reason.as_ref())
    }
    fn object<'v>(&self, value: &'v Value, field: &str) -> ConfigResult<&'v Map<String, Value>> {
        match value {
            Value::Object(object) => Ok(object),
            _ => Err(self.error(field, "expected an object")),
        }
    }
    fn array<'v>(&self, value: &'v Value, field: &str) -> ConfigResult<&'v Vec<Value>> {
        match value {
            Value::Array(array) => Ok(array),
            _ => Err(self.error(field, "expected an array")),
        }
    }
    fn string(&self, value: &Value, field: &str) -> ConfigResult<Arc<str>> {
        match value {
            Value::String(text) => Ok(Arc::from(text.as_str())),
            _ => Err(self.error(field, "expected a string")),
        }
    }
    fn boolean(&self, value: &Value, field: &str) -> ConfigResult<bool> {
        match value {
            Value::Bool(flag) => Ok(*flag),
            _ => Err(self.error(field, "expected a boolean")),
        }
    }
    fn unsigned_text(&self, value: &Value, field: &str) -> ConfigResult<u32> {
        match value {
            Value::Number(number) => number
                .as_u64()
                .and_then(|number| u32::try_from(number).ok())
                .ok_or_else(|| self.error(field, "expected a non-negative integer")),
            _ => Err(self.error(field, "expected a non-negative integer")),
        }
    }
}

/// `null` means "not set", matching VS Code's optional properties.
fn member<'v>(object: &'v Map<String, Value>, name: &str) -> Option<&'v Value> {
    object.get(name).filter(|value| !value.is_null())
}
fn required<'v>(
    value: Option<&'v Value>,
    document: &Doc<'_>,
    field: &str,
) -> ConfigResult<&'v Value> {
    value.ok_or_else(|| document.error(field, "missing field"))
}

/// Parse a regular-expression-valued property. VS Code's `_parseRegex` accepts
/// either a source string (`new RegExp(source)`) or `{pattern, flags}`, and that
/// applies to `wordPattern`, `folding.markers`, `indentationRules` and
/// `onEnterRules` alike.
fn parse_regex(document: &Doc<'_>, value: &Value, field: &str) -> ConfigResult<JsRegex> {
    match value {
        Value::String(source) => Ok(JsRegex {
            source: Arc::from(source.as_str()),
            flags: Arc::from(""),
        }),
        Value::Object(object) => {
            let pattern_field = format!("{field}.pattern");
            let pattern = required(member(object, "pattern"), document, &pattern_field)?;
            Ok(JsRegex {
                source: document.string(pattern, &pattern_field)?,
                flags: match member(object, "flags") {
                    Some(flags) => document.string(flags, &format!("{field}.flags"))?,
                    None => Arc::from(""),
                },
            })
        }
        _ => Err(document.error(
            field,
            "expected a regular expression source or an object with `pattern` and optional `flags`",
        )),
    }
}

fn parse_pair(document: &Doc<'_>, value: &Value, field: &str) -> ConfigResult<Pair> {
    match value {
        Value::Array(items) => {
            if items.len() != 2 {
                return Err(document.error(
                    field,
                    format!("expected exactly two strings, found {}", items.len()),
                ));
            }
            Ok(Pair {
                open: document.string(&items[0], &format!("{field}[0]"))?,
                close: document.string(&items[1], &format!("{field}[1]"))?,
            })
        }
        Value::Object(object) => Ok(Pair {
            open: document.string(
                required(member(object, "open"), document, &format!("{field}.open"))?,
                &format!("{field}.open"),
            )?,
            close: document.string(
                required(member(object, "close"), document, &format!("{field}.close"))?,
                &format!("{field}.close"),
            )?,
        }),
        _ => Err(document.error(field, "expected a pair of strings")),
    }
}

fn parse_pairs(document: &Doc<'_>, value: &Value, field: &str) -> ConfigResult<Vec<Pair>> {
    let entries = document.array(value, field)?;
    let mut parsed = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        parsed.push(parse_pair(document, entry, &format!("{field}[{index}]"))?);
    }
    Ok(parsed)
}

fn parse_auto_closing(
    document: &Doc<'_>,
    value: &Value,
    field: &str,
) -> ConfigResult<AutoClosingPair> {
    let pair = parse_pair(document, value, field)?;
    let not_in = match value {
        Value::Object(object) => match member(object, "notIn") {
            Some(list) => {
                let entries = document.array(list, &format!("{field}.notIn"))?;
                let mut scope = Vec::with_capacity(entries.len());
                for (index, entry) in entries.iter().enumerate() {
                    scope.push(document.string(entry, &format!("{field}.notIn[{index}]"))?);
                }
                Arc::from(scope)
            }
            None => Arc::from(Vec::<Arc<str>>::new()),
        },
        _ => Arc::from(Vec::<Arc<str>>::new()),
    };
    Ok(AutoClosingPair { pair, not_in })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> LanguageConfiguration {
        parse_language_configuration("test", "extensions/test/language-configuration.json", text)
            .unwrap()
    }
    fn error(text: &str) -> LanguageConfigError {
        parse_language_configuration("test", "extensions/test/language-configuration.json", text)
            .unwrap_err()
    }

    #[test]
    fn jsonc_comments_and_trailing_commas_are_accepted() {
        let configuration = parse(
            r#"{
                // line comment
                "brackets": [["{", "}"],], /* block */
                /* trailing */ "comments": { "lineComment": "//" },
            }"#,
        );
        assert_eq!(configuration.brackets.len(), 1);
        assert_eq!(configuration.line_comment.as_deref(), Some("//"));
    }

    #[test]
    fn missing_and_null_optionals_stay_unset() {
        let configuration = parse(r#"{"comments": null, "brackets": null}"#);
        assert!(configuration.line_comment.is_none());
        assert!(configuration.brackets.is_empty());
        assert!(!configuration.folding_off_side);
        assert!(configuration.auto_closing.is_empty());
    }

    #[test]
    fn pairs_accept_tuple_and_object_forms() {
        let configuration = parse(
            r#"{
                "brackets": [["(", ")"], {"open": "[", "close": "]"}],
                "surroundingPairs": [{"open": "'", "close": "'"}],
                "autoClosingPairs": [
                    ["{", "}"],
                    {"open": "\"", "close": "\"", "notIn": ["string", "comment"]}
                ]
            }"#,
        );
        assert_eq!(configuration.brackets.len(), 2);
        assert_eq!(configuration.brackets[1].open.as_ref(), "[");
        assert_eq!(configuration.surrounding.len(), 1);
        assert_eq!(configuration.auto_closing.len(), 2);
        assert!(configuration.auto_closing[0].not_in.is_empty());
        assert_eq!(configuration.auto_closing[1].not_in.len(), 2);
    }

    #[test]
    fn folding_markers_and_off_side_are_parsed() {
        let configuration = parse(
            r#"{"folding": {"offSide": true, "markers": {"start": "^#region", "end": "^#endregion"}}}"#,
        );
        assert!(configuration.folding_off_side);
        let (start, end) = configuration.folding_markers.unwrap();
        assert_eq!(start.source.as_ref(), "^#region");
        assert_eq!(end.flags.as_ref(), "");
    }

    #[test]
    fn word_pattern_accepts_object_and_string() {
        let configuration = parse(r#"{"wordPattern": {"pattern": "\\w+", "flags": "ug"}}"#);
        let pattern = configuration.word_pattern.unwrap();
        assert_eq!(pattern.source.as_ref(), "\\w+");
        assert_eq!(pattern.flags.as_ref(), "ug");
        let configuration = parse(r#"{"wordPattern": "\\d+"}"#);
        assert_eq!(configuration.word_pattern.unwrap().flags.as_ref(), "");
    }

    #[test]
    fn indentation_rules_and_vscode_spelling() {
        let configuration = parse(
            r#"{"indentationRules": {
                "increaseIndentPattern": "^\\{",
                "decreaseIndentPattern": "^\\}",
                "indentNextLinePattern": "case .*:",
                "unIndentedLinePattern": "^\\s*#"
            }}"#,
        );
        let rules = configuration.indentation.unwrap();
        assert_eq!(rules.increase.source.as_ref(), "^\\{");
        assert_eq!(rules.decrease.source.as_ref(), "^\\}");
        assert_eq!(rules.indent_next.unwrap().source.as_ref(), "case .*:");
        assert_eq!(rules.unindented.unwrap().source.as_ref(), "^\\s*#");
    }

    #[test]
    fn enter_rule_actions_map_to_indent_actions() {
        let configuration = parse(
            r#"{"onEnterRules": [
                {"beforeText": "^///", "action": {"indent": "none", "appendText": "/// "}},
                {"beforeText": "^/\\*", "afterText": "\\*/$", "previousLineText": "x",
                 "action": {"indent": "indentOutdent", "removeText": 1}}
            ]}"#,
        );
        assert_eq!(configuration.on_enter.len(), 2);
        assert!(matches!(
            &configuration.on_enter[0].indent,
            IndentAction::None
        ));
        assert_eq!(
            configuration.on_enter[0].append_text.as_deref(),
            Some("/// ")
        );
        assert!(matches!(
            &configuration.on_enter[1].indent,
            IndentAction::IndentOutdent
        ));
        assert_eq!(configuration.on_enter[1].remove_text, Some(1));
        assert!(configuration.on_enter[1].after.is_some());
        assert!(configuration.on_enter[1].previous_line.is_some());
    }

    #[test]
    fn comments_line_token_may_be_an_object() {
        let configuration =
            parse(r##"{"comments": {"lineComment": {"comment": "#", "noIndent": true}}}"##);
        assert_eq!(configuration.line_comment.as_deref(), Some("#"));
    }

    #[test]
    fn errors_name_the_field() {
        let missing = error(r#"{"onEnterRules": [{"action": {"indent": "none"}}]}"#);
        assert_eq!(missing.field.as_ref(), "onEnterRules[0].beforeText");
        assert_eq!(missing.reason.as_ref(), "missing field");

        let bad_pair = error(r#"{"brackets": [["(", 1]]}"#);
        assert_eq!(bad_pair.field.as_ref(), "brackets[0][1]");
        assert_eq!(bad_pair.reason.as_ref(), "expected a string");

        let bad_action =
            error(r#"{"onEnterRules": [{"beforeText": "x", "action": {"indent": "deep"}}]}"#);
        assert_eq!(bad_action.field.as_ref(), "onEnterRules[0].action.indent");
        assert!(bad_action.reason.contains("deep"));

        let invalid = error(r#"{"brackets": }"#);
        assert_eq!(invalid.field.as_ref(), "$");
        assert!(invalid.reason.contains("invalid JSONC"));
    }

    #[test]
    fn unknown_properties_are_ignored() {
        let configuration =
            parse(r#"{"comments": {"lineComment": "//", "futureField": 1}, "newTopLevel": {}}"#);
        assert_eq!(configuration.line_comment.as_deref(), Some("//"));
    }

    #[test]
    fn every_regex_property_accepts_pattern_and_flags() {
        let configuration = parse(
            r#"{
                "folding": {"markers": {
                    "start": {"pattern": "^#region", "flags": "i"},
                    "end": {"pattern": "^#endregion", "flags": "i"}
                }},
                "indentationRules": {
                    "increaseIndentPattern": {"pattern": "\\{$", "flags": "m"},
                    "decreaseIndentPattern": {"pattern": "^\\}"}
                },
                "onEnterRules": [{
                    "beforeText": {"pattern": "^//", "flags": "i"},
                    "afterText": "x",
                    "action": {"indent": "none"}
                }]
            }"#,
        );
        let (start, end) = configuration.folding_markers.as_ref().unwrap();
        assert_eq!(start.source.as_ref(), "^#region");
        assert_eq!(start.flags.as_ref(), "i");
        assert_eq!(end.flags.as_ref(), "i");
        let rules = configuration.indentation.as_ref().unwrap();
        assert_eq!(rules.increase.source.as_ref(), "\\{$");
        assert_eq!(rules.increase.flags.as_ref(), "m");
        assert_eq!(rules.decrease.source.as_ref(), "^\\}");
        assert_eq!(rules.decrease.flags.as_ref(), "");
        assert_eq!(configuration.on_enter[0].before.flags.as_ref(), "i");
        assert_eq!(
            configuration.on_enter[0]
                .after
                .as_ref()
                .unwrap()
                .source
                .as_ref(),
            "x"
        );
    }

    #[test]
    fn regex_object_without_pattern_names_the_nested_field() {
        let error = error(r#"{"folding": {"markers": {"start": {"flags": "i"}, "end": "x"}}}"#);
        assert_eq!(error.field.as_ref(), "folding.markers.start.pattern");
        assert_eq!(error.reason.as_ref(), "missing field");
    }
}
