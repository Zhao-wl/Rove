//! Registration data for lists, trees and quick input (#18): command IDs, context keys, strings,
//! color keys. Default keys come from #14's generated VS Code table (`crate::keys`).
//!
//! Everything goes through the #1 registries. Colors/commands/settings that a central owner
//! (#10/#13/#14) registers first keep their entry; the defaults here are the same VS Code values.
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use rove_theme::{Color, ColorDefaults, ColorRegistry};
use std::sync::Arc;

/// Context keys produced by lists/trees/quick input (VS Code names).
pub mod context {
    pub const LIST_FOCUS: &str = "listFocus";
    pub const LIST_SUPPORTS_MULTISELECT: &str = "listSupportsMultiselect";
    pub const LIST_HAS_SELECTION_OR_FOCUS: &str = "listHasSelectionOrFocus";
    pub const LIST_SCROLL_AT_BOUNDARY: &str = "listScrollAtBoundary";
    pub const LIST_SUPPORTS_FIND: &str = "listSupportsFind";
    pub const TREE_FIND_OPEN: &str = "treeFindOpen";
    pub const TREE_ELEMENT_CAN_COLLAPSE: &str = "treeElementCanCollapse";
    pub const TREE_ELEMENT_CAN_EXPAND: &str = "treeElementCanExpand";
    pub const TREE_ELEMENT_HAS_PARENT: &str = "treeElementHasParent";
    pub const TREE_ELEMENT_HAS_CHILD: &str = "treeElementHasChild";
    pub const TREE_STICKY_SCROLL_FOCUSED: &str = "treestickyScrollFocused";
    pub const INPUT_FOCUS: &str = "inputFocus";
    pub const IN_QUICK_INPUT: &str = "inQuickInput";
    pub const QUICK_INPUT_TYPE: &str = "quickInputType";
    pub const CURSOR_AT_END_OF_QUICK_INPUT_BOX: &str = "cursorAtEndOfQuickInputBox";
    pub const HISTORY_NAVIGATION_WIDGET_FOCUS: &str = "historyNavigationWidgetFocus";
    pub const HISTORY_NAVIGATION_BACKWARDS_ENABLED: &str = "historyNavigationBackwardsEnabled";
    pub const HISTORY_NAVIGATION_FORWARDS_ENABLED: &str = "historyNavigationForwardsEnabled";
}

/// `list.*` command IDs (KEYBINDINGS.md 通用列表) plus `list.triggerTypeNavigation`, which has no
/// default key but is required by `workbench.list.typeNavigationMode = trigger`.
pub const LIST_COMMANDS: &[(&str, &str)] = &[
    ("list.clear", "清除选择"),
    ("list.closeFind", "关闭查找"),
    ("list.collapse", "折叠"),
    ("list.collapseAll", "全部折叠"),
    ("list.expand", "展开"),
    ("list.expandSelectionDown", "向下扩展选择"),
    ("list.expandSelectionUp", "向上扩展选择"),
    ("list.find", "查找"),
    ("list.focusAnyDown", "聚焦下一项（不打开）"),
    ("list.focusAnyFirst", "聚焦第一项（不打开）"),
    ("list.focusAnyLast", "聚焦最后一项（不打开）"),
    ("list.focusAnyUp", "聚焦上一项（不打开）"),
    ("list.focusDown", "聚焦下一项"),
    ("list.focusFirst", "聚焦第一项"),
    ("list.focusLast", "聚焦最后一项"),
    ("list.focusPageDown", "向下翻页聚焦"),
    ("list.focusPageUp", "向上翻页聚焦"),
    ("list.focusUp", "聚焦上一项"),
    ("list.scrollDown", "向下滚动"),
    ("list.scrollUp", "向上滚动"),
    ("list.select", "选择"),
    ("list.selectAll", "全选"),
    ("list.showHover", "显示悬停"),
    ("list.stickyScroll.collapse", "折叠粘性项"),
    ("list.stickyScrollselect", "选择粘性项"),
    ("list.stickyScrolltoggleExpand", "切换粘性项展开"),
    ("list.toggleExpand", "切换展开"),
    ("list.toggleSelection", "切换选择"),
    ("list.triggerTypeNavigation", "触发键入导航"),
    ("rove.list.contextMenu", "显示上下文菜单"),
];

/// `quickInput.*` IDs (KEYBINDINGS.md 快速访问).
pub const QUICK_INPUT_COMMANDS: &[(&str, &str)] = &[
    ("quickInput.accept", "接受"),
    ("quickInput.acceptInBackground", "在后台接受"),
    ("quickInput.first", "第一项"),
    ("quickInput.hide", "隐藏"),
    ("quickInput.last", "最后一项"),
    ("quickInput.next", "下一项"),
    ("quickInput.nextSeparator", "下一组"),
    (
        "quickInput.nextSeparatorWithQuickAccessFallback",
        "下一组（快速访问回退）",
    ),
    ("quickInput.pageNext", "下一页"),
    ("quickInput.pagePrevious", "上一页"),
    ("quickInput.previous", "上一项"),
    ("quickInput.previousSeparator", "上一组"),
    (
        "quickInput.previousSeparatorWithQuickAccessFallback",
        "上一组（快速访问回退）",
    ),
    ("quickInput.toggleCheckbox", "切换复选框"),
    ("quickInput.toggleHover", "切换悬停"),
];

/// Input history navigation shared by quick input, find widget (#21) and search (#25).
pub const HISTORY_COMMANDS: &[(&str, &str)] = &[
    ("history.showNext", "下一条历史记录"),
    ("history.showPrevious", "上一条历史记录"),
];

/// Fixed UI wording used by the components. View-specific text (empty states, loading rows) is
/// supplied by each view as a `Message`.
pub const STRINGS: &[(&str, &str)] = &[
    ("list.find.placeholder", "查找"),
    ("list.find.filterPlaceholder", "筛选"),
    ("list.find.noResults", "无结果"),
    ("list.find.toggleFilter", "筛选"),
    ("list.find.toggleFuzzy", "模糊匹配"),
    ("list.find.close", "关闭"),
    ("list.twistie.expanded", "▾"),
    ("list.twistie.collapsed", "▸"),
    ("list.checkbox.checked", "☑"),
    ("list.checkbox.unchecked", "☐"),
    ("quickInput.noResults", "没有匹配的结果"),
    ("quickInput.countSelected", "已选 {count} 项"),
    ("quickInput.ok", "确定"),
];

fn title_key(id: &str) -> String {
    format!("command.{id}")
}

/// Registers every command title string and the fixed component strings.
pub fn register_strings(table: &mut StringTable) -> Result<()> {
    for (id, title) in LIST_COMMANDS
        .iter()
        .chain(QUICK_INPUT_COMMANDS)
        .chain(HISTORY_COMMANDS)
    {
        tolerate_duplicate(table.register(title_key(id), Arc::from(*title)))?;
    }
    for (k, v) in STRINGS {
        tolerate_duplicate(table.register(*k, Arc::from(*v)))?;
    }
    Ok(())
}

pub fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, _) in LIST_COMMANDS
        .iter()
        .chain(QUICK_INPUT_COMMANDS)
        .chain(HISTORY_COMMANDS)
    {
        tolerate_duplicate(registry.register(
            *id,
            Command {
                title_key: title_key(id).into(),
                category_key: None,
            },
        ))?;
    }
    Ok(())
}

fn tolerate_duplicate(r: Result<()>) -> Result<()> {
    match r {
        Ok(()) | Err(Error::Duplicate(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

const fn hex(v: u32) -> Option<Color> {
    Some(Color(
        (v >> 24) as u8,
        (v >> 16) as u8,
        (v >> 8) as u8,
        v as u8,
    ))
}
const N: Option<Color> = None;

/// VS Code color-registry defaults (dark, light, hcDark, hcLight), RGBA. `None` = unset.
#[rustfmt::skip]
pub const COLORS: &[(&str, [Option<Color>; 4])] = &[
    ("list.activeSelectionBackground", [hex(0x04395EFF), hex(0x0060C0FF), N, hex(0x0F4A851A)]),
    ("list.activeSelectionForeground", [hex(0xFFFFFFFF), hex(0xFFFFFFFF), N, N]),
    ("list.activeSelectionIconForeground", [N, N, N, N]),
    ("list.dropBackground", [hex(0x062F4AFF), hex(0xD6EBFFFF), N, N]),
    ("list.dropBetweenBackground", [hex(0xC5C5C5FF), hex(0x424242FF), hex(0xFFFFFFFF), hex(0x292929FF)]),
    ("list.focusBackground", [N, N, N, N]),
    ("list.focusForeground", [N, N, N, N]),
    ("list.focusHighlightForeground", [hex(0x2AAAFFFF), hex(0xBBE7FFFF), hex(0xF38518FF), hex(0x006BBDFF)]),
    ("list.focusOutline", [hex(0x007FD4FF), hex(0x0090F1FF), hex(0xF38518FF), hex(0x006BBDFF)]),
    ("list.focusAndSelectionOutline", [N, N, N, N]),
    ("list.highlightForeground", [hex(0x2AAAFFFF), hex(0x0066BFFF), hex(0xF38518FF), hex(0x006BBDFF)]),
    ("list.hoverBackground", [hex(0x2A2D2EFF), hex(0xF0F0F0FF), hex(0xFFFFFF1A), hex(0x0F4A851A)]),
    ("list.hoverForeground", [N, N, N, N]),
    ("list.inactiveSelectionBackground", [hex(0x37373DFF), hex(0xE4E6F1FF), N, hex(0x0F4A851A)]),
    ("list.inactiveSelectionForeground", [N, N, N, N]),
    ("list.inactiveSelectionIconForeground", [N, N, N, N]),
    ("list.inactiveFocusBackground", [N, N, N, N]),
    ("list.inactiveFocusOutline", [N, N, N, N]),
    ("list.invalidItemForeground", [hex(0xB89500FF), hex(0xB89500FF), hex(0xB89500FF), hex(0xB5200DFF)]),
    ("list.errorForeground", [hex(0xF88070FF), hex(0xB01011FF), N, N]),
    ("list.warningForeground", [hex(0xCCA700FF), hex(0x855F00FF), N, N]),
    ("listFilterWidget.background", [hex(0x252526FF), hex(0xF3F3F3FF), hex(0x0C141FFF), hex(0xFFFFFFFF)]),
    ("listFilterWidget.outline", [hex(0x00000000), hex(0x00000000), hex(0xF38518FF), hex(0x0F4A85FF)]),
    ("listFilterWidget.noMatchesOutline", [hex(0xBE1100FF), hex(0xBE1100FF), hex(0x6FC3DFFF), hex(0x0F4A85FF)]),
    ("listFilterWidget.shadow", [hex(0x0000005C), hex(0x00000029), N, N]),
    ("list.filterMatchBackground", [hex(0xEA5C0055), hex(0xEA5C0055), N, N]),
    ("list.filterMatchBorder", [N, N, hex(0xF38518FF), hex(0x0F4A85FF)]),
    ("list.deemphasizedForeground", [hex(0x8C8C8CFF), hex(0x8E8E90FF), hex(0xA7A8A9FF), hex(0x666666FF)]),
    ("tree.indentGuidesStroke", [hex(0x585858FF), hex(0xA9A9A9FF), hex(0xA9A9A9FF), hex(0xA5A5A5FF)]),
    ("tree.inactiveIndentGuidesStroke", [hex(0x58585866), hex(0xA9A9A966), hex(0xA9A9A966), hex(0xA5A5A566)]),
    ("tree.tableColumnsBorder", [hex(0xCCCCCC20), hex(0x61616120), N, N]),
    ("tree.tableOddRowsBackground", [hex(0xCCCCCC0A), hex(0x6161610A), N, N]),
    ("pickerGroup.border", [hex(0x3F3F46FF), hex(0xCCCEDBFF), hex(0xFFFFFFFF), hex(0x0F4A85FF)]),
    ("pickerGroup.foreground", [hex(0x3794FFFF), hex(0x0066BFFF), hex(0xFFFFFFFF), hex(0x0F4A85FF)]),
    ("quickInput.background", [hex(0x252526FF), hex(0xF3F3F3FF), hex(0x0C141FFF), hex(0xFFFFFFFF)]),
    ("quickInput.foreground", [hex(0xCCCCCCFF), hex(0x616161FF), hex(0xFFFFFFFF), hex(0x292929FF)]),
    ("quickInputList.focusBackground", [hex(0x04395EFF), hex(0x0060C0FF), N, hex(0x0F4A851A)]),
    ("quickInputList.focusForeground", [hex(0xFFFFFFFF), hex(0xFFFFFFFF), N, N]),
    ("quickInputList.focusIconForeground", [hex(0xFFFFFFFF), hex(0xFFFFFFFF), N, N]),
    ("quickInputTitle.background", [hex(0xFFFFFF1B), hex(0x0000000F), hex(0x000000FF), hex(0xFFFFFFFF)]),
    ("input.background", [hex(0x3C3C3CFF), hex(0xFFFFFFFF), hex(0x000000FF), hex(0xFFFFFFFF)]),
    ("input.border", [N, N, hex(0x6FC3DFFF), hex(0x0F4A85FF)]),
    ("input.foreground", [hex(0xCCCCCCFF), hex(0x616161FF), hex(0xFFFFFFFF), hex(0x292929FF)]),
    ("input.placeholderForeground", [hex(0xA6A6A6FF), hex(0x767676FF), hex(0xFFFFFFB3), hex(0x292929B3)]),
    ("inputValidation.errorBackground", [hex(0x5A1D1DFF), hex(0xF2DEDEFF), hex(0x000000FF), hex(0xFFFFFFFF)]),
    ("inputValidation.errorForeground", [N, N, N, N]),
    ("inputValidation.errorBorder", [hex(0xBE1100FF), hex(0xBE1100FF), hex(0x6FC3DFFF), hex(0x0F4A85FF)]),
    ("inputValidation.infoBackground", [hex(0x063B49FF), hex(0xD6ECF2FF), hex(0x000000FF), hex(0xFFFFFFFF)]),
    ("inputValidation.infoForeground", [N, N, N, N]),
    ("inputValidation.infoBorder", [hex(0x007ACCFF), hex(0x007ACCFF), hex(0x6FC3DFFF), hex(0x0F4A85FF)]),
    ("inputValidation.warningBackground", [hex(0x352A05FF), hex(0xF6F5D2FF), hex(0x000000FF), hex(0xFFFFFFFF)]),
    ("inputValidation.warningForeground", [N, N, N, N]),
    ("inputValidation.warningBorder", [hex(0xB89500FF), hex(0xB89500FF), hex(0x6FC3DFFF), hex(0x0F4A85FF)]),
];

pub fn register_colors(registry: &mut ColorRegistry) -> Result<()> {
    for (key, [dark, light, high_contrast_dark, high_contrast_light]) in COLORS {
        tolerate_duplicate(registry.register(
            *key,
            ColorDefaults {
                dark: *dark,
                light: *light,
                high_contrast_dark: *high_contrast_dark,
                high_contrast_light: *high_contrast_light,
            },
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::{Message, Strings};

    #[test]
    fn every_bound_command_is_registered_with_a_title() {
        let mut commands = CommandRegistry::default();
        let mut strings = StringTable::default();
        register_commands(&mut commands).unwrap();
        register_strings(&mut strings).unwrap();
        // Idempotent against central owners registering first.
        register_commands(&mut commands).unwrap();
        for (id, c) in commands.entries() {
            strings
                .text(&Message::key(c.title_key.clone()))
                .unwrap_or_else(|_| panic!("{id}"));
        }
    }

    #[test]
    fn colors_cover_lists_trees_and_quick_picker() {
        let mut colors = ColorRegistry::default();
        register_colors(&mut colors).unwrap();
        register_colors(&mut colors).unwrap();
        assert_eq!(colors.entries().count(), COLORS.len());
        assert_eq!(
            colors.get("list.activeSelectionBackground").unwrap().dark,
            Some(Color(0x04, 0x39, 0x5e, 0xff))
        );
    }
}
