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
    div, prelude::*, px, App, Context, Entity, EventEmitter, Focusable, Hsla, KeyContext,
    SharedString, Subscription, Window,
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
    write_error: Option<String>,
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
                RowHeight::Fixed(SETTING_HEIGHT),
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
                    if view.query != *text {
                        view.query.clone_from(text);
                        view.history_cursor = None;
                        view.refresh(cx);
                    }
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
        let values = settings.changes().subscribe(Arc::new(move |_| {
            let _ = on_change.try_send(());
        }));
        let problems = settings.problem_changes().subscribe(Arc::new(move |_| {
            let _ = send.try_send(());
        }));
        cx.spawn(async move |this, cx| {
            let (_values, _problems) = (values, problems);
            while receive.recv().await.is_ok() {
                if this.update(cx, |view, cx| view.refresh(cx)).is_err() {
                    break;
                }
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
            write_error: None,
            _listeners: vec![search_events, category_events, result_events],
        };
        view.refresh(cx);
        view
    }

    pub fn focus_search(&self, window: &mut Window, cx: &App) {
        window.focus(&self.search.focus_handle(cx));
    }

    pub fn navigate(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        match command {
            reg::CLEAR_SEARCH => {
                self.search.update(cx, |input, cx| input.set_text("", cx));
            }
            reg::SEARCH | reg::SEARCH_FROM_ROW | reg::LEVEL_UP => self.focus_search(window, cx),
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
                    self.search
                        .update(cx, |input, cx| input.set_text(&query, cx));
                    self.history_cursor = Some(index);
                }
            }
            _ => {}
        }
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
        self.write_error = None;
        let write = self
            .settings
            .write(key, Scope::default(), value, Cancellation::default());
        cx.spawn(async move |this, cx| {
            if let Err(error) = write.await {
                let _ = this.update(cx, |view, cx| {
                    view.write_error = Some(error.to_string());
                    cx.notify();
                });
            }
        })
        .detach();
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
        let list_rows = rows
            .iter()
            .map(|entry| Row {
                id: ItemId(entry.row.key.as_str().into()),
                label: entry.row.title.as_str().into(),
                detail: Some(entry.row.key.as_str().into()),
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
                if entry.row.rove_default {
                    row = row.child(
                        div().text_size(px(11.)).child(
                            strings
                                .text(&Message::key("settings.roveDefault"))
                                .unwrap_or_default(),
                        ),
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
                                        view.write_value(key.clone(), Value::Bool(!checked), cx)
                                    });
                                }
                            });
                        }
                        row = row.child(control);
                    }
                    Some(Value::Object(_) | Value::Array(_)) => {
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
                    Some(Value::String(value)) => {
                        row = row.child(
                            div()
                                .text_size(px(12.))
                                .child(SharedString::from(value.to_string())),
                        );
                    }
                    Some(Value::Number(value)) => {
                        row = row.child(div().text_size(px(12.)).child(value.to_string()));
                    }
                    _ => {}
                }
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
            .child(body)
    }
}
