//! Parallel parsing at complete member boundaries in a single C# class.
//!
//! Each parser sees the real class header and footer, so its members retain their
//! original offsets and grammar context. Unsupported lexical forms and trees
//! that do not validate as independent method declarations use the whole parser.

use super::{
    advance,
    grammar::{Capture, Grammar, Work},
};
use rove_base::Result;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
    thread,
};
use tree_sitter::{Point, Range as TreeRange, Tree};

const MIN_BYTES: usize = 512 * 1024;
const MAX_WORKERS: usize = 8;
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Bound additional workers even when several documents are opened at once.
struct Parallel {
    workers: usize,
}

impl Parallel {
    fn acquire(bytes: usize) -> Option<Self> {
        if bytes < MIN_BYTES {
            return None;
        }
        let workers = thread::available_parallelism().ok()?.get().min(MAX_WORKERS);
        if workers < 2 || ACTIVE.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(Self { workers })
    }
}

pub(super) fn try_parse(source: &str, work: &Work<'_>) -> Result<Option<Vec<Part>>> {
    let Some(parallel) = Parallel::acquire(source.len()) else {
        return Ok(None);
    };
    parse_parts(source, parallel.workers, work)
}

fn parse_parts(source: &str, workers: usize, work: &Work<'_>) -> Result<Option<Vec<Part>>> {
    let Some(plan) = plan(source, workers, work)? else {
        return Ok(None);
    };
    let parts = parallel_map(&plan.chunks, |chunk| {
        let tree = Grammar::Csharp.parse_ranges(source.as_bytes(), &chunk.included, work)?;
        Ok(
            is_independent_class(&tree, plan.header_end, plan.footer_start).then(|| Part {
                span: chunk.span.clone(),
                tree,
            }),
        )
    })?;
    Ok(parts.into_iter().collect())
}

impl Drop for Parallel {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
    }
}

pub(super) struct Part {
    span: Range<usize>,
    pub tree: Tree,
}

impl Part {
    pub fn contains(&self, offset: usize) -> bool {
        self.span.contains(&offset)
    }
}

pub(super) fn capture(
    parts: &[Part],
    source: &[u8],
    scan: Range<usize>,
    output: &mut Vec<Capture>,
    work: &Work<'_>,
) -> Result<()> {
    let relevant: Vec<_> = parts
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
        Ok(captures)
    };
    let parallel = (relevant.len() > 1)
        .then(|| Parallel::acquire(source.len()))
        .flatten();
    let captures = if parallel.is_some() {
        parallel_map(&relevant, read)?
    } else {
        relevant.iter().map(read).collect::<Result<Vec<_>>>()?
    };
    output.extend(captures.into_iter().flatten());
    Ok(())
}

fn parallel_map<T: Sync, U: Send>(
    items: &[T],
    read: impl Fn(&T) -> Result<U> + Sync,
) -> Result<Vec<U>> {
    let Some((first, rest)) = items.split_first() else {
        return Ok(Vec::new());
    };
    thread::scope(|scope| {
        let read = &read;
        let tasks: Vec<_> = rest
            .iter()
            .map(
                |item| match thread::Builder::new().spawn_scoped(scope, move || read(item)) {
                    Ok(task) => Ok(task),
                    Err(_) => Err(read(item)),
                },
            )
            .collect();
        std::iter::once(read(first))
            .chain(tasks.into_iter().map(|task| match task {
                Ok(task) => task.join().expect("syntax worker panicked"),
                Err(result) => result,
            }))
            .collect()
    })
}

struct Plan {
    header_end: usize,
    footer_start: usize,
    chunks: Vec<Chunk>,
}

struct Chunk {
    included: [TreeRange; 3],
    span: Range<usize>,
}

fn plan(source: &str, workers: usize, work: &Work<'_>) -> Result<Option<Plan>> {
    let Some((header_end, footer_start, cuts)) = boundaries(source, workers, work)? else {
        return Ok(None);
    };
    // Avoid duplicating a large preamble or starting workers for one dominant member.
    let target = source.len().div_ceil(workers);
    if header_end > target / 4
        || source.len() - footer_start > target / 4
        || std::iter::once(header_end)
            .chain(cuts.iter().copied())
            .zip(cuts.iter().copied().chain(std::iter::once(footer_start)))
            .any(|(start, end)| end - start > target * 2)
    {
        return Ok(None);
    }
    let offsets = std::iter::once(0)
        .chain(std::iter::once(header_end))
        .chain(cuts)
        .chain([footer_start, source.len()]);
    let mut previous = 0;
    let mut point = Point::default();
    let positions: Vec<_> = offsets
        .map(|offset| {
            point = advance(point, &source.as_bytes()[previous..offset]);
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
    let header = range(0, 1);
    let closing = range(footer, footer + 1);
    let chunks = (1..footer)
        .map(|index| Chunk {
            included: [header, range(index, index + 1), closing],
            span: positions[if index == 1 { 0 } else { index }].0
                ..positions[if index + 1 == footer {
                    footer + 1
                } else {
                    index + 1
                }]
                .0,
        })
        .collect();
    Ok(Some(Plan {
        header_end,
        footer_start,
        chunks,
    }))
}

type Boundaries = (usize, usize, Vec<usize>);

/// Recognize only the lexical forms for which a brace is an unambiguous boundary.
/// Directives and complex strings can carry lexer context across a cut.
fn boundaries(source: &str, workers: usize, work: &Work<'_>) -> Result<Option<Boundaries>> {
    if source.contains(['\u{85}', '\u{2028}', '\u{2029}']) {
        return Ok(None);
    }
    let bytes = source.as_bytes();
    let target = bytes.len().div_ceil(workers);
    let mut cuts = Vec::with_capacity(workers.saturating_sub(1));
    let mut depth = 0usize;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut header = None;
    let mut next_cut = target;
    let mut next_check = 0;
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if index >= next_check {
            work.check()?;
            next_check = index + 4096;
        }
        match byte {
            b'#' | b'@' | b'$' | b'\\' => return Ok(None),
            b'"' | b'\'' => {
                let Some(end) = simple_literal(bytes, index, byte) else {
                    return Ok(None);
                };
                index = end;
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while bytes.get(index).is_some_and(|&b| b != b'\r' && b != b'\n') {
                    index += 1;
                }
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let Some(end) = bytes[index + 2..].windows(2).position(|pair| pair == b"*/") else {
                    return Ok(None);
                };
                index += end + 4;
                continue;
            }
            b'(' => parentheses += 1,
            b'[' => brackets += 1,
            b')' => match parentheses.checked_sub(1) {
                Some(count) => parentheses = count,
                None => return Ok(None),
            },
            b']' => match brackets.checked_sub(1) {
                Some(count) => brackets = count,
                None => return Ok(None),
            },
            b'{' => {
                if depth == 0 {
                    if !source[..index]
                        .split_ascii_whitespace()
                        .any(|word| word == "class")
                    {
                        return Ok(None);
                    }
                    header = Some(index + 1);
                }
                depth += 1;
            }
            b'}' => {
                let Some(remaining) = depth.checked_sub(1) else {
                    return Ok(None);
                };
                depth = remaining;
                if depth == 0 {
                    if parentheses != 0
                        || brackets != 0
                        || !bytes[index + 1..].iter().all(u8::is_ascii_whitespace)
                    {
                        return Ok(None);
                    }
                    if cuts.last().is_some_and(|cut| index - cut < target / 4) {
                        cuts.pop();
                    }
                    return Ok(header
                        .filter(|_| !cuts.is_empty())
                        .map(|start| (start, index, cuts)));
                }
                if depth == 1
                    && parentheses == 0
                    && brackets == 0
                    && index + 1 >= next_cut
                    && cuts.len() + 1 < workers
                {
                    cuts.push(index + 1);
                    next_cut = index + 1 + target;
                }
            }
            _ => {}
        }
        index += 1;
    }
    Ok(None)
}

fn simple_literal(bytes: &[u8], start: usize, quote: u8) -> Option<usize> {
    if quote == b'"' && bytes.get(start + 1..start + 3) == Some(b"\"\"") {
        return None;
    }
    let mut index = start + 1;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'\r' | b'\n' => return None,
            b'\\' => index += 2,
            b if b == quote => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

fn is_independent_class(tree: &Tree, header_end: usize, footer_start: usize) -> bool {
    let root = tree.root_node();
    if root.kind() != "compilation_unit" || root.has_error() {
        return false;
    }
    let mut cursor = root.walk();
    let mut class = None;
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "class_declaration" if class.is_none() => class = Some(child),
            "using_directive" | "extern_alias_directive" | "file_scoped_namespace_declaration"
                if class.is_none() => {}
            "comment" => {}
            _ => return false,
        }
    }
    let Some(body) = class.and_then(|node| node.child_by_field_name("body")) else {
        return false;
    };
    if body.start_byte() + 1 != header_end || body.end_byte() != footer_start + 1 {
        return false;
    }
    let mut cursor = body.walk();
    let mut members = body.named_children(&mut cursor);
    members.all(|node| {
        matches!(
            node.kind(),
            "method_declaration"
                | "constructor_declaration"
                | "destructor_declaration"
                | "operator_declaration"
                | "conversion_operator_declaration"
                | "comment"
        )
    })
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

    #[test]
    fn partitioned_captures_match_serial_tree_with_unicode_and_comments() {
        let cancel = Cancellation::default();
        let work = Work {
            cancel: &cancel,
            deadline: None,
        };
        let source = class_source(concat!(
            "    // braces } { and quote \"\r\n",
            "    [Obsolete]\r\n",
            "    string METHOD(string text) {\r\n",
            "        /* outside scopes { */ if (text == \"甲}\\\"{\") { return \"乙\"; }\r\n",
            "        return text;\r\n",
            "    }\r\n",
        ));
        let parts = parse_parts(&source, 4, &work)
            .unwrap()
            .expect("independent method bodies");
        assert!(parts.len() >= 3);
        let mut split = Vec::new();
        capture(
            &parts,
            source.as_bytes(),
            0..source.len(),
            &mut split,
            &work,
        )
        .unwrap();
        let full = Grammar::Csharp
            .parse(source.as_bytes(), None, &work)
            .unwrap();
        assert!(!full.root_node().has_error());
        let mut serial = Vec::new();
        Grammar::Csharp
            .capture(
                &full,
                source.as_bytes(),
                0..source.len(),
                &mut serial,
                &work,
            )
            .unwrap();
        fn normalized(captures: Vec<Capture>) -> Vec<(usize, usize, ArcScopes)> {
            let mut items: Vec<_> = captures
                .into_iter()
                .map(|c| (c.start, c.end, (c.priority, c.scopes)))
                .collect();
            items.sort_unstable();
            items
        }
        type ArcScopes = (
            (usize, usize, usize),
            &'static std::sync::Arc<[std::sync::Arc<str>]>,
        );
        assert_eq!(normalized(split), normalized(serial));
    }

    #[test]
    fn unsafe_boundaries_and_invalid_members_use_whole_parser() {
        let cancel = Cancellation::default();
        let work = Work {
            cancel: &cancel,
            deadline: None,
        };
        for member in [
            "    #if X\n    int METHOD() { return 1; }\n    #endif\n",
            "    string METHOD() { return @\"{\"; }\n",
            "    string METHOD() { return \"\"\"{\"\"\"; }\n",
            "    int METHOD() { return 1 }\n",
            "    int METHOD { get { return 1; } }\n",
        ] {
            assert!(
                parse_parts(&class_source(member), 4, &work)
                    .unwrap()
                    .is_none(),
                "{member}"
            );
        }
    }
}
