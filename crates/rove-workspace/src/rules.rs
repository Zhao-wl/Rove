//! Registered workspace settings, exclusion merging and the read-only truth table (#15).
//!
//! Every key this crate reads is registered here with its default, so the workspace never
//! hard-codes a default that `rove-config` does not know about. Settings file loading is #13's
//! job; this module only owns the registration and the strongly typed snapshot
//! ([`WorkspaceConfig`]) that the scan/watcher use.

use crate::glob::RuleSet;
use rove_base::{Result, Value};
use rove_buffer::ReadOnlyReason;
use rove_config::{DefaultSettings, Scope, Setting, SettingRegistry, SettingValue, Settings};
use std::collections::BTreeMap;
use std::sync::Arc;

/// `files.exclude`: VS Code key, object of globs to booleans.
pub const FILES_EXCLUDE: &str = "files.exclude";
/// `files.watcherExclude`: registered separately from the explorer exclusion.
pub const FILES_WATCHER_EXCLUDE: &str = "files.watcherExclude";
/// `files.readonlyExclude`: cancels the rule-based read-only reasons.
pub const FILES_READONLY_EXCLUDE: &str = "files.readonlyExclude";
/// `files.readonlyFromPermissions`: use the on-disk read-only attribute.
pub const FILES_READONLY_FROM_PERMISSIONS: &str = "files.readonlyFromPermissions";
/// `files.readonlyInclude`: rule-based read-only.
pub const FILES_READONLY_INCLUDE: &str = "files.readonlyInclude";
/// `rove.files.extraExclude`: Rove's additional explorer exclusion.
pub const ROVE_EXTRA_EXCLUDE: &str = "rove.files.extraExclude";
/// `rove.files.extraReadonlyInclude`: Rove's additional read-only rules.
pub const ROVE_EXTRA_READONLY_INCLUDE: &str = "rove.files.extraReadonlyInclude";

/// `ReadOnlyReason::Rule` value for `files.readonlyInclude`.
///
/// The string is the setting key, not a user-visible sentence: the UI maps the key to a
/// registered `Message` (ARCHITECTURE §4) instead of the core crate owning Chinese copy.
pub const REASON_READONLY_INCLUDE: &str = FILES_READONLY_INCLUDE;
/// `ReadOnlyReason::Rule` value for `rove.files.extraReadonlyInclude`.
pub const REASON_EXTRA_READONLY_INCLUDE: &str = ROVE_EXTRA_READONLY_INCLUDE;

/// VS Code's default `files.exclude` (compat/vscode/default-settings.jsonc).
const FILES_EXCLUDE_DEFAULT: &[(&str, bool)] = &[
    ("**/.git", true),
    ("**/.svn", true),
    ("**/.hg", true),
    ("**/.jj", true),
    ("**/.DS_Store", true),
    ("**/Thumbs.db", true),
];

/// VS Code's default `files.watcherExclude`.
const WATCHER_EXCLUDE_DEFAULT: &[(&str, bool)] = &[
    (".git/objects/**", true),
    (".git/subtree-cache/**", true),
    (".hg/store/**", true),
    ("*/.git/objects/**", true),
    ("*/.git/subtree-cache/**", true),
    ("*/.hg/store/**", true),
];

/// Rove's default `rove.files.extraExclude` (compat/SETTINGS.md §4).
const ROVE_EXTRA_EXCLUDE_DEFAULT: &[(&str, bool)] = &[
    ("**/Library", true),
    ("**/Temp", true),
    ("**/obj", true),
    ("**/Logs", true),
];

/// Rove's default `rove.files.extraReadonlyInclude` (compat/SETTINGS.md §4).
const ROVE_EXTRA_READONLY_INCLUDE_DEFAULT: &[(&str, bool)] =
    &[("**/Library/PackageCache/**", true)];

fn object(pairs: &[(&str, bool)]) -> Value {
    let map: BTreeMap<String, Value> = pairs
        .iter()
        .map(|(pattern, value)| ((*pattern).to_string(), Value::Bool(*value)))
        .collect();
    Value::Object(Arc::new(map))
}

fn empty_object() -> Value {
    Value::Object(Arc::new(BTreeMap::new()))
}

/// Registers every setting this crate reads. #13 wires this into the real settings service.
pub fn register_settings(registry: &mut SettingRegistry) -> Result<()> {
    let setting = |default: Value| Setting {
        default,
        rove_default: None,
        language_overridable: false,
    };
    registry.register(FILES_EXCLUDE, setting(object(FILES_EXCLUDE_DEFAULT)))?;
    registry.register(
        FILES_WATCHER_EXCLUDE,
        setting(object(WATCHER_EXCLUDE_DEFAULT)),
    )?;
    registry.register(FILES_READONLY_EXCLUDE, setting(empty_object()))?;
    registry.register(FILES_READONLY_INCLUDE, setting(empty_object()))?;
    registry.register(FILES_READONLY_FROM_PERMISSIONS, setting(Value::Bool(false)))?;
    registry.register(
        ROVE_EXTRA_EXCLUDE,
        setting(object(ROVE_EXTRA_EXCLUDE_DEFAULT)),
    )?;
    registry.register(
        ROVE_EXTRA_READONLY_INCLUDE,
        setting(object(ROVE_EXTRA_READONLY_INCLUDE_DEFAULT)),
    )?;
    Ok(())
}

/// `DefaultSettings` carrying exactly this crate's registered defaults.
///
/// Until #13 lands the settings loader, this is how the workspace (and its tests) obtain the
/// registered defaults without duplicating them.
pub fn default_settings() -> DefaultSettings {
    let mut registry = SettingRegistry::default();
    register_settings(&mut registry)
        .expect("workspace settings are registered once with distinct ids");
    DefaultSettings(registry)
}

/// Read-only evaluation: rules plus the on-disk permission flag.
#[derive(Clone, Debug)]
pub struct ReadonlyRules {
    /// `files.readonlyFromPermissions`.
    pub from_permissions: bool,
    /// `files.readonlyInclude`.
    pub include: RuleSet,
    /// `files.readonlyExclude`.
    pub exclude: RuleSet,
    /// `rove.files.extraReadonlyInclude`.
    pub extra_include: RuleSet,
}

impl ReadonlyRules {
    /// Truth table, in evaluation order:
    ///
    /// 1. `files.readonlyExclude` matches -> no rule-based or permission-based reason
    ///    (VS Code lets `readonlyExclude` substitute `files.readonlyFromPermissions`).
    /// 2. `files.readonlyInclude` matches -> [`ReadOnlyReason::Rule`] with
    ///    [`REASON_READONLY_INCLUDE`].
    /// 3. `rove.files.extraReadonlyInclude` matches -> [`ReadOnlyReason::Rule`] with
    ///    [`REASON_EXTRA_READONLY_INCLUDE`].
    /// 4. `files.readonlyFromPermissions` and the on-disk attribute is read-only ->
    ///    [`ReadOnlyReason::Permissions`].
    ///
    /// Several rule reasons can apply at once; the caller decides how to present them.
    pub fn evaluate(&self, path: &str, permission_read_only: bool) -> Vec<ReadOnlyReason> {
        if self.exclude.is_excluded(path) {
            return Vec::new();
        }
        let mut reasons = Vec::new();
        if self.include.is_excluded(path) {
            reasons.push(ReadOnlyReason::Rule(Arc::from(REASON_READONLY_INCLUDE)));
        }
        if self.extra_include.is_excluded(path) {
            reasons.push(ReadOnlyReason::Rule(Arc::from(
                REASON_EXTRA_READONLY_INCLUDE,
            )));
        }
        if self.from_permissions && permission_read_only {
            reasons.push(ReadOnlyReason::Permissions);
        }
        reasons
    }
}

/// Immutable settings snapshot used by scanning, watching and rule queries.
#[derive(Clone, Debug)]
pub struct WorkspaceConfig {
    /// `files.exclude` merged over `rove.files.extraExclude`.
    pub exclude: RuleSet,
    /// `files.watcherExclude`, applied **before** watchers are registered.
    pub watcher_exclude: RuleSet,
    /// Read-only truth table inputs.
    pub readonly: ReadonlyRules,
    /// Start the file watcher after the initial scan. Benchmarks turn it off.
    pub watch: bool,
    /// Walker threads; `0` means one per available core.
    pub threads: usize,
    /// Pending watcher events beyond this count mean events were probably lost: the batch is
    /// dropped and the workspace re-scans (`FileEvent::RescanRequired`).
    pub event_queue_limit: usize,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self::from_settings(&default_settings()).expect("registered workspace defaults are valid")
    }
}

impl WorkspaceConfig {
    /// Reads every registered key through the settings contract.
    pub fn from_settings(settings: &dyn Settings) -> Result<Self> {
        let files_exclude = read(settings, FILES_EXCLUDE)?;
        let extra_exclude = read(settings, ROVE_EXTRA_EXCLUDE)?;
        let watcher_exclude = read(settings, FILES_WATCHER_EXCLUDE)?;
        let readonly_include = read(settings, FILES_READONLY_INCLUDE)?;
        let readonly_exclude = read(settings, FILES_READONLY_EXCLUDE)?;
        let extra_readonly_include = read(settings, ROVE_EXTRA_READONLY_INCLUDE)?;
        let from_permissions = bool::decode(&read(settings, FILES_READONLY_FROM_PERMISSIONS)?)?;
        Ok(Self {
            exclude: RuleSet::merged(&files_exclude, &extra_exclude)?,
            watcher_exclude: RuleSet::from_value(&watcher_exclude)?,
            readonly: ReadonlyRules {
                from_permissions,
                include: RuleSet::from_value(&readonly_include)?,
                exclude: RuleSet::from_value(&readonly_exclude)?,
                extra_include: RuleSet::from_value(&extra_readonly_include)?,
            },
            watch: true,
            threads: 0,
            event_queue_limit: 4096,
        })
    }

    /// Disables the watcher (benchmarks and read-only snapshots).
    pub fn without_watching(mut self) -> Self {
        self.watch = false;
        self
    }

    /// Overrides the pending-event limit; used by the overflow test and by callers with a known
    /// small event budget.
    pub fn with_event_queue_limit(mut self, limit: usize) -> Self {
        self.event_queue_limit = limit;
        self
    }
}

fn read(settings: &dyn Settings, key: &str) -> Result<Value> {
    settings.read(key, &Scope::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::Error;

    fn value(pairs: &[(&str, bool)]) -> Value {
        Value::Object(Arc::new(
            pairs
                .iter()
                .map(|(pattern, enabled)| (pattern.to_string(), Value::Bool(*enabled)))
                .collect(),
        ))
    }

    /// Builds a `DefaultSettings` with the registered defaults plus explicit overrides, which is
    /// how a user setting would look after #13 loads it.
    fn settings_with(overrides: &[(&str, Value)]) -> DefaultSettings {
        let mut registry = SettingRegistry::default();
        register_settings(&mut registry).unwrap();
        for (key, replacement) in overrides {
            let mut fresh = SettingRegistry::default();
            for (id, setting) in registry.entries() {
                let default = if id == *key {
                    replacement.clone()
                } else {
                    setting.default.clone()
                };
                fresh
                    .register(
                        id,
                        Setting {
                            default,
                            rove_default: None,
                            language_overridable: false,
                        },
                    )
                    .unwrap();
            }
            registry = fresh;
        }
        DefaultSettings(registry)
    }

    #[test]
    fn defaults_are_registered_and_read_through_the_contract() {
        let config = WorkspaceConfig::from_settings(&default_settings()).unwrap();
        // Rove extra defaults.
        assert!(config.exclude.is_excluded("Library"));
        assert!(config.exclude.is_excluded("Assets/Library"));
        assert!(config.exclude.is_excluded("Assets/obj/Debug"));
        assert!(config.exclude.is_excluded("Assets/Logs"));
        assert!(config.exclude.is_excluded("Temp"));
        assert!(config.exclude.is_excluded(".git"));
        // VS Code defaults that are not Rove extras.
        assert!(config.exclude.is_excluded("Assets/Thumbs.db"));
        assert!(!config.exclude.is_excluded("Assets/Player.cs"));
        // Watcher exclusions are a separate key.
        assert!(config.watcher_exclude.is_excluded(".git/objects/ab"));
        assert!(!config.watcher_exclude.is_excluded("Assets/Library"));
        // Read-only defaults.
        assert!(config
            .readonly
            .extra_include
            .is_excluded("Library/PackageCache/com.unity.x/Editor/A.cs"));
        assert!(!config.readonly.from_permissions);
        assert!(config.readonly.include.is_empty());
    }

    #[test]
    fn readonly_truth_table() {
        let rules = ReadonlyRules {
            from_permissions: true,
            include: RuleSet::from_value(&value(&[("**/Generated/**", true)])).unwrap(),
            exclude: RuleSet::from_value(&value(&[("**/Generated/Keep.cs", true)])).unwrap(),
            extra_include: RuleSet::from_value(&value(&[("**/Library/PackageCache/**", true)]))
                .unwrap(),
        };
        // 1. permission only
        assert_eq!(
            rules.evaluate("Assets/A.cs", true),
            vec![ReadOnlyReason::Permissions]
        );
        assert!(rules.evaluate("Assets/A.cs", false).is_empty());
        // 2. include
        assert_eq!(
            rules.evaluate("Generated/B.cs", false),
            vec![ReadOnlyReason::Rule(Arc::from(REASON_READONLY_INCLUDE))]
        );
        // 3. exclude overrides include (and the permission flag)
        assert!(rules.evaluate("Generated/Keep.cs", true).is_empty());
        // 4. extra include default
        assert_eq!(
            rules.evaluate("Library/PackageCache/A.cs", false),
            vec![ReadOnlyReason::Rule(Arc::from(
                REASON_EXTRA_READONLY_INCLUDE
            ))]
        );
        // 5. several reasons combine.
        assert_eq!(
            rules.evaluate("Library/PackageCache/A.cs", true),
            vec![
                ReadOnlyReason::Rule(Arc::from(REASON_EXTRA_READONLY_INCLUDE)),
                ReadOnlyReason::Permissions
            ]
        );
    }

    #[test]
    fn a_user_false_for_library_cancels_the_extra_default() {
        let settings = settings_with(&[(FILES_EXCLUDE, value(&[("**/Library", false)]))]);
        let config = WorkspaceConfig::from_settings(&settings).unwrap();
        assert!(!config.exclude.is_excluded("Library"));
        assert!(!config.exclude.is_excluded("Assets/Library/A.cs"));
        assert!(config.exclude.is_excluded("Temp"));
    }

    #[test]
    fn non_object_setting_values_are_invalid_input() {
        let settings = settings_with(&[(FILES_EXCLUDE, Value::Bool(true))]);
        assert_eq!(
            WorkspaceConfig::from_settings(&settings).unwrap_err(),
            Error::InvalidInput
        );
    }
}
