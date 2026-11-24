//! Relocate recorded ranges using fingerprints of the decoded text.
use super::model::LineRange;

const SEARCH_RADIUS: usize = 64;
const MAX_LINE_DELTA: i64 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub file_lines: usize,
    pub selected: u64,
    pub first: u64,
    pub last: u64,
    pub previous: Option<u64>,
    pub following: Option<u64>,
}

impl Anchor {
    pub fn capture(rows: &[&str], range: LineRange) -> Option<Self> {
        let start = usize::try_from(range.first() - 1).ok()?;
        let end = usize::try_from(range.last()).ok()?;
        let selected = rows.get(start..end)?;
        Some(Self {
            file_lines: rows.len(),
            selected: fingerprint(selected),
            first: fingerprint(&[*selected.first()?]),
            last: fingerprint(&[*selected.last()?]),
            previous: start
                .checked_sub(1)
                .map(|index| fingerprint(&[rows[index]])),
            following: rows.get(end).map(|row| fingerprint(&[*row])),
        })
    }

    /// Match a small edit or find one unambiguous copy of the previous selection nearby.
    /// A wholesale rewrite, conflicting anchors or ambiguous match needs manual re-selection.
    pub fn relocate(self, previous: LineRange, rows: &[&str]) -> Option<(LineRange, Self)> {
        let delta = i64::try_from(rows.len()).ok()? - i64::try_from(self.file_lines).ok()?;
        if delta.abs() > MAX_LINE_DELTA {
            return None;
        }
        let old = usize::try_from(previous.first() - 1).ok()?;
        let span = usize::try_from(previous.lines()).ok()?;
        if delta == 0 {
            if let Some(current) = Self::capture(rows, previous) {
                let endpoints_same = self.first == current.first && self.last == current.last;
                let surroundings_same =
                    self.previous == current.previous && self.following == current.following;
                let stable_inside_edit = span >= 3
                    && self.previous.is_some()
                    && self.following.is_some()
                    && endpoints_same;
                if surroundings_same && (self.selected == current.selected || stable_inside_edit) {
                    return Some((previous, current));
                }
            }
        }
        let guess = old.saturating_add_signed(delta as isize);
        let from = old.min(guess).saturating_sub(SEARCH_RADIUS);
        let through = old
            .max(guess)
            .saturating_add(SEARCH_RADIUS)
            .min(rows.len().checked_sub(span)?);
        let mut matched = None;
        for start in from..=through {
            let end = start + span;
            if fingerprint(&[rows[start]]) != self.first
                || fingerprint(&[rows[end - 1]]) != self.last
                || fingerprint(&rows[start..end]) != self.selected
            {
                continue;
            }
            if matched.is_some() {
                return None;
            }
            let range = LineRange::new(start as u64 + 1, end as u64)?;
            matched = Some((range, Self::capture(rows, range)?));
        }
        matched
    }
}

fn fingerprint(rows: &[&str]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for row in rows {
        for byte in row.as_bytes().iter().copied().chain(std::iter::once(b'\n')) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_before_the_selected_code_tracks_the_exact_range() {
        let rows = ["before", "fn calculate() {", "  sum", "}", "after"];
        let range = LineRange::new(2, 4).unwrap();
        let anchor = Anchor::capture(&rows, range).unwrap();
        let edited = [
            "before",
            "new comment",
            "fn calculate() {",
            "  sum",
            "}",
            "after",
        ];
        let (moved, _) = anchor.relocate(range, &edited).unwrap();
        assert_eq!(moved, LineRange::new(3, 5).unwrap());
    }

    #[test]
    fn stable_boundary_and_context_allow_an_edit_inside_the_selection() {
        let rows = ["before", "fn calculate() {", "  sum", "}", "after"];
        let range = LineRange::new(2, 4).unwrap();
        let anchor = Anchor::capture(&rows, range).unwrap();
        let edited = ["before", "fn calculate() {", "  doubled_sum", "}", "after"];
        let (retained, refreshed) = anchor.relocate(range, &edited).unwrap();
        assert_eq!(retained, range);
        assert_ne!(refreshed.selected, anchor.selected);
    }

    #[test]
    fn missing_or_duplicate_anchors_remain_ambiguous() {
        let rows = ["before", "fn calculate() {", "  sum", "}", "after"];
        let range = LineRange::new(2, 4).unwrap();
        let anchor = Anchor::capture(&rows, range).unwrap();
        assert!(anchor
            .relocate(range, &["totally", "different", "file"])
            .is_none());
        let duplicate = [
            "before",
            "fn calculate() {",
            "  sum",
            "}",
            "fn calculate() {",
            "  sum",
            "}",
            "after",
        ];
        assert!(anchor.relocate(range, &duplicate).is_none());
    }
}
