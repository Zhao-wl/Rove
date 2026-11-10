use std::ops::Range;

// IME selection offsets are relative to the replacement string, not the document.
pub fn composition_selection(
    start: usize,
    text: &str,
    selection: Option<Range<usize>>,
) -> Range<usize> {
    fn byte_offset(text: &str, target: usize) -> usize {
        let mut units = 0;
        for (byte, ch) in text.char_indices() {
            if units >= target {
                return byte;
            }
            units += ch.len_utf16();
        }
        text.len()
    }
    match selection {
        Some(range) => start + byte_offset(text, range.start)..start + byte_offset(text, range.end),
        None => start + text.len()..start + text.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ascii_composition_after_cjk_prefix() {
        assert_eq!(
            composition_selection("中文".len(), "nihao", Some(5..5)),
            11..11
        );
    }
    #[test]
    fn cjk_composition_after_emoji_prefix() {
        let start = "A😀中文".len();
        assert_eq!(
            composition_selection(start, "你好", Some(1..2)),
            start + 3..start + 6
        );
    }
    #[test]
    fn surrogate_pair_inside_composition() {
        assert_eq!(composition_selection(3, "😀好", Some(2..3)), 7..10);
    }
    #[test]
    fn default_and_out_of_range_stay_in_replacement() {
        assert_eq!(composition_selection(6, "你", None), 9..9);
        assert_eq!(composition_selection(6, "你", Some(99..99)), 9..9);
        assert_eq!(composition_selection(6, "", Some(0..0)), 6..6);
    }
}
