//! `search.*` 设置项的登记与读取（#25）。
//!
//! 只登记本 crate 读取的键，默认值与 VS Code 一致（`compat/SETTINGS.md`）；`rove-config` 的
//! `register_builtin` 也会登记同名键，先登记者胜出，`app_settings` 的测试校验两边默认值相同。
//! `search.useIgnoreFiles`、`search.useParentIgnoreFiles`、`search.useGlobalIgnoreFiles` **不登记、不读取**：
//! Rove 从不读取 `.gitignore`（FEATURES §6，SETTINGS.md §3）。

use rove_base::{Error, Result, Value};
use rove_config::{Key, Scope, Setting, SettingRegistry, Settings};
use rove_workspace::{RuleSet, FILES_EXCLUDE, ROVE_EXTRA_EXCLUDE};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const SEARCH_EXCLUDE: &str = "search.exclude";
pub const FOLLOW_SYMLINKS: &str = "search.followSymlinks";
pub const MAX_RESULTS: &str = "search.maxResults";
pub const MAX_THREADS: &str = "search.ripgrep.maxThreads";
pub const SMART_CASE: &str = "search.smartCase";
pub const SEARCH_ON_TYPE: &str = "search.searchOnType";
pub const SEARCH_ON_TYPE_DEBOUNCE: &str = "search.searchOnTypeDebouncePeriod";
pub const COLLAPSE_RESULTS: &str = "search.collapseResults";
pub const SHOW_LINE_NUMBERS: &str = "search.showLineNumbers";
pub const SORT_ORDER: &str = "search.sortOrder";
pub const DECORATION_BADGES: &str = "search.decorations.badges";
pub const DECORATION_COLORS: &str = "search.decorations.colors";
pub const MODE: &str = "search.mode";
pub const SEED_WITH_NEAREST_WORD: &str = "search.seedWithNearestWord";

/// VS Code 的 `search.exclude` 默认值。
const SEARCH_EXCLUDE_DEFAULT: &[(&str, bool)] = &[
    ("**/node_modules", true),
    ("**/bower_components", true),
    ("**/*.code-search", true),
];

/// `search.maxResults` 的 VS Code 默认值。
pub const DEFAULT_MAX_RESULTS: usize = 20_000;

fn object(pairs: &[(&str, bool)]) -> Value {
    Value::Object(Arc::new(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), Value::Bool(*v)))
            .collect::<BTreeMap<_, _>>(),
    ))
}

/// 本 crate 读取的 `search.*` 键与 VS Code 默认值。
pub fn defaults() -> Vec<(&'static str, Value)> {
    vec![
        (SEARCH_EXCLUDE, object(SEARCH_EXCLUDE_DEFAULT)),
        (FOLLOW_SYMLINKS, Value::Bool(true)),
        (MAX_RESULTS, Value::Number(DEFAULT_MAX_RESULTS as f64)),
        (MAX_THREADS, Value::Number(0.0)),
        (SMART_CASE, Value::Bool(false)),
        (SEARCH_ON_TYPE, Value::Bool(true)),
        (SEARCH_ON_TYPE_DEBOUNCE, Value::Number(300.0)),
        (COLLAPSE_RESULTS, Value::String("alwaysExpand".into())),
        (SHOW_LINE_NUMBERS, Value::Bool(false)),
        (SORT_ORDER, Value::String("default".into())),
        (DECORATION_BADGES, Value::Bool(true)),
        (DECORATION_COLORS, Value::Bool(true)),
        (MODE, Value::String("view".into())),
        (SEED_WITH_NEAREST_WORD, Value::Bool(false)),
    ]
}

/// 登记本 crate 的设置项；已被其他模块登记的键保持原样。
pub fn register_settings(registry: &mut SettingRegistry) -> Result<()> {
    for (key, default) in defaults() {
        match registry.register(
            key,
            Setting {
                default,
                rove_default: None,
                language_overridable: false,
            },
        ) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 一次搜索用到的设置快照（不可变，可跨线程共享）。
#[derive(Clone)]
pub struct SearchConfig {
    /// `files.exclude` 叠在 `rove.files.extraExclude` 之上（与 #15 的工作区同一套合并实现）。
    pub files_exclude: RuleSet,
    /// 只有 `files.exclude`：用来判断某个目录是否**只**因为 Rove 的附加排除而被排除。
    pub files_exclude_only: RuleSet,
    /// `search.exclude`。
    pub search_exclude: RuleSet,
    pub follow_symlinks: bool,
    /// `None` 表示不限（`search.maxResults` 为 `null` 或非正数）。
    pub max_results: Option<usize>,
    /// `0` 表示自动。
    pub threads: usize,
    pub smart_case: bool,
    /// 解码非 UTF-8 文件时传给 #7 的编码识别（`files.encoding`、`files.autoGuessEncoding`、候选编码）。
    pub settings: Arc<dyn Settings>,
}

impl SearchConfig {
    /// 读不到或类型不符的键回落到 VS Code 默认值，不让一个坏设置让搜索失败。
    pub fn from_settings(settings: Arc<dyn Settings>) -> Self {
        let scope = Scope::default();
        let value = |key: &'static str| Key::<Value>::new(key).read(&*settings, &scope).ok();
        let empty = Value::Object(Arc::new(BTreeMap::new()));
        let files = value(FILES_EXCLUDE).unwrap_or_else(|| empty.clone());
        let extra = value(ROVE_EXTRA_EXCLUDE).unwrap_or_else(|| empty.clone());
        let search = value(SEARCH_EXCLUDE).unwrap_or_else(|| object(SEARCH_EXCLUDE_DEFAULT));
        let rules = |v: &Value| RuleSet::from_value(v).unwrap_or_default();
        let boolean = |key: &'static str, default: bool| {
            Key::<bool>::new(key)
                .read(&*settings, &scope)
                .unwrap_or(default)
        };
        let max_results = match value(MAX_RESULTS) {
            Some(Value::Number(n)) if n >= 1.0 => Some(n as usize),
            Some(Value::Number(_)) | Some(Value::Null) => None,
            _ => Some(DEFAULT_MAX_RESULTS),
        };
        let threads = match value(MAX_THREADS) {
            Some(Value::Number(n)) if n >= 1.0 => n as usize,
            _ => 0,
        };
        Self {
            files_exclude: RuleSet::merged(&files, &extra).unwrap_or_default(),
            files_exclude_only: rules(&files),
            search_exclude: rules(&search),
            follow_symlinks: boolean(FOLLOW_SYMLINKS, true),
            max_results,
            threads,
            smart_case: boolean(SMART_CASE, false),
            settings,
        }
    }
}

/// 界面读取的展示类设置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewSettings {
    pub search_on_type: bool,
    pub debounce_ms: u64,
    /// `search.collapseResults`：`auto` 时超过 10 处匹配的文件默认折叠；`alwaysCollapse` / `alwaysExpand`。
    pub collapse: Collapse,
    pub show_line_numbers: bool,
    pub sort_order: SortOrder,
    pub badges: bool,
    pub colors: bool,
    pub seed_with_nearest_word: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collapse {
    Auto,
    AlwaysCollapse,
    AlwaysExpand,
}

/// `search.sortOrder`。`modified` 需要文件修改时间，按 `default` 处理（见 ARCHITECTURE §32）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Default,
    FileNames,
    Type,
    Modified,
    CountDescending,
    CountAscending,
}

impl ViewSettings {
    pub fn read(settings: &dyn Settings) -> Self {
        let scope = Scope::default();
        let boolean = |key: &'static str, default: bool| {
            Key::<bool>::new(key)
                .read(settings, &scope)
                .unwrap_or(default)
        };
        let string = |key: &'static str| {
            Key::<String>::new(key)
                .read(settings, &scope)
                .unwrap_or_default()
        };
        let debounce = Key::<f64>::new(SEARCH_ON_TYPE_DEBOUNCE)
            .read(settings, &scope)
            .ok()
            .filter(|n| *n >= 0.0)
            .unwrap_or(300.0) as u64;
        Self {
            search_on_type: boolean(SEARCH_ON_TYPE, true),
            debounce_ms: debounce,
            collapse: match string(COLLAPSE_RESULTS).as_str() {
                "auto" => Collapse::Auto,
                "alwaysCollapse" => Collapse::AlwaysCollapse,
                _ => Collapse::AlwaysExpand,
            },
            show_line_numbers: boolean(SHOW_LINE_NUMBERS, false),
            sort_order: match string(SORT_ORDER).as_str() {
                "fileNames" => SortOrder::FileNames,
                "type" => SortOrder::Type,
                "modified" => SortOrder::Modified,
                "countDescending" => SortOrder::CountDescending,
                "countAscending" => SortOrder::CountAscending,
                _ => SortOrder::Default,
            },
            badges: boolean(DECORATION_BADGES, true),
            colors: boolean(DECORATION_COLORS, true),
            seed_with_nearest_word: boolean(SEED_WITH_NEAREST_WORD, false),
        }
    }
}

/// 只含本 crate、`rove-workspace` 与 #7 编码键默认值的设置（测试与基准用）。
pub fn default_settings() -> rove_config::DefaultSettings {
    let mut registry = SettingRegistry::default();
    register_settings(&mut registry).expect("search settings register once");
    rove_workspace::register_settings(&mut registry).expect("workspace settings register once");
    rove_buffer::settings::register(&mut registry).expect("encoding settings register once");
    rove_config::DefaultSettings(registry)
}
