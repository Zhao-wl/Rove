//! 括号对树的解析与**增量更新**（#57）。
//!
//! 对应 VS Code 的 `bracketPairsTextModelPart/parser.ts` + `nodeReader.ts` +
//! `beforeEditPositionMapper.ts`，三件事：
//!
//! 1. [`Parser::parse_list`] / [`Parser::parse_child`]：一次递归下降，按「已打开的括号集合」
//!    做错误恢复（右括号属于外层时就把当前这层收尾，而不是乱配对）。
//! 2. [`NodeReader`]：按文档顺序在**旧树**上走，给出「从某个位置开始、最长的、可复用的」子树。
//! 3. [`PositionMapper`]：把新文档的位置映射回旧文档，并给出「到下一处改动还有多远」——
//!    只有短于这个距离的旧子树才允许复用，否则就会把改动跳过去。
//!
//! 复杂度：设文档有 n 个括号、树高 h = O(log n)。一次编辑只重建从根到被改区域的那条路径上的
//! 节点，其余子树按引用复用，所以是 **O(改动涉及的 token 数 + h)**，与 n 无关。
//! 整篇首次解析是 O(n)（自底向上建 2-3 树）。

use super::ast::{concat_all, make_pair, KindSet, Leaf, Node, MAX_DEPTH, MAX_KINDS};
use super::length::Length;
use super::tokenizer::{Token, Tokenizer};
use std::ops::Range;

/// 一处行级变化：旧文本的 `old` 行区间被新文本的 `new` 行区间取代（半开区间）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineEdit {
    pub old: Range<u64>,
    pub new: Range<u64>,
}

/// 解析统计（基准与测试用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParseStats {
    /// 从旧树里原样复用的节点数。
    pub reused: usize,
    /// 新建的节点数。
    pub constructed: usize,
}

/// 新文档位置 → 旧文档位置，以及到下一处改动的距离。
#[derive(Clone, Debug, Default)]
pub struct PositionMapper {
    edits: Vec<LineEdit>,
}

impl PositionMapper {
    pub fn new(mut edits: Vec<LineEdit>) -> Self {
        edits.sort_by_key(|e| e.new.start);
        Self { edits }
    }

    /// 改动在新文档里覆盖的行区间（至少一行：纯删除也会改到接缝那一行）。
    fn barrier(edit: &LineEdit) -> Range<u64> {
        edit.new.start..edit.new.end.max(edit.new.start + 1)
    }

    /// 新坐标 → 旧坐标。只在未改动的区域调用。
    pub fn to_old(&self, position: Length) -> Length {
        let mut delta: i64 = 0;
        for edit in &self.edits {
            if position.lines < Self::barrier(edit).end {
                break;
            }
            delta = edit.old.end as i64 - edit.new.end as i64;
        }
        Length::new(
            (position.lines as i64 + delta).max(0) as u64,
            position.columns,
        )
    }

    /// 从 `position` 到下一处改动的距离；`None` 表示后面没有改动了。
    /// 返回 [`Length::ZERO`] 表示就在改动里。
    pub fn distance_to_change(&self, position: Length) -> Option<Length> {
        for edit in &self.edits {
            let barrier = Self::barrier(edit);
            if position.lines >= barrier.end {
                continue;
            }
            if position.lines >= barrier.start {
                return Some(Length::ZERO);
            }
            return Some(position.to(Length::new(barrier.start, 0)));
        }
        None
    }
}

/// 在旧树上按文档顺序前进的读取器。
///
/// `nodes` 是一条从根到「当前候选节点」的路径，每项带着该节点的起始偏移；
/// `idxs[i]` 是 `nodes[i + 1]` 在 `nodes[i]` 里的下标。
pub struct NodeReader {
    nodes: Vec<(Node, Length)>,
    idxs: Vec<usize>,
    last: Length,
}

impl NodeReader {
    pub fn new(root: Node) -> Self {
        Self {
            nodes: vec![(root, Length::ZERO)],
            idxs: Vec::new(),
            last: Length::ZERO,
        }
    }

    /// 从 `offset` 开始、满足 `accept` 的**最长**子树；没有就返回 `None`。
    /// `offset` 必须单调不减。
    pub fn read_longest_at(
        &mut self,
        offset: Length,
        accept: &mut dyn FnMut(&Node) -> bool,
    ) -> Option<Node> {
        if offset < self.last {
            return None;
        }
        self.last = offset;
        loop {
            let (node, start) = self.nodes.last()?.clone();
            if offset < start {
                // 还没走到这里。
                return None;
            }
            if start < offset {
                // 当前节点在 offset 之前开始：整段都在前面就跳过它，
                // 否则下探到它的子节点；叶子节点无可下探，同样跳过。
                let past = start.add(node.length()) <= offset;
                if past || !self.descend() {
                    self.advance();
                }
                continue;
            }
            // start == offset：这是从这个位置开始的最长候选。
            if accept(&node) {
                self.advance();
                return Some(node);
            }
            if !self.descend() {
                return None;
            }
        }
    }

    /// 下探到当前节点的第一个子节点。
    fn descend(&mut self) -> bool {
        let Some((node, start)) = self.nodes.last().cloned() else {
            return false;
        };
        match node.child(0) {
            Some(child) => {
                self.idxs.push(0);
                self.nodes.push((child, start));
                true
            }
            None => false,
        }
    }

    /// 前进到文档顺序上的下一个节点（跳过当前节点的全部内容）。
    fn advance(&mut self) {
        loop {
            let Some((node, start)) = self.nodes.pop() else {
                return;
            };
            let Some(index) = self.idxs.pop() else {
                // 根节点走完了。
                return;
            };
            let Some((parent, _)) = self.nodes.last().cloned() else {
                return;
            };
            let next = index + 1;
            if let Some(sibling) = parent.child(next) {
                self.idxs.push(next);
                self.nodes.push((sibling, start.add(node.length())));
                return;
            }
            // 父节点的子节点走完了，继续往上退。
        }
    }
}

struct Parser<'t, 'a> {
    tokenizer: &'t mut Tokenizer<'a>,
    reader: Option<NodeReader>,
    mapper: PositionMapper,
    stats: ParseStats,
}

/// 解析整篇文档。`old` 是上一棵树，`edits` 是从那棵树到现在的行级变化；
/// 两者都给出时做增量更新，否则整篇重新解析。
pub fn parse(
    tokenizer: &mut Tokenizer<'_>,
    old: Option<Node>,
    edits: Vec<LineEdit>,
) -> (Option<Node>, ParseStats) {
    let mut parser = Parser {
        tokenizer,
        reader: old.map(NodeReader::new),
        mapper: PositionMapper::new(edits),
        stats: ParseStats::default(),
    };
    let root = parser.parse_list(0, 0);
    (root, parser.stats)
}

impl Parser<'_, '_> {
    fn parse_list(&mut self, opened: KindSet, depth: u32) -> Option<Node> {
        let mut items: Vec<Node> = Vec::new();
        loop {
            if let Some(cached) = self.read_from_cache(opened) {
                items.push(cached);
                continue;
            }
            match self.tokenizer.peek() {
                None => break,
                // 右括号属于外层：收尾，交给外层处理。
                Some(Token::Close { kind, .. }) if (1u64 << kind) & opened != 0 => break,
                _ => {}
            }
            let child = self.parse_child(opened, depth + 1);
            items.push(child);
        }
        concat_all(items)
    }

    fn parse_child(&mut self, opened: KindSet, depth: u32) -> Node {
        let token = self.tokenizer.read().expect("调用前已经 peek 过");
        self.stats.constructed += 1;
        match token {
            Token::Text(length) => Node::Leaf(Leaf::Text(length)),
            Token::Close { length, kind } => Node::Leaf(Leaf::Unexpected {
                length,
                kinds: 1u64 << kind,
            }),
            // 嵌套太深：把括号当普通文本，保证递归深度有界。
            Token::Open { length, .. } if depth >= MAX_DEPTH => Node::Leaf(Leaf::Text(length)),
            Token::Open { length, kind } => {
                let open = Leaf::Bracket { length, kind };
                let child = self.parse_list(opened | (1u64 << kind), depth + 1);
                let close = match self.tokenizer.peek() {
                    Some(Token::Close { kind: k, .. }) if k == kind => {
                        match self.tokenizer.read() {
                            Some(Token::Close { length, kind }) => {
                                Some(Leaf::Bracket { length, kind })
                            }
                            _ => unreachable!("刚 peek 到右括号"),
                        }
                    }
                    _ => None,
                };
                make_pair(open, child, close)
            }
        }
    }

    /// 试着从旧树里原样取一棵子树。
    fn read_from_cache(&mut self, opened: KindSet) -> Option<Node> {
        let reader = self.reader.as_mut()?;
        let offset = self.tokenizer.offset();
        // 复用的子树必须整段落在下一处改动之前，否则就把改动跳过去了。
        let max = self.mapper.distance_to_change(offset);
        if max.is_some_and(|m| m.is_zero()) {
            return None;
        }
        let old_offset = self.mapper.to_old(offset);
        let node = reader.read_longest_at(old_offset, &mut |node: &Node| {
            if let Some(max) = max {
                if node.length() >= max {
                    return false;
                }
            }
            node.can_be_reused(opened)
        })?;
        if node.length().is_zero() {
            // 零长度节点复用不会推进 token 流，会死循环。
            return None;
        }
        self.stats.reused += 1;
        self.tokenizer.skip(node.length());
        Some(node)
    }
}

/// 把括号种类表里能参与匹配的种类做成位集合。
pub fn matchable_set(kinds: &[super::tokenizer::BracketKind]) -> KindSet {
    let mut set = 0;
    for (i, k) in kinds.iter().enumerate().take(MAX_KINDS) {
        if k.matchable {
            set |= 1u64 << i;
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_mapper_maps_back_and_reports_barriers() {
        // 第 3 行（旧）被换成新文档的第 3–4 行：之后的行号 −1。
        let mapper = PositionMapper::new(vec![LineEdit {
            old: 3..4,
            new: 3..5,
        }]);
        assert_eq!(mapper.to_old(Length::new(2, 7)), Length::new(2, 7));
        assert_eq!(mapper.to_old(Length::new(5, 0)), Length::new(4, 0));
        assert_eq!(
            mapper.distance_to_change(Length::new(1, 2)),
            Some(Length::new(2, 0))
        );
        assert_eq!(
            mapper.distance_to_change(Length::new(3, 0)),
            Some(Length::ZERO)
        );
        assert_eq!(mapper.distance_to_change(Length::new(9, 0)), None);
    }

    #[test]
    fn pure_deletion_still_blocks_one_line() {
        // 旧的第 2–4 行被删掉：新文档里接缝落在第 2 行上，那一行必须算改动。
        let mapper = PositionMapper::new(vec![LineEdit {
            old: 2..4,
            new: 2..2,
        }]);
        assert_eq!(
            mapper.distance_to_change(Length::new(2, 0)),
            Some(Length::ZERO)
        );
        assert_eq!(mapper.to_old(Length::new(3, 0)), Length::new(5, 0));
    }
}
