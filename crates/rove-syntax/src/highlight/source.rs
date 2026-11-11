//! Parser input assembled from a [`TextSource`] in shared pieces.
//!
//! A full build copies the text once into pieces of at most [`PIECE`] bytes. An edit
//! against the previous source only rebuilds the pieces it touches and shares every other
//! piece, so the incremental path no longer copies the whole document. Tree-sitter reads
//! the pieces directly through [`Source::read`] and [`TextProvider`].

use super::grammar::Work;
use rove_base::{ByteOffset, Error, Result, Span};
use rove_buffer::TextSource;
use std::{borrow::Cow, ops::Range, sync::Arc};
use tree_sitter::{Node, TextProvider};

/// Target piece size. Pieces split only at UTF-8 scalar boundaries.
const PIECE: usize = 64 * 1024;
/// Bytes on each side of an edit that are compared against the new text.
pub(super) const EDIT_CONTEXT: usize = 256;

#[derive(Clone, Default)]
pub(super) struct Source {
    pieces: Arc<[Arc<str>]>,
    /// Start offset of each piece, plus the total length as the final entry.
    starts: Arc<[usize]>,
}

impl Source {
    /// Copy the whole text once, in reading order.
    pub fn read(text: &dyn TextSource, work: &Work<'_>) -> Result<Self> {
        let len = usize::try_from(text.byte_len()).map_err(|_| Error::OutOfBounds)?;
        let mut builder = Builder::default();
        let mut cancelled = false;
        text.visit_chunks(
            Span::new(ByteOffset(0), ByteOffset(len as u64))?,
            &mut |chunk| {
                if !cancelled {
                    cancelled = work.cancel.check().is_err();
                    builder.push(chunk);
                }
            },
        )?;
        work.cancel.check()?;
        let source = builder.finish();
        if source.len() != len {
            return Err(Error::InvalidInput);
        }
        Ok(source)
    }

    pub fn from_str(text: &str) -> Self {
        let mut builder = Builder::default();
        builder.push(text);
        builder.finish()
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_str(&String::from_utf8_lossy(bytes))
    }

    pub fn len(&self) -> usize {
        self.starts.last().copied().unwrap_or(0)
    }

    fn piece_at(&self, offset: usize) -> usize {
        self.starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1)
            .min(self.pieces.len().saturating_sub(1))
    }

    /// Bytes from `offset` to the end of its piece; empty at or past the end.
    pub fn chunk_at(&self, offset: usize) -> &[u8] {
        if offset >= self.len() {
            return &[];
        }
        let index = self.piece_at(offset);
        &self.pieces[index].as_bytes()[offset - self.starts[index]..]
    }

    /// Ordered borrowed slices covering `range` (clamped to the source).
    pub fn chunks(&self, range: Range<usize>) -> Chunks<'_> {
        let end = range.end.min(self.len());
        Chunks {
            source: self,
            at: range.start.min(end),
            end,
        }
    }

    /// Contiguous bytes of `range`; borrowed when it lies within one piece.
    pub fn bytes(&self, range: Range<usize>) -> Cow<'_, [u8]> {
        let mut chunks = self.chunks(range.clone());
        match (chunks.next(), chunks.next()) {
            (None, _) => Cow::Borrowed(&[]),
            (Some(first), None) => Cow::Borrowed(first),
            (Some(first), Some(second)) => {
                let mut owned = Vec::with_capacity(range.len());
                owned.extend_from_slice(first);
                owned.extend_from_slice(second);
                chunks.for_each(|chunk| owned.extend_from_slice(chunk));
                Cow::Owned(owned)
            }
        }
    }

    /// Text of `range`. Callers pass scalar-aligned ranges.
    pub fn text(&self, range: Range<usize>) -> Cow<'_, str> {
        match self.bytes(range) {
            Cow::Borrowed(bytes) => String::from_utf8_lossy(bytes),
            Cow::Owned(bytes) => Cow::Owned(
                String::from_utf8(bytes)
                    .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into()),
            ),
        }
    }

    pub fn byte(&self, offset: usize) -> Option<u8> {
        self.chunk_at(offset).first().copied()
    }

    /// The whole text as one slice; borrowed for single-piece sources.
    pub fn contiguous(&self) -> Cow<'_, str> {
        self.text(0..self.len())
    }

    /// Apply one replacement, reusing every piece outside the edited range.
    pub fn splice(&self, range: Range<usize>, inserted: &str) -> Option<Self> {
        if range.start > range.end || range.end > self.len() {
            return None;
        }
        if self.pieces.is_empty() {
            let mut builder = Builder::default();
            builder.push(inserted);
            return Some(builder.finish());
        }
        let first = self.piece_at(range.start);
        // The piece containing the end; an edit ending at a piece boundary keeps that piece.
        let last = if range.end > range.start {
            self.piece_at(range.end - 1)
        } else {
            first
        };
        let local = &self.pieces[first];
        let start_in = range.start - self.starts[first];
        let tail = &self.pieces[last];
        let end_in = range.end - self.starts[last];
        if !local.is_char_boundary(start_in) || !tail.is_char_boundary(end_in) {
            return None;
        }
        let mut builder = Builder::default();
        for piece in &self.pieces[..first] {
            builder.share(piece.clone());
        }
        builder.push(&local[..start_in]);
        builder.push(inserted);
        builder.push(&tail[end_in..]);
        builder.flush();
        for piece in &self.pieces[last + 1..] {
            builder.share(piece.clone());
        }
        Some(builder.finish())
    }
}

/// Feed tree-sitter queries from the pieces without joining them.
impl<'a> TextProvider<&'a [u8]> for &'a Source {
    type I = Chunks<'a>;

    fn text(&mut self, node: Node) -> Self::I {
        self.chunks(node.byte_range())
    }
}

pub(super) struct Chunks<'a> {
    source: &'a Source,
    at: usize,
    end: usize,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.at >= self.end {
            return None;
        }
        let chunk = self.source.chunk_at(self.at);
        let take = chunk.len().min(self.end - self.at);
        self.at += take;
        Some(&chunk[..take])
    }
}

#[derive(Default)]
struct Builder {
    pieces: Vec<Arc<str>>,
    starts: Vec<usize>,
    len: usize,
    pending: String,
}

impl Builder {
    fn push(&mut self, mut text: &str) {
        while !text.is_empty() {
            if self.pending.capacity() == 0 {
                self.pending.reserve(text.len().min(PIECE));
            }
            let room = PIECE.saturating_sub(self.pending.len());
            if text.len() <= room {
                self.pending.push_str(text);
                return;
            }
            let mut cut = room;
            while cut > 0 && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            if cut == 0 && self.pending.is_empty() {
                // A single scalar longer than the room cannot happen (PIECE > 4).
                cut = text.char_indices().nth(1).map_or(text.len(), |(i, _)| i);
            }
            self.pending.push_str(&text[..cut]);
            text = &text[cut..];
            self.flush();
        }
    }

    fn flush(&mut self) {
        if !self.pending.is_empty() {
            let piece: Arc<str> = std::mem::take(&mut self.pending).into();
            self.share(piece);
        }
    }

    fn share(&mut self, piece: Arc<str>) {
        debug_assert!(self.pending.is_empty());
        if piece.is_empty() {
            return;
        }
        self.starts.push(self.len);
        self.len += piece.len();
        self.pieces.push(piece);
    }

    fn finish(mut self) -> Source {
        self.flush();
        self.starts.push(self.len);
        Source {
            pieces: self.pieces.into(),
            starts: self.starts.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(source: &Source) -> String {
        source.contiguous().into_owned()
    }

    #[test]
    fn pieces_split_on_scalar_boundaries_and_splice_shares_the_rest() {
        let text = "é".repeat(PIECE) + "tail";
        let source = Source::from_str(&text);
        assert!(source.pieces.len() > 1);
        assert!(source.pieces.iter().all(|p| p.len() <= PIECE));
        assert_eq!(collect(&source), text);

        let at = PIECE * 2 - 2; // a boundary of "é" inside the second piece
        let edited = source.splice(at..at + 2, "xyz").unwrap();
        let mut expected = text.clone();
        expected.replace_range(at..at + 2, "xyz");
        assert_eq!(collect(&edited), expected);
        assert!(Arc::ptr_eq(&source.pieces[0], &edited.pieces[0]));
        assert!(source.splice(1..1, "x").is_none(), "inside a scalar");

        let bytes = edited.bytes(PIECE - 4..PIECE + 4);
        assert_eq!(&*bytes, &expected.as_bytes()[PIECE - 4..PIECE + 4]);
        assert_eq!(
            edited
                .chunks(0..edited.len())
                .map(<[u8]>::len)
                .sum::<usize>(),
            edited.len()
        );
    }

    #[test]
    fn splice_at_the_ends_and_of_the_empty_source() {
        let empty = Source::default();
        assert_eq!(collect(&empty.splice(0..0, "a").unwrap()), "a");
        let source = Source::from_str("abc");
        assert_eq!(collect(&source.splice(3..3, "d").unwrap()), "abcd");
        assert_eq!(collect(&source.splice(0..3, "").unwrap()), "");
        assert!(source.splice(2..4, "").is_none());
    }
}
