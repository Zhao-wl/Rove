//! 语法折叠区间（#118）：把折叠查询命中的节点换算成 [`Fold`]，并在增量编辑时复用上一版区间。
//!
//! 约定（与 #12 的标记 / 缩进折叠、#20 的界面侧一致）：区间首行折叠后仍可见，末行（含）被隐藏，
//! 所以 `Fold::lines` 是 `首行 .. 末行 + 1`，至少隐藏一行。
//!
//! 增量：重扫范围 `scan`（行）来自 #94 的 [`super::rows::Incremental`]，已经覆盖编辑行与新旧树的
//! changed ranges。折叠查询按 `scan` 的整行字节范围执行，tree-sitter 会返回**与该范围相交**的全部
//! 匹配（包含把编辑整个括起来的类 / 命名空间），所以只有与 `scan` 完全无关的旧区间才复用：
//! 编辑之前的原样保留，编辑之后的整体平移。两类都不会与新算出的区间重复。

use super::{grammar::Work, rows::RowRanges, source::Source};
use crate::{Fold, FoldKind};
use rove_base::{Line, LineRange, Result};
use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};
use tree_sitter::{Language, Tree};

/// 行区间：首行与不含的末行，和 [`Fold::lines`] 同义。
pub(super) type Rows = (u64, u64);

static CSHARP_FOLD_KINDS: LazyLock<Option<Vec<bool>>> =
    LazyLock::new(|| single_node_fold_kinds(include_str!("queries/csharp/folds.scm")));

/// The C# query lists one named node per line. Only dispatch directly when the
/// entire source is in that simple form. More complex queries use tree-sitter.
fn single_node_fold_kinds(query: &str) -> Option<Vec<bool>> {
    let mut lines = query
        .lines()
        .map(|line| line.split(';').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty());
    if lines.next()? != "[" {
        return None;
    }
    let language: Language = tree_sitter_c_sharp::LANGUAGE.into();
    let mut kinds = vec![false; language.node_kind_count()];
    loop {
        let pattern = lines.next()?;
        if pattern == "] @fold" {
            return (lines.next().is_none() && kinds.contains(&true)).then_some(kinds);
        }
        let name = pattern.strip_prefix('(')?.strip_suffix(')')?;
        let id = language.id_for_node_kind(name, true);
        if id == 0 {
            return None;
        }
        kinds[id as usize] = true;
    }
}

pub(super) fn csharp_nodes(
    tree: &Tree,
    scan: &Range<usize>,
    output: &mut Vec<Range<usize>>,
    work: &Work<'_>,
) -> Result<bool> {
    let root = tree.root_node();
    if scan.is_empty()
        || scan.start > root.start_byte()
        || scan.end < root.end_byte()
        || root.has_error()
    {
        return Ok(false);
    }
    let Some(kinds) = &*CSHARP_FOLD_KINDS else {
        return Ok(false);
    };

    work.check()?;
    let mut cursor = root.walk();
    let mut steps = 0u32;
    loop {
        steps = steps.wrapping_add(1);
        if steps.is_multiple_of(512) {
            work.check()?;
        }
        let node = cursor.node();
        if kinds.get(node.kind_id() as usize).copied().unwrap_or(false) {
            output.push(node.byte_range());
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                work.check()?;
                return Ok(true);
            }
        }
    }
}

/// `scan` 这些行的完整字节范围（含行尾），空范围返回 `0..0`。
pub(super) fn row_bytes(ranges: &RowRanges, rows: &Range<usize>, len: usize) -> Range<usize> {
    if rows.is_empty() || ranges.len() == 0 {
        return 0..0;
    }
    let start = ranges.get(rows.start).map_or(0, |row| row.start);
    let end = ranges.get(rows.end).map_or(len, |row| row.start);
    start..end.max(start)
}

/// 节点字节范围 → 行区间。跨行不足两行的节点、区间末尾的空白行都被丢弃
/// （Markdown 的 `section` 会一直延伸到下一个标题之前的空行）。
pub(super) fn rows_of(nodes: Vec<Range<usize>>, source: &Source, ranges: &RowRanges) -> Vec<Rows> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        if node.is_empty() {
            continue;
        }
        let start = ranges.row_at(node.start);
        let mut last = ranges.row_at(node.end - 1);
        while last > start && ranges.get(last).is_ok_and(|row| blank(source, row)) {
            last -= 1;
        }
        if last > start {
            out.push((start as u64, last as u64 + 1));
        }
    }
    out
}

fn blank(source: &Source, row: &Range<usize>) -> bool {
    source
        .bytes(row.clone())
        .iter()
        .all(u8::is_ascii_whitespace)
}

/// 复用上一版快照里与 `scan` 无关的区间：`scan` 之前的行号不变，之后的整体平移 `shift` 行。
pub(super) fn reusable(old: &[Fold], scan: &Range<usize>, shift: isize, out: &mut Vec<Rows>) {
    let (first, after) = (scan.start as u64, scan.end as u64);
    let shift = shift as i64;
    for fold in old {
        let (start, end) = (fold.lines.start().0, fold.lines.end().0);
        if end <= first {
            out.push((start, end));
            continue;
        }
        let moved = start
            .checked_add_signed(shift)
            .zip(end.checked_add_signed(shift));
        if let Some((start, end)) = moved.filter(|&(start, _)| start >= after) {
            out.push((start, end));
        }
    }
}

/// 排序（首行升序、末行降序，即外层在前）、去重并转成契约里的 [`Fold`]。
pub(super) fn finish(mut rows: Vec<Rows>) -> Arc<[Fold]> {
    rows.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    rows.dedup();
    rows.into_iter()
        .filter_map(|(start, end)| {
            Some(Fold {
                lines: LineRange::new(Line(start), Line(end)).ok()?,
                kind: FoldKind::Syntax,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_folding_declines_complex_queries() {
        assert!(CSHARP_FOLD_KINDS.is_some());
        assert!(single_node_fold_kinds("[\n(class_declaration (block))\n] @fold").is_none());
        assert!(single_node_fold_kinds("[\n(block)\n] @fold\n(comment) @fold").is_none());
    }

    fn fold(start: u64, end: u64) -> Fold {
        Fold {
            lines: LineRange::new(Line(start), Line(end)).unwrap(),
            kind: FoldKind::Syntax,
        }
    }

    fn pairs(folds: &[Fold]) -> Vec<Rows> {
        folds
            .iter()
            .map(|f| (f.lines.start().0, f.lines.end().0))
            .collect()
    }

    #[test]
    fn outer_ranges_come_first_and_duplicates_collapse() {
        let folds = finish(vec![(3, 5), (0, 9), (3, 7), (0, 9)]);
        assert_eq!(pairs(&folds), [(0, 9), (3, 7), (3, 5)]);
    }

    #[test]
    fn reuse_keeps_ranges_before_the_scan_and_shifts_those_after() {
        let old = [fold(0, 40), fold(2, 6), fold(10, 14), fold(20, 24)];
        let mut out = Vec::new();
        // 第 12 行附近重扫，编辑多出一行。
        reusable(&old, &(11..13), 1, &mut out);
        assert_eq!(
            out,
            [(2, 6), (21, 25)],
            "跨越重扫范围的区间交给新一次查询，不复用"
        );
    }
}
