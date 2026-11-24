use super::Workbench;
use crate::{
    context_basket::{model::Availability, registry as reg},
    documents::OpenOptions,
    list_widget::{
        options::ListOptions,
        text_input::{TextInput, TextInputColors, TextInputEvent},
        view::{default_row, DropPosition, ListView, ListViewEvent},
        widget::{ListEvent, ListWidget, RowHeight},
        StaticList,
    },
    lists::{EmptyList, ItemId, Row},
};
use gpui::{div, prelude::*, px, AnyElement, Context, Entity, Focusable, SharedString, Window};
use rove_base::{Cancellation, Line, Position, Resource, TextRange, Utf8Column, Version};
use std::{rc::Rc, sync::Arc};

pub(super) struct BasketPanel {
    list: Entity<ListView>,
    intro: Entity<TextInput>,
    note: Option<(u64, Entity<TextInput>)>,
    rendered_revision: Option<u64>,
}

fn entry_id(id: &ItemId) -> Option<u64> {
    id.0.strip_prefix("basket:")?.parse().ok()
}

fn note_input(
    view: &Workbench,
    cx: &mut Context<Workbench>,
    placeholder: &str,
    value: &str,
) -> Entity<TextInput> {
    let colors = TextInputColors {
        text: view.ui_color("input.foreground"),
        placeholder: view.ui_color("input.placeholderForeground"),
        caret: view.ui_color("input.foreground"),
        selection: view.ui_color("editor.selectionBackground"),
    };
    let hint = view.label(placeholder, &[]);
    cx.new(|cx| {
        let mut input = TextInput::new(cx, colors);
        input.set_placeholder(hint);
        input.set_text(value, cx);
        input
    })
}

impl Workbench {
    fn make_basket_panel(&mut self, cx: &mut Context<Self>) -> BasketPanel {
        let theme = self.theme.clone();
        let strings = self.find_strings.clone();
        let list = cx.new(|cx| {
            let widget = ListWidget::new(
                Arc::new(EmptyList),
                RowHeight::Fixed(70.0),
                ListOptions::default(),
            );
            let mut view = ListView::new(widget, theme, strings, cx);
            view.drag_and_drop = true;
            view
        });
        cx.subscribe(&list, |view, _, event: &ListEvent, cx| {
            if let ListEvent::Open { ids, .. } = event {
                if let Some(id) = ids.first().and_then(entry_id) {
                    view.open_basket_entry(id, cx);
                }
            }
        })
        .detach();
        cx.subscribe(&list, |view, _, event: &ListViewEvent, cx| {
            let ListViewEvent::Drop {
                items,
                target,
                position,
            } = event
            else {
                return;
            };
            let Some(id) = items.first().and_then(entry_id) else {
                return;
            };
            let target = target.as_ref().and_then(entry_id);
            let before = match position {
                DropPosition::Before | DropPosition::Over => target,
                DropPosition::After => target.and_then(|target| {
                    let items = view.basket.contents.items();
                    let index = items.iter().position(|item| item.id() == target)?;
                    items.get(index + 1).map(|item| item.id())
                }),
            };
            if view.basket.contents.reorder_before(id, before) {
                view.basket_changed(cx);
            }
        })
        .detach();
        let intro = note_input(
            self,
            cx,
            "contextBasket.introduction",
            &self.basket.contents.introduction,
        );
        cx.subscribe(&intro, |view, _, event: &TextInputEvent, cx| {
            if let TextInputEvent::Changed { text, .. } = event {
                view.basket.contents.introduction.clone_from(text);
                view.basket_changed(cx);
            }
        })
        .detach();
        BasketPanel {
            list,
            intro,
            note: None,
            rendered_revision: None,
        }
    }

    pub(super) fn render_basket(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.basket.loaded {
            return div()
                .px_3()
                .py_2()
                .child(self.label("contextBasket.loading", &[]))
                .into_any_element();
        }
        if self.basket.panel.is_none() {
            self.basket.panel = Some(self.make_basket_panel(cx));
        }
        if self
            .basket
            .panel
            .as_ref()
            .and_then(|panel| panel.rendered_revision)
            != Some(self.basket.revision)
        {
            self.update_basket_panel(cx);
        }
        let panel = self
            .basket
            .panel
            .as_ref()
            .expect("basket panel is initialized");
        if std::mem::take(&mut self.basket.focus_requested) {
            window.focus(&panel.intro.focus_handle(cx));
        }
        let has_items = !self.basket.contents.items().is_empty();
        let list = panel.list.clone();
        let intro = panel.intro.clone();
        let note = panel.note.as_ref().map(|(_, input)| input.clone());
        let mut toolbar = div().px_3().py_2().flex().flex_row().flex_wrap().gap_2();
        if has_items {
            for (command, label, primary) in [
                (reg::COPY_MARKDOWN, "contextBasket.copyMarkdown", true),
                (reg::COPY_REFERENCES, "contextBasket.copyReferences", false),
                (reg::CLEAR, "contextBasket.clear", false),
            ] {
                let bg = if primary {
                    self.ui_color("button.background")
                } else {
                    self.ui_color("button.secondaryBackground")
                };
                let fg = if primary {
                    self.ui_color("button.foreground")
                } else {
                    self.ui_color("button.secondaryForeground")
                };
                toolbar = toolbar.child(
                    div()
                        .id(command)
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(bg)
                        .text_color(fg)
                        .cursor_pointer()
                        .child(self.label(label, &[]))
                        .on_click(cx.listener(move |workbench, _, _, cx| {
                            workbench.basket_command(command, cx);
                        })),
                );
            }
        }
        let mut area = div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .text_color(self.ui_color("sideBar.foreground"));
        area = area.child(
            div()
                .m_2()
                .px_2()
                .py_1()
                .bg(self.ui_color("input.background"))
                .child(intro),
        );
        if let Some(note) = note {
            area = area.child(
                div()
                    .mx_2()
                    .px_2()
                    .py_1()
                    .bg(self.ui_color("input.background"))
                    .child(note),
            );
        }
        if !has_items {
            let icon = crate::shell::icons::codepoint("inbox").unwrap_or(' ');
            return area
                .child(
                    div()
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .font_family(crate::shell::icons::FONT_FAMILY)
                                .text_size(px(20.0))
                                .child(icon.to_string()),
                        )
                        .child(self.label("contextBasket.empty", &[])),
                )
                .into_any_element();
        }
        let lines = self
            .basket
            .contents
            .items()
            .iter()
            .filter(|item| item.availability == Availability::Current)
            .map(|item| item.lines.lines())
            .sum::<u64>();
        let chars = self
            .basket
            .previews
            .values()
            .map(|preview| preview.characters)
            .sum::<usize>();
        let stats = self.label(
            "contextBasket.stats",
            &[
                ("lines", lines.to_string()),
                ("chars", chars.to_string()),
                ("tokens", chars.div_ceil(4).to_string()),
            ],
        );
        area.child(div().flex_1().min_h_0().child(list))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_color(self.ui_color(if chars > 100_000 {
                        "editorWarning.foreground"
                    } else {
                        "descriptionForeground"
                    }))
                    .child(stats),
            )
            .child(toolbar)
            .into_any_element()
    }

    fn update_basket_panel(&mut self, cx: &mut Context<Self>) {
        let panel = self
            .basket
            .panel
            .as_mut()
            .expect("basket panel is initialized");
        panel.rendered_revision = Some(self.basket.revision);
        if panel.intro.read(cx).text() != self.basket.contents.introduction {
            let introduction = &self.basket.contents.introduction;
            panel
                .intro
                .update(cx, |input, cx| input.set_text(introduction, cx));
        }
        if panel.note.as_ref().is_some_and(|(id, _)| {
            !self
                .basket
                .contents
                .items()
                .iter()
                .any(|entry| entry.id() == *id)
        }) {
            panel.note = None;
        }
        let entries = self.basket.contents.items().to_vec();
        let previews = self.basket.previews.clone();
        let rows: Vec<_> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let summary = format!(
                    "{}. {}:{}–{}",
                    index + 1,
                    entry.path.as_str(),
                    entry.lines.first(),
                    entry.lines.last()
                );
                Row {
                    id: ItemId(format!("basket:{}", entry.id()).into()),
                    label: summary.into(),
                    detail: entry.symbol.as_deref().map(Arc::from),
                    depth: 0,
                    expandable: false,
                    expanded: false,
                    disabled: false,
                }
            })
            .collect();
        let weak = cx.entity().downgrade();
        let revision = Version(self.basket.revision);
        panel.list.update(cx, |list, cx| {
            list.widget
                .set_model(Arc::new(StaticList::new(rows, revision)));
            list.set_renderer(Rc::new(move |context, window, app| {
                let Some(entry) = entries.get(context.index) else {
                    return default_row(context, window, app);
                };
                let status = match entry.availability {
                    Availability::Current => None,
                    Availability::Stale => Some("contextBasket.stale"),
                    Availability::Missing => Some("contextBasket.missing"),
                };
                let id = entry.id();
                let mut body = div()
                    .size_full()
                    .px_2()
                    .py_1()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .child(SharedString::from(context.row.label.clone())),
                    );
                if let Some(status) = status {
                    body = body.child(
                        div().truncate().child(
                            context
                                .strings
                                .text(&rove_i18n::Message::key(status))
                                .unwrap_or_default(),
                        ),
                    );
                } else if let Some(symbol) = &entry.symbol {
                    body = body.child(div().truncate().child(symbol.clone()));
                }
                if let Some(preview) = previews.get(&id) {
                    for line in &preview.lines {
                        body = body.child(div().truncate().child(line.clone()));
                    }
                }
                if !entry.note.is_empty() {
                    body = body.child(div().truncate().child(entry.note.clone()));
                }
                if context.hovered || context.focused {
                    let weak_for_note = weak.clone();
                    let weak_for_delete = weak.clone();
                    let label = |key| {
                        context
                            .strings
                            .text(&rove_i18n::Message::key(key))
                            .unwrap_or_default()
                    };
                    body = body.child(
                        div()
                            .absolute()
                            .right_2()
                            .top_1()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id(("basket-edit-note", id))
                                    .cursor_pointer()
                                    .child(label("contextBasket.editNote"))
                                    .on_click(move |_, window, cx| {
                                        cx.stop_propagation();
                                        let _ = weak_for_note.update(cx, |view, cx| {
                                            view.edit_basket_note(id, window, cx)
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .id(("basket-remove", id))
                                    .cursor_pointer()
                                    .child(label("contextBasket.remove"))
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        let _ = weak_for_delete.update(cx, |view, cx| {
                                            if view.basket.contents.remove(id) {
                                                view.basket.pending_anchors.remove(&id);
                                                view.basket.previews.remove(&id);
                                                view.basket_changed(cx);
                                            }
                                        });
                                    }),
                            ),
                    );
                }
                body.into_any_element()
            }));
            cx.notify();
        });
    }

    fn edit_basket_note(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self
            .basket
            .contents
            .items()
            .iter()
            .find(|entry| entry.id() == id)
        else {
            return;
        };
        let input = note_input(self, cx, "contextBasket.note", &entry.note);
        cx.subscribe(&input, move |view, _, event: &TextInputEvent, cx| {
            if let TextInputEvent::Changed { text, .. } = event {
                if view.basket.contents.edit_note(id, text.clone()) {
                    view.basket_changed(cx);
                }
            }
        })
        .detach();
        self.basket
            .panel
            .as_mut()
            .expect("basket panel is initialized")
            .note = Some((id, input.clone()));
        window.focus(&input.focus_handle(cx));
        cx.notify();
    }

    fn open_basket_entry(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(entry) = self
            .basket
            .contents
            .items()
            .iter()
            .find(|entry| entry.id() == id)
        else {
            return;
        };
        if entry.availability == Availability::Missing {
            self.report_basket("contextBasket.missing", Arc::from([]), cx);
            return;
        }
        let start = Position {
            line: Line(entry.lines.first() - 1),
            column: Utf8Column(0),
        };
        let end = Position {
            line: Line(entry.lines.last()),
            column: Utf8Column(0),
        };
        let resource = Resource::Workspace(entry.path.clone());
        let stale = entry.availability == Availability::Stale;
        let options = OpenOptions {
            preview_tab: false,
            location: Some(start),
            selection: (!stale).then(|| TextRange::new(start, end).expect("valid basket range")),
            ..Default::default()
        };
        if self
            .open_documents()
            .open(resource.clone(), options, Cancellation::default())
            .is_ok()
            && stale
        {
            self.basket.reselect = Some((id, resource));
            self.report_basket("contextBasket.reselect", Arc::from([]), cx);
        }
        cx.notify();
    }
}
