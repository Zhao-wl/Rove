//! 把文档切成「文本 / 左括号 / 右括号」三种 token（#57）。
//!
//! 对应 VS Code 的 `bracketPairsTextModelPart/tokenizer.ts`。两点要注意：
//!
//! - **注释与字符串里的括号不算括号**。是否在注释 / 字符串里由 #9 的 tree-sitter token（scope）
//!   判定（[`BracketSource::ignored_ranges`]），本模块不自己扫引号与注释起止。
//! - **同一位置取最长的括号符号**（XML 的 `<!--` 优先于 `<`），与 #20 的括号匹配一致。
//!
//! [`Tokenizer::skip`] 是增量更新的关键：复用一棵旧子树时直接按长度跳过去，
//! 不去读那一段文本，所以一次编辑只会扫到被改动的那几行。

use super::length::Length;
use std::ops::Range;
use std::sync::Arc;

/// 一种括号。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketKind {
    pub open: Arc<str>,
    pub close: Arc<str>,
    /// 参与括号对着色（`colorizedBracketPairs`）。
    pub colorized: bool,
    /// 参与括号匹配（`brackets`）：活动括号对只认这些种类，
    /// 这样 `editor.matchBrackets` 的语义与 #20 完全一致。
    pub matchable: bool,
}

/// 括号对树读取文档的接口。刻意不碰 GPUI 与 `TextSource`，单元测试可以直接喂字符串。
pub trait BracketSource {
    fn line_count(&self) -> u64;
    /// 一行源文本（不含行终止符）；越界返回 `None`。
    fn line_text(&self, line: u64) -> Option<Arc<str>>;
    /// 该行里属于注释或字符串的字节区间（升序、互不重叠）。默认实现为「没有」。
    fn ignored_ranges(&self, _line: u64, _out: &mut Vec<Range<u32>>) {}
}

/// 括号符号表：(符号, 种类下标, 是否左括号)，按符号长度降序排列。
pub type Symbols = Vec<(Arc<str>, u8, bool)>;

/// 从括号种类表造出扫描用的符号表。
pub fn symbols(kinds: &[BracketKind]) -> Symbols {
    let mut out: Symbols = Vec::with_capacity(kinds.len() * 2);
    for (i, k) in kinds.iter().enumerate() {
        if k.open.is_empty() || k.close.is_empty() || i >= super::ast::MAX_KINDS {
            continue;
        }
        out.push((k.open.clone(), i as u8, true));
        out.push((k.close.clone(), i as u8, false));
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.0.len()));
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Text(Length),
    Open { length: Length, kind: u8 },
    Close { length: Length, kind: u8 },
}

impl Token {
    pub fn length(self) -> Length {
        match self {
            Token::Text(l) => l,
            Token::Open { length, .. } | Token::Close { length, .. } => length,
        }
    }
}

/// 括号符号的首字节表：绝大多数字节都不可能是括号的开头，先查一次表就跳过，
/// 省掉逐字节对每个符号做 `starts_with`（5,000 行 C# 的整篇解析因此快了一个量级）。
fn first_bytes(symbols: &[(Arc<str>, u8, bool)]) -> [bool; 256] {
    let mut table = [false; 256];
    for (s, _, _) in symbols {
        if let Some(&b) = s.as_bytes().first() {
            table[b as usize] = true;
        }
    }
    table
}

/// 一行里从 `start` 列开始的第一个括号：(列, 字节长度, 种类, 是否左括号)。
fn first_bracket(
    text: &str,
    ignored: &[Range<u32>],
    start: u32,
    symbols: &[(Arc<str>, u8, bool)],
    starts: &[bool; 256],
) -> Option<(u32, u32, u8, bool)> {
    let bytes = text.as_bytes();
    let mut i = start as usize;
    let mut ir = 0usize;
    while i < bytes.len() {
        // 跳过注释 / 字符串（区间升序，游标只前进）。
        while ir < ignored.len() && ignored[ir].end as usize <= i {
            ir += 1;
        }
        if ir < ignored.len() && ignored[ir].start as usize <= i {
            i = ignored[ir].end as usize;
            continue;
        }
        if !starts[bytes[i] as usize] {
            i += 1;
            continue;
        }
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let rest = &text[i..];
        if let Some((s, kind, open)) = symbols.iter().find(|(s, _, _)| rest.starts_with(&**s)) {
            return Some((i as u32, s.len() as u32, *kind, *open));
        }
        i += 1;
    }
    None
}

/// 文档的 token 流。
pub struct Tokenizer<'a> {
    source: &'a dyn BracketSource,
    symbols: &'a [(Arc<str>, u8, bool)],
    starts: [bool; 256],
    /// 文档末尾位置。
    end: Length,
    pos: Length,
    line: Option<(u64, Arc<str>, Vec<Range<u32>>)>,
    /// 预读的 token 与**预读之前**的位置：`offset` 要报预读之前的位置，
    /// 否则「先 peek 再复用旧子树」会从错误的地方跳。
    peeked: Option<(Option<Token>, Length)>,
    /// 已经产出的括号个数。
    pub brackets: usize,
    /// 括号个数上限；超过之后 token 流提前结束并置位 [`Tokenizer::over_limit`]。
    pub limit: usize,
    pub over_limit: bool,
}

impl<'a> Tokenizer<'a> {
    pub fn new(
        source: &'a dyn BracketSource,
        symbols: &'a [(Arc<str>, u8, bool)],
        limit: usize,
    ) -> Self {
        let lines = source.line_count().max(1);
        let last = lines - 1;
        let end = Length::new(last, source.line_text(last).map_or(0, |t| t.len() as u32));
        Self {
            source,
            symbols,
            starts: first_bytes(symbols),
            end,
            pos: Length::ZERO,
            line: None,
            peeked: None,
            brackets: 0,
            limit,
            over_limit: false,
        }
    }

    pub fn document_length(&self) -> Length {
        self.end
    }

    /// 当前位置（绝对）。预读过的 token 还没被消费，所以报预读之前的位置。
    pub fn offset(&self) -> Length {
        self.peeked.map_or(self.pos, |(_, before)| before)
    }

    pub fn peek(&mut self) -> Option<Token> {
        if self.peeked.is_none() {
            let before = self.pos;
            let token = self.produce();
            self.peeked = Some((token, before));
        }
        self.peeked.expect("刚填过").0
    }

    pub fn read(&mut self) -> Option<Token> {
        match self.peeked.take() {
            Some((token, _)) => token,
            None => self.produce(),
        }
    }

    /// 跳过 `length` 这么长（复用旧子树时用），不读这一段的文本。
    pub fn skip(&mut self, length: Length) {
        let base = self.offset();
        self.peeked = None;
        self.pos = base.add(length).min(self.end);
    }

    fn with_line<T>(&mut self, line: u64, f: impl FnOnce(&str, &[Range<u32>]) -> T) -> Option<T> {
        if self.line.as_ref().map(|(l, _, _)| *l) != Some(line) {
            let text = self.source.line_text(line)?;
            let mut ignored = Vec::new();
            self.source.ignored_ranges(line, &mut ignored);
            self.line = Some((line, text, ignored));
        }
        let (_, text, ignored) = self.line.as_ref()?;
        Some(f(text, ignored))
    }

    /// `from` 之后（含）的第一个括号的绝对位置与形态。
    fn next_bracket(&mut self, from: Length) -> Option<(Length, u32, u8, bool)> {
        let mut line = from.lines;
        let mut column = from.columns;
        let lines = self.source.line_count().max(1);
        let symbols = self.symbols;
        let starts = self.starts;
        while line < lines {
            let hit = self.with_line(line, |text, ignored| {
                first_bracket(
                    text,
                    ignored,
                    column.min(text.len() as u32),
                    symbols,
                    &starts,
                )
            })?;
            if let Some((col, len, kind, open)) = hit {
                return Some((Length::new(line, col), len, kind, open));
            }
            line += 1;
            column = 0;
        }
        None
    }

    fn produce(&mut self) -> Option<Token> {
        if self.over_limit || self.pos >= self.end {
            return None;
        }
        match self.next_bracket(self.pos) {
            Some((at, len, kind, open)) if at == self.pos => {
                self.brackets += 1;
                if self.brackets > self.limit {
                    self.over_limit = true;
                    return None;
                }
                let length = Length::new(0, len);
                self.pos = self.pos.add(length).min(self.end);
                Some(if open {
                    Token::Open { length, kind }
                } else {
                    Token::Close { length, kind }
                })
            }
            Some((at, ..)) => {
                let length = self.pos.to(at);
                self.pos = at;
                (!length.is_zero()).then_some(Token::Text(length))
            }
            None => {
                let length = self.pos.to(self.end);
                self.pos = self.end;
                (!length.is_zero()).then_some(Token::Text(length))
            }
        }
    }
}

/// 测试与基准用的纯文本文档。
#[derive(Clone, Debug, Default)]
pub struct TextDocument {
    lines: Vec<Arc<str>>,
    /// 每行要忽略的区间（注释 / 字符串）。
    ignored: Vec<Vec<Range<u32>>>,
}

impl TextDocument {
    pub fn new(text: &str) -> Self {
        let lines: Vec<Arc<str>> = text.split('\n').map(Arc::from).collect();
        let ignored = vec![Vec::new(); lines.len()];
        Self { lines, ignored }
    }

    /// 标记一段注释 / 字符串（行内字节区间）。
    pub fn ignore(mut self, line: u64, range: Range<u32>) -> Self {
        if let Some(slot) = self.ignored.get_mut(line as usize) {
            slot.push(range);
            slot.sort_by_key(|r| r.start);
        }
        self
    }

    /// 替换一行（测试增量更新用）。
    pub fn set_line(&mut self, line: u64, text: &str) {
        if let Some(slot) = self.lines.get_mut(line as usize) {
            *slot = Arc::from(text);
        }
    }

    pub fn insert_line(&mut self, line: u64, text: &str) {
        let at = (line as usize).min(self.lines.len());
        self.lines.insert(at, Arc::from(text));
        self.ignored.insert(at, Vec::new());
    }

    pub fn remove_line(&mut self, line: u64) {
        let at = line as usize;
        if at < self.lines.len() {
            self.lines.remove(at);
            self.ignored.remove(at);
        }
    }
}

impl BracketSource for TextDocument {
    fn line_count(&self) -> u64 {
        self.lines.len().max(1) as u64
    }
    fn line_text(&self, line: u64) -> Option<Arc<str>> {
        self.lines.get(line as usize).cloned()
    }
    fn ignored_ranges(&self, line: u64, out: &mut Vec<Range<u32>>) {
        if let Some(r) = self.ignored.get(line as usize) {
            out.extend(r.iter().cloned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds() -> Vec<BracketKind> {
        [("{", "}"), ("[", "]"), ("(", ")")]
            .iter()
            .map(|(o, c)| BracketKind {
                open: (*o).into(),
                close: (*c).into(),
                colorized: true,
                matchable: true,
            })
            .collect()
    }

    fn tokens(doc: &TextDocument, kinds: &[BracketKind]) -> Vec<Token> {
        let syms = symbols(kinds);
        let mut t = Tokenizer::new(doc, &syms, usize::MAX);
        let mut out = Vec::new();
        while let Some(token) = t.read() {
            out.push(token);
        }
        out
    }

    #[test]
    fn text_and_brackets_alternate() {
        let doc = TextDocument::new("a{b}\nc");
        assert_eq!(
            tokens(&doc, &kinds()),
            vec![
                Token::Text(Length::new(0, 1)),
                Token::Open {
                    length: Length::new(0, 1),
                    kind: 0
                },
                Token::Text(Length::new(0, 1)),
                Token::Close {
                    length: Length::new(0, 1),
                    kind: 0
                },
                // 跨行的文本 token：长度是 (1 行, 1 列)。
                Token::Text(Length::new(1, 1)),
            ]
        );
    }

    #[test]
    fn longest_symbol_wins_and_ignored_ranges_are_skipped() {
        let xml: Vec<BracketKind> = [("<!--", "-->"), ("<", ">")]
            .iter()
            .map(|(o, c)| BracketKind {
                open: (*o).into(),
                close: (*c).into(),
                colorized: true,
                matchable: true,
            })
            .collect();
        let doc = TextDocument::new("<!--<a>-->");
        let all = tokens(&doc, &xml);
        assert_eq!(all.len(), 5, "{all:?}");
        assert!(matches!(all[0], Token::Open { kind: 0, .. }));
        assert!(matches!(all[4], Token::Close { kind: 0, .. }));

        // 把注释体标成忽略区间后，里面的 `<a>` 不再产生括号 token，只剩中间的普通文本。
        let doc = TextDocument::new("<!--<a>-->").ignore(0, 4..7);
        let all = tokens(&doc, &xml);
        assert_eq!(all.len(), 3, "{all:?}");
        assert!(matches!(all[1], Token::Text(_)));
    }

    #[test]
    fn skip_jumps_without_reading_text() {
        let doc = TextDocument::new("aaa{b}ccc");
        let syms = symbols(&kinds());
        let mut t = Tokenizer::new(&doc, &syms, usize::MAX);
        t.skip(Length::new(0, 6));
        assert_eq!(t.offset(), Length::new(0, 6));
        assert_eq!(t.read(), Some(Token::Text(Length::new(0, 3))));
        assert_eq!(t.read(), None);
    }

    #[test]
    fn limit_stops_the_stream() {
        let doc = TextDocument::new("{{{{{{");
        let syms = symbols(&kinds());
        let mut t = Tokenizer::new(&doc, &syms, 3);
        let mut n = 0;
        while t.read().is_some() {
            n += 1;
        }
        assert_eq!(n, 3);
        assert!(t.over_limit);
    }
}
