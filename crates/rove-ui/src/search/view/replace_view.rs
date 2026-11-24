//! #62 搜索替换控件：预览生成与写入代次互相独立，后台结果只更新自己的代次。
use super::*;
use crate::search::replace::{ApplyRequest, PlannedMatch, Preview, PreviewFile, PreviewRequest};
use rove_base::DocumentId;
use rove_config::{Key, Scope};
use std::collections::BTreeMap;

pub(crate) enum ReplacementEvent {
    Preview(PreviewRequest),
    Apply(ApplyRequest),
    Undo,
}

impl EventEmitter<ReplacementEvent> for SearchView {}

pub(super) type Rendered = Rc<RefCell<Option<Arc<Preview>>>>;

#[derive(Default)]
pub(super) struct ReplacementState {
    pub rendered: Rendered,
    pub(super) open: bool,
    preserve_case: bool,
    serial: u64,
    pending: Option<Cancellation>,
    plans: Option<Arc<Preview>>,
    applying: Option<Cancellation>,
    pub(crate) can_undo: bool,
    result: Option<Report>,
}

#[derive(Default)]
pub(crate) struct Report {
    pub applied: BTreeMap<WorkspacePath, Vec<PlannedMatch>>,
    pub remaining_lines: BTreeMap<WorkspacePath, RemainingLines>,
    pub skipped: Vec<WorkspacePath>,
    pub failed: Vec<WorkspacePath>,
    pub cancelled: bool,
    pub undone: bool,
}

pub(crate) struct RemainingLines {
    pub lines: Vec<rove_search::LineMatch>,
    pub open_result: Option<DocumentId>,
}

impl SearchView {
    pub(crate) fn replace_active(&self, cx: &App) -> bool {
        self.replace.open && !self.replace_input.read(cx).text().is_empty()
    }

    pub(crate) fn invalidate_replace_preview(&mut self) {
        if let Some(cancel) = self.replace.pending.take() {
            cancel.cancel();
        }
        self.replace.serial = self.replace.serial.wrapping_add(1);
        self.replace.plans = None;
        *self.replace.rendered.borrow_mut() = None;
    }

    pub(crate) fn schedule_replace_preview(&mut self, cx: &mut Context<Self>) {
        self.invalidate_replace_preview();
        if !self.replace.open
            || self.replace.applying.is_some()
            || !matches!(self.state, State::Done(_))
        {
            return;
        }
        let Some(query) = self
            .last_query
            .as_ref()
            .filter(|query| **query == self.current_query(cx))
        else {
            return;
        };
        let cancel = Cancellation::default();
        self.replace.pending = Some(cancel.clone());
        let request = PreviewRequest {
            serial: self.replace.serial,
            root: self.root.clone(),
            query: query.clone(),
            template: self.replace_input.read(cx).text().to_string(),
            preserve_case: self.replace.preserve_case,
            files: self.results.files().to_vec(),
            cancel,
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(70))
                .await;
            if request.cancel.check().is_ok() {
                let _ = this.update(cx, |_, cx| cx.emit(ReplacementEvent::Preview(request)));
            }
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn receive_replace_preview(
        &mut self,
        serial: u64,
        preview: Option<Preview>,
        cx: &mut Context<Self>,
    ) {
        if self.replace.serial != serial {
            return;
        }
        self.replace.pending = None;
        self.replace.plans = preview.map(Arc::new);
        let show = Key::<bool>::new(rove_search::settings::USE_REPLACE_PREVIEW)
            .read(&*self.settings, &Scope::default())
            .unwrap_or(true);
        *self.replace.rendered.borrow_mut() = if show {
            self.replace.plans.clone()
        } else {
            None
        };
        self.list.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub(crate) fn finish_replacement(
        &mut self,
        serial: u64,
        report: Report,
        can_undo: bool,
        cx: &mut Context<Self>,
    ) {
        self.replace.applying = None;
        self.replace.can_undo = can_undo;
        if serial == self.replace.serial {
            for (path, applied) in &report.applied {
                let Some(file) = self.results.file(path).cloned() else {
                    continue;
                };
                if let Some(remaining) = report.remaining_lines.get(path) {
                    self.results.remove(&Target::File(path.clone()));
                    if !remaining.lines.is_empty() {
                        self.results.add_with_origin(
                            rove_search::FileMatches {
                                path: path.clone(),
                                lines: remaining.lines.clone(),
                            },
                            remaining.open_result,
                        );
                    }
                    continue;
                }
                if applied.len() == file.count() {
                    self.results.remove(&Target::File(path.clone()));
                    continue;
                }
                let indices: Vec<_> = file
                    .lines
                    .iter()
                    .enumerate()
                    .flat_map(|(i, line)| {
                        line.ranges
                            .iter()
                            .enumerate()
                            .filter_map(move |(j, columns)| {
                                applied
                                    .iter()
                                    .any(|matched| {
                                        matched.line == line.line && matched.columns == *columns
                                    })
                                    .then_some((i, j))
                            })
                    })
                    .collect();
                for (line, range) in indices.into_iter().rev() {
                    self.results
                        .remove(&Target::Match(path.clone(), line, range));
                }
            }
        }
        self.replace.result = Some(report);
        self.refresh_rows(cx);
    }

    pub(super) fn replace_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match id {
            reg::REPLACE_IN_FILES => {
                self.replace.open = true;
                self.schedule_replace_preview(cx);
                window.focus(&self.replace_input.focus_handle(cx));
            }
            reg::CLOSE_REPLACE => {
                self.replace.open = false;
                self.invalidate_replace_preview();
                self.focus_query(window, cx);
                cx.notify();
            }
            reg::PRESERVE_CASE => {
                self.replace.preserve_case = !self.replace.preserve_case;
                self.schedule_replace_preview(cx);
            }
            reg::REPLACE_MATCH | reg::REPLACE_FILE | reg::REPLACE_FOLDER | reg::REPLACE_ALL => {
                self.start_replacement(id, cx);
            }
            reg::UNDO_REPLACE => {
                if self.replace.applying.is_none() && self.replace.can_undo {
                    cx.emit(ReplacementEvent::Undo);
                }
            }
            reg::CANCEL if self.replace.applying.is_some() => self.cancel_replacement(cx),
            _ => return false,
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn replacement_test_state(&self) -> (bool, bool, bool, bool) {
        (
            self.replace.open,
            self.replace.plans.is_some(),
            self.replace.rendered.borrow().is_some(),
            self.replace.can_undo,
        )
    }

    #[cfg(test)]
    pub(crate) fn replacement_test_writable_files(&self) -> Vec<WorkspacePath> {
        self.replace
            .plans
            .iter()
            .flat_map(|preview| preview.iter())
            .filter_map(|(path, file)| match file {
                PreviewFile::Planned(plan)
                    if !plan.read_only
                        && plan
                            .changes
                            .iter()
                            .any(|change| change.old.as_ref() != change.edit.text.as_ref()) =>
                {
                    Some(path.clone())
                }
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn replacement_test_comparison(
        &self,
        path: &WorkspacePath,
    ) -> Option<(String, String)> {
        let file = self.results.file(path)?;
        let line = file.lines.first()?;
        let columns = line.ranges.first()?;
        let rendered = self.replace.rendered.borrow();
        let (old, new) = rendered
            .as_ref()?
            .get(path)?
            .comparison(line.line, columns)?;
        Some((old.to_owned(), new.to_owned()))
    }

    #[cfg(all(test, windows))]
    pub(crate) fn replacement_test_editable(&self, window: &Window, cx: &App) -> bool {
        let mut keys = KeyContext::default();
        self.replace_context(&mut keys, window, cx);
        keys.contains(ck::EDITABLE_ITEM)
    }

    #[cfg(test)]
    pub(crate) fn replacement_test_report(&self) -> Option<&Report> {
        self.replace.result.as_ref()
    }

    pub(crate) fn start_undo(&mut self, cancel: Cancellation, cx: &mut Context<Self>) {
        self.replace.applying = Some(cancel);
        cx.notify();
    }

    pub(crate) fn start_replacement(&mut self, command: &str, cx: &mut Context<Self>) {
        if !self.replace.open
            || self.replace.applying.is_some()
            || !matches!(self.state, State::Done(_))
            || (command == reg::REPLACE_ALL
                && matches!(&self.state, State::Done(summary) if summary.limit_hit))
        {
            return;
        }
        let (Some(preview), Some(query)) = (&self.replace.plans, &self.last_query) else {
            return;
        };
        let target = self.focused_target(cx);
        let path = target.as_ref().map(target_path);
        if path.is_none() && command != reg::REPLACE_ALL {
            return;
        }
        let parent = |path: &WorkspacePath| {
            path.as_str()
                .rsplit_once('/')
                .map_or(String::new(), |(p, _)| format!("{p}/"))
        };
        let scope = path.map(parent);
        let selected = match target.as_ref() {
            Some(Target::Match(path, line, range)) if command == reg::REPLACE_MATCH => self
                .results
                .file(path)
                .and_then(|file| file.lines.get(*line))
                .and_then(|line| {
                    line.ranges
                        .get(*range)
                        .map(|range| (line.line, range.clone()))
                }),
            _ if command == reg::REPLACE_MATCH => return,
            _ => None,
        };
        if command == reg::REPLACE_MATCH && selected.is_none() {
            return;
        }
        let in_scope = |candidate: &WorkspacePath| match command {
            reg::REPLACE_ALL => true,
            reg::REPLACE_FOLDER => scope
                .as_ref()
                .is_some_and(|p| candidate.as_str().starts_with(p)),
            _ => Some(candidate) == path,
        };
        let mut plans = Vec::new();
        let mut skipped = Vec::new();
        let mut stale = Vec::new();
        for (path, file) in preview.iter().filter(|(path, _)| in_scope(path)) {
            match file {
                PreviewFile::Planned(plan) if !plan.read_only => {
                    let mut plan = plan.clone();
                    if let Some((line, columns)) = &selected {
                        let Some(index) = plan
                            .changes
                            .iter()
                            .position(|change| change.line == *line && change.columns == *columns)
                        else {
                            continue;
                        };
                        let choice = plan.changes.remove(index);
                        plan.remaining = std::mem::replace(&mut plan.changes, vec![choice]);
                    }
                    if !plan.changes.is_empty() {
                        plans.push(plan);
                    }
                }
                PreviewFile::Skipped(_) | PreviewFile::Planned(_) => skipped.push(path.clone()),
                PreviewFile::Changed => stale.push(path.clone()),
            }
        }
        let cancel = Cancellation::default();
        self.replace.applying = Some(cancel.clone());
        cx.emit(ReplacementEvent::Apply(ApplyRequest {
            serial: self.replace.serial,
            plans,
            query: query.clone(),
            skipped,
            stale,
            cancel,
        }));
        cx.notify();
    }

    pub(crate) fn cancel_replacement(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = &self.replace.applying {
            cancel.cancel();
        }
        cx.notify();
    }

    pub(super) fn replace_context(&self, keys: &mut KeyContext, window: &Window, cx: &App) {
        if self.replace_input.focus_handle(cx).is_focused(window) {
            keys.add(ck::REPLACE_INPUT_FOCUS);
            keys.add(ck::INPUT_BOX_FOCUS);
        }
        if let Some(target) = self.focused_target(cx) {
            if self.replace_folder_focus.is_focused(window) {
                keys.add(ck::FOLDER_FOCUS);
            }
            let path = target_path(&target);
            if matches!(
                self.replace.plans.as_ref().and_then(|p| p.get(path)),
                Some(PreviewFile::Planned(file)) if !file.read_only
            ) {
                keys.add(ck::EDITABLE_ITEM);
            }
        }
    }

    pub(super) fn replace_toggle(&self, cx: &Context<Self>) -> impl IntoElement {
        let icon = if self.replace.open {
            "chevron-down"
        } else {
            "chevron-right"
        };
        div()
            .id("search-replace-toggle")
            .cursor_pointer()
            .child(crate::shell::view::icon(
                icons::codepoint(icon).unwrap_or(' '),
                px(16.0),
            ))
            .on_click(cx.listener(|this, _, window, cx| {
                let command = if this.replace.open {
                    reg::CLOSE_REPLACE
                } else {
                    reg::REPLACE_IN_FILES
                };
                this.replace_command(command, window, cx);
            }))
    }

    pub(super) fn render_replacement(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.replace.open {
            return None;
        }
        let button = |id, label, enabled| {
            div()
                .id(id)
                .px(px(4.0))
                .text_size(px(11.0))
                .text_color(paint(if enabled {
                    self.colors.fg
                } else {
                    self.colors.description
                }))
                .when(enabled, |el| el.cursor_pointer())
                .child(label)
        };
        let (files, matches) = self.replace.plans.as_ref().map_or((0, 0), |plans| {
            plans
                .values()
                .fold((0, 0), |(files, count), plan| match plan {
                    PreviewFile::Planned(file) if !file.read_only => {
                        (files + 1, count + file.changes.len())
                    }
                    _ => (files, count),
                })
        });
        let limited = matches!(&self.state, State::Done(summary) if summary.limit_hit);
        let busy = self.replace.applying.is_some();
        let enabled = !limited && !busy && self.replace.plans.is_some();
        let summary = if limited {
            self.text("search.replace.limit", &[])
        } else if self.replace.pending.is_some() {
            self.text("search.replace.preparing", &[])
        } else if busy {
            self.text("search.replace.applying", &[])
        } else {
            self.text(
                "search.replace.summary",
                &[
                    ("files", files.to_string()),
                    ("matches", matches.to_string()),
                ],
            )
        };
        let summary = div().text_size(px(11.0)).child(summary);
        let replace_box = div()
            .flex()
            .items_center()
            .child(div().flex_1().min_w_0().child(self.input_box(
                &self.replace_input,
                false,
                window,
                cx,
            )))
            .child(self.toggle_button(
                "search-replace-case",
                "preserve-case",
                self.replace.preserve_case,
                self.tip("search.replace.case", reg::PRESERVE_CASE, cx),
                reg::PRESERVE_CASE,
                cx,
            ))
            .child(
                button(
                    "search-replace-all",
                    self.text("search.replace.all", &[]),
                    enabled,
                )
                .when(enabled, |el| {
                    el.on_click(
                        cx.listener(|v, _, _, cx| v.start_replacement(reg::REPLACE_ALL, cx)),
                    )
                }),
            );
        let selected = self.focused_target(cx);
        let context_enabled = !busy && selected.as_ref().is_some_and(|target| {
            matches!(self.replace.plans.as_ref().and_then(|plans| plans.get(target_path(target))),
                Some(PreviewFile::Planned(plan)) if !plan.read_only)
        });
        let actions = div().flex().items_center().when(context_enabled, |el| {
            el.when(matches!(selected, Some(Target::Match(..))), |el| {
                el.child(
                    button(
                        "search-replace-match",
                        self.text("search.replace.match", &[]),
                        true,
                    )
                    .on_click(
                        cx.listener(|v, _, _, cx| v.start_replacement(reg::REPLACE_MATCH, cx)),
                    ),
                )
            })
            .child(
                button(
                    "search-replace-file",
                    self.text("search.replace.file", &[]),
                    true,
                )
                .on_click(cx.listener(|v, _, _, cx| v.start_replacement(reg::REPLACE_FILE, cx))),
            )
            .child(
                button(
                    "search-replace-folder",
                    self.text("search.replace.folder", &[]),
                    true,
                )
                .track_focus(&self.replace_folder_focus)
                .tab_stop(true)
                .on_click(cx.listener(|v, _, _, cx| v.start_replacement(reg::REPLACE_FOLDER, cx))),
            )
        });
        let summary = div().flex().items_center().child(summary).when(busy, |el| {
            el.child(
                button(
                    "search-replace-cancel",
                    self.text("search.replace.cancel", &[]),
                    true,
                )
                .on_click(cx.listener(|v, _, _, cx| v.cancel_replacement(cx))),
            )
        });
        let mut result = div().flex().flex_col().text_size(px(11.0));
        if let Some(report) = &self.replace.result {
            if report.undone {
                result = result.child(self.text("search.replace.undone", &[]));
            }
            if !report.applied.is_empty() && !report.undone {
                let count: usize = report.applied.values().map(Vec::len).sum();
                result = result.child(self.text(
                    "search.replace.applied",
                    &[
                        ("files", report.applied.len().to_string()),
                        ("matches", count.to_string()),
                    ],
                ));
            }
            if !report.skipped.is_empty() {
                let first = report.skipped[0].clone();
                result = result.child(
                    button(
                        "search-replace-skipped",
                        self.text(
                            "search.replace.skipped",
                            &[("files", report.skipped.len().to_string())],
                        ),
                        true,
                    )
                    .on_click(cx.listener(move |v, _, _, cx| {
                        if let Some(index) = v.rows().file_row(&first) {
                            v.list.update(cx, |l, cx| {
                                l.widget.set_focus(Some(index));
                                l.widget.reveal(index);
                                cx.notify();
                            });
                        }
                    })),
                );
            }
            if !report.failed.is_empty() {
                let paths = report
                    .failed
                    .iter()
                    .map(WorkspacePath::as_str)
                    .collect::<Vec<_>>()
                    .join("、");
                result = result.child(self.text(
                    "search.replace.failure",
                    &[("files", report.failed.len().to_string()), ("paths", paths)],
                ));
            }
            if report.cancelled {
                result = result.child(self.text("search.replace.cancelled", &[]));
            }
        }
        if self.replace.can_undo && !busy {
            result = result.child(
                button(
                    "search-replace-undo",
                    self.text("search.replace.undo", &[]),
                    true,
                )
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ReplacementEvent::Undo))),
            );
        }
        Some(
            div()
                .flex()
                .flex_col()
                .child(replace_box)
                .child(summary)
                .child(actions)
                .child(result)
                .into_any_element(),
        )
    }
}
