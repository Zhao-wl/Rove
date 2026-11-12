//! 悬停卡片的绘制（VS Code `editorHoverWidget.*` 颜色）：签名（代码字体）、文档段落、定义预览，
//! 底部状态行只在近似结果时出现（灰色“近似结果”）。宿主负责定位（`gpui::anchored`）、焦点与事件。

use super::hover::HoverContent;
use super::registry::NavColors;
use gpui::{div, prelude::*, px, rgba, AnyElement, Hsla, SharedString};
use rove_i18n::{Message, Strings};
use rove_theme::Color;
use std::sync::Arc;

/// 卡片的尺寸与字体（已乘窗口缩放）。
#[derive(Clone, Debug)]
pub struct CardStyle {
    pub colors: NavColors,
    pub code_font: SharedString,
    pub code_size: f32,
    pub ui_size: f32,
    pub max_width: f32,
    pub max_height: f32,
}

fn color(c: Color) -> Hsla {
    rgba(((c.0 as u32) << 24) | ((c.1 as u32) << 16) | ((c.2 as u32) << 8) | c.3 as u32).into()
}

fn text(strings: &dyn Strings, key: &str, args: &[(&str, &str)]) -> String {
    let m = Message {
        key: key.into(),
        args: args
            .iter()
            .map(|(k, v)| (Arc::from(*k), Arc::from(*v)))
            .collect(),
    };
    rove_i18n::tr!(strings, &m).unwrap_or_default()
}

/// 卡片正文（不含外框与定位）。`loading` 时只有“正在加载…”。
pub fn card_body(
    content: Option<&HoverContent>,
    loading: bool,
    strings: &dyn Strings,
    style: &CardStyle,
) -> Vec<AnyElement> {
    let mut out: Vec<AnyElement> = Vec::new();
    let pad = |d: gpui::Div| d.px(px(8.0)).py(px(4.0));
    let Some(content) = content.filter(|_| !loading) else {
        out.push(
            pad(div())
                .text_size(px(style.ui_size))
                .child(text(strings, "nav.hover.loading", &[]))
                .into_any_element(),
        );
        return out;
    };
    let code_block = |lines: Vec<String>| {
        let mut d = pad(div())
            .flex()
            .flex_col()
            .font_family(style.code_font.clone())
            .text_size(px(style.code_size));
        if let Some(bg) = style.colors.code_background {
            d = d.bg(color(bg));
        }
        d.children(lines.into_iter().map(|l| {
            div()
                .whitespace_nowrap()
                .child(if l.is_empty() { " ".into() } else { l })
        }))
        .into_any_element()
    };
    if !content.signature.is_empty() {
        out.push(code_block(
            content.signature.lines().map(str::to_owned).collect(),
        ));
    }
    if !content.preview.is_empty() {
        out.push(code_block(content.preview.clone()));
    }
    let doc = &content.doc;
    let mut paras: Vec<String> = Vec::new();
    if !doc.summary.is_empty() {
        paras.extend(doc.summary.lines().map(str::to_owned));
    }
    for (name, body) in &doc.params {
        paras.push(text(
            strings,
            "nav.hover.param",
            &[("name", name), ("text", body)],
        ));
    }
    if !doc.returns.is_empty() {
        paras.push(text(
            strings,
            "nav.hover.returns",
            &[("text", &doc.returns)],
        ));
    }
    for (name, body) in &doc.exceptions {
        paras.push(text(
            strings,
            "nav.hover.exception",
            &[("name", name), ("text", body)],
        ));
    }
    if !doc.remarks.is_empty() {
        paras.extend(doc.remarks.lines().map(str::to_owned));
    }
    if content.long_line {
        paras.push(text(strings, "nav.hover.longLine", &[]));
    }
    if !paras.is_empty() {
        out.push(
            pad(div())
                .flex()
                .flex_col()
                .gap(px(4.0))
                .text_size(px(style.ui_size))
                .children(paras.into_iter().map(|p| div().child(p)))
                .into_any_element(),
        );
    }
    out
}

/// 底部状态行：只有近似结果才有（灰色小字“近似结果”）；精确结果不加任何标记。
pub fn status_row(
    content: Option<&HoverContent>,
    strings: &dyn Strings,
    style: &CardStyle,
) -> Option<AnyElement> {
    let content = content?;
    if !content.approximate {
        return None;
    }
    let mut d = div()
        .px(px(8.0))
        .py(px(2.0))
        .text_size(px(style.ui_size * 0.9))
        .child(text(strings, "nav.hover.approximate", &[]));
    if let Some(bg) = style.colors.hover_status_background {
        d = d.bg(color(bg));
    }
    if let Some(fg) = style.colors.description {
        d = d.text_color(color(fg));
    }
    Some(d.into_any_element())
}

/// 卡片外框（背景、前景、边框、阴影色），子元素由宿主放入。
pub fn frame(style: &CardStyle) -> gpui::Div {
    let mut d = div()
        .flex()
        .flex_col()
        .max_w(px(style.max_width))
        .max_h(px(style.max_height))
        .overflow_hidden()
        .rounded(px(3.0))
        .border_1();
    if let Some(bg) = style.colors.hover_background {
        d = d.bg(color(bg));
    }
    if let Some(fg) = style.colors.hover_foreground {
        d = d.text_color(color(fg));
    }
    if let Some(b) = style.colors.hover_border {
        d = d.border_color(color(b));
    }
    if let Some(s) = style.colors.shadow {
        d = d.shadow(vec![gpui::BoxShadow {
            color: color(s),
            offset: gpui::point(px(0.0), px(2.0)),
            blur_radius: px(8.0),
            spread_radius: px(0.0),
        }]);
    }
    d
}
