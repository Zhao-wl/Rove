//! Filesystem deletion and its undo receipt. Confirmation stays on the UI
//! thread, while the workspace and Shell operations run in the background.

use super::Workbench;
use crate::context_basket::model::{at_or_below, same_path};
use crate::explorer::{registry, view::ExplorerUndo};
use crate::feedback::FeedbackAction;
use crate::save::controller::relative_path;
use crate::shell::feedback::{Reason, Severity};
use gpui::Context;
use rove_base::{Resource, Value, WorkspacePath};
use rove_config::{Key, Scope};
use rove_i18n::Message;
use rove_workspace::{
    file_operations::{EntryStamp, OperationError},
    FileEvent, Folder,
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
    watched_paths: Vec<WorkspacePath>,
    deferred_events: Vec<FileEvent>,
    // Keep the newest moves and unrelated receipts for paths still in use.
    completed_moves: Vec<WorkspaceMove>,
    #[cfg(windows)]
    clipboard: Option<transfer_ops::ClipboardToken>,
}

struct FileMove {
    from: PathBuf,
    to: PathBuf,
    stamp: Option<EntryStamp>,
}

struct WorkspaceMove {
    from: WorkspacePath,
    to: WorkspacePath,
    stamp: Option<EntryStamp>,
}

impl WorkspaceMove {
    fn covers(&self, path: &WorkspacePath) -> bool {
        at_or_below(path, &self.from) || at_or_below(path, &self.to)
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.covers(&other.from)
            || self.covers(&other.to)
            || other.covers(&self.from)
            || other.covers(&self.to)
    }
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

impl Workbench {
    pub(super) fn apply_explorer_workspace_events(
        &mut self,
        mut events: Vec<FileEvent>,
        cx: &mut Context<Self>,
    ) {
        if !self.file_operations.watched_paths.is_empty() {
            let relative_to_operation = |path: &WorkspacePath| {
                self.file_operations
                    .watched_paths
                    .iter()
                    .any(|reserved| at_or_below(path, reserved) || at_or_below(reserved, path))
            };
            let (deferred, ready): (Vec<_>, Vec<_>) =
                events.into_iter().partition(|event| match event {
                    FileEvent::Created(path)
                    | FileEvent::Changed(path)
                    | FileEvent::Removed(path) => relative_to_operation(path),
                    FileEvent::Renamed { from, to } => {
                        relative_to_operation(from) || relative_to_operation(to)
                    }
                    FileEvent::RescanRequired => true,
                });
            self.file_operations.deferred_events.extend(deferred);
            events = ready;
        }
        self.reconcile_move_notifications(&mut events);
        self.apply_explorer_document_changes(events, cx);
    }

    fn reconcile_move_notifications(&self, events: &mut Vec<FileEvent>) {
        let moves = &self.file_operations.completed_moves;
        let exists = |path: &WorkspacePath| self.root.join(path.as_str()).try_exists();
        events.retain_mut(|event| match event {
            FileEvent::Removed(path) => {
                // The old occupant may have been recycled before a replacement was installed.
                if moves.iter().any(|movement| at_or_below(path, &movement.to))
                    && matches!(exists(path), Ok(true))
                {
                    *event = FileEvent::Changed(path.clone());
                }
                true
            }
            FileEvent::Renamed { from, to } => {
                // Only suppress an inverse notification if the restored object is still at
                // the source. A newly created file there can be part of a real rename chain.
                !moves.iter().any(|movement| {
                    if movement.to != *from || movement.from != *to {
                        return false;
                    }
                    let source = self.root.join(from.as_str());
                    let Some(stamp) = movement.stamp.as_ref() else {
                        return false;
                    };
                    if !matches!(stamp.matches_path(&source), Ok(true)) {
                        return false;
                    }
                    if same_path(from, to) {
                        // Windows looks up both spellings in a case-only move.
                        return source
                            .canonicalize()
                            .is_ok_and(|path| path.ends_with(from.as_str()));
                    }
                    // Undo can put the displaced previous occupant back at the old destination.
                    // An unrelated object there confirms the old move no longer describes disk.
                    let target = self.root.join(to.as_str());
                    match stamp.matches_path(&target) {
                        Ok(same) => !same,
                        Err(error) => error.kind() == io::ErrorKind::NotFound,
                    }
                })
            }
            _ => true,
        });
    }

    fn apply_explorer_document_changes(&mut self, events: Vec<FileEvent>, cx: &mut Context<Self>) {
        if events.is_empty() {
            return;
        }
        self.follow_workspace_renames(&events, cx);
        self.note_disk_events(&events, cx);
        self.refresh_workspace_images(&events, cx);
        self.basket_workspace_files_changed(&events, cx);
        if self.editor.settings.close_on_delete {
            let mut changed = false;
            for event in events {
                if let FileEvent::Removed(path) = event {
                    changed |= self
                        .editor
                        .file_removed(&rove_base::Resource::Workspace(path));
                }
            }
            if changed {
                cx.notify();
            }
        }
    }

    fn finish_explorer_workspace_files(&mut self, moves: &[FileMove], cx: &mut Context<Self>) {
        self.file_operations.watched_paths.clear();
        let mut logical = Vec::new();
        let mut completed = Vec::new();
        for movement in moves {
            if let (Some(from), Some(to)) = (
                workspace_path(&self.root, &movement.from),
                workspace_path(&self.root, &movement.to),
            ) {
                completed.push(WorkspaceMove {
                    from: from.clone(),
                    to: to.clone(),
                    stamp: movement.stamp.clone(),
                });
                logical.push(FileEvent::Renamed { from, to });
            }
        }
        self.remember_workspace_moves(completed);
        // A native cross-directory move can arrive as removal and creation.
        // Apply the confirmed mapping before anything can close the source tab.
        let mut deferred = std::mem::take(&mut self.file_operations.deferred_events);
        self.reconcile_move_notifications(&mut deferred);
        deferred.retain(|event| {
            let FileEvent::Renamed { from, to } = event else {
                return true;
            };
            !logical.iter().any(|logical| {
                matches!(logical, FileEvent::Renamed { from: old, to: new } if old == from && new == to)
            })
        });
        logical.append(&mut deferred);
        self.apply_explorer_document_changes(logical, cx);
    }

    fn remember_workspace_moves(&mut self, moves: Vec<WorkspaceMove>) {
        if moves.is_empty() {
            return;
        }
        let paths: Vec<_> = self
            .editor
            .groups
            .values()
            .flat_map(|group| &group.tabs)
            .filter_map(|tab| match &tab.resource {
                Resource::Workspace(path) => Some(path.clone()),
                Resource::File(path) => workspace_path(&self.root, path),
                Resource::Virtual(_) => None,
            })
            .chain(
                self.basket
                    .contents
                    .items()
                    .iter()
                    .map(|entry| entry.path.clone()),
            )
            .collect();
        self.file_operations.completed_moves.retain(|previous| {
            !moves.iter().any(|movement| previous.overlaps(movement))
                && paths.iter().any(|path| previous.covers(path))
        });
        self.file_operations.completed_moves.extend(moves);
    }
}

fn workspace_path(root: &Path, absolute: &Path) -> Option<WorkspacePath> {
    let relative = relative_path(absolute, root)?;
    WorkspacePath::new(&relative.to_str()?.replace('\\', "/")).ok()
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
        let watched_paths = paths
            .iter()
            .filter_map(|path| workspace_path(&root, path))
            .collect();
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
        self.file_operations.watched_paths = watched_paths;
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
                let save_moves: Vec<_> = result
                    .moves
                    .iter()
                    .map(|movement| (movement.from.clone(), movement.to.clone()))
                    .collect();
                view.editing
                    .saves
                    .borrow_mut()
                    .complete_file_operation(ticket, &save_moves);
                view.finish_explorer_workspace_files(&result.moves, cx);
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
    moves: Vec<FileMove>,
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
            UndoStep::Explorer(ExplorerUndo::Renamed {
                source,
                target,
                stamp,
            }) => {
                outcome.moves.push(FileMove {
                    from: root.join(target.as_str()),
                    to: root.join(source.as_str()),
                    stamp: Some(stamp.clone()),
                });
            }
            #[cfg(windows)]
            UndoStep::Moved {
                path,
                original,
                stamp,
            } => {
                outcome.moves.push(FileMove {
                    from: root.join(path.as_str()),
                    to: display_path(original, root, folder),
                    stamp: Some(stamp.clone()),
                });
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
