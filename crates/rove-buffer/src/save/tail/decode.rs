//! Lossless UTF-8 append and line-ending counts for ordinary text buffers.
#[derive(Debug, Eq, PartialEq)]
pub enum Utf8Append<'a> {
    Complete(&'a str),
    /// Keep the prior published snapshot and disk marker; reread the suffix when more bytes arrive.
    Incomplete,
    Invalid,
}

pub fn utf8_append(bytes: &[u8]) -> Utf8Append<'_> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Utf8Append::Complete(text),
        Err(error) if error.error_len().is_none() => Utf8Append::Incomplete,
        Err(_) => Utf8Append::Invalid,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LineEndingCounts {
    pub lf: u64,
    pub crlf: u64,
    pub cr: u64,
    last_was_cr: bool,
}

impl LineEndingCounts {
    /// Keeps exact counts across chunks without counting a CRLF split across two writes twice.
    pub fn line_endings(&self) -> crate::LineEndings {
        use crate::{Eol, LineEndings};
        let kind_count = u8::from(self.lf > 0) + u8::from(self.crlf > 0) + u8::from(self.cr > 0);
        let preferred = if kind_count == 0 {
            Eol::Lf
        } else if self.crlf >= self.lf && self.crlf >= self.cr {
            Eol::CrLf
        } else if self.lf >= self.cr {
            Eol::Lf
        } else {
            Eol::Cr
        };
        if kind_count > 1 {
            LineEndings::Mixed { preferred }
        } else {
            LineEndings::Uniform(preferred)
        }
    }

    pub fn add(&mut self, text: &str) {
        for byte in text.bytes() {
            match byte {
                b'\r' => self.cr += 1,
                b'\n' if self.last_was_cr => {
                    self.cr -= 1;
                    self.crlf += 1;
                }
                b'\n' => self.lf += 1,
                _ => {}
            }
            self.last_was_cr = byte == b'\r';
        }
    }
}

/// FNV-1a from the previous save stamp, including the original bytes of the appended suffix.
pub fn extend_disk_hash(previous: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(previous, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_no_replacement_character_when_a_write_ends_inside_a_codepoint() {
        let append = "extra\n😀\n";
        let split = "extra\n".len() + 2;
        assert_eq!(
            utf8_append(&append.as_bytes()[..split]),
            Utf8Append::Incomplete
        );
        assert_eq!(utf8_append(append.as_bytes()), Utf8Append::Complete(append));
        assert_eq!(utf8_append(b"broken\xff"), Utf8Append::Invalid);
    }

    #[test]
    fn a_late_lf_completes_the_previous_standalone_cr() {
        let mut counts = LineEndingCounts::default();
        counts.add("a\r");
        assert_eq!((counts.cr, counts.crlf, counts.lf), (1, 0, 0));
        counts.add("");
        counts.add("\nb\nc\r");
        assert_eq!((counts.cr, counts.crlf, counts.lf), (1, 1, 1));
        counts.add("\n");
        assert_eq!((counts.cr, counts.crlf, counts.lf), (0, 2, 1));
    }

    #[test]
    fn append_extends_the_saved_raw_disk_hash() {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        let initial = b"\xef\xbb\xbfbefore\r";
        let appended = "\n追加\n".as_bytes();
        let joined = [initial.as_slice(), appended].concat();
        let hash = extend_disk_hash(extend_disk_hash(OFFSET, initial), appended);
        assert_eq!(hash, extend_disk_hash(OFFSET, &joined));
    }
}
