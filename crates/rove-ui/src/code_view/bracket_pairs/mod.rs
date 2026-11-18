//! 括号对树、括号对着色与括号对参考线（#57）。
//!
//! # 为什么要一棵树
//!
//! 括号对着色默认开启（`editor.bracketPairColorization.enabled`），而且是**全文**范围的计算。
//! 做成「每次编辑全文重扫」在 5,000 行文件上还能忍，在 30 万行文件上会直接卡住。
//! 这里移植 VS Code `src/vs/editor/common/model/bracketPairsTextModelPart/` 的做法：维护一棵
//! **平衡的括号对树**，节点只存长度不存绝对偏移，一次编辑只重建覆盖编辑位置的那条根到叶路径。
//!
//! | 本模块 | 对应的 VS Code 文件 |
//! |---|---|
//! | [`length`] | `bracketPairsTree/length.ts` |
//! | [`ast`] | `bracketPairsTree/ast.ts` + `concat23Trees.ts` |
//! | [`tokenizer`] | `bracketPairsTree/tokenizer.ts` |
//! | [`parser`] | `bracketPairsTree/parser.ts` + `nodeReader.ts` + `beforeEditPositionMapper.ts` |
//! | [`query`] | `bracketPairsTree/brackets.ts` 的 `getBracketsInRange` 与 `matchBracket`，以及参考线用的 `getBracketPairsInRange` |
//!
//! 复杂度：首次解析 O(n)（n 为括号数，自底向上建 2-3 树）；一次编辑 O(改动涉及的 token 数 + log n)；
//! 按行查询 O(log n + 该行括号数)。实测数据见 `xtask/benches/brackets.rs` 与 PR 说明。
//!
//! # 数据来源
//!
//! - 括号定义来自 #12 的 `colorizedBracketPairs`，未定义时回落到 `brackets`；
//!   用户可用 `editor.language.colorizedBracketPairs` / `editor.language.brackets` 覆盖。
//! - **注释与字符串里的括号不参与配对**，由 #9 的 tree-sitter token（scope）判定，
//!   不自己扫引号与注释起止（C# 的逐字字符串、XML 的 CDATA 都靠这一条）。
//! - 着色以行内样式区间的形式经 #19 的装饰接口提交，优先级高于 token 前景色
//!   （[`priority::BRACKET_PAIR`](crate::code_view::decorations::priority::BRACKET_PAIR)），
//!   按行缓存、按行失效。
//! - **括号对参考线**（`editor.guides.bracketPairs` / `editor.guides.bracketPairsHorizontal`）
//!   用同一棵树与同一套深度计数（[`depth_index`]），逐行几何在
//!   [`code_view::guides`](crate::code_view::guides)，绘制在 `element.rs`。
//!
//! # 关闭条件
//!
//! - 大文件与超大文件关闭着色（`UX_SPEC` §3.1），原因登记到 #20 的「当前关闭了哪些功能」接口；
//! - 括号数超过 [`DEFAULT_MAX_BRACKETS`]（`rove.bracketPairColorization.maximumBrackets`，
//!   与 `editor.foldingMaximumRegions` 同量级）时**整体关闭着色**，而不是画错。

pub mod ast;
pub mod length;
pub mod parser;
pub mod query;
pub mod tokenizer;

#[cfg(test)]
mod tests;

use super::decorations::{InlineDecoration, TextStyle};
use ast::{KindSet, Node};
use length::Length;
use parser::{LineEdit, ParseStats};
use query::BracketHit;
use rove_theme::Color;
use std::ops::Range;

pub use query::{active_pair, brackets_in_line};
pub use tokenizer::{BracketKind, BracketSource, TextDocument};

/// `rove.bracketPairColorization.maximumBrackets` 的默认值。
///
/// 与 `editor.foldingMaximumRegions`（默认 5,000、上限 65,000）同量级：超过之后括号对树的
/// 内存与解析耗时不再值得，整体关闭着色而不是画错。
pub const DEFAULT_MAX_BRACKETS: usize = 50_000;

/// 深度颜色一共 6 个，第 7 层回到第 1 层（与 VS Code 一致，第一层用 `foreground1`）。
pub const DEPTH_COLORS: usize = 6;

/// 深度颜色的颜色键。
pub const FOREGROUND_KEYS: [&str; DEPTH_COLORS] = [
    "editorBracketHighlight.foreground1",
    "editorBracketHighlight.foreground2",
    "editorBracketHighlight.foreground3",
    "editorBracketHighlight.foreground4",
    "editorBracketHighlight.foreground5",
    "editorBracketHighlight.foreground6",
];

/// 未配对括号的颜色键。
pub const UNEXPECTED_KEY: &str = "editorBracketHighlight.unexpectedBracket.foreground";

/// #57 第二部分：括号对参考线的颜色键（非活动）。
pub const GUIDE_KEYS: [&str; DEPTH_COLORS] = [
    "editorBracketPairGuide.background1",
    "editorBracketPairGuide.background2",
    "editorBracketPairGuide.background3",
    "editorBracketPairGuide.background4",
    "editorBracketPairGuide.background5",
    "editorBracketPairGuide.background6",
];

/// 活动括号对参考线的颜色键。
pub const ACTIVE_GUIDE_KEYS: [&str; DEPTH_COLORS] = [
    "editorBracketPairGuide.activeBackground1",
    "editorBracketPairGuide.activeBackground2",
    "editorBracketPairGuide.activeBackground3",
    "editorBracketPairGuide.activeBackground4",
    "editorBracketPairGuide.activeBackground5",
    "editorBracketPairGuide.activeBackground6",
];

/// 活动缩进参考线的颜色键。
pub const ACTIVE_INDENT_KEYS: [&str; DEPTH_COLORS] = [
    "editorIndentGuide.activeBackground1",
    "editorIndentGuide.activeBackground2",
    "editorIndentGuide.activeBackground3",
    "editorIndentGuide.activeBackground4",
    "editorIndentGuide.activeBackground5",
    "editorIndentGuide.activeBackground6",
];

/// 根据 #12 的语言配置与用户覆盖算出当前生效的括号定义。
///
/// - `brackets` 决定哪些括号参与**匹配**（#20 的 `editor.matchBrackets`）；
/// - `colorizedBracketPairs` 决定哪些括号参与**着色**，未定义时回落到 `brackets`
///   （YAML 就是这种情况；Markdown 与 XML 定义成空表，所以一个都不着色）；
/// - 两者取并集建同一棵树：C# 的 `colorizedBracketPairs` 比 `brackets` 多一对 `<>`，
///   XML 的 `brackets` 比 `colorizedBracketPairs` 多出注释标记。
/// - `editor.language.brackets` / `editor.language.colorizedBracketPairs` 覆盖对应的一项。
pub fn kinds_for(
    data: Option<&rove_syntax::LanguageConfiguration>,
    brackets_override: Option<&[(std::sync::Arc<str>, std::sync::Arc<str>)]>,
    colorized_override: Option<&[(std::sync::Arc<str>, std::sync::Arc<str>)]>,
) -> Vec<BracketKind> {
    type Pairs = Vec<(std::sync::Arc<str>, std::sync::Arc<str>)>;
    let to_pairs = |list: &[rove_syntax::Pair]| -> Pairs {
        list.iter()
            .map(|p| (p.open.clone(), p.close.clone()))
            .collect()
    };
    let brackets: Pairs = match brackets_override {
        Some(list) => list.to_vec(),
        None => data.map(|d| to_pairs(&d.brackets)).unwrap_or_default(),
    };
    let colorized: Pairs = match colorized_override {
        Some(list) => list.to_vec(),
        None => data
            .and_then(|d| d.colorized_brackets.as_deref().map(to_pairs))
            .unwrap_or_else(|| brackets.clone()),
    };
    let mut kinds: Vec<BracketKind> = Vec::with_capacity(brackets.len() + colorized.len());
    for (open, close) in brackets {
        if open.is_empty() || close.is_empty() {
            continue;
        }
        if kinds.iter().any(|k| k.open == open && k.close == close) {
            continue;
        }
        kinds.push(BracketKind {
            open,
            close,
            colorized: false,
            matchable: true,
        });
    }
    for (open, close) in colorized {
        if open.is_empty() || close.is_empty() {
            continue;
        }
        match kinds
            .iter_mut()
            .find(|k| k.open == open && k.close == close)
        {
            Some(k) => k.colorized = true,
            None => kinds.push(BracketKind {
                open,
                close,
                colorized: true,
                matchable: false,
            }),
        }
    }
    kinds
}

/// 一个括号该用哪个颜色键。`independent_pool` 对应
/// `editor.bracketPairColorization.independentColorPoolPerBracketType`。
pub fn color_key(hit: &BracketHit, independent_pool: bool) -> &'static str {
    if hit.unmatched {
        return UNEXPECTED_KEY;
    }
    FOREGROUND_KEYS[color_index(hit, independent_pool)]
}

fn color_index(hit: &BracketHit, independent_pool: bool) -> usize {
    depth_index(hit.level, hit.level_of_kind, independent_pool)
}

/// 深度 → 颜色下标（0 起，每 [`DEPTH_COLORS`] 层循环一次）。着色与参考线共用，保证颜色一致。
pub fn depth_index(level: u32, level_of_kind: u32, independent_pool: bool) -> usize {
    let level = if independent_pool {
        level_of_kind
    } else {
        level
    };
    (level.max(1) as usize - 1) % DEPTH_COLORS
}

/// 一条括号对参考线该用哪个颜色键（快照测试用；实际取色见 [`BracketColors`]）。
pub fn guide_color_key(
    level: u32,
    level_of_kind: u32,
    independent_pool: bool,
    active: bool,
) -> &'static str {
    let i = depth_index(level, level_of_kind, independent_pool);
    if active {
        ACTIVE_GUIDE_KEYS[i]
    } else {
        GUIDE_KEYS[i]
    }
}

/// 括号对着色的配色（由 #10 的主题解析出来，见 `colors.rs`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BracketColors {
    pub depth: [Option<Color>; DEPTH_COLORS],
    pub unexpected: Option<Color>,
    /// #57 第二部分：括号对参考线（非活动），`editorBracketPairGuide.background1`–`6`。
    pub guide: [Option<Color>; DEPTH_COLORS],
    /// 活动括号对参考线，`editorBracketPairGuide.activeBackground1`–`6`。
    pub active_guide: [Option<Color>; DEPTH_COLORS],
}

impl BracketColors {
    /// 一条参考线的颜色。
    pub fn guide_color(
        &self,
        level: u32,
        level_of_kind: u32,
        independent_pool: bool,
        active: bool,
    ) -> Option<Color> {
        let i = depth_index(level, level_of_kind, independent_pool);
        if active {
            self.active_guide[i]
        } else {
            self.guide[i]
        }
    }
}

/// 视图持有的括号对状态（提供方只读）。
pub struct BracketPairs {
    kinds: Vec<BracketKind>,
    symbols: tokenizer::Symbols,
    matchable: KindSet,
    tree: Option<Node>,
    /// 上一次解析对应的文档长度（供调试与测试）。
    length: Length,
    pending: Option<Vec<LineEdit>>,
    dirty: bool,
    needs_full: bool,
    /// 已经解析过一次（没有括号的文档 `tree` 也是 `None`，靠这个标志区分）。
    parsed: bool,
    /// 括号数超过上限：整体关闭着色，直到 [`BracketPairs::reset`]。
    over_limit: bool,
    stats: ParseStats,
    /// `editor.bracketPairColorization.enabled`。
    pub enabled: bool,
    /// 文档级别允许建树（普通文件；大文件与超大文件都关闭，见 `UX_SPEC` §3.1）。
    pub level_allows: bool,
    /// `editor.bracketPairColorization.independentColorPoolPerBracketType`。
    pub independent_pool: bool,
    /// `editor.guides.highlightActiveBracketPair`。
    pub highlight_active: bool,
    /// #57 第二部分：`editor.guides.bracketPairs` 或 `editor.guides.bracketPairsHorizontal`
    /// 任意一个不是 `false`——参考线也要用这棵树，着色关了也得建。
    pub guides: bool,
    pub limit: usize,
    pub colors: BracketColors,
}

impl Default for BracketPairs {
    fn default() -> Self {
        Self {
            kinds: Vec::new(),
            symbols: Vec::new(),
            matchable: 0,
            tree: None,
            length: Length::ZERO,
            pending: None,
            dirty: true,
            needs_full: true,
            parsed: false,
            over_limit: false,
            stats: ParseStats::default(),
            enabled: true,
            level_allows: true,
            independent_pool: false,
            highlight_active: true,
            guides: false,
            limit: DEFAULT_MAX_BRACKETS,
            colors: BracketColors::default(),
        }
    }
}

impl BracketPairs {
    /// 换语言（或用户改了 `editor.language.brackets` 等覆盖）：括号定义变了，整棵树重建。
    pub fn set_kinds(&mut self, kinds: Vec<BracketKind>) {
        if self.kinds == kinds {
            return;
        }
        self.symbols = tokenizer::symbols(&kinds);
        self.matchable = parser::matchable_set(&kinds);
        self.kinds = kinds;
        self.reset();
    }

    pub fn kinds(&self) -> &[BracketKind] {
        &self.kinds
    }

    /// 换文档：丢弃旧树，并重新判定括号数上限。
    pub fn reset(&mut self) {
        self.tree = None;
        self.pending = None;
        self.dirty = true;
        self.needs_full = true;
        self.parsed = false;
        self.over_limit = false;
        self.stats = ParseStats::default();
    }

    /// 整篇重新解析（主题 / 设置以外的原因，例如语法快照整体失效）。
    pub fn invalidate_all(&mut self) {
        self.pending = None;
        self.needs_full = true;
        self.dirty = true;
    }

    /// 记下一批行级变化（来自 #47 的编辑或 #9 的变化行集合）。
    ///
    /// 一次 [`BracketPairs::update`] 之前只接受一批：多批变化的坐标合成太容易出错，
    /// 直接退化成整篇重算（多事务批次很少见，单次输入永远是一批）。
    pub fn note_lines(&mut self, edits: Vec<LineEdit>) {
        self.dirty = true;
        if !self.parsed || self.needs_full {
            return;
        }
        match &mut self.pending {
            None => self.pending = Some(edits),
            Some(_) => {
                self.pending = None;
                self.needs_full = true;
            }
        }
    }

    /// 是否需要重新解析。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 括号数超过上限、整体关闭了着色。
    pub fn over_limit(&self) -> bool {
        self.over_limit
    }

    pub fn stats(&self) -> ParseStats {
        self.stats
    }

    pub fn tree(&self) -> Option<&Node> {
        self.tree.as_ref()
    }

    /// 当前是否需要维护这棵树（着色或活动括号对要用）。
    fn wanted(&self) -> bool {
        self.level_allows
            && (self.enabled || self.highlight_active || self.guides)
            && !self.symbols.is_empty()
            && !self.over_limit
    }

    /// 重新解析（必要时）。返回树是否发生了变化。
    pub fn update(&mut self, source: &dyn BracketSource) -> bool {
        if !self.wanted() {
            let had = self.parsed;
            self.tree = None;
            self.parsed = false;
            self.needs_full = true;
            self.pending = None;
            self.dirty = false;
            return had;
        }
        if !self.dirty && self.parsed {
            return false;
        }
        let old = if self.needs_full || !self.parsed {
            None
        } else {
            self.tree.clone()
        };
        let edits = if old.is_some() {
            self.pending.take().unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut tokenizer = tokenizer::Tokenizer::new(source, &self.symbols, self.limit);
        let (tree, stats) = parser::parse(&mut tokenizer, old, edits);
        self.dirty = false;
        self.needs_full = false;
        self.pending = None;
        self.stats = stats;
        if tokenizer.over_limit {
            // 画错不如不画：整体关闭着色，并在「当前关闭了哪些功能」里登记。
            self.over_limit = true;
            self.tree = None;
            self.parsed = false;
            return true;
        }
        self.length = tokenizer.document_length();
        self.tree = tree;
        self.parsed = true;
        true
    }

    /// 某一行上的括号及其深度（快照测试与集成测试用）。
    pub fn brackets_in_line(&self, line: u64) -> Vec<BracketHit> {
        match &self.tree {
            Some(tree) => query::brackets_in_line(tree, line),
            None => Vec::new(),
        }
    }

    /// 某一行上参与着色的括号及其颜色键（快照测试用；不受配色是否设了颜色影响，
    /// 但与 [`BracketPairs::colorize`] 一样受 `editor.bracketPairColorization.enabled` 与级别控制）。
    pub fn colored_keys(&self, line: u64) -> Vec<(Range<u32>, u32, &'static str)> {
        if !self.enabled {
            return Vec::new();
        }
        self.brackets_in_line(line)
            .into_iter()
            .filter(|hit| self.is_colorized(hit.kind))
            .map(|hit| {
                let key = color_key(&hit, self.independent_pool);
                (hit.range, hit.level, key)
            })
            .collect()
    }

    fn is_colorized(&self, kind: u8) -> bool {
        self.kinds.get(kind as usize).is_some_and(|k| k.colorized)
    }

    /// 着色：把一行的括号样式区间写进 `out`。
    pub fn colorize(&self, line: u64, out: &mut Vec<InlineDecoration>) {
        if !self.enabled {
            return;
        }
        let Some(tree) = &self.tree else { return };
        for hit in query::brackets_in_line(tree, line) {
            if !self.is_colorized(hit.kind) {
                continue;
            }
            let color = if hit.unmatched {
                self.colors.unexpected
            } else {
                self.colors.depth[color_index(&hit, self.independent_pool)]
            };
            // VS Code 的 `foreground4`–`foreground6` 默认是 `#00000000`（完全透明），
            // 含义是「不改前景色」，不是「画成透明」。
            let Some(color) = color.filter(|c| c.3 != 0) else {
                continue;
            };
            out.push(InlineDecoration {
                range: hit.range,
                style: TextStyle {
                    foreground: Some(color),
                    ..Default::default()
                },
            });
        }
    }

    /// 光标所在的活动括号对（`editor.guides.highlightActiveBracketPair`）。
    ///
    /// 返回 `None` 有两种情况：关闭了这个设置，或者树还没建好——后者由调用方回落到 #20 的
    /// 就近扫描（[`BracketPairs::has_tree`] 区分）。
    pub fn active_pair(
        &self,
        line: u64,
        column: u32,
        mode: query::MatchBrackets,
    ) -> Option<query::BracketMatch> {
        if !self.highlight_active {
            return None;
        }
        let tree = self.tree.as_ref()?;
        query::active_pair(tree, line, column, mode, self.matchable)
    }

    /// #57 第二部分：与 `lines` 行区间相交、配对完整的括号对（外层在前）。
    pub fn guide_pairs(&self, lines: Range<u64>) -> Vec<query::GuidePair> {
        match &self.tree {
            Some(tree) => query::guide_pairs(tree, lines),
            None => Vec::new(),
        }
    }

    /// 光标所在的活动括号对参考线（包含光标、配对完整且跨行的最内层括号对）。
    pub fn active_guide_pair(&self, line: u64, column: u32) -> Option<query::GuidePair> {
        let tree = self.tree.as_ref()?;
        query::active_guide_pair(tree, Length::new(line, column))
    }

    /// 树是否可用（可用时活动括号对一律以树为准，不再走就近扫描）。
    /// 文档里一个括号都没有时 `tree` 也是 `None`，但同样算「可用」。
    pub fn has_tree(&self) -> bool {
        self.parsed
    }
}
