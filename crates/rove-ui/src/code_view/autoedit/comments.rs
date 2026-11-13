//! 注释切换：行注释（`lineCommentCommand.ts`）与块注释（`blockCommentCommand.ts`）。
//!
//! - 多行选区里**只要有一行未注释就全部注释**，否则全部取消（[`CommentType::Toggle`]）。
//! - 注释前缀插在各行的**公共缩进**处（`_normalizeInsertionPoint`），不是行首。
//! - `editor.comments.insertSpace`（默认 true）在前缀后加一个空格，取消注释时也吃掉它。
//! - `editor.comments.ignoreEmptyLines`（默认 true）跳过空行；整段都是空行时照常注释。
//! - 没有行注释的语言退化为块注释（VS Code 的 `LineCommentCommand` 同样会退化）。

use super::super::columns::LineSource;
use super::indent::visible_width;
use super::{first_non_whitespace, AutoEdit, Change};
use rove_syntax::LanguageConfiguration;

/// 注释命令的三种语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentType {
    /// `editor.action.commentLine`
    Toggle,
    /// `editor.action.addCommentLine`
    ForceAdd,
    /// `editor.action.removeCommentLine`
    ForceRemove,
}

/// `editor.comments.*`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommentOptions {
    pub insert_space: bool,
    pub ignore_empty_lines: bool,
    pub tab_size: usize,
}

impl Default for CommentOptions {
    fn default() -> Self {
        Self {
            insert_space: true,
            ignore_empty_lines: true,
            tab_size: 4,
        }
    }
}

#[derive(Clone, Debug)]
struct LineData {
    ignore: bool,
    /// 前缀插入点（行内字节偏移）。
    offset: usize,
    /// 取消注释时要删掉的长度（含 `insertSpace` 吃掉的空格）。
    length: usize,
}

/// 行注释切换的改动。没有 `lineComment` 时返回 `None`（调用方退化为块注释）。
pub fn toggle_line_comment(
    lines: &dyn LineSource,
    config: &LanguageConfiguration,
    first: u64,
    last: u64,
    kind: CommentType,
    options: CommentOptions,
) -> Option<Vec<Change>> {
    let token = config.line_comment.as_ref()?.to_string();
    if token.is_empty() {
        return None;
    }
    let texts: Vec<String> = (first..=last)
        .map(|l| lines.line_text(l).unwrap_or_default())
        .collect();
    let mut data: Vec<LineData> = texts
        .iter()
        .map(|_| LineData {
            ignore: false,
            offset: 0,
            length: token.len(),
        })
        .collect();

    // `_analyzeLines`
    let mut only_whitespace = true;
    let mut remove = kind != CommentType::ForceAdd;
    for (i, text) in texts.iter().enumerate() {
        let Some(start) = first_non_whitespace(text) else {
            data[i].ignore = options.ignore_empty_lines;
            data[i].offset = text.len();
            continue;
        };
        only_whitespace = false;
        data[i].ignore = false;
        data[i].offset = start;
        let commented = text[start..].starts_with(&token);
        if remove && !commented {
            match kind {
                CommentType::Toggle => remove = false,
                CommentType::ForceRemove => data[i].ignore = true,
                CommentType::ForceAdd => {}
            }
        }
        if remove && options.insert_space {
            let end = start + token.len();
            if text.as_bytes().get(end) == Some(&b' ') {
                data[i].length = token.len() + 1;
            }
        }
    }
    if kind == CommentType::Toggle && only_whitespace {
        remove = false;
        for d in data.iter_mut() {
            d.ignore = false;
        }
    }

    let mut changes = Vec::new();
    if remove {
        for (i, d) in data.iter().enumerate() {
            if d.ignore {
                continue;
            }
            let line = first + i as u64;
            let text = &texts[i];
            if !text[d.offset.min(text.len())..].starts_with(&token) {
                continue;
            }
            changes.push(Change::delete(
                (line, d.offset),
                (line, (d.offset + d.length).min(text.len())),
            ));
        }
    } else {
        normalize_insertion_point(&texts, &mut data, options.tab_size);
        let inserted = if options.insert_space {
            format!("{token} ")
        } else {
            token.clone()
        };
        for (i, d) in data.iter().enumerate() {
            if d.ignore {
                continue;
            }
            changes.push(Change::insert(
                (first + i as u64, d.offset),
                inserted.clone(),
            ));
        }
    }
    Some(changes)
}

/// `_normalizeInsertionPoint`：所有参与行的前缀插在公共缩进（最小可见列）处。
fn normalize_insertion_point(texts: &[String], data: &mut [LineData], tab_size: usize) {
    let tab_size = tab_size.max(1);
    let mut min_visible = usize::MAX;
    for (i, d) in data.iter().enumerate() {
        if d.ignore {
            continue;
        }
        let visible = visible_width(&texts[i][..d.offset.min(texts[i].len())], tab_size);
        min_visible = min_visible.min(visible);
    }
    if min_visible == usize::MAX {
        return;
    }
    for (i, d) in data.iter_mut().enumerate() {
        if d.ignore {
            continue;
        }
        let text = &texts[i];
        let mut visible = 0;
        let mut at = 0;
        for (index, ch) in text[..d.offset.min(text.len())].char_indices() {
            if visible >= min_visible {
                break;
            }
            visible = if ch == '\t' {
                visible + tab_size - visible % tab_size
            } else {
                visible + 1
            };
            at = index + ch.len_utf8();
        }
        // 制表符跨过了公共列：退回它前面一格（VS Code 的 `j - 1`）。
        d.offset = if visible > min_visible {
            text[..at]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0)
        } else {
            at
        };
    }
}

/// 块注释切换（`editor.action.blockComment`，以及没有行注释时的退化路径）。
pub fn toggle_block_comment(
    lines: &dyn LineSource,
    config: &LanguageConfiguration,
    start: (u64, usize),
    end: (u64, usize),
    insert_space: bool,
) -> Option<AutoEdit> {
    let pair = config.block_comment.as_ref()?;
    let (open, close) = (pair.open.to_string(), pair.close.to_string());
    let start_text = lines.line_text(start.0).unwrap_or_default();
    let end_text = lines.line_text(end.0).unwrap_or_default();
    // 选区内（或两端外侧紧邻）已经有一对块注释：删掉它。
    let open_at = start_text
        .get(..(start.1 + open.len()).min(start_text.len()))
        .and_then(|s| s.rfind(&open));
    let close_at = end_text
        .get(end.1.saturating_sub(close.len())..)
        .and_then(|s| s.find(&close))
        .map(|i| i + end.1.saturating_sub(close.len()))
        .or_else(|| {
            end_text
                .get(end.1..)
                .and_then(|s| s.find(&close))
                .map(|i| i + end.1)
        });
    if let (Some(o), Some(c)) = (open_at, close_at) {
        if start.0 < end.0 || o + open.len() <= c {
            let mut open_end = o + open.len();
            if insert_space && start_text.as_bytes().get(open_end) == Some(&b' ') {
                open_end += 1;
            }
            let mut close_start = c;
            if insert_space && close_start > 0 && end_text.as_bytes()[close_start - 1] == b' ' {
                close_start -= 1;
            }
            return Some(AutoEdit {
                changes: vec![
                    Change::delete((start.0, o), (start.0, open_end)),
                    Change::delete((end.0, close_start), (end.0, c + close.len())),
                ],
                caret: None,
                anchor: None,
            });
        }
    }
    // 否则插一对。空选区时把光标放在中间。
    let space = if insert_space { " " } else { "" };
    if start == end {
        return Some(AutoEdit {
            changes: vec![Change::insert(
                start,
                format!("{open}{space}{space}{close}"),
            )],
            caret: Some((start.0, start.1 + open.len() + space.len())),
            anchor: None,
        });
    }
    Some(AutoEdit {
        changes: vec![
            Change::insert(start, format!("{open}{space}")),
            Change::insert(end, format!("{space}{close}")),
        ],
        caret: None,
        anchor: None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::testing::lines;
    use super::*;
    use rove_syntax::language_config::BuiltinLanguages;
    use rove_syntax::LanguageConfigurations;
    use std::sync::Arc;

    fn data(language: &str) -> Arc<LanguageConfiguration> {
        BuiltinLanguages::shared().0.get(language).unwrap()
    }

    /// 把改动套到行数组上（改动按行、互不重叠，从后往前应用）。
    fn apply(rows: &[&str], changes: &[Change]) -> Vec<String> {
        let mut out: Vec<String> = rows.iter().map(|s| s.to_string()).collect();
        let mut sorted = changes.to_vec();
        sorted.sort_by_key(|c| (c.start.0, c.start.1));
        for c in sorted.iter().rev() {
            assert_eq!(c.start.0, c.end.0, "测试辅助只处理行内改动");
            let line = &mut out[c.start.0 as usize];
            line.replace_range(c.start.1..c.end.1, &c.text);
        }
        out
    }

    fn toggle(language: &str, rows: &[&str], options: CommentOptions) -> Vec<String> {
        let l = lines(rows);
        let c = data(language);
        let changes = toggle_line_comment(
            &l,
            &c,
            0,
            rows.len() as u64 - 1,
            CommentType::Toggle,
            options,
        )
        .unwrap();
        apply(rows, &changes)
    }

    #[test]
    fn csharp_lines_are_commented_and_uncommented() {
        let out = toggle("csharp", &["int a;", "int b;"], CommentOptions::default());
        assert_eq!(out, vec!["// int a;", "// int b;"]);
        let out = toggle(
            "csharp",
            &["// int a;", "// int b;"],
            CommentOptions::default(),
        );
        assert_eq!(out, vec!["int a;", "int b;"]);
    }

    #[test]
    fn mixed_state_comments_everything() {
        let out = toggle("csharp", &["// a", "b"], CommentOptions::default());
        assert_eq!(out, vec!["// // a", "// b"]);
    }

    #[test]
    fn json_yaml_markdown_and_xml_use_their_own_tokens() {
        // JSON（jsonc 的 `//`）与 YAML 的 `#` 都是行注释。
        let out = toggle("jsonc", &["\"a\": 1"], CommentOptions::default());
        assert_eq!(out, vec!["// \"a\": 1"]);
        let out = toggle("yaml", &["a: 1"], CommentOptions::default());
        assert_eq!(out, vec!["# a: 1"]);
        // XML 与 Markdown 没有行注释：退化为块注释。
        let l = lines(&["<a/>"]);
        assert!(toggle_line_comment(
            &l,
            &data("xml"),
            0,
            0,
            CommentType::Toggle,
            CommentOptions::default()
        )
        .is_none());
        let block = toggle_block_comment(&l, &data("xml"), (0, 0), (0, 4), true).unwrap();
        assert_eq!(apply(&["<a/>"], &block.changes), vec!["<!-- <a/> -->"]);
        let md = lines(&["text"]);
        assert!(toggle_line_comment(
            &md,
            &data("markdown"),
            0,
            0,
            CommentType::Toggle,
            CommentOptions::default()
        )
        .is_none());
        let block = toggle_block_comment(&md, &data("markdown"), (0, 0), (0, 4), true).unwrap();
        assert_eq!(apply(&["text"], &block.changes), vec!["<!-- text -->"]);
    }

    #[test]
    fn insert_space_has_both_values() {
        let no_space = CommentOptions {
            insert_space: false,
            ..CommentOptions::default()
        };
        assert_eq!(toggle("csharp", &["a"], no_space), vec!["//a"]);
        assert_eq!(toggle("csharp", &["//a"], no_space), vec!["a"]);
        // insertSpace 取消注释时也吃掉一个空格。
        assert_eq!(
            toggle("csharp", &["// a"], CommentOptions::default()),
            vec!["a"]
        );
        // 但没有空格时不多删。
        assert_eq!(
            toggle("csharp", &["//a"], CommentOptions::default()),
            vec!["a"]
        );
    }

    #[test]
    fn ignore_empty_lines_has_both_values() {
        let rows = &["a", "", "b"];
        assert_eq!(
            toggle("csharp", rows, CommentOptions::default()),
            vec!["// a", "", "// b"]
        );
        let keep = CommentOptions {
            ignore_empty_lines: false,
            ..CommentOptions::default()
        };
        assert_eq!(toggle("csharp", rows, keep), vec!["// a", "// ", "// b"]);
    }

    #[test]
    fn prefix_goes_at_the_common_indent() {
        let out = toggle(
            "csharp",
            &["    deep();", "  shallow();"],
            CommentOptions::default(),
        );
        // 前缀对齐到公共缩进，更深的那一行把多出来的缩进留在前缀之后（VS Code 的做法）。
        assert_eq!(out, vec!["  //   deep();", "  // shallow();"]);
    }

    #[test]
    fn first_line_deeper_still_uses_the_common_indent() {
        let out = toggle(
            "csharp",
            &["        a();", "    b();", "    c();"],
            CommentOptions::default(),
        );
        assert_eq!(out, vec!["    //     a();", "    // b();", "    // c();"]);
    }

    #[test]
    fn whitespace_only_block_is_commented() {
        let out = toggle("csharp", &["   ", "  "], CommentOptions::default());
        assert_eq!(out, vec!["  //  ", "  // "]);
    }

    #[test]
    fn force_add_and_force_remove() {
        let l = lines(&["// a", "b"]);
        let c = data("csharp");
        let add = toggle_line_comment(
            &l,
            &c,
            0,
            1,
            CommentType::ForceAdd,
            CommentOptions::default(),
        )
        .unwrap();
        assert_eq!(apply(&["// a", "b"], &add), vec!["// // a", "// b"]);
        let remove = toggle_line_comment(
            &l,
            &c,
            0,
            1,
            CommentType::ForceRemove,
            CommentOptions::default(),
        )
        .unwrap();
        assert_eq!(apply(&["// a", "b"], &remove), vec!["a", "b"]);
    }

    #[test]
    fn block_comment_round_trips() {
        let l = lines(&["int a;"]);
        let c = data("csharp");
        let added = toggle_block_comment(&l, &c, (0, 0), (0, 6), true).unwrap();
        let rows = apply(&["int a;"], &added.changes);
        assert_eq!(rows, vec!["/* int a; */"]);
        let l2 = lines(&["/* int a; */"]);
        let removed = toggle_block_comment(&l2, &c, (0, 0), (0, 12), true).unwrap();
        assert_eq!(apply(&["/* int a; */"], &removed.changes), vec!["int a;"]);
    }

    #[test]
    fn empty_selection_block_comment_puts_the_caret_inside() {
        let l = lines(&["x"]);
        let c = data("csharp");
        let e = toggle_block_comment(&l, &c, (0, 1), (0, 1), true).unwrap();
        assert_eq!(apply(&["x"], &e.changes), vec!["x/*  */"]);
        assert_eq!(e.caret, Some((0, 4)));
    }

    #[test]
    fn tabs_in_the_indent_keep_the_insertion_point_on_a_boundary() {
        let out = toggle("csharp", &["\ta();", "    b();"], CommentOptions::default());
        assert_eq!(out, vec!["\t// a();", "    // b();"]);
    }
}
