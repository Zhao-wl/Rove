//! Compiles the ECMAScript regular expressions found in
//! `language-configuration.json` with `fancy-regex`.
//!
//! `fancy-regex` supplies look-around and backreferences, but its shorthands are
//! Rust/Unicode shorthands: `\d` is any Unicode decimal digit and `\w`/`\b` are
//! Unicode-aware. JavaScript is different, and even its `u` flag does not change
//! this: `\d` is `[0-9]` and `\w` is `[0-9A-Za-z_]` in both modes. This module
//! rewrites only those differences plus a few escapes that `fancy-regex` rejects
//! (`\cX`, `\0`), and reports everything it cannot translate as a
//! [`LanguageConfigError`] naming the configuration and field.
//!
//! Deliberate limits, all reported as errors instead of silently weakening the
//! pattern:
//!
//! - `\D`, `\W`, `\S` and `\B` inside a character class cannot be expressed as a
//!   class member, so they are rejected. None of the shipped configurations use
//!   them there.
//! - The sticky (`y`) and Unicode-sets (`v`) flags are stateful or change set
//!   syntax; they are rejected.
//! - `m` uses Rust's line terminators (`\n`) rather than ECMAScript's
//!   (`\n`, `\r`, `\u2028`, `\u2029`); no shipped configuration passes `m`.
//!
//! Character classes follow ECMAScript where Rust differs: `[]` (never matches)
//! and `[^]` (any character) are rewritten, `&`, `~` and `[` are escaped because
//! Rust would read `&&`/`~~` as set operations and `[` as a nested or POSIX
//! class, and `]` always closes the class. A `-` is kept as a range only between
//! single-character atoms; next to a set escape (`\d`, `\w`, `\s`, `\p{...}`) it
//! is escaped, because ECMAScript treats it as a literal there.
//!
//! `\p{...}` and `\u{...}` are Unicode-mode escapes: with the `u` flag they are
//! compiled as property/code-point escapes, and without it they are the identity
//! escape plus literal braces, which is what ECMAScript matches. Identity escapes
//! of letters (e.g. `\A`) are accepted in both modes even though `u` mode would
//! reject them; the shipped configurations only use valid escapes.
use super::{ConfigResult, LanguageConfigError};
use crate::JsRegex;
use fancy_regex::{Regex, RegexBuilder};
use rove_base::{Error, Result};
use std::{fmt, sync::Arc};
/// ECMAScript `\w` members: always ASCII, even in Unicode mode.
const JS_WORD_CLASS: &str = "0-9A-Za-z_";
/// ECMAScript `\s` members: `WhiteSpace` plus `LineTerminator` (includes U+FEFF,
/// excludes U+0085, unlike Rust's `\s`).
const JS_SPACE_CLASS: &str = "\\t\\n\\x0B\\f\\r \\u{00A0}\\u{1680}\\u{2000}-\\u{200A}\
\\u{2028}\\u{2029}\\u{202F}\\u{205F}\\u{3000}\\u{FEFF}";
/// ECMAScript `.` without the `s` flag: every code point except a line terminator.
const JS_ANY_CHAR: &str = "[^\\n\\r\\u{2028}\\u{2029}]";
/// Any character, including line terminators; ECMAScript `[^]`.
const ANY_CHAR: &str = "(?s:.)";
/// A pattern that never matches; ECMAScript `[]`.
const NEVER_MATCH: &str = "(?!)";
/// ASCII word boundary, built from look-around because `fancy-regex` rejects
/// `(?-u:\b)` ("changing Unicode mode inline is not supported").
const ASCII_WORD_BOUNDARY: &str = "(?:(?<![0-9A-Za-z_])(?=[0-9A-Za-z_])\
|(?<=[0-9A-Za-z_])(?![0-9A-Za-z_]))";
/// ASCII non-word-boundary counterpart of [`ASCII_WORD_BOUNDARY`].
const ASCII_NON_WORD_BOUNDARY: &str = "(?:(?<=[0-9A-Za-z_])(?=[0-9A-Za-z_])\
|(?<![0-9A-Za-z_])(?![0-9A-Za-z_]))";
/// Where a regular expression came from; used to make failures actionable.
#[derive(Clone, Debug)]
pub struct RegexOrigin {
    pub language: Arc<str>,
    pub config: Arc<str>,
    pub field: Arc<str>,
}
impl RegexOrigin {
    pub fn new(
        language: impl Into<Arc<str>>,
        config: impl Into<Arc<str>>,
        field: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            language: language.into(),
            config: config.into(),
            field: field.into(),
        }
    }
    fn error(&self, reason: impl Into<Arc<str>>) -> LanguageConfigError {
        LanguageConfigError {
            language: self.language.clone(),
            config: self.config.clone(),
            field: self.field.clone(),
            reason: reason.into(),
        }
    }
}
impl fmt::Display for RegexOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "\"{}\" (language \"{}\") field {}",
            self.config, self.language, self.field
        )
    }
}
/// Byte ranges of a match and of its capture groups; `None` for a group that did not
/// participate. Group 0 is the whole match. Added for #48's regular expression replacement.
pub type CaptureRanges = Vec<Option<(usize, usize)>>;

/// A JavaScript regular expression compiled for matching against `str`.
#[derive(Clone)]
pub struct CompiledRegex {
    javascript: JsRegex,
    expression: Regex,
}
impl CompiledRegex {
    /// Translate and compile `javascript`, attributing any failure to `origin`.
    pub fn compile(javascript: &JsRegex, origin: &RegexOrigin) -> ConfigResult<Self> {
        let flags = JsFlags::parse(&javascript.flags, origin)?;
        let translated = translate(&javascript.source, flags, origin)?;
        let mut builder = RegexBuilder::new(&translated);
        builder
            .case_insensitive(flags.case_insensitive)
            .multi_line(flags.multi_line)
            .dot_matches_new_line(flags.dot_all);
        let expression = builder.build().map_err(|error| {
            origin.error(format!(
                "cannot compile JavaScript regular expression /{}/{} as /{}/: {error}",
                javascript.source, javascript.flags, translated
            ))
        })?;
        Ok(Self {
            javascript: javascript.clone(),
            expression,
        })
    }
    /// The JavaScript source and flags, unchanged.
    pub fn javascript(&self) -> &JsRegex {
        &self.javascript
    }
    /// Whether `text` contains a match. Backtracking-limit failures surface as
    /// `Err(Error::Unsupported)` rather than as "no match".
    pub fn is_match(&self, text: &str) -> Result<bool> {
        self.expression
            .is_match(text)
            .map_err(|_| Error::Unsupported)
    }
    /// Whether capture group 1 participated in the first match; used by the
    /// combined `(start)|(?:end)` folding-marker matcher.
    pub(crate) fn captures_group1(&self, text: &str) -> Result<Option<bool>> {
        match self.expression.captures(text) {
            Ok(Some(captures)) => Ok(Some(captures.get(1).is_some())),
            Ok(None) => Ok(None),
            Err(_) => Err(Error::Unsupported),
        }
    }
    /// Byte range of the first match at or after byte `start`.
    pub fn find_from(&self, text: &str, start: usize) -> Result<Option<(usize, usize)>> {
        if start > text.len() || !text.is_char_boundary(start) {
            return Err(Error::OutOfBounds);
        }
        match self.expression.find_from_pos(text, start) {
            Ok(found) => Ok(found.map(|m| (m.start(), m.end()))),
            Err(_) => Err(Error::Unsupported),
        }
    }
    /// Byte range of the first match.
    pub fn find(&self, text: &str) -> Result<Option<(usize, usize)>> {
        self.find_from(text, 0)
    }
    /// Byte ranges of the whole match and of every capture group of the first match at or after
    /// byte `start`; `None` for a group that did not participate. Added for #48, whose regular
    /// expression replacement expands `$1`…`$9` from the groups of the match it is replacing.
    pub fn capture_ranges(&self, text: &str, start: usize) -> Result<Option<CaptureRanges>> {
        if start > text.len() || !text.is_char_boundary(start) {
            return Err(Error::OutOfBounds);
        }
        match self.expression.captures_from_pos(text, start) {
            Ok(Some(captures)) => Ok(Some(
                captures
                    .iter()
                    .map(|group| group.map(|m| (m.start(), m.end())))
                    .collect(),
            )),
            Ok(None) => Ok(None),
            Err(_) => Err(Error::Unsupported),
        }
    }
}
/// Compile one configuration regular expression.
pub fn compile_regex(javascript: &JsRegex, origin: &RegexOrigin) -> ConfigResult<CompiledRegex> {
    CompiledRegex::compile(javascript, origin)
}
#[derive(Clone, Copy, Default)]
struct JsFlags {
    case_insensitive: bool,
    multi_line: bool,
    dot_all: bool,
    /// `u`: enables `\p{...}` and `\u{...}`. Without it those are identity
    /// escapes, because ECMAScript only defines them in Unicode mode.
    unicode: bool,
}
impl JsFlags {
    fn parse(flags: &str, origin: &RegexOrigin) -> ConfigResult<Self> {
        let mut parsed = Self::default();
        for flag in flags.chars() {
            match flag {
                'i' => parsed.case_insensitive = true,
                'm' => parsed.multi_line = true,
                's' => parsed.dot_all = true,
                'u' => parsed.unicode = true,
                // `g` only makes a JavaScript regexp stateful; callers iterate
                // explicitly.
                'g' => {}
                other => {
                    return Err(origin.error(format!(
                        "unsupported JavaScript regular expression flag '{other}'"
                    )));
                }
            }
        }
        Ok(parsed)
    }
}
/// Rewrite the JavaScript-specific parts of `source`.
fn translate(source: &str, flags: JsFlags, origin: &RegexOrigin) -> ConfigResult<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len() + 16);
    let mut index = 0;
    let mut in_class = false;
    // The previous in-class atom expanded to a set (`\d`, `\w`, `\s`, `\p{...}`).
    // A `-` next to such an atom is a literal in ECMAScript, never a range.
    let mut previous_was_set = false;
    while index < chars.len() {
        let current = chars[index];
        if current == '\\' {
            let (next, expansion_is_set) =
                push_escape(&chars, index, &mut out, in_class, flags.unicode, origin)?;
            previous_was_set = in_class && expansion_is_set;
            index = next;
            continue;
        }
        if in_class {
            match current {
                // ECMAScript ends the class at the first `]`; `[]` and `[^]` are
                // handled before entering, so no member-position exception exists.
                ']' => {
                    in_class = false;
                    out.push(']');
                }
                // A range needs single-character atoms on both sides. Next to a
                // set escape the `-` is a literal, and Rust would read the
                // expansion's trailing characters as a range bound.
                '-' => {
                    if previous_was_set || next_escape_is_set(&chars, index + 1, flags.unicode) {
                        out.push_str("\\-");
                    } else {
                        out.push('-');
                    }
                }
                // Rust-only class syntax: `&&`/`~~` are set operations and `[`
                // starts a nested class or POSIX class. ECMAScript has none of
                // them, so escape the literal character.
                '&' | '~' | '[' => {
                    out.push('\\');
                    out.push(current);
                }
                other => out.push(other),
            }
            previous_was_set = false;
            index += 1;
            continue;
        }
        match current {
            '[' => match chars.get(index + 1) {
                // ECMAScript: `[]` never matches (not an empty Rust class, which
                // is a parse error), and `[^]` matches any character.
                Some(']') => {
                    out.push_str(NEVER_MATCH);
                    index += 2;
                }
                Some('^') if chars.get(index + 2) == Some(&']') => {
                    out.push_str(ANY_CHAR);
                    index += 3;
                }
                _ => {
                    out.push('[');
                    index += 1;
                    if chars.get(index) == Some(&'^') {
                        out.push('^');
                        index += 1;
                    }
                    in_class = true;
                }
            },
            // A `]` outside a class is a literal in ECMAScript; escape it so the
            // Rust parser cannot read it as a stray class delimiter.
            ']' => {
                out.push_str("\\]");
                index += 1;
            }
            // ECMAScript treats a `{` that cannot start a quantifier as a literal
            // (in non-Unicode mode); Rust rejects it. A real quantifier is copied
            // verbatim.
            '{' => match quantifier_end(&chars, index) {
                Some(end) => {
                    out.extend(chars[index..=end].iter().copied());
                    index = end + 1;
                }
                None => {
                    out.push_str("\\{");
                    index += 1;
                }
            },
            '.' if !flags.dot_all => {
                out.push_str(JS_ANY_CHAR);
                index += 1;
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    if in_class {
        return Err(origin.error("unterminated character class in JavaScript regular expression"));
    }
    Ok(out)
}

/// Whether the atom starting at `index` (just after a class `-`) expands to a set,
/// which would make that `-` a literal in ECMAScript.
fn next_escape_is_set(chars: &[char], index: usize, unicode: bool) -> bool {
    if chars.get(index) != Some(&'\\') {
        return false;
    }
    match chars.get(index + 1) {
        Some('d' | 'D' | 'w' | 'W' | 's' | 'S') => true,
        Some('p' | 'P') => unicode && chars.get(index + 2) == Some(&'{'),
        _ => false,
    }
}

/// End index of a `{n}`, `{n,}` or `{n,m}` quantifier starting at `index`, if any.
fn quantifier_end(chars: &[char], index: usize) -> Option<usize> {
    let mut cursor = index + 1;
    let digits = cursor;
    while chars.get(cursor).is_some_and(char::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == digits {
        return None;
    }
    if chars.get(cursor) == Some(&',') {
        cursor += 1;
        while chars.get(cursor).is_some_and(char::is_ascii_digit) {
            cursor += 1;
        }
    }
    (chars.get(cursor) == Some(&'}')).then_some(cursor)
}

/// Append the translation of the escape at `index` (which points at `\`) and
/// return the index just past it, plus whether it expanded to a character set.
fn push_escape(
    chars: &[char],
    index: usize,
    out: &mut String,
    in_class: bool,
    unicode: bool,
    origin: &RegexOrigin,
) -> ConfigResult<(usize, bool)> {
    let escape = match chars.get(index + 1) {
        Some(&escape) => escape,
        None => return Err(origin.error("regular expression ends with a backslash")),
    };
    match escape {
        'd' => {
            out.push_str(if in_class { "0-9" } else { "[0-9]" });
            Ok((index + 2, true))
        }
        'D' => {
            if in_class {
                return Err(origin.error(
                    "unsupported JavaScript regular expression feature: \\D inside a character class",
                ));
            }
            out.push_str("[^0-9]");
            Ok((index + 2, true))
        }
        'w' => {
            out.push_str(if in_class {
                JS_WORD_CLASS
            } else {
                "[0-9A-Za-z_]"
            });
            Ok((index + 2, true))
        }
        'W' => {
            if in_class {
                return Err(origin.error(
                    "unsupported JavaScript regular expression feature: \\W inside a character class",
                ));
            }
            out.push_str("[^0-9A-Za-z_]");
            Ok((index + 2, true))
        }
        's' => {
            if in_class {
                out.push_str(JS_SPACE_CLASS);
            } else {
                out.push('[');
                out.push_str(JS_SPACE_CLASS);
                out.push(']');
            }
            Ok((index + 2, true))
        }
        'S' => {
            if in_class {
                return Err(origin.error(
                    "unsupported JavaScript regular expression feature: \\S inside a character class",
                ));
            }
            out.push_str("[^");
            out.push_str(JS_SPACE_CLASS);
            out.push(']');
            Ok((index + 2, true))
        }
        'b' => {
            if in_class {
                // Inside a class `\b` is backspace in ECMAScript.
                out.push_str("\\x08");
            } else {
                out.push_str(ASCII_WORD_BOUNDARY);
            }
            Ok((index + 2, false))
        }
        'B' => {
            if in_class {
                return Err(origin.error(
                    "unsupported JavaScript regular expression feature: \\B inside a character class",
                ));
            }
            out.push_str(ASCII_NON_WORD_BOUNDARY);
            Ok((index + 2, false))
        }
        'c' => {
            let letter = match chars.get(index + 2).copied() {
                Some(letter) if letter.is_ascii_alphabetic() => letter,
                _ => {
                    return Err(origin
                        .error("invalid JavaScript control escape: expected a letter after \\c"));
                }
            };
            out.push_str(&format!("\\x{{{:X}}}", (letter as u32) % 32));
            Ok((index + 3, false))
        }
        '0' => {
            if chars.get(index + 2).is_some_and(char::is_ascii_digit) {
                return Err(
                    origin.error("unsupported JavaScript regular expression feature: octal escape")
                );
            }
            out.push_str("\\x00");
            Ok((index + 2, false))
        }
        '1'..='9' => {
            // Backreference; `fancy-regex` supports numbered backreferences.
            out.push('\\');
            out.push(escape);
            Ok((index + 2, false))
        }
        'p' | 'P' if chars.get(index + 2) == Some(&'{') => {
            if unicode {
                // Property escapes only exist with the `u` flag; without it `\p`
                // is the identity escape `p` followed by literal braces.
                copy_braced(chars, index, out, origin, "Unicode property escape")
                    .map(|end| (end, true))
            } else {
                out.push(escape);
                Ok((index + 2, false))
            }
        }
        'p' | 'P' | 'k' if chars.get(index + 2) != Some(&'<') => {
            // Without its delimiter the escape is an ECMAScript identity escape,
            // not a `fancy-regex` assertion.
            out.push(escape);
            Ok((index + 2, false))
        }
        'k' => copy_delimited(chars, index, out, origin, '<', '>').map(|end| (end, false)),
        'u' if chars.get(index + 2) == Some(&'{') => {
            if unicode {
                copy_braced(chars, index, out, origin, "code point escape").map(|end| (end, false))
            } else {
                // Without `u`, `\u{...}` is the identity escape `u` followed by
                // literal braces rather than a code point escape.
                out.push('u');
                Ok((index + 2, false))
            }
        }
        'n' | 'r' | 't' | 'f' | 'v' | 'x' | 'u' => {
            out.push('\\');
            out.push(escape);
            Ok((index + 2, false))
        }
        other if other.is_ascii_alphabetic() => {
            // Other letter escapes are identity escapes in non-Unicode mode;
            // emitting the bare letter keeps them literal instead of letting the
            // engine read e.g. `\A` as "start of text".
            out.push(other);
            Ok((index + 2, false))
        }
        other if other.is_ascii() => {
            out.push('\\');
            out.push(other);
            Ok((index + 2, false))
        }
        other => {
            out.push(other);
            Ok((index + 2, false))
        }
    }
}

fn copy_braced(
    chars: &[char],
    index: usize,
    out: &mut String,
    origin: &RegexOrigin,
    what: &str,
) -> ConfigResult<usize> {
    let close = (index + 3..chars.len())
        .find(|&position| chars[position] == '}')
        .ok_or_else(|| origin.error(format!("unterminated {what}")))?;
    out.extend(chars[index..=close].iter().copied());
    Ok(close + 1)
}

fn copy_delimited(
    chars: &[char],
    index: usize,
    out: &mut String,
    origin: &RegexOrigin,
    open: char,
    close: char,
) -> ConfigResult<usize> {
    let end = (index + 2..chars.len())
        .find(|&position| chars[position] == close)
        .ok_or_else(|| origin.error(format!("unterminated escape delimited by {open}")))?;
    out.extend(chars[index..=end].iter().copied());
    Ok(end + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(source: &str) -> CompiledRegex {
        compile_with(source, "")
    }
    fn compile_with(source: &str, flags: &str) -> CompiledRegex {
        CompiledRegex::compile(
            &JsRegex {
                source: source.into(),
                flags: flags.into(),
            },
            &RegexOrigin::new(
                "test",
                "test/language-configuration.json",
                "wordPattern.pattern",
            ),
        )
        .unwrap()
    }
    fn error(source: &str, flags: &str) -> LanguageConfigError {
        CompiledRegex::compile(
            &JsRegex {
                source: source.into(),
                flags: flags.into(),
            },
            &RegexOrigin::new(
                "jsonc",
                "extensions/json/language-configuration.json",
                "indentationRules.increaseIndentPattern",
            ),
        )
        .err()
        .expect("expected an error")
    }

    #[test]
    fn digits_are_ascii_like_javascript() {
        let regex = compile(r"\d+");
        assert!(regex.is_match("1234").unwrap());
        assert!(
            !regex.is_match("\u{663}\u{664}").unwrap(),
            "Arabic-Indic digits are not \\d in JS"
        );
        assert!(compile(r"[\d]").is_match("7").unwrap());
        assert!(!compile(r"[\d]").is_match("\u{667}").unwrap());
    }

    #[test]
    fn word_characters_are_ascii_like_javascript() {
        let regex = compile(r"^\w+$");
        assert!(regex.is_match("abc_123").unwrap());
        assert!(!regex.is_match("abc\u{E9}").unwrap());
        assert!(!regex.is_match("\u{FF57}\u{FF49}\u{FF44}\u{FF45}").unwrap());
    }

    #[test]
    fn word_boundaries_are_ascii_like_javascript() {
        let regex = compile(r"^\s*#region\b");
        assert!(regex.is_match("#region").unwrap());
        assert!(regex.is_match("   #region // note").unwrap());
        // ECMAScript's `\b` is ASCII: a non-ASCII letter after `region` is not a
        // word character, so the boundary still matches. Rust's Unicode `\b`
        // would refuse here.
        assert!(regex.is_match("#region\u{E9}").unwrap());
        assert!(!regex.is_match("#regionX").unwrap());
        assert!(compile(r"\Babc").is_match("xabc").unwrap());
        assert!(compile(r"[\b]").is_match("\u{8}").unwrap());
    }

    #[test]
    fn whitespace_matches_the_ecmascript_set() {
        let regex = compile(r"^\s$");
        assert!(regex.is_match(" ").unwrap());
        assert!(
            regex.is_match("\u{FEFF}").unwrap(),
            "JS \\s includes U+FEFF"
        );
        assert!(!regex.is_match("\u{85}").unwrap(), "JS \\s excludes U+0085");
        assert!(regex.is_match("\u{A0}").unwrap());
        assert!(compile(r"\S").is_match("x").unwrap());
        assert!(!compile(r"\S").is_match("\u{A0}").unwrap());
        assert!(compile(r"[\s]").is_match(" ").unwrap());
    }

    #[test]
    fn dot_excludes_every_javascript_line_terminator() {
        let regex = compile(r"^a.b$");
        assert!(regex.is_match("axb").unwrap());
        assert!(!regex.is_match("a\nb").unwrap());
        assert!(!regex.is_match("a\rb").unwrap());
        assert!(!regex.is_match("a\u{2028}b").unwrap());
        assert!(compile_with(r"^a.b$", "s").is_match("a\nb").unwrap());
    }

    #[test]
    fn look_around_and_backreferences_survive() {
        assert!(compile(r"^(?!\s*$)").is_match("  x").unwrap());
        assert!(!compile(r"^(?!\s*$)").is_match("   ").unwrap());
        assert!(compile(r"(?<=\d+)x").is_match("12x").unwrap());
        assert!(compile(r"(?<n>ab)\k<n>").is_match("abab").unwrap());
        assert!(compile(r"(a)\1").is_match("aa").unwrap());
        assert!(compile(r#"(?<=["'])[^"']+"#).is_match("\"x\"").unwrap());
    }

    #[test]
    fn escapes_fancy_regex_rejects_are_translated() {
        assert!(compile(r"\cA").is_match("\u{1}").unwrap());
        assert!(compile(r"\0").is_match("\0").unwrap());
        // `\A` is not a JavaScript escape: it is the literal letter A, not the
        // "start of text" assertion it means in Rust.
        assert!(compile(r"\A").is_match("A").unwrap());
        assert!(!compile(r"\A").is_match("xyz").unwrap());
        assert!(compile(r"^\A$").is_match("A").unwrap());
        assert!(!compile(r"^\A$").is_match("B").unwrap());
        assert!(compile(r"\x41").is_match("A").unwrap());
        assert!(compile(r"\u0041").is_match("A").unwrap());
    }

    #[test]
    fn unicode_escapes_require_the_u_flag() {
        // With `u`: property and code point escapes.
        assert!(compile_with(r"\p{Alphabetic}+", "u")
            .is_match("hello")
            .unwrap());
        assert!(compile_with(r"\u{1F600}", "u")
            .is_match("\u{1F600}")
            .unwrap());
        // Without `u` ECMAScript reads both as an identity escape plus literal
        // braces; Node 22 matches "p{Alphabetic}" and "u{1F600}".
        let property = compile(r"\p{Alphabetic}");
        assert!(property.is_match("p{Alphabetic}").unwrap());
        assert!(!property.is_match("abc").unwrap());
        let code_point = compile(r"\u{1F600}");
        assert!(code_point.is_match("u{1F600}").unwrap());
        assert!(!code_point.is_match("\u{1F600}").unwrap());
    }

    #[test]
    fn dash_is_a_range_only_between_single_character_atoms() {
        // `\w` expands to several characters; the following `-` must stay a
        // literal instead of becoming a Rust range bound.
        let word_dash = compile(r"[\w\-.]");
        for text in ["-", ".", "_", "a", "0"] {
            assert!(word_dash.is_match(text).unwrap(), "matches {text:?}");
        }
        assert!(!word_dash.is_match(" ").unwrap());
        // `-` before or after a set escape is literal in ECMAScript.
        let dash_before_set = compile(r"[a-\d]");
        for text in ["a", "-", "5"] {
            assert!(dash_before_set.is_match(text).unwrap(), "matches {text:?}");
        }
        let set_before_dash = compile(r"[\d-a]");
        for text in ["0", "-", "a"] {
            assert!(set_before_dash.is_match(text).unwrap(), "matches {text:?}");
        }
        // A range followed by `-` (valid ECMAScript) stays parseable in Rust.
        let chained = compile(r"[a-b-c]");
        for text in ["-", "b", "c"] {
            assert!(chained.is_match(text).unwrap(), "matches {text:?}");
        }
        assert!(!chained.is_match("d").unwrap());
        // Explicitly escaped `-` and ranges with single-character escapes.
        assert!(compile(r"[\w\-.]").is_match("-").unwrap());
        assert!(compile(r"[\x41-\x5A]").is_match("M").unwrap());
    }

    #[test]
    fn flags_are_applied() {
        assert!(compile_with(r"^abc$", "i").is_match("ABC").unwrap());
        assert!(!compile_with(r"^abc$", "").is_match("ABC").unwrap());
        assert!(compile_with(r"^b$", "m").is_match("a\nb").unwrap());
        // `g` and `u` do not change matching in this module.
        assert!(compile_with(r"a", "gu").is_match("a").unwrap());
    }

    #[test]
    fn character_classes_follow_ecmascript_rules() {
        // `[]` never matches and `[^]` matches any character; Rust rejects the
        // first and reads the second as "unterminated".
        assert!(!compile(r"[]").is_match("x").unwrap());
        assert!(!compile(r"[]x]").is_match("x]").unwrap());
        assert!(compile(r"[^]").is_match("\n").unwrap());
        assert!(compile(r"[^]").is_match("\u{1F600}").unwrap());
        // `&&`, `~~` and `[` are literals in ECMAScript but set operations or
        // nested classes in Rust.
        let intersection = compile(r"[a&&b]");
        for text in ["a", "b", "&"] {
            assert!(
                intersection.is_match(text).unwrap(),
                "[a&&b] matches {text:?}"
            );
        }
        assert!(!intersection.is_match("c").unwrap());
        assert!(compile(r"[a[b]").is_match("[").unwrap());
        // POSIX classes do not exist in ECMAScript: `[[:alpha:]]` is a class of
        // `[ : a l p h` followed by a literal `]`.
        assert!(compile(r"[[:alpha:]]").is_match("a]").unwrap());
        assert!(!compile(r"[[:alpha:]]").is_match("b").unwrap());
        // A `]` outside a class is a literal in both dialects.
        assert!(compile(r"a]b").is_match("a]b").unwrap());
        // `[^]]` is `[^]` (any character) followed by a literal `]`.
        assert!(compile(r"[^]]").is_match("q]").unwrap());
        assert!(!compile(r"[^]]").is_match("q").unwrap());
    }

    #[test]
    fn unsupported_features_name_the_configuration_and_field() {
        let flag = error("a", "v");
        assert!(flag
            .to_string()
            .contains("extensions/json/language-configuration.json"));
        assert!(flag.to_string().contains("jsonc"));
        assert!(flag
            .to_string()
            .contains("indentationRules.increaseIndentPattern"));
        assert!(flag.to_string().contains("flag 'v'"));

        let class = error(r"[\D]", "");
        assert!(class.reason.contains("\\D inside a character class"));

        let class = error(r"[\S]", "");
        assert!(class.reason.contains("\\S inside a character class"));

        let unterminated = error(r"[abc", "");
        assert!(unterminated.reason.contains("unterminated character class"));

        let trailing = error("abc\\", "");
        assert!(trailing.reason.contains("ends with a backslash"));

        let engine = error("(abc", "");
        assert!(engine.reason.contains("cannot compile"));
        assert!(engine.to_string().contains("field indentationRules"));
    }

    #[test]
    fn capture_ranges_report_every_group_of_the_first_match() {
        let regex = compile(r"(\w+)\s*=\s*(\d+)?;");
        let text = "int a  = 12; int b = ;";
        let groups = regex.capture_ranges(text, 0).unwrap().unwrap();
        assert_eq!(groups[0], Some((4, 12)));
        assert_eq!(&text[groups[1].unwrap().0..groups[1].unwrap().1], "a");
        assert_eq!(&text[groups[2].unwrap().0..groups[2].unwrap().1], "12");
        // 第二处匹配里 `(\d+)` 没有参与。
        let groups = regex.capture_ranges(text, 12).unwrap().unwrap();
        assert_eq!(&text[groups[1].unwrap().0..groups[1].unwrap().1], "b");
        assert_eq!(groups[2], None);
        assert_eq!(regex.capture_ranges("nothing here", 0).unwrap(), None);
        assert!(matches!(
            regex.capture_ranges(text, text.len() + 1),
            Err(Error::OutOfBounds)
        ));
    }

    #[test]
    fn runtime_errors_are_not_reported_as_no_match() {
        let regex = compile(r"^(a+)+$");
        assert!(regex.is_match("aaaa").unwrap());
        assert!(matches!(
            regex.is_match(&"a".repeat(64)),
            Ok(true) | Err(Error::Unsupported)
        ));
    }
}
