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

    #[test]
    fn non_matchable_kinds_are_skipped() {
        let root = tree("{[a]}");
        // 只允许花括号参与匹配：方括号旁边的光标找不到匹配，always 退回到外层花括号。
        let m = active_pair(&root, 0, 2, MatchBrackets::Always, 0b001).expect("退到花括号");
        assert_eq!((m.open.1, m.close.1), (0..1, 4..5));
    }
}
