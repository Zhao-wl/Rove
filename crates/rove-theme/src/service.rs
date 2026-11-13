//! 主题服务：内置主题目录、按设置选择主题、用户覆盖、重新加载与变更通知（#10）。

use crate::defaults::register_p0_colors;
use crate::file::{self, Diagnostic, ThemeSource};
use crate::resolved::{Customizations, ResolvedTheme};
use crate::{Color, ColorRegistry, Theme, ThemeKind, TokenStyle};
use rove_base::{Error, Events, Result, Subscription, Value, Version};
use rove_config::{Key, Scope, Settings};
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};

/// 未设置 `workbench.colorTheme` 或它指向不存在的主题时使用的主题（VS Code 1.145 的默认值）。
pub const DEFAULT_THEME: &str = "Dark 2026";

pub const COLOR_THEME: Key<String> = Key::new("workbench.colorTheme");
pub const COLOR_CUSTOMIZATIONS: Key<Value> = Key::new("workbench.colorCustomizations");
pub const TOKEN_COLOR_CUSTOMIZATIONS: Key<Value> = Key::new("editor.tokenColorCustomizations");
/// 触发重新加载的设置键。
pub const THEME_SETTING_KEYS: [&str; 3] = [
    COLOR_THEME.id,
    COLOR_CUSTOMIZATIONS.id,
    TOKEN_COLOR_CUSTOMIZATIONS.id,
];

/// 一个可选主题：设置中使用的 ID、显示名称、界面类型与主题文件。
#[derive(Clone)]
pub struct ThemeEntry {
    /// `workbench.colorTheme` 的取值（VS Code 的 settingsId）。
    pub id: Arc<str>,
    pub label: Arc<str>,
    pub kind: ThemeKind,
    pub path: Arc<str>,
    pub source: Arc<dyn ThemeSource>,
}

/// 随二进制打包的 VS Code 默认主题（`extensions/theme-defaults`，MIT，原样复制）。
struct Builtin;

const BUILTIN_FILES: &[(&str, &str)] = &[
    (
        "2026-dark.json",
        include_str!("../data/themes/2026-dark.json"),
    ),
    (
        "2026-light.json",
        include_str!("../data/themes/2026-light.json"),
    ),
    (
        "dark_modern.json",
        include_str!("../data/themes/dark_modern.json"),
    ),
    (
        "light_modern.json",
        include_str!("../data/themes/light_modern.json"),
    ),
    (
        "dark_plus.json",
        include_str!("../data/themes/dark_plus.json"),
    ),
    (
        "light_plus.json",
        include_str!("../data/themes/light_plus.json"),
    ),
    ("dark_vs.json", include_str!("../data/themes/dark_vs.json")),
    (
        "light_vs.json",
        include_str!("../data/themes/light_vs.json"),
    ),
    (
        "hc_black.json",
        include_str!("../data/themes/hc_black.json"),
    ),
    (
        "hc_light.json",
        include_str!("../data/themes/hc_light.json"),
    ),
];

impl ThemeSource for Builtin {
    fn read(&self, path: &str) -> Result<Cow<'static, str>> {
        BUILTIN_FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, text)| Cow::Borrowed(*text))
            .ok_or(Error::NotFound)
    }
}

/// 与 `extensions/theme-defaults/package.json` 的 `contributes.themes` 一致。
const BUILTIN_THEMES: &[(&str, &str, ThemeKind, &str)] = &[
    ("Dark 2026", "Dark 2026", ThemeKind::Dark, "2026-dark.json"),
    (
        "Light 2026",
        "Light 2026",
        ThemeKind::Light,
        "2026-light.json",
    ),
    (
        "Dark Modern",
        "Dark Modern",
        ThemeKind::Dark,
        "dark_modern.json",
    ),
    (
        "Light Modern",
        "Light Modern",
        ThemeKind::Light,
        "light_modern.json",
    ),
    ("Dark+", "Dark+", ThemeKind::Dark, "dark_plus.json"),
    ("Light+", "Light+", ThemeKind::Light, "light_plus.json"),
    (
        "Visual Studio Dark",
        "Dark (Visual Studio)",
        ThemeKind::Dark,
        "dark_vs.json",
    ),
    (
        "Visual Studio Light",
        "Light (Visual Studio)",
        ThemeKind::Light,
        "light_vs.json",
    ),
    (
        "Default High Contrast",
        "Dark High Contrast",
        ThemeKind::HighContrastDark,
        "hc_black.json",
    ),
    (
        "Default High Contrast Light",
        "Light High Contrast",
        ThemeKind::HighContrastLight,
        "hc_light.json",
    ),
];

/// 可选主题目录。默认只含内置主题；R68 加载扩展主题时向这里追加条目。
#[derive(Clone)]
pub struct ThemeCatalog(Vec<ThemeEntry>);

impl Default for ThemeCatalog {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ThemeCatalog {
    pub fn builtin() -> Self {
        let source: Arc<dyn ThemeSource> = Arc::new(Builtin);
        Self(
            BUILTIN_THEMES
                .iter()
                .map(|(id, label, kind, path)| ThemeEntry {
                    id: (*id).into(),
                    label: (*label).into(),
                    kind: *kind,
                    path: (*path).into(),
                    source: source.clone(),
                })
                .collect(),
        )
    }

    /// 追加一个主题；ID 重复返回 `Duplicate`。
    pub fn add(&mut self, entry: ThemeEntry) -> Result<()> {
        if self.get(&entry.id).is_some() {
            return Err(Error::Duplicate(entry.id.to_string()));
        }
        self.0.push(entry);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&ThemeEntry> {
        self.0.iter().find(|e| &*e.id == id)
    }

    pub fn entries(&self) -> &[ThemeEntry] {
        &self.0
    }
}

/// 加载目录中的一个主题；`kind` 以登记的界面类型为准（与 VS Code 的 `uiTheme` 相同）。
pub fn load_theme(
    entry: &ThemeEntry,
    custom: &Customizations,
    registry: Arc<ColorRegistry>,
) -> Result<ResolvedTheme> {
    let doc = file::load(&*entry.source, &entry.path)?;
    Ok(ResolvedTheme::build(
        &entry.id, entry.kind, doc, custom, registry,
    ))
}

/// 登记了全部 P0 颜色键的登记表。
pub fn p0_registry() -> ColorRegistry {
    let mut registry = ColorRegistry::default();
    register_p0_colors(&mut registry).expect("P0 keys are unique VS Code colors");
    registry
}

type Listener = Arc<dyn Fn(Version) + Send + Sync>;

/// 简单的事件广播：回调在发布线程上、锁释放后调用。
#[derive(Default)]
struct Broadcast {
    next: AtomicU64,
    listeners: Mutex<Vec<(u64, Listener)>>,
}

struct Hub(Arc<Broadcast>);

impl Events<Version> for Hub {
    fn subscribe(&self, listener: Listener) -> Subscription {
        let id = self.0.next.fetch_add(1, Ordering::Relaxed);
        self.0
            .listeners
            .lock()
            .expect("listeners")
            .push((id, listener));
        let weak: Weak<Broadcast> = Arc::downgrade(&self.0);
        Subscription::new(move || {
            if let Some(b) = weak.upgrade() {
                if let Ok(mut l) = b.listeners.lock() {
                    l.retain(|(i, _)| *i != id);
                }
            }
        })
    }
}

impl Hub {
    fn emit(&self, v: Version) {
        let listeners: Vec<Listener> = self
            .0
            .listeners
            .lock()
            .expect("listeners")
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        for l in listeners {
            l(v);
        }
    }
}

/// 正式的主题服务，实现 #1 的 [`Theme`] 契约。
///
/// - 读取：`color` 是对构建好的快照查表（O(log 键数)），`token_style` 先查缓存，未命中时做
///   O(scope 深度 × 候选规则) 的匹配；都不做 I/O，可以在 UI 线程调用。
/// - 重新加载：设置中的 `workbench.colorTheme` / `workbench.colorCustomizations` /
///   `editor.tokenColorCustomizations` 变化时在后台线程重新加载；主题文件被修改时由监听方调用
///   [`ThemeService::reload`]。成功后替换快照并通过 `changes()` 发布新的修订号；
///   入口文件无法加载时保留旧快照并记录诊断。
pub struct ThemeService {
    current: RwLock<Arc<ResolvedTheme>>,
    registry: Arc<ColorRegistry>,
    catalog: ThemeCatalog,
    settings: Arc<dyn Settings>,
    revision: AtomicU64,
    hub: Hub,
    reload_pending: AtomicBool,
    /// 重新加载的代次：每次 `reload` 开始时领取一个号，读设置在领号之后。
    reload_requested: AtomicU64,
    /// 已应用到 `current` 的最大代次；较早开始、较晚完成的加载不得覆盖较新的结果。
    reload_applied: Mutex<u64>,
    log: Mutex<Vec<Diagnostic>>,
    _settings_subscription: Mutex<Option<Subscription>>,
}

impl ThemeService {
    /// 用内置主题目录与全部 P0 颜色键创建，并订阅设置变化。
    pub fn new(settings: Arc<dyn Settings>) -> Arc<Self> {
        Self::with_catalog(settings, ThemeCatalog::builtin(), Arc::new(p0_registry()))
    }

    pub fn with_catalog(
        settings: Arc<dyn Settings>,
        catalog: ThemeCatalog,
        registry: Arc<ColorRegistry>,
    ) -> Arc<Self> {
        let (theme, diags) = Self::build(&*settings, &catalog, &registry);
        // 首次加载失败（例如目录里的主题文件损坏）时退回内置默认主题，保证总有可用快照。
        let theme = theme.unwrap_or_else(|_| {
            let builtin = ThemeCatalog::builtin();
            let entry = builtin
                .get(DEFAULT_THEME)
                .expect("default theme is built in");
            load_theme(entry, &Customizations::default(), registry.clone())
                .expect("built-in default theme loads")
        });
        let service = Arc::new(Self {
            current: RwLock::new(Arc::new(theme)),
            registry,
            catalog,
            settings: settings.clone(),
            revision: AtomicU64::new(1),
            hub: Hub(Arc::default()),
            reload_pending: AtomicBool::new(false),
            reload_requested: AtomicU64::new(0),
            reload_applied: Mutex::new(0),
            log: Mutex::new(Vec::new()),
            _settings_subscription: Mutex::new(None),
        });
        service.record(diags);
        let weak = Arc::downgrade(&service);
        let sub = settings.changes().subscribe(Arc::new(move |change| {
            let relevant = change.keys.is_empty()
                || change
                    .keys
                    .iter()
                    .any(|k| THEME_SETTING_KEYS.contains(&&**k));
            if let (true, Some(s)) = (relevant, weak.upgrade()) {
                s.schedule_reload();
            }
        }));
        *service._settings_subscription.lock().expect("subscription") = Some(sub);
        service
    }

    fn read_settings(
        settings: &dyn Settings,
        diags: &mut Vec<Diagnostic>,
    ) -> (Option<String>, Customizations) {
        let scope = Scope::default();
        let mut opt = |key: &str, r: Result<Value>| match r {
            Ok(Value::Null) | Err(Error::NotFound) => None,
            Ok(v) => Some(v),
            Err(e) => {
                diags.push(Diagnostic {
                    file: "settings.json".into(),
                    field: key.into(),
                    message: format!("无法读取设置：{e}"),
                });
                None
            }
        };
        let theme =
            opt(COLOR_THEME.id, settings.read(COLOR_THEME.id, &scope)).and_then(|v| match v {
                Value::String(s) => Some(s.to_string()),
                _ => None,
            });
        let custom = Customizations {
            colors: opt(
                COLOR_CUSTOMIZATIONS.id,
                COLOR_CUSTOMIZATIONS.read(settings, &scope),
            ),
            token_colors: opt(
                TOKEN_COLOR_CUSTOMIZATIONS.id,
                TOKEN_COLOR_CUSTOMIZATIONS.read(settings, &scope),
            ),
        };
        (theme, custom)
    }

    fn build(
        settings: &dyn Settings,
        catalog: &ThemeCatalog,
        registry: &Arc<ColorRegistry>,
    ) -> (Result<ResolvedTheme>, Vec<Diagnostic>) {
        let mut diags = Vec::new();
        let (wanted, custom) = Self::read_settings(settings, &mut diags);
        let wanted = wanted.unwrap_or_else(|| DEFAULT_THEME.to_owned());
        let entry = match catalog.get(&wanted) {
            Some(e) => e,
            None => {
                diags.push(Diagnostic {
                    file: "settings.json".into(),
                    field: COLOR_THEME.id.into(),
                    message: format!(
                        "主题 {wanted:?} 未安装（扩展主题由 R68 支持），回退到内置主题 {DEFAULT_THEME:?}"
                    ),
                });
                catalog
                    .get(DEFAULT_THEME)
                    .expect("default theme is built in")
            }
        };
        let theme = load_theme(entry, &custom, registry.clone());
        if let Err(e) = &theme {
            diags.push(Diagnostic {
                file: entry.path.to_string(),
                field: String::new(),
                message: format!("无法加载主题 {:?}：{e}，保留当前主题", entry.id),
            });
        }
        (theme, diags)
    }

    fn record(&self, mut diags: Vec<Diagnostic>) {
        diags.extend(self.current().diagnostics().iter().cloned());
        for d in &diags {
            eprintln!("rove-theme: {d}");
        }
        *self.log.lock().expect("log") = diags;
    }

    fn schedule_reload(self: Arc<Self>) {
        if self.reload_pending.swap(true, Ordering::AcqRel) {
            return; // 已有一次待执行的重新加载，合并
        }
        std::thread::spawn(move || {
            self.reload_pending.store(false, Ordering::Release);
            let _ = self.reload(); // 失败已记录诊断，旧快照保留
        });
    }

    /// 同步重新加载当前设置指向的主题。成功时替换快照、发布新修订号并返回它；
    /// 失败时保留旧快照，返回错误。
    ///
    /// 多次重新加载可以并发执行（例如快速连续切换两次主题）。每次加载在读设置之前领取代次，
    /// 只有代次不低于已应用代次的结果才会替换快照；被更新的加载取代的旧结果返回 `Stale`，
    /// 因此最后生效的总是最后开始（读到最新设置）的那一次。
    pub fn reload(&self) -> Result<Version> {
        let ticket = self.reload_requested.fetch_add(1, Ordering::AcqRel) + 1;
        let (theme, diags) = Self::build(&*self.settings, &self.catalog, &self.registry);
        let result = match theme {
            Ok(theme) => {
                let v = {
                    let mut applied = self.reload_applied.lock().expect("reload generation");
                    if *applied > ticket {
                        return Err(Error::Stale);
                    }
                    *applied = ticket;
                    *self.current.write().expect("theme lock") = Arc::new(theme);
                    Version(self.revision.fetch_add(1, Ordering::AcqRel) + 1)
                };
                self.record(diags);
                self.hub.emit(v);
                Ok(v)
            }
            Err(e) => {
                self.record(diags);
                Err(e)
            }
        };
        result
    }

    /// 只在内存中换一个主题（#28 的颜色主题实时预览）：**不写设置文件**。
    ///
    /// 与 [`reload`](Self::reload) 共用代次闸门，所以预览期间收到的设置变更仍会按顺序生效。
    /// `id` 不在目录里时返回 `NotFound` 并保留当前快照。还原预览用
    /// [`reload`](Self::reload)：它按设置重新构建，等于回到用户真正选择的主题。
    pub fn preview(&self, id: &str) -> Result<Version> {
        let ticket = self.reload_requested.fetch_add(1, Ordering::AcqRel) + 1;
        let entry = self.catalog.get(id).ok_or(Error::NotFound)?.clone();
        let mut diags = Vec::new();
        let (_, custom) = Self::read_settings(&*self.settings, &mut diags);
        let theme = load_theme(&entry, &custom, self.registry.clone())?;
        let version = {
            let mut applied = self.reload_applied.lock().expect("reload generation");
            if *applied > ticket {
                return Err(Error::Stale);
            }
            *applied = ticket;
            *self.current.write().expect("theme lock") = Arc::new(theme);
            Version(self.revision.fetch_add(1, Ordering::AcqRel) + 1)
        };
        self.hub.emit(version);
        Ok(version)
    }

    /// 当前快照；可以跨线程持有，不受之后的重新加载影响。
    pub fn current(&self) -> Arc<ResolvedTheme> {
        self.current.read().expect("theme lock").clone()
    }

    pub fn revision(&self) -> Version {
        Version(self.revision.load(Ordering::Acquire))
    }

    /// 最近一次加载的诊断（设置与主题文件），与写到 stderr 的日志相同。
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.log.lock().expect("log").clone()
    }

    pub fn catalog(&self) -> &ThemeCatalog {
        &self.catalog
    }

    pub fn registry(&self) -> &ColorRegistry {
        &self.registry
    }
}

impl Theme for ThemeService {
    fn color(&self, id: &str) -> Result<Option<Color>> {
        self.current().color(id)
    }

    fn token_style(&self, scopes: &[Arc<str>]) -> TokenStyle {
        self.current().token_style(scopes)
    }

    fn changes(&self) -> &dyn Events<Version> {
        &self.hub
    }
}
