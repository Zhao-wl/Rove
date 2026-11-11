//! GPUI quick input (#18): input box + optional title/buttons + virtualized result list with
//! separators and checkboxes. Behavior lives in [`QuickInputService`]; consumers (#24 quick open
//! and command palette, #27 encoding/EOL/language pickers, #28 theme picker) call
//! [`QuickInputView::show`] and subscribe to `service.events()` / `service.pick_events()`.
use super::{
    keys::quick_input_actions,
    quick_input::{QuickInputService, QuickPickConfig},
    registry::context as ck,
    text_input::{TextInput, TextInputColors, TextInputEvent},
    view::{highlighted_label, hsla, ListColors, RowContext},
    widget::Modifiers,
};
use crate::lists::{QuickInputId, QuickInputOptions};
use gpui::{
    div, prelude::*, px, AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Hsla, KeyContext, MouseButton, MouseDownEvent, ScrollDelta, ScrollWheelEvent, SharedString,
    Window,
};
use rove_base::{Result, Value};
use rove_i18n::{Message, Strings};
use rove_theme::Theme;
use std::sync::Arc;

/// Maximum visible result rows (VS Code shows about this many before scrolling).
pub const MAX_VISIBLE_ROWS: usize = 15;
pub const WIDTH: f32 = 600.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuickInputViewEvent {
    /// The active input was accepted (not in background) or hidden; the host can drop focus back.
    Closed(QuickInputId),
}

pub struct QuickInputView {
    pub service: QuickInputService,
    input: Entity<TextInput>,
    focus: FocusHandle,
    theme: Arc<dyn Theme>,
    strings: Arc<dyn Strings>,
    _input_events: gpui::Subscription,
}

impl EventEmitter<QuickInputViewEvent> for QuickInputView {}

fn color(theme: &dyn Theme, key: &str) -> Option<Hsla> {
    theme.color(key).ok().flatten().map(hsla)
}

impl QuickInputView {
    pub fn new(
        service: QuickInputService,
        theme: Arc<dyn Theme>,
        strings: Arc<dyn Strings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let c = |k| color(&*theme, k).unwrap_or(gpui::white());
        let colors = TextInputColors {
            text: c("input.foreground"),
            placeholder: c("input.placeholderForeground"),
            caret: c("input.foreground"),
            selection: c("list.activeSelectionBackground"),
        };
        let input = cx.new(|cx| TextInput::new(cx, colors));
        let sub = cx.subscribe(&input, |v: &mut Self, _, e: &TextInputEvent, cx| {
            match e {
                TextInputEvent::Changed { text, cursor } => v.service.set_text(text, *cursor),
                TextInputEvent::CursorMoved { cursor } => v.service.set_cursor(*cursor),
            }
            cx.notify();
        });
        Self {
            service,
            input,
            focus: cx.focus_handle(),
            theme,
            strings,
            _input_events: sub,
        }
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// Shows a picker and focuses its input. Replaces (cancels) any active one.
    pub fn show(
        &mut self,
        options: QuickInputOptions,
        config: QuickPickConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<QuickInputId> {
        let placeholder = self.strings.text(&options.placeholder).unwrap_or_default();
        let initial = options.initial.clone();
        let id = self.service.show_with(options, config)?;
        self.input.update(cx, |i, cx| {
            i.set_placeholder(placeholder);
            i.set_text(&initial, cx);
        });
        let f = self.input.focus_handle(cx);
        window.focus(&f);
        cx.notify();
        Ok(id)
    }

    /// Runs a `quickInput.*` command with the current keyboard modifiers.
    pub fn run_command(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let m = window.modifiers();
        let before = self.service.active().map(|a| a.id);
        let modifiers = Modifiers {
            ctrl: m.control,
            shift: m.shift,
            alt: m.alt,
            meta: m.platform,
        };
        if let Err(e) = self.service.execute(id, modifiers) {
            eprintln!("{id}: {e}");
        }
        self.after_change(before, cx);
    }

    fn after_change(&mut self, before: Option<QuickInputId>, cx: &mut Context<Self>) {
        match (before, self.service.active()) {
            (Some(id), None) => cx.emit(QuickInputViewEvent::Closed(id)),
            (_, Some(a)) => {
                let text = a.text.clone();
                if self.input.read(cx).text() != text {
                    self.input.update(cx, |i, cx| i.set_text(&text, cx));
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn key_context(&self) -> KeyContext {
        let mut k = KeyContext::default();
        k.add("QuickInput");
        for (name, value) in self.service.context_keys() {
            // inputFocus comes from the text input's own context when it is focused.
            if name == ck::INPUT_FOCUS {
                continue;
            }
            match value {
                Value::Bool(true) => k.add(name),
                Value::String(s) => k.set(name, s.to_string()),
                _ => {}
            }
        }
        k
    }

    fn rows(&self, colors: &ListColors, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(a) = self.service.active() else {
            return vec![];
        };
        let l = &a.list;
        let range = l.visible_range();
        let rows = l.display_rows(range.clone());
        let focus_bg = color(&*self.theme, "quickInputList.focusBackground");
        let focus_fg = color(&*self.theme, "quickInputList.focusForeground");
        let mut out = Vec::with_capacity(rows.len());
        for (k, row) in rows.iter().enumerate() {
            let i = range.start + k;
            let focused = l.focus() == Some(i);
            let separator = l
                .group_starts()
                .iter()
                .find(|s| s.0 == i)
                .map(|s| s.1.clone());
            let ctx = RowContext {
                index: i,
                row,
                kind: super::tree::RowKind::Item,
                focused,
                selected: focused,
                hovered: false,
                list_focused: true,
                highlights: l.highlights(i),
                find_open: false,
                indent: 0.0,
                is_tree: false,
                separator: separator.clone(),
                colors,
                strings: &*self.strings,
                loading_text: None,
            };
            let mut content = div().flex().items_center().gap(px(6.)).min_w_0();
            if a.multi_select {
                let checked = a.checked.contains(&row.id);
                let key = if checked {
                    "list.checkbox.checked"
                } else {
                    "list.checkbox.unchecked"
                };
                content = content.child(SharedString::from(
                    self.strings.text(&Message::key(key)).unwrap_or_default(),
                ));
            }
            content = content.child(highlighted_label(&ctx));
            if let Some(d) = &row.detail {
                content = content.child(
                    div()
                        .when_some(colors.deemphasized_fg, |d, c| d.text_color(c))
                        .child(SharedString::from(d.to_string())),
                );
            }
            if a.hover_visible && focused {
                if let Some(d) = &row.detail {
                    content = content.child(div().px_2().child(SharedString::from(d.to_string())));
                }
            }
            let mut el = div()
                .id(("quick-row", i))
                .absolute()
                .top(px(l.row_top(i) - l.scroll_top()))
                .h(px(l.row_height(i)))
                .w_full()
                .px_2()
                .flex()
                .items_center()
                .overflow_hidden()
                .child(content)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, e: &MouseDownEvent, _, cx| {
                        let before = v.service.active().map(|a| a.id);
                        let m = Modifiers {
                            ctrl: e.modifiers.control,
                            shift: e.modifiers.shift,
                            alt: e.modifiers.alt,
                            meta: e.modifiers.platform,
                        };
                        v.service.click(i, m);
                        v.after_change(before, cx);
                    }),
                );
            if focused {
                el = el
                    .when_some(focus_bg, |d, c| d.bg(c))
                    .when_some(focus_fg, |d, c| d.text_color(c));
            }
            if let Some(label) = separator {
                if i > 0 {
                    if let Some(b) = colors.picker_border {
                        el = el.child(div().absolute().top_0().left_0().right_0().h(px(1.)).bg(b));
                    }
                }
                if let Some(label) = label {
                    el = el.child(
                        div()
                            .absolute()
                            .right(px(12.))
                            .when_some(colors.picker_fg, |d, c| d.text_color(c))
                            .child(SharedString::from(label.to_string())),
                    );
                }
            }
            out.push(el.into_any_element());
        }
        if let (true, Some(m)) = (l.loading_tail(), a.config.loading_tail.as_ref()) {
            let n = l.len();
            out.push(
                div()
                    .absolute()
                    .top(px(l.row_top(n) - l.scroll_top()))
                    .h(px(a.config.row_height))
                    .px_2()
                    .when_some(colors.deemphasized_fg, |d, c| d.text_color(c))
                    .child(SharedString::from(self.strings.text(m).unwrap_or_default()))
                    .into_any_element(),
            );
        }
        out
    }
}

impl Focusable for QuickInputView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for QuickInputView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input_focused = self.input.focus_handle(cx).is_focused(window);
        self.service.set_input_focused(input_focused);
        let Some(a) = self.service.active() else {
            return div().id("quick-input").into_any_element();
        };
        let row_h = a.config.row_height;
        let shown = a.list.len().min(MAX_VISIBLE_ROWS) + usize::from(a.list.loading_tail());
        let list_h = shown as f32 * row_h;
        let no_results = a.list.is_empty() && !a.text.is_empty() && !a.list.loading_tail();
        let title = a.config.title.clone();
        let buttons = a.config.buttons.clone();
        let empty = a
            .config
            .empty
            .clone()
            .unwrap_or_else(|| Message::key("quickInput.noResults"));
        self.service.set_viewport(list_h, WIDTH);
        let colors = ListColors::resolve(&*self.theme);
        let t = |k| color(&*self.theme, k);
        let text = |m: &Message| SharedString::from(self.strings.text(m).unwrap_or_default());
        let mut root = div()
            .id("quick-input")
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .p_1()
            .gap(px(4.))
            .when_some(t("quickInput.background"), |d, c| d.bg(c))
            .when_some(t("quickInput.foreground"), |d, c| d.text_color(c))
            .when_some(
                t("widget.shadow").or(colors.filter_widget_shadow),
                |d, c| {
                    d.shadow(vec![gpui::BoxShadow {
                        color: c,
                        offset: gpui::point(px(0.), px(2.)),
                        blur_radius: px(8.),
                        spread_radius: px(0.),
                    }])
                },
            );
        if let Some(title) = &title {
            root = root.child(
                div()
                    .px_2()
                    .when_some(t("quickInputTitle.background"), |d, c| d.bg(c))
                    .child(text(title)),
            );
        }
        let mut input_row = div()
            .flex()
            .items_center()
            .h(px(26.))
            .px_1()
            .when_some(t("input.background"), |d, c| d.bg(c))
            .when_some(t("input.border"), |d, c| d.border_1().border_color(c))
            .child(self.input.clone());
        for (k, b) in buttons.into_iter().enumerate() {
            let id = b.id.clone();
            input_row = input_row.child(
                div()
                    .id(("quick-button", k))
                    .px_1()
                    .child(SharedString::from(b.icon.to_string()))
                    .on_click(cx.listener(move |v, _, _, cx| {
                        let _ = v.service.trigger_button(&id);
                        cx.notify();
                    })),
            );
        }
        root = root.child(input_row);
        if no_results {
            root = root.child(div().px_2().child(text(&empty)));
        }
        let rows = self.rows(&colors, cx);
        root = root.child(
            div()
                .id("quick-list")
                .relative()
                .h(px(list_h))
                .overflow_hidden()
                .children(rows)
                .on_scroll_wheel(cx.listener(move |v, e: &ScrollWheelEvent, _, cx| {
                    let dy = match e.delta {
                        ScrollDelta::Pixels(p) => -f32::from(p.y),
                        ScrollDelta::Lines(p) => -p.y * row_h,
                    };
                    if let Some(a) = v.service.active_mut() {
                        a.list.wheel(0.0, dy, Modifiers::default());
                    }
                    cx.notify();
                })),
        );
        quick_input_actions(root, Self::run_command, cx).into_any_element()
    }
}
