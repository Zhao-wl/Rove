//! 菜单登记（#16，UX_SPEC §1.2）。
//!
//! 菜单栏只有：文件、编辑、选择、查看、转到、帮助（**没有运行与终端**），另有活动栏底部的齿轮菜单。
//! 各 issue 用 [`MenuRegistry`] 登记菜单项，菜单项只引用命令 ID；
//! **命令未登记或未实现的菜单项直接不显示，不置灰**（[`visible_items`]）。
//! 这是产品内的贡献点，不是插件系统。
use rove_base::Registry;
use rove_commands::CommandRegistry;
use rove_i18n::Message;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MenuId {
    File,
    Edit,
    Selection,
    View,
    Go,
    Help,
    /// 活动栏底部的“管理”齿轮菜单（命令面板、设置、键盘快捷方式、颜色主题）。
    Manage,
}
impl MenuId {
    /// 菜单栏的顺序（齿轮菜单不在菜单栏里）。
    pub const MENU_BAR: [MenuId; 6] = [
        MenuId::File,
        MenuId::Edit,
        MenuId::Selection,
        MenuId::View,
        MenuId::Go,
        MenuId::Help,
    ];
    /// 菜单标题的字符串 key。
    pub fn title_key(self) -> &'static str {
        match self {
            MenuId::File => "menubar.file",
            MenuId::Edit => "menubar.edit",
            MenuId::Selection => "menubar.selection",
            MenuId::View => "menubar.view",
            MenuId::Go => "menubar.go",
            MenuId::Help => "menubar.help",
            MenuId::Manage => "activityBar.manage",
        }
    }
}

/// 一个菜单项。`group` 相同的项排在一起，组与组之间画分隔线（与 VS Code 的 `group` 写法相同，
/// 例如 `"1_toggle"`，按字典序排序）；组内按 `order` 排序。
#[derive(Clone, Debug)]
pub struct MenuItem {
    pub menu: MenuId,
    pub group: Arc<str>,
    pub order: i32,
    pub command: Arc<str>,
    /// None 时用命令的标题。
    pub title: Option<Message>,
}

/// 登记表：ID 一般写成 `<menu>.<command>`；重复 ID 返回 Duplicate。
pub type MenuRegistry = Registry<MenuItem>;

/// 某个菜单里实际显示的项：命令必须已登记且已实现。结果按 (group, order, id) 排序。
pub fn visible_items<'a>(
    menus: &'a MenuRegistry,
    menu: MenuId,
    commands: &CommandRegistry,
    implemented: &dyn Fn(&str) -> bool,
) -> Vec<(&'a str, &'a MenuItem)> {
    let mut items: Vec<_> = menus
        .entries()
        .filter(|(_, item)| {
            item.menu == menu && commands.get(&item.command).is_some() && implemented(&item.command)
        })
        .collect();
    items.sort_by(|a, b| {
        (a.1.group.as_ref(), a.1.order, a.0).cmp(&(b.1.group.as_ref(), b.1.order, b.0))
    });
    items
}

/// 菜单栏上实际显示的顶层菜单：没有任何可显示项的菜单整个不显示。
pub fn visible_menus(
    menus: &MenuRegistry,
    commands: &CommandRegistry,
    implemented: &dyn Fn(&str) -> bool,
) -> Vec<MenuId> {
    MenuId::MENU_BAR
        .into_iter()
        .filter(|m| !visible_items(menus, *m, commands, implemented).is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_commands::Command;

    #[test]
    fn hides_unregistered_and_unimplemented_commands() {
        let mut commands = CommandRegistry::default();
        for id in ["a.done", "a.todo"] {
            commands
                .register(
                    id,
                    Command {
                        title_key: id.into(),
                        category_key: None,
                    },
                )
                .unwrap();
        }
        let mut menus = MenuRegistry::default();
        for (id, command, group) in [
            ("view.done", "a.done", "2_b"),
            ("view.todo", "a.todo", "1_a"),
            ("view.missing", "a.missing", "1_a"),
        ] {
            menus
                .register(
                    id,
                    MenuItem {
                        menu: MenuId::View,
                        group: group.into(),
                        order: 0,
                        command: command.into(),
                        title: None,
                    },
                )
                .unwrap();
        }
        let implemented = |id: &str| id == "a.done";
        let items = visible_items(&menus, MenuId::View, &commands, &implemented);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, "view.done");
        assert_eq!(
            visible_menus(&menus, &commands, &implemented),
            vec![MenuId::View]
        );
    }
}
