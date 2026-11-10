use rove_base::{
    ByteOffset, ByteRange, Error, Line, Position, Result, Span, Utf16Column, Utf8Column, Version,
};
use rove_buffer::{LineCount, TextSource};

#[derive(Clone)]
pub struct Text {
    source: std::sync::Arc<str>,
    version: Version,
    starts: std::sync::Arc<[usize]>,
}

impl Text {
    pub fn new(source: impl Into<String>, version: u64) -> Self {
        let source = source.into();
        let starts = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(byte, _)| byte + 1))
            .collect();
        Self {
            source: source.into(),
            version: Version(version),
            starts,
        }
    }

    #[allow(dead_code)] // Used by the benchmark sharing this fixture.
    pub fn at_version(&self, version: Version) -> Self {
        Self {
            version,
            ..self.clone()
        }
    }

    fn slice(&self, span: ByteRange) -> Result<&str> {
        self.source
            .get(span.start().0 as usize..span.end().0 as usize)
            .ok_or(Error::OutOfBounds)
    }
}

impl TextSource for Text {
    fn version(&self) -> Version {
        self.version
    }

    fn byte_len(&self) -> u64 {
        self.source.len() as u64
    }

    fn lines(&self) -> LineCount {
        LineCount {
            known: self.starts.len() as u64,
            indexed_bytes: ByteOffset(self.byte_len()),
            total_bytes: self.byte_len(),
            complete: true,
        }
    }

    fn line_range(&self, line: Line) -> Result<ByteRange> {
        let row = line.0 as usize;
        let start = *self.starts.get(row).ok_or(Error::OutOfBounds)?;
        let end = self
            .starts
            .get(row + 1)
            .copied()
            .unwrap_or(self.source.len());
        let line = &self.source[start..end];
        let content = line
            .strip_suffix("\r\n")
            .or_else(|| line.strip_suffix('\n'))
            .unwrap_or(line);
        Span::new(
            ByteOffset(start as u64),
            ByteOffset((start + content.len()) as u64),
        )
    }

    fn visit_chunks(&self, range: ByteRange, visitor: &mut dyn FnMut(&str)) -> Result<()> {
        visitor(self.slice(range)?);
        Ok(())
    }

    fn offset(&self, position: Position) -> Result<ByteOffset> {
        let span = self.line_range(position.line)?;
        let offset = span
            .start()
            .0
            .checked_add(position.column.0)
            .ok_or(Error::OutOfBounds)?;
        if offset <= span.end().0 && self.source.is_char_boundary(offset as usize) {
            Ok(ByteOffset(offset))
        } else {
            Err(Error::OutOfBounds)
        }
    }

    fn position(&self, offset: ByteOffset) -> Result<Position> {
        if !self.source.is_char_boundary(offset.0 as usize) {
            return Err(Error::OutOfBounds);
        }
        let row = self
            .starts
            .partition_point(|&start| start <= offset.0 as usize)
            - 1;
        let pos = Position {
            line: Line(row as u64),
            column: Utf8Column(offset.0 - self.starts[row] as u64),
        };
        self.offset(pos)?;
        Ok(pos)
    }

    fn utf16_to_utf8(&self, line: Line, column: Utf16Column) -> Result<Utf8Column> {
        let text = self.slice(self.line_range(line)?)?;
        let mut units = 0;
        for (byte, ch) in text.char_indices() {
            if units == column.0 {
                return Ok(Utf8Column(byte as u64));
            }
            units += ch.len_utf16() as u64;
        }
        if units == column.0 {
            Ok(Utf8Column(text.len() as u64))
        } else {
            Err(Error::OutOfBounds)
        }
    }

    fn utf8_to_utf16(&self, line: Line, column: Utf8Column) -> Result<Utf16Column> {
        let end = self.offset(Position { line, column })?;
        let start = self.line_range(line)?.start();
        Ok(Utf16Column(
            self.slice(Span::new(start, end)?)?.encode_utf16().count() as u64,
        ))
    }
}
