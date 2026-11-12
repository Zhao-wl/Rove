//! 剪贴板的文本语义（#46）：复制 / 剪切 / 粘贴分别移植 VS Code 的
//! `ViewModel.getPlainTextToCopy`、`DeleteOperations.cut` 与 `PasteOperation`。
//! 系统剪贴板的读写在 R46；这里只决定“复制出什么”“删哪里”“粘到哪里”。

use rove_base::*;

use super::lines::line_count;
use super::selection::{CursorPlacement, Selections};
use super::transaction::eol_str;
use crate::contracts::{Eol, TextSource};

/// `editor.multiCursorPaste`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MultiCursorPaste {
    /// 粘贴文本的行数与光标数相同时，每个光标粘贴一行。
    #[default]
    Spread,
    /// 每个光标都粘贴全部文本。
    Full,
}

/// 复制出的内容。`text` 写进系统剪贴板；`multicursor` 与 `from_empty_selection` 作为
/// 元数据随之保存（VS Code 的 `ClipboardStoredMetadata`），粘贴时原样交回。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardData {
    pub text: String,
    /// 多个选区各自的文本（只有一段时为 `None`）。
    pub multicursor: Option<Vec<String>>,
    /// 来自“无选区时复制整行”。
    pub from_empty_selection: bool,
}

impl ClipboardData {
    /// 来自外部程序的纯文本（没有元数据）。
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            multicursor: None,
            from_empty_selection: false,
        }
    }
}

fn read(text: &dyn TextSource, range: ByteRange) -> Result<String> {
    let mut out = String::with_capacity((range.end().0 - range.start().0) as usize);
    text.visit_chunks(range, &mut |chunk| out.push_str(chunk))?;
    Ok(out)
}

/// 复制：没有任何非空选区且 `empty_selection_clipboard` 为假时返回 `None`。
///
/// 选区内的换行符保持原样；整行复制追加、多段拼接用的换行符是文档的主换行符 `eol`。
pub fn copy(
    text: &dyn TextSource,
    selections: &Selections,
    empty_selection_clipboard: bool,
    eol: Eol,
) -> Result<Option<ClipboardData>> {
    let sorted = selections.sorted();
    let has_empty = sorted.iter().any(|s| s.is_empty());
    let has_non_empty = sorted.iter().any(|s| !s.is_empty());
    if !has_non_empty && !empty_selection_clipboard {
        return Ok(None);
    }
    let newline = eol_str(eol);
    let mut segments = Vec::with_capacity(sorted.len());
    if has_empty && empty_selection_clipboard {
        let mut previous_line = None;
        for selection in &sorted {
            let line = text.position(selection.start())?.line;
            if selection.is_empty() {
                if previous_line != Some(line) {
                    let mut segment = read(text, text.line_range(line)?)?;
                    segment.push_str(newline);
                    segments.push(segment);
                }
            } else {
                segments.push(read(text, selection.range())?);
            }
            previous_line = Some(line);
        }
    } else {
        for selection in sorted.iter().filter(|s| !s.is_empty()) {
            segments.push(read(text, selection.range())?);
        }
    }
    let joined = segments.join(newline);
    Ok(Some(ClipboardData {
        text: joined,
        multicursor: (segments.len() > 1).then_some(segments),
        from_empty_selection: empty_selection_clipboard
            && selections.len() == 1
            && selections.primary().is_empty(),
    }))
}

/// 剪切要删除的区间（升序、互不重叠）：非空选区删选区；空选区在 `empty_selection_clipboard`
/// 为真时删整行（含行尾；最后一行删它前面的换行），否则不删。
pub fn cut_ranges(
    text: &dyn TextSource,
    selections: &Selections,
    empty_selection_clipboard: bool,
) -> Result<Vec<ByteRange>> {
    let count = line_count(text)?;
    let mut ranges: Vec<ByteRange> = Vec::with_capacity(selections.len());
    let mut last_cut_end_line: Option<u64> = None;
    for selection in selections.sorted() {
        let range = if selection.is_empty() {
            if !empty_selection_clipboard {
                continue;
            }
            let line = text.position(selection.active)?.line.0;
            let range = if line + 1 < count {
                Span::new(
                    text.line_range(Line(line))?.start(),
                    text.line_range(Line(line + 1))?.start(),
                )?
            } else if line > 0 && last_cut_end_line != Some(line) {
                Span::new(
                    text.line_range(Line(line - 1))?.end(),
                    text.line_range(Line(line))?.end(),
                )?
            } else {
                text.line_range(Line(line))?
            };
            last_cut_end_line = Some(text.position(range.end())?.line.0);
            range
        } else {
            selection.range()
        };
        if range.start() == range.end() {
            continue;
        }
        // 同一行上的多个空光标会得到相同的整行区间，只删一次。
        match ranges.last() {
            Some(previous) if range.start() < previous.end() => {
                if range.end() > previous.end() {
                    let merged = Span::new(previous.start(), range.end())?;
                    *ranges.last_mut().expect("非空") = merged;
                }
            }
            _ => ranges.push(range),
        }
    }
    Ok(ranges)
}

/// 一个选区的粘贴动作：替换 `range` 为 `text`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteEdit {
    pub range: ByteRange,
    pub text: String,
    pub placement: CursorPlacement,
}

/// 计算每个选区（按创建顺序，与 `selections.all()` 一一对应）的粘贴动作。
pub fn paste_edits(
    text: &dyn TextSource,
    selections: &Selections,
    data: &ClipboardData,
    empty_selection_clipboard: bool,
    mode: MultiCursorPaste,
) -> Result<Vec<PasteEdit>> {
    let paste_on_new_line = empty_selection_clipboard && data.from_empty_selection;
    if let Some(distributed) = distribute(selections, data, paste_on_new_line, mode) {
        // 分配给按位置排序后的各个选区。
        let mut order: Vec<usize> = (0..selections.len()).collect();
        order.sort_by_key(|&i| {
            let s = &selections.all()[i];
            (s.start(), s.end())
        });
        let mut edits = vec![None; selections.len()];
        for (slot, index) in order.into_iter().enumerate() {
            edits[index] = Some(PasteEdit {
                range: selections.all()[index].range(),
                text: distributed[slot].clone(),
                placement: CursorPlacement::AfterInserted,
            });
        }
        return Ok(edits
            .into_iter()
            .map(|e| e.expect("每个选区一段"))
            .collect());
    }
    // `_simplePaste`：标志一旦被某个选区关掉，后面的选区也不再整行粘贴（照搬 VS Code）。
    let mut on_new_line = paste_on_new_line;
    let mut edits = Vec::with_capacity(selections.len());
    for selection in selections.all() {
        if on_new_line && !selection.is_empty() {
            on_new_line = false;
        }
        let first_newline = data.text.find('\n').map_or(-1, |at| at as i64);
        if on_new_line && first_newline != data.text.len() as i64 - 1 {
            on_new_line = false;
        }
        edits.push(if on_new_line {
            let line = text.position(selection.active)?.line;
            let start = text.line_range(line)?.start();
            PasteEdit {
                range: Span::new(start, start)?,
                text: data.text.clone(),
                placement: CursorPlacement::Preserve,
            }
        } else {
            PasteEdit {
                range: selection.range(),
                text: data.text.clone(),
                placement: CursorPlacement::AfterInserted,
            }
        });
    }
    Ok(edits)
}

/// `_distributePasteToCursors`。
fn distribute(
    selections: &Selections,
    data: &ClipboardData,
    paste_on_new_line: bool,
    mode: MultiCursorPaste,
) -> Option<Vec<String>> {
    if selections.len() == 1 {
        return None;
    }
    if let Some(segments) = &data.multicursor {
        if segments.len() == selections.len() {
            return Some(segments.clone());
        }
    }
    if paste_on_new_line {
        return None;
    }
    if mode == MultiCursorPaste::Spread {
        let mut text = data.text.as_str();
        text = text.strip_suffix('\n').unwrap_or(text);
        text = text.strip_suffix('\r').unwrap_or(text);
        let lines = split_lines(text);
        if lines.len() == selections.len() {
            return Some(lines);
        }
    }
    None
}

/// `strings.splitLines`：按 `\r\n|\r|\n` 切分。
fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;
    loop {
        match rest.find(['\r', '\n']) {
            Some(at) => {
                lines.push(rest[..at].to_string());
                let skip = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[at + skip..];
            }
            None => {
                lines.push(rest.to_string());
                return lines;
            }
        }
    }
}
