//! Tree-sitter highlighting behind the frozen syntax contracts.

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
    sync::{Arc, Mutex},
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

    /// Maximum time to wait for a refresh when a compatible previous snapshot exists.
    /// The initial parse has no fallback and runs to completion or cancellation.
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
            let deadline = compatible.and_then(|_| {
                Instant::now()
                    .checked_add(service.syntax.budget.unwrap_or(Duration::from_millis(50)))
            });
            let work = Work {
                cancel: &cancel,
                deadline,
            };
            let result = if let Some(grammar) = grammar.filter(|_| text.byte_len() < size_limit) {
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
    tree: Tree,
    ranges: RowRanges,
    lines: Vec<LineTokens>,
}

impl SyntaxSnapshot for Snapshot {
    fn version(&self) -> Version {
        self.text.version()
    }

    fn tokens(&self, line: Line) -> Result<Arc<[Token]>> {
        if let Some(parsed) = &self.parsed {
            let row = usize::try_from(line.0).map_err(|_| Error::OutOfBounds)?;
            let range = parsed.ranges.get(row)?;
            return Ok(parsed.lines[row].at(range.start));
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
    work.check()?;
    let count = text.lines();
    if !count.complete {
        return Err(Error::NotReady);
    }
    let mut source = String::with_capacity(text.byte_len() as usize);
    text.visit_chunks(
        Span::new(ByteOffset(0), ByteOffset(text.byte_len()))?,
        &mut |s| {
            if !work.cancelled() {
                source.push_str(s);
            }
        },
    )?;
    work.check()?;

    let old_parse = old.and_then(|s| s.parsed.as_ref());
    let patch = edit
        .zip(old)
        .zip(old_parse)
        .and_then(|((edit, old), parsed)| {
            (edit.version == old.version() && old.version() < text.version())
                .then(|| input_edit(&parsed.source, &source, edit))
                .flatten()
        });
    let baseline = patch.zip(old_parse);
    let (tree, edited_tree) = if let Some((patch, old)) = baseline {
        if grammar.reparse_unchanged_token(&old.tree, &old.source, &source, patch, work)? {
            // The re-lexed leaf has the same kind, width and surrounding token boundaries.
            // Tree-sitter nodes store positions and kinds, so this tree also describes the new text.
            (old.tree.clone(), None)
        } else {
            let mut edited = old.tree.clone();
            edited.edit(&patch);
            (
                grammar.parse(source.as_bytes(), Some(&edited), work)?,
                Some(edited),
            )
        }
    } else {
        (grammar.parse(source.as_bytes(), None, work)?, None)
    };
    let ranges = RowRanges::new(text.as_ref(), count.known as usize, work)?;
    let incremental = baseline.map(|(patch, old)| {
        rows::Incremental::new(patch, old, edited_tree.as_ref(), &tree, &ranges)
    });

    let scan = match &incremental {
        Some(update) if grammar != Grammar::Markdown => update.scan.clone(),
        _ => 0..ranges.len(),
    };
    let mut captures = Vec::new();
    grammar.capture(
        &tree,
        source.as_bytes(),
        ranges.bytes(scan.clone()),
        &mut captures,
        work,
    )?;
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
                tree,
                ranges,
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
