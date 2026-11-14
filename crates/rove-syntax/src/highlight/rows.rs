use super::{
    grammar::{Capture, Work},
    source::Source,
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

/// Rows sampled against `TextSource::line_range` after a terminator scan.
const SAMPLES: usize = 64;

impl RowRanges {
    /// Line table of `text`. Splits `source` at LF, CRLF and lone CR (the `RopeText`
    /// convention) in one pass; when the line count or any sampled row disagrees with the
    /// text source, the table is read row by row from `line_range` instead.
    pub fn new(
        text: &dyn TextSource,
        source: &Source,
        count: usize,
        work: &Work<'_>,
    ) -> Result<Self> {
        if let Some(ranges) = Self::split(text, source, count)? {
            return Ok(ranges);
        }
        Self::read(text, count, work)
    }

    fn split(text: &dyn TextSource, source: &Source, count: usize) -> Result<Option<Self>> {
        let mut ranges = Vec::with_capacity(count);
        let mut line_start = 0;
        let mut offset = 0;
        let mut after_cr = false;
        for chunk in source.chunks(0..source.len()) {
            for at in memchr::memchr2_iter(b'\r', b'\n', chunk) {
                let at = offset + at;
                if chunk[at - offset] == b'\n' && after_cr && line_start == at {
                    // The LF of a CRLF: the line already ended at the CR.
                    line_start = at + 1;
                    after_cr = false;
                    continue;
                }
                after_cr = chunk[at - offset] == b'\r';
                ranges.push(line_start..at);
                line_start = at + 1;
            }
            offset += chunk.len();
        }
        ranges.push(line_start..source.len());
        if ranges.len() != count {
            return Ok(None);
        }
        let step = (count / SAMPLES).max(1);
        for row in (0..count).step_by(step).chain(std::iter::once(count - 1)) {
            let line = text.line_range(Line(row as u64))?;
            if line.start().0 as usize != ranges[row].start
                || line.end().0 as usize != ranges[row].end
            {
                return Ok(None);
            }
        }
        Ok(Some(Self(ranges.into())))
    }

    fn read(text: &dyn TextSource, count: usize, work: &Work<'_>) -> Result<Self> {
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

    /// Update the table for one edit: rows around the edit are read again from the new
    /// text, rows after it move by the byte delta. `None` when the result is inconsistent
    /// with the new text (the caller then rebuilds the whole table).
    pub fn edit(
        &self,
        text: &dyn TextSource,
        count: usize,
        patch: &InputEdit,
        work: &Work<'_>,
    ) -> Result<Option<Self>> {
        if self.0.is_empty() || count == 0 {
            return Ok(None);
        }
        let first = self.row_at(patch.start_byte).saturating_sub(1);
        let old_last = (self.row_at(patch.old_end_byte) + 1).min(self.0.len() - 1);
        let Some(last) = (old_last + count)
            .checked_sub(self.0.len())
            .filter(|&last| last >= first && last < count)
        else {
            return Ok(None);
        };
        let delta = patch.new_end_byte as isize - patch.old_end_byte as isize;
        let mut ranges = Vec::with_capacity(count);
        ranges.extend_from_slice(&self.0[..first]);
        for row in first..=last {
            if (row - first).is_multiple_of(256) {
                work.check()?;
            }
            let line = text.line_range(Line(row as u64))?;
            ranges.push(line.start().0 as usize..line.end().0 as usize);
        }
        ranges.extend(
            self.0[old_last + 1..]
                .iter()
                .map(|r| r.start.wrapping_add_signed(delta)..r.end.wrapping_add_signed(delta)),
        );
        if ranges.len() != count {
            return Ok(None);
        }
        // The first moved row must agree with the new text.
        if let Some(moved) = ranges.get(last + 1) {
            let line = text.line_range(Line(last as u64 + 1))?;
            if line.start().0 as usize != moved.start || line.end().0 as usize != moved.end {
                return Ok(None);
            }
        }
        Ok(Some(Self(ranges.into())))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn get(&self, row: usize) -> Result<&Range<usize>> {
        self.0.get(row).ok_or(Error::OutOfBounds)
    }

    pub fn row_at(&self, offset: usize) -> usize {
        self.0
            .partition_point(|r| r.start <= offset)
            .saturating_sub(1)
    }

    pub fn bytes(&self, scan: Range<usize>) -> Range<usize> {
        if scan.is_empty() {
            return 0..0;
        }
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

    /// 旧行号 → 新行号的整体位移（编辑之后的行）。
    pub fn shift(&self) -> isize {
        self.shift
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

pub(super) static EMPTY_LINE: LazyLock<Arc<[Token]>> = LazyLock::new(|| Arc::from([]));

pub(super) type Rows = (Vec<LineTokens>, Option<Arc<[LineRange]>>);

#[allow(clippy::too_many_arguments)]
pub(super) fn assemble(
    ranges: &RowRanges,
    source: &Source,
    scan: Range<usize>,
    mut captures: Vec<Capture>,
    baseline: Option<Incremental<'_>>,
    limit: usize,
    workers: usize,
    work: &Work<'_>,
) -> Result<Rows> {
    if !captures.is_sorted_by_key(|c| (c.start, Reverse(c.end), c.priority)) {
        super::csharp_parts::sort_captures(&mut captures);
    }
    if baseline.is_none()
        && scan == (0..ranges.len())
        && workers > 1
        && ranges.len() >= PARALLEL_ROWS
    {
        return Ok((
            assemble_parallel(ranges, source, &captures, limit, workers, work)?,
            None,
        ));
    }

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
            // Without a baseline only a partial (visible-first) build skips rows.
            lines.push(match (previous, &baseline) {
                (Some(previous), _) => previous.clone(),
                (None, None) => LineTokens {
                    origin: range.start,
                    tokens: EMPTY_LINE.clone(),
                },
                (None, Some(_)) => return Err(Error::InvalidInput),
            });
            continue;
        }
        active.retain(|c| c.end > range.start);
        while pending.peek().is_some_and(|c| c.start < range.end) {
            let capture = pending.next().unwrap();
            if capture.end > range.start {
                active.push(capture);
            }
        }
        let tokens = row_tokens(range, &active, source, limit);
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

/// Tokens of one row from the captures that overlap it; empty past the line limit.
fn row_tokens(
    range: &Range<usize>,
    active: &[&Capture],
    source: &Source,
    limit: usize,
) -> Arc<[Token]> {
    if !active.is_empty()
        && !range.is_empty()
        && (range.len() <= limit
            || (range.len() <= limit.saturating_mul(3)
                && source
                    .text(range.clone())
                    .encode_utf16()
                    .take(limit + 1)
                    .count()
                    <= limit))
    {
        project_line(range, active)
    } else {
        EMPTY_LINE.clone()
    }
}

/// Rows per thread below which a full build assembles on one thread.
const PARALLEL_ROWS: usize = 4096;

/// Full build: split the rows into blocks, one per thread. A block starts with the
/// captures that began earlier and still cover its first row, found through the running
/// maximum of capture ends, so the result equals the sequential pass.
fn assemble_parallel(
    ranges: &RowRanges,
    source: &Source,
    captures: &[Capture],
    limit: usize,
    workers: usize,
    work: &Work<'_>,
) -> Result<Vec<LineTokens>> {
    let rows = &ranges.0;
    let mut reach = Vec::with_capacity(captures.len());
    let mut furthest = 0;
    for capture in captures {
        furthest = furthest.max(capture.end);
        reach.push(furthest);
    }
    let blocks = workers.min(rows.len() / (PARALLEL_ROWS / 4)).max(1);
    let size = rows.len().div_ceil(blocks);
    let block = |first: usize| -> Result<Vec<LineTokens>> {
        let last = (first + size).min(rows.len());
        let origin = rows[first].start;
        let next = captures.partition_point(|c| c.start < origin);
        let mut active = Vec::new();
        let mut index = next;
        while index > 0 && reach[index - 1] > origin {
            index -= 1;
            if captures[index].end > origin {
                active.push(&captures[index]);
            }
        }
        active.reverse();
        let mut pending = captures[next..].iter().peekable();
        let mut lines = Vec::with_capacity(last - first);
        for (offset, range) in rows[first..last].iter().enumerate() {
            if offset % 256 == 0 {
                work.check()?;
            }
            active.retain(|c| c.end > range.start);
            while pending.peek().is_some_and(|c| c.start < range.end) {
                let capture = pending.next().unwrap();
                if capture.end > range.start {
                    active.push(capture);
                }
            }
            lines.push(LineTokens {
                origin: range.start,
                tokens: row_tokens(range, &active, source, limit),
            });
        }
        Ok(lines)
    };
    let starts: Vec<_> = (0..rows.len()).step_by(size).collect();
    let parts: Vec<Result<Vec<LineTokens>>> = std::thread::scope(|scope| {
        let block = &block;
        let helpers: Vec<_> = starts[1..]
            .iter()
            .map(|&first| scope.spawn(move || block(first)))
            .collect();
        std::iter::once(block(starts[0]))
            .chain(
                helpers
                    .into_iter()
                    .map(|helper| helper.join().unwrap_or(Err(Error::NotReady))),
            )
            .collect()
    });
    let mut lines = Vec::with_capacity(rows.len());
    for part in parts {
        lines.extend(part?);
    }
    Ok(lines)
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
