//! Streaming workspace: scan, snapshot queries, rules and watched updates (#15).
//!
//! Construction comes in two flavours:
//!
//! - [`Folder::start`] returns immediately after listing the root directory; the parallel scan
//!   keeps publishing batches in the background. `files`/`children` expose whatever is known so
//!   far (`Entries::complete == false`), which is what lets the explorer paint the root in the
//!   first frame and quick-open answer before the scan finishes.
//! - [`Folder::scan_root`] performs the same scan on the calling thread and returns only once
//!   the initial snapshot is complete (used by the skeleton workbench and by benchmarks).
//!
//! Both paths share one scanner, so streaming behaviour is not a separate code path.

use crate::contracts::{Entries, FileEvent, FileEvents, FileRules, Workspace};
use crate::events::EventSource;
use crate::io::read_shared;
use crate::path::{canonical_root, relative_str};
use crate::rules::WorkspaceConfig;
use crate::scan::{self, ScanEntry};
use crate::snapshot::Snapshot;
use crate::watch::{self, WatchChange, WatcherHandle};
use rove_base::{Cancellation, Error, Events, Result, Version, WorkspacePath};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// A live, immutable-snapshot workspace over one root folder.
pub struct Folder {
    root: PathBuf,
    config: WorkspaceConfig,
    snapshot: RwLock<Snapshot>,
    events: EventSource,
    skipped: AtomicU64,
    cancel: Cancellation,
    watcher: Mutex<Option<WatcherHandle>>,
}

impl Folder {
    /// Starts the streaming scan. The returned workspace is usable immediately: the root
    /// directory has already been listed, the rest arrives through later revisions.
    pub fn start(root: impl AsRef<Path>, config: WorkspaceConfig) -> Result<Arc<Self>> {
        Self::start_with_cancel(root, config, Cancellation::default())
    }

    /// Streaming variant with an explicit cancellation token.
    pub fn start_with_cancel(
        root: impl AsRef<Path>,
        config: WorkspaceConfig,
        cancel: Cancellation,
    ) -> Result<Arc<Self>> {
        let folder = Self::create(root, config, cancel)?;
        let scan = Arc::clone(&folder);
        std::thread::Builder::new()
            .name("rove-workspace-scan".into())
            .spawn(move || {
                let _ = scan.scan_initial();
            })?;
        Ok(folder)
    }

    /// Blocking initial scan: returns only after `files(..).complete` is true.
    pub fn scan_root(root: impl AsRef<Path>, config: WorkspaceConfig) -> Result<Arc<Self>> {
        Self::scan_root_with_cancel(root, config, Cancellation::default())
    }

    /// Blocking initial scan with an explicit cancellation token.
    pub fn scan_root_with_cancel(
        root: impl AsRef<Path>,
        config: WorkspaceConfig,
        cancel: Cancellation,
    ) -> Result<Arc<Self>> {
        let folder = Self::create(root, config, cancel)?;
        folder.scan_initial()?;
        Ok(folder)
    }

    fn create(
        root: impl AsRef<Path>,
        config: WorkspaceConfig,
        cancel: Cancellation,
    ) -> Result<Arc<Self>> {
        let root = canonical_root(root.as_ref())?;
        let folder = Arc::new(Self {
            root,
            config,
            snapshot: RwLock::new(Snapshot::new()),
            events: EventSource::default(),
            skipped: AtomicU64::new(0),
            cancel,
            watcher: Mutex::new(None),
        });
        // The root listing is synchronous: this is the "root visible in the first frame" part of
        // the streaming contract, and it is one readdir call.
        folder.publish_root_listing()?;
        Ok(folder)
    }

    fn scan_initial(self: &Arc<Self>) -> Result<()> {
        let result = scan::full(Arc::clone(self), self.cancel.clone(), true);
        // Even a cancelled or partially failed scan must stop reporting `complete == false`
        // forever; what was found is a valid (if partial) snapshot.
        let revision = {
            let mut snapshot = self.write();
            snapshot.finish()
        };
        self.events.emit(FileEvents {
            revision,
            events: Arc::from([]),
        });
        if self.config.watch {
            self.start_watcher();
        }
        result
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> &WorkspaceConfig {
        &self.config
    }

    /// Number of entries skipped because they were unreadable, non-UTF-8 or special files.
    /// A single bad name or unreadable directory never fails the scan; it is counted here.
    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    /// True once the file watcher is registered. The flag is only set after the backend accepted
    /// the initial watch calls, so a write issued after this returns is observed.
    pub fn watching(&self) -> bool {
        match self.watcher.lock() {
            Ok(slot) => slot.is_some(),
            Err(poisoned) => poisoned.into_inner().is_some(),
        }
    }

    pub(crate) fn note_skipped(&self, count: u64) {
        self.skipped.fetch_add(count, Ordering::Relaxed);
    }

    pub(crate) fn event_queue_limit(&self) -> usize {
        self.config.event_queue_limit
    }

    /// Absolute path of a workspace path. The root keeps the Windows extended-length prefix.
    pub fn absolute(&self, path: &WorkspacePath) -> PathBuf {
        self.root.join(path.as_str())
    }

    /// Reads a workspace file with shared-lock tolerance (see [`crate::io::read_shared`]).
    pub fn read_file(&self, path: &WorkspacePath) -> Result<Vec<u8>> {
        read_shared(&self.absolute(path))
    }

    /// Cancels the scan; queries keep working on the partial snapshot.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Snapshot> {
        match self.snapshot.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Snapshot> {
        match self.snapshot.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Applies one mutation under the snapshot lock and emits a single revision batch.
    ///
    /// The callback returns the events it produced plus whether the snapshot actually changed; the
    /// revision advances when either is true, so consumers polling `revision()` see progress even
    /// when nobody is subscribed to events.
    fn commit<R>(&self, mutate: impl FnOnce(&mut Snapshot) -> (Vec<FileEvent>, bool, R)) -> R {
        let (events, revision, result) = {
            let mut snapshot = self.write();
            let (events, changed, result) = mutate(&mut snapshot);
            let revision = if events.is_empty() && !changed {
                snapshot.revision()
            } else {
                snapshot.bump()
            };
            (events, revision, result)
        };
        if !events.is_empty() {
            self.events.emit(FileEvents {
                revision,
                events: Arc::from(events),
            });
        }
        result
    }

    fn publish_root_listing(&self) -> Result<()> {
        let entries = std::fs::read_dir(&self.root)?;
        let mut batch = Vec::new();
        let mut skipped = 0u64;
        for entry in entries {
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            match self.classify_dir_entry(entry) {
                Some(scan_entry) => batch.push(scan_entry),
                None => skipped += 1,
            }
        }
        self.note_skipped(skipped);
        self.publish_entries(batch, true);
        Ok(())
    }

    /// Classifies one directory entry into a scan entry, applying the exclusion rules, the
    /// reparse-point rule and the read-only attribute tracking.
    fn classify_dir_entry(&self, entry: std::fs::DirEntry) -> Option<ScanEntry> {
        let file_type = entry.file_type().ok()?;
        let relative = relative_str(&self.root, &entry.path())?;
        let path = WorkspacePath::new(&relative).ok()?;
        if self.config.exclude.matches_self(path.as_str()) == Some(true) {
            return None;
        }
        if scan::is_reparse_path(&entry.path(), &file_type) {
            return None;
        }
        let kind = if file_type.is_dir() {
            crate::contracts::EntryKind::Directory
        } else if file_type.is_file() {
            crate::contracts::EntryKind::File
        } else {
            return None;
        };
        let read_only = self.config.readonly.from_permissions
            && matches!(kind, crate::contracts::EntryKind::File)
            && scan::is_read_only_attribute(&entry.path());
        Some((path, kind, read_only))
    }

    /// Publishes a scanner batch. Called only by the scan thread.
    pub(crate) fn publish_entries(&self, entries: Vec<ScanEntry>, emit_events: bool) -> usize {
        let emit_events = emit_events && self.events.has_listeners();
        self.commit(|snapshot| {
            let mut events = Vec::new();
            let mut added = 0usize;
            for (path, kind, read_only) in entries {
                if read_only {
                    snapshot.set_permission_read_only(&path, true);
                }
                if snapshot.add(path.clone(), kind) {
                    added += 1;
                    if emit_events {
                        events.push(FileEvent::Created(path));
                    }
                }
            }
            (events, added > 0, added)
        })
    }

    fn start_watcher(self: &Arc<Self>) {
        let dirs = self.watched_dirs();
        let handle = watch::spawn(self, dirs);
        match self.watcher.lock() {
            Ok(mut slot) => *slot = handle,
            Err(poisoned) => *poisoned.into_inner() = handle,
        }
    }

    fn watched_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.root.clone()];
        {
            let snapshot = self.read();
            for path in snapshot.dir_paths() {
                if self.config.watcher_exclude.is_excluded(path.as_str()) {
                    continue;
                }
                dirs.push(self.absolute(&path));
            }
        }
        dirs
    }

    fn register_dirs(&self, dirs: Vec<PathBuf>) {
        if dirs.is_empty() {
            return;
        }
        match self.watcher.lock() {
            Ok(slot) => {
                if let Some(handle) = slot.as_ref() {
                    handle.add_dirs(dirs);
                }
            }
            Err(poisoned) => {
                if let Some(handle) = poisoned.into_inner().as_ref() {
                    handle.add_dirs(dirs);
                }
            }
        }
    }

    /// True when a change under this path must be ignored because the path is excluded.
    pub(crate) fn is_ignored_change(&self, path: &WorkspacePath) -> bool {
        self.config.exclude.is_excluded(path.as_str())
    }

    /// Applies a debounced watcher batch.
    ///
    /// `overflow` means the pending queue exceeded [`WorkspaceConfig::event_queue_limit`] (or the
    /// backend asked for a rescan): pending events are discarded, the snapshot is rebuilt from
    /// disk and [`FileEvent::RescanRequired`] is reported.
    pub(crate) fn apply_watch_events(self: &Arc<Self>, events: Vec<notify::Event>, overflow: bool) {
        if overflow || watch::needs_rescan(&events) {
            self.rescan_and_report();
            return;
        }
        let changes = watch::parse(&events);
        if changes.is_empty() {
            return;
        }
        // Directories touched by this batch. Re-listing them after applying the events makes the
        // snapshot converge even when the OS dropped part of the batch (Windows buffers are
        // 16 KiB per watched directory and notify does not surface every overflow).
        let mut dirty: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
        for event in &events {
            for path in &event.paths {
                if let Some(parent) = path.parent() {
                    if parent.starts_with(&self.root) {
                        dirty.insert(parent.to_path_buf());
                    }
                }
            }
        }
        let mut dirs_to_watch = Vec::new();
        self.commit(|snapshot| {
            let mut emitted = Vec::new();
            for change in changes {
                self.apply_change(snapshot, change, &mut emitted, &mut dirs_to_watch);
            }
            let changed = !emitted.is_empty();
            (emitted, changed, ())
        });
        self.reconcile_dirs(dirty);
        self.register_dirs(dirs_to_watch);
    }

    /// Re-lists the given directories and makes the snapshot match them exactly.
    ///
    /// This is the safety net for lost events; it only touches the listed directory, not the
    /// whole tree.
    fn reconcile_dirs(&self, dirs: std::collections::BTreeSet<PathBuf>) {
        if dirs.is_empty() {
            return;
        }
        // Phase 1: read the disk outside the snapshot lock.
        let mut listings: Vec<(String, Vec<ScanEntry>)> = Vec::new();
        let mut skipped = 0u64;
        for dir in dirs {
            let relative = if dir == self.root {
                String::new()
            } else {
                match relative_str(&self.root, &dir) {
                    Some(relative) => relative,
                    None => continue,
                }
            };
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut found = Vec::new();
            for entry in entries {
                let Ok(entry) = entry else {
                    skipped += 1;
                    continue;
                };
                if let Some(scan_entry) = self.classify_dir_entry(entry) {
                    found.push(scan_entry);
                }
            }
            listings.push((relative, found));
        }
        self.note_skipped(skipped);
        if listings.is_empty() {
            return;
        }
        // Phase 2: diff each listing against the snapshot.
        let mut dirs_to_watch = Vec::new();
        self.commit(|snapshot| {
            let mut emitted = Vec::new();
            for (relative, found) in listings {
                let parent_key = crate::path::identity_key(&relative);
                let expected: std::collections::BTreeSet<String> = found
                    .iter()
                    .map(|(path, _, _)| crate::path::identity_key(path.as_str()))
                    .collect();
                for (key, display) in snapshot.children_keys(&parent_key) {
                    if !expected.contains(&key) {
                        for removed in snapshot.remove(&display) {
                            emitted.push(FileEvent::Removed(removed));
                        }
                    }
                }
                for (path, kind, read_only) in found {
                    if snapshot.contains(&path) {
                        continue;
                    }
                    snapshot.add(path.clone(), kind.clone());
                    emitted.push(FileEvent::Created(path.clone()));
                    if read_only {
                        snapshot.set_permission_read_only(&path, true);
                    }
                    if matches!(kind, crate::contracts::EntryKind::Directory) {
                        let absolute = self.absolute(&path);
                        self.add_subtree(snapshot, &absolute, &mut emitted, &mut dirs_to_watch);
                    }
                }
            }
            let changed = !emitted.is_empty();
            (emitted, changed, ())
        });
        self.register_dirs(dirs_to_watch);
    }

    fn apply_change(
        &self,
        snapshot: &mut Snapshot,
        change: WatchChange,
        emitted: &mut Vec<FileEvent>,
        dirs_to_watch: &mut Vec<PathBuf>,
    ) {
        match change {
            WatchChange::Created(absolute) => {
                let Some(path) = self.relative(&absolute) else {
                    return;
                };
                if self.is_ignored_change(&path) {
                    return;
                }
                match std::fs::symlink_metadata(&absolute) {
                    Ok(metadata) if metadata.is_dir() => {
                        if snapshot.add(path.clone(), crate::contracts::EntryKind::Directory) {
                            emitted.push(FileEvent::Created(path.clone()));
                        }
                        self.add_subtree(snapshot, &absolute, emitted, dirs_to_watch);
                    }
                    Ok(_) => {
                        let read_only = self.config.readonly.from_permissions
                            && scan::is_read_only_attribute(&absolute);
                        if snapshot.add(path.clone(), crate::contracts::EntryKind::File) {
                            emitted.push(FileEvent::Created(path.clone()));
                        }
                        if read_only {
                            snapshot.set_permission_read_only(&path, true);
                        }
                    }
                    // The file disappeared between the event and now: nothing to add.
                    Err(_) => {}
                }
            }
            WatchChange::Changed(absolute) => {
                let Some(path) = self.relative(&absolute) else {
                    return;
                };
                if self.is_ignored_change(&path) {
                    return;
                }
                if snapshot.contains(&path) {
                    emitted.push(FileEvent::Changed(path));
                } else if absolute.exists() {
                    // A missed creation: report it as such instead of losing the file.
                    self.apply_change(
                        snapshot,
                        WatchChange::Created(absolute),
                        emitted,
                        dirs_to_watch,
                    );
                }
            }
            WatchChange::Removed(absolute) => {
                let Some(path) = self.relative(&absolute) else {
                    return;
                };
                if self.is_ignored_change(&path) {
                    return;
                }
                for removed in snapshot.remove(&path) {
                    emitted.push(FileEvent::Removed(removed));
                }
            }
            WatchChange::Renamed(from, to) => {
                let (Some(from_path), Some(to_path)) = (self.relative(&from), self.relative(&to))
                else {
                    return;
                };
                if self.is_ignored_change(&from_path) || self.is_ignored_change(&to_path) {
                    return;
                }
                snapshot.remove(&from_path);
                match std::fs::symlink_metadata(&to) {
                    Ok(metadata) if metadata.is_dir() => {
                        snapshot.add(to_path.clone(), crate::contracts::EntryKind::Directory);
                        self.add_subtree(snapshot, &to, emitted, dirs_to_watch);
                    }
                    Ok(_) => {
                        snapshot.add(to_path.clone(), crate::contracts::EntryKind::File);
                    }
                    Err(_) => {}
                }
                emitted.push(FileEvent::Renamed {
                    from: from_path,
                    to: to_path,
                });
            }
        }
    }

    /// Adds every visible entry below `absolute_dir` to the snapshot; new directories are
    /// returned so the watcher can start watching them.
    fn add_subtree(
        &self,
        snapshot: &mut Snapshot,
        absolute_dir: &Path,
        emitted: &mut Vec<FileEvent>,
        dirs_to_watch: &mut Vec<PathBuf>,
    ) {
        let (entries, subdirectories) = scan::subtree(self, absolute_dir);
        for (path, kind, read_only) in entries {
            if read_only {
                snapshot.set_permission_read_only(&path, true);
            }
            if snapshot.add(path.clone(), kind) {
                emitted.push(FileEvent::Created(path));
            }
        }
        dirs_to_watch.push(absolute_dir.to_path_buf());
        for path in subdirectories {
            if !self.config.watcher_exclude.is_excluded(path.as_str()) {
                dirs_to_watch.push(self.absolute(&path));
            }
        }
    }

    fn relative(&self, absolute: &Path) -> Option<WorkspacePath> {
        relative_str(&self.root, absolute).and_then(|relative| WorkspacePath::new(&relative).ok())
    }

    /// Full re-scan after an event overflow; reports [`FileEvent::RescanRequired`] once the
    /// snapshot matches the disk again.
    fn rescan_and_report(self: &Arc<Self>) {
        {
            let mut snapshot = self.write();
            snapshot.clear();
        }
        // Suppress per-entry events: the single RescanRequired below is the contract, and a
        // flood of Created events would defeat the point of recovering from a flood.
        let scanned = scan::full(Arc::clone(self), self.cancel.clone(), false);
        let revision = {
            let mut snapshot = self.write();
            snapshot.finish()
        };
        if scanned.is_ok() {
            self.events.emit(FileEvents {
                revision,
                events: Arc::from([FileEvent::RescanRequired]),
            });
            let dirs = self.watched_dirs();
            self.register_dirs(dirs);
        }
    }
}

impl Workspace for Folder {
    fn revision(&self) -> Version {
        self.read().revision()
    }

    fn files(&self, range: std::ops::Range<usize>) -> Result<Entries> {
        self.read().files_page(range)
    }

    fn children(
        &self,
        parent: Option<&WorkspacePath>,
        range: std::ops::Range<usize>,
    ) -> Result<Entries> {
        self.read().children_page(parent, range)
    }

    fn rules(&self, path: &WorkspacePath) -> Result<FileRules> {
        let snapshot = self.read();
        let excluded = self.config.exclude.is_excluded(path.as_str());
        let known = snapshot.contains(path);
        if !known && !excluded {
            return Err(Error::NotFound);
        }
        let read_only = if known && !excluded {
            let permission = snapshot.permission_read_only(path);
            self.config.readonly.evaluate(path.as_str(), permission)
        } else {
            Vec::new()
        };
        Ok(FileRules {
            excluded,
            read_only: Arc::from(read_only),
        })
    }

    fn changes(&self) -> &dyn Events<FileEvents> {
        &self.events
    }
}

/// Convenience constructor: streaming workspace with the registered defaults.
pub fn open(root: impl AsRef<Path>) -> Result<Arc<Folder>> {
    Folder::start(root, WorkspaceConfig::default())
}

/// Waits (polling) until the initial scan is complete, or the timeout elapses.
///
/// Only for tests and callers that cannot await an event (the workspace itself reports progress
/// through revisions/events; this helper exists so integration tests do not sleep blindly).
pub fn wait_until_complete(folder: &Folder, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if folder
            .files(0..0)
            .map(|page| page.complete)
            .unwrap_or(false)
        {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::EntryKind;
    use crate::glob::RuleSet;
    use crate::path::identity_key;
    use std::sync::Mutex;

    fn config() -> WorkspaceConfig {
        WorkspaceConfig::from_settings(&crate::rules::default_settings()).unwrap()
    }

    fn collect(folder: &Arc<Folder>) -> (Arc<Mutex<Vec<FileEvent>>>, rove_base::Subscription) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        let subscription = folder
            .changes()
            .subscribe(Arc::new(move |batch: FileEvents| {
                sink.lock().unwrap().extend(batch.events.iter().cloned());
            }));
        (log, subscription)
    }

    #[test]
    fn overflow_rescans_and_reports_the_rescan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.cs"), "class A {}\n").unwrap();
        let folder = Folder::scan_root(dir.path(), config().without_watching()).unwrap();
        let (log, _subscription) = collect(&folder);

        // Simulate events that were lost: the disk changed while the watcher was not listening.
        std::fs::write(dir.path().join("B.cs"), "class B {}\n").unwrap();
        std::fs::remove_file(dir.path().join("A.cs")).unwrap();

        folder.apply_watch_events(Vec::new(), true);

        let events = log.lock().unwrap().clone();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, FileEvent::RescanRequired)),
            "events: {events:?}"
        );
        let page = folder.files(0..100).unwrap();
        let names: Vec<&str> = page.items.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(
            names,
            ["B.cs"],
            "snapshot must match the disk after a rescan"
        );
        assert!(page.complete);
    }

    #[test]
    fn a_rescan_flag_from_the_backend_also_triggers_a_rescan() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.cs"), "class A {}\n").unwrap();
        let folder = Folder::scan_root(dir.path(), config().without_watching()).unwrap();
        let (log, _subscription) = collect(&folder);
        std::fs::write(dir.path().join("B.cs"), "class B {}\n").unwrap();

        let event = notify::Event::new(notify::EventKind::Other);
        let event = event.set_flag(notify::event::Flag::Rescan);
        folder.apply_watch_events(vec![event], false);

        assert!(log
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, FileEvent::RescanRequired)));
        assert_eq!(folder.files(0..100).unwrap().items.len(), 2);
    }

    #[test]
    fn watch_registration_skips_watcher_excluded_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Ignored/Sub")).unwrap();
        std::fs::create_dir_all(dir.path().join("Watched/Sub")).unwrap();
        let mut config = config();
        config.watcher_exclude = RuleSet::from_value(&rove_base::Value::Object(Arc::new(
            [("**/Ignored".to_string(), rove_base::Value::Bool(true))].into(),
        )))
        .unwrap();
        let folder = Folder::scan_root(dir.path(), config).unwrap();
        let watched: Vec<String> = folder
            .watched_dirs()
            .iter()
            .filter_map(|path| path.strip_prefix(folder.root()).ok())
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(watched.iter().any(|path| path == "Watched"), "{watched:?}");
        assert!(
            watched.iter().any(|path| path == "Watched/Sub"),
            "{watched:?}"
        );
        assert!(
            !watched.iter().any(|path| path.starts_with("Ignored")),
            "{watched:?}"
        );
    }

    #[test]
    fn children_use_disk_case_but_lookups_are_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Assets")).unwrap();
        std::fs::write(dir.path().join("Assets/Player.cs"), "class Player {}\n").unwrap();
        let folder = Folder::scan_root(dir.path(), config().without_watching()).unwrap();

        let page = folder.children(None, 0..16).unwrap();
        assert_eq!(page.items[0].path.as_str(), "Assets");
        assert!(matches!(page.items[0].kind, EntryKind::Directory));

        #[cfg(windows)]
        {
            let lower = WorkspacePath::new("assets/player.cs").unwrap();
            let rules = folder.rules(&lower).unwrap();
            assert!(!rules.excluded);
            assert_eq!(
                folder.files(0..16).unwrap().items[0].path.as_str(),
                "Assets/Player.cs"
            );
            assert_eq!(
                identity_key(lower.as_str()),
                identity_key("Assets/Player.cs")
            );
        }
    }

    #[test]
    fn long_paths_are_scanned_and_readable() {
        let dir = tempfile::tempdir().unwrap();
        // Windows needs the extended-length prefix to *create* paths past MAX_PATH; the scanner
        // canonicalizes the root itself, so this only affects the test fixture setup.
        #[cfg(windows)]
        let mut absolute = std::path::PathBuf::from(format!(r"\\?\{}", dir.path().display()));
        #[cfg(not(windows))]
        let mut absolute = dir.path().to_path_buf();
        // 16 segments of ~20 characters each: well past MAX_PATH once the temp root is included.
        for index in 0..16 {
            absolute.push(format!("segment{index:02}aaaaaaaaaa"));
        }
        std::fs::create_dir_all(&absolute).expect("create long directory chain");
        let file = absolute.join("Deep.cs");
        std::fs::write(&file, b"class Deep {}\n").expect("write long path file");

        let folder = Folder::scan_root(dir.path(), config().without_watching()).unwrap();
        let page = folder.files(0..100).unwrap();
        let entry = page
            .items
            .iter()
            .find(|entry| entry.path.as_str().ends_with("Deep.cs"))
            .expect("long-path file is in the snapshot");
        assert!(entry.path.as_str().len() > 260, "{}", entry.path.as_str());
        assert_eq!(folder.read_file(&entry.path).unwrap(), b"class Deep {}\n");
    }
}
