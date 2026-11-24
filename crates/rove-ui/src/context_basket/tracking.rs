//! Relocate recorded ranges using fingerprints of the decoded text.
use super::model::LineRange;

const SEARCH_RADIUS: usize = 64;
const MAX_LINE_DELTA: usize = 512;

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
    #[cfg(test)]
    pub fn capture(rows: &[&str], range: LineRange) -> Option<Self> {
        Self::capture_window(rows.len(), 1, rows, range)
    }

    pub fn capture_window(
        file_lines: usize,
        first_row: u64,
        rows: &[&str],
        range: LineRange,
    ) -> Option<Self> {
        let start = usize::try_from(range.first().checked_sub(first_row)?).ok()?;
        let end = usize::try_from(range.last().checked_sub(first_row)?).ok()? + 1;
        let selected = rows.get(start..end)?;
        Some(Self {
            file_lines,
            selected: fingerprint(selected),
            first: fingerprint(&[*selected.first()?]),
            last: fingerprint(&[*selected.last()?]),
            previous: start
                .checked_sub(1)
                .map(|index| fingerprint(&[rows[index]])),
            following: rows.get(end).map(|row| fingerprint(&[*row])),
        })
    }

    fn starts(
        self,
        previous: LineRange,
        file_lines: usize,
    ) -> Option<(usize, usize, usize, isize)> {
        let delta = isize::try_from(file_lines).ok()? - isize::try_from(self.file_lines).ok()?;
        if delta.unsigned_abs() > MAX_LINE_DELTA {
            return None;
        }
        let old = usize::try_from(previous.first() - 1).ok()?;
        let span = usize::try_from(previous.lines()).ok()?;
        let guess = old.saturating_add_signed(delta);
        let first = old.min(guess).saturating_sub(SEARCH_RADIUS);
        let through = old
            .max(guess)
            .saturating_add(SEARCH_RADIUS)
            .min(file_lines.checked_sub(span)?);
        (first <= through).then_some((first, through, span, delta))
    }

    /// The bounded set of rows required to test the nearby candidate locations, including
    /// one row of surrounding context where it exists. Positions here are one-based.
    pub fn relocation_window(self, previous: LineRange, file_lines: usize) -> Option<(u64, u64)> {
        let (first, through, span, _) = self.starts(previous, file_lines)?;
        let first_row = first.saturating_sub(1) + 1;
        let last_row = through
            .saturating_add(span)
            .saturating_add(1)
            .min(file_lines);
        Some((first_row as u64, last_row as u64))
    }

    /// Match a small edit or find one unambiguous copy of the previous selection nearby.
    /// A wholesale rewrite, conflicting anchors or ambiguous match needs manual re-selection.
    pub fn relocate_window(
        self,
        previous: LineRange,
        file_lines: usize,
        first_row: u64,
        rows: &[&str],
    ) -> Option<(LineRange, Self)> {
        let (from, through, span, delta) = self.starts(previous, file_lines)?;
        if delta == 0 {
            if let Some(current) = Self::capture_window(file_lines, first_row, rows, previous) {
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
        let window_start = usize::try_from(first_row.checked_sub(1)?).ok()?;
        let mut matched = None;
        for start in from..=through {
            let local = start.checked_sub(window_start)?;
            let end = local.checked_add(span)?;
            let content = rows.get(local..end)?;
            if fingerprint(&[*content.first()?]) != self.first
                || fingerprint(&[*content.last()?]) != self.last
                || fingerprint(content) != self.selected
            {
                continue;
            }
            if matched.is_some() {
                return None;
            }
            let range = LineRange::new(start as u64 + 1, (start + span) as u64)?;
            matched = Some((
                range,
                Self::capture_window(file_lines, first_row, rows, range)?,
            ));
        }
        matched
    }

    #[cfg(test)]
    pub fn relocate(self, previous: LineRange, rows: &[&str]) -> Option<(LineRange, Self)> {
        self.relocate_window(previous, rows.len(), 1, rows)
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
