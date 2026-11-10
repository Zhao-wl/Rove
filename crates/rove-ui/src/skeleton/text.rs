//! SKELETON(buffer): UTF-8-only immutable buffer; replaced by #7.
use rove_base::*;
use rove_buffer::*;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
pub struct Utf8Text {
    text: Arc<str>,
    starts: Vec<usize>,
}
impl Utf8Text {
    pub fn new(text: String) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
        );
        Self {
            text: text.into(),
            starts,
        }
    }
    fn slice(&self, r: ByteRange) -> Result<&str> {
        self.text
            .get(r.start().0 as usize..r.end().0 as usize)
            .ok_or(Error::OutOfBounds)
    }
}
impl TextSource for Utf8Text {
    fn version(&self) -> Version {
        Version(0)
    }
    fn byte_len(&self) -> u64 {
        self.text.len() as u64
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
        let start = *self.starts.get(line.0 as usize).ok_or(Error::OutOfBounds)?;
        let mut end = self
            .starts
            .get(line.0 as usize + 1)
            .copied()
            .unwrap_or(self.text.len());
        if end > start && self.text.as_bytes()[end - 1] == b'\n' {
            end -= 1;
            if end > start && self.text.as_bytes()[end - 1] == b'\r' {
                end -= 1;
            }
        }
        Span::new(ByteOffset(start as u64), ByteOffset(end as u64))
    }
    fn visit_chunks(&self, r: ByteRange, v: &mut dyn FnMut(&str)) -> Result<()> {
        v(self.slice(r)?);
        Ok(())
    }
    fn offset(&self, p: Position) -> Result<ByteOffset> {
        let r = self.line_range(p.line)?;
        let o = r
            .start()
            .0
            .checked_add(p.column.0)
            .ok_or(Error::OutOfBounds)?;
        if o > r.end().0 || !self.text.is_char_boundary(o as usize) {
            return Err(Error::OutOfBounds);
        }
        Ok(ByteOffset(o))
    }
    fn position(&self, o: ByteOffset) -> Result<Position> {
        if o.0 > self.byte_len() || !self.text.is_char_boundary(o.0 as usize) {
            return Err(Error::OutOfBounds);
        }
        let line = self.starts.partition_point(|s| *s <= o.0 as usize) - 1;
        let range = self.line_range(Line(line as u64))?;
        if o > range.end() {
            return Err(Error::OutOfBounds);
        }
        Ok(Position {
            line: Line(line as u64),
            column: Utf8Column(o.0 - self.starts[line] as u64),
        })
    }
    fn utf16_to_utf8(&self, line: Line, column: Utf16Column) -> Result<Utf8Column> {
        let s = self.slice(self.line_range(line)?)?;
        let mut units = 0;
        for (byte, ch) in s.char_indices() {
            if units == column.0 {
                return Ok(Utf8Column(byte as u64));
            }
            units += ch.len_utf16() as u64;
        }
        if units == column.0 {
            Ok(Utf8Column(s.len() as u64))
        } else {
            Err(Error::OutOfBounds)
        }
    }
    fn utf8_to_utf16(&self, line: Line, column: Utf8Column) -> Result<Utf16Column> {
        self.offset(Position { line, column })?;
        let s = self.slice(self.line_range(line)?)?;
        Ok(Utf16Column(
            s[..column.0 as usize].encode_utf16().count() as u64
        ))
    }
}
/// SKELETON(editors): model cache/open policy replaced by #17. Only clean read-only views for now.
pub struct FileDocuments {
    root: PathBuf,
    cache: Arc<Mutex<BTreeMap<Resource, Arc<Document>>>>,
    next: Arc<AtomicU64>,
}
impl FileDocuments {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            cache: Default::default(),
            next: Arc::new(AtomicU64::new(1)),
        }
    }
}
impl Documents for FileDocuments {
    fn find(&self, r: &Resource) -> Option<Arc<Document>> {
        self.cache.lock().unwrap().get(r).cloned()
    }
    fn open(&self, resource: Resource, cancel: Cancellation) -> Task<Arc<Document>> {
        let root = self.root.clone();
        let cache = self.cache.clone();
        let next = self.next.clone();
        Box::pin(async move {
            cancel.check()?;
            if let Some(doc) = cache.lock().unwrap().get(&resource).cloned() {
                return Ok(doc);
            }
            let path = match &resource {
                Resource::Workspace(p) => root.join(p.as_str()),
                Resource::File(p) => p.clone(),
                _ => return Err(Error::Unsupported),
            };
            let path = std::fs::canonicalize(path)?;
            let identity = Resource::File(path.clone());
            {
                let mut docs = cache.lock().unwrap();
                if let Some(doc) = docs.get(&identity).cloned() {
                    docs.insert(resource, doc.clone());
                    return Ok(doc);
                }
            }
            let bytes = std::fs::read(&path)?;
            cancel.check()?;
            let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
            let text = String::from_utf8(if bom { bytes[3..].to_vec() } else { bytes })
                .map_err(|_| Error::Unsupported)?;
            let crlf = text.matches("\r\n").count();
            let lf = text.bytes().filter(|b| *b == b'\n').count();
            let eol = if crlf > 0 && crlf < lf {
                LineEndings::Mixed {
                    preferred: if crlf > lf / 2 { Eol::CrLf } else { Eol::Lf },
                }
            } else {
                LineEndings::Uniform(if crlf > 0 { Eol::CrLf } else { Eol::Lf })
            };
            let lang = if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("cs"))
            {
                "csharp"
            } else {
                "plaintext"
            };
            let mut docs = cache.lock().unwrap();
            let doc = docs
                .entry(identity)
                .or_insert_with(|| {
                    Arc::new(Document {
                        id: DocumentId(next.fetch_add(1, Ordering::Relaxed)),
                        resource: resource.clone(),
                        metadata: DocumentMetadata {
                            language: lang.into(),
                            encoding: Encoding::Utf8,
                            bom,
                            line_endings: eol,
                            read_only: Arc::from([ReadOnlyReason::Session]),
                            size: SizeClass::Normal,
                        },
                        text: Arc::new(Utf8Text::new(text)),
                    })
                })
                .clone();
            docs.insert(resource, doc.clone());
            Ok(doc)
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_coordinates_and_crlf() {
        let t = Utf8Text::new("a😀中\r\n".into());
        assert_eq!(t.lines().known, 2);
        assert_eq!(t.line_range(Line(0)).unwrap().end(), ByteOffset(8));
        assert_eq!(
            t.utf16_to_utf8(Line(0), Utf16Column(3)).unwrap(),
            Utf8Column(5)
        );
        assert!(t.utf16_to_utf8(Line(0), Utf16Column(2)).is_err());
        assert!(t
            .offset(Position {
                line: Line(0),
                column: Utf8Column(2)
            })
            .is_err());
    }
}
