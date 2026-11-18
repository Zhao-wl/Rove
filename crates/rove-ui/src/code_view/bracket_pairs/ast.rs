//! 括号对树的节点（#57）。
//!
//! 移植 VS Code `bracketPairsTextModelPart/ast.ts` 的节点模型：
//!
//! | 节点 | 含义 |
//! |---|---|
//! | [`Leaf::Text`] | 两个括号之间的普通文本 |
//! | [`Leaf::Bracket`] | 一个括号符号（只作为 [`PairNode`] 的开闭括号出现） |
//! | [`Leaf::Unexpected`] | 多余的右括号（没有对应的左括号） |
//! | [`PairNode`] | 一对括号；`close` 为 `None` 表示缺少右括号 |
//! | [`ListNode`] | 兄弟节点的序列，按 2-3 树组织 |
//!
//! 三条关键性质（直接决定增量更新的复杂度）：
//!
//! 1. **节点只存长度**（[`Length`]），不存绝对偏移。编辑后旁边的子树原样复用，不需要平移。
//! 2. **列表是 2-3 树**（[`ListNode::height`]）。如果列表是平铺的 `Vec`，一次编辑就要重建整条
//!    兄弟序列（压缩 JSON 里可能有几十万个兄弟）；2-3 树让「复用左边一大段 + 改中间一小段 +
//!    复用右边一大段」只花 O(log n) 次拼接（[`concat_all`]）。
//! 3. **[`Node::can_be_reused`]** 决定一个旧子树能不能在新的「已打开括号集合」下原样复用：
//!    里面不能有会被外层括号抢走的右括号（`missing`），也不能以缺右括号的括号对结尾
//!    （否则它会把后面的文本吞进来）。

use super::length::Length;
use std::rc::Rc;

/// 括号种类的位集合：一种括号占一位，所以最多 [`MAX_KINDS`] 种。
///
/// VS Code 用 `SmallImmutableSet`，目的一样：括号种类极少（语言配置里 3–4 种），
/// 用位集合做并集与相交判定是常数时间。
pub type KindSet = u64;

/// 位集合能表示的括号种类上限。
pub const MAX_KINDS: usize = 64;

/// 括号对的最大嵌套层数；超过之后把括号当普通文本，避免递归过深。
/// 与 VS Code 的 `300` 一致。
pub const MAX_DEPTH: u32 = 300;

/// 叶子节点（`Copy`，不需要引用计数）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leaf {
    /// 括号之间的普通文本。
    Text(Length),
    /// 一个括号符号。
    Bracket { length: Length, kind: u8 },
    /// 多余的右括号。
    Unexpected { length: Length, kinds: KindSet },
}

impl Leaf {
    pub fn length(self) -> Length {
        match self {
            Leaf::Text(l) => l,
            Leaf::Bracket { length, .. } | Leaf::Unexpected { length, .. } => length,
        }
    }

    pub fn kind(self) -> u8 {
        match self {
            Leaf::Bracket { kind, .. } => kind,
            Leaf::Unexpected { kinds, .. } => kinds.trailing_zeros().min(63) as u8,
            Leaf::Text(_) => 0,
        }
    }
}

/// 一对括号。`child` 是两个括号之间的内容，`close` 为 `None` 表示缺少右括号。
#[derive(Debug)]
pub struct PairNode {
    pub open: Leaf,
    pub child: Option<Node>,
    pub close: Option<Leaf>,
    pub length: Length,
    /// 内部**没配上对**的右括号种类集合（决定能否在别的上下文里复用）。
    pub missing: KindSet,
}

/// 兄弟节点序列（2-3 树的内部节点）。`children` 的长度恒为 2 或 3，
/// 且所有子节点的 [`Node::height`] 相同。
#[derive(Debug)]
pub struct ListNode {
    pub children: Vec<Node>,
    pub length: Length,
    pub missing: KindSet,
    pub height: u8,
}

/// 括号对树的节点。克隆只复制一个 `Rc`。
#[derive(Clone, Debug)]
pub enum Node {
    Leaf(Leaf),
    Pair(Rc<PairNode>),
    List(Rc<ListNode>),
}

impl Node {
    pub fn length(&self) -> Length {
        match self {
            Node::Leaf(l) => l.length(),
            Node::Pair(p) => p.length,
            Node::List(l) => l.length,
        }
    }

    /// 内部没配上对的右括号种类。
    pub fn missing(&self) -> KindSet {
        match self {
            Node::Leaf(Leaf::Unexpected { kinds, .. }) => *kinds,
            Node::Leaf(_) => 0,
            Node::Pair(p) => p.missing,
            Node::List(l) => l.missing,
        }
    }

    /// 2-3 树中的高度；非列表节点恒为 0。
    pub fn height(&self) -> u8 {
        match self {
            Node::List(l) => l.height,
            _ => 0,
        }
    }

    /// 在「已经打开了 `opened` 这些括号」的上下文里重新解析，结果是否与本节点相同。
    ///
    /// 两个条件（与 VS Code 一致，并且更严一点，见模块文档第 3 条）：
    /// 1. 节点里没有「如果外层开着就会被抢走」的右括号；
    /// 2. 节点不以缺右括号的括号对结尾（否则它会把后面的文本吞进来）。
    pub fn can_be_reused(&self, opened: KindSet) -> bool {
        if self.missing() & opened != 0 {
            return false;
        }
        match self {
            Node::Leaf(Leaf::Text(_)) => true,
            // 单个括号只作为括号对的一部分出现，不允许作为列表项直接复用。
            Node::Leaf(Leaf::Bracket { .. }) => false,
            Node::Leaf(Leaf::Unexpected { .. }) => true,
            Node::Pair(p) => p.close.is_some(),
            Node::List(l) => l.children.last().is_some_and(|c| c.can_be_reused(opened)),
        }
    }

    /// 子节点个数（[`NodeReader`](super::parser::NodeReader) 下探时用）。
    pub fn child_count(&self) -> usize {
        match self {
            Node::Leaf(_) => 0,
            Node::Pair(p) => 1 + usize::from(p.child.is_some()) + usize::from(p.close.is_some()),
            Node::List(l) => l.children.len(),
        }
    }

    /// 第 `index` 个子节点（按文档顺序）。
    pub fn child(&self, index: usize) -> Option<Node> {
        match self {
            Node::Leaf(_) => None,
            Node::Pair(p) => {
                let mut i = index;
                if i == 0 {
                    return Some(Node::Leaf(p.open));
                }
                i -= 1;
                if let Some(child) = &p.child {
                    if i == 0 {
                        return Some(child.clone());
                    }
                    i -= 1;
                }
                if i == 0 {
                    return p.close.map(Node::Leaf);
                }
                None
            }
            Node::List(l) => l.children.get(index).cloned(),
        }
    }
}

/// 造一对括号。
pub fn make_pair(open: Leaf, child: Option<Node>, close: Option<Leaf>) -> Node {
    let mut length = open.length();
    let mut missing = 0;
    if let Some(c) = &child {
        length = length.add(c.length());
        missing |= c.missing();
    }
    if let Some(c) = close {
        length = length.add(c.length());
    }
    Node::Pair(Rc::new(PairNode {
        open,
        child,
        close,
        length,
        missing,
    }))
}

/// 造一个列表节点；`children` 必须有 2–3 个且高度一致。
fn make_list(children: Vec<Node>) -> Node {
    debug_assert!((2..=3).contains(&children.len()));
    debug_assert!(children.iter().all(|c| c.height() == children[0].height()));
    let height = children[0].height() + 1;
    let mut length = Length::ZERO;
    let mut missing = 0;
    for c in &children {
        length = length.add(c.length());
        missing |= c.missing();
    }
    Node::List(Rc::new(ListNode {
        children,
        length,
        missing,
        height,
    }))
}

/// 把若干高度相同的节点组装成一棵平衡的 2-3 树，O(n)。
fn balanced(mut nodes: Vec<Node>) -> Node {
    debug_assert!(!nodes.is_empty());
    while nodes.len() > 3 {
        let mut next = Vec::with_capacity(nodes.len().div_ceil(3));
        let mut rest = nodes.into_iter().collect::<std::collections::VecDeque<_>>();
        while !rest.is_empty() {
            // 剩 4 个时取 2，避免最后落下孤零零的 1 个。
            let take = if rest.len() == 4 {
                2
            } else {
                rest.len().min(3)
            };
            let group: Vec<Node> = rest.drain(..take).collect();
            if group.len() == 1 {
                next.push(group.into_iter().next().expect("非空"));
            } else {
                next.push(make_list(group));
            }
        }
        nodes = next;
    }
    if nodes.len() == 1 {
        nodes.pop().expect("非空")
    } else {
        make_list(nodes)
    }
}

/// 拼接两棵 2-3 树（持久化版本，不改原节点）。结果高度为 `max(h1, h2)` 或 `max(h1, h2) + 1`。
///
/// 复杂度 O(|h1 − h2|)：只沿着高的那棵树的一侧脊走下去。
fn concat(a: Node, b: Node) -> Node {
    match a.height().cmp(&b.height()) {
        std::cmp::Ordering::Equal => make_list(vec![a, b]),
        std::cmp::Ordering::Greater => {
            let Node::List(list) = &a else {
                unreachable!("高度大于 0 的只能是列表");
            };
            let mut kids = list.children.clone();
            let last = kids.pop().expect("列表非空");
            let merged = concat(last, b);
            regroup(kids, merged, list.height, true)
        }
        std::cmp::Ordering::Less => {
            let Node::List(list) = &b else {
                unreachable!("高度大于 0 的只能是列表");
            };
            let mut kids = list.children.clone();
            let first = kids.remove(0);
            let merged = concat(a, first);
            regroup(kids, merged, list.height, false)
        }
    }
}

/// `merged` 替换掉原列表的首/末子节点后重新成组；`merged` 高度可能比兄弟高一层（分裂）。
fn regroup(mut siblings: Vec<Node>, merged: Node, height: u8, append: bool) -> Node {
    let mut kids: Vec<Node> = Vec::with_capacity(5);
    let merged_split = merged.height() == height;
    let parts: Vec<Node> = if merged_split {
        let Node::List(m) = &merged else {
            unreachable!("分裂结果一定是列表");
        };
        m.children.clone()
    } else {
        vec![merged]
    };
    if append {
        kids.append(&mut siblings);
        kids.extend(parts);
    } else {
        kids.extend(parts);
        kids.append(&mut siblings);
    }
    if kids.len() <= 3 {
        make_list(kids)
    } else {
        // 4 或 5 个：拆成两组，每组 2–3 个。
        let right = kids.split_off(kids.len() / 2);
        make_list(vec![make_list(kids), make_list(right)])
    }
}

/// 把解析出来的若干节点拼成一棵平衡列表。
///
/// 先把**高度相同的连续段**各自组装成平衡树（整篇重新解析时所有叶子高度都是 0，这一步就是
/// 一次 O(n) 的自底向上建树），再把剩下的少数几棵按高度拼起来。
pub fn concat_all(items: Vec<Node>) -> Option<Node> {
    let mut items = items;
    match items.len() {
        0 => return None,
        1 => return items.pop(),
        _ => {}
    }
    let mut parts: Vec<Node> = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let height = items[i].height();
        let start = i;
        while i < items.len() && items[i].height() == height {
            i += 1;
        }
        if i - start >= 2 {
            parts.push(balanced(items[start..i].to_vec()));
        } else {
            parts.push(items[start].clone());
        }
    }
    let mut iter = parts.into_iter();
    let mut acc = iter.next().expect("非空");
    for node in iter {
        acc = concat(acc, node);
    }
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(columns: u32) -> Node {
        Node::Leaf(Leaf::Text(Length::new(0, columns)))
    }

    /// 树里所有叶子的长度之和应等于根的长度；高度也要一致。
    fn check(node: &Node) -> (Length, u8) {
        match node {
            Node::Leaf(l) => (l.length(), 0),
            Node::Pair(p) => (p.length, 0),
            Node::List(l) => {
                assert!((2..=3).contains(&l.children.len()), "2-3 树的度数");
                let mut total = Length::ZERO;
                for c in &l.children {
                    let (len, h) = check(c);
                    assert_eq!(h + 1, l.height, "子树高度一致");
                    total = total.add(len);
                }
                assert_eq!(total, l.length);
                (total, l.height)
            }
        }
    }

    #[test]
    fn balanced_list_stays_23_shaped() {
        for n in 1..200usize {
            let items: Vec<Node> = (0..n).map(|i| text(i as u32 % 7 + 1)).collect();
            let expected = items
                .iter()
                .fold(Length::ZERO, |acc, i| acc.add(i.length()));
            let root = concat_all(items).expect("非空");
            let (len, height) = check(&root);
            assert_eq!(len, expected, "n={n}");
            // 2-3 树：n 个叶子的高度不超过 log2(n) + 1。
            let bound = (usize::BITS - n.leading_zeros()) as u8 + 1;
            assert!(height <= bound, "n={n} height={height} bound={bound}");
        }
    }

    #[test]
    fn concat_of_unequal_heights_stays_balanced() {
        let big = concat_all((0..100).map(|_| text(1)).collect()).expect("非空");
        let small = concat_all((0..3).map(|_| text(2)).collect()).expect("非空");
        assert!(big.height() > small.height());
        let joined = concat_all(vec![big.clone(), small.clone(), text(5)]).expect("非空");
        let (len, _) = check(&joined);
        assert_eq!(len, big.length().add(small.length()).add(Length::new(0, 5)));
        // 反过来接也一样。
        let joined = concat_all(vec![text(5), small, big]).expect("非空");
        check(&joined);
    }

    #[test]
    fn reuse_rules() {
        let paren: KindSet = 1;
        let brace: KindSet = 2;
        let closed = make_pair(
            Leaf::Bracket {
                length: Length::new(0, 1),
                kind: 0,
            },
            Some(text(3)),
            Some(Leaf::Bracket {
                length: Length::new(0, 1),
                kind: 0,
            }),
        );
        assert!(closed.can_be_reused(0));
        assert!(closed.can_be_reused(brace));

        // 缺右括号的括号对不能复用：它会把后面的文本吞进来。
        let unclosed = make_pair(
            Leaf::Bracket {
                length: Length::new(0, 1),
                kind: 0,
            },
            Some(text(3)),
            None,
        );
        assert!(!unclosed.can_be_reused(0));

        // 内部有多余的右括号：外层开着同种括号时不能复用。
        let stray = make_pair(
            Leaf::Bracket {
                length: Length::new(0, 1),
                kind: 1,
            },
            Some(Node::Leaf(Leaf::Unexpected {
                length: Length::new(0, 1),
                kinds: paren,
            })),
            Some(Leaf::Bracket {
                length: Length::new(0, 1),
                kind: 1,
            }),
        );
        assert!(stray.can_be_reused(brace));
        assert!(!stray.can_be_reused(paren));

        // 列表以缺右括号的括号对结尾时同样不能复用。
        let list = concat_all(vec![closed.clone(), unclosed]).expect("非空");
        assert!(!list.can_be_reused(0));
        let list = concat_all(vec![closed.clone(), text(1)]).expect("非空");
        assert!(list.can_be_reused(0));
        let list = concat_all(vec![stray, text(1)]).expect("非空");
        assert!(!list.can_be_reused(paren), "中间的多余右括号也要拦住");
    }

    #[test]
    fn pair_children_are_indexed_in_document_order() {
        let open = Leaf::Bracket {
            length: Length::new(0, 1),
            kind: 0,
        };
        let close = Leaf::Bracket {
            length: Length::new(0, 1),
            kind: 0,
        };
        let pair = make_pair(open, Some(text(3)), Some(close));
        assert_eq!(pair.child_count(), 3);
        assert_eq!(pair.child(0).map(|n| n.length()), Some(Length::new(0, 1)));
        assert_eq!(pair.child(1).map(|n| n.length()), Some(Length::new(0, 3)));
        assert_eq!(pair.child(2).map(|n| n.length()), Some(Length::new(0, 1)));
        assert!(pair.child(3).is_none());

        let pair = make_pair(open, None, None);
        assert_eq!(pair.child_count(), 1);
        assert!(pair.child(1).is_none());
    }
}
