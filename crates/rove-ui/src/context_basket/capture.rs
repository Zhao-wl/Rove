use super::model::LineRange;

#[derive(Clone, Copy, Debug)]
pub enum SourceRange {
    WholeFile,
    Selected(LineRange),
    Around { line: u64, context: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeFailure {
    SelectionRequired,
    TooLong,
    NoSuchLine,
}

/// Resolve one-based ranges for each entry point before inserting anything into the basket.
pub fn allowed_range(
    selection: SourceRange,
    line_count: u64,
    huge: bool,
) -> Result<LineRange, RangeFailure> {
    match selection {
        SourceRange::WholeFile => {
            if huge {
                return Err(RangeFailure::SelectionRequired);
            }
            LineRange::new(1, line_count).ok_or(RangeFailure::TooLong)
        }
        SourceRange::Selected(range) if range.last() <= line_count => Ok(range),
        SourceRange::Selected(_) => Err(RangeFailure::NoSuchLine),
        SourceRange::Around { line, context } if line != 0 && line <= line_count => {
            let start = line.saturating_sub(context).max(1);
            let end = line.saturating_add(context).min(line_count);
            LineRange::new(start, end).ok_or(RangeFailure::TooLong)
        }
        SourceRange::Around { .. } => Err(RangeFailure::NoSuchLine),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_configured_context_on_both_sides_of_peek_or_search_hits() {
        assert_eq!(
            allowed_range(
                SourceRange::Around {
                    line: 6,
                    context: 3
                },
                20,
                false
            ),
            Ok(LineRange::new(3, 9).unwrap()),
        );
        assert_eq!(
            allowed_range(
                SourceRange::Around {
                    line: 2,
                    context: 5
                },
                20,
                false
            ),
            Ok(LineRange::new(1, 7).unwrap()),
        );
        assert_eq!(
            allowed_range(
                SourceRange::Around {
                    line: 19,
                    context: 3
                },
                20,
                false
            ),
            Ok(LineRange::new(16, 20).unwrap()),
        );
    }

    #[test]
    fn enforces_the_same_limit_without_rejecting_valid_selected_large_file_ranges() {
        assert_eq!(
            allowed_range(SourceRange::WholeFile, 10001, false),
            Err(RangeFailure::TooLong),
        );
        assert_eq!(
            allowed_range(SourceRange::WholeFile, 100, true),
            Err(RangeFailure::SelectionRequired),
        );
        let range = LineRange::new(8000, 8005).unwrap();
        assert_eq!(
            allowed_range(SourceRange::Selected(range), 11200, true),
            Ok(range)
        );
        assert_eq!(
            allowed_range(SourceRange::Selected(range), 4000, true),
            Err(RangeFailure::NoSuchLine),
        );
    }
}
