//! 内置设置项目录：`docs/compat/SETTINGS.md` 中“支持”的全部设置项（P0/P1/P2）、
//! VS Code 隐藏设置、`rove.*` 专有设置，以及 VS Code 的默认语言覆盖（`"[csharp]": {...}`）。
//!
//! 数据文件 `data/settings-catalog.json` 由 `scripts/compat/gen_settings_catalog.py` 生成，
//! 与 `SETTINGS.md` 同源；随二进制编译，运行期不读外部文件、不联网。

use crate::{jsonc, model::language_ids, Setting, SettingRegistry};
use rove_base::{Result, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

const DATA: &str = include_str!("../data/settings-catalog.json");

/// 功能优先级：决定该设置何时生效，不影响读取（P1/P2 只登记、按功能落地时生效）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    P0,
    P1,
    P2,
}

/// 设置项的来源。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// VS Code 默认设置导出中的设置项。
    VsCode,
    /// VS Code 的隐藏设置（不在默认设置导出中，例如 `editor.stopRenderingLineAfter`）。
    VsCodeHidden,
    /// Rove 专有的 `rove.*` 设置项。
    Rove,
}

#[derive(Clone, Debug)]
pub struct CatalogEntry {
    pub key: Arc<str>,
    pub priority: Priority,
    pub source: Source,
    pub setting: Setting,
}

/// 只读的内置目录。
pub struct Catalog {
    entries: BTreeMap<Arc<str>, CatalogEntry>,
    language_defaults: LanguageDefaults,
}

/// VS Code 的默认语言覆盖：语言 ID → 设置键 → 值。只包含 Rove 支持且可按语言覆盖的键。
#[derive(Clone, Debug, Default)]
pub struct LanguageDefaults(BTreeMap<String, BTreeMap<String, Value>>);

impl LanguageDefaults {
    pub fn get(&self, language: &str, key: &str) -> Option<&Value> {
        self.0.get(language)?.get(key)
    }

    /// 某个语言的全部默认覆盖。
    pub fn language(&self, language: &str) -> Option<&BTreeMap<String, Value>> {
        self.0.get(language)
    }
}

/// 进程内共享的内置目录（首次调用时解析一次）。
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| Catalog::parse(DATA).expect("embedded settings catalog is valid"))
}

/// 一张只含内置目录的设置表。
pub fn builtin_registry() -> SettingRegistry {
    let mut registry = SettingRegistry::default();
    register_builtin(&mut registry).expect("empty registry accepts the catalog");
    registry
}

/// 把内置目录登记进 `registry`；已由消费方登记的键保持原样（先登记者胜出），不报重复。
///
/// 装配顺序：各 crate 先调用自己的 `register`，最后调用本函数补齐其余设置项。
pub fn register_builtin(registry: &mut SettingRegistry) -> Result<()> {
    for (key, entry) in &catalog().entries {
        if registry.get(key).is_none() {
            registry.register(key.to_string(), entry.setting.clone())?;
        }
    }
    Ok(())
}

impl Catalog {
    fn parse(text: &str) -> std::result::Result<Self, String> {
        let root = jsonc::parse_value(text)
            .map_err(|e| e.to_string())?
            .ok_or("empty catalog")?;
        let Value::Object(root) = root else {
            return Err("catalog root is not an object".into());
        };
        let Some(Value::Array(settings)) = root.get("settings") else {
            return Err("missing settings".into());
        };
        let mut entries = BTreeMap::new();
        for item in settings.iter() {
            let Value::Object(item) = item else {
                return Err("setting is not an object".into());
            };
            let str_field = |name: &str| match item.get(name) {
                Some(Value::String(s)) => Ok(s.clone()),
                _ => Err(format!("missing {name}")),
            };
            let key = str_field("key")?;
            let priority = match &*str_field("priority")? {
                "P0" => Priority::P0,
                "P1" => Priority::P1,
                "P2" => Priority::P2,
                other => return Err(format!("{key}: bad priority {other}")),
            };
            let source = match &*str_field("source")? {
                "vscode" => Source::VsCode,
                "vscode-hidden" => Source::VsCodeHidden,
                "rove" => Source::Rove,
                other => return Err(format!("{key}: bad source {other}")),
            };
            let setting = Setting {
                default: item
                    .get("default")
                    .cloned()
                    .ok_or(format!("{key}: no default"))?,
                rove_default: item.get("roveDefault").cloned(),
                language_overridable: matches!(
                    item.get("languageOverridable"),
                    Some(Value::Bool(true))
                ),
            };
            entries.insert(
                key.clone(),
                CatalogEntry {
                    key,
                    priority,
                    source,
                    setting,
                },
            );
        }
        let mut language_defaults = BTreeMap::<String, BTreeMap<String, Value>>::new();
        if let Some(Value::Object(blocks)) = root.get("languageDefaults") {
            for (spec, block) in blocks.iter() {
                let (Some(languages), Value::Object(block)) = (language_ids(spec), block) else {
                    return Err(format!("bad language block {spec}"));
                };
                for language in languages {
                    let target = language_defaults.entry(language.to_string()).or_default();
                    for (k, v) in block.iter() {
                        target.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        Ok(Self {
            entries,
            language_defaults: LanguageDefaults(language_defaults),
        })
    }

    pub fn get(&self, key: &str) -> Option<&CatalogEntry> {
        self.entries.get(key)
    }

    /// 按键名排序遍历。
    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.values()
    }

    pub fn language_defaults(&self) -> &LanguageDefaults {
        &self.language_defaults
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_parses_with_expected_shape() {
        let c = catalog();
        let tab = c.get("editor.tabSize").unwrap();
        assert_eq!(tab.setting.default, Value::Number(4.0));
        assert!(tab.setting.language_overridable);
        let rove = c.get("rove.largeFile.threshold").unwrap();
        assert_eq!(rove.source, Source::Rove);
        assert_eq!(rove.setting.default, Value::Number(67_108_864.0));
        assert_eq!(
            c.get("editor.stopRenderingLineAfter").unwrap().source,
            Source::VsCodeHidden
        );
        assert_eq!(
            c.language_defaults()
                .get("csharp", "editor.maxTokenizationLineLength"),
            Some(&Value::Number(2500.0))
        );
        // 不支持的内层键（如 gitlens.*、editor.quickSuggestions）不进入默认语言覆盖。
        assert!(c
            .language_defaults()
            .get("yaml", "editor.quickSuggestions")
            .is_none());
        assert_eq!(
            c.language_defaults().get("yaml", "editor.tabSize"),
            Some(&Value::Number(2.0))
        );
    }

    #[test]
    fn register_builtin_keeps_consumer_definitions() {
        let mut r = SettingRegistry::default();
        r.register(
            "files.encoding",
            Setting {
                default: Value::String("utf8".into()),
                rove_default: None,
                language_overridable: false,
            },
        )
        .unwrap();
        register_builtin(&mut r).unwrap();
        assert!(!r.get("files.encoding").unwrap().language_overridable);
        assert!(r.get("editor.fontSize").is_some());
        register_builtin(&mut r).unwrap();
    }
}
