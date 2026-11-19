//! 智能扩选与括号命令。

use super::{brackets::MatchBrackets, columns, edit::EditOutcome, CodeView, Reveal, Selection};
use gpui::{Context, Window};
use rove_base::{ByteOffset, ByteRange, DocumentId, Position, Span, Utf8Column};
use rove_buffer::{
    edit::{
        word::{self, MoveWord, WordClassifier},
        Replacement, Transaction,
    },
    TextSource,
};
use rove_syntax::highlight::DocumentHighlighter;
use std::{ops::Range, sync::Arc};

pub(crate) const EXPAND: &str = "editor.action.smartSelect.expand";
pub(crate) const SHRINK: &str = "editor.action.smartSelect.shrink";
pub(crate) const GROW: &str = "editor.action.smartSelect.grow";
pub(crate) const JUMP_TO_BRACKET: &str = "editor.action.jumpToBracket";
pub(crate) const SELECT_TO_BRACKET: &str = "editor.action.selectToBracket";
pub(crate) const REMOVE_BRACKETS: &str = "editor.action.removeBrackets";

pub(crate) struct SmartSelection {
    chains: Vec<Vec<Selection>>,
    step: usize,
}

fn position(line: u64, column: u64) -> Position {
    Position {
        line: rove_base::Line(line),
        column: Utf8Column(column),
    }
}

fn selection_bytes(text: &dyn TextSource, selection: Selection) -> Option<ByteRange> {
    Span::new(
        text.offset(selection.start()).ok()?,
        text.offset(selection.end()).ok()?,
    )
    .ok()
}

fn to_selection(text: &dyn TextSource, range: &ByteRange, reversed: bool) -> Option<Selection> {
    let start = text.position(range.start()).ok()?;
    let end = text.position(range.end()).ok()?;
    Some(if reversed {
        Selection::new(end, start)
    } else {
        Selection::new(start, end)
    })
}

impl CodeView {
    pub(crate) fn set_selection_syntax(
        &mut self,
        document: DocumentId,
        syntax: Arc<dyn DocumentHighlighter>,
    ) {
        self.selection_syntax = Some((document, syntax));
    }

    /// 已经缓存的扩选链继续沿同一路径移动。普通选区变化会清掉状态。
    pub fn smart_select(&mut self, expand: bool, cx: &mut Context<Self>) {
        let mut state = match self.smart_selection.take() {
            Some(state) => state,
            None if expand => SmartSelection {
                chains: self
                    .selections()
                    .into_iter()
                    .map(|sel| self.selection_chain(sel))
                    .collect(),
                step: 0,
            },
            None => return,
        };
        let max = state
            .chains
            .iter()
            .map(|chain| chain.len().saturating_sub(1))
            .max()
            .unwrap_or(0);
        let next = if expand {
            (state.step + 1).min(max)
        } else {
            state.step.saturating_sub(1)
        };
        if next != state.step {
            let selections = state
                .chains
                .iter()
                .map(|chain| chain[next.min(chain.len() - 1)])
                .collect();
            self.set_selections(selections, Some(Reveal::CenterIfOutside), cx);
            state.step = next;
        }
        self.smart_selection = Some(state);
    }

    fn selection_chain(&self, input: Selection) -> Vec<Selection> {
        let mut chain = vec![input];
        let Some(text) = self.text.as_deref() else {
            return chain;
        };
        let Some(mut current) = selection_bytes(text, input) else {
            return chain;
        };
        let original = current.clone();
        let reversed = input.active < input.anchor;
        let mut push = |range: ByteRange| {
            if range.start() <= current.start() && current.end() <= range.end() && current != range
            {
                if let Some(selection) = to_selection(text, &range, reversed) {
                    chain.push(selection);
                    current = range;
                }
            }
        };
        let active = input.active;
        if let Some(line) = columns::read_line(text, active.line.0) {
            let word = self.word_rule().word_at(&line, active.column.0 as usize);
            if let Some((start, end)) = word {
                let from = position(active.line.0, start as u64);
                let to = position(active.line.0, end as u64);
                if self.options.smart_select_subwords {
                    let classifier = WordClassifier::new(&self.options.word_separators);
                    let subword_right =
                        word::move_word(text, &classifier, active, MoveWord::PartRight, false)
                            .ok()
                            .map(|pos| pos.min(to));
                    if let Some(right) = subword_right {
                        if let Ok(left) =
                            word::move_word(text, &classifier, right, MoveWord::PartLeft, false)
                        {
                            if let Some(range) =
                                selection_bytes(text, Selection::new(left.max(from), right))
                            {
                                push(range);
                            }
                        }
                    }
                }
                if let Some(range) = selection_bytes(text, Selection::new(from, to)) {
                    push(range);
                }
            }
        }

        let syntax_ranges = self
            .selection_syntax
            .as_ref()
            .zip(self.language.as_deref())
            .map_or_else(Vec::new, |((document, syntax), language)| {
                syntax.selection_ranges(*document, text.version(), language, &original)
            });
        if syntax_ranges.is_empty() {
            if let Some(range) = line_content(
                text,
                input.start().line.0,
                input.end().line.0,
                self.options.smart_select_whitespace,
            ) {
                push(range);
            }
            if let Some((open, close)) = self.bracket_selections(active, true) {
                if let Some(range) = selection_bytes(text, Selection::new(open, close)) {
                    push(range);
                }
            }
        } else {
            for range in syntax_ranges {
                if self.options.smart_select_whitespace {
                    let padded = including_adjacent_whitespace(text, &range);
                    push(range);
                    if let Some(padded) = padded {
                        push(padded);
                    }
                } else {
                    push(range);
                }
            }
        }
        if let Ok(all) = Span::new(ByteOffset(0), ByteOffset(text.byte_len())) {
            push(all);
        }
        chain
    }

    fn bracket_selections(&self, at: Position, include: bool) -> Option<(Position, Position)> {
        let pair = self.find_brackets_at(at, MatchBrackets::Always)?;
        let open = if include {
            pair.open.1.start
        } else {
            pair.open.1.end
        };
        let close = if include {
            pair.close.1.end
        } else {
            pair.close.1.start
        };
        Some((
            position(pair.open.0, u64::from(open)),
            position(pair.close.0, u64::from(close)),
        ))
    }

    pub fn select_to_bracket(&mut self, include: bool, cx: &mut Context<Self>) {
        let old = self.selections();
        let selections = old
            .iter()
            .map(|sel| {
                self.bracket_selections(sel.start(), include)
                    .map_or(*sel, |(open, close)| {
                        if sel.active < sel.anchor {
                            Selection::new(close, open)
                        } else {
                            Selection::new(open, close)
                        }
                    })
            })
            .collect::<Vec<_>>();
        if selections != old {
            self.set_selections(selections, Some(Reveal::CenterIfOutside), cx);
        }
    }

    fn jump_to_bracket(&mut self, cx: &mut Context<Self>) {
        let old = self.selections();
        let selections = old
            .iter()
            .map(|sel| {
                let at = sel.start();
                let Some(pair) = self.find_brackets_at(at, MatchBrackets::Always) else {
                    return *sel;
                };
                let close = pair.close.0 == at.line.0
                    && (u64::from(pair.close.1.start)..=u64::from(pair.close.1.end))
                        .contains(&at.column.0);
                let (line, range) = if close { pair.open } else { pair.close };
                Selection::caret(position(line, u64::from(range.start)))
            })
            .collect::<Vec<_>>();
        if selections != old {
            self.set_selections(selections, Some(Reveal::CenterIfOutside), cx);
        }
    }

    fn remove_brackets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_read_only() {
            self.edit_blocked(cx);
            return;
        }
        let Some(text) = self.text.as_deref() else {
            return;
        };
        let mut ranges = Vec::new();
        for selection in self.selections() {
            let Some(pair) = self.find_brackets_at(selection.start(), MatchBrackets::Always) else {
                continue;
            };
            for (line, bytes) in [pair.open, pair.close] {
                let marker = |col| position(line, u64::from(col));
                if let Some(range) =
                    selection_bytes(text, Selection::new(marker(bytes.start), marker(bytes.end)))
                {
                    ranges.push(range);
                }
            }
        }
        ranges.sort_unstable_by_key(|range| (range.start(), range.end()));
        ranges.dedup();
        if ranges.is_empty() {
            return;
        }
        let version = text.version();
        self.run_edit(Some(window), cx, move |session, snap, now| {
            if snap.version() != version {
                return Ok(EditOutcome::default());
            }
            let txn =
                Transaction::new(ranges.into_iter().map(Replacement::delete).collect())?.verbatim();
            Ok(EditOutcome {
                edits: vec![session.edit(txn, now)?],
                selection: None,
            })
        });
    }

    pub(crate) fn execute_smart_selection(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match id {
            EXPAND | GROW => self.smart_select(true, cx),
            SHRINK => self.smart_select(false, cx),
            JUMP_TO_BRACKET => self.jump_to_bracket(cx),
            SELECT_TO_BRACKET => self.select_to_bracket(true, cx),
            REMOVE_BRACKETS => self.remove_brackets(window, cx),
            _ => return false,
        }
        true
    }
}

fn trim_line(line: &str) -> Range<usize> {
    let start = line.len() - line.trim_start().len();
    let end = line.trim_end().len().max(start);
    start..end
}

fn line_content(
    text: &dyn TextSource,
    first: u64,
    last: u64,
    whitespace: bool,
) -> Option<ByteRange> {
    let a = text.line_range(rove_base::Line(first)).ok()?;
    let b = text.line_range(rove_base::Line(last)).ok()?;
    if whitespace {
        return Span::new(a.start(), b.end()).ok();
    }
    let left = trim_line(&columns::read_line(text, first)?);
    let right = if last == first {
        left.clone()
    } else {
        trim_line(&columns::read_line(text, last)?)
    };
    Span::new(
        ByteOffset(a.start().0 + left.start as u64),
        ByteOffset(b.start().0 + right.end as u64),
    )
    .ok()
}

fn including_adjacent_whitespace(text: &dyn TextSource, range: &ByteRange) -> Option<ByteRange> {
    let start = text.position(range.start()).ok()?;
    let end = text.position(range.end()).ok()?;
    let before = columns::read_line(text, start.line.0)?;
    let after = if end.line == start.line {
        before.clone()
    } else {
        columns::read_line(text, end.line.0)?
    };
    let left = before.get(..start.column.0 as usize)?;
    let right = after.get(end.column.0 as usize..)?;
    let first = if left.trim().is_empty() {
        range.start().0 - left.len() as u64
    } else {
        range.start().0
    };
    let last = if right.trim().is_empty() {
        range.end().0 + right.len() as u64
    } else {
        range.end().0
    };
    Span::new(ByteOffset(first), ByteOffset(last)).ok()
}
