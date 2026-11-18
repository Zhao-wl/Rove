//! 行操作（#52）：把选区变成 #46 的一个多区间事务。
//!
//! 这里只算“文本怎么变、光标落在哪”，不碰 GPUI，也不自己改缓冲区：产出的
//! [`Transaction`] 交给 [`EditSession::edit`](rove_buffer::edit::EditSession::edit) 执行，
//! 所以**一次行操作只产生一次撤销**，多光标在多行上执行同一个行操作也只有一次撤销。
//!
//! 换行符：移动行 / 复制行 / 排序把**每行连同它自己的行终止符**一起搬动，事务用
//! [`Transaction::verbatim`]（不做主换行符归一），因此混合换行符的文件不会被统一；
//! 区间之外的字节一个都不动。只有“文件最后一行没有终止符”这一处需要补 / 去终止符
//! （补的时候用文档的主换行符）。
//!
//! 行为对照 VS Code `linesOperations.ts`、`sortLinesCommand.ts`、`caseOperations.ts`、
//! `copyLinesCommand.ts`、`moveLinesCommand.ts`、`trimTrailingWhitespaceCommand.ts`
//! （最后一个直接复用 #49 的保存时变换 [`rove_buffer::save::transforms`]，不写第二份）。

use rove_base::{ByteOffset, Error, Line, Position, Result, Span, Utf8Column};
use rove_buffer::edit::{Replacement, Transaction};
use rove_buffer::TextSource;
use unicode_segmentation::UnicodeSegmentation;

use super::columns::read_line;
use super::selection::{pos, Selection};

/// 一条行操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOp {
    MoveUp,
    MoveDown,
    CopyUp,
    CopyDown,
    Delete,
    /// `editor.action.duplicateSelection`：有选区复制选区，无选区复制整行。
    DuplicateSelection,
    JoinLines,
    SortAscending,
    SortDescending,
    Transpose,
    TransposeLetters,
    /// `editor.action.trimTrailingWhitespace`。
    TrimTrailingWhitespace,
    Case(CaseKind),
}

/// 大小写 / 命名风格转换（VS Code `caseOperations.ts`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseKind {
    Upper,
    Lower,
    Title,
    Camel,
    Pascal,
    Snake,
    Kebab,
}

/// 行操作的上下文（设置项）。
#[derive(Clone, Debug)]
pub struct LineOpContext {
    /// `editor.wordSeparators`：标题大小写的分隔符、空选区时取光标所在的词。
    pub word_separators: String,
    /// 文档的主换行符（只在需要补一个行终止符时使用）。
    pub eol: String,
    /// 当前已折叠的区间 `(首行, 末行的下一行)`（#20 `Folding::collapsed_keys`）。
    /// 移动行 / 复制行按**屏幕行**取块（#171）：折叠首行连同被它隐藏的行是一个整体。
    pub collapsed: Vec<(u64, u64)>,
}

/// 一次行操作的结果。
#[derive(Clone, Debug)]
pub struct LinePlan {
    pub transaction: Transaction,
    /// 事务之后每个选区的位置（顺序与传入的选区一致）。`None` 表示按事务自动映射。
    pub selections: Option<Vec<Selection>>,
    /// 行号的搬动：`(首行, 末行（含）, 平移量)`，没列出的行行号不变。移动行 / 复制行用它让
    /// 折叠状态跟着行走（#171）；其他操作为空。
    pub line_map: Vec<(u64, u64, i64)>,
}

impl LinePlan {
    /// 旧行号 → 新行号（按 [`Self::line_map`]）。
    pub fn map_line(&self, line: u64) -> u64 {
        map_line(&self.line_map, line)
    }
}

/// 按 [`LinePlan::line_map`] 把旧行号换成新行号；没列出的行不变。
pub fn map_line(map: &[(u64, u64, i64)], line: u64) -> u64 {
    map.iter()
        .find(|(lo, hi, _)| *lo <= line && line <= *hi)
        .map_or(line, |(_, _, d)| (line as i64 + d).max(0) as u64)
}

/// 按 `op` 计算事务；没有任何变化时返回 `Ok(None)`。
pub fn plan(
    op: LineOp,
    text: &dyn TextSource,
    selections: &[Selection],
    ctx: &LineOpContext,
) -> Result<Option<LinePlan>> {
    match op {
        LineOp::MoveUp => move_lines(text, selections, ctx, true),
        LineOp::MoveDown => move_lines(text, selections, ctx, false),
        LineOp::CopyUp => copy_lines(text, selections, ctx, true),
        LineOp::CopyDown => copy_lines(text, selections, ctx, false),
        LineOp::Delete => delete_lines(text, selections),
        LineOp::DuplicateSelection => duplicate_selection(text, selections, ctx),
        LineOp::JoinLines => join_lines(text, selections),
        LineOp::SortAscending => sort_lines(text, selections, ctx, false),
        LineOp::SortDescending => sort_lines(text, selections, ctx, true),
        LineOp::Transpose => transpose(text, selections),
        LineOp::TransposeLetters => transpose_letters(text, selections),
        LineOp::TrimTrailingWhitespace => Ok(
            rove_buffer::save::transforms::trim_trailing_whitespace(text, None, &[])?.map(|t| {
                LinePlan {
                    transaction: t,
                    selections: None,
                    line_map: Vec::new(),
                }
            }),
        ),
        LineOp::Case(kind) => case_transform(text, selections, ctx, kind),
    }
}

// ------------------------------------------------------------------ 行读写的小工具

fn line_count(text: &dyn TextSource) -> Result<u64> {
    let lines = text.lines();
    if !lines.complete {
        return Err(Error::NotReady);
    }
    Ok(lines.known.max(1))
}

fn line_str(text: &dyn TextSource, line: u64) -> String {
    read_line(text, line).unwrap_or_default()
}

fn line_start(text: &dyn TextSource, line: u64) -> Result<ByteOffset> {
    Ok(text.line_range(Line(line))?.start())
}

/// 一行的行终止符（末行为空串）。
fn terminator(text: &dyn TextSource, line: u64, count: u64) -> Result<String> {
    if line + 1 >= count {
        return Ok(String::new());
    }
    let end = text.line_range(Line(line))?.end();
    let next = text.line_range(Line(line + 1))?.start();
    let mut out = String::new();
    text.visit_chunks(Span::new(end, next)?, &mut |c| out.push_str(c))?;
    Ok(out)
}

/// 选区覆盖的行区间（含两端）。VS Code：选区止于下一行行首时不含那一行。
pub fn block_of(s: &Selection) -> (u64, u64) {
    let start = s.start().line.0;
    let mut end = s.end().line.0;
    if end > start && s.end().column.0 == 0 {
        end -= 1;
    }
    (start, end)
}

/// `line` 所在的屏幕行覆盖的文档行（含两端）：被折叠的区间算一整行（取最外层）。
fn screen_line(collapsed: &[(u64, u64)], line: u64) -> (u64, u64) {
    collapsed
        .iter()
        .filter(|(s, e)| *s <= line && line < *e)
        .fold((line, line), |(lo, hi), (s, e)| (lo.min(*s), hi.max(e - 1)))
}

/// 把若干选区的行区间合并成互不相交、按起点升序的若干块。两端按屏幕行扩展：
/// 落在折叠区间里的行带上整个区间（#171）。
fn blocks(selections: &[Selection], collapsed: &[(u64, u64)]) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = selections
        .iter()
        .map(|s| {
            let (lo, hi) = block_of(s);
            (screen_line(collapsed, lo).0, screen_line(collapsed, hi).1)
        })
        .collect();
    out.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(out.len());
    for (lo, hi) in out {
        match merged.last_mut() {
            // 相邻（hi + 1 == lo）的块也合并：移动 / 复制时它们要一起搬。
            Some(last) if lo <= last.1 + 1 => last.1 = last.1.max(hi),
            _ => merged.push((lo, hi)),
        }
    }
    merged
}

/// 一块行的 `(内容, 终止符)` 列表。
fn read_block(
    text: &dyn TextSource,
    lo: u64,
    hi: u64,
    count: u64,
) -> Result<Vec<(String, String)>> {
    (lo..=hi)
        .map(|l| Ok((line_str(text, l), terminator(text, l, count)?)))
        .collect()
}

/// 一块行连同终止符的字节区间（末行没有终止符时止于行尾）。
fn block_range(
    text: &dyn TextSource,
    lo: u64,
    hi: u64,
    count: u64,
) -> Result<rove_base::ByteRange> {
    let start = line_start(text, lo)?;
    let end = if hi + 1 < count {
        line_start(text, hi + 1)?
    } else {
        text.line_range(Line(hi))?.end()
    };
    Span::new(start, end)
}

/// 把 `(内容, 终止符)` 列表拼回文本：当这块行以文件末行结尾时，末项不要终止符，
/// 其余项缺终止符的补上主换行符。
fn join_block(items: &[(String, String)], ends_at_eof: bool, eol: &str) -> String {
    let mut out = String::new();
    for (i, (content, term)) in items.iter().enumerate() {
        out.push_str(content);
        let last = i + 1 == items.len();
        if ends_at_eof && last {
            continue;
        }
        if term.is_empty() {
            out.push_str(eol);
        } else {
            out.push_str(term);
        }
    }
    out
}

/// 选区整体上下平移 `delta` 行（行内列不变）。
fn shift_selection(s: &Selection, delta: i64) -> Selection {
    let shift = |p: Position| pos((p.line.0 as i64 + delta).max(0) as u64, p.column.0);
    Selection::new(shift(s.anchor), shift(s.active))
}

// ------------------------------------------------------------------ 移动行 / 复制行 / 删除行

fn move_lines(
    text: &dyn TextSource,
    selections: &[Selection],
    ctx: &LineOpContext,
    up: bool,
) -> Result<Option<LinePlan>> {
    let count = line_count(text)?;
    let mut edits = Vec::new();
    // (块首, 块末, 块的平移量)。
    let mut moved: Vec<(u64, u64, i64)> = Vec::new();
    let mut line_map = Vec::new();
    for (lo, hi) in blocks(selections, &ctx.collapsed) {
        if up && lo == 0 {
            continue;
        }
        if !up && hi + 1 >= count {
            continue;
        }
        // 被跨过的是相邻的**一个屏幕行**：折叠着的区域整体跨过（VS Code 按屏幕行移动）。
        let (rlo, rhi) = if up {
            (screen_line(&ctx.collapsed, lo - 1).0, hi)
        } else {
            (lo, screen_line(&ctx.collapsed, hi + 1).1)
        };
        let mut items = read_block(text, rlo, rhi, count)?;
        let block = (hi - lo + 1) as i64;
        let skipped = if up { lo - rlo } else { rhi - hi } as i64;
        if up {
            // 被跨过的那几行挪到块后面。
            items.rotate_left(skipped as usize);
            line_map.push((lo, hi, -skipped));
            line_map.push((rlo, lo - 1, block));
        } else {
            items.rotate_right(skipped as usize);
            line_map.push((lo, hi, skipped));
            line_map.push((hi + 1, rhi, -block));
        }
        let range = block_range(text, rlo, rhi, count)?;
        let ends_at_eof = rhi + 1 >= count;
        edits.push(Replacement::new(
            range,
            join_block(&items, ends_at_eof, &ctx.eol),
        ));
        moved.push((lo, hi, if up { -skipped } else { skipped }));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    let selections = selections
        .iter()
        .map(|s| {
            let (lo, hi) = block_of(s);
            match moved.iter().find(|(a, b, _)| *a <= lo && hi <= *b) {
                Some((_, _, delta)) => shift_selection(s, *delta),
                None => *s,
            }
        })
        .collect();
    Ok(Some(LinePlan {
        transaction: Transaction::new(edits)?.verbatim(),
        selections: Some(selections),
        line_map,
    }))
}

fn copy_lines(
    text: &dyn TextSource,
    selections: &[Selection],
    ctx: &LineOpContext,
    up: bool,
) -> Result<Option<LinePlan>> {
    let count = line_count(text)?;
    let mut edits = Vec::new();
    let mut copied: Vec<(u64, u64)> = Vec::new();
    for (lo, hi) in blocks(selections, &ctx.collapsed) {
        let items = read_block(text, lo, hi, count)?;
        // 在块的起点插入一份副本：光标留在上一份就是“向上复制”，落到下一份就是“向下复制”
        // （VS Code `CopyLinesCommand` 用的同一个技巧）。副本一定在文件中间，所以每行都要终止符。
        let copy = join_block(&items, false, &ctx.eol);
        edits.push(Replacement::insert(line_start(text, lo)?, copy));
        copied.push((lo, hi));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    let selections = selections
        .iter()
        .map(|s| {
            let (lo, hi) = block_of(s);
            // 本块之前插入的行数（向上复制时光标留在原位之上的那一份）。
            let before: i64 = copied
                .iter()
                .filter(|(a, b)| *b < lo || (!up && *a <= lo && hi <= *b))
                .map(|(a, b)| (b - a + 1) as i64)
                .sum();
            shift_selection(s, before)
        })
        .collect();
    // 副本插在每块的起点：块及其后的行都下移，下移量是它之前（含自身）插入的行数。
    let mut line_map = Vec::new();
    let mut shift = 0i64;
    for (i, (lo, hi)) in copied.iter().enumerate() {
        shift += (hi - lo + 1) as i64;
        let next = copied.get(i + 1).map_or(u64::MAX, |(n, _)| n - 1);
        line_map.push((*lo, next, shift));
    }
    Ok(Some(LinePlan {
        transaction: Transaction::new(edits)?.verbatim(),
        selections: Some(selections),
        line_map,
    }))
}

fn delete_lines(text: &dyn TextSource, selections: &[Selection]) -> Result<Option<LinePlan>> {
    let count = line_count(text)?;
    let mut edits = Vec::new();
    for (lo, hi) in blocks(selections, &[]) {
        let range = if hi + 1 < count {
            // 整行 + 终止符。
            Span::new(line_start(text, lo)?, line_start(text, hi + 1)?)?
        } else if lo > 0 {
            // 删到文件末尾：连同上一行的终止符一起删，不留空行。
            Span::new(
                text.line_range(Line(lo - 1))?.end(),
                text.line_range(Line(hi))?.end(),
            )?
        } else {
            Span::new(line_start(text, lo)?, text.line_range(Line(hi))?.end())?
        };
        if range.start() != range.end() {
            edits.push(Replacement::delete(range));
        }
    }
    if edits.is_empty() {
        return Ok(None);
    }
    Ok(Some(LinePlan {
        transaction: Transaction::new(edits)?.verbatim(),
        selections: None,
        line_map: Vec::new(),
    }))
}

fn duplicate_selection(
    text: &dyn TextSource,
    selections: &[Selection],
    ctx: &LineOpContext,
) -> Result<Option<LinePlan>> {
    if selections.iter().all(|s| s.is_empty()) {
        return copy_lines(text, selections, ctx, false);
    }
    let count = line_count(text)?;
    let mut edits = Vec::new();
    for s in selections {
        if s.is_empty() {
            let (lo, hi) = block_of(s);
            let items = read_block(text, lo, hi, count)?;
            edits.push(Replacement::insert(
                line_start(text, lo)?,
                join_block(&items, false, &ctx.eol),
            ));
            continue;
        }
        let start = text.offset(s.start())?;
        let end = text.offset(s.end())?;
        let mut copy = String::new();
        text.visit_chunks(Span::new(start, end)?, &mut |c| copy.push_str(c))?;
        edits.push(Replacement::insert(end, copy));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    Ok(Some(LinePlan {
        transaction: Transaction::new(edits)?.verbatim(),
        selections: None,
        line_map: Vec::new(),
    }))
}

// ------------------------------------------------------------------ 合并行

fn join_lines(text: &dyn TextSource, selections: &[Selection]) -> Result<Option<LinePlan>> {
    let count = line_count(text)?;
    let mut edits = Vec::new();
    for (lo, mut hi) in blocks(selections, &[]) {
        if lo == hi {
            // 只有一行：与下一行合并。
            if hi + 1 >= count {
                continue;
            }
            hi += 1;
        }
        let items = read_block(text, lo, hi, count)?;
        let mut joined = items[0].0.clone();
        for (content, _) in &items[1..] {
            let first = content
                .find(|c: char| c != ' ' && c != '\t')
                .unwrap_or(content.len());
            let rest = &content[first..];
            if rest.is_empty() {
                continue;
            }
            let mut insert_space = !joined.is_empty();
            if insert_space && joined.ends_with([' ', '\t']) {
                // 上一行本来就以空白结尾：折成一个空格，不再另加（VS Code `JoinLinesAction`）。
                insert_space = false;
                joined = format!("{} ", joined.trim_end_matches([' ', '\t']));
            }
            if insert_space {
                joined.push(' ');
            }
            joined.push_str(rest);
        }
        let range = Span::new(line_start(text, lo)?, text.line_range(Line(hi))?.end())?;
        edits.push(Replacement::new(range, joined));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    Ok(Some(LinePlan {
        transaction: Transaction::new(edits)?.verbatim(),
        selections: None,
        line_map: Vec::new(),
    }))
}

// ------------------------------------------------------------------ 排序

fn sort_lines(
    text: &dyn TextSource,
    selections: &[Selection],
    ctx: &LineOpContext,
    descending: bool,
) -> Result<Option<LinePlan>> {
    let count = line_count(text)?;
    // VS Code：选区只占一行（或是光标）时排序整个文件。与 VS Code 的差异：文件以换行结尾时
    // 末尾那个空行不参与排序（VS Code 会把它排到最前面，留下一个开头的空行）。
    let ranges: Vec<(u64, u64)> = if selections.iter().all(|s| block_of(s).0 == block_of(s).1) {
        let last = if count > 1 && line_str(text, count - 1).is_empty() {
            count - 2
        } else {
            count - 1
        };
        vec![(0, last)]
    } else {
        blocks(selections, &[])
    };
    let mut edits = Vec::new();
    for (lo, hi) in ranges {
        if lo >= hi {
            continue;
        }
        let items = read_block(text, lo, hi, count)?;
        let mut sorted: Vec<&(String, String)> = items.iter().collect();
        // 稳定排序：内容相同的行保持原来的先后（与 VS Code 的 Array.sort 一致）。
        sorted.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.0.cmp(&b.0))
        });
        if descending {
            sorted.reverse();
        }
        if sorted.iter().zip(&items).all(|(a, b)| std::ptr::eq(*a, b)) {
            continue;
        }
        // 内容按新顺序，终止符按位置留在原处：文件末行始终没有终止符。
        let reordered: Vec<(String, String)> = sorted
            .iter()
            .zip(&items)
            .map(|(new, old)| (new.0.clone(), old.1.clone()))
            .collect();
        edits.push(Replacement::new(
            block_range(text, lo, hi, count)?,
            join_block(&reordered, hi + 1 >= count, &ctx.eol),
        ));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    Ok(Some(LinePlan {
        transaction: Transaction::new(edits)?.verbatim(),
        selections: None,
        line_map: Vec::new(),
    }))
}

// ------------------------------------------------------------------ 字符对调

/// `editor.action.transpose`：对调光标两侧的字符；在行尾时把该字符挪到下一行行首
/// （VS Code `TransposeAction`，整个行终止符按一个单位处理，CRLF 不会被拆开）。
fn transpose(text: &dyn TextSource, selections: &[Selection]) -> Result<Option<LinePlan>> {
    let count = line_count(text)?;
    let mut edits = Vec::new();
    for s in selections {
        if !s.is_empty() {
            continue;
        }
        let p = s.active;
        let line = line_str(text, p.line.0);
        let col = p.column.0 as usize;
        let (range, source) = if col >= line.len() {
            if p.line.0 + 1 >= count {
                continue;
            }
            let from = prev_grapheme(&line, col);
            let start = text.offset(pos(p.line.0, from as u64))?;
            let end = line_start(text, p.line.0 + 1)?;
            let mut raw = String::new();
            text.visit_chunks(Span::new(start, end)?, &mut |c| raw.push_str(c))?;
            (Span::new(start, end)?, raw)
        } else {
            let from = prev_grapheme(&line, col);
            let to = next_grapheme(&line, col);
            if from == to {
                continue;
            }
            let start = text.offset(pos(p.line.0, from as u64))?;
            let end = text.offset(pos(p.line.0, to as u64))?;
            (Span::new(start, end)?, line[from..to].to_string())
        };
        let mut units: Vec<&str> = source.graphemes(true).collect();
        if units.len() < 2 {
            continue;
        }
        units.reverse();
        edits.push(Replacement::new(range, units.concat()));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    Ok(Some(LinePlan {
        transaction: Transaction::new(edits)?.verbatim(),
        selections: None,
        line_map: Vec::new(),
    }))
}

/// `editor.action.transposeLetters`：对调光标**之前**的两个字符。
fn transpose_letters(text: &dyn TextSource, selections: &[Selection]) -> Result<Option<LinePlan>> {
    let mut edits = Vec::new();
    for s in selections {
        if !s.is_empty() {
            continue;
        }
        let p = s.active;
        let line = line_str(text, p.line.0);
        let col = (p.column.0 as usize).min(line.len());
        let b = prev_grapheme(&line, col);
        if b == 0 {
            continue;
        }
        let a = prev_grapheme(&line, b);
        let start = text.offset(pos(p.line.0, a as u64))?;
        let end = text.offset(pos(p.line.0, col as u64))?;
        let swapped = format!("{}{}", &line[b..col], &line[a..b]);
        edits.push(Replacement::new(Span::new(start, end)?, swapped));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    Ok(Some(LinePlan {
        transaction: Transaction::new(edits)?.verbatim(),
        selections: None,
        line_map: Vec::new(),
    }))
}

fn prev_grapheme(line: &str, at: usize) -> usize {
    line[..at.min(line.len())]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(i, _)| i)
}

fn next_grapheme(line: &str, at: usize) -> usize {
    let at = at.min(line.len());
    line[at..]
        .graphemes(true)
        .next()
        .map_or(at, |g| at + g.len())
}

// ------------------------------------------------------------------ 大小写与命名风格

fn case_transform(
    text: &dyn TextSource,
    selections: &[Selection],
    ctx: &LineOpContext,
    kind: CaseKind,
) -> Result<Option<LinePlan>> {
    let mut edits = Vec::new();
    for s in selections {
        let (start, end) = if s.is_empty() {
            // VS Code：空选区时作用于光标所在的词。
            let line = line_str(text, s.active.line.0);
            let (a, b) = super::selection::word_range_at(
                &line,
                s.active.column.0 as usize,
                &ctx.word_separators,
            );
            if a == b {
                continue;
            }
            (
                pos(s.active.line.0, a as u64),
                pos(s.active.line.0, b as u64),
            )
        } else {
            (s.start(), s.end())
        };
        let range = Span::new(text.offset(start)?, text.offset(end)?)?;
        let mut source = String::new();
        text.visit_chunks(range.clone(), &mut |c| source.push_str(c))?;
        let out = transform(&source, kind, &ctx.word_separators);
        if out == source {
            continue;
        }
        edits.push(Replacement::new(range, out));
    }
    if edits.is_empty() {
        return Ok(None);
    }
    Ok(Some(LinePlan {
        transaction: Transaction::new(edits)?.verbatim(),
        selections: None,
        line_map: Vec::new(),
    }))
}

/// VS Code `caseOperations.ts` 的七种变换。
pub fn transform(text: &str, kind: CaseKind, word_separators: &str) -> String {
    match kind {
        CaseKind::Upper => text.to_uppercase(),
        CaseKind::Lower => text.to_lowercase(),
        CaseKind::Title => title_case(text, word_separators),
        CaseKind::Camel => camel_case(text, false),
        CaseKind::Pascal => camel_case(text, true),
        CaseKind::Snake => delimit_case(text, '_'),
        CaseKind::Kebab => delimit_case(text, '-'),
    }
}

fn title_case(text: &str, word_separators: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start_upper = true;
    for ch in text.chars() {
        if ch == '\r' || ch == '\n' || ch == '\t' || ch == ' ' || word_separators.contains(ch) {
            start_upper = true;
            out.push(ch);
        } else if start_upper {
            start_upper = false;
            out.extend(ch.to_uppercase());
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

fn camel_case(text: &str, pascal: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut first_word = true;
    for word in text.split(|c: char| c == '_' || c == '-' || c.is_whitespace()) {
        if word.is_empty() {
            continue;
        }
        let mut chars = word.chars();
        let head = chars.next().expect("非空");
        if first_word && !pascal {
            out.extend(head.to_lowercase());
        } else {
            out.extend(head.to_uppercase());
        }
        out.push_str(chars.as_str());
        first_word = false;
    }
    out
}

/// `snake_case` / `kebab-case`：在“小写后接大写”与“大写后接大写+小写”处断词，再整体小写
/// （VS Code 的两条 `\p{Ll}\p{Lu}` / `(\p{Lu}|\p{N})\p{Lu}\p{Ll}` 正则）。
fn delimit_case(text: &str, sep: char) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut first = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        first.push(chars[i]);
        if i + 1 < chars.len() && chars[i].is_lowercase() && chars[i + 1].is_uppercase() {
            first.push(sep);
            first.push(chars[i + 1]);
            i += 2;
            continue;
        }
        i += 1;
    }
    let chars: Vec<char> = first.chars().collect();
    let mut second = String::with_capacity(first.len());
    let mut i = 0;
    while i < chars.len() {
        if i + 2 < chars.len()
            && (chars[i].is_uppercase() || chars[i].is_numeric())
            && chars[i + 1].is_uppercase()
            && chars[i + 2].is_lowercase()
        {
            second.push(chars[i]);
            second.push(sep);
            second.push(chars[i + 1]);
            second.push(chars[i + 2]);
            i += 3;
            continue;
        }
        second.push(chars[i]);
        i += 1;
    }
    second.to_lowercase()
}

/// 行操作后的光标位置也要落在合法的字符边界上。
pub fn clamp(text: &dyn TextSource, p: Position) -> Position {
    let Ok(count) = line_count(text) else {
        return Position::default();
    };
    let line = p.line.0.min(count - 1);
    let content = line_str(text, line);
    let mut col = (p.column.0 as usize).min(content.len());
    while !content.is_char_boundary(col) {
        col -= 1;
    }
    Position {
        line: Line(line),
        column: Utf8Column(col as u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_buffer::edit::EditBuffer;
    use rove_buffer::{Eol, LineEndings, RopeText};
    use std::sync::Arc;

    fn buffer(text: &str) -> EditBuffer {
        EditBuffer::new(text, LineEndings::Mixed { preferred: Eol::Lf })
    }

    fn source(text: &str) -> Arc<RopeText> {
        buffer(text).snapshot()
    }

    fn ctx() -> LineOpContext {
        LineOpContext {
            word_separators: "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".into(),
            eol: "\n".into(),
            collapsed: Vec::new(),
        }
    }

    /// 执行一次行操作，返回新全文与新选区。
    fn run(text: &str, op: LineOp, sels: &[Selection]) -> (String, Vec<Selection>) {
        let mut buffer = buffer(text);
        let snapshot = buffer.snapshot();
        let plan = plan(op, &*snapshot, sels, &ctx()).unwrap().expect("有变化");
        let applied = buffer.apply(&plan.transaction).unwrap();
        let after = buffer.snapshot();
        let selections = plan.selections.unwrap_or_else(|| {
            sels.iter()
                .map(|s| {
                    let map = |p: Position| {
                        after
                            .position(applied.map_offset(
                                snapshot.offset(p).unwrap(),
                                rove_buffer::edit::Bias::After,
                            ))
                            .unwrap()
                    };
                    Selection::new(map(s.anchor), map(s.active))
                })
                .collect()
        });
        (buffer.contents(), selections)
    }

    fn caret(line: u64, column: u64) -> Selection {
        Selection::caret(pos(line, column))
    }

    #[test]
    fn move_lines_keeps_each_line_ending() {
        // 混合换行符：第 0 行 CRLF、第 1 行 LF、第 2 行 CR。
        let text = "a\r\nb\nc\rd";
        let (out, sels) = run(text, LineOp::MoveDown, &[caret(1, 0)]);
        assert_eq!(out, "a\r\nc\rb\nd");
        assert_eq!(sels[0], caret(2, 0));
        // 未改动的第 0 行与末行的行尾一个字节都没变。
        assert!(out.starts_with("a\r\n") && out.ends_with("\nd"));
        let (back, _) = run(&out, LineOp::MoveUp, &[caret(2, 0)]);
        assert_eq!(back, text);
    }

    #[test]
    fn move_lines_at_file_edges_do_nothing() {
        let text = "one\ntwo\nthree";
        assert!(plan(LineOp::MoveUp, &*source(text), &[caret(0, 1)], &ctx())
            .unwrap()
            .is_none());
        assert!(
            plan(LineOp::MoveDown, &*source(text), &[caret(2, 1)], &ctx())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn move_last_line_up_moves_the_missing_terminator() {
        let (out, _) = run("one\ntwo\nthree", LineOp::MoveUp, &[caret(2, 0)]);
        assert_eq!(out, "one\nthree\ntwo");
    }

    #[test]
    fn move_lines_moves_a_multi_line_selection_as_a_block() {
        let (out, sels) = run(
            "a\nb\nc\nd",
            LineOp::MoveDown,
            &[Selection::new(pos(0, 0), pos(1, 1))],
        );
        assert_eq!(out, "c\na\nb\nd");
        assert_eq!(sels[0], Selection::new(pos(1, 0), pos(2, 1)));
    }

    #[test]
    fn copy_lines_up_and_down_place_the_caret_differently() {
        let (out, sels) = run("a\nb\nc", LineOp::CopyDown, &[caret(1, 1)]);
        assert_eq!(out, "a\nb\nb\nc");
        assert_eq!(sels[0], caret(2, 1));
        let (out, sels) = run("a\nb\nc", LineOp::CopyUp, &[caret(1, 1)]);
        assert_eq!(out, "a\nb\nb\nc");
        assert_eq!(sels[0], caret(1, 1));
    }

    /// 带折叠执行：返回新全文、新选区与行号搬动。
    fn run_folded(
        text: &str,
        op: LineOp,
        sels: &[Selection],
        collapsed: &[(u64, u64)],
    ) -> (String, Vec<Selection>, LinePlan) {
        let mut buffer = buffer(text);
        let snapshot = buffer.snapshot();
        let ctx = LineOpContext {
            collapsed: collapsed.to_vec(),
            ..ctx()
        };
        let plan = plan(op, &*snapshot, sels, &ctx).unwrap().expect("有变化");
        buffer.apply(&plan.transaction).unwrap();
        let sels = plan.selections.clone().unwrap();
        (buffer.contents(), sels, plan)
    }

    /// #171：折叠首行上的移动把整个折叠区域一起搬走，折叠状态跟着走。
    #[test]
    fn moving_a_folded_header_moves_the_whole_region() {
        // 第 1 行是折叠首行，隐藏第 2、3 行（折叠键 (1, 4)）。
        let text = "a\nfn {\n  x\n}\nb\nc";
        let (out, sels, plan) = run_folded(text, LineOp::MoveDown, &[caret(1, 0)], &[(1, 4)]);
        assert_eq!(out, "a\nb\nfn {\n  x\n}\nc");
        assert_eq!(sels[0], caret(2, 0));
        assert_eq!(plan.map_line(1), 2, "折叠首行跟着块下移一行");
        assert_eq!(plan.map_line(3), 4);
        assert_eq!(plan.map_line(4), 1, "被跨过的行挪到块前面");
        assert_eq!(plan.map_line(5), 5);

        let (out, sels, plan) = run_folded(text, LineOp::MoveUp, &[caret(1, 0)], &[(1, 4)]);
        assert_eq!(out, "fn {\n  x\n}\na\nb\nc");
        assert_eq!(sels[0], caret(0, 0));
        assert_eq!((plan.map_line(1), plan.map_line(0)), (0, 3));
    }

    /// #171：普通行跨过一个折叠区域时，整个区域算一个屏幕行。
    #[test]
    fn moving_across_a_folded_region_skips_it_as_one_screen_line() {
        let text = "a\nfn {\n  x\n}\nb";
        let (out, sels, plan) = run_folded(text, LineOp::MoveDown, &[caret(0, 0)], &[(1, 4)]);
        assert_eq!(out, "fn {\n  x\n}\na\nb");
        assert_eq!(sels[0], caret(3, 0));
        assert_eq!((plan.map_line(0), plan.map_line(1)), (3, 0));

        let (out, sels, _) = run_folded(text, LineOp::MoveUp, &[caret(4, 0)], &[(1, 4)]);
        assert_eq!(out, "a\nb\nfn {\n  x\n}");
        assert_eq!(sels[0], caret(1, 0));
    }

    /// #171：复制折叠首行复制整个区域；原块下移，折叠状态跟着原块走。
    #[test]
    fn copying_a_folded_header_copies_the_whole_region() {
        let text = "fn {\n  x\n}\nb";
        let (out, sels, plan) = run_folded(text, LineOp::CopyDown, &[caret(0, 0)], &[(0, 3)]);
        assert_eq!(out, "fn {\n  x\n}\nfn {\n  x\n}\nb");
        assert_eq!(sels[0], caret(3, 0));
        assert_eq!((plan.map_line(0), plan.map_line(3)), (3, 6));
    }

    #[test]
    fn copy_last_line_gets_a_terminator() {
        let (out, _) = run("a\r\nb", LineOp::CopyDown, &[caret(1, 0)]);
        assert_eq!(out, "a\r\nb\nb");
    }

    #[test]
    fn delete_lines_removes_the_terminator_too() {
        let (out, _) = run("a\r\nb\nc\n", LineOp::Delete, &[caret(1, 0)]);
        assert_eq!(out, "a\r\nc\n");
        let (out, _) = run("a\nb\nc", LineOp::Delete, &[caret(2, 0)]);
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn multiple_cursors_share_one_transaction() {
        let text = "a\nb\nc\nd\ne";
        let plan = plan(
            LineOp::Delete,
            &*source(text),
            &[caret(1, 0), caret(3, 0)],
            &ctx(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.transaction.edits().len(), 2);
        let (out, _) = run(text, LineOp::Delete, &[caret(1, 0), caret(3, 0)]);
        assert_eq!(out, "a\nc\ne");
    }

    #[test]
    fn duplicate_selection_copies_text_or_the_whole_line() {
        let (out, _) = run(
            "alpha beta",
            LineOp::DuplicateSelection,
            &[Selection::new(pos(0, 0), pos(0, 5))],
        );
        assert_eq!(out, "alphaalpha beta");
        let (out, _) = run("alpha", LineOp::DuplicateSelection, &[caret(0, 2)]);
        assert_eq!(out, "alpha\nalpha");
    }

    #[test]
    fn join_lines_follows_vscode_spacing() {
        let (out, _) = run("one\n   two", LineOp::JoinLines, &[caret(0, 0)]);
        assert_eq!(out, "one two");
        let (out, _) = run("one \n   two", LineOp::JoinLines, &[caret(0, 0)]);
        assert_eq!(out, "one two");
        let (out, _) = run(
            "a\nb\nc\nd",
            LineOp::JoinLines,
            &[Selection::new(pos(0, 0), pos(2, 1))],
        );
        assert_eq!(out, "a b c\nd");
    }

    #[test]
    fn sort_lines_ascending_and_descending() {
        let (out, _) = run(
            "b\na\nC\n",
            LineOp::SortAscending,
            &[Selection::new(pos(0, 0), pos(2, 1))],
        );
        assert_eq!(out, "a\nb\nC\n");
        let (out, _) = run(
            "b\na\nC\n",
            LineOp::SortDescending,
            &[Selection::new(pos(0, 0), pos(2, 1))],
        );
        assert_eq!(out, "C\nb\na\n");
        // 单行选区：排序整个文件。
        let (out, _) = run("b\na\nc", LineOp::SortAscending, &[caret(0, 0)]);
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn transpose_swaps_around_the_caret_and_across_the_line_break() {
        let (out, _) = run("abc", LineOp::Transpose, &[caret(0, 1)]);
        assert_eq!(out, "bac");
        let (out, _) = run("ab\r\ncd", LineOp::Transpose, &[caret(0, 2)]);
        assert_eq!(out, "a\r\nbcd");
        let (out, _) = run("ab", LineOp::TransposeLetters, &[caret(0, 2)]);
        assert_eq!(out, "ba");
    }

    #[test]
    fn case_transforms_match_vscode() {
        let seps = ctx().word_separators;
        assert_eq!(
            transform("hello world", CaseKind::Upper, &seps),
            "HELLO WORLD"
        );
        assert_eq!(transform("HELLO", CaseKind::Lower, &seps), "hello");
        assert_eq!(
            transform("hello world", CaseKind::Title, &seps),
            "Hello World"
        );
        assert_eq!(
            transform("foo.bar baz", CaseKind::Title, &seps),
            "Foo.Bar Baz"
        );
        assert_eq!(
            transform("foo_bar baz", CaseKind::Camel, &seps),
            "fooBarBaz"
        );
        assert_eq!(transform("foo_bar", CaseKind::Pascal, &seps), "FooBar");
        assert_eq!(transform("fooBar", CaseKind::Snake, &seps), "foo_bar");
        assert_eq!(
            transform("HTTPRequest", CaseKind::Snake, &seps),
            "http_request"
        );
        assert_eq!(transform("fooBar", CaseKind::Kebab, &seps), "foo-bar");
    }

    #[test]
    fn case_transform_uses_the_word_under_an_empty_caret() {
        let (out, _) = run(
            "let value = 1;",
            LineOp::Case(CaseKind::Upper),
            &[caret(0, 5)],
        );
        assert_eq!(out, "let VALUE = 1;");
    }

    #[test]
    fn trim_trailing_whitespace_reuses_the_save_transform() {
        let (out, _) = run(
            "a   \r\nb\t\nc",
            LineOp::TrimTrailingWhitespace,
            &[caret(0, 0)],
        );
        assert_eq!(out, "a\r\nb\nc");
    }
}
