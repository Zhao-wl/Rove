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
//! - **取消**：取消令牌传到每个工作线程的内层循环——每个目录项、每个文件、每 1 MiB 的块、每一处匹配都检查，
//!   所以取消后几毫秒内所有线程退出；[`search`] 返回时工作线程已全部结束。
//! - **编码**：先按字节快速判定（BOM、合法 UTF-8），只有疑似非 UTF-8 的文件才走 #7 的完整编码识别
//!   （`files.autoGuessEncoding`，GBK 中文能搜到）。含 NUL 的文件按二进制跳过（与 ripgrep / VS Code 一致）。
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

/// 一次 `search_slice` 处理的最大字节数：内层循环在块之间检查取消。
const CHUNK: usize = 1 << 20;
/// 超过这个大小的文件用内存映射读取（不复制、不做 UTF-8 校验，只看 BOM 与二进制）。
const MMAP_THRESHOLD: u64 = 16 << 20;
/// 二进制判定只看文件开头这么多字节里有没有 NUL（与 ripgrep 的快速判定一致）。
const BINARY_PROBE: usize = 8 << 10;
/// 行预览最多保留的字节数；更长的行只保留第一处匹配附近的一段。
const PREVIEW_BYTES: usize = 1000;
/// 预览在第一处匹配之前保留的字节数。
const PREVIEW_LEAD: usize = 100;
/// 第一批之后，结果每隔这么久交给回调一次。
const BATCH_INTERVAL: Duration = Duration::from_millis(30);

/// 一行里的匹配。列是该行解码后文本的 UTF-8 字节列（`Utf8Column`，不是磁盘字节）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineMatch {
    /// 从 0 开始的行号。
    pub line: u64,
    /// 行预览（不含行终止符；超长行只是一段，前导空白已去掉）。
    pub preview: Arc<str>,
    /// `preview` 第一个字节在该行中的列。
    pub preview_start: u32,
    /// 匹配区间（行内列，半开）。跨行的匹配截到本行末尾。
    pub ranges: Vec<Range<u32>>,
}

impl LineMatch {
    /// 匹配区间换算成 `preview` 内的字节区间（落在预览之外的部分被截掉）。
    pub fn preview_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        let start = self.preview_start as usize;
        let len = self.preview.len();
        self.ranges.iter().filter_map(move |r| {
            let s = (r.start as usize).checked_sub(start).unwrap_or(0).min(len);
            let e = (r.end as usize).saturating_sub(start).min(len);
            (r.end as usize > start && s <= e).then_some(s..e)
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
    if len > MMAP_THRESHOLD {
        // SAFETY: 只读映射；文件在搜索期间被别人截断属于外部竞争，最坏结果是这次读到的内容不完整，
        // 与 ripgrep 的取舍相同。
        let map = unsafe { memmap2::Mmap::map(&file)? };
        let text = decode(&map, shared, false)?;
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
    let text = decode(&bytes, shared, true)?;
    search_text(&text, shared, searcher, out)
}

/// 字节 → 用来搜索的 UTF-8 文本。
///
/// 1. UTF-8 BOM：去掉 BOM（行首列号与编辑器一致）；UTF-16 BOM：转码；
/// 2. 开头 8 KiB 有 NUL：二进制，跳过；
/// 3. 合法 UTF-8（绝大多数文件）：原样搜索，不复制；
/// 4. 否则交给 #7 的编码识别（`files.autoGuessEncoding`、候选编码 GBK…）解码；识别失败按原始字节搜索
///    （与 ripgrep 一样，ASCII 部分仍能搜到）。
fn decode<'a>(bytes: &'a [u8], shared: &Shared, validate: bool) -> Result<Cow<'a, [u8]>> {
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
    if !validate || std::str::from_utf8(bytes).is_ok() {
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

/// 分块搜索：每块约 1 MiB、在行边界切开，块之间检查取消。多行正则需要整段文本，不分块。
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
        base_line += memchr::memchr_iter(b'\n', chunk).count() as u64;
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
    out: &'a mut Vec<LineMatch>,
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
        let first_line = self.base_line + mat.line_number().unwrap_or(1) - 1;
        let bytes = mat.bytes();
        // (行在 bytes 中的起点, 行号, 区间)
        let mut per_line: Vec<(usize, u64, Vec<Range<u32>>)> = Vec::new();
        let _ = self.shared.matcher.find_iter(bytes, |m| {
            if m.start() == m.end() {
                return true;
            }
            let line_start = memchr::memrchr(b'\n', &bytes[..m.start()]).map_or(0, |i| i + 1);
            let line_index = memchr::memchr_iter(b'\n', &bytes[..line_start]).count() as u64;
            let line_end = line_end(bytes, line_start);
            let start = (m.start() - line_start) as u32;
            let end = (m.end().min(line_end).max(m.start()) - line_start) as u32;
            match per_line.last_mut() {
                Some((s, _, ranges)) if *s == line_start => ranges.push(start..end),
                _ => per_line.push((line_start, first_line + line_index, vec![start..end])),
            }
            true
        });
        for (line_start, line, mut ranges) in per_line {
            let granted = self.shared.claim(ranges.len());
            ranges.truncate(granted);
            if ranges.is_empty() {
                return Ok(false);
            }
            let text = &bytes[line_start..line_end(bytes, line_start)];
            self.out.push(preview(line, text, ranges));
            if granted == 0 || self.shared.cancel.check().is_err() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// 从 `start` 起这一行（不含 `\n` 与前面的 `\r`）的结束位置。
fn line_end(bytes: &[u8], start: usize) -> usize {
    let end = memchr::memchr(b'\n', &bytes[start..]).map_or(bytes.len(), |i| start + i);
    if end > start && bytes[end - 1] == b'\r' {
        end - 1
    } else {
        end
    }
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
}
