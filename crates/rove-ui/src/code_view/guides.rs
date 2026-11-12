//! 缩进参考线（`editor.guides.indentation`）与空白字符显示（`editor.renderWhitespace`）的纯计算，
//! 不依赖 GPUI；绘制在 `element.rs`。

use super::columns::{next_tab_stop, DisplayLine};
use std::ops::Range;

/// 行首空白的可见列数；空行或只有空白的行为 `None`。
pub fn indent_columns(source: &str, tab_size: u32) -> Option<u32> {
    let mut col = 0;
    for c in source.chars() {
        match c {
            ' ' => col += 1,
            '\t' => col = next_tab_stop(col, tab_size),
            _ => return Some(col),
        }
    }
    None
}

/// 有内容的行的参考线条数（每个缩进单位一条，部分缩进也算一条）。
pub fn levels(indent_columns: u32, tab_size: u32) -> u32 {
    indent_columns.div_ceil(tab_size.max(1))
}

/// 空行的参考线条数：由上下最近的有内容行决定（VS Code `getLinesIndentGuides` 的规则）。
pub fn blank_line_levels(above: Option<u32>, below: Option<u32>, off_side: bool) -> u32 {
    match (above, below) {
        (Some(a), Some(b)) if a < b => a + 1,
        (Some(a), Some(b)) if a == b => b,
        (Some(_), Some(b)) => {
            if off_side {
                b
            } else {
                b + 1
            }
        }
        _ => 0,
    }
}

/// `editor.renderWhitespace`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderWhitespace {
    None,
    Boundary,
    Selection,
    Trailing,
    All,
}

/// 一行里要画空白符号的位置：(字素簇边界下标, 是否 Tab)。`selection` 是本行被选中的源字节区间。
pub fn whitespace_marks(
    source: &str,
    display: &DisplayLine,
    mode: RenderWhitespace,
    selection: Option<Range<u32>>,
) -> Vec<(usize, bool)> {
    if mode == RenderWhitespace::None {
        return Vec::new();
    }
    let stops = display.stops();
    let bytes = source.as_bytes();
    let first_text = source
        .find(|c: char| c != ' ' && c != '\t')
        .unwrap_or(source.len()) as u32;
    let last_text = source
        .rfind(|c: char| c != ' ' && c != '\t')
        .map_or(0, |i| i as u32 + 1);
    let mut out = Vec::new();
    for i in 0..stops.len().saturating_sub(1) {
        let src = stops[i].src;
        let end = stops[i + 1].src;
        if end != src + 1 {
            continue;
        }
        let tab = match bytes.get(src as usize) {
            Some(b' ') => false,
            Some(b'\t') => true,
            _ => continue,
        };
        let show = match mode {
            RenderWhitespace::All => true,
            RenderWhitespace::Trailing => src >= last_text,
            RenderWhitespace::Selection => selection.as_ref().is_some_and(|r| r.contains(&src)),
            RenderWhitespace::Boundary => {
                // 除了单词之间的单个空格，其余空白都画。
                let leading_or_trailing = src < first_text || src >= last_text;
                let run = |j: u32| matches!(bytes.get(j as usize), Some(b' ' | b'\t'));
                let single = !tab && !(src > 0 && run(src - 1)) && !run(src + 1);
                leading_or_trailing || !single
            }
            RenderWhitespace::None => false,
        };
        if show {
            out.push((i, tab));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(s: &str, mode: RenderWhitespace, sel: Option<Range<u32>>) -> Vec<(usize, bool)> {
        whitespace_marks(s, &DisplayLine::build(s, 4, None), mode, sel)
    }

    #[test]
    fn four_whitespace_modes() {
        let s = "\tint  a = b; ";
        assert!(marks(s, RenderWhitespace::None, None).is_empty());
        assert_eq!(
            marks(s, RenderWhitespace::All, None),
            vec![
                (0, true),
                (4, false),
                (5, false),
                (7, false),
                (9, false),
                (12, false)
            ]
        );
        // boundary：行首 Tab、两个连续空格、行尾空格；单个空格不画。
        assert_eq!(
            marks(s, RenderWhitespace::Boundary, None),
            vec![(0, true), (4, false), (5, false), (12, false)]
        );
        assert_eq!(
            marks(s, RenderWhitespace::Selection, Some(4..8)),
            vec![(4, false), (5, false), (7, false)]
        );
        assert_eq!(
            marks(s, RenderWhitespace::Trailing, None),
            vec![(12, false)]
        );
    }

    #[test]
    fn indentation_levels() {
        assert_eq!(indent_columns("\t  x", 4), Some(6));
        assert_eq!(indent_columns("   ", 4), None);
        assert_eq!(levels(6, 4), 2);
        assert_eq!(blank_line_levels(Some(1), Some(2), false), 2);
        assert_eq!(blank_line_levels(Some(2), Some(2), false), 2);
        assert_eq!(blank_line_levels(Some(2), Some(1), false), 2);
        assert_eq!(blank_line_levels(None, Some(1), false), 0);
    }
}
