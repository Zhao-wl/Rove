//! 粘性滚动（#55）：把视口首行所在的命名空间、类型、方法声明行钉在编辑器顶部。
//!
//! 这里只有**纯计算**（不依赖 gpui）：按视口首行求出作用域链，取各级的声明行。三档模型照搬 VS Code
//! （`editor.stickyScroll.defaultModel`）：大纲模型 → 折叠提供方模型 → 缩进模型，顺序在任何情况下都一致，
//! 上一档没有数据时自动回退到下一档。
//!
//! 性能（`PROJECT_CHARTER` §6：每帧 < 1 ms，粘性滚动只能占 < 100 µs）：滚动时视口首行每帧都在变，
//! 但作用域链大多数帧不变。[`StickyScroll::update`] 因此缓存「这条链在哪个首行区间内有效」
//! （[`StickyResult::valid`]），首行仍落在区间里时直接复用，不再查找；越界时才重算，而重算本身是
//! 「按位置二分查找 + 复用上一次路径」（大纲模型用 [`DocumentSymbols::path_from`]，与面包屑同一做法）。

use super::folding::FoldRange;
use rove_base::{Line, Position, Utf8Column};
use rove_syntax::symbols::{DocumentSymbols, SymbolId};
use std::ops::Range;

/// `editor.stickyScroll.defaultModel` 的三个取值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickyModel {
    /// 大纲模型：符号树（#26）的符号路径。
    Outline,
    /// 折叠提供方模型：折叠区间（语法、`#region`、缩进）的区间链。
    FoldingProvider,
    /// 缩进模型：按缩进层级回溯。
    Indentation,
}

impl StickyModel {
    /// 设置值 → 模型（未知值按 VS Code 的默认 `outlineModel`）。
    pub fn parse(value: &str) -> Self {
        match value {
            "foldingProviderModel" => StickyModel::FoldingProvider,
            "indentationModel" => StickyModel::Indentation,
            _ => StickyModel::Outline,
        }
    }

    /// 从本档开始的回退顺序（三档的相对顺序任何情况下都不变）。
    fn fallbacks(self) -> &'static [StickyModel] {
        match self {
            StickyModel::Outline => &[
                StickyModel::Outline,
                StickyModel::FoldingProvider,
                StickyModel::Indentation,
            ],
            StickyModel::FoldingProvider => {
                &[StickyModel::FoldingProvider, StickyModel::Indentation]
            }
            StickyModel::Indentation => &[StickyModel::Indentation],
        }
    }
}

/// 一条粘性行：声明行与该作用域的末行（含）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StickyLine {
    /// 要钉在顶部的文档行（声明行）。
    pub line: u64,
    /// 作用域的末行（含），供命中与“作用域已结束”判定使用。
    pub end: u64,
}

/// 计算用的数据来源（帧内只读引用，不读文本）。
#[derive(Clone, Copy, Default)]
pub struct StickySources<'a> {
    /// #26 的符号树（没有时回退到下一档）。
    pub symbols: Option<&'a DocumentSymbols>,
    /// #20 的折叠区间（语法、`#region`、缩进、注释、手动），按 (首行升序, 末行降序) 排列。
    pub folds: &'a [FoldRange],
    /// 纯缩进区间（后台与折叠区间一起算出，`editor.folding` 关闭时也可用）。
    pub indent: &'a [FoldRange],
}

impl StickySources<'_> {
    fn has(&self, model: StickyModel) -> bool {
        match model {
            StickyModel::Outline => self.symbols.is_some_and(|s| !s.is_empty()),
            StickyModel::FoldingProvider => !self.folds.is_empty(),
            StickyModel::Indentation => !self.indent.is_empty(),
        }
    }

    /// 实际生效的那一档（按 [`StickyModel::fallbacks`] 顺序找第一个有数据的）。
    pub fn effective(&self, model: StickyModel) -> Option<StickyModel> {
        model.fallbacks().iter().copied().find(|m| self.has(*m))
    }
}

/// 一次计算的结果：粘性行 + 这条结果在哪个「视口首行」区间内保持不变。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickyResult {
    pub lines: Vec<StickyLine>,
    /// 有效的视口首行区间（半开）。首行仍在区间里时可以直接复用 `lines`。
    pub valid: Range<u64>,
}

/// 粘性滚动的状态：上一次的结果、缓存键与焦点 / 悬停。
#[derive(Clone, Debug, Default)]
pub struct StickyScroll {
    result: Option<StickyResult>,
    /// 上一次的符号路径（`path_from` 的起点）。
    hint: Vec<SymbolId>,
    key: Option<(StickyModel, usize)>,
    /// 实际生效的档（回退之后）。
    effective: Option<StickyModel>,
    /// 焦点在粘性区时的行下标（`stickyScrollFocused`）。
    focus: Option<usize>,
    /// 指针悬停的行下标。
    hover: Option<usize>,
}

impl StickyScroll {
    /// 数据来源变了（新符号、新折叠区间、新文本、换设置）：丢掉缓存。
    pub fn invalidate(&mut self) {
        self.result = None;
        self.hint.clear();
        self.effective = None;
    }

    /// 清空（关闭粘性滚动、没有文本）。
    pub fn clear(&mut self) {
        self.invalidate();
        self.key = None;
        self.focus = None;
        self.hover = None;
    }

    pub fn lines(&self) -> &[StickyLine] {
        self.result.as_ref().map_or(&[], |r| &r.lines)
    }

    /// 当前生效的模型（回退之后），没有数据时为 `None`。
    pub fn effective_model(&self) -> Option<StickyModel> {
        self.effective
    }

    /// 上一次结果的有效首行区间（测试与基准用）。
    pub fn valid_range(&self) -> Option<Range<u64>> {
        self.result.as_ref().map(|r| r.valid.clone())
    }

    pub fn focus(&self) -> Option<usize> {
        self.focus
    }

    pub fn hover(&self) -> Option<usize> {
        self.hover
    }

    /// 设置悬停行；有变化返回 `true`。
    pub fn set_hover(&mut self, index: Option<usize>) -> bool {
        let index = index.filter(|i| *i < self.lines().len());
        let changed = self.hover != index;
        self.hover = index;
        changed
    }

    /// 焦点进入粘性区（`editor.action.focusStickyScroll`）；没有粘性行时返回 `false`。
    pub fn focus_first(&mut self) -> bool {
        if self.lines().is_empty() {
            return false;
        }
        self.focus = Some(0);
        true
    }

    /// 上 / 下移动焦点（`selectPrevious/NextStickyScrollLine`）。
    pub fn move_focus(&mut self, down: bool) -> bool {
        let count = self.lines().len();
        let Some(current) = self.focus.filter(|i| *i < count) else {
            return false;
        };
        let next = if down {
            (current + 1).min(count.saturating_sub(1))
        } else {
            current.saturating_sub(1)
        };
        self.focus = Some(next);
        true
    }

    /// 当前聚焦的粘性行。
    pub fn focused_line(&self) -> Option<StickyLine> {
        self.lines().get(self.focus?).copied()
    }

    /// 焦点退回编辑器（`editor.action.selectEditor`、失焦、粘性行消失）。
    pub fn blur(&mut self) -> bool {
        self.focus.take().is_some()
    }

    /// 按视口首行更新粘性行。`first` 是**文档行**（粘性区下方的第一行），`max` 是
    /// `editor.stickyScroll.maxLineCount`。返回粘性行是否变化（变化时才需要重绘）。
    pub fn update(
        &mut self,
        first: u64,
        max: usize,
        model: StickyModel,
        sources: StickySources<'_>,
    ) -> bool {
        if self.key != Some((model, max)) {
            self.key = Some((model, max));
            self.invalidate();
        }
        if let Some(r) = &self.result {
            if r.valid.contains(&first) {
                return false;
            }
        }
        let effective = sources.effective(model);
        self.effective = effective;
        let next = match effective {
            Some(StickyModel::Outline) => outline_lines(
                sources.symbols.expect("outline model"),
                first,
                &mut self.hint,
            ),
            Some(StickyModel::FoldingProvider) => range_lines(sources.folds, first),
            Some(StickyModel::Indentation) => range_lines(sources.indent, first),
            None => StickyResult {
                lines: Vec::new(),
                valid: first..first.saturating_add(1),
            },
        };
        let next = truncate(next, max);
        let changed = self.result.as_ref().map(|r| &r.lines) != Some(&next.lines);
        self.result = Some(next);
        if changed {
            let count = self.lines().len();
            if count == 0 {
                self.focus = None;
                self.hover = None;
            } else {
                self.focus = self.focus.map(|i| i.min(count - 1));
                self.hover = self.hover.map(|i| i.min(count - 1));
            }
        }
        changed
    }
}

/// 超过上限时**保留最内层**的若干级（VS Code 同样如此）。有效区间不受裁剪影响：
/// 被裁掉的是更外层的作用域，它们的末行不会早于保留下来的。
fn truncate(mut result: StickyResult, max: usize) -> StickyResult {
    if result.lines.len() > max {
        let drop = result.lines.len() - max;
        result.lines.drain(..drop);
    }
    if max == 0 {
        result.lines.clear();
    }
    result
}

/// 大纲模型：符号路径（由外到内）里每一级的声明行。
fn outline_lines(symbols: &DocumentSymbols, first: u64, hint: &mut Vec<SymbolId>) -> StickyResult {
    let at = Position {
        line: Line(first),
        column: Utf8Column(0),
    };
    let path = symbols.path_from(hint, at);
    *hint = path.clone();
    let mut lines: Vec<StickyLine> = Vec::with_capacity(path.len());
    let mut lo = 0u64;
    let mut hi = u64::MAX;
    for id in &path {
        let Some(node) = symbols.node(*id) else {
            continue;
        };
        let scope_start = node.symbol.range.start().line.0;
        let scope_end = node.symbol.range.end().line.0;
        // 声明行：符号名所在的行（`selection` 落在 `range` 内，见 #1 的契约）。
        let decl = node.symbol.selection.start().line.0.max(scope_start);
        if decl < first {
            lines.push(StickyLine {
                line: decl,
                end: scope_end,
            });
            lo = lo.max(decl + 1);
            // 首行越过作用域末行后这一级就不再显示。
            hi = hi.min(scope_end.saturating_add(1));
        } else {
            // 声明行还在视口里（不显示）；首行再往下一行它就要出现了。
            hi = hi.min(decl.saturating_add(1));
        }
    }
    // 最内层符号的兄弟：首行退回上一个子符号、或进入下一个子符号时路径都会变。
    let children = symbols.children(path.last().copied());
    let start_line = |id: &SymbolId| {
        symbols
            .node(*id)
            .map_or(u64::MAX, |n| n.symbol.range.start().line.0)
    };
    let idx = children.partition_point(|id| start_line(id) <= first);
    if let Some(next) = children.get(idx) {
        hi = hi.min(start_line(next).saturating_add(1));
    }
    if let Some(prev) = idx.checked_sub(1).and_then(|i| children.get(i)) {
        // 这个兄弟不含首行（含就会在路径里），但首行再往上就会落进它。
        if let Some(node) = symbols.node(*prev) {
            lo = lo.max(node.symbol.range.end().line.0.saturating_add(1));
        }
    }
    // 首行退出最内层符号时路径也会变。
    if let Some(node) = path.last().and_then(|id| symbols.node(*id)) {
        lo = lo.max(node.symbol.range.start().line.0);
    }
    StickyResult {
        lines,
        valid: valid_range(lo, hi, first),
    }
}

/// 折叠提供方模型与缩进模型：包含首行的区间链的起始行。`ranges` 按 (首行升序, 末行降序) 排列。
fn range_lines(ranges: &[FoldRange], first: u64) -> StickyResult {
    let after = ranges.partition_point(|r| r.start <= first);
    let mut chain: Vec<StickyLine> = Vec::new();
    let mut lo = 0u64;
    let mut hi = u64::MAX;
    // 由内向外回溯：只有比已收集到的最外层更靠前的区间才可能是祖先。
    let mut outer = u64::MAX;
    let mut i = after;
    while i > 0 {
        i -= 1;
        let r = ranges[i];
        if r.start >= outer {
            continue;
        }
        if !r.contains(first) {
            // 不含首行的区间（已经滚过去了）：首行退回它的末行时结果就会变。
            lo = lo.max(r.end);
            continue;
        }
        outer = r.start;
        if r.start < first {
            chain.push(StickyLine {
                line: r.start,
                end: r.end.saturating_sub(1),
            });
            lo = lo.max(r.start + 1);
            hi = hi.min(r.end);
            // 最内层的若干级就够了：更外层的区间即使存在也会被 maxLineCount 裁掉。
            if chain.len() >= MAX_CHAIN {
                break;
            }
        } else {
            // 首行正好是区间首行：再往下一行它就要出现了。
            hi = hi.min(r.start.saturating_add(1));
        }
        if r.start == 0 {
            break;
        }
    }
    chain.reverse();
    if let Some(next) = ranges.get(after) {
        hi = hi.min(next.start.saturating_add(1));
    }
    StickyResult {
        lines: chain,
        valid: valid_range(lo, hi, first),
    }
}

/// 回溯时最多收集的层数：`editor.stickyScroll.maxLineCount` 的上限是 VS Code 的 10，
/// 多收集一级让“上限之外还有没有作用域”的判断不影响结果。
const MAX_CHAIN: usize = MAX_LINE_COUNT + 1;

/// `editor.stickyScroll.maxLineCount` 的上限（VS Code 的 `maximum`）。
pub const MAX_LINE_COUNT: usize = 10;

fn valid_range(lo: u64, hi: u64, first: u64) -> Range<u64> {
    let lo = lo.min(first);
    let hi = hi.max(first.saturating_add(1));
    lo..hi
}

#[cfg(test)]
mod tests {
    use super::super::folding::FoldSource;
    use super::*;
    use rove_base::{TextRange, Version};
    use rove_syntax::{Symbol, SymbolKind};
    use std::sync::Arc;

    fn at(line: u64, column: u64) -> Position {
        Position {
            line: Line(line),
            column: Utf8Column(column),
        }
    }

    fn range(a: (u64, u64), b: (u64, u64)) -> TextRange {
        TextRange::new(at(a.0, a.1), at(b.0, b.1)).unwrap()
    }

    fn symbol(name: &str, kind: SymbolKind, decl: u64, end: u64, children: Vec<Symbol>) -> Symbol {
        Symbol {
            name: name.into(),
            kind,
            range: range((decl, 0), (end, 1)),
            selection: range((decl, 6), (decl, 10)),
            children: children.into(),
        }
    }

    /// 命名空间 0..99 → 类型 2..98 → 两个方法 10..30、40..60。
    fn symbols() -> DocumentSymbols {
        let methods = vec![
            symbol("A", SymbolKind::Method, 10, 30, vec![]),
            symbol("B", SymbolKind::Method, 40, 60, vec![]),
        ];
        let class = symbol("C", SymbolKind::Class, 2, 98, methods);
        let ns = symbol("N", SymbolKind::Namespace, 0, 99, vec![class]);
        DocumentSymbols::new(Version(1), "csharp".into(), Arc::from([ns]))
    }

    fn folds(spans: &[(u64, u64)]) -> Vec<FoldRange> {
        let mut v: Vec<FoldRange> = spans
            .iter()
            .filter_map(|&(s, e)| FoldRange::new(s, e, FoldSource::Syntax))
            .collect();
        v.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        v
    }

    fn lines(result: &StickyResult) -> Vec<u64> {
        result.lines.iter().map(|l| l.line).collect()
    }

    #[test]
    fn outline_model_shows_namespace_type_then_method() {
        let symbols = symbols();
        let mut hint = Vec::new();
        // 方法体中间：命名空间 → 类型 → 方法。
        let r = outline_lines(&symbols, 20, &mut hint);
        assert_eq!(lines(&r), vec![0, 2, 10]);
        // 两个方法之间：只剩命名空间与类型。
        let r = outline_lines(&symbols, 35, &mut hint);
        assert_eq!(lines(&r), vec![0, 2]);
        // 第二个方法里。
        let r = outline_lines(&symbols, 50, &mut hint);
        assert_eq!(lines(&r), vec![0, 2, 40]);
        // 声明行本身可见时不进粘性区。
        let r = outline_lines(&symbols, 10, &mut hint);
        assert_eq!(lines(&r), vec![0, 2]);
        let r = outline_lines(&symbols, 0, &mut hint);
        assert!(r.lines.is_empty());
    }

    #[test]
    fn cached_path_is_reused_inside_its_line_range() {
        let symbols = symbols();
        let mut sticky = StickyScroll::default();
        let sources = StickySources {
            symbols: Some(&symbols),
            ..Default::default()
        };
        assert!(sticky.update(20, 5, StickyModel::Outline, sources));
        let valid = sticky.valid_range().unwrap();
        assert!(
            valid.contains(&20) && valid.contains(&30),
            "方法体内一直有效"
        );
        assert!(!valid.contains(&31), "越过方法末行要重算");
        // 区间内再更新不重算（结果不变）。
        for first in valid.clone() {
            assert!(!sticky.update(first, 5, StickyModel::Outline, sources));
        }
        assert_eq!(
            sticky.lines().iter().map(|l| l.line).collect::<Vec<_>>(),
            vec![0, 2, 10]
        );
        // 越界后换一条路径。
        assert!(sticky.update(50, 5, StickyModel::Outline, sources));
        assert_eq!(
            sticky.lines().iter().map(|l| l.line).collect::<Vec<_>>(),
            vec![0, 2, 40]
        );
    }

    #[test]
    fn valid_range_tracks_the_next_child() {
        let symbols = symbols();
        let mut hint = Vec::new();
        // 类型里、第一个方法之前：下一个子符号的首行 + 1 是上界。
        let r = outline_lines(&symbols, 5, &mut hint);
        assert_eq!(lines(&r), vec![0, 2]);
        assert!(r.valid.contains(&10));
        assert!(!r.valid.contains(&11));
    }

    #[test]
    fn max_line_count_keeps_the_innermost_levels() {
        let symbols = symbols();
        let mut sticky = StickyScroll::default();
        let sources = StickySources {
            symbols: Some(&symbols),
            ..Default::default()
        };
        for (max, expected) in [
            (1usize, vec![10u64]),
            (2, vec![2, 10]),
            (3, vec![0, 2, 10]),
            (5, vec![0, 2, 10]),
        ] {
            sticky.update(20, max, StickyModel::Outline, sources);
            assert_eq!(
                sticky.lines().iter().map(|l| l.line).collect::<Vec<_>>(),
                expected,
                "maxLineCount = {max}"
            );
        }
    }

    #[test]
    fn folding_and_indentation_models_use_range_chains() {
        let regions = folds(&[(3, 40), (8, 20), (25, 35)]);
        let r = range_lines(&regions, 12);
        assert_eq!(lines(&r), vec![3, 8]);
        assert!(r.valid.contains(&19) && !r.valid.contains(&20));
        let r = range_lines(&regions, 30);
        assert_eq!(lines(&r), vec![3, 25]);
        let r = range_lines(&regions, 2);
        assert!(r.lines.is_empty());
        // 区间首行本身可见时不进粘性区。
        let r = range_lines(&regions, 8);
        assert_eq!(lines(&r), vec![3]);
        assert!(!r.valid.contains(&9));
    }

    #[test]
    fn model_falls_back_in_a_fixed_order() {
        let symbols = symbols();
        let regions = folds(&[(3, 40)]);
        let indent = folds(&[(1, 50)]);
        let all = StickySources {
            symbols: Some(&symbols),
            folds: &regions,
            indent: &indent,
        };
        assert_eq!(
            all.effective(StickyModel::Outline),
            Some(StickyModel::Outline)
        );
        assert_eq!(
            all.effective(StickyModel::FoldingProvider),
            Some(StickyModel::FoldingProvider)
        );
        assert_eq!(
            all.effective(StickyModel::Indentation),
            Some(StickyModel::Indentation)
        );
        // 没有符号树：大纲模型退到折叠提供方模型。
        let no_symbols = StickySources {
            symbols: None,
            folds: &regions,
            indent: &indent,
        };
        assert_eq!(
            no_symbols.effective(StickyModel::Outline),
            Some(StickyModel::FoldingProvider)
        );
        // 也没有折叠区间：退到缩进模型。
        let only_indent = StickySources {
            symbols: None,
            folds: &[],
            indent: &indent,
        };
        assert_eq!(
            only_indent.effective(StickyModel::Outline),
            Some(StickyModel::Indentation)
        );
        assert_eq!(
            StickySources::default().effective(StickyModel::Outline),
            None
        );
    }

    #[test]
    fn focus_moves_within_the_band_and_clamps() {
        let symbols = symbols();
        let mut sticky = StickyScroll::default();
        let sources = StickySources {
            symbols: Some(&symbols),
            ..Default::default()
        };
        sticky.update(20, 5, StickyModel::Outline, sources);
        assert_eq!(sticky.focus(), None);
        assert!(sticky.focus_first());
        assert_eq!(sticky.focus(), Some(0));
        assert!(sticky.move_focus(true));
        assert!(sticky.move_focus(true));
        assert_eq!(sticky.focus(), Some(2));
        assert!(sticky.move_focus(true), "到底后停在最后一行");
        assert_eq!(sticky.focus(), Some(2));
        assert_eq!(sticky.focused_line().map(|l| l.line), Some(10));
        assert!(sticky.move_focus(false));
        assert_eq!(sticky.focus(), Some(1));
        // 粘性行变少时焦点被钳制。
        sticky.update(35, 5, StickyModel::Outline, sources);
        assert_eq!(sticky.focus(), Some(1));
        sticky.update(0, 5, StickyModel::Outline, sources);
        assert_eq!(sticky.focus(), None, "没有粘性行时焦点退回编辑器");
        assert!(!sticky.focus_first());
    }

    #[test]
    fn parses_the_three_setting_values() {
        assert_eq!(StickyModel::parse("outlineModel"), StickyModel::Outline);
        assert_eq!(
            StickyModel::parse("foldingProviderModel"),
            StickyModel::FoldingProvider
        );
        assert_eq!(
            StickyModel::parse("indentationModel"),
            StickyModel::Indentation
        );
        assert_eq!(StickyModel::parse("nonsense"), StickyModel::Outline);
    }
}
