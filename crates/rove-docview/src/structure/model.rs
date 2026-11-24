use crate::{StructureNode, StructureNodeId};
use rove_base::{Cancellation, Result, Span};
use rove_syntax::{symbols::kinds, Symbol, SymbolKind};
use std::{collections::HashMap, sync::Arc};
use tree_sitter::Node;

use super::{LineIndex, Location, StructuredLanguage};

pub(super) struct Model<'a> {
    pub language: StructuredLanguage,
    pub text: &'a str,
    pub lines: &'a LineIndex,
    pub limit: usize,
    pub cancel: &'a Cancellation,
}

pub(super) struct Entry {
    pub symbol: Symbol,
    pub path: Arc<str>,
    pub preview: Arc<str>,
    pub value: Option<std::ops::Range<usize>>,
    pub(super) container: Option<Location>,
}

impl Entry {
    pub fn into_node(self, id: StructureNodeId) -> StructureNode {
        StructureNode {
            id,
            symbol: self.symbol,
            path: self.path,
            value_preview: self.preview,
            children: self.container.is_none().then(|| Arc::from([])),
        }
    }
}

pub(super) fn named_children(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    // The cursor owns its traversal state. Visiting siblings stays linear for wide arrays.
    let mut cursor = node.walk();
    let mut more = cursor.goto_first_child();
    std::iter::from_fn(move || {
        while more {
            let child = cursor.node();
            more = cursor.goto_next_sibling();
            if child.is_named() {
                return Some(child);
            }
        }
        None
    })
}

struct Label {
    name: Arc<str>,
    path: Arc<str>,
    kind: SymbolKind,
    preview: Arc<str>,
    value: Option<std::ops::Range<usize>>,
}

impl Model<'_> {
    pub fn roots<'tree>(&self, root: Node<'tree>) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        match self.language {
            StructuredLanguage::Json | StructuredLanguage::Jsonc => {
                for node in named_children(root).filter(|n| n.kind() != "comment") {
                    self.data_root(node, "$", &mut out)?;
                }
            }
            StructuredLanguage::Yaml => {
                let mut documents = named_children(root)
                    .filter(|n| n.kind() == "document")
                    .peekable();
                if let Some(doc) = documents.next() {
                    if documents.peek().is_none() {
                        if let Some(value) = named_children(doc).find_map(yaml_value) {
                            self.data_root(value, "$", &mut out)?;
                        }
                    } else {
                        for (i, doc) in std::iter::once(doc)
                            .chain(documents)
                            .take(self.limit)
                            .enumerate()
                        {
                            let path = format!("$[{}]", i);
                            let mut selection = doc.byte_range();
                            selection.end = self.text[selection.clone()]
                                .find(['\n', '\r'])
                                .map_or(selection.end, |end| selection.start + end);
                            out.push(self.entry(
                                doc,
                                selection,
                                Label {
                                    name: (i + 1).to_string().into(),
                                    path: path.into(),
                                    kind: other(kinds::DOCUMENT),
                                    preview: "".into(),
                                    value: Some(doc.byte_range()),
                                },
                                Some(doc),
                            )?);
                        }
                    }
                }
            }
            StructuredLanguage::Xml => self.xml_children(root, "", &mut out)?,
        }
        Ok(out)
    }

    pub fn children<'tree>(&self, node: Node<'tree>, path: &str) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        if self.language == StructuredLanguage::Xml {
            self.xml_children(node, path, &mut out)?;
        } else if node.kind() == "document" {
            if let Some(value) = named_children(node).find_map(yaml_value) {
                self.data_root(value, path, &mut out)?;
            }
        } else {
            self.data_children(node, path, &mut out)?;
        }
        Ok(out)
    }

    fn data_root<'tree>(&self, node: Node<'tree>, path: &str, out: &mut Vec<Entry>) -> Result<()> {
        if container_kind(node).is_some() {
            self.data_children(node, path, out)
        } else if node.is_error() {
            for child in named_children(node) {
                self.data_root(child, path, out)?;
            }
            Ok(())
        } else if out.len() < self.limit {
            let kind = data_kind(Some(node));
            out.push(self.entry(
                node,
                node.byte_range(),
                Label {
                    name: "$".into(),
                    path: path.into(),
                    kind,
                    preview: self.scalar(node),
                    value: Some(node.byte_range()),
                },
                None,
            )?);
            Ok(())
        } else {
            Ok(())
        }
    }

    fn data_children<'tree>(
        &self,
        node: Node<'tree>,
        parent: &str,
        out: &mut Vec<Entry>,
    ) -> Result<()> {
        let is_object = container_kind(node) == Some(SymbolKind::Object);
        for (i, child) in named_children(node)
            .filter(|child| is_data_item(*child, is_object))
            .take(self.limit.saturating_sub(out.len()))
            .enumerate()
        {
            self.cancel.check()?;
            let (name, key, value) = if is_object {
                let key = child.child_by_field_name("key");
                let name = key.map(|node| self.key(node)).unwrap_or_default();
                (
                    name,
                    key,
                    child.child_by_field_name("value").and_then(yaml_value),
                )
            } else {
                let value = if child.kind() == "block_sequence_item" {
                    named_children(child).find_map(yaml_value)
                } else {
                    yaml_value(child)
                };
                (i.to_string(), None, value)
            };
            let path = if is_object {
                key_path(parent, &name)
            } else {
                format!("{parent}[{i}]")
            };
            let kind = data_kind(value);
            let container = value.filter(|value| container_kind(*value).is_some());
            let preview = match container {
                Some(container) => {
                    let is_object = kind == SymbolKind::Object;
                    let count = self.count_items(container, is_object)?;
                    if is_object {
                        format!("{{{count}}}").into()
                    } else {
                        format!("[{count}]").into()
                    }
                }
                None => value.map_or_else(|| "null".into(), |node| self.scalar(node)),
            };
            out.push(self.entry(
                child,
                key.map_or_else(|| child.byte_range(), |key| key.byte_range()),
                Label {
                    name: name.into(),
                    path: path.into(),
                    kind,
                    preview,
                    value: value.map(|node| node.byte_range()),
                },
                container,
            )?);
        }
        Ok(())
    }

    fn count_items(&self, node: Node<'_>, is_object: bool) -> Result<usize> {
        let mut count = 0;
        for (i, child) in named_children(node).enumerate() {
            if i % 256 == 0 {
                self.cancel.check()?;
            }
            if is_data_item(child, is_object) {
                count += 1;
            }
        }
        Ok(count)
    }

    fn xml_children<'tree>(
        &self,
        node: Node<'tree>,
        parent: &str,
        out: &mut Vec<Entry>,
    ) -> Result<()> {
        if let Some(tag) = xml_tag(node) {
            for attr in named_children(tag)
                .filter(|n| n.kind() == "Attribute")
                .take(self.limit)
            {
                if let Some(name) = named_children(attr).find(|n| n.kind() == "Name") {
                    let value = named_children(attr).find(|n| n.kind() == "AttValue");
                    let name_text = &self.text[name.byte_range()];
                    out.push(self.entry(
                        attr,
                        name.byte_range(),
                        Label {
                            name: name_text.into(),
                            path: format!("{parent}/@{name_text}").into(),
                            kind: SymbolKind::Property,
                            preview: value.map_or_else(|| "".into(), |node| self.scalar(node)),
                            value: value.map(|node| node.byte_range()),
                        },
                        None,
                    )?);
                }
            }
        }
        let mut names = HashMap::<&str, usize>::new();
        let mut children = Vec::new();
        visit_xml_elements(node, |element| {
            self.cancel.check()?;
            if let Some(tag) = xml_tag(element) {
                if let Some(name) = named_children(tag).find(|n| n.kind() == "Name") {
                    let key = &self.text[name.byte_range()];
                    let index = names.entry(key).or_default();
                    *index += 1;
                    if children.len() < self.limit.saturating_sub(out.len()) {
                        children.push((element, tag, name, key, *index));
                    }
                }
            }
            Ok(())
        })?;
        for (element, tag, name, key, index) in children {
            let path = if names[key] == 1 {
                format!("{parent}/{key}")
            } else {
                format!("{parent}/{key}[{index}]")
            };
            let mut count = 0;
            visit_xml_elements(element, |_| {
                self.cancel.check()?;
                count += 1;
                Ok(())
            })?;
            let mut preview = Vec::new();
            let mut has_attributes = false;
            for attr in named_children(tag).filter(|n| n.kind() == "Attribute") {
                self.cancel.check()?;
                has_attributes = true;
                let name = named_children(attr).find(|n| n.kind() == "Name");
                let value = named_children(attr).find(|n| n.kind() == "AttValue");
                if let (Some(name), Some(value)) = (name, value) {
                    let name = &self.text[name.byte_range()];
                    if matches!(name, "id" | "name") {
                        preview.push(format!("{name}={}", self.scalar(value)));
                    }
                }
            }
            preview.push(format!("[{count}]"));
            out.push(self.entry(
                element,
                name.byte_range(),
                Label {
                    name: key.into(),
                    path: path.into(),
                    kind: SymbolKind::Field,
                    preview: preview.join(" ").into(),
                    value: Some(element.byte_range()),
                },
                (has_attributes || count != 0).then_some(element),
            )?);
        }
        Ok(())
    }

    fn entry<'tree>(
        &self,
        full: Node<'tree>,
        selection: std::ops::Range<usize>,
        label: Label,
        container: Option<Node<'tree>>,
    ) -> Result<Entry> {
        self.cancel.check()?;
        let range = full.byte_range();
        Ok(Entry {
            symbol: Symbol {
                name: label.name,
                kind: label.kind,
                range: Span::new(self.lines.at(range.start), self.lines.at(range.end))?,
                selection: Span::new(self.lines.at(selection.start), self.lines.at(selection.end))?,
                children: Arc::from([]),
            },
            path: label.path,
            preview: label.preview,
            value: label.value,
            container: container.map(|node| Location::Tree {
                bytes: node.byte_range(),
                kind: node.kind_id(),
            }),
        })
    }

    fn scalar(&self, node: Node<'_>) -> Arc<str> {
        preview(&self.text[node.byte_range()])
    }

    fn key(&self, node: Node<'_>) -> String {
        let raw = self.text[node.byte_range()].trim();
        if self.language == StructuredLanguage::Yaml {
            let raw = yaml_value(node).map_or(raw, |n| self.text[n.byte_range()].trim());
            if let Some(s) = raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                return s.replace("''", "'");
            }
            if let Ok(s) = serde_json::from_str::<String>(raw) {
                return s;
            }
            return raw.to_owned();
        }
        serde_json::from_str(raw).unwrap_or_else(|_| raw.trim_matches('"').to_owned())
    }
}

pub(super) fn preview(text: &str) -> Arc<str> {
    let text = text.trim();
    let end = text.char_indices().nth(64).map_or(text.len(), |(at, _)| at);
    let compact = text[..end].replace(['\r', '\n', '\t'], " ");
    if end == text.len() {
        compact.into()
    } else {
        format!("{compact}…").into()
    }
}

pub(super) fn key_path(parent: &str, name: &str) -> String {
    let mut chars = name.chars();
    if chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
    {
        format!("{parent}.{name}")
    } else {
        format!(
            "{parent}[{}]",
            serde_json::to_string(name).expect("serializable text")
        )
    }
}

fn container_kind(node: Node<'_>) -> Option<SymbolKind> {
    match node.kind() {
        "object" | "block_mapping" | "flow_mapping" => Some(SymbolKind::Object),
        "array" | "block_sequence" | "flow_sequence" => Some(SymbolKind::Array),
        _ => None,
    }
}

fn is_data_item(node: Node<'_>, is_object: bool) -> bool {
    if is_object {
        matches!(node.kind(), "pair" | "block_mapping_pair" | "flow_pair")
    } else {
        node.kind() != "comment"
    }
}

fn other(kind: &str) -> SymbolKind {
    SymbolKind::Other(kind.into())
}

fn data_kind(value: Option<Node<'_>>) -> SymbolKind {
    let Some(value) = value else {
        return other(kinds::NULL);
    };
    container_kind(value).unwrap_or_else(|| match value.kind() {
        "number" => other(kinds::NUMBER),
        "true" | "false" => other(kinds::BOOLEAN),
        "null" => other(kinds::NULL),
        "plain_scalar" => match value.named_child(0).map(|c| c.kind()) {
            Some("boolean_scalar") => other(kinds::BOOLEAN),
            Some("integer_scalar" | "float_scalar") => other(kinds::NUMBER),
            Some("null_scalar") => other(kinds::NULL),
            _ => other(kinds::STRING),
        },
        "alias" => SymbolKind::Variable,
        _ => other(kinds::STRING),
    })
}

fn yaml_value(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "block_node" | "flow_node" => named_children(node).find_map(yaml_value),
        "anchor" | "tag" | "comment" => None,
        _ => Some(node),
    }
}

fn xml_tag(node: Node<'_>) -> Option<Node<'_>> {
    (node.kind() == "element")
        .then(|| named_children(node).find(|child| matches!(child.kind(), "STag" | "EmptyElemTag")))
        .flatten()
}

fn visit_xml_elements<'tree>(
    node: Node<'tree>,
    mut visit: impl FnMut(Node<'tree>) -> Result<()>,
) -> Result<()> {
    let mut stack = vec![named_children(node)];
    while let Some(children) = stack.last_mut() {
        match children.next() {
            Some(child) if child.kind() == "element" => visit(child)?,
            Some(child) if matches!(child.kind(), "ERROR" | "content") => {
                stack.push(named_children(child));
            }
            Some(_) => {}
            None => {
                stack.pop();
            }
        }
    }
    Ok(())
}
