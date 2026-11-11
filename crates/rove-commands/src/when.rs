//! `when` clause parser and evaluator (#14), following VS Code `vs/platform/contextkey`.
//!
//! Grammar (precedence low → high): `||`, `&&`, `!`, then a primary:
//! `true` | `false` | `( expr )` | `key` | `key ==|!=|===|!== value` | `key =~ /regex/flags`
//! | `key in key2` | `key not in key2` | `key <|<=|>|>= number`.
//!
//! Semantics match VS Code: a key that is not set reads as `undefined`, so a bare unknown key and
//! any comparison against it evaluate to false (FEATURES §13 rule 7). Negation is plain logical
//! negation, so `!unknownKey` is true — exactly as in VS Code, which many default bindings rely on
//! (`!isWeb`, `!config.editor.stablePeek`). Equality uses JavaScript loose equality against the
//! literal. Regular-expression literals are lexed as one token (they may contain `/`, `:` ...).
//! Parsing happens once per expression (cached); evaluation reads only cached context values.
use crate::{ContextKeys, When, WhenEvaluator};
use rove_base::{Error, Result, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// A parsed `when` clause. Cheap to evaluate: no allocation for boolean structure.
#[derive(Clone, Debug)]
pub struct WhenExpr(Node);

#[derive(Clone, Debug)]
enum Node {
    True,
    False,
    Has(Arc<str>),
    Not(Box<Node>),
    And(Vec<Node>),
    Or(Vec<Node>),
    Equals(Arc<str>, Arc<str>),
    NotEquals(Arc<str>, Arc<str>),
    /// `None` when the literal is not a valid regular expression: evaluates false (as VS Code).
    Regex(Arc<str>, Arc<str>, Option<Arc<fancy_regex::Regex>>),
    In(Arc<str>, Arc<str>),
    NotIn(Arc<str>, Arc<str>),
    Compare(Arc<str>, Cmp, f64),
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmp {
    Less,
    LessEq,
    Greater,
    GreaterEq,
}

impl WhenExpr {
    /// Parse a `when` clause. Empty or whitespace-only text means "always".
    /// Malformed input => `Error::InvalidInput`.
    pub fn parse(source: &str) -> Result<Self> {
        let tokens = lex(source)?;
        if tokens.is_empty() {
            return Ok(Self(Node::True));
        }
        let mut p = Parser { tokens, pos: 0 };
        let node = p.or()?;
        if p.pos != p.tokens.len() {
            return Err(Error::InvalidInput);
        }
        Ok(Self(node))
    }
    pub fn evaluate(&self, context: &dyn ContextKeys) -> bool {
        eval(&self.0, context)
    }
    /// Canonical text, used to compare expressions (`-command` removal with a `when`).
    /// `&&` / `||` operands are flattened and sorted, like VS Code's normalized form.
    pub fn canonical(&self) -> String {
        canonical(&self.0)
    }
}

fn canonical(n: &Node) -> String {
    let join = |items: &[Node], sep: &str| {
        let mut parts: Vec<String> = items.iter().map(canonical).collect();
        parts.sort();
        parts.dedup();
        format!("({})", parts.join(sep))
    };
    match n {
        Node::True => "true".into(),
        Node::False => "false".into(),
        Node::Has(k) => k.to_string(),
        Node::Not(inner) => format!("!{}", canonical(inner)),
        Node::And(items) => join(items, " && "),
        Node::Or(items) => join(items, " || "),
        Node::Equals(k, v) => format!("{k} == '{v}'"),
        Node::NotEquals(k, v) => format!("{k} != '{v}'"),
        Node::Regex(k, src, _) => format!("{k} =~ {src}"),
        Node::In(k, v) => format!("{k} in {v}"),
        Node::NotIn(k, v) => format!("{k} not in {v}"),
        Node::Compare(k, c, v) => {
            let op = match c {
                Cmp::Less => "<",
                Cmp::LessEq => "<=",
                Cmp::Greater => ">",
                Cmp::GreaterEq => ">=",
            };
            format!("{k} {op} {v}")
        }
    }
}

// ---------------------------------------------------------------- evaluation

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// JavaScript `Number(string)`.
fn js_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ if t
            .chars()
            .any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E') =>
        {
            f64::NAN
        }
        _ => t.parse().unwrap_or(f64::NAN),
    }
}

/// JavaScript `parseFloat(value)`: longest numeric prefix of the string form.
fn js_parse_float(v: &Value) -> f64 {
    let s = match v {
        Value::Number(n) => return *n,
        Value::String(s) => s.trim_start().to_owned(),
        Value::Array(items) => js_string(&Value::Array(items.clone())),
        _ => return f64::NAN,
    };
    float_prefix(&s)
}

/// Longest prefix matching JS `StrDecimalLiteral` (`[+-]? (Infinity | digits [. digits] [e[+-]digits])`).
/// The scan only advances over ASCII bytes, so every slice ends on a char boundary even when the
/// text continues with multibyte characters ("1中", "中").
fn float_prefix(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i = 1;
    }
    if s[i..].starts_with("Infinity") {
        return if b[0] == b'-' {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - start
    };
    let mut mantissa = digits(&mut i);
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        let fraction = digits(&mut j);
        if mantissa + fraction > 0 {
            mantissa += fraction;
            i = j;
        }
    }
    if mantissa == 0 {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if digits(&mut j) > 0 {
            i = j;
        }
    }
    s[..i].parse().unwrap_or(f64::NAN)
}

/// JavaScript `String(value)` for the value kinds a context key can hold.
fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 1e21 => format!("{}", *n as i64),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_string(),
        Value::Array(items) => items
            .iter()
            .map(|i| match i {
                Value::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// JavaScript `value == literal` where the literal is a string.
fn loose_equals(v: &Value, literal: &str) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => s.as_ref() == literal,
        Value::Number(n) => *n == js_number(literal),
        Value::Bool(b) => (if *b { 1.0 } else { 0.0 }) == js_number(literal),
        Value::Array(_) | Value::Object(_) => js_string(v) == literal,
    }
}

fn eval(n: &Node, cx: &dyn ContextKeys) -> bool {
    match n {
        Node::True => true,
        Node::False => false,
        Node::Has(k) => cx.get(k).is_some_and(|v| truthy(&v)),
        Node::Not(inner) => !eval(inner, cx),
        Node::And(items) => items.iter().all(|i| eval(i, cx)),
        Node::Or(items) => items.iter().any(|i| eval(i, cx)),
        Node::Equals(k, lit) => cx.get(k).is_some_and(|v| loose_equals(&v, lit)),
        Node::NotEquals(k, lit) => !cx.get(k).is_some_and(|v| loose_equals(&v, lit)),
        Node::Regex(k, _, re) => match (re, cx.get(k)) {
            (Some(re), Some(v)) if !matches!(v, Value::Null) => {
                re.is_match(&js_string(&v)).unwrap_or(false)
            }
            _ => false,
        },
        Node::In(k, container) => is_in(cx, k, container),
        Node::NotIn(k, container) => !is_in(cx, k, container),
        Node::Compare(k, op, rhs) => {
            let lhs = cx.get(k).map_or(f64::NAN, |v| js_parse_float(&v));
            match op {
                Cmp::Less => lhs < *rhs,
                Cmp::LessEq => lhs <= *rhs,
                Cmp::Greater => lhs > *rhs,
                Cmp::GreaterEq => lhs >= *rhs,
            }
        }
    }
}

fn is_in(cx: &dyn ContextKeys, key: &str, container: &str) -> bool {
    let (Some(item), Some(source)) = (cx.get(key), cx.get(container)) else {
        return false;
    };
    match (&source, &item) {
        (Value::Array(items), _) => items.contains(&item),
        (Value::Object(map), Value::String(s)) => map.contains_key(s.as_ref()),
        _ => false,
    }
}

// ---------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    LParen,
    RParen,
    Not,
    And,
    Or,
    Eq,
    NotEq,
    RegexOp,
    Lt,
    Le,
    Gt,
    Ge,
    Word(Arc<str>),
    Quoted(Arc<str>),
    Regex {
        source: Arc<str>,
        pattern: String,
        flags: String,
    },
}

fn lex(src: &str) -> Result<Vec<Tok>> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        let next2 = b.get(i + 2).copied();
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'(' => {
                out.push(Tok::LParen);
                i += 1
            }
            b')' => {
                out.push(Tok::RParen);
                i += 1
            }
            b'!' if next == Some(b'=') => {
                out.push(Tok::NotEq);
                i += if next2 == Some(b'=') { 3 } else { 2 }
            }
            b'!' => {
                out.push(Tok::Not);
                i += 1
            }
            b'=' if next == Some(b'=') => {
                out.push(Tok::Eq);
                i += if next2 == Some(b'=') { 3 } else { 2 }
            }
            b'=' if next == Some(b'~') => {
                out.push(Tok::RegexOp);
                i += 2;
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                if b.get(i) == Some(&b'/') {
                    let (tok, end) = lex_regex(src, i)?;
                    out.push(tok);
                    i = end;
                }
            }
            b'&' if next == Some(b'&') => {
                out.push(Tok::And);
                i += 2
            }
            b'|' if next == Some(b'|') => {
                out.push(Tok::Or);
                i += 2
            }
            b'<' if next == Some(b'=') => {
                out.push(Tok::Le);
                i += 2
            }
            b'<' => {
                out.push(Tok::Lt);
                i += 1
            }
            b'>' if next == Some(b'=') => {
                out.push(Tok::Ge);
                i += 2
            }
            b'>' => {
                out.push(Tok::Gt);
                i += 1
            }
            b'\'' => {
                let start = i + 1;
                let end = src[start..]
                    .find('\'')
                    .map(|e| start + e)
                    .ok_or(Error::InvalidInput)?;
                out.push(Tok::Quoted(src[start..end].into()));
                i = end + 1;
            }
            b'=' | b'&' | b'|' | b'"' => return Err(Error::InvalidInput),
            _ => {
                let start = i;
                while i < b.len()
                    && !b[i].is_ascii_whitespace()
                    && !matches!(
                        b[i],
                        b'(' | b')' | b'!' | b'=' | b'<' | b'>' | b'&' | b'|' | b'\'' | b'"'
                    )
                {
                    i += 1;
                }
                out.push(Tok::Word(src[start..i].into()));
            }
        }
    }
    Ok(out)
}

/// Lex `/pattern/flags` starting at the opening slash. A `/` inside `[...]` or escaped does not
/// terminate the literal (VS Code scanner behaviour).
fn lex_regex(src: &str, open: usize) -> Result<(Tok, usize)> {
    let b = src.as_bytes();
    let mut i = open + 1;
    let (mut escaped, mut class) = (false, false);
    let close = loop {
        let c = *b.get(i).ok_or(Error::InvalidInput)?;
        if escaped {
            escaped = false;
        } else if c == b'\\' {
            escaped = true;
        } else if c == b'[' {
            class = true;
        } else if c == b']' {
            class = false;
        } else if c == b'/' && !class {
            break i;
        }
        i += 1;
    };
    let mut end = close + 1;
    while end < b.len() && b[end].is_ascii_alphabetic() {
        end += 1;
    }
    Ok((
        Tok::Regex {
            source: src[open..end].into(),
            pattern: src[open + 1..close].to_owned(),
            flags: src[close + 1..end].to_owned(),
        },
        end,
    ))
}

fn compile_regex(pattern: &str, flags: &str) -> Option<Arc<fancy_regex::Regex>> {
    // JS allows `\/` inside a literal; drop the escape (the slash is not special in Rust).
    let mut p = String::with_capacity(pattern.len() + 8);
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('/') => p.push('/'),
                Some(other) => {
                    p.push('\\');
                    p.push(other)
                }
                None => p.push('\\'),
            }
        } else {
            p.push(c)
        }
    }
    // Like `new RegExp(p, flags)`: unknown or repeated flags make the literal invalid.
    let mut seen = String::new();
    for f in flags.chars() {
        if !"dgimsuyv".contains(f) || seen.contains(f) {
            return None;
        }
        seen.push(f);
    }
    let mut prefix = String::new();
    for (flag, inline) in [('i', 'i'), ('m', 'm'), ('s', 's')] {
        if flags.contains(flag) {
            prefix.push(inline);
        }
    }
    let full = if prefix.is_empty() {
        p
    } else {
        format!("(?{prefix}){p}")
    };
    fancy_regex::Regex::new(&full).ok().map(Arc::new)
}

// ---------------------------------------------------------------- parser

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}
impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn or(&mut self) -> Result<Node> {
        let mut items = vec![self.and()?];
        while self.eat(&Tok::Or) {
            items.push(self.and()?);
        }
        Ok(if items.len() == 1 {
            items.pop().unwrap()
        } else {
            Node::Or(items)
        })
    }
    fn and(&mut self) -> Result<Node> {
        let mut items = vec![self.unary()?];
        while self.eat(&Tok::And) {
            items.push(self.unary()?);
        }
        Ok(if items.len() == 1 {
            items.pop().unwrap()
        } else {
            Node::And(items)
        })
    }
    fn unary(&mut self) -> Result<Node> {
        if self.eat(&Tok::Not) {
            return Ok(Node::Not(Box::new(self.unary()?)));
        }
        self.primary()
    }
    fn primary(&mut self) -> Result<Node> {
        match self.next().ok_or(Error::InvalidInput)? {
            Tok::LParen => {
                let inner = self.or()?;
                if !self.eat(&Tok::RParen) {
                    return Err(Error::InvalidInput);
                }
                Ok(inner)
            }
            Tok::Word(w) if w.as_ref() == "true" => Ok(Node::True),
            Tok::Word(w) if w.as_ref() == "false" => Ok(Node::False),
            Tok::Word(key) => self.after_key(key),
            _ => Err(Error::InvalidInput),
        }
    }
    fn after_key(&mut self, key: Arc<str>) -> Result<Node> {
        match self.peek() {
            Some(Tok::Eq) | Some(Tok::NotEq) => {
                let negate = self.next() == Some(Tok::NotEq);
                let value = self.value()?;
                let node = match value.as_ref() {
                    // `key == true` is `key`; `key == false` is `!key` (VS Code).
                    "true" if !matches!(self.tokens[self.pos - 1], Tok::Quoted(_)) => {
                        Node::Has(key)
                    }
                    "false" if !matches!(self.tokens[self.pos - 1], Tok::Quoted(_)) => {
                        Node::Not(Box::new(Node::Has(key)))
                    }
                    _ if negate => return Ok(Node::NotEquals(key, value)),
                    _ => return Ok(Node::Equals(key, value)),
                };
                Ok(if negate {
                    Node::Not(Box::new(node))
                } else {
                    node
                })
            }
            Some(Tok::RegexOp) => {
                self.pos += 1;
                match self.next().ok_or(Error::InvalidInput)? {
                    Tok::Regex {
                        source,
                        pattern,
                        flags,
                    } => {
                        let re = compile_regex(&pattern, &flags);
                        Ok(Node::Regex(key, source, re))
                    }
                    // VS Code also accepts a quoted literal: `key =~ '/x/i'`.
                    Tok::Quoted(q) => {
                        let (pattern, flags) = q
                            .strip_prefix('/')
                            .and_then(|r| r.rfind('/').map(|e| (&r[..e], &r[e + 1..])))
                            .unwrap_or((&q, ""));
                        let re = compile_regex(pattern, flags);
                        Ok(Node::Regex(key, q.clone(), re))
                    }
                    _ => Err(Error::InvalidInput),
                }
            }
            Some(Tok::Word(w)) if w.as_ref() == "in" => {
                self.pos += 1;
                Ok(Node::In(key, self.key()?))
            }
            Some(Tok::Word(w)) if w.as_ref() == "not" => {
                self.pos += 1;
                match self.next() {
                    Some(Tok::Word(w)) if w.as_ref() == "in" => Ok(Node::NotIn(key, self.key()?)),
                    _ => Err(Error::InvalidInput),
                }
            }
            Some(Tok::Lt) | Some(Tok::Le) | Some(Tok::Gt) | Some(Tok::Ge) => {
                let op = match self.next() {
                    Some(Tok::Lt) => Cmp::Less,
                    Some(Tok::Le) => Cmp::LessEq,
                    Some(Tok::Gt) => Cmp::Greater,
                    _ => Cmp::GreaterEq,
                };
                let value = self.value()?;
                let n: f64 = value.trim().parse().unwrap_or(f64::NAN);
                if n.is_nan() {
                    // VS Code turns a non-numeric comparison into `false`.
                    return Ok(Node::False);
                }
                Ok(Node::Compare(key, op, n))
            }
            _ => Ok(Node::Has(key)),
        }
    }
    fn value(&mut self) -> Result<Arc<str>> {
        match self.next() {
            Some(Tok::Word(w)) | Some(Tok::Quoted(w)) => Ok(w),
            _ => Err(Error::InvalidInput),
        }
    }
    fn key(&mut self) -> Result<Arc<str>> {
        match self.next() {
            Some(Tok::Word(w)) => Ok(w),
            _ => Err(Error::InvalidInput),
        }
    }
}

// ---------------------------------------------------------------- evaluator service

/// `WhenEvaluator` implementation with a parse cache keyed by source text. Thread-safe; parsing
/// a new expression happens once, later evaluations only read context values.
#[derive(Default)]
pub struct WhenEngine {
    cache: Mutex<HashMap<Arc<str>, Result<Arc<WhenExpr>>>>,
}
impl WhenEngine {
    pub fn prepare(&self, source: &When) -> Result<Arc<WhenExpr>> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .entry(source.0.clone())
            .or_insert_with(|| WhenExpr::parse(&source.0).map(Arc::new))
            .clone()
    }
}
impl WhenEvaluator for WhenEngine {
    fn evaluate(&self, expression: &When, context: &dyn ContextKeys) -> Result<bool> {
        Ok(self.prepare(expression)?.evaluate(context))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContextSnapshot;
    use std::collections::BTreeMap;

    fn ctx(pairs: &[(&str, Value)]) -> ContextSnapshot {
        pairs
            .iter()
            .map(|(k, v)| (Arc::from(*k), v.clone()))
            .collect()
    }
    fn s(v: &str) -> Value {
        Value::String(v.into())
    }
    fn eval_in(expr: &str, c: &ContextSnapshot) -> bool {
        WhenExpr::parse(expr)
            .unwrap_or_else(|e| panic!("{expr}: {e}"))
            .evaluate(c)
    }

    // Ported from VS Code src/vs/platform/contextkey/test/common/contextkey.test.ts
    // ("evaluate", "ContextKeyInExpr", "ContextKeyNotInExpr", "Greater, GreaterEquals ...").
    #[test]
    fn vscode_evaluate() {
        let c = ctx(&[
            ("a", Value::Bool(true)),
            ("b", Value::Bool(false)),
            ("c", s("5")),
            ("d", s("d")),
        ]);
        let t = |e: &str, expected: bool| assert_eq!(eval_in(e, &c), expected, "{e}");
        t("a", true);
        t("b", false);
        t("c", true);
        t("d", true);
        t("z", false);
        t("!a", false);
        t("!b", true);
        t("!z", true);
        t("a == true", true);
        t("a != true", false);
        t("b == false", true);
        t("b != false", false);
        t("a == 5", false);
        t("a != 5", true);
        t("c == 5", true);
        t("c != 5", false);
        t("d == d", true);
        t("d != d", false);
        t("z == 5", false);
        t("z != 5", true);
        t("a && b", false);
        t("a && !b", true);
        t("a && b && c", false);
        t("a || b", true);
        t("b || !a", false);
        t("a && (b || c)", true);
        t("(a && b) || c", true);
        t("a && b || c", true);
        t("b && a || b", false);
        // `!` binds tighter than `&&`, `&&` tighter than `||`.
        t("!a || a && b", false);
        t("!(a && b)", true);
        t("true", true);
        t("false", false);
        t("!false && a", true);
    }

    #[test]
    fn vscode_in_and_not_in() {
        let c = ctx(&[
            ("a", Value::Array(Arc::from([s("x")]))),
            ("b", s("x")),
            ("c", s("y")),
            (
                "d",
                Value::Object(Arc::new(BTreeMap::from([("x".into(), Value::Bool(true))]))),
            ),
            ("e", s("e")),
        ]);
        let t = |e: &str, expected: bool| assert_eq!(eval_in(e, &c), expected, "{e}");
        t("b in a", true);
        t("c in a", false);
        t("b in d", true);
        t("c in d", false);
        t("b in z", false);
        t("z in a", false);
        t("e in d", false);
        t("b not in a", false);
        t("c not in a", true);
        t("b not in d", false);
        t("c not in d", true);
    }

    #[test]
    fn vscode_comparisons() {
        let c = ctx(&[
            ("five", Value::Number(5.0)),
            ("fiveStr", s("5")),
            ("str", s("x")),
            ("flag", Value::Bool(true)),
        ]);
        let t = |e: &str, expected: bool| assert_eq!(eval_in(e, &c), expected, "{e}");
        t("five > 4", true);
        t("five > 5", false);
        t("five >= 5", true);
        t("five < 6", true);
        t("five <= 5", true);
        t("five <= 4", false);
        t("fiveStr > 4", true);
        t("str > 4", false);
        t("flag > 0", false);
        t("missing > 0", false);
        t("missing < 0", false);
        // Non-numeric right-hand side is constant false.
        t("five > x", false);
        t("five < 'abc'", false);
    }

    #[test]
    fn comparisons_on_non_ascii_values_do_not_panic() {
        let c = ctx(&[
            ("zh", s("中文")),
            ("mixed", s("1中")),
            ("frac", s("2.5中文")),
            ("exp", s("1e3é")),
            ("dot", s(".5")),
            ("neg", s("-3x")),
            ("inf", s("Infinity中")),
            ("emoji", s("😀1")),
        ]);
        let t = |e: &str, expected: bool| assert_eq!(eval_in(e, &c), expected, "{e}");
        t("zh > 0", false);
        t("zh <= 0", false);
        t("mixed == 1", false);
        t("mixed >= 1", true);
        t("mixed < 1", false);
        t("frac > 2", true);
        t("frac < 3", true);
        t("exp >= 1000", true);
        t("dot > 0.4", true);
        t("neg < -2", true);
        t("inf > 100", true);
        t("emoji > 0", false);
        assert!(float_prefix("1.").eq(&1.0));
        assert!(float_prefix(".").is_nan());
        assert!(float_prefix("1e").eq(&1.0));
        assert!(float_prefix("").is_nan());
    }

    #[test]
    fn regex_literals() {
        let c = ctx(&[
            (
                "resource",
                s("file:///c:/Users/me/AppData/Roaming/Code/User/keybindings.json"),
            ),
            ("editorLangId", s("markdown")),
            ("lang", s("CSharp")),
            ("n", Value::Number(42.0)),
        ]);
        let t = |e: &str, expected: bool| assert_eq!(eval_in(e, &c), expected, "{e}");
        t(r"resource =~ /\.json$/", true);
        t(r"resource =~ /\.md$/", false);
        // A literal that contains `/`, `:` and words is one token (issue #14 risk note).
        t(
            r"resource =~ /c:\/Users\/me\/AppData\/Roaming\/Code\/User\/keybindings.json/",
            true,
        );
        // Unknown flags make the literal invalid (like `new RegExp`), so it never matches.
        t("lang =~ /CSharp/zz", false);
        t("lang =~ /CSharp/ii", false);
        t(
            "editorLangId =~ /^(markdown|prompt|instructions|chatagent|skill)$/",
            true,
        );
        t("editorLangId =~ /^(json|xml)$/", false);
        t("lang =~ /csharp/", false);
        t("lang =~ /csharp/i", true);
        t("n =~ /^42$/", true);
        t("missing =~ /.*/", false);
        // `/` inside a character class does not close the literal.
        t(r"resource =~ /[/]keybindings/", true);
        t("resource =~ /.json$/ && editorLangId == markdown", true);
        t("!(editorLangId =~ /^markdown$/)", false);
        // Quoted regex form is accepted as in VS Code.
        t("lang =~ '/^csharp$/i'", true);
        // An invalid regular expression never matches but does not break the clause.
        t("lang =~ /(/ || true", true);
    }

    #[test]
    fn loose_equality_and_quotes() {
        let c = ctx(&[
            ("activeEditor", s("WebviewEditor")),
            ("count", Value::Number(2.0)),
            ("on", Value::Bool(true)),
            ("view", s("workbench.explorer.fileView")),
            ("scheme", s("vscode-userdata")),
        ]);
        let t = |e: &str, expected: bool| assert_eq!(eval_in(e, &c), expected, "{e}");
        t("activeEditor == 'WebviewEditor'", true);
        t("activeEditor === 'WebviewEditor'", true);
        t("activeEditor !== 'WebviewEditor'", false);
        t("activeEditor == WebviewEditor", true);
        t("activeEditor != 'Other'", true);
        t("count == 2", true);
        t("count == '2'", true);
        t("count == 3", false);
        t("on == 1", true);
        t("on == 'true'", false);
        t("view == workbench.explorer.fileView", true);
        t("scheme == vscode-userdata", true);
        t("activeEditor == 'Webview Editor'", false);
    }

    #[test]
    fn unknown_context_keys_are_false() {
        let c = ContextSnapshot::new();
        for e in [
            "unknownKey",
            "unknownKey == 'x'",
            "unknownKey =~ /x/",
            "unknownKey > 1",
            "unknownKey in other",
            "unknownKey && true",
            "terminalFocus",
        ] {
            assert!(!eval_in(e, &c), "{e}");
        }
        // Negation is logical negation of `undefined`, exactly as in VS Code.
        assert!(eval_in("!unknownKey", &c));
        assert!(eval_in("!isWeb && !config.editor.stablePeek", &c));
    }

    #[test]
    fn syntax_errors() {
        for e in [
            "a &&",
            "&& a",
            "(a",
            "a)",
            "a ==",
            "a = b",
            "a & b",
            "a | b",
            "a =~",
            "a =~ /unterminated",
            "a in",
            "a not b",
            "'unterminated",
            "a b",
            "!",
        ] {
            assert_eq!(WhenExpr::parse(e).unwrap_err(), Error::InvalidInput, "{e}");
        }
        assert!(WhenExpr::parse("")
            .unwrap()
            .evaluate(&ContextSnapshot::new()));
        assert!(WhenExpr::parse("   ")
            .unwrap()
            .evaluate(&ContextSnapshot::new()));
    }

    #[test]
    fn every_default_when_clause_parses() {
        let data = include_str!("../data/default-keybindings.json");
        let rows: Vec<serde_json::Value> = serde_json::from_str(data).unwrap();
        for row in rows {
            if let Some(w) = row["when"].as_str() {
                WhenExpr::parse(w).unwrap_or_else(|e| panic!("{w}: {e}"));
            }
        }
    }

    #[test]
    fn engine_caches_and_reports_errors() {
        let engine = WhenEngine::default();
        let c = ctx(&[("editorTextFocus", Value::Bool(true))]);
        let w = When("editorTextFocus && !editorReadonly".into());
        assert!(engine.evaluate(&w, &c).unwrap());
        assert!(Arc::ptr_eq(
            &engine.prepare(&w).unwrap(),
            &engine.prepare(&w).unwrap()
        ));
        assert_eq!(
            engine.evaluate(&When("(".into()), &c),
            Err(Error::InvalidInput)
        );
        assert!(engine.evaluate(&When("".into()), &c).unwrap());
    }

    #[test]
    fn canonical_form_ignores_operand_order() {
        let a = WhenExpr::parse("a && b || c").unwrap().canonical();
        let b = WhenExpr::parse("c || b && a").unwrap().canonical();
        assert_eq!(a, b);
        assert_ne!(a, WhenExpr::parse("a && c").unwrap().canonical());
    }
}
