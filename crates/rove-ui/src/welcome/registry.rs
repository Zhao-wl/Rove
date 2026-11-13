//! 欢迎页与首选项入口的登记项（#28）：命令（VS Code ID）、界面字符串、颜色键、齿轮菜单项与 GPUI action。
//!
//! - 设置项 `workbench.startupEditor`、`window.openFilesInNewWindow`、`window.openFoldersInNewWindow`、
//!   `workbench.colorTheme` 都在 `rove_config` 的内置目录里（`compat/SETTINGS.md`），这里不重复登记；
//! - 默认键位来自 #14 生成的 VS Code 默认键位表（`ctrl+o`、`ctrl+k ctrl+o`、`ctrl+r`、
//!   `ctrl+oem_comma`、`ctrl+k ctrl+s`、`ctrl+k ctrl+t`），这里不重复登记；
//!   `workbench.action.showWelcomePage` 在 VS Code 里也没有默认键位（KEYBINDINGS.md §2：沿用 VS Code 的命令 ID）；
//! - **不登记**：演练（`workbench.action.openWalkthrough`）、`welcome.showNewFileEntries`、
//!   图形化设置界面（`workbench.action.openSettings2`，见 R70）、新建文件 / 克隆仓库 / 连接远程。

use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use rove_theme::ColorRegistry;
use std::sync::Arc;

use crate::shell::menus::{MenuId, MenuItem, MenuRegistry};

pub const OPEN_FILE: &str = "workbench.action.files.openFile";
pub const OPEN_FOLDER: &str = "workbench.action.files.openFolder";
pub const OPEN_RECENT: &str = "workbench.action.openRecent";
pub const OPEN_SETTINGS: &str = "workbench.action.openSettings";
pub const OPEN_KEYBINDINGS: &str = "workbench.action.openGlobalKeybindings";
pub const SELECT_THEME: &str = "workbench.action.selectTheme";
pub const SHOW_WELCOME: &str = "workbench.action.showWelcomePage";

/// 欢迎页的编辑器类型 ID 与它的资源名（虚拟文档，没有磁盘文件）。
pub const EDITOR_TYPE: &str = "rove.welcomePage";
pub const RESOURCE: &str = "rove-welcome";

/// `workbench.startupEditor`（VS Code 默认 `welcomePage`）。
pub const STARTUP_EDITOR: &str = "workbench.startupEditor";
pub const STARTUP_WELCOME: &str = "welcomePage";
pub const STARTUP_NONE: &str = "none";

/// 共用文件提示条的去重键（也是“不再提示”的持久化键，存在 Rove 自己的状态目录里）。
pub const SHARED_FILE_BAR: &str = "welcome.sharedFile";

/// (命令 ID, 标题字符串 key, 中文标题)
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (OPEN_FILE, "welcome.command.openFile", "打开文件..."),
    (OPEN_FOLDER, "welcome.command.openFolder", "打开文件夹..."),
    (OPEN_RECENT, "welcome.command.openRecent", "打开最近的文件"),
    (
        OPEN_SETTINGS,
        "welcome.command.openSettings",
        "打开用户设置(JSON)",
    ),
    (
        OPEN_KEYBINDINGS,
        "welcome.command.openKeybindings",
        "打开键盘快捷方式(JSON)",
    ),
    (SELECT_THEME, "welcome.command.selectTheme", "颜色主题"),
    (SHOW_WELCOME, "welcome.command.showWelcome", "欢迎使用"),
];

/// 界面字符串（全部走登记机制，不写死）。
pub const STRINGS: &[(&str, &str)] = &[
    ("welcome.tabTitle", "欢迎使用"),
    ("welcome.noFolderTitle", "无打开的文件夹"),
    ("welcome.heading", "Rove"),
    ("welcome.subtitle", "高性能代码阅读编辑器"),
    ("welcome.section.start", "开始"),
    ("welcome.section.recent", "最近"),
    ("welcome.openFile", "打开文件"),
    ("welcome.openFolder", "打开文件夹"),
    ("welcome.noRecent", "你还没有打开过文件夹。"),
    ("welcome.more", "更多…"),
    ("welcome.remove", "从列表中移除"),
    ("welcome.showOnStartup", "启动时显示欢迎页"),
    (
        "welcome.sharedFile",
        "此文件与 VS Code 共用。Rove 专有的设置放在 `rove.*` 下。",
    ),
    ("welcome.theme.placeholder", "选择颜色主题（上下键预览）"),
    ("welcome.recent.placeholder", "选择要打开的文件夹或文件"),
    ("welcome.recent.empty", "没有最近打开的记录。"),
    ("welcome.openFailed", "无法打开：{path}"),
    ("welcome.watermark.quickOpen", "快速打开"),
    ("welcome.watermark.commandPalette", "命令面板"),
    ("welcome.watermark.openFile", "打开文件"),
    ("welcome.watermark.openFolder", "打开文件夹"),
];

/// 齿轮菜单（活动栏底部）的四项：命令面板由 #16 登记，这里补另外三项。
const MENU_ITEMS: &[(&str, i32, &str, &str)] = &[
    (
        "2_configuration",
        10,
        OPEN_SETTINGS,
        "welcome.menu.settings",
    ),
    (
        "2_configuration",
        20,
        OPEN_KEYBINDINGS,
        "welcome.menu.keybindings",
    ),
    ("2_configuration", 30, SELECT_THEME, "welcome.menu.theme"),
];

const MENU_STRINGS: &[(&str, &str)] = &[
    ("welcome.menu.settings", "设置"),
    ("welcome.menu.keybindings", "键盘快捷方式"),
    ("welcome.menu.theme", "颜色主题"),
];

/// 欢迎页用到的颜色键（THEME_COLORS.md 的 `welcomePage.*`）。只用这三个。
pub const COLORS: &[&str] = &[
    "welcomePage.background",
    "welcomePage.tileBackground",
    "welcomePage.tileHoverBackground",
];

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

/// 颜色键：默认值取 #10 生成的 VS Code 颜色登记表，用户主题可以覆盖。
pub fn register_colors(registry: &mut ColorRegistry) -> Result<()> {
    for key in COLORS {
        if registry.get(key).is_some() {
            continue;
        }
        if let Some(defaults) = rove_theme::defaults::static_defaults(key) {
            registry.register(*key, defaults)?;
        }
    }
    Ok(())
}

/// 本模块命令的 GPUI action；由窗口（宿主）处理，键位服务按 when 解析后分发。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = welcome, no_json)]
pub struct WelcomeAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(WelcomeAction(cmd)) as Box<dyn Action>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::Message;

    #[test]
    fn commands_strings_menus_and_colors_all_register() {
        let mut commands = CommandRegistry::default();
        register_commands(&mut commands).unwrap();
        // 重复登记不报错（其他模块可能先登记了同一个 VS Code ID）。
        register_commands(&mut commands).unwrap();
        let mut strings = StringTable::default();
        register_strings(&mut strings).unwrap();
        let mut menus = MenuRegistry::default();
        register_menus(&mut menus).unwrap();
        register_menus(&mut menus).unwrap();
        // 每个命令都有标题字符串，每个菜单项都指向已登记的命令与标题。
        for (_, command) in commands.entries() {
            rove_i18n::Strings::text(&strings, &Message::key(command.title_key.clone())).unwrap();
        }
        for (_, item) in menus.entries() {
            assert!(commands.get(&item.command).is_some(), "{}", item.command);
            rove_i18n::Strings::text(&strings, item.title.as_ref().unwrap()).unwrap();
            assert_eq!(item.menu, MenuId::Manage);
        }
        let mut colors = ColorRegistry::default();
        register_colors(&mut colors).unwrap();
        for key in COLORS {
            assert!(colors.get(key).is_some(), "{key}");
        }
        // 不登记演练与图形化设置界面。
        for id in [
            "workbench.action.openWalkthrough",
            "welcome.showNewFileEntries",
            "workbench.action.openSettings2",
        ] {
            assert!(commands.get(id).is_none(), "{id}");
        }
    }

    #[test]
    fn every_command_has_an_action() {
        for (id, _, _) in COMMANDS {
            assert!(action_for(id).is_some(), "{id}");
        }
        assert!(action_for("workbench.action.openWalkthrough").is_none());
    }
}
