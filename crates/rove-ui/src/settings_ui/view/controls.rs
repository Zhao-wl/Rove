use super::*;

#[derive(Clone, Copy)]
enum RowAction {
    Reset,
    CopyId,
    CopyJson,
    LanguageOverride,
    Choose(usize),
}

pub(super) enum RowEditor {
    Checkbox {
        key: Arc<str>,
    },
    Input {
        key: Arc<str>,
        initial: Value,
        input: Entity<TextInput>,
    },
    Menu {
        key: Arc<str>,
        choices: bool,
        selected: usize,
    },
}

impl SettingsView {
    pub(super) fn activate_value(
        &mut self,
        key: Arc<str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = model::catalog_rows().iter().find(|r| r.key == key.as_ref()) else {
            return;
        };
        let Some(value) = model::effective_value(&self.settings, row) else {
            return;
        };
        match value {
            Value::Object(_) | Value::Array(_) | Value::Null => self.open_key(key, false, cx),
            _ if self.read_only() => {}
            Value::Bool(_) => {
                self.row_editor = Some(RowEditor::Checkbox { key });
                window.focus(&self.menu_focus);
                cx.notify();
            }
            Value::String(_) if !row.choices.is_empty() => {
                self.show_row_menu(key, true, window, cx)
            }
            Value::String(_) | Value::Number(_) => {
                let number = matches!(value, Value::Number(_));
                let suffix = if number {
                    "settings.numberInputForeground"
                } else {
                    "settings.textInputForeground"
                };
                let colors = TextInputColors {
                    text: self.color(suffix),
                    placeholder: self.color("input.placeholderForeground"),
                    caret: self.color(suffix),
                    selection: self.color("editor.selectionBackground"),
                };
                let value_text = match &value {
                    Value::String(value) => value.to_string(),
                    Value::Number(value) => value.to_string(),
                    _ => unreachable!(),
                };
                let input = cx.new(|cx| {
                    let mut input = TextInput::new(cx, colors);
                    input.set_text(&value_text, cx);
                    input.select_all(cx);
                    input
                });
                window.focus(&input.focus_handle(cx));
                self.row_editor = Some(RowEditor::Input {
                    key,
                    initial: value,
                    input,
                });
                self.invalid_input = false;
                cx.notify();
            }
        }
    }

    pub(super) fn show_row_menu(
        &mut self,
        key: Arc<str>,
        choices: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.row_editor = Some(RowEditor::Menu {
            key,
            choices,
            selected: 0,
        });
        window.focus(&self.menu_focus);
        cx.notify();
    }

    pub(super) fn commit_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(RowEditor::Input {
            key,
            initial,
            input,
        }) = &self.row_editor
        else {
            return;
        };
        if self.read_only() {
            return;
        }
        let text = input.read(cx).text();
        let parsed = model::text_value(initial, text).filter(|value| model::valid_edit(key, value));
        if let Some(value) = parsed {
            let key = key.clone();
            self.row_editor = None;
            self.invalid_input = false;
            self.focus_results(window, cx);
            self.write_value(key, value, cx);
        } else {
            self.invalid_input = true;
        }
        cx.notify();
    }

    fn run_row_action(
        &mut self,
        key: Arc<str>,
        action: RowAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = model::catalog_rows().iter().find(|r| r.key == key.as_ref()) else {
            return;
        };
        self.row_editor = None;
        self.focus_results(window, cx);
        match action {
            RowAction::Reset if !self.read_only() => {
                let task = self
                    .settings
                    .remove(key, Scope::default(), Cancellation::default());
                self.finish_mutation(task, cx);
            }
            RowAction::CopyId => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(key.to_string()))
            }
            RowAction::CopyJson => {
                if let Some(json) = model::effective_value(&self.settings, row)
                    .and_then(|value| model::clipboard_json(&key, &value))
                {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(json));
                }
            }
            RowAction::LanguageOverride => self.open_key(key, true, cx),
            RowAction::Choose(index) => {
                if let Some(choice) = row.choices.get(index) {
                    self.write_value(key, Value::String(choice.value.as_str().into()), cx);
                }
            }
            RowAction::Reset => {}
        }
        cx.notify();
    }

    pub(super) fn row_editor_element(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        match self.row_editor.as_ref()? {
            RowEditor::Checkbox { key } => {
                let Value::Bool(value) = self.settings.read(key, &Scope::default()).ok()? else {
                    return None;
                };
                let key = key.clone();
                let click_key = key.clone();
                Some(
                    div()
                        .id("settings-active-checkbox")
                        .track_focus(&self.menu_focus)
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .p(px(8.))
                        .border_1()
                        .border_color(self.color("settings.focusedRowBorder"))
                        .bg(self.color("settings.focusedRowBackground"))
                        .child(
                            div()
                                .size(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .border_1()
                                .border_color(self.color("settings.checkboxBorder"))
                                .bg(self.color("settings.checkboxBackground"))
                                .children(value.then(|| {
                                    crate::outline::view::codicon(
                                        "check",
                                        px(14.),
                                        Some(self.color("settings.checkboxForeground")),
                                    )
                                })),
                        )
                        .child(SharedString::from(key.to_string()))
                        .on_click(
                            cx.listener(move |v, _, _, cx| {
                                v.toggle_checkbox(click_key.clone(), cx)
                            }),
                        )
                        .on_key_down(cx.listener(move |v, event: &gpui::KeyDownEvent, _, cx| {
                            if event.keystroke.key == "space" || event.keystroke.key == " " {
                                v.toggle_checkbox(key.clone(), cx);
                                cx.stop_propagation();
                            }
                        }))
                        .into_any_element(),
                )
            }
            RowEditor::Input {
                key,
                initial,
                input,
            } => {
                let (border, background) = if matches!(initial, Value::Number(_)) {
                    (
                        "settings.numberInputBorder",
                        "settings.numberInputBackground",
                    )
                } else {
                    ("settings.textInputBorder", "settings.textInputBackground")
                };
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .p(px(8.))
                        .child(SharedString::from(key.to_string()))
                        .child(
                            div()
                                .flex()
                                .border_1()
                                .border_color(self.color(border))
                                .bg(self.color(background))
                                .child(input.clone())
                                .on_key_down(cx.listener(
                                    |view, event: &gpui::KeyDownEvent, window, cx| {
                                        if event.keystroke.key == "enter" {
                                            view.commit_input(window, cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                )),
                        )
                        .child(
                            div()
                                .id("settings-apply-value")
                                .cursor_pointer()
                                .child(self.label("settings.apply"))
                                .on_click(
                                    cx.listener(|view, _, window, cx| {
                                        view.commit_input(window, cx)
                                    }),
                                ),
                        )
                        .children(self.invalid_input.then(|| {
                            div()
                                .text_color(self.color("errorForeground"))
                                .child(self.label("settings.invalidValue"))
                        }))
                        .into_any_element(),
                )
            }
            RowEditor::Menu {
                key,
                choices,
                selected,
            } => {
                let row = model::catalog_rows()
                    .iter()
                    .find(|row| row.key == key.as_ref())?;
                let items: Vec<_> = if *choices {
                    row.choices
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            (
                                format!("{} — {}", c.value, c.description),
                                RowAction::Choose(i),
                            )
                        })
                        .collect()
                } else {
                    [
                        ("settings.reset", RowAction::Reset),
                        ("settings.copyId", RowAction::CopyId),
                        ("settings.copyJson", RowAction::CopyJson),
                        ("settings.languageOverrides", RowAction::LanguageOverride),
                    ]
                    .into_iter()
                    .filter(|(_, action)| match action {
                        RowAction::Reset => !self.read_only(),
                        RowAction::LanguageOverride => {
                            row.language_overridable && self.language_overrides.contains(&row.key)
                        }
                        _ => true,
                    })
                    .map(|(label, action)| (self.label(label), action))
                    .collect()
                };
                let actions: Vec<_> = items.iter().map(|(_, action)| *action).collect();
                let selected = *selected;
                let selected_key = key.clone();
                let menu = div()
                    .id("settings-row-menu")
                    .track_focus(&self.menu_focus)
                    .flex()
                    .flex_col()
                    .p(px(8.))
                    .border_1()
                    .border_color(self.color("settings.dropdownListBorder"))
                    .bg(self.color("settings.dropdownBackground"))
                    .text_color(self.color("settings.dropdownForeground"))
                    .on_key_down(cx.listener(
                        move |view, event: &gpui::KeyDownEvent, window, cx| {
                            let Some(RowEditor::Menu { selected, .. }) = &mut view.row_editor
                            else {
                                return;
                            };
                            match event.keystroke.key.as_str() {
                                "up" => *selected = selected.saturating_sub(1),
                                "down" => {
                                    *selected = (*selected + 1).min(actions.len().saturating_sub(1))
                                }
                                "enter" => {
                                    if let Some(&action) = actions.get(*selected) {
                                        view.run_row_action(
                                            selected_key.clone(),
                                            action,
                                            window,
                                            cx,
                                        );
                                    }
                                }
                                _ => return,
                            }
                            cx.stop_propagation();
                            cx.notify();
                        },
                    ))
                    .children(
                        items
                            .into_iter()
                            .enumerate()
                            .map(|(index, (text, action))| {
                                let key = key.clone();
                                let mut button = div()
                                    .id(("settings-menu-option", index))
                                    .px(px(8.))
                                    .py(px(3.))
                                    .cursor_pointer()
                                    .child(text)
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.run_row_action(key.clone(), action, window, cx)
                                    }));
                                if index == selected {
                                    button = button.bg(self.color("settings.focusedRowBackground"));
                                }
                                button
                            }),
                    );
                Some(menu.into_any_element())
            }
        }
    }
}
