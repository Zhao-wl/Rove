//! 悬停卡片的内容模型（不依赖 GPUI）：签名 + XML 文档注释（来自 #38 的 `hover`）、近似标记、
//! `Ctrl+悬停` 的定义预览，以及 XML 文档注释到纯文本段落的转换。
//!
//! 卡片只显示签名与文档，不显示诊断与快速修复（FEATURES §3）。精确结果没有任何精确度标记；
//! 只有近似结果在底部加一行灰色小字“近似结果”（UX_SPEC §2.2）。

use std::sync::Arc;

/// XML 文档注释拆成的几段（`<summary>`、`<param>`、`<returns>`、`<exception>`、`<remarks>`）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocParts {
    pub summary: String,
    pub params: Vec<(String, String)>,
    pub returns: String,
    pub exceptions: Vec<(String, String)>,
    pub remarks: String,
}

impl DocParts {
    pub fn is_empty(&self) -> bool {
        self.summary.is_empty()
            && self.params.is_empty()
            && self.returns.is_empty()
            && self.exceptions.is_empty()
            && self.remarks.is_empty()
    }
}

/// 卡片显示的内容。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverContent {
    /// 声明签名（代码字体）；L0 回退时是声明文本。
    pub signature: Arc<str>,
    pub doc: DocParts,
    /// `Ctrl+悬停` 的定义预览：定义处开始的若干行源码。
    pub preview: Vec<String>,
    /// 只有近似结果才为 true（底部灰色“近似结果”）。
    pub approximate: bool,
    /// 悬停的行超过 `editor.maxTokenizationLineLength`（`editor.hover.showLongLineWarning`）。
    pub long_line: bool,
}

impl HoverContent {
    pub fn is_empty(&self) -> bool {
        self.signature.is_empty()
            && self.doc.is_empty()
            && self.preview.is_empty()
            && !self.long_line
    }
}

/// 定义预览最多显示的行数（VS Code 的定义预览同样只截取开头几行）。
pub const PREVIEW_LINES: usize = 8;

/// XML 文档注释 → 段落。接受原始 XML 或带 `///` 前缀的注释行；不认识的标签保留其文本。
pub fn parse_xml_doc(raw: &str) -> DocParts {
    let text: String = raw
        .lines()
        .map(|l| {
            let t = l.trim_start();
            t.strip_prefix("///").map(str::trim_start).unwrap_or(t)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut parts = DocParts::default();
    let mut rest = text.as_str();
    let mut loose = String::new();
    while let Some(open) = rest.find('<') {
        loose.push_str(&rest[..open]);
        rest = &rest[open..];
        let Some(end) = rest.find('>') else {
            break;
        };
        let tag = &rest[1..end];
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        let self_closing = tag.ends_with('/');
        let after = &rest[end + 1..];
        let block = matches!(
            name,
            "summary" | "param" | "returns" | "exception" | "remarks" | "value" | "typeparam"
        );
        if block && !self_closing {
            let close = format!("</{name}>");
            let (body, next) = match after.find(&close) {
                Some(i) => (&after[..i], &after[i + close.len()..]),
                None => (after, ""),
            };
            let body = inline_text(body);
            let attr = attribute(tag, "name").or_else(|| attribute(tag, "cref").map(short_cref));
            match name {
                "summary" | "value" => push_para(&mut parts.summary, &body),
                "remarks" => push_para(&mut parts.remarks, &body),
                "returns" => push_para(&mut parts.returns, &body),
                "param" | "typeparam" => parts.params.push((attr.unwrap_or_default(), body)),
                "exception" => parts.exceptions.push((attr.unwrap_or_default(), body)),
                _ => {}
            }
            rest = next;
        } else {
            // 行内标签出现在块外（没有 <summary> 的注释）：按行内规则处理。
            loose.push_str(&inline_tag(tag));
            rest = after;
        }
    }
    loose.push_str(rest);
    let loose = decode_entities(&loose)
        .split(PARA)
        .map(collapse)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if !loose.is_empty() {
        push_para(&mut parts.summary, &loose);
    }
    parts
}

/// `<para>` 的段落分隔标记（内部使用，不会出现在输出里）。
const PARA: char = '\u{1}';

fn push_para(into: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !into.is_empty() {
        into.push('\n');
    }
    into.push_str(text);
}

/// 块内文本：行内标签换成文字，`<para>` / `<br/>` 换成换行，空白折叠。
fn inline_text(body: &str) -> String {
    // 源码里的换行只是排版；段落只由 `<para>` / `<br/>` 产生（PARA 标记）。
    let body = body.replace(['\r', '\n'], " ");
    let mut out = String::new();
    let mut rest = body.as_str();
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let Some(end) = rest[open..].find('>') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let tag = &rest[open + 1..open + end];
        out.push_str(&inline_tag(tag));
        rest = &rest[open + end + 1..];
    }
    out.push_str(rest);
    decode_entities(&out)
        .split(PARA)
        .map(collapse)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 行内标签的文字：`<see cref="T:A.B"/>` → `B`，`<paramref name="x"/>` → `x`，`<para>` → 换行，其余丢弃标签保留内容。
fn inline_tag(tag: &str) -> String {
    let name = tag
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or("");
    match name {
        "see" | "seealso" => attribute(tag, "cref")
            .map(short_cref)
            .or_else(|| attribute(tag, "langword"))
            .or_else(|| attribute(tag, "href"))
            .unwrap_or_default(),
        "paramref" | "typeparamref" => attribute(tag, "name").unwrap_or_default(),
        "para" | "br" if !tag.starts_with('/') => PARA.to_string(),
        _ => String::new(),
    }
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}="))?;
    let rest = &tag[at + name.len() + 1..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = rest[1..].find(quote)?;
    Some(decode_entities(&rest[1..1 + end]))
}

/// `T:UnityEngine.Transform` → `Transform`；`M:A.B.C(System.Int32)` → `C`。
fn short_cref(cref: String) -> String {
    let body = cref.split_once(':').map_or(cref.as_str(), |(_, b)| b);
    let body = body.split('(').next().unwrap_or(body);
    body.rsplit('.').next().unwrap_or(body).to_string()
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 悬停卡片的滚动状态（`editor.action.*Hover` 命令）。偏移以像素计，绘制时钳制到内容范围。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HoverScroll {
    pub y: f32,
    pub x: f32,
}

/// 卡片内滚动命令（VS Code `ContentHoverController` 的键盘导航）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoverMove {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

impl HoverScroll {
    /// 按一行高 `line`、一页高 `page`、内容高 `content`、可见高 `visible` 移动并钳制。
    pub fn apply(
        &mut self,
        m: HoverMove,
        line: f32,
        page: f32,
        content: (f32, f32),
        visible: (f32, f32),
    ) {
        let max_y = (content.1 - visible.1).max(0.0);
        let max_x = (content.0 - visible.0).max(0.0);
        match m {
            HoverMove::Up => self.y -= line,
            HoverMove::Down => self.y += line,
            HoverMove::Left => self.x -= line * 2.0,
            HoverMove::Right => self.x += line * 2.0,
            HoverMove::PageUp => self.y -= page,
            HoverMove::PageDown => self.y += page,
            HoverMove::Top => self.y = 0.0,
            HoverMove::Bottom => self.y = max_y,
        }
        self.y = self.y.clamp(0.0, max_y);
        self.x = self.x.clamp(0.0, max_x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_doc_comments_become_paragraphs() {
        let d = parse_xml_doc(
            "/// <summary>Moves the <see cref=\"T:Game.Player\"/> by\n/// <paramref name=\"speed\"/> units.<para>Second &lt;line&gt;.</para></summary>\n/// <param name=\"speed\">How fast.</param>\n/// <returns>Nothing <c>null</c>.</returns>\n/// <exception cref=\"T:System.ArgumentException\">Bad.</exception>",
        );
        assert_eq!(
            d.summary,
            "Moves the Player by speed units.\nSecond <line>."
        );
        assert_eq!(d.params, vec![("speed".into(), "How fast.".into())]);
        assert_eq!(d.returns, "Nothing null.");
        assert_eq!(
            d.exceptions,
            vec![("ArgumentException".into(), "Bad.".into())]
        );
    }

    #[test]
    fn plain_text_and_empty_docs() {
        assert_eq!(parse_xml_doc("Just text.").summary, "Just text.");
        assert!(parse_xml_doc("").is_empty());
        assert_eq!(
            parse_xml_doc("<see langword=\"null\"/> or text").summary,
            "null or text"
        );
    }

    #[test]
    fn hover_scroll_is_clamped() {
        let mut s = HoverScroll::default();
        s.apply(HoverMove::Up, 10.0, 100.0, (300.0, 500.0), (200.0, 200.0));
        assert_eq!(s.y, 0.0);
        s.apply(
            HoverMove::PageDown,
            10.0,
            100.0,
            (300.0, 500.0),
            (200.0, 200.0),
        );
        assert_eq!(s.y, 100.0);
        s.apply(
            HoverMove::Bottom,
            10.0,
            100.0,
            (300.0, 500.0),
            (200.0, 200.0),
        );
        assert_eq!(s.y, 300.0);
        s.apply(HoverMove::Down, 10.0, 100.0, (300.0, 500.0), (200.0, 200.0));
        assert_eq!(s.y, 300.0);
        s.apply(
            HoverMove::Right,
            10.0,
            100.0,
            (300.0, 500.0),
            (200.0, 200.0),
        );
        assert_eq!(s.x, 20.0);
        s.apply(HoverMove::Top, 10.0, 100.0, (300.0, 500.0), (200.0, 200.0));
        assert_eq!(s.y, 0.0);
    }
}
