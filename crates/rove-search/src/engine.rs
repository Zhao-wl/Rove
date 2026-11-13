//! 并行流式搜索（#25）。
//!
//! - **文件集合**：用与 #15 工作区扫描相同的 walker 设置遍历工作区根目录——所有忽略文件开关全部关闭，
//!   **从不读取 `.gitignore`**；排除规则是 `files.exclude` 叠在 `rove.files.extraExclude` 之上
//!   （[`RuleSet::merged`]，与工作区同一套实现）再加 `search.exclude`，以及搜索视图里“排除的文件”。
//!   搜索自己遍历而不是读工作区快照，是因为 `search.followSymlinks`（默认 true）与“不使用排除设置”都需要
//!   快照之外的文件；被剪掉的目录顺带回答“是否只因 Rove 的附加排除而被排除”（空状态的提示用）。
//! - **并行**：`ignore` 的并行 walker 在每个工作线程里直接搜索遍历到的文件（ripgrep 的做法）；
//!   线程数取 `search.ripgrep.maxThreads`，0 为自动。
//! - **流式**：每个有匹配的文件整体经通道送回调用线程，调用线程按批（第一批立即，之后约 30 ms 一批）交给回调。
//! - **取消**：取消令牌传到每个工作线程的内层循环——每个目录项、每个文件、每 256 KiB 的块、每一处匹配都检查，
//!   所以取消后几毫秒内所有线程退出；[`search`] 返回时工作线程已全部结束。
//! - **编码**：先按字节快速判定（BOM、合法 UTF-8），只有疑似非 UTF-8 的文件才走 #7 的完整编码识别
//!   （`files.autoGuessEncoding`，GBK 中文能搜到）。含 NUL 的文件按二进制跳过（与 ripgrep / VS Code 一致）。
//!   走内存映射的大文件同样识别编码（#145）：编辑器的大文件档从 20 MB 起，17 MiB 的 GBK 文件在编辑器里
//!   是正常文本，搜索不能漏掉其中的中文字面量。解码有上限（[`MAX_DECODE`]，64 MB）：再大的非 UTF-8
//!   文件按原始字节搜索，免得为一次搜索占住文件几倍大小的内存。
//! - **行列口径**：报告的行列与编辑器（`rove-buffer` 的 `TextSource::position`）一致——搜索器只认 `\n`，
//!   这里再把 `\n` 行按独立 `\r` 切开（编辑器把 `\r` 也算换行，#145）。
//! - **零宽匹配**：`^`、`$`、`\b` 这类零宽命中保留为空区间（与文件内查找 `rove-ui::find` 的约定一致），
//!   匹配器的 `find_iter` 遇到空匹配会前进一个字节，不会死循环。
//!   已知限制：切段只改行列的换算，正则引擎的行终止符仍然只有 `\n`，所以在**独立 `\r`** 分出来的段上，
//!   `^` / `$` 不会各自命中（`ab\rcd` 只报一处 `^`、一处 `$`），`.` 与 `[^x]` 也能跨过这个 `\r`
//!   （跨段的区间截到段末）。独立 `\r` 的文件极少（经典 Mac 换行），行号与列号本身仍然与编辑器一致。
//! - **上限**：`search.maxResults` 按匹配处数计；达到上限时停止全部线程并在摘要里报告 `limit_hit`。

use crate::query::{self, SearchQuery};
use crate::settings::SearchConfig;
use grep_matcher::{LineTerminator, Matcher};
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::{DirEntry, WalkBuilder, WalkState};
use rove_base::{Cancellation, Error, Result, WorkspacePath};
use rove_workspace::RuleSet;
use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 一次 `search_slice` 处理的最大字节数：内层循环在块之间检查取消。按时间而不是字节定间隔：
/// release 下 256 KiB 约 0.3 ms，调试构建的正则引擎慢一个数量级，用 64 KiB 让取消间隔同样在几毫秒内。
const CHUNK: usize = if cfg!(debug_assertions) {
    64 << 10
} else {
    256 << 10
};
/// 超过这个大小的文件用内存映射读取（不复制；仍然看 BOM、二进制与编码）。
const MMAP_THRESHOLD: u64 = 16 << 20;
/// 走内存映射的大小阈值。测试里可以调低（见 [`set_mmap_threshold`]），免得为了覆盖这条分支
/// 造一个 17 MiB 的样本文件。
#[cfg(test)]
static MMAP_THRESHOLD_OVERRIDE: AtomicU64 = AtomicU64::new(MMAP_THRESHOLD);

#[cfg(not(test))]
fn mmap_threshold() -> u64 {
    MMAP_THRESHOLD
}

#[cfg(test)]
fn mmap_threshold() -> u64 {
    MMAP_THRESHOLD_OVERRIDE.load(Ordering::Relaxed)
}

/// 阈值是进程内全局的，而 libtest 并发跑用例：这把锁让“调低 → 搜索 → 还原”整段互斥，
/// 不靠“别同时用”的约定。
#[cfg(test)]
static MMAP_THRESHOLD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 仅测试用：在作用域内临时调低内存映射阈值，析构时还原。
#[cfg(test)]
struct MmapThresholdGuard {
    /// 只为了持锁到析构。
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl MmapThresholdGuard {
    fn new(bytes: u64) -> Self {
        // 别的用例 panic 时不让这把锁跟着中毒：阈值由析构无条件还原，锁本身没有需要保护的不变量。
        let lock = MMAP_THRESHOLD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        MMAP_THRESHOLD_OVERRIDE.store(bytes, Ordering::Relaxed);
        Self { _lock: lock }
    }
}

#[cfg(test)]
impl Drop for MmapThresholdGuard {
    fn drop(&mut self) {
        MMAP_THRESHOLD_OVERRIDE.store(MMAP_THRESHOLD, Ordering::Relaxed);
    }
}

/// 二进制判定只看文件开头这么多字节里有没有 NUL（与 ripgrep 的快速判定一致）。
const BINARY_PROBE: usize = 8 << 10;
/// 需要转码的文件最多解码这么多字节（#145）。`rove_buffer::open_bytes` 会在内存里建一棵完整的
/// Rope、这里再复制成一段 UTF-8，峰值约文件大小的 2~3 倍；超过这个上限的非 UTF-8 文件退回按原始
/// 字节搜索（ASCII 部分仍能搜到）。取 `rove.largeFile.threshold` 的默认值 64 MB：编辑器从这里起
/// 只读映射、不再当成普通文本，搜索的解码上限跟着它走。
const MAX_DECODE: usize = 64 << 20;
/// 行预览最多保留的字节数；更长的行只保留第一处匹配附近的一段。
const PREVIEW_BYTES: usize = 1000;
/// 预览在第一处匹配之前保留的字节数。
const PREVIEW_LEAD: usize = 100;
/// 第一批之后，结果每隔这么久交给回调一次。
const BATCH_INTERVAL: Duration = Duration::from_millis(30);

/// 一行里的匹配。行与列都按编辑器口径：行终止符包括 `\n`、`\r\n` 与独立的 `\r`，
/// 列是该行解码后文本的 UTF-8 字节列（`Utf8Column`，不是磁盘字节）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineMatch {
    /// 从 0 开始的行号。
    pub line: u64,
    /// 行预览（不含行终止符；超长行只是一段，前导空白已去掉）。
    pub preview: Arc<str>,
    /// `preview` 第一个字节在该行中的列。
    pub preview_start: u32,
    /// 匹配区间（行内列，半开）。跨行的匹配截到本行末尾；`^`、`$` 这类零宽命中是空区间。
    pub ranges: Vec<Range<u32>>,
}

impl LineMatch {
    /// 匹配区间换算成 `preview` 内的字节区间（落在预览之外的部分被截掉）。
    /// 零宽命中（空区间）落在预览起点上时保留，不当作“在预览之前”丢掉。
    pub fn preview_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        let start = self.preview_start as usize;
        let len = self.preview.len();
        self.ranges.iter().filter_map(move |r| {
            let s = (r.start as usize).saturating_sub(start).min(len);
            let e = (r.end as usize).saturating_sub(start).min(len);
            let visible = r.end as usize > start || (r.start == r.end && r.end as usize >= start);
            (visible && s <= e).then_some(s..e)
        })
    }
}

/// 一个文件的全部匹配（按行号升序）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMatches {
    pub path: WorkspacePath,
    pub lines: Vec<LineMatch>,
}

impl FileMatches {
    /// 匹配处数（`search.maxResults` 的计数口径）。
    pub fn match_count(&self) -> usize {
        self.lines.iter().map(|l| l.ranges.len()).sum()
    }
}

/// 一次搜索的结果摘要。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchSummary {
    pub files_searched: u64,
    pub files_with_matches: u64,
    pub matches: u64,
    /// 达到 `search.maxResults`，后面的结果没有搜。
    pub limit_hit: bool,
    /// 有目录或文件**只**因为 `rove.files.extraExclude`（Unity 生成的目录等）被排除。
    pub excluded_by_rove_defaults: bool,
    /// 读不了的文件与二进制文件。
    pub skipped: u64,
}

struct Shared {
    matcher: RegexMatcher,
    multi_line: bool,
    /// 纯 ASCII 的字面量查询：先在原始字节上判定“有没有匹配”，没有就不必做 UTF-8 校验与编码识别
    /// （GBK 等 ASCII 兼容编码解码后不会凭空出现 ASCII 匹配）。
    ascii_literal: bool,
    cancel: Cancellation,
    /// 剩余可报告的匹配数；`None` 不限。
    budget: Option<AtomicUsize>,
    limit_hit: AtomicBool,
    files_searched: AtomicU64,
    skipped: AtomicU64,
    settings: Arc<dyn rove_config::Settings>,
}

impl Shared {
    /// 申请 `n` 处匹配的配额，返回实际得到的数量（用完时触发上限、停止全部线程）。
    fn claim(&self, n: usize) -> usize {
        let Some(budget) = &self.budget else {
            return n;
        };
        let mut current = budget.load(Ordering::Relaxed);
        loop {
            let granted = current.min(n);
            match budget.compare_exchange_weak(
                current,
                current - granted,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    if current == granted {
                        // 配额用完（达到 `search.maxResults`）：停止全部工作线程。
                        self.limit_hit.store(true, Ordering::Relaxed);
                        self.cancel.cancel();
                    }
                    return granted;
                }
                Err(actual) => current = actual,
            }
        }
    }
}

/// 在 `root` 下搜索。结果按批交给 `on_batch`（在调用线程上执行）；返回时所有工作线程都已结束。
///
/// 错误：查询为空或正则非法 → `InvalidInput`；`cancel` 被取消 → `Cancelled`（取消前已交出的批次仍然有效，
/// 由调用方按代次丢弃）。达到 `search.maxResults` 不是错误，见 [`SearchSummary::limit_hit`]。
pub fn search(
    root: &Path,
    query: &SearchQuery,
    config: &SearchConfig,
    cancel: &Cancellation,
    mut on_batch: impl FnMut(Vec<FileMatches>),
) -> Result<SearchSummary> {
    let matcher = query::compile(query, config.smart_case)?;
    cancel.check()?;
    // 工作线程用子作用域：达到上限时只停本次搜索，不动调用方的令牌。
    let inner = cancel.child();
    let shared = Arc::new(Shared {
        matcher,
        multi_line: query.is_multiline(),
        ascii_literal: !query.is_regex && query.pattern.is_ascii(),
        cancel: inner.clone(),
        budget: config.max_results.map(AtomicUsize::new),
        limit_hit: AtomicBool::new(false),
        files_searched: AtomicU64::new(0),
        skipped: AtomicU64::new(0),
        settings: config.settings.clone(),
    });
    let rove_excluded = Arc::new(AtomicBool::new(false));
    let includes = query::rules(&query.includes).map(Arc::new);
    let walker = build_walker(root, query, config, &inner, &rove_excluded);

    let (sender, receiver) = mpsc::channel::<FileMatches>();
    let mut summary = SearchSummary::default();
    std::thread::scope(|scope| {
        let shared_for_walk = shared.clone();
        let root = root.to_path_buf();
        scope.spawn(move || {
            walker.run(|| {
                let sender = sender.clone();
                let shared = shared_for_walk.clone();
                let includes = includes.clone();
                let root = root.clone();
                let mut searcher = searcher(shared.multi_line);
                let mut worker = Worker::default();
                Box::new(move |result| {
                    visit(
                        result,
                        &root,
                        &shared,
                        includes.as_deref(),
                        &mut searcher,
                        &mut worker,
                        &sender,
                    )
                })
            });
            drop(sender);
        });

        let mut batch: Vec<FileMatches> = Vec::new();
        let mut last = Instant::now();
        let mut first = true;
        let mut flush = |batch: &mut Vec<FileMatches>, summary: &mut SearchSummary| {
            for file in batch.iter() {
                summary.files_with_matches += 1;
                summary.matches += file.match_count() as u64;
            }
            on_batch(std::mem::take(batch));
        };
        loop {
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(file) => {
                    batch.push(file);
                    if first || last.elapsed() >= BATCH_INTERVAL {
                        flush(&mut batch, &mut summary);
                        first = false;
                        last = Instant::now();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !batch.is_empty() && last.elapsed() >= BATCH_INTERVAL {
                        flush(&mut batch, &mut summary);
                        last = Instant::now();
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if !batch.is_empty() {
            flush(&mut batch, &mut summary);
        }
    });
    cancel.check()?;
    summary.files_searched = shared.files_searched.load(Ordering::Relaxed);
    summary.skipped = shared.skipped.load(Ordering::Relaxed);
    summary.limit_hit = shared.limit_hit.load(Ordering::Relaxed);
    summary.excluded_by_rove_defaults = rove_excluded.load(Ordering::Relaxed);
    Ok(summary)
}

fn searcher(multi_line: bool) -> Searcher {
    SearcherBuilder::new()
        .line_number(true)
        .line_terminator(LineTerminator::byte(b'\n'))
        .multi_line(multi_line)
        .build()
}

fn build_walker(
    root: &Path,
    query: &SearchQuery,
    config: &SearchConfig,
    cancel: &Cancellation,
    rove_excluded: &Arc<AtomicBool>,
) -> ignore::WalkParallel {
    let mut builder = WalkBuilder::new(root);
    builder
        // 硬约束：从不读取 `.gitignore` / `.git/info/exclude` / 全局忽略文件 / `.ignore`，也不隐藏点文件。
        // 与 #15 的工作区扫描逐项相同；`search.useIgnoreFiles` 等三项设置一律忽略。
        .standard_filters(false)
        .hidden(false)
        .ignore(false)
        .parents(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(config.follow_symlinks);
    if config.threads > 0 {
        builder.threads(config.threads);
    }
    let settings = query.use_exclude_settings.then(|| {
        (
            config.files_exclude.clone(),
            config.files_exclude_only.clone(),
            config.search_exclude.clone(),
        )
    });
    let user_excludes = query::rules(&query.excludes);
    let root = root.to_path_buf();
    let cancel = cancel.clone();
    let rove_excluded = rove_excluded.clone();
    let follow = config.follow_symlinks;
    builder.filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }
        if cancel.check().is_err() {
            return false;
        }
        if !follow && entry.path_is_symlink() {
            return false;
        }
        let Some(relative) = relative(&root, entry.path()) else {
            return false;
        };
        if let Some((merged, files_only, search)) = &settings {
            let by_files = merged.matches_self(&relative) == Some(true);
            let by_search = search.matches_self(&relative) == Some(true);
            if by_files || by_search {
                if !by_search && files_only.matches_self(&relative) != Some(true) {
                    rove_excluded.store(true, Ordering::Relaxed);
                }
                return false;
            }
        }
        if let Some(excludes) = &user_excludes {
            if excludes.matches_self(&relative) == Some(true) {
                return false;
            }
        }
        true
    });
    builder.build_parallel()
}

/// 工作区相对路径（`/` 分隔）；不能表示为 UTF-8 的路径返回 `None`（与工作区扫描一样跳过）。
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?.to_str()?;
    if rel.is_empty() {
        return None;
    }
    Some(if cfg!(windows) {
        rel.replace('\\', "/")
    } else {
        rel.to_string()
    })
}

/// 每个工作线程复用的缓冲。
#[derive(Default)]
struct Worker {
    lines: Vec<LineMatch>,
}

fn visit(
    result: std::result::Result<DirEntry, ignore::Error>,
    root: &Path,
    shared: &Shared,
    includes: Option<&RuleSet>,
    searcher: &mut Searcher,
    worker: &mut Worker,
    sender: &Sender<FileMatches>,
) -> WalkState {
    if shared.cancel.check().is_err() {
        return WalkState::Quit;
    }
    let entry = match result {
        Ok(entry) => entry,
        Err(_) => {
            shared.skipped.fetch_add(1, Ordering::Relaxed);
            return WalkState::Continue;
        }
    };
    if !entry.file_type().is_some_and(|t| t.is_file()) {
        return WalkState::Continue;
    }
    let Some(relative) = relative(root, entry.path()) else {
        return WalkState::Continue;
    };
    if let Some(includes) = includes {
        if includes.matches(&relative) != Some(true) {
            return WalkState::Continue;
        }
    }
    let Ok(path) = WorkspacePath::new(&relative) else {
        return WalkState::Continue;
    };
    shared.files_searched.fetch_add(1, Ordering::Relaxed);
    worker.lines.clear();
    match search_file(entry.path(), shared, searcher, &mut worker.lines) {
        Ok(()) => {}
        Err(Error::Cancelled) => {}
        Err(_) => {
            shared.skipped.fetch_add(1, Ordering::Relaxed);
        }
    }
    if !worker.lines.is_empty() {
        let lines = std::mem::take(&mut worker.lines);
        if sender.send(FileMatches { path, lines }).is_err() {
            return WalkState::Quit;
        }
    }
    if shared.cancel.check().is_err() {
        WalkState::Quit
    } else {
        WalkState::Continue
    }
}

/// 读取并解码一个文件，然后分块搜索。二进制文件返回 `Unsupported`（计入跳过）。
fn search_file(
    path: &Path,
    shared: &Shared,
    searcher: &mut Searcher,
    out: &mut Vec<LineMatch>,
) -> Result<()> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(());
    }
    if len > mmap_threshold() {
        // SAFETY: 只读映射；文件在搜索期间被别人截断属于外部竞争，最坏结果是这次读到的内容不完整，
        // 与 ripgrep 的取舍相同。
        let map = unsafe { memmap2::Mmap::map(&file)? };
        // 顺序与小文件一致，二进制判定必须在字面量扫描之前：Unity 项目里几百 MB 的贴图、模型、
        // 程序集只看开头 8 KiB 就跳过（并计入摘要的 `skipped`），不会为了一次“多半没有”的
        // 字面量扫描把整个映射全部换页进来。
        let utf16 = map.starts_with(b"\xFF\xFE") || map.starts_with(b"\xFE\xFF");
        if !utf16 && memchr::memchr(0, &map[..map.len().min(BINARY_PROBE)]).is_some() {
            return Err(Error::Unsupported);
        }
        if shared.ascii_literal && !utf16 && shared.matcher.find(&map).ok().flatten().is_none() {
            // 与小文件同样的快速路径：ASCII 字面量在原始字节里都没有，解码后也不会有。
            return Ok(());
        }
        // 大文件也做 UTF-8 校验与编码识别（#145）：校验只是一次顺序扫描，真的不是 UTF-8 才解码。
        let text = decode(&map, shared)?;
        return search_text(&text, shared, searcher, out);
    }
    // 先只读开头一段做二进制判定：Unity 项目里贴图、模型、程序集占了绝大多数字节，
    // 判成二进制的文件不再读剩下的内容（ripgrep 同样边读边判定）。
    use std::io::Read;
    let mut file = file;
    let mut bytes = Vec::with_capacity(len as usize + 1);
    (&mut file)
        .take(BINARY_PROBE as u64)
        .read_to_end(&mut bytes)?;
    let utf16 = bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF");
    if !utf16 && memchr::memchr(0, &bytes).is_some() {
        return Err(Error::Unsupported);
    }
    file.read_to_end(&mut bytes)?;
    if shared.ascii_literal && !utf16 && shared.matcher.find(&bytes).ok().flatten().is_none() {
        // 绝大多数文件没有匹配：一次 memchr 级别的扫描就结束，不校验、不解码、不分行。
        return Ok(());
    }
    let text = decode(&bytes, shared)?;
    search_text(&text, shared, searcher, out)
}

/// 字节 → 用来搜索的 UTF-8 文本。
///
/// 1. UTF-8 BOM：去掉 BOM（行首列号与编辑器一致）；UTF-16 BOM：转码；
/// 2. 开头 8 KiB 有 NUL：二进制，跳过；
/// 3. 合法 UTF-8（绝大多数文件）：原样搜索，不复制；
/// 4. 否则交给 #7 的编码识别（`files.autoGuessEncoding`、候选编码 GBK…）解码；识别失败、
///    或者文件大过 [`MAX_DECODE`]，按原始字节搜索（与 ripgrep 一样，ASCII 部分仍能搜到）。
fn decode<'a>(bytes: &'a [u8], shared: &Shared) -> Result<Cow<'a, [u8]>> {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return Ok(Cow::Borrowed(rest));
    }
    if bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF") {
        let (encoding, bom) = encoding_rs::Encoding::for_bom(bytes).ok_or(Error::Unsupported)?;
        let (text, _) = encoding.decode_without_bom_handling(&bytes[bom..]);
        return Ok(Cow::Owned(text.into_owned().into_bytes()));
    }
    let probe = &bytes[..bytes.len().min(BINARY_PROBE)];
    if memchr::memchr(0, probe).is_some() {
        return Err(Error::Unsupported);
    }
    if std::str::from_utf8(bytes).is_ok() {
        return Ok(Cow::Borrowed(bytes));
    }
    if bytes.len() > MAX_DECODE {
        // 解码的峰值内存是文件的几倍（见 [`MAX_DECODE`]）：太大的文件退回按原始字节搜索。
        return Ok(Cow::Borrowed(bytes));
    }
    let options = rove_buffer::LoadOptions::new("", &*shared.settings);
    match rove_buffer::open_bytes(bytes, &options) {
        Ok(loaded) => {
            use rove_buffer::TextSource;
            let text = &loaded.text;
            let mut out = String::with_capacity(bytes.len() + bytes.len() / 2);
            let range = rove_base::ByteRange::new(
                rove_base::ByteOffset(0),
                rove_base::ByteOffset(text.byte_len()),
            )?;
            text.visit_chunks(range, &mut |chunk| out.push_str(chunk))?;
            Ok(Cow::Owned(out.into_bytes()))
        }
        Err(_) => Ok(Cow::Borrowed(bytes)),
    }
}

/// 分块搜索：每块约 256 KiB、在行边界切开，块之间检查取消。多行正则需要整段文本，不分块。
fn search_text(
    text: &[u8],
    shared: &Shared,
    searcher: &mut Searcher,
    out: &mut Vec<LineMatch>,
) -> Result<()> {
    if shared.multi_line || text.len() <= CHUNK {
        return search_chunk(text, 0, shared, searcher, out);
    }
    let mut start = 0;
    let mut base_line = 0u64;
    while start < text.len() {
        shared.cancel.check()?;
        let mut end = (start + CHUNK).min(text.len());
        if end < text.len() {
            end = match memchr::memchr(b'\n', &text[end..]) {
                Some(i) => end + i + 1,
                None => text.len(),
            };
        }
        let chunk = &text[start..end];
        search_chunk(chunk, base_line, shared, searcher, out)?;
        // 块在 `\n` 之后切开，`\r\n` 不会被切断；行号按编辑器口径累加（独立 `\r` 也是一行）。
        base_line += line_breaks(chunk);
        start = end;
    }
    Ok(())
}

fn search_chunk(
    chunk: &[u8],
    base_line: u64,
    shared: &Shared,
    searcher: &mut Searcher,
    out: &mut Vec<LineMatch>,
) -> Result<()> {
    let mut sink = Collect {
        shared,
        base_line,
        chunk,
        scanned: 0,
        lone_cr: 0,
        out,
    };
    searcher
        .search_slice(&shared.matcher, chunk, &mut sink)
        .map_err(Error::from)?;
    shared.cancel.check()
}

struct Collect<'a> {
    shared: &'a Shared,
    base_line: u64,
    /// 本块的全部字节：用来把搜索器只认 `\n` 的行号补成编辑器口径。
    chunk: &'a [u8],
    /// `chunk` 中已经数过独立 `\r` 的前缀长度（匹配按位置递增到达，只向前扫一遍）。
    scanned: usize,
    /// `chunk[..scanned]` 里独立 `\r` 的个数。
    lone_cr: u64,
    out: &'a mut Vec<LineMatch>,
}

impl Collect<'_> {
    /// `chunk` 开头到 `offset` 之间独立 `\r` 的个数（每个都让编辑器多算一行）。
    fn lone_cr_before(&mut self, offset: usize) -> u64 {
        let offset = offset.min(self.chunk.len());
        if offset > self.scanned {
            let (chunk, from) = (self.chunk, self.scanned);
            self.lone_cr += memchr::memchr_iter(b'\r', &chunk[from..offset])
                .filter(|i| chunk.get(from + i + 1) != Some(&b'\n'))
                .count() as u64;
            self.scanned = offset;
        }
        self.lone_cr
    }
}

impl Sink for Collect<'_> {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _: &Searcher,
        mat: &SinkMatch<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        if self.shared.cancel.check().is_err() {
            return Ok(false);
        }
        let offset = usize::try_from(mat.absolute_byte_offset()).unwrap_or(usize::MAX);
        let first_line =
            self.base_line + mat.line_number().unwrap_or(1) - 1 + self.lone_cr_before(offset);
        let raw = mat.bytes();
        // 非多行搜索的行终止符不参与匹配（匹配器设了 `line_terminator`），先剥掉行尾的 `\n`：
        // 否则 `$` 这类零宽模式会匹配到行终止符之后，报成“下一行的行首”
        // （grep-searcher 的慢路径同样先剥终止符）。多行搜索的 `\n` 可能是模式的一部分，不能剥。
        let bytes = if self.shared.multi_line {
            raw
        } else {
            raw.strip_suffix(b"\n").unwrap_or(raw)
        };
        // (行在 bytes 中的起止, 行号, 区间)。这里的“行”已经按独立 `\r` 切开。
        let mut per_line: Vec<(usize, usize, u64, Vec<Range<u32>>)> = Vec::new();
        let _ = self.shared.matcher.find_iter(bytes, |m| {
            // 零宽命中（`^` / `$` / `\b`）保留为空区间；`find_iter` 遇到空匹配会前进一个字节。
            // 整段末尾、行终止符之后的空匹配是“下一行的行首”，不是一处命中。
            if m.start() == m.end() && m.start() == bytes.len() && bytes.ends_with(b"\n") {
                return true;
            }
            let lf_start = memchr::memrchr(b'\n', &bytes[..m.start()]).map_or(0, |i| i + 1);
            let lf_index = memchr::memchr_iter(b'\n', &bytes[..lf_start]).count() as u64;
            let lf_end = line_end(bytes, lf_start);
            // 独立 `\r` 也是换行（编辑器口径）：把 `\n` 行再按 `\r` 切成段，跨行匹配截到段末。
            let pos = m.start().min(lf_end).max(lf_start);
            let start = memchr::memrchr(b'\r', &bytes[lf_start..pos])
                .map_or(lf_start, |i| lf_start + i + 1);
            let cr_index = memchr::memchr_iter(b'\r', &bytes[lf_start..start]).count() as u64;
            let end = memchr::memchr(b'\r', &bytes[start..lf_end]).map_or(lf_end, |i| start + i);
            let range = (pos - start) as u32..(m.end().min(end).max(pos) - start) as u32;
            match per_line.last_mut() {
                Some((s, _, _, ranges)) if *s == start => ranges.push(range),
                _ => per_line.push((
                    start,
                    end,
                    first_line + lf_index + cr_index,
                    std::iter::once(range).collect(),
                )),
            }
            true
        });
        for (start, end, line, mut ranges) in per_line {
            let granted = self.shared.claim(ranges.len());
            ranges.truncate(granted);
            if ranges.is_empty() {
                return Ok(false);
            }
            self.out.push(preview(line, &bytes[start..end], ranges));
            if granted == 0 || self.shared.cancel.check().is_err() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// 从 `start` 起这一行（不含 `\n` 与前面的 `\r`）的结束位置。独立的 `\r` 不在这里处理。
fn line_end(bytes: &[u8], start: usize) -> usize {
    let end = memchr::memchr(b'\n', &bytes[start..]).map_or(bytes.len(), |i| start + i);
    if end > start && bytes[end - 1] == b'\r' {
        end - 1
    } else {
        end
    }
}

/// 按编辑器口径（`rove-buffer`：`\n`、`\r\n`、独立的 `\r`）统计换行数。
fn line_breaks(bytes: &[u8]) -> u64 {
    let lf = memchr::memchr_iter(b'\n', bytes).count() as u64;
    let lone_cr = memchr::memchr_iter(b'\r', bytes)
        .filter(|i| bytes.get(i + 1) != Some(&b'\n'))
        .count() as u64;
    lf + lone_cr
}

fn preview(line: u64, text: &[u8], ranges: Vec<Range<u32>>) -> LineMatch {
    let text = String::from_utf8_lossy(text);
    let first = ranges
        .first()
        .map_or(0, |r| r.start as usize)
        .min(text.len());
    let mut start = if text.len() > PREVIEW_BYTES {
        floor_boundary(&text, first.saturating_sub(PREVIEW_LEAD))
    } else {
        0
    };
    // 前导空白不显示（VS Code 的预览同样去掉缩进）。
    while start < first && matches!(text.as_bytes()[start], b' ' | b'\t') {
        start += 1;
    }
    let end = floor_boundary(&text, (start + PREVIEW_BYTES).min(text.len()));
    LineMatch {
        line,
        preview: Arc::from(&text[start..end]),
        preview_start: start as u32,
        ranges,
    }
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // 一行一处匹配本来就是只有一个区间的 Vec
mod tests {
    use super::*;

    #[test]
    fn preview_trims_indentation_and_maps_ranges() {
        let m = preview(3, b"    let x = Foo();", vec![12..15]);
        assert_eq!(&*m.preview, "let x = Foo();");
        assert_eq!(m.preview_start, 4);
        assert_eq!(m.preview_ranges().collect::<Vec<_>>(), vec![8..11]);
    }

    #[test]
    fn long_lines_keep_a_window_around_the_first_match() {
        let mut line = "a".repeat(5000);
        line.push_str("NEEDLE");
        line.push_str(&"b".repeat(5000));
        let m = preview(0, line.as_bytes(), vec![5000..5006]);
        assert!(m.preview.len() <= PREVIEW_BYTES);
        assert_eq!(m.preview_start, 4900);
        let r = m.preview_ranges().next().unwrap();
        assert_eq!(&m.preview[r], "NEEDLE");
    }

    #[test]
    fn line_end_strips_crlf() {
        assert_eq!(line_end(b"ab\r\ncd", 0), 2);
        assert_eq!(line_end(b"ab\r\ncd", 4), 6);
    }

    #[test]
    fn line_breaks_count_lone_cr_like_the_editor() {
        // 与 `rove-buffer` 的 `line_ranges_exclude_all_terminator_forms` 同一个样本。
        assert_eq!(line_breaks(b"a\r\nb\nc\rd"), 3);
        assert_eq!(line_breaks(b"abc"), 0);
        assert_eq!(line_breaks(b"a\r"), 1);
    }

    #[test]
    fn empty_range_at_the_preview_start_is_kept() {
        let m = preview(0, b"let x = 1;", vec![0..0]);
        assert_eq!(m.preview_ranges().collect::<Vec<_>>(), vec![0..0]);
    }

    fn shared_for(query: &SearchQuery) -> Shared {
        Shared {
            matcher: query::compile(query, false).expect("合法查询"),
            multi_line: query.is_multiline(),
            ascii_literal: !query.is_regex && query.pattern.is_ascii(),
            cancel: Cancellation::default(),
            budget: None,
            limit_hit: AtomicBool::new(false),
            files_searched: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            settings: Arc::new(crate::settings::default_settings()),
        }
    }

    /// 以下几条都走内存映射分支（#145）：阈值调低到几个字节，免得造 17 MiB 的样本。
    fn search_mmapped(path: &Path, pattern: &str) -> (Result<()>, Vec<LineMatch>) {
        let _guard = MmapThresholdGuard::new(8);
        let mut out = Vec::new();
        let result = search_file(
            path,
            &shared_for(&SearchQuery::new(pattern)),
            &mut searcher(false),
            &mut out,
        );
        (result, out)
    }

    /// 走内存映射的大文件同样做编码识别（#145）。
    #[test]
    fn mmapped_files_still_detect_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Gbk.cs");
        let (gbk, _, _) = encoding_rs::GBK.encode("// 玩家移动的速度\nfloat 速度 = 1;\n");
        std::fs::write(&path, &gbk).unwrap();
        assert!(gbk.len() > 8, "样本要大于调低后的阈值");

        let (result, out) = search_mmapped(&path, "速度");
        result.expect("GBK 文件能搜索");
        assert_eq!(out.len(), 2, "两行都有「速度」");
        assert_eq!(&*out[1].preview, "float 速度 = 1;");
        assert_eq!(out[1].ranges, vec![6..12]);
    }

    /// 内存映射分支的 ASCII 字面量快速路径：在文件里的要照常报，不在的才短路。
    #[test]
    fn mmapped_ascii_literal_fast_path_keeps_real_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Player.cs");
        std::fs::write(&path, "class Player {\n    void Move() {}\n}\n").unwrap();

        let (result, out) = search_mmapped(&path, "Move");
        result.expect("可以搜索");
        assert_eq!(out.len(), 1, "字面量在文件里，不能被短路吞掉");
        assert_eq!(out[0].line, 1);
        assert_eq!(out[0].ranges, vec![9..13]);

        let (result, out) = search_mmapped(&path, "Jump");
        result.expect("可以搜索");
        assert!(out.is_empty(), "字面量不在文件里：短路后没有匹配");
    }

    /// UTF-16 的原始字节里 ASCII 字面量是断开的（夹着 NUL）：既不能被二进制判定跳过，
    /// 也不能走“原始字节里没有就短路”的快速路径，必须先转码。
    #[test]
    fn mmapped_utf16_files_are_transcoded_before_matching() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Utf16.cs");
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "float speed = 1;\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();

        let (result, out) = search_mmapped(&path, "speed");
        result.expect("UTF-16 文件能搜索");
        assert_eq!(out.len(), 1);
        assert_eq!(&*out[0].preview, "float speed = 1;");
        assert_eq!(out[0].ranges, vec![6..11]);
    }

    /// 二进制判定排在字面量扫描之前：几百 MB 的贴图只读开头 8 KiB 就跳过，
    /// 而且不管字面量在不在里面，都计入摘要的 `skipped`。
    #[test]
    fn mmapped_binary_files_are_skipped_before_the_literal_scan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Texture.psd");
        let mut bytes = b"8BPS\x00\x01\x00\x00Move".to_vec();
        bytes.resize(4096, b'x');
        std::fs::write(&path, &bytes).unwrap();

        for pattern in ["Move", "Jump"] {
            let (result, out) = search_mmapped(&path, pattern);
            assert!(
                matches!(result, Err(Error::Unsupported)),
                "{pattern}：二进制文件要按跳过报告"
            );
            assert!(out.is_empty());
        }
    }
}
