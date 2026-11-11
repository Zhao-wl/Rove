//! Markdown（标题层级）与 XML（元素名）。
use super::{
    kinds,
    source::{named_children, span, Source},
};
use crate::{Symbol, SymbolKind};
use rove_base::Result;
use tree_sitter::Node;

/// Markdown：`#`–`######` 与 setext 标题，按层级嵌套；区间是标题所在小节（到下一个同级或更高级标题前）。
/// 种类与 VS Code 的 Markdown 大纲一致（`string`）。
pub(super) fn markdown(src: &Source) -> Result<Vec<Symbol>> {
    let tree = src.parse(tree_sitter_md::LANGUAGE.into())?;
    sections(src, tree.root_node())
}

fn sections(src: &Source, node: Node) -> Result<Vec<Symbol>> {
    let mut out = Vec::new();
    // 不是小节第一个子节点的标题（setext 标题不单独成节）：在下一个同级标题前收集更深的小节。
    let mut loose: Option<(Node, u8, Vec<Symbol>)> = None;
    let flush = |loose: &mut Option<(Node, u8, Vec<Symbol>)>, end: usize, out: &mut Vec<Symbol>| {
        if let Some((h, _, children)) = loose.take() {
            out.push(src.symbol(
                heading_text(src, h),
                SymbolKind::Other(kinds::STRING.into()),
                (h.start_byte(), end),
                span(h),
                children,
            )?);
        }
        Ok::<_, rove_base::Error>(())
    };
    for (i, child) in named_children(node).into_iter().enumerate() {
        match child.kind() {
            "atx_heading" | "setext_heading" if i > 0 => {
                let level = heading_level(child);
                flush(&mut loose, child.start_byte(), &mut out)?;
                loose = Some((child, level, Vec::new()));
            }
            "section" => {
                let heading = child
                    .named_child(0)
                    .filter(|h| matches!(h.kind(), "atx_heading" | "setext_heading"));
                let nested = sections(src, child)?;
                let target = match &mut loose {
                    Some((_, level, children))
                        if heading.is_some_and(|h| heading_level(h) > *level) =>
                    {
                        children
                    }
                    _ => {
                        flush(&mut loose, child.start_byte(), &mut out)?;
                        &mut out
                    }
                };
                match heading {
                    Some(h) => target.push(src.symbol(
                        heading_text(src, h),
                        SymbolKind::Other(kinds::STRING.into()),
                        span(child),
                        span(h),
                        nested,
                    )?),
                    // 第一个标题之前的内容自成一个无标题小节。
                    None => target.extend(nested),
                }
            }
            _ => {}
        }
    }
    flush(&mut loose, node.end_byte(), &mut out)?;
    Ok(out)
}

fn heading_level(heading: Node) -> u8 {
    for child in named_children(heading) {
        let kind = child.kind();
        if let Some(n) = kind
            .strip_prefix("atx_h")
            .and_then(|k| k.strip_suffix("_marker"))
        {
            return n.parse().unwrap_or(1);
        }
        match kind {
            "setext_h1_underline" => return 1,
            "setext_h2_underline" => return 2,
            _ => {}
        }
    }
    1
}

fn heading_text(src: &Source, heading: Node) -> String {
    let content = heading
        .child_by_field_name("heading_content")
        .map(|c| src.text(c))
        .unwrap_or_default();
    let mut text = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if heading.kind() == "atx_heading" {
        // 可选的结尾 `#` 序列（前面必须有空格）不属于标题文字。
        let trimmed = text.trim_end_matches('#');
        if trimmed.len() < text.len() && (trimmed.is_empty() || trimmed.ends_with(' ')) {
            text = trimmed.trim_end().to_string();
        }
    }
    text
}

/// XML：元素按嵌套产出，名称是标签名，种类 `field`（与常用 XML 语言服务一致）。属性不进入大纲。
pub(super) fn xml(src: &Source) -> Result<Vec<Symbol>> {
    let tree = src.parse(tree_sitter_xml::LANGUAGE_XML.into())?;
    let mut out = Vec::new();
    elements(src, tree.root_node(), &mut out)?;
    Ok(out)
}

fn elements(src: &Source, node: Node, out: &mut Vec<Symbol>) -> Result<()> {
    for child in named_children(node) {
        if !src.room() {
            return Ok(());
        }
        match child.kind() {
            "element" => out.push(element(src, child)?),
            // 语法错误时元素可能挂在 ERROR 下；内容节点也可能直接出现在容器里。
            "ERROR" | "content" => elements(src, child, out)?,
            _ => {}
        }
    }
    Ok(())
}

fn element(src: &Source, node: Node) -> Result<Symbol> {
    let tag = named_children(node)
        .into_iter()
        .find(|c| matches!(c.kind(), "STag" | "EmptyElemTag"));
    let name = tag.and_then(|t| named_children(t).into_iter().find(|c| c.kind() == "Name"));
    let mut children = Vec::new();
    for child in named_children(node) {
        if child.kind() == "content" {
            elements(src, child, &mut children)?;
        }
    }
    src.symbol(
        name.map(|n| src.text(n)).unwrap_or_default(),
        SymbolKind::Field,
        span(node),
        name.map_or(span(node), span),
        children,
    )
}
