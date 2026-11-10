//! Workspace snapshots and file-event contracts (#15); scanning never blocks the UI.
use rove_base::*;
use rove_buffer::ReadOnlyReason;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub enum EntryKind {
    File,
    Directory,
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: WorkspacePath,
    pub kind: EntryKind,
}
/// Partial scanning is explicit; completed=false never means no matching files exist.
#[derive(Clone, Debug)]
pub struct Entries {
    pub items: Arc<[Entry]>,
    pub complete: bool,
}
#[derive(Clone, Debug)]
pub enum FileEvent {
    Created(WorkspacePath),
    Changed(WorkspacePath),
    Removed(WorkspacePath),
    Renamed {
        from: WorkspacePath,
        to: WorkspacePath,
    },
    RescanRequired,
}
#[derive(Clone, Debug)]
pub struct FileEvents {
    pub revision: Version,
    pub events: Arc<[FileEvent]>,
}
#[derive(Clone, Debug)]
pub struct FileRules {
    pub excluded: bool,
    pub read_only: Arc<[ReadOnlyReason]>,
}
/// Send + Sync immutable cached tree: O(log n + returned items), range paging avoids cloning the
/// whole file list. None parent is root. Missing directory => NotFound. Rules combine files.* and
/// rove.* extras, never .gitignore. Snapshot/revision and events allow the UI to discard stale work.
pub trait Workspace: Send + Sync {
    fn revision(&self) -> Version;
    fn files(&self, range: std::ops::Range<usize>) -> Result<Entries>;
    fn children(
        &self,
        parent: Option<&WorkspacePath>,
        range: std::ops::Range<usize>,
    ) -> Result<Entries>;
    fn rules(&self, path: &WorkspacePath) -> Result<FileRules>;
    fn changes(&self) -> &dyn Events<FileEvents>;
}
/// Empty, fully scanned workspace; unknown paths do not pretend to have valid rule results.
pub struct EmptyWorkspace;
impl Workspace for EmptyWorkspace {
    fn revision(&self) -> Version {
        Version(0)
    }
    fn files(&self, r: std::ops::Range<usize>) -> Result<Entries> {
        if r.start > r.end {
            Err(Error::InvalidInput)
        } else if r.start > 0 {
            Err(Error::OutOfBounds)
        } else {
            Ok(Entries {
                items: Arc::from([]),
                complete: true,
            })
        }
    }
    fn children(&self, p: Option<&WorkspacePath>, r: std::ops::Range<usize>) -> Result<Entries> {
        if p.is_some() {
            Err(Error::NotFound)
        } else {
            self.files(r)
        }
    }
    fn rules(&self, _: &WorkspacePath) -> Result<FileRules> {
        Err(Error::NotFound)
    }
    fn changes(&self) -> &dyn Events<FileEvents> {
        &NoEvents
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explorer_consumes_initial_snapshot() {
        let w: &dyn Workspace = &EmptyWorkspace;
        assert!(w.children(None, 0..100).unwrap().items.is_empty());
        assert!(w.files(0..100).unwrap().complete);
        assert!(w.rules(&WorkspacePath::new("A.cs").unwrap()).is_err());
        let _sub = w.changes().subscribe(Arc::new(|_| {}));
    }
}
