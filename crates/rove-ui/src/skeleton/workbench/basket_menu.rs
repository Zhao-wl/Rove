use super::{
    basket_host::BasketCapture, markdown_host::MarkdownSourceSelection, Context, Workbench,
};
use crate::{
    code_view::Selection,
    context_basket::{capture::SourceRange, source},
    lists::ItemId,
};
use gpui::{anchored, deferred, div, prelude::*, px, AnyElement, Corner, Pixels, Point, Window};
use rove_base::{DocumentId, Resource, Version};
use std::sync::Arc;

struct EditorSelection {
    group: crate::documents::EditorGroupId,
    secondary: bool,
    document: DocumentId,
    version: Version,
    selections: Vec<Selection>,
}

enum MenuAction {
    Editor(EditorSelection),
    Sources(Vec<BasketCapture>, Option<(DocumentId, Version)>),
}

pub(super) struct BasketMenu {
    position: Option<Point<Pixels>>,
    options: Vec<(&'static str, MenuAction)>,
}

impl Workbench {
    fn show_basket_menu(
        &mut self,
        options: Vec<(&'static str, MenuAction)>,
        position: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        if !options.is_empty() {
            self.basket.menu = Some(BasketMenu { position, options });
            cx.notify();
        }
    }

    pub(super) fn show_basket_editor_menu(
        &mut self,
        group: crate::documents::EditorGroupId,
        secondary: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.shown_document(group, secondary) else {
            return;
        };
        let Some(editor) = self.group_code(group, secondary) else {
            return;
        };
        let editor = editor.read(cx);
        let Some(version) = editor.text().map(|text| text.version()) else {
            return;
        };
        let selection = EditorSelection {
            group,
            secondary,
            document: document.id,
            version,
            selections: editor.selections(),
        };
        self.show_basket_menu(
            vec![(
                "command.rove.contextBasket.add",
                MenuAction::Editor(selection),
            )],
            None,
            cx,
        );
    }

    pub(super) fn show_basket_sources_menu(
        &mut self,
        primary: Vec<BasketCapture>,
        whole: Vec<BasketCapture>,
        cx: &mut Context<Self>,
    ) {
        let mut options = Vec::new();
        if !primary.is_empty() {
            options.push((
                "command.rove.contextBasket.add",
                MenuAction::Sources(primary, None),
            ));
        }
        if !whole.is_empty() {
            options.push((
                "contextBasket.addWholeFile",
                MenuAction::Sources(whole, None),
            ));
        }
        self.show_basket_menu(options, None, cx);
    }

    pub(super) fn show_basket_markdown_menu(
        &mut self,
        document: DocumentId,
        selection: &MarkdownSourceSelection,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.open_document(document) else {
            return;
        };
        let Resource::Workspace(path) = &doc.resource else {
            return;
        };
        let Some(lines) = crate::context_basket::model::LineRange::new(
            selection.lines.start().0 + 1,
            selection.lines.end().0,
        ) else {
            self.report_basket("contextBasket.tooLong", Arc::from([]), cx);
            return;
        };
        let capture = BasketCapture {
            path: path.clone(),
            range: SourceRange::Selected(lines),
            symbol: None,
        };
        self.show_basket_menu(
            vec![(
                "command.rove.contextBasket.add",
                MenuAction::Sources(vec![capture], Some((document, selection.version))),
            )],
            Some(selection.anchor),
            cx,
        );
    }

    pub(super) fn show_basket_result_menu(
        &mut self,
        targets: &[(rove_base::WorkspacePath, Option<u64>)],
        offer_whole_files: bool,
        cx: &mut Context<Self>,
    ) {
        let context = crate::context_basket::registry::CONTEXT_LINES
            .read(self.settings.as_ref(), &rove_config::Scope::default())
            .ok()
            .filter(|value| value.is_finite())
            .unwrap_or(3.0)
            .max(0.0) as u64;
        let primary = targets
            .iter()
            .filter_map(|(path, line)| {
                Some(BasketCapture {
                    path: path.clone(),
                    range: SourceRange::Around {
                        line: (*line)?,
                        context,
                    },
                    symbol: None,
                })
            })
            .collect();
        let whole = if offer_whole_files {
            targets
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .map(|path| BasketCapture {
                    path,
                    range: SourceRange::WholeFile,
                    symbol: None,
                })
                .collect()
        } else {
            Vec::new()
        };
        self.show_basket_sources_menu(primary, whole, cx);
    }

    pub(super) fn show_basket_outline_menu(
        &mut self,
        document: DocumentId,
        ids: &[ItemId],
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.open_document(document) else {
            return;
        };
        let Resource::Workspace(path) = &doc.resource else {
            return;
        };
        let Some(symbols) = self
            .outline
            .cached(document)
            .filter(|symbols| symbols.version() == doc.text.version())
        else {
            return;
        };
        let targets = ids
            .iter()
            .filter_map(|key| {
                let node = symbols.node(symbols.find_key(&key.0)?)?;
                let range =
                    source::selection_lines(node.symbol.range.start(), node.symbol.range.end())?;
                Some(BasketCapture {
                    path: path.clone(),
                    range: SourceRange::Selected(range),
                    symbol: Some(node.symbol.name.to_string()),
                })
            })
            .collect::<Vec<_>>();
        if !targets.is_empty() {
            self.show_basket_menu(
                vec![(
                    "command.rove.contextBasket.add",
                    MenuAction::Sources(targets, Some((document, doc.text.version()))),
                )],
                None,
                cx,
            );
        }
    }

    pub(super) fn dismiss_basket_menu(&mut self) -> bool {
        self.basket.menu.take().is_some()
    }

    pub(super) fn render_basket_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.basket.menu.as_mut()?;
        let at = *menu.position.get_or_insert_with(|| window.mouse_position());
        let keys: Vec<_> = menu.options.iter().map(|(key, _)| *key).collect();
        let hover = self.ui_color("list.hoverBackground");
        let mut card = div()
            .id("context-basket-source-menu")
            .flex()
            .flex_col()
            .p_1()
            .bg(self.ui_color("menu.background"))
            .text_color(self.ui_color("menu.foreground"))
            .border_1()
            .border_color(self.ui_color("menu.border"))
            .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                if view.dismiss_basket_menu() {
                    cx.notify();
                }
            }));
        for (index, key) in keys.into_iter().enumerate() {
            card = card.child(
                div()
                    .id(("context-basket-source-action", index))
                    .debug_selector(move || format!("context-basket-source-action-{index}"))
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|style| style.bg(hover))
                    .child(self.label(key, &[]))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        let action = view
                            .basket
                            .menu
                            .take()
                            .and_then(|menu| menu.options.into_iter().nth(index));
                        match action.map(|(_, action)| action) {
                            Some(MenuAction::Editor(selection)) => {
                                let current_doc = view
                                    .shown_document(selection.group, selection.secondary)
                                    .is_some_and(|document| document.id == selection.document);
                                let current_selection = view
                                    .group_code(selection.group, selection.secondary)
                                    .is_some_and(|editor| {
                                        let code = editor.read(cx);
                                        code.text()
                                            .is_some_and(|text| text.version() == selection.version)
                                            && code.selections() == selection.selections
                                    });
                                if current_doc && current_selection {
                                    view.add_basket_editor_pane(
                                        selection.group,
                                        selection.secondary,
                                        cx,
                                    );
                                } else {
                                    view.report_basket("contextBasket.reselect", Arc::from([]), cx);
                                }
                            }
                            Some(MenuAction::Sources(targets, document)) => {
                                view.add_basket_sources(targets, document, cx);
                            }
                            None => {}
                        }
                        cx.notify();
                    })),
            );
        }
        Some(
            deferred(
                anchored()
                    .anchor(Corner::TopLeft)
                    .position(at)
                    .snap_to_window_with_margin(px(4.0))
                    .child(card.child(crate::markdown::occlusion_marker())),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}
