//! 基于 `ropey` 的不可变文本快照（#7）。
//!
//! 接口是 #1 冻结的 [`TextSource`]：行与坐标转换都以**解码后的 UTF-8** 为准。
//! 行终止符是 LF、CRLF 与单独的 CR（与 VS Code 一致）；行内文本不含终止符，
//! NEL / LS / PS 不当作换行（`ropey` 的 `unicode_lines` feature 已关闭）。
//!
//! 线程：`Rope` 是 `Send + Sync`，快照创建后不再变化；坐标转换 O(log n + 行内字符)。

use crate::contracts::{LineCount, TextSource};
use ropey::Rope;
use rove_base::*;

/// 一个不可变的文本快照。构造后内容与版本都不再变化。
#[derive(Debug)]
pub struct RopeText {
    rope: Rope,
}

impl RopeText {
    /// 从已解码的 UTF-8 文本建立快照；会复制一次全文。
    pub fn new(text: String) -> Self {
        Self {
            rope: Rope::from(text),
        }
    }

    /// 从借用的 UTF-8 文本建立快照；会复制一次全文。
    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
        }
    }

    /// 第 `line` 行的内容在全文中的字节区间（不含行终止符）。
    fn line_bytes(&self, line: usize) -> Result<(usize, usize)> {
        if line >= self.rope.len_lines() {
            return Err(Error::OutOfBounds);
        }
        let slice = self.rope.line(line);
        // 去掉行终止符：先看最后一个字符，必要时再看前一个字符（CRLF 是两个字符）。
        let mut chars = slice.len_chars();
        if chars > 0 {
            match slice.char(chars - 1) {
                '\n' => {
                    chars -= 1;
                    if chars > 0 && slice.char(chars - 1) == '\r' {
                        chars -= 1;
                    }
                }
                '\r' => chars -= 1,
                _ => {}
            }
        }
        let start = self.rope.line_to_byte(line);
        Ok((start, start + slice.char_to_byte(chars)))
    }

    /// 文本内容为空则为真（只有一条空行）。
    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    /// 字节偏移是否落在 Unicode scalar 边界上（空区间切片成功即为边界）。
    fn at_char_boundary(&self, byte: usize) -> bool {
        self.rope.get_byte_slice(byte..byte).is_some()
    }
}

impl TextSource for RopeText {
    fn version(&self) -> Version {
        Version(0)
    }

    fn byte_len(&self) -> u64 {
        self.rope.len_bytes() as u64
    }

    fn lines(&self) -> LineCount {
        let total = self.byte_len();
        LineCount {
            known: self.rope.len_lines() as u64,
            indexed_bytes: ByteOffset(total),
            total_bytes: total,
            complete: true,
        }
    }

    fn line_range(&self, line: Line) -> Result<ByteRange> {
        let line = usize::try_from(line.0).map_err(|_| Error::OutOfBounds)?;
        let (start, end) = self.line_bytes(line)?;
        Span::new(ByteOffset(start as u64), ByteOffset(end as u64))
    }

    fn visit_chunks(&self, range: ByteRange, visitor: &mut dyn FnMut(&str)) -> Result<()> {
        let start = usize::try_from(range.start().0).map_err(|_| Error::OutOfBounds)?;
        let end = usize::try_from(range.end().0).map_err(|_| Error::OutOfBounds)?;
        if start > end || end > self.rope.len_bytes() {
            return Err(Error::OutOfBounds);
        }
        // 请求区间必须落在 Unicode scalar 边界上，否则拼出的片段不是合法 UTF-8。
        let slice = self
            .rope
            .get_byte_slice(start..end)
            .ok_or(Error::OutOfBounds)?;
        for chunk in slice.chunks() {
            visitor(chunk);
        }
        Ok(())
    }

    fn offset(&self, position: Position) -> Result<ByteOffset> {
        let line = usize::try_from(position.line.0).map_err(|_| Error::OutOfBounds)?;
        let column = usize::try_from(position.column.0).map_err(|_| Error::OutOfBounds)?;
        let (start, end) = self.line_bytes(line)?;
        let offset = start.checked_add(column).ok_or(Error::OutOfBounds)?;
        if offset > end || !self.at_char_boundary(offset) {
            return Err(Error::OutOfBounds);
        }
        Ok(ByteOffset(offset as u64))
    }

    fn position(&self, offset: ByteOffset) -> Result<Position> {
        let byte = usize::try_from(offset.0).map_err(|_| Error::OutOfBounds)?;
        if byte > self.rope.len_bytes() || !self.at_char_boundary(byte) {
            return Err(Error::OutOfBounds);
        }
        let line = self.rope.byte_to_line(byte);
        let (start, end) = self.line_bytes(line)?;
        // 行终止符内部的字节不是合法的文本位置。
        if byte < start || byte > end {
            return Err(Error::OutOfBounds);
        }
        Ok(Position {
            line: Line(line as u64),
            column: Utf8Column((byte - start) as u64),
        })
    }

    fn utf16_to_utf8(&self, line: Line, column: Utf16Column) -> Result<Utf8Column> {
        let line = usize::try_from(line.0).map_err(|_| Error::OutOfBounds)?;
        let (start, end) = self.line_bytes(line)?;
        let mut units = 0u64;
        let mut byte = start;
        for ch in self.rope.byte_slice(start..end).chars() {
            if units == column.0 {
                return Ok(Utf8Column((byte - start) as u64));
            }
            units += ch.len_utf16() as u64;
            byte += ch.len_utf8();
        }
        if units == column.0 {
            Ok(Utf8Column((end - start) as u64))
        } else {
            // 落在代理对中间，或超出行尾。
            Err(Error::OutOfBounds)
        }
    }

    fn utf8_to_utf16(&self, line: Line, column: Utf8Column) -> Result<Utf16Column> {
        let line = usize::try_from(line.0).map_err(|_| Error::OutOfBounds)?;
        let target = usize::try_from(column.0).map_err(|_| Error::OutOfBounds)?;
        let (start, end) = self.line_bytes(line)?;
        let mut byte = 0usize;
        let mut units = 0u64;
        for ch in self.rope.byte_slice(start..end).chars() {
            if byte == target {
                return Ok(Utf16Column(units));
            }
            byte += ch.len_utf8();
            if byte > target {
                // 列落在 UTF-8 字符中间。
                return Err(Error::OutOfBounds);
            }
            units += ch.len_utf16() as u64;
        }
        if byte == target {
            Ok(Utf16Column(units))
        } else {
            Err(Error::OutOfBounds)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(text: &dyn TextSource, range: ByteRange) -> String {
        let mut out = String::new();
        text.visit_chunks(range, &mut |chunk| out.push_str(chunk))
            .unwrap();
        out
    }

    #[test]
    fn empty_text_has_one_empty_line() {
        let text = RopeText::new(String::new());
        assert_eq!(text.lines().known, 1);
        assert_eq!(text.line_range(Line(0)).unwrap(), span(0, 0));
        assert!(text.line_range(Line(1)).is_err());
        assert_eq!(text.position(ByteOffset(0)).unwrap(), Position::default());
        assert!(text.is_empty());
    }

    #[test]
    fn line_ranges_exclude_all_terminator_forms() {
        let text = RopeText::from_text("a\r\nb\nc\rd");
        assert_eq!(text.lines().known, 4);
        assert_eq!(text.line_range(Line(0)).unwrap(), span(0, 1));
        assert_eq!(text.line_range(Line(1)).unwrap(), span(3, 4));
        assert_eq!(text.line_range(Line(2)).unwrap(), span(5, 6));
        assert_eq!(text.line_range(Line(3)).unwrap(), span(7, 8));
        assert_eq!(collect(&text, span(0, 8)), "a\r\nb\nc\rd");
    }

    #[test]
    fn trailing_newline_adds_an_empty_last_line() {
        let text = RopeText::from_text("a\n");
        assert_eq!(text.lines().known, 2);
        assert_eq!(text.line_range(Line(1)).unwrap(), span(2, 2));
        assert_eq!(
            text.position(ByteOffset(2)).unwrap(),
            Position {
                line: Line(1),
                column: Utf8Column(0)
            }
        );
    }

    #[test]
    fn positions_reject_line_terminator_interiors() {
        let text = RopeText::from_text("ab\r\ncd");
        // 行内容末尾（CRLF 之前）是合法位置。
        assert_eq!(
            text.position(ByteOffset(2)).unwrap(),
            Position {
                line: Line(0),
                column: Utf8Column(2)
            }
        );
        // CRLF 内部的字节不是合法位置。
        assert!(text.position(ByteOffset(3)).is_err());
        assert_eq!(
            text.position(ByteOffset(4)).unwrap(),
            Position {
                line: Line(1),
                column: Utf8Column(0)
            }
        );
        assert!(text.position(ByteOffset(6)).is_ok());
        assert!(text.position(ByteOffset(7)).is_err());
    }

    #[test]
    fn unicode_columns_handle_surrogate_pairs_and_combining_marks() {
        let text = RopeText::from_text("a😀e\u{301}中\r\n");
        // "a"=1 byte/1 unit, "😀"=4 bytes/2 units, "e"+combining=3 bytes/2 units, "中"=3 bytes/1 unit
        // 字节边界：a 0, 😀 1..5, e 5..6, combining 6..8, 中 8..11，行内容共 11 字节。
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(0)).unwrap(),
            Utf8Column(0)
        );
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(1)).unwrap(),
            Utf8Column(1)
        );
        // 代理对与组合字符内部没有对应的 UTF-8 列。
        assert!(text.utf16_to_utf8(Line(0), Utf16Column(2)).is_err());
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(3)).unwrap(),
            Utf8Column(5)
        );
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(4)).unwrap(),
            Utf8Column(6)
        );
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(5)).unwrap(),
            Utf8Column(8)
        );
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(6)).unwrap(),
            Utf8Column(11)
        );
        assert!(text.utf16_to_utf8(Line(0), Utf16Column(7)).is_err());

        for (utf8, utf16) in [(0, 0), (1, 1), (5, 3), (6, 4), (8, 5), (11, 6)] {
            assert_eq!(
                text.utf8_to_utf16(Line(0), Utf8Column(utf8)).unwrap(),
                Utf16Column(utf16)
            );
        }
        // 落在 UTF-8 字符中间或超出行内容末尾。
        assert!(text.utf8_to_utf16(Line(0), Utf8Column(2)).is_err());
        assert!(text.utf8_to_utf16(Line(0), Utf8Column(12)).is_err());
    }

    #[test]
    fn visit_chunks_rejects_scalar_boundaries_and_out_of_bounds() {
        let text = RopeText::from_text("a😀b");
        assert!(text.visit_chunks(span(0, 2), &mut |_| {}).is_err());
        assert!(text.visit_chunks(span(0, 99), &mut |_| {}).is_err());
        let mut count = 0;
        text.visit_chunks(span(1, 5), &mut |chunk| count += chunk.chars().count())
            .unwrap();
        assert_eq!(count, 1);
    }

    fn span(start: u64, end: u64) -> ByteRange {
        Span::new(ByteOffset(start), ByteOffset(end)).unwrap()
    }
}
