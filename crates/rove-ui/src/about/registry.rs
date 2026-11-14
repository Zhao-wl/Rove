//! 「关于」对话框的登记项（#76 第 4 节）：命令、界面字符串、齿轮菜单项与 GPUI action。
//!
//! 命令 ID 沿用 VS Code 的 `workbench.action.showAboutDialog`（`KEYBINDINGS.md` §7 附录
//! 「P0 · 窗口与设置」，VS Code 也没有给它默认键位，所以这里不登记键位，只登记命令与菜单项）。
//! 颜色键复用 #16 工作台已登记的编辑器 / 按钮颜色，不新增颜色键。

use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use std::sync::Arc;

use crate::shell::menus::{MenuId, MenuItem, MenuRegistry};

pub const SHOW_ABOUT: &str = "workbench.action.showAboutDialog";

/// 对话框里的按钮 ID（也是测试与宿主分发用的 action 名）。
pub const ACTION_COPY: &str = "copy";
pub const ACTION_NOTICES: &str = "notices";
pub const ACTION_CLOSE: &str = "close";

/// (命令 ID, 标题 key, 中文标题)
pub const COMMANDS: &[(&str, &str, &str)] = &[(SHOW_ABOUT, "about.command.show", "关于 Rove")];

pub const STRINGS: &[(&str, &str)] = &[
    ("about.title", "关于 Rove"),
    ("about.product", "Rove"),
    ("about.tagline", "高性能代码阅读编辑器"),
    ("about.label.version", "版本"),
    ("about.label.commit", "提交"),
    ("about.label.date", "构建日期"),
    ("about.label.channel", "通道"),
    ("about.label.gpui", "gpui"),
    ("about.label.indexer", "C# 索引组件"),
    ("about.label.executable", "可执行文件"),
    ("about.indexer.installed", "已安装 {version}"),
    (
        "about.indexer.installedUnknown",
        "已安装（开发覆盖，无版本清单）",
    ),
    (
        "about.indexer.notInstalled",
        "未安装（C# 导航降级为声明索引）",
    ),
    (
        "about.indexer.componentTooOld",
        "不兼容 {version}：组件比 Rove 旧，请更新组件",
    ),
    (
        "about.indexer.componentTooNew",
        "不兼容 {version}：组件比 Rove 新，请更新 Rove",
    ),
    ("about.unknown", "未知"),
    ("about.copy", "复制"),
    ("about.copied", "已复制版本信息"),
    ("about.notices", "第三方声明"),
    (
        "about.noticesMissing",
        "找不到随包的 THIRD_PARTY_NOTICES.md。",
    ),
    ("about.close", "关闭"),
];

/// 浮层用到的颜色键（全是 `THEME_COLORS` 里已有的键，**不新造**；默认值取 #10 的 VS Code 颜色表）。
pub const COLORS: &[&str] = &[
    "editorWidget.background",
    "editorWidget.foreground",
    "editorWidget.border",
    "descriptionForeground",
    "button.background",
    "button.foreground",
];

const MENU_ITEMS: &[(&str, i32, &str, &str)] = &[("3_help", 10, SHOW_ABOUT, "about.menu.about")];

const MENU_STRINGS: &[(&str, &str)] = &[("about.menu.about", "关于")];

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

/// 齿轮菜单（活动栏底部）里的「关于」。
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
pub fn register_colors(registry: &mut rove_theme::ColorRegistry) -> Result<()> {
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

/// 本模块命令的 GPUI action；窗口（宿主）处理，键位服务按 when 解析后分发。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = about, no_json)]
pub struct AboutAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(AboutAction(cmd)) as Box<dyn Action>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::{Message, Strings};

    #[test]
    fn command_strings_and_menu_all_register() {
        let mut commands = CommandRegistry::default();
        register_commands(&mut commands).unwrap();
        // 重复登记不报错（其他模块可能先登记了同一个 VS Code ID）。
        register_commands(&mut commands).unwrap();
        assert!(commands.get(SHOW_ABOUT).is_some());

        let mut strings = StringTable::default();
        register_strings(&mut strings).unwrap();
        register_strings(&mut strings).unwrap();
        for (_, command) in commands.entries() {
            Strings::text(&strings, &Message::key(command.title_key.clone())).unwrap();
        }

        let mut menus = MenuRegistry::default();
        register_menus(&mut menus).unwrap();
        register_menus(&mut menus).unwrap();
        for (_, item) in menus.entries() {
            assert_eq!(item.command.as_ref(), SHOW_ABOUT);
            Strings::text(&strings, item.title.as_ref().unwrap()).unwrap();
        }
        let mut colors = rove_theme::ColorRegistry::default();
        register_colors(&mut colors).unwrap();
        register_colors(&mut colors).unwrap();
        for key in COLORS {
            assert!(colors.get(key).is_some(), "{key} 没有默认值");
        }
        assert!(action_for(SHOW_ABOUT).is_some());
        assert!(action_for("workbench.action.quit").is_none());
    }
}
