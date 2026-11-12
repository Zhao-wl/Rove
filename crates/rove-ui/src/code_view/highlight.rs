//! 语法着色层（#20）：把 #9 的按行 token（字节区间 + TextMate scope）经 #10 的 scope 匹配解析成
//! 样式（前景、背景、粗体、斜体、下划线、删除线），作为装饰层提交给代码视图。
//!
//! - 结果按行缓存在 [`super::decorations::DecorationStore`] 里，只在快照或主题变化时失效；
//!   新快照带有变化行集合时只让这些行失效（[`SyntaxLayer::update`]）。
//! - 过期结果：快照落后于文本时仍按**快照自己的文本**换算列，失效前继续用旧样式绘制，不闪烁。
//! - `editor.maxTokenizationLineLength`：超过的行不着色（与 VS Code 一样按纯文本显示）。
//! - 粗体/斜体进入整形缓存键（见 `view.rs` 的 `font_runs`），颜色不进入。

use super::decorations::{DecorationProvider, InlineDecoration, LineDecorations, TextStyle};
use rove_base::Line;
use rove_buffer::TextSource;
use rove_syntax::SyntaxSnapshot;
use rove_theme::Theme;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

/// 语法快照及其对应的文本（同一版本）。
pub type SyntaxPair = (Arc<dyn SyntaxSnapshot>, Arc<dyn TextSource>);

/// 着色层的状态（视图持有，提供方只读）。
pub struct SyntaxLayer {
    /// 快照及其对应的文本（版本相同）；快照落后时仍用它换算列。
    pub current: Option<SyntaxPair>,
    pub theme: Arc<dyn Theme>,
    /// `editor.maxTokenizationLineLength`（字符）；`None` 为不限。
    pub max_line_length: Option<usize>,
    /// 级别关闭（超大文件）时为假。
    pub enabled: bool,
}

impl SyntaxLayer {
    pub fn new(theme: Arc<dyn Theme>) -> Self {
        Self {
            current: None,
            theme,
            max_line_length: Some(20_000),
            enabled: true,
        }
    }

    /// 换快照。返回需要失效的行：`None` 为全部，`Some(ranges)` 为这些行（新快照坐标）。
    /// `changed` 来自 `SyntaxUpdate::changed_lines`：`None` 表示全部变化（首次解析或丢失基线）。
    pub fn update(
        &mut self,
        snapshot: Option<Arc<dyn SyntaxSnapshot>>,
        text: Option<Arc<dyn TextSource>>,
        changed: Option<&[rove_base::LineRange]>,
    ) -> Option<Vec<Range<u64>>> {
        let had = self.current.is_some();
        self.current = snapshot.zip(text);
        match (had, &self.current, changed) {
            // 首次有结果、或结果被撤掉：整体替换。
            (false, _, _) | (_, None, _) | (_, _, None) => None,
            (true, Some(_), Some(ranges)) => Some(
                ranges
                    .iter()
                    .map(|r| r.start().0..r.end().0.max(r.start().0 + 1))
                    .collect(),
            ),
        }
    }

    /// 某行源文本的样式区间（列为该行内的 UTF-8 字节偏移）。
    pub fn styles(&self, line: u64, source: &str, out: &mut Vec<InlineDecoration>) {
        if !self.enabled {
            return;
        }
        let Some((snapshot, text)) = &self.current else {
            return;
        };
        if self
            .max_line_length
            .is_some_and(|max| source.len() > max && source.chars().count() > max)
        {
            return;
        }
        let (Ok(range), Ok(tokens)) = (text.line_range(Line(line)), snapshot.tokens(Line(line)))
        else {
            return;
        };
        let (line_start, line_end) = (range.start().0, range.end().0);
        let len = source.len() as u64;
        for token in tokens.iter() {
            let start = token.range.start().0.max(line_start) - line_start;
            let end = token
                .range
                .end()
                .0
                .min(line_end)
                .saturating_sub(line_start)
                .min(len);
            if start >= end {
                continue;
            }
            let s = self.theme.token_style(&token.scopes);
            let fg = s.foreground;
            let style = TextStyle {
                foreground: fg,
                background: s.background,
                bold: s.bold.then_some(true),
                italic: s.italic.then_some(true),
                underline: if s.underline { fg } else { None },
                strikethrough: if s.strikethrough { fg } else { None },
                border: None,
            };
            if style == TextStyle::default() {
                continue;
            }
            out.push(InlineDecoration {
                range: start as u32..end as u32,
                style,
            });
        }
    }

    /// 某行某列（UTF-8 字节）所在 token 的 scope 是否属于字符串或注释（括号匹配跳过这些位置）。
    pub fn in_string_or_comment(&self, line: u64, column: u32) -> bool {
        let Some((snapshot, text)) = &self.current else {
            return false;
        };
        let (Ok(range), Ok(tokens)) = (text.line_range(Line(line)), snapshot.tokens(Line(line)))
        else {
            return false;
        };
        let at = range.start().0 + column as u64;
        tokens
            .iter()
            .find(|t| t.range.start().0 <= at && at < t.range.end().0)
            .is_some_and(|t| {
                t.scopes
                    .iter()
                    .any(|s| s.starts_with("string") || s.starts_with("comment"))
            })
    }
}

/// 着色层的装饰提供方。
pub struct SyntaxProvider(pub Rc<RefCell<SyntaxLayer>>);

impl DecorationProvider for SyntaxProvider {
    fn decorate(&self, line: u64, text: &str, out: &mut LineDecorations) {
        self.0.borrow().styles(line, text, &mut out.inline);
    }
}
