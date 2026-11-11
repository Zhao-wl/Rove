//! 保留格式的 JSONC 区间编辑（对应 VS Code `jsonc-parser` 的 `modify`）。
//!
//! 只替换目标键的值所在字节区间，或在目标对象末尾插入一个成员；文件其余字节原样保留：
//! 注释、空行、键顺序、缩进、换行符与 BOM 都不动。**从不**反序列化后整份重写。
//! 新增成员的缩进、换行符、冒号两侧空格和尾逗号习惯跟随文件现有风格。

use crate::jsonc::{Document, Member, Node, NodeKind, SyntaxError};
use rove_base::Value;
use std::ops::Range;

/// 文件里没有可参考的缩进时使用的缩进（VS Code 默认 `editor.tabSize = 4`、插入空格）。
const DEFAULT_INDENT: &str = "    ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// 文件当前有语法错误：拒绝编辑，避免在损坏的文件上越改越坏。
    Syntax(SyntaxError),
    /// 根节点不是对象，无法定位键。
    RootNotObject,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax(e) => write!(f, "settings file has a syntax error: {e}"),
            Self::RootNotObject => write!(f, "settings file root is not an object"),
        }
    }
}

impl std::error::Error for EditError {}

/// 把 `path` 指向的键设为 `value`；`value` 为 `None` 时删除该键（重复键全部删除）。
///
/// `path` 是对象键的路径，例如 `["editor.fontSize"]` 或 `["[csharp]", "editor.tabSize"]`。
/// 中间层不存在或不是对象时整体替换/新建。返回新文本；内容无变化时返回原文本。
pub fn set(text: &str, path: &[&str], value: Option<&Value>) -> Result<String, EditError> {
    assert!(!path.is_empty(), "edit path must not be empty");
    let mut text = text.to_string();
    loop {
        let doc = Document::parse(&text).map_err(EditError::Syntax)?;
        let style = Style::infer(&text, &doc);
        let edits = match &doc.root {
            None => match value {
                None => return Ok(text),
                Some(v) => vec![style.new_document(&text, path, v)],
            },
            Some(Node {
                kind: NodeKind::Object(members),
                span,
            }) => match plan(&text, &style, span, members, path, value) {
                Some(edits) => edits,
                None => return Ok(text),
            },
            Some(_) => return Err(EditError::RootNotObject),
        };
        text = apply(&text, edits);
        // 设值一次完成；删除每轮去掉一个（最后一个）重复键，直到不存在。
        if value.is_some() {
            return Ok(text);
        }
    }
}

type Edit = (Range<usize>, String);

fn apply(text: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut out = text.to_string();
    for (range, replacement) in edits {
        out.replace_range(range, &replacement);
    }
    out
}

/// 在对象 `span`/`members` 内为 `path` 规划编辑；无事可做返回 `None`。
fn plan(
    text: &str,
    style: &Style,
    span: &Range<usize>,
    members: &[Member],
    path: &[&str],
    value: Option<&Value>,
) -> Option<Vec<Edit>> {
    let (head, rest) = (path[0], &path[1..]);
    let found = members.iter().rposition(|m| m.key == head);
    match (found, value) {
        (None, None) => None,
        (None, Some(v)) => Some(insert_member(
            text,
            style,
            span,
            members,
            head,
            &nest(rest, v),
        )),
        (Some(i), _) if !rest.is_empty() => {
            let m = &members[i];
            match (&m.value.kind, value) {
                (NodeKind::Object(inner), _) => {
                    plan(text, style, &m.value.span, inner, rest, value)
                }
                (_, None) => None,
                (_, Some(v)) => {
                    let indent = line_indent(text, m.key_span.start);
                    Some(vec![(
                        m.value.span.clone(),
                        style.write(&nest(rest, v), indent),
                    )])
                }
            }
        }
        (Some(i), Some(v)) => {
            let m = &members[i];
            let indent = line_indent(text, m.key_span.start);
            let new = style.write(v, indent);
            (text[m.value.span.clone()] != new).then(|| vec![(m.value.span.clone(), new)])
        }
        (Some(i), None) => Some(remove_member(text, members, i)),
    }
}

/// 把 `rest` 路径包成嵌套对象：`["a"], v` → `{"a": v}`。
fn nest(rest: &[&str], value: &Value) -> Value {
    rest.iter().rev().fold(value.clone(), |inner, key| {
        Value::Object(std::sync::Arc::new([(key.to_string(), inner)].into()))
    })
}

fn insert_member(
    text: &str,
    style: &Style,
    span: &Range<usize>,
    members: &[Member],
    key: &str,
    value: &Value,
) -> Vec<Edit> {
    let open = span.start;
    let close = span.end - 1; // `}` 的偏移
    let multiline = text[open..close].contains(['\n', '\r']);
    let Some(last) = members.last() else {
        // 空对象：`{}` / `{ }` 变为多行；已是多行（可能只有注释）时插在最后一个非空白内容之后。
        let outer = line_indent(text, open);
        let indent = format!("{outer}{}", style.indent);
        let member = style.member(key, value, &indent);
        let inner_end = open + 1 + text[open + 1..close].trim_end().len();
        return if multiline {
            vec![(
                inner_end..inner_end,
                format!("{}{indent}{member}", style.eol),
            )]
        } else {
            let eol = style.eol;
            vec![(
                inner_end..close,
                format!("{eol}{indent}{member}{eol}{outer}"),
            )]
        };
    };
    if !multiline {
        let indent = line_indent(text, last.key_span.start);
        let member = style.member(key, value, indent);
        return match last.comma {
            Some(c) => vec![(c + 1..c + 1, format!(" {member},"))],
            None => {
                let end = last.value.span.end;
                vec![(end..end, format!(", {member}"))]
            }
        };
    }
    let indent = line_indent(text, last.key_span.start);
    let member = style.member(key, value, indent);
    let anchor = last.comma.map_or(last.value.span.end, |c| c + 1);
    let at = end_of_line_content(text, anchor);
    let eol = style.eol;
    match last.comma {
        Some(_) => vec![(at..at, format!("{eol}{indent}{member},"))],
        None => {
            let end = last.value.span.end;
            // 同一位置的两次插入按列表顺序应用：先插成员、再插逗号，逗号才会在成员之前。
            vec![
                (at..at, format!("{eol}{indent}{member}")),
                (end..end, ",".into()),
            ]
        }
    }
}

fn remove_member(text: &str, members: &[Member], i: usize) -> Vec<Edit> {
    let m = &members[i];
    let line_start = line_start(text, m.key_span.start);
    let own_line = text[line_start..m.key_span.start].trim().is_empty();
    match m.comma {
        Some(c) => {
            let start = if own_line {
                line_start
            } else {
                m.key_span.start
            };
            let after =
                c + 1 + text[c + 1..].len() - text[c + 1..].trim_start_matches([' ', '\t']).len();
            let end = match eol_len(text, after) {
                n if n > 0 && own_line => after + n,
                _ => after,
            };
            vec![(start..end, String::new())]
        }
        None => {
            // 最后一个成员：前一个成员的逗号也要去掉（其后的注释保留）。
            let prev_comma = i.checked_sub(1).and_then(|p| members[p].comma);
            let mut end = m.value.span.end;
            let rest = &text[end..];
            let trailing = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            if eol_len(text, end + trailing) > 0 {
                end += trailing;
            }
            if own_line {
                // 连同上一行的换行符一起去掉，不留空行。
                let before = text[..line_start].trim_end_matches(['\r', '\n']).len();
                let start = before.max(prev_comma.map_or(0, |c| c + 1));
                let mut edits = vec![(start..end, String::new())];
                if let Some(c) = prev_comma {
                    edits.push((c..c + 1, String::new()));
                }
                edits
            } else {
                vec![(prev_comma.unwrap_or(m.key_span.start)..end, String::new())]
            }
        }
    }
}

/// 从 `pos` 起跳过同一行内的空白和注释，返回该行内容的结束位置（换行符之前）。
/// 遇到其他内容（例如同一行的 `}`）就停在它之前的空白起点。
fn end_of_line_content(text: &str, pos: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = pos;
    let mut content_end = pos;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            b'\r' | b'\n' => return content_end,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                return i + text[i..].find(['\r', '\n']).unwrap_or(text.len() - i);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => match text[i + 2..].find("*/") {
                Some(e) if !text[i..i + 2 + e].contains(['\r', '\n']) => {
                    i += 2 + e + 2;
                    content_end = i;
                }
                _ => return content_end,
            },
            _ => return content_end,
        }
    }
    content_end
}

fn eol_len(text: &str, pos: usize) -> usize {
    let rest = &text.as_bytes()[pos.min(text.len())..];
    match rest {
        [b'\r', b'\n', ..] => 2,
        [b'\n', ..] | [b'\r', ..] => 1,
        _ => 0,
    }
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind(['\n', '\r']).map_or(0, |i| i + 1)
}

/// `pos` 所在行行首的空白（BOM 不算缩进）。
fn line_indent(text: &str, pos: usize) -> &str {
    let start = line_start(text, pos);
    let start = if start == 0 && text.starts_with('\u{feff}') {
        3
    } else {
        start
    };
    let line = &text[start..];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// 从文件推断的写入风格。
struct Style {
    eol: &'static str,
    indent: String,
    colon: String,
}

impl Style {
    fn infer(text: &str, doc: &Document) -> Self {
        let eol = if text.contains("\r\n") {
            "\r\n"
        } else if !text.contains('\n') && text.contains('\r') {
            "\r"
        } else {
            "\n"
        };
        let members = doc.root_members();
        let indent = members
            .iter()
            .map(|m| (line_start(text, m.key_span.start), m.key_span.start))
            .find(|(s, k)| *s > 0 && text[*s..*k].trim().is_empty() && *k > *s)
            .map(|(s, k)| text[s..k].to_string())
            .unwrap_or_else(|| DEFAULT_INDENT.to_string());
        let colon = members
            .iter()
            .map(|m| &text[m.key_span.end..m.value.span.start])
            .find(|between| !between.contains(['\n', '\r', '/']))
            .unwrap_or(": ")
            .to_string();
        Self { eol, indent, colon }
    }

    fn member(&self, key: &str, value: &Value, indent: &str) -> String {
        format!("{}{}{}", quote(key), self.colon, self.write(value, indent))
    }

    /// 序列化一个值；`indent` 是值所在行的缩进，嵌套成员再加一级。
    fn write(&self, value: &Value, indent: &str) -> String {
        let mut out = String::new();
        self.write_into(&mut out, value, indent);
        out
    }

    fn write_into(&self, out: &mut String, value: &Value, indent: &str) {
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(&number(*n)),
            Value::String(s) => out.push_str(&quote(s)),
            Value::Array(items) if items.is_empty() => out.push_str("[]"),
            Value::Array(items) if items.iter().all(is_scalar) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    self.write_into(out, item, indent);
                }
                out.push(']');
            }
            Value::Array(items) => {
                let inner = format!("{indent}{}", self.indent);
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    out.push_str(if i > 0 { "," } else { "" });
                    out.push_str(self.eol);
                    out.push_str(&inner);
                    self.write_into(out, item, &inner);
                }
                out.push_str(self.eol);
                out.push_str(indent);
                out.push(']');
            }
            Value::Object(map) if map.is_empty() => out.push_str("{}"),
            Value::Object(map) => {
                let inner = format!("{indent}{}", self.indent);
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    out.push_str(if i > 0 { "," } else { "" });
                    out.push_str(self.eol);
                    out.push_str(&inner);
                    out.push_str(&self.member(k, v, &inner));
                }
                out.push_str(self.eol);
                out.push_str(indent);
                out.push('}');
            }
        }
    }

    /// 空文件（或只有注释）：在末尾追加一个新对象。
    fn new_document(&self, text: &str, path: &[&str], value: &Value) -> Edit {
        let obj = nest(path, value);
        let body = self.write(&obj, "");
        let content_end = text.trim_end().len();
        let lead = if text[..content_end]
            .trim_start_matches('\u{feff}')
            .is_empty()
        {
            ""
        } else {
            self.eol
        };
        (content_end..text.len(), format!("{lead}{body}{}", self.eol))
    }
}

fn is_scalar(v: &Value) -> bool {
    !matches!(v, Value::Array(_) | Value::Object(_))
}

fn number(n: f64) -> String {
    if !n.is_finite() {
        "null".into()
    } else if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// JSON 字符串字面量；非 ASCII 原样保留（与 VS Code 写入一致）。
pub(crate) fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Arc};

    fn s(v: &str) -> Value {
        Value::String(v.into())
    }

    fn set_ok(text: &str, path: &[&str], v: Value) -> String {
        let out = set(text, path, Some(&v)).unwrap();
        crate::jsonc::Document::parse(&out).expect("edited text stays valid");
        out
    }

    /// 只有目标值所在区间变化：两侧字节完全相同。
    fn assert_only_value_changed(before: &str, after: &str, old: &str, new: &str) {
        let at = before.find(old).unwrap();
        assert_eq!(&after[..at], &before[..at]);
        assert_eq!(&after[at..at + new.len()], new);
        assert_eq!(&after[at + new.len()..], &before[at + old.len()..]);
    }

    const USER: &str = "\u{feff}// 我的设置\r\n{\r\n\t// 外观\r\n\t\"workbench.colorTheme\": \"Dark 2026\", /* 主题 */\r\n\r\n\t\"editor.fontSize\":14,\r\n\t\"[csharp]\": {\r\n\t\t\"editor.tabSize\": 4 // C#\r\n\t},\r\n\t\"editor.fontSize\": 15,\r\n}\r\n";

    #[test]
    fn replacing_a_value_keeps_every_other_byte() {
        let out = set_ok(USER, &["workbench.colorTheme"], s("Light 2026"));
        assert_only_value_changed(USER, &out, "\"Dark 2026\"", "\"Light 2026\"");
        let out = set_ok(USER, &["[csharp]", "editor.tabSize"], Value::Number(2.0));
        assert_only_value_changed(USER, &out, "4 // C#", "2 // C#");
    }

    #[test]
    fn duplicate_keys_edit_the_effective_last_occurrence() {
        let out = set_ok(USER, &["editor.fontSize"], Value::Number(16.0));
        assert!(out.contains("\"editor.fontSize\":14,"));
        assert_only_value_changed(USER, &out, "15,\r\n}", "16,\r\n}");
    }

    #[test]
    fn unchanged_value_returns_identical_text() {
        assert_eq!(
            set_ok(USER, &["editor.fontSize"], Value::Number(15.0)),
            USER
        );
    }

    #[test]
    fn new_key_follows_indent_eol_and_trailing_comma_style() {
        let out = set_ok(USER, &["editor.lineHeight"], Value::Number(20.0));
        assert_eq!(
            out,
            USER.replace(
                "\t\"editor.fontSize\": 15,\r\n}",
                "\t\"editor.fontSize\": 15,\r\n\t\"editor.lineHeight\": 20,\r\n}"
            )
        );
        let spaces = "{\n  \"a\":1 // note\n}\n";
        assert_eq!(
            set_ok(spaces, &["b"], Value::Bool(true)),
            "{\n  \"a\":1, // note\n  \"b\":true\n}\n"
        );
    }

    #[test]
    fn new_language_block_and_nested_key() {
        let out = set_ok(USER, &["[markdown]", "editor.wordWrap"], s("off"));
        assert!(out.ends_with(
            "\t\"editor.fontSize\": 15,\r\n\t\"[markdown]\": {\r\n\t\t\"editor.wordWrap\": \"off\"\r\n\t},\r\n}\r\n"
        ));
        let out = set_ok(
            USER,
            &["[csharp]", "editor.insertSpaces"],
            Value::Bool(false),
        );
        assert!(out.contains(
            "\t\t\"editor.tabSize\": 4, // C#\r\n\t\t\"editor.insertSpaces\": false\r\n\t},"
        ));
    }

    #[test]
    fn empty_objects_and_documents() {
        assert_eq!(
            set_ok("{}", &["a"], Value::Number(1.0)),
            "{\n    \"a\": 1\n}"
        );
        assert_eq!(
            set_ok("{\n  // only a comment\n}", &["a"], Value::Number(1.0)),
            "{\n  // only a comment\n    \"a\": 1\n}"
        );
        assert_eq!(
            set_ok("", &["a"], Value::Bool(true)),
            "{\n    \"a\": true\n}\n"
        );
        assert_eq!(
            set_ok("\u{feff}// header\r\n", &["[csharp]", "x"], Value::Null),
            "\u{feff}// header\r\n{\r\n    \"[csharp]\": {\r\n        \"x\": null\r\n    }\r\n}\r\n"
        );
        assert_eq!(
            set_ok("{\"a\": 1}", &["b"], s("x")),
            "{\"a\": 1, \"b\": \"x\"}"
        );
        assert_eq!(
            set_ok("{\"a\": 1,}", &["b"], s("x")),
            "{\"a\": 1, \"b\": \"x\",}"
        );
    }

    #[test]
    fn complex_values_are_written_with_file_indent() {
        let map: BTreeMap<String, Value> = [
            ("*.md".to_string(), s("default")),
            ("x".into(), Value::Array(Arc::from([Value::Number(1.5)]))),
        ]
        .into();
        let out = set_ok("{\n\t\"a\": 1\n}", &["o"], Value::Object(Arc::new(map)));
        assert_eq!(
            out,
            "{\n\t\"a\": 1,\n\t\"o\": {\n\t\t\"*.md\": \"default\",\n\t\t\"x\": [1.5]\n\t}\n}"
        );
        assert_eq!(quote("a\"\\\n\u{1}中"), "\"a\\\"\\\\\\n\\u0001中\"");
    }

    #[test]
    fn removal_keeps_neighbours_and_comments() {
        let text = "{\n  \"a\": 1, // keep a\n  \"b\": 2,\n  \"c\": 3\n}\n";
        assert_eq!(
            set(text, &["b"], None).unwrap(),
            "{\n  \"a\": 1, // keep a\n  \"c\": 3\n}\n"
        );
        assert_eq!(
            set(text, &["c"], None).unwrap(),
            "{\n  \"a\": 1, // keep a\n  \"b\": 2\n}\n"
        );
        assert_eq!(set("{\n  \"a\": 1\n}", &["a"], None).unwrap(), "{\n}");
        assert_eq!(set("{\"a\": 1, \"a\": 2}", &["a"], None).unwrap(), "{}");
        assert_eq!(set(text, &["zzz"], None).unwrap(), text);
    }

    #[test]
    fn invalid_documents_are_refused() {
        assert!(matches!(
            set("{\"a\": }", &["a"], Some(&Value::Null)),
            Err(EditError::Syntax(_))
        ));
        assert_eq!(
            set("[1]", &["a"], Some(&Value::Null)),
            Err(EditError::RootNotObject)
        );
    }
}
