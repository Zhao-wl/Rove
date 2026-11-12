//! 保存时的文本变换（#49）：`files.trimTrailingWhitespace`、`files.trimFinalNewlines`、
//! `files.insertFinalNewline`。
//!
//! 每个变换只产出一个 [`Transaction`]（没有变化时返回 `None`），由 [`super::TextFile`] 依次经
//! #46 的 [`EditSession`](crate::edit::EditSession) 执行，并与用户最后一次改动合并成同一个撤销单元。
//! 执行顺序与 VS Code 的保存参与者相同：先去行尾空白、再去末尾多余空行、最后补末尾换行
//! （三者默认都关闭；同时开启时互不打架）。
//!
//! 移植自 VS Code `src/vs/workbench/api/browser/mainThreadSaveParticipant.ts`
//! （`TrimWhitespaceParticipant`、`TrimFinalNewLinesParticipant`、`FinalNewLineParticipant`）
//! 与 `trimTrailingWhitespaceCommand.ts`；“只有空格与 Tab 算空白”与 VS Code 一致。

use rove_base::*;

use crate::contracts::{Eol, TextSource};
use crate::edit::transaction::eol_str;
use crate::edit::{Replacement, Transaction};

/// 一个位置是否在字符串或正则字面量内部（由语法层的 token 判断，#9）。
/// `files.trimTrailingWhitespaceInRegexAndStrings = false` 时，这类位置上的行尾空白保留。
pub type InStringOrRegex<'a> = &'a dyn Fn(Line, Utf8Column) -> bool;

/// `files.trimTrailingWhitespace`：删除每行末尾的空格与 Tab。
///
/// - `protect`：给出时，行尾空白起点位于字符串 / 正则内部的行不处理；
/// - `keep_lines`：不处理的行（自动保存时光标所在的行，VS Code 相同）。
pub fn trim_trailing_whitespace(
    text: &dyn TextSource,
    protect: Option<InStringOrRegex<'_>>,
    keep_lines: &[Line],
) -> Result<Option<Transaction>> {
    let count = line_count(text)?;
    let mut edits = Vec::new();
    for line in 0..count {
        let line = Line(line);
        if keep_lines.contains(&line) {
            continue;
        }
        let range = text.line_range(line)?;
        let content = read(text, range.clone())?;
        let trimmed = content.trim_end_matches([' ', '\t']).len();
        if trimmed == content.len() {
            continue;
        }
        if let Some(protect) = protect {
            if protect(line, Utf8Column(trimmed as u64)) {
                continue;
            }
        }
        let start = ByteOffset(range.start().0 + trimmed as u64);
        edits.push(Replacement::delete(Span::new(start, range.end())?));
    }
    finish(edits)
}

/// `files.trimFinalNewlines`：删除最后一个非空行之后的所有空行（保留一个末尾换行）。
/// 全文都是空行时清空全文（VS Code 相同）。
pub fn trim_final_newlines(text: &dyn TextSource) -> Result<Option<Transaction>> {
    let count = line_count(text)?;
    let mut last_non_empty = None;
    for line in (0..count).rev() {
        let range = text.line_range(Line(line))?;
        if range.start() != range.end() {
            last_non_empty = Some(line);
            break;
        }
    }
    let delete_from = match last_non_empty {
        // 最后一个非空行的行尾终止符之后。
        Some(line) if line + 1 < count => text.line_range(Line(line + 1))?.start(),
        Some(_) => return Ok(None),
        None => ByteOffset(0),
    };
    let end = ByteOffset(text.byte_len());
    if delete_from >= end {
        return Ok(None);
    }
    finish(vec![Replacement::delete(Span::new(delete_from, end)?)])
}

/// `files.insertFinalNewline`：最后一行不为空时在末尾补一个换行符（文档的主换行符）。
/// 空文件不补。
pub fn insert_final_newline(text: &dyn TextSource, eol: Eol) -> Result<Option<Transaction>> {
    let count = line_count(text)?;
    if count == 0 {
        return Ok(None);
    }
    let last = text.line_range(Line(count - 1))?;
    if last.start() == last.end() {
        return Ok(None);
    }
    finish(vec![Replacement::insert(
        ByteOffset(text.byte_len()),
        eol_str(eol),
    )])
}

fn line_count(text: &dyn TextSource) -> Result<u64> {
    let lines = text.lines();
    if !lines.complete {
        return Err(Error::NotReady);
    }
    Ok(lines.known)
}

fn read(text: &dyn TextSource, range: ByteRange) -> Result<String> {
    let mut out = String::new();
    text.visit_chunks(range, &mut |chunk| out.push_str(chunk))?;
    Ok(out)
}

fn finish(edits: Vec<Replacement>) -> Result<Option<Transaction>> {
    if edits.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Transaction::new(edits)?.verbatim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::LineEndings;
    use crate::edit::EditBuffer;

    fn run(input: &str, transaction: Option<Transaction>) -> String {
        let mut buffer = EditBuffer::new(input, LineEndings::Uniform(Eol::Lf));
        if let Some(transaction) = transaction {
            buffer.apply(&transaction).unwrap();
        }
        buffer.contents()
    }

    fn rope(input: &str) -> crate::RopeText {
        crate::RopeText::from_text(input)
    }

    #[test]
    fn trims_spaces_and_tabs_only() {
        let input = "a  \r\nb\t\n c\u{3000}\n  \n";
        let t = rope(input);
        let out = run(input, trim_trailing_whitespace(&t, None, &[]).unwrap());
        assert_eq!(out, "a\r\nb\n c\u{3000}\n\n");
    }

    #[test]
    fn protected_and_kept_lines_are_untouched() {
        let input = "s = \"x  \n  y\";  \nz  \n";
        let t = rope(input);
        // 第 0 行的行尾空白在字符串内部。
        let protect = |line: Line, _: Utf8Column| line == Line(0);
        let out = run(
            input,
            trim_trailing_whitespace(&t, Some(&protect), &[Line(2)]).unwrap(),
        );
        assert_eq!(out, "s = \"x  \n  y\";\nz  \n");
    }

    #[test]
    fn trim_final_newlines_cases() {
        for (input, expected) in [
            ("a\n\n\n", "a\n"),
            ("a\r\n\r\n", "a\r\n"),
            ("a\n", "a\n"),
            ("a", "a"),
            ("\n\n", ""),
            ("", ""),
            ("a\n  \n", "a\n  \n"),
        ] {
            let t = rope(input);
            assert_eq!(
                run(input, trim_final_newlines(&t).unwrap()),
                expected,
                "{input:?}"
            );
        }
    }

    #[test]
    fn insert_final_newline_cases() {
        for (input, eol, expected) in [
            ("a", Eol::CrLf, "a\r\n"),
            ("a\n", Eol::CrLf, "a\n"),
            ("", Eol::Lf, ""),
            ("a\nb", Eol::Lf, "a\nb\n"),
        ] {
            let t = rope(input);
            assert_eq!(run(input, insert_final_newline(&t, eol).unwrap()), expected);
        }
    }

    #[test]
    fn vscode_order_does_not_fight() {
        // 先去行尾空白，再去末尾空行，最后补换行：结果稳定且只有一个末尾换行。
        let mut text = String::from("a  \n  \n\n");
        let t = rope(&text);
        text = run(&text, trim_trailing_whitespace(&t, None, &[]).unwrap());
        let t = rope(&text);
        text = run(&text, trim_final_newlines(&t).unwrap());
        let t = rope(&text);
        text = run(&text, insert_final_newline(&t, Eol::Lf).unwrap());
        assert_eq!(text, "a\n");
    }
}
