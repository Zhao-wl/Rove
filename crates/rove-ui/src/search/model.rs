//! 搜索结果模型（无 GPUI）：按文件分组、排序（`search.sortOrder`）、折叠（`search.collapseResults`）、
//! 删除（`search.action.remove`），以及给 #18 列表用的展平快照 [`ResultRows`]。
//!
//! 流式到达的文件逐个插入到排序位置；每次刷新只重建 O(文件数) 的行前缀和，取可见行是
//! O(log 文件数 + 可见行)，所以几万条结果也只为可见行生成 `Row`（验收：1 万条以上滚动流畅）。

use crate::lists::{ItemId, ListModel, Row};
use rove_base::{Error, Result, Version, WorkspacePath};
use rove_search::settings::{Collapse, SortOrder};
use rove_search::{FileMatches, LineMatch};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

/// `search.collapseResults = auto` 时，超过这么多处匹配的文件默认折叠（VS Code 的阈值）。
const AUTO_COLLAPSE_MATCHES: usize = 10;
/// 行 ID 里的分隔符（路径里不可能出现）。
const SEP: char = '\u{1}';

/// 一个文件的结果。`starts[i]` 是第 i 行之前的匹配处数（行内的第 k 处是第 `starts[i] + k` 个子行）。
#[derive(Clone, Debug)]
pub struct FileNode {
    pub path: WorkspacePath,
    pub lines: Vec<LineMatch>,
    starts: Vec<usize>,
    count: usize,
}

impl FileNode {
    pub fn new(path: WorkspacePath, lines: Vec<LineMatch>) -> Self {
        let mut node = Self {
            path,
            lines,
            starts: vec![],
            count: 0,
        };
        node.reindex();
        node
    }

    fn reindex(&mut self) {
        self.starts.clear();
        let mut total = 0;
        for line in &self.lines {
            self.starts.push(total);
            total += line.ranges.len();
        }
        self.count = total;
    }

    /// 匹配处数（徽章上的数字）。
    pub fn count(&self) -> usize {
        self.count
    }

    /// 第 `k` 处匹配：(行下标, 行内区间下标)。
    pub fn nth(&self, k: usize) -> Option<(usize, usize)> {
        if k >= self.count {
            return None;
        }
        let line = self.starts.partition_point(|&s| s <= k) - 1;
        Some((line, k - self.starts[line]))
    }

    /// 文件名（最后一段）。
    pub fn name(&self) -> &str {
        let p = self.path.as_str();
        p.rsplit('/').next().unwrap_or(p)
    }

    /// 所在目录（相对路径，根目录下为空）。
    pub fn dir(&self) -> &str {
        let p = self.path.as_str();
        p.rfind('/').map_or("", |i| &p[..i])
    }
}

/// 列表里的一行指向的东西。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    File(WorkspacePath),
    /// 文件、行下标、行内区间下标。
    Match(WorkspacePath, usize, usize),
}

pub fn file_id(path: &WorkspacePath) -> ItemId {
    ItemId(Arc::from(format!("f{SEP}{}", path.as_str())))
}

pub fn match_id(path: &WorkspacePath, line: usize, range: usize) -> ItemId {
    ItemId(Arc::from(format!(
        "m{SEP}{}{SEP}{line}{SEP}{range}",
        path.as_str()
    )))
}

/// 行 ID → 目标；不是本模型产生的 ID 返回 `None`。
pub fn parse_id(id: &ItemId) -> Option<Target> {
    let mut parts = id.0.split(SEP);
    match parts.next()? {
        "f" => Some(Target::File(WorkspacePath::new(parts.next()?).ok()?)),
        "m" => {
            let path = WorkspacePath::new(parts.next()?).ok()?;
            let line = parts.next()?.parse().ok()?;
            let range = parts.next()?.parse().ok()?;
            Some(Target::Match(path, line, range))
        }
        _ => None,
    }
}

/// 可变的结果集（UI 线程持有）。
pub struct Results {
    files: Vec<Arc<FileNode>>,
    sort: SortOrder,
    collapse: Collapse,
    /// 用户手动展开 / 折叠过的文件（覆盖默认）。
    toggled: HashMap<Arc<str>, bool>,
    matches: usize,
    revision: u64,
}

impl Results {
    pub fn new(sort: SortOrder, collapse: Collapse) -> Self {
        Self {
            files: vec![],
            sort,
            collapse,
            toggled: HashMap::new(),
            matches: 0,
            revision: 0,
        }
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.toggled.clear();
        self.matches = 0;
        self.revision += 1;
    }

    pub fn set_view_options(&mut self, sort: SortOrder, collapse: Collapse) {
        if self.sort != sort {
            self.sort = sort;
            let mut files = std::mem::take(&mut self.files);
            files.sort_by(|a, b| compare(sort, a, b));
            self.files = files;
        }
        self.collapse = collapse;
        self.revision += 1;
    }

    pub fn files(&self) -> &[Arc<FileNode>] {
        &self.files
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn match_count(&self) -> usize {
        self.matches
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// 流式到达的一个文件：插入到排序位置。同一文件不会在一次搜索里出现两次。
    pub fn add(&mut self, file: FileMatches) {
        let node = Arc::new(FileNode::new(file.path, file.lines));
        self.matches += node.count();
        let sort = self.sort;
        let at = self
            .files
            .partition_point(|f| compare(sort, f, &node) != Ordering::Greater);
        self.files.insert(at, node);
        self.revision += 1;
    }

    fn position(&self, path: &WorkspacePath) -> Option<usize> {
        self.files.iter().position(|f| &f.path == path)
    }

    pub fn file(&self, path: &WorkspacePath) -> Option<&Arc<FileNode>> {
        self.position(path).map(|i| &self.files[i])
    }

    /// 文件默认是否展开（`search.collapseResults`），用户切换过的按切换后的状态。
    pub fn is_expanded(&self, file: &FileNode) -> bool {
        if let Some(&on) = self.toggled.get(file.path.as_str()) {
            return on;
        }
        match self.collapse {
            Collapse::AlwaysExpand => true,
            Collapse::AlwaysCollapse => false,
            Collapse::Auto => file.count() <= AUTO_COLLAPSE_MATCHES,
        }
    }

    pub fn set_expanded(&mut self, path: &WorkspacePath, expanded: bool) -> bool {
        let Some(file) = self.file(path) else {
            return false;
        };
        if self.is_expanded(file) == expanded {
            return false;
        }
        self.toggled.insert(Arc::from(path.as_str()), expanded);
        self.revision += 1;
        true
    }

    pub fn set_all_expanded(&mut self, expanded: bool) {
        for f in &self.files {
            self.toggled.insert(Arc::from(f.path.as_str()), expanded);
        }
        self.revision += 1;
    }

    /// `search.action.remove`：删掉一个文件或一处匹配（行空了删行，文件空了删文件）。
    pub fn remove(&mut self, target: &Target) -> bool {
        match target {
            Target::File(path) => {
                let Some(i) = self.position(path) else {
                    return false;
                };
                let node = self.files.remove(i);
                self.matches -= node.count();
            }
            Target::Match(path, line, range) => {
                let Some(i) = self.position(path) else {
                    return false;
                };
                let node = Arc::make_mut(&mut self.files[i]);
                let Some(l) = node.lines.get_mut(*line) else {
                    return false;
                };
                if *range >= l.ranges.len() {
                    return false;
                }
                l.ranges.remove(*range);
                if l.ranges.is_empty() {
                    node.lines.remove(*line);
                }
                node.reindex();
                self.matches -= 1;
                if node.count() == 0 {
                    self.files.remove(i);
                }
            }
        }
        self.revision += 1;
        true
    }

    /// 给列表的不可变快照（O(文件数)）。
    pub fn rows(&self) -> Arc<ResultRows> {
        let mut starts = Vec::with_capacity(self.files.len());
        let mut expanded = Vec::with_capacity(self.files.len());
        let mut total = 0;
        for f in &self.files {
            starts.push(total);
            let open = self.is_expanded(f);
            expanded.push(open);
            total += 1 + if open { f.count() } else { 0 };
        }
        Arc::new(ResultRows {
            files: self.files.clone(),
            expanded,
            starts,
            total,
            revision: Version(self.revision),
        })
    }
}

/// `search.sortOrder` 的比较。`modified` 没有修改时间可比，按 `default` 排。
fn compare(sort: SortOrder, a: &FileNode, b: &FileNode) -> Ordering {
    let by_path = || compare_paths(a.path.as_str(), b.path.as_str());
    match sort {
        SortOrder::Default | SortOrder::Modified => by_path(),
        SortOrder::FileNames => compare_text(a.name(), b.name()).then_with(by_path),
        SortOrder::Type => compare_text(extension(a.name()), extension(b.name()))
            .then_with(|| compare_text(a.name(), b.name()))
            .then_with(by_path),
        SortOrder::CountDescending => b.count().cmp(&a.count()).then_with(by_path),
        SortOrder::CountAscending => a.count().cmp(&b.count()).then_with(by_path),
    }
}

fn extension(name: &str) -> &str {
    name.rfind('.').map_or("", |i| &name[i + 1..])
}

/// 按段比较路径（VS Code 的 `comparePaths`：同一目录下的内容排在一起），段内忽略大小写。
fn compare_paths(a: &str, b: &str) -> Ordering {
    let mut x = a.split('/');
    let mut y = b.split('/');
    loop {
        match (x.next(), y.next()) {
            (Some(p), Some(q)) => match compare_text(p, q) {
                Ordering::Equal => continue,
                other => return other,
            },
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

fn compare_text(a: &str, b: &str) -> Ordering {
    let fold = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<String>();
    fold(a).cmp(&fold(b)).then_with(|| a.cmp(b))
}

/// 展平后的结果行：文件行（深度 0，可展开）+ 展开文件的每一处匹配（深度 1）。
pub struct ResultRows {
    files: Vec<Arc<FileNode>>,
    expanded: Vec<bool>,
    /// 每个文件行的行号。
    starts: Vec<usize>,
    total: usize,
    revision: Version,
}

impl ResultRows {
    pub fn empty() -> Arc<Self> {
        Results::new(SortOrder::Default, Collapse::AlwaysExpand).rows()
    }

    /// 第 `index` 行：(文件, 匹配序号；文件行为 `None`)。
    pub fn at(&self, index: usize) -> Option<(&Arc<FileNode>, Option<usize>)> {
        if index >= self.total {
            return None;
        }
        let f = self.starts.partition_point(|&s| s <= index) - 1;
        let offset = index - self.starts[f];
        Some((&self.files[f], offset.checked_sub(1)))
    }

    pub fn target(&self, index: usize) -> Option<Target> {
        let (file, k) = self.at(index)?;
        match k {
            None => Some(Target::File(file.path.clone())),
            Some(k) => {
                let (line, range) = file.nth(k)?;
                Some(Target::Match(file.path.clone(), line, range))
            }
        }
    }

    /// 文件行的下标。
    pub fn file_row(&self, path: &WorkspacePath) -> Option<usize> {
        self.files
            .iter()
            .position(|f| &f.path == path)
            .map(|i| self.starts[i])
    }

    pub fn is_expanded_at(&self, file_index: usize) -> bool {
        self.expanded.get(file_index).copied().unwrap_or(false)
    }

    fn row(&self, index: usize) -> Option<Row> {
        let f = self.starts.partition_point(|&s| s <= index) - 1;
        let file = &self.files[f];
        let offset = index - self.starts[f];
        Some(match offset.checked_sub(1) {
            None => Row {
                id: file_id(&file.path),
                label: Arc::from(file.name()),
                detail: (!file.dir().is_empty()).then(|| Arc::from(file.dir())),
                depth: 0,
                expandable: true,
                expanded: self.expanded[f],
                disabled: false,
            },
            Some(k) => {
                let (line, range) = file.nth(k)?;
                Row {
                    id: match_id(&file.path, line, range),
                    label: file.lines[line].preview.clone(),
                    detail: None,
                    depth: 1,
                    expandable: false,
                    expanded: false,
                    disabled: false,
                }
            }
        })
    }

    /// 行内高亮：`preview` 中第 `range` 处匹配的字节区间。
    pub fn highlight(file: &FileNode, line: usize, range: usize) -> Option<Range<usize>> {
        file.lines.get(line)?.preview_ranges().nth(range)
    }
}

impl ListModel for ResultRows {
    fn revision(&self) -> Version {
        self.revision
    }

    fn count(&self) -> usize {
        self.total
    }

    fn rows(&self, visible: Range<usize>) -> Result<Arc<[Row]>> {
        if visible.start > visible.end {
            return Err(Error::InvalidInput);
        }
        if visible.start > self.total {
            return Err(Error::OutOfBounds);
        }
        let end = visible.end.min(self.total);
        Ok((visible.start..end).filter_map(|i| self.row(i)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, lines: &[(u64, &[Range<u32>])]) -> FileMatches {
        FileMatches {
            path: WorkspacePath::new(path).unwrap(),
            lines: lines
                .iter()
                .map(|(line, ranges)| LineMatch {
                    line: *line,
                    preview: Arc::from("abcdefghijklmnopqrstuvwxyz"),
                    preview_start: 0,
                    ranges: ranges.to_vec(),
                })
                .collect(),
        }
    }

    fn labels(rows: &ResultRows) -> Vec<String> {
        rows.rows(0..rows.count())
            .unwrap()
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), r.label))
            .collect()
    }

    #[test]
    fn streamed_files_are_inserted_in_path_order_and_flattened() {
        let mut r = Results::new(SortOrder::Default, Collapse::AlwaysExpand);
        r.add(file("b/Z.cs", &[(3, &[0..1])]));
        r.add(file("a/B.cs", &[(0, &[0..1, 2..3]), (5, &[4..5])]));
        r.add(file("a/a.cs", &[(1, &[1..2])]));
        let rows = r.rows();
        assert_eq!(rows.count(), 3 + 4);
        assert_eq!(
            rows.rows(0..1).unwrap()[0].id,
            file_id(&WorkspacePath::new("a/a.cs").unwrap())
        );
        assert_eq!(
            rows.target(3),
            Some(Target::Match(WorkspacePath::new("a/B.cs").unwrap(), 0, 1))
        );
        assert_eq!(
            rows.target(4),
            Some(Target::Match(WorkspacePath::new("a/B.cs").unwrap(), 1, 0))
        );
        assert_eq!(r.match_count(), 5);
        assert_eq!(r.file_count(), 3);
        for i in 0..rows.count() {
            let id = rows.rows(i..i + 1).unwrap()[0].id.clone();
            assert_eq!(parse_id(&id), rows.target(i));
        }
    }

    #[test]
    fn collapse_settings_and_toggles() {
        let many: Vec<(u64, &[Range<u32>])> = (0..11).map(|i| (i, &[0..1][..])).collect();
        let mut r = Results::new(SortOrder::Default, Collapse::Auto);
        r.add(file("big.cs", &many));
        r.add(file("small.cs", &[(0, &[0..1])]));
        assert_eq!(r.rows().count(), 1 + 1 + 1, "auto：超过 10 处的文件折叠");
        let big = WorkspacePath::new("big.cs").unwrap();
        assert!(r.set_expanded(&big, true));
        assert_eq!(r.rows().count(), 1 + 11 + 2);
        r.set_view_options(SortOrder::Default, Collapse::AlwaysCollapse);
        assert_eq!(r.rows().count(), 1 + 11 + 1, "用户切换过的保持");
        r.set_all_expanded(false);
        assert_eq!(r.rows().count(), 2);
    }

    #[test]
    fn remove_matches_lines_and_files() {
        let mut r = Results::new(SortOrder::Default, Collapse::AlwaysExpand);
        r.add(file("a.cs", &[(0, &[0..1, 2..3]), (4, &[1..2])]));
        r.add(file("b.cs", &[(0, &[0..1])]));
        let a = WorkspacePath::new("a.cs").unwrap();
        assert!(r.remove(&Target::Match(a.clone(), 0, 0)));
        assert_eq!(r.file(&a).unwrap().count(), 2);
        assert!(r.remove(&Target::Match(a.clone(), 0, 0)));
        assert_eq!(r.file(&a).unwrap().lines.len(), 1);
        assert!(r.remove(&Target::Match(a.clone(), 0, 0)));
        assert!(r.file(&a).is_none(), "最后一处删掉后文件消失");
        assert!(r.remove(&Target::File(WorkspacePath::new("b.cs").unwrap())));
        assert!(r.is_empty());
        assert_eq!(r.match_count(), 0);
    }

    #[test]
    fn sort_orders() {
        let mut r = Results::new(SortOrder::CountDescending, Collapse::AlwaysCollapse);
        r.add(file("x/one.txt", &[(0, &[0..1])]));
        r.add(file("y/three.cs", &[(0, &[0..1, 1..2, 2..3])]));
        r.add(file("a/two.cs", &[(0, &[0..1]), (1, &[0..1])]));
        assert_eq!(labels(&r.rows()), vec!["three.cs", "two.cs", "one.txt"]);
        r.set_view_options(SortOrder::Type, Collapse::AlwaysCollapse);
        assert_eq!(labels(&r.rows()), vec!["three.cs", "two.cs", "one.txt"]);
        r.set_view_options(SortOrder::FileNames, Collapse::AlwaysCollapse);
        assert_eq!(labels(&r.rows()), vec!["one.txt", "three.cs", "two.cs"]);
        r.set_view_options(SortOrder::Default, Collapse::AlwaysCollapse);
        assert_eq!(labels(&r.rows()), vec!["two.cs", "one.txt", "three.cs"]);
    }

    #[test]
    fn twenty_thousand_rows_only_materialize_the_viewport() {
        let mut r = Results::new(SortOrder::Default, Collapse::AlwaysExpand);
        for i in 0..2000 {
            let lines: Vec<(u64, &[Range<u32>])> = (0..10).map(|l| (l, &[0..1][..])).collect();
            r.add(file(&format!("d/f{i:04}.cs"), &lines));
        }
        let started = std::time::Instant::now();
        let rows = r.rows();
        let page = rows.rows(15_000..15_040).unwrap();
        assert_eq!(rows.count(), 22_000);
        assert_eq!(page.len(), 40);
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
    }
}
