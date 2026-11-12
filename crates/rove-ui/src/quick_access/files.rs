//! 快速打开的文件候选：工作区文件列表的增量索引，按键间的前缀剪枝，历史文件加权。
//!
//! 排序完全由 `rove-fuzzy`（VS Code `fuzzyScorer`）决定：label = 文件名，description = 相对目录，
//! path = 相对路径（分隔符为 `\`，与 VS Code 在 Windows 上一致）。
//!
//! 性能（大型 Unity 项目约 2.1 万个 `.cs` 文件外加其他文件，每次按键 < 16 ms）：
//! - 纯 ASCII 的路径（绝大多数）放在一整块连续的字节里，字符集合掩码单独放在紧凑的数组里：打分按顺序扫内存，
//!   绝大多数候选在掩码这一步就被否决；非 ASCII 的候选用预先算好 UTF-16 与小写形式的 [`Target`]；
//! - 打分只产生摘要（分数与匹配区间的首尾，不分配）；排名确定后才给前 [`MAX_RESULTS`] 个生成高亮区间；
//! - 查询是上一次查询的延长时，只在上一次的匹配集合里重新打分（前缀剪枝，见 [`Search`]）；
//! - 候选多时在线程池里分块并行打分，最后归并各块的前 N 名（VS Code 同样最多显示 512 条文件结果）。
use super::Picks;
use crate::{list_widget::quick_input::RowDecor, lists::ItemId, lists::Row};
use rove_base::WorkspacePath;
use rove_fuzzy::{
    ascii_masks, compare_keys, compare_scored, prepare_query, score_ascii_summary, score_item,
    score_target_summary, utf16_to_byte_ranges, CmpKey, CmpTexts, ItemScore, PreparedQuery, Scored,
    Summary, Target, LABEL_SCORE_THRESHOLD, SEP,
};
use rove_workspace::{EntryKind, FileEvent, Workspace};
use std::{
    cmp::Ordering,
    collections::HashMap,
    ops::Range,
    sync::{
        atomic::{AtomicI32, Ordering as AtomicOrdering},
        Arc,
    },
};

/// 文件结果的最大条数（VS Code `AnythingQuickAccessProvider.MAX_RESULTS`）。
pub const MAX_RESULTS: usize = 512;

/// 一个文件候选的冷数据（路径、非 ASCII 候选的预处理文本）；打分用的热数据在 [`FileIndex`] 的紧凑数组里。
#[derive(Debug)]
pub struct FileEntry {
    pub path: WorkspacePath,
    /// 非 ASCII 的候选：预先算好 UTF-16 与小写形式。ASCII 的候选为 `None`。
    wide: Option<Box<Target>>,
}

/// 打分用的热数据，每个候选 32 字节，按顺序存放。
#[derive(Clone, Copy, Debug, Default)]
struct Hot {
    /// ASCII 路径在 `arena` 里的位置与长度（非 ASCII 候选为 0）。
    offset: u32,
    len: u16,
    /// label（文件名）在路径里的起点；没有目录时为 0。
    label_start: u16,
    ascii: bool,
    label_mask: u64,
    combined_mask: u64,
}

/// 一个候选的显示文本。
pub struct FileText<'a> {
    pub label: &'a str,
    /// 相对目录（没有目录时为 `None`）。
    pub description: Option<&'a str>,
    /// 相对路径（分隔符为平台分隔符）。
    pub path: &'a str,
}

/// 工作区文件列表。查找按路径（区分大小写的显示路径）。
#[derive(Default)]
pub struct FileIndex {
    entries: Vec<FileEntry>,
    hot: Vec<Hot>,
    /// 全部 ASCII 路径依次追加（删除只留下空洞，见 `wasted`）。
    arena: Vec<u8>,
    wasted: usize,
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
        while let Ok(page) = workspace.files(start..start + PAGE) {
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
        let native = path.as_str().replace('/', &SEP.to_string());
        let label_start = native.rfind(SEP).map_or(0, |i| i + 1);
        let mut hot = Hot {
            label_start: label_start.min(u16::MAX as usize) as u16,
            ..Hot::default()
        };
        let wide = if native.is_ascii() && native.len() < u16::MAX as usize {
            let (label_mask, combined_mask) = ascii_masks(native.as_bytes(), label_start);
            hot = Hot {
                offset: self.arena.len() as u32,
                len: native.len() as u16,
                label_start: label_start as u16,
                ascii: true,
                label_mask,
                combined_mask,
            };
            self.arena.extend_from_slice(native.as_bytes());
            None
        } else {
            let (description, label) = native.split_at(label_start);
            let description = description.strip_suffix(SEP).unwrap_or(description);
            Some(Box::new(Target::new(
                label,
                (!description.is_empty()).then_some(description),
                Some(&native),
            )))
        };
        let key: Arc<str> = path.as_str().into();
        self.by_path.insert(key, self.entries.len());
        self.entries.push(FileEntry { path, wide });
        self.hot.push(hot);
        self.generation += 1;
    }

    fn remove(&mut self, path: &str) {
        let Some(at) = self.by_path.remove(path) else {
            return;
        };
        self.wasted += self.hot[at].len as usize;
        self.entries.swap_remove(at);
        self.hot.swap_remove(at);
        if at < self.entries.len() {
            let moved: Arc<str> = self.entries[at].path.as_str().into();
            self.by_path.insert(moved, at);
        }
        self.generation += 1;
        if self.wasted > 1 << 20 && self.wasted > self.arena.len() / 2 {
            self.compact();
        }
    }

    /// 删除留下的空洞超过一半时重排字节块。
    fn compact(&mut self) {
        let mut arena = Vec::with_capacity(self.arena.len() - self.wasted);
        for hot in &mut self.hot {
            if hot.ascii {
                let from = hot.offset as usize;
                hot.offset = arena.len() as u32;
                arena.extend_from_slice(&self.arena[from..from + hot.len as usize]);
            }
        }
        self.arena = arena;
        self.wasted = 0;
    }

    /// 候选的显示文本。
    pub fn text(&self, index: u32) -> Option<FileText<'_>> {
        let i = index as usize;
        let entry = self.entries.get(i)?;
        if let Some(target) = &entry.wide {
            return Some(FileText {
                label: target.label(),
                description: target.description(),
                path: target.path().unwrap_or(""),
            });
        }
        let hot = &self.hot[i];
        let bytes = &self.arena[hot.offset as usize..hot.offset as usize + hot.len as usize];
        // SAFETY: `arena` 只会追加 `insert` 里检查过的纯 ASCII 字节。
        let path = unsafe { std::str::from_utf8_unchecked(bytes) };
        let start = hot.label_start as usize;
        Some(FileText {
            label: &path[start..],
            description: (start > 0).then(|| &path[..start - 1]),
            path,
        })
    }

    /// 打分（摘要）。`ascii_query` 为真且候选是 ASCII 时走紧凑的字节路径。
    fn summary(
        &self,
        index: u32,
        query: &PreparedQuery,
        ascii_query: bool,
        combined_is_useless: bool,
    ) -> Summary {
        let i = index as usize;
        let hot = &self.hot[i];
        if hot.ascii && ascii_query {
            let bytes = &self.arena[hot.offset as usize..hot.offset as usize + hot.len as usize];
            return score_ascii_summary(
                bytes,
                hot.label_start as usize,
                (hot.label_mask, hot.combined_mask),
                query,
                true,
                combined_is_useless,
            );
        }
        match &self.entries[i].wide {
            Some(target) => score_target_summary(target, query, true),
            None => match self.text(index) {
                Some(t) => {
                    let target = Target::new(t.label, t.description, Some(t.path));
                    score_target_summary(&target, query, true)
                }
                None => Summary::default(),
            },
        }
    }

    /// 完整打分（带匹配区间，用来高亮）。
    pub fn full_score(&self, index: u32, query: &PreparedQuery) -> ItemScore {
        match self.text(index) {
            Some(t) => score_item(t.label, t.description, Some(t.path), query, true),
            None => ItemScore::default(),
        }
    }

    /// 比较器用的长度（UTF-16 码元）：label、description、路径。
    fn cmp_lens(&self, index: u32) -> (u32, u32, u32) {
        let Some(t) = self.text(index) else {
            return (0, 0, 0);
        };
        let ascii = self.hot[index as usize].ascii;
        let units = |s: &str| -> u32 {
            if ascii {
                s.len() as u32
            } else {
                s.chars().map(char::len_utf16).sum::<usize>() as u32
            }
        };
        (
            units(t.label),
            t.description.map_or(0, units),
            units(t.path),
        )
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
    /// 打分线程数；`None` 按候选数量与 CPU 核数决定（测试与基准可以固定）。
    threads: Option<usize>,
}

impl Search {
    pub fn reset(&mut self) {
        self.prune = None;
    }

    /// 固定打分线程数（`None` 恢复自动）。
    pub fn set_threads(&mut self, threads: Option<usize>) {
        self.threads = threads;
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
            let score = index.full_score(entry, &query);
            if score.score != 0 {
                results.history.push(Hit { entry, score });
            }
        }
        if options.history_sort == HistorySort::Default {
            insertion_sort(&mut results.history, |a, b| {
                compare_hits(index, a, b, &query)
            });
        }

        // 文件：只在上一次匹配集合里重新打分（查询是上一次的延长时）；候选多时分给几个线程。
        let candidates = self.candidates(index, &query);
        let total = candidates.map_or(index.len(), <[u32]>::len);
        let threads = self.threads.unwrap_or_else(|| threads_for(total));
        let chunks = split(candidates, index.len(), threads);
        // 各块共享“第 MAX_RESULTS 名的分数下限”：任何一块的前 N 名满了，别的块里低于它的候选就不用考虑了
        // （否则每块都要从头填满自己的前 N 名，插入与比较的总量是单线程的几倍）。
        let floor = AtomicI32::new(0);
        let work = |chunk: Chunk| score_chunk(chunk, index, &query, &in_history, &floor);
        let parts: Vec<Partial> = if chunks.len() <= 1 {
            chunks.into_iter().map(work).collect()
        } else {
            // 常驻线程池：每次按键新开线程的固定开销（Windows 上以毫秒计）比打分本身还大。
            use rayon::prelude::*;
            chunks.into_par_iter().map(work).collect()
        };
        let mut matched: Vec<u32> = Vec::with_capacity(parts.iter().map(|p| p.matched.len()).sum());
        let mut tops = Vec::with_capacity(parts.len());
        for part in parts {
            matched.extend(part.matched);
            tops.push(part.top);
        }
        results.files = merge_tops(tops, index, &query)
            .into_iter()
            .map(|hit| Hit {
                entry: hit.entry,
                score: index.full_score(hit.entry, &query),
            })
            .collect();
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
    let (Some(fa), Some(fb)) = (index.text(a.entry), index.text(b.entry)) else {
        return Ordering::Equal;
    };
    compare_scored(
        &Scored {
            score: &a.score,
            label: fa.label,
            description: fa.description,
            path: Some(fa.path),
        },
        &Scored {
            score: &b.score,
            label: fb.label,
            description: fb.description,
            path: Some(fb.path),
        },
        query,
    )
}

/// 一块候选：上一次匹配集合的一段，或索引下标的一个区间。
#[derive(Clone)]
enum Chunk<'a> {
    List(&'a [u32]),
    Range(Range<u32>),
}

/// 一块的结果：所有匹配的下标（供下一次按键剪枝）与这一块里最好的 [`MAX_RESULTS`] 个。
struct Partial {
    matched: Vec<u32>,
    top: Vec<SummaryHit>,
}

/// 搜索过程中的候选：只有分数与匹配区间的首尾（不分配）；排名确定后才为前 N 名生成完整的高亮区间。
struct SummaryHit {
    entry: u32,
    summary: Summary,
    /// 比较器的整数部分：每次比较不再去取候选的字符串。
    label_len: u32,
    description_len: u32,
    path_len: u32,
}

/// 候选少于这个数时不开线程（线程启动的固定开销比打分还大）。
const PARALLEL_THRESHOLD: usize = 12_000;

fn threads_for(total: usize) -> usize {
    if total < PARALLEL_THRESHOLD {
        return 1;
    }
    rayon::current_num_threads()
        .clamp(1, 16)
        .min(total / (PARALLEL_THRESHOLD / 2))
        .max(1)
}

fn split<'a>(candidates: Option<&'a [u32]>, all: usize, parts: usize) -> Vec<Chunk<'a>> {
    let total = candidates.map_or(all, <[u32]>::len);
    let size = total.div_ceil(parts.max(1)).max(1);
    (0..total.div_ceil(size))
        .map(|i| {
            let (start, end) = (i * size, ((i + 1) * size).min(total));
            match candidates {
                Some(list) => Chunk::List(&list[start..end]),
                None => Chunk::Range(start as u32..end as u32),
            }
        })
        .collect()
}

fn score_chunk(
    chunk: Chunk,
    index: &FileIndex,
    query: &PreparedQuery,
    in_history: &std::collections::HashSet<u32>,
    floor: &AtomicI32,
) -> Partial {
    let mut matched: Vec<u32> = Vec::new();
    let mut top: Vec<SummaryHit> = Vec::with_capacity(MAX_RESULTS + 1);
    let ascii_query = query.is_ascii();
    let mut score_one = |entry: u32| {
        // 前 N 名已经全是 label 级的结果时，只在 description 里匹配的候选排不进去，不必再为它们跑矩阵。
        let useless = floor.load(AtomicOrdering::Relaxed) >= LABEL_SCORE_THRESHOLD;
        let summary = index.summary(entry, query, ascii_query, useless);
        if summary.score == 0 {
            return;
        }
        matched.push(entry);
        if in_history.contains(&entry) {
            return;
        }
        // 排序的第一关键字就是分数：低于已知的第 MAX_RESULTS 名的直接丢弃。
        if summary.score < floor.load(AtomicOrdering::Relaxed) {
            return;
        }
        let (label_len, description_len, path_len) = index.cmp_lens(entry);
        let hit = SummaryHit {
            entry,
            summary,
            label_len,
            description_len,
            path_len,
        };
        insert_top(&mut top, hit, index, query);
        if top.len() >= MAX_RESULTS {
            if let Some(worst) = top.last() {
                floor.fetch_max(worst.summary.score, AtomicOrdering::Relaxed);
            }
        }
    };
    match chunk {
        Chunk::List(list) => list.iter().for_each(|e| score_one(*e)),
        Chunk::Range(range) => range.for_each(score_one),
    }
    Partial { matched, top }
}

fn cmp_key(hit: &SummaryHit) -> CmpKey {
    CmpKey {
        score: hit.summary.score,
        label_span: hit.summary.label,
        description_span: hit.summary.description,
        label_len: hit.label_len as usize,
        description_len: hit.description_len as usize,
        path_len: hit.path_len as usize,
        has_path: hit.path_len > 0,
    }
}

fn compare_summary_hits(
    index: &FileIndex,
    a: &SummaryHit,
    b: &SummaryHit,
    query: &PreparedQuery,
) -> Ordering {
    compare_keys(&cmp_key(a), &cmp_key(b), query, || {
        let text = |hit: &SummaryHit| {
            index.text(hit.entry).map_or(
                CmpTexts {
                    label: "",
                    description: None,
                    path: None,
                },
                |t| CmpTexts {
                    label: t.label,
                    description: t.description,
                    path: Some(t.path),
                },
            )
        };
        (text(a), text(b))
    })
}

/// 把各块已排好序的前 N 名归并成全局的前 N 名（逐个取各块队首里最好的）。
fn merge_tops(
    tops: Vec<Vec<SummaryHit>>,
    index: &FileIndex,
    query: &PreparedQuery,
) -> Vec<SummaryHit> {
    if tops.len() == 1 {
        return tops.into_iter().next().unwrap_or_default();
    }
    let mut iters: Vec<std::vec::IntoIter<SummaryHit>> =
        tops.into_iter().map(Vec::into_iter).collect();
    let mut heads: Vec<Option<SummaryHit>> = iters.iter_mut().map(Iterator::next).collect();
    let mut merged = Vec::with_capacity(MAX_RESULTS);
    while merged.len() < MAX_RESULTS {
        let mut best: Option<usize> = None;
        for (i, head) in heads.iter().enumerate() {
            let Some(head) = head else { continue };
            let better = match best.and_then(|b| heads[b].as_ref()) {
                Some(current) => {
                    compare_summary_hits(index, head, current, query) == Ordering::Less
                }
                None => true,
            };
            if better {
                best = Some(i);
            }
        }
        let Some(best) = best else { break };
        merged.extend(heads[best].take());
        heads[best] = iters[best].next();
    }
    merged
}

/// 维护“最好的 `MAX_RESULTS` 个”的有序列表。比较器不保证是严格全序（VS Code 的规则可能不传递），
/// 所以用二分查找插入而不是 `sort`：不会因为不一致的比较器 panic。
fn insert_top(
    top: &mut Vec<SummaryHit>,
    hit: SummaryHit,
    index: &FileIndex,
    query: &PreparedQuery,
) {
    if top.len() >= MAX_RESULTS {
        let worst = top.last().unwrap();
        if compare_summary_hits(index, &hit, worst, query) != Ordering::Less {
            return;
        }
    }
    let at =
        top.partition_point(|x| compare_summary_hits(index, x, &hit, query) != Ordering::Greater);
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
    let (Some(file), Some(text)) = (index.entry(hit.entry), index.text(hit.entry)) else {
        return;
    };
    let ranges = |matches: &Option<Vec<rove_fuzzy::Match>>, text: &str| -> Vec<Range<usize>> {
        matches
            .as_deref()
            .map(|m| utf16_to_byte_ranges(text, m))
            .unwrap_or_default()
    };
    let label_highlights = ranges(&hit.score.label_match, text.label);
    let detail_highlights = text
        .description
        .map(|d| ranges(&hit.score.description_match, d))
        .unwrap_or_default();
    picks.push(
        Row {
            id: ItemId(file.path.as_str().into()),
            label: text.label.into(),
            detail: text.description.map(Into::into),
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
