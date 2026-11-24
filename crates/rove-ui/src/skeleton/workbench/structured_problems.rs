//! Problems view shared by the structured document diagnostics and the settings service.
use super::*;
use crate::list_widget::text_input::{TextInput, TextInputColors, TextInputEvent};
use crate::outline::structure as s;
use gpui::ClipboardItem;
use rove_base::{Line, Position, Utf8Column};

pub(super) const CLEAR_FILTER: &str = "problems.action.clearFilterText";
pub(super) const COPY: &str = "problems.action.copy";
pub(super) const FOCUS_FILTER: &str = "problems.action.focusFilter";
pub(super) const FOCUS_PROBLEMS: &str = "problems.action.focusProblemsFromFilter";
pub(super) const OPEN: &str = "problems.action.open";
pub(super) const OPEN_SIDE: &str = "problems.action.openToSide";
pub(super) const QUICK_FIXES: &str = "problems.action.showQuickFixes";

#[derive(Default)]
pub(super) struct Panel {
    filter: Option<Entity<TextInput>>,
    rows_focus: Option<FocusHandle>,
    selected: Option<usize>,
    truncate_messages: bool,
    settings_subscription: Option<rove_base::Subscription>,
    settings: Arc<[rove_config::SettingsProblem]>,
}

struct Row {
    resource: Resource,
    position: Position,
    file: String,
    message: String,
    warning: bool,
}

impl Workbench {
    pub(super) fn watch_settings_problems(
        &mut self,
        service: Arc<rove_config::SettingsService>,
        cx: &mut Context<Self>,
    ) {
        let (send, receive) = async_channel::bounded(1);
        self.problem_panel.settings_subscription =
            Some(service.problem_changes().subscribe(Arc::new(move |_| {
                let _ = send.try_send(());
            })));
        self.problem_panel.settings = service.problems();
        self.update_problem_count();
        cx.spawn(async move |weak, cx| {
            while receive.recv().await.is_ok() {
                if weak
                    .update(cx, |view, cx| {
                        // Callbacks can arrive out of order; the service owns the latest snapshot.
                        view.problem_panel.settings = service.problems();
                        view.update_problem_count();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn settings_error_counts(&self) -> (usize, usize) {
        let mut errors = 0;
        let mut warnings = 0;
        for p in self.problem_panel.settings.iter() {
            if p.is_error() {
                errors += 1;
            } else {
                warnings += 1;
            }
        }
        (errors, warnings)
    }

    fn prepare_problem_panel(
        &mut self,
        cx: &mut Context<Self>,
    ) -> (Entity<TextInput>, FocusHandle) {
        if self.problem_panel.filter.is_none() {
            let colors = TextInputColors {
                text: self.ui_color("input.foreground"),
                placeholder: self.ui_color("input.placeholderForeground"),
                caret: self.ui_color("input.foreground"),
                selection: self.ui_color("editor.selectionBackground"),
            };
            let placeholder = self.label("structure.problemsFilter", &[]);
            let input = cx.new(|cx| {
                let mut input = TextInput::new(cx, colors);
                input.set_placeholder(placeholder);
                input
            });
            cx.subscribe(&input, |view, _, event: &TextInputEvent, cx| {
                if matches!(event, TextInputEvent::Changed { .. }) {
                    view.problem_panel.selected = None;
                    cx.notify();
                }
            })
            .detach();
            self.problem_panel.filter = Some(input);
        }
        let rows_focus = self
            .problem_panel
            .rows_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let filter = self
            .problem_panel
            .filter
            .as_ref()
            .expect("initialized")
            .clone();
        (filter, rows_focus)
    }

    fn listed_problems(&self, cx: &App) -> Vec<Row> {
        let filter = self
            .problem_panel
            .filter
            .as_ref()
            .map_or_else(String::new, |input| input.read(cx).text().to_lowercase());
        let structure = self.structured.documents.values().flat_map(|doc| {
            doc.problems.iter().map(|p| Row {
                resource: doc.resource.clone(),
                position: p.range.start(),
                file: file_label(&doc.resource),
                message: self.label(super::structured_host::problem_key(p.kind), &[]),
                warning: false,
            })
        });
        let settings = self.problem_panel.settings.iter().map(|p| {
            let resource = Resource::File(p.path.as_ref().to_path_buf());
            let position = Position {
                line: Line(p.line.saturating_sub(1).into()),
                column: Utf8Column(p.column.saturating_sub(1).into()),
            };
            Row {
                file: file_label(&resource),
                resource,
                position,
                message: self.label(
                    if p.is_error() {
                        "structure.settingsProblem"
                    } else {
                        "structure.settingsWarning"
                    },
                    &[],
                ),
                warning: !p.is_error(),
            }
        });
        let mut rows: Vec<_> = structure
            .chain(settings)
            .filter(|item| {
                filter.is_empty()
                    || item.file.to_lowercase().contains(&filter)
                    || item.message.to_lowercase().contains(&filter)
            })
            .collect();
        let order = Key::<String>::new("problems.sortOrder")
            .read(&*self.settings, &Scope::default())
            .unwrap_or_default();
        rows.sort_by(|a, b| {
            let position = (&a.file, a.position).cmp(&(&b.file, b.position));
            if order == "position" {
                position
            } else {
                a.warning.cmp(&b.warning).then(position)
            }
        });
        rows
    }

    #[cfg(test)]
    pub(super) fn listed_problem_positions(&self, cx: &App) -> Vec<(Resource, Position)> {
        self.listed_problems(cx)
            .into_iter()
            .map(|row| (row.resource, row.position))
            .collect()
    }

    pub(super) fn render_structured_problems(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self.structured_bool("problems.visibility", true) {
            return shell::view::render_problems_empty(self);
        }
        let (filter, rows_focus) = self.prepare_problem_panel(cx);
        let data = self.listed_problems(cx);
        let auto_reveal = self.structured_bool("problems.autoReveal", true);
        let revealed =
            auto_reveal
                .then(|| self.focused_text())
                .flatten()
                .and_then(|(_, document, at)| {
                    data.iter()
                        .position(|row| {
                            row.resource == document.resource && row.position.line == at.line
                        })
                        .or_else(|| {
                            data.iter()
                                .position(|row| row.resource == document.resource)
                        })
                });
        let selected = self.problem_panel.selected.or(revealed).unwrap_or(0);

        let tree = Key::<String>::new("problems.defaultViewMode")
            .read(&*self.settings, &Scope::default())
            .is_ok_and(|mode| mode == "tree");
        let mut rows = div()
            .id("structure-problem-list")
            .track_focus(&rows_focus)
            .key_context("problemFocus")
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, _, cx| {
                let data = view.listed_problems(cx);
                let selected = view.problem_panel.selected.unwrap_or_default();
                let next = match event.keystroke.key.as_str() {
                    "up" => selected.saturating_sub(1),
                    "down" => (selected + 1).min(data.len().saturating_sub(1)),
                    _ => return,
                };
                view.problem_panel.selected = Some(next);
                cx.stop_propagation();
                cx.notify();
            }))
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let mut last_file = "";
        for (index, row) in data.iter().enumerate() {
            if tree && row.file != last_file {
                rows = rows.child(
                    div()
                        .px_2()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(row.file.clone()),
                );
                last_file = &row.file;
            }
            let place = if tree {
                format!("{}:{}", row.position.line.0 + 1, row.position.column.0 + 1)
            } else {
                format!(
                    "{}:{}:{}",
                    row.file,
                    row.position.line.0 + 1,
                    row.position.column.0 + 1
                )
            };
            let color = self.ui_color(if row.warning {
                "problemsWarningIcon.foreground"
            } else {
                "problemsErrorIcon.foreground"
            });
            let icon = if row.warning { "warning" } else { "error" };
            let active = self.ui_color("list.activeSelectionBackground");
            let resource = row.resource.clone();
            let position = row.position;
            let focus = rows_focus.clone();
            rows = rows.child(
                div()
                    .id(("structure-problem", index))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .flex()
                    .gap_2()
                    .when(index == selected, |view| view.bg(active))
                    .child(crate::outline::view::codicon(icon, px(16.0), Some(color)))
                    .child(place)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(self.problem_panel.truncate_messages, |message| {
                                message.truncate()
                            })
                            .child(row.message.clone()),
                    )
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.problem_panel.selected = Some(index);
                        window.focus(&focus);
                        view.open_structured_location(
                            resource.clone(),
                            position,
                            crate::documents::OpenTarget::Active,
                            cx,
                        );
                    })),
            );
        }
        let mut context = gpui::KeyContext::default();
        context.add("problemsVisibility");
        context.set("focusedView", "workbench.panel.markers.view");
        let rendered = div()
            .id("structure-problems")
            .size_full()
            .flex()
            .flex_col()
            .key_context(context)
            .child(
                div()
                    .key_context("problemsFilterFocus")
                    .bg(self.ui_color("input.background"))
                    .child(filter),
            )
            .child(if data.is_empty() {
                shell::view::render_problems_empty(self)
            } else {
                rows.into_any_element()
            });
        rendered.into_any_element()
    }

    pub(super) fn problem_panel_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        use shell::{catalog::PROBLEMS, layout::PanelLocation};
        match id {
            s::FOCUS_PANEL | s::FOCUS_VIEW | s::SHOW_PANEL => {
                self.shell.layout.active_panel_view = Some(PROBLEMS.into());
                shell::view::run_command(self, "workbench.action.focusPanel", window, cx);
                return true;
            }
            s::RESET_CONTAINER | s::RESET_VIEW => {
                let location = Key::<String>::new("workbench.panel.defaultLocation")
                    .read(&*self.settings, &Scope::default())
                    .unwrap_or_default();
                self.shell.layout.panel_location = Some(PanelLocation::from_setting(&location));
                self.shell.schedule_save(cx);
                cx.notify();
                return true;
            }
            s::REMOVE_VIEW => {
                if self.shell.active_panel_view() == PROBLEMS {
                    shell::view::run_command(self, "workbench.action.closePanel", window, cx);
                }
                return true;
            }
            s::TOGGLE_VIEW => {
                shell::view::run_command(self, "workbench.actions.view.toggleProblems", window, cx);
                return true;
            }
            s::WRAP_MESSAGE | s::TRUNCATE_MESSAGE => {
                self.problem_panel.truncate_messages = id == s::TRUNCATE_MESSAGE;
                cx.notify();
                return true;
            }
            CLEAR_FILTER
            | COPY
            | s::COPY_MESSAGE
            | FOCUS_FILTER
            | FOCUS_PROBLEMS
            | OPEN
            | OPEN_SIDE
            | QUICK_FIXES => {}
            _ => return false,
        }
        let (filter, focus) = self.prepare_problem_panel(cx);
        match id {
            CLEAR_FILTER => filter.update(cx, |input, cx| input.set_text("", cx)),
            FOCUS_FILTER => window.focus(&filter.focus_handle(cx)),
            FOCUS_PROBLEMS => window.focus(&focus),
            OPEN | OPEN_SIDE | COPY | s::COPY_MESSAGE => {
                let data = self.listed_problems(cx);
                let index = self.problem_panel.selected.unwrap_or(0);
                if let Some(row) = data.get(index) {
                    if id == COPY {
                        cx.write_to_clipboard(ClipboardItem::new_string(format!(
                            "{}:{}:{} {}",
                            row.file,
                            row.position.line.0 + 1,
                            row.position.column.0 + 1,
                            row.message,
                        )));
                    } else if id == s::COPY_MESSAGE {
                        cx.write_to_clipboard(ClipboardItem::new_string(row.message.clone()));
                    } else {
                        let target = if id == OPEN_SIDE {
                            crate::documents::OpenTarget::Beside
                        } else {
                            crate::documents::OpenTarget::Active
                        };
                        self.open_structured_location(
                            row.resource.clone(),
                            row.position,
                            target,
                            cx,
                        );
                    }
                }
            }
            _ => {}
        }
        true
    }
}

fn file_label(resource: &Resource) -> String {
    match resource {
        Resource::Workspace(path) => path.as_str().to_owned(),
        Resource::File(path) => path.display().to_string(),
        Resource::Virtual(name) => name.to_string(),
    }
}
