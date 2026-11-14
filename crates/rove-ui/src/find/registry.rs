//! 查找条的登记项（#21、#48）：命令（VS Code ID）、界面字符串、上下文键名与颜色键。
//!
//! - 设置项 `editor.find.*`（含 #48 的 `editor.find.replaceHistory`）与 `editor.wordSeparators` 由
//!   `rove_config::register_builtin` 按 compat/SETTINGS.md 登记，这里只读（[`super::host::FindSettings`]）；
//! - 默认键位来自 #14 生成的 VS Code 默认键位表（`ctrl+f`、`f3`、`alt+c`、`ctrl+h`、`ctrl+alt+enter`、
//!   `ctrl+shift+1`、`alt+p` 等），不重复登记；
//! - #48 登记四个替换命令（`editor.action.startFindReplaceAction` / `replaceOne` / `replaceAll` /
//!   `togglePreserveCase`）与替换行的界面字符串，**不新增颜色键**（复用查找条已登记的
//!   `editorWidget.*`、`input.*`、`inputOption.active*`）；
//! - **不登记** `editor.action.selectAllMatches`（随多光标 #52）与跨文件替换（#62）的命令。

use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use rove_theme::{Color, ColorRegistry};
use std::sync::Arc;

pub const FIND: &str = "actions.find";
pub const FIND_WITH_SELECTION: &str = "actions.findWithSelection";
pub const CLOSE: &str = "closeFindWidget";
pub const NEXT: &str = "editor.action.nextMatchFindAction";
pub const PREVIOUS: &str = "editor.action.previousMatchFindAction";
pub const NEXT_SELECTION: &str = "editor.action.nextSelectionMatchFindAction";
pub const PREVIOUS_SELECTION: &str = "editor.action.previousSelectionMatchFindAction";
pub const TOGGLE_CASE: &str = "toggleFindCaseSensitive";
pub const TOGGLE_REGEX: &str = "toggleFindRegex";
pub const TOGGLE_WHOLE_WORD: &str = "toggleFindWholeWord";
pub const TOGGLE_IN_SELECTION: &str = "toggleFindInSelection";
// #48：替换。
pub const START_REPLACE: &str = "editor.action.startFindReplaceAction";
pub const REPLACE_ONE: &str = "editor.action.replaceOne";
pub const REPLACE_ALL: &str = "editor.action.replaceAll";
pub const TOGGLE_PRESERVE_CASE: &str = "togglePreserveCase";
/// #52：`alt+enter`，查找条可见时选中全部匹配（#21 把它留给了多光标）。
pub const SELECT_ALL_MATCHES: &str = "editor.action.selectAllMatches";

/// 发布在查找条上的 VS Code 上下文键（KEYBINDINGS.md 的 when 子句用到）。
pub mod context {
    pub const WIDGET_VISIBLE: &str = "findWidgetVisible";
    pub const INPUT_FOCUSSED: &str = "findInputFocussed";
    /// #48：替换输入框有焦点（框内 `enter` = 替换当前匹配）。与 `findInputFocussed` 互斥：
    /// 哪个输入框有焦点就只发布哪一个，否则两条 `enter` 规则会同时命中。
    pub const REPLACE_INPUT_FOCUSSED: &str = "replaceInputFocussed";
}

/// (命令 ID, 标题字符串 key, 中文标题)。
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (FIND, "find.command.find", "查找"),
    (
        FIND_WITH_SELECTION,
        "find.command.findWithSelection",
        "用选中内容查找",
    ),
    (CLOSE, "find.command.close", "关闭查找小组件"),
    (NEXT, "find.command.next", "查找下一个"),
    (PREVIOUS, "find.command.previous", "查找上一个"),
    (
        NEXT_SELECTION,
        "find.command.nextSelection",
        "查找下一个选择",
    ),
    (
        PREVIOUS_SELECTION,
        "find.command.previousSelection",
        "查找上一个选择",
    ),
    (TOGGLE_CASE, "find.command.toggleCase", "切换区分大小写"),
    (TOGGLE_REGEX, "find.command.toggleRegex", "切换正则表达式"),
    (
        TOGGLE_WHOLE_WORD,
        "find.command.toggleWholeWord",
        "切换全字匹配",
    ),
    (
        TOGGLE_IN_SELECTION,
        "find.command.toggleInSelection",
        "切换在选定内容中查找",
    ),
    // #48：替换。
    (START_REPLACE, "find.command.startReplace", "替换"),
    (REPLACE_ONE, "find.command.replaceOne", "替换当前匹配项"),
    (REPLACE_ALL, "find.command.replaceAll", "全部替换"),
    (
        TOGGLE_PRESERVE_CASE,
        "find.command.togglePreserveCase",
        "切换保留大小写",
    ),
    // #52：选中全部匹配（`alt+enter`）。
    (
        SELECT_ALL_MATCHES,
        "find.command.selectAllMatches",
        "选择所有匹配项",
    ),
];

/// 命令分类（命令面板里显示为“编辑器: …”，与代码视图共用）。
pub const CATEGORY_KEY: &str = "codeView.category";

/// 其余界面字符串。
pub const STRINGS: &[(&str, &str)] = &[
    ("find.placeholder", "查找"),
    ("find.toggle.case", "区分大小写"),
    ("find.toggle.wholeWord", "全字匹配"),
    ("find.toggle.regex", "使用正则表达式"),
    ("find.toggle.inSelection", "在选定内容中查找"),
    ("find.button.previous", "上一个匹配项"),
    ("find.button.next", "下一个匹配项"),
    ("find.button.close", "关闭"),
    ("find.count.none", "无结果"),
    ("find.count.of", "第 {index} 项，共 {total} 项"),
    ("find.count.total", "共 {total} 项"),
    ("find.count.overLimit", "超过 {limit} 项结果"),
    ("find.count.found", "已找到 {found} 处"),
    (
        "find.count.searching",
        "已找到 {found} 处 · 已搜索 {percent}%",
    ),
    // #48：替换行。
    ("find.replace.placeholder", "替换"),
    ("find.toggle.preserveCase", "保留大小写"),
    ("find.button.replace", "替换"),
    ("find.button.replaceAll", "全部替换"),
    ("find.button.toggleReplace", "切换替换"),
    ("find.error.regex", "正则表达式无效：{reason}"),
    (
        "find.error.tooLarge",
        "文件太大，无法用跨行正则表达式查找。",
    ),
    ("find.tooltip.withKey", "{label} ({key})"),
];

/// 查找条用到的颜色键（均为 VS Code 颜色，默认值取 #10 的默认值表；主题服务的 P0 表已含其中大多数）。
pub const COLORS: &[&str] = &[
    "editor.findMatchBackground",
    "editor.findMatchForeground",
    "editor.findMatchBorder",
    "editor.findMatchHighlightBackground",
    "editor.findMatchHighlightForeground",
    "editor.findMatchHighlightBorder",
    "editor.findRangeHighlightBackground",
    "editor.findRangeHighlightBorder",
    "editorWidget.background",
    "editorWidget.border",
    "editorWidget.foreground",
    "editorWidget.resizeBorder",
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
    "inputValidation.errorForeground",
    "widget.shadow",
    "focusBorder",
    "toolbar.hoverBackground",
    "foreground",
    "errorForeground",
    "list.focusOutline",
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

/// 查找条的调色板；主题缺键时用默认值表。`None` 表示该键明确不设色。
#[derive(Clone, Debug, PartialEq)]
pub struct FindColors {
    pub match_background: Option<Color>,
    pub match_foreground: Option<Color>,
    pub match_border: Option<Color>,
    pub highlight_background: Option<Color>,
    pub highlight_foreground: Option<Color>,
    pub highlight_border: Option<Color>,
    pub range_background: Option<Color>,
    pub range_border: Option<Color>,
    pub widget_background: Option<Color>,
    pub widget_border: Option<Color>,
    pub widget_foreground: Option<Color>,
    pub shadow: Option<Color>,
    pub input_background: Option<Color>,
    pub input_border: Option<Color>,
    pub input_foreground: Option<Color>,
    pub input_placeholder: Option<Color>,
    pub option_active_background: Option<Color>,
    pub option_active_border: Option<Color>,
    pub option_active_foreground: Option<Color>,
    pub option_hover_background: Option<Color>,
    pub error_background: Option<Color>,
    pub error_border: Option<Color>,
    pub error_foreground: Option<Color>,
    pub focus_border: Option<Color>,
    pub hover_background: Option<Color>,
    pub foreground: Option<Color>,
}

impl FindColors {
    pub fn resolve(theme: &dyn rove_theme::Theme, kind: rove_theme::ThemeKind) -> Self {
        let get = |key: &str| match theme.color(key) {
            Ok(c) => c,
            Err(_) => rove_theme::defaults::static_defaults(key).and_then(|d| d.for_kind(kind)),
        };
        Self {
            match_background: get("editor.findMatchBackground"),
            match_foreground: get("editor.findMatchForeground"),
            match_border: get("editor.findMatchBorder"),
            highlight_background: get("editor.findMatchHighlightBackground"),
            highlight_foreground: get("editor.findMatchHighlightForeground"),
            highlight_border: get("editor.findMatchHighlightBorder"),
            range_background: get("editor.findRangeHighlightBackground"),
            range_border: get("editor.findRangeHighlightBorder"),
            widget_background: get("editorWidget.background"),
            widget_border: get("editorWidget.border"),
            widget_foreground: get("editorWidget.foreground"),
            shadow: get("widget.shadow"),
            input_background: get("input.background"),
            input_border: get("input.border"),
            input_foreground: get("input.foreground"),
            input_placeholder: get("input.placeholderForeground"),
            option_active_background: get("inputOption.activeBackground"),
            option_active_border: get("inputOption.activeBorder"),
            option_active_foreground: get("inputOption.activeForeground"),
            option_hover_background: get("inputOption.hoverBackground"),
            error_background: get("inputValidation.errorBackground"),
            error_border: get("inputValidation.errorBorder"),
            error_foreground: get("inputValidation.errorForeground").or(get("foreground")),
            focus_border: get("focusBorder"),
            hover_background: get("toolbar.hoverBackground"),
            foreground: get("foreground"),
        }
    }
}

/// 本模块命令的 GPUI action；由查找条宿主处理，键位服务按 when 解析后分发。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = find, no_json)]
pub struct FindAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(FindAction(cmd)) as Box<dyn Action>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_find_and_replace_commands() {
        let mut r = CommandRegistry::default();
        register_commands(&mut r).unwrap();
        register_commands(&mut r).unwrap();
        let mut t = StringTable::default();
        register_strings(&mut t).unwrap();
        for (id, title, _) in COMMANDS {
            assert!(r.get(id).is_some());
            assert!(t.get(title).is_some(), "{title}");
            assert!(action_for(id).is_some());
        }
        for (key, _) in STRINGS {
            assert!(t.get(key).is_some());
        }
        // #48 的四个替换入口都在；跨文件替换（#62）与多选区（#52）的命令仍然不登记。
        for id in [
            START_REPLACE,
            REPLACE_ONE,
            REPLACE_ALL,
            TOGGLE_PRESERVE_CASE,
        ] {
            assert!(r.get(id).is_some(), "{id}");
        }
        for (id, _) in r.entries() {
            assert!(!id.starts_with("search.action.replace"), "{id}");
            assert_ne!(id, "workbench.action.replaceInFiles");
            assert_ne!(id, "closeReplaceInFilesWidget");
        }
        // #52 接手后 `editor.action.selectAllMatches` 由查找条登记。
        assert!(r.get(SELECT_ALL_MATCHES).is_some());
    }

    #[test]
    fn every_color_key_resolves_to_a_default() {
        let mut registry = ColorRegistry::default();
        register_colors(&mut registry).unwrap();
        for key in COLORS {
            assert!(
                registry.get(key).is_some() || rove_theme::defaults::static_defaults(key).is_some(),
                "{key}"
            );
        }
    }
}
