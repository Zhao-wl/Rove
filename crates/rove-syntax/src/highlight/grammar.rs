use super::source::Source;
use rove_base::{Cancellation, Error, Result};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{Arc, LazyLock},
    time::Instant,
};
use tree_sitter::{
    Language, Node, ParseOptions, Parser, Query, QueryCursor, QueryCursorOptions,
    QueryPredicateArg, Range as TreeRange, StreamingIterator, Tree,
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
    predefined_types: Option<PredefinedTypes>,
}

struct PredefinedTypes {
    dispatch: u32,
    rules: BTreeMap<Box<[u8]>, (usize, u32)>,
}

impl PredefinedTypes {
    const MARKER: &'static str = "rove.internal.predefined_type";

    fn compile(query: &mut Query) -> Option<Self> {
        let dispatch = query.capture_index_for_name(Self::MARKER)?;
        let mut rules = BTreeMap::new();
        for pattern in 0..query.pattern_count() {
            let Some(predicate) = query
                .general_predicates(pattern)
                .iter()
                .find(|predicate| predicate.operator.as_ref() == "rove-predefined-type?")
            else {
                continue;
            };
            let [QueryPredicateArg::Capture(scope), QueryPredicateArg::String(literal)] =
                predicate.args.as_ref()
            else {
                panic!("invalid bundled predefined type rule")
            };
            assert!(rules
                .insert(literal.as_bytes().into(), (pattern, *scope))
                .is_none());
            query.disable_pattern(pattern);
        }
        Some(Self { dispatch, rules })
    }
}

impl HighlightQuery {
    fn new(grammar: Grammar, source: &str) -> Self {
        let expanded;
        let source = if grammar == Grammar::Csharp {
            // Tree-sitter names all C# built-in types "predefined_type". Dispatch
            // their equality rules once instead of matching every rule at each node.
            expanded = format!("{source}\n(predefined_type) @{}\n", PredefinedTypes::MARKER);
            &expanded
        } else {
            source
        };
        let mut query =
            Query::new(&grammar.language(), source).expect("valid bundled highlight query");
        let predefined_types = PredefinedTypes::compile(&mut query);
        let scopes = query
            .capture_names()
            .iter()
            .map(|&name| Arc::from([Arc::from(name)]))
            .collect();
        Self {
            query,
            scopes,
            predefined_types,
        }
    }

    fn capture_scope(
        &self,
        pattern: usize,
        capture: u32,
        source: &Source,
        span: &Range<usize>,
    ) -> Option<(usize, &Arc<[Arc<str>]>)> {
        let (pattern, capture) = match &self.predefined_types {
            Some(types) if capture == types.dispatch => {
                *types.rules.get(source.bytes(span.clone()).as_ref())?
            }
            _ => (pattern, capture),
        };
        Some((pattern, &self.scopes[capture as usize]))
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

    /// VS Code 给围栏代码块内容加的外层 scope（`meta.embedded.block.<语言>`）。主题里的
    /// `meta.embedded` 规则靠它生效：代码块里没有被更内层规则命中的字符用的是这条规则的颜色，
    /// 而不是编辑器前景色。围栏标记与语言名不在其内。
    fn embedded_block_scope(self) -> &'static Arc<[Arc<str>]> {
        static SCOPES: LazyLock<[Arc<[Arc<str>]>; 6]> = LazyLock::new(|| {
            [
                "meta.embedded.block.csharp",
                "meta.embedded.block.json",
                "meta.embedded.block.yaml",
                "meta.embedded.block.xml",
                "meta.embedded.block.markdown",
                "meta.embedded.block.markdown",
            ]
            .map(|scope| Arc::from([Arc::from(scope)]))
        });
        &SCOPES[self as usize]
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

    pub fn parse(self, source: &Source, previous: Option<&Tree>, work: &Work<'_>) -> Result<Tree> {
        parse(&mut self.parser(), source, previous, work)
    }

    /// Parse only `ranges` of the document, optionally reusing an edited tree of them.
    pub fn parse_ranges(
        self,
        source: &Source,
        ranges: &[TreeRange],
        previous: Option<&Tree>,
        work: &Work<'_>,
    ) -> Result<Tree> {
        let mut parser = self.parser();
        parser
            .set_included_ranges(ranges)
            .map_err(|_| Error::InvalidInput)?;
        parse(&mut parser, source, previous, work)
    }

    /// Reparse a lexeme in isolation only when the surrounding parse is provably unchanged.
    /// Keep the regular edited-tree path for edits to delimiters, operators, identifiers, or
    /// anything that can change the parser's lookahead or scanner state.
    pub fn reparse_unchanged_token(
        self,
        tree: &Tree,
        before: &Source,
        after: &Source,
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
        let old = before.bytes(bytes.clone());
        let new = after.bytes(bytes.clone());
        // C# decimal digits are interchangeable only inside a standalone, unsuffixed integer.
        // In particular, editing 1 to 0 in malformed "1x" could start a hexadecimal token.
        if node.kind() != "integer_literal"
            || old.is_empty()
            || !old.iter().all(u8::is_ascii_digit)
            || !new.iter().all(u8::is_ascii_digit)
            || !after
                .byte(bytes.end)
                .is_none_or(|b| b" \t\r\n);,]}".contains(&b))
            || !bytes
                .start
                .checked_sub(1)
                .and_then(|i| after.byte(i))
                .is_none_or(|b| b" \t\r\n([=,:{".contains(&b))
        {
            return Ok(false);
        }

        let local = self.parse(&Source::from_bytes(&new), None, work)?;
        let leaf = local.root_node().descendant_for_byte_range(0, new.len());
        Ok(leaf.is_some_and(|n| n.kind() == "integer_literal" && n.byte_range() == (0..new.len())))
    }

    pub fn capture(
        self,
        tree: &Tree,
        source: &Source,
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
    source: &Source,
    previous: Option<&Tree>,
    work: &Work<'_>,
) -> Result<Tree> {
    let mut stop = |_: &_| work.cancelled();
    let options = ParseOptions::new().progress_callback(&mut stop);
    let result = parser.parse_with_options(
        &mut |offset, _| source.chunk_at(offset),
        previous,
        Some(options),
    );
    work.check()?;
    result.ok_or(Error::NotReady)
}

/// 独占一行的注释，起点补齐到行首的空白处。
///
/// VS Code 的 C# 语法用 `(^\s+)?(?=//)` 把注释行的前导空白也算进注释
/// （`punctuation.whitespace.comment.leading.cs`），而 tree-sitter 的 `comment` 节点从 `//`
/// 开始。缩进与注释同色在小地图、注释带背景的主题下看得出区别，这里与 VS Code 对齐。
/// 行内注释（前面还有代码）不受影响：扫到非空白就原样返回。
fn leading_whitespace(source: &Source, start: usize) -> usize {
    let mut at = start;
    while at > 0 {
        match source.byte(at - 1) {
            Some(b' ' | b'\t') => at -= 1,
            Some(b'\n' | b'\r') => return at,
            _ => return start,
        }
    }
    at
}

struct Highlighter<'a, 'b> {
    source: &'a Source,
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
        self.cursor.set_byte_range(scan.clone());
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
                let Some((pattern, scopes)) =
                    highlights.capture_scope(m.pattern_index, c.index, self.source, &span)
                else {
                    continue;
                };
                let span = if grammar == Grammar::Csharp
                    && scopes
                        .first()
                        .is_some_and(|s| s.starts_with("comment.line"))
                {
                    leading_whitespace(self.source, span.start)..span.end
                } else {
                    span
                };
                let priority = (depth, pattern, index);
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
            self.markdown_children(root, scan, included, depth + 1, output)?;
        }
        Ok(())
    }

    fn markdown_children(
        &mut self,
        root: Node<'_>,
        scan: Range<usize>,
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
        cursor.set_byte_range(scan.clone());
        let mut stop = |_: &_| self.work.cancelled();
        let options = QueryCursorOptions::new().progress_callback(&mut stop);
        let mut matches = cursor.matches_with_options(&INJECTIONS, root, self.source, options);
        let mut parsers: Vec<(Grammar, Parser)> = Vec::new();
        while let Some(m) = matches.next() {
            self.work.check()?;
            let (grammar, node, embedded) = match m.captures {
                [inline] => (Grammar::MarkdownInline, inline.node, false),
                [language, body] => {
                    let id = self.source.text(language.node.byte_range());
                    let Some(grammar) = Grammar::for_fence(&id) else {
                        continue;
                    };
                    (grammar, body.node, true)
                }
                _ => unreachable!("unexpected Markdown injection pattern"),
            };
            let ranges = content_ranges(node, parent);
            if ranges.is_empty() {
                continue;
            }
            if embedded {
                let scopes = grammar.embedded_block_scope();
                for r in &ranges {
                    output.push(Capture {
                        start: r.start_byte,
                        end: r.end_byte,
                        // 外层 scope：同一范围内排在注入语法自己的捕获之前。
                        priority: (depth, 0, 0),
                        scopes,
                    });
                }
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
            let range = node.byte_range();
            self.highlight(
                grammar,
                tree.root_node(),
                range.start.max(scan.start)..range.end.min(scan.end),
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
        let before_source = Source::from_str(before);
        let tree = grammar.parse(&before_source, None, &work).unwrap();
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
            .reparse_unchanged_token(
                &tree,
                &before_source,
                &Source::from_str(&after),
                patch,
                &work
            )
            .unwrap());
        let structural = before.replace('1', "x");
        assert!(!grammar
            .reparse_unchanged_token(
                &tree,
                &before_source,
                &Source::from_str(&structural),
                patch,
                &work
            )
            .unwrap());
    }

    #[test]
    fn predefined_type_dispatch_preserves_the_query_matches() {
        let source = b"class C { bool a; byte b; char c; decimal d; double e; float f; int g; long h; object i; sbyte j; short k; string l; uint m; ulong n; ushort o; nint p; nuint q; void M() {} }";
        let cancel = Cancellation::default();
        let work = Work {
            cancel: &cancel,
            deadline: None,
        };
        let grammar = Grammar::Csharp;
        let text = Source::from_bytes(source);
        let tree = grammar.parse(&text, None, &work).unwrap();
        let reference =
            include_str!("queries/csharp/highlights.scm").replace("rove-predefined-type?", "eq?");
        let reference = Query::new(&grammar.language(), &reference).unwrap();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&reference, tree.root_node(), source.as_slice());
        let mut expected = Vec::new();
        while let Some(m) = matches.next() {
            for (index, capture) in m.captures.iter().enumerate() {
                expected.push((
                    capture.node.start_byte(),
                    capture.node.end_byte(),
                    (0, m.pattern_index, index),
                    reference.capture_names()[capture.index as usize],
                ));
            }
        }
        let mut captures = Vec::new();
        grammar
            .capture(&tree, &text, 0..source.len(), &mut captures, &work)
            .unwrap();
        let mut actual: Vec<_> = captures
            .iter()
            .map(|c| (c.start, c.end, c.priority, c.scopes[0].as_ref()))
            .collect();
        expected.sort_unstable();
        actual.sort_unstable();
        assert_eq!(actual, expected);
    }
}
