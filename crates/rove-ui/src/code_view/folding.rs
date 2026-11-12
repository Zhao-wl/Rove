//! 折叠（#20）：折叠区间模型、折叠状态、折叠命令，以及“文档行 ↔ 屏幕行”的映射 [`FoldMap`]。
//!
//! - 区间来源：语法（`SyntaxSnapshot::folds`）、`#region` 标记、缩进（#12 的 `fold_ranges`）、块注释与
//!   手动区间（`editor.createFoldingRangeFromSelection`）。区间 `start` 是折叠后仍可见的首行，
//!   `end` 不含（与 `rove_syntax::Fold` 的 `LineRange` 一致），隐藏的是 `start + 1 .. end`。
//! - 折叠状态只记“哪些区间折叠了”（按 `(start, end)`），保存在标签的 `ViewState::folds`；区间在后台
//!   重新计算后按首行重新对应，找不到对应区间的折叠被丢弃（手动区间除外）。
//! - [`FoldMap`] 是独立的映射结构：软换行（R57）、粘性滚动（R54）、缩略图（R55）都基于它换算行号。
//!   所有查询 O(log 折叠数)。

use rove_base::{Line, LineRange};
use std::ops::Range;

/// 区间来源。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FoldSource {
    Syntax,
    Region,
    Indentation,
    Comment,
    Manual,
}

/// 一个可折叠区间：`start` 为首行（折叠后可见），`end` 不含。至少隐藏一行（`end >= start + 2`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldRange {
    pub start: u64,
    pub end: u64,
    pub source: FoldSource,
}

/// 首末行相同的区间按来源去重时的优先顺序（越具体越优先）。
fn rank(source: FoldSource) -> u8 {
    match source {
        FoldSource::Region => 0,
        FoldSource::Comment => 1,
        FoldSource::Manual => 2,
        FoldSource::Syntax => 3,
        FoldSource::Indentation => 4,
    }
}

/// 按 (首行升序, 末行降序, 来源优先) 排序并按 (首行, 末行) 去重。
fn sort_dedup(v: &mut Vec<FoldRange>) {
    v.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.end.cmp(&a.end))
            .then(rank(a.source).cmp(&rank(b.source)))
    });
    v.dedup_by_key(|r| (r.start, r.end));
}

impl FoldRange {
    pub fn new(start: u64, end: u64, source: FoldSource) -> Option<Self> {
        (end >= start + 2).then_some(Self { start, end, source })
    }
    /// 行是否在区间内（含首行）。
    pub fn contains(&self, line: u64) -> bool {
        self.start <= line && line < self.end
    }
    /// 行是否会被这个区间隐藏。
    pub fn hides(&self, line: u64) -> bool {
        self.start < line && line < self.end
    }
    fn encloses(&self, o: &FoldRange) -> bool {
        self.start <= o.start && o.end <= self.end && (self.start, self.end) != (o.start, o.end)
    }
    pub fn key(&self) -> (u64, u64) {
        (self.start, self.end)
    }
}

/// 文档行 ↔ 屏幕行（隐藏行被折叠掉之后的行号）的映射。
#[derive(Clone, Debug, PartialEq)]
pub struct FoldMap {
    /// 互不相交、按起点有序的隐藏行区间。
    hidden: Vec<Range<u64>>,
    /// `before[i]` = 前 i 个区间隐藏的行数（长度 = 区间数 + 1）。
    before: Vec<u64>,
    /// `visible_before[i]` = 第 i 个区间之前的可见行数（屏幕行 → 文档行的二分键）。
    visible_before: Vec<u64>,
}

impl Default for FoldMap {
    /// 没有折叠：恒等映射（`before` 必须有首项 0）。
    fn default() -> Self {
        Self::new([], 0)
    }
}

impl FoldMap {
    /// 由折叠区间构建（区间可以嵌套或重叠，会合并）；超出 `doc_lines` 的部分被截掉。
    pub fn new(collapsed: impl IntoIterator<Item = (u64, u64)>, doc_lines: u64) -> Self {
        let mut ranges: Vec<Range<u64>> = collapsed
            .into_iter()
            .map(|(s, e)| (s + 1)..e.min(doc_lines))
            .filter(|r| r.start < r.end)
            .collect();
        ranges.sort_by_key(|r| r.start);
        let mut hidden: Vec<Range<u64>> = Vec::with_capacity(ranges.len());
        for r in ranges {
            match hidden.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => hidden.push(r),
            }
        }
        let mut before = Vec::with_capacity(hidden.len() + 1);
        let mut sum = 0;
        before.push(0);
        for r in &hidden {
            sum += r.end - r.start;
            before.push(sum);
        }
        let visible_before = hidden
            .iter()
            .zip(&before)
            .map(|(r, b)| r.start - b)
            .collect();
        Self {
            hidden,
            before,
            visible_before,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty()
    }

    /// 隐藏的行数。
    pub fn hidden_lines(&self) -> u64 {
        *self.before.last().unwrap_or(&0)
    }

    pub fn hidden_ranges(&self) -> &[Range<u64>] {
        &self.hidden
    }

    /// 屏幕行数。
    pub fn view_count(&self, doc_lines: u64) -> u64 {
        doc_lines.saturating_sub(self.hidden_lines())
    }

    pub fn is_hidden(&self, doc: u64) -> bool {
        let i = self.hidden.partition_point(|r| r.end <= doc);
        self.hidden.get(i).is_some_and(|r| r.start <= doc)
    }

    /// 文档行 → 屏幕行；隐藏行映射到包含它的折叠的首行。
    pub fn to_view(&self, doc: u64) -> u64 {
        let i = self.hidden.partition_point(|r| r.end <= doc);
        match self.hidden.get(i) {
            Some(r) if r.start <= doc => (r.start - 1) - self.before[i],
            _ => doc - self.before[i],
        }
    }

    /// 屏幕行 → 文档行。
    pub fn to_doc(&self, view: u64) -> u64 {
        let i = self.visible_before.partition_point(|&v| v <= view);
        view + self.before[i]
    }

    /// 从 `doc` 起（含）的下一个可见行；没有则为 `None`。
    pub fn next_visible(&self, doc: u64, doc_lines: u64) -> Option<u64> {
        let i = self.hidden.partition_point(|r| r.end <= doc);
        let line = match self.hidden.get(i) {
            Some(r) if r.start <= doc => r.end,
            _ => doc,
        };
        (line < doc_lines).then_some(line)
    }

    /// 在 `doc` 处或之前的可见行（隐藏行回到折叠首行）。
    pub fn prev_visible(&self, doc: u64) -> u64 {
        let i = self.hidden.partition_point(|r| r.end <= doc);
        match self.hidden.get(i) {
            Some(r) if r.start <= doc => r.start - 1,
            _ => doc,
        }
    }
}

/// 折叠命令（VS Code 命令 ID，KEYBINDINGS.md 阅读一节）。
pub const FOLD_COMMANDS: &[&str] = &[
    "editor.fold",
    "editor.unfold",
    "editor.toggleFold",
    "editor.foldRecursively",
    "editor.unfoldRecursively",
    "editor.toggleFoldRecursively",
    "editor.foldAll",
    "editor.unfoldAll",
    "editor.foldLevel1",
    "editor.foldLevel2",
    "editor.foldLevel3",
    "editor.foldLevel4",
    "editor.foldLevel5",
    "editor.foldLevel6",
    "editor.foldLevel7",
    "editor.foldAllBlockComments",
    "editor.foldAllMarkerRegions",
    "editor.unfoldAllMarkerRegions",
    "editor.foldAllExcept",
    "editor.unfoldAllExcept",
    "editor.createFoldingRangeFromSelection",
    "editor.removeManualFoldingRanges",
];

/// 一个视图的折叠状态：可折叠区间 + 已折叠集合 + 映射。
#[derive(Clone, Debug, Default)]
pub struct Folding {
    /// 计算出的区间与手动区间，按 (start 升序, end 降序) 排列、按 (start, end) 去重。
    ranges: Vec<FoldRange>,
    computed: Vec<FoldRange>,
    manual: Vec<FoldRange>,
    /// 已折叠的 (start, end)，有序。
    collapsed: Vec<(u64, u64)>,
    /// 区间是否已经算出来（恢复的折叠在此之前原样保留）。
    ready: bool,
    doc_lines: u64,
    map: FoldMap,
    /// 状态每变一次加一（视图据此发事件、失效缓存）。
    pub generation: u64,
}

impl Folding {
    pub fn map(&self) -> &FoldMap {
        &self.map
    }
    pub fn ranges(&self) -> &[FoldRange] {
        &self.ranges
    }
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// 清空（换文档、关闭折叠）。
    pub fn clear(&mut self) {
        let had = !self.collapsed.is_empty() || !self.ranges.is_empty();
        self.ranges.clear();
        self.computed.clear();
        self.manual.clear();
        self.collapsed.clear();
        self.ready = false;
        self.rebuild();
        if had {
            self.generation += 1;
        }
    }

    pub fn set_doc_lines(&mut self, n: u64) {
        if n != self.doc_lines {
            self.doc_lines = n;
            self.rebuild();
        }
    }

    /// 后台算出新的区间后调用。已折叠的区间按首行对应到新区间。
    pub fn set_computed(&mut self, mut computed: Vec<FoldRange>, max_regions: usize) {
        computed.retain(|r| r.end >= r.start + 2);
        limit_regions(&mut computed, max_regions);
        self.computed = computed;
        self.ready = true;
        self.merge();
        let collapsed = std::mem::take(&mut self.collapsed);
        self.collapsed = self.rematch(collapsed);
        self.rebuild();
        self.generation += 1;
    }

    fn merge(&mut self) {
        let mut all: Vec<FoldRange> = self.computed.iter().chain(&self.manual).copied().collect();
        sort_dedup(&mut all);
        self.ranges = all;
    }

    fn rematch(&self, collapsed: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = collapsed
            .into_iter()
            .filter_map(|(s, e)| {
                let same: Vec<&FoldRange> = self.ranges.iter().filter(|r| r.start == s).collect();
                same.iter()
                    .find(|r| r.end == e)
                    .or(same.first())
                    .map(|r| r.key())
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// 恢复保存的折叠（标签切换）。区间尚未算出时原样保留，算出后再对应。
    pub fn restore(&mut self, folds: &[LineRange]) {
        let collapsed: Vec<(u64, u64)> = folds
            .iter()
            .map(|r| (r.start().0, r.end().0))
            .filter(|(s, e)| *e >= s + 2)
            .collect();
        let mut collapsed = if self.ready {
            self.rematch(collapsed)
        } else {
            collapsed
        };
        collapsed.sort_unstable();
        collapsed.dedup();
        if collapsed != self.collapsed {
            self.collapsed = collapsed;
            self.rebuild();
            self.generation += 1;
        }
    }

    /// 当前折叠（保存到标签的 `ViewState::folds`）。
    pub fn collapsed(&self) -> Vec<LineRange> {
        self.collapsed
            .iter()
            .filter_map(|&(s, e)| LineRange::new(Line(s), Line(e)).ok())
            .collect()
    }

    pub fn collapsed_keys(&self) -> &[(u64, u64)] {
        &self.collapsed
    }

    fn rebuild(&mut self) {
        self.map = FoldMap::new(self.collapsed.iter().copied(), self.doc_lines);
    }

    fn is_collapsed(&self, r: &FoldRange) -> bool {
        self.collapsed.binary_search(&r.key()).is_ok()
    }

    /// 首行为 `line` 的区间（最外层的那个）及其是否折叠。用于行号栏的折叠控件。
    pub fn header(&self, line: u64) -> Option<(FoldRange, bool)> {
        let i = self.ranges.partition_point(|r| r.start < line);
        let first = self.ranges.get(i).filter(|r| r.start == line)?;
        // 同一首行有多个区间时，任何一个折叠都算折叠。
        let collapsed = self.ranges[i..]
            .iter()
            .take_while(|r| r.start == line)
            .any(|r| self.is_collapsed(r));
        Some((*first, collapsed))
    }

    /// 包含 `line` 的区间下标，由外到内。
    fn containing(&self, line: u64) -> Vec<usize> {
        let end = self.ranges.partition_point(|r| r.start <= line);
        let mut v: Vec<usize> = (0..end)
            .filter(|&i| self.ranges[i].contains(line))
            .collect();
        v.sort_by(|&a, &b| {
            let (ra, rb) = (&self.ranges[a], &self.ranges[b]);
            (rb.end - rb.start)
                .cmp(&(ra.end - ra.start))
                .then(ra.start.cmp(&rb.start))
        });
        v
    }

    fn descendants(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let outer = self.ranges[i];
        (0..self.ranges.len()).filter(move |&j| j != i && outer.encloses(&self.ranges[j]))
    }

    /// 嵌套深度（最外层为 1）。
    fn level(&self, i: usize) -> usize {
        let r = self.ranges[i];
        1 + self.ranges.iter().filter(|o| o.encloses(&r)).count()
    }

    fn set(&mut self, keys: impl IntoIterator<Item = (u64, u64)>, collapse: bool) -> bool {
        let mut changed = false;
        for k in keys {
            match (self.collapsed.binary_search(&k), collapse) {
                (Err(at), true) => {
                    self.collapsed.insert(at, k);
                    changed = true;
                }
                (Ok(at), false) => {
                    self.collapsed.remove(at);
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            self.rebuild();
            self.generation += 1;
        }
        changed
    }

    /// 展开所有隐藏了 `line` 的折叠（光标或跳转目标落进折叠时）。
    pub fn reveal(&mut self, line: u64) -> bool {
        if !self.map.is_hidden(line) {
            return false;
        }
        let keys: Vec<(u64, u64)> = self
            .collapsed
            .iter()
            .copied()
            .filter(|&(s, e)| s < line && line < e)
            .collect();
        self.set(keys, false)
    }

    /// 切换首行为 `line` 的折叠（行号栏控件、点击占位标记）。
    pub fn toggle_header(&mut self, line: u64) -> bool {
        let Some((range, collapsed)) = self.header(line) else {
            return false;
        };
        if collapsed {
            let keys: Vec<(u64, u64)> = self
                .ranges
                .iter()
                .filter(|r| r.start == line)
                .map(|r| r.key())
                .collect();
            self.set(keys, false)
        } else {
            self.set([range.key()], true)
        }
    }

    /// 执行折叠命令。`lines` 为各选区的活动行，`selection` 为主选区的 (首行, 末行)。
    /// 返回是否识别了命令（无论状态有没有变化）。
    pub fn execute(&mut self, id: &str, lines: &[u64], selection: (u64, u64)) -> bool {
        let level = id
            .strip_prefix("editor.foldLevel")
            .and_then(|n| n.parse::<usize>().ok());
        let all: Vec<usize> = (0..self.ranges.len()).collect();
        let keys = |s: &Self, v: &[usize]| -> Vec<(u64, u64)> {
            v.iter().map(|&i| s.ranges[i].key()).collect()
        };
        // 包含任一选区行的区间（及其祖先）不受“除外”类命令影响。
        let protected = |s: &Self| -> Vec<usize> {
            let mut v: Vec<usize> = lines.iter().flat_map(|&l| s.containing(l)).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        match id {
            "editor.fold" => {
                let mut k = Vec::new();
                for &l in lines {
                    // 由内向外找第一个还没折叠的区间（连按逐级向外折叠）。
                    if let Some(&i) = self
                        .containing(l)
                        .iter()
                        .rev()
                        .find(|&&i| !self.is_collapsed(&self.ranges[i]))
                    {
                        k.push(self.ranges[i].key());
                    }
                }
                self.set(k, true);
            }
            "editor.unfold" => {
                let mut k = Vec::new();
                for &l in lines {
                    // 可见的折叠是最外层的那个。
                    if let Some(&i) = self
                        .containing(l)
                        .iter()
                        .find(|&&i| self.is_collapsed(&self.ranges[i]))
                    {
                        k.push(self.ranges[i].key());
                    }
                }
                self.set(k, false);
            }
            "editor.toggleFold" => {
                for &l in lines {
                    let c = self.containing(l);
                    if let Some(&i) = c.iter().find(|&&i| self.is_collapsed(&self.ranges[i])) {
                        let key = self.ranges[i].key();
                        self.set([key], false);
                    } else if let Some(&i) = c.last() {
                        let key = self.ranges[i].key();
                        self.set([key], true);
                    }
                }
            }
            "editor.foldRecursively"
            | "editor.unfoldRecursively"
            | "editor.toggleFoldRecursively" => {
                for &l in lines {
                    let c = self.containing(l);
                    // 递归命令作用于包含该行的最内层区间及其全部子区间。
                    let Some(&i) = (if id == "editor.foldRecursively" {
                        c.iter()
                            .rev()
                            .find(|&&i| !self.is_collapsed(&self.ranges[i]))
                            .or(c.last())
                    } else {
                        c.iter()
                            .find(|&&i| self.is_collapsed(&self.ranges[i]))
                            .or(c.last())
                    }) else {
                        continue;
                    };
                    let collapse = match id {
                        "editor.foldRecursively" => true,
                        "editor.unfoldRecursively" => false,
                        _ => !self.is_collapsed(&self.ranges[i]),
                    };
                    let mut v: Vec<usize> = self.descendants(i).collect();
                    v.push(i);
                    let k = keys(self, &v);
                    self.set(k, collapse);
                }
            }
            "editor.foldAll" => {
                let k = keys(self, &all);
                self.set(k, true);
            }
            "editor.unfoldAll" => {
                let k = self.collapsed.clone();
                self.set(k, false);
            }
            _ if level.is_some_and(|n| (1..=7).contains(&n)) => {
                let n = level.unwrap();
                let keep = protected(self);
                let v: Vec<usize> = all
                    .iter()
                    .copied()
                    .filter(|&i| self.level(i) == n && !keep.contains(&i))
                    .collect();
                let k = keys(self, &v);
                self.set(k, true);
            }
            "editor.foldAllBlockComments" | "editor.foldAllMarkerRegions" => {
                let source = if id == "editor.foldAllBlockComments" {
                    FoldSource::Comment
                } else {
                    FoldSource::Region
                };
                let keep = protected(self);
                let v: Vec<usize> = all
                    .iter()
                    .copied()
                    .filter(|&i| self.ranges[i].source == source && !keep.contains(&i))
                    .collect();
                let k = keys(self, &v);
                self.set(k, true);
            }
            "editor.unfoldAllMarkerRegions" => {
                let v: Vec<usize> = all
                    .iter()
                    .copied()
                    .filter(|&i| self.ranges[i].source == FoldSource::Region)
                    .collect();
                let k = keys(self, &v);
                self.set(k, false);
            }
            "editor.foldAllExcept" | "editor.unfoldAllExcept" => {
                let keep = protected(self);
                let v: Vec<usize> = all.iter().copied().filter(|i| !keep.contains(i)).collect();
                let k = keys(self, &v);
                self.set(k, id == "editor.foldAllExcept");
            }
            "editor.createFoldingRangeFromSelection" => {
                let (a, b) = selection;
                // 单行选区按 VS Code：从该行到下一行之前没有可隐藏的行，不创建。
                if let Some(r) = FoldRange::new(a, b + 1, FoldSource::Manual) {
                    self.manual.retain(|m| m.key() != r.key());
                    self.manual.push(r);
                    self.merge();
                    // 新区间立即折叠，其中原有的折叠并入。
                    let inner: Vec<(u64, u64)> = self
                        .collapsed
                        .iter()
                        .copied()
                        .filter(|&(s, e)| r.start <= s && e <= r.end)
                        .collect();
                    self.set(inner, false);
                    self.set([r.key()], true);
                    self.generation += 1;
                }
            }
            "editor.removeManualFoldingRanges" => {
                let (a, b) = selection;
                let before = self.manual.len();
                let removed: Vec<(u64, u64)> = self
                    .manual
                    .iter()
                    .filter(|m| m.start <= b && a < m.end)
                    .map(|m| m.key())
                    .collect();
                self.manual.retain(|m| !(m.start <= b && a < m.end));
                if self.manual.len() != before {
                    self.merge();
                    let still: Vec<(u64, u64)> = removed
                        .into_iter()
                        .filter(|k| !self.ranges.iter().any(|r| r.key() == *k))
                        .collect();
                    self.set(still, false);
                    self.generation += 1;
                }
            }
            _ => return false,
        }
        true
    }
}

/// `editor.foldingMaximumRegions`：保留层级浅的区间（VS Code 同样先丢最深的）。
fn limit_regions(ranges: &mut Vec<FoldRange>, max: usize) {
    if ranges.len() <= max {
        return;
    }
    ranges.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut depth = vec![0usize; ranges.len()];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..ranges.len() {
        while let Some(&top) = stack.last() {
            if ranges[top].end >= ranges[i].end && ranges[top].start <= ranges[i].start {
                break;
            }
            stack.pop();
        }
        depth[i] = stack.len();
        stack.push(i);
    }
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| (depth[i], ranges[i].start));
    let mut keep: Vec<usize> = order.into_iter().take(max).collect();
    keep.sort_unstable();
    *ranges = keep.into_iter().map(|i| ranges[i]).collect();
}

/// 块注释区间（`/* … */` 等跨行注释）：按语言配置的块注释符号逐行扫描。字符串内的符号不排除，
/// 与语法级区间一样只是“可折叠的候选”。
pub fn comment_ranges(text: &str, open: &str, close: &str) -> Vec<FoldRange> {
    let mut out = Vec::new();
    if open.is_empty() || close.is_empty() {
        return out;
    }
    let mut start: Option<u64> = None;
    for (n, line) in text.split('\n').enumerate() {
        let n = n as u64;
        let mut rest = line;
        loop {
            match start {
                None => match rest.find(open) {
                    Some(i) => {
                        start = Some(n);
                        rest = &rest[i + open.len()..];
                    }
                    None => break,
                },
                Some(s) => match rest.find(close) {
                    Some(i) => {
                        if let Some(r) = FoldRange::new(s, n + 1, FoldSource::Comment) {
                            out.push(r);
                        }
                        start = None;
                        rest = &rest[i + close.len()..];
                    }
                    None => break,
                },
            }
        }
    }
    out
}

/// `editor.foldingStrategy`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldingStrategy {
    Auto,
    Indentation,
}

/// 由各来源合成区间（后台线程调用）。`syntax_allowed` 为假（大文件）时只用缩进与标记。
pub fn compute_ranges(
    text: &str,
    language: Option<&str>,
    syntax: &[rove_syntax::Fold],
    strategy: FoldingStrategy,
    tab_size: usize,
    syntax_allowed: bool,
) -> Vec<FoldRange> {
    use rove_syntax::language_config::BuiltinLanguages;
    let config = language.and_then(|l| BuiltinLanguages::shared().0.compiled(l));
    let indent_and_markers = match config {
        Some(c) => c.fold_ranges(text, tab_size),
        None => rove_syntax::language_config::fold_ranges(None, false, text, tab_size),
    }
    .unwrap_or_default();
    let convert = |f: &rove_syntax::Fold| {
        let source = match f.kind {
            rove_syntax::FoldKind::Syntax => FoldSource::Syntax,
            rove_syntax::FoldKind::Region => FoldSource::Region,
            rove_syntax::FoldKind::Indentation => FoldSource::Indentation,
        };
        FoldRange::new(f.lines.start().0, f.lines.end().0, source)
    };
    let use_syntax = syntax_allowed && strategy == FoldingStrategy::Auto && !syntax.is_empty();
    let mut out: Vec<FoldRange> = if use_syntax {
        // 有语法区间时按 VS Code 的 auto：语法区间为主，标记区间照常可用。
        syntax
            .iter()
            .filter_map(convert)
            .chain(
                indent_and_markers
                    .iter()
                    .filter(|f| matches!(f.kind, rove_syntax::FoldKind::Region))
                    .filter_map(convert),
            )
            .collect()
    } else {
        indent_and_markers.iter().filter_map(convert).collect()
    };
    if syntax_allowed {
        if let Some(pair) = config.and_then(|c| c.data().block_comment.clone()) {
            out.extend(comment_ranges(text, &pair.open, &pair.close));
        }
    }
    sort_dedup(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: u64, e: u64) -> FoldRange {
        FoldRange::new(s, e, FoldSource::Indentation).unwrap()
    }

    #[test]
    fn fold_map_round_trips_and_merges_nested() {
        // 折叠 2..6（隐藏 3,4,5）与嵌套的 3..5，以及 8..10（隐藏 9）。
        let m = FoldMap::new([(2, 6), (3, 5), (8, 10)], 20);
        assert_eq!(m.hidden_ranges(), &[3..6, 9..10]);
        assert_eq!(m.view_count(20), 16);
        let visible: Vec<u64> = (0..20).filter(|l| !m.is_hidden(*l)).collect();
        for (v, d) in visible.iter().enumerate() {
            assert_eq!(m.to_doc(v as u64), *d);
            assert_eq!(m.to_view(*d), v as u64);
        }
        assert_eq!(m.to_view(4), m.to_view(2), "隐藏行映射到折叠首行");
        assert_eq!(m.next_visible(4, 20), Some(6));
        assert_eq!(m.prev_visible(9), 8);
        assert_eq!(FoldMap::new([(18, 30)], 20).view_count(20), 19);
        let identity = FoldMap::default();
        assert_eq!((identity.to_doc(7), identity.to_view(7)), (7, 7));
    }

    #[test]
    fn restored_folds_survive_until_ranges_arrive_and_rematch_by_start() {
        let mut f = Folding::default();
        f.set_doc_lines(30);
        f.restore(&[LineRange::new(Line(2), Line(9)).unwrap()]);
        assert!(f.map().is_hidden(5), "区间算出前恢复的折叠照样生效");
        // 文本变了：同一首行的区间变成 2..7，折叠跟过去；首行不存在的折叠丢弃。
        f.set_computed(vec![r(2, 7), r(10, 12)], 5000);
        assert_eq!(f.collapsed_keys(), &[(2, 7)]);
        f.restore(&[LineRange::new(Line(4), Line(6)).unwrap()]);
        assert!(f.collapsed_keys().is_empty());
    }

    #[test]
    fn maximum_regions_keeps_shallow_ones() {
        let mut v = vec![r(0, 10), r(1, 5), r(2, 4), r(6, 9)];
        limit_regions(&mut v, 2);
        assert_eq!(v, vec![r(0, 10), r(1, 5)]);
    }

    #[test]
    fn comment_ranges_need_two_lines() {
        let t = "a /* x */\n/* one\n two\n three */ b\n/* c */";
        assert_eq!(
            comment_ranges(t, "/*", "*/"),
            vec![FoldRange::new(1, 4, FoldSource::Comment).unwrap()]
        );
    }
}
