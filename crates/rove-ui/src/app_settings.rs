//! 设置层装配（#104）：一张统一的设置登记表与一个共享的 `SettingsService` 实例。
//!
//! 各模块先登记自己消费的键（工作台、文本缓冲、代码视图、编辑器组、列表、工作区、语法），
//! 最后由 `rove_config::register_builtin` 补齐 `compat/SETTINGS.md` 的其余设置项。
//! 同一个 VS Code 设置项可能有多个消费方：先登记者胜出，默认值必须一致（有测试校验）。
//! 应用只创建一个服务实例，所有消费方共享同一个 `Arc<dyn Settings>`（#94 也从这里取）。

use rove_base::Result;
use rove_config::{SettingRegistry, Settings, SettingsOptions, SettingsService};
use std::{path::PathBuf, sync::Arc};

type Register = fn(&mut SettingRegistry) -> Result<()>;

/// 各模块的登记函数，按装配顺序排列。
fn modules() -> [(&'static str, Register); 8] {
    [
        ("shell", crate::shell::catalog::register_settings),
        ("rove-buffer", rove_buffer::settings::register),
        ("code_view", crate::code_view::options::register),
        ("editors", |r| {
            merge(r, &crate::editors::settings::register()?)
        }),
        (
            "list_widget",
            crate::list_widget::options::register_settings,
        ),
        ("rove-workspace", rove_workspace::register_settings),
        ("rove-syntax", rove_syntax::highlight::register_settings),
        // #49：保存路径读取的 files.*（eol、insertFinalNewline、trim*、saveConflictResolution、autoSave*）。
        ("rove-buffer-save", rove_buffer::save::settings::register),
    ]
}

/// 把 `from` 中尚未登记的键并入 `into`（先登记者胜出）。
fn merge(into: &mut SettingRegistry, from: &SettingRegistry) -> Result<()> {
    for (key, setting) in from.entries() {
        if into.get(key).is_none() {
            into.register(key, setting.clone())?;
        }
    }
    Ok(())
}

/// 应用的统一设置登记表：各模块的键 + 内置目录。
pub fn registry() -> Result<SettingRegistry> {
    let mut registry = SettingRegistry::default();
    for (_, register) in modules() {
        let mut own = SettingRegistry::default();
        register(&mut own)?;
        merge(&mut registry, &own)?;
    }
    rove_config::register_builtin(&mut registry)?;
    Ok(registry)
}

/// 打开共享 `settings.json` 的设置服务（FEATURES §13 规则 1：`--vscode-user-dir` >
/// `ROVE_VSCODE_USER_DIR` > `%APPDATA%\Code\User`）。读文件并开始监听热加载；不创建、不写入文件。
pub fn open(user_dir_arg: Option<PathBuf>) -> Result<Arc<SettingsService>> {
    let options = SettingsOptions::from_environment(user_dir_arg)?;
    Ok(Arc::new(SettingsService::open(options, registry()?)))
}

/// 只读的 VS Code 默认值（不读文件、不写、不发事件）：只给测试与没有用户目录的嵌入场景用，
/// 应用入口一律用 [`open`]。
pub fn defaults_only() -> Arc<dyn Settings> {
    Arc::new(rove_config::DefaultSettings(
        registry().expect("settings registry"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_registrations_agree_on_shared_keys() {
        let mut seen: std::collections::BTreeMap<String, (&str, rove_config::Setting)> =
            Default::default();
        for (module, register) in modules() {
            let mut own = SettingRegistry::default();
            register(&mut own).unwrap();
            for (key, setting) in own.entries() {
                if let Some((first, earlier)) = seen.get(key) {
                    assert_eq!(
                        earlier.default, setting.default,
                        "{key}: {first} 与 {module} 登记的默认值不同"
                    );
                } else {
                    seen.insert(key.to_string(), (module, setting.clone()));
                }
            }
        }
        // 模块登记的默认值与 VS Code（内置目录）一致；目录里没有的只能是模块自己的键。
        for (key, (module, setting)) in &seen {
            if let Some(entry) = rove_config::catalog::catalog().get(key) {
                assert_eq!(
                    entry.setting.default, setting.default,
                    "{key}: {module} 的默认值与 compat/SETTINGS.md 不同"
                );
            }
        }
        let registry = registry().unwrap();
        for key in ["editor.fontSize", "workbench.colorTheme", "files.exclude"] {
            assert!(registry.get(key).is_some(), "{key}");
        }
    }
}
