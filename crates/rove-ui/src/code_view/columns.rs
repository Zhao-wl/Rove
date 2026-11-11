//! 行内坐标：字素簇边界、可见列（VS Code 语义）、Tab 展开、单行渲染上限与像素换算。
//!
//! 一切按**源行**的 UTF-8 字节列（[`rove_base::Utf8Column`]）对外；内部另有一份
//! “显示文本”（Tab 展开成空格、超过渲染上限的部分截掉）交给整形。两者之间的映射、
//! 以及每个字素簇边界的 x 坐标都在这里，不依赖 GPUI，可单独测试。

use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

/// VS Code `strings.isFullWidthCharacter`：可见列按 2 计。
pub fn is_full_width(c: char) -> bool {
    let c = c as u32;
    (0x2E80..=0xD7AF).contains(&c)
        || (0xF900..=0xFAFF).contains(&c)
        || (0xFF01..=0xFF5E).contains(&c)
}

/// VS Code `strings.isEmojiImprecise`：可见列按 2 计。
pub fn is_emoji_imprecise(c: char) -> bool {
    let x = c as u32;
    (0x1F1E6..=0x1F1FF).contains(&x)
        || x == 8986
        || x == 8987
        || x == 9200
        || x == 9203
        || (9728..=10175).contains(&x)
        || x == 11088
        || x == 11093
        || (127744..=128591).contains(&x)
        || (128640..=128764).contains(&x)
        || (128992..=129008).contains(&x)
        || (129280..=129535).contains(&x)
        || (129648..=129782).contains(&x)
}

/// 一个字素簇占的可见列（Tab 另算）。
fn grapheme_cells(g: &str) -> u32 {
    match g.chars().next() {
        Some(c) if is_full_width(c) || is_emoji_imprecise(c) => 2,
        _ => 1,
    }
}

/// 下一个 Tab 停靠位。
pub fn next_tab_stop(column: u32, tab_size: u32) -> u32 {
    let tab = tab_size.max(1);
    column + tab - column % tab
}

/// 一个字素簇边界。`src`/`disp` 是该边界在源行/显示文本里的字节偏移，`col` 是可见列。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stop {
    pub src: u32,
    pub disp: u32,
    pub col: u32,
}

/// 一行的显示形式：整形输入 + 源列 ↔ 显示列 ↔ 可见列映射。
///
/// 只为可见行与预取行构建（O(本行字节)），结果随行缓存，绝不针对全文构建。
#[derive(Clone, Debug)]
pub struct DisplayLine {
    /// 交给整形的文本：Tab 展开为空格，超过渲染上限的部分已截掉。
    pub text: Arc<str>,
    /// 源行完整字节长度（不含行终止符）。
    pub source_len: u32,
    /// 是否因单行渲染上限被截断（#22 据此画截断标记）。
    pub truncated: bool,
    /// 字素簇边界，首项恒为 (0,0,0)，末项是最后一个渲染的边界。
    stops: Vec<Stop>,
}

impl DisplayLine {
    /// `limit_chars`：单行渲染上限（字符数，`None` 表示不限）。截断只发生在字素簇边界。
    pub fn build(source: &str, tab_size: u32, limit_chars: Option<usize>) -> Self {
        let mut text = String::with_capacity(source.len());
        let mut stops = Vec::with_capacity(source.len() / 2 + 2);
        stops.push(Stop {
            src: 0,
            disp: 0,
            col: 0,
        });
        let mut col = 0u32;
        let mut chars = 0usize;
        let mut truncated = false;
        for (offset, g) in source.grapheme_indices(true) {
            if let Some(limit) = limit_chars {
                if chars >= limit {
                    truncated = true;
                    break;
                }
            }
            chars += g.chars().count();
            if g == "\t" {
                let next = next_tab_stop(col, tab_size);
                for _ in col..next {
                    text.push(' ');
                }
                col = next;
            } else {
                text.push_str(g);
                col += grapheme_cells(g);
            }
            stops.push(Stop {
                src: (offset + g.len()) as u32,
                disp: text.len() as u32,
                col,
            });
        }
        Self {
            text: text.into(),
            source_len: source.len() as u32,
            truncated,
            stops,
        }
    }

    pub fn stops(&self) -> &[Stop] {
        &self.stops
    }

    /// 已渲染部分覆盖的源字节数。
    pub fn rendered_source_len(&self) -> u32 {
        self.stops.last().map_or(0, |s| s.src)
    }

    /// 源列所在（或之前最近）的边界下标；落在字素簇中间时取簇首。
    pub fn stop_index_for_source(&self, src: u32) -> usize {
        match self.stops.binary_search_by_key(&src, |s| s.src) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    /// 显示字节偏移所在的边界下标（向前取整）。
    pub fn stop_index_for_display(&self, disp: u32) -> usize {
        match self.stops.binary_search_by_key(&disp, |s| s.disp) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    /// 源列 → 可见列。超出渲染部分的列按“每字节 1 列”外推（只用于上下移动的列记忆）。
    pub fn visible_column(&self, src: u32) -> u32 {
        let i = self.stop_index_for_source(src);
        let s = self.stops[i];
        if i + 1 == self.stops.len() {
            s.col + src.saturating_sub(s.src)
        } else {
            s.col
        }
    }

    /// VS Code `columnFromVisibleColumn`：取离目标可见列最近的边界，落在宽字符/Tab 中间时取近者；
    /// 截断行超出渲染部分时按“每字节 1 列”外推，调用方需再对齐到源行的字符边界。
    pub fn source_for_visible_column(&self, col: u32) -> u32 {
        let i = self.stops.partition_point(|s| s.col <= col);
        let before = self.stops[i.saturating_sub(1)];
        if i == self.stops.len() {
            if self.truncated {
                return (before.src + (col - before.col)).min(self.source_len);
            }
            return before.src;
        }
        if i == 0 || before.col == col {
            return before.src;
        }
        let after = self.stops[i];
        if after.col - col < col - before.col {
            after.src
        } else {
            before.src
        }
    }

    /// 源列 → 显示字节偏移（落在簇中间取簇首；超出渲染部分取显示末尾）。
    pub fn display_for_source(&self, src: u32) -> u32 {
        self.stops[self.stop_index_for_source(src)].disp
    }

    /// 按给定的边界 x 坐标表做命中测试：返回离 `x` 最近的边界对应的源列。
    /// `xs` 必须与 [`Self::stops`] 一一对应且单调不减。
    pub fn source_for_x(&self, xs: &[f32], x: f32) -> u32 {
        debug_assert_eq!(xs.len(), self.stops.len());
        let i = xs.partition_point(|&v| v <= x);
        if i == 0 {
            return 0;
        }
        if i == xs.len() {
            return self.stops[i - 1].src;
        }
        if xs[i] - x < x - xs[i - 1] {
            self.stops[i].src
        } else {
            self.stops[i - 1].src
        }
    }

    /// 源列 → x（按边界 x 坐标表）。
    pub fn x_for_source(&self, xs: &[f32], src: u32) -> f32 {
        xs[self.stop_index_for_source(src)]
    }

    /// 等宽近似的边界 x：`可见列 × 单元宽 + 字间距 × 簇序号`。缓存未命中的占位渲染与命中测试都用它，
    /// 正式整形结果到达后换成真实坐标；行高不变。
    pub fn monospace_xs(&self, cell_width: f32, letter_spacing: f32) -> Vec<f32> {
        self.stops
            .iter()
            .enumerate()
            .map(|(i, s)| s.col as f32 * cell_width + i as f32 * letter_spacing)
            .collect()
    }

    /// 由整形结果的字形位置算出每个边界的 x：`glyphs` 是按显示字节偏移排序的 (偏移, x)，
    /// `width` 是整行宽度。落在连字内部的边界取下一个字形的 x（与 GPUI `x_for_index` 一致）。
    pub fn shaped_xs(&self, glyphs: &[(u32, f32)], width: f32, letter_spacing: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.stops.len());
        let mut g = 0;
        for (i, s) in self.stops.iter().enumerate() {
            while g < glyphs.len() && glyphs[g].0 < s.disp {
                g += 1;
            }
            let x = if g < glyphs.len() { glyphs[g].1 } else { width };
            out.push(x + i as f32 * letter_spacing);
        }
        // 保证单调（双向文本或异常字形位置时退化为不减序列）。
        for i in 1..out.len() {
            if out[i] < out[i - 1] {
                out[i] = out[i - 1];
            }
        }
        out
    }
}

/// 只读一行的源文本，供插入符移动、选词等操作使用；实现方可以是 `TextSource` 或测试里的数组。
pub trait LineSource {
    /// 已知行数（至少 1）。
    fn line_count(&self) -> u64;
    /// 读一行（不含行终止符）。越界返回 `None`。
    fn line_text(&self, line: u64) -> Option<String>;
}

/// 把 `TextSource` 适配成 [`LineSource`]。
pub struct TextLines<'a>(pub &'a dyn rove_buffer::TextSource);

impl LineSource for TextLines<'_> {
    fn line_count(&self) -> u64 {
        self.0.lines().known.max(1)
    }
    fn line_text(&self, line: u64) -> Option<String> {
        read_line(self.0, line)
    }
}

/// 从 `TextSource` 读一行（不含终止符）。
pub fn read_line(text: &dyn rove_buffer::TextSource, line: u64) -> Option<String> {
    let range = text.line_range(rove_base::Line(line)).ok()?;
    let mut out = String::with_capacity((range.end().0 - range.start().0) as usize);
    text.visit_chunks(range, &mut |s| out.push_str(s)).ok()?;
    Some(out)
}

/// 字素簇边界（源字节偏移），含 0 与行尾。
pub fn grapheme_boundaries(line: &str) -> Vec<u32> {
    let mut v: Vec<u32> = line.grapheme_indices(true).map(|(i, _)| i as u32).collect();
    v.push(line.len() as u32);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_expand_to_tab_stops_and_map_back() {
        let line = DisplayLine::build("a\tb\t\tc", 4, None);
        assert_eq!(&*line.text, "a   b       c");
        assert_eq!(line.visible_column(1), 1);
        assert_eq!(line.visible_column(2), 4);
        assert_eq!(line.visible_column(4), 8);
        assert_eq!(line.visible_column(5), 12);
        // 落在 Tab 内部的可见列按距离取前后边界。
        assert_eq!(line.source_for_visible_column(2), 1);
        assert_eq!(line.source_for_visible_column(3), 2);
        assert_eq!(line.display_for_source(2), 4);
    }

    #[test]
    fn cjk_and_emoji_take_two_columns_and_clusters_stay_whole() {
        let family = "👨‍👩‍👧‍👦";
        let skin = "🧑🏽‍💻";
        let src = format!("中a{family}{skin}e\u{301}x");
        let line = DisplayLine::build(&src, 4, None);
        let b = grapheme_boundaries(&src);
        // 中, a, family, skin, é(组合字符), x
        assert_eq!(b.len(), 7);
        assert_eq!(line.stops().len(), 7);
        let cols: Vec<u32> = line.stops().iter().map(|s| s.col).collect();
        assert_eq!(cols, vec![0, 2, 3, 5, 7, 8, 9]);
        // 簇中间的列取簇首。
        let mid_family = 4 + 4; // 进入 ZWJ 序列内部
        assert_eq!(line.display_for_source(mid_family), 4);
    }

    #[test]
    fn hit_test_picks_nearest_boundary() {
        let line = DisplayLine::build("中ab", 4, None);
        let xs = line.monospace_xs(10.0, 0.0);
        assert_eq!(xs, vec![0.0, 20.0, 30.0, 40.0]);
        assert_eq!(line.source_for_x(&xs, -5.0), 0);
        assert_eq!(line.source_for_x(&xs, 9.0), 0);
        assert_eq!(line.source_for_x(&xs, 11.0), 3);
        assert_eq!(line.source_for_x(&xs, 26.0), 4);
        assert_eq!(line.source_for_x(&xs, 1000.0), 5);
        assert_eq!(line.x_for_source(&xs, 4), 30.0);
    }

    #[test]
    fn render_limit_truncates_at_cluster_boundary() {
        let src = "x".repeat(100_000);
        let line = DisplayLine::build(&src, 4, Some(10_000));
        assert!(line.truncated);
        assert_eq!(line.text.len(), 10_000);
        assert_eq!(line.rendered_source_len(), 10_000);
        assert_eq!(line.source_len, 100_000);
        // 超出渲染部分的点击落到渲染末尾。
        let xs = line.monospace_xs(8.0, 0.0);
        assert_eq!(line.source_for_x(&xs, 1e9), 10_000);
    }

    #[test]
    fn shaped_xs_follow_glyph_positions_and_letter_spacing() {
        let line = DisplayLine::build("ab", 4, None);
        let xs = line.shaped_xs(&[(0, 0.0), (1, 7.5)], 15.0, 1.0);
        assert_eq!(xs, vec![0.0, 8.5, 17.0]);
    }
}
