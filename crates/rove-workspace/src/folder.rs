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
//!
//! # Locking discipline
//!
//! The snapshot write lock only guards in-memory mutation. Every filesystem call (`read_dir`,
//! `symlink_metadata`, subtree walks) happens in a planning phase *before* the lock is taken, so a
//! copied-in directory tree or a git branch switch never blocks `files()`/`children()`/`rules()`.

use crate::contracts::{Entries, FileEvent, FileEvents, FileRules, Workspace};
use crate::events::EventSource;
use crate::io::read_shared;
use crate::path::{canonical_root, identity_key, relative_str};
use crate::rules::WorkspaceConfig;
use crate::scan::{self, ScanEntry};
use crate::snapshot::Snapshot;
use crate::watch::{self, WatchChange, WatcherHandle};
use rove_base::{Cancellation, Error, Events, Result, Version, WorkspacePath};
use std::collections::BTreeSet;
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
    /// Directories the backend already watches, so rescans/reconciles never register the same
    /// directory twice (a duplicate `ReadDirectoryChangesW` handle would duplicate every event).
    registered_dirs: Mutex<BTreeSet<PathBuf>>,
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
            registered_dirs: Mutex::new(BTreeSet::new()),
        });
        // The root listing is synchronous: this is the "root visible in the first frame" part of
        // the streaming contract, and it is one readdir call.
        folder.publish_root_listing()?;
        Ok(folder)
    }

    fn scan_initial(self: &Arc<Self>) -> Result<()> {
        // Register watchers *before* walking. The walker and the watcher would otherwise race:
        // a file created after the walker passed its directory and before the watcher was
        // registered could be lost forever (and a file deleted in that window would stay).
        if self.config.watch {
            self.start_watcher();
        }
        let result = scan::full(Arc::clone(self), self.cancel.clone(), true);
        if self.config.watch {
            // The watcher only knew the root while the walk ran; register every discovered
            // directory and then verify all of them against the disk exactly once. This closes
            // the scan/watcher race for changes made after the walker passed a directory.
            self.register_dirs_blocking(self.watched_dirs());
            let discovered = self.reconcile_all();
            if !discovered.is_empty() {
                // Directories found by the reconcile itself were never walked; register them
                // synchronously and check them once more before publishing `complete`.
                self.register_dirs_blocking(discovered.clone());
                self.reconcile_dirs(discovered.into_iter().collect());
            }
        }
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
    /// when nobody is subscribed to events. The callback must not perform filesystem I/O.
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
        let read_only = self.permission_flag(&entry.path()).unwrap_or(false)
            && matches!(kind, crate::contracts::EntryKind::File);
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
        self.mark_registered(&dirs);
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

    fn register_dirs_blocking(&self, dirs: Vec<PathBuf>) {
        let fresh = self.filter_new_dirs(dirs);
        if fresh.is_empty() {
            return;
        }
        let Some(commands) = self.watcher_commands() else {
            return;
        };
        let (done, wait) = std::sync::mpsc::channel();
        if commands
            .send(watch::WatchCommand::WatchDirs {
                dirs: fresh,
                done: Some(done),
            })
            .is_ok()
        {
            let _ = wait.recv_timeout(Duration::from_secs(5));
        }
    }

    /// Directories that are not registered yet, without marking them.
    ///
    /// Used by the watcher thread, which owns the backend: it can only mark a directory as
    /// registered after `watch` succeeded.
    pub(crate) fn new_watch_dirs(&self, dirs: Vec<PathBuf>) -> Vec<PathBuf> {
        match self.registered_dirs.lock() {
            Ok(registered) => dirs
                .into_iter()
                .filter(|dir| !registered.contains(dir))
                .collect(),
            Err(poisoned) => {
                let registered = poisoned.into_inner();
                dirs.into_iter()
                    .filter(|dir| !registered.contains(dir))
                    .collect()
            }
        }
    }

    fn filter_new_dirs(&self, dirs: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut registered = match self.registered_dirs.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        dirs.into_iter()
            .filter(|dir| registered.insert(dir.clone()))
            .collect()
    }

    pub(crate) fn mark_registered(&self, dirs: &[PathBuf]) {
        match self.registered_dirs.lock() {
            Ok(mut registered) => registered.extend(dirs.iter().cloned()),
            Err(poisoned) => poisoned.into_inner().extend(dirs.iter().cloned()),
        }
    }

    /// Forgets watched directories that no longer exist, so a re-created directory is watched
    /// again.
    fn forget_registered_all(&self, directories: &[PathBuf]) {
        if directories.is_empty() {
            return;
        }
        match self.registered_dirs.lock() {
            Ok(mut registered) => {
                for directory in directories {
                    registered.retain(|dir| !dir.starts_with(directory));
                }
            }
            Err(poisoned) => {
                let mut registered = poisoned.into_inner();
                for directory in directories {
                    registered.retain(|dir| !dir.starts_with(directory));
                }
            }
        }
    }

    fn watcher_commands(&self) -> Option<std::sync::mpsc::Sender<watch::WatchCommand>> {
        match self.watcher.lock() {
            Ok(slot) => slot.as_ref().map(WatcherHandle::commands),
            Err(poisoned) => poisoned.into_inner().as_ref().map(WatcherHandle::commands),
        }
    }

    /// True when a change under this path must be ignored because the path is excluded.
    pub(crate) fn is_ignored_change(&self, path: &WorkspacePath) -> bool {
        self.config.exclude.is_excluded(path.as_str())
    }

    /// Read-only attribute of a file, or `None` when `files.readonlyFromPermissions` is off.
    fn permission_flag(&self, absolute: &Path) -> Option<bool> {
        self.config
            .readonly
            .from_permissions
            .then(|| scan::is_read_only_attribute(absolute))
    }

    fn relative(&self, absolute: &Path) -> Option<WorkspacePath> {
        relative_str(&self.root, absolute).and_then(|relative| WorkspacePath::new(&relative).ok())
    }

    fn push_watch_dirs(
        &self,
        dirs_to_watch: &mut Vec<PathBuf>,
        directory: &WorkspacePath,
        subdirectories: &[WorkspacePath],
    ) {
        if !self.config.watcher_exclude.is_excluded(directory.as_str()) {
            dirs_to_watch.push(self.absolute(directory));
        }
        for subdirectory in subdirectories {
            if !self
                .config
                .watcher_exclude
                .is_excluded(subdirectory.as_str())
            {
                dirs_to_watch.push(self.absolute(subdirectory));
            }
        }
    }

    // ---------------------------------------------------------------- watcher events

    /// Applies a debounced watcher batch and returns the directories that must be watched next.
    ///
    /// `overflow` means the pending queue exceeded [`WorkspaceConfig::event_queue_limit`] (or the
    /// backend asked for a rescan): pending events are discarded, the snapshot is rebuilt off-lock
    /// and [`FileEvent::RescanRequired`] is reported.
    ///
    /// The caller (the watcher thread) owns the `notify::Watcher`, so registration happens there;
    /// passing the directories back instead of registering here is what lets the watcher thread
    /// acknowledge registration synchronously and then re-list the new directories.
    pub(crate) fn apply_watch_events(
        self: &Arc<Self>,
        events: Vec<notify::Event>,
        overflow: bool,
    ) -> Vec<PathBuf> {
        if overflow || watch::needs_rescan(&events) {
            return self.rescan_and_report();
        }
        let changes = watch::parse(&events);
        if changes.is_empty() {
            return Vec::new();
        }
        // Directories touched by this batch. Re-listing them after applying the events makes the
        // snapshot converge even when the OS dropped part of the batch (Windows buffers are
        // 16 KiB per watched directory and notify does not surface every overflow).
        let dirty = self.dirty_dirs(&events);
        // Phase 1: all filesystem I/O (stat + subtree walks) happens here, without the lock.
        let plans = self.plan_changes(changes);
        // Phase 2: in-memory mutation under the write lock.
        let mut dirs_to_watch = Vec::new();
        let removed_directories = self.commit(|snapshot| {
            let mut emitted = Vec::new();
            let removed = self.apply_plans(snapshot, plans, &mut emitted, &mut dirs_to_watch);
            let changed = !emitted.is_empty();
            (emitted, changed, removed)
        });
        // `registered_dirs` is a different mutex; forgetting removed directories must happen
        // outside the snapshot write lock so a burst delete does not hold it.
        self.forget_registered_all(&removed_directories);
        dirs_to_watch.extend(self.reconcile_dirs(dirty));
        dirs_to_watch
    }

    fn dirty_dirs(&self, events: &[notify::Event]) -> BTreeSet<PathBuf> {
        let mut dirty = BTreeSet::new();
        for event in events {
            for path in &event.paths {
                if let Some(parent) = path.parent() {
                    if parent.starts_with(&self.root) {
                        dirty.insert(parent.to_path_buf());
                    }
                }
            }
        }
        dirty
    }

    /// Turns structural changes into a mutation plan, resolving every filesystem question
    /// (existence, kind, read-only attribute, new subtree contents) without holding the lock.
    fn plan_changes(&self, changes: Vec<WatchChange>) -> Vec<Plan> {
        let mut plans = Vec::with_capacity(changes.len());
        for change in changes {
            match change {
                WatchChange::Created(absolute) => {
                    let Some(path) = self.relative(&absolute) else {
                        continue;
                    };
                    if self.is_ignored_change(&path) {
                        continue;
                    }
                    match std::fs::symlink_metadata(&absolute) {
                        Ok(metadata) if metadata.file_type().is_symlink() => {}
                        Ok(metadata) if metadata.is_dir() => {
                            let (entries, subdirectories) = scan::subtree(self, &absolute);
                            plans.push(Plan::CreateDir {
                                path,
                                entries,
                                subdirectories,
                            });
                        }
                        Ok(_) => plans.push(Plan::CreateFile {
                            path,
                            read_only: self.permission_flag(&absolute),
                        }),
                        // The file disappeared between the event and now: nothing to add.
                        Err(_) => {}
                    }
                }
                WatchChange::Changed(absolute) => {
                    let Some(path) = self.relative(&absolute) else {
                        continue;
                    };
                    if self.is_ignored_change(&path) {
                        continue;
                    }
                    match std::fs::symlink_metadata(&absolute) {
                        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_dir() => {}
                        Ok(_) => plans.push(Plan::Changed {
                            path,
                            read_only: self.permission_flag(&absolute),
                            exists: true,
                        }),
                        // The file was deleted before we got here.
                        Err(_) => plans.push(Plan::Removed { path }),
                    }
                }
                WatchChange::Removed(absolute) => {
                    let Some(path) = self.relative(&absolute) else {
                        continue;
                    };
                    if self.is_ignored_change(&path) {
                        continue;
                    }
                    // The disk is the truth: the path may already have been re-created.
                    match std::fs::symlink_metadata(&absolute) {
                        Ok(metadata) if metadata.file_type().is_symlink() => {}
                        Ok(metadata) if metadata.is_dir() => {
                            let (entries, subdirectories) = scan::subtree(self, &absolute);
                            plans.push(Plan::CreateDir {
                                path,
                                entries,
                                subdirectories,
                            });
                        }
                        Ok(_) => plans.push(Plan::Changed {
                            path,
                            read_only: self.permission_flag(&absolute),
                            exists: true,
                        }),
                        Err(_) => plans.push(Plan::Removed { path }),
                    }
                }
                WatchChange::Renamed(from_absolute, to_absolute) => {
                    let (Some(from), Some(to)) =
                        (self.relative(&from_absolute), self.relative(&to_absolute))
                    else {
                        continue;
                    };
                    if self.is_ignored_change(&from) || self.is_ignored_change(&to) {
                        continue;
                    }
                    match std::fs::symlink_metadata(&to_absolute) {
                        Ok(metadata) if metadata.file_type().is_symlink() => {
                            plans.push(Plan::Removed { path: from });
                        }
                        Ok(metadata) if metadata.is_dir() => {
                            let (entries, subdirectories) = scan::subtree(self, &to_absolute);
                            plans.push(Plan::RenameDir {
                                from,
                                to,
                                entries,
                                subdirectories,
                            });
                        }
                        Ok(_) => plans.push(Plan::RenameFile {
                            from,
                            to,
                            read_only: self.permission_flag(&to_absolute),
                        }),
                        // The target vanished again; only the source is gone from the tree.
                        Err(_) => plans.push(Plan::Removed { path: from }),
                    }
                }
            }
        }
        plans
    }

    /// Applies a plan under the snapshot lock. No filesystem access happens here.
    ///
    /// Returns the absolute directories that no longer exist, so the caller can forget their
    /// watcher registrations outside the lock.
    fn apply_plans(
        &self,
        snapshot: &mut Snapshot,
        plans: Vec<Plan>,
        emitted: &mut Vec<FileEvent>,
        dirs_to_watch: &mut Vec<PathBuf>,
    ) -> Vec<PathBuf> {
        // Batch removals: one pass for ordinary removals and one for rename sources.
        let plain_removals: Vec<WorkspacePath> = plans
            .iter()
            .filter_map(|plan| match plan {
                Plan::Removed { path } => Some(path.clone()),
                _ => None,
            })
            .collect();
        let rename_sources: Vec<WorkspacePath> = plans
            .iter()
            .filter_map(|plan| match plan {
                Plan::RenameFile { from, .. } | Plan::RenameDir { from, .. } => Some(from.clone()),
                _ => None,
            })
            .collect();
        // Resolve the kinds *before* removing: after `remove_many` the snapshot no longer knows
        // them. Directories that disappeared must be watchable again if re-created later.
        let removed_directories: Vec<PathBuf> = plans
            .iter()
            .filter_map(|plan| match plan {
                Plan::Removed { path } => matches!(
                    snapshot.kind(path),
                    Some(crate::contracts::EntryKind::Directory)
                )
                .then(|| self.absolute(path)),
                Plan::RenameDir { from, .. } => Some(self.absolute(from)),
                _ => None,
            })
            .collect();
        let removed_plain = snapshot.remove_many(&plain_removals);
        let removed_by_rename = snapshot.remove_many(&rename_sources);
        let rename_source_keys: BTreeSet<String> = rename_sources
            .iter()
            .map(|path| identity_key(path.as_str()))
            .collect();

        // Renames first: one `Renamed` per pair, plus the old/new subtree entries.
        for plan in &plans {
            match plan {
                Plan::RenameFile {
                    from,
                    to,
                    read_only,
                } => {
                    if snapshot.add(to.clone(), crate::contracts::EntryKind::File) {
                        if let Some(read_only) = read_only {
                            snapshot.set_permission_read_only(to, *read_only);
                        }
                    }
                    emitted.push(FileEvent::Renamed {
                        from: from.clone(),
                        to: to.clone(),
                    });
                }
                Plan::RenameDir {
                    from,
                    to,
                    entries,
                    subdirectories,
                } => {
                    snapshot.add(to.clone(), crate::contracts::EntryKind::Directory);
                    emitted.push(FileEvent::Renamed {
                        from: from.clone(),
                        to: to.clone(),
                    });
                    self.add_scan_entries(snapshot, entries, emitted);
                    self.push_watch_dirs(dirs_to_watch, to, subdirectories);
                }
                _ => {}
            }
        }

        // Old subtree entries of directory renames: consumers must drop them, while the renamed
        // directory itself is already covered by `Renamed`.
        for removed in removed_by_rename {
            if rename_source_keys.contains(&identity_key(removed.as_str())) {
                continue;
            }
            emitted.push(FileEvent::Removed(removed));
        }
        for removed in removed_plain {
            emitted.push(FileEvent::Removed(removed));
        }

        // Creations and content changes.
        for plan in plans {
            match plan {
                Plan::CreateFile { path, read_only } => {
                    if snapshot.add(path.clone(), crate::contracts::EntryKind::File) {
                        emitted.push(FileEvent::Created(path.clone()));
                    }
                    if let Some(read_only) = read_only {
                        snapshot.set_permission_read_only(&path, read_only);
                    }
                }
                Plan::CreateDir {
                    path,
                    entries,
                    subdirectories,
                } => {
                    if snapshot.add(path.clone(), crate::contracts::EntryKind::Directory) {
                        emitted.push(FileEvent::Created(path.clone()));
                    }
                    self.add_scan_entries(snapshot, &entries, emitted);
                    self.push_watch_dirs(dirs_to_watch, &path, &subdirectories);
                }
                Plan::Changed {
                    path,
                    read_only,
                    exists,
                } => {
                    if snapshot.contains(&path) {
                        if let Some(read_only) = read_only {
                            snapshot.set_permission_read_only(&path, read_only);
                        }
                        emitted.push(FileEvent::Changed(path));
                    } else if exists
                        && snapshot.add(path.clone(), crate::contracts::EntryKind::File)
                    {
                        if let Some(read_only) = read_only {
                            snapshot.set_permission_read_only(&path, read_only);
                        }
                        emitted.push(FileEvent::Created(path));
                    }
                }
                Plan::Removed { .. } | Plan::RenameFile { .. } | Plan::RenameDir { .. } => {}
            }
        }
        removed_directories
    }

    fn add_scan_entries(
        &self,
        snapshot: &mut Snapshot,
        entries: &[ScanEntry],
        emitted: &mut Vec<FileEvent>,
    ) {
        for (path, kind, read_only) in entries {
            if snapshot.add(path.clone(), kind.clone()) {
                emitted.push(FileEvent::Created(path.clone()));
            }
            if *read_only {
                snapshot.set_permission_read_only(path, true);
            }
        }
    }

    // ---------------------------------------------------------------- reconciliation

    /// Re-lists every known directory and makes the snapshot match the disk.
    ///
    /// Returns the directories discovered by the reconcile itself, so the caller can register and
    /// re-check them (files may have been written between discovery and registration).
    fn reconcile_all(&self) -> Vec<PathBuf> {
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        dirs.insert(self.root.clone());
        {
            let snapshot = self.read();
            for path in snapshot.dir_paths() {
                dirs.insert(self.absolute(&path));
            }
        }
        self.reconcile_dirs(dirs)
    }

    /// Re-lists the given directories and makes the snapshot match them exactly.
    ///
    /// This is the safety net for lost events and for the scan/watcher race; it only touches the
    /// listed directories, not the whole tree. All disk reads happen before the write lock.
    /// Returns the directories that need a watcher and were not visible when the caller last
    /// registered; the caller registers them and may call this again for them.
    pub(crate) fn reconcile_dirs(&self, dirs: BTreeSet<PathBuf>) -> Vec<PathBuf> {
        if dirs.is_empty() {
            return Vec::new();
        }
        // Phase 1: read the disk listings (no lock).
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
            return Vec::new();
        }
        // Phase 2: one read pass to learn which children are known and which directories are new.
        let mut known: Vec<ChildIndex> = Vec::with_capacity(listings.len());
        let mut new_directories: Vec<WorkspacePath> = Vec::new();
        {
            let snapshot = self.read();
            for (relative, listing) in &listings {
                let children = snapshot.children_keys(&identity_key(relative));
                let keys: BTreeSet<String> = children.iter().map(|(key, _)| key.clone()).collect();
                for (path, kind, _) in listing {
                    if matches!(kind, crate::contracts::EntryKind::Directory)
                        && !keys.contains(&identity_key(path.as_str()))
                    {
                        new_directories.push(path.clone());
                    }
                }
                known.push((keys, children));
            }
        }
        // Phase 3: pre-scan subtrees of directories that appeared since the last reading.
        let mut subtrees: Vec<(WorkspacePath, Vec<ScanEntry>, Vec<WorkspacePath>)> = Vec::new();
        for directory in new_directories {
            let absolute = self.absolute(&directory);
            let (entries, subdirectories) = scan::subtree(self, &absolute);
            subtrees.push((directory, entries, subdirectories));
        }
        // Phase 4: diff and apply under the lock, batching every removal into one pass.
        let mut dirs_to_watch = Vec::new();
        self.commit(|snapshot| {
            let mut emitted = Vec::new();
            let removals: Vec<WorkspacePath> = listings
                .iter()
                .zip(&known)
                .flat_map(|((_, listing), (keys, children))| {
                    let expected: BTreeSet<String> = listing
                        .iter()
                        .map(|(path, _, _)| identity_key(path.as_str()))
                        .collect();
                    children
                        .iter()
                        .filter(|(key, _)| keys.contains(key) && !expected.contains(key))
                        .map(|(_, display)| display.clone())
                        .collect::<Vec<_>>()
                })
                .collect();
            for removed in snapshot.remove_many(&removals) {
                emitted.push(FileEvent::Removed(removed));
            }
            for (relative, listing) in listings {
                for (path, kind, read_only) in listing {
                    if snapshot.add(path.clone(), kind.clone()) {
                        emitted.push(FileEvent::Created(path.clone()));
                        if matches!(kind, crate::contracts::EntryKind::Directory) {
                            self.push_watch_dirs(&mut dirs_to_watch, &path, &[]);
                        }
                    }
                    if read_only {
                        snapshot.set_permission_read_only(&path, true);
                    }
                }
                let _ = relative;
            }
            for (directory, entries, subdirectories) in subtrees {
                self.add_scan_entries(snapshot, &entries, &mut emitted);
                self.push_watch_dirs(&mut dirs_to_watch, &directory, &subdirectories);
            }
            let changed = !emitted.is_empty();
            (emitted, changed, ())
        });
        dirs_to_watch
    }

    // ---------------------------------------------------------------- rescan

    /// Full re-scan after an event overflow; reports [`FileEvent::RescanRequired`] once the
    /// snapshot matches the disk again.
    ///
    /// The replacement snapshot is built off-lock and swapped in atomically, so observers never
    /// see an empty workspace while the rescan runs, and a failed rescan leaves the old snapshot
    /// untouched instead of claiming a complete (but empty) tree. A failed rescan still emits
    /// `RescanRequired`: the events that triggered it were dropped, so consumers must treat their
    /// view as stale. Returns the directories the caller has to watch.
    fn rescan_and_report(self: &Arc<Self>) -> Vec<PathBuf> {
        // One retry: a transient sharing/permission failure during the walk is common (an editor
        // or antivirus holding a file) and the second pass usually succeeds.
        let replacement =
            scan::collect(self, &self.cancel).or_else(|_| scan::collect(self, &self.cancel));
        let Ok(replacement) = replacement else {
            let revision = {
                let mut snapshot = self.write();
                snapshot.bump()
            };
            self.events.emit(FileEvents {
                revision,
                events: Arc::from([FileEvent::RescanRequired]),
            });
            return Vec::new();
        };
        let revision = {
            let mut snapshot = self.write();
            let revision = Version(snapshot.revision().0.wrapping_add(1));
            let mut replacement = replacement;
            replacement.set_revision(revision);
            *snapshot = replacement;
            revision
        };
        self.events.emit(FileEvents {
            revision,
            events: Arc::from([FileEvent::RescanRequired]),
        });
        // Watches for directories that no longer exist are dead (Windows drops them); forget them
        // so a re-created directory is watched again, then register everything that is missing.
        let dirs = self.watched_dirs();
        let live: BTreeSet<PathBuf> = dirs.iter().cloned().collect();
        match self.registered_dirs.lock() {
            Ok(mut registered) => registered.retain(|dir| live.contains(dir)),
            Err(poisoned) => poisoned.into_inner().retain(|dir| live.contains(dir)),
        }
        dirs
    }
}

/// Known direct children of one directory: their identity keys and display paths.
type ChildIndex = (BTreeSet<String>, Vec<(String, WorkspacePath)>);

/// One planned mutation, resolved from the disk before the snapshot lock is taken.
enum Plan {
    CreateFile {
        path: WorkspacePath,
        read_only: Option<bool>,
    },
    CreateDir {
        path: WorkspacePath,
        entries: Vec<ScanEntry>,
        subdirectories: Vec<WorkspacePath>,
    },
    Changed {
        path: WorkspacePath,
        read_only: Option<bool>,
        exists: bool,
    },
    Removed {
        path: WorkspacePath,
    },
    RenameFile {
        from: WorkspacePath,
        to: WorkspacePath,
        read_only: Option<bool>,
    },
    RenameDir {
        from: WorkspacePath,
        to: WorkspacePath,
        entries: Vec<ScanEntry>,
        subdirectories: Vec<WorkspacePath>,
    },
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
        let before = folder.revision();

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
        assert!(
            folder.revision() > before,
            "the rescan must advance the revision"
        );
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
    fn a_removal_for_a_recreated_file_does_not_lose_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.cs"), "class A {}\n").unwrap();
        let folder = Folder::scan_root(dir.path(), config().without_watching()).unwrap();
        // Delete and immediately re-create: the queued Removed event is stale by the time it is
        // applied, so the disk must win.
        std::fs::write(dir.path().join("A.cs"), "class A { int X; }\n").unwrap();
        let event = notify::Event::new(notify::EventKind::Remove(notify::event::RemoveKind::File))
            .add_path(folder.root().join("A.cs"));
        folder.apply_watch_events(vec![event], false);
        let page = folder.files(0..10).unwrap();
        assert_eq!(
            page.items.len(),
            1,
            "re-created file must stay in the snapshot"
        );
    }

    #[test]
    fn a_new_directory_is_returned_for_registration_and_reconciled() {
        let dir = tempfile::tempdir().unwrap();
        let folder = Folder::scan_root(dir.path(), config().without_watching()).unwrap();
        std::fs::create_dir_all(dir.path().join("Assets/Copied")).unwrap();
        // First visible state: one file, as if the subtree scan ran while a copy was in flight.
        std::fs::write(dir.path().join("Assets/Copied/First.cs"), "class F {}\n").unwrap();
        let event =
            notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::Folder))
                .add_path(folder.root().join("Assets/Copied"));
        let to_watch = folder.apply_watch_events(vec![event], false);
        assert!(
            to_watch.iter().any(|path| path.ends_with("Copied")),
            "new directory must be reported for registration: {to_watch:?}"
        );

        // Files written between the first scan and the watch registration produce no events; the
        // reconcile that follows registration (what the watcher thread does) must pick them up.
        std::fs::write(dir.path().join("Assets/Copied/Late.cs"), "class L {}\n").unwrap();
        folder.reconcile_dirs(to_watch.into_iter().collect());

        let page = folder.files(0..100).unwrap();
        let paths: Vec<&str> = page.items.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(paths, ["Assets/Copied/First.cs", "Assets/Copied/Late.cs"]);
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
