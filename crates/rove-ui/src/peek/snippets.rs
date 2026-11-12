//! 结果行的单行代码片段（不依赖 GPUI）。只为**展开的文件**读取：初始展开的文件在查询的同一个
//! 后台任务里读好，其余文件在用户展开时再读（树先显示加载行）。
//!
//! 文本经 #7/#17 的 `Documents` 取得：已打开的文档复用同一份快照（与主编辑器一致），未打开的
//! 文件由加载器在后台读盘（编码识别与主编辑器相同）。

use super::model::PeekResults;
use rove_base::{Cancellation, Line, Resource, Result};
use rove_buffer::{Documents, TextSource};
use std::ops::Range;
use std::sync::Arc;

/// 片段最多保留的字节数（匹配之前最多保留 [`LEAD_BYTES`]）。
pub const MAX_BYTES: usize = 240;
pub const LEAD_BYTES: usize = 60;

/// 一条结果的片段：行号（0 起）、去掉缩进并截断后的文本、匹配在文本里的字节区间。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub line: u64,
    pub text: Arc<str>,
    pub highlight: Range<usize>,
}

/// 一个文件里全部结果的片段，顺序与 `PeekResults::files[file].items()` 一致。
pub type FileSnippets = Vec<Snippet>;

/// 读一行源文本（不含行尾）。
pub fn line_text(text: &dyn TextSource, line: u64) -> Option<String> {
    let range = text.line_range(Line(line)).ok()?;
    let mut out = String::new();
    text.visit_chunks(range, &mut |s| out.push_str(s)).ok()?;
    while out.ends_with(['\n', '\r']) {
        out.pop();
    }
    Some(out)
}

/// 由整行与匹配列区间（UTF-8 字节，可越界）生成片段：去掉行首空白，匹配前最多留
/// [`LEAD_BYTES`]，总长不超过 [`MAX_BYTES`]，切点落在字符边界上。
pub fn snippet(line: u64, text: &str, columns: Range<usize>) -> Snippet {
    let floor = |mut i: usize| {
        i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (start, end) = (floor(columns.start), floor(columns.end.max(columns.start)));
    let indent = text.len() - text.trim_start().len();
    let mut from = indent.min(start);
    if start - from > LEAD_BYTES {
        from = floor(start - LEAD_BYTES);
    }
    let to = floor((from + MAX_BYTES).max(end).min(text.len()));
    let to = if to - from > MAX_BYTES && end - from <= MAX_BYTES {
        floor(from + MAX_BYTES)
    } else {
        to
    };
    let body = &text[from..to];
    let body = body.trim_end();
    let highlight = (start - from).min(body.len())..(end - from).min(body.len());
    Snippet {
        line,
        text: body.into(),
        highlight,
    }
}

/// 读取 `file` 里每条结果的片段。外部符号（虚拟资源）没有源码行，片段是符号键本身。
pub async fn load(
    results: &PeekResults,
    file: usize,
    documents: &dyn Documents,
    cancel: &Cancellation,
) -> Result<FileSnippets> {
    let group = &results.files[file];
    let items = &results.items[group.items()];
    if let Resource::Virtual(key) = &group.resource {
        return Ok(items
            .iter()
            .map(|i| Snippet {
                line: i.range.start().line.0,
                text: key.clone(),
                highlight: 0..0,
            })
            .collect());
    }
    let document = match documents.find(&group.resource) {
        Some(d) => d,
        None => {
            documents
                .open(group.resource.clone(), cancel.clone())
                .await?
        }
    };
    let text = &*document.text;
    let mut out = Vec::with_capacity(items.len());
    let mut cached: Option<(u64, String)> = None;
    for item in items {
        cancel.check()?;
        let start = item.range.start();
        let end = item.range.end();
        let line = start.line.0;
        if cached.as_ref().is_none_or(|(l, _)| *l != line) {
            cached = Some((line, line_text(text, line).unwrap_or_default()));
        }
        let (_, body) = cached.as_ref().unwrap();
        let end_col = if end.line == start.line {
            end.column.0 as usize
        } else {
            body.len()
        };
        out.push(snippet(line, body, start.column.0 as usize..end_col));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_trims_indent_and_keeps_the_match() {
        let s = snippet(3, "        player.Move(dir);", 15..19);
        assert_eq!(&*s.text, "player.Move(dir);");
        assert_eq!(&s.text[s.highlight.clone()], "Move");
    }

    #[test]
    fn long_lines_are_cut_around_the_match_on_char_boundaries() {
        let line = format!("{}目标{}", "a".repeat(500), "b".repeat(500));
        let at = line.find("目标").unwrap();
        let s = snippet(0, &line, at..at + "目标".len());
        assert!(s.text.len() <= MAX_BYTES);
        assert_eq!(&s.text[s.highlight.clone()], "目标");
        // 越界与倒置的列也不会 panic。
        #[allow(clippy::reversed_empty_ranges)]
        let s = snippet(0, "短", 7..2);
        assert_eq!(s.highlight, 3..3);
    }
}
