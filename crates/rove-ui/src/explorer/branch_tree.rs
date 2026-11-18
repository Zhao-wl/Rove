use super::{branch_history::Changes, model::parent_path};
use rove_base::WorkspacePath;
use rove_workspace::{Entry, EntryKind, Workspace};
use std::collections::BTreeMap;

pub(super) fn entries(
    changes: &mut Changes,
    workspace: &dyn Workspace,
) -> BTreeMap<String, Vec<Entry>> {
    changes.retain(|path, _| {
        let mut current = Some(path.as_str());
        while let Some(path) = current {
            let Ok(workspace_path) = WorkspacePath::new(path) else {
                return false;
            };
            if workspace
                .rules(&workspace_path)
                .is_ok_and(|rules| rules.excluded)
            {
                return false;
            }
            current = parent_path(path);
        }
        true
    });
    let mut nodes = BTreeMap::new();
    for path in changes.keys() {
        nodes.insert(path.as_str(), EntryKind::File);
        let mut current = parent_path(path);
        while let Some(path) = current {
            nodes.insert(path, EntryKind::Directory);
            current = parent_path(path);
        }
    }
    let mut result = BTreeMap::<String, Vec<Entry>>::new();
    for (path, kind) in nodes {
        if let Ok(workspace_path) = WorkspacePath::new(path) {
            result
                .entry(parent_path(path).unwrap_or_default().to_owned())
                .or_default()
                .push(Entry {
                    path: workspace_path,
                    kind,
                });
        }
    }
    result
}
