//! 语义着色层（#59）：C# 语义 token 的行内样式 + 不生效预处理区域的置灰（D9）。
//!
//! 与 #20 的语法着色层（[`super::highlight::SyntaxLayer`]）同构，只是数据来自 #1 契约的
//! C# 导航查询而不是 tree-sitter，优先级 [`super::priority::SEMANTIC`] 高于语法层：
//!
//! - **首屏是语法高亮**：语义结果到达前这一层什么都不画，到达后只让**被覆盖的行**失效
//!   （[`SemanticLayer::update`] 返回要失效的行集合），不整文件重画；
//! - **不闪烁**：结果过期（文本改了、索引换代了）时继续用旧结果绘制，直到新结果到达；
//! - **只改前景色与 fontStyle**，不改背景、不改整形结果；粗体 / 斜体经 #20 的
//!   `font_runs` 进入整形缓存键（与语法层同一条路径）；
//! - `editor.maxTokenizationLineLength`：超长行不着色，与语法层一致；
//! - 级别关闭（大文件 / 超大文件，UX_SPEC §3.1）时整层不画，原因登记到
//!   [`super::view::CodeView::disabled_features`]。

use super::decorations::{DecorationProvider, InlineDecoration, LineDecorations, TextStyle};
use rove_buffer::edit::LineChanges;
use rove_lang_cs::query::SemanticToken;
use rove_theme::semantic::SemanticStyle;
use rove_theme::{Color, ThemeKind};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

/// 语言 ID：语义选择器里的 `:csharp` 按它匹配。
pub const LANGUAGE: &str = "csharp";

/// 一行上的语义样式区间（源行的 UTF-8 字节列）。
type LineRuns = Vec<(Range<u32>, TextStyle)>;

/// （类型, 修饰符）→ 样式的本次解析缓存。
type StyleCache = std::collections::HashMap<(Arc<str>, Arc<[Arc<str>]>), SemanticStyle>;

/// 一份已经解析成样式的语义结果。
#[derive(Default)]
pub struct SemanticStyles {
    /// 行号 → 该行的样式区间（已按列排序）。
    lines: BTreeMap<u64, LineRuns>,
}

impl SemanticStyles {
    /// 把语义 token 解析成样式。`style_of` 由主题给出（类型 + 修饰符 → 样式）。
    pub fn build(
        tokens: &[SemanticToken],
        style_of: &dyn Fn(&str, &[&str]) -> SemanticStyle,
    ) -> Self {
        let mut lines: BTreeMap<u64, LineRuns> = BTreeMap::new();
        // 同一个（类型, 修饰符）组合在一个文件里会出现很多次，解析一次就够。
        let mut cache: StyleCache = StyleCache::new();
        for token in tokens {
            if token.length_bytes == 0 {
                continue;
            }
            let key = (token.token_type.clone(), token.modifiers.clone());
            let style = *cache.entry(key).or_insert_with(|| {
                let names: Vec<&str> = token.modifiers.iter().map(|m| &**m).collect();
                style_of(&token.token_type, &names)
            });
            if style.is_empty() {
                continue;
            }
            let fg = style.foreground;
            let text = TextStyle {
                foreground: fg,
                background: None,
                bold: style.bold,
                italic: style.italic,
                underline: style.underline.and_then(|u| u.then_some(fg).flatten()),
                strikethrough: style.strikethrough.and_then(|s| s.then_some(fg).flatten()),
                border: None,
            };
            if text == TextStyle::default() {
                continue;
            }
            let start = token.column.0 as u32;
            let end = start.saturating_add(token.length_bytes as u32);
            lines
                .entry(token.line.0)
                .or_default()
                .push((start..end, text));
        }
        for runs in lines.values_mut() {
            runs.sort_by_key(|(r, _)| r.start);
        }
        Self { lines }
    }

    /// 有语义样式的行（用于按行失效）。
    pub fn lines(&self) -> impl Iterator<Item = u64> + '_ {
        self.lines.keys().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// 不生效的预处理区域：整行区间（半开，行号）。
pub type InactiveLines = Vec<Range<u64>>;

/// 语义层的状态（视图持有，提供方只读）。
pub struct SemanticLayer {
    styles: SemanticStyles,
    inactive: InactiveLines,
    /// `editorUnnecessaryCode.opacity` 的 alpha 通道（`None` 为不置灰）。
    pub unnecessary_opacity: Option<u8>,
    /// `editor.maxTokenizationLineLength`（字符）；`None` 为不限。
    pub max_line_length: Option<usize>,
    /// 设置开关（`editor.semanticHighlighting.enabled` 与主题的 `semanticHighlighting`）。
    /// 为假时整层不画，宿主也**不发起任何查询**。
    pub enabled: bool,
    /// 文档级别允许（普通文件为真；大文件与超大文件为假，UX_SPEC §3.1）。
    pub level_allows: bool,
}

impl Default for SemanticLayer {
    fn default() -> Self {
        Self {
            styles: SemanticStyles::default(),
            inactive: Vec::new(),
            unnecessary_opacity: None,
            max_line_length: Some(2500),
            enabled: true,
            level_allows: true,
        }
    }
}

/// `editorUnnecessaryCode.opacity` 的默认值（VS Code：深色 `#000a`、浅色 `#0007`，
/// 高对比度不置灰）。主题可以覆盖；这里只在主题没登记这个键时兜底。
pub fn default_unnecessary_opacity(kind: ThemeKind) -> Option<u8> {
    match kind {
        ThemeKind::Dark => Some(0xaa),
        ThemeKind::Light => Some(0x77),
        ThemeKind::HighContrastDark | ThemeKind::HighContrastLight => None,
    }
}

/// 颜色的 alpha 通道就是不透明度（VS Code 用 `editorUnnecessaryCode.opacity` 的 alpha）。
pub fn opacity_of(color: Option<Color>, kind: ThemeKind) -> Option<u8> {
    match color {
        Some(c) => Some(c.3),
        None => default_unnecessary_opacity(kind),
    }
}

impl SemanticLayer {
    /// 换一份语义结果。返回需要失效的行：新旧结果覆盖到的行的并集（**只有这些行**重画）。
    pub fn update(&mut self, styles: SemanticStyles) -> Vec<Range<u64>> {
        let mut lines: Vec<u64> = self.styles.lines().chain(styles.lines()).collect();
        self.styles = styles;
        lines.sort_unstable();
        lines.dedup();
        to_ranges(lines)
    }

    /// 换一份不生效预处理区域。返回需要失效的行区间（新旧并集）。
    pub fn update_inactive(&mut self, inactive: InactiveLines) -> Vec<Range<u64>> {
        let mut out = std::mem::replace(&mut self.inactive, inactive);
        out.extend(self.inactive.iter().cloned());
        out.retain(|r| r.start < r.end);
        out
    }

    /// Keep styling already displayed on unchanged rows while the next query runs.
    pub(super) fn remap(&mut self, changes: &LineChanges) {
        let Some(first) = changes.ranges.first() else {
            return;
        };
        if self.is_empty() {
            return;
        }
        let same_rows = changes.ranges.iter().all(|change| change.old == change.new);
        if same_rows {
            for change in &changes.ranges {
                for line in change.old.start().0..change.old.end().0 {
                    self.styles.lines.remove(&line);
                }
            }
        } else {
            let changed = self.styles.lines.split_off(&first.old.start().0);
            self.styles
                .lines
                .extend(changed.into_iter().filter_map(|(line, runs)| {
                    super::edit::old_to_new(changes, line).map(|line| (line, runs))
                }));
        }
        let previous = std::mem::take(&mut self.inactive);
        for region in previous {
            let mut start = region.start;
            for change in &changes.ranges {
                let start_of_edit = change.old.start().0;
                let end_of_edit = change.old.end().0;
                if start_of_edit >= region.end {
                    break;
                }
                if end_of_edit <= start {
                    continue;
                }
                if start < start_of_edit {
                    self.keep_inactive(changes, start..start_of_edit);
                }
                start = end_of_edit;
            }
            if start < region.end {
                self.keep_inactive(changes, start..region.end);
            }
        }
    }

    fn keep_inactive(&mut self, changes: &LineChanges, range: Range<u64>) {
        if let Some(mapped) = super::edit::old_to_new(changes, range.start) {
            self.inactive
                .push(mapped..mapped + (range.end - range.start));
        }
    }

    /// 清空（文档关闭、语言不是 C#、开关关闭）。返回需要失效的行。
    pub fn clear(&mut self) -> Vec<Range<u64>> {
        let mut out = self.update(SemanticStyles::default());
        out.extend(self.update_inactive(Vec::new()));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty() && self.inactive.is_empty()
    }

    /// 这一层现在画不画（设置开着 + 级别允许）。
    pub fn is_active(&self) -> bool {
        self.enabled && self.level_allows
    }

    fn is_inactive(&self, line: u64) -> bool {
        self.inactive.iter().any(|r| r.contains(&line))
    }

    /// 某行源文本的语义样式区间。
    pub fn decorate(&self, line: u64, source: &str, out: &mut LineDecorations) {
        if !self.is_active() {
            return;
        }
        if self.is_inactive(line) {
            out.line.opacity = self.unnecessary_opacity;
        }
        if self
            .max_line_length
            .is_some_and(|max| source.len() > max && source.chars().count() > max)
        {
            return;
        }
        let Some(runs) = self.styles.lines.get(&line) else {
            return;
        };
        let len = source.len() as u32;
        for (range, style) in runs {
            let end = range.end.min(len);
            if range.start >= end {
                continue;
            }
            out.inline.push(InlineDecoration {
                range: range.start..end,
                style: *style,
            });
        }
    }
}

/// 连续行号合并成区间。
fn to_ranges(lines: Vec<u64>) -> Vec<Range<u64>> {
    let mut out: Vec<Range<u64>> = Vec::new();
    for line in lines {
        match out.last_mut() {
            Some(last) if last.end == line => last.end = line + 1,
            _ => out.push(line..line + 1),
        }
    }
    out
}

/// 语义层的装饰提供方。
pub struct SemanticProvider(pub Rc<RefCell<SemanticLayer>>);

impl DecorationProvider for SemanticProvider {
    fn decorate(&self, line: u64, text: &str, out: &mut LineDecorations) {
        self.0.borrow().decorate(line, text, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::{Line, Utf8Column};

    fn token(line: u64, column: u64, len: u64, ty: &str, mods: &[&str]) -> SemanticToken {
        SemanticToken {
            line: Line(line),
            column: Utf8Column(column),
            length_bytes: len,
            token_type: ty.into(),
            modifiers: mods.iter().map(|m| Arc::<str>::from(*m)).collect(),
        }
    }

    const RED: Color = Color(255, 0, 0, 255);
    const BLUE: Color = Color(0, 0, 255, 255);

    fn styles(tokens: &[SemanticToken]) -> SemanticStyles {
        SemanticStyles::build(tokens, &|ty, mods| match (ty, mods.first().copied()) {
            ("field", _) => SemanticStyle {
                foreground: Some(RED),
                ..SemanticStyle::default()
            },
            ("variable", Some("readonly")) => SemanticStyle {
                foreground: Some(BLUE),
                bold: Some(true),
                ..SemanticStyle::default()
            },
            _ => SemanticStyle::default(),
        })
    }

    #[test]
    fn tokens_become_line_runs_and_untouched_types_are_skipped() {
        let s = styles(&[
            token(3, 8, 6, "field", &[]),
            token(3, 0, 3, "method", &[]),
            token(7, 4, 2, "variable", &["readonly"]),
        ]);
        assert_eq!(s.lines().collect::<Vec<_>>(), vec![3, 7]);
        let mut layer = SemanticLayer::default();
        let invalid = layer.update(s);
        assert_eq!(invalid, vec![3..4, 7..8]);
        let mut out = LineDecorations::default();
        layer.decorate(3, "    private int health;", &mut out);
        assert_eq!(out.inline.len(), 1, "method 没有样式，不产生区间");
        assert_eq!(out.inline[0].range, 8..14);
        assert_eq!(out.inline[0].style.foreground, Some(RED));
        let mut out = LineDecorations::default();
        layer.decorate(7, "    ok", &mut out);
        assert_eq!(out.inline[0].style.bold, Some(true));
    }

    #[test]
    fn edits_remap_clean_styles_and_split_inactive_regions() {
        use rove_base::Span;
        use rove_buffer::edit::LineChange;

        let mut layer = SemanticLayer::default();
        layer.update(styles(&[
            token(1, 0, 3, "field", &[]),
            token(3, 0, 3, "field", &[]),
            token(4, 0, 3, "field", &[]),
            token(7, 0, 3, "field", &[]),
        ]));
        layer.update_inactive(vec![1..6]);
        let lines = |start, end| Span::new(Line(start), Line(end)).unwrap();
        layer.remap(&LineChanges {
            ranges: vec![
                LineChange {
                    old: lines(1, 2),
                    new: lines(1, 3),
                },
                LineChange {
                    old: lines(4, 5),
                    new: lines(5, 6),
                },
            ],
            line_delta: 1,
        });
        assert_eq!(layer.styles.lines().collect::<Vec<_>>(), [4, 8]);
        assert_eq!(layer.inactive, [3..5, 6..7]);
    }

    #[test]
    fn only_the_covered_lines_are_invalidated() {
        let mut layer = SemanticLayer::default();
        layer.update(styles(&[token(3, 0, 3, "field", &[])]));
        // 新结果覆盖第 10 行：第 3 行（旧的）与第 10 行（新的）都要失效，其余不动。
        let invalid = layer.update(styles(&[token(10, 0, 3, "field", &[])]));
        assert_eq!(invalid, vec![3..4, 10..11]);
        // 连续行合并成一段。
        let invalid = layer.update(styles(&[
            token(20, 0, 3, "field", &[]),
            token(21, 0, 3, "field", &[]),
            token(22, 0, 3, "field", &[]),
        ]));
        assert_eq!(invalid, vec![10..11, 20..23]);
    }

    #[test]
    fn long_lines_are_left_to_the_syntax_layer() {
        let mut layer = SemanticLayer::default();
        layer.max_line_length = Some(10);
        layer.update(styles(&[token(0, 0, 3, "field", &[])]));
        let mut out = LineDecorations::default();
        layer.decorate(0, &"x".repeat(11), &mut out);
        assert!(out.inline.is_empty());
        let mut out = LineDecorations::default();
        layer.decorate(0, "xxxx", &mut out);
        assert_eq!(out.inline.len(), 1);
    }

    #[test]
    fn inactive_regions_dim_the_whole_line() {
        let mut layer = SemanticLayer::default();
        layer.unnecessary_opacity = Some(0xaa);
        let invalid = layer.update_inactive(vec![4..7]);
        assert_eq!(invalid, vec![4..7]);
        let mut out = LineDecorations::default();
        layer.decorate(5, "#if UNITY_EDITOR", &mut out);
        assert_eq!(out.line.opacity, Some(0xaa));
        let mut out = LineDecorations::default();
        layer.decorate(8, "normal", &mut out);
        assert_eq!(out.line.opacity, None);
        // 置灰与超长行无关：超长的不生效行照样变淡。
        layer.max_line_length = Some(2);
        let mut out = LineDecorations::default();
        layer.decorate(5, "very long line", &mut out);
        assert_eq!(out.line.opacity, Some(0xaa));
    }

    #[test]
    fn a_disabled_layer_draws_nothing() {
        let mut layer = SemanticLayer::default();
        layer.level_allows = false;
        layer.unnecessary_opacity = Some(0xaa);
        layer.update(styles(&[token(0, 0, 3, "field", &[])]));
        layer.update_inactive(vec![0..2]);
        let mut out = LineDecorations::default();
        layer.decorate(0, "abc", &mut out);
        assert!(out.inline.is_empty());
        assert_eq!(out.line.opacity, None);
    }

    #[test]
    fn unnecessary_opacity_comes_from_the_theme_colors_alpha() {
        assert_eq!(
            opacity_of(Some(Color(0, 0, 0, 0xaa)), ThemeKind::Dark),
            Some(0xaa)
        );
        assert_eq!(opacity_of(None, ThemeKind::Dark), Some(0xaa));
        assert_eq!(opacity_of(None, ThemeKind::Light), Some(0x77));
        assert_eq!(opacity_of(None, ThemeKind::HighContrastDark), None);
    }
}
