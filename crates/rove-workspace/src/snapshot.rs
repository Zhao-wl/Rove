//! Immutable-cached workspace snapshot: paged queries, identity lookups, incremental updates.
//!
//! One snapshot is shared by every query; the scan publishes batches into it and the watcher
//! applies event batches. Revision increments once per published batch, so consumers can discard
//! stale work. `files`/`children` never touch the disk.

use crate::contracts::{Entries, Entry, EntryKind};
use crate::path::{identity_cmp, identity_key, parent_key};
use rove_base::{Error, Result, Version, WorkspacePath};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) struct Snapshot {
    revision: Version,
    complete: bool,
    /// Flat file list. Arrival order while `complete == false`, sorted by identity afterwards.
    files: Vec<Entry>,
    /// Children by parent identity key; the root key is `""`.
    dirs: BTreeMap<String, Vec<Entry>>,
    /// Identity key -> (display path, kind) for every known file and directory. Keeping the
    /// display path here is what lets events report the on-disk case instead of the folded key.
    existing: BTreeMap<String, (WorkspacePath, EntryKind)>,
    /// Identity keys of files whose on-disk attribute is read-only (only tracked when
    /// `files.readonlyFromPermissions` is enabled).
    permission_read_only: BTreeSet<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot {
    pub(crate) fn new() -> Self {
        let mut dirs = BTreeMap::new();
        dirs.insert(String::new(), Vec::new());
        Self {
            revision: Version(0),
            complete: false,
            files: Vec::new(),
            dirs,
            existing: BTreeMap::new(),
            permission_read_only: BTreeSet::new(),
        }
    }

    pub(crate) fn revision(&self) -> Version {
        self.revision
    }

    pub(crate) fn bump(&mut self) -> Version {
        self.revision = Version(self.revision.0.wrapping_add(1));
        self.revision
    }

    /// Adds one entry, creating missing ancestor directories so `children` works regardless of
    /// the walker's arrival order. Returns `false` when the entry is already known.
    pub(crate) fn add(&mut self, path: WorkspacePath, kind: EntryKind) -> bool {
        let key = identity_key(path.as_str());
        if self.existing.contains_key(&key) {
            return false;
        }
        self.ensure_parents(path.as_str());
        let parent = identity_key(parent_key(path.as_str()).to_string().as_str());
        self.dirs.entry(parent).or_default().push(Entry {
            path: path.clone(),
            kind: kind.clone(),
        });
        if matches!(kind, EntryKind::File) {
            self.files.push(Entry {
                path: path.clone(),
                kind: kind.clone(),
            });
        }
        self.existing.insert(key, (path, kind));
        true
    }

    /// Synthesizes missing ancestor directories using the **display** path (the child's spelling),
    /// never the folded identity key, so a directory reached only through its children still
    /// keeps the on-disk case.
    fn ensure_parents(&mut self, display: &str) {
        let Some((parent_display, _)) = display.rsplit_once('/') else {
            return;
        };
        let mut parent_display = parent_display.to_string();
        loop {
            let key = identity_key(&parent_display);
            if self.existing.contains_key(&key) {
                break;
            }
            let Ok(path) = WorkspacePath::new(&parent_display) else {
                break;
            };
            let grandparent = path
                .as_str()
                .rsplit_once('/')
                .map_or_else(String::new, |(parent, _)| parent.to_string());
            self.dirs
                .entry(identity_key(&grandparent))
                .or_default()
                .push(Entry {
                    path: path.clone(),
                    kind: EntryKind::Directory,
                });
            self.existing.insert(key, (path, EntryKind::Directory));
            if grandparent.is_empty() {
                break;
            }
            parent_display = grandparent;
        }
    }

    /// Marks a file as read-only because of its on-disk attribute.
    pub(crate) fn set_permission_read_only(&mut self, path: &WorkspacePath, read_only: bool) {
        let key = identity_key(path.as_str());
        if read_only {
            self.permission_read_only.insert(key);
        } else {
            self.permission_read_only.remove(&key);
        }
    }

    pub(crate) fn permission_read_only(&self, path: &WorkspacePath) -> bool {
        self.permission_read_only
            .contains(&identity_key(path.as_str()))
    }

    /// Removes one path and, for a directory, everything below it.
    ///
    /// Returns the display paths that disappeared, so the watcher can emit one `Removed` per
    /// stale entry. A path that was never known returns an empty list.
    pub(crate) fn remove(&mut self, path: &WorkspacePath) -> Vec<WorkspacePath> {
        let key = identity_key(path.as_str());
        let prefix = format!("{key}/");
        let stale: Vec<(String, WorkspacePath)> = self
            .existing
            .iter()
            .filter(|(candidate, _)| **candidate == key || candidate.starts_with(&prefix))
            .map(|(candidate, (display, _))| (candidate.clone(), display.clone()))
            .collect();
        let stale_keys: BTreeSet<String> = stale.iter().map(|(key, _)| key.clone()).collect();
        for stale_key in &stale_keys {
            self.existing.remove(stale_key);
            self.permission_read_only.remove(stale_key);
        }
        // Children vectors: drop stale entries and stale directory buckets.
        for entries in self.dirs.values_mut() {
            entries.retain(|entry| {
                let entry_key = identity_key(entry.path.as_str());
                !stale_keys.contains(&entry_key)
            });
        }
        self.dirs
            .retain(|bucket, _| bucket.is_empty() || !stale_keys.contains(bucket));
        self.files.retain(|entry| {
            let entry_key = identity_key(entry.path.as_str());
            !stale_keys.contains(&entry_key)
        });
        stale.into_iter().map(|(_, display)| display).collect()
    }

    pub(crate) fn contains(&self, path: &WorkspacePath) -> bool {
        self.existing.contains_key(&identity_key(path.as_str()))
    }

    /// Input for `Workspace::files`: paged flat file list.
    pub(crate) fn files_page(&self, range: std::ops::Range<usize>) -> Result<Entries> {
        page(&self.files, range, self.complete)
    }

    /// Input for `Workspace::children`: paged children of a directory.
    pub(crate) fn children_page(
        &self,
        parent: Option<&WorkspacePath>,
        range: std::ops::Range<usize>,
    ) -> Result<Entries> {
        let key = parent.map_or_else(String::new, |p| identity_key(p.as_str()));
        let entries = self.dirs.get(&key).ok_or(Error::NotFound)?;
        page(entries, range, self.complete)
    }

    /// Identity key + display path of the direct children of one directory bucket.
    pub(crate) fn children_keys(&self, parent_key: &str) -> Vec<(String, WorkspacePath)> {
        self.dirs
            .get(parent_key)
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| (identity_key(entry.path.as_str()), entry.path.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Display paths of every known directory (root excluded), used to register watchers.
    pub(crate) fn dir_paths(&self) -> Vec<WorkspacePath> {
        self.existing
            .values()
            .filter(|(_, kind)| matches!(kind, EntryKind::Directory))
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// Sorts the snapshot once the initial scan finished.
    pub(crate) fn finish(&mut self) -> Version {
        self.files
            .sort_by(|a, b| identity_cmp(a.path.as_str(), b.path.as_str()));
        for entries in self.dirs.values_mut() {
            entries.sort_by(|a, b| {
                let a_dir = matches!(a.kind, EntryKind::Directory);
                let b_dir = matches!(b.kind, EntryKind::Directory);
                b_dir
                    .cmp(&a_dir)
                    .then_with(|| identity_cmp(a.path.as_str(), b.path.as_str()))
            });
        }
        self.complete = true;
        self.bump()
    }

    /// Clears everything for a full re-scan; the revision keeps increasing.
    pub(crate) fn clear(&mut self) {
        self.files.clear();
        self.dirs.clear();
        self.dirs.insert(String::new(), Vec::new());
        self.existing.clear();
        self.permission_read_only.clear();
        self.complete = false;
    }
}

fn page(entries: &[Entry], range: std::ops::Range<usize>, complete: bool) -> Result<Entries> {
    if range.start > range.end {
        return Err(Error::InvalidInput);
    }
    if range.start > entries.len() {
        return Err(Error::OutOfBounds);
    }
    Ok(Entries {
        items: Arc::from(&entries[range.start..range.end.min(entries.len())]),
        complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(value: &str) -> WorkspacePath {
        WorkspacePath::new(value).unwrap()
    }

    #[test]
    fn children_are_created_for_out_of_order_arrivals() {
        let mut snapshot = Snapshot::new();
        assert!(snapshot.add(path("Assets/Scripts/Player.cs"), EntryKind::File));
        // The directory entries were synthesized, so children lookups already work.
        assert_eq!(snapshot.children_page(None, 0..16).unwrap().items.len(), 1);
        assert_eq!(
            snapshot
                .children_page(Some(&path("Assets")), 0..16)
                .unwrap()
                .items
                .len(),
            1
        );
        // Adding the real directory later is a no-op, not a duplicate.
        assert!(!snapshot.add(path("Assets/Scripts"), EntryKind::Directory));
    }

    #[test]
    fn partial_snapshots_report_complete_false() {
        let mut snapshot = Snapshot::new();
        snapshot.add(path("A.cs"), EntryKind::File);
        assert!(!snapshot.files_page(0..1).unwrap().complete);
        snapshot.finish();
        assert!(snapshot.files_page(0..1).unwrap().complete);
    }

    #[test]
    fn removing_a_directory_removes_its_subtree_with_display_paths() {
        let mut snapshot = Snapshot::new();
        snapshot.add(path("Assets/A.cs"), EntryKind::File);
        snapshot.add(path("Assets/Sub/B.cs"), EntryKind::File);
        snapshot.add(path("Other.cs"), EntryKind::File);
        let removed = snapshot.remove(&path("Assets"));
        let removed: Vec<&str> = removed.iter().map(|p| p.as_str()).collect();
        assert!(removed.contains(&"Assets"));
        assert!(removed.contains(&"Assets/A.cs"));
        assert!(removed.contains(&"Assets/Sub/B.cs"));
        assert_eq!(snapshot.file_count(), 1);
        assert!(snapshot
            .children_page(Some(&path("Assets")), 0..16)
            .is_err());
    }

    #[test]
    fn paging_errors_are_distinct() {
        let mut snapshot = Snapshot::new();
        snapshot.add(path("A.cs"), EntryKind::File);
        assert_eq!(
            snapshot
                .files_page(std::ops::Range { start: 2, end: 1 })
                .unwrap_err(),
            Error::InvalidInput
        );
        assert_eq!(snapshot.files_page(9..10).unwrap_err(), Error::OutOfBounds);
        assert_eq!(snapshot.files_page(0..100).unwrap().items.len(), 1);
    }

    #[test]
    fn directories_sort_before_files_once_finished() {
        let mut snapshot = Snapshot::new();
        snapshot.add(path("b.cs"), EntryKind::File);
        snapshot.add(path("A"), EntryKind::Directory);
        snapshot.add(path("C"), EntryKind::Directory);
        snapshot.finish();
        let children = snapshot.children_page(None, 0..16).unwrap();
        let names: Vec<&str> = children.items.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(names, ["A", "C", "b.cs"]);
    }
}
