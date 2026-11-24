//! Clipboard commands and transfer planning for Explorer.

use super::{
    button, native_clipboard as clipboard, operation_io, Failure, Outcome, Receipt, Request,
    SelectedEntry, UndoStep, Workbench,
};
use crate::{explorer::registry, shell::feedback::Reason};
use gpui::{Context, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use rove_base::WorkspacePath;
use rove_config::{Key, Scope};
use rove_i18n::Message;
use rove_workspace::{
    file_operations::{
        entry_path, recycle,
        transfer::{
            inspect_source, next_duplicate_name, Naming, Transfer, TransferOutcome, TransferSource,
        },
        OperationError,
    },
    EntryKind, Folder,
};
use std::{
    collections::{BTreeSet, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use windows::Win32::Foundation::HWND;

const CONFIRM_NATIVE: Key<bool> = Key::new("explorer.confirmPasteNative");
const CONFIRM_DROP: Key<bool> = Key::new("explorer.confirmDragAndDrop");
const NAMING: Key<String> = Key::new("explorer.incrementalNaming");

#[derive(Clone, Copy)]
pub(super) struct ClipboardToken {
    owner: usize,
    revision: u32,
    pub cut: bool,
}

pub(super) struct Batch {
    entries: VecDeque<TransferItem>,
    transfer: Transfer,
    clear: Option<ClipboardToken>,
}

impl Batch {
    pub(super) fn would_replace_open_path(&self, root: &Path, open: &Path) -> bool {
        self.entries.iter().any(|item| {
            item.replace.is_some()
                && crate::save::controller::relative_path(open, &root.join(item.to.as_str()))
                    .is_some()
        })
    }

    pub(super) fn operation_paths(&self, root: &Path, folder: &Folder) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for item in &self.entries {
            paths.push(root.join(item.to.as_str()));
            if self.transfer == Transfer::Move {
                paths.push(super::display_path(&item.from.path, root, folder));
            }
        }
        paths
    }
}

struct TransferItem {
    from: TransferSource,
    to: WorkspacePath,
    replace: Option<SelectedEntry>,
}

impl Workbench {
    pub(crate) fn explorer_clipboard_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(owner) = hwnd(window) else { return };
        if id == registry::CANCEL_CUT {
            let _ = self.cancel_explorer_cut(cx);
            return;
        }
        if self.file_operations.busy || self.shell.feedback.modal().is_some() {
            return;
        }
        if id == registry::PASTE_FILES {
            self.paste_explorer_files(owner, cx);
            return;
        }
        let cut = id == registry::CUT_FILES;
        let explorer = self.explorer.read(cx);
        let selected = explorer.operation_targets(cut, cx);
        let paths = selected
            .iter()
            .map(|path| self.root.join(path.as_str()))
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }
        let effect = if cut {
            clipboard::Effect::Move
        } else {
            clipboard::Effect::Copy
        };
        match clipboard::write(owner, &paths, effect) {
            Ok(revision) => {
                self.file_operations.clipboard = Some(ClipboardToken {
                    owner: owner.0 as usize,
                    revision,
                    cut,
                });
                let cut_paths = if cut { selected.as_slice() } else { &[] };
                self.explorer
                    .update(cx, |explorer, cx| explorer.set_cut(cut_paths, cx));
            }
            Err(error) => self.file_operation_error(error, None, cx),
        }
    }

    pub(crate) fn cancel_explorer_cut(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(token) = self.file_operations.clipboard.filter(|token| token.cut) else {
            return false;
        };
        self.file_operations.clipboard = None;
        self.explorer
            .update(cx, |explorer, cx| explorer.set_cut(&[], cx));
        if let Err(error) = clipboard::clear_current(token.owner, token.revision) {
            eprintln!("explorer cancel cut: {error}");
        }
        true
    }

    pub(crate) fn sync_explorer_clipboard(&mut self, cx: &mut Context<Self>) {
        if self
            .file_operations
            .clipboard
            .is_some_and(|state| state.revision != clipboard::sequence())
        {
            self.file_operations.clipboard = None;
            self.explorer
                .update(cx, |explorer, cx| explorer.set_cut(&[], cx));
        }
    }

    fn paste_explorer_files(&mut self, owner: HWND, cx: &mut Context<Self>) {
        let view = self.explorer.read(cx);
        let Some(folder) = view.operations_folder().cloned() else {
            return;
        };
        let Some(parent) = view.creation_parent(cx) else {
            return;
        };
        let own = self.file_operations.clipboard.map(|state| state.revision);
        let owner = owner.0 as usize;
        let naming = naming(self);
        let confirm_native = CONFIRM_NATIVE
            .read(&*self.settings, &Scope::default())
            .unwrap_or(true);
        self.file_operations.busy = true;
        let worker = cx.background_executor().spawn(async move {
            let Some(data) = clipboard::read()? else {
                return Ok(None);
            };
            let cut = data.effect == clipboard::Effect::Move;
            let transfer = if cut { Transfer::Move } else { Transfer::Copy };
            let clear = cut.then_some(ClipboardToken {
                owner,
                revision: data.sequence,
                cut,
            });
            let batch = prepare(
                &folder,
                data.paths,
                parent.as_ref(),
                transfer,
                naming,
                clear,
            )?;
            Ok::<_, io::Error>(Some((batch, confirm_native && own != Some(data.sequence))))
        });
        cx.spawn(async move |this, cx| {
            let result = worker.await;
            let _ = this.update(cx, |view, cx| {
                view.file_operations.busy = false;
                match result {
                    Ok(Some((batch, native))) => view.confirm_transfer(batch, native, false, cx),
                    Ok(None) => {}
                    Err(error) => view.file_operation_error(error, None, cx),
                }
            });
        })
        .detach();
    }

    pub(crate) fn move_explorer_drop(
        &mut self,
        paths: &[WorkspacePath],
        parent: Option<WorkspacePath>,
        cx: &mut Context<Self>,
    ) {
        if self.file_operations.busy || self.shell.feedback.modal().is_some() {
            return;
        }
        let Some(folder) = self.explorer.read(cx).operations_folder().cloned() else {
            return;
        };
        let paths = paths
            .iter()
            .map(|path| self.root.join(path.as_str()))
            .collect();
        let confirm = CONFIRM_DROP
            .read(&*self.settings, &Scope::default())
            .unwrap_or(true);
        self.file_operations.busy = true;
        let worker = cx.background_executor().spawn(async move {
            prepare(
                &folder,
                paths,
                parent.as_ref(),
                Transfer::Move,
                Naming::Disabled,
                None,
            )
        });
        cx.spawn(async move |this, cx| {
            let result = worker.await;
            let _ = this.update(cx, |view, cx| {
                view.file_operations.busy = false;
                match result {
                    Ok(batch) => view.confirm_transfer(batch, false, confirm, cx),
                    Err(error) => view.file_operation_error(error, None, cx),
                }
            });
        })
        .detach();
    }

    fn confirm_transfer(&mut self, batch: Batch, native: bool, drop: bool, cx: &mut Context<Self>) {
        if batch.entries.is_empty() {
            return;
        }
        let overwrites = batch
            .entries
            .iter()
            .filter(|item| item.replace.is_some())
            .count();
        if !native && !drop && overwrites == 0 {
            self.run_file_operation(Request::Transfer(batch), cx);
            return;
        }
        let count = batch.entries.len();
        let key = self.file_operations.pending(Request::Transfer(batch));
        let message = if overwrites > 0 {
            "explorer.paste.replace"
        } else if drop {
            "explorer.drop.confirm"
        } else {
            "explorer.paste.native"
        };
        let _ = self.shell.feedback.report(
            key,
            if overwrites > 0 {
                Reason::DataLoss
            } else {
                Reason::SecurityConfirmation
            },
            Message {
                key: message.into(),
                args: Arc::from([
                    (Arc::from("count"), Arc::from(count.to_string())),
                    (Arc::from("replace"), Arc::from(overwrites.to_string())),
                ]),
            },
            Arc::from([
                button(registry::FILE_OP_CONFIRM, "explorer.operation.continue"),
                button(registry::FILE_OP_CANCEL, "explorer.operation.cancel"),
            ]),
        );
        cx.notify();
    }
}

fn naming(view: &Workbench) -> Naming {
    match NAMING.read(&*view.settings, &Scope::default()).as_deref() {
        Ok("smart") => Naming::Smart,
        Ok("disabled") => Naming::Disabled,
        _ => Naming::Simple,
    }
}

fn hwnd(window: &Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(window) => Some(HWND(window.hwnd.get() as *mut _)),
        _ => None,
    }
}

fn prepare(
    folder: &Folder,
    sources: Vec<PathBuf>,
    parent: Option<&WorkspacePath>,
    transfer: Transfer,
    naming: Naming,
    clear: Option<ClipboardToken>,
) -> io::Result<Batch> {
    let mut entries = VecDeque::new();
    let mut observed = BTreeSet::new();
    let mut destinations = BTreeSet::new();
    for original in sources {
        let from = inspect_source(&original)?;
        if !observed.insert(rove_workspace::fold_key(&from.path.to_string_lossy())) {
            continue;
        }
        let mut name = from
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(io::ErrorKind::InvalidInput)?
            .to_owned();
        let directory = matches!(from.kind, EntryKind::Directory);
        loop {
            let to = entry_path(parent, &name, false).map_err(other)?;
            let key = rove_workspace::fold_key(to.as_str());
            if (transfer == Transfer::Move || naming == Naming::Disabled)
                && same_path(&from.path, &folder.absolute(&to))
            {
                break;
            }
            let occupied = destinations.contains(&key);
            let replace = match folder.restore_target(&to) {
                Ok(_) if !occupied => None,
                Ok(_) => {
                    name = next_duplicate_name(&name, directory, naming)
                        .ok_or(io::ErrorKind::AlreadyExists)?;
                    continue;
                }
                Err(OperationError::Io { source, .. })
                    if source.kind() == io::ErrorKind::AlreadyExists =>
                {
                    if let Some(next) = next_duplicate_name(&name, directory, naming) {
                        name = next;
                        continue;
                    }
                    if occupied {
                        return Err(io::ErrorKind::AlreadyExists.into());
                    }
                    Some(SelectedEntry {
                        stamp: folder.entry_stamp(&to).map_err(other)?,
                        path: to.clone(),
                    })
                }
                Err(error) => return Err(other(error)),
            };
            folder
                .verify_transfer_tree(&from.path, &to, transfer)
                .map_err(operation_io)?;
            destinations.insert(key);
            entries.push_back(TransferItem { from, to, replace });
            break;
        }
    }
    Ok(Batch {
        entries,
        transfer,
        clear,
    })
}

fn same_path(left: &Path, right: &Path) -> bool {
    rove_workspace::fold_key(&left.to_string_lossy().replace('/', "\\"))
        == rove_workspace::fold_key(&right.to_string_lossy().replace('/', "\\"))
}

fn other(error: impl std::fmt::Debug) -> io::Error {
    io::Error::other(format!("{error:?}"))
}

pub(super) fn execute(folder: &Folder, root: &Path, mut batch: Batch) -> Outcome {
    let mut result = Outcome::default();
    while let Some(item) = batch.entries.pop_front() {
        if let Err(error) = item.from.verify() {
            result.failure = Some(source_failure(error, batch, item));
            return result;
        }
        let replaced = match item
            .replace
            .as_ref()
            .map(|entry| replace(folder, entry))
            .transpose()
        {
            Ok(replaced) => replaced,
            Err(error) => {
                batch.entries.push_front(item);
                result.failure = Some(Failure::io(error, Request::Transfer(batch)));
                return result;
            }
        };
        let verified = item.from.verify();
        let source_invalid = verified.is_err();
        let operation = verified.and_then(|()| {
            folder
                .transfer_from(&item.from.path, &item.to, batch.transfer)
                .map_err(operation_io)
        });
        if let Err(error) = operation {
            if let Some(receipt) = replaced {
                if let Err(restoration) = folder
                    .restore_target(&receipt.path)
                    .map_err(other)
                    .and_then(|path| recycle::restore(&receipt.item, &path))
                {
                    eprintln!("explorer displaced item remains recoverable: {restoration}");
                    result.undo.push(UndoStep::Trashed(receipt));
                }
            }
            result.failure = Some(if source_invalid {
                source_failure(error, batch, item)
            } else {
                batch.entries.push_front(item);
                Failure::io(error, Request::Transfer(batch))
            });
            return result;
        }
        if let Err(error) = record_completion(
            folder,
            root,
            &mut result,
            item,
            replaced,
            operation.expect("transfer error handled"),
        ) {
            result.failure = Some(Failure {
                message: "explorer.operation.sourceRetained",
                error,
                retry: (!batch.entries.is_empty()).then_some(Request::Transfer(batch)),
            });
            return result;
        }
    }
    if let Some(token) = batch.clear {
        if let Err(error) = clipboard::clear_current(token.owner, token.revision) {
            eprintln!("explorer consumed cut: {error}");
        }
    }
    result
}

fn source_failure(error: io::Error, mut batch: Batch, item: TransferItem) -> Failure {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidData | io::ErrorKind::NotFound
    ) {
        return Failure {
            message: "explorer.operation.sourceChanged",
            error,
            retry: None,
        };
    }
    batch.entries.push_front(item);
    Failure::io(error, Request::Transfer(batch))
}

fn record_completion(
    folder: &Folder,
    root: &Path,
    result: &mut Outcome,
    item: TransferItem,
    replaced: Option<Receipt>,
    outcome: TransferOutcome,
) -> io::Result<()> {
    // After partial source cleanup, the destination may contain the only copy
    // of some files. It cannot be recycled like an ordinary pasted duplicate.
    if let TransferOutcome::SourceRetained(error) = outcome {
        return Err(error);
    }
    if let Some(receipt) = replaced {
        result.undo.push(UndoStep::Trashed(receipt));
    }
    let stamp = folder.entry_stamp(&item.to).ok();
    if matches!(outcome, TransferOutcome::Moved) {
        result.moves.push(super::FileMove {
            from: super::display_path(&item.from.path, root, folder),
            to: root.join(item.to.as_str()),
            stamp: stamp.clone(),
        });
    }
    if let Some(stamp) = stamp {
        match outcome {
            TransferOutcome::Moved => result.undo.push(UndoStep::Moved {
                path: item.to,
                original: item.from.path,
                stamp,
            }),
            TransferOutcome::Copied => result.undo.push(UndoStep::Pasted {
                path: item.to,
                stamp,
            }),
            _ => {}
        }
    }
    Ok(())
}

fn replace(folder: &Folder, selected: &SelectedEntry) -> io::Result<Receipt> {
    let path = folder
        .checked_entry_path(&selected.path, &selected.stamp)
        .map_err(operation_io)?;
    let item = recycle::recycle(&path)?;
    Ok(Receipt {
        path: selected.path.clone(),
        item,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_protects_open_destination_paths_and_rechecks_cut_source_before_recycling() {
        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let root = workspace.path();
        let target = root.join("Target");
        let source = external.path().join("Target");
        std::fs::create_dir(&target).unwrap();
        std::fs::create_dir(&source).unwrap();
        std::fs::write(target.join("Keep.txt"), "old destination").unwrap();
        std::fs::write(source.join("Other.txt"), "approved source").unwrap();
        let folder = Folder::scan_root(
            root,
            rove_workspace::WorkspaceConfig::default().without_watching(),
        )
        .unwrap();
        let batch = prepare(
            &folder,
            vec![source.clone()],
            None,
            Transfer::Move,
            Naming::Disabled,
            None,
        )
        .unwrap();
        assert!(batch.would_replace_open_path(root, &root.join("TARGET/Keep.txt")));
        assert!(!batch.would_replace_open_path(root, &root.join("Targeted/Other.txt")));

        std::fs::rename(&source, external.path().join("Original")).unwrap();
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("Unapproved.txt"), "unconfirmed replacement").unwrap();
        let result = execute(&folder, root, batch);
        let failure = result.failure.unwrap();
        assert_eq!(failure.message, "explorer.operation.sourceChanged");
        assert!(failure.retry.is_none());
        assert!(result.undo.is_empty());
        assert_eq!(
            std::fs::read_to_string(target.join("Keep.txt")).unwrap(),
            "old destination"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("Unapproved.txt")).unwrap(),
            "unconfirmed replacement"
        );
    }

    #[test]
    fn partial_cut_keeps_the_destination_out_of_the_paste_undo() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        std::fs::write(root.join("OnlyCopy.txt"), "original moved bytes").unwrap();
        let folder = Folder::scan_root(
            root,
            rove_workspace::WorkspaceConfig::default().without_watching(),
        )
        .unwrap();
        let source = root.join("Source.txt");
        std::fs::write(&source, "not the payload").unwrap();
        let item = TransferItem {
            from: inspect_source(&source).unwrap(),
            to: WorkspacePath::new("OnlyCopy.txt").unwrap(),
            replace: None,
        };
        let mut result = Outcome::default();
        let incomplete = TransferOutcome::SourceRetained(io::ErrorKind::DirectoryNotEmpty.into());
        assert!(record_completion(&folder, root, &mut result, item, None, incomplete).is_err());
        assert!(result.undo.is_empty());
        assert!(result.moves.is_empty());
        assert_eq!(
            std::fs::read_to_string(root.join("OnlyCopy.txt")).unwrap(),
            "original moved bytes"
        );
    }
}
