//! 在括号对树上查询（#57）：某一行上每个括号的嵌套深度，以及光标所在的活动括号对。
//!
//! 节点只存长度，所以查询是「一边往下走一边累加起始位置」。按行查询会把起点在该行之后、
//! 终点在该行之前的子树整棵剪掉，所以是 O(树高 + 该行括号数)。

use super::ast::{KindSet, Leaf, Node, MAX_KINDS};
use super::length::Length;
use std::ops::Range;

/// 一行里的一个括号。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketHit {
    /// 行内 UTF-8 字节区间。
    pub range: Range<u32>,
    pub kind: u8,
    /// 嵌套深度，1 起（所有括号种类一起计数）。
    pub level: u32,
    /// 同种括号的嵌套深度，1 起（`independentColorPoolPerBracketType` 用）。
    pub level_of_kind: u32,
    /// 没配上对：多余的右括号，或缺少右括号的左括号。
    pub unmatched: bool,
}

/// 一对括号的绝对位置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairSpan {
    pub kind: u8,
    pub open: (Length, u32),
    pub close: Option<(Length, u32)>,
}

impl PairSpan {
    fn open_end(&self) -> Length {
        self.open.0.add(Length::new(0, self.open.1))
    }
    fn close_end(&self) -> Option<Length> {
        self.close
            .map(|(start, len)| start.add(Length::new(0, len)))
    }
}

/// 某一行上的全部括号，按列升序。
pub fn brackets_in_line(root: &Node, line: u64) -> Vec<BracketHit> {
    let mut out = Vec::new();
    let mut per_kind = [0u32; MAX_KINDS];
    walk(root, Length::ZERO, line, 0, &mut per_kind, &mut out);
    out
}

/// 一个括号的深度信息（把 [`walk`] 要传的几项打包，免得参数列表过长）。
#[derive(Clone, Copy)]
struct Depth {
    kind: u8,
    level: u32,
    level_of_kind: u32,
    unmatched: bool,
}

/// 只收集落在 `line` 上的括号。
fn push_hit(out: &mut Vec<BracketHit>, line: u64, start: Length, length: Length, depth: Depth) {
    if start.lines != line {
        return;
    }
    out.push(BracketHit {
        range: start.columns..start.columns.saturating_add(length.columns),
        kind: depth.kind,
        level: depth.level,
        level_of_kind: depth.level_of_kind,
        unmatched: depth.unmatched,
    });
}

fn walk(
    node: &Node,
    start: Length,
    line: u64,
    level: u32,
    per_kind: &mut [u32; MAX_KINDS],
    out: &mut Vec<BracketHit>,
) {
    if start.lines > line || start.add(node.length()).lines < line {
        return;
    }
    match node {
        Node::Leaf(Leaf::Text(_)) | Node::Leaf(Leaf::Bracket { .. }) => {}
        Node::Leaf(Leaf::Unexpected { length, kinds }) => push_hit(
            out,
            line,
            start,
            *length,
            Depth {
                kind: kinds.trailing_zeros().min(63) as u8,
                level,
                level_of_kind: 0,
                unmatched: true,
            },
        ),
        Node::List(list) => {
            let mut at = start;
            for child in &list.children {
                if at.lines > line {
                    break;
                }
                walk(child, at, line, level, per_kind, out);
                at = at.add(child.length());
            }
        }
        Node::Pair(pair) => {
            let kind = pair.open.kind() as usize;
            let slot = per_kind.get(kind).copied().unwrap_or(0) + 1;
            if let Some(s) = per_kind.get_mut(kind) {
                *s = slot;
            }
            let level = level + 1;
            let depth = Depth {
                kind: kind as u8,
                level,
                level_of_kind: slot,
                unmatched: pair.close.is_none(),
            };
            push_hit(out, line, start, pair.open.length(), depth);
            let mut at = start.add(pair.open.length());
            if let Some(child) = &pair.child {
                walk(child, at, line, level, per_kind, out);
                at = at.add(child.length());
            }
            if let Some(close) = pair.close {
                push_hit(
                    out,
                    line,
                    at,
                    close.length(),
                    Depth {
                        unmatched: false,
                        ..depth
                    },
                );
            }
            if let Some(s) = per_kind.get_mut(kind) {
                *s = slot - 1;
            }
        }
    }
}

/// 一对**配对完整**的括号对（括号对参考线用，#57 第二部分）。
///
/// 与 [`BracketHit`] 的区别是这里给的是整对括号的绝对位置：参考线要在左右括号之间画竖线，
/// 只知道某一行上的括号不够用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidePair {
    pub kind: u8,
    /// 嵌套深度，1 起（与着色共用同一套计数，保证颜色一致）。
    pub level: u32,
    /// 同种括号的嵌套深度，1 起（`independentColorPoolPerBracketType` 用）。
    pub level_of_kind: u32,
    /// 左括号起点与字节长度。
    pub open: Length,
    pub open_len: u32,
    /// 右括号起点与字节长度。
    pub close: Length,
    pub close_len: u32,
}

impl GuidePair {
    /// 单行括号对（VS Code 的活动括号对参考线会把它们排除在外）。
    pub fn is_single_line(&self) -> bool {
        self.open.lines == self.close.lines
    }

    /// 闭区间 `[左括号起点, 右括号终点]` 是否包含 `position`。
    pub fn contains(&self, position: Length) -> bool {
        self.open <= position && position <= self.close.add(Length::new(0, self.close_len))
    }
}

/// 与 `lines` 行区间相交、且配对完整的括号对，**外层在前**（同一层按文档顺序）。
///
/// 起点在该区间之后、终点在该区间之前的子树整棵剪掉，所以代价与「相交的括号对数」成正比，
/// 与文档大小无关。
pub fn guide_pairs(root: &Node, lines: Range<u64>) -> Vec<GuidePair> {
    let mut out = Vec::new();
    let mut per_kind = [0u32; MAX_KINDS];
    collect_guides(root, Length::ZERO, &lines, 0, &mut per_kind, &mut out);
    out
}

fn collect_guides(
    node: &Node,
    start: Length,
    lines: &Range<u64>,
    level: u32,
    per_kind: &mut [u32; MAX_KINDS],
    out: &mut Vec<GuidePair>,
) {
    if lines.is_empty() || start.lines >= lines.end || start.add(node.length()).lines < lines.start
    {
        return;
    }
    match node {
        Node::Leaf(_) => {}
        Node::List(list) => {
            let mut at = start;
            for child in &list.children {
                if at.lines >= lines.end {
                    break;
                }
                collect_guides(child, at, lines, level, per_kind, out);
                at = at.add(child.length());
            }
        }
        Node::Pair(pair) => {
            let kind = pair.open.kind() as usize;
            let slot = per_kind.get(kind).copied().unwrap_or(0) + 1;
            if let Some(s) = per_kind.get_mut(kind) {
                *s = slot;
            }
            let level = level + 1;
            let after_open = start.add(pair.open.length());
            let child_end = match &pair.child {
                Some(child) => after_open.add(child.length()),
                None => after_open,
            };
            if let Some(close) = pair.close {
                out.push(GuidePair {
                    kind: kind as u8,
                    level,
                    level_of_kind: slot,
                    open: start,
                    open_len: pair.open.length().columns,
                    close: child_end,
                    close_len: close.length().columns,
                });
            }
            if let Some(child) = &pair.child {
                collect_guides(child, after_open, lines, level, per_kind, out);
            }
            if let Some(s) = per_kind.get_mut(kind) {
                *s = slot - 1;
            }
        }
    }
}

/// 光标所在的**活动**括号对参考线：包含光标、配对完整且**跨行**的最内层括号对。
///
/// 排除单行括号对与 VS Code 一致（否则 `if (true) { | }` 这种写法会把整行标成活动）。
pub fn active_guide_pair(root: &Node, position: Length) -> Option<GuidePair> {
    guide_pairs(root, position.lines..position.lines + 1)
        .into_iter()
        .rfind(|p| !p.is_single_line() && p.contains(position))
}

/// 定位结果：包含某个位置（闭区间）的括号对链（外 → 内），以及该位置附近多余的右括号。
#[derive(Clone, Debug, Default)]
pub struct Located {
    pub pairs: Vec<PairSpan>,
    /// (起点, 字节长度, 种类集合)。
    pub unmatched: Vec<(Length, u32, KindSet)>,
}

/// 找出所有「闭区间包含 `position`」的括号对。
pub fn locate(root: &Node, position: Length) -> Located {
    let mut out = Located::default();
    locate_in(root, Length::ZERO, position, &mut out);
    out
}

fn locate_in(node: &Node, start: Length, position: Length, out: &mut Located) {
    let end = start.add(node.length());
    if position < start || position > end {
        return;
    }
    match node {
        Node::Leaf(Leaf::Unexpected { length, kinds }) => {
            out.unmatched.push((start, length.columns, *kinds));
        }
        Node::Leaf(_) => {}
        Node::List(list) => {
            let mut at = start;
            for child in &list.children {
                if at > position {
                    break;
                }
                locate_in(child, at, position, out);
                at = at.add(child.length());
            }
        }
        Node::Pair(pair) => {
            let after_open = start.add(pair.open.length());
            let child_end = match &pair.child {
                Some(child) => after_open.add(child.length()),
                None => after_open,
            };
            out.pairs.push(PairSpan {
                kind: pair.open.kind(),
                open: (start, pair.open.length().columns),
                close: pair.close.map(|c| (child_end, c.length().columns)),
            });
            if let Some(child) = &pair.child {
                locate_in(child, after_open, position, out);
            }
        }
    }
}

/// `editor.matchBrackets` 的三个取值（与 #20 的 [`MatchBrackets`](crate::code_view::brackets::MatchBrackets) 同义）。
pub use crate::code_view::brackets::{BracketMatch, MatchBrackets};

fn to_match(pair: &PairSpan) -> Option<BracketMatch> {
    let (open_start, open_len) = pair.open;
    let (close_start, close_len) = pair.close?;
    Some(BracketMatch {
        open: (
            open_start.lines,
            open_start.columns..open_start.columns + open_len,
        ),
        close: (
            close_start.lines,
            close_start.columns..close_start.columns + close_len,
        ),
    })
}

/// 光标所在的活动括号对。
///
/// 语义与 #20 的就近扫描一致：先看光标**左边**紧挨着的括号，再看右边；都没有时
/// `always` 再找包围光标的最内层括号对，`near` 直接没有，`never` 永远没有。
/// 紧挨着的括号没配上对时返回 `None`（不再往外找），这一点也与 #20 相同。
///
/// `matchable` 是允许参与匹配的括号种类集合（即 #12 的 `brackets`）：
/// `colorizedBracketPairs` 里多出来的种类（C# 的 `<>`）只着色、不参与匹配。
pub fn active_pair(
    root: &Node,
    line: u64,
    column: u32,
    mode: MatchBrackets,
    matchable: KindSet,
) -> Option<BracketMatch> {
    if mode == MatchBrackets::Never {
        return None;
    }
    let position = Length::new(line, column);
    let located = locate(root, position);
    let allowed = |kind: u8| matchable & (1u64 << kind) != 0;

    // 紧挨着光标的括号：先左后右，同一侧取最内层。
    for want_left in [true, false] {
        for pair in located.pairs.iter().rev() {
            if !allowed(pair.kind) {
                continue;
            }
            let hit = if want_left {
                pair.open_end() == position || pair.close_end() == Some(position)
            } else {
                pair.open.0 == position || pair.close.map(|c| c.0) == Some(position)
            };
            if hit {
                return to_match(pair);
            }
        }
        for (start, len, kinds) in &located.unmatched {
            if kinds & matchable == 0 {
                continue;
            }
            let end = start.add(Length::new(0, *len));
            let hit = if want_left {
                end == position
            } else {
                *start == position
            };
            if hit {
                // 多余的右括号：没有配对，与 #20 一样不高亮。
                return None;
            }
        }
    }
    if mode == MatchBrackets::Near {
        return None;
    }
    // 包围光标的最内层括号对。
    located
        .pairs
        .iter()
        .rev()
        .find(|pair| {
            allowed(pair.kind)
                && pair.open_end() <= position
                && pair.close.is_some_and(|(start, _)| start >= position)
        })
        .and_then(to_match)
}

#[cfg(test)]
mod tests {
    use super::super::tokenizer::{symbols, BracketKind, TextDocument, Tokenizer};
    use super::*;

    pub(super) fn kinds() -> Vec<BracketKind> {
        [("{", "}"), ("[", "]"), ("(", ")")]
            .iter()
            .map(|(o, c)| BracketKind {
                open: (*o).into(),
                close: (*c).into(),
                colorized: true,
                matchable: true,
            })
            .collect()
    }

    fn tree(text: &str) -> Node {
        let doc = TextDocument::new(text);
        let kinds = kinds();
        let syms = symbols(&kinds);
        let mut tokenizer = Tokenizer::new(&doc, &syms, usize::MAX);
        super::super::parser::parse(&mut tokenizer, None, Vec::new())
            .0
            .expect("非空")
    }

    #[test]
    fn levels_count_from_one_and_per_kind() {
        let root = tree("{[({})]}");
        let hits = brackets_in_line(&root, 0);
        let levels: Vec<u32> = hits.iter().map(|h| h.level).collect();
        assert_eq!(levels, vec![1, 2, 3, 4, 4, 3, 2, 1]);
        let per_kind: Vec<u32> = hits.iter().map(|h| h.level_of_kind).collect();
        assert_eq!(per_kind, vec![1, 1, 1, 2, 2, 1, 1, 1]);
        assert!(hits.iter().all(|h| !h.unmatched));
    }

    #[test]
    fn active_pair_matches_the_nearest_brackets() {
        let root = tree("f(a, (b))");
        let all: KindSet = 0b111;
        // 光标在 `(` 右边。
        let m = active_pair(&root, 0, 2, MatchBrackets::Near, all).expect("有匹配");
        assert_eq!((m.open.1, m.close.1), (1..2, 8..9));
        // 光标在 `)` 左边。
        let m = active_pair(&root, 0, 7, MatchBrackets::Near, all).expect("有匹配");
        assert_eq!((m.open.1, m.close.1), (5..6, 7..8));
        // 不挨着括号。
        assert!(active_pair(&root, 0, 3, MatchBrackets::Near, all).is_none());
        let m = active_pair(&root, 0, 3, MatchBrackets::Always, all).expect("有包围");
        assert_eq!((m.open.1, m.close.1), (1..2, 8..9));
        assert!(active_pair(&root, 0, 3, MatchBrackets::Never, all).is_none());
    }

    #[test]
    fn unmatched_brackets_do_not_highlight() {
        let root = tree("(a))");
        // 光标挨着多余的右括号：不高亮。
        assert!(active_pair(&root, 0, 4, MatchBrackets::Always, 0b111).is_none());
        let root = tree("((a)");
        // 光标挨着缺右括号的左括号：不高亮。
        assert!(active_pair(&root, 0, 1, MatchBrackets::Always, 0b111).is_none());
    }

    // ------------------------------------------------------------------ 括号对参考线（第二部分）

    const NESTED: &str = concat!(
        "class C\n",            // 0
        "{\n",                  // 1
        "    void M()\n",       // 2
        "    {\n",              // 3
        "        f(a, (b));\n", // 4
        "    }\n",              // 5
        "}\n",                  // 6
    );

    fn spans(pairs: &[GuidePair]) -> Vec<(u64, u32, u64, u32, u32)> {
        pairs
            .iter()
            .map(|p| {
                (
                    p.open.lines,
                    p.open.columns,
                    p.close.lines,
                    p.close.columns,
                    p.level,
                )
            })
            .collect()
    }

    #[test]
    fn guide_pairs_are_outermost_first_and_carry_the_colorization_level() {
        let root = tree(NESTED);
        let all = guide_pairs(&root, 0..7);
        assert_eq!(
            spans(&all),
            vec![
                // class C 的 `{` … `}`
                (1, 0, 6, 0, 1),
                // M()
                (2, 10, 2, 11, 2),
                // 方法体
                (3, 4, 5, 4, 2),
                // f( … )
                (4, 9, 4, 16, 3),
                // (b)
                (4, 13, 4, 15, 4),
            ]
        );
        // 同种括号各自计数：`M()` 闭合之后圆括号的计数回到 0，所以 `f(` 又是第 1 对。
        assert_eq!(
            all.iter().map(|p| p.level_of_kind).collect::<Vec<_>>(),
            vec![1, 1, 2, 1, 2]
        );
    }

    #[test]
    fn guide_pairs_only_returns_pairs_crossing_the_range() {
        let root = tree(NESTED);
        // 只要第 5 行：外层两对跨过它，第 4 行那两对不跨。
        let hit = guide_pairs(&root, 5..6);
        assert_eq!(spans(&hit), vec![(1, 0, 6, 0, 1), (3, 4, 5, 4, 2)]);
        // 空区间什么都不返回。
        assert!(guide_pairs(&root, 3..3).is_empty());
    }

    #[test]
    fn unmatched_open_brackets_have_no_guide() {
        // 缺右括号的 `(` 不画参考线，里面配上对的照画。
        let root = tree("f(\n  g(x)\n");
        assert_eq!(spans(&guide_pairs(&root, 0..2)), vec![(1, 3, 1, 5, 2)]);
    }

    #[test]
    fn the_active_guide_pair_is_the_innermost_multiline_one() {
        let root = tree(NESTED);
        let at = |line, col| {
            active_guide_pair(&root, Length::new(line, col)).map(|p| (p.open.lines, p.level))
        };
        // 第 4 行在方法体里：活动的是方法体那对，而不是同一行的单行括号对。
        assert_eq!(at(4, 14), Some((3, 2)));
        // 光标在方法体的 `{` 上：这一对的闭区间含它。
        assert_eq!(at(3, 4), Some((3, 2)));
        // 第 2 行只在 class 的括号对里。
        assert_eq!(at(2, 0), Some((1, 1)));
        // 第 0 行在所有括号对之外。
        assert_eq!(at(0, 0), None);
    }

    #[test]
    fn a_single_line_pair_is_never_active() {
        let root = tree("f(a);\n");
        assert_eq!(active_guide_pair(&root, Length::new(0, 3)), None);
    }

    #[test]
    fn non_matchable_kinds_are_skipped() {
        let root = tree("{[a]}");
        // 只允许花括号参与匹配：方括号旁边的光标找不到匹配，always 退回到外层花括号。
        let m = active_pair(&root, 0, 2, MatchBrackets::Always, 0b001).expect("退到花括号");
        assert_eq!((m.open.1, m.close.1), (0..1, 4..5));
    }
}
