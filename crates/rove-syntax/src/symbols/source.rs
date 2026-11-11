//! 提取时共用的源文本访问：解析、字节偏移 → 契约坐标、符号构造与协作式取消。
use crate::{Symbol, SymbolKind};
use rove_base::{Cancellation, Error, Line, Position, Result, Span, TextRange, Utf8Column};
use std::{cell::Cell, sync::Arc};
use tree_sitter::{Language, Node, ParseOptions, Parser, Tree};

/// 每提取这么多个符号检查一次取消。
const CHECK_EVERY: usize = 256;

pub(super) struct Source<'a> {
    pub text: &'a str,
    /// 每行起始字节；行终止符与 `TextSource` 一致：`\r\n`、`\n`、单独的 `\r`。
    lines: Vec<usize>,
    cancel: &'a Cancellation,
    count: Cell<usize>,
    limit: usize,
}

impl<'a> Source<'a> {
    pub fn new(text: &'a str, cancel: &'a Cancellation, limit: usize) -> Self {
        let bytes = text.as_bytes();
        let mut lines = vec![0];
        let mut i = 0;
        while let Some(off) = memchr::memchr2(b'\n', b'\r', &bytes[i..]) {
            let at = i + off;
            let next = if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
                at + 2
            } else {
                at + 1
            };
            lines.push(next);
            i = next;
        }
        Self {
            text,
            lines,
            cancel,
            count: Cell::new(0),
            limit,
        }
    }

    /// 带取消的完整解析（只在后台线程调用）。
    pub fn parse(&self, language: Language) -> Result<Tree> {
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|_| Error::Unsupported)?;
        let bytes = self.text.as_bytes();
        let mut stop = |_: &_| self.cancel.check().is_err();
        let options = ParseOptions::new().progress_callback(&mut stop);
        let tree = parser.parse_with_options(
            &mut |offset, _| bytes.get(offset..).unwrap_or_default(),
            None,
            Some(options),
        );
        self.cancel.check()?;
        tree.ok_or(Error::NotReady)
    }

    /// 已产出的符号数达到上限时返回 false（数据文件可能有上百万个键）。
    pub fn room(&self) -> bool {
        self.count.get() < self.limit
    }

    pub fn truncated(&self) -> bool {
        self.count.get() >= self.limit
    }

    fn tick(&self) -> Result<()> {
        let n = self.count.get() + 1;
        self.count.set(n);
        if n.is_multiple_of(CHECK_EVERY) {
            self.cancel.check()?;
        }
        Ok(())
    }

    pub fn position(&self, byte: usize) -> Position {
        let byte = byte.min(self.text.len());
        let line = self.lines.partition_point(|&start| start <= byte) - 1;
        Position {
            line: Line(line as u64),
            column: Utf8Column((byte - self.lines[line]) as u64),
        }
    }

    pub fn range(&self, start: usize, end: usize) -> TextRange {
        let (start, end) = (start.min(end), end.max(start));
        Span::new(self.position(start), self.position(end)).expect("ordered range")
    }

    pub fn text(&self, node: Node) -> &'a str {
        self.text.get(node.byte_range()).unwrap_or_default()
    }

    /// 节点文本，连续空白折叠成一个空格（类型名、参数列表跨行时）。
    pub fn compact(&self, node: Node) -> String {
        let mut out = String::new();
        for word in self.text(node).split_whitespace() {
            if !out.is_empty() && needs_space(out.as_bytes()[out.len() - 1], word.as_bytes()[0]) {
                out.push(' ');
            }
            out.push_str(word);
        }
        out
    }

    /// 构造符号；`selection` 总是被夹在 `range` 之内（契约要求）。
    pub fn symbol(
        &self,
        name: impl Into<Arc<str>>,
        kind: SymbolKind,
        range: (usize, usize),
        selection: (usize, usize),
        children: Vec<Symbol>,
    ) -> Result<Symbol> {
        self.tick()?;
        let (start, end) = (range.0.min(range.1), range.1.max(range.0));
        let sel_start = selection.0.clamp(start, end);
        let sel_end = selection.1.clamp(sel_start, end);
        Ok(Symbol {
            name: name.into(),
            kind,
            range: self.range(start, end),
            selection: self.range(sel_start, sel_end),
            children: children.into(),
        })
    }
}

/// 折叠空白时，标点两侧不补空格（`List < int >` → `List<int>`，`int, string` 保留逗号后的空格）。
fn needs_space(prev: u8, next: u8) -> bool {
    !(matches!(prev, b'<' | b'(' | b'[' | b'.')
        || matches!(next, b'<' | b'>' | b')' | b']' | b',' | b'.' | b'?' | b'*'))
}

pub(super) fn span(node: Node) -> (usize, usize) {
    (node.start_byte(), node.end_byte())
}

pub(super) fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}
