//! 基于 `ropey` 的不可变文本快照（#7）。
//!
//! 接口是 #1 冻结的 [`TextSource`]：行与坐标转换都以**解码后的 UTF-8** 为准。
//! 行终止符是 LF、CRLF 与单独的 CR（与 VS Code 一致）；行内文本不含终止符，
//! NEL / LS / PS 不当作换行（`ropey` 的 `unicode_lines` feature 已关闭）。
//!
//! 线程：`Rope` 是 `Send + Sync`，快照创建后不再变化；行与坐标转换都是
//! O(log n)（不逐字符扫描整行）。

use std::sync::atomic::{AtomicU64, Ordering};

use crate::contracts::{LineCount, TextSource};
use ropey::Rope;
use rove_base::*;

/// 进程级版本发号。版本只要求在同一文档内单调，全局唯一只会更安全：
/// 重新加载同一文件时不会复用旧版本号，缓存不会把旧快照误判成当前快照。
static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

/// 取一个新的快照版本号（[`RopeText`] 与超大文件的 `MappedText` 共用）。
pub(crate) fn next_version() -> Version {
    Version(NEXT_VERSION.fetch_add(1, Ordering::Relaxed))
}

/// 一个不可变的文本快照。构造后内容与版本都不再变化。
#[derive(Debug)]
pub struct RopeText {
    rope: Rope,
    version: Version,
}

impl RopeText {
    /// 从已解码的 UTF-8 文本建立快照；会复制一次全文。
    pub fn new(text: String) -> Self {
        Self::from_rope(Rope::from(text))
    }

    /// 从借用的 UTF-8 文本建立快照；会复制一次全文。
    pub fn from_text(text: &str) -> Self {
        Self::from_rope(Rope::from_str(text))
    }

    pub(crate) fn from_rope(rope: Rope) -> Self {
        Self {
            rope,
            version: next_version(),
        }
    }

    /// 底层 rope（#46 编辑核心用它做写时复制：克隆是 O(1)，修改只复制路径上的节点）。
    pub(crate) fn rope(&self) -> &Rope {
        &self.rope
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
        self.version
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
        let column = usize::try_from(column.0).map_err(|_| Error::OutOfBounds)?;
        let (start, end) = self.line_bytes(line)?;
        let char_start = self.rope.byte_to_char(start);
        let char_end = self.rope.byte_to_char(end);
        let unit_start = self.rope.char_to_utf16_cu(char_start);
        let unit_end = self.rope.char_to_utf16_cu(char_end);
        let target = unit_start.checked_add(column).ok_or(Error::OutOfBounds)?;
        if target > unit_end {
            return Err(Error::OutOfBounds);
        }
        let char_index = self.rope.utf16_cu_to_char(target);
        // 落在代理对中间时 ropey 会取整到所属字符；这里必须拒绝。
        if char_index > char_end || self.rope.char_to_utf16_cu(char_index) != target {
            return Err(Error::OutOfBounds);
        }
        Ok(Utf8Column(
            (self.rope.char_to_byte(char_index) - start) as u64,
        ))
    }

    fn utf8_to_utf16(&self, line: Line, column: Utf8Column) -> Result<Utf16Column> {
        let line = usize::try_from(line.0).map_err(|_| Error::OutOfBounds)?;
        let column = usize::try_from(column.0).map_err(|_| Error::OutOfBounds)?;
        let (start, end) = self.line_bytes(line)?;
        let byte = start.checked_add(column).ok_or(Error::OutOfBounds)?;
        if byte > end || !self.at_char_boundary(byte) {
            return Err(Error::OutOfBounds);
        }
        let unit_start = self.rope.char_to_utf16_cu(self.rope.byte_to_char(start));
        let unit = self.rope.char_to_utf16_cu(self.rope.byte_to_char(byte));
        Ok(Utf16Column((unit - unit_start) as u64))
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
    fn utf16_columns_on_a_long_single_line() {
        // 5000 个 "a😀"：每 5 字节 / 3 个 UTF-16 码元，共 25000 字节 / 15000 码元。
        // 旧实现逐字符扫整行，这里同时验证边界与代理对中间被拒绝。
        let text = RopeText::from_text(&"a😀".repeat(5000));
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(14_997)).unwrap(),
            Utf8Column(24_995)
        );
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(14_998)).unwrap(),
            Utf8Column(24_996)
        );
        // 代理对的第二个码元。
        assert!(text.utf16_to_utf8(Line(0), Utf16Column(14_999)).is_err());
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(15_000)).unwrap(),
            Utf8Column(25_000)
        );
        assert!(text.utf16_to_utf8(Line(0), Utf16Column(15_001)).is_err());

        assert_eq!(
            text.utf8_to_utf16(Line(0), Utf8Column(24_995)).unwrap(),
            Utf16Column(14_997)
        );
        assert_eq!(
            text.utf8_to_utf16(Line(0), Utf8Column(24_996)).unwrap(),
            Utf16Column(14_998)
        );
        assert!(text.utf8_to_utf16(Line(0), Utf8Column(24_997)).is_err());
        assert_eq!(
            text.utf8_to_utf16(Line(0), Utf8Column(25_000)).unwrap(),
            Utf16Column(15_000)
        );
        assert!(text.utf8_to_utf16(Line(0), Utf8Column(25_001)).is_err());
    }

    #[test]
    fn every_snapshot_gets_a_fresh_version() {
        let first = RopeText::new("first".into());
        let second = RopeText::new("second".into());
        assert!(first.version().0 >= 1);
        assert_ne!(first.version(), second.version());
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
