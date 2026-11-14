//! 搜索视图的登记项（#25）：命令（VS Code ID）、界面字符串、颜色键、视图小节与命令的 GPUI action。
//!
//! - `workbench.view.search` / `workbench.action.findInFiles` 由 #16 的工作台登记并打开视图容器，
//!   本模块不重复登记；视图被显示后由宿主把焦点交给查询框；
//! - 设置项 `search.*` 由 `rove_search::settings::register_settings` 登记（`app_settings` 装配）；
//! - 默认键位来自 #14 生成的 VS Code 默认键位表（`f4`、`alt+c`、`delete`…），不重复登记；
//! - **不登记**替换相关命令（`search.action.replace*`、`workbench.action.replaceInFiles`、
//!   `toggleSearchPreserveCase`，属于 R61）与搜索编辑器（P2）。
use crate::contributions::{Contribution, ContributionRegistry, Placement};
use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::{Message, StringTable};
use rove_theme::ColorRegistry;
use std::sync::Arc;

/// 搜索视图容器里的小节（视图 ID 与工厂 key）。
pub const SEARCH_VIEW: &str = "workbench.view.search.results";

pub const OPEN_RESULT: &str = "search.action.openResult";
pub const OPEN_RESULT_TO_SIDE: &str = "search.action.openResultToSide";
pub const NEXT_RESULT: &str = "search.action.focusNextSearchResult";
pub const PREVIOUS_RESULT: &str = "search.action.focusPreviousSearchResult";
pub const REMOVE: &str = "search.action.remove";
pub const COPY_MATCH: &str = "search.action.copyMatch";
pub const COPY_PATH: &str = "search.action.copyPath";
pub const COPY_ALL: &str = "search.action.copyAll";
pub const RESTRICT_TO_FOLDER: &str = "search.action.restrictSearchToFolder";
pub const CANCEL: &str = "search.action.cancel";
pub const CLEAR: &str = "search.action.clearSearchResults";
pub const REFRESH: &str = "search.action.refreshSearchResults";
pub const COLLAPSE_ALL: &str = "search.action.collapseSearchResults";
pub const EXPAND_ALL: &str = "search.action.expandSearchResults";
pub const FOCUS_FROM_RESULTS: &str = "search.action.focusSearchFromResults";
pub const NEXT_INPUT: &str = "search.focus.nextInputBox";
pub const PREVIOUS_INPUT: &str = "search.focus.previousInputBox";
pub const TOGGLE_CASE: &str = "toggleSearchCaseSensitive";
pub const TOGGLE_WORD: &str = "toggleSearchWholeWord";
pub const TOGGLE_REGEX: &str = "toggleSearchRegex";
/// #52：`ctrl+shift+l`，把结果列表里当前文件的匹配变成编辑器里的多光标。
pub const ADD_CURSORS: &str = "addCursorsAtSearchResults";

/// 发布在搜索视图上的 VS Code 上下文键（默认键位的 when 子句用到）。
pub mod context {
    /// 全局：搜索视图正在显示。
    pub const VIEWLET_VISIBLE: &str = "searchViewletVisible";
    /// 全局：结果列表里有结果（`f4` / `shift+f4` 在编辑器里也可用）。
    pub const HAS_RESULT: &str = "hasSearchResult";
    pub const VIEWLET_FOCUS: &str = "searchViewletFocus";
    pub const INPUT_BOX_FOCUS: &str = "inputBoxFocus";
    pub const SEARCH_INPUT_FOCUS: &str = "searchInputBoxFocus";
    pub const FILE_OR_MATCH_FOCUS: &str = "fileMatchOrMatchFocus";
    pub const FILE_FOCUS: &str = "fileMatchFocus";
    pub const MATCH_FOCUS: &str = "matchFocus";
    pub const FIRST_MATCH_FOCUS: &str = "firstMatchFocus";
    pub const FILE_OR_FOLDER_RESOURCE_FOCUS: &str = "fileMatchOrFolderMatchWithResourceFocus";
    pub const FOLDER_RESOURCE_FOCUS: &str = "folderMatchWithResourceFocus";
    /// `searchState`：`0` 空闲、`1` 正在搜索（VS Code 的 `SearchUIState`）。
    pub const STATE: &str = "searchState";
}

/// (命令 ID, 标题字符串 key, 中文标题)
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (OPEN_RESULT, "search.command.openResult", "打开结果"),
    (
        OPEN_RESULT_TO_SIDE,
        "search.command.openResultToSide",
        "在侧边打开",
    ),
    (NEXT_RESULT, "search.command.next", "转到下一个搜索结果"),
    (
        PREVIOUS_RESULT,
        "search.command.previous",
        "转到上一个搜索结果",
    ),
    (REMOVE, "search.command.remove", "消除"),
    (COPY_MATCH, "search.command.copyMatch", "复制"),
    (COPY_PATH, "search.command.copyPath", "复制路径"),
    (COPY_ALL, "search.command.copyAll", "全部复制"),
    (
        RESTRICT_TO_FOLDER,
        "search.command.restrictToFolder",
        "限制搜索文件夹",
    ),
    (CANCEL, "search.command.cancel", "取消搜索"),
    (CLEAR, "search.command.clear", "清除搜索结果"),
    (REFRESH, "search.command.refresh", "刷新"),
    (COLLAPSE_ALL, "search.command.collapseAll", "全部折叠"),
    (EXPAND_ALL, "search.command.expandAll", "全部展开"),
    (
        FOCUS_FROM_RESULTS,
        "search.command.focusFromResults",
        "从结果聚焦到搜索输入",
    ),
    (NEXT_INPUT, "search.command.nextInput", "聚焦下一个输入框"),
    (
        PREVIOUS_INPUT,
        "search.command.previousInput",
        "聚焦上一个输入框",
    ),
    (TOGGLE_CASE, "search.command.toggleCase", "切换区分大小写"),
    (TOGGLE_WORD, "search.command.toggleWord", "切换全字匹配"),
    (
        TOGGLE_REGEX,
        "search.command.toggleRegex",
        "切换使用正则表达式",
    ),
    // #52：在搜索结果处添加光标。
    (
        ADD_CURSORS,
        "search.command.addCursorsAtSearchResults",
        "在搜索结果处添加光标",
    ),
];

/// 命令分类（命令面板里显示为“搜索: …”）。
pub const CATEGORY_KEY: &str = "search.category";

/// 其余界面字符串（UX_SPEC §6.2 / §6.3）。
pub const STRINGS: &[(&str, &str)] = &[
    ("search.category", "搜索"),
    ("view.searchResults", "搜索"),
    ("search.placeholder", "搜索"),
    ("search.includes", "包含的文件"),
    ("search.excludes", "排除的文件"),
    ("search.includes.placeholder", "例如 *.cs, ./Assets/Scripts"),
    ("search.excludes.placeholder", "例如 *.meta, Plugins"),
    ("search.useExcludeSettings", "使用“排除设置”"),
    ("search.toggle.case", "区分大小写"),
    ("search.toggle.word", "全字匹配"),
    ("search.toggle.regex", "使用正则表达式"),
    ("search.tooltip.withKey", "{label} ({key})"),
    ("search.summary", "{files} 文件中有 {matches} 个结果"),
    ("search.summary.one", "1 文件中有 {matches} 个结果"),
    ("search.searching", "正在搜索…"),
    (
        "search.limitHit",
        "结果集仅包含所有匹配项的子集。请使你的搜索更加具体，减少结果。",
    ),
    ("search.noResults", "未找到结果。"),
    ("search.openSettings", "查看排除设置"),
    ("search.roveExcluded", "已排除 Unity 生成的目录。"),
    ("search.searchExcluded", "包含排除的文件再搜索一次"),
    ("search.invalidRegex", "正则表达式无效。"),
    ("search.cancelled", "已取消搜索。"),
    ("search.lineNumber", "{line}:"),
];

/// 视图用到的颜色键（均为 VS Code 颜色；默认值取 #10 的默认值表）。
pub const COLORS: &[&str] = &[
    "search.resultsInfoForeground",
    "editor.findMatchHighlightBackground",
    "badge.background",
    "badge.foreground",
    "progressBar.background",
    "input.background",
    "input.border",
    "input.foreground",
    "input.placeholderForeground",
    "inputOption.activeBackground",
    "inputOption.activeBorder",
    "inputOption.activeForeground",
    "inputOption.hoverBackground",
    "inputValidation.errorBackground",
    "inputValidation.errorBorder",
    "textLink.foreground",
    "descriptionForeground",
    "focusBorder",
    "sideBar.foreground",
    "icon.foreground",
    "errorForeground",
];

pub fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, title, _) in COMMANDS {
        match registry.register(
            *id,
            Command {
                title_key: (*title).into(),
                category_key: Some(CATEGORY_KEY.into()),
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
    for (key, text) in STRINGS.iter().copied().chain(commands) {
        match table.register(key, Arc::from(text)) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 登记颜色键；已存在的键跳过（主题或其他视图已登记）。
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

/// 搜索视图容器（#16 登记的 `workbench.view.search`）里唯一的小节。
pub fn register_contributions(registry: &mut ContributionRegistry) -> Result<()> {
    registry.register(
        SEARCH_VIEW,
        Contribution {
            title: Message::key("view.searchResults"),
            placement: Placement::Sidebar {
                container: crate::shell::catalog::SEARCH.into(),
            },
            order: 0,
            icon: None,
            command: None,
            when: None,
            factory_key: SEARCH_VIEW.into(),
        },
    )
}

/// 本模块命令的 GPUI action；由窗口（宿主）处理后转给搜索视图，键位服务按 when 解析后分发。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = search, no_json)]
pub struct SearchAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(SearchAction(cmd)) as Box<dyn Action>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::Strings;

    #[test]
    fn every_command_has_an_action_and_a_title() {
        let mut registry = CommandRegistry::default();
        register_commands(&mut registry).unwrap();
        let mut strings = StringTable::default();
        register_strings(&mut strings).unwrap();
        for (id, title, _) in COMMANDS {
            assert!(action_for(id).is_some(), "{id}");
            assert!(registry.get(id).is_some(), "{id}");
            assert!(strings.text(&Message::key(*title)).is_ok(), "{title}");
        }
        assert!(
            !COMMANDS.iter().any(|(id, _, _)| id.contains("replace")),
            "替换属于 R61"
        );
    }

    #[test]
    fn colors_have_vscode_defaults() {
        let mut colors = ColorRegistry::default();
        register_colors(&mut colors).unwrap();
        for key in COLORS {
            assert!(colors.get(key).is_some(), "{key}");
        }
    }
}
