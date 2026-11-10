use super::{
    grammar::{Capture, Work},
    Parsed,
};
use crate::Token;
use rove_base::{ByteOffset, Error, Line, LineRange, Result, Span};
use rove_buffer::TextSource;
use std::{
    cmp::Reverse,
    ops::Range,
    sync::{Arc, LazyLock},
};
use tree_sitter::{InputEdit, Tree};

#[derive(Clone)]
pub(super) struct RowRanges(Arc<[Range<usize>]>);

impl RowRanges {
    pub fn new(text: &dyn TextSource, count: usize, work: &Work<'_>) -> Result<Self> {
        let mut ranges = Vec::with_capacity(count);
        for row in 0..count {
            if row % 256 == 0 {
                work.check()?;
            }
            let line = text.line_range(Line(row as u64))?;
            ranges.push(line.start().0 as usize..line.end().0 as usize);
        }
        Ok(Self(ranges.into()))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn get(&self, row: usize) -> Result<&Range<usize>> {
        self.0.get(row).ok_or(Error::OutOfBounds)
    }

    fn row_at(&self, offset: usize) -> usize {
        self.0
            .partition_point(|r| r.start <= offset)
            .saturating_sub(1)
    }

    pub fn bytes(&self, scan: Range<usize>) -> Range<usize> {
        self.0[scan.start].start..self.0[scan.end - 1].end
    }
}

/// Unchanged lines share token storage even when their absolute byte offsets move.
#[derive(Clone)]
pub(super) struct LineTokens {
    origin: usize,
    tokens: Arc<[Token]>,
}

impl LineTokens {
    pub fn at(&self, origin: usize) -> Arc<[Token]> {
        if self.origin == origin || self.tokens.is_empty() {
            return self.tokens.clone();
        }
        self.tokens
            .iter()
            .map(|token| {
                let rebase =
                    |offset: ByteOffset| ByteOffset(offset.0 - self.origin as u64 + origin as u64);
                Token {
                    range: Span::new(rebase(token.range.start()), rebase(token.range.end()))
                        .unwrap(),
                    scopes: token.scopes.clone(),
                }
            })
            .collect()
    }

    fn same_highlighting(&self, other: &Self) -> bool {
        self.tokens.len() == other.tokens.len()
            && self.tokens.iter().zip(other.tokens.iter()).all(|(a, b)| {
                a.range.start().0 - self.origin as u64 == b.range.start().0 - other.origin as u64
                    && a.range.end().0 - self.origin as u64 == b.range.end().0 - other.origin as u64
                    && a.scopes == b.scopes
            })
    }
}

pub(super) struct Incremental<'a> {
    old: &'a Parsed,
    edited: Range<usize>,
    shift: isize,
    pub scan: Range<usize>,
}

impl<'a> Incremental<'a> {
    pub fn new(
        patch: InputEdit,
        old: &'a Parsed,
        changed: Option<(&Tree, &Tree)>,
        ranges: &RowRanges,
    ) -> Self {
        let first = ranges.row_at(patch.start_byte);
        let last = ranges.row_at(patch.new_end_byte);
        let old_last = old.ranges.row_at(patch.old_end_byte);
        let edited = first..last + 1;
        let mut scan = edited.clone();
        for change in changed
            .into_iter()
            .flat_map(|(old, new)| old.changed_ranges(new))
        {
            scan.start = scan.start.min(ranges.row_at(change.start_byte));
            scan.end = scan
                .end
                .max(ranges.row_at(change.end_byte.saturating_sub(1)) + 1);
        }
        Self {
            old,
            edited,
            shift: last as isize - old_last as isize,
            scan,
        }
    }

    fn previous(&self, row: usize) -> Option<&LineTokens> {
        if self.edited.contains(&row) {
            return None;
        }
        let old_row = if row < self.edited.start {
            row
        } else {
            row.checked_add_signed(-self.shift)?
        };
        self.old.lines.get(old_row)
    }
}

static EMPTY_LINE: LazyLock<Arc<[Token]>> = LazyLock::new(|| Arc::from([]));

type Rows = (Vec<LineTokens>, Option<Arc<[LineRange]>>);

pub(super) fn assemble(
    ranges: &RowRanges,
    source: &str,
    scan: Range<usize>,
    mut captures: Vec<Capture>,
    baseline: Option<Incremental<'_>>,
    limit: usize,
    work: &Work<'_>,
) -> Result<Rows> {
    captures.sort_unstable_by_key(|c| (c.start, Reverse(c.end), c.priority));
    let mut lines = Vec::with_capacity(ranges.len());
    let mut changed: Vec<Range<usize>> = Vec::new();
    let mut active = Vec::<&Capture>::new();
    let mut pending = captures.iter().peekable();
    for (row, range) in ranges.0.iter().enumerate() {
        if row % 256 == 0 {
            work.check()?;
        }
        let previous = baseline.as_ref().and_then(|old| old.previous(row));
        if !scan.contains(&row) {
            lines.push(previous.ok_or(Error::InvalidInput)?.clone());
            continue;
        }
        active.retain(|c| c.end > range.start);
        while pending.peek().is_some_and(|c| c.start < range.end) {
            let capture = pending.next().unwrap();
            if capture.end > range.start {
                active.push(capture);
            }
        }
        let tokens = if !active.is_empty()
            && !range.is_empty()
            && (range.len() <= limit
                || (range.len() <= limit.saturating_mul(3)
                    && source[range.clone()].encode_utf16().take(limit + 1).count() <= limit))
        {
            project_line(range, &active)
        } else {
            EMPTY_LINE.clone()
        };
        let current = LineTokens {
            origin: range.start,
            tokens,
        };
        if let Some(prior) = previous.filter(|prior| prior.same_highlighting(&current)) {
            lines.push(prior.clone());
        } else {
            lines.push(current);
            if baseline.is_some() {
                match changed.last_mut() {
                    Some(last) if last.end == row => last.end += 1,
                    _ => changed.push(row..row + 1),
                }
            }
        }
    }

    let changed = baseline.map(|_| {
        changed
            .into_iter()
            .map(|r| Span::new(Line(r.start as u64), Line(r.end as u64)).unwrap())
            .collect()
    });
    Ok((lines, changed))
}

/// Convert overlapping tree captures into disjoint tokens with outer-to-inner scope stacks.
fn project_line(range: &Range<usize>, captures: &[&Capture]) -> Arc<[Token]> {
    if captures.is_empty() {
        return EMPTY_LINE.clone();
    }
    let mut tokens = Vec::with_capacity(captures.len());
    let mut index = 0;
    while let Some(first) = captures.get(index) {
        let start = first.start.max(range.start);
        let end = first.end.min(range.end);
        let mut next = index + 1;
        let mut group_end = end;
        while let Some(capture) = captures.get(next).filter(|c| c.start < group_end) {
            group_end = group_end.max(capture.end.min(range.end));
            next += 1;
        }
        if next == index + 1 {
            tokens.push(token(start, end, Arc::clone(first.scopes)));
        } else {
            project_overlaps(start..group_end, &captures[index..next], &mut tokens);
        }
        index = next;
    }
    tokens.into()
}

fn project_overlaps(range: Range<usize>, captures: &[&Capture], tokens: &mut Vec<Token>) {
    let mut edges = Vec::with_capacity(captures.len() * 2);
    for c in captures {
        edges.push(c.start.max(range.start));
        edges.push(c.end.min(range.end));
    }
    edges.sort_unstable();
    edges.dedup();
    for pair in edges.windows(2) {
        let [start, end] = [pair[0], pair[1]];
        let mut covering = captures
            .iter()
            .filter(|c| c.start <= start && end <= c.end)
            .peekable();
        let Some(first) = covering.next() else {
            continue;
        };
        let scopes = if covering.peek().is_none() {
            Arc::clone(first.scopes)
        } else {
            let mut stack = Vec::new();
            for capture in std::iter::once(first).chain(covering) {
                for scope in capture.scopes.iter() {
                    if !stack.contains(scope) {
                        stack.push(scope.clone());
                    }
                }
            }
            stack.into()
        };
        if let Some(last) = tokens
            .last_mut()
            .filter(|last| last.range.end() == ByteOffset(start as u64) && last.scopes == scopes)
        {
            last.range = Span::new(last.range.start(), ByteOffset(end as u64)).unwrap();
        } else {
            tokens.push(token(start, end, scopes));
        }
    }
}

fn token(start: usize, end: usize, scopes: Arc<[Arc<str>]>) -> Token {
    Token {
        range: Span::new(ByteOffset(start as u64), ByteOffset(end as u64)).unwrap(),
        scopes,
    }
}
