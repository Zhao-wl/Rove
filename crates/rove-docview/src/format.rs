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
    format_document(language, text, options, mode, None, usize::MAX, cancel)
        .map(|(formatted, _)| formatted)
}

/// Format a preview using the same formatter, rejecting output that exceeds its byte budget.
pub fn format_text_bounded(
    language: StructuredLanguage,
    text: &str,
    options: FormatOptions<'_>,
    mode: FormatMode,
    max_output: usize,
    cancel: &Cancellation,
) -> Result<String> {
    format_document(language, text, options, mode, None, max_output, cancel)
        .map(|(formatted, _)| formatted)
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
    format_selections(
        language,
        text,
        std::slice::from_ref(&range),
        options,
        cancel,
    )
    .map(|mut results| results.pop().expect("one selection"))
}

/// Format sorted, disjoint ranges with a single parse and return their replacements in order.
pub fn format_selections(
    language: StructuredLanguage,
    text: &str,
    ranges: &[Range<usize>],
    options: FormatOptions<'_>,
    cancel: &Cancellation,
) -> Result<Vec<String>> {
    if ranges.iter().any(|range| text.get(range.clone()).is_none()) {
        return Err(Error::OutOfBounds);
    }
    if ranges.windows(2).any(|pair| pair[0].end > pair[1].start) {
        return Err(Error::InvalidInput);
    }
    if ranges.is_empty() {
        return Ok(Vec::new());
    }
    let (formatted, lengths) = format_document(
        language,
        text,
        options,
        FormatMode::Pretty,
        Some(ranges),
        usize::MAX,
        cancel,
    )?;
    let mut input_end = 0;
    let mut output_end = 0;
    ranges
        .iter()
        .zip(lengths)
        .map(|(range, len)| {
            let start = output_end + (range.start - input_end);
            output_end = start + len;
            input_end = range.end;
            formatted
                .get(start..output_end)
                .map(str::to_owned)
                .ok_or(Error::OutOfBounds)
        })
        .collect()
}

fn format_document(
    language: StructuredLanguage,
    text: &str,
    options: FormatOptions<'_>,
    mode: FormatMode,
    selections: Option<&[Range<usize>]>,
    max_output: usize,
    cancel: &Cancellation,
) -> Result<(String, Vec<usize>)> {
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
    let lengths: Vec<_> = selections
        .unwrap_or_default()
        .iter()
        .map(|range| range.len())
        .collect();
    if text.trim().is_empty() {
        return if text.len() <= max_output {
            Ok((text.to_owned(), lengths))
        } else {
            Err(Error::OutOfBounds)
        };
    }
    let tree = parse_tree(language, text, None, cancel)?;
    let root = tree.root_node();
    if root.has_error()
        && (language != StructuredLanguage::Xml || !xml_dtd_spacing_valid(root, text, cancel)?)
    {
        return Err(Error::InvalidInput);
    }
    let mut formatter = Formatter {
        options,
        text,
        output: FormatOutput {
            text: String::with_capacity(text.len().min(max_output)),
            max: max_output,
        },
        selections,
        lengths,
        cancel,
    };
    if language == StructuredLanguage::Xml {
        formatter.xml(root)?;
    } else {
        formatter.json(root, language == StructuredLanguage::Jsonc, mode)?;
    }
    cancel.check()?;
    Ok((formatter.output.text, formatter.lengths))
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

struct FormatOutput {
    text: String,
    max: usize,
}

impl FormatOutput {
    fn len(&self) -> usize {
        self.text.len()
    }

    fn reserve(&mut self, additional: usize) -> Result<()> {
        let required = self
            .len()
            .checked_add(additional)
            .ok_or(Error::OutOfBounds)?;
        if required > self.max {
            return Err(Error::OutOfBounds);
        }
        if required > self.text.capacity() {
            let capacity = self
                .text
                .capacity()
                .saturating_mul(2)
                .min(self.max)
                .max(required);
            self.text
                .try_reserve_exact(capacity - self.len())
                .map_err(|_| Error::OutOfBounds)?;
        }
        Ok(())
    }

    fn push(&mut self, ch: char) -> Result<()> {
        self.reserve(ch.len_utf8())?;
        self.text.push(ch);
        Ok(())
    }

    fn push_str(&mut self, text: &str) -> Result<()> {
        self.reserve(text.len())?;
        self.text.push_str(text);
        Ok(())
    }

    fn indent(&mut self, unit: &str, depth: usize) -> Result<()> {
        if unit.is_empty() {
            return Ok(());
        }
        self.reserve(unit.len().checked_mul(depth).ok_or(Error::OutOfBounds)?)?;
        for _ in 0..depth {
            self.text.push_str(unit);
        }
        Ok(())
    }
}

struct Formatter<'a> {
    options: FormatOptions<'a>,
    text: &'a str,
    output: FormatOutput,
    selections: Option<&'a [Range<usize>]>,
    lengths: Vec<usize>,
    cancel: &'a Cancellation,
}

impl Formatter<'_> {
    fn selection_for(&self, gap: &Range<usize>) -> Option<usize> {
        let selections = self.selections?;
        let index = selections.partition_point(|range| {
            range.end < gap.start || (!gap.is_empty() && range.end == gap.start)
        });
        let range = selections.get(index)?;
        (range.start <= gap.start && gap.end <= range.end).then_some(index)
    }

    fn note_gap(&mut self, selection: Option<usize>, original: usize, output_start: usize) {
        if let Some(index) = selection {
            self.lengths[index] =
                self.lengths[index] - original + (self.output.len() - output_start);
        }
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
                let selected = self.selection_for(&span);
                if self.selections.is_some() && selected.is_none() {
                    self.output.push_str(gap)?;
                } else {
                    let output_start = self.output.len();
                    let (previous_kind, gap) = if comments {
                        if let Some(comma) = gap.find(',') {
                            self.output.push(',')?;
                            (TokenKind::Comma, &gap[comma + 1..])
                        } else {
                            (prev.kind, gap)
                        }
                    } else {
                        (prev.kind, gap)
                    };
                    match mode {
                        FormatMode::Pretty => {
                            self.pretty_gap(previous_kind, token.kind, gap, depth)?;
                        }
                        FormatMode::Minify if previous_kind == TokenKind::LineComment => {
                            self.breaks(gap, 0)?;
                        }
                        FormatMode::Minify => {}
                    }
                    self.note_gap(selected, span.len(), output_start);
                }
            } else if self.selections.is_some() {
                self.output.push_str(&self.text[..token.span.start])?;
            }
            self.output.push_str(&self.text[token.span.clone()])?;
            if token.kind == TokenKind::Open {
                depth += 1;
            }
            last = Some(token);
        }
        if mode == FormatMode::Pretty {
            if let Some(last) = last {
                let tail = &self.text[last.span.end..];
                if self.selections.is_some() {
                    self.output.push_str(tail)?;
                } else if tail.contains(['\r', '\n']) {
                    self.breaks(tail, 0)?;
                }
            }
        }
        Ok(())
    }

    fn pretty_gap(
        &mut self,
        prev: TokenKind,
        next: TokenKind,
        gap: &str,
        depth: usize,
    ) -> Result<()> {
        use TokenKind::*;
        let has_break = gap.contains(['\r', '\n']);
        if prev == LineComment || has_break {
            self.breaks(gap, depth)?;
        } else if next.is_comment() || prev == BlockComment {
            self.output.push(' ')?;
        } else if !self.options.keep_lines
            && (prev == Comma || (prev == Open && next != Close) || (next == Close && prev != Open))
        {
            self.breaks(gap, depth)?;
        } else if matches!(prev, Colon | Comma) {
            self.output.push(' ')?;
        }
        Ok(())
    }

    fn breaks(&mut self, gap: &str, depth: usize) -> Result<()> {
        let mut rest = gap;
        let mut found = false;
        while let Some(at) = rest.find(['\r', '\n']) {
            let end = at + if rest[at..].starts_with("\r\n") { 2 } else { 1 };
            self.output.push_str(&rest[at..end])?;
            rest = &rest[end..];
            found = true;
        }
        if !found {
            self.output.push_str(self.options.newline)?;
        }
        self.output.indent(self.options.indentation, depth)
    }

    fn xml(&mut self, root: Node<'_>) -> Result<()> {
        // A DTD can set xml:space independently on every element with no explicit value.
        // Treat those elements as preserved without reading or evaluating declarations.
        let has_dtd = xml_doctype(root).is_some();
        let mut edits = Vec::new();
        let mut stack = vec![(root, 0, false)];
        while let Some((node, depth, inherited_preserve)) = stack.pop() {
            self.cancel.check()?;
            let tag = named(node).find(|child| matches!(child.kind(), "STag" | "EmptyElemTag"));
            let preserve = tag
                .and_then(|tag| xml_space(self.text, tag))
                .unwrap_or(inherited_preserve || (has_dtd && node.kind() == "element"));
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
        edits.sort_unstable_by_key(|(span, _)| (span.start, span.end));
        let mut last = 0;
        for (gap, depth) in edits {
            self.cancel.check()?;
            let selected = self.selection_for(&gap);
            if self.selections.is_some() && selected.is_none() {
                continue;
            }
            self.output.push_str(&self.text[last..gap.start])?;
            let output_start = self.output.len();
            self.breaks(&self.text[gap.clone()], depth)?;
            self.note_gap(selected, gap.len(), output_start);
            last = gap.end;
        }
        self.output.push_str(&self.text[last..])?;
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

fn xml_doctype(root: Node<'_>) -> Option<Node<'_>> {
    let prolog = named(root).find(|child| child.kind() == "prolog")?;
    named(prolog).find(|child| child.kind() == "doctypedecl")
}

fn xml_dtd_spacing_valid(root: Node<'_>, text: &str, cancel: &Cancellation) -> Result<bool> {
    let Some(doctype) = xml_doctype(root).filter(Node::has_error) else {
        return Ok(false);
    };
    let mut cursor = doctype.walk();
    let Some(close) = doctype
        .children(&mut cursor)
        .find(|child| child.kind() == "]")
    else {
        return Ok(false);
    };
    let at = close.start_byte();
    if text.as_bytes().get(at.wrapping_sub(1)) != Some(&b'>') {
        return Ok(false);
    }
    // The bundled grammar requires a separator between the last internal subset
    // declaration and ']'. Validate with the optional separator present, then keep
    // using the original tree and text so no byte ranges or DTD bytes change.
    let mut validation = String::with_capacity(text.len() + 1);
    validation.push_str(&text[..at]);
    validation.push(' ');
    validation.push_str(&text[at..]);
    Ok(
        !parse_tree(StructuredLanguage::Xml, &validation, None, cancel)?
            .root_node()
            .has_error(),
    )
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
                let quoted = &text[value.byte_range()];
                let value = quoted
                    .get(1..quoted.len().saturating_sub(1))
                    .unwrap_or_default();
                // Only a known "default" overrides preservation. Named entities may depend
                // on DTD declarations, which this local formatter does not evaluate.
                return Some(!xml_space_is_default(value));
            }
        }
    }
    None
}

fn xml_space_is_default(mut value: &str) -> bool {
    for expected in "default".chars() {
        if let Some(rest) = value.strip_prefix(expected) {
            value = rest;
            continue;
        }
        let Some((reference, rest)) = value
            .strip_prefix("&#")
            .and_then(|text| text.split_once(';'))
        else {
            return false;
        };
        let (digits, radix) = reference
            .strip_prefix('x')
            .map_or((reference, 10), |digits| (digits, 16));
        if digits.starts_with(['+', '-'])
            || u32::from_str_radix(digits, radix).ok() != Some(u32::from(expected))
        {
            return false;
        }
        value = rest;
    }
    value.is_empty()
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
