//! 侧边「引用」视图的数据层（#60，不依赖 GPUI）：结果与历史、逐条导航、消除某一条、
//! 复制文案，以及两个上下文键的取值。
//!
//! 结果用的是 #41 已经做好的分组索引 [`PeekResults`]（文件 → 计数 + 偏移），**不把
//! `Vec<Location>` 展平成界面节点**：4.7 万条结果下慢的是树的构建，不是滚动（issue #60 风险与提示）。

use crate::peek::model::{FileGroup, PeekItem, PeekKind, PeekResults};
use rove_base::Resource;
use rove_i18n::Message;
use rove_lang_cs::query::{Precision, QueryPoint};
use std::sync::Arc;

/// 历史记录最多留多少条查询（VS Code 的引用视图同样保留最近的若干条）。
pub const HISTORY_LIMIT: usize = 20;

/// 视图当前显示的是结果树还是历史列表（`references-view.pickFromHistory`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Results,
    History,
}

/// 一次查找的全部产物：重新运行需要的查询点，加上分组好的结果。
#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: PeekKind,
    /// 标题里的符号名。
    pub symbol: Arc<str>,
    /// 重新运行（`refind` / `refresh`）用的查询点。
    pub point: QueryPoint,
    pub results: Arc<PeekResults>,
    /// 批级精确度：空结果时区分“确实没有”与“不能确定”。
    pub precision: Precision,
}

impl Entry {
    /// 标题：符号名 + 结果数。
    pub fn title(&self) -> Message {
        let key = match self.kind {
            PeekKind::Implementation => "references.title.implementations",
            _ => "references.title.references",
        };
        Message {
            key: key.into(),
            args: Arc::from([
                ("symbol".into(), self.symbol.clone()),
                ("count".into(), self.results.total().to_string().into()),
            ]),
        }
    }

    /// 标题上的近似标记（`UX_SPEC` §2.2）。精确结果不加任何标记。
    pub fn marker(&self) -> Option<Message> {
        (self.results.precision() != Precision::Exact)
            .then(|| Message::key("references.marker.approximate"))
    }

    /// 空结果时的提示（`UX_SPEC` §6.3）；不能确定时追加“（语义索引构建中）”。
    pub fn empty_hint(&self) -> Message {
        let base = match self.kind {
            PeekKind::Implementation => "references.empty.implementations",
            _ => "references.empty",
        };
        let key = if self.precision == Precision::Exact {
            base.to_string()
        } else {
            format!("{base}.approximate")
        };
        Message {
            key: key.into(),
            args: Arc::from([("symbol".into(), self.symbol.clone())]),
        }
    }
}

/// 视图的全部状态。
#[derive(Clone, Debug, Default)]
pub struct ReferencesState {
    /// 最近的在前；[`ReferencesState::current`] 指向正在显示的那一条。
    pub history: Vec<Entry>,
    pub current: Option<usize>,
    /// 当前结果里选中的条目下标。
    pub selected: Option<usize>,
    pub mode: Mode,
    /// 历史列表里聚焦的那一条（`pickFromHistory` 模式）。
    pub history_focus: usize,
}

impl ReferencesState {
    /// 新的一次查找：放到历史最前面并显示它（同一个查询点的旧条目被替换）。
    pub fn show(&mut self, entry: Entry) {
        let key = point_key(&entry.point);
        self.history
            .retain(|e| e.kind != entry.kind || point_key(&e.point) != key);
        self.history.insert(0, entry);
        self.history.truncate(HISTORY_LIMIT);
        self.current = Some(0);
        self.selected = None;
        self.mode = Mode::Results;
        self.history_focus = 0;
    }

    pub fn entry(&self) -> Option<&Entry> {
        self.history.get(self.current?)
    }

    pub fn results(&self) -> Option<&Arc<PeekResults>> {
        Some(&self.entry()?.results)
    }

    /// `reference-list.hasResult`：视图里有结果。
    pub fn has_result(&self) -> bool {
        self.results().is_some_and(|r| !r.is_empty())
    }

    /// `references-view.canNavigate`：当前结果可以用 `f4` / `shift+f4` 逐条导航。
    /// 历史列表模式下为假（那时 `f4` 不该跳到引用上）。
    pub fn can_navigate(&self) -> bool {
        self.mode == Mode::Results && self.has_result()
    }

    /// `references-view.next` / `prev`：下一条 / 上一条（跨文件、首尾循环），返回新的条目下标。
    pub fn step(&mut self, forward: bool) -> Option<usize> {
        let next = self.results()?.step(self.selected, forward)?;
        self.selected = Some(next);
        Some(next)
    }

    /// `references-view.clear`：清除当前结果（历史保留）。
    pub fn clear(&mut self) {
        self.current = None;
        self.selected = None;
        self.mode = Mode::Results;
    }

    /// `references-view.clearHistory`：清除历史（当前结果一并清除，与 VS Code 一致）。
    pub fn clear_history(&mut self) {
        self.history.clear();
        self.clear();
        self.history_focus = 0;
    }

    /// `references-view.pickFromHistory`：切到历史列表；历史是空的时候什么都不做。
    pub fn pick_from_history(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        self.mode = Mode::History;
        self.history_focus = self.current.unwrap_or(0).min(self.history.len() - 1);
        true
    }

    /// 从历史列表里选中第 `index` 条，回到结果树。
    pub fn restore(&mut self, index: usize) -> Option<&Entry> {
        if index >= self.history.len() {
            return None;
        }
        self.current = Some(index);
        self.selected = None;
        self.mode = Mode::Results;
        self.history.get(index)
    }

    /// `references-view.removeReferenceItem`：消除当前结果里的第 `item` 条。
    /// 分组索引按条目重建（与 [`PeekResults::group`] 同一口径），选中顺移到下一条。
    pub fn remove_item(&mut self, item: usize) -> bool {
        let Some(index) = self.current else {
            return false;
        };
        let Some(entry) = self.history.get_mut(index) else {
            return false;
        };
        let Some(results) = remove(&entry.results, item) else {
            return false;
        };
        let total = results.total();
        entry.results = Arc::new(results);
        self.selected = match total {
            0 => None,
            n => Some(item.min(n - 1)),
        };
        true
    }

    /// 选中条目所在的资源与位置（打开文档服务用）。
    pub fn location(&self, item: usize) -> Option<(Resource, rove_base::TextRange)> {
        let results = self.results()?;
        let it = results.items.get(item)?;
        let file = results.files.get(it.file as usize)?;
        Some((file.resource.clone(), it.range.clone()))
    }
}

/// 同一个查询点的键（`QueryPoint` 没有实现 `PartialEq`）。
fn point_key(point: &QueryPoint) -> (String, u64, u64) {
    (
        crate::nav::model::resource_key(&point.resource),
        point.position.line.0,
        point.position.column.0,
    )
}

/// 去掉第 `item` 条后的分组索引；下标越界时返回 `None`。空掉的文件分组一并去掉。
pub fn remove(results: &PeekResults, item: usize) -> Option<PeekResults> {
    if item >= results.items.len() {
        return None;
    }
    let mut files: Vec<FileGroup> = Vec::with_capacity(results.files.len());
    let mut items: Vec<PeekItem> = Vec::with_capacity(results.items.len() - 1);
    for (k, it) in results.items.iter().enumerate() {
        if k == item {
            continue;
        }
        let group = &results.files[it.file as usize];
        if files.last().is_none_or(|f| f.resource != group.resource) {
            files.push(FileGroup {
                resource: group.resource.clone(),
                label: group.label.clone(),
                description: group.description.clone(),
                start: items.len(),
                count: 0,
            });
        }
        let file = files.len() - 1;
        files[file].count += 1;
        items.push(PeekItem {
            file: file as u32,
            range: it.range.clone(),
            precision: it.precision,
            role: it.role,
        });
    }
    Some(PeekResults { files, items })
}

/// 复制一条结果的文案参数（`references.copy.item` 模板：路径、行、列，行列从 1 起算）。
pub fn copy_args(results: &PeekResults, item: usize) -> Option<Vec<(&'static str, String)>> {
    let it = results.items.get(item)?;
    let file = results.files.get(it.file as usize)?;
    let start = it.range.start();
    Some(vec![
        ("path", crate::workspace_symbols::path_of(&file.resource)),
        ("line", (start.line.0 + 1).to_string()),
        ("column", (start.column.0 + 1).to_string()),
    ])
}

/// `references-view.copyPath`：一条结果所在文件的路径。
pub fn copy_path(results: &PeekResults, item: usize) -> Option<String> {
    let it = results.items.get(item)?;
    let file = results.files.get(it.file as usize)?;
    Some(crate::workspace_symbols::path_of(&file.resource))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::{Line, Position, Resource, Span, Utf8Column, Version, WorkspacePath};
    use rove_lang_cs::query::Target;
    use rove_lang_cs::router::{RoutedTarget, TargetRole};

    fn pos(line: u64, col: u64) -> Position {
        Position {
            line: Line(line),
            column: Utf8Column(col),
        }
    }

    fn target(path: &str, line: u64) -> RoutedTarget {
        let range = Span::new(pos(line, 4), pos(line, 8)).unwrap();
        RoutedTarget {
            target: Target {
                resource: Resource::Workspace(WorkspacePath::new(path).unwrap()),
                range: range.clone(),
                selection: range,
            },
            precision: Precision::Exact,
            reason: None,
            role: TargetRole::Reference,
        }
    }

    fn point(path: &str, line: u64) -> QueryPoint {
        QueryPoint {
            resource: Resource::Workspace(WorkspacePath::new(path).unwrap()),
            position: pos(line, 0),
            version: Version(1),
        }
    }

    fn entry(symbol: &str, path: &str, line: u64, targets: Vec<RoutedTarget>) -> Entry {
        Entry {
            kind: PeekKind::References,
            symbol: symbol.into(),
            point: point(path, line),
            results: Arc::new(PeekResults::group(&targets)),
            precision: Precision::Exact,
        }
    }

    fn sample() -> ReferencesState {
        let mut state = ReferencesState::default();
        state.show(entry(
            "Player",
            "A.cs",
            0,
            vec![
                target("a/One.cs", 3),
                target("b/Two.cs", 1),
                target("b/Two.cs", 9),
            ],
        ));
        state
    }

    #[test]
    fn context_keys_follow_the_result_and_the_mode() {
        let mut state = ReferencesState::default();
        assert!(!state.has_result());
        assert!(!state.can_navigate());
        state = sample();
        assert!(state.has_result());
        assert!(state.can_navigate());
        // 历史列表模式：`f4` 不该跳引用。
        assert!(state.pick_from_history());
        assert!(state.has_result());
        assert!(!state.can_navigate());
        state.restore(0).unwrap();
        assert!(state.can_navigate());
        // 空结果不是“有结果”。
        state.show(entry("None", "A.cs", 7, vec![]));
        assert!(!state.has_result());
        assert!(!state.can_navigate());
    }

    #[test]
    fn next_and_prev_wrap_across_files() {
        let mut state = sample();
        assert_eq!(state.step(true), Some(0));
        assert_eq!(state.step(true), Some(1));
        assert_eq!(state.step(true), Some(2));
        assert_eq!(state.step(true), Some(0), "首尾循环");
        assert_eq!(state.step(false), Some(2));
        let (resource, range) = state.location(2).unwrap();
        assert_eq!(crate::nav::model::resource_key(&resource), "w:b/Two.cs");
        assert_eq!(range.start().line, Line(9));
    }

    #[test]
    fn clear_keeps_history_and_clear_history_empties_everything() {
        let mut state = sample();
        state.clear();
        assert!(!state.has_result());
        assert_eq!(state.history.len(), 1);
        assert!(state.pick_from_history());
        state.restore(0).unwrap();
        assert!(state.has_result());
        state.clear_history();
        assert!(state.history.is_empty());
        assert!(!state.has_result());
        assert!(!state.pick_from_history(), "历史是空的时候什么都不做");
    }

    #[test]
    fn a_new_search_at_the_same_point_replaces_the_old_entry() {
        let mut state = sample();
        state.show(entry("Other", "B.cs", 2, vec![target("a/One.cs", 1)]));
        assert_eq!(state.history.len(), 2);
        assert_eq!(&*state.entry().unwrap().symbol, "Other");
        state.show(entry("Other", "B.cs", 2, vec![target("a/One.cs", 1)]));
        assert_eq!(state.history.len(), 2, "同一个查询点只留一条");
    }

    #[test]
    fn removing_an_item_rebuilds_the_grouping_index() {
        let mut state = sample();
        assert!(state.remove_item(1));
        let r = state.results().unwrap();
        assert_eq!(r.total(), 2);
        assert_eq!(r.files.len(), 2);
        assert_eq!(r.files[1].count, 1);
        assert_eq!(r.files[1].start, 1);
        assert_eq!(state.selected, Some(1));
        // 最后一条被消除：文件分组一起消失。
        assert!(state.remove_item(1));
        let r = state.results().unwrap();
        assert_eq!(r.files.len(), 1);
        assert!(state.remove_item(0));
        assert!(!state.has_result());
        assert_eq!(state.selected, None);
        assert!(!state.remove_item(0), "越界");
    }

    #[test]
    fn copy_templates_use_one_based_line_and_column() {
        let state = sample();
        let r = state.results().unwrap();
        let args = copy_args(r, 0).unwrap();
        assert_eq!(args[0], ("path", "a/One.cs".to_string()));
        assert_eq!(args[1], ("line", "4".to_string()));
        assert_eq!(args[2], ("column", "5".to_string()));
        assert_eq!(copy_path(r, 2).unwrap(), "b/Two.cs");
        assert!(copy_args(r, 9).is_none());
    }

    #[test]
    fn titles_and_empty_hints_follow_the_kind_and_the_precision() {
        let state = sample();
        let e = state.entry().unwrap();
        assert_eq!(&*e.title().key, "references.title.references");
        assert_eq!(&*e.title().args[1].1, "3");
        assert!(e.marker().is_none(), "精确结果不加任何标记");
        let mut approximate = e.clone();
        approximate.precision = Precision::Approximate;
        approximate.results = Arc::new(PeekResults::group(&[]));
        assert_eq!(
            &*approximate.empty_hint().key,
            "references.empty.approximate"
        );
        let mut implementations = e.clone();
        implementations.kind = PeekKind::Implementation;
        implementations.results = Arc::new(PeekResults::group(&[]));
        assert_eq!(
            &*implementations.title().key,
            "references.title.implementations"
        );
        assert_eq!(
            &*implementations.empty_hint().key,
            "references.empty.implementations"
        );
    }

    /// 4.7 万条结果：视图拿到的是分组索引本身，没有任何按条目的展平。
    #[test]
    fn forty_seven_thousand_results_stay_a_grouping_index() {
        let targets: Vec<_> = (0..47_000)
            .map(|i| target(&format!("d/f{:04}.cs", i % 3000), i as u64))
            .collect();
        let mut state = ReferencesState::default();
        state.show(entry("Hot", "A.cs", 0, targets));
        let r = state.results().unwrap();
        assert_eq!(r.total(), 47_000);
        assert_eq!(r.files.len(), 3000);
        // 初始只展开第一个文件（> 100 条）。
        assert_eq!(r.initially_expanded(), 0..1);
        assert!(state.can_navigate());
    }
}
