//! 快速打开的文件候选：工作区文件列表的增量索引，按键间的前缀剪枝，历史文件加权。
//!
//! 排序完全由 `rove-fuzzy`（VS Code `fuzzyScorer`）决定：label = 文件名，description = 相对目录，
//! path = 相对路径（分隔符为 `\`，与 VS Code 在 Windows 上一致）。
//!
//! 性能（大型 Unity 项目约 2.1 万个 `.cs` 文件外加其他文件，每次按键 < 16 ms）：
//! - 每个候选的 UTF-16 与小写形式在建索引时算好（[`Target`]），按键时不再分配；
//! - 查询是上一次查询的延长时，只在上一次的匹配集合里重新打分（前缀剪枝，见 [`Search`]）；
//! - 只对排名前 [`MAX_RESULTS`] 的结果做完整排序（VS Code 同样最多显示 512 条文件结果）。
use super::Picks;
use crate::{list_widget::quick_input::RowDecor, lists::ItemId, lists::Row};
use rove_base::WorkspacePath;
use rove_fuzzy::{
    compare_scored, prepare_query, score_target, utf16_to_byte_ranges, ItemScore, PreparedQuery,
    Scored, Target, SEP,
};
use rove_workspace::{EntryKind, FileEvent, Workspace};
use std::{cmp::Ordering, collections::HashMap, ops::Range, sync::Arc};

/// 文件结果的最大条数（VS Code `AnythingQuickAccessProvider.MAX_RESULTS`）。
pub const MAX_RESULTS: usize = 512;

/// 一个文件候选。
#[derive(Debug)]
pub struct FileEntry {
    pub path: WorkspacePath,
    target: Target,
}

impl FileEntry {
    pub fn new(path: WorkspacePath) -> Self {
        let text = path.as_str();
        let (dir, name) = match text.rfind('/') {
            Some(i) => (&text[..i], &text[i + 1..]),
            None => ("", text),
        };
        let native = |s: &str| s.replace('/', &SEP.to_string());
        let description = native(dir);
        let relative = native(text);
        let target = Target::new(
            name,
            (!description.is_empty()).then_some(description.as_str()),
            Some(&relative),
        );
        Self { path, target }
    }

    pub fn label(&self) -> &str {
        self.target.label()
    }

    pub fn description(&self) -> Option<&str> {
        self.target.description()
    }
}

/// 工作区文件列表。查找按路径（区分大小写的显示路径）。
#[derive(Default)]
pub struct FileIndex {
    entries: Vec<FileEntry>,
    by_path: HashMap<Arc<str>, usize>,
    /// 每次内容变化加一：作废按键间的剪枝缓存。
    generation: u64,
    /// 对应的工作区快照版本：晚于它的文件事件才需要应用。
    pub revision: u64,
    /// 工作区初次扫描是否已完成。
    pub complete: bool,
}

impl FileIndex {
    /// 从工作区读取全部文件（分页）。后台线程调用。
    pub fn from_workspace(workspace: &dyn Workspace) -> Self {
        let revision = workspace.revision().0;
        let mut index = Self::default();
        let mut start = 0;
        const PAGE: usize = 8192;
        loop {
            let Ok(page) = workspace.files(start..start + PAGE) else {
                break;
            };
            let got = page.items.len();
            index.complete = page.complete;
            for entry in page.items.iter() {
                if matches!(entry.kind, EntryKind::File) {
                    index.insert(entry.path.clone());
                }
            }
            if got < PAGE {
                break;
            }
            start += PAGE;
        }
        index.revision = revision;
        index
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entry(&self, index: u32) -> Option<&FileEntry> {
        self.entries.get(index as usize)
    }

    pub fn find(&self, path: &str) -> Option<u32> {
        self.by_path.get(path).map(|i| *i as u32)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn insert(&mut self, path: WorkspacePath) {
        if self.by_path.contains_key(path.as_str()) {
            return;
        }
        let key: Arc<str> = path.as_str().into();
        self.by_path.insert(key, self.entries.len());
        self.entries.push(FileEntry::new(path));
        self.generation += 1;
    }

    fn remove(&mut self, path: &str) {
        let Some(at) = self.by_path.remove(path) else {
            return;
        };
        self.entries.swap_remove(at);
        if at < self.entries.len() {
            let moved: Arc<str> = self.entries[at].path.as_str().into();
            self.by_path.insert(moved, at);
        }
        self.generation += 1;
    }

    /// 应用一批文件事件。返回 `true` 表示需要重新读取整个列表（`RescanRequired`）。
    /// 只有目录事件（路径不是文件）或内容变化（`Changed`）不影响候选。
    pub fn apply(&mut self, events: &[FileEvent]) -> bool {
        let mut rescan = false;
        for event in events {
            match event {
                FileEvent::Created(path) => self.insert(path.clone()),
                FileEvent::Removed(path) => {
                    self.remove(path.as_str());
                    // 目录被删除：其下的文件都不再存在
                    let prefix = format!("{}/", path.as_str());
                    let gone: Vec<Arc<str>> = self
                        .by_path
                        .keys()
                        .filter(|k| k.starts_with(&prefix))
                        .cloned()
                        .collect();
                    for k in gone {
                        self.remove(&k);
                    }
                }
                FileEvent::Renamed { from, to } => {
                    let prefix = format!("{}/", from.as_str());
                    let moved: Vec<Arc<str>> = self
                        .by_path
                        .keys()
                        .filter(|k| k.starts_with(&prefix))
                        .cloned()
                        .collect();
                    for k in moved {
                        self.remove(&k);
                        let renamed = format!("{}/{}", to.as_str(), &k[prefix.len()..]);
                        if let Ok(p) = WorkspacePath::new(&renamed) {
                            self.insert(p);
                        }
                    }
                    if self.by_path.contains_key(from.as_str()) {
                        self.remove(from.as_str());
                        self.insert(to.clone());
                    }
                }
                FileEvent::Changed(_) => {}
                FileEvent::RescanRequired => rescan = true,
            }
        }
        rescan
    }
}

/// 打分后的一个候选。
pub struct Hit {
    pub entry: u32,
    pub score: ItemScore,
}

/// `search.quickOpen.history.filterSortOrder`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistorySort {
    /// 按相关度（默认）。
    Default,
    /// 按最近打开。
    Recency,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchOptions {
    pub include_history: bool,
    pub history_sort: HistorySort,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            include_history: true,
            history_sort: HistorySort::Default,
        }
    }
}

#[derive(Default)]
pub struct FileResults {
    /// 最近打开的文件里匹配的（无查询时是全部历史）。
    pub history: Vec<Hit>,
    /// 其余文件的匹配（已按相关度排序，最多 [`MAX_RESULTS`]）。
    pub files: Vec<Hit>,
}

struct Prune {
    original: String,
    generation: u64,
    candidates: Vec<u32>,
}

/// 一次快速打开会话里的搜索状态：记住上一次查询的匹配集合，用于前缀剪枝。
#[derive(Default)]
pub struct Search {
    prune: Option<Prune>,
}

impl Search {
    pub fn reset(&mut self) {
        self.prune = None;
    }

    /// 对 `query_text` 搜索。`history` 是最近打开的文件（最近的在前）。
    pub fn run(
        &mut self,
        index: &FileIndex,
        query_text: &str,
        history: &[WorkspacePath],
        options: SearchOptions,
    ) -> FileResults {
        let query = prepare_query(query_text);
        let mut results = FileResults::default();
        let history_entries: Vec<u32> = if options.include_history {
            history
                .iter()
                .filter_map(|p| index.find(p.as_str()))
                .collect()
        } else {
            Vec::new()
        };

        if query.normalized.is_empty() {
            // 没有有效查询：只列出历史
            self.prune = None;
            results.history = history_entries
                .into_iter()
                .map(|entry| Hit {
                    entry,
                    score: ItemScore::default(),
                })
                .collect();
            return results;
        }

        // 历史文件：打分；默认按相关度排序，`recency` 保持最近打开的顺序
        let mut in_history = std::collections::HashSet::new();
        for entry in history_entries {
            in_history.insert(entry);
            let Some(file) = index.entry(entry) else {
                continue;
            };
            let score = score_target(&file.target, &query, true);
            if score.score != 0 {
                results.history.push(Hit { entry, score });
            }
        }
        if options.history_sort == HistorySort::Default {
            insertion_sort(&mut results.history, |a, b| {
                compare_hits(index, a, b, &query)
            });
        }

        // 文件：只在上一次匹配集合里重新打分（查询是上一次的延长时）
        let candidates = self.candidates(index, &query);
        let mut matched: Vec<u32> = Vec::new();
        let mut top: Vec<Hit> = Vec::with_capacity(MAX_RESULTS + 1);
        let mut score_one = |entry: u32| {
            let Some(file) = index.entry(entry) else {
                return;
            };
            let score = score_target(&file.target, &query, true);
            if score.score == 0 {
                return;
            }
            matched.push(entry);
            if in_history.contains(&entry) {
                return;
            }
            let hit = Hit { entry, score };
            insert_top(&mut top, hit, index, &query);
        };
        match candidates {
            Some(list) => list.iter().for_each(|e| score_one(*e)),
            None => (0..index.len() as u32).for_each(&mut score_one),
        }
        results.files = top;
        self.prune = Some(Prune {
            original: query.original.clone(),
            generation: index.generation(),
            candidates: matched,
        });
        results
    }

    /// 上一次匹配集合（当且仅当本次查询是上一次的延长，且上一次有有效查询、没有引号）。
    fn candidates<'a>(&'a self, index: &FileIndex, query: &PreparedQuery) -> Option<&'a [u32]> {
        let prune = self.prune.as_ref()?;
        let extends = query.original.starts_with(&prune.original)
            && !prune.original.is_empty()
            && !prune.original.contains('"')
            && prune.generation == index.generation();
        extends.then_some(&prune.candidates[..])
    }
}

fn compare_hits(index: &FileIndex, a: &Hit, b: &Hit, query: &PreparedQuery) -> Ordering {
    let (Some(fa), Some(fb)) = (index.entry(a.entry), index.entry(b.entry)) else {
        return Ordering::Equal;
    };
    compare_scored(
        &Scored {
            score: &a.score,
            label: fa.target.label(),
            description: fa.target.description(),
            path: fa.target.path(),
        },
        &Scored {
            score: &b.score,
            label: fb.target.label(),
            description: fb.target.description(),
            path: fb.target.path(),
        },
        query,
    )
}

/// 维护“最好的 `MAX_RESULTS` 个”的有序列表。比较器不保证是严格全序（VS Code 的规则可能不传递），
/// 所以用二分查找插入而不是 `sort`：不会因为不一致的比较器 panic。
fn insert_top(top: &mut Vec<Hit>, hit: Hit, index: &FileIndex, query: &PreparedQuery) {
    if top.len() >= MAX_RESULTS {
        let worst = top.last().unwrap();
        if compare_hits(index, &hit, worst, query) != Ordering::Less {
            return;
        }
    }
    let at = top.partition_point(|x| compare_hits(index, x, &hit, query) != Ordering::Greater);
    top.insert(at, hit);
    top.truncate(MAX_RESULTS);
}

/// 稳定的插入排序：历史条目很少（几十个），且不依赖比较器是严格全序（`sort` 可能因此 panic）。
fn insertion_sort<T>(items: &mut [T], cmp: impl Fn(&T, &T) -> Ordering) {
    for i in 1..items.len() {
        let mut j = i;
        while j > 0 && cmp(&items[j - 1], &items[j]) == Ordering::Greater {
            items.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// 分组标签。
pub struct FileLabels {
    pub recent: Arc<str>,
    pub files: Arc<str>,
}

impl FileResults {
    pub fn is_empty(&self) -> bool {
        self.history.is_empty() && self.files.is_empty()
    }

    /// 转成列表行：历史在前（“最近打开”），其后是“文件结果”。
    pub fn picks(&self, index: &FileIndex, labels: &FileLabels) -> Picks {
        let mut picks = Picks::default();
        if !self.history.is_empty() {
            picks.separator(labels.recent.clone());
        }
        for hit in &self.history {
            push_hit(&mut picks, index, hit);
        }
        if !self.files.is_empty() {
            picks.separator(labels.files.clone());
        }
        for hit in &self.files {
            push_hit(&mut picks, index, hit);
        }
        picks
    }
}

fn push_hit(picks: &mut Picks, index: &FileIndex, hit: &Hit) {
    let Some(file) = index.entry(hit.entry) else {
        return;
    };
    let ranges = |matches: &Option<Vec<rove_fuzzy::Match>>, text: &str| -> Vec<Range<usize>> {
        matches
            .as_deref()
            .map(|m| utf16_to_byte_ranges(text, m))
            .unwrap_or_default()
    };
    let label_highlights = ranges(&hit.score.label_match, file.label());
    let detail_highlights = file
        .description()
        .map(|d| ranges(&hit.score.description_match, d))
        .unwrap_or_default();
    picks.push(
        Row {
            id: ItemId(file.path.as_str().into()),
            label: file.label().into(),
            detail: file.description().map(Into::into),
            depth: 0,
            expandable: false,
            expanded: false,
            disabled: false,
        },
        RowDecor {
            label_highlights,
            detail_highlights,
            trailing: None,
        },
    );
}
