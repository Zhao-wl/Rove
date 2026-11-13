//! 查找引擎（#21）：把查询编译成 [`Searcher`]，再按“整行块”流式扫描 [`TextSource`]。
//!
//! 不依赖 GPUI，可在后台线程运行；#22（超大文件的流式查找）与 #48（替换）直接复用这里的
//! [`search_all`]（带进度回调的全量扫描）与 [`find_next`]（从某位置起的导航）。
//!
//! # 语义（对齐 VS Code `TextModelSearch`）
//!
//! - 搜索文本把 `\r\n` 规整为 `\n`；列号始终是源行内的 UTF-8 字节列，与 `TextSource` 一致。
//! - 普通文本（含忽略大小写、全词）在整块上直接查找；正则按 JavaScript 语义（复用 #12 的
//!   [`CompiledRegex`]，标志 `mu` + 可选 `i`）。源码里不含 `\n`、`\r`、`\W` 的正则按**行**匹配
//!   （`^` / `$` 是行首行尾），否则在整个范围的文本上匹配，可以跨行。
//! - 全词用 `editor.wordSeparators` 判定边界（与 VS Code `isValidMatch` 相同）。
//! - 匹配不重叠；空匹配（如 `^`）保留，并向前推进一个字符。
//! - 块大小约 [`BLOCK_BYTES`]；多行正则需要整个范围一次读入，超过 [`MULTILINE_LIMIT_BYTES`]
//!   返回 `Unsupported`（超大文件由 #22 的流式查找处理）。

use fancy_regex::Regex;
use memchr::{memchr, memchr_iter, memmem};
use rove_base::{Cancellation, Error, Line, Position, Result, Span, TextRange, Utf8Column};
use rove_buffer::TextSource;
use rove_syntax::language_config::{CompiledRegex, RegexOrigin};
use rove_syntax::JsRegex;
use std::ops::Range;
use std::sync::Arc;

/// 单块目标字节数：足够大以摊薄每次调用的开销，又小到取消响应及时。
pub const BLOCK_BYTES: usize = 1 << 20;
/// 多行正则一次读入的字节上限。
pub const MULTILINE_LIMIT_BYTES: u64 = 128 << 20;
/// 与 VS Code 相同的匹配数上限：超过后只保留前 `MATCHES_LIMIT` 个并标记“超过上限”。
pub const MATCHES_LIMIT: usize = 19999;

/// 一处匹配：`[start, end)`，`end` 可以在后面的行（多行匹配）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    pub start: Position,
    pub end: Position,
}

impl Match {
    pub fn range(&self) -> TextRange {
        Span::new(self.start, self.end).expect("match is ordered")
    }
}

/// 三个“查找选项”开关（在选区内查找是范围，不在这里）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// 一次查询：文本 + 选项 + 单词分隔符。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub options: FindOptions,
    pub word_separators: Arc<str>,
}

/// 查询本身的问题（界面显示为输入框校验错误，而不是静默无结果）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryError {
    /// 正则无法编译；附编译器的原因。
    InvalidRegex(String),
}

enum Kind {
    /// 区分大小写的普通文本。
    Literal(memmem::Finder<'static>),
    /// 忽略大小写的普通文本（转义后的正则）。
    LiteralFolded(Regex),
    /// JavaScript 正则。
    Js { re: CompiledRegex, multiline: bool },
}

/// 编译后的查询，可跨线程共享。
pub struct Searcher {
    kind: Kind,
    whole_word: bool,
    separators: Arc<str>,
}

impl Searcher {
    /// 编译查询；空文本返回 `Ok(None)`（没有可查找的内容）。
    pub fn compile(query: &Query) -> std::result::Result<Option<Searcher>, QueryError> {
        if query.text.is_empty() {
            return Ok(None);
        }
        let kind = if query.options.regex {
            let flags = if query.options.case_sensitive {
                "mu"
            } else {
                "imu"
            };
            let js = JsRegex {
                source: query.text.as_str().into(),
                flags: flags.into(),
            };
            let origin = RegexOrigin::new("find", "find-widget", "search");
            let re = CompiledRegex::compile(&js, &origin)
                .map_err(|e| QueryError::InvalidRegex(e.reason.to_string()))?;
            Kind::Js {
                re,
                multiline: is_multiline_regex_source(&query.text),
            }
        } else if query.options.case_sensitive {
            Kind::Literal(memmem::Finder::new(query.text.as_bytes()).into_owned())
        } else {
            let pattern = format!("(?i){}", fancy_regex::escape(&query.text));
            Kind::LiteralFolded(
                Regex::new(&pattern).map_err(|e| QueryError::InvalidRegex(e.to_string()))?,
            )
        };
        Ok(Some(Searcher {
            kind,
            whole_word: query.options.whole_word,
            separators: query.word_separators.clone(),
        }))
    }

    /// 匹配可能跨行（普通文本含换行，或正则含 `\n` / `\r` / `\W`）：需要整个范围的文本。
    pub fn is_multiline(&self) -> bool {
        match &self.kind {
            Kind::Literal(f) => f.needle().contains(&b'\n'),
            Kind::LiteralFolded(_) => false,
            Kind::Js { multiline, .. } => *multiline,
        }
    }

    /// 在 `hay` 里从字节偏移 `from` 起依次回调每个匹配（`hay.text` 内的字节范围）；
    /// 回调返回 `false` 停止。
    pub fn scan(&self, hay: &Haystack, from: usize, f: &mut dyn FnMut(Range<usize>) -> bool) {
        let text = hay.text.as_str();
        let mut emit = |s: usize, e: usize| -> bool {
            if self.whole_word && !self.is_word_bounded(text, s, e) {
                return true;
            }
            f(s..e)
        };
        match &self.kind {
            Kind::Literal(finder) => {
                if from > text.len() {
                    return;
                }
                for at in finder.find_iter(&text.as_bytes()[from..]) {
                    let s = from + at;
                    if !emit(s, s + finder.needle().len()) {
                        return;
                    }
                }
            }
            Kind::LiteralFolded(re) => {
                let mut at = from;
                while at <= text.len() {
                    match re.find_from_pos(text, at) {
                        Ok(Some(m)) => {
                            at = if m.end() > m.start() {
                                m.end()
                            } else {
                                next_char(text, m.end())
                            };
                            if !emit(m.start(), m.end()) {
                                return;
                            }
                        }
                        _ => return,
                    }
                }
            }
            Kind::Js {
                re,
                multiline: true,
            } => {
                let mut at = from;
                while at <= text.len() {
                    match re.find_from(text, at) {
                        Ok(Some((s, e))) => {
                            at = if e > s { e } else { next_char(text, e) };
                            if !emit(s, e) {
                                return;
                            }
                        }
                        _ => return,
                    }
                }
            }
            Kind::Js {
                re,
                multiline: false,
            } => {
                let mut line = hay.line_index_at(from);
                while line < hay.line_count() {
                    let start = hay.line_starts[line];
                    let line_text = hay.line_text(line);
                    let mut at = from.saturating_sub(start).min(line_text.len());
                    if from > start + line_text.len() {
                        line += 1;
                        continue;
                    }
                    while at <= line_text.len() {
                        match re.find_from(line_text, at) {
                            Ok(Some((s, e))) => {
                                at = if e > s { e } else { next_char(line_text, e) };
                                if !emit(start + s, start + e) {
                                    return;
                                }
                            }
                            _ => break,
                        }
                    }
                    line += 1;
                }
            }
        }
    }

    /// 一处匹配（块内字节区间 `range`，必须来自 [`Searcher::scan`]）的捕获组文本：组 0 是整个
    /// 匹配，1 号起只有正则查询才有，`None` 表示该组没有参与。按与 [`Searcher::scan`] 完全相同的
    /// 口径重跑一次正则（逐行匹配的查询在本行上跑，跨行查询在整块上跑），重跑的起点与终点对不上
    /// 时只给出组 0，不猜测。#48 的正则替换用它展开 `$1`…`$9`，不另写一套匹配。
    pub fn capture_texts<'t>(
        &self,
        hay: &'t Haystack,
        range: Range<usize>,
    ) -> Vec<Option<&'t str>> {
        let whole = vec![hay.text.get(range.clone())];
        let Kind::Js { re, multiline } = &self.kind else {
            return whole;
        };
        let (text, base) = if *multiline {
            (hay.text.as_str(), 0)
        } else {
            let line = hay.line_index_at(range.start);
            (hay.line_text(line), hay.line_starts[line])
        };
        let (Some(from), Some(to)) = (
            range.start.checked_sub(base),
            range.end.checked_sub(base).filter(|to| *to <= text.len()),
        ) else {
            return whole;
        };
        match re.capture_ranges(text, from) {
            Ok(Some(groups)) if groups.first().copied().flatten() == Some((from, to)) => groups
                .into_iter()
                .map(|g| g.map(|(s, e)| &text[s..e]))
                .collect(),
            _ => whole,
        }
    }

    /// VS Code `isValidMatch`：全词时匹配两端必须落在单词边界上。
    fn is_word_bounded(&self, text: &str, start: usize, end: usize) -> bool {
        let regular = |c: char| c != ' ' && c != '\t' && !self.separators.contains(c);
        let left = start == 0 || {
            let before = text[..start].chars().next_back().unwrap();
            !regular(before)
                || before == '\n'
                || before == '\r'
                || (end > start && !regular(text[start..].chars().next().unwrap()))
        };
        let right = end == text.len() || {
            let after = text[end..].chars().next().unwrap();
            !regular(after)
                || after == '\n'
                || after == '\r'
                || (end > start && !regular(text[..end].chars().next_back().unwrap()))
        };
        left && right
    }
}

fn next_char(text: &str, at: usize) -> usize {
    match text[at.min(text.len())..].chars().next() {
        Some(c) => at + c.len_utf8(),
        None => text.len() + 1,
    }
}

/// VS Code `isMultilineRegexSource`：含 `\n`、`\r`、`\W` 或字面换行。
pub fn is_multiline_regex_source(source: &str) -> bool {
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        match c {
            '\n' | '\r' => return true,
            '\\' => {
                if matches!(chars.next(), Some('n' | 'r' | 'W')) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// 一块整行文本（`\r\n` 已规整为 `\n`）。偏移 ↔ 位置换算 O(log 行数)。
pub struct Haystack {
    pub text: String,
    line_starts: Vec<usize>,
    first_line: u64,
}

impl Haystack {
    /// 读取 `[first, end)` 行（至少一行）。
    pub fn read(text: &dyn TextSource, first: u64, end: u64) -> Result<Haystack> {
        debug_assert!(first < end);
        let start = text.line_range(Line(first))?.start();
        let stop = text.line_range(Line(end - 1))?.end();
        let mut out = String::with_capacity((stop.0 - start.0) as usize);
        let mut pending_cr = false;
        text.visit_chunks(Span::new(start, stop)?, &mut |chunk| {
            if pending_cr {
                if !chunk.starts_with('\n') {
                    out.push('\r');
                }
                pending_cr = false;
            }
            let (body, cr) = match chunk.strip_suffix('\r') {
                Some(body) => (body, true),
                None => (chunk, false),
            };
            if memchr(b'\r', body.as_bytes()).is_none() {
                out.push_str(body);
            } else {
                out.push_str(&body.replace("\r\n", "\n"));
            }
            pending_cr = cr;
        })?;
        if pending_cr {
            out.push('\r');
        }
        let mut line_starts = vec![0];
        line_starts.extend(memchr_iter(b'\n', out.as_bytes()).map(|i| i + 1));
        Ok(Haystack {
            text: out,
            line_starts,
            first_line: first,
        })
    }

    /// 直接用字符串建（测试与小文本）。
    pub fn from_str(text: &str, first_line: u64) -> Haystack {
        let mut line_starts = vec![0];
        line_starts.extend(memchr_iter(b'\n', text.as_bytes()).map(|i| i + 1));
        Haystack {
            text: text.to_string(),
            line_starts,
            first_line,
        }
    }

    pub fn first_line(&self) -> u64 {
        self.first_line
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn line_index_at(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= offset) - 1
    }

    pub fn line_text(&self, index: usize) -> &str {
        let start = self.line_starts[index];
        let end = match self.line_starts.get(index + 1) {
            Some(next) => next - 1,
            None => self.text.len(),
        };
        &self.text[start..end]
    }

    pub fn position(&self, offset: usize) -> Position {
        let i = self.line_index_at(offset);
        Position {
            line: Line(self.first_line + i as u64),
            column: Utf8Column((offset - self.line_starts[i]) as u64),
        }
    }

    /// 位置 → 块内偏移；位置在块之前为 0，之后为文本末尾。
    pub fn offset(&self, p: Position) -> usize {
        if p.line.0 < self.first_line {
            return 0;
        }
        let i = (p.line.0 - self.first_line) as usize;
        if i >= self.line_starts.len() {
            return self.text.len();
        }
        let start = self.line_starts[i];
        (start + p.column.0 as usize).min(start + self.line_text(i).len())
    }

    fn to_match(&self, r: Range<usize>) -> Match {
        Match {
            start: self.position(r.start),
            end: self.position(r.end),
        }
    }
}

/// 全量扫描的进度（每读完一块回调一次；#22 用它显示“已找到 N 处 · 已搜索 37%”）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub found: usize,
    pub scanned_bytes: u64,
    pub total_bytes: u64,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        if self.total_bytes == 0 {
            1.0
        } else {
            (self.scanned_bytes as f64 / self.total_bytes as f64).min(1.0) as f32
        }
    }
}

/// 全量扫描结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchOutput {
    /// 按位置排序，最多 `limit` 个。
    pub matches: Vec<Match>,
    /// 还有更多匹配没有收录（超过上限）。
    pub limit_exceeded: bool,
    /// 行索引尚未建完，只扫描了已知的行（超大文件，#22）。
    pub partial: bool,
}

/// 搜索的行范围 `[first, end)`；`scope` 给定时限制在选区的行上。
fn line_span(text: &dyn TextSource, scope: Option<&TextRange>) -> (u64, u64, bool) {
    let counts = text.lines();
    let known = counts.known.max(1);
    match scope {
        Some(s) => (
            s.start().line.0.min(known - 1),
            (s.end().line.0 + 1).min(known),
            !counts.complete,
        ),
        None => (0, known, !counts.complete),
    }
}

fn in_scope(m: &Match, scope: Option<&TextRange>) -> bool {
    scope.is_none_or(|s| m.start >= s.start() && m.end <= s.end())
}

fn span_bytes(text: &dyn TextSource, first: u64, end: u64) -> Result<(u64, u64)> {
    let a = text.line_range(Line(first))?.start().0;
    let b = text.line_range(Line(end - 1))?.end().0;
    Ok((a, b))
}

/// 在 `[first, end)` 行上按块读取并依次回调；回调返回 `false` 结束。多行查询只读一块。
fn for_each_block(
    text: &dyn TextSource,
    searcher: &Searcher,
    first: u64,
    end: u64,
    cancel: &Cancellation,
    f: &mut dyn FnMut(&Haystack) -> bool,
) -> Result<()> {
    if first >= end {
        return Ok(());
    }
    if searcher.is_multiline() {
        let (a, b) = span_bytes(text, first, end)?;
        if b - a > MULTILINE_LIMIT_BYTES {
            return Err(Error::Unsupported);
        }
        cancel.check()?;
        f(&Haystack::read(text, first, end)?);
        return Ok(());
    }
    let mut lines = 4096u64;
    let mut cur = first;
    while cur < end {
        cancel.check()?;
        let stop = (cur + lines).min(end);
        let hay = Haystack::read(text, cur, stop)?;
        let avg = (hay.text.len() as u64 / (stop - cur)).max(1);
        lines = (BLOCK_BYTES as u64 / avg).clamp(1, 1 << 22);
        if !f(&hay) {
            return Ok(());
        }
        cur = stop;
    }
    Ok(())
}

/// 全量扫描：按块依次产出匹配（最多 `limit` 个，多一个即标记超限）。
pub fn search_all(
    text: &dyn TextSource,
    searcher: &Searcher,
    scope: Option<&TextRange>,
    limit: usize,
    cancel: &Cancellation,
    progress: &mut dyn FnMut(Progress),
) -> Result<SearchOutput> {
    let (first, end, partial) = line_span(text, scope);
    let (span_start, span_end) = span_bytes(text, first, end)?;
    let total = span_end - span_start;
    let mut out = SearchOutput {
        partial,
        ..Default::default()
    };
    let mut scanned = 0u64;
    for_each_block(text, searcher, first, end, cancel, &mut |hay| {
        searcher.scan(hay, 0, &mut |r| {
            let m = hay.to_match(r);
            if !in_scope(&m, scope) {
                return true;
            }
            if out.matches.len() >= limit {
                out.limit_exceeded = true;
                return false;
            }
            out.matches.push(m);
            true
        });
        scanned += hay.text.len() as u64 + 1;
        progress(Progress {
            found: out.matches.len(),
            scanned_bytes: scanned.min(total),
            total_bytes: total,
        });
        !out.limit_exceeded
    })?;
    Ok(out)
}

/// 导航结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub found: Match,
    /// 越过了文件（或选区）末尾 / 开头后从另一端找到。
    pub wrapped: bool,
}

/// 从 `from` 起找下一个（`forward`）或上一个匹配。下一个：第一个起点 ≥ `from`；上一个：最后一个起点 < `from`。
/// `wrap` 为 `false` 时到头即返回 `None`（`editor.find.loop` 关闭）。
pub fn find_next(
    text: &dyn TextSource,
    searcher: &Searcher,
    scope: Option<&TextRange>,
    from: Position,
    forward: bool,
    wrap: bool,
    cancel: &Cancellation,
) -> Result<Option<Step>> {
    let (first, end, _) = line_span(text, scope);
    let lo = scope.map_or(Position::default(), |s| s.start());
    let hi = scope.map_or(
        Position {
            line: Line(end.saturating_sub(1)),
            column: Utf8Column(u64::MAX),
        },
        |s| s.end(),
    );
    let from = from.clamp(lo, hi);
    // 先找 [from, hi]，再（允许时）找 [lo, from)。
    let regions: [(Position, Position, bool); 2] = if forward {
        [(from, hi, false), (lo, from, true)]
    } else {
        [(lo, from, false), (from, hi, true)]
    };
    for (i, (a, b, wrapped)) in regions.into_iter().enumerate() {
        if i == 1 && !wrap {
            break;
        }
        // 到达范围末端的一侧包含 `hi`（空匹配可以起于末端）。
        let region = Region {
            a,
            b,
            inclusive: b == hi,
        };
        let found = if forward {
            first_in(text, searcher, scope, (first, end), region, cancel)?
        } else {
            last_in(text, searcher, scope, (first, end), region, cancel)?
        };
        if let Some(found) = found {
            return Ok(Some(Step { found, wrapped }));
        }
    }
    Ok(None)
}

/// 导航时关心的起点范围：`[a, b)`，`inclusive` 时含 `b`。
#[derive(Clone, Copy)]
struct Region {
    a: Position,
    b: Position,
    inclusive: bool,
}

impl Region {
    fn wants(&self, m: &Match, scope: Option<&TextRange>) -> bool {
        m.start >= self.a
            && (m.start < self.b || (self.inclusive && m.start == self.b))
            && in_scope(m, scope)
    }

    /// 起点已经越过本范围：后面的匹配都不需要了。
    fn passed(&self, m: &Match) -> bool {
        m.start > self.b || (m.start == self.b && !self.inclusive)
    }
}

/// 起点在 `region` 内的第一个匹配。
fn first_in(
    text: &dyn TextSource,
    searcher: &Searcher,
    scope: Option<&TextRange>,
    (first, end): (u64, u64),
    region: Region,
    cancel: &Cancellation,
) -> Result<Option<Match>> {
    let mut found = None;
    let start_line = region.a.line.0.max(first);
    let stop_line = (region.b.line.0 + 1).min(end);
    for_each_block(text, searcher, start_line, stop_line, cancel, &mut |hay| {
        searcher.scan(hay, hay.offset(region.a), &mut |r| {
            let m = hay.to_match(r);
            if region.wants(&m, scope) {
                found = Some(m);
                return false;
            }
            !region.passed(&m)
        });
        found.is_none()
    })?;
    Ok(found)
}

/// 起点在 `region` 内的最后一个匹配（从后向前按块读取，找到即停）。
fn last_in(
    text: &dyn TextSource,
    searcher: &Searcher,
    scope: Option<&TextRange>,
    (first, end): (u64, u64),
    region: Region,
    cancel: &Cancellation,
) -> Result<Option<Match>> {
    let lo = region.a.line.0.max(first);
    let hi = (region.b.line.0 + 1).min(end);
    if lo >= hi {
        return Ok(None);
    }
    let scan_block = |hay: &Haystack| {
        let mut last = None;
        searcher.scan(hay, 0, &mut |r| {
            let m = hay.to_match(r);
            if region.wants(&m, scope) {
                last = Some(m);
            }
            !region.passed(&m)
        });
        last
    };
    if searcher.is_multiline() {
        let mut found = None;
        for_each_block(text, searcher, lo, hi, cancel, &mut |hay| {
            found = scan_block(hay);
            false
        })?;
        return Ok(found);
    }
    let mut stop = hi;
    let mut lines = 4096u64;
    while stop > lo {
        cancel.check()?;
        let start = stop.saturating_sub(lines).max(lo);
        let hay = Haystack::read(text, start, stop)?;
        if let Some(found) = scan_block(&hay) {
            return Ok(Some(found));
        }
        let avg = (hay.text.len() as u64 / (stop - start)).max(1);
        lines = (BLOCK_BYTES as u64 / avg).clamp(1, 1 << 22);
        stop = start;
    }
    Ok(None)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rove_buffer::RopeText;

    pub(crate) fn pos(line: u64, col: u64) -> Position {
        Position {
            line: Line(line),
            column: Utf8Column(col),
        }
    }

    fn query(text: &str, case: bool, word: bool, regex: bool) -> Query {
        Query {
            text: text.into(),
            options: FindOptions {
                case_sensitive: case,
                whole_word: word,
                regex,
            },
            word_separators: "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".into(),
        }
    }

    fn all(doc: &str, q: Query, scope: Option<&TextRange>) -> Result<SearchOutput> {
        let rope = RopeText::from_text(doc);
        let s = Searcher::compile(&q).unwrap().unwrap();
        search_all(
            &rope,
            &s,
            scope,
            usize::MAX,
            &Cancellation::default(),
            &mut |_| {},
        )
    }

    fn spans(out: &SearchOutput) -> Vec<(u64, u64, u64, u64)> {
        out.matches
            .iter()
            .map(|m| {
                (
                    m.start.line.0,
                    m.start.column.0,
                    m.end.line.0,
                    m.end.column.0,
                )
            })
            .collect()
    }

    #[test]
    fn literal_matches_case_and_whole_word() {
        let doc = "Foo foo FOO\nfoobar foo.bar\n";
        let out = all(doc, query("foo", true, false, false), None).unwrap();
        assert_eq!(spans(&out), [(0, 4, 0, 7), (1, 0, 1, 3), (1, 7, 1, 10)]);
        let out = all(doc, query("foo", false, false, false), None).unwrap();
        assert_eq!(out.matches.len(), 5);
        let out = all(doc, query("foo", false, true, false), None).unwrap();
        assert_eq!(
            spans(&out),
            [(0, 0, 0, 3), (0, 4, 0, 7), (0, 8, 0, 11), (1, 7, 1, 10)]
        );
    }

    #[test]
    fn whole_word_handles_punctuation_and_line_edges() {
        let out = all("a.b\nab", query("b", true, true, false), None).unwrap();
        // `a.b` 的 b 前面是分隔符 `.`；第二行的 b 前面是 a。
        assert_eq!(spans(&out), [(0, 2, 0, 3)]);
    }

    #[test]
    fn crlf_is_normalised_and_columns_stay_line_relative() {
        let out = all("ab\r\ncd\r\nab", query("ab", true, false, false), None).unwrap();
        assert_eq!(spans(&out), [(0, 0, 0, 2), (2, 0, 2, 2)]);
        let out = all("a\r\nb", query(r"a\nb", true, false, true), None).unwrap();
        assert_eq!(spans(&out), [(0, 0, 1, 1)]);
    }

    #[test]
    fn regex_anchors_are_per_line_and_groups_work() {
        let doc = "foo1\nbar22\nfoo333\n";
        let out = all(doc, query("^foo\\d+$", true, false, true), None).unwrap();
        assert_eq!(spans(&out), [(0, 0, 0, 4), (2, 0, 2, 6)]);
        let out = all(doc, query("(\\w)(\\d)\\b", true, false, true), None).unwrap();
        assert_eq!(out.matches.len(), 3);
        let out = all("A\nb", query("a", false, false, true), None).unwrap();
        assert_eq!(out.matches.len(), 1);
    }

    #[test]
    fn regex_can_match_across_lines_with_newline_escape() {
        let out = all("a\nb\nc\n", query("a\\nb", true, false, true), None).unwrap();
        assert_eq!(spans(&out), [(0, 0, 1, 1)]);
        // `$` 在多行模式下匹配行尾。
        let out = all("a\nb\n", query("a$\\n^b", true, false, true), None).unwrap();
        assert_eq!(spans(&out), [(0, 0, 1, 1)]);
        let out = all("x\ty", query("\\t", true, false, true), None).unwrap();
        assert_eq!(spans(&out), [(0, 1, 0, 2)]);
    }

    #[test]
    fn empty_regex_matches_advance_and_are_kept() {
        let out = all("ab\ncd", query("^", true, false, true), None).unwrap();
        assert_eq!(spans(&out), [(0, 0, 0, 0), (1, 0, 1, 0)]);
        let out = all("é€", query("x*", true, false, true), None).unwrap();
        assert_eq!(out.matches.len(), 3); // 0, 2, 5
    }

    #[test]
    fn invalid_regex_is_reported() {
        let err = Searcher::compile(&query("(", true, false, true))
            .err()
            .unwrap();
        assert!(matches!(err, QueryError::InvalidRegex(_)));
        assert!(Searcher::compile(&query("", true, false, true))
            .unwrap()
            .is_none());
    }

    #[test]
    fn scope_limits_matches_to_the_selection() {
        let doc = "foo\nfoo foo\nfoo\n";
        let scope = Span::new(pos(1, 0), pos(1, 7)).unwrap();
        let out = all(doc, query("foo", true, false, false), Some(&scope)).unwrap();
        assert_eq!(spans(&out), [(1, 0, 1, 3), (1, 4, 1, 7)]);
        // 选区把第二个 foo 切开：不算匹配。
        let scope = Span::new(pos(1, 0), pos(1, 5)).unwrap();
        let out = all(doc, query("foo", true, false, false), Some(&scope)).unwrap();
        assert_eq!(out.matches.len(), 1);
    }

    #[test]
    fn limit_marks_exceeded_and_keeps_the_first_matches() {
        let doc = "a ".repeat(50);
        let rope = RopeText::from_text(&doc);
        let s = Searcher::compile(&query("a", true, false, false))
            .unwrap()
            .unwrap();
        let out = search_all(&rope, &s, None, 10, &Cancellation::default(), &mut |_| {}).unwrap();
        assert_eq!(out.matches.len(), 10);
        assert!(out.limit_exceeded);
        let out = search_all(&rope, &s, None, 50, &Cancellation::default(), &mut |_| {}).unwrap();
        assert!(!out.limit_exceeded);
    }

    #[test]
    fn blocks_do_not_lose_or_duplicate_matches() {
        // 超过一块：每行 40 字节，5 万行 ≈ 2 MB。
        let mut doc = String::new();
        for i in 0..50_000 {
            doc.push_str(&format!("line {i:06} needle {}\n", "x".repeat(18)));
        }
        let out = all(&doc, query("needle", true, false, false), None).unwrap();
        assert_eq!(out.matches.len(), 50_000);
        assert_eq!(out.matches[49_999].start.line.0, 49_999);
        let out = all(&doc, query("n.edle", true, false, true), None).unwrap();
        assert_eq!(out.matches.len(), 50_000);
    }

    #[test]
    fn progress_reports_every_block_and_reaches_the_end() {
        let doc = "needle\n".repeat(200_000);
        let rope = RopeText::from_text(&doc);
        let s = Searcher::compile(&query("needle", true, false, false))
            .unwrap()
            .unwrap();
        let mut seen = Vec::new();
        search_all(
            &rope,
            &s,
            None,
            usize::MAX,
            &Cancellation::default(),
            &mut |p| seen.push(p),
        )
        .unwrap();
        assert!(seen.len() >= 2, "应分成多块");
        assert!(seen
            .windows(2)
            .all(|w| w[0].scanned_bytes <= w[1].scanned_bytes));
        assert_eq!(seen.last().unwrap().fraction(), 1.0);
    }

    #[test]
    fn cancellation_stops_the_scan() {
        let rope = RopeText::from_text(&"needle\n".repeat(10));
        let s = Searcher::compile(&query("needle", true, false, false))
            .unwrap()
            .unwrap();
        let cancel = Cancellation::default();
        cancel.cancel();
        let r = search_all(&rope, &s, None, 100, &cancel, &mut |_| {});
        assert_eq!(r.err(), Some(Error::Cancelled));
    }

    fn step(doc: &str, q: Query, from: Position, forward: bool, wrap: bool) -> Option<Step> {
        let rope = RopeText::from_text(doc);
        let s = Searcher::compile(&q).unwrap().unwrap();
        find_next(
            &rope,
            &s,
            None,
            from,
            forward,
            wrap,
            &Cancellation::default(),
        )
        .unwrap()
    }

    #[test]
    fn next_and_previous_loop_or_stop() {
        let doc = "a\nb a\nc a\n";
        let q = || query("a", true, false, false);
        let s = step(doc, q(), pos(0, 0), true, true).unwrap();
        assert_eq!((s.found.start, s.wrapped), (pos(0, 0), false));
        let s = step(doc, q(), pos(0, 1), true, true).unwrap();
        assert_eq!((s.found.start, s.wrapped), (pos(1, 2), false));
        // 越过最后一个：循环回第一个。
        let s = step(doc, q(), pos(2, 3), true, true).unwrap();
        assert_eq!((s.found.start, s.wrapped), (pos(0, 0), true));
        assert!(step(doc, q(), pos(2, 3), true, false).is_none());
        // 上一个。
        let s = step(doc, q(), pos(2, 2), false, true).unwrap();
        assert_eq!((s.found.start, s.wrapped), (pos(1, 2), false));
        let s = step(doc, q(), pos(0, 0), false, true).unwrap();
        assert_eq!((s.found.start, s.wrapped), (pos(2, 2), true));
        assert!(step(doc, q(), pos(0, 0), false, false).is_none());
    }

    #[test]
    fn next_with_a_single_match_wraps_onto_itself() {
        let s = step(
            "xx a xx",
            query("a", true, false, false),
            pos(0, 4),
            true,
            true,
        )
        .unwrap();
        assert_eq!((s.found.start, s.wrapped), (pos(0, 3), true));
    }

    #[test]
    fn navigation_respects_scope() {
        let doc = "a\na\na\na\n";
        let rope = RopeText::from_text(doc);
        let s = Searcher::compile(&query("a", true, false, false))
            .unwrap()
            .unwrap();
        let scope = Span::new(pos(1, 0), pos(2, 1)).unwrap();
        let c = Cancellation::default();
        let n = find_next(&rope, &s, Some(&scope), pos(2, 1), true, true, &c)
            .unwrap()
            .unwrap();
        assert_eq!((n.found.start, n.wrapped), (pos(1, 0), true));
        let p = find_next(&rope, &s, Some(&scope), pos(1, 0), false, true, &c)
            .unwrap()
            .unwrap();
        assert_eq!((p.found.start, p.wrapped), (pos(2, 0), true));
        assert!(
            find_next(&rope, &s, Some(&scope), pos(2, 1), true, false, &c)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn multiline_navigation_uses_the_whole_range() {
        let doc = "a\nb\nx\na\nb\n";
        let q = || query("a\\nb", true, false, true);
        let s = step(doc, q(), pos(0, 1), true, true).unwrap();
        assert_eq!(s.found.start, pos(3, 0));
        let s = step(doc, q(), pos(3, 0), false, true).unwrap();
        assert_eq!(s.found.start, pos(0, 0));
    }

    #[test]
    fn multiline_source_detection() {
        assert!(is_multiline_regex_source("a\\nb"));
        assert!(is_multiline_regex_source("\\W+"));
        assert!(!is_multiline_regex_source("\\\\n"));
        assert!(!is_multiline_regex_source("\\s+"));
    }
}
