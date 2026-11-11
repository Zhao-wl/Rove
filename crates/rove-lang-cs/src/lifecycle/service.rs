//! 索引生命周期服务：一个项目一个实例，一个后台线程，所有状态迁移都在这个线程里发生。
//!
//! 对外：
//! - [`IndexLifecycle::status`] / [`IndexLifecycle::changes`]：状态与订阅（界面用）；
//! - [`IndexLifecycle::snapshot`]：当前可查询的 L2 索引（#38 用）；替换是换一个 `Arc`，
//!   已经拿到旧快照的查询继续读旧 mmap，不会读到半截文件；
//! - [`IndexLifecycle::is_stale`]：被修改过、索引里的位置可能已经不对的文件；
//! - [`IndexLifecycle::rebuild`]、[`IndexLifecycle::files_changed`]、关闭（drop）。
use super::{
    evict::{evict, CachePolicy},
    inputs::{self, is_config_path, is_source_path, ProjectInputs},
    layout::{resolve_root, ProjectCache},
    log::IndexLog,
    meta::{hex, now_ms, BuildRecord, ProjectMeta},
    prefetch::{prefetch, PrefetchMode},
    process::{BuildFailure, BuildRequest, IndexBuilder, Progress},
    state::{Failure, FailureKind, IndexState, IndexStatus, PrefetchState},
    validity::{open_checked, sample_files, Invalid, SampleRule, EXPECTED_INDEXER_VERSION},
};
use crate::index::CsIndex;
use rove_base::{Cancellation, Events, Subscription};
use rove_config::{Key, Scope, Settings};
use std::{
    collections::{BTreeMap, BTreeSet},
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
}

impl LifecycleConfig {
    pub fn new(enabled: bool, cache_root: Result<PathBuf, String>) -> Self {
        Self {
            enabled,
            cache_root,
            prefetch: PrefetchMode::default(),
            sample: SampleRule::default(),
            cache: CachePolicy::default(),
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

/// 一份可查询的 L2 索引。
pub struct IndexSnapshot {
    index: Option<CsIndex>,
    pub generation: u64,
    pub path: PathBuf,
    /// false：这份索引已知与项目对不上（配置变化、抽样不一致），正在重建；结果整体按近似处理。
    pub trusted: bool,
    retired: AtomicBool,
}

impl IndexSnapshot {
    pub fn index(&self) -> &CsIndex {
        self.index.as_ref().expect("index present until drop")
    }
}

impl Drop for IndexSnapshot {
    /// 被新世代替换后，最后一个持有者放手时删除文件（先解除映射，Windows 上映射中的文件删不掉）。
    fn drop(&mut self) {
        drop(self.index.take());
        if self.retired.load(Ordering::Acquire) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

type Listener = Arc<dyn Fn(IndexStatus) + Send + Sync>;

#[derive(Default)]
struct Shared {
    status: Mutex<IndexStatus>,
    changed: Condvar,
    snapshot: Mutex<Option<Arc<IndexSnapshot>>>,
    stale: Mutex<BTreeSet<String>>,
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
        self.changed.notify_all();
        let listeners: Vec<_> = lock(&self.listeners).1.values().cloned().collect();
        for listener in listeners {
            listener(status.clone());
        }
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
            builder,
            shared: shared.clone(),
            cancel: cancel.clone(),
            cache: None,
            log: None,
            meta: ProjectMeta::default(),
            inputs: ProjectInputs::default(),
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
        lock(&self.shared.stale).iter().cloned().collect()
    }
    /// `rove.index.rebuild` / 失败通知的“重试”。
    pub fn rebuild(&self) {
        self.shared.cancel_prefetch();
        let _ = self.tx.send(Command::Rebuild);
    }

    /// 订阅 #15 的文件事件后调用。
    ///
    /// **#35（增量索引）合并前后的切换点就在这里**：目前源文件变化只标记为过期，项目配置变化
    /// 全量重建；#35 合并后，源文件变化改为转发给常驻索引进程（状态进入 `Updating`），
    /// 更新完成后再清除这些文件的过期标记。
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
        if !marked.is_empty() && self.snapshot().is_some() {
            let count = {
                let mut stale = lock(&self.shared.stale);
                stale.extend(marked);
                stale.len()
            };
            self.shared.update(|s| s.info.stale_files = count);
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
        // 关项目：立刻停止预读与构建（索引进程被结束），后台线程自行退出。
        self.cancel.cancel();
        self.shared.cancel_prefetch();
    }
}

struct Worker {
    root: PathBuf,
    config: LifecycleConfig,
    builder: Arc<dyn IndexBuilder>,
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
            if self.cancel.check().is_err() {
                break;
            }
            // 合并排队的命令：任何一个要求重建就重建一次。
            let mut commands = vec![first];
            commands.extend(rx.try_iter());
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
    }

    fn log(&self, text: &str) {
        if let Some(log) = &self.log {
            log.line(text);
        }
    }

    fn set_state(&self, state: IndexState) {
        self.shared.update(|s| s.state = state);
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
        match open_checked(&path, record.as_ref(), &self.inputs) {
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
            snapshot.index(),
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
            let count = {
                let mut stale = lock(&self.shared.stale);
                stale.extend(outcome.mismatched.iter().cloned());
                stale.len()
            };
            self.shared.update(|s| s.info.stale_files = count);
        }
        if let Err(invalid) = outcome.verdict(&self.config.sample) {
            self.log(&format!("cached index invalid: {invalid}"));
            self.publish_untrusted();
            self.build();
        }
    }

    fn publish_untrusted(&self) {
        let mut slot = lock(&self.shared.snapshot);
        if let Some(old) = slot.take() {
            // 同一世代换成“不可信”标记的新快照；旧 Arc 的持有者继续读同一个文件。
            let index = CsIndex::open(&old.path).ok();
            *slot = index.map(|index| {
                Arc::new(IndexSnapshot {
                    index: Some(index),
                    generation: old.generation,
                    path: old.path.clone(),
                    trusted: false,
                    retired: AtomicBool::new(false),
                })
            });
        }
    }

    fn publish(
        &self,
        generation: u64,
        path: PathBuf,
        index: CsIndex,
        trusted: bool,
        record: Option<&BuildRecord>,
    ) {
        let files = index.files().len() as u64;
        let symbols = index.symbols().len() as u64;
        let snapshot = Arc::new(IndexSnapshot {
            index: Some(index),
            generation,
            path,
            trusted,
            retired: AtomicBool::new(false),
        });
        let old = lock(&self.shared.snapshot).replace(snapshot);
        if let Some(old) = old.filter(|o| o.generation != generation) {
            old.retired.store(true, Ordering::Release);
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

    /// 删除当前世代以外的旧文件与遗留的临时文件（被别的窗口映射着的删不掉，下次再试）。
    fn clean_l2(&self, cache: &ProjectCache, keep: Option<u64>) {
        let keep = keep.or_else(|| cache.generations().first().map(|g| g.0));
        for (generation, path) in cache.generations() {
            if Some(generation) != keep {
                let _ = fs::remove_file(path);
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
            let result = result.and_then(|()| self.adopt(generation, &output, started));
            match result {
                Ok(()) => return,
                Err(BuildFailure::Cancelled) => return,
                Err(f) => {
                    self.log(&format!("build attempt {attempt} failed: {f}"));
                    let _ = fs::remove_file(&output);
                    failure = f;
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
    fn adopt(
        &mut self,
        generation: u64,
        output: &Path,
        started: Instant,
    ) -> Result<(), BuildFailure> {
        let index =
            CsIndex::open(output).map_err(|e| BuildFailure::InvalidOutput(e.to_string()))?;
        if index.header().indexer_version != EXPECTED_INDEXER_VERSION {
            return Err(BuildFailure::InvalidOutput(format!(
                "indexer version {:?}",
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
        lock(&self.shared.stale).clear();
        if let Some(cache) = self.cache.clone() {
            self.clean_l2(&cache, Some(generation));
        }
        self.shared.update(|s| {
            s.info.stale_files = 0;
            // 刚写完的文件就在系统缓存里。
            s.info.prefetch = PrefetchState::Skipped;
            s.state = IndexState::Ready;
        });
        Ok(())
    }
}
