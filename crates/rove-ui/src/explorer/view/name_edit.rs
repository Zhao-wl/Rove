//! Inline create/rename input over the Explorer tree. The placeholder is local
//! UI state; only the workspace watcher adds or renames filesystem rows.

use super::*;
use crate::list_widget::{
    text_input::{TextInput, TextInputColors},
    tree::{ChildCount, TreeItem},
};
use rove_workspace::{
    file_operations::{rename_selection, NameError, OperationError},
    EntryKind, Folder,
};

const DRAFT_ID: &str = "\0explorer.nameDraft";
const ERROR_ID: &str = "\0explorer.nameError";

#[derive(Clone)]
enum Operation {
    Create {
        parent: Option<WorkspacePath>,
        kind: EntryKind,
    },
    Rename {
        source: WorkspacePath,
    },
}

pub(super) struct NameEdit {
    operation: Operation,
    input: Entity<TextInput>,
    row: ItemId,
    slot_parent: Option<ItemId>,
    error: Option<&'static str>,
    busy: bool,
}

impl NameEdit {
    pub(super) fn row(
        &self,
        id: &ItemId,
        theme: &dyn Theme,
        strings: &dyn Strings,
    ) -> Option<gpui::AnyElement> {
        if *id == self.row {
            let border = if self.error.is_some() {
                "inputValidation.errorBorder"
            } else {
                "focusBorder"
            };
            return Some(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .px(px(3.0))
                    .border_1()
                    .border_color(color_of(theme, border).unwrap_or_else(gpui::transparent_black))
                    .bg(color_of(theme, "input.background").unwrap_or_else(gpui::transparent_black))
                    .child(self.input.clone())
                    .into_any_element(),
            );
        }
        if id.0.as_ref() == ERROR_ID {
            let label = strings.text(&Message::key(self.error?)).unwrap_or_default();
            return Some(
                div()
                    .w_full()
                    .px(px(3.0))
                    .bg(color_of(theme, "inputValidation.errorBackground")
                        .unwrap_or_else(gpui::transparent_black))
                    .text_color(
                        color_of(theme, "inputValidation.errorForeground")
                            .unwrap_or_else(gpui::white),
                    )
                    .child(SharedString::from(label))
                    .into_any_element(),
            );
        }
        None
    }
}

impl ExplorerView {
    pub(crate) fn creation_parent(&self, cx: &App) -> Option<Option<WorkspacePath>> {
        if self.branch_filter.enabled || self.render.borrow().name_edit.is_some() {
            return None;
        }
        let focused = self.focused_row(cx).zip(self.focused_path(cx));
        let target = match focused {
            Some((row, path)) if self.action_writable(Some(&path)) => {
                if row.expandable {
                    Some(path)
                } else {
                    path_parent(&path)
                }
            }
            Some(_) => return None,
            None => None,
        };
        self.action_writable(target.as_ref()).then_some(target)
    }

    #[cfg(windows)]
    pub(crate) fn set_cut(&mut self, paths: &[WorkspacePath], cx: &mut Context<Self>) {
        self.render.borrow_mut().cut_paths = paths
            .iter()
            .map(|path| rove_workspace::fold_key(path.as_str()))
            .collect();
        self.list.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub(crate) fn operation_targets(&self, require_writable: bool, cx: &App) -> Vec<WorkspacePath> {
        use rove_workspace::file_operations::path_after_rename;
        if self.branch_filter.enabled || self.render.borrow().name_edit.is_some() {
            return Vec::new();
        }
        let focus = self.focused_row(cx).zip(self.focused_path(cx));
        let mut paths: Vec<WorkspacePath> = self
            .selected(cx)
            .iter()
            .filter_map(|id| {
                if let Some((row, path)) = &focus {
                    if id.as_str() == row.id.0.as_ref() {
                        return Some(path.clone());
                    }
                }
                WorkspacePath::new(id).ok()
            })
            .collect();
        if paths.is_empty() {
            if let Some((_, path)) = focus {
                paths.push(path);
            }
        }
        if require_writable && paths.iter().any(|path| !self.action_writable(Some(path))) {
            return Vec::new();
        }
        paths.sort_by_key(|path| path.as_str().len());
        let mut unique = Vec::new();
        for path in paths {
            if !unique
                .iter()
                .any(|parent| path_after_rename(&path, parent, parent).is_some())
            {
                unique.push(path);
            }
        }
        unique
    }

    pub(crate) fn operations_folder(&self) -> Option<&Arc<Folder>> {
        self.operations_folder.as_ref()
    }

    pub(crate) fn set_operations_folder(
        &mut self,
        folder: Arc<Folder>,
        root: std::path::PathBuf,
        saves: Rc<RefCell<crate::save::controller::SaveController>>,
    ) {
        self.operations_folder = Some(folder);
        self.operation_saves = Some((root, saves));
    }

    pub(crate) fn name_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.operations_folder.is_none()
            || self.render.borrow().name_edit.is_some()
            || self.source.is_none()
            || self.branch_filter.enabled
        {
            return;
        }
        let focused = self.focused_row(cx).zip(self.focused_path(cx));
        let (operation, row, parent, initial, selection) = if id == registry::RENAME_FILE {
            let Some((row, source)) = focused else {
                return;
            };
            let kind = if row.expandable {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            let initial = source
                .as_str()
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned();
            let selection = rename_selection(&initial, kind);
            let parent = if row.expandable {
                path_parent(&WorkspacePath::new(&row.id.0).expect("workspace row"))
            } else {
                path_parent(&source)
            };
            if !self.action_writable(Some(&source)) {
                return;
            }
            (
                Operation::Rename { source },
                row.id,
                parent,
                initial,
                selection,
            )
        } else {
            let parent = focused.and_then(|(row, path)| {
                if row.expandable {
                    Some(path)
                } else {
                    path_parent(&path)
                }
            });
            if !self.action_writable(parent.as_ref()) {
                return;
            }
            let kind = if matches!(id, registry::NEW_FOLDER | registry::CREATE_FOLDER) {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            (
                Operation::Create {
                    parent: parent.clone(),
                    kind,
                },
                ItemId(DRAFT_ID.into()),
                parent,
                String::new(),
                0..0,
            )
        };
        let slot_parent = parent.as_ref().and_then(|path| {
            self.source
                .as_ref()?
                .reveal_plan(path)
                .map(|plan| plan.target)
        });
        if parent.is_some() && slot_parent.is_none() {
            return;
        }
        let theme = self.theme_for_name_edit.clone();
        let fg = color_of(&*theme, "input.foreground").unwrap_or_else(gpui::white);
        let colors = TextInputColors {
            text: fg,
            placeholder: color_of(&*theme, "input.placeholderForeground").unwrap_or(fg),
            caret: fg,
            selection: color_of(&*theme, "editor.selectionBackground")
                .unwrap_or_else(gpui::transparent_black),
        };
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx, colors);
            input.set_text(&initial, cx);
            input.select_range(selection, cx);
            input
        });
        let focus = input.focus_handle(cx);
        self.render.borrow_mut().name_edit = Some(NameEdit {
            operation,
            input,
            row,
            slot_parent,
            error: None,
            busy: false,
        });
        self.restore_name_slots(cx);
        window.focus(&focus);
        cx.notify();
    }

    fn action_writable(&self, path: Option<&WorkspacePath>) -> bool {
        path.is_none_or(|path| {
            self.workspace
                .as_ref()
                .and_then(|workspace| workspace.rules(path).ok())
                .is_some_and(|rules| !rules.excluded && rules.read_only.is_empty())
        })
    }

    pub(super) fn restore_name_slots(&mut self, cx: &mut Context<Self>) {
        let update = self.render.borrow().name_edit.as_ref().and_then(|edit| {
            (edit.row.0.as_ref() == DRAFT_ID || edit.error.is_some()).then_some((
                edit.slot_parent.clone(),
                Some(edit.row.clone()),
                edit.error.is_some(),
            ))
        });
        if let Some((parent, draft, error)) = update {
            self.update_name_slot(parent, draft, error, cx);
        }
        if let Some(path) = self.rename_or_create_reveal.clone() {
            if self.reveal(&path, true, cx) {
                self.rename_or_create_reveal = None;
            }
        }
    }

    fn update_name_slot(
        &mut self,
        parent: Option<ItemId>,
        draft: Option<ItemId>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.source.as_ref() else {
            return;
        };
        let parent_path = parent
            .as_ref()
            .and_then(|id| WorkspacePath::new(&id.0).ok());
        let mut items = source.items(parent_path.as_ref()).to_vec();
        if let Some(draft) = draft {
            let index = if draft.0.as_ref() == DRAFT_ID {
                items.insert(0, placeholder(DRAFT_ID));
                Some(0)
            } else {
                items.iter().position(|item| item.id == draft)
            };
            if let (Some(index), true) = (index, error) {
                items.insert(index + 1, placeholder(ERROR_ID));
            }
            self.list.update(cx, |list, cx| {
                if let Some(parent) = parent.as_ref() {
                    let _ = list.widget.expand_id(parent, true);
                }
                list.widget.update_tree(|tree| {
                    let _ = tree.set_children(parent.as_ref(), items.into());
                });
                if let Some(index) = list
                    .widget
                    .tree_model()
                    .and_then(|tree| tree.index_of(&draft))
                {
                    list.widget.reveal(index);
                }
                list.widget.take_events();
                cx.notify();
            });
        } else {
            self.list.update(cx, |list, cx| {
                list.widget.update_tree(|tree| {
                    let _ = tree.set_children(parent.as_ref(), items.into());
                });
                list.widget.take_events();
                cx.notify();
            });
        }
    }

    pub(super) fn name_input_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((input, busy, operation)) = self
            .render
            .borrow()
            .name_edit
            .as_ref()
            .map(|edit| (edit.input.clone(), edit.busy, edit.operation.clone()))
        else {
            return;
        };
        if !input.focus_handle(cx).contains_focused(window, cx)
            || input.read(cx).has_composition()
            || !matches!(event.keystroke.key.as_str(), "enter" | "escape")
        {
            return;
        }
        cx.stop_propagation();
        if busy {
            return;
        }
        if event.keystroke.key == "escape" {
            self.close_name_edit(cx);
            window.focus(&self.focus_handle(cx));
            return;
        }
        let Some(folder) = self.operations_folder.clone() else {
            return;
        };
        let name = input.read(cx).text().to_owned();
        let rename_from = match &operation {
            Operation::Rename { source } => Some(source.clone()),
            Operation::Create { .. } => None,
        };
        let lease = if let Some((root, saves)) = self.operation_saves.clone() {
            let (parent, source) = match &operation {
                Operation::Rename { source } => (path_parent(source), Some(source)),
                Operation::Create { parent, .. } => (parent.clone(), None),
            };
            let target = rove_workspace::file_operations::entry_path(
                parent.as_ref(),
                &name,
                source.is_none(),
            );
            let paths = source
                .into_iter()
                .map(|path| root.join(path.as_str()))
                .chain(target.ok().map(|path| root.join(path.as_str())))
                .collect();
            let ticket = saves.borrow_mut().reserve_file_operation(paths);
            let Some(ticket) = ticket else {
                if let Some(edit) = self.render.borrow_mut().name_edit.as_mut() {
                    edit.error = Some("explorer.name.locked");
                }
                self.restore_name_slots(cx);
                cx.notify();
                return;
            };
            Some((root, saves, ticket))
        } else {
            None
        };
        if let Some(edit) = self.render.borrow_mut().name_edit.as_mut() {
            edit.busy = true;
        }
        let worker = cx.background_executor().spawn(async move {
            match operation {
                Operation::Create { parent, kind } => {
                    let open = matches!(kind, EntryKind::File);
                    let created = folder.create_entry(parent.as_ref(), &name, kind)?;
                    let directories = created.directories.iter().map(|path| (path, true));
                    let file = open.then_some((&created.path, false));
                    // Capturing an undo stamp can fail after an external change.
                    // A successful creation still opens its result in that case.
                    let undo = directories
                        .chain(file)
                        .map(|(path, directory)| {
                            folder.entry_stamp(path).map(|stamp| ExplorerUndo::Created {
                                path: path.clone(),
                                stamp,
                                directory,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap_or_default();
                    Ok((created.path, open, undo))
                }
                Operation::Rename { source } => {
                    let target = folder.rename_entry(&source, &name)?;
                    let undo = if source == target {
                        Vec::new()
                    } else {
                        folder
                            .entry_stamp(&target)
                            .ok()
                            .map(|stamp| ExplorerUndo::Renamed {
                                source,
                                target: target.clone(),
                                stamp,
                            })
                            .into_iter()
                            .collect()
                    };
                    Ok((target, false, undo))
                }
            }
        });
        let window = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = worker.await;
            if let Some((root, saves, ticket)) = lease {
                let moved = rename_from
                    .zip(result.as_ref().ok())
                    .map(|(from, (to, _, _))| (root.join(from.as_str()), root.join(to.as_str())));
                saves
                    .borrow_mut()
                    .complete_file_operation(ticket, moved.as_slice());
            }
            let focus = this.update(cx, |view, cx| match result {
                Ok((path, open, undo)) => {
                    view.close_name_edit(cx);
                    view.rename_or_create_reveal = Some(path.clone());
                    view.restore_name_slots(cx);
                    if !undo.is_empty() {
                        cx.emit(ExplorerEvent::UndoRecorded(undo));
                    }
                    if open {
                        cx.emit(ExplorerEvent::Open {
                            path,
                            preserve_focus: false,
                            pinned: true,
                            side: false,
                        });
                    }
                    cx.notify();
                    (!open).then(|| view.focus_handle(cx))
                }
                Err(error) => {
                    eprintln!("explorer name operation: {error:?}");
                    if let Some(edit) = view.render.borrow_mut().name_edit.as_mut() {
                        edit.busy = false;
                        edit.error = Some(error_label(&error));
                    }
                    view.restore_name_slots(cx);
                    cx.notify();
                    None
                }
            });
            if let Ok(Some(focus)) = focus {
                let _ = window.update(cx, |_, window, _| window.focus(&focus));
            }
        })
        .detach();
    }

    fn close_name_edit(&mut self, cx: &mut Context<Self>) {
        let edit = self.render.borrow_mut().name_edit.take();
        if let Some(edit) = edit {
            if edit.row.0.as_ref() == DRAFT_ID || edit.error.is_some() {
                self.update_name_slot(edit.slot_parent, None, false, cx);
            }
            self.list.update(cx, |_, cx| cx.notify());
            cx.notify();
        }
    }
}

fn path_parent(path: &WorkspacePath) -> Option<WorkspacePath> {
    let (parent, _) = path.as_str().rsplit_once('/')?;
    WorkspacePath::new(parent).ok()
}

fn placeholder(id: &str) -> TreeItem {
    TreeItem {
        id: ItemId(id.into()),
        label: "".into(),
        detail: None,
        children: ChildCount::None,
    }
}

fn error_label(error: &OperationError) -> &'static str {
    match error {
        OperationError::Name(NameError::Empty) => "explorer.name.empty",
        OperationError::Name(NameError::ReservedDevice) => "explorer.name.device",
        OperationError::Name(NameError::ComponentTooLong) => "explorer.name.long",
        OperationError::Name(_) => "explorer.name.invalid",
        OperationError::ReadOnly(_) => "explorer.name.readonly",
        OperationError::Io { source, .. } => match source.kind() {
            std::io::ErrorKind::AlreadyExists => "explorer.name.exists",
            _ if matches!(source.raw_os_error(), Some(32 | 33)) => "explorer.name.locked",
            _ if source.raw_os_error() == Some(206) => "explorer.name.long",
            std::io::ErrorKind::PermissionDenied => "explorer.name.denied",
            _ => "explorer.name.other",
        },
        OperationError::RenameRecovery { .. } => "explorer.name.recovery",
    }
}
