//! 装饰：按行的样式区间与整行样式，多个提供方按优先级叠加。
//!
//! 设计约束（issue #19 “风险与提示”）：装饰**不是**每帧由提供方重算。提供方只在某行进入视口
//! 或预取范围、且该行缓存已失效时被调用一次（在绘制之外的空闲时间），结果按行缓存；
//! 提供方通过 [`DecorationStore::invalidate`] 按行或整体失效。失效后在重算前继续用旧结果绘制，
//! 不会闪烁。
//!
//! 颜色只影响绘制、不影响整形；粗体/斜体会进入整形缓存键。

use rove_theme::Color;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

/// 常用优先级（数值大的覆盖数值小的；同优先级按登记顺序，后登记的覆盖先登记的）。
pub mod priority {
    /// #20：语法 token。
    pub const SYNTAX: i32 = 0;
    /// #40：C# 语义 token。
    pub const SEMANTIC: i32 = 10;
    /// #25：全局搜索结果的范围高亮。
    pub const SEARCH_RANGE: i32 = 20;
    /// #40：同名符号高亮。
    pub const SYMBOL_HIGHLIGHT: i32 = 30;
    /// #21：查找匹配。
    pub const FIND_MATCH: i32 = 40;
}

/// 一段文字的样式；`None` 表示不覆盖下层。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStyle {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<Color>,
    pub strikethrough: Option<Color>,
    /// 区间外框（查找匹配等）。
    pub border: Option<Color>,
}

impl TextStyle {
    /// 用 `top` 覆盖 `self` 中 `top` 指定了的属性。
    pub fn overlay(&mut self, top: &TextStyle) {
        macro_rules! take {
            ($($f:ident),*) => {$(if top.$f.is_some() { self.$f = top.$f; })*};
        }
        take!(
            foreground,
            background,
            bold,
            italic,
            underline,
            strikethrough,
            border
        );
    }
}

/// 行内样式区间：`range` 是源行的 UTF-8 字节列（半开）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineDecoration {
    pub range: Range<u32>,
    pub style: TextStyle,
}

/// 字形边距图标。`icon` 是图标 ID（由提供方登记），`color` 为空时用前景色。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphIcon {
    pub icon: Arc<str>,
    pub color: Option<Color>,
}

/// 整行样式。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineStyle {
    pub background: Option<Color>,
    pub border: Option<Color>,
    pub glyph: Option<GlyphIcon>,
}

/// 行内覆盖物：在 `column` 处叠画一段文字（不参与本行整形与列换算），
/// 例如 #22 的超长行截断标记。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub column: u32,
    pub text: Arc<str>,
    pub style: TextStyle,
}

/// 提供方为一行产出的全部装饰。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineDecorations {
    pub inline: Vec<InlineDecoration>,
    pub line: LineStyle,
    pub overlays: Vec<Overlay>,
}

/// 装饰提供方。只在 UI 线程、绘制之外调用；必须快速返回（只读自己的缓存，耗时工作放后台，
/// 完成后再调用 [`DecorationStore::invalidate`]）。
pub trait DecorationProvider {
    /// `line` 为 0 起行号，`text` 为该行源文本（不含终止符，可能很长）。
    fn decorate(&self, line: u64, text: &str, out: &mut LineDecorations);
}

impl<F: Fn(u64, &str, &mut LineDecorations)> DecorationProvider for F {
    fn decorate(&self, line: u64, text: &str, out: &mut LineDecorations) {
        self(line, text, out)
    }
}

/// 一段已合并的样式（源行字节列）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyledRun {
    pub range: Range<u32>,
    pub style: TextStyle,
}

/// 一行合并后的装饰结果（缓存单元）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergedLine {
    /// 覆盖整行、互不重叠、按列有序；未被装饰的空隙不出现。
    pub runs: Vec<StyledRun>,
    pub line: LineStyle,
    pub overlays: Vec<Overlay>,
}

impl MergedLine {
    /// 是否有影响整形的样式（粗体/斜体）。
    pub fn font_runs(&self) -> impl Iterator<Item = (Range<u32>, bool, bool)> + '_ {
        self.runs
            .iter()
            .filter(|r| r.style.bold.is_some() || r.style.italic.is_some())
            .map(|r| {
                (
                    r.range.clone(),
                    r.style.bold.unwrap_or(false),
                    r.style.italic.unwrap_or(false),
                )
            })
    }
}

/// 按优先级合并多层装饰（`layers` 已按优先级升序）。O(d log d)，d 为本行装饰数。
pub fn merge(layers: &[&LineDecorations], line_len: u32) -> MergedLine {
    let mut cuts: Vec<u32> = vec![0, line_len];
    for l in layers {
        for d in &l.inline {
            cuts.push(d.range.start.min(line_len));
            cuts.push(d.range.end.min(line_len));
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut runs: Vec<StyledRun> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        let mut style = TextStyle::default();
        let mut any = false;
        for l in layers {
            for d in &l.inline {
                if d.range.start <= a && b <= d.range.end && d.range.start < d.range.end {
                    style.overlay(&d.style);
                    any = true;
                }
            }
        }
        if !any {
            continue;
        }
        match runs.last_mut() {
            Some(last) if last.range.end == a && last.style == style => last.range.end = b,
            _ => runs.push(StyledRun { range: a..b, style }),
        }
    }
    let mut line = LineStyle::default();
    let mut overlays = Vec::new();
    for l in layers {
        if l.line.background.is_some() {
            line.background = l.line.background;
        }
        if l.line.border.is_some() {
            line.border = l.line.border;
        }
        if l.line.glyph.is_some() {
            line.glyph = l.line.glyph.clone();
        }
        overlays.extend(l.overlays.iter().cloned());
    }
    MergedLine {
        runs,
        line,
        overlays,
    }
}

/// 装饰层句柄（登记时返回，用于失效与移除）。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(pub Arc<str>);

struct Layer {
    id: LayerId,
    priority: i32,
    order: u64,
    provider: Rc<dyn DecorationProvider>,
    lines: HashMap<u64, Rc<LineDecorations>>,
}

struct Entry {
    merged: Rc<MergedLine>,
    stale: bool,
}

/// 视图持有的装饰存储：各层的行缓存 + 合并结果缓存。
#[derive(Default)]
pub struct DecorationStore {
    layers: Vec<Layer>,
    merged: HashMap<u64, Entry>,
    next_order: u64,
    /// 失效计数，视图据此决定是否需要重新准备可见行。
    pub generation: u64,
}

impl DecorationStore {
    /// 登记一层装饰。重复 ID 返回 `Duplicate` 且不覆盖（与 #1 的登记语义一致）。
    pub fn add(
        &mut self,
        id: &str,
        priority: i32,
        provider: Rc<dyn DecorationProvider>,
    ) -> rove_base::Result<LayerId> {
        if id.is_empty() {
            return Err(rove_base::Error::InvalidInput);
        }
        if self.layers.iter().any(|l| &*l.id.0 == id) {
            return Err(rove_base::Error::Duplicate(id.to_string()));
        }
        let id = LayerId(id.into());
        self.layers.push(Layer {
            id: id.clone(),
            priority,
            order: self.next_order,
            provider,
            lines: HashMap::new(),
        });
        self.next_order += 1;
        self.layers.sort_by_key(|l| (l.priority, l.order));
        self.invalidate_all();
        Ok(id)
    }

    pub fn remove(&mut self, id: &LayerId) -> bool {
        let before = self.layers.len();
        self.layers.retain(|l| &l.id != id);
        let removed = self.layers.len() != before;
        if removed {
            self.invalidate_all();
        }
        removed
    }

    /// 某层的若干行失效（`None` 为整层）。
    pub fn invalidate(&mut self, id: &LayerId, lines: Option<Range<u64>>) {
        let Some(layer) = self.layers.iter_mut().find(|l| &l.id == id) else {
            return;
        };
        self.generation += 1;
        match lines {
            None => {
                layer.lines.clear();
                for e in self.merged.values_mut() {
                    e.stale = true;
                }
            }
            Some(r) => {
                if r.end.saturating_sub(r.start) as usize > layer.lines.len() {
                    layer.lines.retain(|l, _| !r.contains(l));
                    for (l, e) in self.merged.iter_mut() {
                        if r.contains(l) {
                            e.stale = true;
                        }
                    }
                } else {
                    for l in r {
                        layer.lines.remove(&l);
                        if let Some(e) = self.merged.get_mut(&l) {
                            e.stale = true;
                        }
                    }
                }
            }
        }
    }

    /// 文本换了（新快照/新文档）：全部失效且丢弃旧结果。
    pub fn reset(&mut self) {
        for l in &mut self.layers {
            l.lines.clear();
        }
        self.merged.clear();
        self.generation += 1;
    }

    fn invalidate_all(&mut self) {
        for l in &mut self.layers {
            l.lines.clear();
        }
        for e in self.merged.values_mut() {
            e.stale = true;
        }
        self.generation += 1;
    }

    /// 绘制用：取缓存的合并结果（可能是失效前的旧结果）。
    pub fn cached(&self, line: u64) -> Option<&Rc<MergedLine>> {
        self.merged.get(&line).map(|e| &e.merged)
    }

    /// 是否需要（重新）计算。
    pub fn needs(&self, line: u64) -> bool {
        !self.layers.is_empty() && self.merged.get(&line).is_none_or(|e| e.stale)
    }

    /// 准备阶段：必要时调用提供方并合并。返回是否发生了变化。
    pub fn prepare(&mut self, line: u64, text: &str) -> bool {
        if !self.needs(line) {
            return false;
        }
        let mut decorated: Vec<Rc<LineDecorations>> = Vec::with_capacity(self.layers.len());
        for layer in &mut self.layers {
            let d = layer
                .lines
                .entry(line)
                .or_insert_with(|| {
                    let mut out = LineDecorations::default();
                    layer.provider.decorate(line, text, &mut out);
                    Rc::new(out)
                })
                .clone();
            decorated.push(d);
        }
        let refs: Vec<&LineDecorations> = decorated.iter().map(|d| d.as_ref()).collect();
        let merged = merge(&refs, text.len() as u32);
        let changed = self.merged.get(&line).is_none_or(|e| *e.merged != merged);
        self.merged.insert(
            line,
            Entry {
                merged: Rc::new(merged),
                stale: false,
            },
        );
        changed
    }

    /// 丢弃远离视口的缓存（保持内存有界）。
    pub fn retain_near(&mut self, keep: Range<u64>) {
        if self.merged.len() < 4096 {
            return;
        }
        self.merged.retain(|l, _| keep.contains(l));
        for layer in &mut self.layers {
            layer.lines.retain(|l, _| keep.contains(l));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const RED: Color = Color(255, 0, 0, 255);
    const BLUE: Color = Color(0, 0, 255, 255);
    const YELLOW: Color = Color(255, 255, 0, 255);

    fn fg(range: Range<u32>, c: Color) -> InlineDecoration {
        InlineDecoration {
            range,
            style: TextStyle {
                foreground: Some(c),
                ..Default::default()
            },
        }
    }

    #[test]
    fn higher_priority_overrides_per_attribute() {
        let syntax = LineDecorations {
            inline: vec![fg(0..6, RED), fg(7..12, BLUE)],
            ..Default::default()
        };
        let find = LineDecorations {
            inline: vec![InlineDecoration {
                range: 4..9,
                style: TextStyle {
                    background: Some(YELLOW),
                    ..Default::default()
                },
            }],
            line: LineStyle {
                background: Some(YELLOW),
                ..Default::default()
            },
            ..Default::default()
        };
        let m = merge(&[&syntax, &find], 12);
        let runs: Vec<_> = m
            .runs
            .iter()
            .map(|r| (r.range.clone(), r.style.foreground, r.style.background))
            .collect();
        assert_eq!(
            runs,
            vec![
                (0..4, Some(RED), None),
                (4..6, Some(RED), Some(YELLOW)),
                (6..7, None, Some(YELLOW)),
                (7..9, Some(BLUE), Some(YELLOW)),
                (9..12, Some(BLUE), None),
            ]
        );
        assert_eq!(m.line.background, Some(YELLOW));
    }

    #[test]
    fn store_caches_per_line_and_invalidates_by_range() {
        let calls = Rc::new(Cell::new(0));
        let c = calls.clone();
        let mut store = DecorationStore::default();
        let id = store
            .add(
                "test",
                priority::SYNTAX,
                Rc::new(move |line: u64, _: &str, out: &mut LineDecorations| {
                    c.set(c.get() + 1);
                    out.inline
                        .push(fg(0..1, if line.is_multiple_of(2) { RED } else { BLUE }));
                }),
            )
            .unwrap();
        assert!(store
            .add(
                "test",
                0,
                Rc::new(|_: u64, _: &str, _: &mut LineDecorations| {})
            )
            .is_err());
        for l in 0..10 {
            store.prepare(l, "abc");
        }
        for l in 0..10 {
            store.prepare(l, "abc");
        }
        assert_eq!(calls.get(), 10, "缓存命中时不重算");
        store.invalidate(&id, Some(3..5));
        assert!(store.cached(3).is_some(), "失效后旧结果保留到重算前");
        assert!(store.needs(3) && store.needs(4) && !store.needs(5));
        for l in 0..10 {
            store.prepare(l, "abc");
        }
        assert_eq!(calls.get(), 12);
        store.invalidate(&id, None);
        for l in 0..10 {
            store.prepare(l, "abc");
        }
        assert_eq!(calls.get(), 22);
    }
}
