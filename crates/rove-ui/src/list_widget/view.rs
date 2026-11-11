//! GPUI list/tree view (#18). Only rows intersecting the viewport are built and painted; all
//! state lives in [`ListWidget`], so the view is a thin translation of input events and theme
//! colors. Views customize rows with a [`RowRenderer`] closure; the list never sees item types.
use super::{
    keys::list_actions,
    options::{IndentGuides, ListOptions},
    registry::context as ck,
    text_input::{TextInput, TextInputColors, TextInputEvent},
    tree::RowKind,
    widget::{ListEvent, ListWidget, Modifiers},
};
use crate::lists::{ItemId, Row};
use gpui::{
    canvas, div, prelude::*, px, AnyElement, App, Bounds, Context, DragMoveEvent, Entity,
    EventEmitter, FocusHandle, Focusable, HighlightStyle, Hsla, KeyContext, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent,
    SharedString, StyledText, Window,
};
use rove_base::Value;
use rove_i18n::{Message, Strings};
use rove_theme::{Color, Theme};
use std::{cell::RefCell, ops::Range, rc::Rc, sync::Arc, time::Instant};

pub fn hsla(c: Color) -> Hsla {
    gpui::rgba(((c.0 as u32) << 24) | ((c.1 as u32) << 16) | ((c.2 as u32) << 8) | c.3 as u32)
        .into()
}

/// Left padding of a row at `depth` (`workbench.tree.indent`).
pub fn indent_px(options: &ListOptions, depth: u32) -> f32 {
    options.indent * depth as f32
}

/// `workbench.tree.renderIndentGuides`: `onHover` shows guides while the pointer is over the
/// tree (or it has keyboard focus).
pub fn guides_visible(options: &ListOptions, hovering_or_focused: bool) -> bool {
    match options.indent_guides {
        IndentGuides::None => false,
        IndentGuides::Always => true,
        IndentGuides::OnHover => hovering_or_focused,
    }
}

/// Theme colors used by list rows, resolved once per frame.
#[derive(Clone)]
pub struct ListColors {
    pub foreground: Option<Hsla>,
    pub active_selection_bg: Option<Hsla>,
    pub active_selection_fg: Option<Hsla>,
    pub inactive_selection_bg: Option<Hsla>,
    pub inactive_selection_fg: Option<Hsla>,
    pub focus_bg: Option<Hsla>,
    pub focus_fg: Option<Hsla>,
    pub focus_outline: Option<Hsla>,
    pub inactive_focus_outline: Option<Hsla>,
    pub hover_bg: Option<Hsla>,
    pub highlight_fg: Option<Hsla>,
    pub focus_highlight_fg: Option<Hsla>,
    pub filter_match_bg: Option<Hsla>,
    pub deemphasized_fg: Option<Hsla>,
    pub guides: Option<Hsla>,
    pub inactive_guides: Option<Hsla>,
    pub picker_border: Option<Hsla>,
    pub picker_fg: Option<Hsla>,
    pub drop_bg: Option<Hsla>,
    pub drop_between: Option<Hsla>,
    pub filter_widget_bg: Option<Hsla>,
    pub filter_widget_outline: Option<Hsla>,
    pub no_matches_outline: Option<Hsla>,
    pub filter_widget_shadow: Option<Hsla>,
    pub input_bg: Option<Hsla>,
    pub input_fg: Option<Hsla>,
    pub input_placeholder: Option<Hsla>,
    pub scrollbar: Hsla,
    pub sticky_bg: Option<Hsla>,
}

impl ListColors {
    pub fn resolve(theme: &dyn Theme) -> Self {
        let c = |k: &str| theme.color(k).ok().flatten().map(hsla);
        Self {
            foreground: c("sideBar.foreground").or(c("foreground")),
            active_selection_bg: c("list.activeSelectionBackground"),
            active_selection_fg: c("list.activeSelectionForeground"),
            inactive_selection_bg: c("list.inactiveSelectionBackground"),
            inactive_selection_fg: c("list.inactiveSelectionForeground"),
            focus_bg: c("list.focusBackground"),
            focus_fg: c("list.focusForeground"),
            focus_outline: c("list.focusOutline"),
            inactive_focus_outline: c("list.inactiveFocusOutline"),
            hover_bg: c("list.hoverBackground"),
            highlight_fg: c("list.highlightForeground"),
            focus_highlight_fg: c("list.focusHighlightForeground"),
            filter_match_bg: c("list.filterMatchBackground"),
            deemphasized_fg: c("list.deemphasizedForeground"),
            guides: c("tree.indentGuidesStroke"),
            inactive_guides: c("tree.inactiveIndentGuidesStroke"),
            picker_border: c("pickerGroup.border"),
            picker_fg: c("pickerGroup.foreground"),
            drop_bg: c("list.dropBackground"),
            drop_between: c("list.dropBetweenBackground"),
            filter_widget_bg: c("listFilterWidget.background"),
            filter_widget_outline: c("listFilterWidget.outline"),
            no_matches_outline: c("listFilterWidget.noMatchesOutline"),
            filter_widget_shadow: c("listFilterWidget.shadow"),
            input_bg: c("input.background"),
            input_fg: c("input.foreground"),
            input_placeholder: c("input.placeholderForeground"),
            scrollbar: c("scrollbarSlider.background")
                .unwrap_or_else(|| hsla(Color(0x79, 0x79, 0x79, 0x66))),
            sticky_bg: c("sideBar.background").or(c("listFilterWidget.background")),
        }
    }
}

/// Everything a row renderer needs; built only for visible rows.
pub struct RowContext<'a> {
    pub index: usize,
    pub row: &'a Row,
    pub kind: RowKind,
    pub focused: bool,
    pub selected: bool,
    pub hovered: bool,
    pub list_focused: bool,
    pub highlights: Vec<Range<usize>>,
    pub find_open: bool,
    pub indent: f32,
    pub is_tree: bool,
    /// Group label when this row starts a `pickerGroup` section.
    pub separator: Option<Option<Arc<str>>>,
    pub colors: &'a ListColors,
    pub strings: &'a dyn Strings,
    /// Text for loading placeholder rows.
    pub loading_text: Option<SharedString>,
}

pub type RowRenderer = Rc<dyn Fn(&RowContext, &mut Window, &mut App) -> AnyElement>;

fn text(strings: &dyn Strings, key: &str) -> SharedString {
    strings
        .text(&Message::key(key))
        .unwrap_or_else(|_| key.to_string())
        .into()
}

/// Label with match highlights (UTF-8 ranges from the matcher).
pub fn highlighted_label(ctx: &RowContext) -> StyledText {
    let fg = if ctx.focused && ctx.selected && ctx.list_focused {
        ctx.colors.focus_highlight_fg.or(ctx.colors.highlight_fg)
    } else {
        ctx.colors.highlight_fg
    };
    let bg = ctx
        .find_open
        .then_some(ctx.colors.filter_match_bg)
        .flatten();
    let label = SharedString::from(ctx.row.label.to_string());
    let highlights: Vec<_> = ctx
        .highlights
        .iter()
        .filter(|r| {
            r.end <= label.len() && label.is_char_boundary(r.start) && label.is_char_boundary(r.end)
        })
        .map(|r| {
            (
                r.clone(),
                HighlightStyle {
                    color: fg,
                    background_color: bg,
                    ..Default::default()
                },
            )
        })
        .collect();
    StyledText::new(label).with_highlights(highlights)
}

/// Default row: twistie (trees) + highlighted label + dimmed detail.
pub fn default_row(ctx: &RowContext, _: &mut Window, _: &mut App) -> AnyElement {
    if ctx.kind == RowKind::Loading {
        return div()
            .text_color(
                ctx.colors
                    .deemphasized_fg
                    .unwrap_or(gpui::transparent_black()),
            )
            .child(ctx.loading_text.clone().unwrap_or_default())
            .into_any_element();
    }
    let mut el = div()
        .flex()
        .items_center()
        .gap(px(4.))
        .min_w_0()
        .overflow_hidden();
    if ctx.is_tree {
        let twistie = if ctx.row.expandable {
            text(
                ctx.strings,
                if ctx.row.expanded {
                    "list.twistie.expanded"
                } else {
                    "list.twistie.collapsed"
                },
            )
        } else {
            SharedString::default()
        };
        el = el.child(div().w(px(16.)).flex_shrink_0().child(twistie));
    }
    el = el.child(div().flex_shrink_0().child(highlighted_label(ctx)));
    if let Some(d) = &ctx.row.detail {
        el = el.child(
            div()
                .text_color(
                    ctx.colors
                        .deemphasized_fg
                        .unwrap_or(gpui::transparent_black()),
                )
                .overflow_hidden()
                .child(SharedString::from(d.to_string())),
        );
    }
    el.into_any_element()
}

/// Drag payload for list drag-and-drop (business semantics belong to the consuming view).
#[derive(Clone)]
pub struct DraggedItems {
    pub ids: Arc<[ItemId]>,
    pub label: SharedString,
}

struct DragPreview(SharedString);
impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().px_2().child(self.0.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropPosition {
    Before,
    Over,
    After,
}

#[derive(Clone, Debug)]
pub enum ListViewEvent {
    /// A button of the empty state was clicked; the payload is its command ID.
    EmptyAction(Arc<str>),
    Drop {
        items: Arc<[ItemId]>,
        target: Option<ItemId>,
        position: DropPosition,
    },
}

/// Empty-state slot (UX_SPEC §6.3): message plus optional command buttons.
#[derive(Clone)]
pub struct EmptyState {
    pub message: Message,
    pub actions: Vec<(Message, Arc<str>)>,
}

/// Per-frame CPU probe for the scroll benchmark (render → layout → prepaint → paint).
#[derive(Default)]
pub struct FrameProbe {
    pub render_start: Option<Instant>,
    pub cpu_us: Vec<f64>,
}

pub struct ListView {
    pub widget: ListWidget,
    focus: FocusHandle,
    find_input: Entity<TextInput>,
    theme: Arc<dyn Theme>,
    strings: Arc<dyn Strings>,
    renderer: Option<RowRenderer>,
    empty: Option<EmptyState>,
    loading: Option<Message>,
    loading_tail: Option<Message>,
    hovered: Option<usize>,
    mouse_inside: bool,
    bounds: Option<Bounds<Pixels>>,
    epoch: Instant,
    drag_over: Option<(usize, DropPosition)>,
    thumb_drag: Option<f32>,
    hover_item: Option<ItemId>,
    /// Enables drag sources and drop targets on rows.
    pub drag_and_drop: bool,
    pub frame_probe: Option<Rc<RefCell<FrameProbe>>>,
    _find_events: gpui::Subscription,
}

impl EventEmitter<ListEvent> for ListView {}
impl EventEmitter<ListViewEvent> for ListView {}

impl ListView {
    pub fn new(
        widget: ListWidget,
        theme: Arc<dyn Theme>,
        strings: Arc<dyn Strings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let colors = ListColors::resolve(&*theme);
        let find_input = cx.new(|cx| {
            TextInput::new(
                cx,
                TextInputColors {
                    text: colors.input_fg.unwrap_or(gpui::white()),
                    placeholder: colors.input_placeholder.unwrap_or(gpui::white()),
                    caret: colors.input_fg.unwrap_or(gpui::white()),
                    selection: colors.active_selection_bg.unwrap_or(gpui::blue()),
                },
            )
        });
        let sub = cx.subscribe(&find_input, |v: &mut Self, _, e: &TextInputEvent, cx| {
            if let TextInputEvent::Changed { text, .. } = e {
                v.widget.set_find_query(text);
                v.flush(cx);
            }
        });
        Self {
            widget,
            focus: cx.focus_handle(),
            find_input,
            theme,
            strings,
            renderer: None,
            empty: None,
            loading: None,
            loading_tail: None,
            hovered: None,
            mouse_inside: false,
            bounds: None,
            epoch: Instant::now(),
            drag_over: None,
            thumb_drag: None,
            hover_item: None,
            drag_and_drop: false,
            frame_probe: None,
            _find_events: sub,
        }
    }

    pub fn set_renderer(&mut self, renderer: RowRenderer) {
        self.renderer = Some(renderer);
    }

    pub fn set_empty_state(&mut self, empty: Option<EmptyState>) {
        self.empty = empty;
    }

    /// Whole-list loading slot shown while there are no rows yet.
    pub fn set_loading(&mut self, message: Option<Message>) {
        self.loading = message;
    }

    /// Loading row appended after the last item (e.g. scan still running).
    pub fn set_loading_tail(&mut self, message: Option<Message>) {
        self.widget.set_loading_tail(message.is_some());
        self.loading_tail = message;
    }

    pub fn set_options(&mut self, options: ListOptions, cx: &mut Context<Self>) {
        self.widget.set_options(options);
        cx.notify();
    }

    pub fn find_input(&self) -> &Entity<TextInput> {
        &self.find_input
    }

    pub fn hover_item(&self) -> Option<&ItemId> {
        self.hover_item.as_ref()
    }

    /// Runs a `list.*` command (key bindings, menus, or #14's resolver call this).
    pub fn run_command(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "rove.list.contextMenu" => self.widget.context_menu(),
            "list.find" => {
                if self.widget.execute(id).unwrap_or(false) {
                    self.find_input.update(cx, |i, cx| i.set_text("", cx));
                    let f = self.find_input.focus_handle(cx);
                    window.focus(&f);
                }
            }
            "list.closeFind" => {
                let _ = self.widget.execute(id);
                window.focus(&self.focus);
            }
            _ => {
                if let Err(e) = self.widget.execute(id) {
                    eprintln!("{id}: {e}");
                }
            }
        }
        self.flush(cx);
    }

    fn flush(&mut self, cx: &mut Context<Self>) {
        for e in self.widget.take_events() {
            match &e {
                ListEvent::ShowHover(id) => self.hover_item = Some(id.clone()),
                ListEvent::FocusChanged(_) => self.hover_item = None,
                _ => {}
            }
            cx.emit(e);
        }
        cx.notify();
    }

    fn key_context(&self) -> KeyContext {
        let mut k = KeyContext::default();
        k.add("List");
        k.add(ck::LIST_FOCUS);
        for (name, value) in self.widget.context_keys() {
            match value {
                Value::Bool(true) => k.add(name),
                Value::String(s) => k.set(name, s.to_string()),
                _ => {}
            }
        }
        k
    }

    fn now_ms(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64() * 1000.0
    }

    fn modifiers(m: &gpui::Modifiers) -> Modifiers {
        Modifiers {
            ctrl: m.control,
            shift: m.shift,
            alt: m.alt,
            meta: m.platform,
        }
    }

    fn row_element(
        &self,
        i: usize,
        row: &Row,
        colors: &ListColors,
        list_focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let w = &self.widget;
        let focused = w.focus() == Some(i) && w.sticky_focus().is_none();
        let selected = w.is_selected(i);
        let hovered = self.hovered == Some(i);
        let kind = w.row_kind(i);
        let indent = if w.is_tree() {
            indent_px(w.options(), row.depth)
        } else {
            0.0
        };
        let separator = w
            .group_starts()
            .iter()
            .find(|s| s.0 == i)
            .map(|s| s.1.clone());
        let ctx = RowContext {
            index: i,
            row,
            kind,
            focused,
            selected,
            hovered,
            list_focused,
            highlights: w.highlights(i),
            find_open: w.find_state().open,
            indent,
            is_tree: w.is_tree(),
            separator: separator.clone(),
            colors,
            strings: &*self.strings,
            loading_text: (kind == RowKind::Loading)
                .then(|| text(&*self.strings, "common.loading")),
        };
        let content = match &self.renderer {
            Some(r) => r(&ctx, window, cx),
            None => default_row(&ctx, window, cx),
        };
        let height = w.row_height(i);
        let top = w.row_top(i) - w.scroll_top();
        let (bg, fg) = if selected {
            if list_focused {
                (colors.active_selection_bg, colors.active_selection_fg)
            } else {
                (colors.inactive_selection_bg, colors.inactive_selection_fg)
            }
        } else if focused && list_focused && colors.focus_bg.is_some() {
            (colors.focus_bg, colors.focus_fg)
        } else if hovered {
            (colors.hover_bg, None)
        } else {
            (None, None)
        };
        let mut el = div()
            .id(("list-row", i))
            .absolute()
            .top(px(top))
            .left(px(-w.scroll_left()))
            .h(px(height))
            .when(w.options().horizontal_scrolling, |d| d.min_w_full())
            .when(!w.options().horizontal_scrolling, |d| d.w_full())
            .flex()
            .items_center()
            .pl(px(6.0 + indent))
            .pr(px(12.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .overflow_hidden()
            .child(content);
        if let Some(bg) = bg {
            el = el.bg(bg);
        }
        if let Some(fg) = fg {
            el = el.text_color(fg);
        }
        if focused {
            let outline = if list_focused {
                colors.focus_outline
            } else {
                colors.inactive_focus_outline
            };
            if let Some(o) = outline {
                el = el.border_color(o);
            }
        }
        if let Some((_, pos)) = self.drag_over.filter(|d| d.0 == i) {
            match pos {
                DropPosition::Over => {
                    if let Some(c) = colors.drop_bg {
                        el = el.bg(c);
                    }
                }
                DropPosition::Before | DropPosition::After => {
                    if let Some(c) = colors.drop_between {
                        let edge = div().absolute().left_0().right_0().h(px(2.)).bg(c);
                        el = el.child(if pos == DropPosition::Before {
                            edge.top_0()
                        } else {
                            edge.bottom_0()
                        });
                    }
                }
            }
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
        // Indent guides.
        if w.is_tree()
            && row.depth > 0
            && guides_visible(w.options(), self.mouse_inside || list_focused)
        {
            let active_parent = w.focus().and_then(|f| w.tree_model()?.parent_index(f));
            let parent_depth = active_parent.map(|p| w.tree_model().map_or(0, |t| t.depth_at(p)));
            for d in 0..row.depth {
                let color = if Some(d) == parent_depth {
                    colors.guides
                } else {
                    colors.inactive_guides
                };
                if let Some(c) = color {
                    el = el.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(px(6.0 + indent_px(w.options(), d) + 8.0))
                            .w(px(1.))
                            .bg(c),
                    );
                }
            }
        }
        let twistie_end = 6.0 + indent + 16.0;
        el = el
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, e: &MouseDownEvent, window, cx| {
                    window.focus(&v.focus);
                    let x = v.bounds.map_or(0.0, |b| f32::from(e.position.x - b.left()));
                    let on_twistie = v.widget.is_tree() && x < twistie_end;
                    v.widget.click(
                        i,
                        Self::modifiers(&e.modifiers),
                        e.click_count as u32,
                        on_twistie,
                    );
                    v.flush(cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |v, _: &MouseDownEvent, window, cx| {
                    window.focus(&v.focus);
                    if !v.widget.is_selected(i) {
                        v.widget.click(i, Modifiers::default(), 0, false);
                    }
                    v.widget.context_menu();
                    v.flush(cx);
                }),
            )
            .on_mouse_move(cx.listener(move |v, _: &MouseMoveEvent, _, cx| {
                if v.hovered != Some(i) {
                    v.hovered = Some(i);
                    cx.notify();
                }
            }));
        if self.drag_and_drop && kind == RowKind::Item {
            let ids: Arc<[ItemId]> = if selected {
                w.selected_ids()
            } else {
                Arc::from([row.id.clone()])
            };
            let payload = DraggedItems {
                ids,
                label: row.label.to_string().into(),
            };
            el = el
                .on_drag(payload, |d: &DraggedItems, _: Point<Pixels>, _, cx| {
                    let label = d.label.clone();
                    cx.new(|_| DragPreview(label))
                })
                .on_drag_move(
                    cx.listener(move |v, e: &DragMoveEvent<DraggedItems>, _, cx| {
                        if !e.bounds.contains(&e.event.position) {
                            return;
                        }
                        let y = f32::from(e.event.position.y - e.bounds.top());
                        let h = f32::from(e.bounds.size.height);
                        let pos = if y < h * 0.25 {
                            DropPosition::Before
                        } else if y > h * 0.75 {
                            DropPosition::After
                        } else {
                            DropPosition::Over
                        };
                        if v.drag_over != Some((i, pos)) {
                            v.drag_over = Some((i, pos));
                            cx.notify();
                        }
                    }),
                )
                .on_drop(cx.listener(move |v, d: &DraggedItems, _, cx| {
                    let position = v.drag_over.map_or(DropPosition::Over, |p| p.1);
                    v.drag_over = None;
                    let target = v.widget.row(i).map(|r| r.id);
                    cx.emit(ListViewEvent::Drop {
                        items: d.ids.clone(),
                        target,
                        position,
                    });
                    cx.notify();
                }));
        }
        el.into_any_element()
    }

    fn scrollbar(&self, cx: &mut Context<Self>, colors: &ListColors) -> Option<AnyElement> {
        let w = &self.widget;
        let total = w.total_height();
        let vp = w.viewport();
        if total <= vp || vp <= 0.0 {
            return None;
        }
        let thumb = (vp * vp / total).max(20.0);
        let max = (total - vp).max(1.0);
        let top = w.scroll_top() / max * (vp - thumb);
        Some(
            div()
                .id("list-scrollbar")
                .absolute()
                .top_0()
                .right_0()
                .w(px(10.))
                .h_full()
                .occlude()
                .child(
                    div()
                        .absolute()
                        .top(px(top))
                        .w_full()
                        .h(px(thumb))
                        .bg(colors.scrollbar),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, e: &MouseDownEvent, _, cx| {
                        let y = v.bounds.map_or(0.0, |b| f32::from(e.position.y - b.top()));
                        if y >= top && y <= top + thumb {
                            v.thumb_drag = Some(y - top);
                        } else {
                            v.widget.scrollbar_click(y);
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .into_any_element(),
        )
    }

    fn find_widget(&self, colors: &ListColors, cx: &mut Context<Self>) -> AnyElement {
        let f = self.widget.find_state();
        let outline = if self.widget.has_matches() {
            colors.filter_widget_outline
        } else {
            colors.no_matches_outline
        };
        let placeholder = text(
            &*self.strings,
            match f.mode {
                super::matcher::FindMode::Filter => "list.find.filterPlaceholder",
                super::matcher::FindMode::Highlight => "list.find.placeholder",
            },
        );
        self.find_input
            .update(cx, |i, _| i.set_placeholder(placeholder));
        let toggle = |id: &'static str, key: &str, on: bool| {
            div()
                .id(id)
                .px_1()
                .when(on, |d| {
                    d.when_some(colors.focus_outline, |d, c| d.border_1().border_color(c))
                })
                .child(text(&*self.strings, key))
        };
        div()
            .id("list-find")
            .absolute()
            .top(px(2.))
            .right(px(14.))
            .w(px(260.))
            .h(px(26.))
            .flex()
            .items_center()
            .gap(px(4.))
            .px_1()
            .occlude()
            .when_some(colors.filter_widget_bg, |d, c| d.bg(c))
            .when_some(colors.filter_widget_shadow, |d, c| {
                d.shadow(vec![gpui::BoxShadow {
                    color: c,
                    offset: gpui::point(px(0.), px(2.)),
                    blur_radius: px(8.),
                    spread_radius: px(0.),
                }])
            })
            .border_1()
            .border_color(outline.unwrap_or(gpui::transparent_black()))
            .child(self.find_input.clone())
            .child(
                toggle(
                    "list-find-mode",
                    "list.find.toggleFilter",
                    f.mode == super::matcher::FindMode::Filter,
                )
                .on_click(cx.listener(|v, _, _, cx| {
                    v.widget.toggle_find_mode();
                    v.flush(cx)
                })),
            )
            .child(
                toggle(
                    "list-find-fuzzy",
                    "list.find.toggleFuzzy",
                    f.match_type == super::matcher::MatchType::Fuzzy,
                )
                .on_click(cx.listener(|v, _, _, cx| {
                    v.widget.toggle_find_match_type();
                    v.flush(cx)
                })),
            )
            .child(
                toggle("list-find-close", "list.find.close", false)
                    .on_click(cx.listener(|v, _, w, cx| v.run_command("list.closeFind", w, cx))),
            )
            .into_any_element()
    }
}

impl Focusable for ListView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ListView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(p) = &self.frame_probe {
            p.borrow_mut().render_start = Some(Instant::now());
        }
        if let Some(b) = self.bounds {
            self.widget
                .set_viewport(f32::from(b.size.height), f32::from(b.size.width));
        }
        if self.widget.is_animating() {
            let now = self.now_ms();
            if self.widget.tick(now) {
                window.request_animation_frame();
            }
        }
        let colors = ListColors::resolve(&*self.theme);
        let list_focused = self.focus.contains_focused(window, cx);
        let range = self.widget.visible_range();
        let rows = self.widget.display_rows(range.clone());
        let mut children: Vec<AnyElement> = Vec::with_capacity(rows.len() + 4);
        let mut widest = 0.0f32;
        for (k, row) in rows.iter().enumerate() {
            let i = range.start + k;
            if self.widget.options().horizontal_scrolling {
                widest = widest.max(
                    row.label.chars().count() as f32 * 8.0
                        + indent_px(self.widget.options(), row.depth)
                        + 48.0,
                );
            }
            children.push(self.row_element(i, row, &colors, list_focused, window, cx));
        }
        if self.widget.options().horizontal_scrolling {
            self.widget.set_content_width(widest);
        }
        // Loading tail row.
        if let (true, Some(m)) = (self.widget.loading_tail(), self.loading_tail.as_ref()) {
            let n = self.widget.len();
            let top = self.widget.row_top(n) - self.widget.scroll_top();
            children.push(
                div()
                    .absolute()
                    .top(px(top))
                    .h(px(self.widget.total_height() - self.widget.row_top(n)))
                    .w_full()
                    .pl(px(6.))
                    .flex()
                    .items_center()
                    .when_some(colors.deemphasized_fg, |d, c| d.text_color(c))
                    .child(SharedString::from(self.strings.text(m).unwrap_or_default()))
                    .into_any_element(),
            );
        }
        // Sticky ancestors.
        let sticky = self.widget.sticky_rows();
        if !sticky.is_empty() {
            let sticky_rows: Vec<Row> = sticky.iter().filter_map(|&i| self.widget.row(i)).collect();
            let mut y = 0.0;
            let mut band = Vec::new();
            for (k, row) in sticky_rows.iter().enumerate() {
                let i = sticky[k];
                let h = self.widget.row_height(i);
                let indent = indent_px(self.widget.options(), row.depth);
                let focused = self.widget.sticky_focus() == Some(i);
                let ctx = RowContext {
                    index: i,
                    row,
                    kind: RowKind::Item,
                    focused,
                    selected: false,
                    hovered: false,
                    list_focused,
                    highlights: self.widget.highlights(i),
                    find_open: self.widget.find_state().open,
                    indent,
                    is_tree: true,
                    separator: None,
                    colors: &colors,
                    strings: &*self.strings,
                    loading_text: None,
                };
                let content = match &self.renderer {
                    Some(r) => r(&ctx, window, cx),
                    None => default_row(&ctx, window, cx),
                };
                band.push(
                    div()
                        .id(("list-sticky", k))
                        .absolute()
                        .top(px(y))
                        .h(px(h))
                        .w_full()
                        .pl(px(6.0 + indent))
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(if focused {
                            colors.focus_outline.unwrap_or(gpui::transparent_black())
                        } else {
                            gpui::transparent_black()
                        })
                        .when_some(colors.sticky_bg, |d, c| d.bg(c))
                        .child(content)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |v, _: &MouseDownEvent, window, cx| {
                                window.focus(&v.focus);
                                v.widget.click_sticky(k);
                                cx.stop_propagation();
                                v.flush(cx);
                            }),
                        )
                        .into_any_element(),
                );
                y += h;
            }
            children.push(
                div()
                    .absolute()
                    .top_0()
                    .w_full()
                    .h(px(y))
                    .occlude()
                    .children(band)
                    .into_any_element(),
            );
        }
        // Hover card for list.showHover.
        if let Some(i) = self
            .hover_item
            .as_ref()
            .and_then(|_| self.widget.focus())
            .filter(|&f| f < self.widget.len())
        {
            if let Some(row) = self.widget.row(i) {
                let top =
                    self.widget.row_top(i) + self.widget.row_height(i) - self.widget.scroll_top();
                children.push(
                    div()
                        .absolute()
                        .top(px(top))
                        .left(px(24.))
                        .px_2()
                        .when_some(colors.filter_widget_bg, |d, c| d.bg(c))
                        .child(SharedString::from(
                            row.detail.as_deref().unwrap_or(&row.label).to_string(),
                        ))
                        .into_any_element(),
                );
            }
        }
        // Empty / loading slots.
        if self.widget.is_empty() && !self.widget.loading_tail() {
            let slot = if let Some(m) = &self.loading {
                Some((m.clone(), vec![]))
            } else {
                self.empty
                    .as_ref()
                    .map(|e| (e.message.clone(), e.actions.clone()))
            };
            if let Some((message, actions)) = slot {
                let mut el = div()
                    .absolute()
                    .top_0()
                    .w_full()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(SharedString::from(
                        self.strings.text(&message).unwrap_or_default(),
                    ));
                for (k, (label, command)) in actions.into_iter().enumerate() {
                    el = el.child(
                        div()
                            .id(("list-empty-action", k))
                            .px_2()
                            .when_some(colors.active_selection_bg, |d, c| d.bg(c))
                            .when_some(colors.active_selection_fg, |d, c| d.text_color(c))
                            .child(SharedString::from(
                                self.strings.text(&label).unwrap_or_default(),
                            ))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ListViewEvent::EmptyAction(command.clone()))
                            })),
                    );
                }
                children.push(el.into_any_element());
            }
        }
        if let Some(sb) = self.scrollbar(cx, &colors) {
            children.push(sb);
        }
        if self.widget.find_state().open {
            children.push(self.find_widget(&colors, cx));
        }
        let entity = cx.entity();
        let probe = self.frame_probe.clone();
        let root = div()
            .id("list-view")
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .when_some(colors.foreground, |d, c| d.text_color(c))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        entity.update(cx, |v, cx| {
                            if v.bounds != Some(bounds) {
                                v.bounds = Some(bounds);
                                cx.notify();
                            }
                        })
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .children(children)
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, _, _| {
                        if let Some(p) = probe {
                            let mut p = p.borrow_mut();
                            if let Some(start) = p.render_start.take() {
                                p.cpu_us.push(start.elapsed().as_secs_f64() * 1e6);
                            }
                        }
                    },
                )
                .absolute()
                .size_0(),
            )
            .on_scroll_wheel(cx.listener(|v, e: &ScrollWheelEvent, _, cx| {
                let line = v.widget.row_height(0).max(1.0);
                let (dx, dy) = match e.delta {
                    ScrollDelta::Pixels(p) => (-f32::from(p.x), -f32::from(p.y)),
                    ScrollDelta::Lines(p) => (-p.x * line, -p.y * line),
                };
                v.widget.wheel(dx, dy, Self::modifiers(&e.modifiers));
                cx.notify();
            }))
            .on_hover(cx.listener(|v, hovered: &bool, _, cx| {
                v.mouse_inside = *hovered;
                if !hovered {
                    v.hovered = None;
                }
                cx.notify();
            }))
            .on_mouse_move(cx.listener(|v, e: &MouseMoveEvent, _, cx| {
                if let (Some(grab), Some(b)) = (v.thumb_drag, v.bounds) {
                    if e.pressed_button == Some(MouseButton::Left) {
                        let total = v.widget.total_height();
                        let vp = v.widget.viewport();
                        let thumb = (vp * vp / total).max(20.0);
                        let y = f32::from(e.position.y - b.top()) - grab;
                        let ratio = (y / (vp - thumb).max(1.0)).clamp(0.0, 1.0);
                        v.widget.set_scroll_top(ratio * (total - vp));
                        cx.notify();
                    } else {
                        v.thumb_drag = None;
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|v, _, _, _| {
                    v.thumb_drag = None;
                }),
            )
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, _, cx| {
                let m = &e.keystroke.modifiers;
                if m.control || m.alt || m.platform {
                    return;
                }
                if let Some(ch) = e.keystroke.key_char.as_ref() {
                    if !ch.chars().any(char::is_control) {
                        let now = v.now_ms();
                        if v.widget.type_char(ch, now) {
                            cx.stop_propagation();
                            v.flush(cx);
                        }
                    }
                }
            }));
        list_actions(root, Self::run_command, cx)
    }
}
