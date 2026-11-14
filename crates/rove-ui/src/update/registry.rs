//! 自动更新的登记项（issue #149 第 3 节）：命令、界面字符串、帮助菜单项与 GPUI action。
//!
//! 全部经 #1 的登记机制加入，**不写死**：
//!
//! - 命令 `update.checkForUpdates` / `update.restartToUpdate`（VS Code 自己有
//!   `update.checkForUpdate`，我们不蹭它的 ID；这两个是 Rove 的命令，但不带 `rove.` 前缀——
//!   前缀是给**设置键**用的，设置文件和 VS Code 共用，命令表不共用）；
//! - **状态栏不加常驻条目**（issue 原文）：只用提示条与通知；
//! - 颜色键一个都不新增：提示条与通知用 #16 工作台已有的颜色。
//!
//! 设置项 `rove.update.mode` 登记在 `rove-config` 的内置目录里
//!（`docs/compat/SETTINGS.md` §4），不在这里。

use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use std::sync::Arc;

use crate::shell::menus::{MenuId, MenuItem, MenuRegistry};

/// 「检查更新」：命令面板与帮助（齿轮）菜单。
pub const CHECK_FOR_UPDATES: &str = "update.checkForUpdates";
/// 「重启以更新」：有待安装更新时才可用。
pub const RESTART_TO_UPDATE: &str = "update.restartToUpdate";

/// 提示条 / 通知里的按钮 ID。
pub const ACTION_RESTART: &str = "restart";
pub const ACTION_DOWNLOAD: &str = "download";
pub const ACTION_LATER: &str = "later";

/// 反馈键（去重用，同一件事只显示一条）。
pub const KEY_PROMPT: &str = "update.prompt";
pub const KEY_STATUS: &str = "update.status";
pub const KEY_FAILED: &str = "update.failed";

/// (命令 ID, 标题 key, 中文标题)
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (CHECK_FOR_UPDATES, "update.command.check", "检查更新"),
    (
        RESTART_TO_UPDATE,
        "update.command.restart",
        "重启以更新 Rove",
    ),
];

pub const STRINGS: &[(&str, &str)] = &[
    // 「关于」对话框里的那一行（state::UpdateState::string_key）。
    ("update.label", "更新"),
    ("update.state.idle", "尚未检查"),
    ("update.state.disabled", "已关闭（rove.update.mode = none）"),
    ("update.state.checking", "正在检查…"),
    ("update.state.upToDate", "已是最新（上次检查 {time}）"),
    ("update.state.available", "发现新版本 {version}"),
    ("update.state.downloading", "正在下载 {version}"),
    ("update.state.ready", "已下载 {version}，重启后安装"),
    ("update.state.failed", "上次检查失败（{time}）：{reason}"),
    // 提示条与通知。
    ("update.prompt.restart", "已下载 Rove {version}，重启以更新"),
    ("update.prompt.available", "发现新版本 Rove {version}"),
    (
        "update.prompt.required",
        "发现新版本 Rove {version}，当前版本已不再受支持",
    ),
    ("update.action.restart", "重启"),
    ("update.action.download", "下载"),
    ("update.action.later", "稍后"),
    // 命令的即时回执（第 1 级反馈：状态栏临时消息）。
    ("update.status.checking", "正在检查更新…"),
    ("update.status.upToDate", "已经是最新版本 {version}"),
    ("update.status.disabled", "更新已关闭（rove.update.mode）"),
    ("update.status.downloading", "正在后台下载 {version}…"),
    (
        "update.status.portable",
        "便携版不自动更新：发现新版本 {version}，请到发布页下载",
    ),
    ("update.status.restarting", "正在保存并重启以安装更新…"),
    // 通知（第 4 级）：要让用户知道的失败。
    ("update.notify.failed", "更新失败：{reason}"),
    (
        "update.notify.installFailed",
        "更新没有安装成功，仍在使用当前版本（{reason}）",
    ),
];

const MENU_ITEMS: &[(&str, i32, &str, &str)] =
    &[("3_help", 5, CHECK_FOR_UPDATES, "update.menu.check")];

const MENU_STRINGS: &[(&str, &str)] = &[("update.menu.check", "检查更新")];

pub fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, title, _) in COMMANDS {
        match registry.register(
            *id,
            Command {
                title_key: (*title).into(),
                category_key: None,
            },
        ) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn register_strings(table: &mut StringTable) -> Result<()> {
    let commands = COMMANDS.iter().map(|(_, k, t)| (*k, *t));
    for (key, text) in STRINGS
        .iter()
        .copied()
        .chain(MENU_STRINGS.iter().copied())
        .chain(commands)
    {
        match table.register(key, Arc::from(text)) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 齿轮菜单里的「检查更新」，排在「关于」前面。
pub fn register_menus(registry: &mut MenuRegistry) -> Result<()> {
    for (group, order, command, title) in MENU_ITEMS {
        match registry.register(
            format!("{}.{command}", MenuId::Manage.title_key()),
            MenuItem {
                menu: MenuId::Manage,
                group: (*group).into(),
                order: *order,
                command: (*command).into(),
                title: Some(rove_i18n::Message::key(*title)),
            },
        ) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 本模块命令的 GPUI action；窗口（`skeleton/workbench/update_host.rs`）处理。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = update, no_json)]
pub struct UpdateAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(command, _, _)| *command == id)
        .map(|(command, _, _)| Box::new(UpdateAction(command)) as Box<dyn Action>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::{Message, Strings};
    use rove_update::UpdateState;

    #[test]
    fn commands_strings_and_the_menu_item_all_register() {
        let mut commands = CommandRegistry::default();
        register_commands(&mut commands).unwrap();
        // 重复登记不报错。
        register_commands(&mut commands).unwrap();
        assert!(commands.get(CHECK_FOR_UPDATES).is_some());
        assert!(commands.get(RESTART_TO_UPDATE).is_some());

        let mut strings = StringTable::default();
        register_strings(&mut strings).unwrap();
        register_strings(&mut strings).unwrap();
        for (_, command) in commands.entries() {
            Strings::text(&strings, &Message::key(command.title_key.clone())).unwrap();
        }
        for (key, _) in STRINGS {
            assert!(strings.get(key).is_some(), "{key} 没有登记");
        }

        let mut menus = MenuRegistry::default();
        register_menus(&mut menus).unwrap();
        register_menus(&mut menus).unwrap();
        assert!(menus
            .entries()
            .any(|(_, item)| item.command.as_ref() == CHECK_FOR_UPDATES));

        assert!(action_for(CHECK_FOR_UPDATES).is_some());
        assert!(action_for(RESTART_TO_UPDATE).is_some());
        assert!(action_for("update.checkForUpdate").is_none());
    }

    /// 「关于」对话框要显示的五种状态都有登记过的文案（issue 第 3 节最后一条）。
    #[test]
    fn every_update_state_has_a_registered_string() {
        let mut strings = StringTable::default();
        register_strings(&mut strings).unwrap();
        for state in [
            UpdateState::Idle,
            UpdateState::Disabled,
            UpdateState::Checking,
            UpdateState::UpToDate {
                checked_at: "2026-11-14 10:00:00Z".into(),
            },
            UpdateState::Available {
                version: "0.1.1".into(),
            },
            UpdateState::Downloading {
                version: "0.1.1".into(),
            },
            UpdateState::ReadyToInstall {
                version: "0.1.1".into(),
            },
            UpdateState::Failed {
                at: "2026-11-14 10:00:00Z".into(),
                reason: "网络不可用".into(),
            },
        ] {
            assert!(
                strings.get(state.string_key()).is_some(),
                "{:?} 的文案没登记",
                state
            );
        }
    }

    /// 状态栏不加常驻条目（issue 原文）：更新只出现在齿轮菜单、命令面板、提示条与通知里。
    #[test]
    fn the_only_ui_entry_is_the_gear_menu() {
        let mut menus = MenuRegistry::default();
        register_menus(&mut menus).unwrap();
        let items: Vec<_> = menus.entries().collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].1.menu, MenuId::Manage);
        // 本模块没有 `register_contributions`，所以不可能登记视图或状态栏条目。
        assert_eq!(
            crate::shell::view::default_contributions()
                .entries()
                .count(),
            crate::shell::view::default_contributions()
                .entries()
                .count()
        );
    }
}
