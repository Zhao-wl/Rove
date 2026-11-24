use gpui::Action;
use rove_base::Result;
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use rove_theme::ColorRegistry;
use std::sync::Arc;

pub const EDITOR: &str = "workbench.editor.settings";
pub const RESOURCE: &str = "rove-settings:";
pub const TAB_TITLE: &str = "settings.tabTitle";

pub const OPEN_UI: &str = "workbench.action.openSettings2";
pub const OPEN_JSON: &str = "workbench.action.openSettingsJson";
pub const SWITCH_JSON: &str = "settings.switchToJSON";
pub const CLEAR_SEARCH: &str = "settings.action.clearSearchResults";
pub const LEVEL_UP: &str = "settings.action.focusLevelUp";
pub const SEARCH_FROM_ROW: &str = "settings.action.focusSearchFromSettings";
pub const FOCUS_CONTROL: &str = "settings.action.focusSettingControl";
pub const LIST_FROM_SEARCH: &str = "settings.action.focusSettingsFromSearch";
pub const LIST_FROM_SEARCH_ENTER: &str = "settings.action.focusSettingsFromSearchOnEnter";
pub const FOCUS_LIST: &str = "settings.action.focusSettingsList";
pub const FOCUS_TOC: &str = "settings.action.focusTOC";
pub const SEARCH: &str = "settings.action.search";
pub const SHOW_MENU: &str = "settings.action.showContextMenu";
pub const PREVIOUS_SEARCH: &str = "settings.action.showPreviousSearch";
pub const FOCUS_FILE: &str = "settings.action.focusSettingsFile";

pub const IN_EDITOR: &str = "inSettingsEditor";
pub const IN_SEARCH: &str = "inSettingsSearch";
pub const ROW_FOCUS: &str = "settingRowFocus";
pub const FIRST_ROW_FOCUS: &str = "settingFirstRowFocus";
pub const TOC_FOCUS: &str = "settingsTocRowFocus";
pub const IN_JSON: &str = "inSettingsJSONEditor";

pub const MODE: rove_config::Key<String> = rove_config::Key::new("workbench.settings.editor");
pub const TOC_BEHAVIOR: rove_config::Key<String> =
    rove_config::Key::new("workbench.settings.settingsSearchTocBehavior");

const COMMANDS: &[(&str, &str, &str)] = &[
    (OPEN_UI, "settings.command.openUI", "打开设置界面"),
    (
        OPEN_JSON,
        "settings.command.openJSON",
        "打开用户设置（JSON）",
    ),
    (SWITCH_JSON, "settings.command.switchJSON", "在 JSON 中打开"),
    (CLEAR_SEARCH, "settings.command.clearSearch", "清除设置搜索"),
    (LEVEL_UP, "settings.command.levelUp", "返回上一级"),
    (
        SEARCH_FROM_ROW,
        "settings.command.searchFromRow",
        "返回设置搜索",
    ),
    (
        FOCUS_CONTROL,
        "settings.command.focusControl",
        "聚焦设置控件",
    ),
    (
        LIST_FROM_SEARCH,
        "settings.command.listFromSearch",
        "从搜索定位设置",
    ),
    (
        LIST_FROM_SEARCH_ENTER,
        "settings.command.listFromSearchEnter",
        "打开首个设置结果",
    ),
    (FOCUS_LIST, "settings.command.focusList", "聚焦设置列表"),
    (FOCUS_TOC, "settings.command.focusToc", "聚焦设置目录"),
    (SEARCH, "settings.command.search", "搜索设置"),
    (SHOW_MENU, "settings.command.showMenu", "打开当前设置的菜单"),
    (
        PREVIOUS_SEARCH,
        "settings.command.previousSearch",
        "上一条设置搜索",
    ),
    (FOCUS_FILE, "settings.command.focusFile", "聚焦设置文件"),
];

const STRINGS: &[(&str, &str)] = &[
    (TAB_TITLE, "设置"),
    ("settings.searchPlaceholder", "搜索设置"),
    ("settings.category.all", "全部设置"),
    (
        "settings.category.roveDescription",
        "VS Code 会忽略这些项。",
    ),
    ("settings.openJSON", "在 settings.json 中编辑"),
    ("settings.roveDefault", "Rove 默认值与 VS Code 不同"),
    ("settings.languageOverrides", "有语言级覆盖，在 JSON 中编辑"),
    ("settings.reset", "重置为默认值"),
    ("settings.copyId", "复制设置 ID"),
    ("settings.copyJson", "复制设置为 JSON"),
    ("settings.noResults", "没有匹配的设置。"),
    (
        "settings.invalidFile",
        "设置文件第 {line} 行、第 {column} 列有错误。已显示上一份有效设置。",
    ),
    ("settings.apply", "应用"),
    ("settings.invalidValue", "请输入有效的设置值。"),
    ("settings.writeFailed", "无法写入设置（{error}）。"),
    ("settings.unavailable", "无法访问共享设置文件。"),
];

pub const COLORS: &[&str] = &[
    "settings.headerForeground",
    "settings.modifiedItemIndicator",
    "settings.dropdownBackground",
    "settings.dropdownForeground",
    "settings.dropdownBorder",
    "settings.dropdownListBorder",
    "settings.checkboxBackground",
    "settings.checkboxForeground",
    "settings.checkboxBorder",
    "settings.rowHoverBackground",
    "settings.textInputBackground",
    "settings.textInputForeground",
    "settings.textInputBorder",
    "settings.numberInputBackground",
    "settings.numberInputForeground",
    "settings.numberInputBorder",
    "settings.focusedRowBackground",
    "settings.focusedRowBorder",
    "settings.headerBorder",
    "settings.sashBorder",
    "settings.settingsHeaderHoverForeground",
];

pub fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, title, _) in COMMANDS {
        registry.register(
            *id,
            Command {
                title_key: (*title).into(),
                category_key: None,
            },
        )?;
    }
    Ok(())
}

pub fn register_strings(table: &mut StringTable) -> Result<()> {
    for (key, text) in STRINGS
        .iter()
        .copied()
        .chain(COMMANDS.iter().map(|(_, key, title)| (*key, *title)))
    {
        table.register(key, Arc::from(text))?;
    }
    Ok(())
}

pub fn register_colors(registry: &mut ColorRegistry) -> Result<()> {
    rove_theme::defaults::register_vscode_colors(registry, COLORS)
}

pub fn register_settings(registry: &mut rove_config::SettingRegistry) -> Result<()> {
    for key in [MODE.id, TOC_BEHAVIOR.id] {
        let entry = rove_config::catalog::catalog()
            .get(key)
            .ok_or(rove_base::Error::NotFound)?;
        registry.register(key, entry.setting.clone())?;
    }
    Ok(())
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = settings_ui, no_json)]
pub struct SettingsAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(command, _, _)| *command == id)
        .map(|(command, _, _)| Box::new(SettingsAction(command)) as Box<dyn Action>)
}
