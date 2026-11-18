//! 缩进参考线（`editor.guides.indentation`）与空白字符显示（`editor.renderWhitespace`）的纯计算，
//! 不依赖 GPUI；绘制在 `element.rs`。
//!
//! #57 第二部分在这里补上两件事（同样只做纯计算）：
//!
//! - **括号对参考线**（`editor.guides.bracketPairs` / `editor.guides.bracketPairsHorizontal`）：
//!   把 `bracket_pairs::query::GuidePair` 摊成逐行的 [`BracketGuide`]，括号对树的查询在
//!   `bracket_pairs/query.rs`，可见列换算在 `view.rs`（要读行文本）。
//! - **活动缩进参考线**（`editor.guides.highlightActiveIndentation`）：[`active_indent_guide`]
//!   给出「哪一条缩进参考线是活动的」，绘制仍在 `element.rs`。

use super::columns::{next_tab_stop, DisplayLine};
use std::ops::Range;

/// 空行的参考线条数要看上下最近的有内容行，最多往两边各找这么多行。
pub const GUIDE_SEARCH: u64 = 64;

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

/// 一行的参考线条数：有内容的行直接数缩进，空行按 [`blank_line_levels`] 由上下最近的
/// 有内容行推定。`content_levels` 只对有内容的行返回 `Some`。
pub fn effective_levels(
    line: u64,
    line_count: u64,
    search: u64,
    off_side: bool,
    content_levels: &dyn Fn(u64) -> Option<u32>,
) -> u32 {
    if let Some(levels) = content_levels(line) {
        return levels;
    }
    let above = (line.saturating_sub(search)..line)
        .rev()
        .find_map(content_levels);
    let below = (line + 1..(line + search).min(line_count)).find_map(content_levels);
    blank_line_levels(above, below, off_side)
}

// ------------------------------------------------------------------ 活动缩进参考线（#57）

/// `editor.guides.highlightActiveIndentation` 的三个取值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActiveIndentMode {
    /// `false`：不高亮。
    Off,
    /// `true`：高亮；但括号对参考线已经标出活动括号对时让位给它（VS Code 的说明）。
    #[default]
    On,
    /// `"always"`：即使括号对参考线也在高亮，缩进参考线照样高亮。
    Always,
}

/// 活动缩进参考线：第几条（0 起，可见列为 `level * tab_size`）以及高亮覆盖的行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveIndent {
    pub level: u32,
    /// 闭开区间。
    pub lines: Range<u64>,
}

/// 判定光标所在层级的缩进参考线（VS Code `TextModel.getActiveIndentGuide` 的等价实现）。
///
/// 规则：
/// 1. 取光标所在行的参考线条数 `base`（空行按上下最近的有内容行推定）；
/// 2. 光标那一行如果是**块的开头**（下一条有内容的行更深），活动的是它开出来的那一层，
///    即 `base` 条参考线里的第 `base` 条（下标从 0 起，比本行多一条）；否则是本行最里面的
///    那条，即下标 `base - 1`；`base` 为 0 且不开块时没有活动参考线；
/// 3. 高亮范围是锚点行上下「参考线条数 ≥ 目标层数」的连续行，**截到 `clamp` 为止**
///    （与 VS Code 一样只在可见范围里找，所以代价与视口大小成正比，与文档大小无关）。
pub fn active_indent_guide(
    cursor_line: u64,
    line_count: u64,
    clamp: Range<u64>,
    search: u64,
    off_side: bool,
    content_levels: &dyn Fn(u64) -> Option<u32>,
) -> Option<ActiveIndent> {
    if line_count == 0 || clamp.is_empty() {
        return None;
    }
    let levels = |line: u64| effective_levels(line, line_count, search, off_side, content_levels);
    let base = levels(cursor_line);
    // 下一条有内容的行更深 → 光标这一行开了一个块。
    let next = (cursor_line + 1..(cursor_line + search).min(line_count)).find_map(content_levels);
    let opens_block = next.is_some_and(|n| n > base);
    let target = if opens_block { base + 1 } else { base };
    if target == 0 {
        return None;
    }
    let anchor = if base >= target {
        cursor_line
    } else {
        cursor_line + 1
    };
    if anchor >= line_count {
        return None;
    }
    let lo = clamp.start;
    let hi = clamp.end.min(line_count);
    // 从锚点往两边走，走到参考线变浅为止；两个方向都只走到可见范围的边。锚点在可见范围之外时
    // 只会往可见范围那一侧走（与 VS Code 一样），块够不到可见范围就等于没有高亮。
    let (mut start, mut end) = (anchor, anchor);
    while start > lo && levels(start - 1) >= target {
        start -= 1;
    }
    while end + 1 < hi && levels(end + 1) >= target {
        end += 1;
    }
    Some(ActiveIndent {
        level: target - 1,
        lines: start..end + 1,
    })
}

// ------------------------------------------------------------------ 括号对参考线（#57）

/// `editor.guides.bracketPairs` / `editor.guides.bracketPairsHorizontal` 的三个取值。
///
/// 两者**不是互相独立的**：`bracketPairs` 为 `false` 时一条括号对参考线都不画，水平线也不画
/// （VS Code 的 `indentGuides.ts` 只在 `bracketPairs !== false` 时才去取括号对参考线，
/// 所以默认设置 `false` + `"active"` 下什么都看不到）。`bracketPairsHorizontal` 只决定
/// **已经要画的那些括号对**里哪些再加一条水平连接线。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GuideMode {
    /// `false`
    #[default]
    Off,
    /// `"active"`：只画光标所在的那一对。
    Active,
    /// `true`
    On,
}

impl GuideMode {
    /// 这一对括号要不要画。
    pub fn wants(self, active: bool) -> bool {
        match self {
            GuideMode::Off => false,
            GuideMode::Active => active,
            GuideMode::On => true,
        }
    }
}

/// 竖线在一行里覆盖的范围。左括号那一行只画下半截、右括号那一行只画上半截，
/// 于是整条参考线正好起于左括号、止于右括号。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalPart {
    None,
    /// 整行。
    Full,
    /// 行高中线到行底。
    Lower,
    /// 行顶到行高中线。
    Upper,
}

/// 水平连接线：从竖线画到 `to_column` 这个可见列。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Horizontal {
    pub to_column: u32,
    /// 贴着行底画（右括号前面还有别的内容时），否则画在行高中线上。
    pub at_bottom: bool,
}

/// 括号对参考线在某一行上的一段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketGuide {
    pub line: u64,
    /// 竖线所在的可见列。
    pub column: u32,
    /// 嵌套深度（1 起，与着色同一套计数）。
    pub level: u32,
    pub level_of_kind: u32,
    /// 是不是光标所在的那一对。
    pub active: bool,
    pub vertical: VerticalPart,
    pub horizontal: Option<Horizontal>,
}

/// 一对括号对在屏幕上的几何量（可见列由调用方按行文本换算好）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairGeometry {
    pub open_line: u64,
    pub open_column: u32,
    pub close_line: u64,
    pub close_column: u32,
    /// 右括号是不是所在行的第一个非空白字符（否则水平线贴行底，从内容下面穿过）。
    pub close_is_first: bool,
    pub level: u32,
    pub level_of_kind: u32,
    pub active: bool,
}

/// 把一对括号对摊成 `visible` 区间里逐行的参考线段。
///
/// 调用方只在这一对要画的时候才调过来（`editor.guides.bracketPairs` 的判定），所以竖线一定画；
/// `horizontal` 是 `editor.guides.bracketPairsHorizontal` 对这一对的判定。
///
/// 竖线画在 `min(左括号可见列, 右括号可见列)` 上（VS Code 还会再和括号对内部的最小缩进取一次
/// 最小值；那一步要扫过整对括号的所有行，代价与括号对长度成正比，而且只在「内容比括号本身更靠左」
/// 这种少见的排版下才会生效，这里不做，见 `ARCHITECTURE.md` 的说明）。
pub fn bracket_guide_lines(
    pair: &PairGeometry,
    visible: Range<u64>,
    horizontal: bool,
    out: &mut Vec<BracketGuide>,
) {
    let mut push = |line: u64, column: u32, v: VerticalPart, h: Option<Horizontal>| {
        if !visible.contains(&line) || (v == VerticalPart::None && h.is_none()) {
            return;
        }
        out.push(BracketGuide {
            line,
            column,
            level: pair.level,
            level_of_kind: pair.level_of_kind,
            active: pair.active,
            vertical: v,
            horizontal: h,
        });
    };
    if pair.open_line == pair.close_line {
        // 单行括号对只有一条水平线（没有竖线可画）。
        if horizontal {
            push(
                pair.open_line,
                pair.open_column,
                VerticalPart::None,
                Some(Horizontal {
                    to_column: pair.close_column,
                    at_bottom: false,
                }),
            );
        }
        return;
    }
    let column = pair.open_column.min(pair.close_column);
    push(
        pair.open_line,
        column,
        VerticalPart::Lower,
        horizontal.then_some(Horizontal {
            to_column: pair.open_column,
            at_bottom: false,
        }),
    );
    let first = (pair.open_line + 1).max(visible.start);
    let last = pair.close_line.min(visible.end);
    for line in first..last {
        push(line, column, VerticalPart::Full, None);
    }
    let at_bottom = !pair.close_is_first;
    push(
        pair.close_line,
        column,
        if at_bottom {
            VerticalPart::Full
        } else {
            VerticalPart::Upper
        },
        horizontal.then_some(Horizontal {
            to_column: pair.close_column,
            at_bottom,
        }),
    );
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

    // ------------------------------------------------------------------ 活动缩进参考线（#57）

    const BLOCK: &[&str] = &[
        "class C {",
        "    void M() {",
        "        a();",
        "",
        "        b();",
        "    }",
        "}",
    ];

    fn active(cursor: u64, clamp: Range<u64>) -> Option<ActiveIndent> {
        let content = |line: u64| {
            BLOCK
                .get(line as usize)
                .and_then(|s| indent_columns(s, 4))
                .map(|c| levels(c, 4))
        };
        active_indent_guide(cursor, BLOCK.len() as u64, clamp, 64, false, &content)
    }

    #[test]
    fn the_active_indent_guide_follows_the_cursor_level() {
        let all = 0..BLOCK.len() as u64;
        // 块里面：活动的是本行最里面那条（下标 1，可见列 4）。
        assert_eq!(
            active(2, all.clone()),
            Some(ActiveIndent {
                level: 1,
                lines: 2..5
            })
        );
        // 空行也算在块里。
        assert_eq!(active(3, all.clone()).unwrap().level, 1);
        // 光标在块的开头那一行：活动的是它开出来的那一层，与块里一致。
        assert_eq!(
            active(1, all.clone()),
            Some(ActiveIndent {
                level: 1,
                lines: 2..5
            })
        );
        // 最外层：下标 0，覆盖整个类体。
        assert_eq!(
            active(0, all.clone()),
            Some(ActiveIndent {
                level: 0,
                lines: 1..6
            })
        );
        // 不在任何块里（也不开块）：没有活动参考线。
        assert_eq!(active(6, all), None);
    }

    #[test]
    fn the_active_indent_guide_is_clamped_to_the_visible_range() {
        // 视口只到第 4 行：高亮也只算到那里。
        assert_eq!(active(2, 0..4).unwrap().lines, 2..4);
        // 光标在视口上方：只往下走，块够得到视口就照样高亮。
        assert_eq!(active(2, 4..7).unwrap().lines, 2..5);
        // 光标在视口上方、块够不到视口：范围与视口不相交。
        let out = active(2, 5..7).unwrap();
        assert!(out.lines.end <= 5, "{:?}", out.lines);
        assert_eq!(active(2, 2..2), None);
    }

    // ------------------------------------------------------------------ 括号对参考线（#57）

    fn geometry(open: (u64, u32), close: (u64, u32), close_is_first: bool) -> PairGeometry {
        PairGeometry {
            open_line: open.0,
            open_column: open.1,
            close_line: close.0,
            close_column: close.1,
            close_is_first,
            level: 2,
            level_of_kind: 1,
            active: true,
        }
    }

    fn lines(
        pair: &PairGeometry,
        visible: Range<u64>,
        horizontal: bool,
    ) -> Vec<(u64, u32, VerticalPart, Option<Horizontal>)> {
        let mut out = Vec::new();
        bracket_guide_lines(pair, visible, horizontal, &mut out);
        out.into_iter()
            .map(|g| (g.line, g.column, g.vertical, g.horizontal))
            .collect()
    }

    #[test]
    fn a_multiline_pair_starts_at_the_open_bracket_and_ends_at_the_close_bracket() {
        // `    if (x) {` … `    }`：竖线落在两个括号里靠左的那个（右括号，可见列 4）。
        let pair = geometry((0, 11), (3, 4), true);
        assert_eq!(
            lines(&pair, 0..4, true),
            vec![
                (
                    0,
                    4,
                    VerticalPart::Lower,
                    Some(Horizontal {
                        to_column: 11,
                        at_bottom: false
                    })
                ),
                (1, 4, VerticalPart::Full, None),
                (2, 4, VerticalPart::Full, None),
                (
                    3,
                    4,
                    VerticalPart::Upper,
                    Some(Horizontal {
                        to_column: 4,
                        at_bottom: false
                    })
                ),
            ]
        );
    }

    #[test]
    fn the_bottom_line_hugs_the_line_bottom_when_the_close_bracket_is_not_first() {
        // `    b);`：右括号前面还有内容，水平线贴行底，竖线整行。
        let pair = geometry((0, 3), (2, 5), false);
        let out = lines(&pair, 0..3, true);
        assert_eq!(
            out[2],
            (
                2,
                3,
                VerticalPart::Full,
                Some(Horizontal {
                    to_column: 5,
                    at_bottom: true
                })
            )
        );
    }

    #[test]
    fn single_line_pairs_only_get_a_horizontal_line() {
        let pair = geometry((1, 4), (1, 9), false);
        assert_eq!(
            lines(&pair, 0..3, true),
            vec![(
                1,
                4,
                VerticalPart::None,
                Some(Horizontal {
                    to_column: 9,
                    at_bottom: false
                })
            )]
        );
        // 水平线关掉之后单行括号对什么都不画。
        assert!(lines(&pair, 0..3, false).is_empty());
    }

    #[test]
    fn horizontal_lines_can_be_switched_off_on_their_own() {
        // `bracketPairsHorizontal` 只决定加不加水平线；竖线由 `bracketPairs` 在调用方那里判定，
        // 走到这里就一定要画（整对四行都有竖线，一条水平线都没有）。
        let pair = geometry((0, 11), (3, 4), true);
        let only_vertical = lines(&pair, 0..4, false);
        assert_eq!(only_vertical.len(), 4);
        assert!(only_vertical.iter().all(|g| g.3.is_none()));
    }

    #[test]
    fn only_the_visible_lines_are_emitted() {
        let pair = geometry((0, 11), (9, 4), true);
        assert_eq!(
            lines(&pair, 3..6, true)
                .iter()
                .map(|g| g.0)
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
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
