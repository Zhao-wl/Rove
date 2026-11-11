//! JSONC 解析：行注释、块注释、尾逗号、重复键（后者胜出）、UTF-8 BOM、CRLF。
//!
//! 解析结果保留每个节点的字节区间，供 [`crate::edit`] 做区间替换；语法错误带 1 起始的行列。
//! 与 VS Code 的差别：VS Code 的 `jsonc-parser` 出错后会尽量恢复并继续使用解析出的部分，
//! Rove 按 `UX_SPEC` §6.4 整份拒绝并沿用上一份有效设置。

use rove_base::Value;
use std::{collections::BTreeMap, ops::Range, sync::Arc};

/// 嵌套上限：防止恶意或损坏的文件把解析递归压爆栈。
const MAX_DEPTH: usize = 256;

/// 语法错误的类别；展示文案由 UI 按类别选择，这里不带中文字符串。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxErrorKind {
    UnexpectedEnd,
    UnexpectedCharacter,
    UnterminatedComment,
    UnterminatedString,
    InvalidEscape,
    ControlCharacterInString,
    InvalidNumber,
    ExpectedPropertyName,
    ExpectedColon,
    ExpectedCommaOrEnd,
    ExpectedValue,
    TrailingContent,
    TooDeep,
}

/// 语法错误位置：`offset` 是含 BOM 的 UTF-8 字节偏移；`line`/`column` 从 1 开始，
/// 列按 Unicode 标量计数（`\r\n`、`\n`、`\r` 都算一次换行）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub kind: SyntaxErrorKind,
    pub offset: usize,
    pub line: u32,
    pub column: u32,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} at {}:{}", self.kind, self.line, self.column)
    }
}

impl std::error::Error for SyntaxError {}

/// 带字节区间的语法树节点。
#[derive(Clone, Debug)]
pub struct Node {
    pub span: Range<usize>,
    pub kind: NodeKind,
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    /// 成员按文件顺序保留，包括重复键。
    Object(Vec<Member>),
    Array(Vec<Node>),
    Scalar(Value),
}

#[derive(Clone, Debug)]
pub struct Member {
    pub key: String,
    /// 含引号的键区间。
    pub key_span: Range<usize>,
    pub value: Node,
    /// 紧随值之后的逗号偏移（中间可以有空白与注释）。
    pub comma: Option<usize>,
}

/// 一份解析好的 JSONC 文本。
#[derive(Clone, Debug)]
pub struct Document {
    /// 空文件（只有空白、注释或 BOM）时为 `None`，VS Code 把它当作 `{}`。
    pub root: Option<Node>,
}

impl Document {
    pub fn parse(text: &str) -> Result<Self, SyntaxError> {
        let mut p = Parser {
            text,
            bytes: text.as_bytes(),
            pos: if text.starts_with('\u{feff}') { 3 } else { 0 },
            depth: 0,
        };
        p.skip_trivia()?;
        if p.pos == p.bytes.len() {
            return Ok(Self { root: None });
        }
        let root = p.value()?;
        p.skip_trivia()?;
        if p.pos != p.bytes.len() {
            return Err(p.error(SyntaxErrorKind::TrailingContent, p.pos));
        }
        Ok(Self { root: Some(root) })
    }

    /// 根对象的成员；根不是对象或文件为空时返回空切片。
    pub fn root_members(&self) -> &[Member] {
        match &self.root {
            Some(Node {
                kind: NodeKind::Object(members),
                ..
            }) => members,
            _ => &[],
        }
    }
}

impl Node {
    /// 转为契约的 `Value`；对象里的重复键后者胜出（与 VS Code 一致）。
    pub fn to_value(&self) -> Value {
        match &self.kind {
            NodeKind::Scalar(v) => v.clone(),
            NodeKind::Array(items) => {
                Value::Array(items.iter().map(Node::to_value).collect::<Vec<_>>().into())
            }
            NodeKind::Object(members) => {
                let mut map = BTreeMap::new();
                for m in members {
                    map.insert(m.key.clone(), m.value.to_value());
                }
                Value::Object(Arc::new(map))
            }
        }
    }
}

/// 解析一段 JSONC 为 `Value`（空文本为 `None`）。
pub fn parse_value(text: &str) -> Result<Option<Value>, SyntaxError> {
    Ok(Document::parse(text)?.root.map(|n| n.to_value()))
}

/// 把字节偏移换算成 1 起始的行列（列按 Unicode 标量计数）。
pub fn line_column(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let mut line = 1u32;
    let mut line_start = 0usize;
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate().take(offset) {
        if b == b'\n' || (b == b'\r' && bytes.get(i + 1) != Some(&b'\n')) {
            line += 1;
            line_start = i + 1;
        }
    }
    let start = line_start.min(offset);
    let mut column_text = &text[start..floor_boundary(text, offset)];
    if line == 1 {
        column_text = column_text.strip_prefix('\u{feff}').unwrap_or(column_text);
    }
    (line, column_text.chars().count() as u32 + 1)
}

fn floor_boundary(text: &str, mut offset: usize) -> usize {
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, kind: SyntaxErrorKind, offset: usize) -> SyntaxError {
        let (line, column) = line_column(self.text, offset);
        SyntaxError {
            kind,
            offset,
            line,
            column,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_trivia(&mut self) -> Result<(), SyntaxError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.pos += 1,
                Some(b'/') if self.bytes.get(self.pos + 1) == Some(&b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' || c == b'\r' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.bytes.get(self.pos + 1) == Some(&b'*') => {
                    let start = self.pos;
                    match self.text[self.pos + 2..].find("*/") {
                        Some(end) => self.pos += 2 + end + 2,
                        None => return Err(self.error(SyntaxErrorKind::UnterminatedComment, start)),
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn value(&mut self) -> Result<Node, SyntaxError> {
        let start = self.pos;
        match self.peek() {
            None => Err(self.error(SyntaxErrorKind::UnexpectedEnd, start)),
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(b'"') => {
                let s = self.string()?;
                Ok(self.scalar(start, Value::String(s.into())))
            }
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error(SyntaxErrorKind::ExpectedValue, start)),
        }
    }

    fn nested(
        &mut self,
        f: fn(&mut Self) -> Result<Node, SyntaxError>,
    ) -> Result<Node, SyntaxError> {
        if self.depth >= MAX_DEPTH {
            return Err(self.error(SyntaxErrorKind::TooDeep, self.pos));
        }
        self.depth += 1;
        let node = f(self);
        self.depth -= 1;
        node
    }

    fn scalar(&self, start: usize, value: Value) -> Node {
        Node {
            span: start..self.pos,
            kind: NodeKind::Scalar(value),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Node, SyntaxError> {
        let start = self.pos;
        if !self.text[start..].starts_with(word) {
            return Err(self.error(SyntaxErrorKind::ExpectedValue, start));
        }
        self.pos += word.len();
        if matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_') {
            return Err(self.error(SyntaxErrorKind::UnexpectedCharacter, self.pos));
        }
        Ok(self.scalar(start, value))
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<Node, SyntaxError> {
        let start = self.pos;
        let bad = |p: &Self| p.error(SyntaxErrorKind::InvalidNumber, start);
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(bad(self)),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(bad(self));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(bad(self));
            }
        }
        if matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'.') {
            return Err(bad(self));
        }
        let n: f64 = self.text[start..self.pos].parse().map_err(|_| bad(self))?;
        Ok(self.scalar(start, Value::Number(n)))
    }

    /// 读取一个字符串（当前位置是左引号），返回解码后的内容。
    fn string(&mut self) -> Result<String, SyntaxError> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let run_start = self.pos;
            while matches!(self.peek(), Some(c) if c != b'"' && c != b'\\' && c >= 0x20) {
                self.pos += 1;
            }
            out.push_str(&self.text[run_start..self.pos]);
            match self.peek() {
                None => return Err(self.error(SyntaxErrorKind::UnterminatedString, start)),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    let esc = self.pos;
                    self.pos += 1;
                    let c = match self.peek() {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.pos += 1;
                            let hi = self.hex4(esc)?;
                            let c = if (0xD800..0xDC00).contains(&hi)
                                && self.text[self.pos..].starts_with("\\u")
                            {
                                let save = self.pos;
                                self.pos += 2;
                                let lo = self.hex4(esc)?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                                } else {
                                    self.pos = save;
                                    None
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            out.push(c.unwrap_or('\u{fffd}'));
                            continue;
                        }
                        _ => return Err(self.error(SyntaxErrorKind::InvalidEscape, esc)),
                    };
                    self.pos += 1;
                    out.push(c);
                }
                Some(c) if c == b'\n' || c == b'\r' => {
                    return Err(self.error(SyntaxErrorKind::UnterminatedString, start))
                }
                Some(_) => {
                    return Err(self.error(SyntaxErrorKind::ControlCharacterInString, self.pos))
                }
            }
        }
    }

    fn hex4(&mut self, esc: usize) -> Result<u32, SyntaxError> {
        let digits = self.text.get(self.pos..self.pos + 4);
        match digits.and_then(|d| {
            d.bytes()
                .all(|b| b.is_ascii_hexdigit())
                .then(|| u32::from_str_radix(d, 16).ok())
                .flatten()
        }) {
            Some(v) => {
                self.pos += 4;
                Ok(v)
            }
            None => Err(self.error(SyntaxErrorKind::InvalidEscape, esc)),
        }
    }

    fn object(&mut self) -> Result<Node, SyntaxError> {
        let start = self.pos;
        self.pos += 1;
        let mut members = Vec::new();
        loop {
            self.skip_trivia()?;
            match self.peek() {
                Some(b'}') => break,
                Some(b'"') => {}
                None => return Err(self.error(SyntaxErrorKind::UnexpectedEnd, self.pos)),
                Some(_) => return Err(self.error(SyntaxErrorKind::ExpectedPropertyName, self.pos)),
            }
            let key_start = self.pos;
            let key = self.string()?;
            let key_span = key_start..self.pos;
            self.skip_trivia()?;
            if self.peek() != Some(b':') {
                return Err(self.error(SyntaxErrorKind::ExpectedColon, self.pos));
            }
            self.pos += 1;
            self.skip_trivia()?;
            let value = self.value()?;
            self.skip_trivia()?;
            let comma = match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    Some(self.pos - 1)
                }
                Some(b'}') => None,
                None => return Err(self.error(SyntaxErrorKind::UnexpectedEnd, self.pos)),
                Some(_) => return Err(self.error(SyntaxErrorKind::ExpectedCommaOrEnd, self.pos)),
            };
            members.push(Member {
                key,
                key_span,
                value,
                comma,
            });
            if comma.is_none() {
                break;
            }
        }
        self.pos += 1;
        Ok(Node {
            span: start..self.pos,
            kind: NodeKind::Object(members),
        })
    }

    fn array(&mut self) -> Result<Node, SyntaxError> {
        let start = self.pos;
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_trivia()?;
            match self.peek() {
                Some(b']') => break,
                None => return Err(self.error(SyntaxErrorKind::UnexpectedEnd, self.pos)),
                _ => {}
            }
            items.push(self.value()?);
            self.skip_trivia()?;
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => break,
                None => return Err(self.error(SyntaxErrorKind::UnexpectedEnd, self.pos)),
                Some(_) => return Err(self.error(SyntaxErrorKind::ExpectedCommaOrEnd, self.pos)),
            }
        }
        self.pos += 1;
        Ok(Node {
            span: start..self.pos,
            kind: NodeKind::Array(items),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(text: &str) -> BTreeMap<String, Value> {
        match parse_value(text).unwrap().unwrap() {
            Value::Object(m) => (*m).clone(),
            other => panic!("not an object: {other:?}"),
        }
    }

    #[test]
    fn line_and_block_comments_are_ignored() {
        let m = obj("// head\n{ /* a */ \"a\": 1, // tail\n /* multi\n line */ \"b\": \"//not a comment\" }");
        assert_eq!(m["a"], Value::Number(1.0));
        assert_eq!(m["b"], Value::String("//not a comment".into()));
    }

    #[test]
    fn trailing_commas_are_accepted() {
        let m = obj("{\"a\": [1, 2,], \"b\": {\"c\": true,},}");
        assert_eq!(
            m["a"],
            Value::Array(vec![Value::Number(1.0), Value::Number(2.0)].into())
        );
        assert!(matches!(&m["b"], Value::Object(b) if b["c"] == Value::Bool(true)));
    }

    #[test]
    fn duplicate_keys_last_one_wins_but_tree_keeps_both() {
        let text = "{\"a\": 1, \"a\": 2}";
        assert_eq!(obj(text)["a"], Value::Number(2.0));
        assert_eq!(Document::parse(text).unwrap().root_members().len(), 2);
    }

    #[test]
    fn bom_and_crlf_are_handled() {
        let text = "\u{feff}{\r\n\t\"a\": \"x\",\r\n\t\"b\": null\r\n}\r\n";
        let m = obj(text);
        assert_eq!(m["a"], Value::String("x".into()));
        assert_eq!(m["b"], Value::Null);
        let err = Document::parse("\u{feff}{\r\n\t\"a\": 1\r\n\t\"b\": 2\r\n}").unwrap_err();
        assert_eq!(err.kind, SyntaxErrorKind::ExpectedCommaOrEnd);
        assert_eq!((err.line, err.column), (3, 2));
        let err = Document::parse("\u{feff}{,}").unwrap_err();
        assert_eq!((err.line, err.column), (1, 2));
    }

    #[test]
    fn empty_or_comment_only_documents_have_no_root() {
        assert!(parse_value("").unwrap().is_none());
        assert!(parse_value("\u{feff}  // nothing\n/* here */")
            .unwrap()
            .is_none());
    }

    #[test]
    fn strings_and_numbers_follow_json() {
        let m = obj(r#"{"s": "q\"\\\/\b\f\n\r\t\u4e2d\ud83d\ude00", "n": -1.5e3, "z": 0}"#);
        assert_eq!(m["s"], Value::String("q\"\\/\u{8}\u{c}\n\r\t中😀".into()));
        assert_eq!(m["n"], Value::Number(-1500.0));
        assert_eq!(m["z"], Value::Number(0.0));
    }

    #[test]
    fn syntax_errors_report_position_and_kind() {
        let cases = [
            ("{\"a\": 1", SyntaxErrorKind::UnexpectedEnd, (1, 8)),
            ("{\n  \"a\" 1}", SyntaxErrorKind::ExpectedColon, (2, 7)),
            ("{\"a\": tru}", SyntaxErrorKind::ExpectedValue, (1, 7)),
            ("{\"a\": 01}", SyntaxErrorKind::InvalidNumber, (1, 7)),
            (
                "{\"a\": \"x\n\"}",
                SyntaxErrorKind::UnterminatedString,
                (1, 7),
            ),
            ("{\"a\": \"\\q\"}", SyntaxErrorKind::InvalidEscape, (1, 8)),
            ("{/* open", SyntaxErrorKind::UnterminatedComment, (1, 2)),
            ("{a: 1}", SyntaxErrorKind::ExpectedPropertyName, (1, 2)),
            ("{} {}", SyntaxErrorKind::TrailingContent, (1, 4)),
            ("{\"中文\": ?}", SyntaxErrorKind::ExpectedValue, (1, 8)),
        ];
        for (text, kind, pos) in cases {
            let err = Document::parse(text).unwrap_err();
            assert_eq!((err.kind, (err.line, err.column)), (kind, pos), "{text}");
        }
        let deep = "[".repeat(10_000);
        assert_eq!(
            Document::parse(&deep).unwrap_err().kind,
            SyntaxErrorKind::TooDeep
        );
    }

    #[test]
    fn spans_point_at_source_bytes() {
        let text = "{ \"a\" : [1, 2] , \"b\": {} }";
        let doc = Document::parse(text).unwrap();
        let m = &doc.root_members()[0];
        assert_eq!(&text[m.key_span.clone()], "\"a\"");
        assert_eq!(&text[m.value.span.clone()], "[1, 2]");
        assert_eq!(&text[m.comma.unwrap()..m.comma.unwrap() + 1], ",");
        assert_eq!(doc.root_members()[1].comma, None);
    }
}
