//! Versioned, lazily materialized structure of JSON, JSONC, YAML, and XML.
mod diagnostics;
mod model;

pub use diagnostics::{ProblemKind, SyntaxProblem};

use self::model::{Entry, Model};
use crate::{Structure, StructureNode, StructureNodeId};
use rove_base::{
    ByteRange, Cancellation, Error, Line, Position, Result, Span, Task, TextRange, Utf8Column,
    Version,
};
use std::{
    ops::Range,
    sync::{Arc, Mutex},
};
use tree_sitter::{InputEdit, Language, Node, ParseOptions, Parser, Point, Tree};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructuredLanguage {
    Json,
    Jsonc,
    Yaml,
    Xml,
}

impl StructuredLanguage {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "json" => Some(Self::Json),
            "jsonc" => Some(Self::Jsonc),
            "yaml" => Some(Self::Yaml),
            "xml" => Some(Self::Xml),
            _ => None,
        }
    }

    pub(crate) fn grammar(self) -> Language {
        match self {
            Self::Json | Self::Jsonc => tree_sitter_json::LANGUAGE,
            Self::Yaml => tree_sitter_yaml::LANGUAGE,
            Self::Xml => tree_sitter_xml::LANGUAGE_XML,
        }
        .into()
    }
}

#[derive(Clone)]
pub struct ParsedStructure(Arc<Document>);

struct Document {
    language: StructuredLanguage,
    version: Version,
    text: Arc<str>,
    tree: Tree,
    lines: LineIndex,
    limit: usize,
    roots: Arc<[StructureNode]>,
    cache: Mutex<Vec<CachedNode>>,
}

struct CachedNode {
    container: Option<Location>,
    path: Arc<str>,
    children: Option<Arc<[StructureNode]>>,
}

struct Location {
    bytes: Range<usize>,
    kind: u16,
}

impl Location {
    fn node<'tree>(&self, tree: &'tree Tree) -> Option<Node<'tree>> {
        let mut node = tree
            .root_node()
            .descendant_for_byte_range(self.bytes.start, self.bytes.end)?;
        loop {
            if node.byte_range() == self.bytes && node.kind_id() == self.kind {
                return Some(node);
            }
            node = node.parent()?;
        }
    }
}

struct LineIndex(Vec<usize>);

impl LineIndex {
    fn new(text: &str, cancel: &Cancellation) -> Result<Self> {
        let mut starts = vec![0];
        let mut offset = 0;
        for chunk in text.as_bytes().chunks(1 << 16) {
            cancel.check()?;
            for relative in memchr::memchr2_iter(b'\n', b'\r', chunk) {
                let byte = offset + relative;
                if text.as_bytes()[byte] == b'\n' || text.as_bytes().get(byte + 1) != Some(&b'\n') {
                    starts.push(byte + 1);
                }
            }
            offset += chunk.len();
        }
        Ok(Self(starts))
    }

    fn at(&self, offset: usize) -> Position {
        let line = self.0.partition_point(|start| *start <= offset) - 1;
        Position {
            line: Line(line as u64),
            column: Utf8Column((offset - self.0[line]) as u64),
        }
    }
}

/// New snapshot and the invalidated ranges in its source revision.
pub struct StructureUpdate {
    pub snapshot: ParsedStructure,
    pub changed: Vec<TextRange>,
}

impl ParsedStructure {
    /// Parse on a background worker. Only the first layer of the model is materialized.
    pub fn parse(
        language: StructuredLanguage,
        version: Version,
        text: Arc<str>,
        max_items: usize,
        cancel: &Cancellation,
    ) -> Result<Self> {
        let tree = parse_tree(language, &text, None, cancel)?;
        Self::from_tree(language, version, text, tree, max_items, cancel)
    }

    fn from_tree(
        language: StructuredLanguage,
        version: Version,
        text: Arc<str>,
        tree: Tree,
        limit: usize,
        cancel: &Cancellation,
    ) -> Result<Self> {
        let lines = LineIndex::new(&text, cancel)?;
        let model = Model {
            language,
            text: &text,
            lines: &lines,
            limit,
            cancel,
        };
        let mut cache = Vec::new();
        let roots = cache_entries(&mut cache, model.roots(tree.root_node())?);
        Ok(Self(Arc::new(Document {
            language,
            version,
            text,
            tree,
            lines,
            limit,
            roots,
            cache: Mutex::new(cache),
        })))
    }

    /// Edits and returned positions use decoded UTF-8. Existing observers can keep reading the
    /// previous snapshot while tree-sitter reuses unchanged syntax for the new revision.
    pub fn reparse(
        &self,
        range: &ByteRange,
        inserted: &str,
        version: Version,
        cancel: &Cancellation,
    ) -> Result<StructureUpdate> {
        if version <= self.0.version {
            return Err(Error::Stale);
        }
        let start = usize::try_from(range.start().0).map_err(|_| Error::OutOfBounds)?;
        let end = usize::try_from(range.end().0).map_err(|_| Error::OutOfBounds)?;
        let before = &self.0.text;
        let prefix = before.get(..start).ok_or(Error::OutOfBounds)?;
        let suffix = before.get(end..).ok_or(Error::OutOfBounds)?;
        let mut text = String::with_capacity(prefix.len() + inserted.len() + suffix.len());
        text.push_str(prefix);
        text.push_str(inserted);
        text.push_str(suffix);
        let end_after = start + inserted.len();
        let mut old_tree = self.0.tree.clone();
        old_tree.edit(&InputEdit {
            start_byte: start,
            old_end_byte: end,
            new_end_byte: end_after,
            start_position: point_at(before, start),
            old_end_position: point_at(before, end),
            new_end_position: point_at(&text, end_after),
        });
        let tree = parse_tree(self.0.language, &text, Some(&old_tree), cancel)?;
        let mut changed: Vec<_> = old_tree
            .changed_ranges(&tree)
            .map(|change| change.start_byte..change.end_byte)
            .collect();
        // Two literals of the same grammar kind can have identical syntax and different previews.
        changed.push(start..end_after);
        changed.sort_unstable_by_key(|r| r.start);
        let snapshot = Self::from_tree(
            self.0.language,
            version,
            text.into(),
            tree,
            self.0.limit,
            cancel,
        )?;
        let mut combined: Vec<Range<usize>> = Vec::new();
        for change in changed {
            if let Some(last) = combined.last_mut().filter(|last| change.start <= last.end) {
                last.end = last.end.max(change.end);
            } else {
                combined.push(change);
            }
        }
        let changed = combined
            .into_iter()
            .map(|r| Span::new(snapshot.0.lines.at(r.start), snapshot.0.lines.at(r.end)))
            .collect::<Result<_>>()?;
        Ok(StructureUpdate { snapshot, changed })
    }

    /// Find the most specific structural node at this position. Only ancestor lists are loaded.
    pub fn node_at(
        &self,
        position: Position,
        cancel: &Cancellation,
    ) -> Result<Option<StructureNode>> {
        let mut nodes = self.0.roots.clone();
        let mut found = None;
        loop {
            cancel.check()?;
            let Some(node) = nodes.iter().find(|node| {
                let range = &node.symbol.range;
                range.start() <= position
                    && (position < range.end()
                        || (position == range.end() && range.start() == range.end()))
            }) else {
                return Ok(found);
            };
            found = Some(node.clone());
            if node.children.is_some() {
                return Ok(found);
            }
            nodes = self.load(node.id, cancel)?;
        }
    }

    /// Resolve a displayed JSONPath/XPath. Duplicate keys select the first occurrence.
    pub fn node_for_path(
        &self,
        path: &str,
        cancel: &Cancellation,
    ) -> Result<Option<StructureNode>> {
        let mut candidates = vec![self.0.roots.clone()];
        while let Some(nodes) = candidates.pop() {
            cancel.check()?;
            for node in nodes.iter() {
                if node.path.as_ref() == path {
                    return Ok(Some(node.clone()));
                }
            }
            for node in nodes.iter().rev() {
                let prefix = path.strip_prefix(node.path.as_ref());
                if prefix.is_some_and(|rest| rest.starts_with(['.', '[', '/']))
                    && node.children.is_none()
                {
                    candidates.push(self.load(node.id, cancel)?);
                }
            }
        }
        Ok(None)
    }

    fn load(&self, node: StructureNodeId, cancel: &Cancellation) -> Result<Arc<[StructureNode]>> {
        cancel.check()?;
        let mut cache = self
            .0
            .cache
            .lock()
            .expect("structure cache is not shared with external code");
        let index = usize::try_from(node.0).map_err(|_| Error::NotFound)?;
        let known = cache.get(index).ok_or(Error::NotFound)?;
        if let Some(cached) = &known.children {
            return Ok(cached.clone());
        }
        let Some(container) = &known.container else {
            return Ok(Arc::from([]));
        };
        let container = container.node(&self.0.tree).ok_or(Error::NotFound)?;
        let model = Model {
            language: self.0.language,
            text: &self.0.text,
            lines: &self.0.lines,
            limit: self.0.limit.saturating_sub(cache.len()),
            cancel,
        };
        let entries = model.children(container, &known.path)?;
        cancel.check()?;
        let children = cache_entries(&mut cache, entries);
        cache[index].children = Some(children.clone());
        Ok(children)
    }
}

impl Structure for ParsedStructure {
    fn version(&self) -> Version {
        self.0.version
    }

    fn roots(&self) -> Result<Arc<[StructureNode]>> {
        Ok(self.0.roots.clone())
    }

    fn children(
        &self,
        node: StructureNodeId,
        expected_version: Version,
        cancel: Cancellation,
    ) -> Task<Arc<[StructureNode]>> {
        let snapshot = self.clone();
        Box::pin(async move {
            cancel.check()?;
            if expected_version != snapshot.version() {
                return Err(Error::Stale);
            }
            snapshot.load(node, &cancel)
        })
    }
}

fn cache_entries(cache: &mut Vec<CachedNode>, entries: Vec<Entry<'_>>) -> Arc<[StructureNode]> {
    entries
        .into_iter()
        .map(|entry| {
            let id = StructureNodeId(cache.len() as u64);
            cache.push(CachedNode {
                path: Arc::clone(&entry.path),
                container: entry.container.map(|node| Location {
                    bytes: node.byte_range(),
                    kind: node.kind_id(),
                }),
                children: None,
            });
            entry.into_node(id)
        })
        .collect()
}

pub(crate) fn parse_tree(
    language: StructuredLanguage,
    text: &str,
    old: Option<&Tree>,
    cancel: &Cancellation,
) -> Result<Tree> {
    cancel.check()?;
    let mut parser = Parser::new();
    parser
        .set_language(&language.grammar())
        .expect("bundled grammar agrees with tree-sitter");
    let mut stop = |_: &_| cancel.check().is_err();
    let options = ParseOptions::new().progress_callback(&mut stop);
    let tree = parser.parse_with_options(
        &mut |offset, _| text.as_bytes().get(offset..).unwrap_or_default(),
        old,
        Some(options),
    );
    cancel.check()?;
    tree.ok_or(Error::NotReady)
}

fn point_at(text: &str, offset: usize) -> Point {
    let prefix = &text.as_bytes()[..offset];
    let breaks = memchr::memchr_iter(b'\n', prefix);
    let line_start = memchr::memrchr(b'\n', prefix).map_or(0, |p| p + 1);
    Point::new(breaks.count(), offset - line_start)
}
