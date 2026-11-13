//! 索引生命周期服务：一个项目一个实例，一个后台线程，所有状态迁移都在这个线程里发生。
//!
//! 对外：
//! - [`IndexLifecycle::status`] / [`IndexLifecycle::changes`]：状态与订阅（界面用）；
//! - [`IndexLifecycle::snapshot`]：当前可查询的 L2 索引（#38 用）；替换是换一个 `Arc`，
//!   已经拿到旧快照的查询继续读旧 mmap，不会读到半截文件；
//! - [`IndexLifecycle::is_stale`]：被修改过、索引里的位置可能已经不对的文件；
//! - [`IndexLifecycle::rebuild`]、[`IndexLifecycle::files_changed`]、关闭（drop）。
//!
//! #35 增量更新：源文件变化转发给常驻索引进程（`resident`），状态进入 `Updating`；进程把重绑结果追加到
//! 与基础索引配套的覆盖层日志，这里读出新段、与基础索引合并（`overlay`），发布新的快照并清除过期标记。
//! 覆盖层的文件数或日志体积超过 [`OverlayLimits`] 时触发后台全量重建，期间旧索引 + 覆盖层照常可查询。
use super::{
    evict::{evict, CachePolicy},
    inputs::{self, is_config_path, is_source_path, ProjectInputs},
    layout::{resolve_root, ProjectCache},
    log::IndexLog,
    meta::{hex, now_ms, BuildRecord, ProjectMeta},
    prefetch::{prefetch, PrefetchMode},
    process::{BuildFailure, BuildRequest, IndexBuilder, Progress},
    resident::{
        IncrementalIndexer, Pending, ResidentEvent, ResidentOptions, ResidentSession,
        SessionRequest,
    },
    state::{Failure, FailureKind, IndexState, IndexStatus, PrefetchState},
    validity::{open_checked, sample_files, Invalid, SampleRule, EXPECTED_INDEXER_VERSION},
};
use crate::index::CsIndex;
use crate::overlay::{read_journal_from, KeyCache, L2View, Overlay, Segment};
use rove_base::{Cancellation, Events, Subscription};
use rove_config::{Key, Scope, Settings};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Condvar, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};

pub const ENABLED_KEY: &str = "rove.index.enabled";
pub const CACHE_DIR_KEY: &str = "rove.index.cacheDir";

/// 生命周期的配置。`enabled` 与缓存目录来自设置（#13 的统一设置服务），其余是策略默认值。
#[derive(Clone, Debug)]
pub struct LifecycleConfig {
    pub enabled: bool,
    /// 解析好的缓存根目录；`Err` 是不可用的原因（界面显示“失败：缓存目录不可用”）。
    pub cache_root: Result<PathBuf, String>,
    pub prefetch: PrefetchMode,
    pub sample: SampleRule,
    pub cache: CachePolicy,
    /// #35：覆盖层达到这个规模时后台全量重建。
    pub overlay: OverlayLimits,
    /// #35：常驻索引进程的空闲退出等策略。
    pub resident: ResidentOptions,
}

/// 覆盖层的上限（#35）：超过任一项就“该重建了”，由生命周期在后台全量重建并原子替换。
///
/// 默认 256 个文件或 32 MiB 日志。依据：每次更新都要把合并结构整体重建一次，耗时与覆盖层的出现位置总数
/// 成正比——`cs_overlay/build_1024` 在合成的最坏情况（每个文件约 550 个出现位置、符号互不相同）上约 0.5 ms / 文件，
/// 256 个文件约 130 ms，加上重绑仍在单文件更新 300 ms 的目标之内；再大就该在后台全量重建（大型项目约 90 s，
/// 期间旧索引照常可查询）。日志只追加、同一文件改多次会留下多份，所以另设体积上限。查询本身与段数无关。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayLimits {
    pub max_files: usize,
    pub max_journal_bytes: u64,
}

impl Default for OverlayLimits {
    fn default() -> Self {
        Self {
            max_files: 256,
            max_journal_bytes: 32 << 20,
        }
    }
}

impl OverlayLimits {
    /// 是否该重建了。
    pub fn exceeded(&self, files: usize, journal_bytes: u64) -> bool {
        files >= self.max_files || journal_bytes >= self.max_journal_bytes
    }
}

impl LifecycleConfig {
    pub fn new(enabled: bool, cache_root: Result<PathBuf, String>) -> Self {
        Self {
            enabled,
            cache_root,
            prefetch: PrefetchMode::default(),
            sample: SampleRule::default(),
            cache: CachePolicy::default(),
            overlay: OverlayLimits::default(),
            resident: ResidentOptions::default(),
        }
    }
    /// 通过设置读取接口取 `rove.index.enabled` 与 `rove.index.cacheDir`（不自己读文件）。
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let scope = Scope::default();
        let enabled = Key::<bool>::new(ENABLED_KEY)
            .read(settings, &scope)
            .unwrap_or(true);
        let dir = Key::<String>::new(CACHE_DIR_KEY)
            .read(settings, &scope)
            .unwrap_or_default();
        Self::new(enabled, resolve_root(&dir, |k| std::env::var_os(k)))
    }
    /// 这些设置变化时，界面关闭旧的生命周期并按新配置重新打开。
    pub fn is_lifecycle_key(key: &str) -> bool {
        key == ENABLED_KEY || key == CACHE_DIR_KEY
    }
}

/// 工作区文件变化（由界面从 #15 的文件事件转换而来，路径为项目相对、`/` 分隔）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceChange {
    Created(String),
    Changed(String),
    Removed(String),
    Renamed {
        from: String,
        to: String,
    },
    /// 监听溢出，变化未知。
    Rescan,
}

/// 一个世代的基础索引文件（同一世代的多个快照共享一份映射）。
struct BaseFile {
    index: Option<CsIndex>,
    /// 这个世代的全部文件（索引、覆盖层日志、声明指纹表），被替换后一并删除。
    files: Vec<PathBuf>,
    retired: AtomicBool,
}

impl Drop for BaseFile {
    /// 被新世代替换后，最后一个持有者放手时删除文件（先解除映射，Windows 上映射中的文件删不掉）。
    fn drop(&mut self) {
        drop(self.index.take());
        if self.retired.load(Ordering::Acquire) {
            for path in &self.files {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// 一份可查询的 L2 索引：基础索引 + 覆盖层（#35）。覆盖层更新时发布新的快照，共享同一个基础映射。
pub struct IndexSnapshot {
    base: Arc<BaseFile>,
    overlay: Option<Arc<Overlay>>,
    pub generation: u64,
    pub path: PathBuf,
    /// false：这份索引已知与项目对不上（配置变化、抽样不一致），正在重建；结果整体按近似处理。
    pub trusted: bool,
}

impl IndexSnapshot {
    /// 基础索引（不含覆盖层）。查询请用 [`IndexSnapshot::view`]。
    pub fn index(&self) -> &CsIndex {
        self.base.index.as_ref().expect("index present until drop")
    }
    /// 基础索引 + 覆盖层的查询视图。
    pub fn view(&self) -> L2View<'_> {
        L2View::new(self.index(), self.overlay.as_deref())
    }
    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_deref()
    }
}

type Listener = Arc<dyn Fn(IndexStatus) + Send + Sync>;

/// 过期文件集合：每个文件记下标记时的构建轮次（`epoch`，每次全量构建开始时 +1）。
///
/// 构建期间到达的变化索引器可能已经来不及读（它可能读的是旧内容），所以 `adopt` 只能清掉
/// 构建开始之前标记的文件，构建期间新标记的保留到增量更新写入覆盖层、或下一次全量重建。
#[derive(Default)]
struct StaleFiles {
    epoch: u64,
    files: BTreeMap<String, u64>,
}

impl StaleFiles {
    fn contains(&self, rel_path: &str) -> bool {
        self.files.contains_key(rel_path)
    }
    fn paths(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
    fn len(&self) -> usize {
        self.files.len()
    }
    /// 标记为过期（记在当前轮次下），返回集合大小。
    fn mark<I: IntoIterator<Item = String>>(&mut self, paths: I) -> usize {
        let epoch = self.epoch;
        self.files.extend(paths.into_iter().map(|p| (p, epoch)));
        self.files.len()
    }
    fn unmark(&mut self, rel_path: &str) {
        self.files.remove(rel_path);
    }
    /// 一次全量构建开始：开启新一轮，返回这一轮的编号。
    fn begin_build(&mut self) -> u64 {
        self.epoch += 1;
        self.epoch
    }
    /// 这一轮构建发布完成：丢掉构建开始前标记的（新索引已经包含），留下构建期间新标记的。
    fn adopted(&mut self, epoch: u64) -> usize {
        self.files.retain(|_, marked| *marked >= epoch);
        self.files.len()
    }
}

#[derive(Default)]
struct Shared {
    status: Mutex<IndexStatus>,
    changed: Condvar,
    snapshot: Mutex<Option<Arc<IndexSnapshot>>>,
    stale: Mutex<StaleFiles>,
    listeners: Arc<Mutex<(u64, BTreeMap<u64, Listener>)>>,
    prefetch_cancel: Mutex<Option<Cancellation>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    /// 唯一的状态出口：改完释放锁再通知订阅者。
    fn update(&self, f: impl FnOnce(&mut IndexStatus)) {
        let status = {
            let mut status = lock(&self.status);
            let before = status.clone();
            f(&mut status);
            if *status == before {
                return;
            }
            status.revision += 1;
            status.clone()
        };
        let listeners: Vec<_> = lock(&self.listeners).1.values().cloned().collect();
        for listener in listeners {
            listener(status.clone());
        }
        // 订阅者先收到（只入队），等待者再醒来：等到状态的一方可以确定事件已经发出。
        self.changed.notify_all();
    }
    fn cancel_prefetch(&self) {
        if let Some(c) = lock(&self.prefetch_cancel).take() {
            c.cancel();
        }
    }
}

impl Events<IndexStatus> for Shared {
    fn subscribe(&self, listener: Listener) -> Subscription {
        let mut state = lock(&self.listeners);
        state.0 += 1;
        let id = state.0;
        state.1.insert(id, listener);
        let weak = Arc::downgrade(&self.listeners);
        Subscription::new(move || {
            if let Some(listeners) = weak.upgrade() {
                lock(&listeners).1.remove(&id);
            }
        })
    }
}

enum Command {
    Rebuild,
    /// 项目配置文件变化：重新扫描输入并全量重建。
    ConfigChanged,
    /// 重新打开（监听溢出、原本没有 C# 代码的工作区出现了 .cs 文件）。
    Reopen,
    /// #35：源文件变化（项目相对路径），转发给常驻索引进程。
    Sources(Vec<String>),
    /// #35：常驻索引进程的回复与通知。
    Resident(ResidentEvent),
    /// 关闭项目（常驻进程的事件回调持有发送端，所以不能只靠通道断开）。
    Close,
}

/// 一个项目的索引生命周期。drop 时取消后台工作并结束索引进程，不等待。
pub struct IndexLifecycle {
    shared: Arc<Shared>,
    tx: mpsc::Sender<Command>,
    cancel: Cancellation,
}

impl IndexLifecycle {
    /// 在后台线程打开项目：检查缓存 → 可用就立即发布并预读，否则构建。
    pub fn start(
        project_root: PathBuf,
        config: LifecycleConfig,
        builder: Arc<dyn IndexBuilder>,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        let (tx, rx) = mpsc::channel();
        let cancel = Cancellation::default();
        let worker = Worker {
            root: project_root,
            config,
            incremental: builder.incremental(),
            builder,
            shared: shared.clone(),
            cancel: cancel.clone(),
            cache: None,
            log: None,
            meta: ProjectMeta::default(),
            inputs: ProjectInputs::default(),
            tx: tx.clone(),
            resident: None,
            pending: Pending::default(),
            next_request: 1,
            journal: None,
        };
        thread::Builder::new()
            .name("rove-index-lifecycle".into())
            .spawn(move || worker.run(rx))
            .expect("spawn index lifecycle thread");
        Self { shared, tx, cancel }
    }

    pub fn status(&self) -> IndexStatus {
        lock(&self.shared.status).clone()
    }
    /// 状态变化（在后台线程上回调；回调只入队并迅速返回）。
    pub fn changes(&self) -> &dyn Events<IndexStatus> {
        &*self.shared
    }
    /// 当前可查询的 L2 索引；没有时 C# 导航停留在 L0/L1。
    pub fn snapshot(&self) -> Option<Arc<IndexSnapshot>> {
        lock(&self.shared.snapshot).clone()
    }
    /// 文件是否已过期（修改过但索引还没更新）。#38 据此把它的结果降级为近似。
    pub fn is_stale(&self, rel_path: &str) -> bool {
        lock(&self.shared.stale).contains(rel_path)
    }
    pub fn stale_files(&self) -> Vec<String> {
        lock(&self.shared.stale).paths()
    }
    /// `rove.index.rebuild` / 失败通知的“重试”。
    pub fn rebuild(&self) {
        self.shared.cancel_prefetch();
        let _ = self.tx.send(Command::Rebuild);
    }

    /// 订阅 #15 的文件事件后调用。
    ///
    /// 源文件变化先标记为过期（#38 据此降级为近似），再转发给常驻索引进程（#35，状态进入 `Updating`），
    /// 覆盖层写入后清除过期标记；没有常驻进程（组件缺失、测试替身）时停留在“过期”，等下一次全量重建。
    /// 项目结构文件（`.csproj` / `.asmdef` / `.asmref`）变化不做增量，直接全量重建。重命名按“删除旧路径 +
    /// 新建新路径”处理。
    ///
    /// 首次全量构建期间（还没有快照）的变化同样记下来：构建发布后它们仍然是过期的，再按上面的流程转发。
    pub fn files_changed(&self, changes: &[SourceChange]) {
        let mut reopen = false;
        let mut config = false;
        let mut created_source = false;
        let mut marked = Vec::new();
        for change in changes {
            let paths: Vec<&String> = match change {
                SourceChange::Rescan => {
                    reopen = true;
                    continue;
                }
                SourceChange::Created(p) => {
                    created_source |= is_source_path(p);
                    vec![p]
                }
                SourceChange::Changed(p) | SourceChange::Removed(p) => vec![p],
                SourceChange::Renamed { from, to } => vec![from, to],
            };
            for path in paths {
                if is_config_path(path) {
                    config = true;
                } else if is_source_path(path) {
                    marked.push(path.clone());
                }
            }
        }
        let state = self.status().state;
        if state == IndexState::NoCSharp && (created_source || config) {
            reopen = true;
        }
        // 还没有快照、但正在打开或构建时也要记下来：索引器可能已经读过旧内容，这一批变化
        // 在 `adopt` 之后保持过期，并转发给常驻进程做增量更新（没有常驻进程时停留在“过期”）。
        let indexing = matches!(state, IndexState::Checking | IndexState::Building { .. });
        if !marked.is_empty() && (indexing || self.snapshot().is_some()) {
            let count = lock(&self.shared.stale).mark(marked.iter().cloned());
            self.shared.update(|s| s.info.stale_files = count);
            if !config {
                let _ = self.tx.send(Command::Sources(marked));
            }
        }
        if config && state != IndexState::NoCSharp && state != IndexState::Disabled {
            self.shared.cancel_prefetch();
            let _ = self.tx.send(Command::ConfigChanged);
        } else if reopen && state != IndexState::Disabled {
            let _ = self.tx.send(Command::Reopen);
        }
    }

    /// 等到状态满足条件或超时（测试与基准用；界面用订阅）。
    pub fn wait_until(&self, timeout: Duration, f: impl Fn(&IndexStatus) -> bool) -> IndexStatus {
        let deadline = Instant::now() + timeout;
        let mut status = lock(&self.shared.status);
        while !f(&status) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            status = self
                .shared
                .changed
                .wait_timeout(status, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        status.clone()
    }
}

impl Drop for IndexLifecycle {
    fn drop(&mut self) {
        // 关项目：立刻停止预读与构建（索引进程被结束），常驻进程落盘退出，后台线程自行退出。
        self.cancel.cancel();
        self.shared.cancel_prefetch();
        let _ = self.tx.send(Command::Close);
    }
}

/// 一个世代的覆盖层日志读取状态。
struct JournalState {
    generation: u64,
    segments: Vec<Segment>,
    offset: u64,
    keys: KeyCache,
}

struct Worker {
    root: PathBuf,
    config: LifecycleConfig,
    builder: Arc<dyn IndexBuilder>,
    incremental: Option<Arc<dyn IncrementalIndexer>>,
    /// 给常驻进程的事件回调用（事件进同一个命令队列，由本线程处理）。
    tx: mpsc::Sender<Command>,
    resident: Option<Box<dyn ResidentSession>>,
    pending: Pending,
    next_request: u64,
    /// 当前世代已读出的覆盖层段（只追加；每次更新只读日志的新增部分）。
    journal: Option<JournalState>,
    shared: Arc<Shared>,
    cancel: Cancellation,
    cache: Option<ProjectCache>,
    log: Option<Arc<IndexLog>>,
    meta: ProjectMeta,
    inputs: ProjectInputs,
}

/// 超过这个时间的 `*.tmp` 视为崩溃遗留（同一项目在另一个窗口里正在构建的临时文件不删）。
const TMP_MAX_AGE: Duration = Duration::from_secs(3600);

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Command>) {
        self.open();
        while let Ok(first) = rx.recv() {
            if self.cancel.check().is_err() || matches!(first, Command::Close) {
                break;
            }
            // 合并排队的命令：任何一个要求重建就重建一次。增量相关的命令按顺序逐个处理。
            let mut commands = vec![first];
            commands.extend(rx.try_iter());
            if commands.iter().any(|c| matches!(c, Command::Close)) {
                break;
            }
            let mut sources = Vec::new();
            let mut rebuild = Vec::new();
            for command in commands {
                match command {
                    Command::Sources(paths) => sources.extend(paths),
                    Command::Resident(event) => {
                        if self.on_resident(event) {
                            rebuild.push(Command::Rebuild);
                        }
                    }
                    other => rebuild.push(other),
                }
            }
            if !sources.is_empty() && rebuild.is_empty() {
                self.forward(sources);
            } else if !sources.is_empty() {
                // 随后全量重建会包含它们；重建完成前它们保持“过期”。
                self.log(&format!(
                    "{} changed files wait for the rebuild",
                    sources.len()
                ));
            }
            if rebuild.is_empty() {
                continue;
            }
            let commands = rebuild;
            if commands.iter().any(|c| matches!(c, Command::Reopen)) {
                self.open();
            } else if commands.iter().any(|c| matches!(c, Command::ConfigChanged)) {
                self.log("project configuration changed: full rebuild");
                self.rescan_inputs();
                // 当前索引与新配置对不上：重建期间按近似结果继续提供查询。
                self.publish_untrusted();
                self.build();
            } else if self.cache.is_some() {
                self.log("rebuild requested");
                self.build();
            } else {
                self.open();
            }
        }
        self.shared.cancel_prefetch();
        self.stop_resident("project closed");
    }

    // ───────────── #35 增量更新 ─────────────

    /// 把源文件变化交给常驻进程（必要时先拉起）。没有常驻组件时只保留过期标记。
    fn forward(&mut self, mut paths: Vec<String>) {
        paths.sort();
        paths.dedup();
        let Some(snapshot) = lock(&self.shared.snapshot).clone() else {
            return;
        };
        if !snapshot.trusted {
            return;
        }
        // 构建期间排队的变化：重新按当前这一轮标上，直到覆盖层写入
        {
            let count = lock(&self.shared.stale).mark(paths.iter().cloned());
            self.shared.update(|s| s.info.stale_files = count);
        }
        if self.resident.is_none() && !self.start_resident(&snapshot) {
            return;
        }
        let id = self.next_request;
        self.next_request += 1;
        self.pending.request(id, &paths);
        let ok = self
            .resident
            .as_mut()
            .is_some_and(|r| r.changed(id, &paths));
        if !ok {
            self.log("resident indexer is gone: changes stay stale");
            self.stop_resident("pipe closed");
            return;
        }
        let files = self.pending.len() as u32;
        self.set_state(IndexState::Updating { files });
    }

    fn start_resident(&mut self, snapshot: &IndexSnapshot) -> bool {
        let (Some(incremental), Some(cache), Some(log)) = (
            self.incremental.clone(),
            self.cache.clone(),
            self.log.clone(),
        ) else {
            return false;
        };
        let request = SessionRequest {
            project_root: self.root.clone(),
            base: snapshot.path.clone(),
            journal: cache.overlay_path(snapshot.generation),
            options: self.config.resident.clone(),
        };
        let tx = self.tx.clone();
        let generation = snapshot.generation;
        let events = Box::new(move |event: ResidentEvent| {
            let _ = tx.send(Command::Resident(event));
        });
        match incremental.start(&request, events, log) {
            Ok(session) => {
                self.log(&format!(
                    "resident indexer started for generation {generation:016x}"
                ));
                self.resident = Some(session);
                true
            }
            Err(BuildFailure::Missing) => {
                // 没有组件：以后不再尝试（全量构建也会报告“未安装”）
                self.incremental = None;
                false
            }
            Err(BuildFailure::Incompatible(why)) => {
                self.log(&format!("resident indexer unusable: {why}"));
                self.incremental = None;
                self.shared.update(|s| s.info.component_issue = Some(why));
                false
            }
            Err(e) => {
                self.log(&format!("resident indexer failed to start: {e}"));
                false
            }
        }
    }

    fn stop_resident(&mut self, why: &str) {
        if let Some(mut session) = self.resident.take() {
            self.log(&format!("stopping resident indexer: {why}"));
            session.shutdown();
        }
        self.pending.clear();
    }

    /// 处理常驻进程的事件；返回 true 表示需要全量重建。
    fn on_resident(&mut self, event: ResidentEvent) -> bool {
        match event {
            ResidentEvent::Ready { indexer_version } => {
                self.log(&format!("resident indexer ready ({indexer_version})"));
                false
            }
            ResidentEvent::Applied {
                id,
                files,
                locked,
                rebuild_required,
                queued,
                elapsed_ms,
            } => {
                if rebuild_required {
                    self.log("project structure changed: full rebuild");
                    return true;
                }
                self.log(&format!(
                    "incremental update #{id}: {} files in {elapsed_ms:.0} ms, {} locked, {queued} queued",
                    files.len(),
                    locked.len()
                ));
                let done = self.pending.applied(id, &files);
                self.reload_overlay(&done)
            }
            ResidentEvent::Overlay { files, queued } => {
                self.log(&format!(
                    "background update: {} files, {queued} queued",
                    files.len()
                ));
                let done = self.pending.completed(&files);
                self.reload_overlay(&done)
            }
            ResidentEvent::Incompatible(why) => {
                self.log(&format!("resident indexer incompatible: {why}"));
                self.stop_resident("incompatible protocol");
                self.incremental = None;
                self.shared.update(|s| {
                    s.info.component_issue = Some(why);
                    if matches!(s.state, IndexState::Updating { .. }) {
                        s.state = IndexState::Ready;
                    }
                });
                false
            }
            ResidentEvent::Failed { id, message } => {
                self.log(&format!("resident request {id:?} failed: {message}"));
                if id == Some(0) {
                    self.stop_resident("initialize failed");
                }
                false
            }
            ResidentEvent::Exited { idle } => {
                if self.resident.is_some() {
                    self.log(if idle {
                        "resident indexer exited after being idle"
                    } else {
                        "resident indexer exited"
                    });
                    self.resident = None;
                }
                // 在途请求作废：这些文件保持过期，下一次变化会重新拉起进程
                self.pending.clear();
                if matches!(self.status().state, IndexState::Updating { .. }) {
                    self.set_state(IndexState::Ready);
                }
                false
            }
        }
    }

    fn status(&self) -> super::state::IndexStatus {
        lock(&self.shared.status).clone()
    }

    /// 重新读覆盖层日志并发布新快照；清除 `done` 的过期标记。返回 true 表示覆盖层超过上限、该重建了。
    fn reload_overlay(&mut self, done: &[String]) -> bool {
        let Some(snapshot) = lock(&self.shared.snapshot).clone() else {
            return false;
        };
        let Some(cache) = self.cache.clone() else {
            return false;
        };
        let journal = cache.overlay_path(snapshot.generation);
        let Some(overlay) = self.read_overlay(snapshot.generation, snapshot.index(), &journal)
        else {
            return false;
        };
        let files = overlay.file_count();
        let bytes = fs::metadata(&journal).map_or(0, |m| m.len());
        self.publish_overlay(&snapshot, Some(Arc::new(overlay)));
        if !done.is_empty() {
            let count = {
                let mut stale = lock(&self.shared.stale);
                for f in done {
                    stale.unmark(f);
                }
                stale.len()
            };
            self.shared.update(|s| s.info.stale_files = count);
        }
        if self.pending.is_empty() {
            self.set_state(IndexState::Ready);
        } else {
            let files = self.pending.len() as u32;
            self.set_state(IndexState::Updating { files });
        }
        if self.config.overlay.exceeded(files, bytes) {
            self.log(&format!(
                "overlay reached {files} files / {bytes} bytes: background full rebuild"
            ));
            return true;
        }
        false
    }

    /// 读出日志的新增段，与已读的段一起合并（键查找缓存同一世代复用）。没有任何段时返回 `None`。
    fn read_overlay(
        &mut self,
        generation: u64,
        index: &CsIndex,
        journal: &Path,
    ) -> Option<Overlay> {
        if self
            .journal
            .as_ref()
            .is_none_or(|j| j.generation != generation)
        {
            self.journal = Some(JournalState {
                generation,
                segments: Vec::new(),
                offset: 0,
                keys: KeyCache::default(),
            });
        }
        let state = self.journal.as_mut().expect("journal state");
        let last = state.segments.last().map_or(0, |s| s.sequence);
        match read_journal_from(journal, &index.identity(), state.offset, last) {
            Ok((segments, offset)) => {
                state.segments.extend(segments);
                state.offset = offset;
            }
            Err(e) => {
                let message = format!("cannot read overlay journal: {e}");
                self.log(&message);
                return None;
            }
        }
        let state = self.journal.as_mut().expect("journal state");
        (!state.segments.is_empty())
            .then(|| Overlay::build_cached(index, &state.segments, &mut state.keys))
    }

    /// 同一世代换上新的覆盖层（共享基础映射）。
    fn publish_overlay(&self, current: &Arc<IndexSnapshot>, overlay: Option<Arc<Overlay>>) {
        let next = Arc::new(IndexSnapshot {
            base: current.base.clone(),
            overlay,
            generation: current.generation,
            path: current.path.clone(),
            trusted: current.trusted,
        });
        let mut slot = lock(&self.shared.snapshot);
        if slot
            .as_ref()
            .is_some_and(|s| s.generation == current.generation)
        {
            *slot = Some(next);
        }
    }

    /// 打开世代时读出与它配套的覆盖层日志（上次会话落盘的增量）。
    fn load_overlay(&mut self, generation: u64, index: &CsIndex) -> Option<Arc<Overlay>> {
        let journal = self.cache.as_ref()?.overlay_path(generation);
        self.journal = None;
        let overlay = self.read_overlay(generation, index, &journal)?;
        self.log(&format!(
            "overlay journal: {} segments, {} files",
            overlay.segments,
            overlay.file_count()
        ));
        Some(Arc::new(overlay))
    }

    fn log(&self, text: &str) {
        if let Some(log) = &self.log {
            log.line(text);
        }
    }

    fn set_state(&self, state: IndexState) {
        self.shared.update(|s| s.state = state);
    }

    /// 缓存索引头部应有的索引进程版本：已安装组件的版本，未知时用编译期常量。
    fn expected_version(&self) -> [u16; 4] {
        self.builder
            .component_version()
            .unwrap_or(EXPECTED_INDEXER_VERSION)
    }

    fn fail(&self, kind: FailureKind, detail: String) {
        self.log(&format!("failed ({}): {detail}", kind.id()));
        self.set_state(IndexState::Failed(Failure { kind, detail }));
    }

    fn rescan_inputs(&mut self) -> bool {
        match inputs::scan(&self.root, &self.cancel) {
            Ok(inputs) => {
                self.inputs = inputs;
                let inferred = self.inputs.inferred();
                self.shared.update(|s| s.info.inferred_project = inferred);
                true
            }
            Err(e) => {
                if self.cancel.check().is_ok() {
                    self.log(&format!("cannot scan project inputs: {e}"));
                }
                false
            }
        }
    }

    fn open(&mut self) {
        if !self.config.enabled {
            self.set_state(IndexState::Disabled);
            return;
        }
        self.set_state(IndexState::Checking);
        if !self.rescan_inputs() {
            return;
        }
        if !self.inputs.has_csharp() {
            self.set_state(IndexState::NoCSharp);
            return;
        }
        let root = match self.config.cache_root.clone() {
            Ok(root) => root,
            Err(e) => return self.fail(FailureKind::CacheDir, e),
        };
        let cache = ProjectCache::new(&root, &self.root);
        if let Err(e) = cache.ensure() {
            return self.fail(
                FailureKind::CacheDir,
                format!("{}: {e}", cache.dir().display()),
            );
        }
        let log = Arc::new(IndexLog::new(cache.log_path()));
        self.log = Some(log);
        self.log(&format!("open project {}", self.root.display()));
        let mut meta = ProjectMeta::read(&cache.meta_path()).unwrap_or_default();
        meta.project = self.root.display().to_string();
        meta.last_opened_ms = now_ms();
        // 写 project.json 同时验证缓存目录可写（目录存在但没有写权限时在这里失败）。
        if let Err(e) = meta.write(&cache.meta_path()) {
            return self.fail(
                FailureKind::CacheDir,
                format!("{}: {e}", cache.dir().display()),
            );
        }
        self.meta = meta;
        for gone in evict(&root, cache.key(), self.config.cache) {
            self.log(&format!(
                "evicted {} ({} bytes): {:?}",
                gone.dir.display(),
                gone.bytes,
                gone.result
            ));
        }
        self.clean_l2(&cache, None);
        let log_path = cache.log_path();
        let dir = cache.dir().to_owned();
        self.shared.update(|s| {
            s.info.cache_dir = Some(dir);
            s.info.log = Some(log_path);
        });
        self.cache = Some(cache);
        self.open_cached();
    }

    /// 检查缓存中的最新世代：通过检查 1–3 就立即发布，再在后台预读与抽样。
    fn open_cached(&mut self) {
        let cache = self.cache.clone().expect("cache");
        let Some((generation, path)) = cache.generations().into_iter().next() else {
            self.log("no cached index");
            return self.build();
        };
        let record = self
            .meta
            .build
            .clone()
            .filter(|b| b.generation == generation);
        match open_checked(
            &path,
            record.as_ref(),
            &self.inputs,
            self.expected_version(),
        ) {
            Ok(index) => {
                self.log(&format!("cached index {generation:016x} is valid"));
                self.publish(generation, path.clone(), index, true, record.as_ref());
                self.set_state(IndexState::Ready);
                self.prefetch(&path);
                self.verify_samples(generation);
            }
            Err(invalid) => {
                self.log(&format!(
                    "cached index {generation:016x} invalid: {invalid}"
                ));
                // 版本与配置不对时文件本身仍可读：重建期间按近似结果继续提供查询。
                if matches!(
                    invalid,
                    Invalid::IndexerVersion { .. } | Invalid::ProjectConfig
                ) {
                    if let Ok(index) = CsIndex::open(&path) {
                        self.publish(generation, path, index, false, record.as_ref());
                    }
                }
                self.build();
            }
        }
    }

    fn verify_samples(&mut self, generation: u64) {
        let Some(snapshot) = lock(&self.shared.snapshot).clone() else {
            return;
        };
        let started = Instant::now();
        let outcome = sample_files(
            snapshot.view(),
            &self.root,
            &self.config.sample,
            now_ms() ^ generation,
            &self.cancel,
        );
        if self.cancel.check().is_err() {
            return;
        }
        self.log(&format!(
            "sampled {} files in {} ms: {} match",
            outcome.sampled,
            started.elapsed().as_millis(),
            outcome.matched
        ));
        if !outcome.mismatched.is_empty() {
            let count = lock(&self.shared.stale).mark(outcome.mismatched.iter().cloned());
            self.shared.update(|s| s.info.stale_files = count);
        }
        if let Err(invalid) = outcome.verdict(&self.config.sample) {
            self.log(&format!("cached index invalid: {invalid}"));
            self.publish_untrusted();
            self.build();
        } else if !outcome.mismatched.is_empty() {
            // 项目关闭期间改过的文件：交给常驻进程增量更新（#35）
            let _ = self.tx.send(Command::Sources(outcome.mismatched));
        }
    }

    fn publish_untrusted(&mut self) {
        // 配置对不上：增量更新停下（常驻进程的项目模型也过时了），等全量重建。
        self.stop_resident("index no longer trusted");
        let mut slot = lock(&self.shared.snapshot);
        if let Some(old) = slot.take() {
            // 同一世代换成“不可信”标记的新快照；旧 Arc 的持有者继续读同一个映射。
            *slot = Some(Arc::new(IndexSnapshot {
                base: old.base.clone(),
                overlay: old.overlay.clone(),
                generation: old.generation,
                path: old.path.clone(),
                trusted: false,
            }));
        }
    }

    fn publish(
        &mut self,
        generation: u64,
        path: PathBuf,
        index: CsIndex,
        trusted: bool,
        record: Option<&BuildRecord>,
    ) {
        let files = index.files().len() as u64;
        let symbols = index.symbols().len() as u64;
        let overlay = self.load_overlay(generation, &index);
        let generation_files = self
            .cache
            .as_ref()
            .map(|c| c.generation_files(generation).to_vec())
            .unwrap_or_else(|| vec![path.clone()]);
        let snapshot = Arc::new(IndexSnapshot {
            base: Arc::new(BaseFile {
                index: Some(index),
                files: generation_files,
                retired: AtomicBool::new(false),
            }),
            overlay,
            generation,
            path,
            trusted,
        });
        let old = lock(&self.shared.snapshot).replace(snapshot);
        if let Some(old) = old.filter(|o| o.generation != generation) {
            old.base.retired.store(true, Ordering::Release);
            // 先放手旧快照（没有其他持有者时文件随即删除），再发布状态。
            drop(old);
        }
        let built_at = record.map(|r| r.built_at_ms);
        let bytes = self.cache.as_ref().map_or(0, ProjectCache::size);
        self.shared.update(|s| {
            s.info.generation = generation;
            s.info.files = files;
            s.info.symbols = symbols;
            s.info.built_at_ms = built_at;
            s.info.cache_bytes = bytes;
        });
    }

    fn prefetch(&self, path: &Path) {
        let cancel = self.cancel.child();
        *lock(&self.shared.prefetch_cancel) = Some(cancel.clone());
        self.shared
            .update(|s| s.info.prefetch = PrefetchState::Running);
        let state = match prefetch(path, self.config.prefetch, &cancel) {
            Ok(stats) if stats.cancelled => PrefetchState::Cancelled,
            Ok(stats) => {
                self.log(&format!(
                    "prefetched {} bytes in {} ms ({:?})",
                    stats.bytes,
                    stats.elapsed.as_millis(),
                    self.config.prefetch
                ));
                PrefetchState::Done {
                    bytes: stats.bytes,
                    millis: stats.elapsed.as_millis() as u64,
                }
            }
            Err(e) => {
                self.log(&format!("prefetch failed: {e}"));
                PrefetchState::Cancelled
            }
        };
        lock(&self.shared.prefetch_cancel).take();
        self.shared.update(|s| s.info.prefetch = state);
    }

    /// 删除当前世代以外的旧文件（含 #35 的覆盖层日志与声明指纹表）与遗留的临时文件
    /// （被别的窗口映射着的删不掉，下次再试）。
    fn clean_l2(&self, cache: &ProjectCache, keep: Option<u64>) {
        let keep = keep.or_else(|| cache.generations().first().map(|g| g.0));
        for (generation, path) in cache.generations() {
            if Some(generation) != keep {
                let _ = fs::remove_file(path);
            }
        }
        for entry in fs::read_dir(cache.l2_dir()).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let sidecar = name.ends_with(".rvcsovl") || name.ends_with(".rvcsidx.decls");
            let generation = name.get(..16).and_then(|g| u64::from_str_radix(g, 16).ok());
            if sidecar && generation.is_some() && generation != keep {
                let _ = fs::remove_file(entry.path());
            }
        }
        for entry in fs::read_dir(cache.l2_dir()).into_iter().flatten().flatten() {
            let path = entry.path();
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().unwrap_or_default() > TMP_MAX_AGE);
            if path.extension().is_some_and(|e| e == "tmp") && old {
                let _ = fs::remove_file(path);
            }
        }
    }

    /// 全量构建：失败重试一次，仍失败就降级（没有组件 → 未安装；其他 → 失败）。
    fn build(&mut self) {
        self.shared.cancel_prefetch();
        // 常驻进程属于旧世代（它的覆盖层日志配套旧的基础索引）；新世代就绪后按需重新拉起。
        // 重建期间旧索引 + 已写入的覆盖层照常可查询，新的变化排队，重建后转发。
        self.stop_resident("full rebuild");
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let log = self.log.clone().expect("log");
        let rebuilding = lock(&self.shared.snapshot).is_some();
        let mut failure = BuildFailure::Missing;
        for attempt in 1..=2 {
            if self.cancel.check().is_err() {
                return;
            }
            let generation = cache.next_generation();
            let output = cache.l2_path(generation);
            let started = Instant::now();
            // 这一轮开始之前标记的过期由新索引覆盖；构建期间到达的变化记在这一轮下，adopt 保留。
            let epoch = lock(&self.shared.stale).begin_build();
            self.set_state(IndexState::Building {
                progress: Progress::default(),
                eta: None,
                rebuilding,
            });
            self.log(&format!(
                "build attempt {attempt}: generation {generation:016x}"
            ));
            let request = BuildRequest {
                project_root: self.root.clone(),
                output: output.clone(),
            };
            let shared = self.shared.clone();
            let result = self.builder.build(
                &request,
                &mut |progress| {
                    let eta = (progress.percent >= 5 && progress.percent < 100).then(|| {
                        started.elapsed().mul_f64(f64::from(100 - progress.percent))
                            / u32::from(progress.percent)
                    });
                    shared.update(|s| {
                        s.state = IndexState::Building {
                            progress,
                            eta,
                            rebuilding,
                        }
                    });
                },
                &log,
                &self.cancel,
            );
            let result = result.and_then(|()| self.adopt(generation, &output, started, epoch));
            match result {
                Ok(()) => return,
                Err(BuildFailure::Cancelled) => return,
                Err(f) => {
                    self.log(&format!("build attempt {attempt} failed: {f}"));
                    let _ = fs::remove_file(&output);
                    failure = f;
                    if matches!(failure, BuildFailure::Incompatible(_)) {
                        break; // 重试也一样：不兼容的组件不会变兼容
                    }
                }
            }
        }
        if failure == BuildFailure::Missing {
            self.log("semantic component not installed: falling back to L1/L0");
            let trusted = lock(&self.shared.snapshot)
                .as_ref()
                .is_some_and(|s| s.trusted);
            if trusted {
                // 用户要求重建但组件不在：已有的有效索引继续用。
                self.set_state(IndexState::Ready);
            } else {
                lock(&self.shared.snapshot).take();
                self.set_state(IndexState::NotInstalled);
            }
            return;
        }
        if let BuildFailure::Incompatible(why) = &failure {
            self.log("semantic component incompatible: falling back to L1/L0");
            let trusted = lock(&self.shared.snapshot)
                .as_ref()
                .is_some_and(|s| s.trusted);
            if trusted {
                let why = why.clone();
                self.shared.update(|s| {
                    s.info.component_issue = Some(why);
                    s.state = IndexState::Ready;
                });
            } else {
                lock(&self.shared.snapshot).take();
                self.set_state(IndexState::Incompatible(why.clone()));
            }
            return;
        }
        // 降级到 L1/L0：已知对不上的旧索引不再提供；仍然有效的索引保留。
        let mut slot = lock(&self.shared.snapshot);
        if slot.as_ref().is_some_and(|s| !s.trusted) {
            slot.take();
        }
        drop(slot);
        let failure = Failure::from_build(&failure);
        self.fail(failure.kind, failure.detail);
    }

    /// 打开新世代、写构建记录、原子替换快照、清理旧世代。
    ///
    /// `epoch` 是这一轮构建的编号：构建期间到达的变化（索引器可能读的是旧内容）保持过期，
    /// 随后由常驻进程增量更新，或等下一次全量重建。
    fn adopt(
        &mut self,
        generation: u64,
        output: &Path,
        started: Instant,
        epoch: u64,
    ) -> Result<(), BuildFailure> {
        let index =
            CsIndex::open(output).map_err(|e| BuildFailure::InvalidOutput(e.to_string()))?;
        let expected = self.expected_version();
        if index.header().indexer_version != expected {
            return Err(BuildFailure::InvalidOutput(format!(
                "indexer version {:?}, expected {expected:?}",
                index.header().indexer_version
            )));
        }
        let record = BuildRecord {
            generation,
            config_hash: hex(&index.header().project_config_hash),
            fingerprint: self.inputs.fingerprint,
            built_at_ms: now_ms(),
            duration_ms: started.elapsed().as_millis() as u64,
            files: index.files().len() as u64,
            symbols: index.symbols().len() as u64,
        };
        self.meta.build = Some(record.clone());
        if let Some(cache) = &self.cache {
            if let Err(e) = self.meta.write(&cache.meta_path()) {
                self.log(&format!("cannot write project.json: {e}"));
            }
        }
        self.log(&format!(
            "adopted generation {generation:016x}: {} files, {} symbols, {} ms",
            record.files, record.symbols, record.duration_ms
        ));
        self.publish(generation, output.to_owned(), index, true, Some(&record));
        let stale = lock(&self.shared.stale).adopted(epoch);
        if stale > 0 {
            self.log(&format!(
                "{stale} files changed during the build: still stale"
            ));
        }
        if let Some(cache) = self.cache.clone() {
            self.clean_l2(&cache, Some(generation));
        }
        self.shared.update(|s| {
            s.info.component_issue = None;
            s.info.stale_files = stale;
            // 刚写完的文件就在系统缓存里。
            s.info.prefetch = PrefetchState::Skipped;
            s.state = IndexState::Ready;
        });
        Ok(())
    }
}
