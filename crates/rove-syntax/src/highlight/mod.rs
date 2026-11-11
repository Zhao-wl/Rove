//! Tree-sitter highlighting behind the frozen syntax contracts.

mod csharp_parts;
mod documents;
mod grammar;
mod rows;
mod settings;
mod source;

pub use csharp_parts::WorkerPool;
pub use documents::{DocumentHighlighter, Stateless, SyntaxDocuments};
pub use settings::{language_for_path, register_settings};

use crate::{Fold, Symbol, SyntaxService, SyntaxSnapshot, SyntaxUpdate, Token};
use grammar::{Grammar, Work};
use rove_base::{
    ByteOffset, Cancellation, Error, Line, LineRange, Result, Span, Task, TextEdit, Version,
};
use rove_buffer::TextSource;
use rove_config::Settings;
use rows::{LineTokens, RowRanges};
use source::{Source, EDIT_CONTEXT};
use std::{
    ops::Range,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tree_sitter::{InputEdit, Node, Point, Tree};

/// Executor-neutral highlighter. Poll the returned task on a background executor.
#[derive(Clone)]
pub struct TreeSitterSyntax {
    settings: Option<Arc<dyn Settings>>,
    budget: Option<Duration>,
    pool: Option<Arc<WorkerPool>>,
}

impl Default for TreeSitterSyntax {
    fn default() -> Self {
        Self {
            settings: None,
            budget: None,
            pool: Some(WorkerPool::shared()),
        }
    }
}

impl TreeSitterSyntax {
    pub fn with_settings(mut self, settings: Arc<dyn Settings>) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Time budget when reusing a whole-tree baseline. Full rebuilds run to completion
    /// or cancellation, including rebuilds after a partitioned C# parse.
    pub fn with_refresh_budget(mut self, budget: Duration) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Threads for partitioned C# parsing, from a pool private to this service (and its
    /// clones). Fewer than two disables partitioning. The default shares one process-wide
    /// pool sized to the hardware threads.
    pub fn with_parallelism(mut self, threads: usize) -> Self {
        self.pool = (threads >= 2).then(|| WorkerPool::new(threads));
        self
    }

    /// Allocate a baseline for one open document. Versions are local to that document.
    pub fn document(&self) -> DocumentSyntax {
        DocumentSyntax {
            syntax: self.clone(),
            baseline: Arc::default(),
            edit: None,
            visible: None,
        }
    }
}

impl SyntaxService for TreeSitterSyntax {
    fn analyze(
        &self,
        text: Arc<dyn TextSource>,
        language: Arc<str>,
        cancel: Cancellation,
    ) -> Task<SyntaxUpdate> {
        self.document().analyze(text, language, cancel)
    }
}

/// Syntax service for one document, including its last successfully parsed version.
#[derive(Clone)]
pub struct DocumentSyntax {
    syntax: TreeSitterSyntax,
    baseline: Arc<Mutex<Option<Arc<Snapshot>>>>,
    edit: Option<TextEdit>,
    visible: Option<LineRange>,
}

impl DocumentSyntax {
    /// Attach one replacement against the previous text version. Missing or mismatched
    /// baselines cause a full invalidation, so skipped revisions can be analyzed safely.
    pub fn with_edit(&self, edit: TextEdit) -> Self {
        Self {
            edit: Some(edit),
            ..self.clone()
        }
    }

    /// Highlight only these lines when there is no baseline to update (visible-first).
    /// The result is partial: other lines have no tokens until the next analysis of the
    /// same text without this hint completes it, reusing the parse.
    pub fn with_visible(&self, lines: LineRange) -> Self {
        Self {
            visible: Some(lines),
            ..self.clone()
        }
    }

    /// Whether the latest successful result covers only a visible range.
    pub fn is_partial(&self) -> bool {
        self.baseline
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|snapshot| snapshot.partial)
    }
}

impl SyntaxService for DocumentSyntax {
    fn analyze(
        &self,
        text: Arc<dyn TextSource>,
        language: Arc<str>,
        cancel: Cancellation,
    ) -> Task<SyntaxUpdate> {
        let service = self.clone();
        Box::pin(async move {
            cancel.check()?;
            let old = service.baseline.lock().unwrap().clone();
            let version = text.version();
            if old.as_ref().is_some_and(|old| old.version() > version) {
                return Err(Error::Stale);
            }

            let (size_limit, line_limit) =
                settings::limits(service.syntax.settings.as_deref(), language.clone());
            let grammar = Grammar::for_id(&language);
            let plain = language.as_ref() == "plaintext";
            if grammar.is_none() && !plain {
                return Err(Error::Unsupported);
            }
            let compatible = old
                .as_deref()
                .filter(|s| s.language == language && s.line_limit == line_limit);

            let grammar = grammar.filter(|_| text.byte_len() < size_limit);
            let same_text = compatible.filter(|snapshot| {
                service.edit.is_none()
                    && Arc::ptr_eq(&snapshot.text, &text)
                    && snapshot.parsed.is_some() == grammar.is_some()
            });
            if let Some(snapshot) =
                same_text.filter(|snapshot| !snapshot.partial || service.visible.is_some())
            {
                return Ok(SyntaxUpdate {
                    snapshot: old.clone().unwrap(),
                    previous: Some(snapshot.version()),
                    changed_lines: Some(Arc::from([])),
                });
            }
            // A partial snapshot is only completed, never updated by an edit.
            let compatible = compatible.filter(|snapshot| !snapshot.partial);
            let deadline = Instant::now()
                .checked_add(service.syntax.budget.unwrap_or(Duration::from_millis(50)));
            let work = Work {
                cancel: &cancel,
                deadline,
            };
            let result = if let Some(grammar) = grammar {
                build(
                    Request {
                        text,
                        language,
                        line_limit,
                        grammar,
                        old: compatible,
                        complete: same_text.and_then(|s| s.parsed.as_ref()),
                        edit: service.edit.as_ref(),
                        visible: service.visible,
                        pool: service.syntax.pool.as_deref(),
                    },
                    &work,
                )
            } else {
                Ok((
                    Snapshot {
                        text,
                        language,
                        line_limit,
                        parsed: None,
                        partial: false,
                    },
                    None,
                ))
            };

            let (snapshot, changed_lines) = match result {
                Err(Error::Stale) if compatible.is_some() => {
                    cancel.check()?;
                    return Ok(SyntaxUpdate {
                        snapshot: old.clone().unwrap(),
                        previous: old.as_ref().map(|s| s.version()),
                        changed_lines: Some(Arc::from([])),
                    });
                }
                other => other?,
            };
            cancel.check()?;
            let snapshot = Arc::new(snapshot);
            let mut baseline = service.baseline.lock().unwrap();
            if baseline.as_ref().is_some_and(|current| {
                current.version() > version
                    || (current.version() == version
                        && old
                            .as_ref()
                            .is_none_or(|previous| !Arc::ptr_eq(previous, current)))
            }) {
                return Err(Error::Stale);
            }
            let replaced = baseline.replace(snapshot.clone());
            drop(baseline);
            drop(replaced);
            Ok(SyntaxUpdate {
                snapshot,
                previous: old.as_ref().map(|s| s.version()),
                changed_lines,
            })
        })
    }
}

struct Snapshot {
    text: Arc<dyn TextSource>,
    language: Arc<str>,
    line_limit: usize,
    parsed: Option<Parsed>,
    /// Only the requested visible lines carry tokens.
    partial: bool,
}

struct Parsed {
    source: Source,
    trees: ParseTrees,
    ranges: RowRanges,
    lines: Vec<LineTokens>,
    /// Lines reused from an older snapshot keep old offsets; rebase each once per snapshot.
    rebased: Vec<OnceLock<Arc<[Token]>>>,
}

#[derive(Clone)]
enum ParseTrees {
    Whole(Tree),
    Csharp(Arc<csharp_parts::Parts>),
}

impl ParseTrees {
    /// Parse from scratch. Partitioned C# also returns the captures of the whole document.
    fn full(
        grammar: Grammar,
        source: &Source,
        pool: Option<&WorkerPool>,
        work: &Work<'_>,
    ) -> Result<(Self, Option<Vec<grammar::Capture>>)> {
        if grammar == Grammar::Csharp {
            if let Some((parts, captures)) = csharp_parts::try_parse(source, pool, work)? {
                return Ok((Self::Csharp(parts.into()), Some(captures)));
            }
        }
        Ok((Self::Whole(grammar.parse(source, None, work)?), None))
    }

    fn whole(&self) -> Option<&Tree> {
        match self {
            Self::Whole(tree) => Some(tree),
            Self::Csharp(_) => None,
        }
    }

    fn at(&self, offset: usize) -> Option<&Tree> {
        match self {
            Self::Whole(tree) => Some(tree),
            Self::Csharp(parts) => parts.at(offset),
        }
    }
}

impl SyntaxSnapshot for Snapshot {
    fn version(&self) -> Version {
        self.text.version()
    }

    fn tokens(&self, line: Line) -> Result<Arc<[Token]>> {
        if let Some(parsed) = &self.parsed {
            let row = usize::try_from(line.0).map_err(|_| Error::OutOfBounds)?;
            let range = parsed.ranges.get(row)?;
            let tokens = parsed.rebased[row].get_or_init(|| parsed.lines[row].at(range.start));
            return Ok(tokens.clone());
        }
        let count = self.text.lines();
        if line.0 < count.known {
            Ok(rows::EMPTY_LINE.clone())
        } else if count.complete {
            Err(Error::OutOfBounds)
        } else {
            Err(Error::NotReady)
        }
    }

    fn folds(&self) -> Arc<[Fold]> {
        Arc::from([])
    }

    fn symbols(&self) -> Arc<[Symbol]> {
        Arc::from([])
    }
}

type BuildResult = (Snapshot, Option<Arc<[LineRange]>>);

struct Request<'a> {
    text: Arc<dyn TextSource>,
    language: Arc<str>,
    line_limit: usize,
    grammar: Grammar,
    /// Compatible, complete baseline of an earlier version.
    old: Option<&'a Snapshot>,
    /// Parse of a partial snapshot of this very text, to be completed.
    complete: Option<&'a Parsed>,
    edit: Option<&'a TextEdit>,
    visible: Option<LineRange>,
    pool: Option<&'a WorkerPool>,
}

fn build(request: Request<'_>, work: &Work<'_>) -> Result<BuildResult> {
    let Request {
        text,
        language,
        line_limit,
        grammar,
        old,
        complete,
        edit,
        visible,
        pool,
    } = request;
    work.cancel.check()?;
    let count = text.lines();
    if !count.complete {
        return Err(Error::NotReady);
    }
    let full_work = Work {
        cancel: work.cancel,
        deadline: None,
    };
    if let Some(parsed) = complete {
        // Same text as the partial snapshot: keep its parse, highlight every line.
        let scan = 0..parsed.ranges.len();
        let lines = highlight(
            grammar,
            &parsed.trees,
            &parsed.source,
            &parsed.ranges,
            scan,
            None,
            None,
            line_limit,
            pool,
            &full_work,
        )?
        .0;
        return Ok((
            snapshot(
                text,
                language,
                line_limit,
                parsed.source.clone(),
                parsed.trees.clone(),
                parsed.ranges.clone(),
                lines,
                false,
            ),
            None,
        ));
    }

    let old_parse = old.and_then(|s| s.parsed.as_ref());
    let patched = edit
        .zip(old)
        .zip(old_parse)
        .and_then(|((edit, old), parsed)| {
            (edit.version == old.version() && old.version() < text.version())
                .then(|| apply_edit(parsed, text.as_ref(), edit))
                .flatten()
        });
    // Only an edit against a whole tree can perform a regular incremental parse within the
    // refresh budget. Full rebuilds, and edits confined to one independent C# member, must
    // be allowed to advance the baseline even when the budget has already elapsed.
    let baseline = patched.as_ref().map(|(_, patch)| *patch).zip(old_parse);
    let whole_baseline = baseline.is_some_and(|(_, old)| old.trees.whole().is_some());
    let work = if whole_baseline { work } else { &full_work };
    let source = match patched {
        Some((source, _)) => source,
        None => Source::read(text.as_ref(), work)?,
    };
    work.check()?;

    // The previous tree after the edit and its reparse, for changed-range detection.
    let mut diff: Option<(Tree, Tree)> = None;
    let mut captured = None;
    let mut full = |source: &Source| -> Result<ParseTrees> {
        let (trees, captures) = ParseTrees::full(grammar, source, pool, work)?;
        captured = captures;
        Ok(trees)
    };
    let (trees, scan_all) = if let Some((patch, old)) = baseline {
        let reuse = match old.trees.at(patch.start_byte) {
            Some(tree) => {
                grammar.reparse_unchanged_token(tree, &old.source, &source, patch, work)?
            }
            None => false,
        };
        if reuse {
            // The re-lexed leaf has the same kind, width and token boundaries.
            (old.trees.clone(), false)
        } else {
            match &old.trees {
                ParseTrees::Whole(tree) => {
                    let mut edited = tree.clone();
                    edited.edit(&patch);
                    let updated = grammar.parse(&source, Some(&edited), work)?;
                    diff = Some((edited, updated.clone()));
                    (ParseTrees::Whole(updated), false)
                }
                ParseTrees::Csharp(parts) => match parts.reparse(&source, &patch, work)? {
                    Some((parts, edited, index)) => {
                        diff = Some((edited, parts.tree(index).clone()));
                        (ParseTrees::Csharp(parts.into()), false)
                    }
                    // Compare the new tokens against the previous snapshot across every row.
                    None => (full(&source)?, true),
                },
            }
        }
    } else {
        (full(&source)?, true)
    };

    let edited_ranges = match baseline {
        Some((patch, old)) => old
            .ranges
            .edit(text.as_ref(), count.known as usize, &patch, work)?,
        None => None,
    };
    let ranges = match edited_ranges {
        Some(ranges) => ranges,
        None => RowRanges::new(text.as_ref(), &source, count.known as usize, work)?,
    };
    let incremental = baseline
        .filter(|(_, old)| ranges.len() > 0 && old.ranges.len() > 0)
        .map(|(patch, old)| {
            rows::Incremental::new(patch, old, diff.as_ref().map(|(a, b)| (a, b)), &ranges)
        });
    let all = 0..ranges.len();
    let scan = match (&incremental, visible) {
        (Some(update), _) if !scan_all => update.scan.clone(),
        (None, Some(lines)) => {
            let start = (lines.start().0 as usize).min(all.end);
            start..(lines.end().0 as usize).clamp(start, all.end)
        }
        _ => all.clone(),
    };
    let scan = match &trees {
        ParseTrees::Whole(tree) if grammar == Grammar::Markdown && scan != all => {
            markdown_rows(tree, &source, &ranges, scan)
        }
        _ => scan,
    };
    let partial = incremental.is_none() && scan != all;
    let captured = captured.filter(|_| scan == all);
    let (lines, changed_lines) = highlight(
        grammar,
        &trees,
        &source,
        &ranges,
        scan,
        captured,
        incremental,
        line_limit,
        pool,
        work,
    )?;
    work.check()?;
    Ok((
        snapshot(
            text, language, line_limit, source, trees, ranges, lines, partial,
        ),
        changed_lines.filter(|_| !partial),
    ))
}

#[allow(clippy::too_many_arguments)]
fn highlight(
    grammar: Grammar,
    trees: &ParseTrees,
    source: &Source,
    ranges: &RowRanges,
    scan: Range<usize>,
    captured: Option<Vec<grammar::Capture>>,
    incremental: Option<rows::Incremental<'_>>,
    line_limit: usize,
    pool: Option<&WorkerPool>,
    work: &Work<'_>,
) -> Result<rows::Rows> {
    let bytes = ranges.bytes(scan.clone());
    let captures = match (captured, trees) {
        (Some(captures), _) => captures,
        (None, ParseTrees::Whole(tree)) => {
            let mut captures = Vec::new();
            grammar.capture(tree, source, bytes, &mut captures, work)?;
            captures
        }
        (None, ParseTrees::Csharp(parts)) => {
            let mut captures = Vec::new();
            csharp_parts::capture(parts, pool, source, bytes, &mut captures, work)?;
            captures
        }
    };

    rows::assemble(
        ranges,
        source,
        scan,
        captures,
        incremental,
        line_limit,
        work,
    )
}

#[allow(clippy::too_many_arguments)]
fn snapshot(
    text: Arc<dyn TextSource>,
    language: Arc<str>,
    line_limit: usize,
    source: Source,
    trees: ParseTrees,
    ranges: RowRanges,
    lines: Vec<LineTokens>,
    partial: bool,
) -> Snapshot {
    Snapshot {
        text,
        language,
        line_limit,
        partial,
        parsed: Some(Parsed {
            source,
            trees,
            ranges,
            rebased: std::iter::repeat_with(OnceLock::new)
                .take(lines.len())
                .collect(),
            lines,
        }),
    }
}

/// Widen Markdown rows to whole inline regions and code fences, so an injected region is
/// either highlighted again in full or reused in full.
fn markdown_rows(
    tree: &Tree,
    source: &Source,
    ranges: &RowRanges,
    rows: Range<usize>,
) -> Range<usize> {
    if rows.is_empty() {
        return rows;
    }
    let mut bytes = ranges.bytes(rows.clone());
    loop {
        let expanded = widen(tree.root_node(), bytes.clone());
        if expanded == bytes {
            break;
        }
        bytes = expanded;
    }
    let bytes = bytes.start..bytes.end.min(source.len());
    let start = ranges.row_at(bytes.start);
    let end = ranges.row_at(bytes.end.saturating_sub(1).max(bytes.start)) + 1;
    start.min(rows.start)..end.max(rows.end)
}

fn widen(node: Node<'_>, range: Range<usize>) -> Range<usize> {
    let mut out = range.clone();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.end_byte() < range.start || child.start_byte() > range.end {
            continue;
        }
        if matches!(
            child.kind(),
            "inline" | "pipe_table_cell" | "fenced_code_block" | "setext_heading"
        ) {
            out.start = out.start.min(child.start_byte());
            out.end = out.end.max(child.end_byte());
        } else {
            let inner = widen(child, range.clone());
            out.start = out.start.min(inner.start);
            out.end = out.end.max(inner.end);
        }
    }
    out
}

/// Apply one replacement to the previous source. Only the edit and [`EDIT_CONTEXT`] bytes
/// around it are compared with the new text, together with its length; the rest of the
/// document is shared with the previous source instead of being copied or compared.
fn apply_edit(
    parsed: &Parsed,
    text: &dyn TextSource,
    edit: &TextEdit,
) -> Option<(Source, InputEdit)> {
    let start = usize::try_from(edit.range.start().0).ok()?;
    let old_end = usize::try_from(edit.range.end().0).ok()?;
    let old_len = parsed.source.len();
    let new_len = old_len
        .checked_sub(old_end.checked_sub(start)?)?
        .checked_add(edit.inserted.len())?;
    if old_end > old_len || text.byte_len() != new_len as u64 {
        return None;
    }
    let source = parsed.source.splice(start..old_end, &edit.inserted)?;
    let new_end = start + edit.inserted.len();
    let mut low = start.saturating_sub(EDIT_CONTEXT);
    while low > 0 && source.byte(low).is_some_and(is_continuation) {
        low -= 1;
    }
    let mut high = (new_end + EDIT_CONTEXT).min(new_len);
    while high < new_len && source.byte(high).is_some_and(is_continuation) {
        high += 1;
    }
    let mut actual = Vec::with_capacity(high - low);
    text.visit_chunks(
        Span::new(ByteOffset(low as u64), ByteOffset(high as u64)).ok()?,
        &mut |chunk| actual.extend_from_slice(chunk.as_bytes()),
    )
    .ok()?;
    if actual != *source.bytes(low..high) {
        return None;
    }
    let start_position = point_at(parsed.trees.at(start)?, &parsed.source, start);
    Some((
        source,
        InputEdit {
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: new_end,
            start_position,
            old_end_position: advance(start_position, &parsed.source.bytes(start..old_end)),
            new_end_position: advance(start_position, edit.inserted.as_bytes()),
        },
    ))
}

fn is_continuation(byte: u8) -> bool {
    (0x80..0xc0).contains(&byte)
}

/// Tree-sitter position of `offset`, from the nearest preceding node of the tree.
fn point_at(tree: &Tree, source: &Source, offset: usize) -> Point {
    let root = tree.root_node();
    let mut base = (0, Point::default());
    if root.end_byte() <= offset {
        base = (root.end_byte(), root.end_position());
    } else {
        let mut cursor = root.walk();
        while cursor.goto_first_child_for_byte(offset).is_some() {
            let node = cursor.node();
            if node.start_byte() <= offset {
                base = (node.start_byte(), node.start_position());
                continue;
            }
            if cursor.goto_previous_sibling() {
                let previous = cursor.node();
                if previous.end_byte() <= offset && previous.end_byte() >= base.0 {
                    base = (previous.end_byte(), previous.end_position());
                }
            }
            break;
        }
    }
    advance(base.1, &source.bytes(base.0..offset))
}

fn advance(mut point: Point, bytes: &[u8]) -> Point {
    for &byte in bytes {
        if byte == b'\n' {
            point.row += 1;
            point.column = 0;
        } else {
            point.column += 1;
        }
    }
    point
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_lookup_matches_a_scan_from_the_start() {
        let text = "class C {\n  int a; // é\r\n\n  void M() { }\n}\n";
        let source = Source::from_str(text);
        let cancel = Cancellation::default();
        let work = Work {
            cancel: &cancel,
            deadline: None,
        };
        let tree = Grammar::Csharp.parse(&source, None, &work).unwrap();
        for offset in 0..=text.len() {
            if text.is_char_boundary(offset) {
                assert_eq!(
                    point_at(&tree, &source, offset),
                    advance(Point::default(), &text.as_bytes()[..offset]),
                    "{offset}"
                );
            }
        }
    }
}
