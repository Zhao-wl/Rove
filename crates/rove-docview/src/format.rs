//! In-memory formatting of decoded JSON/JSONC and XML. Token spelling and mixed XML content
//! are kept; the caller applies the result in one verbatim edit transaction.
use crate::structure::{parse_tree, StructuredLanguage};
use rove_base::{Cancellation, Error, Result};
use std::ops::Range;
use tree_sitter::Node;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatMode {
    Pretty,
    Minify,
}

#[derive(Clone, Copy, Debug)]
pub struct FormatOptions<'a> {
    pub indentation: &'a str,
    pub newline: &'a str,
    pub keep_lines: bool,
}

impl Default for FormatOptions<'_> {
    fn default() -> Self {
        Self {
            indentation: "    ",
            newline: "\n",
            keep_lines: false,
        }
    }
}

/// Run on a worker if the document is large. Every retained line-ending sequence is copied
/// verbatim. Newly inserted line breaks use the supplied preferred terminator.
pub fn format_text(
    language: StructuredLanguage,
    text: &str,
    options: FormatOptions<'_>,
    mode: FormatMode,
    cancel: &Cancellation,
) -> Result<String> {
    format_document(language, text, options, mode, None, cancel)
}

/// Format the selected gaps using the surrounding document's indentation depth.
/// Bytes before and after the range remain untouched.
pub fn format_selection(
    language: StructuredLanguage,
    text: &str,
    range: Range<usize>,
    options: FormatOptions<'_>,
    cancel: &Cancellation,
) -> Result<String> {
    if text.get(range.clone()).is_none() {
        return Err(Error::OutOfBounds);
    }
    let formatted = format_document(
        language,
        text,
        options,
        FormatMode::Pretty,
        Some(range.clone()),
        cancel,
    )?;
    let suffix = text.len() - range.end;
    let end = formatted
        .len()
        .checked_sub(suffix)
        .ok_or(Error::OutOfBounds)?;
    formatted
        .get(range.start..end)
        .map(str::to_owned)
        .ok_or(Error::OutOfBounds)
}

fn format_document(
    language: StructuredLanguage,
    text: &str,
    options: FormatOptions<'_>,
    mode: FormatMode,
    selection: Option<Range<usize>>,
    cancel: &Cancellation,
) -> Result<String> {
    if !matches!(options.newline, "\n" | "\r\n" | "\r")
        || !options
            .indentation
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t'))
    {
        return Err(Error::InvalidInput);
    }
    match (language, mode) {
        (StructuredLanguage::Yaml, _) | (StructuredLanguage::Xml, FormatMode::Minify) => {
            return Err(Error::Unsupported);
        }
        _ => {}
    }
    if text.trim().is_empty() {
        return Ok(text.to_owned());
    }
    let tree = parse_tree(language, text, None, cancel)?;
    let root = tree.root_node();
    if root.has_error() {
        return Err(Error::InvalidInput);
    }
    let mut formatter = Formatter {
        options,
        text,
        output: String::with_capacity(text.len()),
        selection,
        cancel,
    };
    if language == StructuredLanguage::Xml {
        formatter.xml(root)?;
    } else {
        formatter.json(root, language == StructuredLanguage::Jsonc, mode)?;
    }
    cancel.check()?;
    Ok(formatter.output)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Open,
    Close,
    Comma,
    Colon,
    LineComment,
    BlockComment,
    Value,
}

impl TokenKind {
    fn is_comment(self) -> bool {
        matches!(self, Self::LineComment | Self::BlockComment)
    }
}

struct Token {
    kind: TokenKind,
    span: Range<usize>,
}

struct Formatter<'a> {
    options: FormatOptions<'a>,
    text: &'a str,
    output: String,
    selection: Option<Range<usize>>,
    cancel: &'a Cancellation,
}

impl Formatter<'_> {
    fn selected(&self, gap: &Range<usize>) -> bool {
        self.selection
            .as_ref()
            .is_none_or(|range| gap.start >= range.start && gap.end <= range.end)
    }

    fn json(&mut self, root: Node<'_>, comments: bool, mode: FormatMode) -> Result<()> {
        let mut cursor = root.walk();
        if root
            .named_children(&mut cursor)
            .filter(|node| node.kind() != "comment")
            .count()
            != 1
        {
            return Err(Error::InvalidInput);
        }
        let mut last: Option<Token> = None;
        let mut depth: usize = 0;
        for token in leaves(root, self.text) {
            self.cancel.check()?;
            if token.kind.is_comment() && !comments {
                return Err(Error::InvalidInput);
            }
            if token.kind == TokenKind::Close {
                depth = depth.saturating_sub(1);
            }
            if let Some(prev) = &last {
                let span = prev.span.end..token.span.start;
                let gap = &self.text[span.clone()];
                if !self.selected(&span) {
                    self.output.push_str(gap);
                } else {
                    let (previous_kind, gap) = if comments {
                        if let Some(comma) = gap.find(',') {
                            self.output.push(',');
                            (TokenKind::Comma, &gap[comma + 1..])
                        } else {
                            (prev.kind, gap)
                        }
                    } else {
                        (prev.kind, gap)
                    };
                    match mode {
                        FormatMode::Pretty => {
                            self.pretty_gap(previous_kind, token.kind, gap, depth)
                        }
                        FormatMode::Minify if previous_kind == TokenKind::LineComment => {
                            self.breaks(gap, 0);
                        }
                        FormatMode::Minify => {}
                    }
                }
            } else if self.selection.is_some() {
                self.output.push_str(&self.text[..token.span.start]);
            }
            self.output.push_str(&self.text[token.span.clone()]);
            if token.kind == TokenKind::Open {
                depth += 1;
            }
            last = Some(token);
        }
        if mode == FormatMode::Pretty {
            if let Some(last) = last {
                let tail = &self.text[last.span.end..];
                if self.selection.is_some() {
                    self.output.push_str(tail);
                } else if tail.contains(['\r', '\n']) {
                    self.breaks(tail, 0);
                }
            }
        }
        Ok(())
    }

    fn pretty_gap(&mut self, prev: TokenKind, next: TokenKind, gap: &str, depth: usize) {
        use TokenKind::*;
        let has_break = gap.contains(['\r', '\n']);
        if prev == LineComment || has_break {
            self.breaks(gap, depth);
        } else if next.is_comment() || prev == BlockComment {
            self.output.push(' ');
        } else if !self.options.keep_lines
            && (prev == Comma || (prev == Open && next != Close) || (next == Close && prev != Open))
        {
            self.breaks(gap, depth);
        } else if matches!(prev, Colon | Comma) {
            self.output.push(' ');
        }
    }

    fn breaks(&mut self, gap: &str, depth: usize) {
        let mut rest = gap;
        let mut found = false;
        while let Some(at) = rest.find(['\r', '\n']) {
            let end = at + if rest[at..].starts_with("\r\n") { 2 } else { 1 };
            self.output.push_str(&rest[at..end]);
            rest = &rest[end..];
            found = true;
        }
        if !found {
            self.output.push_str(self.options.newline);
        }
        for _ in 0..depth {
            self.output.push_str(self.options.indentation);
        }
    }

    fn xml(&mut self, root: Node<'_>) -> Result<()> {
        let mut edits = Vec::new();
        let mut stack = vec![(root, 0, false)];
        while let Some((node, depth, inherited_preserve)) = stack.pop() {
            self.cancel.check()?;
            let tag = named(node).find(|child| matches!(child.kind(), "STag" | "EmptyElemTag"));
            let preserve = tag
                .and_then(|tag| xml_space(self.text, tag))
                .unwrap_or(inherited_preserve);
            if node.kind() == "element" {
                if let Some(tag) = tag {
                    let start = named(tag).find(|child| child.kind() == "Name");
                    let end = named(node)
                        .find(|child| child.kind() == "ETag")
                        .and_then(|child| named(child).find(|name| name.kind() == "Name"));
                    if let (Some(start), Some(end)) = (start, end) {
                        if self.text[start.byte_range()] != self.text[end.byte_range()] {
                            return Err(Error::InvalidInput);
                        }
                    }
                }
            }
            let content = if node.kind() == "document" {
                Some(node)
            } else {
                named(node).find(|child| child.kind() == "content")
            };
            let Some(content) = content else {
                continue;
            };
            let is_document = node.kind() == "document";
            let child_depth = if is_document { depth } else { depth + 1 };
            let mut markup = Vec::new();
            for child in named(content) {
                self.cancel.check()?;
                if child.kind() == "element" {
                    stack.push((child, child_depth, preserve));
                }
                if matches!(
                    child.kind(),
                    "element" | "Comment" | "PI" | "prolog" | "XMLDecl" | "doctypedecl"
                ) {
                    markup.push(child.byte_range());
                }
            }
            if preserve || markup.is_empty() {
                continue;
            }
            let start = if is_document {
                markup[0].start
            } else {
                content.start_byte()
            };
            let end = if is_document {
                markup.last().expect("nonempty").end
            } else {
                content.end_byte()
            };
            let mut last = start;
            // A single meaningful text/ref/CDATA before, between or after tags makes the
            // entire content mixed. Formatting its gaps would change XML text nodes.
            let mut gaps = Vec::new();
            let mut mixed = false;
            for span in markup {
                if !xml_whitespace(&self.text[last..span.start]) {
                    mixed = true;
                    break;
                }
                if !is_document || last != start {
                    gaps.push((last..span.start, child_depth));
                }
                last = span.end;
            }
            if !mixed && xml_whitespace(&self.text[last..end]) {
                if !is_document {
                    gaps.push((last..end, depth));
                }
                edits.extend(gaps);
            }
        }
        edits.retain(|(span, _)| self.selected(span));
        edits.sort_unstable_by_key(|(span, _)| (span.start, span.end));
        let mut last = 0;
        for (gap, depth) in edits {
            self.cancel.check()?;
            self.output.push_str(&self.text[last..gap.start]);
            self.breaks(&self.text[gap.clone()], depth);
            last = gap.end;
        }
        self.output.push_str(&self.text[last..]);
        Ok(())
    }
}

fn named(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    let mut cursor = node.walk();
    let mut more = cursor.goto_first_child();
    std::iter::from_fn(move || {
        while more {
            let child = cursor.node();
            more = cursor.goto_next_sibling();
            if child.is_named() {
                return Some(child);
            }
        }
        None
    })
}

fn xml_whitespace(text: &str) -> bool {
    text.bytes()
        .all(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
}

fn xml_space(text: &str, tag: Node<'_>) -> Option<bool> {
    for attr in named(tag).filter(|node| node.kind() == "Attribute") {
        let name = named(attr).find(|node| node.kind() == "Name");
        let value = named(attr).find(|node| node.kind() == "AttValue");
        if let (Some(name), Some(value)) = (name, value) {
            if &text[name.byte_range()] == "xml:space" {
                return match text[value.byte_range()].trim_matches(['\'', '"']) {
                    "preserve" => Some(true),
                    "default" => Some(false),
                    _ => None,
                };
            }
        }
    }
    None
}

fn leaves<'a>(root: Node<'a>, text: &'a str) -> impl Iterator<Item = Token> + 'a {
    let mut cursor = root.walk();
    let mut next = Some(root);
    std::iter::from_fn(move || loop {
        let node = next?;
        let kind = match node.kind() {
            "{" | "[" => TokenKind::Open,
            "}" | "]" => TokenKind::Close,
            "," => TokenKind::Comma,
            ":" => TokenKind::Colon,
            "comment" => {
                // The grammar does not distinguish line and block comments by node type.
                let span = node.byte_range();
                let line_comment = text[span.clone()].starts_with("//");
                next = next_sibling(&mut cursor);
                return Some(Token {
                    kind: if line_comment {
                        TokenKind::LineComment
                    } else {
                        TokenKind::BlockComment
                    },
                    span,
                });
            }
            "string" | "number" | "true" | "false" | "null" => TokenKind::Value,
            _ if cursor.goto_first_child() => {
                next = Some(cursor.node());
                continue;
            }
            _ => {
                next = next_sibling(&mut cursor);
                continue;
            }
        };
        next = next_sibling(&mut cursor);
        return Some(Token {
            kind,
            span: node.byte_range(),
        });
    })
}

fn next_sibling<'tree>(cursor: &mut tree_sitter::TreeCursor<'tree>) -> Option<Node<'tree>> {
    loop {
        if cursor.goto_next_sibling() {
            return Some(cursor.node());
        }
        if !cursor.goto_parent() {
            return None;
        }
    }
}
