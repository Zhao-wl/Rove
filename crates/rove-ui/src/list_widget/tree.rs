//! Lazily loaded tree flattened into the #1 [`ListModel`] contract (#18).
//!
//! Children are requested from the [`TreeSource`] only when a node is expanded. A source that
//! cannot answer synchronously returns [`Children::Pending`] and later pushes the result through
//! [`TreeModel::set_children`]; until then a loading placeholder row is shown under the node.
use super::matcher::Matcher;
use crate::lists::{ItemId, ListModel, Row};
use rove_base::{Error, Result, Version};
use std::{collections::HashMap, ops::Range, sync::Arc};

/// Whether a node has children. `Unknown` (scan/index still running) is distinct from a known
/// count, so the UI never claims "empty" before the source knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildCount {
    None,
    Unknown,
    Known(usize),
}

#[derive(Clone, Debug)]
pub struct TreeItem {
    /// Unique across the whole tree and stable across refreshes.
    pub id: ItemId,
    pub label: Arc<str>,
    pub detail: Option<Arc<str>>,
    pub children: ChildCount,
}

pub enum Children {
    Ready(Arc<[TreeItem]>),
    /// The source started its own background work and will call `TreeModel::set_children`.
    Pending,
}

/// Data source. Called on the UI thread; must return promptly (cached data or `Pending`).
pub trait TreeSource: Send + Sync {
    /// `None` = the root level.
    fn children(&self, parent: Option<&ItemId>) -> Children;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Item,
    /// Placeholder shown while a node's children are pending.
    Loading,
}

enum ChildState {
    NotLoaded,
    Loading,
    Loaded(Vec<u32>),
}

struct Node {
    item: TreeItem,
    parent: Option<u32>,
    depth: u32,
    expanded: bool,
    children: ChildState,
    alive: bool,
}

#[derive(Clone, Copy)]
enum Flat {
    Node(u32),
    Loading { parent: Option<u32> },
}

struct Filter {
    /// Per node: visible under the current query (matches or has a visible descendant).
    visible: Vec<bool>,
    /// Per node: shown expanded only because a descendant matches.
    forced: Vec<bool>,
}

pub struct TreeModel {
    source: Arc<dyn TreeSource>,
    nodes: Vec<Node>,
    /// Keyed by the ItemId text (ItemId itself is not Hash).
    index: HashMap<Arc<str>, u32>,
    roots: ChildState,
    flat: Vec<Flat>,
    /// node -> flat position + 1 (0 = not displayed).
    position: Vec<u32>,
    filter: Option<Filter>,
    revision: Version,
}

impl TreeModel {
    pub fn new(source: Arc<dyn TreeSource>) -> Self {
        let mut t = Self {
            source,
            nodes: vec![],
            index: HashMap::new(),
            roots: ChildState::NotLoaded,
            flat: vec![],
            position: vec![],
            filter: None,
            revision: Version(0),
        };
        t.load(None);
        t.rebuild();
        t
    }

    fn add_children(&mut self, parent: Option<u32>, items: &[TreeItem]) -> Vec<u32> {
        let depth = parent.map_or(0, |p| self.nodes[p as usize].depth + 1);
        let mut ids = Vec::with_capacity(items.len());
        self.nodes.reserve(items.len());
        self.index.reserve(items.len());
        for item in items {
            let n = self.nodes.len() as u32;
            self.index.insert(item.id.0.clone(), n);
            self.nodes.push(Node {
                item: item.clone(),
                parent,
                depth,
                expanded: false,
                children: if item.children == ChildCount::None {
                    ChildState::Loaded(vec![])
                } else {
                    ChildState::NotLoaded
                },
                alive: true,
            });
            ids.push(n);
        }
        ids
    }

    fn retire(&mut self, list: &[u32]) {
        let mut stack = list.to_vec();
        while let Some(n) = stack.pop() {
            let node = &mut self.nodes[n as usize];
            node.alive = false;
            if let ChildState::Loaded(c) =
                std::mem::replace(&mut node.children, ChildState::NotLoaded)
            {
                stack.extend(c);
            }
            let id = node.item.id.0.clone();
            if self.index.get(&id) == Some(&n) {
                self.index.remove(&id);
            }
        }
    }

    fn load(&mut self, parent: Option<u32>) {
        let id = parent.map(|p| self.nodes[p as usize].item.id.clone());
        match self.source.children(id.as_ref()) {
            Children::Ready(items) => self.store(parent, &items),
            Children::Pending => *self.child_state(parent) = ChildState::Loading,
        }
    }

    fn child_state(&mut self, parent: Option<u32>) -> &mut ChildState {
        match parent {
            None => &mut self.roots,
            Some(p) => &mut self.nodes[p as usize].children,
        }
    }

    fn store(&mut self, parent: Option<u32>, items: &[TreeItem]) {
        if let ChildState::Loaded(old) =
            std::mem::replace(self.child_state(parent), ChildState::NotLoaded)
        {
            self.retire(&old);
        }
        // Keep expansion of children that survive a refresh (same ItemId).
        let ids = self.add_children(parent, items);
        *self.child_state(parent) = ChildState::Loaded(ids);
    }

    /// Delivers children for a `Pending` (or refreshed) node. `None` = roots. Unknown parent =>
    /// NotFound (it may have been removed while loading; the caller drops the stale result).
    pub fn set_children(&mut self, parent: Option<&ItemId>, items: Arc<[TreeItem]>) -> Result<()> {
        let p = match parent {
            None => None,
            Some(id) => Some(*self.index.get(&*id.0).ok_or(Error::NotFound)?),
        };
        let keep: Vec<ItemId> = self.expanded_descendants(p);
        self.store(p, &items);
        for id in keep {
            if let Some(&n) = self.index.get(&*id.0) {
                self.nodes[n as usize].expanded = true;
                if matches!(self.nodes[n as usize].children, ChildState::NotLoaded) {
                    self.load(Some(n));
                }
            }
        }
        self.changed();
        Ok(())
    }

    fn expanded_descendants(&self, p: Option<u32>) -> Vec<ItemId> {
        let start = match p {
            None => &self.roots,
            Some(p) => &self.nodes[p as usize].children,
        };
        let mut out = vec![];
        let mut stack = match start {
            ChildState::Loaded(c) => c.clone(),
            _ => vec![],
        };
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            if node.expanded {
                out.push(node.item.id.clone());
                if let ChildState::Loaded(c) = &node.children {
                    stack.extend(c.iter().copied());
                }
            }
        }
        out
    }

    fn changed(&mut self) {
        self.revision.0 += 1;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.flat.clear();
        self.position.clear();
        self.position.resize(self.nodes.len(), 0);
        fn walk(t: &mut TreeModel, state: Option<u32>) {
            let list = match state {
                None => &t.roots,
                Some(p) => &t.nodes[p as usize].children,
            };
            let children: Vec<u32> = match list {
                ChildState::Loaded(c) => c.clone(),
                ChildState::Loading => {
                    t.flat.push(Flat::Loading { parent: state });
                    return;
                }
                ChildState::NotLoaded => return,
            };
            for n in children {
                if let Some(f) = &t.filter {
                    if !f.visible[n as usize] {
                        continue;
                    }
                }
                t.flat.push(Flat::Node(n));
                t.position[n as usize] = t.flat.len() as u32;
                if t.is_expanded_node(n) {
                    walk(t, Some(n));
                }
            }
        }
        walk(self, None);
    }

    fn is_expanded_node(&self, n: u32) -> bool {
        self.nodes[n as usize].expanded
            || self
                .filter
                .as_ref()
                .is_some_and(|f| f.forced.get(n as usize).copied().unwrap_or(false))
    }

    pub fn row_kind(&self, index: usize) -> Option<RowKind> {
        self.flat.get(index).map(|f| match f {
            Flat::Node(_) => RowKind::Item,
            Flat::Loading { .. } => RowKind::Loading,
        })
    }

    fn node_at(&self, index: usize) -> Option<u32> {
        match self.flat.get(index)? {
            Flat::Node(n) => Some(*n),
            Flat::Loading { .. } => None,
        }
    }

    pub fn index_of(&self, id: &ItemId) -> Option<usize> {
        let n = *self.index.get(&*id.0)?;
        match self.position.get(n as usize) {
            Some(&p) if p > 0 => Some(p as usize - 1),
            _ => None,
        }
    }

    /// Display index of `id`, or of its nearest displayed ancestor when `id` itself is hidden
    /// (collapsed parent, filtered out).
    pub fn visible_index_or_ancestor(&self, id: &ItemId) -> Option<usize> {
        let mut n = Some(*self.index.get(&*id.0)?);
        while let Some(k) = n {
            match self.position[k as usize] {
                0 => n = self.nodes[k as usize].parent,
                p => return Some(p as usize - 1),
            }
        }
        None
    }

    pub fn id_at(&self, index: usize) -> Option<ItemId> {
        self.node_at(index)
            .map(|n| self.nodes[n as usize].item.id.clone())
    }

    pub fn label_at(&self, index: usize) -> Option<&str> {
        self.node_at(index)
            .map(|n| &*self.nodes[n as usize].item.label)
    }

    pub fn can_expand(&self, index: usize) -> bool {
        self.node_at(index).is_some_and(|n| {
            self.nodes[n as usize].item.children != ChildCount::None && !self.is_expanded_node(n)
        })
    }

    pub fn can_collapse(&self, index: usize) -> bool {
        self.node_at(index).is_some_and(|n| {
            self.nodes[n as usize].item.children != ChildCount::None && self.is_expanded_node(n)
        })
    }

    pub fn is_expandable(&self, index: usize) -> bool {
        self.node_at(index)
            .is_some_and(|n| self.nodes[n as usize].item.children != ChildCount::None)
    }

    /// Display index of the parent row, if any (also for loading placeholders).
    pub fn parent_index(&self, index: usize) -> Option<usize> {
        let parent = match self.flat.get(index)? {
            Flat::Node(n) => self.nodes[*n as usize].parent,
            Flat::Loading { parent } => *parent,
        }?;
        match self.position[parent as usize] {
            0 => None,
            p => Some(p as usize - 1),
        }
    }

    /// Display index of the first visible child, if the node is expanded with visible children.
    pub fn first_child_index(&self, index: usize) -> Option<usize> {
        let n = self.node_at(index)?;
        if !self.is_expanded_node(n) {
            return None;
        }
        let next = index + 1;
        (self.parent_index(next) == Some(index)).then_some(next)
    }

    pub fn depth_at(&self, index: usize) -> u32 {
        match self.flat.get(index) {
            Some(Flat::Node(n)) => self.nodes[*n as usize].depth,
            Some(Flat::Loading { parent }) => {
                parent.map_or(0, |p| self.nodes[p as usize].depth + 1)
            }
            None => 0,
        }
    }

    /// Expands/collapses the node at `index`. Returns false when nothing changed (leaf, already in
    /// that state). Expanding loads children on demand.
    pub fn set_expanded_at(&mut self, index: usize, expanded: bool) -> bool {
        let Some(n) = self.node_at(index) else {
            return false;
        };
        self.set_expanded_node(n, expanded)
    }

    fn set_expanded_node(&mut self, n: u32, expanded: bool) -> bool {
        let node = &self.nodes[n as usize];
        if node.item.children == ChildCount::None || self.is_expanded_node(n) == expanded {
            return false;
        }
        if let Some(f) = self.filter.as_mut() {
            if let Some(forced) = f.forced.get_mut(n as usize) {
                *forced = false;
            }
        }
        self.nodes[n as usize].expanded = expanded;
        if expanded && matches!(self.nodes[n as usize].children, ChildState::NotLoaded) {
            self.load(Some(n));
        }
        self.changed();
        true
    }

    pub fn set_expanded(&mut self, id: &ItemId, expanded: bool) -> Result<bool> {
        let n = *self.index.get(&*id.0).ok_or(Error::NotFound)?;
        Ok(self.set_expanded_node(n, expanded))
    }

    pub fn is_expanded(&self, id: &ItemId) -> Option<bool> {
        self.index.get(&*id.0).map(|&n| self.is_expanded_node(n))
    }

    pub fn collapse_all(&mut self) {
        for node in &mut self.nodes {
            node.expanded = false;
        }
        if let Some(f) = self.filter.as_mut() {
            f.forced.iter_mut().for_each(|v| *v = false);
        }
        self.changed();
    }

    /// Visible display indices of the subtree under `scope` (None = whole tree), excluding scope.
    pub fn scope_range(&self, scope: Option<usize>) -> Range<usize> {
        let Some(s) = scope else {
            return 0..self.flat.len();
        };
        let depth = self.depth_at(s);
        let mut end = s + 1;
        while end < self.flat.len() && self.depth_at(end) > depth {
            end += 1;
        }
        s + 1..end
    }

    /// Ancestors (display indices, outermost first) of the row at `index`.
    pub fn ancestors(&self, index: usize) -> Vec<usize> {
        let mut out = vec![];
        let mut cur = self.parent_index(index);
        while let Some(p) = cur {
            out.push(p);
            cur = self.parent_index(p);
        }
        out.reverse();
        out
    }

    /// Applies a filter over the *loaded* nodes (VS Code does not load to filter). Matching nodes
    /// and their ancestors stay visible; ancestors are shown expanded. `None` restores the user's
    /// own expansion state untouched.
    pub fn set_filter(&mut self, filter: Option<(&dyn Matcher, &str)>) {
        self.filter = filter.map(|(m, q)| {
            let len = self.nodes.len();
            let mut visible = vec![false; len];
            let mut forced = vec![false; len];
            for n in (0..len).rev() {
                let node = &self.nodes[n];
                if !node.alive {
                    continue;
                }
                if m.is_match(q, &node.item.label) {
                    visible[n] = true;
                }
                if visible[n] {
                    // Children are always added after their parent, so reverse order sees them first.
                    if let Some(p) = node.parent {
                        visible[p as usize] = true;
                        forced[p as usize] = true;
                    }
                }
            }
            Filter { visible, forced }
        });
        self.changed();
    }
}

impl ListModel for TreeModel {
    fn revision(&self) -> Version {
        self.revision
    }
    fn count(&self) -> usize {
        self.flat.len()
    }
    fn rows(&self, r: Range<usize>) -> Result<Arc<[Row]>> {
        if r.start > r.end {
            return Err(Error::InvalidInput);
        }
        if r.start > self.flat.len() {
            return Err(Error::OutOfBounds);
        }
        let end = r.end.min(self.flat.len());
        Ok(self.flat[r.start..end]
            .iter()
            .map(|f| match *f {
                Flat::Node(n) => {
                    let node = &self.nodes[n as usize];
                    Row {
                        id: node.item.id.clone(),
                        label: node.item.label.clone(),
                        detail: node.item.detail.clone(),
                        depth: node.depth,
                        expandable: node.item.children != ChildCount::None,
                        expanded: self.is_expanded_node(n),
                        disabled: false,
                    }
                }
                Flat::Loading { parent } => Row {
                    id: ItemId(
                        format!(
                            "{}\u{1}loading",
                            parent.map_or("", |p| &*self.nodes[p as usize].item.id.0)
                        )
                        .into(),
                    ),
                    label: "".into(),
                    detail: None,
                    depth: parent.map_or(0, |p| self.nodes[p as usize].depth + 1),
                    expandable: false,
                    expanded: false,
                    disabled: true,
                },
            })
            .collect())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::list_widget::matcher::ContiguousMatcher;
    use std::sync::Mutex;

    /// Test source: `a/b/c` style paths; `pending` parents answer asynchronously.
    pub struct PathSource {
        pub paths: Vec<&'static str>,
        pub pending: Vec<&'static str>,
        pub calls: Mutex<Vec<Option<String>>>,
    }
    impl PathSource {
        pub fn new(paths: &[&'static str]) -> Self {
            Self {
                paths: paths.to_vec(),
                pending: vec![],
                calls: Mutex::new(vec![]),
            }
        }
        pub fn items(&self, parent: Option<&str>) -> Arc<[TreeItem]> {
            let prefix = parent.map(|p| format!("{p}/")).unwrap_or_default();
            self.paths
                .iter()
                .filter(|p| {
                    p.starts_with(&prefix)
                        && !p[prefix.len()..].contains('/')
                        && p.len() > prefix.len()
                })
                .map(|p| TreeItem {
                    id: ItemId((*p).into()),
                    label: p.rsplit('/').next().unwrap().into(),
                    detail: None,
                    children: if self.paths.iter().any(|q| q.starts_with(&format!("{p}/"))) {
                        ChildCount::Unknown
                    } else {
                        ChildCount::None
                    },
                })
                .collect()
        }
    }
    impl TreeSource for PathSource {
        fn children(&self, parent: Option<&ItemId>) -> Children {
            let p = parent.map(|p| p.0.to_string());
            self.calls.lock().unwrap().push(p.clone());
            if p.as_deref().is_some_and(|p| self.pending.contains(&p)) {
                return Children::Pending;
            }
            Children::Ready(self.items(p.as_deref()))
        }
    }

    fn labels(t: &TreeModel) -> Vec<String> {
        t.rows(0..t.count())
            .unwrap()
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), r.label))
            .collect()
    }

    #[test]
    fn children_are_requested_only_on_expand() {
        let src = Arc::new(PathSource::new(&[
            "src",
            "src/a.cs",
            "src/deep",
            "src/deep/x.cs",
            "b.cs",
        ]));
        let mut t = TreeModel::new(src.clone());
        assert_eq!(labels(&t), ["src", "b.cs"]);
        assert_eq!(src.calls.lock().unwrap().len(), 1);
        assert!(t.set_expanded_at(0, true));
        assert_eq!(labels(&t), ["src", "  a.cs", "  deep", "b.cs"]);
        assert_eq!(src.calls.lock().unwrap().len(), 2);
        assert!(!t.set_expanded_at(0, true), "already expanded");
        assert!(!t.set_expanded_at(1, true), "leaf");
        t.set_expanded_at(0, false);
        t.set_expanded_at(0, true);
        assert_eq!(
            src.calls.lock().unwrap().len(),
            2,
            "loaded children are cached"
        );
    }

    #[test]
    fn pending_children_show_a_loading_row_until_delivered() {
        let mut s = PathSource::new(&["src", "src/a.cs"]);
        s.pending = vec!["src"];
        let s = Arc::new(s);
        let mut t = TreeModel::new(s.clone());
        t.set_expanded_at(0, true);
        assert_eq!(t.row_kind(1), Some(RowKind::Loading));
        assert_eq!(t.parent_index(1), Some(0));
        t.set_children(Some(&ItemId("src".into())), s.items(Some("src")))
            .unwrap();
        assert_eq!(labels(&t), ["src", "  a.cs"]);
        assert_eq!(
            t.set_children(Some(&ItemId("gone".into())), Arc::from([])),
            Err(Error::NotFound)
        );
    }

    #[test]
    fn filter_reveals_ancestors_and_restores_expansion() {
        let src = Arc::new(PathSource::new(&[
            "a",
            "a/b",
            "a/b/needle.cs",
            "a/other.cs",
            "z.cs",
        ]));
        let mut t = TreeModel::new(src);
        t.set_expanded_at(0, true);
        t.set_expanded_at(1, true);
        t.set_expanded_at(0, false);
        assert_eq!(labels(&t), ["a", "z.cs"]);
        t.set_filter(Some((&ContiguousMatcher, "needle")));
        assert_eq!(labels(&t), ["a", "  b", "    needle.cs"]);
        t.set_filter(None);
        assert_eq!(labels(&t), ["a", "z.cs"]);
        t.set_expanded_at(0, true);
        assert_eq!(
            labels(&t),
            ["a", "  b", "    needle.cs", "  other.cs", "z.cs"]
        );
    }
}
