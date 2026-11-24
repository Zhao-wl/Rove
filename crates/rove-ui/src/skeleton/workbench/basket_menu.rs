use super::{basket_host::BasketCapture, Context, Workbench};
use crate::{
    context_basket::{capture::SourceRange, source},
    lists::ItemId,
};
use gpui::{anchored, deferred, div, prelude::*, px, AnyElement, Corner, Pixels, Point, Window};
use rove_base::{DocumentId, Resource};

enum MenuAction {
    Editor(crate::documents::EditorGroupId, bool),
    Sources(Vec<BasketCapture>),
}

pub(super) struct BasketMenu {
    position: Option<Point<Pixels>>,
    options: Vec<(&'static str, MenuAction)>,
}

impl Workbench {
    fn show_basket_menu(
        &mut self,
        options: Vec<(&'static str, MenuAction)>,
        cx: &mut Context<Self>,
    ) {
        if !options.is_empty() {
            self.basket.menu = Some(BasketMenu {
                position: None,
                options,
            });
            cx.notify();
        }
    }

    pub(super) fn show_basket_editor_menu(
        &mut self,
        group: crate::documents::EditorGroupId,
        secondary: bool,
        cx: &mut Context<Self>,
    ) {
        self.show_basket_menu(
            vec![(
                "command.rove.contextBasket.add",
                MenuAction::Editor(group, secondary),
            )],
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
                MenuAction::Sources(primary),
            ));
        }
        if !whole.is_empty() {
            options.push(("contextBasket.addWholeFile", MenuAction::Sources(whole)));
        }
        self.show_basket_menu(options, cx);
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
            .collect();
        self.show_basket_sources_menu(targets, Vec::new(), cx);
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
                            Some(MenuAction::Editor(group, secondary)) => {
                                view.add_basket_editor_pane(group, secondary, cx);
                            }
                            Some(MenuAction::Sources(targets)) => {
                                view.add_basket_sources(targets, cx)
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
                    .child(card),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}
