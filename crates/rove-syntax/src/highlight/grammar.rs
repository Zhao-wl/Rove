use rove_base::{Cancellation, Error, Result};
use std::{
    ops::Range,
    sync::{Arc, LazyLock},
    time::Instant,
};
use tree_sitter::{
    Language, Node, ParseOptions, Parser, Query, QueryCursor, QueryCursorOptions,
    Range as TreeRange, StreamingIterator, Tree,
};

pub(super) struct Work<'a> {
    pub cancel: &'a Cancellation,
    pub deadline: Option<Instant>,
}

impl Work<'_> {
    pub fn cancelled(&self) -> bool {
        self.cancel.check().is_err() || self.deadline.is_some_and(|at| Instant::now() >= at)
    }

    pub fn check(&self) -> Result<()> {
        self.cancel.check()?;
        if self.deadline.is_some_and(|at| Instant::now() >= at) {
            return Err(Error::Stale);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Grammar {
    Csharp,
    Json,
    Yaml,
    Xml,
    Markdown,
    MarkdownInline,
}

pub(super) struct Capture {
    pub start: usize,
    pub end: usize,
    pub priority: (usize, usize, usize),
    pub scopes: &'static Arc<[Arc<str>]>,
}

struct HighlightQuery {
    query: Query,
    scopes: Vec<Arc<[Arc<str>]>>,
}

impl HighlightQuery {
    fn new(grammar: Grammar, source: &str) -> Self {
        let query = Query::new(&grammar.language(), source).expect("valid bundled highlight query");
        let scopes = query
            .capture_names()
            .iter()
            .map(|&name| Arc::from([Arc::from(name)]))
            .collect();
        Self { query, scopes }
    }
}

macro_rules! highlight_queries {
    ($($variant:ident => $path:literal),+ $(,)?) => {
        fn highlight_query(self) -> &'static HighlightQuery {
            match self {
                $(Self::$variant => {
                    static QUERY: LazyLock<HighlightQuery> = LazyLock::new(|| {
                        HighlightQuery::new(Grammar::$variant, include_str!($path))
                    });
                    &QUERY
                })+
            }
        }
    };
}

impl Grammar {
    pub fn for_id(id: &str) -> Option<Self> {
        Some(match id {
            "csharp" => Self::Csharp,
            "json" | "jsonc" => Self::Json,
            "yaml" => Self::Yaml,
            "xml" => Self::Xml,
            "markdown" => Self::Markdown,
            _ => return None,
        })
    }

    fn for_fence(id: &str) -> Option<Self> {
        match id.to_ascii_lowercase().as_str() {
            "cs" | "c#" => Some(Self::Csharp),
            "yml" => Some(Self::Yaml),
            "md" => Some(Self::Markdown),
            id => Self::for_id(id),
        }
    }

    fn language(self) -> Language {
        match self {
            Self::Csharp => tree_sitter_c_sharp::LANGUAGE,
            Self::Json => tree_sitter_json::LANGUAGE,
            Self::Yaml => tree_sitter_yaml::LANGUAGE,
            Self::Xml => tree_sitter_xml::LANGUAGE_XML,
            Self::Markdown => tree_sitter_md::LANGUAGE,
            Self::MarkdownInline => tree_sitter_md::INLINE_LANGUAGE,
        }
        .into()
    }

    highlight_queries! {
        Csharp => "queries/csharp/highlights.scm",
        Json => "queries/json/highlights.scm",
        Yaml => "queries/yaml/highlights.scm",
        Xml => "queries/xml/highlights.scm",
        Markdown => "queries/markdown/highlights.scm",
        MarkdownInline => "queries/markdown/inline.scm",
    }

    fn parser(self) -> Parser {
        let mut parser = Parser::new();
        parser
            .set_language(&self.language())
            .expect("compatible bundled grammar");
        parser
    }

    pub fn parse(self, source: &[u8], previous: Option<&Tree>, work: &Work<'_>) -> Result<Tree> {
        parse(&mut self.parser(), source, previous, work)
    }

    /// Reparse a lexeme in isolation only when the surrounding parse is provably unchanged.
    /// Keep the regular edited-tree path for edits to delimiters, operators, identifiers, or
    /// anything that can change the parser's lookahead or scanner state.
    pub fn reparse_unchanged_token(
        self,
        tree: &Tree,
        before: &str,
        after: &str,
        patch: tree_sitter::InputEdit,
        work: &Work<'_>,
    ) -> Result<bool> {
        if self != Self::Csharp
            || patch.old_end_byte != patch.new_end_byte
            || patch.start_byte == patch.new_end_byte
        {
            return Ok(false);
        }
        let Some(node) = tree
            .root_node()
            .descendant_for_byte_range(patch.start_byte, patch.old_end_byte)
        else {
            return Ok(false);
        };
        let bytes = node.byte_range();
        let old = &before.as_bytes()[bytes.clone()];
        let new = &after.as_bytes()[bytes.clone()];
        // C# decimal digits are interchangeable only inside a standalone, unsuffixed integer.
        // In particular, editing 1 to 0 in malformed "1x" could start a hexadecimal token.
        if node.kind() != "integer_literal"
            || old.is_empty()
            || !old.iter().all(u8::is_ascii_digit)
            || !new.iter().all(u8::is_ascii_digit)
            || !after
                .as_bytes()
                .get(bytes.end)
                .is_none_or(|b| b" \t\r\n);,]}".contains(b))
            || !bytes
                .start
                .checked_sub(1)
                .is_none_or(|i| b" \t\r\n([=,:{".contains(&after.as_bytes()[i]))
        {
            return Ok(false);
        }

        let local = self.parse(new, None, work)?;
        let leaf = local.root_node().descendant_for_byte_range(0, new.len());
        Ok(leaf.is_some_and(|n| n.kind() == "integer_literal" && n.byte_range() == (0..new.len())))
    }

    pub fn capture(
        self,
        tree: &Tree,
        source: &[u8],
        scan: Range<usize>,
        output: &mut Vec<Capture>,
        work: &Work<'_>,
    ) -> Result<()> {
        let mut highlighter = Highlighter {
            source,
            work,
            cursor: QueryCursor::new(),
        };
        highlighter.highlight(self, tree.root_node(), scan, &[], 0, output)
    }
}

fn parse(
    parser: &mut Parser,
    source: &[u8],
    previous: Option<&Tree>,
    work: &Work<'_>,
) -> Result<Tree> {
    let mut stop = |_: &_| work.cancelled();
    let options = ParseOptions::new().progress_callback(&mut stop);
    let result = parser.parse_with_options(
        &mut |offset, _| source.get(offset..).unwrap_or_default(),
        previous,
        Some(options),
    );
    work.check()?;
    result.ok_or(Error::NotReady)
}

struct Highlighter<'a, 'b> {
    source: &'a [u8],
    work: &'a Work<'b>,
    cursor: QueryCursor,
}

impl Highlighter<'_, '_> {
    fn highlight(
        &mut self,
        grammar: Grammar,
        root: Node<'_>,
        scan: Range<usize>,
        included: &[TreeRange],
        depth: usize,
        output: &mut Vec<Capture>,
    ) -> Result<()> {
        if scan.is_empty() {
            return Ok(());
        }
        self.work.check()?;
        let highlights = grammar.highlight_query();
        self.cursor.set_byte_range(scan);
        let mut stop = |_: &_| self.work.cancelled();
        let options = QueryCursorOptions::new().progress_callback(&mut stop);
        let mut matches =
            self.cursor
                .matches_with_options(&highlights.query, root, self.source, options);
        while let Some(m) = matches.next() {
            for (index, c) in m.captures.iter().enumerate() {
                let span = c.node.byte_range();
                if span.is_empty() {
                    continue;
                }
                let scopes = &highlights.scopes[c.index as usize];
                let priority = (depth, m.pattern_index, index);
                if included.is_empty() {
                    output.push(Capture {
                        start: span.start,
                        end: span.end,
                        priority,
                        scopes,
                    });
                } else {
                    for r in &included[included.partition_point(|r| r.end_byte <= span.start)..] {
                        if r.start_byte >= span.end {
                            break;
                        }
                        output.push(Capture {
                            start: span.start.max(r.start_byte),
                            end: span.end.min(r.end_byte),
                            priority,
                            scopes,
                        });
                    }
                }
            }
        }
        drop(matches);
        self.work.check()?;
        if grammar == Grammar::Markdown && depth < 16 {
            self.markdown_children(root, included, depth + 1, output)?;
        }
        Ok(())
    }

    fn markdown_children(
        &mut self,
        root: Node<'_>,
        parent: &[TreeRange],
        depth: usize,
        output: &mut Vec<Capture>,
    ) -> Result<()> {
        static INJECTIONS: LazyLock<Query> = LazyLock::new(|| {
            Query::new(
                &Grammar::Markdown.language(),
                include_str!("queries/markdown/injections.scm"),
            )
            .expect("valid bundled injection query")
        });
        let mut cursor = QueryCursor::new();
        let mut stop = |_: &_| self.work.cancelled();
        let options = QueryCursorOptions::new().progress_callback(&mut stop);
        let mut matches = cursor.matches_with_options(&INJECTIONS, root, self.source, options);
        let mut parsers: Vec<(Grammar, Parser)> = Vec::new();
        while let Some(m) = matches.next() {
            self.work.check()?;
            let (grammar, node) = match m.captures {
                [inline] => (Grammar::MarkdownInline, inline.node),
                [language, body] => {
                    let id = std::str::from_utf8(&self.source[language.node.byte_range()])
                        .unwrap_or_default();
                    let Some(grammar) = Grammar::for_fence(id) else {
                        continue;
                    };
                    (grammar, body.node)
                }
                _ => unreachable!("unexpected Markdown injection pattern"),
            };
            let ranges = content_ranges(node, parent);
            if ranges.is_empty() {
                continue;
            }
            let index = parsers
                .iter()
                .position(|(known, _)| *known == grammar)
                .unwrap_or_else(|| {
                    parsers.push((grammar, grammar.parser()));
                    parsers.len() - 1
                });
            let parser = &mut parsers[index].1;
            parser
                .set_included_ranges(&ranges)
                .map_err(|_| Error::InvalidInput)?;
            let tree = parse(parser, self.source, None, self.work)?;
            self.highlight(
                grammar,
                tree.root_node(),
                node.byte_range(),
                &ranges,
                depth,
                output,
            )?;
        }
        self.work.check()
    }
}

/// Block quotes and indented list items use continuation nodes in the block tree.
/// Exclude those prefixes from the injected input while retaining document coordinates.
fn content_ranges(node: Node<'_>, parent: &[TreeRange]) -> Vec<TreeRange> {
    let mut whole = node.range();
    let mut ranges = Vec::new();
    let mut cursor = node.walk();
    for continuation in node
        .named_children(&mut cursor)
        .filter(|n| n.kind() == "block_continuation")
    {
        if whole.start_byte < continuation.start_byte() {
            ranges.push(TreeRange {
                end_byte: continuation.start_byte(),
                end_point: continuation.start_position(),
                ..whole
            });
        }
        whole.start_byte = continuation.end_byte();
        whole.start_point = continuation.end_position();
    }
    if whole.start_byte < whole.end_byte {
        ranges.push(whole);
    }
    if parent.is_empty() {
        return ranges;
    }
    let mut intersections = Vec::new();
    for range in ranges {
        for p in &parent[parent.partition_point(|p| p.end_byte <= range.start_byte)..] {
            if p.start_byte >= range.end_byte {
                break;
            }
            let start = if p.start_byte > range.start_byte {
                p
            } else {
                &range
            };
            let end = if p.end_byte < range.end_byte {
                p
            } else {
                &range
            };
            intersections.push(TreeRange {
                start_byte: start.start_byte,
                start_point: start.start_point,
                end_byte: end.end_byte,
                end_point: end.end_point,
            });
        }
    }
    intersections
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::{InputEdit, Point};

    #[test]
    fn decimal_substitution_reparses_a_leaf_without_editing_the_full_tree() {
        let before = "class C { int M() { return 1; } }";
        let after = before.replace('1', "2");
        let cancel = Cancellation::default();
        let work = Work {
            cancel: &cancel,
            deadline: None,
        };
        let grammar = Grammar::Csharp;
        let tree = grammar.parse(before.as_bytes(), None, &work).unwrap();
        let byte = before.find('1').unwrap();
        let patch = InputEdit {
            start_byte: byte,
            old_end_byte: byte + 1,
            new_end_byte: byte + 1,
            start_position: Point::new(0, byte),
            old_end_position: Point::new(0, byte + 1),
            new_end_position: Point::new(0, byte + 1),
        };
        assert!(grammar
            .reparse_unchanged_token(&tree, before, &after, patch, &work)
            .unwrap());
        let structural = before.replace('1', "x");
        assert!(!grammar
            .reparse_unchanged_token(&tree, before, &structural, patch, &work)
            .unwrap());
    }
}
