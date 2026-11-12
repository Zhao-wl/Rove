//! 括号匹配（#20，`editor.matchBrackets`）：按 #12 的语言配置 `brackets` 找光标两侧的括号及其配对；
//! `always` 时光标不挨着括号也高亮包围它的那一对（VS Code 行为）。字符串与注释里的括号不参与匹配
//! （由调用方根据语法 token 判断）。扫描范围有上限，不会在超大文件上卡住 UI 线程。

use rove_syntax::Pair;
use std::ops::Range;
use std::sync::Arc;

/// 每个方向最多扫描的行数。
pub const SCAN_LINES: u64 = 5_000;

/// `editor.matchBrackets`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchBrackets {
    Always,
    Near,
    Never,
}

/// 一对匹配的括号：(行, 该行内 UTF-8 字节区间)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BracketMatch {
    pub open: (u64, Range<u32>),
    pub close: (u64, Range<u32>),
}

impl BracketMatch {
    pub fn lines(&self) -> [u64; 2] {
        [self.open.0, self.close.0]
    }
}

#[derive(Clone, Copy, Debug)]
struct Occ {
    col: u32,
    len: u32,
    pair: usize,
    open: bool,
}

/// 一行里的括号（同一位置取最长的符号，例如 XML 的 `<!--` 优先于 `<`）。
fn scan_line(text: &str, symbols: &[(Arc<str>, usize, bool)]) -> Vec<Occ> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &text[i..];
        if let Some((s, pair, open)) = symbols.iter().find(|(s, _, _)| rest.starts_with(&**s)) {
            out.push(Occ {
                col: i as u32,
                len: s.len() as u32,
                pair: *pair,
                open: *open,
            });
            i += s.len();
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// 查找光标处的括号匹配。`read(line)` 读一行源文本（越界返回 `None`），`skip(line, col)` 为真表示
/// 该位置在字符串或注释里。
pub fn find(
    pairs: &[Pair],
    line: u64,
    column: u32,
    mode: MatchBrackets,
    read: &dyn Fn(u64) -> Option<Arc<str>>,
    skip: &dyn Fn(u64, u32) -> bool,
) -> Option<BracketMatch> {
    if mode == MatchBrackets::Never || pairs.is_empty() {
        return None;
    }
    let mut symbols: Vec<(Arc<str>, usize, bool)> = Vec::new();
    for (i, p) in pairs.iter().enumerate() {
        if p.open.is_empty() || p.close.is_empty() {
            continue;
        }
        symbols.push((p.open.clone(), i, true));
        symbols.push((p.close.clone(), i, false));
    }
    symbols.sort_by_key(|s| std::cmp::Reverse(s.0.len()));
    let occs = |l: u64| -> Option<Vec<Occ>> {
        let text = read(l)?;
        Some(
            scan_line(&text, &symbols)
                .into_iter()
                .filter(|o| !skip(l, o.col))
                .collect(),
        )
    };
    let here = occs(line)?;
    // 先看光标左边紧挨着的括号，再看右边。
    let adjacent = here
        .iter()
        .find(|o| o.col + o.len == column)
        .or_else(|| here.iter().find(|o| o.col == column))
        .copied();
    if let Some(o) = adjacent {
        return matching(line, o, &occs);
    }
    if mode == MatchBrackets::Near {
        return None;
    }
    enclosing(line, column, &here, &occs)
}

fn matching(line: u64, o: Occ, occs: &dyn Fn(u64) -> Option<Vec<Occ>>) -> Option<BracketMatch> {
    let me = (line, o.col..o.col + o.len);
    let mut depth = 1u32;
    if o.open {
        for l in line..line.saturating_add(SCAN_LINES) {
            let list = occs(l)?;
            for c in list.iter().filter(|c| c.pair == o.pair) {
                if l == line && c.col <= o.col {
                    continue;
                }
                if c.open {
                    depth += 1;
                } else {
                    depth -= 1;
                    if depth == 0 {
                        return Some(BracketMatch {
                            open: me,
                            close: (l, c.col..c.col + c.len),
                        });
                    }
                }
            }
        }
    } else {
        let lo = line.saturating_sub(SCAN_LINES);
        for l in (lo..=line).rev() {
            let list = occs(l)?;
            for c in list.iter().rev().filter(|c| c.pair == o.pair) {
                if l == line && c.col >= o.col {
                    continue;
                }
                if c.open {
                    depth -= 1;
                    if depth == 0 {
                        return Some(BracketMatch {
                            open: (l, c.col..c.col + c.len),
                            close: me,
                        });
                    }
                } else {
                    depth += 1;
                }
            }
        }
    }
    None
}

/// 包围光标的最近一对括号（向前找未配对的开括号，再向后找它的配对）。
fn enclosing(
    line: u64,
    column: u32,
    here: &[Occ],
    occs: &dyn Fn(u64) -> Option<Vec<Occ>>,
) -> Option<BracketMatch> {
    let mut depth: Vec<u32> = Vec::new();
    let lo = line.saturating_sub(SCAN_LINES);
    for l in (lo..=line).rev() {
        let list = if l == line { here.to_vec() } else { occs(l)? };
        for c in list.iter().rev() {
            if l == line && c.col + c.len > column {
                continue;
            }
            if depth.len() <= c.pair {
                depth.resize(c.pair + 1, 0);
            }
            if !c.open {
                depth[c.pair] += 1;
            } else if depth[c.pair] > 0 {
                depth[c.pair] -= 1;
            } else {
                // 未配对的开括号：它的配对若在光标之后，就是包围光标的一对。
                let m = matching(l, *c, occs)?;
                let (cl, cr) = (&m.close.0, &m.close.1);
                return (*cl > line || (*cl == line && cr.start >= column)).then_some(m);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<Pair> {
        list.iter()
            .map(|(o, c)| Pair {
                open: (*o).into(),
                close: (*c).into(),
            })
            .collect()
    }

    fn run(text: &str, line: u64, col: u32, mode: MatchBrackets) -> Option<BracketMatch> {
        let lines: Vec<Arc<str>> = text.split('\n').map(Arc::from).collect();
        let p = pairs(&[
            ("{", "}"),
            ("[", "]"),
            ("(", ")"),
            ("<!--", "-->"),
            ("<", ">"),
        ]);
        find(
            &p,
            line,
            col,
            mode,
            &|l| lines.get(l as usize).cloned(),
            &|_, _| false,
        )
    }

    #[test]
    fn both_sides_of_the_caret() {
        let t = "f(a, (b))";
        // 光标在 `(` 右侧（左边挨着开括号）。
        let m = run(t, 0, 2, MatchBrackets::Near).unwrap();
        assert_eq!((m.open.1, m.close.1), (1..2, 8..9));
        // 光标在 `)` 左侧（右边挨着闭括号）。
        let m = run(t, 0, 7, MatchBrackets::Near).unwrap();
        assert_eq!((m.open.1, m.close.1), (5..6, 7..8));
        // 不挨着括号：near 不高亮，always 高亮包围的一对。
        assert!(run(t, 0, 3, MatchBrackets::Near).is_none());
        let m = run(t, 0, 3, MatchBrackets::Always).unwrap();
        assert_eq!((m.open.1, m.close.1), (1..2, 8..9));
        assert!(run(t, 0, 3, MatchBrackets::Never).is_none());
    }

    #[test]
    fn multi_line_and_longest_symbol() {
        let t = "<!-- x\n-->\n<a>\n</a>";
        let m = run(t, 0, 0, MatchBrackets::Near).unwrap();
        assert_eq!((m.open, m.close), ((0, 0..4), (1, 0..3)));
        let m = run(t, 3, 4, MatchBrackets::Near).unwrap();
        assert_eq!((m.open, m.close), ((3, 0..1), (3, 3..4)));
    }

    #[test]
    fn skipped_positions_do_not_count() {
        let lines: Vec<Arc<str>> = vec!["{ \"}\" }".into()];
        let p = pairs(&[("{", "}")]);
        let m = find(
            &p,
            0,
            1,
            MatchBrackets::Near,
            &|l| lines.get(l as usize).cloned(),
            &|_, c| (2..5).contains(&c),
        )
        .unwrap();
        assert_eq!(m.close.1, 6..7);
    }
}
