//! Parallel C# parsing at complete member boundaries of one container.
//!
//! The container is the innermost single declaration body that encloses the whole file
//! (for example `namespace N { class C { ... } }`), or the compilation unit itself when the
//! file has several top-level declarations. Each part is parsed with the real header and
//! footer as included ranges, so its members keep their offsets and grammar context.
//! A lexer pass recognises comments, every string form except raw strings, verbatim
//! identifiers and conditional directives, and only cuts where none of them is open.
//! Parts whose trees do not validate as error-free, independent members make the caller
//! fall back to the whole-file parser.

use super::{
    grammar::{Capture, Grammar, Work},
    source::Source,
};
use rove_base::{Error, Result};
use std::{
    any::Any,
    ops::Range,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, LazyLock,
    },
    thread,
};
use tree_sitter::{InputEdit, Point, Range as TreeRange, Tree};

/// Smaller documents parse quickly enough on one thread.
pub(super) const MIN_BYTES: usize = 256 * 1024;
const MAX_THREADS: usize = 16;
/// Parts per worker: more, smaller parts balance uneven members across threads.
const PARTS_PER_WORKER: usize = 2;
const MIN_PART: usize = 16 * 1024;

/// Threads that partitioned parsing may use across all documents of one pool.
/// Each analysis takes what is free (at least two) and returns it when done, so
/// concurrent documents share the machine instead of serialising on a global flag.
pub struct WorkerPool {
    free: AtomicUsize,
}

impl WorkerPool {
    pub fn new(threads: usize) -> Arc<Self> {
        Arc::new(Self {
            free: AtomicUsize::new(threads),
        })
    }

    /// Process-wide pool sized to the hardware threads (at most 16).
    pub fn shared() -> Arc<Self> {
        static SHARED: LazyLock<Arc<WorkerPool>> = LazyLock::new(|| {
            WorkerPool::new(
                thread::available_parallelism()
                    .map_or(1, |n| n.get())
                    .min(MAX_THREADS),
            )
        });
        SHARED.clone()
    }

    pub(super) fn acquire(&self, want: usize) -> Option<Permit<'_>> {
        let mut free = self.free.load(Ordering::Acquire);
        loop {
            let take = free.min(want);
            if take < 2 {
                return None;
            }
            match self.free.compare_exchange_weak(
                free,
                free - take,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(Permit {
                        pool: self,
                        workers: take,
                    })
                }
                Err(current) => free = current,
            }
        }
    }
}

pub(super) struct Permit<'a> {
    pool: &'a WorkerPool,
    pub(super) workers: usize,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.pool.free.fetch_add(self.workers, Ordering::AcqRel);
    }
}

/// Independent member trees of one document.
pub(super) struct Parts {
    header_end: usize,
    footer_start: usize,
    container: bool,
    parts: Vec<Part>,
}

pub(super) struct Part {
    span: Range<usize>,
    /// Header, member range and footer; empty header or footer ranges are omitted.
    included: Vec<TreeRange>,
    pub tree: Tree,
}

impl Part {
    pub fn contains(&self, offset: usize) -> bool {
        self.span.contains(&offset)
    }
}

impl Parts {
    pub fn at(&self, offset: usize) -> Option<&Tree> {
        self.parts
            .iter()
            .find(|part| part.contains(offset))
            .or(self.parts.last().filter(|part| part.span.end == offset))
            .map(|part| &part.tree)
    }

    /// Reparse the single part that contains an edit of member text, reusing its old tree.
    /// Returns the new parts and the old and new tree of the edited part, or `None` when the
    /// edit touches the header, footer or a part boundary, or the result does not validate.
    pub fn reparse(
        &self,
        source: &Source,
        patch: &InputEdit,
        work: &Work<'_>,
    ) -> Result<Option<(Parts, Tree, usize)>> {
        let Some(index) = self.parts.iter().position(|part| {
            let member = member_range(part, self.header_end, self.footer_start);
            member.start < patch.start_byte && patch.old_end_byte < member.end
        }) else {
            return Ok(None);
        };
        let header_end = self.header_end;
        let footer_start = shift(self.footer_start, patch);
        let mut parts = Vec::with_capacity(self.parts.len());
        let mut edited = None;
        for (i, part) in self.parts.iter().enumerate() {
            let mut tree = part.tree.clone();
            tree.edit(patch);
            let included: Vec<_> = part
                .included
                .iter()
                .map(|range| edit_range(*range, patch))
                .collect();
            let span = if i < index {
                part.span.clone()
            } else if i == index {
                part.span.start..shift(part.span.end, patch)
            } else {
                shift(part.span.start, patch)..shift(part.span.end, patch)
            };
            if i == index {
                let new = Grammar::Csharp.parse_ranges(source, &included, Some(&tree), work)?;
                if !is_independent(&new, header_end, footer_start, self.container) {
                    return Ok(None);
                }
                edited = Some(tree);
                tree = new;
            }
            parts.push(Part {
                span,
                included,
                tree,
            });
        }
        Ok(edited.map(|old| {
            (
                Parts {
                    header_end,
                    footer_start,
                    container: self.container,
                    parts,
                },
                old,
                index,
            )
        }))
    }

    pub fn tree(&self, index: usize) -> &Tree {
        &self.parts[index].tree
    }
}

fn member_range(part: &Part, header_end: usize, footer_start: usize) -> Range<usize> {
    part.span.start.max(header_end)..part.span.end.min(footer_start)
}

fn shift(offset: usize, patch: &InputEdit) -> usize {
    if offset >= patch.old_end_byte {
        offset - patch.old_end_byte + patch.new_end_byte
    } else {
        offset
    }
}

fn shift_point(point: Point, patch: &InputEdit) -> Point {
    let old = patch.old_end_position;
    let new = patch.new_end_position;
    if point.row == old.row {
        Point::new(new.row, point.column - old.column + new.column)
    } else {
        Point::new(point.row - old.row + new.row, point.column)
    }
}

/// Move a range that lies entirely before, around or after the edit.
fn edit_range(mut range: TreeRange, patch: &InputEdit) -> TreeRange {
    if range.start_byte >= patch.old_end_byte {
        range.start_point = shift_point(range.start_point, patch);
        range.start_byte = shift(range.start_byte, patch);
    }
    if range.end_byte >= patch.old_end_byte {
        range.end_point = shift_point(range.end_point, patch);
        range.end_byte = shift(range.end_byte, patch);
    }
    range
}

/// Parts with the captures taken while their trees were hot, sorted for assembly, and
/// the byte range those captures cover.
pub(super) type Parsed = (Parts, Vec<Capture>, Range<usize>);

/// Parse and capture each part on the worker that parsed it, while its tree is hot.
/// `wanted` limits the capture step to the parts that reach into it (visible-first);
/// `None` captures the whole document.
pub(super) fn try_parse(
    source: &Source,
    pool: Option<&WorkerPool>,
    wanted: Option<Range<usize>>,
    work: &Work<'_>,
) -> Result<Option<Parsed>> {
    if source.len() < MIN_BYTES {
        return Ok(None);
    }
    let Some(pool) = pool else {
        return Ok(None);
    };
    let want = source.len().div_ceil(MIN_PART * PARTS_PER_WORKER);
    let Some(permit) = pool.acquire(want) else {
        return Ok(None);
    };
    parse_parts(source, permit.workers, wanted, work)
}

fn parse_parts(
    source: &Source,
    workers: usize,
    wanted: Option<Range<usize>>,
    work: &Work<'_>,
) -> Result<Option<Parsed>> {
    let text = source.contiguous();
    let Some(layout) = scan(text.as_bytes(), work)? else {
        return Ok(None);
    };
    let Some(plan) = plan(text.as_bytes(), &layout, workers * PARTS_PER_WORKER) else {
        return Ok(None);
    };
    let container = layout.container;
    let interesting = |span: &Range<usize>| match &wanted {
        Some(wanted) => span.start < wanted.end && wanted.start < span.end,
        None => true,
    };
    let parsed = parallel_map(&plan.chunks, workers, |chunk| {
        let tree = Grammar::Csharp.parse_ranges(source, &chunk.included, None, work)?;
        if !is_independent(&tree, plan.header_end, plan.footer_start, container) {
            return Ok(None);
        }
        let mut captures = Vec::new();
        if interesting(&chunk.span) {
            Grammar::Csharp.capture(&tree, source, chunk.span.clone(), &mut captures, work)?;
            captures.retain(|capture| chunk.span.contains(&capture.start));
            sort_captures(&mut captures);
        }
        Ok(Some((tree, captures)))
    })?;
    let mut parts = Vec::with_capacity(parsed.len());
    let mut all = Vec::with_capacity(parsed.iter().flatten().map(|(_, c)| c.len()).sum());
    let mut covered: Option<Range<usize>> = None;
    for (chunk, parsed) in plan.chunks.into_iter().zip(parsed) {
        let Some((tree, captures)) = parsed else {
            return Ok(None);
        };
        // Parts are disjoint and ordered, so the concatenation stays sorted, and the
        // captured parts that reach into `wanted` form one contiguous range.
        if interesting(&chunk.span) {
            all.extend(captures);
            covered = Some(match covered {
                Some(range) => range.start..chunk.span.end,
                None => chunk.span.clone(),
            });
        }
        parts.push(Part {
            span: chunk.span,
            included: chunk.included,
            tree,
        });
    }
    Ok(Some((
        Parts {
            header_end: plan.header_end,
            footer_start: plan.footer_start,
            container,
            parts,
        },
        all,
        covered.unwrap_or(0..0),
    )))
}

/// Assembly order: by start, longer first, then query priority.
pub(super) fn sort_captures(captures: &mut [Capture]) {
    captures.sort_unstable_by_key(|c| (c.start, std::cmp::Reverse(c.end), c.priority));
}

pub(super) fn capture(
    parts: &Parts,
    pool: Option<&WorkerPool>,
    source: &Source,
    scan: Range<usize>,
    output: &mut Vec<Capture>,
    work: &Work<'_>,
) -> Result<()> {
    let relevant: Vec<_> = parts
        .parts
        .iter()
        .filter(|part| part.span.start < scan.end && scan.start < part.span.end)
        .collect();
    let read = |part: &&Part| {
        let start = scan.start.max(part.span.start);
        let end = scan.end.min(part.span.end);
        let mut captures = Vec::new();
        Grammar::Csharp.capture(&part.tree, source, start..end, &mut captures, work)?;
        // A pattern can return other captures in its match outside the cursor's
        // range. The shared header and footer belong to the first and last parts.
        captures.retain(|capture| part.contains(capture.start));
        sort_captures(&mut captures);
        Ok(captures)
    };
    let permit = (relevant.len() > 1)
        .then(|| pool.and_then(|pool| pool.acquire(relevant.len())))
        .flatten();
    let captures = match &permit {
        Some(permit) => parallel_map(&relevant, permit.workers, read)?,
        None => relevant.iter().map(read).collect::<Result<Vec<_>>>()?,
    };
    output.reserve(captures.iter().map(Vec::len).sum());
    output.extend(captures.into_iter().flatten());
    Ok(())
}

/// Text of a panic payload, for diagnostics.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".into())
}

/// Run `read` over `items` on up to `workers` threads (the caller's thread included).
/// Panics become `Error::NotReady` after their message is written to stderr, and every
/// thread is joined before an error is returned.
fn parallel_map<T: Sync, U: Send>(
    items: &[T],
    workers: usize,
    read: impl Fn(&T) -> Result<U> + Sync,
) -> Result<Vec<U>> {
    let next = AtomicUsize::new(0);
    let run = &|| {
        let mut done = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(index) else {
                return done;
            };
            let result = catch_unwind(AssertUnwindSafe(|| read(item))).unwrap_or_else(|payload| {
                eprintln!(
                    "rove-syntax: C# part worker panicked: {}",
                    panic_message(payload.as_ref())
                );
                Err(Error::NotReady)
            });
            let failed = result.is_err();
            done.push((index, result));
            if failed {
                // Stop handing out work; other threads finish their current item.
                next.store(items.len(), Ordering::Relaxed);
                return done;
            }
        }
    };
    let mut results: Vec<(usize, Result<U>)> = thread::scope(|scope| {
        let helpers: Vec<_> = (1..workers.min(items.len()))
            .filter_map(|_| thread::Builder::new().spawn_scoped(scope, run).ok())
            .collect();
        let mut all = run();
        for helper in helpers {
            // `run` catches panics, so a join error cannot carry an analysis panic.
            match helper.join() {
                Ok(done) => all.extend(done),
                Err(payload) => {
                    eprintln!(
                        "rove-syntax: C# part worker failed: {}",
                        panic_message(payload.as_ref())
                    );
                    all.push((usize::MAX, Err(Error::NotReady)));
                }
            }
        }
        all
    });
    if let Some(error) = results.iter().find_map(|(_, r)| r.as_ref().err()) {
        return Err(error.clone());
    }
    if results.len() != items.len() {
        return Err(Error::NotReady);
    }
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

/// Brace structure of a lexically simple file.
struct Layout {
    /// After the container's `{`, or 0 for the compilation unit.
    header_end: usize,
    /// At the container's `}`, or the length for the compilation unit.
    footer_start: usize,
    container: bool,
    /// Offsets just after a member's closing brace at container level.
    cuts: Vec<usize>,
}

struct Plan {
    header_end: usize,
    footer_start: usize,
    chunks: Vec<Chunk>,
}

struct Chunk {
    included: Vec<TreeRange>,
    span: Range<usize>,
}

fn plan(bytes: &[u8], layout: &Layout, parts: usize) -> Option<Plan> {
    let len = bytes.len();
    let target = len.div_ceil(parts.max(2)).max(MIN_PART);
    // Every part reparses the header and footer.
    if layout.header_end > target / 4 || len - layout.footer_start > target / 4 {
        return None;
    }
    let mut cuts = Vec::new();
    let mut last = layout.header_end;
    for &cut in &layout.cuts {
        if cut - last >= target && layout.footer_start - cut >= target / 2 {
            cuts.push(cut);
            last = cut;
        }
    }
    if cuts.is_empty() {
        return None;
    }
    let bounds: Vec<_> = std::iter::once(layout.header_end)
        .chain(cuts.iter().copied())
        .chain(std::iter::once(layout.footer_start))
        .collect();
    // One dominant member leaves little to parallelise.
    if bounds.windows(2).any(|w| w[1] - w[0] > len / 2) {
        return None;
    }
    Some(chunks(bytes, layout.header_end, layout.footer_start, &cuts))
}

/// Parts for the given cuts: each sees the header, its members and the footer.
fn chunks(bytes: &[u8], header_end: usize, footer_start: usize, cuts: &[usize]) -> Plan {
    let len = bytes.len();
    let mut offsets: Vec<usize> = vec![0, header_end];
    offsets.extend(cuts);
    offsets.extend([footer_start, len]);
    let mut previous = 0;
    let mut point = Point::default();
    let positions: Vec<_> = offsets
        .iter()
        .map(|&offset| {
            point = super::advance(point, &bytes[previous..offset]);
            previous = offset;
            (offset, point)
        })
        .collect();
    let range = |start: usize, end: usize| TreeRange {
        start_byte: positions[start].0,
        start_point: positions[start].1,
        end_byte: positions[end].0,
        end_point: positions[end].1,
    };
    let footer = positions.len() - 2;
    let chunks = (1..footer)
        .map(|index| Chunk {
            included: [
                range(0, 1),
                range(index, index + 1),
                range(footer, footer + 1),
            ]
            .into_iter()
            .filter(|r| r.start_byte < r.end_byte)
            .collect(),
            span: positions[if index == 1 { 0 } else { index }].0
                ..positions[if index + 1 == footer {
                    footer + 1
                } else {
                    index + 1
                }]
                .0,
        })
        .collect();
    Plan {
        header_end,
        footer_start,
        chunks,
    }
}

#[derive(Clone, Copy)]
struct Brace {
    at: usize,
    /// Depth outside this brace.
    depth: usize,
    open: bool,
    /// No parenthesis, bracket or conditional directive is open here.
    safe: bool,
}

/// Lex the file once, then pick the container and its member boundaries.
fn scan(bytes: &[u8], work: &Work<'_>) -> Result<Option<Layout>> {
    let Some(braces) = Lexer::new(bytes).run(work)? else {
        return Ok(None);
    };
    // Descend through containers that are the only brace pair at their depth.
    let mut level = 0;
    let mut container = None;
    loop {
        let mut opens = braces.iter().filter(|b| b.open && b.depth == level);
        let (Some(open), None) = (opens.next(), opens.next()) else {
            break;
        };
        let Some(close) = braces
            .iter()
            .find(|b| !b.open && b.depth == level && b.at > open.at)
        else {
            break;
        };
        // Only whitespace may follow the container's closing brace inside its parent.
        let rest = match container {
            Some((_, footer)) => &bytes[close.at + 1..footer],
            None => &bytes[close.at + 1..],
        };
        if !rest.iter().all(u8::is_ascii_whitespace) || !open.safe || !close.safe {
            break;
        }
        container = Some((open.at + 1, close.at));
        level += 1;
    }
    let (header_end, footer_start) = container.unwrap_or((0, bytes.len()));
    let cuts = braces
        .iter()
        .filter(|b| {
            !b.open
                && b.depth == level
                && b.safe
                && b.at > header_end
                && b.at < footer_start
                && ends_member(bytes, b.at + 1)
        })
        .map(|b| b.at + 1)
        .collect();
    Ok(Some(Layout {
        header_end,
        footer_start,
        container: container.is_some(),
        cuts,
    }))
}

/// A closing brace ends a member unless an initializer, accessor or `;` continues it.
fn ends_member(bytes: &[u8], mut at: usize) -> bool {
    loop {
        match bytes.get(at) {
            Some(b) if b.is_ascii_whitespace() => at += 1,
            Some(b'/') if bytes.get(at + 1) == Some(&b'/') => {
                while bytes.get(at).is_some_and(|&b| b != b'\n' && b != b'\r') {
                    at += 1;
                }
            }
            Some(b';' | b'=' | b'.' | b',' | b')' | b'?' | b':' | b'/') => return false,
            _ => return true,
        }
    }
}

struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
    line_start: bool,
}

impl<'a> Lexer<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            line_start: true,
        }
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.at + offset).copied()
    }

    fn run(mut self, work: &Work<'_>) -> Result<Option<Vec<Brace>>> {
        let mut braces = Vec::new();
        let mut depth = 0usize;
        let mut parentheses = 0usize;
        let mut brackets = 0usize;
        let mut conditions: Vec<usize> = Vec::new();
        let mut next_check = 0;
        while let Some(byte) = self.peek(0) {
            if self.at >= next_check {
                work.check()?;
                next_check = self.at + 16 * 1024;
            }
            let line_start = self.line_start;
            self.line_start = false;
            match byte {
                b'\n' | b'\r' => {
                    self.line_start = true;
                    self.at += 1;
                }
                b' ' | b'\t' | b'\x0b' | b'\x0c' => {
                    self.line_start = line_start;
                    self.at += 1;
                }
                b'/' if self.peek(1) == Some(b'/') => self.skip_line(),
                b'/' if self.peek(1) == Some(b'*') => {
                    let Some(end) = find(&self.bytes[self.at + 2..], b"*/") else {
                        return Ok(None);
                    };
                    self.at += end + 4;
                }
                b'#' if line_start => {
                    let name_start = self.at + 1;
                    self.skip_line();
                    let line = &self.bytes[name_start..self.at];
                    let name = line
                        .iter()
                        .position(|b| !b.is_ascii_whitespace())
                        .map_or(&line[..0], |s| &line[s..]);
                    let word = name
                        .iter()
                        .position(|b| !b.is_ascii_alphabetic())
                        .map_or(name, |end| &name[..end]);
                    match word {
                        b"if" => conditions.push(depth),
                        b"elif" | b"else" | b"endif" => {
                            if conditions.last() != Some(&depth) {
                                return Ok(None);
                            }
                            if word == b"endif" {
                                conditions.pop();
                            }
                        }
                        b"region" | b"endregion" | b"pragma" | b"nullable" | b"define"
                        | b"undef" | b"line" | b"warning" | b"error" => {}
                        _ => return Ok(None),
                    }
                }
                b'"' | b'\'' | b'$' | b'@' => {
                    if byte == b'@' && self.peek(1).is_some_and(is_identifier_start) {
                        self.at += 1;
                        continue;
                    }
                    match skip_literal(self.bytes, self.at) {
                        Some(end) => self.at = end,
                        None => return Ok(None),
                    }
                }
                b'#' | b'\\' => return Ok(None),
                b'(' => {
                    parentheses += 1;
                    self.at += 1;
                }
                b'[' => {
                    brackets += 1;
                    self.at += 1;
                }
                b')' | b']' => {
                    let counter = if byte == b')' {
                        &mut parentheses
                    } else {
                        &mut brackets
                    };
                    let Some(count) = counter.checked_sub(1) else {
                        return Ok(None);
                    };
                    *counter = count;
                    self.at += 1;
                }
                b'{' | b'}' => {
                    let open = byte == b'{';
                    if !open {
                        let Some(d) = depth.checked_sub(1) else {
                            return Ok(None);
                        };
                        depth = d;
                    }
                    braces.push(Brace {
                        at: self.at,
                        depth,
                        open,
                        safe: parentheses == 0 && brackets == 0 && conditions.is_empty(),
                    });
                    if open {
                        depth += 1;
                    }
                    self.at += 1;
                }
                // Line terminators the lexer does not track would break directive detection.
                0xc2 if self.peek(1) == Some(0x85) => return Ok(None),
                0xe2 if self.peek(1) == Some(0x80) && matches!(self.peek(2), Some(0xa8 | 0xa9)) => {
                    return Ok(None)
                }
                _ => {
                    // Identifiers, operators and spaces inside a line: skip the run.
                    self.at += 1;
                    while self.peek(0).is_some_and(|b| !SPECIAL[b as usize]) {
                        self.at += 1;
                    }
                }
            }
        }
        Ok(
            (depth == 0 && parentheses == 0 && brackets == 0 && conditions.is_empty())
                .then_some(braces),
        )
    }

    fn skip_line(&mut self) {
        while self.peek(0).is_some_and(|b| b != b'\n' && b != b'\r') {
            self.at += 1;
        }
    }
}

/// Bytes the lexer must look at; everything else is skipped in runs.
static SPECIAL: [bool; 256] = {
    let mut table = [false; 256];
    let bytes = b"\n\r\"'$@/#\\()[]{}\xc2\xe2";
    let mut i = 0;
    while i < bytes.len() {
        table[bytes[i] as usize] = true;
        i += 1;
    }
    table
};

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Skip a character, regular, verbatim or interpolated string starting at `start`.
/// Raw strings (`"""`, `$$"`) and anything unrecognised return `None`.
fn skip_literal(bytes: &[u8], start: usize) -> Option<usize> {
    let mut at = start;
    let mut verbatim = false;
    let mut interpolated = false;
    loop {
        match bytes.get(at)? {
            b'@' if !verbatim => verbatim = true,
            b'$' if !interpolated => interpolated = true,
            b'"' => break,
            b'\'' if at == start => return skip_quoted(bytes, at, b'\''),
            _ => return None,
        }
        at += 1;
    }
    if bytes.get(at + 1..at + 3) == Some(b"\"\"") {
        return None;
    }
    if !interpolated {
        return if verbatim {
            skip_verbatim(bytes, at)
        } else {
            skip_quoted(bytes, at, b'"')
        };
    }
    at += 1;
    loop {
        match *bytes.get(at)? {
            b'"' if verbatim && bytes.get(at + 1) == Some(&b'"') => at += 2,
            b'"' => return Some(at + 1),
            b'\\' if !verbatim => at += 2,
            b'\r' | b'\n' if !verbatim => return None,
            b'{' if bytes.get(at + 1) == Some(&b'{') => at += 2,
            b'}' if bytes.get(at + 1) == Some(&b'}') => at += 2,
            b'{' => at = skip_hole(bytes, at + 1)?,
            b'}' => return None,
            _ => at += 1,
        }
    }
}

fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> Option<usize> {
    let mut at = start + 1;
    loop {
        match *bytes.get(at)? {
            b'\r' | b'\n' => return None,
            b'\\' => at += 2,
            b if b == quote => return Some(at + 1),
            _ => at += 1,
        }
    }
}

fn skip_verbatim(bytes: &[u8], start: usize) -> Option<usize> {
    let mut at = start + 1;
    loop {
        match *bytes.get(at)? {
            b'"' if bytes.get(at + 1) == Some(&b'"') => at += 2,
            b'"' => return Some(at + 1),
            _ => at += 1,
        }
    }
}

/// Skip an interpolation hole up to and including its closing brace.
fn skip_hole(bytes: &[u8], mut at: usize) -> Option<usize> {
    let mut nesting = 0usize;
    loop {
        match *bytes.get(at)? {
            b'"' | b'\'' | b'$' | b'@' => {
                if bytes[at] == b'@' && bytes.get(at + 1).copied().is_some_and(is_identifier_start)
                {
                    at += 1;
                } else {
                    at = skip_literal(bytes, at)?;
                }
            }
            b'(' | b'[' | b'{' => {
                nesting += 1;
                at += 1;
            }
            b')' | b']' => {
                nesting = nesting.checked_sub(1)?;
                at += 1;
            }
            b'}' if nesting == 0 => return Some(at + 1),
            b'}' => {
                nesting -= 1;
                at += 1;
            }
            b':' if nesting == 0 => {
                // Format clause: everything up to the closing brace.
                let end = bytes[at..].iter().position(|&b| b == b'}' || b == b'"')?;
                if bytes[at + end] != b'}' {
                    return None;
                }
                return Some(at + end + 1);
            }
            b'/' if matches!(bytes.get(at + 1), Some(b'/' | b'*')) => return None,
            b'\r' | b'\n' => return None,
            _ => at += 1,
        }
    }
}

/// The part's members must parse as an error-free list of independent declarations
/// directly inside the container (or the compilation unit).
fn is_independent(tree: &Tree, header_end: usize, footer_start: usize, container: bool) -> bool {
    let root = tree.root_node();
    if root.kind() != "compilation_unit" || root.has_error() {
        return false;
    }
    let body = if container {
        let Some(brace) = root.descendant_for_byte_range(header_end - 1, header_end) else {
            return false;
        };
        match brace.parent() {
            Some(body)
                if brace.kind() == "{"
                    && body.kind() == "declaration_list"
                    && body.start_byte() + 1 == header_end
                    && body.end_byte() == footer_start + 1 =>
            {
                body
            }
            _ => return false,
        }
    } else {
        root
    };
    let mut cursor = body.walk();
    let all_members = body.named_children(&mut cursor).all(|node| {
        matches!(
            node.kind(),
            "method_declaration"
                | "constructor_declaration"
                | "destructor_declaration"
                | "operator_declaration"
                | "conversion_operator_declaration"
                | "class_declaration"
                | "struct_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "record_declaration"
                | "namespace_declaration"
                | "comment"
                | "preproc_region"
                | "preproc_endregion"
                | "preproc_pragma"
                | "preproc_nullable"
        ) || (!container
            && matches!(
                node.kind(),
                "using_directive" | "extern_alias_directive" | "file_scoped_namespace_declaration"
            ))
    });
    all_members
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::Cancellation;

    fn class_source(member: &str) -> String {
        let mut source = String::from("using System;\r\nnamespace Example;\r\nclass Sample {\r\n");
        for i in 0..32 {
            source.push_str(&member.replace("METHOD", &format!("方法{i}")));
        }
        source.push_str("}\r\n");
        source
    }

    fn work(cancel: &Cancellation) -> Work<'_> {
        Work {
            cancel,
            deadline: None,
        }
    }

    /// Split into small parts regardless of the production size thresholds.
    fn split(source: &str) -> Option<Parts> {
        let cancel = Cancellation::default();
        let work = work(&cancel);
        let source = Source::from_str(source);
        let text = source.contiguous();
        let layout = scan(text.as_bytes(), &work).unwrap()?;
        let cuts: Vec<_> = layout.cuts.iter().copied().step_by(4).collect();
        if cuts.is_empty() {
            return None;
        }
        let plan = chunks(
            text.as_bytes(),
            layout.header_end,
            layout.footer_start,
            &cuts,
        );
        let mut parts = Vec::new();
        for chunk in plan.chunks {
            let tree = Grammar::Csharp
                .parse_ranges(&source, &chunk.included, None, &work)
                .unwrap();
            if !is_independent(&tree, plan.header_end, plan.footer_start, layout.container) {
                return None;
            }
            parts.push(Part {
                span: chunk.span,
                included: chunk.included,
                tree,
            });
        }
        Some(Parts {
            header_end: plan.header_end,
            footer_start: plan.footer_start,
            container: layout.container,
            parts,
        })
    }

    type Normalized = (usize, usize, (usize, usize, usize), &'static str);

    fn normalized(captures: Vec<Capture>) -> Vec<Normalized> {
        let mut items: Vec<_> = captures
            .into_iter()
            .map(|c| (c.start, c.end, c.priority, &*c.scopes[0]))
            .collect();
        items.sort_unstable();
        items
    }

    fn assert_matches_serial(text: &str, parts: &Parts) {
        let cancel = Cancellation::default();
        let work = work(&cancel);
        let source = Source::from_str(text);
        let mut split = Vec::new();
        capture(parts, None, &source, 0..source.len(), &mut split, &work).unwrap();
        let full = Grammar::Csharp.parse(&source, None, &work).unwrap();
        assert!(!full.root_node().has_error());
        let mut serial = Vec::new();
        Grammar::Csharp
            .capture(&full, &source, 0..source.len(), &mut serial, &work)
            .unwrap();
        assert_eq!(normalized(split), normalized(serial));
    }

    #[test]
    fn panics_return_a_service_error_and_keep_the_message() {
        for failed in [0, 1] {
            let result = parallel_map(&[0, 1, 2], 3, |worker| {
                assert_ne!(*worker, failed, "deliberate worker failure");
                Ok(*worker)
            });
            assert_eq!(result, Err(Error::NotReady));
        }
        let payload = catch_unwind(|| panic!("detail {}", 7)).unwrap_err();
        assert_eq!(panic_message(payload.as_ref()), "detail 7");
        assert_eq!(
            parallel_map(&[1, 2, 3], 2, |n| Ok(n * 2)),
            Ok(vec![2, 4, 6])
        );
    }

    #[test]
    fn pools_share_threads_between_documents() {
        let pool = WorkerPool::new(5);
        let first = pool.acquire(4).unwrap();
        assert_eq!(first.workers, 4);
        assert!(pool.acquire(4).is_none(), "one thread left");
        drop(first);
        let a = pool.acquire(3).unwrap();
        let b = pool.acquire(3).unwrap();
        assert_eq!((a.workers, b.workers), (3, 2));
    }

    #[test]
    fn partitioned_captures_match_serial_tree_with_unicode_and_comments() {
        let source = class_source(concat!(
            "    // braces } { and quote \"\r\n",
            "    [Obsolete]\r\n",
            "    string METHOD(string text) {\r\n",
            "        /* outside scopes { */ if (text == \"甲}\\\"{\") { return \"乙\"; }\r\n",
            "        return text;\r\n",
            "    }\r\n",
        ));
        let parts = split(&source).expect("independent method bodies");
        assert!(parts.parts.len() >= 3);
        assert_matches_serial(&source, &parts);
    }

    #[test]
    fn unity_namespaces_with_directives_and_interpolation_are_partitioned() {
        let mut source = String::from("using UnityEngine;\nnamespace Game {\n");
        for i in 0..24 {
            source.push_str(&format!(
                concat!(
                    "    public sealed class Mover{i} : MonoBehaviour\n    {{\n",
                    "        #region Fields\n        [SerializeField] private float speed = 2.5f;\n",
                    "        public bool IsActive {{ get; private set; }}\n        #endregion\n",
                    "        #if UNITY_EDITOR\n        private void OnValidate() {{\n",
                    "            Debug.Log($\"Speed {{speed:F2}} on {{gameObject.name}} {{{{x}}}}\", this);\n",
                    "            var path = @\"C:\\{{dir}}\"\"\";\n            var @class = '}}';\n",
                    "        }}\n        #endif\n    }}\n"
                ),
                i = i
            ));
        }
        source.push_str("}\n");
        let parts = split(&source).expect("classes of one namespace");
        assert!(parts.container);
        assert!(parts.parts.len() >= 3);
        assert_matches_serial(&source, &parts);
    }

    #[test]
    fn top_level_declarations_partition_the_compilation_unit() {
        let mut source = String::from("using System;\nnamespace Example;\n");
        for i in 0..16 {
            source.push_str(&format!(
                "class C{i} {{ void M() {{ }} }}\nstruct S{i} {{ }}\n"
            ));
        }
        let parts = split(&source).expect("top-level types");
        assert!(!parts.container);
        assert_matches_serial(&source, &parts);
    }

    #[test]
    fn unsafe_boundaries_and_invalid_members_use_whole_parser() {
        for member in [
            "    #if X\n    int METHOD() { return 1; }\n    #endif\n",
            "    string METHOD() { return \"\"\"{\"\"\"; }\n",
            "    string METHOD() { return $$\"{{x}}\"; }\n",
            "    int METHOD() { return 1 }\n",
            "    int METHOD { get { return 1; } }\n",
            "    int METHOD() { return 1; }\n    #weird\n",
        ] {
            assert!(split(&class_source(member)).is_none(), "{member}");
        }
    }

    #[test]
    fn edits_reparse_only_the_containing_part() {
        let cancel = Cancellation::default();
        let work = work(&cancel);
        let text = class_source("    int METHOD() {\r\n        return 1;\r\n    }\r\n");
        let parts = split(&text).unwrap();
        let start = text.rfind("return 1").unwrap() + "return ".len();
        let source = Source::from_str(&text)
            .splice(start..start + 1, "(2 + 3)")
            .unwrap();
        let mut edited = text.clone();
        edited.replace_range(start..start + 1, "(2 + 3)");
        let start_position = crate::highlight::advance(Point::default(), &text.as_bytes()[..start]);
        let patch = InputEdit {
            start_byte: start,
            old_end_byte: start + 1,
            new_end_byte: start + 7,
            start_position,
            old_end_position: Point::new(start_position.row, start_position.column + 1),
            new_end_position: Point::new(start_position.row, start_position.column + 7),
        };
        let (updated, _, index) = parts.reparse(&source, &patch, &work).unwrap().unwrap();
        assert_eq!(index, updated.parts.len() - 1);
        assert_matches_serial(&edited, &updated);
        // A header edit cannot be confined to one part.
        let header = InputEdit {
            start_byte: 1,
            old_end_byte: 2,
            new_end_byte: 2,
            start_position: Point::new(0, 1),
            old_end_position: Point::new(0, 2),
            new_end_position: Point::new(0, 2),
        };
        assert!(parts.reparse(&source, &header, &work).unwrap().is_none());
    }
}
