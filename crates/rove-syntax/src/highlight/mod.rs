//! Tree-sitter highlighting behind the frozen syntax contracts.

mod csharp_parts;
mod grammar;
mod rows;
mod settings;

pub use settings::{language_for_path, register_settings};

use crate::{Fold, Symbol, SyntaxService, SyntaxSnapshot, SyntaxUpdate, Token};
use grammar::{Grammar, Work};
use rove_base::{ByteOffset, Cancellation, Error, Line, Result, Span, Task, TextEdit, Version};
use rove_buffer::TextSource;
use rove_config::Settings;
use rows::{LineTokens, RowRanges};
use std::{
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tree_sitter::{InputEdit, Point, Tree};

/// Executor-neutral highlighter. Poll the returned task on a background executor.
#[derive(Clone, Default)]
pub struct TreeSitterSyntax {
    settings: Option<Arc<dyn Settings>>,
    budget: Option<Duration>,
}

impl TreeSitterSyntax {
    pub fn with_settings(mut self, settings: Arc<dyn Settings>) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Time budget for a verified incremental edit. Full rebuilds run to completion
    /// or cancellation so the baseline can recover after a skipped revision.
    pub fn with_refresh_budget(mut self, budget: Duration) -> Self {
        self.budget = Some(budget);
        self
    }

    /// Allocate a baseline for one open document. Versions are local to that document.
    pub fn document(&self) -> DocumentSyntax {
        DocumentSyntax {
            syntax: self.clone(),
            baseline: Arc::default(),
            edit: None,
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
            if let Some(snapshot) = old.as_ref().filter(|snapshot| {
                compatible.is_some()
                    && service.edit.is_none()
                    && Arc::ptr_eq(&snapshot.text, &text)
                    && snapshot.parsed.is_some() == grammar.is_some()
            }) {
                return Ok(SyntaxUpdate {
                    snapshot: snapshot.clone(),
                    previous: Some(version),
                    changed_lines: Some(Arc::from([])),
                });
            }
            let deadline = Instant::now()
                .checked_add(service.syntax.budget.unwrap_or(Duration::from_millis(50)));
            let work = Work {
                cancel: &cancel,
                deadline,
            };
            let result = if let Some(grammar) = grammar {
                build(
                    text,
                    language,
                    line_limit,
                    grammar,
                    compatible,
                    service.edit.as_ref(),
                    &work,
                )
            } else {
                Ok((
                    Snapshot {
                        text,
                        language,
                        line_limit,
                        parsed: None,
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
}

struct Parsed {
    source: String,
    trees: ParseTrees,
    ranges: RowRanges,
    lines: Vec<LineTokens>,
    /// Lines reused from an older snapshot keep old offsets; rebase each once per snapshot.
    rebased: Vec<OnceLock<Arc<[Token]>>>,
}

#[derive(Clone)]
enum ParseTrees {
    Whole(Tree),
    Csharp(Arc<[csharp_parts::Part]>),
}

impl ParseTrees {
    fn full(grammar: Grammar, source: &str, work: &Work<'_>) -> Result<Self> {
        if grammar == Grammar::Csharp {
            if let Some(parts) = csharp_parts::try_parse(source, work)? {
                return Ok(Self::Csharp(parts.into()));
            }
        }
        Ok(Self::Whole(grammar.parse(source.as_bytes(), None, work)?))
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
            Self::Csharp(parts) => parts
                .iter()
                .find(|part| part.contains(offset))
                .map(|part| &part.tree),
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
            Ok(Arc::from([]))
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

type BuildResult = (Snapshot, Option<Arc<[rove_base::LineRange]>>);

fn build(
    text: Arc<dyn TextSource>,
    language: Arc<str>,
    line_limit: usize,
    grammar: Grammar,
    old: Option<&Snapshot>,
    edit: Option<&TextEdit>,
    work: &Work<'_>,
) -> Result<BuildResult> {
    work.cancel.check()?;
    let count = text.lines();
    if !count.complete {
        return Err(Error::NotReady);
    }
    let mut source = String::with_capacity(text.byte_len() as usize);
    text.visit_chunks(
        Span::new(ByteOffset(0), ByteOffset(text.byte_len()))?,
        &mut |s| {
            if work.cancel.check().is_ok() {
                source.push_str(s);
            }
        },
    )?;
    work.cancel.check()?;

    let old_parse = old.and_then(|s| s.parsed.as_ref());
    let patch = edit
        .zip(old)
        .zip(old_parse)
        .and_then(|((edit, old), parsed)| {
            (edit.version == old.version() && old.version() < text.version())
                .then(|| input_edit(&parsed.source, &source, edit))
                .flatten()
        });
    // Establish the edit before applying its budget. Invalid or missing edits take the
    // full rebuild path and must advance the baseline even when the budget is exhausted.
    let work = &Work {
        cancel: work.cancel,
        deadline: patch.and(work.deadline),
    };
    work.check()?;
    let baseline = patch.zip(old_parse);
    let (trees, edited_whole, scan_all) = if let Some((patch, old)) = baseline {
        let reuse = match old.trees.at(patch.start_byte) {
            Some(tree) => {
                grammar.reparse_unchanged_token(tree, &old.source, &source, patch, work)?
            }
            None => false,
        };
        if reuse {
            // The re-lexed leaf has the same kind, width and token boundaries.
            (old.trees.clone(), None, false)
        } else if let Some(tree) = old.trees.whole() {
            let mut edited = tree.clone();
            edited.edit(&patch);
            let updated = grammar.parse(source.as_bytes(), Some(&edited), work)?;
            (ParseTrees::Whole(updated), Some(edited), false)
        } else {
            // Independent member trees cannot be passed as a whole-tree baseline.
            // Compare the new tokens against the previous snapshot across every row.
            (ParseTrees::full(grammar, &source, work)?, None, true)
        }
    } else {
        (ParseTrees::full(grammar, &source, work)?, None, true)
    };
    // A same-width edit without line separators keeps every line range at the same offset.
    let unchanged_ranges = baseline.filter(|(patch, old)| {
        patch.old_end_byte == patch.new_end_byte
            && old.ranges.len() as u64 == count.known
            && !old.source[patch.start_byte..patch.old_end_byte].contains(['\r', '\n'])
            && !source[patch.start_byte..patch.new_end_byte].contains(['\r', '\n'])
    });
    let ranges = if let Some((_, old)) = unchanged_ranges {
        old.ranges.clone()
    } else {
        RowRanges::new(text.as_ref(), count.known as usize, work)?
    };
    let incremental = baseline.map(|(patch, old)| {
        rows::Incremental::new(
            patch,
            old,
            edited_whole.as_ref().zip(trees.whole()),
            &ranges,
        )
    });
    let scan = match &incremental {
        Some(update) if !scan_all && grammar != Grammar::Markdown => update.scan.clone(),
        _ => 0..ranges.len(),
    };
    let bytes = ranges.bytes(scan.clone());
    let mut captures = Vec::new();
    match &trees {
        ParseTrees::Whole(tree) => {
            grammar.capture(tree, source.as_bytes(), bytes, &mut captures, work)?
        }
        ParseTrees::Csharp(parts) => {
            csharp_parts::capture(parts, source.as_bytes(), bytes, &mut captures, work)?
        }
    }
    let (lines, changed_lines) = rows::assemble(
        &ranges,
        &source,
        scan,
        captures,
        incremental,
        line_limit,
        work,
    )?;
    work.check()?;

    Ok((
        Snapshot {
            text,
            language,
            line_limit,
            parsed: Some(Parsed {
                source,
                trees,
                ranges,
                rebased: std::iter::repeat_with(OnceLock::new)
                    .take(lines.len())
                    .collect(),
                lines,
            }),
        },
        changed_lines,
    ))
}

/// Verify that the immutable snapshots differ only by the supplied replacement.
fn input_edit(old: &str, new: &str, edit: &TextEdit) -> Option<InputEdit> {
    let start = usize::try_from(edit.range.start().0).ok()?;
    let old_end = usize::try_from(edit.range.end().0).ok()?;
    let new_end = start.checked_add(edit.inserted.len())?;
    if old.get(..start)? != new.get(..start)?
        || old.get(old_end..)? != new.get(new_end..)?
        || new.get(start..new_end)? != edit.inserted.as_ref()
    {
        return None;
    }
    let start_position = point(&old.as_bytes()[..start]);
    Some(InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position,
        old_end_position: advance(start_position, &old.as_bytes()[start..old_end]),
        new_end_position: advance(start_position, edit.inserted.as_bytes()),
    })
}

fn point(bytes: &[u8]) -> Point {
    advance(Point::new(0, 0), bytes)
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
