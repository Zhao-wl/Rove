//! 改写整个文件的事务（#46）：缩进转换与换行符统一。命令与状态栏选择器项由 R46 / R48 登记。

use rove_base::*;

use super::lines::{line_count, line_text};
use super::transaction::{eol_str, Replacement, Transaction};
use crate::contracts::{Eol, TextSource};

/// `editor.action.indentationToSpaces` / `indentationToTabs`（移植 VS Code
/// `getIndentationEditOperations`）：只改每行的前导空白（只有空白的行整行都算前导空白）。
///
/// 转空格：每个 Tab 换成 `tab_size` 个空格；转 Tab：前导空白里每连续 `tab_size` 个空格换成
/// 一个 Tab（不足的余数保留）。没有变化的行不产生编辑。
pub fn indentation_transaction(
    text: &dyn TextSource,
    tab_size: u64,
    to_spaces: bool,
) -> Result<Transaction> {
    let tab_size = tab_size.max(1) as usize;
    let spaces = " ".repeat(tab_size);
    let count = line_count(text)?;
    let mut edits = Vec::new();
    for line in 0..count {
        let content = line_text(text, Line(line))?;
        let indentation_len = content
            .find(|ch: char| ch != ' ' && ch != '\t')
            .unwrap_or(content.len());
        if indentation_len == 0 {
            continue;
        }
        let indentation = &content[..indentation_len];
        let replaced = if to_spaces {
            indentation.replace('\t', &spaces)
        } else {
            indentation.replace(&spaces, "\t")
        };
        if replaced != indentation {
            let start = text.line_range(Line(line))?.start();
            edits.push(Replacement::new(
                Span::new(start, ByteOffset(start.0 + indentation_len as u64))?,
                replaced,
            ));
        }
    }
    Transaction::new(edits)
}

/// 换行符统一（`workbench.action.editor.changeEOL`）：把每个与 `eol` 不同的行尾改写成 `eol`，
/// 行内容一个字节都不动。返回原样模式的事务（插入的就是目标换行符本身）。
pub fn eol_transaction(text: &dyn TextSource, eol: Eol) -> Result<Transaction> {
    let target = eol_str(eol);
    let count = line_count(text)?;
    let mut edits = Vec::new();
    for line in 0..count.saturating_sub(1) {
        let end = text.line_range(Line(line))?.end();
        let following = text.line_range(Line(line + 1))?.start();
        let range = Span::new(end, following)?;
        let mut terminator = String::with_capacity(2);
        text.visit_chunks(range.clone(), &mut |chunk| terminator.push_str(chunk))?;
        if terminator != target {
            edits.push(Replacement::new(range, target));
        }
    }
    Ok(Transaction::new(edits)?.verbatim())
}
