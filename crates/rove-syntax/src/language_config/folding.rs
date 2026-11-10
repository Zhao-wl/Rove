//! Fold ranges derived from a language configuration.
//!
//! This is a port of VS Code 1.145.2
//! `src/vs/editor/contrib/folding/browser/indentRangeProvider.ts`
//! (`computeRanges`, MIT). It produces marker folds (`folding.markers`, e.g. C#
//! `#region`) and indentation folds in one bottom-up pass, because the two share
//! the same stack: a region marker acts as an artificial indentation level, and
//! `folding.offSide` decides whether a whitespace-only line belongs to the block
//! above or below it.
//!
//! Behaviour that matters here and is covered by tests:
//!
//! - A `#region`/`#endregion` pair becomes a [`FoldKind::Region`] range from the
//!   `#region` line through the `#endregion` line, both included.
//! - Regions nest. An unmatched `#endregion` is ignored. An unmatched `#region`
//!   does not produce a fold (VS Code requires both markers); it only ends the
//!   indentation block above it.
//! - With `offSide`, blank lines belong to the next block, so a block's fold ends
//!   before its trailing blank lines. Without it, blank lines stay inside the
//!   block above.
//! - Indentation folds need at least one hidden line; a one-line block is not a
//!   fold, matching VS Code.
//!
//! Not ported: the 5000-region folding limit and its reporting hook, and the
//! `tabSize` source (callers pass the editor's value; VS Code's default is 4).
use super::{CompiledFoldingMarkers, MarkerKind};
use crate::{Fold, FoldKind};
use rove_base::{Line, LineRange, Result};

/// Compute marker and indentation folds for `text`.
///
/// `markers` are the compiled `folding.markers`, if the language has them.
/// Ranges are ordered by start line, outermost first for equal starts.
pub fn fold_ranges(
    markers: Option<&CompiledFoldingMarkers>,
    off_side: bool,
    text: &str,
    tab_size: usize,
) -> Result<Vec<Fold>> {
    let lines = split_lines(text);
    let line_count = lines.len();
    // 1-based line numbers, exactly like VS Code's `PreviousRegion`.
    let mut regions = vec![PreviousRegion {
        indent: -1,
        end_above: line_count + 1,
        line: line_count + 1,
    }];
    let mut folds: Vec<Fold> = Vec::new();

    for line in (1..=line_count).rev() {
        let content = lines[line - 1];
        let indent = compute_indent_level(content, tab_size);
        if indent == -1 {
            if off_side {
                // For off-side languages a blank line belongs to the block below,
                // so it is excluded from the block above.
                regions.last_mut().expect("sentinel").end_above = line;
            }
            continue;
        }
        let marker = match markers {
            Some(markers) => markers.kind(content)?,
            None => None,
        };
        match marker {
            Some(MarkerKind::Start) => {
                // Find the nearest marker region. Without one the line is
                // ordinary text, exactly as in VS Code.
                let mut index = regions.len() - 1;
                while index > 0 && regions[index].indent != -2 {
                    index -= 1;
                }
                if index > 0 {
                    regions.truncate(index + 1);
                    let previous = regions.last_mut().expect("marker region");
                    let end_line = previous.line;
                    folds.push(Fold {
                        lines: LineRange::new(Line((line - 1) as u64), Line(end_line as u64))?,
                        kind: FoldKind::Region,
                    });
                    previous.line = line;
                    previous.indent = indent;
                    previous.end_above = line;
                    continue;
                }
            }
            Some(MarkerKind::End) => {
                regions.push(PreviousRegion {
                    indent: -2,
                    end_above: line,
                    line,
                });
                continue;
            }
            None => {}
        }
        if regions.last().expect("sentinel").indent > indent {
            loop {
                regions.pop();
                if regions.last().expect("sentinel").indent <= indent {
                    break;
                }
            }
            let end_line = regions.last().expect("sentinel").end_above - 1;
            if end_line as isize - line as isize >= 1 {
                folds.push(Fold {
                    lines: LineRange::new(Line((line - 1) as u64), Line(end_line as u64))?,
                    kind: FoldKind::Indentation,
                });
            }
        }
        let previous = regions.last_mut().expect("sentinel");
        if previous.indent == indent {
            previous.end_above = line;
        } else {
            regions.push(PreviousRegion {
                indent,
                end_above: line,
                line,
            });
        }
    }
    // Ranges were prepended in VS Code's collector, i.e. inserted bottom-up.
    folds.reverse();
    Ok(folds)
}

struct PreviousRegion {
    /// Indentation, `-2` for a marker region, `-1` for the sentinel.
    indent: i64,
    /// Last line (1-based) belonging to the region above.
    end_above: usize,
    /// Start line (1-based) of the region, used for marker regions.
    line: usize,
}

/// Leading whitespace width in columns; `-1` for a whitespace-only line, as in
/// VS Code's `computeIndentLevel`.
fn compute_indent_level(line: &str, tab_size: usize) -> i64 {
    let tab_size = tab_size.max(1) as i64;
    let mut indent = 0i64;
    for character in line.chars() {
        match character {
            ' ' => indent += 1,
            '\t' => indent += tab_size - (indent % tab_size),
            _ => return indent,
        }
    }
    -1
}

/// Split like VS Code's text model: `\r\n`, `\r` and `\n` all end a line, the
/// terminator is not part of the content, and a trailing terminator yields a
/// final empty line.
fn split_lines(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                lines.push(&text[start..index]);
                index += 1;
                start = index;
            }
            b'\r' => {
                lines.push(&text[start..index]);
                index += 1;
                if bytes.get(index) == Some(&b'\n') {
                    index += 1;
                }
                start = index;
            }
            _ => index += 1,
        }
    }
    lines.push(&text[start..]);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{language_config::CompiledRegex, JsRegex};

    fn compiled(source: &str) -> CompiledRegex {
        CompiledRegex::compile(
            &JsRegex {
                source: source.into(),
                flags: "".into(),
            },
            &crate::language_config::RegexOrigin::new(
                "csharp",
                "test/language-configuration.json",
                "test",
            ),
        )
        .unwrap()
    }
    fn csharp_markers() -> CompiledFoldingMarkers {
        assembled(r"^\s*#region\b", r"^\s*#endregion\b")
    }
    fn assembled(start: &str, end: &str) -> CompiledFoldingMarkers {
        let start_js = JsRegex {
            source: start.into(),
            flags: "".into(),
        };
        let end_js = JsRegex {
            source: end.into(),
            flags: "".into(),
        };
        CompiledFoldingMarkers::assemble(
            "csharp",
            "test/language-configuration.json",
            compiled(start),
            compiled(end),
            (&start_js, &end_js),
        )
    }
    fn region_ranges(folds: &[Fold]) -> Vec<(u64, u64)> {
        folds
            .iter()
            .filter(|fold| matches!(fold.kind, FoldKind::Region))
            .map(|fold| (fold.lines.start().0, fold.lines.end().0))
            .collect()
    }
    fn indent_ranges(folds: &[Fold]) -> Vec<(u64, u64)> {
        folds
            .iter()
            .filter(|fold| matches!(fold.kind, FoldKind::Indentation))
            .map(|fold| (fold.lines.start().0, fold.lines.end().0))
            .collect()
    }

    #[test]
    fn lines_split_on_all_terminators() {
        assert_eq!(split_lines("a"), ["a"]);
        assert_eq!(split_lines("a\n"), ["a", ""]);
        assert_eq!(split_lines("a\r\nb\r"), ["a", "b", ""]);
        assert_eq!(split_lines(""), [""]);
    }

    #[test]
    fn indentation_is_counted_in_columns() {
        assert_eq!(compute_indent_level("", 4), -1);
        assert_eq!(compute_indent_level("   ", 4), -1);
        assert_eq!(compute_indent_level("\t x", 4), 5);
        assert_eq!(compute_indent_level("\t\tx", 4), 8);
        assert_eq!(compute_indent_level("    x", 4), 4);
    }

    #[test]
    fn markers_nest() {
        // Expected values from VS Code's computeRanges on the same input.
        let text = "#region Outer\nclass C\n{\n    #region Inner\n    void M() { }\n    #endregion\n}\n#endregion";
        let markers = csharp_markers();
        let folds = fold_ranges(Some(&markers), false, text, 4).unwrap();
        assert_eq!(region_ranges(&folds), [(0, 8), (3, 6)]);
        assert_eq!(indent_ranges(&folds), [(2, 6)]);
    }

    #[test]
    fn unmatched_end_marker_is_ignored() {
        let text = "class C\n{\n}\n#endregion";
        let markers = csharp_markers();
        let folds = fold_ranges(Some(&markers), false, text, 4).unwrap();
        assert!(region_ranges(&folds).is_empty());
    }

    #[test]
    fn unmatched_start_marker_is_ignored() {
        let text = "class C\n{\n    #region Inner\n    void M() { }\n}";
        let markers = csharp_markers();
        let folds = fold_ranges(Some(&markers), false, text, 4).unwrap();
        assert!(region_ranges(&folds).is_empty());
        assert_eq!(indent_ranges(&folds), [(1, 4)]);
    }

    #[test]
    fn off_side_moves_blank_lines_to_the_next_block() {
        let text = "class C\n{\n    void M()\n    {\n    }\n\n}";
        let markers = csharp_markers();
        let inside = fold_ranges(Some(&markers), false, text, 4).unwrap();
        assert_eq!(indent_ranges(&inside), [(1, 6)]);
        let outside = fold_ranges(Some(&markers), true, text, 4).unwrap();
        assert_eq!(indent_ranges(&outside), [(1, 5)]);
    }

    #[test]
    fn combined_matcher_scans_once_and_prefers_start() {
        let markers = assembled("^x", "^x");
        assert!(markers.is_combined());
        assert_eq!(markers.kind("x").unwrap(), Some(MarkerKind::Start));
        // A numbered backreference cannot be combined: the added group would
        // renumber it.
        let with_backref = assembled(r"(a)\1", "^end");
        assert!(!with_backref.is_combined());
        assert_eq!(with_backref.kind("aa").unwrap(), Some(MarkerKind::Start));
    }

    #[test]
    fn one_line_blocks_do_not_fold() {
        let markers = csharp_markers();
        assert!(fold_ranges(Some(&markers), false, "class C\n{\n}", 4)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn empty_text_has_no_folds() {
        assert!(fold_ranges(None, true, "", 4).unwrap().is_empty());
    }
}
