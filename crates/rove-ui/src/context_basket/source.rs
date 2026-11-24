use super::{model::LineRange, tracking::Anchor};
use rove_base::{Line, Position, Span, WorkspacePath};
use rove_buffer::{
    edit::{AppliedEdit, Bias},
    TextSource,
};

use std::{io, path::Path, sync::Arc};

pub fn open_workspace_file(
    root: &Path,
    relative: &WorkspacePath,
    settings: &dyn rove_config::Settings,
    policy: rove_buffer::SizePolicy,
) -> rove_base::Result<(Arc<str>, rove_buffer::OpenedFile)> {
    let mut path = root.to_path_buf();
    for component in Path::new(relative.as_str()).components() {
        path.push(component);
        let metadata = std::fs::symlink_metadata(&path)?;
        if workspace_link(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "basket source crosses a link",
            )
            .into());
        }
    }
    let language = rove_syntax::highlight::language_for_path(&path, Some(settings));
    let options = rove_buffer::LoadOptions::new(&language, settings);
    let opened = rove_buffer::open_file(&path, &options, policy)?;
    Ok((language, opened))
}

fn workspace_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

pub fn selection_lines(start: Position, end: Position) -> Option<LineRange> {
    let first = start.line.0 + 1;
    let last = end.line.0 + u64::from(end.line == start.line || end.column.0 != 0);
    LineRange::new(first, last)
}

pub fn read_excerpt(text: &dyn TextSource, range: LineRange) -> Option<String> {
    let start = text.line_range(Line(range.first() - 1)).ok()?.start();
    let end = text.line_range(Line(range.last() - 1)).ok()?.end();
    let mut excerpt = String::new();
    text.visit_chunks(Span::new(start, end).ok()?, &mut |chunk| {
        excerpt.push_str(chunk)
    })
    .ok()?;
    Some(excerpt)
}

pub fn capture_source(text: &dyn TextSource, range: LineRange) -> Option<Anchor> {
    let line_count = text.lines().known;
    let first = range.first().saturating_sub(1).max(1);
    let last = range.last().saturating_add(1).min(line_count);
    if range.last() > last {
        return None;
    }
    let rows = read_rows(text, first, last)?;
    let rows: Vec<_> = rows.iter().map(String::as_str).collect();
    Anchor::capture_window(usize::try_from(line_count).ok()?, first, &rows, range)
}

fn read_rows(text: &dyn TextSource, first: u64, last: u64) -> Option<Vec<String>> {
    (first..=last)
        .map(|index| {
            let mut row = String::new();
            let bytes = text.line_range(Line(index - 1)).ok()?;
            text.visit_chunks(bytes, &mut |chunk| row.push_str(chunk))
                .ok()?;
            Some(row)
        })
        .collect()
}

pub fn relocate_source(
    text: &dyn TextSource,
    range: LineRange,
    anchor: Anchor,
) -> Option<(LineRange, Anchor)> {
    let count = text.lines();
    if !count.complete {
        // An incomplete mapped line index does not prove that an apparently unique match is unique.
        return None;
    }
    let total = usize::try_from(count.known).ok()?;
    let (first, last) = anchor.relocation_window(range, total)?;
    let rows = read_rows(text, first, last)?;
    let rows: Vec<_> = rows.iter().map(String::as_str).collect();
    anchor.relocate_window(range, total, first, &rows)
}

pub fn map_edited_range(
    before: &dyn TextSource,
    after: &dyn TextSource,
    edits: &[AppliedEdit],
    range: LineRange,
) -> Option<LineRange> {
    let first = before.line_range(Line(range.first() - 1)).ok()?;
    let last = before.line_range(Line(range.last() - 1)).ok()?;
    let mut start = first.start();
    let mut end = last.end();
    let was_empty = start == end;
    for edit in edits {
        let replacements = edit.forward().edits();
        let start_inside = replacements
            .iter()
            .any(|change| change.range.start() <= start && start < change.range.end());
        let end_inside = replacements
            .iter()
            .any(|change| change.range.start() < end && end <= change.range.end());
        start = edit.map_offset(
            start,
            if start_inside {
                Bias::Before
            } else {
                Bias::After
            },
        );
        end = edit.map_offset(
            end,
            if end_inside || was_empty {
                Bias::After
            } else {
                Bias::Before
            },
        );
    }
    if start > end || (!was_empty && start == end) {
        return None;
    }
    selection_lines(after.position(start).ok()?, after.position(end).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::Utf8Column;
    use rove_buffer::RopeText;

    #[test]
    fn excludes_the_final_unselected_line_of_a_multiline_selection() {
        let start = Position {
            line: Line(3),
            column: Utf8Column(5),
        };
        let end = Position {
            line: Line(7),
            column: Utf8Column(0),
        };
        assert_eq!(selection_lines(start, end), LineRange::new(4, 7));
        assert_eq!(selection_lines(start, start), LineRange::new(4, 4));
    }

    #[cfg(windows)]
    #[test]
    fn context_basket_workspace_reads_reject_directory_junctions() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("source.txt"), "outside").unwrap();
        std::fs::write(workspace.path().join("inside.txt"), "inside").unwrap();
        let link = workspace.path().join("linked");
        let linked = std::process::Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(linked.status.success(), "{:?}", linked);
        let settings = rove_config::DefaultSettings(Default::default());
        let open = |path| {
            open_workspace_file(
                workspace.path(),
                &WorkspacePath::new(path).unwrap(),
                &settings,
                rove_buffer::SizePolicy::default(),
            )
        };
        assert!(open("inside.txt").is_ok());
        assert!(matches!(
            open("linked/source.txt"),
            Err(rove_base::Error::Io {
                kind: io::ErrorKind::PermissionDenied,
                ..
            })
        ));
    }

    #[test]
    fn source_fingerprint_matches_full_snapshot_without_copying_the_file() {
        let text = RopeText::from_text("other\r\nbefore\r\nselected\r\nafter\r\nother");
        let range = LineRange::new(3, 3).unwrap();
        let expected = Anchor::capture(&["other", "before", "selected", "after", "other"], range);
        assert_eq!(capture_source(&text, range), expected);
    }
    #[test]
    fn relocates_far_from_start_through_a_decoded_snapshot_window() {
        let prefix = "prefix\n".repeat(250);
        let original = RopeText::from_text(&format!("{prefix}before\nselected\nafter"));
        let updated = RopeText::from_text(&format!("{prefix}before\ninserted\nselected\nafter"));
        let old = LineRange::new(252, 252).unwrap();
        let anchor = capture_source(&original, old).unwrap();
        let (range, refreshed) = relocate_source(&updated, old, anchor).unwrap();
        assert_eq!(range, LineRange::new(253, 253).unwrap());
        assert_eq!(Some(refreshed), capture_source(&updated, range));
    }
    #[test]
    fn an_editor_transaction_moves_selected_lines_and_marks_total_deletion_stale() {
        use rove_base::{ByteOffset, Span};
        use rove_buffer::{
            edit::{EditBuffer, Transaction},
            Eol, LineEndings,
        };

        let mut buffer = EditBuffer::new("before\nselected\nafter", LineEndings::Uniform(Eol::Lf));
        let before = buffer.snapshot();
        let start = before.line_range(Line(1)).unwrap().start();
        let range = LineRange::new(2, 2).unwrap();
        let insert = Transaction::single(Span::new(start, start).unwrap(), "new\n");
        let change = buffer.apply(&insert).unwrap();
        let moved = map_edited_range(&*before, buffer.text(), &[change], range).unwrap();
        assert_eq!(moved, LineRange::new(3, 3).unwrap());

        let before_remove = buffer.snapshot();
        let bytes = before_remove.line_range(Line(2)).unwrap();
        let remove = Transaction::single(
            Span::new(bytes.start(), ByteOffset(bytes.end().0 + 1)).unwrap(),
            "",
        );
        let change = buffer.apply(&remove).unwrap();
        assert_eq!(
            map_edited_range(&*before_remove, buffer.text(), &[change], moved),
            None
        );
    }
}
