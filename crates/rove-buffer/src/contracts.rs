//! Read-only text and document contracts, implemented by #7/#8 and consumed by #17/#19.
use rove_base::*;
use std::sync::Arc;
/// Index progress; known includes the final empty line when complete. Empty text has one line.
#[derive(Clone, Debug)]
pub struct LineCount {
    pub known: u64,
    pub indexed_bytes: ByteOffset,
    pub total_bytes: u64,
    pub complete: bool,
}
/// Immutable snapshot. No full-text allocation: visit borrowed rope/mmap chunks; callbacks cannot
/// retain chunks. Metadata O(1), range lookup O(log n + chunks), iteration O(requested bytes).
/// Send + Sync; mmap access and coordinate conversion may fault, so execute on background workers.
/// Invalid ranges/scalar boundaries => OutOfBounds; unindexed lines => NotReady. A snapshot never
/// changes version while being read. UTF-8 is decoded text, disk byte offsets are deliberately absent.
pub trait TextSource: Send + Sync {
    fn version(&self) -> Version;
    fn byte_len(&self) -> u64;
    fn lines(&self) -> LineCount;
    /// Excludes the line terminator; chunks are ordered, UTF-8 aligned, and cover the range exactly.
    fn line_range(&self, line: Line) -> Result<ByteRange>;
    fn visit_chunks(&self, range: ByteRange, visitor: &mut dyn FnMut(&str)) -> Result<()>;
    fn offset(&self, position: Position) -> Result<ByteOffset>;
    fn position(&self, offset: ByteOffset) -> Result<Position>;
    fn utf16_to_utf8(&self, line: Line, column: Utf16Column) -> Result<Utf8Column>;
    fn utf8_to_utf16(&self, line: Line, column: Utf8Column) -> Result<Utf16Column>;
}
/// Fixed empty snapshot; provides one empty line and only position (0,0). No allocation or I/O.
#[derive(Default)]
pub struct EmptyText;
impl TextSource for EmptyText {
    fn version(&self) -> Version {
        Version(0)
    }
    fn byte_len(&self) -> u64 {
        0
    }
    fn lines(&self) -> LineCount {
        LineCount {
            known: 1,
            indexed_bytes: ByteOffset(0),
            total_bytes: 0,
            complete: true,
        }
    }
    fn line_range(&self, l: Line) -> Result<ByteRange> {
        if l.0 == 0 {
            Span::new(ByteOffset(0), ByteOffset(0))
        } else {
            Err(Error::OutOfBounds)
        }
    }
    fn visit_chunks(&self, r: ByteRange, _: &mut dyn FnMut(&str)) -> Result<()> {
        if r.end().0 == 0 {
            Ok(())
        } else {
            Err(Error::OutOfBounds)
        }
    }
    fn offset(&self, p: Position) -> Result<ByteOffset> {
        if p == Position::default() {
            Ok(ByteOffset(0))
        } else {
            Err(Error::OutOfBounds)
        }
    }
    fn position(&self, b: ByteOffset) -> Result<Position> {
        if b.0 == 0 {
            Ok(Position::default())
        } else {
            Err(Error::OutOfBounds)
        }
    }
    fn utf16_to_utf8(&self, l: Line, c: Utf16Column) -> Result<Utf8Column> {
        // Only the empty line's origin exists; this is not a general unit conversion.
        if l == Line(0) && c == Utf16Column(0) {
            Ok(Utf8Column(0))
        } else {
            Err(Error::OutOfBounds)
        }
    }
    fn utf8_to_utf16(&self, l: Line, c: Utf8Column) -> Result<Utf16Column> {
        self.offset(Position { line: l, column: c })?;
        Ok(Utf16Column(0))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Gbk,
    Other(Arc<str>),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Lf,
    CrLf,
    Cr,
}
/// Mixed preserves original terminators; preferred is only for newly inserted lines.
#[derive(Clone, Debug)]
pub enum LineEndings {
    Uniform(Eol),
    Mixed { preferred: Eol },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeClass {
    Normal,
    Large,
    Huge,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOnlyReason {
    HugeFile,
    Metadata,
    Permissions,
    Rule(Arc<str>),
    Session,
}
/// Immutable metadata attached to a snapshot; several read-only reasons may apply simultaneously.
#[derive(Clone, Debug)]
pub struct DocumentMetadata {
    pub language: Arc<str>,
    pub encoding: Encoding,
    pub bom: bool,
    pub line_endings: LineEndings,
    pub read_only: Arc<[ReadOnlyReason]>,
    pub size: SizeClass,
}
#[derive(Clone)]
pub struct Document {
    pub id: DocumentId,
    pub resource: Resource,
    pub metadata: DocumentMetadata,
    pub text: Arc<dyn TextSource>,
}
/// Document model access, no editor-group/focus policy. O(log open documents) find; open is a
/// cancellable background operation, returns Io/Cancelled/Unsupported rather than false success.
/// Providers deduplicate resource aliases, publish immutable snapshots, and preserve document ID.
pub trait Documents: Send + Sync {
    fn find(&self, resource: &Resource) -> Option<Arc<Document>>;
    fn open(&self, resource: Resource, cancel: Cancellation) -> Task<Arc<Document>>;
}
/// No documents available; no filesystem access. #2 replaces this fixed provider.
#[derive(Default)]
pub struct EmptyDocuments;
impl Documents for EmptyDocuments {
    fn find(&self, _: &Resource) -> Option<Arc<Document>> {
        None
    }
    fn open(&self, _: Resource, c: Cancellation) -> Task<Arc<Document>> {
        Box::pin(async move {
            c.check()?;
            Err(Error::Unsupported)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renderer_reads_one_line_without_copy() {
        let text: &dyn TextSource = &EmptyText;
        let mut len = 0;
        text.visit_chunks(text.line_range(Line(0)).unwrap(), &mut |s| len += s.len())
            .unwrap();
        assert_eq!(len, 0);
        assert!(text.line_range(Line(1)).is_err());
        assert_eq!(
            text.utf16_to_utf8(Line(0), Utf16Column(0)).unwrap(),
            Utf8Column(0)
        );
        assert_eq!(
            text.utf8_to_utf16(Line(0), Utf8Column(0)).unwrap(),
            Utf16Column(0)
        );
    }
    #[test]
    fn empty_text_rejects_nonzero_conversion_coordinates() {
        for (line, column) in [(0, 1), (1, 0), (u64::MAX, u64::MAX)] {
            assert_eq!(
                EmptyText.utf16_to_utf8(Line(line), Utf16Column(column)),
                Err(Error::OutOfBounds)
            );
            assert_eq!(
                EmptyText.utf8_to_utf16(Line(line), Utf8Column(column)),
                Err(Error::OutOfBounds)
            );
        }
    }
    #[test]
    fn consumer_finds_no_document() {
        assert!(EmptyDocuments
            .find(&Resource::Virtual("empty".into()))
            .is_none());
    }
}
