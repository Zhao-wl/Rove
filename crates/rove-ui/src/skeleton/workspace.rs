//! SKELETON(workspace): recursive snapshot without watchers; replaced by #15.
use rove_base::*;
use rove_workspace::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
pub struct Folder {
    files: Vec<Entry>,
    directories: BTreeMap<Option<WorkspacePath>, Vec<Entry>>,
}
impl Folder {
    pub fn scan(root: &Path, cancel: &Cancellation) -> Result<Self> {
        let mut s = Self {
            files: vec![],
            directories: BTreeMap::new(),
        };
        let mut pending = vec![(root.to_path_buf(), None::<WorkspacePath>)];
        while let Some((dir, parent)) = pending.pop() {
            cancel.check()?;
            let mut entries = vec![];
            for file in std::fs::read_dir(dir)? {
                cancel.check()?;
                let file = file?;
                let ty = file.file_type()?;
                // Do not follow links/junctions out of the workspace or recurse through cycles.
                if ty.is_symlink() {
                    continue;
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if file.metadata()?.file_attributes() & 0x400 != 0 {
                        continue;
                    }
                }
                let name = file.file_name();
                let name = name.to_str().ok_or(Error::Unsupported)?;
                if ty.is_dir()
                    && [".git", "Library", "Temp"]
                        .iter()
                        .any(|n| name.eq_ignore_ascii_case(n))
                {
                    continue;
                }
                let path = WorkspacePath::new(&match &parent {
                    Some(p) => format!("{}/{name}", p.as_str()),
                    None => name.into(),
                })?;
                let entry = Entry {
                    path: path.clone(),
                    kind: if ty.is_dir() {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                };
                if ty.is_dir() {
                    pending.push((file.path(), Some(path)))
                } else if ty.is_file() {
                    s.files.push(entry.clone())
                } else {
                    continue;
                }
                entries.push(entry);
            }
            entries.sort_by(|a, b| {
                matches!(a.kind, EntryKind::File)
                    .cmp(&matches!(b.kind, EntryKind::File))
                    .then(a.path.cmp(&b.path))
            });
            s.directories.insert(parent, entries);
        }
        s.files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(s)
    }
    pub fn root(path: PathBuf) -> Task<Arc<dyn Workspace>> {
        Box::pin(async move {
            Ok(Arc::new(Self::scan(&path, &Cancellation::default())?) as Arc<dyn Workspace>)
        })
    }
}
fn page(entries: &[Entry], r: std::ops::Range<usize>) -> Result<Entries> {
    if r.start > r.end {
        return Err(Error::InvalidInput);
    }
    if r.start > entries.len() {
        return Err(Error::OutOfBounds);
    }
    Ok(Entries {
        items: Arc::from(&entries[r.start..r.end.min(entries.len())]),
        complete: true,
    })
}
impl Workspace for Folder {
    fn revision(&self) -> Version {
        Version(0)
    }
    fn files(&self, r: std::ops::Range<usize>) -> Result<Entries> {
        page(&self.files, r)
    }
    fn children(&self, p: Option<&WorkspacePath>, r: std::ops::Range<usize>) -> Result<Entries> {
        page(self.directories.get(&p.cloned()).ok_or(Error::NotFound)?, r)
    }
    fn rules(&self, p: &WorkspacePath) -> Result<FileRules> {
        Ok(FileRules {
            excluded: p.as_str().split('/').any(|part| {
                [".git", "Library", "Temp"]
                    .iter()
                    .any(|n| part.eq_ignore_ascii_case(n))
            }),
            read_only: Arc::from([]),
        })
    }
    fn changes(&self) -> &dyn Events<FileEvents> {
        &NoEvents
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_exclusions_and_ignore_file() {
        let t = tempfile::tempdir().unwrap();
        for d in ["Library", "Temp", ".git", "Assets"] {
            std::fs::create_dir(t.path().join(d)).unwrap();
            std::fs::write(t.path().join(d).join("A.cs"), "class A {}").unwrap();
        }
        std::fs::write(t.path().join(".gitignore"), "Assets").unwrap();
        let w = Folder::scan(t.path(), &Cancellation::default()).unwrap();
        let files = w.files(0..usize::MAX).unwrap();
        assert!(files.items.iter().any(|e| e.path.as_str() == "Assets/A.cs"));
        assert!(!files
            .items
            .iter()
            .any(|e| e.path.as_str().starts_with("Library/")));
    }
}
