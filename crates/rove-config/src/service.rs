//! 与 VS Code 共用 `settings.json` 的设置服务（FEATURES §13 的 7 条规则）。
//!
//! - 读：内存快照，`read` 不碰磁盘，O(log keys)，可在 UI 线程调用；
//! - 写：只在用户显式修改时，由调用方在后台执行返回的 `Task`：区间编辑目标键 → 备份 → 原子写入；
//! - 热加载：后台线程轮询 `settings.json` / `keybindings.json` 的元数据，变化稳定（去抖）后读内容，
//!   与最近一次已知内容（包括自己写入的内容）逐字节比较，相同就不重载，从而不会自激；
//! - 语法错误：沿用上一份有效快照，并通过 [`SettingsService::problems`] 报出带行列的位置。
//!
//! 轮询而不是系统文件通知：两个小文件的 `stat` 开销可以忽略，同时天然覆盖编辑器
//! “写临时文件再重命名”、网络盘和目录不存在等情况，也不需要额外依赖。

use crate::{
    catalog::{self, LanguageDefaults},
    edit::{self, EditError},
    events::EventHub,
    file::{self, RetryPolicy},
    jsonc::{Document, Node, NodeKind, SyntaxErrorKind},
    model::{self, Layer, UserLayer},
    paths, Key, Scope, SettingRegistry, SettingValue, Settings, SettingsChange,
};
use rove_base::{Cancellation, Error, Events, Result, Task, Value, Version};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, PoisonError, RwLock, Weak,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

/// 默认保留的备份份数。
pub const DEFAULT_BACKUP_LIMIT: usize = 20;

/// 热加载参数。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WatchOptions {
    /// 检查文件元数据的间隔。
    pub poll_interval: Duration,
    /// 文件最后一次变化后至少静止这么久才读取（合并编辑器的多次写入）。
    pub debounce: Duration,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(250),
            debounce: Duration::from_millis(100),
        }
    }
}

/// 设置服务的装配参数。
#[derive(Clone, Debug)]
pub struct SettingsOptions {
    /// 含 `settings.json` 与 `keybindings.json` 的 VS Code 用户目录。
    pub user_dir: PathBuf,
    /// 写入前的备份目录。
    pub backup_dir: PathBuf,
    /// 保留的备份份数（至少 1）。
    pub backup_limit: usize,
    /// `None` 表示不监听文件（只在 [`SettingsService::reload`] 时重读）。
    pub watch: Option<WatchOptions>,
    /// 文件被占用时的重试策略。
    pub retry: RetryPolicy,
}

impl SettingsOptions {
    pub fn new(user_dir: impl Into<PathBuf>, backup_dir: impl Into<PathBuf>) -> Self {
        Self {
            user_dir: user_dir.into(),
            backup_dir: backup_dir.into(),
            backup_limit: DEFAULT_BACKUP_LIMIT,
            watch: Some(WatchOptions::default()),
            retry: RetryPolicy::default(),
        }
    }

    /// 按 FEATURES §13 规则 1 从启动参数与环境变量确定位置。
    /// `user_dir_arg` 来自 [`paths::split_user_dir_arg`]。缺少 `%APPDATA%` / `%LOCALAPPDATA%` 时返回 `NotFound`。
    pub fn from_environment(user_dir_arg: Option<PathBuf>) -> Result<Self> {
        let env = |name: &str| std::env::var_os(name);
        let user_dir = paths::resolve_user_dir(user_dir_arg, env).ok_or(Error::NotFound)?;
        let backup_dir = paths::resolve_backup_dir(env).ok_or(Error::NotFound)?;
        Ok(Self::new(user_dir, backup_dir))
    }
}

/// 设置文件的问题类别；UI 按类别选择登记过的字符串（#16 的状态栏与问题面板）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    /// JSONC 语法错误：整份文件未应用，沿用上一份有效设置。
    Syntax(SyntaxErrorKind),
    /// 根节点不是对象：整份文件未应用。
    RootNotObject,
    /// 文件不是合法 UTF-8：整份文件未应用。
    NotUtf8,
    /// 读取失败：沿用上一份有效设置。
    Io(std::io::ErrorKind),
    /// 值的类型与设置项不符：该值被跳过，回落到下一层（最终是默认值）。只是警告。
    TypeMismatch { key: Arc<str> },
}

/// 一条可定位的问题；`line`/`column` 从 1 开始。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsProblem {
    pub kind: ProblemKind,
    pub path: Arc<Path>,
    pub line: u32,
    pub column: u32,
}

impl SettingsProblem {
    /// 错误表示整份文件没有应用；否则是只影响单个值的警告。
    pub fn is_error(&self) -> bool {
        !matches!(self.kind, ProblemKind::TypeMismatch { .. })
    }
}

/// `keybindings.json` 内容变化（#14 消费）。`text` 为 `None` 表示文件不存在；非 UTF-8 字节按替换字符解码。
#[derive(Clone, Debug)]
pub struct KeybindingsChange {
    pub text: Option<Arc<str>>,
}

/// 与 VS Code 共用 `settings.json` 的设置服务，实现 #1 的 [`Settings`] 契约。
///
/// drop 时后台监听线程在下一个轮询周期内自行退出，不阻塞调用线程。
pub struct SettingsService {
    inner: Arc<Inner>,
    stop: Option<Arc<AtomicBool>>,
}

struct Inner {
    settings_path: Arc<Path>,
    keybindings_path: PathBuf,
    options: SettingsOptions,
    registry: SettingRegistry,
    language_defaults: &'static LanguageDefaults,
    snapshot: RwLock<Arc<UserLayer>>,
    /// 最近一次已知的磁盘内容：读到的或自己写入的。热加载据此判断是否需要重载。
    disk: Mutex<Disk>,
    /// 串行化写入与重载。
    io_lock: Mutex<()>,
    problems: Mutex<Problems>,
    revision: AtomicU64,
    changes: EventHub<SettingsChange>,
    problem_events: EventHub<Arc<[SettingsProblem]>>,
    keybindings: EventHub<KeybindingsChange>,
}

#[derive(Default)]
struct Disk {
    settings: Option<Vec<u8>>,
    keybindings: Option<Vec<u8>>,
}

#[derive(Default)]
struct Problems {
    file: Option<SettingsProblem>,
    warnings: Vec<SettingsProblem>,
}

impl Problems {
    fn list(&self) -> Arc<[SettingsProblem]> {
        self.file.iter().chain(&self.warnings).cloned().collect()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

type Stat = Option<(u64, Option<SystemTime>)>;

fn stat(path: &Path) -> Stat {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

impl SettingsService {
    /// 读取 `settings.json` 并（按 `options.watch`）开始监听。文件不存在时按空设置处理，
    /// **不会**在启动时创建或写入文件；第一次显式写入时才在同一位置创建。
    /// 第一次读取就有语法错误时，使用默认值并报告问题。
    pub fn open(options: SettingsOptions, registry: SettingRegistry) -> Self {
        let settings_path: Arc<Path> = options.user_dir.join(paths::SETTINGS_FILE).into();
        let keybindings_path = options.user_dir.join(paths::KEYBINDINGS_FILE);
        let watch = options.watch;
        let initial = [stat(&settings_path), stat(&keybindings_path)];
        let inner = Arc::new(Inner {
            settings_path,
            keybindings_path,
            options,
            registry,
            language_defaults: catalog::catalog().language_defaults(),
            snapshot: RwLock::default(),
            disk: Mutex::default(),
            io_lock: Mutex::default(),
            problems: Mutex::default(),
            revision: AtomicU64::new(0),
            changes: EventHub::default(),
            problem_events: EventHub::default(),
            keybindings: EventHub::default(),
        });
        {
            let _io = lock(&inner.io_lock);
            inner.check_settings(true);
            if let Ok(bytes) = file::read_optional(&inner.keybindings_path, inner.options.retry) {
                lock(&inner.disk).keybindings = bytes;
            }
        }
        let stop = watch.map(|w| {
            let stop = Arc::new(AtomicBool::new(false));
            spawn_watcher(Arc::downgrade(&inner), w, initial, stop.clone());
            stop
        });
        Self { inner, stop }
    }

    pub fn settings_path(&self) -> &Path {
        &self.inner.settings_path
    }

    pub fn keybindings_path(&self) -> &Path {
        &self.inner.keybindings_path
    }

    pub fn registry(&self) -> &SettingRegistry {
        &self.inner.registry
    }

    /// 强类型读取。按优先级逐层尝试，类型不符的用户值被跳过，并记录一条带行列的
    /// [`ProblemKind::TypeMismatch`] 警告；最终回落到默认值。只有默认值本身也无法解码
    /// （消费方用错了 `T`）时返回 `InvalidInput`。未登记的键返回 `NotFound`。
    pub fn read_typed<T: SettingValue>(&self, key: &Key<T>, scope: &Scope) -> Result<T> {
        let setting = self.inner.registry.get(key.id).ok_or(Error::NotFound)?;
        let user = self.inner.user();
        let mut mismatches = Vec::new();
        let mut result = Err(Error::InvalidInput);
        for c in model::candidates(&user, self.inner.language_defaults, key.id, setting, scope) {
            match T::decode(c.value) {
                Ok(v) => {
                    result = Ok(v);
                    break;
                }
                Err(_) => mismatches.extend(c.location),
            }
        }
        for (line, column) in mismatches {
            self.inner.warn(SettingsProblem {
                kind: ProblemKind::TypeMismatch { key: key.id.into() },
                path: self.inner.settings_path.clone(),
                line,
                column,
            });
        }
        result
    }

    /// 用户是否在 `settings.json` 中显式设置了 `key`（全局，或 `scope` 语言的覆盖块）。
    /// #7、#25、#29 用它区分“用户的值”与“Rove 默认值”。
    pub fn is_user_set(&self, key: &str, scope: &Scope) -> Result<bool> {
        let setting = self.inner.registry.get(key).ok_or(Error::NotFound)?;
        Ok(self.inner.user().is_set(key, setting, scope))
    }

    /// 当前生效值来自哪一层。
    pub fn layer(&self, key: &str, scope: &Scope) -> Result<Layer> {
        let setting = self.inner.registry.get(key).ok_or(Error::NotFound)?;
        let user = self.inner.user();
        let candidates =
            model::candidates(&user, self.inner.language_defaults, key, setting, scope);
        Ok(candidates[0].layer)
    }

    /// 当前的问题列表：文件级错误（如有）在前，随后是类型不匹配警告。
    pub fn problems(&self) -> Arc<[SettingsProblem]> {
        lock(&self.inner.problems).list()
    }

    /// 问题列表变化时发布完整列表（#16 的状态栏警告图标与问题面板订阅它）。
    pub fn problem_changes(&self) -> &dyn Events<Arc<[SettingsProblem]>> {
        &self.inner.problem_events
    }

    /// 同目录 `keybindings.json` 的内容变化（#14 订阅后自行解析）。
    pub fn keybindings_changes(&self) -> &dyn Events<KeybindingsChange> {
        &self.inner.keybindings
    }

    /// 删除用户的显式设置（“重置为默认值”），保留格式、先备份；语义同 [`Settings::write`]。
    pub fn remove(&self, key: Arc<str>, scope: Scope, cancel: Cancellation) -> Task<()> {
        let inner = self.inner.clone();
        Box::pin(async move { inner.write_blocking(&key, &scope, None, &cancel) })
    }

    /// 立即重读两个文件（不等轮询）。内容与已知一致时什么也不做。
    pub fn reload(&self) {
        let _io = lock(&self.inner.io_lock);
        self.inner.check_settings(false);
        self.inner.check_keybindings();
    }
}

impl Drop for SettingsService {
    fn drop(&mut self) {
        if let Some(stop) = &self.stop {
            stop.store(true, Ordering::Release);
        }
    }
}

impl Settings for SettingsService {
    fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
        let setting = self.inner.registry.get(key).ok_or(Error::NotFound)?;
        let user = self.inner.user();
        let candidates =
            model::candidates(&user, self.inner.language_defaults, key, setting, scope);
        Ok(candidates[0].value.clone())
    }

    fn write(&self, key: Arc<str>, scope: Scope, value: Value, cancel: Cancellation) -> Task<()> {
        let inner = self.inner.clone();
        Box::pin(async move { inner.write_blocking(&key, &scope, Some(value), &cancel) })
    }

    fn changes(&self) -> &dyn Events<SettingsChange> {
        &self.inner.changes
    }
}

impl Inner {
    fn user(&self) -> Arc<UserLayer> {
        self.snapshot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn warn(&self, problem: SettingsProblem) {
        let list = {
            let mut problems = lock(&self.problems);
            if problems.warnings.contains(&problem) {
                return;
            }
            problems.warnings.push(problem);
            problems.list()
        };
        self.problem_events.emit(list);
    }

    fn set_file_problem(&self, problem: Option<SettingsProblem>, clear_warnings: bool) {
        let list = {
            let mut problems = lock(&self.problems);
            let unchanged =
                problems.file == problem && !(clear_warnings && !problems.warnings.is_empty());
            if unchanged {
                return;
            }
            problems.file = problem;
            if clear_warnings {
                problems.warnings.clear();
            }
            problems.list()
        };
        self.problem_events.emit(list);
    }

    fn file_problem(&self, kind: ProblemKind, line: u32, column: u32) -> SettingsProblem {
        SettingsProblem {
            kind,
            path: self.settings_path.clone(),
            line,
            column,
        }
    }

    /// 读 `settings.json`，内容与已知不同（或 `force`）时应用。调用方持有 `io_lock`。
    /// 返回 `false` 表示读取失败（例如仍被占用），稍后应重试。
    fn check_settings(&self, force: bool) -> bool {
        let bytes = match file::read_optional(&self.settings_path, self.options.retry) {
            Ok(b) => b,
            Err(e) => {
                self.set_file_problem(
                    Some(self.file_problem(ProblemKind::Io(e.kind()), 1, 1)),
                    false,
                );
                return false;
            }
        };
        {
            let mut disk = lock(&self.disk);
            if !force && disk.settings == bytes {
                return true;
            }
            disk.settings = bytes.clone();
        }
        self.apply(bytes.as_deref());
        true
    }

    fn check_keybindings(&self) -> bool {
        let Ok(bytes) = file::read_optional(&self.keybindings_path, self.options.retry) else {
            return false;
        };
        {
            let mut disk = lock(&self.disk);
            if disk.keybindings == bytes {
                return true;
            }
            disk.keybindings = bytes.clone();
        }
        self.keybindings.emit(KeybindingsChange {
            text: bytes.map(|b| String::from_utf8_lossy(&b).into()),
        });
        true
    }

    /// 解析并替换快照；失败时保留旧快照并报告问题。成功时清除文件级错误与过期的类型警告，
    /// 只在已登记键的值确有变化时发布 [`SettingsChange`]（不支持的键静默忽略）。
    fn apply(&self, bytes: Option<&[u8]>) {
        let text = match bytes.map(std::str::from_utf8) {
            None => "",
            Some(Ok(t)) => t,
            Some(Err(_)) => {
                self.set_file_problem(Some(self.file_problem(ProblemKind::NotUtf8, 1, 1)), false);
                return;
            }
        };
        let doc = match Document::parse(text) {
            Ok(doc) => doc,
            Err(e) => {
                let p = self.file_problem(ProblemKind::Syntax(e.kind), e.line, e.column);
                self.set_file_problem(Some(p), false);
                return;
            }
        };
        if let Some(Node { kind, span }) = &doc.root {
            if !matches!(kind, NodeKind::Object(_)) {
                let (line, column) = crate::jsonc::line_column(text, span.start);
                self.set_file_problem(
                    Some(self.file_problem(ProblemKind::RootNotObject, line, column)),
                    false,
                );
                return;
            }
        }
        let new = Arc::new(UserLayer::from_document(text, &doc));
        let keys = {
            let mut snapshot = self
                .snapshot
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            let keys = snapshot.changed_keys(&new, &self.registry);
            *snapshot = new;
            keys
        };
        self.set_file_problem(None, true);
        if !keys.is_empty() {
            let revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
            self.changes.emit(SettingsChange {
                revision: Version(revision),
                keys: keys.into(),
            });
        }
    }

    fn write_blocking(
        &self,
        key: &str,
        scope: &Scope,
        value: Option<Value>,
        cancel: &Cancellation,
    ) -> Result<()> {
        cancel.check()?;
        let setting = self.registry.get(key).ok_or(Error::NotFound)?;
        let block;
        let path: Vec<&str> = match &scope.language {
            Some(language) => {
                if !setting.language_overridable
                    || language.is_empty()
                    || language.contains(['[', ']'])
                {
                    return Err(Error::InvalidInput);
                }
                block = format!("[{language}]");
                vec![&block, key]
            }
            None => vec![key],
        };
        let _io = lock(&self.io_lock);
        let retry = self.options.retry;
        let original = file::read_optional(&self.settings_path, retry)?;
        let text = match &original {
            Some(bytes) => std::str::from_utf8(bytes).map_err(|_| Error::InvalidInput)?,
            None => "",
        };
        let updated = edit::set(text, &path, value.as_ref()).map_err(|e| match e {
            EditError::Syntax(_) | EditError::RootNotObject => Error::InvalidInput,
        })?;
        if updated == text && (original.is_some() || value.is_none()) {
            return Ok(());
        }
        cancel.check()?;
        if let Some(bytes) = &original {
            // 备份失败就不写。
            file::backup(
                &self.options.backup_dir,
                bytes,
                self.options.backup_limit,
                SystemTime::now(),
            )?;
        }
        cancel.check()?;
        // 读取之后文件又被别人改了：放弃，不覆盖对方的修改。
        if file::read_optional(&self.settings_path, retry)? != original {
            return Err(Error::Stale);
        }
        let bytes = updated.into_bytes();
        file::atomic_write(&self.settings_path, &bytes, retry)?;
        // 线性化点：此后一律报告成功。先记下自己写入的内容，热加载看到它就不会重载。
        lock(&self.disk).settings = Some(bytes.clone());
        self.apply(Some(&bytes));
        Ok(())
    }
}

fn spawn_watcher(
    inner: Weak<Inner>,
    options: WatchOptions,
    initial: [Stat; 2],
    stop: Arc<AtomicBool>,
) {
    let spawned = thread::Builder::new()
        .name("rove-settings-watch".into())
        .spawn(move || {
            let mut last = initial;
            let mut pending: [Option<Instant>; 2] = [None, None];
            loop {
                thread::sleep(options.poll_interval);
                if stop.load(Ordering::Acquire) {
                    return;
                }
                let Some(inner) = inner.upgrade() else {
                    return;
                };
                let paths: [&Path; 2] = [&inner.settings_path, &inner.keybindings_path];
                for i in 0..2 {
                    let now = stat(paths[i]);
                    if now != last[i] {
                        last[i] = now;
                        pending[i] = Some(Instant::now());
                        continue;
                    }
                    let Some(since) = pending[i] else { continue };
                    if since.elapsed() < options.debounce {
                        continue;
                    }
                    let _io = lock(&inner.io_lock);
                    let done = if i == 0 {
                        inner.check_settings(false)
                    } else {
                        inner.check_keybindings()
                    };
                    pending[i] = (!done).then(Instant::now);
                }
            }
        });
    // 无法创建线程时退化为不监听：设置仍可读写，只是不热加载。
    drop(spawned);
}
