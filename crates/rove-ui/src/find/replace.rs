//! 替换模型（#48）：替换串的解析与展开、保留大小写，以及把一组匹配变成**一次**多区间事务。
//!
//! 不依赖 GPUI、不自己查找匹配：匹配列表、当前项与范围都来自 #21 的 [`FindModel`](super::state::FindModel)，
//! 捕获组由 [`Searcher::capture_texts`] 按同一口径重跑正则得到。界面层（[`super::host`]）只负责
//! 取参数、调用这里，再把事务交给 #46 的 [`EditSession`](rove_buffer::edit::EditSession)。
//!
//! # 语义（对齐 VS Code `replacePattern.ts` 与 `vs/base/common/search.ts`）
//!
//! - **只有正则模式**解析替换串：`$0` / `$&` 是整个匹配，`$1`…`$99` 是捕获组，`$$` 是字面 `$`，
//!   `\n` / `\t` / `\\` 是转义，`\u` / `\U` / `\l` / `\L` 是大小写操作符（作用在紧随其后的 `$n` 上）。
//!   普通文本模式下替换串原样插入（VS Code `ReplacePattern.fromStaticValue`）。
//! - 不认识的转义（`\d`、`$x`、结尾的单个 `\`）按 VS Code 的做法**原样保留**，不报错。
//! - **保留大小写**按匹配到的文本（不是查找串）判断形态，只作用于没有 `$n` 引用的替换串
//!   （VS Code 的 `buildReplaceString` 同此：带参数的替换串不再套用保留大小写）。
//! - 区间全部以**事务开始前**的字节偏移给出，由 #46 的 [`Transaction`] 从后往前应用；零宽匹配
//!   产生空区间（纯插入），首尾相接与同一位置的多次插入都是合法的。

use super::engine::{Haystack, Match, Searcher};
use rove_base::{ByteOffset, Result, Span, TextRange};
use rove_buffer::edit::Replacement;
use rove_buffer::TextSource;
use std::sync::Arc;

/// 取捕获组时一次读入的行数（匹配按位置递增，块沿着匹配向前滑动）。
const CONTEXT_LINES: u64 = 512;

/// 替换串里的一段。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    /// 字面文本（转义已展开）。
    Static(String),
    /// 捕获组引用；`ops` 是紧挨在它前面的大小写操作符。
    Group { index: usize, ops: Vec<CaseOp> },
}

/// `\u`（下一个字符大写）、`\U`（之后全部大写）、`\l`、`\L`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaseOp {
    UpperOne,
    UpperAll,
    LowerOne,
    LowerAll,
}

/// 解析好的替换串。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacePattern {
    pieces: Vec<Piece>,
    has_parameters: bool,
}

impl ReplacePattern {
    /// `regex` 为假时整串原样插入（VS Code 只在正则模式下解析替换串）。
    pub fn parse(template: &str, regex: bool) -> Self {
        if !regex {
            return Self {
                pieces: vec![Piece::Static(template.to_string())],
                has_parameters: false,
            };
        }
        let mut pieces = Vec::new();
        let mut text = String::new();
        let mut ops: Vec<CaseOp> = Vec::new();
        let bytes = template.as_bytes();
        let mut i = 0;
        let flush = |text: &mut String, pieces: &mut Vec<Piece>| {
            if !text.is_empty() {
                pieces.push(Piece::Static(std::mem::take(text)));
            }
        };
        while i < bytes.len() {
            match (bytes[i], bytes.get(i + 1).copied()) {
                (b'\\', Some(b'\\')) => {
                    text.push('\\');
                    i += 2;
                    continue;
                }
                (b'\\', Some(b'n')) => {
                    text.push('\n');
                    i += 2;
                    continue;
                }
                (b'\\', Some(b't')) => {
                    text.push('\t');
                    i += 2;
                    continue;
                }
                (b'\\', Some(op @ (b'u' | b'U' | b'l' | b'L'))) => {
                    ops.push(match op {
                        b'u' => CaseOp::UpperOne,
                        b'U' => CaseOp::UpperAll,
                        b'l' => CaseOp::LowerOne,
                        _ => CaseOp::LowerAll,
                    });
                    i += 2;
                    continue;
                }
                (b'$', Some(b'$')) => {
                    text.push('$');
                    i += 2;
                    continue;
                }
                (b'$', Some(n @ (b'0' | b'&'))) => {
                    let _ = n;
                    flush(&mut text, &mut pieces);
                    pieces.push(Piece::Group {
                        index: 0,
                        ops: std::mem::take(&mut ops),
                    });
                    i += 2;
                    continue;
                }
                (b'$', Some(n @ b'1'..=b'9')) => {
                    let mut index = (n - b'0') as usize;
                    i += 2;
                    if let Some(next @ b'0'..=b'9') = bytes.get(i).copied() {
                        index = index * 10 + (next - b'0') as usize;
                        i += 1;
                    }
                    flush(&mut text, &mut pieces);
                    pieces.push(Piece::Group {
                        index,
                        ops: std::mem::take(&mut ops),
                    });
                    continue;
                }
                _ => {}
            }
            // 不认识的序列原样保留（VS Code 同）。
            let c = template[i..].chars().next().expect("在字符边界上");
            text.push(c);
            i += c.len_utf8();
        }
        flush(&mut text, &mut pieces);
        let has_parameters = pieces.iter().any(|p| matches!(p, Piece::Group { .. }));
        Self {
            pieces,
            has_parameters,
        }
    }

    /// 替换串里有没有 `$n` 引用（有的话每处匹配都要取捕获组）。
    pub fn has_parameters(&self) -> bool {
        self.has_parameters
    }

    /// 展开一处匹配的替换文本。`groups[0]` 是整个匹配，`groups[n]` 是第 n 个捕获组。
    pub fn build(&self, groups: &[Option<&str>], preserve_case: bool) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Static(s) => out.push_str(s),
                Piece::Group { index, ops } => {
                    let value = groups.get(*index).copied().flatten().unwrap_or("");
                    if ops.is_empty() {
                        out.push_str(value);
                    } else {
                        out.push_str(&apply_case_ops(value, ops));
                    }
                }
            }
        }
        if preserve_case && !self.has_parameters {
            let matched = groups.first().copied().flatten().unwrap_or("");
            return preserve_case_text(matched, &out);
        }
        out
    }
}

/// VS Code `buildReplaceString` 里的大小写操作符：`\u` / `\l` 只作用于一个字符，`\U` / `\L`
/// 一直作用到操作符用完为止。
fn apply_case_ops(value: &str, ops: &[CaseOp]) -> String {
    let mut out = String::with_capacity(value.len());
    let mut op = 0usize;
    for c in value.chars() {
        match ops.get(op) {
            None => out.push(c),
            Some(CaseOp::UpperAll) => out.extend(c.to_uppercase()),
            Some(CaseOp::LowerAll) => out.extend(c.to_lowercase()),
            Some(CaseOp::UpperOne) => {
                out.extend(c.to_uppercase());
                op += 1;
            }
            Some(CaseOp::LowerOne) => {
                out.extend(c.to_lowercase());
                op += 1;
            }
        }
    }
    out
}

/// VS Code `buildReplaceStringWithCasePreserved`：按**匹配到的文本**的形态改写替换串。
/// 两边的连字符 / 下划线段数相同时逐段处理（`foo-bar` → `FOO-BAR` 这类命名风格）。
pub fn preserve_case_text(matched: &str, pattern: &str) -> String {
    if matched.is_empty() {
        return pattern.to_string();
    }
    let hyphen = segments_align(matched, pattern, '-');
    let underscore = segments_align(matched, pattern, '_');
    if hyphen && !underscore {
        return preserve_case_by_segment(matched, pattern, '-');
    }
    if !hyphen && underscore {
        return preserve_case_by_segment(matched, pattern, '_');
    }
    if matched.to_uppercase() == matched {
        pattern.to_uppercase()
    } else if matched.to_lowercase() == matched {
        pattern.to_lowercase()
    } else if first_is_uppercase(matched) && !pattern.is_empty() {
        let mut chars = pattern.chars();
        let first = chars.next().expect("非空");
        first.to_uppercase().chain(chars).collect()
    } else {
        // 形态看不出来：原样。
        pattern.to_string()
    }
}

fn first_is_uppercase(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|c| c.to_lowercase().next() != Some(c))
}

fn segments_align(matched: &str, pattern: &str, separator: char) -> bool {
    matched.contains(separator)
        && pattern.contains(separator)
        && matched.split(separator).count() == pattern.split(separator).count()
}

fn preserve_case_by_segment(matched: &str, pattern: &str, separator: char) -> String {
    matched
        .split(separator)
        .zip(pattern.split(separator))
        .map(|(m, p)| preserve_case_text(m, p))
        .collect::<Vec<_>>()
        .join(&separator.to_string())
}

/// 为每一处匹配算出替换区间与文本，结果可以直接交给 [`Transaction::new`](rove_buffer::edit::Transaction::new)
/// 做成**一个**撤销单元。`matches` 必须按位置升序且互不重叠（#21 的匹配列表就是）。
///
/// 替换串没有 `$n` 引用也不保留大小写时不读一个字节的文本（共用同一个 `Arc<str>`），
/// 所以几千处替换也只是几千个区间对象。
pub fn plan_replacements(
    text: &dyn TextSource,
    searcher: &Searcher,
    matches: &[Match],
    pattern: &ReplacePattern,
    preserve_case: bool,
) -> Result<Vec<Replacement>> {
    let needs_text = pattern.has_parameters() || preserve_case;
    let plain: Option<Arc<str>> = (!needs_text).then(|| Arc::from(pattern.build(&[], false)));
    let known = text.lines().known.max(1);
    let mut block: Option<Haystack> = None;
    let mut out = Vec::with_capacity(matches.len());
    for m in matches {
        let range = Span::new(text.offset(m.start)?, text.offset(m.end)?)?;
        let replacement: Arc<str> = match &plain {
            Some(t) => t.clone(),
            None => {
                let hay = context_block(text, &mut block, m, known)?;
                let (from, to) = (hay.offset(m.start), hay.offset(m.end));
                let groups = searcher.capture_texts(hay, from..to);
                Arc::from(pattern.build(&groups, preserve_case))
            }
        };
        out.push(Replacement::new(range, replacement));
    }
    Ok(out)
}

/// 保证当前块覆盖 `m` 的所有行；匹配按位置递增，所以块只会向前滑动。
fn context_block<'b>(
    text: &dyn TextSource,
    block: &'b mut Option<Haystack>,
    m: &Match,
    known: u64,
) -> Result<&'b Haystack> {
    let covered = block.as_ref().is_some_and(|hay| {
        let first = hay.first_line();
        m.start.line.0 >= first && m.end.line.0 < first + hay.line_count() as u64
    });
    if !covered {
        let first = m.start.line.0.min(known - 1);
        let end = (m.end.line.0 + 1).max(first + CONTEXT_LINES).min(known);
        *block = Some(Haystack::read(text, first, end.max(first + 1))?);
    }
    Ok(block.as_ref().expect("刚刚读过"))
}

/// “在选区内查找”的范围里，各处替换一共让文本长了（正）或短了（负）多少字节。
/// `scope_end` 是**替换前**范围终点的字节偏移。
pub fn scope_delta(scope_end: ByteOffset, edits: &[Replacement]) -> i64 {
    let mut delta: i64 = 0;
    for edit in edits {
        if edit.range.start() >= scope_end {
            break;
        }
        delta += edit.text.len() as i64 - (edit.range.end().0 - edit.range.start().0) as i64;
    }
    delta
}

/// 范围在一次替换之后的新位置：起点不动（所有替换都在范围内），终点按 [`scope_delta`] 平移，
/// 再用替换后的文本换算回行列。
pub fn shifted_scope(
    start: ByteOffset,
    end: ByteOffset,
    delta: i64,
    after: &dyn TextSource,
) -> Result<TextRange> {
    let shifted = (end.0 as i64 + delta).max(start.0 as i64) as u64;
    let clamped = ByteOffset(shifted.min(after.byte_len()));
    Span::new(after.position(start)?, after.position(clamped)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::find::engine::{FindOptions, Query};
    use rove_base::{Cancellation, Line, Position, Utf8Column};
    use rove_buffer::edit::{EditBuffer, EditOptions, EditSession, Transaction};
    use rove_buffer::{Eol, LineEndings, RopeText};

    const SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

    fn searcher(text: &str, options: FindOptions) -> Searcher {
        Searcher::compile(&Query {
            text: text.into(),
            options,
            word_separators: SEPARATORS.into(),
        })
        .unwrap()
        .unwrap()
    }

    fn regex(pattern: &str) -> Searcher {
        searcher(
            pattern,
            FindOptions {
                regex: true,
                case_sensitive: true,
                ..Default::default()
            },
        )
    }

    fn matches(text: &RopeText, searcher: &Searcher) -> Vec<Match> {
        crate::find::engine::search_all(
            text,
            searcher,
            None,
            usize::MAX,
            &Cancellation::default(),
            &mut |_| {},
        )
        .unwrap()
        .matches
    }

    /// 整篇替换一次，返回新文本（事务是一个撤销单元）。
    fn replace_all(doc: &str, searcher: &Searcher, template: &str, preserve_case: bool) -> String {
        let text = RopeText::from_text(doc);
        let found = matches(&text, searcher);
        let pattern = ReplacePattern::parse(template, true);
        let edits = plan_replacements(&text, searcher, &found, &pattern, preserve_case).unwrap();
        apply(doc, edits)
    }

    /// 一个事务替换全部区间，返回新文本（替换串里的 `\n` 按文档换行符规整）。
    fn apply(doc: &str, edits: Vec<Replacement>) -> String {
        let buffer = EditBuffer::new(doc, LineEndings::Uniform(Eol::Lf));
        let mut session = EditSession::new(buffer, EditOptions::default());
        session
            .edit(Transaction::new(edits).unwrap(), std::time::Instant::now())
            .unwrap();
        read(&session.snapshot())
    }

    fn read(text: &RopeText) -> String {
        let mut out = String::new();
        text.visit_chunks(
            Span::new(
                rove_base::ByteOffset(0),
                rove_base::ByteOffset(text.byte_len()),
            )
            .unwrap(),
            &mut |s| out.push_str(s),
        )
        .unwrap();
        out
    }

    #[test]
    fn plain_text_replacement_is_inserted_verbatim() {
        // 普通文本模式不解析 `$1` 与转义（VS Code 同）。
        let pattern = ReplacePattern::parse("a$1\\n$$", false);
        assert!(!pattern.has_parameters());
        assert_eq!(pattern.build(&[Some("x")], false), "a$1\\n$$");
    }

    #[test]
    fn regex_replacement_expands_groups_escapes_and_case_operators() {
        let p = |t: &str| ReplacePattern::parse(t, true);
        let groups = [Some("Foo Bar"), Some("Foo"), Some("Bar")];
        assert_eq!(p("$2 $1").build(&groups, false), "Bar Foo");
        assert_eq!(p("[$&]").build(&groups, false), "[Foo Bar]");
        assert_eq!(p("[$0]").build(&groups, false), "[Foo Bar]");
        assert_eq!(p("$$1").build(&groups, false), "$1");
        assert_eq!(p("a\\nb\\tc\\\\d").build(&groups, false), "a\nb\tc\\d");
        // 没有参与的组与越界的组都展开成空串（VS Code 同）。
        assert_eq!(p("<$9>").build(&groups, false), "<>");
        // 不认识的转义与 `$` 用法原样保留。
        assert_eq!(p("\\d$x\\").build(&groups, false), "\\d$x\\");
        // 大小写操作符只作用于紧随其后的 `$n`。
        assert_eq!(p("\\u$2").build(&groups, false), "Bar");
        assert_eq!(p("\\U$2").build(&groups, false), "BAR");
        assert_eq!(p("\\L$1").build(&groups, false), "foo");
        assert_eq!(p("\\l$1").build(&groups, false), "foo");
        // 两位数的组号。
        let many: Vec<Option<&str>> = (0..13)
            .map(|i| Some(["m", "", "", "", "", "", "", "", "", "", "", "", "twelve"][i]))
            .collect();
        assert_eq!(p("$12").build(&many, false), "twelve");
    }

    #[test]
    fn preserve_case_follows_the_matched_text() {
        // 四种源文本形态（验收口径）。
        assert_eq!(preserve_case_text("FOO", "bar"), "BAR");
        assert_eq!(preserve_case_text("Foo", "bar"), "Bar");
        assert_eq!(preserve_case_text("foo", "BAR"), "bar");
        assert_eq!(preserve_case_text("fOo", "bar"), "bar");
        // 段数相同的连字符 / 下划线命名逐段处理。
        assert_eq!(preserve_case_text("Foo-BAR", "one-two"), "One-TWO");
        assert_eq!(preserve_case_text("FOO_bar", "one_two"), "ONE_two");
        // 段数不同就退回整体判断。
        assert_eq!(preserve_case_text("FOO-BAR-BAZ", "one-two"), "ONE-TWO");
        assert_eq!(preserve_case_text("", "bar"), "bar");
    }

    #[test]
    fn preserve_case_only_applies_to_patterns_without_parameters() {
        let plain = ReplacePattern::parse("bar", true);
        assert_eq!(plain.build(&[Some("FOO")], true), "BAR");
        let parameterized = ReplacePattern::parse("$1x", true);
        assert_eq!(parameterized.build(&[Some("FOO"), Some("FO")], true), "FOx");
    }

    #[test]
    fn replace_all_rewrites_every_match_in_one_transaction() {
        let doc = "foo bar foo\nFoo baz\n";
        let found = matches(
            &RopeText::from_text(doc),
            &searcher("foo", FindOptions::default()),
        );
        assert_eq!(found.len(), 3);
        let text = RopeText::from_text(doc);
        let s = searcher("foo", FindOptions::default());
        let pattern = ReplacePattern::parse("qux", false);
        let edits = plan_replacements(&text, &s, &found, &pattern, false).unwrap();
        assert_eq!(edits.len(), 3);
        assert_eq!(apply(doc, edits), "qux bar qux\nqux baz\n");
    }

    #[test]
    fn regex_capture_groups_are_taken_from_the_match_being_replaced() {
        let doc = "int a = 1;\nint bb = 22;\n";
        let out = replace_all(doc, &regex(r"(\w+) = (\d+)"), "$2 -> $1", false);
        assert_eq!(out, "int 1 -> a;\nint 22 -> bb;\n");
    }

    #[test]
    fn multiline_regex_replacement_spans_lines() {
        let doc = "a\nb\nc\n";
        let out = replace_all(doc, &regex("a\\nb"), "x\\ny", false);
        assert_eq!(out, "x\ny\nc\n");
        // 跨行捕获组。
        let out = replace_all(doc, &regex("(a)\\n(b)"), "$2\\n$1", false);
        assert_eq!(out, "b\na\nc\n");
    }

    #[test]
    fn zero_width_matches_become_pure_insertions() {
        let doc = "ab\ncd\n";
        // 文档末尾还有一行空行，`^` 在那里也有一处零宽匹配（VS Code 同）。
        let out = replace_all(doc, &regex("^"), "> ", false);
        assert_eq!(out, "> ab\n> cd\n> ");
        // `a*` 在 "ab" 上：匹配 "a"（0..1）、b 之前与行尾的两处空匹配，末行再一处。
        let out = replace_all("ab\n", &regex("a*"), "-", false);
        assert_eq!(out, "--b-\n-");
    }

    #[test]
    fn preserve_case_in_replace_all_uses_each_match() {
        let doc = "foo FOO Foo fOo\n";
        let out = replace_all(doc, &searcher("foo", FindOptions::default()), "bar", true);
        assert_eq!(out, "bar BAR Bar bar\n");
    }

    #[test]
    fn crlf_and_mixed_newlines_survive_a_replacement() {
        let doc = "foo\r\nfoo\nfoo\r\n";
        let text = RopeText::from_text(doc);
        let s = searcher("foo", FindOptions::default());
        let found = matches(&text, &s);
        assert_eq!(found.len(), 3);
        let pattern = ReplacePattern::parse("bar", false);
        let edits = plan_replacements(&text, &s, &found, &pattern, false).unwrap();
        assert_eq!(apply(doc, edits), "bar\r\nbar\nbar\r\n");
    }

    #[test]
    fn scope_end_follows_the_replacements_inside_it() {
        let doc = "foo foo\nfoo\n";
        let text = RopeText::from_text(doc);
        let s = searcher("foo", FindOptions::default());
        let scope = Span::new(
            Position {
                line: Line(0),
                column: Utf8Column(0),
            },
            Position {
                line: Line(0),
                column: Utf8Column(7),
            },
        )
        .unwrap();
        let found = crate::find::engine::search_all(
            &text,
            &s,
            Some(&scope),
            usize::MAX,
            &Cancellation::default(),
            &mut |_| {},
        )
        .unwrap()
        .matches;
        assert_eq!(found.len(), 2);
        let pattern = ReplacePattern::parse("quux", false);
        let edits = plan_replacements(&text, &s, &found, &pattern, false).unwrap();
        let after = RopeText::from_text(&apply(doc, edits.clone()));
        assert_eq!(read(&after), "quux quux\nfoo\n");
        let end = text.offset(scope.end()).unwrap();
        let delta = scope_delta(end, &edits);
        let moved = shifted_scope(text.offset(scope.start()).unwrap(), end, delta, &after).unwrap();
        assert_eq!(moved.start().column.0, 0);
        assert_eq!(moved.end().column.0, 9);
    }
}
