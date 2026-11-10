use rove_base::{Result, Value};
use rove_config::{DefaultSettings, Key, Scope, Setting, SettingRegistry, Settings};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, LazyLock},
};

const ASSOCIATIONS: &str = "files.associations";
/// VS Code's language-scoped default (`"[csharp]"` in default settings).
const CSHARP_DEFAULTS: &str = "[csharp]";
const LINE_LENGTH: Key<f64> = Key::new("editor.maxTokenizationLineLength");
const LARGE_FILE: Key<f64> = Key::new("rove.largeFile.threshold");
const DEFAULT_LINE_LENGTH: usize = 20_000;
const CSHARP_LINE_LENGTH: usize = 2_500;
const DEFAULT_LARGE_FILE: u64 = 64 * 1024 * 1024;

pub(super) static DEFAULTS: LazyLock<DefaultSettings> = LazyLock::new(|| {
    let mut registry = SettingRegistry::default();
    register_settings(&mut registry).expect("unique syntax settings");
    DefaultSettings(registry)
});

/// Register the settings consumed by syntax and language identification.
pub fn register_settings(registry: &mut SettingRegistry) -> Result<()> {
    for (id, default, language_overridable) in [
        (ASSOCIATIONS, Value::Object(Default::default()), false),
        (
            LINE_LENGTH.id,
            Value::Number(DEFAULT_LINE_LENGTH as f64),
            true,
        ),
        (
            LARGE_FILE.id,
            Value::Number(DEFAULT_LARGE_FILE as f64),
            false,
        ),
        (
            CSHARP_DEFAULTS,
            Value::Object(Arc::new(BTreeMap::from([(
                LINE_LENGTH.id.to_owned(),
                Value::Number(CSHARP_LINE_LENGTH as f64),
            )]))),
            false,
        ),
    ] {
        registry.register(
            id,
            Setting {
                default,
                rove_default: None,
                language_overridable,
            },
        )?;
    }
    Ok(())
}

pub(super) fn limits(settings: Option<&dyn Settings>, language: Arc<str>) -> (u64, usize) {
    let settings = settings.unwrap_or(&*DEFAULTS);
    let fallback = if language.as_ref() == "csharp" {
        CSHARP_LINE_LENGTH
    } else {
        DEFAULT_LINE_LENGTH
    };
    let scope = Scope {
        language: Some(language),
    };
    let read = |key: Key<f64>, scope: &Scope| {
        key.read(settings, scope)
            .ok()
            .filter(|n| *n >= 0.0)
            .map(|n| n as u64)
    };
    (
        read(LARGE_FILE, &Scope::default()).unwrap_or(DEFAULT_LARGE_FILE),
        read(LINE_LENGTH, &scope).map_or(fallback, |limit| limit as usize),
    )
}

/// Use configured file associations before built-in extensions. Path globs match the
/// slash-separated path, while patterns without a slash match just the basename.
pub fn language_for_path(path: &Path, settings: Option<&dyn Settings>) -> Arc<str> {
    if let Ok(Value::Object(patterns)) = settings
        .unwrap_or(&*DEFAULTS)
        .read(ASSOCIATIONS, &Scope::default())
    {
        let full = path.to_string_lossy().replace('\\', "/");
        let basename = full.rsplit('/').next().unwrap_or_default();
        let custom = patterns
            .iter()
            .filter_map(|(pattern, value)| {
                let Value::String(id) = value else {
                    return None;
                };
                let glob = globset::GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .case_insensitive(cfg!(windows))
                    .build()
                    .ok()?;
                let candidate = if pattern.contains('/') {
                    &full
                } else {
                    basename
                };
                glob.compile_matcher()
                    .is_match(candidate)
                    .then_some((pattern.len(), id))
            })
            .max_by_key(|(len, _)| *len);
        if let Some((_, id)) = custom {
            return id.clone();
        }
    }

    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "cs" => "csharp",
        "json" | "asmdef" | "asmref" => "json",
        "jsonc" => "jsonc",
        "yaml" | "yml" | "meta" | "prefab" | "unity" | "asset" => "yaml",
        "xml" => "xml",
        "md" | "markdown" => "markdown",
        _ => "plaintext",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_defaults_keep_the_csharp_line_limit() {
        let settings: &dyn Settings = &*DEFAULTS;
        assert_eq!(
            limits(Some(settings), "csharp".into()).1,
            CSHARP_LINE_LENGTH
        );
        assert_eq!(limits(None, "csharp".into()).1, CSHARP_LINE_LENGTH);
        assert_eq!(limits(Some(settings), "json".into()).1, DEFAULT_LINE_LENGTH);
    }
}
