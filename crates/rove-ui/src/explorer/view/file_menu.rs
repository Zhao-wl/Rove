//! Explorer context menu and tree drop routing.

use super::*;
use crate::list_widget::view::DropPosition;
use gpui::{anchored, deferred, Corner};

impl ExplorerView {
    pub(super) fn menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.menu_open {
            return None;
        }
        let anchor = *self
            .menu_anchor
            .get_or_insert_with(|| window.mouse_position());
        let theme = &*self.theme_for_name_edit;
        let color = |key| color_of(theme, key).unwrap_or_else(gpui::transparent_black);
        let hover = color("menu.selectionBackground");
        let mut menu = div()
            .id("explorer-context-menu")
            .min_w(px(200.0))
            .flex()
            .flex_col()
            .py(px(4.0))
            .bg(color("menu.background"))
            .text_color(color("menu.foreground"))
            .border_1()
            .border_color(color("menu.border"))
            .rounded(px(4.0))
            .shadow_md()
            .on_mouse_down_out(cx.listener(|view, _, _, cx| view.dismiss_file_menu(cx)));
        let has_items = !self.operation_targets(false, cx).is_empty();
        let writable = !self.operation_targets(true, cx).is_empty();
        let can_create = self.creation_parent(cx).is_some();
        for (id, expected) in registry::FILE_CONTEXT_MENU {
            use registry::MenuWhen::*;
            let enabled = match expected {
                Create => can_create,
                Item => has_items,
                Writable => writable,
                Recycle => writable && self.files_enable_trash && cfg!(windows),
                Permanent => writable && (!self.files_enable_trash || !cfg!(windows)),
            };
            if !enabled {
                continue;
            }
            let Some((_, title, _)) = registry::COMMANDS.iter().find(|(cmd, _, _)| cmd == id)
            else {
                continue;
            };
            let label = self.strings.text(&Message::key(*title)).unwrap_or_default();
            menu = menu.child(
                div()
                    .id(*id)
                    .px(px(10.0))
                    .py(px(4.0))
                    .cursor_pointer()
                    .hover(|style| style.bg(hover))
                    .child(SharedString::from(label))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.dismiss_file_menu(cx);
                        // The key listener updates Explorer again. Dispatch only
                        // after the menu's entity update has returned.
                        let id = *id;
                        window.defer(cx, move |window, cx| {
                            window.dispatch_action(Box::new(ExplorerAction(id)), cx);
                        });
                    })),
            );
        }
        Some(
            deferred(
                anchored()
                    .anchor(Corner::TopLeft)
                    .position(anchor)
                    .snap_to_window_with_margin(px(4.0))
                    .child(menu),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    pub(super) fn dismiss_file_menu(&mut self, cx: &mut Context<Self>) {
        self.menu_open = false;
        self.menu_anchor = None;
        cx.notify();
    }

    pub(super) fn open_file_menu(&mut self, cx: &mut Context<Self>) {
        if self.workspace.is_some() && self.render.borrow().name_edit.is_none() {
            self.menu_open = true;
            self.menu_anchor = None;
            cx.notify();
        }
    }

    pub(super) fn file_drop(
        &mut self,
        items: &[ItemId],
        target: Option<&ItemId>,
        position: DropPosition,
        cx: &mut Context<Self>,
    ) {
        if !self.drag_enabled
            || self.branch_filter.enabled
            || self.render.borrow().name_edit.is_some()
        {
            return;
        }
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        let writable = |path: &WorkspacePath| {
            workspace
                .rules(path)
                .ok()
                .is_some_and(|rules| !rules.excluded && rules.read_only.is_empty())
        };
        let list = self.list.read(cx);
        let row = target.and_then(|target| {
            let index = list.widget.tree_model()?.index_of(target)?;
            list.widget.row(index)
        });
        let parent = if let Some(row) = row {
            let path = if row.expandable && position == DropPosition::Over {
                chain_paths(&row.id.0, &row.label).pop()
            } else {
                row.id
                    .0
                    .rsplit_once('/')
                    .map(|(parent, _)| parent.to_owned())
            };
            path.and_then(|path| WorkspacePath::new(&path).ok())
        } else {
            None
        };
        if parent.as_ref().is_some_and(|parent| !writable(parent)) {
            return;
        }
        let focused = self.focused_row(cx).zip(self.focused_path(cx));
        let paths = items
            .iter()
            .filter_map(|id| {
                if let Some((row, path)) = &focused {
                    if row.id == *id {
                        return Some(path.clone());
                    }
                }
                WorkspacePath::new(&id.0).ok()
            })
            .collect::<Vec<_>>();
        if !paths.is_empty() && paths.iter().all(writable) {
            cx.emit(ExplorerEvent::MoveDropped { paths, parent });
        }
    }
}
