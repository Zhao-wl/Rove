//! Single-line text input with IME support, used by quick input and the list find widget (#18);
//! #21/#25 can reuse it with [`InputHistory`]. Adapted from GPUI's `examples/input.rs`
//! (Apache-2.0, see THIRD_PARTY_NOTICES.md).
use super::{
    history::InputHistory,
    keys::{self, *},
    registry::context as ck,
};
use gpui::{
    div, fill, point, prelude::*, px, relative, size, App, Bounds, ClipboardItem, Context,
    CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, Hsla, KeyContext, LayoutId, MouseButton,
    MouseDownEvent, PaintQuad, Pixels, ShapedLine, SharedString, Style, TextRun, UTF16Selection,
    UnderlineStyle, Window,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextInputEvent {
    /// Content changed by the user (typing, IME commit, paste, history).
    Changed { text: String, cursor: usize },
    /// Caret moved without a content change.
    CursorMoved { cursor: usize },
}

#[derive(Clone, Copy)]
pub struct TextInputColors {
    pub text: Hsla,
    pub placeholder: Hsla,
    pub caret: Hsla,
    pub selection: Hsla,
}

pub struct TextInput {
    focus: FocusHandle,
    content: String,
    placeholder: SharedString,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    /// Enables `history.showPrevious/showNext` (find widget, search inputs).
    pub history: Option<InputHistory>,
    /// Extra context keys published by the owner (e.g. `cursorAtEndOfQuickInputBox`).
    pub colors: TextInputColors,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(cx: &mut Context<Self>, colors: TextInputColors) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: String::new(),
            placeholder: SharedString::default(),
            selected: 0..0,
            reversed: false,
            marked: None,
            last_layout: None,
            last_bounds: None,
            history: None,
            colors,
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn cursor(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    pub fn set_placeholder(&mut self, text: impl Into<SharedString>) {
        self.placeholder = text.into();
    }

    /// Programmatic replacement (no `Changed` event).
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_string();
        self.selected = self.content.len()..self.content.len();
        self.marked = None;
        cx.notify();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.history.as_mut() {
            h.reset();
        }
        cx.emit(TextInputEvent::Changed {
            text: self.content.clone(),
            cursor: self.cursor(),
        });
        cx.notify();
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected = offset..offset;
        self.reversed = false;
        cx.emit(TextInputEvent::CursorMoved { cursor: offset });
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selected.start = offset
        } else {
            self.selected.end = offset
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.emit(TextInputEvent::CursorMoved {
            cursor: self.cursor(),
        });
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(i, _)| (i > offset).then_some(i))
            .unwrap_or(self.content.len())
    }

    fn replace(&mut self, range: Range<usize>, text: &str) {
        self.content.replace_range(range.clone(), text);
        let end = range.start + text.len();
        self.selected = end..end;
        self.reversed = false;
    }

    fn on_action(&mut self, id: &'static str, _: &mut Window, cx: &mut Context<Self>) {
        let Some(h) = self.history.as_mut() else {
            cx.propagate();
            return;
        };
        let value = if id == "history.showPrevious" {
            h.show_previous(&self.content)
        } else {
            h.show_next()
        };
        match value {
            Some(v) => {
                self.content = v;
                self.selected = self.content.len()..self.content.len();
                cx.emit(TextInputEvent::Changed {
                    text: self.content.clone(),
                    cursor: self.cursor(),
                });
                cx.notify();
            }
            None => cx.propagate(),
        }
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        // Defensive: never slice inside a char even if a caller hands us a stale offset.
        let mut offset = offset.min(self.content.len());
        while !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        self.content[..offset].chars().map(char::len_utf16).sum()
    }

    fn range_to_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(r.start)..self.offset_to_utf16(r.end)
    }

    fn range_from_utf16(&self, r: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(r.start)..self.offset_from_utf16(r.end)
    }

    fn key_context(&self) -> KeyContext {
        let mut k = KeyContext::default();
        k.add(keys::TEXT_INPUT_CONTEXT);
        k.add(ck::INPUT_FOCUS);
        if let Some(h) = &self.history {
            for (name, value) in h.context_keys(true) {
                if value == rove_base::Value::Bool(true) {
                    k.add(name);
                }
            }
        }
        k
    }
}

/// UTF-16 offset in `s` → UTF-8 byte offset, clamped to the end and to char boundaries.
fn utf16_to_utf8(s: &str, offset: usize) -> usize {
    let mut utf16 = 0;
    for (i, ch) in s.char_indices() {
        if utf16 >= offset {
            return i;
        }
        utf16 += ch.len_utf16();
    }
    s.len()
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range);
        actual.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.range_to_utf16(r))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.replace(range, text);
        self.marked = None;
        self.changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        new_selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.content.replace_range(range.clone(), text);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        // `new_selected` is relative to the composition `text` (UTF-16), not to the content.
        self.selected = new_selected
            .as_ref()
            .map(|r| {
                range.start + utf16_to_utf8(text, r.start)..range.start + utf16_to_utf8(text, r.end)
            })
            .unwrap_or_else(|| range.start + text.len()..range.start + text.len());
        // Composition text is not a committed edit: no Changed event until commit.
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range);
        Some(Bounds::from_corners(
            point(
                bounds.left() + layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                bounds.left() + layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.last_bounds?.localize(&p)?;
        let layout = self.last_layout.as_ref()?;
        let i = layout.index_for_x(p.x - local.x)?;
        Some(self.offset_to_utf16(i))
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let el = div()
            .flex()
            .flex_1()
            .min_w_0()
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(|v, _: &Backspace, w, cx| {
                if v.selected.is_empty() {
                    let p = v.previous_boundary(v.cursor());
                    v.select_to(p, cx);
                }
                v.replace_text_in_range(None, "", w, cx)
            }))
            .on_action(cx.listener(|v, _: &Delete, w, cx| {
                if v.selected.is_empty() {
                    let n = v.next_boundary(v.cursor());
                    v.select_to(n, cx);
                }
                v.replace_text_in_range(None, "", w, cx)
            }))
            .on_action(cx.listener(|v, _: &Left, _, cx| {
                let to = if v.selected.is_empty() {
                    v.previous_boundary(v.cursor())
                } else {
                    v.selected.start
                };
                v.move_to(to, cx)
            }))
            .on_action(cx.listener(|v, _: &Right, _, cx| {
                let to = if v.selected.is_empty() {
                    v.next_boundary(v.selected.end)
                } else {
                    v.selected.end
                };
                v.move_to(to, cx)
            }))
            .on_action(cx.listener(|v, _: &SelectLeft, _, cx| {
                let p = v.previous_boundary(v.cursor());
                v.select_to(p, cx)
            }))
            .on_action(cx.listener(|v, _: &SelectRight, _, cx| {
                let n = v.next_boundary(v.cursor());
                v.select_to(n, cx)
            }))
            .on_action(cx.listener(|v, _: &SelectAll, _, cx| {
                v.move_to(0, cx);
                let len = v.content.len();
                v.select_to(len, cx)
            }))
            .on_action(cx.listener(|v, _: &Home, _, cx| v.move_to(0, cx)))
            .on_action(cx.listener(|v, _: &End, _, cx| {
                let len = v.content.len();
                v.move_to(len, cx)
            }))
            .on_action(cx.listener(|v, _: &Paste, w, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    v.replace_text_in_range(None, &text.replace(['\r', '\n'], " "), w, cx);
                }
            }))
            .on_action(cx.listener(|v, _: &Copy, _, cx| {
                if !v.selected.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        v.content[v.selected.clone()].to_string(),
                    ));
                }
            }))
            .on_action(cx.listener(|v, _: &Cut, w, cx| {
                if !v.selected.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        v.content[v.selected.clone()].to_string(),
                    ));
                    v.replace_text_in_range(None, "", w, cx)
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|v, e: &MouseDownEvent, w, cx| {
                    w.focus(&v.focus);
                    let i = match (v.last_bounds, v.last_layout.as_ref()) {
                        (Some(b), Some(l)) => l.closest_index_for_x(e.position.x - b.left()),
                        _ => v.content.len(),
                    };
                    if e.modifiers.shift {
                        v.select_to(i, cx)
                    } else {
                        v.move_to(i, cx)
                    }
                }),
            );
        history_actions(el, Self::on_action, cx).child(TextElement { input: cx.entity() })
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct Prepaint {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let input = self.input.read(cx);
        let style = window.text_style();
        let (text, color) = if input.content.is_empty() {
            (input.placeholder.clone(), input.colors.placeholder)
        } else {
            (SharedString::from(input.content.clone()), input.colors.text)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match input.marked.as_ref().filter(|_| !input.content.is_empty()) {
            Some(m) => [
                TextRun {
                    len: m.start,
                    ..run.clone()
                },
                TextRun {
                    len: m.end - m.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - m.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(text, font_size, &runs, None);
        let height = bounds.bottom() - bounds.top();
        let (selection, cursor) = if input.selected.is_empty() {
            let x = if input.content.is_empty() {
                px(0.)
            } else {
                line.x_for_index(input.cursor())
            };
            (
                None,
                Some(fill(
                    Bounds::new(point(bounds.left() + x, bounds.top()), size(px(1.), height)),
                    input.colors.caret,
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(input.selected.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(input.selected.end),
                            bounds.bottom(),
                        ),
                    ),
                    input.colors.selection,
                )),
                None,
            )
        };
        Prepaint {
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(s) = prepaint.selection.take() {
            window.paint_quad(s)
        }
        let Some(line) = prepaint.line.take() else {
            return;
        };
        let _ = line.paint(bounds.origin, window.line_height(), window, cx);
        if focus.is_focused(window) {
            if let Some(c) = prepaint.cursor.take() {
                window.paint_quad(c);
            }
        }
        let empty = self.input.read(cx).content.is_empty();
        self.input.update(cx, |input, _| {
            input.last_layout = (!empty).then_some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn utf16_offsets_convert_within_the_composition() {
        assert_eq!(utf16_to_utf8("中", 0), 0);
        assert_eq!(utf16_to_utf8("中", 1), 3);
        assert_eq!(utf16_to_utf8("a😀b", 3), 5);
        assert_eq!(utf16_to_utf8("a😀b", 99), 6);
    }

    /// Regression (PR #102 review): the IME selection was converted against the whole content
    /// and then offset again, landing inside `中` and panicking in `selected_text_range`.
    #[gpui::test]
    fn ime_composition_after_text_keeps_selection_on_char_boundaries(cx: &mut TestAppContext) {
        let colors = TextInputColors {
            text: gpui::white(),
            placeholder: gpui::white(),
            caret: gpui::white(),
            selection: gpui::blue(),
        };
        let (input, cx) = cx.add_window_view(|_, cx| TextInput::new(cx, colors));
        cx.update(|window, cx| {
            input.update(cx, |v, cx| {
                v.replace_text_in_range(None, "a", window, cx);
                // Windows IME: composition "中" with the caret after it (UTF-16 offset 1).
                v.replace_and_mark_text_in_range(None, "中", Some(1..1), window, cx);
                assert_eq!(v.content, "a中");
                assert_eq!(v.selected, 4..4);
                assert_eq!(v.marked, Some(1..4));
                let sel = v.selected_text_range(false, window, cx).unwrap();
                assert_eq!(sel.range, 2..2);
                assert_eq!(v.marked_text_range(window, cx), Some(1..2));
                // Caret inside a longer composition.
                v.replace_and_mark_text_in_range(None, "中文", Some(1..1), window, cx);
                assert_eq!(v.content, "a中文");
                assert_eq!(v.selected, 4..4);
                v.replace_text_in_range(None, "中文", window, cx);
                assert_eq!((v.content.as_str(), v.selected.clone()), ("a中文", 7..7));
                // A stale mid-char offset is clamped instead of panicking.
                assert_eq!(v.offset_to_utf16(2), 1);
            })
        });
    }
}
