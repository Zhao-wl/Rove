//! Filesystem deletion and its undo receipt. Confirmation stays on the UI
//! thread, while the workspace and Shell operations run in the background.

use super::Workbench;
use crate::explorer::{registry, view::ExplorerUndo};
use crate::feedback::FeedbackAction;
use crate::shell::feedback::{Reason, Severity};
use gpui::Context;
use rove_base::{Value, WorkspacePath};
use rove_config::{Key, Scope};
use rove_i18n::Message;
use rove_workspace::{
    file_operations::{EntryStamp, OperationError},
    Folder,
};
use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(windows)]
mod native_clipboard;
#[cfg(windows)]
mod transfer_ops;

const ENABLE_TRASH: Key<bool> = Key::new("files.enableTrash");
const CONFIRM_DELETE: Key<bool> = Key::new("explorer.confirmDelete");
const ENABLE_UNDO: Key<bool> = Key::new("explorer.enableUndo");
const CONFIRM_UNDO: Key<String> = Key::new("explorer.confirmUndo");

#[cfg(windows)]
use rove_workspace::file_operations::recycle::RecycledItem;
#[cfg(not(windows))]
struct RecycledItem;

struct Receipt {
    path: WorkspacePath,
    item: RecycledItem,
}

struct SelectedEntry {
    path: WorkspacePath,
    stamp: EntryStamp,
}

#[derive(Default)]
struct Deletion {
    recycle: VecDeque<SelectedEntry>,
    permanent: VecDeque<SelectedEntry>,
    fallback: bool,
}

enum UndoStep {
    Trashed(Receipt),
    Explorer(ExplorerUndo),
    #[cfg(windows)]
    Pasted {
        path: WorkspacePath,
        stamp: EntryStamp,
    },
    #[cfg(windows)]
    Moved {
        path: WorkspacePath,
        original: std::path::PathBuf,
        stamp: EntryStamp,
    },
}
enum Request {
    Delete(Deletion),
    Restore(Vec<UndoStep>),
    #[cfg(windows)]
    Transfer(transfer_ops::Batch),
}

#[derive(Default)]
pub(super) struct FileOperations {
    busy: bool,
    next: u64,
    pending: Option<(Arc<str>, Request)>,
    undo: Vec<Vec<UndoStep>>,
    #[cfg(windows)]
    clipboard: Option<transfer_ops::ClipboardToken>,
}

impl FileOperations {
    fn pending(&mut self, request: Request) -> Arc<str> {
        self.next += 1;
        let key: Arc<str> = format!(
            "{}{next}",
            registry::FILE_OP_FEEDBACK_PREFIX,
            next = self.next
        )
        .into();
        self.pending = Some((key.clone(), request));
        key
    }

    fn take(&mut self, key: &str) -> Option<Request> {
        self.pending
            .as_ref()
            .filter(|(pending, _)| pending.as_ref() == key)?;
        self.pending.take().map(|(_, request)| request)
    }
}

impl Request {
    fn paths(&self, root: &Path, folder: &Folder) -> Vec<PathBuf> {
        #[cfg(not(windows))]
        let _ = folder;
        let absolute = |path: &WorkspacePath| root.join(path.as_str());
        match self {
            Self::Delete(plan) => plan
                .recycle
                .iter()
                .chain(&plan.permanent)
                .map(|selected| absolute(&selected.path))
                .collect(),
            Self::Restore(items) => items
                .iter()
                .flat_map(|item| match item {
                    UndoStep::Trashed(receipt) => vec![absolute(&receipt.path)],
                    UndoStep::Explorer(ExplorerUndo::Created { path, .. }) => vec![absolute(path)],
                    UndoStep::Explorer(ExplorerUndo::Renamed { source, target, .. }) => {
                        vec![absolute(source), absolute(target)]
                    }
                    #[cfg(windows)]
                    UndoStep::Pasted { path, .. } => vec![absolute(path)],
                    #[cfg(windows)]
                    UndoStep::Moved { path, original, .. } => {
                        vec![absolute(path), display_path(original, root, folder)]
                    }
                })
                .collect(),
            #[cfg(windows)]
            Self::Transfer(batch) => batch.operation_paths(root, folder),
        }
    }
}

/// Workbench saves use the folder spelling supplied by the user. A Shell
/// item may carry the canonical Windows spelling or an extended prefix.
#[cfg(windows)]
fn display_path(path: &Path, root: &Path, folder: &Folder) -> PathBuf {
    path.strip_prefix(folder.root())
        .map_or_else(|_| path.to_path_buf(), |path| root.join(path))
}

pub(super) fn handles(id: &str) -> bool {
    matches!(id, registry::MOVE_TO_TRASH | registry::DELETE_FILE)
}

impl Workbench {
    pub(super) fn remember_explorer_operation(&mut self, steps: &[ExplorerUndo]) {
        if ENABLE_UNDO
            .read(&*self.settings, &Scope::default())
            .unwrap_or(true)
        {
            self.file_operations
                .undo
                .push(steps.iter().cloned().map(UndoStep::Explorer).collect());
        }
    }
    pub(super) fn file_operation(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.file_operations.busy || self.shell.feedback.modal().is_some() {
            return;
        }
        let scope = Scope::default();
        let recycle = id == registry::MOVE_TO_TRASH
            && ENABLE_TRASH.read(&*self.settings, &scope).unwrap_or(true);
        let explorer = self.explorer.read(cx);
        let paths = explorer.operation_targets(true, cx);
        let Some(folder) = explorer.operations_folder().cloned() else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        self.file_operations.busy = true;
        let prepared = cx.background_executor().spawn(async move {
            let mut plan = Deletion::default();
            for path in paths {
                let stamp = folder.entry_stamp(&path)?;
                let absolute = folder.absolute(&path);
                let selected = SelectedEntry { path, stamp };
                if recycle && available(&absolute) {
                    plan.recycle.push_back(selected);
                } else {
                    plan.permanent.push_back(selected);
                    plan.fallback = recycle;
                }
            }
            Ok::<_, OperationError>(plan)
        });
        cx.spawn(async move |this, cx| {
            let result = prepared.await;
            let _ = this.update(cx, |view, cx| {
                view.file_operations.busy = false;
                match result {
                    Ok(plan) => view.confirm_deletion(plan, cx),
                    Err(error) => view.file_operation_error(error, None, cx),
                }
            });
        })
        .detach();
    }

    fn confirm_deletion(&mut self, plan: Deletion, cx: &mut Context<Self>) {
        let recyclable_only = plan.permanent.is_empty();
        if recyclable_only
            && !CONFIRM_DELETE
                .read(&*self.settings, &Scope::default())
                .unwrap_or(true)
        {
            self.run_file_operation(Request::Delete(plan), cx);
            return;
        }
        let count = plan.recycle.len() + plan.permanent.len();
        let permanent = plan.permanent.len();
        let message = if plan.fallback {
            "explorer.delete.fallback"
        } else if recyclable_only {
            "explorer.delete.recycle"
        } else {
            "explorer.delete.permanent"
        };
        let mut actions = vec![
            button(
                registry::FILE_OP_CONFIRM,
                if recyclable_only {
                    "explorer.delete.recycleButton"
                } else {
                    "explorer.delete.permanentButton"
                },
            ),
            button(registry::FILE_OP_CANCEL, "explorer.operation.cancel"),
        ];
        if recyclable_only {
            actions.push(button(
                registry::FILE_OP_REMEMBER,
                "explorer.delete.recycleButton",
            ));
        }
        let key = self.file_operations.pending(Request::Delete(plan));
        let _ = self.shell.feedback.report(
            key.clone(),
            Reason::DataLoss,
            Message {
                key: message.into(),
                args: Arc::from([
                    (Arc::from("count"), Arc::from(count.to_string())),
                    (Arc::from("permanent"), Arc::from(permanent.to_string())),
                ]),
            },
            actions.into(),
        );
        if recyclable_only {
            self.shell.feedback.add_modal_checkbox(
                &key,
                Message::key("explorer.operation.neverAsk"),
                registry::FILE_OP_CONFIRM,
                registry::FILE_OP_REMEMBER,
            );
        }
        cx.notify();
    }

    pub(super) fn undo_file_operation(&mut self, cx: &mut Context<Self>) {
        if self.file_operations.busy
            || self.shell.feedback.modal().is_some()
            || !ENABLE_UNDO
                .read(&*self.settings, &Scope::default())
                .unwrap_or(true)
        {
            return;
        }
        let Some(items) = self.file_operations.undo.pop() else {
            return;
        };
        let level = CONFIRM_UNDO
            .read(&*self.settings, &Scope::default())
            .unwrap_or_default();
        let data_loss = items.iter().any(|item| {
            matches!(
                item,
                UndoStep::Explorer(ExplorerUndo::Created {
                    directory: false,
                    ..
                })
            )
        });
        if level == "verbose" || (level == "default" && data_loss) {
            let key = self.file_operations.pending(Request::Restore(items));
            let _ = self.shell.feedback.report(
                key,
                if data_loss {
                    Reason::DataLoss
                } else {
                    Reason::SecurityConfirmation
                },
                Message::key("explorer.undo.restore"),
                Arc::from([
                    button(registry::FILE_OP_CONFIRM, "explorer.undo.restoreButton"),
                    button(registry::FILE_OP_CANCEL, "explorer.operation.cancel"),
                ]),
            );
            cx.notify();
        } else {
            self.run_file_operation(Request::Restore(items), cx);
        }
    }

    pub(super) fn file_operation_dismissed(&mut self, key: &str) {
        if let Some(Request::Restore(items)) = self.file_operations.take(key) {
            self.file_operations.undo.push(items);
        }
    }

    pub(super) fn file_operation_choice(
        &mut self,
        key: &str,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        if action == registry::FILE_OP_CANCEL {
            self.file_operation_dismissed(key);
            return;
        }
        let Some(request) = self.file_operations.take(key) else {
            return;
        };
        if action == registry::FILE_OP_REMEMBER {
            self.write_setting(CONFIRM_DELETE.id, Value::Bool(false), cx);
        }
        self.run_file_operation(request, cx);
    }

    fn run_file_operation(&mut self, request: Request, cx: &mut Context<Self>) {
        let Some(folder) = self.explorer.read(cx).operations_folder().cloned() else {
            return;
        };
        let root = self.root.clone();
        #[cfg(windows)]
        if let Request::Transfer(batch) = &request {
            let target_is_open = self
                .editor
                .groups
                .values()
                .flat_map(|pane| &pane.tabs)
                .filter_map(|tab| self.disk_path(&tab.resource))
                .any(|path| batch.would_replace_open_path(&root, &path));
            if target_is_open {
                self.report_file_failure(
                    "explorer.operation.targetOpen",
                    "destination belongs to an open editor",
                    Some(request),
                    cx,
                );
                return;
            }
        }
        let paths = request.paths(&root, &folder);
        let ticket = self
            .editing
            .saves
            .borrow_mut()
            .reserve_file_operation(paths);
        let Some(ticket) = ticket else {
            self.report_file_failure(
                "explorer.operation.busy",
                "save in progress",
                Some(request),
                cx,
            );
            return;
        };
        self.file_operations.busy = true;
        let worker = cx.background_executor().spawn(async move {
            match request {
                Request::Delete(plan) => delete(&folder, plan),
                Request::Restore(items) => restore(&folder, &root, items),
                #[cfg(windows)]
                Request::Transfer(batch) => transfer_ops::execute(&folder, &root, batch),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = worker.await;
            let _ = this.update(cx, |view, cx| {
                view.file_operations.busy = false;
                view.editing
                    .saves
                    .borrow_mut()
                    .complete_file_operation(ticket, &result.moves);
                if !result.undo.is_empty()
                    && ENABLE_UNDO
                        .read(&*view.settings, &Scope::default())
                        .unwrap_or(true)
                {
                    view.file_operations.undo.push(result.undo);
                }
                if let Some(plan) = result.fallback {
                    view.confirm_deletion(plan, cx);
                } else if let Some(failure) = result.failure {
                    view.report_file_failure(failure.message, failure.error, failure.retry, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn file_operation_error(
        &mut self,
        error: impl std::fmt::Debug,
        retry: Option<Request>,
        cx: &mut Context<Self>,
    ) {
        self.report_file_failure("explorer.operation.failed", error, retry, cx);
    }

    fn report_file_failure(
        &mut self,
        message: &'static str,
        error: impl std::fmt::Debug,
        retry: Option<Request>,
        cx: &mut Context<Self>,
    ) {
        eprintln!("explorer file operation: {error:?}");
        let (key, actions): (Arc<str>, Arc<[FeedbackAction]>) = match retry {
            Some(request) => (
                self.file_operations.pending(request),
                Arc::from([button(
                    registry::FILE_OP_CONFIRM,
                    "explorer.operation.retry",
                )]),
            ),
            None => (message.into(), Arc::from([])),
        };
        let _ = self.shell.feedback.report(
            key,
            Reason::Global(Severity::Error),
            Message::key(message),
            actions,
        );
        cx.notify();
    }
}

fn button(id: &'static str, label: &'static str) -> FeedbackAction {
    FeedbackAction {
        id: id.into(),
        label: Message::key(label),
    }
}

#[derive(Default)]
struct Outcome {
    undo: Vec<UndoStep>,
    moves: Vec<(PathBuf, PathBuf)>,
    fallback: Option<Deletion>,
    failure: Option<Failure>,
}

struct Failure {
    message: &'static str,
    error: io::Error,
    retry: Option<Request>,
}

impl Failure {
    fn io(error: io::Error, retry: Request) -> Self {
        let message = match error.kind() {
            _ if matches!(error.raw_os_error(), Some(32 | 33)) => "explorer.operation.locked",
            _ if error.raw_os_error() == Some(206) => "explorer.operation.long",
            io::ErrorKind::PermissionDenied => "explorer.operation.denied",
            _ => "explorer.operation.failed",
        };
        Self {
            message,
            error,
            retry: Some(retry),
        }
    }
}

fn operation_io(error: OperationError) -> io::Error {
    match error {
        OperationError::Io { source, .. } => source,
        OperationError::ReadOnly(_) => {
            io::Error::new(io::ErrorKind::PermissionDenied, format!("{error:?}"))
        }
        error => io::Error::other(format!("{error:?}")),
    }
}

fn delete(folder: &Folder, mut plan: Deletion) -> Outcome {
    let mut outcome = Outcome::default();
    while let Some(selected) = plan.recycle.pop_front() {
        let result = folder
            .checked_entry_path(&selected.path, &selected.stamp)
            .map_err(operation_io)
            .and_then(|path| recycle(&path));
        match result {
            Ok(item) => outcome.undo.push(UndoStep::Trashed(Receipt {
                path: selected.path,
                item,
            })),
            Err(error) if unavailable(&error) => {
                plan.fallback = true;
                plan.permanent.push_back(selected);
                outcome.fallback = Some(plan);
                return outcome;
            }
            Err(error) => {
                plan.recycle.push_front(selected);
                outcome.failure = Some(Failure::io(error, Request::Delete(plan)));
                return outcome;
            }
        }
    }
    while let Some(selected) = plan.permanent.pop_front() {
        if let Err(error) = folder.remove_permanently(&selected.path, &selected.stamp) {
            plan.permanent.push_front(selected);
            outcome.failure = Some(Failure::io(operation_io(error), Request::Delete(plan)));
            return outcome;
        }
    }
    outcome
}

fn restore(folder: &Folder, root: &Path, mut items: Vec<UndoStep>) -> Outcome {
    let mut outcome = Outcome::default();
    while let Some(step) = items.last() {
        let result = match step {
            #[cfg(windows)]
            UndoStep::Pasted { path, stamp } => folder
                .checked_entry_identity(path, stamp)
                .map_err(operation_io)
                .and_then(|path| recycle(&path).map(|_| ())),
            #[cfg(windows)]
            UndoStep::Moved {
                path,
                original,
                stamp,
            } => folder
                .checked_entry_identity(path, stamp)
                .and_then(|_| folder.undo_transfer(path, original))
                .map_err(operation_io)
                .and_then(|outcome| match outcome {
                    rove_workspace::file_operations::transfer::TransferOutcome::SourceRetained(
                        error,
                    ) => Err(error),
                    _ => Ok(()),
                }),
            UndoStep::Trashed(receipt) => folder
                .restore_target(&receipt.path)
                .map_err(operation_io)
                .and_then(|path| restore_item(&receipt.item, &path)),
            UndoStep::Explorer(ExplorerUndo::Created {
                path,
                stamp,
                directory,
            }) => {
                if *directory {
                    folder
                        .remove_empty_directory(path, stamp)
                        .map_err(operation_io)
                } else {
                    folder
                        .checked_entry_identity(path, stamp)
                        .map_err(operation_io)
                        .and_then(|path| recycle(&path).map(|_| ()))
                }
            }
            UndoStep::Explorer(ExplorerUndo::Renamed {
                source,
                target,
                stamp,
            }) => folder
                .checked_entry_identity(target, stamp)
                .and_then(|_| folder.move_entry(target, source))
                .map_err(operation_io),
        };
        if let Err(error) = result {
            outcome.failure = Some(Failure::io(error, Request::Restore(items)));
            return outcome;
        }
        match step {
            UndoStep::Explorer(ExplorerUndo::Renamed { source, target, .. }) => {
                outcome
                    .moves
                    .push((root.join(target.as_str()), root.join(source.as_str())));
            }
            #[cfg(windows)]
            UndoStep::Moved { path, original, .. } => {
                outcome.moves.push((
                    root.join(path.as_str()),
                    display_path(original, root, folder),
                ));
            }
            _ => {}
        }
        items.pop();
    }
    outcome
}

#[cfg(windows)]
use rove_workspace::file_operations::recycle::{
    available, recycle, restore as restore_item, unavailable,
};

#[cfg(not(windows))]
fn available(_: &Path) -> bool {
    false
}

#[cfg(not(windows))]
fn unavailable(_: &io::Error) -> bool {
    true
}

#[cfg(not(windows))]
fn recycle(_: &Path) -> io::Result<RecycledItem> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(not(windows))]
fn restore_item(_: &RecycledItem, _: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
