//! 标签悬停提示的小视图（元数据视图的 DLL 完整路径）。

use gpui::{
    div, prelude::*, px, AnyView, App, Context, Hsla, IntoElement, Render, SharedString, Window,
};

pub struct TabTooltip {
    text: SharedString,
    bg: Hsla,
    fg: Hsla,
    border: Hsla,
}

impl Render for TabTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(self.bg)
            .text_color(self.fg)
            .border_1()
            .border_color(self.border)
            .text_size(px(12.0))
            .child(self.text.clone())
    }
}

/// `StatefulInteractiveElement::tooltip` 要的构造闭包。
pub fn tab_tooltip(
    text: String,
    bg: Hsla,
    fg: Hsla,
    border: Hsla,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| {
        cx.new(|_| TabTooltip {
            text: text.clone(),
            bg,
            fg,
            border,
        })
        .into()
    }
}
