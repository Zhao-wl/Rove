//! 语法折叠区间（#118）：把折叠查询命中的节点换算成 [`Fold`]，并在增量编辑时复用上一版区间。
//!
//! 约定（与 #12 的标记 / 缩进折叠、#20 的界面侧一致）：区间首行折叠后仍可见，末行（含）被隐藏，
//! 所以 `Fold::lines` 是 `首行 .. 末行 + 1`，至少隐藏一行。
//!
//! 增量：重扫范围 `scan`（行）来自 #94 的 [`super::rows::Incremental`]，已经覆盖编辑行与新旧树的
//! changed ranges。折叠查询按 `scan` 的整行字节范围执行，tree-sitter 会返回**与该范围相交**的全部
//! 匹配（包含把编辑整个括起来的类 / 命名空间），所以只有与 `scan` 完全无关的旧区间才复用：
//! 编辑之前的原样保留，编辑之后的整体平移。两类都不会与新算出的区间重复。

use super::{rows::RowRanges, source::Source};
use crate::{Fold, FoldKind};
use rove_base::{Line, LineRange};
use std::{ops::Range, sync::Arc};

/// 行区间：首行与不含的末行，和 [`Fold::lines`] 同义。
pub(super) type Rows = (u64, u64);

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
