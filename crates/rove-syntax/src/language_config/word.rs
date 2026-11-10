//! The word under the cursor, from `wordPattern` or VS Code's default pattern.
//!
//! VS Code only defines `wordPattern` for Markdown and XML among the shipped
//! configurations; every other language uses `DEFAULT_WORD_REGEXP` from
//! `src/vs/editor/common/core/wordHelper.ts`. [`DEFAULT_WORD_PATTERN`] and
//! [`DEFAULT_WORD_PATTERN_FLAGS`] are that regular expression verbatim (MIT), so
//! double-click selection, word movement and same-word highlighting agree with
//! VS Code for the other languages too.
//!
//! [`word_at`] ports `getWordAtText` (same file). Its cursor bias is intentional:
//! the match must satisfy `start <= cursor <= end`, so a cursor sitting just after
//! a word still selects that word. Long lines are truncated to a 1000-byte window
//! around the cursor exactly like VS Code's `maxLen` heuristic; results only
//! differ from VS Code for words longer than that window. VS Code's `timeBudget`
//! window scan is not reproduced: it yields the same match as scanning from the
//! start of the line unless the budget expires.
use super::CompiledRegex;
use rove_base::{ByteOffset, ByteRange, Error, Result};

/// VS Code's default word definition (`DEFAULT_WORD_REGEXP`), used when a
/// language configuration has no `wordPattern`.
pub const DEFAULT_WORD_PATTERN: &str = "(-?\\d*\\.\\d\\w*)|([^\\`\\~\\!\\@\\#\\$\\%\\^\\&\\*\\(\\)\\-\\=\\+\\[\\{\\]\\}\\\\\\|\\;\\:\\'\\\"\\,\\.\\<\\>\\/\\?\\s]+)";
/// Flags VS Code compiles [`DEFAULT_WORD_PATTERN`] with (`new RegExp(source, 'g')`).
pub const DEFAULT_WORD_PATTERN_FLAGS: &str = "g";

/// Lines longer than this are truncated to a window before matching, as in VS
/// Code's `getWordAtText` (`maxLen`).
const MAX_LINE_LEN: usize = 1000;

/// Byte range of the word containing `offset`, or `None` when the cursor is not
/// inside a word. `offset` must be a UTF-8 scalar boundary of `text`.
pub fn word_at(
    pattern: &CompiledRegex,
    text: &str,
    offset: ByteOffset,
) -> Result<Option<ByteRange>> {
    let offset = offset.0 as usize;
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err(Error::OutOfBounds);
    }
    let (haystack, base) = window(text, offset);
    let position = offset - base;
    let mut cursor = 0;
    while cursor <= haystack.len() {
        let Some((start, end)) = pattern.find_from(haystack, cursor)? else {
            break;
        };
        // Matches are ordered by start: once one starts after the cursor no later
        // match can contain it.
        if start > position {
            break;
        }
        if end >= position {
            return Ok(Some(ByteRange::new(
                ByteOffset((base + start) as u64),
                ByteOffset((base + end) as u64),
            )?));
        }
        cursor = if end > start {
            end
        } else {
            next_char_boundary(haystack, end)
        };
    }
    Ok(None)
}

/// Line window around `offset`; returns the slice and its byte offset in `text`.
fn window(text: &str, offset: usize) -> (&str, usize) {
    if text.len() <= MAX_LINE_LEN {
        return (text, 0);
    }
    let half = MAX_LINE_LEN / 2;
    let start = floor_char_boundary(text, offset.saturating_sub(half));
    let end = ceil_char_boundary(text, (offset + half).min(text.len()));
    (&text[start..end], start)
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn next_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        text.len() + 1
    } else {
        ceil_char_boundary(text, index + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{language_config::RegexOrigin, JsRegex};

    fn default_pattern() -> CompiledRegex {
        CompiledRegex::compile(
            &JsRegex {
                source: DEFAULT_WORD_PATTERN.into(),
                flags: DEFAULT_WORD_PATTERN_FLAGS.into(),
            },
            &RegexOrigin::new(
                "csharp",
                "test/language-configuration.json",
                "wordPattern.pattern",
            ),
        )
        .unwrap()
    }

    fn word(text: &str, cursor: usize) -> Option<(u64, u64)> {
        word_at(&default_pattern(), text, ByteOffset(cursor as u64))
            .unwrap()
            .map(|range| (range.start().0, range.end().0))
    }

    #[test]
    fn numbers_are_words_of_their_own() {
        assert_eq!(word("var x = 1.5 + y;", 8), Some((8, 11)));
    }

    #[test]
    fn dots_separate_words_by_default() {
        assert_eq!(word("System.Console.WriteLine", 8), Some((7, 14)));
    }

    #[test]
    fn underscores_stay_in_the_word() {
        assert_eq!(word("snake_case", 3), Some((0, 10)));
    }

    #[test]
    fn cursor_after_the_last_character_selects_the_word() {
        assert_eq!(word("hello world", 5), Some((0, 5)));
        assert_eq!(word("hello world", 6), Some((6, 11)));
    }

    #[test]
    fn whitespace_and_punctuation_can_select_nothing() {
        assert_eq!(word("   ", 1), None);
        assert_eq!(word("=", 0), None);
    }

    #[test]
    fn offsets_must_be_scalar_boundaries() {
        let pattern = default_pattern();
        assert_eq!(
            word_at(&pattern, "é", ByteOffset(1)),
            Err(Error::OutOfBounds)
        );
        assert_eq!(
            word_at(&pattern, "é", ByteOffset(9)),
            Err(Error::OutOfBounds)
        );
    }

    #[test]
    fn long_lines_are_windowed() {
        let mut text = "x".repeat(600);
        text.push_str(" cursor ");
        text.push_str(&"y".repeat(600));
        let offset = 600 + 3;
        let (haystack, base) = window(&text, offset);
        assert!(base <= offset);
        assert!(base + haystack.len() >= offset);
        assert!(haystack.len() <= MAX_LINE_LEN);
        assert_eq!(
            word_at(&default_pattern(), &text, ByteOffset(offset as u64)).unwrap(),
            Some(ByteRange::new(ByteOffset(601), ByteOffset(607)).unwrap())
        );
    }
}
