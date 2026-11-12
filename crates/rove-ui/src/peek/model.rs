//! 速览结果的数据层（不依赖 GPUI）：按文件分组的索引、展开规则、结果间导航与标题。
//!
//! 4.7 万条结果也只在这里一次排序、一次分组（O(n log n)）；界面只按需取“某个文件的某一段”，
//! 不遍历界面模型，也不为每条结果建立界面节点（UX_SPEC §2.3、issue #41 “风险与提示”）。

use rove_base::{Position, Resource, TextRange};
use rove_i18n::Message;
use rove_lang_cs::query::{NavigationKind, Precision};
use rove_lang_cs::router::{metadata_symbol_key, RoutedTarget, TargetRole};
use std::ops::Range;
use std::sync::Arc;

/// 结果总数不超过它时展开全部文件；超过时只展开第一个文件（UX_SPEC §2.3，与 VS Code 不同）。
pub const EXPAND_ALL_LIMIT: usize = 100;

/// 三个入口对应的查询。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeekKind {
    References,
    Definition,
    Implementation,
}

impl PeekKind {
    pub fn navigation(self) -> NavigationKind {
        match self {
            PeekKind::References => NavigationKind::References,
            PeekKind::Definition => NavigationKind::Definition,
            PeekKind::Implementation => NavigationKind::Implementations,
        }
    }

    /// 标题模板（`{symbol}`、`{count}`）。
    pub fn title_key(self) -> &'static str {
        match self {
            PeekKind::References => "peek.title.references",
            PeekKind::Definition => "peek.title.definitions",
            PeekKind::Implementation => "peek.title.implementations",
        }
    }

    /// 结果为空时光标处提示的模板（UX_SPEC §6.3）。
    pub fn empty_key(self) -> &'static str {
        match self {
            PeekKind::References => "peek.empty.references",
            PeekKind::Definition => "peek.empty.definitions",
            PeekKind::Implementation => "peek.empty.implementations",
        }
    }
}

/// 一条结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeekItem {
    /// 所在文件（[`PeekResults::files`] 的下标）。
    pub file: u32,
    pub range: TextRange,
    pub precision: Precision,
    pub role: TargetRole,
}

/// 一个文件分组：文件内的结果是 `items[start..start + count]`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileGroup {
    pub resource: Resource,
    /// 文件名（外部符号为符号键里最后一段）。
    pub label: Arc<str>,
    /// 所在目录（工作区相对路径；外部符号为程序集）。
    pub description: Arc<str>,
    pub start: usize,
    pub count: usize,
}

impl FileGroup {
    pub fn items(&self) -> Range<usize> {
        self.start..self.start + self.count
    }
    pub fn is_external(&self) -> bool {
        matches!(self.resource, Resource::Virtual(_))
    }
}

/// 一次查询的全部结果，按文件分组、文件内按位置排序。不可变，界面与后台共享 `Arc`。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PeekResults {
    pub files: Vec<FileGroup>,
    pub items: Vec<PeekItem>,
}

impl PeekResults {
    /// 分组：按资源、再按位置排序，去掉完全相同的位置。O(n log n)，在后台线程调用。
    pub fn group(targets: &[RoutedTarget]) -> Self {
        let mut order: Vec<u32> = (0..targets.len() as u32).collect();
        let key = |i: &u32| {
            let t = &targets[*i as usize].target;
            (&t.resource, t.selection.start(), t.selection.end())
        };
        order.sort_unstable_by(|a, b| key(a).cmp(&key(b)));
        order.dedup_by(|a, b| key(a) == key(b));
        let mut files: Vec<FileGroup> = Vec::new();
        let mut items = Vec::with_capacity(order.len());
        for i in order {
            let routed = &targets[i as usize];
            let resource = &routed.target.resource;
            if files.last().is_none_or(|f| &f.resource != resource) {
                let (label, description) = labels(resource);
                files.push(FileGroup {
                    resource: resource.clone(),
                    label,
                    description,
                    start: items.len(),
                    count: 0,
                });
            }
            let file = files.len() - 1;
            files[file].count += 1;
            items.push(PeekItem {
                file: file as u32,
                range: routed.target.selection.clone(),
                precision: routed.precision,
                role: routed.role,
            });
        }
        Self { files, items }
    }

    pub fn total(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 最弱的精确度（精确 > 近似 > 语法）；空结果为精确。
    pub fn precision(&self) -> Precision {
        let rank = |p: Precision| match p {
            Precision::Exact => 0,
            Precision::Approximate => 1,
            Precision::Syntax => 2,
        };
        self.items
            .iter()
            .map(|i| i.precision)
            .max_by_key(|p| rank(*p))
            .unwrap_or(Precision::Exact)
    }

    /// 第 `item` 条结果所在的文件。
    pub fn file_of(&self, item: usize) -> usize {
        self.items.get(item).map_or(0, |i| i.file as usize)
    }

    /// 初始展开的文件（UX_SPEC §2.3）：总数 ≤ 100 时全部，否则只有第一个。
    pub fn initially_expanded(&self) -> Range<usize> {
        if self.total() <= EXPAND_ALL_LIMIT {
            0..self.files.len()
        } else {
            0..self.files.len().min(1)
        }
    }

    /// 下一条 / 上一条结果（跨文件、首尾循环，与 VS Code `nextOrPreviousReference` 一致）。
    pub fn step(&self, current: Option<usize>, forward: bool) -> Option<usize> {
        let n = self.total();
        if n == 0 {
            return None;
        }
        Some(match (current, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        })
    }

    /// 离 `position` 最近的结果（`resource` 内第一条不早于它的；没有则该文件最后一条），
    /// 打开速览时据此选中“当前位置”的那一条（VS Code `nearestReference`）。
    pub fn nearest(&self, resource: &Resource, position: Position) -> Option<usize> {
        let file = self.files.iter().find(|f| &f.resource == resource)?;
        let items = &self.items[file.items()];
        let k = items.partition_point(|i| i.range.end() < position);
        Some(file.start + k.min(items.len() - 1))
    }
}

/// 文件名与目录。
fn labels(resource: &Resource) -> (Arc<str>, Arc<str>) {
    let split = |path: &str| match path.rsplit_once(['/', '\\']) {
        Some((dir, name)) => (Arc::from(name), Arc::from(dir)),
        None => (Arc::from(path), Arc::from("")),
    };
    match resource {
        Resource::Workspace(p) => split(p.as_str()),
        Resource::File(p) => split(&p.to_string_lossy()),
        Resource::Virtual(v) => {
            // `rove-cs-metadata:<程序集>|<文档注释 ID>`：程序集作目录，ID 最后一段作文件名。
            let key = metadata_symbol_key(resource).unwrap_or(v);
            match key.split_once('|') {
                Some((assembly, id)) => {
                    let name = id.rsplit(['.', ':']).next().unwrap_or(id);
                    (Arc::from(name), Arc::from(assembly))
                }
                None => (Arc::from(key), Arc::from("")),
            }
        }
    }
}

/// 标题上的近似标记（UX_SPEC §2.2、§2.3）。精确结果不加任何标记。
#[derive(Clone, Debug)]
pub enum Marker {
    /// 按结果的精确度决定：精确无标记；定义有多个候选时“近似 · N 个候选”；其他“近似”。
    Auto,
    /// 调用方（#40）给出的文案；`None` 表示不加标记。
    Custom(Option<Message>),
}

/// 实际显示的标记。
pub fn marker_text(marker: &Marker, kind: PeekKind, results: &PeekResults) -> Option<Message> {
    match marker {
        Marker::Custom(m) => m.clone(),
        Marker::Auto => {
            if results.precision() == Precision::Exact {
                None
            } else if kind == PeekKind::Definition && results.total() > 1 {
                Some(Message {
                    key: "peek.marker.candidates".into(),
                    args: Arc::from([("count".into(), results.total().to_string().into())]),
                })
            } else {
                Some(Message::key("peek.marker.approximate"))
            }
        }
    }
}

/// 标题：符号名 + 结果数。
pub fn title(kind: PeekKind, symbol: &str, total: usize) -> Message {
    Message {
        key: kind.title_key().into(),
        args: Arc::from([
            ("symbol".into(), symbol.into()),
            ("count".into(), total.to_string().into()),
        ]),
    }
}

/// 结果为空时光标处的提示；不精确（“不能确定”）时追加“（语义索引构建中）”（UX_SPEC §6.3）。
pub fn empty_hint(kind: PeekKind, symbol: &str, precision: Precision) -> Message {
    let key = if precision == Precision::Exact {
        kind.empty_key().to_string()
    } else {
        format!("{}.approximate", kind.empty_key())
    };
    Message {
        key: key.into(),
        args: Arc::from([("symbol".into(), symbol.into())]),
    }
}

/// 树节点的身份：文件节点与结果节点（`ItemId` 的文本）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    File(usize),
    Item(usize),
}

impl Node {
    pub fn id(self) -> Arc<str> {
        match self {
            Node::File(i) => format!("f{i}").into(),
            Node::Item(i) => format!("r{i}").into(),
        }
    }
    pub fn parse(id: &str) -> Option<Self> {
        let (tag, n) = id.split_at_checked(1)?;
        let n = n.parse().ok()?;
        match tag {
            "f" => Some(Node::File(n)),
            "r" => Some(Node::Item(n)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::{Line, Span, Utf8Column, WorkspacePath};
    use rove_lang_cs::query::Target;

    fn pos(line: u64, col: u64) -> Position {
        Position {
            line: Line(line),
            column: Utf8Column(col),
        }
    }

    pub(crate) fn target(path: &str, line: u64, precision: Precision) -> RoutedTarget {
        let range = Span::new(pos(line, 4), pos(line, 8)).unwrap();
        RoutedTarget {
            target: Target {
                resource: Resource::Workspace(WorkspacePath::new(path).unwrap()),
                range: range.clone(),
                selection: range,
            },
            precision,
            reason: None,
            role: TargetRole::Reference,
        }
    }

    /// `files` 个文件，共 `total` 条结果（轮流分配）。
    fn spread(files: usize, total: usize) -> PeekResults {
        let targets: Vec<_> = (0..total)
            .map(|i| target(&format!("f{:03}.cs", i % files), i as u64, Precision::Exact))
            .collect();
        PeekResults::group(&targets)
    }

    #[test]
    fn groups_by_file_with_counts_and_offsets() {
        let r = PeekResults::group(&[
            target("b/Two.cs", 9, Precision::Exact),
            target("a/One.cs", 5, Precision::Exact),
            target("b/Two.cs", 1, Precision::Exact),
            target("a/One.cs", 5, Precision::Exact),
        ]);
        assert_eq!(r.total(), 3, "完全相同的位置去重");
        assert_eq!(r.files.len(), 2);
        assert_eq!(
            (
                &*r.files[0].label,
                &*r.files[0].description,
                r.files[0].count
            ),
            ("One.cs", "a", 1)
        );
        assert_eq!((r.files[1].start, r.files[1].count), (1, 2));
        assert_eq!(r.items[1].range.start().line, Line(1), "文件内按位置排序");
        assert_eq!(r.file_of(2), 1);
    }

    #[test]
    fn expansion_rule_100_expands_all_101_only_first() {
        let at_limit = spread(7, 100);
        assert_eq!(at_limit.initially_expanded(), 0..7);
        let over = spread(7, 101);
        assert_eq!(over.initially_expanded(), 0..1);
        assert_eq!(over.files.iter().map(|f| f.count).sum::<usize>(), 101);
    }

    #[test]
    fn step_wraps_across_files_and_nearest_picks_the_query_point() {
        let r = spread(3, 6);
        assert_eq!(r.step(None, true), Some(0));
        assert_eq!(r.step(Some(5), true), Some(0));
        assert_eq!(r.step(Some(0), false), Some(5));
        let res = r.files[1].resource.clone();
        // f001.cs 里的结果在第 1 行与第 4 行。
        assert_eq!(r.nearest(&res, pos(2, 0)), Some(r.files[1].start + 1));
        assert_eq!(r.nearest(&res, pos(9, 0)), Some(r.files[1].start + 1));
        assert_eq!(r.nearest(&res, pos(0, 0)), Some(r.files[1].start));
    }

    #[test]
    fn markers_follow_precision_and_caller_override() {
        let exact = spread(2, 3);
        assert!(
            marker_text(&Marker::Auto, PeekKind::References, &exact).is_none(),
            "精确结果不加任何标记"
        );
        let approx = PeekResults::group(&[
            target("a.cs", 1, Precision::Exact),
            target("b.cs", 1, Precision::Approximate),
        ]);
        assert_eq!(approx.precision(), Precision::Approximate);
        let m = marker_text(&Marker::Auto, PeekKind::References, &approx).unwrap();
        assert_eq!(&*m.key, "peek.marker.approximate");
        let m = marker_text(&Marker::Auto, PeekKind::Definition, &approx).unwrap();
        assert_eq!(&*m.key, "peek.marker.candidates");
        assert_eq!(&*m.args[0].1, "2");
        let custom = Marker::Custom(Some(Message::key("x")));
        assert_eq!(
            marker_text(&custom, PeekKind::Definition, &exact).map(|m| m.key),
            Some("x".into())
        );
    }

    #[test]
    fn node_ids_round_trip() {
        for n in [Node::File(0), Node::File(12), Node::Item(47_000)] {
            assert_eq!(Node::parse(&n.id()), Some(n));
        }
        assert_eq!(Node::parse("x1"), None);
        assert_eq!(Node::parse(""), None);
    }

    #[test]
    fn grouping_47k_results_is_one_pass() {
        let r = spread(3000, 47_000);
        assert_eq!(r.files.len(), 3000);
        assert_eq!(r.files.iter().map(|f| f.count).sum::<usize>(), 47_000);
        assert!(r
            .files
            .windows(2)
            .all(|w| w[0].start + w[0].count == w[1].start));
    }
}
