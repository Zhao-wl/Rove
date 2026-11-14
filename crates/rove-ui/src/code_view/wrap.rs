//! 软换行（#58）：换行点计算与「视觉行 ↔ 缓冲行」映射。**不依赖 GPUI**，可单独测试。
//!
//! # 两个产出
//!
//! 1. [`wrap_line`]：按 `editor.wordWrap` / `wordWrapColumn` / `wrappingIndent` /
//!    `wrappingStrategy` / `wordBreak` / `wrapOnEscapedLineFeeds` 算出一条缓冲行的换行点
//!    （[`LineWrap`]）。输入是 [`DisplayLine`]（Tab 已展开、已按 `editor.stopRenderingLineAfter`
//!    截断），所以换行点永远落在字素簇边界上：ZWJ / 肤色修饰的 emoji、组合字符不会被切开。
//! 2. [`VisualMap`]：**一处**权威映射。折叠（[`FoldMap`]）与软换行合在同一个结构里，
//!    对外给出「缓冲行 ↔ 视觉行区间」「视觉行 → (缓冲行, 段号)」「视觉行总数」。
//!    软换行关闭时退化成恒等 + 折叠，不分配任何内存。
//!
//! # 增量
//!
//! [`VisualMap::set_count`] 按行更新视觉行数（Fenwick 树，O(log 行数)），返回这一行的行数有没有变；
//! 折叠变化后调 [`VisualMap::set_folds`]，批量改完调 [`VisualMap::commit`] 重建隐藏行的前缀和。
//! 视口宽度 / 字体 / 字号 / DPI 变化时用 [`WrapPending`] 把全部行排队，分帧重算（见
//! `view.rs` 的 `wrap_slice`），重算期间仍按旧映射绘制。

use super::columns::DisplayLine;
use super::folding::FoldMap;
use std::ops::Range;
use std::sync::Arc;

// ------------------------------------------------------------------ 设置

/// `editor.wordWrap`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WordWrap {
    #[default]
    Off,
    /// 按视口宽度换行。
    On,
    /// 按 `editor.wordWrapColumn` 换行。
    Column,
    /// 取视口宽度与 `editor.wordWrapColumn` 的较小者。
    Bounded,
}

/// `editor.wrappingIndent`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WrappingIndent {
    None,
    #[default]
    Same,
    Indent,
    DeepIndent,
}

/// `editor.wrappingStrategy`：`simple` 按等宽近似（可见列）切分，`advanced` 按真实字形宽度
/// （整形结果的字素簇 x 表）切分。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WrappingStrategy {
    #[default]
    Simple,
    Advanced,
}

/// 换行相关的设置快照（由 `EditorOptions` 读出）。
#[derive(Clone, Debug, PartialEq)]
pub struct WrapOptions {
    pub mode: WordWrap,
    /// `editor.wordWrapColumn`。
    pub column: u32,
    pub indent: WrappingIndent,
    pub strategy: WrappingStrategy,
    /// `editor.wordBreak` 为 `keepAll`：CJK 之间不断行，只在空白与分隔符处断。
    pub keep_all: bool,
    /// `editor.wrapOnEscapedLineFeeds`：文本里的 `\n` 字面量触发换行。
    pub escaped_line_feeds: bool,
    /// `editor.wordWrapIndicator`：在换行列处画指示标记。
    pub indicator: bool,
    /// `editor.wordSeparators`（与双击选词同一份规则，见 [`super::selection`]）。
    pub separators: Arc<str>,
    pub tab_size: u32,
}

impl Default for WrapOptions {
    fn default() -> Self {
        Self {
            mode: WordWrap::Off,
            column: 80,
            indent: WrappingIndent::Same,
            strategy: WrappingStrategy::Simple,
            keep_all: false,
            escaped_line_feeds: false,
            indicator: false,
            separators: "".into(),
            tab_size: 4,
        }
    }
}

impl WrapOptions {
    pub fn enabled(&self) -> bool {
        self.mode != WordWrap::Off
    }

    /// 可用宽度（像素）与半角字符宽 → 换行的列上限。`Off` 返回 `None`。
    pub fn columns(&self, viewport_width: f64, cell_width: f64) -> Option<u32> {
        let by_viewport = if cell_width > 0.0 {
            (viewport_width / cell_width).floor().max(1.0) as u32
        } else {
            u32::MAX
        };
        match self.mode {
            WordWrap::Off => None,
            WordWrap::On => Some(by_viewport),
            WordWrap::Column => Some(self.column.max(1)),
            WordWrap::Bounded => Some(by_viewport.min(self.column.max(1))),
        }
    }
}

// ------------------------------------------------------------------ 换行点

/// 一条缓冲行的换行结果：续行起点（源行 UTF-8 字节列，升序，不含 0）与续行缩进。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineWrap {
    breaks: Vec<u32>,
    indent_columns: u32,
}

/// 不换行的单段结果。
pub const UNWRAPPED: LineWrap = LineWrap {
    breaks: Vec::new(),
    indent_columns: 0,
};

impl LineWrap {
    pub fn new(breaks: Vec<u32>, indent_columns: u32) -> Self {
        Self {
            breaks,
            indent_columns,
        }
    }

    /// 视觉行数（至少 1）。
    pub fn count(&self) -> u32 {
        self.breaks.len() as u32 + 1
    }

    pub fn is_wrapped(&self) -> bool {
        !self.breaks.is_empty()
    }

    /// 续行起点（源字节列）。
    pub fn breaks(&self) -> &[u32] {
        &self.breaks
    }

    /// 第 `seg` 段的续行缩进（可见列数）；首段恒为 0。
    pub fn indent_columns(&self, seg: u32) -> u32 {
        if seg == 0 {
            0
        } else {
            self.indent_columns
        }
    }

    /// 源列落在第几段（续行起点算下一段）。
    pub fn segment_of(&self, column: u32) -> u32 {
        self.breaks.partition_point(|&b| b <= column) as u32
    }

    /// 第 `seg` 段覆盖的源列区间；`line_len` 是行的字节长度。
    pub fn segment_range(&self, seg: u32, line_len: u32) -> Range<u32> {
        let seg = seg.min(self.count() - 1) as usize;
        let start = if seg == 0 { 0 } else { self.breaks[seg - 1] };
        let end = self.breaks.get(seg).copied().unwrap_or(line_len);
        start..end.max(start)
    }
}

/// 换行时的度量方式：`simple` 用可见列，`advanced` 用整形结果的字素簇 x 表。
/// 两者的单位不同，但算法一致（[`wrap_line`] 只看“位置”与“上限”）。
#[derive(Clone, Copy, Debug)]
pub enum Metric<'a> {
    /// 按可见列切分；`limit` 是列上限。
    Columns { limit: u32 },
    /// 按像素切分；`xs` 与 [`DisplayLine::stops`] 一一对应，`limit` 是像素上限，
    /// `cell` 是半角字符宽（把缩进列换算成像素）。
    Pixels {
        xs: &'a [f32],
        limit: f32,
        cell: f32,
    },
}

impl Metric<'_> {
    /// 第 `i` 个字素簇边界的位置。
    fn at(&self, line: &DisplayLine, i: usize) -> f64 {
        match self {
            Metric::Columns { .. } => line.stops()[i].col as f64,
            Metric::Pixels { xs, .. } => DisplayLine::x_at(xs, i) as f64,
        }
    }
    fn limit(&self) -> f64 {
        match self {
            Metric::Columns { limit } => *limit as f64,
            Metric::Pixels { limit, .. } => *limit as f64,
        }
    }
    /// 可见列 → 本度量的单位。
    fn columns_to_unit(&self, columns: u32) -> f64 {
        match self {
            Metric::Columns { .. } => columns as f64,
            Metric::Pixels { cell, .. } => columns as f64 * *cell as f64,
        }
    }
    /// 一个半角字符的宽度（用于判断缩进是否挤掉了正文）。
    fn cell(&self) -> f64 {
        self.columns_to_unit(1)
    }
}

/// 字符在换行时的类别（与 [`super::selection`] 的单词规则同源）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Space,
    Separator,
    Ideographic,
    Regular,
}

fn kind(c: char, separators: &str) -> Kind {
    if c == ' ' || c == '\t' {
        Kind::Space
    } else if separators.contains(c) {
        Kind::Separator
    } else if super::columns::is_full_width(c) {
        Kind::Ideographic
    } else {
        Kind::Regular
    }
}

/// 能否在 `i` 这个字素簇边界处开始新的一行（`prev` 是边界左侧的簇，`next` 是右侧的簇）。
fn can_break(prev: &str, next: &str, o: &WrapOptions) -> bool {
    let (Some(a), Some(b)) = (prev.chars().next_back(), next.chars().next()) else {
        return false;
    };
    let (ka, kb) = (kind(a, &o.separators), kind(b, &o.separators));
    // 空白之后总可以断（行尾的空白留在上一行，照搬 VS Code）。
    if ka == Kind::Space {
        return true;
    }
    // 空白之前不断（否则会把行首空白推到下一行）。
    if kb == Kind::Space {
        return false;
    }
    // CJK：`wordBreak: normal` 时两侧任一是表意文字就可断；`keepAll` 时不断。
    if ka == Kind::Ideographic || kb == Kind::Ideographic {
        return !o.keep_all;
    }
    // 其余按单词边界：类别变化处可断（与双击选词的 `editor.wordSeparators` 同一套规则）。
    ka != kb
}

/// `editor.wrappingIndent` → 续行缩进的可见列数。
///
/// 照搬 VS Code：`none` 为 0；`same` 与原行缩进对齐；`indent` 再加一级；`deepIndent` 加两级。
/// 缩进挤得正文放不下（剩余不足一个制表位）时退回 0。
pub fn indent_columns(source: &str, o: &WrapOptions, limit_columns: u32) -> u32 {
    if o.indent == WrappingIndent::None {
        return 0;
    }
    let tab = o.tab_size.max(1);
    let mut base = 0u32;
    for c in source.chars() {
        match c {
            '\t' => base = super::columns::next_tab_stop(base, tab),
            ' ' => base += 1,
            _ => break,
        }
    }
    let extra = match o.indent {
        WrappingIndent::Indent => tab,
        WrappingIndent::DeepIndent => tab * 2,
        _ => 0,
    };
    let want = base + extra;
    if want + tab > limit_columns {
        0
    } else {
        want
    }
}

/// 算一条缓冲行的换行点。`source` 是源行（不含行终止符），`display` 是它的显示形式。
///
/// 复杂度 O(本行字素簇数)：每个边界最多被扫描两次（一次探到溢出，一次从断点重扫）。
pub fn wrap_line(source: &str, display: &DisplayLine, metric: Metric, o: &WrapOptions) -> LineWrap {
    let stops = display.stops();
    let max = metric.limit();
    if stops.len() < 2 || max <= 0.0 {
        return LineWrap::default();
    }
    let limit_columns = match metric {
        Metric::Columns { limit } => limit,
        Metric::Pixels { limit, cell, .. } if cell > 0.0 => (limit / cell).floor() as u32,
        Metric::Pixels { .. } => u32::MAX,
    };
    let indent_cols = indent_columns(source, o, limit_columns);
    let indent = metric.columns_to_unit(indent_cols);
    // 一个字符都放不下时不换行，避免每簇一行（与 VS Code 的下限保护同义）。
    if max < metric.cell() {
        return LineWrap::default();
    }
    let text = &*display.text;
    let grapheme = |i: usize| -> &str {
        let (a, b) = (stops[i - 1].disp as usize, stops[i].disp as usize);
        text.get(a..b).unwrap_or("")
    };
    // 显示文本里 Tab 已经展开成若干空格，所以整簇都是空白才算空白。
    let is_space = |g: &str| !g.is_empty() && g.bytes().all(|b| b == b' ' || b == b'\t');
    let mut breaks: Vec<u32> = Vec::new();
    let mut start = 0usize;
    let mut candidate: Option<usize> = None;
    // 本段第一个非空白字素簇的右边界：在它之前不许断行，否则会出现只有空白的视觉行
    // （照搬 VS Code“不在首个非空白之前换行”）。
    let mut content: Option<usize> = None;
    let mut i = 1usize;
    while i < stops.len() {
        let here = grapheme(i);
        if content.is_none() && !is_space(here) {
            content = Some(i);
        }
        // `wrapOnEscapedLineFeeds`：`\n` 字面量之后强制换行。
        if o.escaped_line_feeds && i >= 2 && here == "n" && grapheme(i - 1) == "\\" && i > start {
            breaks.push(stops[i].src);
            start = i;
            candidate = None;
            content = None;
            i += 1;
            continue;
        }
        let offset = if start == 0 { 0.0 } else { indent };
        let width = metric.at(display, i) - metric.at(display, start) + offset;
        // 行尾空白允许越过换行列（照搬 VS Code：空白留在上一行，不把它推到下一行）。
        if width > max && !is_space(here) {
            // 优先在最近的可断点换行；没有可断点就在溢出处硬断（至少保留一个字素簇）。
            let at = candidate
                .filter(|&c| c > start)
                .unwrap_or((i - 1).max(start + 1));
            breaks.push(stops[at].src);
            start = at;
            candidate = None;
            content = None;
            i = at + 1;
            continue;
        }
        let next = if i + 1 < stops.len() {
            grapheme(i + 1)
        } else {
            ""
        };
        if content.is_some_and(|c| i >= c) && can_break(here, next, o) {
            candidate = Some(i);
        }
        i += 1;
    }
    LineWrap::new(breaks, indent_cols)
}

// ------------------------------------------------------------------ 视觉行映射

/// 前缀和树（Fenwick）：按行加减视觉行数，O(log n) 求前缀和与按前缀和定位行。
#[derive(Clone, Debug, Default, PartialEq)]
struct Sums {
    tree: Vec<u64>,
}

impl Sums {
    fn with_ones(n: usize) -> Self {
        let mut tree = vec![0u64; n + 1];
        // 全 1 的 Fenwick 树可以直接构造：t[i] = lowbit(i)。
        for (i, slot) in tree.iter_mut().enumerate().skip(1) {
            *slot = (i & i.wrapping_neg()) as u64;
        }
        Self { tree }
    }
    /// 由每行的计数线性建树 O(n)（编辑后重映射用）。
    fn from_counts(counts: &[u32]) -> Self {
        let n = counts.len();
        let mut tree = vec![0u64; n + 1];
        for (i, c) in counts.iter().enumerate() {
            tree[i + 1] += *c as u64;
            let j = (i + 1) + ((i + 1) & (i + 1).wrapping_neg());
            if j <= n {
                let v = tree[i + 1];
                tree[j] += v;
            }
        }
        Self { tree }
    }
    fn add(&mut self, index: usize, delta: i64) {
        let mut i = index + 1;
        while i < self.tree.len() {
            self.tree[i] = (self.tree[i] as i64 + delta) as u64;
            i += i & i.wrapping_neg();
        }
    }
    /// `[0, index)` 的和。
    fn prefix(&self, index: usize) -> u64 {
        let mut i = index.min(self.tree.len().saturating_sub(1));
        let mut sum = 0;
        while i > 0 {
            sum += self.tree[i];
            i -= i & i.wrapping_neg();
        }
        sum
    }
    /// 最大的 `k`，使得 `prefix(k) <= target`（按位倍增）。
    fn seek(&self, target: u64) -> usize {
        let n = self.tree.len().saturating_sub(1);
        let mut pos = 0usize;
        let mut rest = target;
        let mut step = n.next_power_of_two();
        while step > 0 {
            let next = pos + step;
            if next <= n && self.tree[next] <= rest {
                rest -= self.tree[next];
                pos = next;
            }
            step /= 2;
        }
        pos
    }
}

/// 缓冲行 ↔ 视觉行的权威映射：折叠 + 软换行合在一处。
///
/// 软换行关闭时 `counts` 为 `None`，所有换算直接走 [`FoldMap`]，不分配内存（超大文件也安全）。
#[derive(Clone, Debug, PartialEq)]
pub struct VisualMap {
    counts: Option<(Vec<u32>, Sums)>,
    folds: FoldMap,
    /// `hidden_before[i]`：前 i 个隐藏区间一共藏掉的视觉行数（长度 = 区间数 + 1）。
    hidden_before: Vec<u64>,
    /// `visible_before[i]`：第 i 个隐藏区间之前的可见视觉行数（视觉行 → 缓冲行的二分键）。
    visible_before: Vec<u64>,
    doc_lines: u64,
}

impl Default for VisualMap {
    fn default() -> Self {
        Self::new(FoldMap::default(), 1)
    }
}

impl VisualMap {
    /// 不换行的映射（只有折叠）。
    pub fn new(folds: FoldMap, doc_lines: u64) -> Self {
        let mut m = Self {
            counts: None,
            folds,
            hidden_before: Vec::new(),
            visible_before: Vec::new(),
            doc_lines: doc_lines.max(1),
        };
        m.commit();
        m
    }

    /// 开启软换行：分配每行的视觉行数（新行初始为 1），由调用方按行填。
    /// 行数变化时保留已算好的前缀，只有新增的行要重算。
    pub fn enable_wrap(&mut self, doc_lines: u64) {
        let n = doc_lines.max(1) as usize;
        match &mut self.counts {
            Some((c, _)) if c.len() == n => {}
            Some((c, sums)) => {
                c.resize(n, 1);
                *sums = Sums::from_counts(c);
            }
            None => self.counts = Some((vec![1u32; n], Sums::with_ones(n))),
        }
        self.doc_lines = doc_lines.max(1);
        self.commit();
    }

    /// 关闭软换行（回到「一缓冲行一视觉行」）。
    pub fn disable_wrap(&mut self) {
        if self.counts.is_some() {
            self.counts = None;
            self.commit();
        }
    }

    pub fn is_wrapping(&self) -> bool {
        self.counts.is_some()
    }

    pub fn folds(&self) -> &FoldMap {
        &self.folds
    }

    pub fn doc_lines(&self) -> u64 {
        self.doc_lines
    }

    /// 折叠变化：换一份折叠映射（随后要 [`commit`](Self::commit)）。
    pub fn set_folds(&mut self, folds: FoldMap) {
        self.folds = folds;
        self.commit();
    }

    /// 行数变化（#8 的索引增长、编辑增删行）。
    pub fn set_doc_lines(&mut self, doc_lines: u64) {
        let doc_lines = doc_lines.max(1);
        if doc_lines == self.doc_lines {
            return;
        }
        self.doc_lines = doc_lines;
        if self.counts.is_some() {
            self.enable_wrap(doc_lines);
        } else {
            self.commit();
        }
    }

    /// 某缓冲行占的视觉行数。
    pub fn span(&self, doc: u64) -> u32 {
        match &self.counts {
            Some((c, _)) => c.get(doc as usize).copied().unwrap_or(1).max(1),
            None => 1,
        }
    }

    /// 设置某缓冲行的视觉行数；返回是否有变化（供「哪些行的视觉行数变了」上报）。
    /// 改完一批后要调 [`commit`](Self::commit)。
    pub fn set_count(&mut self, doc: u64, count: u32) -> bool {
        let count = count.max(1);
        let Some((c, sums)) = &mut self.counts else {
            return false;
        };
        let Some(slot) = c.get_mut(doc as usize) else {
            return false;
        };
        if *slot == count {
            return false;
        }
        let delta = count as i64 - *slot as i64;
        *slot = count;
        sums.add(doc as usize, delta);
        true
    }

    /// 编辑后按行重映射视觉行数：`f(旧行号)` 为 `None` 表示该行被改写（计数重置为 1）。
    /// 未被改写的行保留原来的视觉行数，于是编辑一行不会让整份映射归零。O(旧行数 + 新行数)。
    pub fn remap(&mut self, new_lines: u64, f: impl Fn(u64) -> Option<u64>) {
        let new_lines = new_lines.max(1);
        if let Some((old, _)) = self.counts.take() {
            let mut counts = vec![1u32; new_lines as usize];
            for (line, c) in old.iter().enumerate() {
                if *c == 1 {
                    continue;
                }
                if let Some(to) = f(line as u64) {
                    if let Some(slot) = counts.get_mut(to as usize) {
                        *slot = *c;
                    }
                }
            }
            let sums = Sums::from_counts(&counts);
            self.counts = Some((counts, sums));
        }
        self.doc_lines = new_lines;
        self.commit();
    }

    /// 重建隐藏区间的前缀和（折叠或一批 [`set_count`](Self::set_count) 之后调用）。
    pub fn commit(&mut self) {
        let hidden = self.folds.hidden_ranges();
        self.hidden_before = Vec::with_capacity(hidden.len() + 1);
        self.visible_before = Vec::with_capacity(hidden.len());
        let mut sum = 0u64;
        self.hidden_before.push(0);
        for r in hidden {
            let before = self.total_before(r.start) - sum;
            self.visible_before.push(before);
            sum += self.total_before(r.end) - self.total_before(r.start);
            self.hidden_before.push(sum);
        }
    }

    /// `[0, doc)` 的视觉行数（不考虑折叠）。
    fn total_before(&self, doc: u64) -> u64 {
        let doc = doc.min(self.doc_lines);
        match &self.counts {
            Some((_, sums)) => sums.prefix(doc as usize),
            None => doc,
        }
    }

    /// 视觉行总数（折叠隐藏的不算）。
    pub fn visual_count(&self) -> u64 {
        (self.total_before(self.doc_lines) - self.hidden_before.last().copied().unwrap_or(0)).max(1)
    }

    /// 缓冲行 → 它的首条视觉行；隐藏行映射到折叠首行的首条视觉行。
    pub fn first_visual(&self, doc: u64) -> u64 {
        let doc = self
            .folds
            .prev_visible(doc.min(self.doc_lines.saturating_sub(1)));
        let i = self.folds.hidden_ranges().partition_point(|r| r.end <= doc);
        self.total_before(doc) - self.hidden_before[i]
    }

    /// 缓冲行占据的视觉行区间。
    pub fn visual_range(&self, doc: u64) -> Range<u64> {
        let first = self.first_visual(doc);
        first..first + self.span(self.folds.prev_visible(doc)) as u64
    }

    /// 视觉行 → (缓冲行, 段号)。
    pub fn to_doc(&self, visual: u64) -> (u64, u32) {
        let i = self.visible_before.partition_point(|&v| v <= visual);
        let target = visual + self.hidden_before[i];
        match &self.counts {
            Some((_, sums)) => {
                let doc = (sums.seek(target) as u64).min(self.doc_lines.saturating_sub(1));
                (doc, (target - sums.prefix(doc as usize)) as u32)
            }
            None => (target.min(self.doc_lines.saturating_sub(1)), 0),
        }
    }

    /// 视觉行 → 缓冲行（只要行号）。
    pub fn doc_line(&self, visual: u64) -> u64 {
        self.to_doc(visual).0
    }
}

// ------------------------------------------------------------------ 分帧重算

/// 待重算换行点的缓冲行队列（视口宽度 / 字体 / 设置变化后排队，按帧分片消化）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrapPending {
    /// 还没处理的行区间（有序、互不相交）。
    ranges: Vec<Range<u64>>,
}

impl WrapPending {
    pub fn clear(&mut self) {
        self.ranges.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// 还剩多少行要算。
    pub fn remaining(&self) -> u64 {
        self.ranges.iter().map(|r| r.end - r.start).sum()
    }

    /// 把一段行排进队列（合并相邻与重叠区间）。
    pub fn mark(&mut self, range: Range<u64>) {
        if range.start >= range.end {
            return;
        }
        let at = self.ranges.partition_point(|r| r.end < range.start);
        let mut merged = range;
        let mut end = at;
        while end < self.ranges.len() && self.ranges[end].start <= merged.end {
            merged.start = merged.start.min(self.ranges[end].start);
            merged.end = merged.end.max(self.ranges[end].end);
            end += 1;
        }
        self.ranges.splice(at..end, [merged]);
    }

    /// 取出下一批（至多 `max` 行）。
    pub fn take(&mut self, max: u64) -> Range<u64> {
        let Some(first) = self.ranges.first_mut() else {
            return 0..0;
        };
        let end = first.end.min(first.start + max.max(1));
        let out = first.start..end;
        if end >= first.end {
            self.ranges.remove(0);
        } else {
            first.start = end;
        }
        out
    }

    /// 某行已经单独算过了：从队列里去掉（可见行在布局时就算好，不必再排队）。
    pub fn done(&mut self, line: u64) {
        let Some(i) = self.ranges.iter().position(|r| r.contains(&line)) else {
            return;
        };
        let r = self.ranges[i].clone();
        let mut repl: Vec<Range<u64>> = Vec::new();
        if r.start < line {
            repl.push(r.start..line);
        }
        if line + 1 < r.end {
            repl.push(line + 1..r.end);
        }
        self.ranges.splice(i..i + 1, repl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(mode: WordWrap) -> WrapOptions {
        WrapOptions {
            mode,
            separators: "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".into(),
            ..Default::default()
        }
    }

    /// 按可见列换行，返回每段的文本（便于断言）。
    fn segments(source: &str, columns: u32, o: &WrapOptions) -> Vec<String> {
        let d = DisplayLine::build(source, o.tab_size, None);
        let w = wrap_line(source, &d, Metric::Columns { limit: columns }, o);
        (0..w.count())
            .map(|s| {
                let r = w.segment_range(s, source.len() as u32);
                source[r.start as usize..r.end as usize].to_string()
            })
            .collect()
    }

    #[test]
    fn word_wrap_modes_pick_the_right_column_limit() {
        let mut o = opts(WordWrap::Off);
        assert_eq!(o.columns(800.0, 10.0), None);
        o.mode = WordWrap::On;
        assert_eq!(o.columns(800.0, 10.0), Some(80));
        o.mode = WordWrap::Column;
        o.column = 40;
        assert_eq!(o.columns(800.0, 10.0), Some(40));
        o.mode = WordWrap::Bounded;
        assert_eq!(o.columns(800.0, 10.0), Some(40));
        assert_eq!(o.columns(200.0, 10.0), Some(20));
    }

    #[test]
    fn breaks_at_word_boundaries_and_falls_back_to_hard_break() {
        let o = opts(WordWrap::On);
        assert_eq!(
            segments("alpha beta gamma delta", 12, &o),
            vec!["alpha beta ", "gamma delta"]
        );
        // 没有可断点的长单词：按宽度硬断。
        assert_eq!(
            segments("aaaaaaaaaaaaaaa", 6, &o),
            vec!["aaaaaa", "aaaaaa", "aaa"]
        );
        // 分隔符处可断（与双击选词同一套 `wordSeparators`）。
        assert_eq!(
            segments("foo.bar.baz.qux", 8, &o),
            vec!["foo.bar.", "baz.qux"]
        );
        // 空行与短行不换。
        assert_eq!(segments("", 10, &o), vec![""]);
        assert_eq!(segments("short", 10, &o), vec!["short"]);
    }

    #[test]
    fn tabs_cjk_and_emoji_never_split_clusters() {
        let o = opts(WordWrap::On);
        // Tab 展开后按可见列计数：\t 占 4 列。
        assert_eq!(segments("\tabcdefgh", 8, &o), vec!["\tabcd", "efgh"]);
        // CJK 每字 2 列，默认 `wordBreak: normal` 可以逐字断。
        assert_eq!(
            segments("中文换行测试", 4, &o),
            vec!["中文", "换行", "测试"]
        );
        // ZWJ 家庭 emoji 与肤色修饰整簇移动，不会被切开。
        let family = "👨‍👩‍👧‍👦";
        let skin = "🧑🏽‍💻";
        let src = format!("{family}{skin}{family}");
        let segs = segments(&src, 4, &o);
        assert_eq!(segs, vec![format!("{family}{skin}"), family.to_string()]);
        for s in &segs {
            assert!(src.contains(s.as_str()));
        }
        // 组合字符（e + U+0301）同理。
        assert_eq!(
            segments("e\u{301}xe\u{301}x", 2, &o),
            vec!["e\u{301}x", "e\u{301}x"]
        );
    }

    #[test]
    fn keep_all_stops_breaking_between_cjk() {
        let mut o = opts(WordWrap::On);
        // `normal`：CJK 与相邻字符之间可以断，于是在“文”之后换行。
        assert_eq!(segments("中文abcdef", 6, &o), vec!["中文", "abcdef"]);
        // `keepAll`：CJK 之间不断，只能在溢出处硬断。
        o.keep_all = true;
        assert_eq!(segments("中文abcdef", 6, &o), vec!["中文ab", "cdef"]);
        // 空格处两种取值都能断；行尾空白允许越过换行列。
        assert_eq!(
            segments("中文词 更多文字", 8, &o),
            vec!["中文词 ", "更多文字"]
        );
    }

    #[test]
    fn escaped_line_feeds_force_a_break() {
        let mut o = opts(WordWrap::On);
        assert_eq!(segments("a\\nb", 80, &o), vec!["a\\nb"]);
        o.escaped_line_feeds = true;
        assert_eq!(segments("a\\nb\\nc", 80, &o), vec!["a\\n", "b\\n", "c"]);
    }

    #[test]
    fn wrapping_indent_follows_the_setting() {
        let mut o = opts(WordWrap::On);
        let src = "    alpha beta gamma";
        o.indent = WrappingIndent::None;
        assert_eq!(indent_columns(src, &o, 20), 0);
        o.indent = WrappingIndent::Same;
        assert_eq!(indent_columns(src, &o, 20), 4);
        o.indent = WrappingIndent::Indent;
        assert_eq!(indent_columns(src, &o, 20), 8);
        o.indent = WrappingIndent::DeepIndent;
        assert_eq!(indent_columns(src, &o, 20), 12);
        // 缩进挤掉正文时退回 0。
        assert_eq!(indent_columns(src, &o, 14), 0);
        // Tab 缩进按制表位算。
        o.indent = WrappingIndent::Same;
        assert_eq!(indent_columns("\t\tx", &o, 40), 8);
        // 续行缩进占掉宽度：同样的列上限下段数变多。
        o.indent = WrappingIndent::None;
        let plain = segments(src, 12, &o).len();
        o.indent = WrappingIndent::Same;
        assert!(segments(src, 12, &o).len() > plain);
    }

    #[test]
    fn advanced_strategy_uses_real_glyph_widths() {
        let o = opts(WordWrap::On);
        let src = "iiii mmmm";
        let d = DisplayLine::build(src, 4, None);
        // 等宽近似：9 列，限 6 列 → 在空格后断。
        let simple = wrap_line(src, &d, Metric::Columns { limit: 6 }, &o);
        assert_eq!(simple.breaks(), &[5]);
        // 真实字形：i 窄 m 宽，按像素度量同样在空格后断，但“mmmm”整段放得下。
        let xs: Vec<f32> = [0.0, 3.0, 6.0, 9.0, 12.0, 16.0, 26.0, 36.0, 46.0, 56.0].into();
        let advanced = wrap_line(
            src,
            &d,
            Metric::Pixels {
                xs: &xs,
                limit: 42.0,
                cell: 6.0,
            },
            &o,
        );
        assert_eq!(advanced.breaks(), &[5]);
        // 窄字符多时一行能放下更多。
        let narrow = "iiiiiiiiii";
        let dn = DisplayLine::build(narrow, 4, None);
        let xs: Vec<f32> = (0..=10).map(|i| i as f32 * 3.0).collect();
        let w = wrap_line(
            narrow,
            &dn,
            Metric::Pixels {
                xs: &xs,
                limit: 30.0,
                cell: 6.0,
            },
            &o,
        );
        assert!(!w.is_wrapped(), "10 个窄字符共 30 px，正好放下");
        assert_eq!(
            wrap_line(narrow, &dn, Metric::Columns { limit: 5 }, &o).count(),
            2,
            "等宽近似按 10 列算，放不下"
        );
    }

    #[test]
    fn segment_lookup_round_trips() {
        let o = opts(WordWrap::On);
        let src = "alpha beta gamma delta";
        let d = DisplayLine::build(src, 4, None);
        let w = wrap_line(src, &d, Metric::Columns { limit: 12 }, &o);
        assert_eq!(w.count(), 2);
        assert_eq!(w.segment_of(0), 0);
        assert_eq!(w.segment_of(10), 0);
        assert_eq!(w.segment_of(11), 1);
        assert_eq!(w.segment_of(22), 1);
        assert_eq!(w.segment_range(0, 22), 0..11);
        assert_eq!(w.segment_range(1, 22), 11..22);
        assert_eq!(w.indent_columns(0), 0);
    }

    #[test]
    fn hundred_thousand_character_line_wraps_quickly() {
        let o = opts(WordWrap::On);
        let src = "x".repeat(100_000);
        let d = DisplayLine::build(&src, 4, Some(10_000));
        let t = std::time::Instant::now();
        let w = wrap_line(&src, &d, Metric::Columns { limit: 120 }, &o);
        // 截断之后的部分不参与换行（`editor.stopRenderingLineAfter`）。
        assert_eq!(w.count(), 10_000 / 120 + 1);
        assert!(t.elapsed().as_millis() < 100, "耗时 {:?}", t.elapsed());
    }

    // ---------------------------------------------------------------- 映射

    fn wrapped_map(counts: &[u32], folds: &[(u64, u64)]) -> VisualMap {
        let n = counts.len() as u64;
        let mut m = VisualMap::new(FoldMap::new(folds.iter().copied(), n), n);
        m.enable_wrap(n);
        for (i, &c) in counts.iter().enumerate() {
            m.set_count(i as u64, c);
        }
        m.commit();
        m
    }

    #[test]
    fn visual_map_without_wrap_is_the_fold_map() {
        let m = VisualMap::new(FoldMap::new([(2, 6)], 20), 20);
        assert!(!m.is_wrapping());
        assert_eq!(m.visual_count(), 17);
        assert_eq!(m.first_visual(7), 4);
        assert_eq!(m.to_doc(4), (7, 0));
        assert_eq!(m.first_visual(4), m.first_visual(2), "隐藏行回到折叠首行");
    }

    #[test]
    fn visual_map_round_trips_with_wrap() {
        // 行 1 占 3 条视觉行，行 3 占 2 条。
        let m = wrapped_map(&[1, 3, 1, 2, 1], &[]);
        assert_eq!(m.visual_count(), 8);
        let expected = [
            (0, 0),
            (1, 0),
            (1, 1),
            (1, 2),
            (2, 0),
            (3, 0),
            (3, 1),
            (4, 0),
        ];
        for (v, want) in expected.iter().enumerate() {
            assert_eq!(m.to_doc(v as u64), *want, "视觉行 {v}");
        }
        for doc in 0..5u64 {
            let first = m.first_visual(doc);
            assert_eq!(m.to_doc(first), (doc, 0));
            assert_eq!(m.visual_range(doc).end - first, m.span(doc) as u64);
        }
        assert_eq!(m.visual_range(1), 1..4);
    }

    #[test]
    fn folding_a_range_keeps_visual_lines_contiguous() {
        let counts = [2u32, 1, 3, 2, 1, 2];
        let open = wrapped_map(&counts, &[]);
        assert_eq!(open.visual_count(), 11);
        // 折叠 1..4：隐藏行 2、3（共 5 条视觉行）。
        let folded = wrapped_map(&counts, &[(1, 4)]);
        assert_eq!(folded.visual_count(), 6);
        let seq: Vec<(u64, u32)> = (0..folded.visual_count())
            .map(|v| folded.to_doc(v))
            .collect();
        assert_eq!(
            seq,
            vec![(0, 0), (0, 1), (1, 0), (4, 0), (5, 0), (5, 1)],
            "视觉行号连续，隐藏行不出现"
        );
        assert_eq!(
            folded.first_visual(3),
            folded.first_visual(1),
            "隐藏行回到折叠首行"
        );
        // 展开后恢复原序列。
        let back = wrapped_map(&counts, &[]);
        assert_eq!(back, open);
        let seq: Vec<(u64, u32)> = (0..back.visual_count()).map(|v| back.to_doc(v)).collect();
        assert_eq!(seq.len(), 11);
        assert_eq!(seq[2], (1, 0));
        assert_eq!(seq[3], (2, 0));
    }

    #[test]
    fn incremental_updates_only_touch_one_line() {
        let mut m = wrapped_map(&[1, 1, 1, 1], &[]);
        assert_eq!(m.visual_count(), 4);
        assert!(m.set_count(1, 4));
        assert!(!m.set_count(1, 4), "没变就不上报");
        m.commit();
        assert_eq!(m.visual_count(), 7);
        assert_eq!(m.to_doc(4), (1, 3));
        assert_eq!(m.first_visual(2), 5);
        assert!(m.set_count(1, 1));
        m.commit();
        assert_eq!(m.visual_count(), 4);
        // 关掉软换行回到恒等映射。
        m.disable_wrap();
        assert!(!m.is_wrapping());
        assert_eq!(m.to_doc(3), (3, 0));
    }

    #[test]
    fn large_map_is_fast() {
        let n = 300_000u64;
        let mut m = VisualMap::new(FoldMap::default(), n);
        m.enable_wrap(n);
        let t = std::time::Instant::now();
        for l in (0..n).step_by(3) {
            m.set_count(l, 2);
        }
        m.commit();
        let build = t.elapsed();
        assert_eq!(m.visual_count(), n + n.div_ceil(3));
        let t = std::time::Instant::now();
        for v in (0..m.visual_count()).step_by(997) {
            let (doc, _) = m.to_doc(v);
            assert!(m.visual_range(doc).contains(&v));
        }
        assert!(build.as_millis() < 2000, "构建耗时 {build:?}");
        assert!(t.elapsed().as_millis() < 500, "查询耗时 {:?}", t.elapsed());
    }

    #[test]
    fn pending_queue_merges_and_slices() {
        let mut p = WrapPending::default();
        p.mark(10..20);
        p.mark(0..5);
        p.mark(5..10); // 与两边相接，合并成 0..20
        assert_eq!(p.remaining(), 20);
        assert_eq!(p.take(8), 0..8);
        assert_eq!(p.remaining(), 12);
        p.done(9);
        assert_eq!(p.remaining(), 11);
        assert_eq!(p.take(100), 8..9);
        assert_eq!(p.take(100), 10..20);
        assert!(p.is_empty());
        assert_eq!(p.take(10), 0..0);
    }
}
