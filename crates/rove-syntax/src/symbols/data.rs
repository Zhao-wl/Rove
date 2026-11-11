//! JSON / JSONC 与 YAML：键名 + 值的种类（对象、数组、字符串、数字、布尔、null），与 VS Code 的 JSON
//! 大纲一致；数组元素以下标命名。YAML 的多文档（`---`）各成一棵子树，节点名是从 1 开始的序号，
//! 种类 [`kinds::DOCUMENT`]（界面用登记的字符串显示为“文档 N”）。
use super::{
    kinds,
    source::{named_children, span, Source},
};
use crate::{Symbol, SymbolKind};
use rove_base::Result;
use tree_sitter::Node;

fn other(kind: &str) -> SymbolKind {
    SymbolKind::Other(kind.into())
}

// ------------------------------------------------------------------ JSON

pub(super) fn json(src: &Source) -> Result<Vec<Symbol>> {
    let tree = src.parse(tree_sitter_json::LANGUAGE.into())?;
    let mut out = Vec::new();
    for value in named_children(tree.root_node()) {
        json_children(src, value, &mut out)?;
    }
    Ok(out)
}

fn json_kind(node: Option<Node>) -> SymbolKind {
    match node.map(|n| n.kind()) {
        Some("object") => SymbolKind::Object,
        Some("array") => SymbolKind::Array,
        Some("number") => other(kinds::NUMBER),
        Some("true" | "false") => other(kinds::BOOLEAN),
        Some("null") | None => other(kinds::NULL),
        _ => other(kinds::STRING),
    }
}

fn json_children(src: &Source, value: Node, out: &mut Vec<Symbol>) -> Result<()> {
    match value.kind() {
        "object" => {
            for pair in named_children(value) {
                if !src.room() {
                    break;
                }
                if pair.kind() != "pair" {
                    continue;
                }
                let key = pair.child_by_field_name("key");
                let val = pair.child_by_field_name("value");
                let mut children = Vec::new();
                if let Some(v) = val {
                    json_children(src, v, &mut children)?;
                }
                out.push(src.symbol(
                    key.map(|k| unquote(src.text(k))).unwrap_or_default(),
                    json_kind(val),
                    span(pair),
                    key.map_or(span(pair), span),
                    children,
                )?);
            }
        }
        "array" => {
            let items = named_children(value)
                .into_iter()
                .filter(|n| n.kind() != "comment");
            for (i, item) in items.enumerate() {
                if !src.room() {
                    break;
                }
                let mut children = Vec::new();
                json_children(src, item, &mut children)?;
                out.push(src.symbol(
                    i.to_string(),
                    json_kind(Some(item)),
                    span(item),
                    span(item),
                    children,
                )?);
            }
        }
        _ => {}
    }
    Ok(())
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(s);
    inner.to_string()
}

// ------------------------------------------------------------------ YAML

pub(super) fn yaml(src: &Source) -> Result<Vec<Symbol>> {
    let tree = src.parse(tree_sitter_yaml::LANGUAGE.into())?;
    let documents: Vec<Node> = named_children(tree.root_node())
        .into_iter()
        .filter(|n| n.kind() == "document")
        .collect();
    let mut out = Vec::new();
    if documents.len() <= 1 {
        for doc in documents {
            yaml_document(src, doc, &mut out)?;
        }
        return Ok(out);
    }
    for (i, doc) in documents.into_iter().enumerate() {
        if !src.room() {
            break;
        }
        let mut children = Vec::new();
        yaml_document(src, doc, &mut children)?;
        // 选择区间是文档的第一行（`--- !u!1 &123` 这类标记所在处）。
        let first_line_end = src.text[doc.start_byte()..doc.end_byte()]
            .find(['\r', '\n'])
            .map_or(doc.end_byte(), |n| doc.start_byte() + n);
        out.push(src.symbol(
            (i + 1).to_string(),
            other(kinds::DOCUMENT),
            span(doc),
            (doc.start_byte(), first_line_end),
            children,
        )?);
    }
    Ok(out)
}

fn yaml_document(src: &Source, doc: Node, out: &mut Vec<Symbol>) -> Result<()> {
    for node in named_children(doc) {
        if let Some(value) = yaml_value(node) {
            yaml_children(src, value, out)?;
        }
    }
    Ok(())
}

/// 去掉 block_node / flow_node 与锚点、标签，得到真正的值节点。
fn yaml_value(node: Node) -> Option<Node> {
    match node.kind() {
        "block_node" | "flow_node" => named_children(node)
            .into_iter()
            .find(|c| !matches!(c.kind(), "anchor" | "tag" | "comment"))
            .and_then(yaml_value),
        "anchor" | "tag" | "comment" => None,
        _ => Some(node),
    }
}

fn yaml_kind(value: Option<Node>) -> SymbolKind {
    let Some(value) = value else {
        return other(kinds::NULL);
    };
    match value.kind() {
        "block_mapping" | "flow_mapping" => SymbolKind::Object,
        "block_sequence" | "flow_sequence" => SymbolKind::Array,
        "plain_scalar" => match value.named_child(0).map(|c| c.kind()) {
            Some("boolean_scalar") => other(kinds::BOOLEAN),
            Some("integer_scalar" | "float_scalar") => other(kinds::NUMBER),
            Some("null_scalar") => other(kinds::NULL),
            _ => other(kinds::STRING),
        },
        "alias" => SymbolKind::Variable,
        _ => other(kinds::STRING),
    }
}

fn yaml_children(src: &Source, value: Node, out: &mut Vec<Symbol>) -> Result<()> {
    match value.kind() {
        "block_mapping" | "flow_mapping" => {
            for pair in named_children(value) {
                if !src.room() {
                    break;
                }
                if !matches!(pair.kind(), "block_mapping_pair" | "flow_pair") {
                    continue;
                }
                let key = pair.child_by_field_name("key");
                let val = pair.child_by_field_name("value").and_then(yaml_value);
                let mut children = Vec::new();
                if let Some(v) = val {
                    yaml_children(src, v, &mut children)?;
                }
                let name = key
                    .and_then(yaml_value)
                    .map(|k| unquote(src.text(k)))
                    .unwrap_or_default();
                out.push(src.symbol(
                    name,
                    yaml_kind(val),
                    span(pair),
                    key.map_or(span(pair), span),
                    children,
                )?);
            }
        }
        "block_sequence" | "flow_sequence" => {
            let items = named_children(value).into_iter().filter_map(|item| {
                if item.kind() == "block_sequence_item" {
                    named_children(item)
                        .into_iter()
                        .find_map(yaml_value)
                        .map(|v| (item, Some(v)))
                        .or(Some((item, None)))
                } else if item.kind() == "comment" {
                    None
                } else {
                    Some((item, yaml_value(item)))
                }
            });
            for (i, (item, val)) in items.enumerate() {
                if !src.room() {
                    break;
                }
                let mut children = Vec::new();
                if let Some(v) = val {
                    yaml_children(src, v, &mut children)?;
                }
                out.push(src.symbol(
                    i.to_string(),
                    yaml_kind(val),
                    span(item),
                    span(item),
                    children,
                )?);
            }
        }
        _ => {}
    }
    Ok(())
}
