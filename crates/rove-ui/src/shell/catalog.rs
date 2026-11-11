//! 工作台的全部登记项（#16）：命令、默认键位、设置项、颜色键、界面字符串、菜单项、视图与状态栏条目。
//!
//! 一律通过 #1 的登记机制加入，不写死在界面代码里；界面只引用这里登记的 ID / key。
//! 默认键位不在这里：它们来自 #14 生成的 VS Code 默认键位表，由键位服务（`crate::keys`）统一解析，
//! 用户的 keybindings.json 可以覆盖。工作台只在焦点元素上发布 VS Code 上下文键（见下）。
use super::menus::{MenuId, MenuItem, MenuRegistry};
use crate::contributions::{Contribution, ContributionRegistry, Placement};
use rove_base::{Result, Value};
use rove_commands::{Command, CommandRegistry};
use rove_config::{Setting, SettingRegistry};
use rove_i18n::{Message, StringTable};
use rove_theme::{Color, ColorDefaults, ColorRegistry};
use std::sync::Arc;

// ------------------------------------------------------------------ 视图与状态栏

pub const EXPLORER: &str = "workbench.view.explorer";
pub const SEARCH: &str = "workbench.view.search";
pub const FILE_VIEW: &str = "workbench.explorer.fileView";
pub const PROBLEMS: &str = "workbench.panel.markers";
pub const STATUS_PROBLEMS: &str = "status.problems";
pub const STATUS_NOTIFICATIONS: &str = "status.notifications";
/// 视图容器由工作台通用工厂绘制（标题 + 可折叠小节）。
pub const CONTAINER_FACTORY: &str = "workbench.viewContainer";
/// 状态栏条目的内容由 `StatusBar::set` 提供，绘制用通用工厂。
pub const STATUS_FACTORY: &str = "workbench.statusItem";

/// 工作台自带的贡献：两个视图容器、资源管理器的“文件夹”小节、问题面板、问题计数与通知铃铛。
/// 大纲（#26）、上下文篮子（R65）等由各自 issue 追加登记。
pub fn register_contributions(r: &mut ContributionRegistry) -> Result<()> {
    let entries = [
        (
            EXPLORER,
            "view.explorer",
            Placement::ActivityBar,
            0,
            Some("files"),
            Some("workbench.view.explorer"),
            CONTAINER_FACTORY,
        ),
        (
            SEARCH,
            "view.search",
            Placement::ActivityBar,
            10,
            Some("search"),
            Some("workbench.view.search"),
            CONTAINER_FACTORY,
        ),
        // 资源管理器的小节顺序：打开的编辑器（P2）0、文件夹 10、大纲 20（UX_SPEC §1.3）。
        (
            FILE_VIEW,
            "view.folders",
            Placement::Sidebar {
                container: EXPLORER.into(),
            },
            10,
            None,
            None,
            FILE_VIEW,
        ),
        (
            PROBLEMS,
            "view.problems",
            Placement::Panel,
            0,
            Some("warning"),
            Some("workbench.actions.view.problems"),
            PROBLEMS,
        ),
        (
            STATUS_PROBLEMS,
            "status.problems.name",
            Placement::StatusLeft,
            50,
            None,
            Some("workbench.actions.view.toggleProblems"),
            STATUS_FACTORY,
        ),
        // 通知铃铛固定在最右侧。
        (
            STATUS_NOTIFICATIONS,
            "status.notifications.name",
            Placement::StatusRight,
            i32::MIN,
            None,
            Some("notifications.toggleList"),
            STATUS_FACTORY,
        ),
    ];
    for (id, title, placement, order, icon, command, factory) in entries {
        r.register(
            id,
            Contribution {
                title: Message::key(title),
                placement,
                order,
                icon: icon.map(Arc::from),
                command: command.map(Arc::from),
                when: None,
                factory_key: factory.into(),
            },
        )?;
    }
    Ok(())
}

// ------------------------------------------------------------------ 命令与默认键位

/// (命令 ID, 标题字符串 key 对应的中文, 分类)
const COMMANDS: &[(&str, &str, Option<&str>)] = &[
    (
        "workbench.action.toggleSidebarVisibility",
        "切换主侧边栏可见性",
        Some("view"),
    ),
    (
        "workbench.action.focusSideBar",
        "聚焦到主侧边栏",
        Some("view"),
    ),
    (
        "workbench.action.toggleSidebarPosition",
        "切换主侧边栏位置",
        Some("view"),
    ),
    (
        "workbench.action.togglePanel",
        "切换面板可见性",
        Some("view"),
    ),
    ("workbench.action.closePanel", "隐藏面板", Some("view")),
    ("workbench.action.focusPanel", "聚焦到面板", Some("view")),
    ("workbench.actions.view.problems", "问题", Some("view")),
    (
        "workbench.actions.view.toggleProblems",
        "切换问题(错误、警告、信息)",
        Some("view"),
    ),
    (
        "workbench.action.focusNextPart",
        "聚焦下一部分",
        Some("view"),
    ),
    (
        "workbench.action.focusPreviousPart",
        "聚焦上一部分",
        Some("view"),
    ),
    (
        "workbench.action.focusActivityBar",
        "聚焦到活动栏",
        Some("view"),
    ),
    (
        "workbench.action.focusStatusBar",
        "聚焦到状态栏",
        Some("view"),
    ),
    (
        "workbench.action.toggleStatusbarVisibility",
        "切换状态栏可见性",
        Some("view"),
    ),
    (
        "workbench.statusBar.focusFirst",
        "聚焦第一个状态栏条目",
        Some("view"),
    ),
    (
        "workbench.statusBar.focusLast",
        "聚焦最后一个状态栏条目",
        Some("view"),
    ),
    (
        "workbench.statusBar.focusNext",
        "聚焦下一个状态栏条目",
        Some("view"),
    ),
    (
        "workbench.statusBar.focusPrevious",
        "聚焦上一个状态栏条目",
        Some("view"),
    ),
    (
        "workbench.statusBar.clearFocus",
        "清除状态栏焦点",
        Some("view"),
    ),
    ("workbench.view.explorer", "显示资源管理器", Some("view")),
    ("workbench.view.search", "显示搜索", Some("view")),
    ("workbench.action.findInFiles", "在文件中查找", Some("view")),
    ("workbench.action.zoomIn", "放大", Some("view")),
    ("workbench.action.zoomOut", "缩小", Some("view")),
    ("workbench.action.zoomReset", "重置缩放", Some("view")),
    ("workbench.action.closeWindow", "关闭窗口", None),
    ("notifications.showList", "显示通知", Some("notifications")),
    ("notifications.hideList", "隐藏通知", Some("notifications")),
    (
        "notifications.toggleList",
        "切换通知中心",
        Some("notifications"),
    ),
    (
        "notifications.hideToasts",
        "隐藏通知 Toast",
        Some("notifications"),
    ),
    (
        "notifications.clearAll",
        "清除所有通知",
        Some("notifications"),
    ),
    (
        "notifications.toggleDoNotDisturbMode",
        "切换勿扰模式",
        Some("notifications"),
    ),
    (
        "notifications.focusToasts",
        "聚焦通知 Toast",
        Some("notifications"),
    ),
    (
        "notifications.focusFirstToast",
        "聚焦第一个通知 Toast",
        Some("notifications"),
    ),
    (
        "notifications.focusLastToast",
        "聚焦最后一个通知 Toast",
        Some("notifications"),
    ),
    (
        "notifications.focusNextToast",
        "聚焦下一个通知 Toast",
        Some("notifications"),
    ),
    (
        "notifications.focusPreviousToast",
        "聚焦上一个通知 Toast",
        Some("notifications"),
    ),
    ("notification.clear", "清除通知", Some("notifications")),
    ("notification.expand", "展开通知", Some("notifications")),
    ("notification.collapse", "折叠通知", Some("notifications")),
    ("notification.toggle", "切换通知", Some("notifications")),
    (
        "notification.acceptPrimaryAction",
        "接受通知主要操作",
        Some("notifications"),
    ),
];

/// 工作台实现了的命令 ID（菜单与命令面板据此决定是否显示）。
pub fn implemented_commands() -> impl Iterator<Item = &'static str> {
    COMMANDS.iter().map(|c| c.0)
}
pub fn is_implemented(id: &str) -> bool {
    COMMANDS.iter().any(|c| c.0 == id)
}
pub fn command_title_key(id: &str) -> String {
    format!("command.{id}")
}

pub fn register_commands(r: &mut CommandRegistry) -> Result<()> {
    for (id, _, category) in COMMANDS {
        r.register(
            *id,
            Command {
                title_key: command_title_key(id).into(),
                category_key: category.map(|c| Arc::from(format!("category.{c}"))),
            },
        )?;
    }
    Ok(())
}

/// 发布在 GPUI key context 上的 VS Code 上下文键：状态栏获得焦点、通知 toast 获得焦点、toast 可见、
/// 通知中心可见。键位服务按这些名字求值 when 子句。
pub const CONTEXT_STATUS_BAR: &str = "statusBarFocused";
pub const CONTEXT_NOTIFICATION: &str = "notificationFocus";
pub const CONTEXT_TOASTS: &str = "notificationToastsVisible";
pub const CONTEXT_CENTER: &str = "notificationCenterVisible";
/// 模态框打开（Rove 内部上下文，只用于模态框的 Esc）。
pub const CONTEXT_MODAL: &str = "roveModalOpen";

/// 菜单与悬停提示里显示的键位文字（VS Code 风格：`Ctrl+Shift+M`），取自生效的键位表（含用户覆盖）。
pub fn key_label(id: &str, cx: &gpui::App) -> Option<String> {
    crate::keys::key_label(id, cx)
}

// ------------------------------------------------------------------ 菜单

/// (菜单, 组, 顺序, 命令, 菜单标题 key)。只登记工作台自己实现的命令；
/// 其余菜单项由各 issue 在实现命令时追加登记（未实现的不显示）。
const MENU_ITEMS: &[(MenuId, &str, i32, &str, &str)] = &[
    (
        MenuId::File,
        "6_close",
        10,
        "workbench.action.closeWindow",
        "menu.closeWindow",
    ),
    (
        MenuId::View,
        "2_views",
        10,
        "workbench.view.explorer",
        "menu.explorer",
    ),
    (
        MenuId::View,
        "2_views",
        20,
        "workbench.view.search",
        "menu.search",
    ),
    (
        MenuId::View,
        "3_panels",
        10,
        "workbench.actions.view.problems",
        "menu.problems",
    ),
    (
        MenuId::View,
        "4_appearance",
        10,
        "workbench.action.toggleSidebarVisibility",
        "menu.toggleSidebar",
    ),
    (
        MenuId::View,
        "4_appearance",
        20,
        "workbench.action.toggleSidebarPosition",
        "menu.toggleSidebarPosition",
    ),
    (
        MenuId::View,
        "4_appearance",
        30,
        "workbench.action.toggleStatusbarVisibility",
        "menu.toggleStatusBar",
    ),
    (
        MenuId::View,
        "4_appearance",
        40,
        "workbench.action.togglePanel",
        "menu.togglePanel",
    ),
    (
        MenuId::View,
        "5_zoom",
        10,
        "workbench.action.zoomIn",
        "menu.zoomIn",
    ),
    (
        MenuId::View,
        "5_zoom",
        20,
        "workbench.action.zoomOut",
        "menu.zoomOut",
    ),
    (
        MenuId::View,
        "5_zoom",
        30,
        "workbench.action.zoomReset",
        "menu.zoomReset",
    ),
    (
        MenuId::Manage,
        "1_commands",
        10,
        "workbench.action.showCommands",
        "menu.commandPalette",
    ),
    (
        MenuId::Help,
        "1_notifications",
        10,
        "notifications.toggleDoNotDisturbMode",
        "menu.toggleDnd",
    ),
];

pub fn register_menus(r: &mut MenuRegistry) -> Result<()> {
    for (menu, group, order, command, title) in MENU_ITEMS {
        r.register(
            format!("{}.{command}", menu.title_key()),
            MenuItem {
                menu: *menu,
                group: (*group).into(),
                order: *order,
                command: (*command).into(),
                title: Some(Message::key(*title)),
            },
        )?;
    }
    Ok(())
}

// ------------------------------------------------------------------ 设置

/// VS Code 默认值（compat/SETTINGS.md）。工作台不写回设置；切换类命令先尝试 `Settings::write`，
/// 不支持写入时只改当前窗口的运行期值。
const SETTINGS: &[(&str, SettingDefault)] = &[
    ("window.titleBarStyle", SettingDefault::Str("custom")),
    (
        "window.customTitleBarVisibility",
        SettingDefault::Str("auto"),
    ),
    ("window.controlsStyle", SettingDefault::Str("native")),
    ("window.commandCenter", SettingDefault::Bool(true)),
    ("window.menuBarVisibility", SettingDefault::Str("classic")),
    ("window.menuStyle", SettingDefault::Str("inherit")),
    ("window.zoomLevel", SettingDefault::Num(0.0)),
    ("window.zoomPerWindow", SettingDefault::Bool(true)),
    ("window.restoreWindows", SettingDefault::Str("all")),
    (
        "workbench.layoutControl.enabled",
        SettingDefault::Bool(true),
    ),
    ("workbench.sideBar.location", SettingDefault::Str("left")),
    (
        "workbench.panel.defaultLocation",
        SettingDefault::Str("bottom"),
    ),
    ("workbench.panel.showLabels", SettingDefault::Bool(true)),
    ("workbench.statusBar.visible", SettingDefault::Bool(true)),
    (
        "workbench.notifications.position",
        SettingDefault::Str("bottom-right"),
    ),
    (
        "workbench.notifications.showInTitleBar",
        SettingDefault::Bool(true),
    ),
];
#[derive(Clone, Copy)]
enum SettingDefault {
    Str(&'static str),
    Bool(bool),
    Num(f64),
}
pub fn register_settings(r: &mut SettingRegistry) -> Result<()> {
    for (key, default) in SETTINGS {
        r.register(
            *key,
            Setting {
                default: match default {
                    SettingDefault::Str(s) => Value::String((*s).into()),
                    SettingDefault::Bool(b) => Value::Bool(*b),
                    SettingDefault::Num(n) => Value::Number(*n),
                },
                rove_default: None,
                language_overridable: false,
            },
        )?;
    }
    Ok(())
}

// ------------------------------------------------------------------ 颜色

/// (key, 深色, 浅色)。取值对照 VS Code 2026 默认主题（深色与 `2026-dark.json` 保持同一色系，
/// 浅色取 Light Modern）；None 为明确不设色。高对比度主题由 #10 加载主题文件时提供，
/// 这里的高对比度默认值暂取同侧（深 / 浅）的值。
type Rgba = Option<u32>;
const COLORS: &[(&str, Rgba, Rgba)] = &[
    (
        "titleBar.activeBackground",
        Some(0x191a1bff),
        Some(0xf8f8f8ff),
    ),
    (
        "titleBar.activeForeground",
        Some(0xccccccff),
        Some(0x1e1e1eff),
    ),
    (
        "titleBar.inactiveBackground",
        Some(0x191a1bff),
        Some(0xf8f8f8ff),
    ),
    (
        "titleBar.inactiveForeground",
        Some(0x8c8c8cff),
        Some(0x8b949eff),
    ),
    ("titleBar.border", Some(0x2b2b2bff), Some(0xe5e5e5ff)),
    (
        "commandCenter.foreground",
        Some(0xccccccff),
        Some(0x1e1e1eff),
    ),
    (
        "commandCenter.activeForeground",
        Some(0xffffffff),
        Some(0x1e1e1eff),
    ),
    (
        "commandCenter.inactiveForeground",
        Some(0x8c8c8cff),
        Some(0x8b949eff),
    ),
    (
        "commandCenter.background",
        Some(0xffffff0d),
        Some(0x0000000a),
    ),
    (
        "commandCenter.activeBackground",
        Some(0xffffff14),
        Some(0x00000014),
    ),
    ("commandCenter.border", Some(0xffffff1f), Some(0x00000020)),
    (
        "commandCenter.inactiveBorder",
        Some(0xffffff14),
        Some(0x00000014),
    ),
    (
        "commandCenter.activeBorder",
        Some(0xffffff2e),
        Some(0x0000002e),
    ),
    (
        "menubar.selectionForeground",
        Some(0xccccccff),
        Some(0x1e1e1eff),
    ),
    (
        "menubar.selectionBackground",
        Some(0xffffff1a),
        Some(0xb8b8b850),
    ),
    ("menubar.selectionBorder", None, None),
    ("menu.foreground", Some(0xccccccff), Some(0x3b3b3bff)),
    ("menu.background", Some(0x202122ff), Some(0xffffffff)),
    (
        "menu.selectionForeground",
        Some(0xffffffff),
        Some(0xffffffff),
    ),
    (
        "menu.selectionBackground",
        Some(0x0078d4ff),
        Some(0x005fb8ff),
    ),
    ("menu.selectionBorder", None, None),
    (
        "menu.separatorBackground",
        Some(0x454545ff),
        Some(0xd4d4d4ff),
    ),
    ("menu.border", Some(0x454545ff), Some(0xcececeff)),
    ("activityBar.background", Some(0x191a1bff), Some(0xf8f8f8ff)),
    ("activityBar.foreground", Some(0xd7d7d7ff), Some(0x1f1f1fff)),
    (
        "activityBar.inactiveForeground",
        Some(0x868686ff),
        Some(0x616161ff),
    ),
    ("activityBar.border", Some(0x2b2b2bff), Some(0xe5e5e5ff)),
    (
        "activityBar.activeBorder",
        Some(0x0078d4ff),
        Some(0x005fb8ff),
    ),
    ("activityBar.activeBackground", None, None),
    (
        "activityBar.activeFocusBorder",
        Some(0x0078d4ff),
        Some(0x005fb8ff),
    ),
    ("activityBar.dropBorder", Some(0xd7d7d7ff), Some(0x1f1f1fff)),
    (
        "activityBarBadge.background",
        Some(0x0078d4ff),
        Some(0x005fb8ff),
    ),
    (
        "activityBarBadge.foreground",
        Some(0xffffffff),
        Some(0xffffffff),
    ),
    ("sideBar.background", Some(0x191a1bff), Some(0xf8f8f8ff)),
    ("sideBar.foreground", Some(0xbfbfbfff), Some(0x3b3b3bff)),
    ("sideBar.border", Some(0x2b2b2bff), Some(0xe5e5e5ff)),
    (
        "sideBarTitle.foreground",
        Some(0xccccccff),
        Some(0x3b3b3bff),
    ),
    (
        "sideBarTitle.background",
        Some(0x191a1bff),
        Some(0xf8f8f8ff),
    ),
    (
        "sideBarSectionHeader.background",
        Some(0x191a1bff),
        Some(0xf8f8f8ff),
    ),
    (
        "sideBarSectionHeader.foreground",
        Some(0xccccccff),
        Some(0x3b3b3bff),
    ),
    (
        "sideBarSectionHeader.border",
        Some(0x2b2b2bff),
        Some(0xe5e5e5ff),
    ),
    ("panel.background", Some(0x191a1bff), Some(0xf8f8f8ff)),
    ("panel.border", Some(0x2b2b2bff), Some(0xe5e5e5ff)),
    (
        "panelTitle.activeBorder",
        Some(0x0078d4ff),
        Some(0x005fb8ff),
    ),
    (
        "panelTitle.activeForeground",
        Some(0xccccccff),
        Some(0x3b3b3bff),
    ),
    (
        "panelTitle.inactiveForeground",
        Some(0x9d9d9dff),
        Some(0x3b3b3bff),
    ),
    ("panelTitle.border", None, None),
    ("statusBar.background", Some(0x191a1bff), Some(0xf8f8f8ff)),
    ("statusBar.foreground", Some(0x8c8c8cff), Some(0x3b3b3bff)),
    ("statusBar.border", Some(0x2b2b2bff), Some(0xe5e5e5ff)),
    ("statusBar.focusBorder", Some(0x0078d4ff), Some(0x005fb8ff)),
    (
        "statusBar.noFolderBackground",
        Some(0x191a1bff),
        Some(0xf8f8f8ff),
    ),
    (
        "statusBar.noFolderForeground",
        Some(0x8c8c8cff),
        Some(0x3b3b3bff),
    ),
    (
        "statusBarItem.activeBackground",
        Some(0xffffff2e),
        Some(0x1f1f1f1f),
    ),
    (
        "statusBarItem.hoverBackground",
        Some(0xffffff1f),
        Some(0x1f1f1f11),
    ),
    (
        "statusBarItem.hoverForeground",
        Some(0xffffffff),
        Some(0x1e1e1eff),
    ),
    (
        "statusBarItem.focusBorder",
        Some(0x0078d4ff),
        Some(0x005fb8ff),
    ),
    (
        "statusBarItem.prominentBackground",
        Some(0x6e768166),
        Some(0x6e768166),
    ),
    (
        "statusBarItem.prominentForeground",
        Some(0xffffffff),
        Some(0x1e1e1eff),
    ),
    (
        "statusBarItem.prominentHoverBackground",
        Some(0x6e768199),
        Some(0x6e768199),
    ),
    (
        "statusBarItem.prominentHoverForeground",
        Some(0xffffffff),
        Some(0x1e1e1eff),
    ),
    (
        "statusBarItem.errorBackground",
        Some(0xc72e0fff),
        Some(0xc72e0fff),
    ),
    (
        "statusBarItem.errorForeground",
        Some(0xffffffff),
        Some(0xffffffff),
    ),
    (
        "statusBarItem.errorHoverBackground",
        Some(0xdb4f32ff),
        Some(0xdb4f32ff),
    ),
    (
        "statusBarItem.errorHoverForeground",
        Some(0xffffffff),
        Some(0xffffffff),
    ),
    (
        "statusBarItem.warningBackground",
        Some(0x7a6400ff),
        Some(0x725102ff),
    ),
    (
        "statusBarItem.warningForeground",
        Some(0xffffffff),
        Some(0xffffffff),
    ),
    (
        "statusBarItem.warningHoverBackground",
        Some(0x9c8000ff),
        Some(0x946a03ff),
    ),
    (
        "statusBarItem.warningHoverForeground",
        Some(0xffffffff),
        Some(0xffffffff),
    ),
    (
        "notifications.background",
        Some(0x202122ff),
        Some(0xffffffff),
    ),
    (
        "notifications.foreground",
        Some(0xccccccff),
        Some(0x3b3b3bff),
    ),
    ("notifications.border", Some(0x2b2b2bff), Some(0xe5e5e5ff)),
    (
        "notificationToast.border",
        Some(0x2b2b2bff),
        Some(0xe5e5e5ff),
    ),
    (
        "notificationCenter.border",
        Some(0x2b2b2bff),
        Some(0xe5e5e5ff),
    ),
    (
        "notificationCenterHeader.background",
        Some(0x191a1bff),
        Some(0xf8f8f8ff),
    ),
    (
        "notificationCenterHeader.foreground",
        Some(0xccccccff),
        Some(0x3b3b3bff),
    ),
    (
        "notificationLink.foreground",
        Some(0x4daafcff),
        Some(0x005fb8ff),
    ),
    (
        "notificationsErrorIcon.foreground",
        Some(0xf14c4cff),
        Some(0xe51400ff),
    ),
    (
        "notificationsWarningIcon.foreground",
        Some(0xcca700ff),
        Some(0xbf8803ff),
    ),
    (
        "notificationsInfoIcon.foreground",
        Some(0x3794ffff),
        Some(0x1a85ffff),
    ),
    ("progressBar.background", Some(0x0078d4ff), Some(0x005fb8ff)),
    ("window.activeBorder", None, None),
    ("window.inactiveBorder", None, None),
];

fn rgba(v: Rgba) -> Option<Color> {
    v.map(|v| Color((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8))
}
pub fn register_colors(r: &mut ColorRegistry) -> Result<()> {
    for (key, dark, light) in COLORS {
        r.register(
            *key,
            ColorDefaults {
                dark: rgba(*dark),
                light: rgba(*light),
                high_contrast_dark: rgba(*dark),
                high_contrast_light: rgba(*light),
            },
        )?;
    }
    Ok(())
}

// ------------------------------------------------------------------ 界面字符串

const STRINGS: &[(&str, &str)] = &[
    ("category.view", "查看"),
    ("category.notifications", "通知"),
    ("menubar.file", "文件"),
    ("menubar.edit", "编辑"),
    ("menubar.selection", "选择"),
    ("menubar.view", "查看"),
    ("menubar.go", "转到"),
    ("menubar.help", "帮助"),
    ("menubar.compact", "应用程序菜单"),
    ("activityBar.manage", "管理"),
    ("menu.closeWindow", "关闭窗口"),
    ("menu.explorer", "资源管理器"),
    ("menu.search", "搜索"),
    ("menu.problems", "问题"),
    ("menu.toggleSidebar", "主侧边栏"),
    ("menu.toggleSidebarPosition", "切换主侧边栏位置"),
    ("menu.toggleStatusBar", "状态栏"),
    ("menu.togglePanel", "面板"),
    ("menu.zoomIn", "放大"),
    ("menu.zoomOut", "缩小"),
    ("menu.zoomReset", "重置缩放"),
    ("menu.commandPalette", "命令面板..."),
    ("menu.toggleDnd", "勿扰模式"),
    ("view.explorer", "资源管理器"),
    ("view.search", "搜索"),
    ("view.folders", "文件夹"),
    ("view.problems", "问题"),
    ("view.empty", "此视图中没有内容。"),
    ("problems.empty", "未在工作区检测到问题。"),
    ("status.problems.name", "问题"),
    (
        "status.problems.text",
        "$(error) {errors}  $(warning) {warnings}",
    ),
    ("status.problems.none", "无问题"),
    (
        "status.problems.tooltip",
        "错误: {errors}，警告: {warnings}",
    ),
    ("status.notifications.name", "通知"),
    ("status.notifications.none", "无新通知"),
    ("status.notifications.some", "{count} 条新通知"),
    ("status.notifications.dnd", "勿扰模式已开启"),
    ("commandCenter.placeholder", "{name}"),
    ("commandCenter.tooltip", "搜索 {name}"),
    ("commandCenter.notImplemented", "快速打开尚未实现"),
    ("layout.toggleSidebar", "切换主侧边栏 ({key})"),
    ("layout.togglePanel", "切换面板 ({key})"),
    ("window.minimize", "最小化"),
    ("window.maximize", "最大化"),
    ("window.restore", "向下还原"),
    ("window.close", "关闭"),
    ("panel.close", "隐藏面板"),
    ("panel.maximize", "最大化面板大小"),
    ("notifications.center", "通知"),
    ("notifications.centerEmpty", "无新通知"),
    ("notifications.clearAll", "清除所有通知"),
    ("notifications.toggleDnd", "切换勿扰模式"),
    ("notifications.hide", "隐藏通知"),
    ("notifications.close", "清除通知"),
    ("notifications.count", "{count}"),
    ("feedback.neverShowAgain", "不再提示"),
    ("feedback.close", "关闭"),
    ("feedback.dialogTitle", "Rove"),
];

pub fn register_strings(t: &mut StringTable) -> Result<()> {
    for (key, value) in STRINGS {
        t.register(*key, Arc::from(*value))?;
    }
    for (id, title, _) in COMMANDS {
        t.register(command_title_key(id), Arc::from(*title))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::Strings;

    #[test]
    fn every_registration_resolves() {
        let mut strings = rove_i18n::chinese().unwrap();
        register_strings(&mut strings).unwrap();
        let mut commands = CommandRegistry::default();
        register_commands(&mut commands).unwrap();
        let mut menus = MenuRegistry::default();
        register_menus(&mut menus).unwrap();
        let mut views = ContributionRegistry::default();
        register_contributions(&mut views).unwrap();
        register_settings(&mut SettingRegistry::default()).unwrap();
        register_colors(&mut ColorRegistry::default()).unwrap();
        for (_, c) in commands.entries() {
            strings.text(&Message::key(c.title_key.clone())).unwrap();
            if let Some(k) = &c.category_key {
                strings.text(&Message::key(k.clone())).unwrap();
            }
        }
        for (_, m) in menus.entries() {
            strings.text(m.title.as_ref().unwrap()).unwrap();
            strings.text(&Message::key(m.menu.title_key())).unwrap();
        }
        for (_, v) in views.entries() {
            strings.text(&v.title).unwrap();
            if let Some(c) = &v.command {
                assert!(commands.get(c).is_some(), "{c} 未登记");
            }
        }
    }

    #[test]
    fn key_labels_follow_vscode_style() {
        gpui::TestAppContext::single().update(|cx| {
            assert_eq!(
                key_label("workbench.action.toggleSidebarVisibility", cx).as_deref(),
                Some("Ctrl+B")
            );
            assert_eq!(
                key_label("workbench.actions.view.problems", cx).as_deref(),
                Some("Ctrl+Shift+M")
            );
            assert_eq!(
                key_label("notifications.showList", cx).as_deref(),
                Some("Ctrl+K Ctrl+Shift+N")
            );
            assert_eq!(
                key_label("workbench.action.focusNextPart", cx).as_deref(),
                Some("F6")
            );
            assert_eq!(
                key_label("workbench.action.zoomIn", cx).as_deref(),
                Some("Ctrl+=")
            );
            assert_eq!(key_label("rove.index.rebuild", cx), None);
        });
    }
}
