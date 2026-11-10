//! Temporary read-only startup adapter. #13 replaces it with the shared settings service.
use rove_base::{Cancellation, Error, Events, NoEvents, Result, Task, Value};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings, SettingsChange};
use serde_json_lenient::Value as Json;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

pub(crate) struct StartupSettings {
    defaults: DefaultSettings,
    overrides: BTreeMap<String, Value>,
}

pub(crate) fn load(registry: SettingRegistry) -> StartupSettings {
    let user_dir = std::env::var_os("ROVE_VSCODE_USER_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("APPDATA")
                .map(|appdata| PathBuf::from(appdata).join("Code").join("User"))
        });
    let mut settings = StartupSettings {
        defaults: DefaultSettings(registry),
        overrides: BTreeMap::new(),
    };
    if let Some(dir) = user_dir {
        match std::fs::read_to_string(dir.join("settings.json")) {
            Ok(text) => settings.read_overrides(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!("cannot read editor settings: {error}"),
        }
    }
    settings
}

impl StartupSettings {
    fn read_overrides(&mut self, text: &str) {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let values = match serde_json_lenient::from_str::<BTreeMap<String, Json>>(text) {
            Ok(values) => values,
            Err(error) => {
                eprintln!("cannot parse editor settings: {error}");
                return;
            }
        };
        for (key, json) in values {
            let Some(setting) = self.defaults.0.get(&key) else {
                continue;
            };
            let value = convert(json);
            if std::mem::discriminant(&value) == std::mem::discriminant(&setting.default) {
                self.overrides.insert(key, value);
            }
        }
    }
}

fn convert(json: Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Bool(value),
        Json::Number(value) => value.as_f64().map_or(Value::Null, Value::Number),
        Json::String(value) => Value::String(value.into()),
        Json::Array(values) => Value::Array(values.into_iter().map(convert).collect()),
        Json::Object(values) => Value::Object(Arc::new(
            values
                .into_iter()
                .map(|(key, value)| (key, convert(value)))
                .collect(),
        )),
    }
}

impl Settings for StartupSettings {
    fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
        if let Some(value) = self.overrides.get(key) {
            return Ok(value.clone());
        }
        self.defaults.read(key, scope)
    }

    fn write(&self, _: Arc<str>, _: Scope, _: Value, cancel: Cancellation) -> Task<()> {
        Box::pin(async move {
            cancel.check()?;
            Err(Error::Unsupported)
        })
    }

    fn changes(&self) -> &dyn Events<SettingsChange> {
        &NoEvents
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        documents::OpenOptions,
        editors::settings::{self, EditorSettings},
    };

    #[test]
    fn registered_jsonc_overrides_reach_the_editor_and_wrong_types_keep_defaults() {
        let mut settings = StartupSettings {
            defaults: DefaultSettings(settings::register().unwrap()),
            overrides: BTreeMap::new(),
        };
        settings.read_overrides(
            r#"{
            // User settings are JSONC.
            "workbench.editor.enablePreview": false,
            "workbench.editor.limit.enabled": true,
            "workbench.editor.limit.value": "invalid",
        }"#,
        );
        let editors = EditorSettings::read(&settings).unwrap();
        assert!(!editors.preview(&OpenOptions::default()));
        assert_eq!(editors.tab_limit, Some(10));
    }
}
