//! Parallel, streaming scan (#15).
//!
//! Uses the `ignore` parallel walker with **every** ignore-file switch turned off: git ignore,
//! git global, git exclude, `.ignore`, parent directories and hidden-file filtering. The product
//! hard constraint is that `.gitignore` is never consulted (AGENTS §1); exclusion goes through
//! `files.exclude`/`rove.files.extraExclude` only. A regression test keeps the switch honest.
//!
//! One unreadable directory or one non-UTF-8 name must not fail the scan: walk errors and names
//! that cannot be represented as a [`WorkspacePath`] are counted and skipped.
//!
//! The walker feeds a bounded channel (`sync_channel`), so a slow snapshot publisher exerts
//! backpressure instead of growing an unbounded queue.

use crate::contracts::EntryKind;
use crate::folder::Folder;
use crate::glob::RuleSet;
use crate::path::relative_str;
use crate::snapshot::Snapshot;
use ignore::{DirEntry, WalkBuilder, WalkState};
use rove_base::{Cancellation, Error, Result, WorkspacePath};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One scanned entry: workspace path, kind, and whether the disk attribute says read-only.
pub(crate) type ScanEntry = (WorkspacePath, EntryKind, bool);

const FLUSH_ENTRIES: usize = 4096;
const FLUSH_INTERVAL: Duration = Duration::from_millis(25);
const QUEUE_CAPACITY: usize = 8192;

/// Full scan of the folder's root, publishing batches as they arrive.
///
/// Runs the walker on the calling thread and a publisher thread that owns the snapshot writes.
pub(crate) fn full(folder: Arc<Folder>, cancel: Cancellation, emit_events: bool) -> Result<()> {
    let root = folder.root().to_path_buf();
    let exclude = folder.config().exclude.clone();
    let threads = folder.config().threads;
    let track_permissions = folder.config().readonly.from_permissions;

    let (sender, receiver) = mpsc::sync_channel::<ScanEntry>(QUEUE_CAPACITY);
    let publisher_folder = Arc::clone(&folder);
    let publisher = std::thread::Builder::new()
        .name("rove-workspace-publish".into())
        .spawn(move || {
            let mut batch: Vec<ScanEntry> = Vec::new();
            let mut last_flush = Instant::now();
            for entry in receiver {
                batch.push(entry);
                if batch.len() >= FLUSH_ENTRIES || last_flush.elapsed() >= FLUSH_INTERVAL {
                    publisher_folder.publish_entries(std::mem::take(&mut batch), emit_events);
                    last_flush = Instant::now();
                }
            }
            if !batch.is_empty() {
                publisher_folder.publish_entries(batch, emit_events);
            }
        })?;

    let skipped = walk(&root, &exclude, &cancel, threads, track_permissions, sender);
    if publisher.join().is_err() {
        return Err(Error::Unsupported);
    }
    folder.note_skipped(skipped);
    cancel.check()?;
    Ok(())
}

/// Scans the whole tree into a fresh [`Snapshot`] without touching the live one.
///
/// Used by overflow recovery: the replacement is built off-lock and swapped in atomically.
pub(crate) fn collect(folder: &Folder, cancel: &Cancellation) -> Result<Snapshot> {
    let root = folder.root().to_path_buf();
    let exclude = folder.config().exclude.clone();
    let threads = folder.config().threads;
    let track_permissions = folder.config().readonly.from_permissions;

    let (sender, receiver) = mpsc::sync_channel::<ScanEntry>(QUEUE_CAPACITY);
    let walker_cancel = cancel.clone();
    let walker = std::thread::Builder::new()
        .name("rove-workspace-rescan".into())
        .spawn(move || {
            walk(
                &root,
                &exclude,
                &walker_cancel,
                threads,
                track_permissions,
                sender,
            )
        })?;

    let mut snapshot = Snapshot::new();
    for (path, kind, read_only) in receiver {
        snapshot.add(path.clone(), kind);
        if read_only {
            snapshot.set_permission_read_only(&path, true);
        }
    }
    let skipped = walker.join().map_err(|_| Error::Unsupported)?;
    folder.note_skipped(skipped);
    cancel.check()?;
    snapshot.finish();
    Ok(snapshot)
}

/// Runs the parallel walker and sends every visible entry through `sender`.
///
/// Returns the number of entries skipped because they were unreadable or could not be represented
/// as a [`WorkspacePath`]; a skipped entry never aborts the scan.
fn walk(
    root: &Path,
    exclude: &RuleSet,
    cancel: &Cancellation,
    threads: usize,
    track_permissions: bool,
    sender: SyncSender<ScanEntry>,
) -> u64 {
    let skipped = Arc::new(AtomicU64::new(0));

    let mut builder = base_builder(root);
    if threads > 0 {
        builder.threads(threads);
    }
    {
        let root = root.to_path_buf();
        let exclude = exclude.clone();
        let cancel = cancel.clone();
        builder.filter_entry(move |entry| should_visit(&root, &exclude, &cancel, entry));
    }

    let root_for_visitor = root.to_path_buf();
    let cancel_for_visitor = cancel.clone();
    let skipped_for_visitor = Arc::clone(&skipped);
    builder.build_parallel().run(move || {
        let sender = sender.clone();
        let skipped = Arc::clone(&skipped_for_visitor);
        let cancel = cancel_for_visitor.clone();
        let root = root_for_visitor.clone();
        Box::new(
            move |result: std::result::Result<DirEntry, ignore::Error>| {
                if cancel.check().is_err() {
                    return WalkState::Quit;
                }
                let entry = match result {
                    Ok(entry) => entry,
                    Err(_) => {
                        skipped.fetch_add(1, Ordering::Relaxed);
                        return WalkState::Continue;
                    }
                };
                if entry.depth() == 0 {
                    return WalkState::Continue;
                }
                let Some(kind) = entry_kind(&entry) else {
                    skipped.fetch_add(1, Ordering::Relaxed);
                    return WalkState::Continue;
                };
                let Some(relative) = relative_str(&root, entry.path()) else {
                    skipped.fetch_add(1, Ordering::Relaxed);
                    return WalkState::Continue;
                };
                let Ok(path) = WorkspacePath::new(&relative) else {
                    skipped.fetch_add(1, Ordering::Relaxed);
                    return WalkState::Continue;
                };
                let read_only = track_permissions
                    && matches!(kind, EntryKind::File)
                    && is_read_only_attribute(entry.path());
                if sender.send((path, kind, read_only)).is_err() {
                    return WalkState::Quit;
                }
                WalkState::Continue
            },
        )
    });

    skipped.load(Ordering::Relaxed)
}

/// Scans one directory subtree serially and returns the visible entries below it plus the
/// directories that need a watcher (relative paths, `absolute_dir` excluded).
/// Used by the watcher when a directory is created or renamed into place.
pub(crate) fn subtree(
    folder: &Folder,
    absolute_dir: &Path,
) -> (Vec<ScanEntry>, Vec<WorkspacePath>) {
    let root = folder.root().to_path_buf();
    let exclude = folder.config().exclude.clone();
    let track_permissions = folder.config().readonly.from_permissions;
    let cancel = Cancellation::default();
    let mut builder = base_builder(absolute_dir);
    {
        let root = root.clone();
        let exclude = exclude.clone();
        let cancel = cancel.clone();
        builder.filter_entry(move |entry| should_visit(&root, &exclude, &cancel, entry));
    }
    let mut entries = Vec::new();
    let mut dirs = Vec::new();
    for result in builder.build() {
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if entry.depth() == 0 {
            continue;
        }
        let Some(kind) = entry_kind(&entry) else {
            continue;
        };
        let Some(relative) = relative_str(&root, entry.path()) else {
            continue;
        };
        let Ok(path) = WorkspacePath::new(&relative) else {
            continue;
        };
        let read_only = track_permissions
            && matches!(kind, EntryKind::File)
            && is_read_only_attribute(entry.path());
        if matches!(kind, EntryKind::Directory) {
            dirs.push(path.clone());
        }
        entries.push((path, kind, read_only));
    }
    (entries, dirs)
}

fn base_builder(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        // Hard constraint: never read `.gitignore`/`.git/info/exclude`/global ignore files.
        // `standard_filters(false)` turns all of them plus hidden-file filtering off; the
        // explicit calls document each switch so a future change cannot silently re-enable one.
        .standard_filters(false)
        .hidden(false)
        .ignore(false)
        .parents(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(false);
    builder
}

fn should_visit(root: &Path, exclude: &RuleSet, cancel: &Cancellation, entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if cancel.check().is_err() {
        return false;
    }
    let Some(relative) = relative_str(root, entry.path()) else {
        // Non-UTF-8 names and paths outside the root are skipped (they cannot be a
        // `WorkspacePath`), and directories among them are not descended into.
        return false;
    };
    let Ok(path) = WorkspacePath::new(&relative) else {
        return false;
    };
    if exclude.matches_self(path.as_str()) == Some(true) {
        return false;
    }
    if is_reparse_reentry(entry) {
        return false;
    }
    true
}

fn entry_kind(entry: &DirEntry) -> Option<EntryKind> {
    let file_type = entry.file_type()?;
    if file_type.is_dir() {
        Some(EntryKind::Directory)
    } else if file_type.is_file() {
        Some(EntryKind::File)
    } else {
        None
    }
}

/// Skips symlinks and Windows reparse points (junctions) so the scan never walks out of the
/// workspace or recurses through a cycle.
fn is_reparse_reentry(entry: &DirEntry) -> bool {
    let Some(file_type) = entry.file_type() else {
        return true;
    };
    is_reparse_path(entry.path(), &file_type)
}

/// True for symlinks and Windows reparse points (junctions, mount points).
///
/// The attribute check is only done for directories: a junction can only be a directory, and
/// stat-ing every file would dominate the scan (cloud placeholders and other files also carry
/// reparse points but are perfectly readable).
pub(crate) fn is_reparse_path(path: &Path, file_type: &std::fs::FileType) -> bool {
    if file_type.is_symlink() {
        return true;
    }
    file_type.is_dir() && is_reparse_attribute(path)
}

#[cfg(windows)]
fn is_reparse_attribute(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_reparse_attribute(_: &Path) -> bool {
    false
}

/// On-disk read-only attribute (`files.readonlyFromPermissions`).
pub(crate) fn is_read_only_attribute(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().readonly())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::Workspace;
    use crate::rules::WorkspaceConfig;
    use rove_base::WorkspacePath;

    fn config_with_excludes(excludes: &[(&str, bool)]) -> WorkspaceConfig {
        let mut config = WorkspaceConfig::default().without_watching();
        config.exclude = crate::glob::RuleSet::from_value(&rove_base::Value::Object(Arc::new(
            excludes
                .iter()
                .map(|(pattern, enabled)| (pattern.to_string(), rove_base::Value::Bool(*enabled)))
                .collect(),
        )))
        .unwrap();
        config
    }

    #[test]
    fn gitignore_is_never_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Assets")).unwrap();
        std::fs::write(dir.path().join("Assets/A.cs"), "class A {}\n").unwrap();
        std::fs::write(dir.path().join("Assets/Ignored.cs"), "class I {}\n").unwrap();
        std::fs::write(dir.path().join(".gitignore"), "Assets/Ignored.cs\n").unwrap();
        let folder =
            Folder::scan_root(dir.path(), WorkspaceConfig::default().without_watching()).unwrap();
        let files = folder.files(0..100).unwrap();
        let paths: Vec<&str> = files
            .items
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();
        assert!(paths.contains(&"Assets/A.cs"), "paths: {paths:?}");
        assert!(paths.contains(&"Assets/Ignored.cs"), "paths: {paths:?}");
    }

    #[test]
    fn exclusions_apply_at_every_depth() {
        let dir = tempfile::tempdir().unwrap();
        for directory in [
            "Library",
            "Assets/Library",
            "Assets/Deep/obj",
            "Assets/Logs",
        ] {
            std::fs::create_dir_all(dir.path().join(directory)).unwrap();
            std::fs::write(dir.path().join(directory).join("A.cs"), "class A {}\n").unwrap();
        }
        std::fs::write(dir.path().join("Keep.cs"), "class Keep {}\n").unwrap();
        let folder =
            Folder::scan_root(dir.path(), WorkspaceConfig::default().without_watching()).unwrap();
        let paths: Vec<String> = folder
            .files(0..100)
            .unwrap()
            .items
            .iter()
            .map(|entry| entry.path.as_str().to_string())
            .collect();
        assert_eq!(paths, ["Keep.cs"]);
    }

    #[test]
    fn a_non_utf8_name_is_skipped_without_failing_the_scan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Good.cs"), "class Good {}\n").unwrap();
        let mut created_bad_name = false;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            // A lone surrogate is a legal Windows file name and cannot be valid UTF-8.
            let mut wide: Vec<u16> = "Bad".encode_utf16().collect();
            wide.push(0xD800);
            wide.extend(".cs".encode_utf16());
            let name = std::ffi::OsString::from_wide(&wide);
            match std::fs::write(dir.path().join(&name), b"x") {
                Ok(()) => created_bad_name = true,
                Err(error) => eprintln!("已跳过：无法创建非 UTF-8 文件名（{error}）"),
            }
        }
        let folder =
            Folder::scan_root(dir.path(), WorkspaceConfig::default().without_watching()).unwrap();
        let page = folder.files(0..100).unwrap();
        let paths: Vec<&str> = page.items.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(paths, ["Good.cs"]);
        if created_bad_name {
            assert_eq!(folder.skipped(), 1);
        }
    }

    #[test]
    fn user_false_reincludes_library() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Library")).unwrap();
        std::fs::write(dir.path().join("Library/A.cs"), "class A {}\n").unwrap();
        let folder =
            Folder::scan_root(dir.path(), config_with_excludes(&[("**/Library", false)])).unwrap();
        let page = folder.files(0..100).unwrap();
        let paths: Vec<&str> = page.items.iter().map(|entry| entry.path.as_str()).collect();
        assert!(paths.contains(&"Library/A.cs"), "paths: {paths:?}");
    }

    #[test]
    fn rules_report_unknown_and_excluded_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.cs"), "class A {}\n").unwrap();
        let folder =
            Folder::scan_root(dir.path(), WorkspaceConfig::default().without_watching()).unwrap();
        assert!(folder.rules(&WorkspacePath::new("A.cs").unwrap()).is_ok());
        assert_eq!(
            folder
                .rules(&WorkspacePath::new("Missing.cs").unwrap())
                .unwrap_err(),
            Error::NotFound
        );
        let excluded = folder
            .rules(&WorkspacePath::new("Library/A.cs").unwrap())
            .unwrap();
        assert!(excluded.excluded);
        assert!(excluded.read_only.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_directory_junction_is_not_followed() {
        use std::os::windows::process::CommandExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Real")).unwrap();
        std::fs::write(dir.path().join("Real/A.cs"), "class A {}\n").unwrap();
        let link = dir.path().join("Link");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(dir.path().join("Real"))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .status();
        let Ok(status) = status else {
            eprintln!("已跳过：无法运行 mklink（{status:?}）");
            return;
        };
        if !status.success() {
            eprintln!("已跳过：当前环境不允许创建目录 junction");
            return;
        }
        // A junction that points back at its own parent must not make the scan loop forever.
        let cycle = dir.path().join("Real/Cycle");
        let _ = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&cycle)
            .arg(dir.path())
            .creation_flags(0x0800_0000)
            .status();

        let folder =
            Folder::scan_root(dir.path(), WorkspaceConfig::default().without_watching()).unwrap();
        let paths: Vec<String> = folder
            .files(0..100)
            .unwrap()
            .items
            .iter()
            .map(|entry| entry.path.as_str().to_string())
            .collect();
        assert_eq!(paths, ["Real/A.cs"], "junctions must be skipped: {paths:?}");
    }
}
