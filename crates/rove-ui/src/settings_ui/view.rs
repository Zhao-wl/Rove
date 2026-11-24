use super::{model, registry as reg};
use crate::list_widget::{
    options::ListOptions,
    text_input::{TextInput, TextInputColors, TextInputEvent},
    tree::{ChildCount, Children, TreeItem, TreeModel, TreeSource},
    view::{hsla, EmptyState, ListView},
    widget::{ListEvent, ListWidget, RowHeight},
    StaticList,
};
use crate::lists::{EmptyList, ItemId, Row};
use gpui::{
    div, prelude::*, px, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    KeyContext, SharedString, Subscription, Window,
};
use rove_base::{Cancellation, Position, Value, Version};
use rove_config::{Scope, Settings, SettingsService};
use rove_i18n::{Message, Strings};
use rove_theme::Theme;
use std::{collections::BTreeSet, sync::Arc};

const ALL: &str = "rove-settings:all";
const SETTING_HEIGHT: f32 = 82.0;
const CATEGORY_HEIGHT: f32 = 28.0;

struct EmptyCategories;

impl TreeSource for EmptyCategories {
    fn children(&self, _: Option<&ItemId>) -> Children {
        Children::Ready(Arc::from([]))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsViewEvent {
    OpenJson(Option<Position>),
}

mod controls;
use controls::RowEditor;

pub struct SettingsView {
    settings: Arc<SettingsService>,
    theme: Arc<dyn Theme>,
    strings: Arc<dyn Strings>,
    search: Entity<TextInput>,
    categories: Entity<ListView>,
    results: Entity<ListView>,
    query: String,
    search_history: Vec<String>,
    history_cursor: Option<usize>,
    category: Option<Arc<str>>,
    show_categories: bool,
    revision: Version,
    mutations: async_channel::Sender<rove_base::Task<()>>,
    write_error: Option<String>,
    language_overrides: Arc<BTreeSet<String>>,
    row_editor: Option<RowEditor>,
    menu_focus: FocusHandle,
    invalid_input: bool,
    _listeners: Vec<Subscription>,
}

impl EventEmitter<SettingsViewEvent> for SettingsView {}

impl SettingsView {
    pub fn new(
        settings: Arc<SettingsService>,
        theme: Arc<dyn Theme>,
        strings: Arc<dyn Strings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = ListOptions::read(settings.as_ref());
        let search = cx.new(|cx| {
            let color = |key| {
                theme
                    .color(key)
                    .ok()
                    .flatten()
                    .map(hsla)
                    .unwrap_or(gpui::white())
            };
            let mut input = TextInput::new(
                cx,
                TextInputColors {
                    text: color("settings.textInputForeground"),
                    placeholder: color("input.placeholderForeground"),
                    caret: color("settings.textInputForeground"),
                    selection: color("editor.selectionBackground"),
                },
            );
            input.set_placeholder(
                strings
                    .text(&Message::key("settings.searchPlaceholder"))
                    .unwrap_or_default(),
            );
            input
        });
        let categories = cx.new(|cx| {
            let tree = TreeModel::new(Arc::new(EmptyCategories));
            let list = ListWidget::tree(tree, RowHeight::Fixed(CATEGORY_HEIGHT), options.clone());
            ListView::new(list, theme.clone(), strings.clone(), cx)
        });
        let results = cx.new(|cx| {
            let list = ListWidget::new(
                Arc::new(EmptyList),
                RowHeight::PerRow(Arc::new(|row| {
                    let extra = usize::from(row.detail.as_deref() == Some("badges"));
                    SETTING_HEIGHT + 26.0 * extra as f32
                })),
                options,
            );
            let mut view = ListView::new(list, theme.clone(), strings.clone(), cx);
            view.set_empty_state(Some(EmptyState {
                message: Message::key("settings.noResults"),
                actions: Vec::new(),
            }));
            view
        });
        let search_events =
            cx.subscribe(&search, |view: &mut Self, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Changed { text, .. } = event {
                    view.query_changed(text, cx);
                }
            });
        let category_events =
            cx.subscribe(&categories, |view: &mut Self, _, event: &ListEvent, cx| {
                if let ListEvent::Open { ids, .. } = event {
                    view.category = ids
                        .first()
                        .map(|item| item.0.clone())
                        .filter(|id| id.as_ref() != ALL);
                    view.refresh(cx);
                }
            });
        let result_events = cx.subscribe(&results, |_: &mut Self, _, event: &ListEvent, cx| {
            if matches!(event, ListEvent::FocusChanged(_)) {
                cx.notify();
            }
        });
        let (send, receive) = async_channel::bounded(1);
        let on_change = send.clone();
        let _ = send.try_send(());
        let values = settings.changes().subscribe(Arc::new(move |_| {
            let _ = on_change.try_send(());
        }));
        let problems = settings.problem_changes().subscribe(Arc::new(move |_| {
            let _ = send.try_send(());
        }));
        let background = cx.background_executor().clone();
        let path = settings.settings_path().to_path_buf();
        cx.spawn(async move |this, cx| {
            let (_values, _problems) = (values, problems);
            while receive.recv().await.is_ok() {
                let path = path.clone();
                let overrides = background
                    .spawn(async move {
                        match std::fs::read_to_string(path) {
                            Ok(text) => model::language_overrides(&text),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                Some(BTreeSet::new())
                            }
                            Err(_) => None,
                        }
                    })
                    .await;
                if this
                    .update(cx, |view, cx| {
                        if let Some(overrides) = overrides {
                            view.language_overrides = Arc::new(overrides);
                        }
                        view.refresh(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        // The settings service performs synchronous file operations when its task is polled.
        // A single queue keeps edits in user order while the window remains responsive.
        let (mutations, pending) = async_channel::unbounded::<rove_base::Task<()>>();
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            while let Ok(write) = pending.recv().await {
                let error = background
                    .spawn(write)
                    .await
                    .err()
                    .map(|error| error.to_string());
                let _ = this.update(cx, |view, cx| {
                    view.write_error = error;
                    cx.notify();
                });
            }
        })
        .detach();

        let mut view = Self {
            settings,
            theme,
            strings,
            search,
            categories,
            results,
            query: String::new(),
            search_history: Vec::new(),
            history_cursor: None,
            category: None,
            show_categories: true,
            revision: Version(0),
            mutations,
            write_error: None,
            language_overrides: Arc::default(),
            row_editor: None,
            menu_focus: cx.focus_handle(),
            invalid_input: false,
            _listeners: vec![search_events, category_events, result_events],
        };
        view.refresh(cx);
        view
    }

    pub(crate) fn search_focus(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }

    pub fn focus_search(&self, window: &mut Window, cx: &App) {
        window.focus(&self.search_focus(cx));
    }

    pub fn navigate(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            reg::CLEAR_SEARCH => self.set_search_query("", cx),
            reg::SEARCH | reg::SEARCH_FROM_ROW => self.focus_search(window, cx),
            reg::LEVEL_UP => {
                if self.row_editor.take().is_some() {
                    self.focus_results(window, cx);
                } else {
                    self.focus_search(window, cx);
                }
                cx.notify();
            }
            reg::FOCUS_CONTROL | reg::SHOW_MENU => {
                if let Some(id) = self.results.read(cx).widget.focused_id() {
                    if command == reg::SHOW_MENU {
                        self.show_row_menu(id.0, false, window, cx);
                    } else {
                        self.activate_value(id.0, window, cx);
                    }
                }
            }
            reg::LIST_FROM_SEARCH | reg::LIST_FROM_SEARCH_ENTER | reg::FOCUS_LIST => {
                if !self.query.trim().is_empty() && self.search_history.last() != Some(&self.query)
                {
                    self.search_history.push(self.query.clone());
                }
                self.results.update(cx, |result, cx| {
                    if result.widget.focus().is_none() {
                        result.widget.focus_first();
                    }
                    window.focus(&result.focus_handle(cx));
                    cx.notify();
                });
                cx.notify();
            }
            reg::FOCUS_TOC if self.show_categories => {
                self.categories.update(cx, |category, cx| {
                    if category.widget.focus().is_none() {
                        category.widget.focus_first();
                    }
                    window.focus(&category.focus_handle(cx));
                    cx.notify();
                });
            }
            reg::PREVIOUS_SEARCH => {
                let index = self
                    .history_cursor
                    .unwrap_or(self.search_history.len())
                    .saturating_sub(1);
                if let Some(query) = self.search_history.get(index) {
                    let query = query.clone();
                    self.set_search_query(&query, cx);
                    self.history_cursor = Some(index);
                }
            }
            _ => {}
        }
    }

    fn query_changed(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.query != query {
            self.query.clear();
            self.query.push_str(query);
            self.history_cursor = None;
            self.refresh(cx);
        }
    }

    fn set_search_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_text(query, cx));
        self.query_changed(query, cx);
    }

    #[cfg(test)]
    pub(crate) fn set_query_for_test(&mut self, query: &str, cx: &mut Context<Self>) {
        self.set_search_query(query, cx);
    }

    #[cfg(test)]
    pub(crate) fn select_result_for_test(&self, key: &str, cx: &mut Context<Self>) -> bool {
        self.results.update(cx, |list, cx| {
            let found = list.widget.focus_id(&ItemId(key.into())).is_ok();
            cx.notify();
            found
        })
    }

    #[cfg(test)]
    pub(crate) fn edit_text_for_test(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(RowEditor::Input { input, .. }) = &self.row_editor else {
            panic!("settings input did not open");
        };
        input.update(cx, |input, cx| input.set_text(value, cx));
        self.commit_input(window, cx);
    }

    fn focus_results(&self, window: &mut Window, cx: &App) {
        window.focus(&self.results.focus_handle(cx));
    }

    fn color(&self, key: &str) -> Hsla {
        self.theme
            .color(key)
            .ok()
            .flatten()
            .map(hsla)
            .unwrap_or(gpui::rgba(0).into())
    }

    fn label(&self, key: &'static str) -> String {
        self.strings.text(&Message::key(key)).unwrap_or_default()
    }

    fn read_only(&self) -> bool {
        self.settings
            .problems()
            .iter()
            .any(|problem| problem.is_error())
    }

    pub fn write_value(&mut self, key: Arc<str>, value: Value, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let write = self
            .settings
            .write(key, Scope::default(), value, Cancellation::default());
        self.finish_mutation(write, cx);
    }

    pub(crate) fn toggle_checkbox(&mut self, key: Arc<str>, cx: &mut Context<Self>) {
        if self.read_only() {
            return;
        }
        let settings = self.settings.clone();
        self.finish_mutation(
            Box::pin(async move {
                let scope = Scope::default();
                let Value::Bool(value) = settings.read(&key, &scope)? else {
                    return Err(rove_base::Error::Unsupported);
                };
                settings
                    .write(key, scope, Value::Bool(!value), Cancellation::default())
                    .await
            }),
            cx,
        );
    }

    fn finish_mutation(&mut self, write: rove_base::Task<()>, cx: &mut Context<Self>) {
        self.write_error = None;
        self.mutations
            .try_send(write)
            .expect("settings write queue is alive");
        cx.notify();
    }

    pub fn open_key(&self, key: Arc<str>, language: bool, cx: &mut Context<Self>) {
        let path = self.settings.settings_path().to_path_buf();
        let locate = cx.background_executor().spawn(async move {
            let contents = std::fs::read_to_string(path).ok()?;
            let (line, column) = if language {
                model::language_json_location(&contents, &key)
            } else {
                model::json_location(&contents, &key)
            }?;
            model::source_position(&contents, line, column)
        });
        cx.spawn(async move |this, cx| {
            let location = locate.await;
            let _ = this.update(cx, |_, cx| cx.emit(SettingsViewEvent::OpenJson(location)));
        })
        .detach();
    }

    fn open_settings_file(&self, error: Option<(u32, u32)>, cx: &mut Context<Self>) {
        let Some((line, column)) = error else {
            cx.emit(SettingsViewEvent::OpenJson(None));
            return;
        };
        let path = self.settings.settings_path().to_path_buf();
        let locate = cx.background_executor().spawn(async move {
            let contents = std::fs::read_to_string(path).ok()?;
            model::source_position(&contents, line, column)
        });
        cx.spawn(async move |this, cx| {
            let location = locate.await;
            let _ = this.update(cx, |_, cx| cx.emit(SettingsViewEvent::OpenJson(location)));
        })
        .detach();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let matches = model::search(model::catalog_rows(), &self.query, |key| {
            self.settings
                .is_user_set(key, &Default::default())
                .unwrap_or_default()
        });
        self.show_categories = self.query.trim().is_empty()
            || reg::TOC_BEHAVIOR
                .read(self.settings.as_ref(), &Default::default())
                .is_ok_and(|behavior| behavior == "filter");
        let categories = matches
            .iter()
            .map(|result| &result.row.category)
            .collect::<BTreeSet<_>>();
        if self.category.as_ref().is_some_and(|current| {
            !self.show_categories
                || !categories
                    .iter()
                    .any(|category| category.as_str() == &**current)
        }) {
            self.category = None;
        }
        let mut items = vec![category(ALL, &self.label("settings.category.all"))];
        items.extend(categories.into_iter().map(|name| category(name, name)));
        self.categories.update(cx, |view, cx| {
            let _ = view
                .widget
                .update_tree(|tree| tree.set_children(None, items.into()));
            cx.notify();
        });
        let rows: Vec<_> = matches
            .into_iter()
            .filter(|entry| {
                self.category
                    .as_ref()
                    .is_none_or(|active| active.as_ref() == entry.row.category)
            })
            .collect();
        self.revision.0 += 1;
        let revision = self.revision;
        let overrides = self.language_overrides.clone();
        let list_rows: Vec<_> = rows
            .iter()
            .map(|entry| Row {
                id: ItemId(entry.row.key.as_str().into()),
                label: entry.row.title.as_str().into(),
                detail: (entry.row.rove_default.is_some()
                    || (entry.row.language_overridable && overrides.contains(&entry.row.key)))
                .then(|| Arc::<str>::from("badges")),
                depth: 0,
                expandable: false,
                expanded: false,
                disabled: false,
            })
            .collect();
        let theme = self.theme.clone();
        let strings = self.strings.clone();
        let settings = self.settings.clone();
        let editor = cx.entity().downgrade();
        let read_only = self.read_only();
        self.results.update(cx, |view, cx| {
            view.widget
                .set_model(Arc::new(StaticList::new(list_rows, revision)));
            view.set_renderer(std::rc::Rc::new(move |context, _, _| {
                let entry = &rows[context.index];
                let highlight = context.colors.highlight_fg;
                let color = |key| {
                    theme
                        .color(key)
                        .ok()
                        .flatten()
                        .map(hsla)
                        .unwrap_or(gpui::rgba(0).into())
                };
                let foreground = color("settings.headerForeground");
                let mut row = div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .px(px(10.))
                    .gap(px(3.))
                    .text_color(foreground)
                    .child(styled(&entry.row.title, &entry.title, highlight))
                    .child(div().text_size(px(11.)).child(styled(
                        &entry.row.key,
                        &entry.key,
                        highlight,
                    )))
                    .child(div().text_size(px(12.)).child(styled(
                        &entry.row.description,
                        &entry.description,
                        highlight,
                    )));
                if let Some(default) = &entry.row.rove_default {
                    let label = strings
                        .text(&Message::key("settings.roveDefault"))
                        .unwrap_or_default();
                    row = row.child(
                        div()
                            .text_size(px(11.))
                            .truncate()
                            .child(format!("{label} {default}")),
                    );
                }

                if entry.row.language_overridable && overrides.contains(&entry.row.key) {
                    let key = context.row.id.0.clone();
                    let editor = editor.clone();
                    row = row.child(
                        div()
                            .id(SharedString::from(format!(
                                "settings-language-{}",
                                entry.row.key
                            )))
                            .text_size(px(11.))
                            .cursor_pointer()
                            .child(
                                strings
                                    .text(&Message::key("settings.languageOverrides"))
                                    .unwrap_or_default(),
                            )
                            .on_click(move |_, _, cx| {
                                if let Some(view) = editor.upgrade() {
                                    view.update(cx, |view, cx| {
                                        view.open_key(key.clone(), true, cx)
                                    });
                                }
                            }),
                    );
                }
                if model::explicit_value(settings.as_ref(), entry.row) {
                    row = row
                        .border_l_2()
                        .border_color(color("settings.modifiedItemIndicator"));
                }
                let key = context.row.id.0.clone();
                match model::effective_value(settings.as_ref(), entry.row) {
                    Some(Value::Bool(checked)) => {
                        let foreground = color("settings.checkboxForeground");
                        let mut control = div()
                            .id(SharedString::from(format!(
                                "settings-checkbox-{}",
                                entry.row.key
                            )))
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_1()
                            .border_color(color("settings.checkboxBorder"))
                            .bg(color("settings.checkboxBackground"));
                        if checked {
                            control = control.child(crate::outline::view::codicon(
                                "check",
                                px(14.),
                                Some(foreground),
                            ));
                        }
                        if !read_only {
                            let editor = editor.clone();
                            control = control.cursor_pointer().on_click(move |_, _, cx| {
                                if let Some(view) = editor.upgrade() {
                                    view.update(cx, |view, cx| {
                                        view.toggle_checkbox(key.clone(), cx)
                                    });
                                }
                            });
                        }
                        row = row.child(control);
                    }
                    Some(Value::Object(_) | Value::Array(_) | Value::Null) => {
                        let editor = editor.clone();
                        row = row.child(
                            div()
                                .id(SharedString::from(format!(
                                    "settings-json-{}",
                                    entry.row.key
                                )))
                                .cursor_pointer()
                                .child(
                                    strings
                                        .text(&Message::key("settings.openJSON"))
                                        .unwrap_or_default(),
                                )
                                .on_click(move |_, _, cx| {
                                    if let Some(view) = editor.upgrade() {
                                        view.update(cx, |view, cx| {
                                            view.open_key(key.clone(), false, cx)
                                        });
                                    }
                                }),
                        );
                    }
                    Some(value @ (Value::String(_) | Value::Number(_))) => {
                        let text = match value {
                            Value::String(value) => value.to_string(),
                            Value::Number(value) => value.to_string(),
                            _ => unreachable!(),
                        };
                        let dropdown = !entry.row.choices.is_empty();
                        let label = if dropdown {
                            format!("{text}  ▾")
                        } else {
                            text
                        };
                        let mut control = div()
                            .id(SharedString::from(format!(
                                "settings-value-{}",
                                entry.row.key
                            )))
                            .px(px(6.))
                            .text_size(px(12.))
                            .border_1()
                            .border_color(color(if dropdown {
                                "settings.dropdownBorder"
                            } else {
                                "settings.textInputBorder"
                            }))
                            .child(label);
                        if !read_only {
                            let editor = editor.clone();
                            control = control.cursor_pointer().on_click(move |_, window, cx| {
                                if let Some(view) = editor.upgrade() {
                                    view.update(cx, |view, cx| {
                                        view.activate_value(key.clone(), window, cx)
                                    });
                                }
                            });
                        }
                        row = row.child(control);
                    }
                    _ => {}
                }
                let key = context.row.id.0.clone();
                let editor = editor.clone();
                row = row.relative().child(
                    div()
                        .id(SharedString::from(format!(
                            "settings-actions-{}",
                            entry.row.key
                        )))
                        .absolute()
                        .top(px(4.))
                        .right(px(8.))
                        .cursor_pointer()
                        .child(crate::outline::view::codicon(
                            "gear",
                            px(14.),
                            Some(foreground),
                        ))
                        .on_click(move |_, window, cx| {
                            if let Some(view) = editor.upgrade() {
                                view.update(cx, |view, cx| {
                                    view.show_row_menu(key.clone(), false, window, cx)
                                });
                            }
                        }),
                );
                row.into_any_element()
            }));
            cx.notify();
        });
        cx.notify();
    }
}

fn category(key: &str, label: &str) -> TreeItem {
    TreeItem {
        id: ItemId(key.into()),
        label: label.into(),
        detail: None,
        children: ChildCount::None,
    }
}

fn styled(text: &str, ranges: &[std::ops::Range<usize>], color: Option<Hsla>) -> gpui::StyledText {
    gpui::StyledText::new(SharedString::from(text.to_owned())).with_highlights(
        ranges
            .iter()
            .cloned()
            .map(|range| {
                (
                    range,
                    gpui::HighlightStyle {
                        color,
                        ..Default::default()
                    },
                )
            })
            .collect::<Vec<_>>(),
    )
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self
            .settings
            .problems()
            .iter()
            .find(|problem| problem.is_error())
            .map(|item| (item.line, item.column));
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .pb(px(10.))
            .border_b_1()
            .border_color(self.color("settings.headerBorder"))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .border_1()
                    .border_color(self.color("settings.textInputBorder"))
                    .bg(self.color("settings.textInputBackground"))
                    .key_context(reg::IN_SEARCH)
                    .child(self.search.clone()),
            )
            .child(
                div()
                    .id("settings-json")
                    .px(px(8.))
                    .cursor_pointer()
                    .child(self.label("settings.openJSON"))
                    .on_click(
                        cx.listener(move |view, _, _, cx| view.open_settings_file(error, cx)),
                    ),
            );
        let mut body = div().flex_1().min_h_0().flex().flex_row();
        if self.show_categories {
            body = body.child(
                div()
                    .w(px(208.))
                    .flex_shrink_0()
                    .h_full()
                    .border_r_1()
                    .border_color(self.color("settings.sashBorder"))
                    .key_context(reg::TOC_FOCUS)
                    .child(self.categories.clone()),
            );
        }
        let mut row_context = KeyContext::default();
        row_context.add(reg::ROW_FOCUS);
        if self.results.read(cx).widget.focus() == Some(0) {
            row_context.add(reg::FIRST_ROW_FOCUS);
        }
        body = body.child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .key_context(row_context)
                .child(self.results.clone()),
        );
        div()
            .key_context(reg::IN_EDITOR)
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .bg(self.color("editor.background"))
            .text_color(self.color("settings.headerForeground"))
            .child(header)
            .children(error.map(|(line, column)| {
                let message = Message {
                    key: Arc::from("settings.invalidFile"),
                    args: Arc::from([
                        (Arc::from("line"), Arc::from(line.to_string())),
                        (Arc::from("column"), Arc::from(column.to_string())),
                    ]),
                };
                div()
                    .text_color(self.color("errorForeground"))
                    .child(self.strings.text(&message).unwrap_or_default())
            }))
            .children(self.write_error.as_ref().map(|error| {
                let message = Message {
                    key: Arc::from("settings.writeFailed"),
                    args: Arc::from([(Arc::from("error"), Arc::from(error.as_str()))]),
                };
                div()
                    .text_color(self.color("errorForeground"))
                    .child(self.strings.text(&message).unwrap_or_default())
            }))
            .children(
                self.category
                    .as_deref()
                    .filter(|category| *category == "Rove")
                    .map(|_| {
                        div()
                            .text_size(px(12.))
                            .child(self.label("settings.category.roveDescription"))
                    }),
            )
            .children(self.row_editor_element(cx))
            .child(body)
    }
}
