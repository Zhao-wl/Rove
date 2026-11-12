//! 按行读取文本源的小工具：行内容、字符与 UTF-8 列之间的换算。

use rove_base::*;

use crate::contracts::TextSource;

/// 行数（文本源必须已经索引完）。
pub(crate) fn line_count(text: &dyn TextSource) -> Result<u64> {
    let lines = text.lines();
    if !lines.complete {
        return Err(Error::NotReady);
    }
    Ok(lines.known)
}

/// 一行的内容（不含行终止符）。
pub(crate) fn line_text(text: &dyn TextSource, line: Line) -> Result<String> {
    let range = text.line_range(line)?;
    let mut out = String::with_capacity((range.end().0 - range.start().0) as usize);
    text.visit_chunks(range, &mut |chunk| out.push_str(chunk))?;
    Ok(out)
}

/// 一行的字符序列，以及每个字符的 UTF-8 列（末尾多一项：行内容长度）。
#[derive(Clone, Debug)]
pub(crate) struct LineChars {
    pub chars: Vec<char>,
    columns: Vec<u64>,
}

impl LineChars {
    pub fn read(text: &dyn TextSource, line: Line) -> Result<Self> {
        Ok(Self::new(&line_text(text, line)?))
    }

    pub fn new(content: &str) -> Self {
        let mut chars = Vec::with_capacity(content.len());
        let mut columns = Vec::with_capacity(content.len() + 1);
        for (at, ch) in content.char_indices() {
            chars.push(ch);
            columns.push(at as u64);
        }
        columns.push(content.len() as u64);
        Self { chars, columns }
    }

    /// 字符个数（VS Code 的 `maxColumn - 1`）。
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// UTF-8 列 → 字符下标；不在字符边界或越界时 [`Error::OutOfBounds`]。
    pub fn index(&self, column: Utf8Column) -> Result<usize> {
        self.columns
            .binary_search(&column.0)
            .map_err(|_| Error::OutOfBounds)
    }

    /// 字符下标 → UTF-8 列。
    pub fn column(&self, index: usize) -> Utf8Column {
        Utf8Column(self.columns[index.min(self.chars.len())])
    }
}
