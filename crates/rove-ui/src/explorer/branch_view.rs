use super::{codicon, color_of, ExplorerView};
use crate::explorer::{
    branch_history::{Branch, Changes},
    branch_tree,
};
use gpui::{
    deferred, div, prelude::*, px, AnyElement, Context, FocusHandle, Hsla, SharedString, Window,
};
use rove_i18n::Message;
use rove_theme::Theme;
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Picker {
    Author,
    Commit,
}

pub(super) struct Filter {
    pub root: Option<PathBuf>,
    pub enabled: bool,
    pub changes: Changes,
    pub branch: Option<Arc<Branch>>,
    pub busy: bool,
    pub failed: bool,
    generation: u64,
    author: Option<String>,
    commit: Option<git2::Oid>,
    open: bool,
    picker: Option<Picker>,
    launcher: FocusHandle,
    menu: FocusHandle,
}

impl Filter {
    pub fn new(cx: &Context<ExplorerView>) -> Self {
        Self {
            root: None,
            enabled: false,
            changes: Changes::new(),
            branch: None,
            busy: false,
            failed: false,
            generation: 0,
            author: None,
            commit: None,
            open: false,
            picker: None,
            launcher: cx.focus_handle(),
            menu: cx.focus_handle(),
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.picker = None;
    }

    pub fn reset_root(&mut self, root: PathBuf) {
        if self.root.as_ref() != Some(&root) {
            self.generation = self.generation.wrapping_add(1);
            self.root = Some(root);
            self.enabled = false;
            self.changes.clear();
            self.branch = None;
            self.busy = false;
            self.failed = false;
            self.author = None;
            self.commit = None;
            self.close();
        }
    }
}

fn button_color(theme: &dyn Theme, name: &str, fallback: Hsla) -> Hsla {
    color_of(theme, name).unwrap_or(fallback)
}

impl ExplorerView {
    #[cfg(test)]
    pub(crate) fn branch_menu_is_open(&self) -> bool {
        self.branch_filter.open
    }

    pub(crate) fn set_branch_root(&mut self, root: PathBuf) {
        self.branch_filter.reset_root(root);
    }

    pub(super) fn branch_label(&self, key: &str) -> SharedString {
        SharedString::from(self.strings.text(&Message::key(key)).unwrap_or_default())
    }

    fn rebuild_branch_tree(&mut self, cx: &mut Context<Self>) {
        self.set_workspace(self.workspace.clone(), cx);
    }

    fn load_branch(&mut self, refresh: bool, cx: &mut Context<Self>) {
        let filter = &mut self.branch_filter;
        let Some(root) = filter.root.clone().filter(|_| filter.enabled) else {
            return;
        };
        filter.generation = filter.generation.wrapping_add(1);
        let generation = filter.generation;
        filter.busy = true;
        filter.failed = false;
        let cached = filter.branch.clone().filter(|_| !refresh);
        let author = filter.author.clone();
        let commit = filter.commit;
        let task = cx.background_executor().spawn(async move {
            let branch = match cached {
                Some(branch) => branch,
                None => Arc::new(Branch::load(&root)?),
            };
            let author =
                author.filter(|name| branch.commits.iter().any(|item| &item.author == name));
            let commit = commit.filter(|id| {
                branch.commits.iter().any(|item| {
                    item.id == *id && author.as_ref().is_none_or(|name| name == &item.author)
                })
            });
            let changes = branch.changes(author.as_deref(), commit)?;
            Ok::<_, git2::Error>((branch, author, commit, changes))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                let filter = &mut view.branch_filter;
                if filter.generation != generation || !filter.enabled {
                    return;
                }
                filter.busy = false;
                match result {
                    Ok((branch, author, commit, changes)) => {
                        filter.branch = Some(branch);
                        filter.author = author;
                        filter.commit = commit;
                        filter.changes = changes;
                    }
                    Err(_) => {
                        filter.failed = true;
                        filter.branch = None;
                        filter.changes.clear();
                    }
                }
                view.rebuild_branch_tree(cx);
            });
        })
        .detach();
        self.rebuild_branch_tree(cx);
    }

    fn toggle_branch(&mut self, cx: &mut Context<Self>) {
        let filter = &mut self.branch_filter;
        filter.enabled = !filter.enabled;
        if filter.enabled {
            self.load_branch(true, cx);
        } else {
            filter.generation = filter.generation.wrapping_add(1);
            filter.busy = false;
            filter.picker = None;
            self.rebuild_branch_tree(cx);
        }
    }

    pub(super) fn branch_summary(&self) -> Option<(SharedString, Option<SharedString>)> {
        let filter = &self.branch_filter;
        if !filter.enabled {
            return None;
        }
        if filter.busy {
            return Some((self.branch_label("explorer.loading"), None));
        }
        if filter.failed {
            return Some((self.branch_label("explorer.branch.unavailable"), None));
        }
        let branch = filter.branch.as_ref()?;
        let mut parts = vec![branch.name.clone()];
        if let Some(name) = &filter.author {
            parts.push(name.clone());
        }
        if let Some(id) = filter.commit {
            parts.push(id.to_string().chars().take(7).collect());
        }
        let count = format!(
            "{} {}",
            self.render.borrow().changes.len(),
            self.branch_label("explorer.branch.files")
        );
        Some((parts.join(" · ").into(), Some(count.into())))
    }

    pub(super) fn filtered_entries(&mut self) -> Option<crate::explorer::model::BranchEntries> {
        if !self.branch_filter.enabled {
            self.render.borrow_mut().changes.clear();
            return None;
        }
        let workspace = self.workspace.as_ref()?;
        let mut changes = self.branch_filter.changes.clone();
        let entries = if self.branch_filter.busy {
            Default::default()
        } else {
            branch_tree::entries(&mut changes, &**workspace)
        };
        self.render.borrow_mut().changes = if self.branch_filter.busy {
            Changes::new()
        } else {
            changes.clone()
        };
        Some(entries)
    }

    fn set_option(&mut self, picker: Picker, index: usize, cx: &mut Context<Self>) {
        let value = self.options(picker).get(index).cloned();
        let Some((choice, _)) = value else {
            return;
        };
        let filter = &mut self.branch_filter;
        match picker {
            Picker::Author => filter.author = choice,
            Picker::Commit => filter.commit = choice.and_then(|id| id.parse().ok()),
        }
        filter.picker = None;
        self.load_branch(false, cx);
    }

    fn options(&self, picker: Picker) -> Vec<(Option<String>, SharedString)> {
        let filter = &self.branch_filter;
        let label = match picker {
            Picker::Author => "explorer.branch.allAuthors",
            Picker::Commit => "explorer.branch.whole",
        };
        let mut options = vec![(None, self.branch_label(label))];
        if let Some(branch) = &filter.branch {
            match picker {
                Picker::Author => {
                    let names = branch
                        .commits
                        .iter()
                        .map(|commit| commit.author.as_str())
                        .collect::<BTreeSet<_>>();
                    options.extend(
                        names
                            .into_iter()
                            .map(|name| (Some(name.to_owned()), name.to_owned().into())),
                    );
                }
                Picker::Commit => {
                    options.extend(
                        branch
                            .commits
                            .iter()
                            .filter(|commit| {
                                filter
                                    .author
                                    .as_ref()
                                    .is_none_or(|name| name == &commit.author)
                            })
                            .map(|commit| {
                                let id = commit.id.to_string();
                                let label = format!("{} · {}", &id[..7], commit.subject);
                                (Some(id), label.into())
                            }),
                    );
                }
            }
        }
        options
    }

    pub(crate) fn branch_toolbar(
        &mut self,
        theme: &dyn Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let filter = &self.branch_filter;
        let open = filter.open;
        let color = button_color(theme, "sideBar.foreground", gpui::white());
        let hover = button_color(theme, "toolbar.hoverBackground", gpui::transparent_black());
        let active = button_color(theme, "toolbar.activeBackground", hover);
        let launcher = div()
            .id("explorer-branch-filter")
            .debug_selector(|| "explorer-branch-filter".into())
            .track_focus(&filter.launcher)
            .key_context(crate::keys::MODAL_CONTEXT)
            .cursor_pointer()
            .size(px(23.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .hover(|s| s.bg(hover))
            .when(open || filter.enabled, |d| d.bg(active))
            .child(codicon("git-branch", px(16.0), Some(color)))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_branch_menu(window, cx)))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.toggle_branch_menu(window, cx);
                }
            }));
        let menu = open.then(|| self.branch_menu(theme, window, cx));
        div()
            .relative()
            .child(launcher)
            .children(menu)
            .into_any_element()
    }

    fn toggle_branch_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = &mut self.branch_filter;
        if filter.open {
            filter.close();
            window.focus(&filter.launcher);
        } else {
            filter.open = true;
            window.focus(&filter.menu);
            if filter.enabled && !filter.busy {
                self.load_branch(true, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn dismiss_branch_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.branch_filter.open {
            return false;
        }
        self.branch_filter.close();
        window.focus(&self.branch_filter.launcher);
        cx.notify();
        true
    }

    fn branch_menu(&self, theme: &dyn Theme, _: &mut Window, cx: &Context<Self>) -> AnyElement {
        let filter = &self.branch_filter;
        let foreground = button_color(theme, "menu.foreground", gpui::white());
        let muted = button_color(theme, "descriptionForeground", foreground);
        let background = button_color(theme, "menu.background", gpui::black());
        let border = button_color(theme, "menu.border", gpui::transparent_black());
        let hover = button_color(theme, "menu.selectionBackground", gpui::transparent_black());
        let enabled = filter.enabled;
        let interactive = enabled && !filter.busy && filter.branch.is_some();
        let toggle = div()
            .id("explorer-branch-toggle")
            .debug_selector(|| "explorer-branch-toggle".into())
            .flex()
            .items_center()
            .gap(px(6.0))
            .p(px(4.0))
            .cursor_pointer()
            .hover(|s| s.bg(hover))
            .child(codicon(
                if enabled { "check" } else { "blank" },
                px(16.0),
                Some(foreground),
            ))
            .child(self.branch_label("explorer.branch.only"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_branch(cx)));
        let mut menu = div()
            .id("explorer-branch-menu")
            .w_full()
            .occlude()
            .track_focus(&filter.menu)
            .key_context(crate::keys::MODAL_CONTEXT)
            .p(px(6.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(border)
            .bg(background)
            .text_color(foreground)
            .text_size(px(12.0))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.dismiss_branch_menu(window, cx);
                } else if event.keystroke.key == "space" {
                    cx.stop_propagation();
                    this.toggle_branch(cx);
                }
            }))
            .child(toggle);
        for (picker, key, button_id) in [
            (
                Picker::Author,
                "explorer.branch.author",
                "explorer-branch-author",
            ),
            (
                Picker::Commit,
                "explorer.branch.commit",
                "explorer-branch-commit",
            ),
        ] {
            let options = self.options(picker);
            let selected = match picker {
                Picker::Author => filter.author.clone(),
                Picker::Commit => filter.commit.map(|id| id.to_string()),
            };
            let value = options
                .iter()
                .find(|(id, _)| id == &selected)
                .unwrap_or(&options[0])
                .1
                .clone();
            let field = div()
                .id(button_id)
                .debug_selector(move || button_id.into())
                .flex()
                .justify_between()
                .items_center()
                .p(px(4.0))
                .border_1()
                .border_color(border)
                .rounded(px(3.0))
                .when(!interactive, |d| d.text_color(muted))
                .child(div().min_w_0().truncate().child(value))
                .child(codicon(
                    "chevron-down",
                    px(14.0),
                    Some(if interactive { foreground } else { muted }),
                ))
                .when(interactive, |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let old = this.branch_filter.picker;
                            this.branch_filter.picker = (old != Some(picker)).then_some(picker);
                            cx.notify();
                        }))
                });
            menu = menu.child(div().mt(px(5.0)).child(self.branch_label(key)).child(field));
            if filter.picker == Some(picker) && interactive {
                menu = menu.child(
                    div()
                        .id(("branch-options", picker as usize))
                        .max_h(px(144.0))
                        .overflow_y_scroll()
                        .children(options.into_iter().enumerate().map(|(index, (_, label))| {
                            div()
                                .id(("branch-option", index))
                                .debug_selector(move || format!("explorer-branch-option-{index}"))
                                .px(px(4.0))
                                .py(px(2.0))
                                .truncate()
                                .cursor_pointer()
                                .hover(|s| s.bg(hover))
                                .child(label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_option(picker, index, cx)
                                }))
                        })),
                );
            }
        }
        let source = if filter.commit.is_some() {
            "explorer.branch.parent"
        } else {
            "explorer.branch.base"
        };
        let clear = div()
            .id("explorer-branch-clear")
            .px(px(4.0))
            .child(self.branch_label("explorer.branch.clear"))
            .when(
                interactive && (filter.author.is_some() || filter.commit.is_some()),
                |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.branch_filter.author = None;
                            this.branch_filter.commit = None;
                            this.branch_filter.picker = None;
                            this.load_branch(false, cx);
                        }))
                },
            );
        deferred(
            div()
                .id("explorer-branch-popup")
                .absolute()
                .top_0()
                .right_0()
                .w(px(258.0))
                .pt(px(27.0))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.branch_filter.close();
                    cx.notify();
                }))
                .child(
                    menu.child(
                        div()
                            .mt(px(5.0))
                            .flex()
                            .justify_between()
                            .child(self.branch_label(source))
                            .child(clear),
                    ),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
}
