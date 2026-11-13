//! 欢迎页与水印的内容模型（#28）。
//!
//! 这里不碰 GPUI：绘制在 `skeleton/workbench/welcome_host.rs`。本模块只决定“显示什么”，
//! 所有文字都用字符串 key，所有快捷键都由 #14 的键位服务解析（含用户在共用 `keybindings.json`
//! 中的覆盖），**不写死**。

use super::{recent::RecentList, registry as reg};
use rove_base::Value;
use rove_config::{Key, Scope, Settings};
use std::sync::Arc;

/// 欢迎页上的一次交互。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// 「开始」里的一个按钮（执行对应命令）。
    Run(&'static str),
    /// 打开「最近」里的第 n 项。
    OpenRecent(usize),
    /// 从「最近」里移除第 n 项。
    RemoveRecent(usize),
    /// 「更多…」：打开 `workbench.action.openRecent` 的快速选择。
    ShowMore,
    /// 勾选 / 取消「启动时显示欢迎页」。
    SetShowOnStartup(bool),
}

/// 「开始」一节：(字符串 key, 命令 ID)。没有「新建文件」「克隆仓库」「连接远程」。
pub const START_TILES: &[(&str, &str)] = &[
    ("welcome.openFile", reg::OPEN_FILE),
    ("welcome.openFolder", reg::OPEN_FOLDER),
];

/// 「最近」里的一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentRow {
    pub index: usize,
    /// 完整路径（UX_SPEC §1.6）。
    pub path: String,
    pub is_folder: bool,
}

pub fn recent_rows(list: &RecentList) -> Vec<RecentRow> {
    list.entries()
        .iter()
        .enumerate()
        .map(|(index, e)| RecentRow {
            index,
            path: e.display(),
            is_folder: matches!(e.kind, super::recent::RecentKind::Folder),
        })
        .collect()
}

/// 打开了文件夹、还没有打开任何文件时的水印：(说明的字符串 key, 命令 ID)。
/// 键位文本由键位服务解析；没有绑定键位的命令整行不显示（与 VS Code 一致）。
pub const WATERMARK: &[(&str, &str)] = &[
    ("welcome.watermark.quickOpen", "workbench.action.quickOpen"),
    (
        "welcome.watermark.commandPalette",
        "workbench.action.showCommands",
    ),
    ("welcome.watermark.openFile", reg::OPEN_FILE),
    ("welcome.watermark.openFolder", reg::OPEN_FOLDER),
];

/// 水印的每一行：`说明 快捷键`。
pub fn watermark_lines(
    label: &dyn Fn(&str) -> String,
    key_label: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    WATERMARK
        .iter()
        .filter_map(|(text, command)| {
            let keys = key_label(command)?;
            Some(format!("{}  {keys}", label(text)))
        })
        .collect()
}

/// 「启动时显示欢迎页」当前是否勾选（`workbench.startupEditor`，VS Code 默认 `welcomePage`）。
pub fn show_on_startup(settings: &dyn Settings) -> bool {
    Key::<String>::new(reg::STARTUP_EDITOR)
        .read(settings, &Scope::default())
        .map(|v| v == reg::STARTUP_WELCOME)
        .unwrap_or(true)
}

/// 勾选状态对应要写进 `settings.json` 的值。只改这一个键（#13 的保留格式写入）。
pub fn startup_value(checked: bool) -> Value {
    Value::String(Arc::from(if checked {
        reg::STARTUP_WELCOME
    } else {
        reg::STARTUP_NONE
    }))
}

/// 启动时是否应该打开欢迎页。没有打开文件夹的空窗口**总是**显示欢迎页（UX_SPEC §6.3）。
pub fn open_at_startup(settings: &dyn Settings, has_folder: bool) -> bool {
    !has_folder || show_on_startup(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::welcome::recent::RecentKind;
    use rove_config::{DefaultSettings, Setting, SettingRegistry};

    fn settings(value: Option<&str>) -> DefaultSettings {
        let mut registry = SettingRegistry::default();
        registry
            .register(
                reg::STARTUP_EDITOR,
                Setting {
                    default: Value::String(Arc::from(reg::STARTUP_WELCOME)),
                    rove_default: value.map(|v| Value::String(Arc::from(v))),
                    language_overridable: false,
                },
            )
            .unwrap();
        DefaultSettings(registry)
    }

    #[test]
    fn start_section_has_only_open_file_and_folder() {
        assert_eq!(START_TILES.len(), 2);
        assert!(START_TILES
            .iter()
            .all(|(_, c)| c.starts_with("workbench.action.files.open")));
    }

    #[test]
    fn recent_rows_show_full_paths_in_order() {
        let mut list = RecentList::default();
        list.add(r"D:\Game", RecentKind::Folder);
        list.add(r"D:\Game\Player.cs", RecentKind::File);
        let rows = recent_rows(&list);
        assert_eq!(rows[0].path, r"D:\Game\Player.cs");
        assert!(!rows[0].is_folder);
        assert_eq!(rows[1].index, 1);
        assert!(rows[1].is_folder);
    }

    #[test]
    fn watermark_uses_resolved_keys_and_skips_unbound_commands() {
        let label = |k: &str| format!("<{k}>");
        let keys = |c: &str| match c {
            "workbench.action.quickOpen" => Some("Ctrl+P".to_string()),
            _ => None,
        };
        let lines = watermark_lines(&label, &keys);
        assert_eq!(lines, vec!["<welcome.watermark.quickOpen>  Ctrl+P"]);
        // 用户在共用 keybindings.json 里改了键位：水印随之变化。
        let keys = |c: &str| (c == "workbench.action.quickOpen").then(|| "Ctrl+Alt+O".to_string());
        assert!(watermark_lines(&label, &keys)[0].ends_with("Ctrl+Alt+O"));
    }

    #[test]
    fn startup_checkbox_reflects_and_writes_only_the_startup_editor_key() {
        assert!(show_on_startup(&settings(None)));
        assert!(!show_on_startup(&settings(Some("none"))));
        assert_eq!(startup_value(false), Value::String(Arc::from("none")));
        assert_eq!(startup_value(true), Value::String(Arc::from("welcomePage")));
        // 空窗口始终显示欢迎页，即使取消了勾选。
        assert!(open_at_startup(&settings(Some("none")), false));
        assert!(!open_at_startup(&settings(Some("none")), true));
        assert!(open_at_startup(&settings(None), true));
    }
}
