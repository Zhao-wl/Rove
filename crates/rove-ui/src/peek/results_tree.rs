//! 结果树的数据源（#18 的 [`TreeSource`]，不依赖 GPUI）：两级——文件节点（文件名、目录、
//! 该文件的结果数）→ 结果行（单行片段）。
//!
//! 文件节点直接来自 [`PeekResults::files`]（分组计数由数据层一次算出）；结果行只在文件展开时
//! 生成，且只生成这一个文件的。片段未读好时返回 `Pending` 并把文件序号发给宿主，宿主后台读完
//! 后经 `TreeModel::set_children` 推送（先显示加载行）。

use super::model::{Node, PeekResults};
use super::snippets::{FileSnippets, Snippet};
use crate::list_widget::tree::{ChildCount, Children, TreeItem, TreeSource};
use crate::lists::ItemId;
use std::sync::{Arc, Mutex};

pub struct ResultsTree {
    pub results: Arc<PeekResults>,
    snippets: Mutex<Vec<Option<Arc<FileSnippets>>>>,
    requests: async_channel::Sender<usize>,
}

impl ResultsTree {
    pub fn new(
        results: Arc<PeekResults>,
        loaded: impl IntoIterator<Item = (usize, Arc<FileSnippets>)>,
        requests: async_channel::Sender<usize>,
    ) -> Self {
        let mut snippets = vec![None; results.files.len()];
        for (file, s) in loaded {
            if let Some(slot) = snippets.get_mut(file) {
                *slot = Some(s);
            }
        }
        Self {
            results,
            snippets: Mutex::new(snippets),
            requests,
        }
    }

    pub fn store(&self, file: usize, snippets: Arc<FileSnippets>) {
        if let Some(slot) = self.lock().get_mut(file) {
            *slot = Some(snippets);
        }
    }

    pub fn is_loaded(&self, file: usize) -> bool {
        self.lock().get(file).is_some_and(Option::is_some)
    }

    /// 第 `item` 条结果的片段（文件已读好时）。
    pub fn snippet(&self, item: usize) -> Option<Snippet> {
        let file = self.results.items.get(item)?.file as usize;
        let start = self.results.files[file].start;
        let s = self.lock().get(file)?.clone()?;
        s.get(item - start).cloned()
    }

    /// 文件 `file` 的结果行（片段已读好时）。
    pub fn items(&self, file: usize) -> Option<Arc<[TreeItem]>> {
        let snippets = self.lock().get(file)?.clone()?;
        let start = self.results.files[file].start;
        Some(
            snippets
                .iter()
                .enumerate()
                .map(|(k, s)| TreeItem {
                    id: ItemId(Node::Item(start + k).id()),
                    label: s.text.clone(),
                    detail: None,
                    children: ChildCount::None,
                })
                .collect(),
        )
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Option<Arc<FileSnippets>>>> {
        self.snippets.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl TreeSource for ResultsTree {
    fn children(&self, parent: Option<&ItemId>) -> Children {
        let Some(parent) = parent else {
            return Children::Ready(
                self.results
                    .files
                    .iter()
                    .enumerate()
                    .map(|(i, f)| TreeItem {
                        id: ItemId(Node::File(i).id()),
                        label: f.label.clone(),
                        detail: Some(f.description.clone()),
                        children: ChildCount::Known(f.count),
                    })
                    .collect(),
            );
        };
        match Node::parse(&parent.0) {
            Some(Node::File(file)) => match self.items(file) {
                Some(items) => Children::Ready(items),
                None => {
                    let _ = self.requests.try_send(file);
                    Children::Pending
                }
            },
            _ => Children::Ready(Arc::from([])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list_widget::tree::TreeModel;
    use crate::lists::ListModel;
    use rove_base::{Line, Position, Resource, Span, Utf8Column, WorkspacePath};
    use rove_lang_cs::query::{Precision, Target};
    use rove_lang_cs::router::{RoutedTarget, TargetRole};

    fn results(files: usize, total: usize) -> Arc<PeekResults> {
        let targets: Vec<_> = (0..total)
            .map(|i| {
                let p = Position {
                    line: Line(i as u64),
                    column: Utf8Column(0),
                };
                let r = Span::new(p, p).unwrap();
                RoutedTarget {
                    target: Target {
                        resource: Resource::Workspace(
                            WorkspacePath::new(&format!("d/f{:04}.cs", i % files)).unwrap(),
                        ),
                        range: r.clone(),
                        selection: r,
                    },
                    precision: Precision::Exact,
                    reason: None,
                    role: TargetRole::Reference,
                }
            })
            .collect();
        Arc::new(PeekResults::group(&targets))
    }

    fn fake_snippets(r: &PeekResults, file: usize) -> Arc<FileSnippets> {
        Arc::new(
            r.items[r.files[file].items()]
                .iter()
                .map(|i| Snippet {
                    line: i.range.start().line.0,
                    text: "x".into(),
                    highlight: 0..1,
                })
                .collect(),
        )
    }

    #[test]
    fn only_files_become_nodes_until_expanded() {
        let r = results(3000, 47_000);
        let (send, receive) = async_channel::unbounded();
        let first = fake_snippets(&r, 0);
        let source = Arc::new(ResultsTree::new(r.clone(), [(0, first)], send));
        let mut tree = TreeModel::new(source.clone());
        assert_eq!(tree.count(), 3000, "初始只有文件节点");
        let row = &tree.rows(0..1).unwrap()[0];
        assert_eq!(&*row.label, "f0000.cs");
        assert!(row.expandable);
        // 第一个文件已有片段：展开立即就绪。
        tree.set_expanded(&ItemId(Node::File(0).id()), true)
            .unwrap();
        assert_eq!(tree.count(), 3000 + r.files[0].count);
        // 第二个文件没读过：展开时先显示加载行，并请求宿主读取。
        tree.set_expanded(&ItemId(Node::File(1).id()), true)
            .unwrap();
        assert_eq!(receive.try_recv(), Ok(1));
        source.store(1, fake_snippets(&r, 1));
        tree.set_children(Some(&ItemId(Node::File(1).id())), source.items(1).unwrap())
            .unwrap();
        assert_eq!(tree.count(), 3000 + r.files[0].count + r.files[1].count);
        assert_eq!(source.snippet(r.files[1].start).unwrap().text.as_ref(), "x");
    }
}
