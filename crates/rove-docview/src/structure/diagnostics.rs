use super::{model::named_children, parse_tree, ParsedStructure, StructuredLanguage};
use rove_base::{Cancellation, Result, Span, TextRange};
use std::{collections::HashSet, ops::Range};
use tree_sitter::Node;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    UnexpectedSyntax,
    MissingSyntax,
    JsonComment,
    JsonRoot,
    MismatchedXmlTag,
    DuplicateXmlAttribute,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxProblem {
    pub kind: ProblemKind,
    pub range: TextRange,
}

impl ParsedStructure {
    /// Read syntax diagnostics from this revision. Grammar diagnostics contain decoded UTF-8
    /// positions; presentation owns the localized explanation.
    pub fn diagnostics(&self, cancel: &Cancellation) -> Result<Vec<SyntaxProblem>> {
        let doc = &self.0;
        let parsed;
        let tree = match &doc.tree {
            Some(tree) => tree,
            None => {
                parsed = parse_tree(doc.language, &doc.text, None, cancel)?;
                &parsed
            }
        };
        let root = tree.root_node();
        let mut raw = Vec::new();
        let mut stack = vec![root];
        let walk_all = !matches!(
            doc.language,
            StructuredLanguage::Jsonc | StructuredLanguage::Yaml
        );
        while let Some(node) = stack.pop() {
            cancel.check()?;
            if node.is_missing() {
                raw.push((ProblemKind::MissingSyntax, node.byte_range()));
                continue;
            }
            if node.is_error() {
                raw.push((ProblemKind::UnexpectedSyntax, node.byte_range()));
            }
            if doc.language == StructuredLanguage::Json && node.kind() == "comment" {
                raw.push((ProblemKind::JsonComment, node.byte_range()));
            }
            if doc.language == StructuredLanguage::Xml && node.kind() == "element" {
                xml_element(&doc.text, node, &mut raw);
            }
            if walk_all || node.has_error() {
                let mut children = node.walk();
                stack.extend(node.children(&mut children));
            }
        }
        if matches!(
            doc.language,
            StructuredLanguage::Json | StructuredLanguage::Jsonc
        ) {
            let mut values = named_children(root).filter(|n| n.kind() != "comment");
            if values.next().is_none() {
                raw.push((ProblemKind::JsonRoot, root.byte_range()));
            }
            raw.extend(values.map(|node| (ProblemKind::JsonRoot, node.byte_range())));
        }
        raw.sort_by_key(|(_, span)| (span.start, span.end));
        raw.dedup();
        raw.into_iter()
            .map(|(kind, span)| {
                Ok(SyntaxProblem {
                    kind,
                    range: Span::new(doc.lines.at(span.start), doc.lines.at(span.end))?,
                })
            })
            .collect()
    }
}

fn xml_element(text: &str, node: Node<'_>, raw: &mut Vec<(ProblemKind, Range<usize>)>) {
    let tag = named_children(node).find(|n| matches!(n.kind(), "STag" | "EmptyElemTag"));
    let Some(tag) = tag else {
        return;
    };
    let name = named_children(tag).find(|n| n.kind() == "Name");
    let closing = named_children(node)
        .find(|n| n.kind() == "ETag")
        .and_then(|end| named_children(end).find(|n| n.kind() == "Name"));
    if let (Some(start), Some(end)) = (name, closing) {
        if text[start.byte_range()] != text[end.byte_range()] {
            raw.push((ProblemKind::MismatchedXmlTag, end.byte_range()));
        }
    }
    let mut names = HashSet::new();
    for attr in named_children(tag).filter(|n| n.kind() == "Attribute") {
        if let Some(name) = named_children(attr).find(|n| n.kind() == "Name") {
            if !names.insert(&text[name.byte_range()]) {
                raw.push((ProblemKind::DuplicateXmlAttribute, name.byte_range()));
            }
        }
    }
}
