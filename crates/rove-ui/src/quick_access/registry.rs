//! 快速打开与命令面板的登记项：命令（VS Code ID）、界面字符串、GPUI action 与设置读取。
//! 设置项（`search.quickOpen.*`、`workbench.quickOpen.*`）由 `rove_config` 的目录登记；颜色键
//! （`quickInput*`、`list.highlightForeground`……）由 #18 登记；默认键位来自 #14 生成的 VS Code 默认键位表
//! （`ctrl+p`、`ctrl+e`、`ctrl+shift+p`、`f1`、`ctrl+g`），这里都不重复登记。
use gpui::Action;
use rove_base::{Error, Result, Value};
use rove_commands::{Command, CommandRegistry};
use rove_config::{Scope, Settings};
use rove_i18n::StringTable;
use std::sync::Arc;

pub const QUICK_OPEN: &str = "workbench.action.quickOpen";
pub const SHOW_COMMANDS: &str = "workbench.action.showCommands";
pub const GOTO_LINE: &str = "workbench.action.gotoLine";

/// (命令 ID, 标题字符串 key, 中文标题)
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (QUICK_OPEN, "commands.quickOpen", "转到文件..."),
    (SHOW_COMMANDS, "commands.palette", "显示所有命令"),
    (GOTO_LINE, "commands.gotoLine", "转到行/列..."),
];

pub const STRINGS: &[(&str, &str)] = &[
    (
        "quickAccess.files.placeholder",
        "通过名称搜索文件(追加 : 转到行或追加 @ 转到符号)",
    ),
    ("quickAccess.commands.placeholder", "键入要运行的命令的名称"),
    ("quickAccess.line.placeholder", "键入要转到的行号"),
    ("quickAccess.symbols.placeholder", "键入要转到的符号的名称"),
    (
        "quickAccess.workspaceSymbols.placeholder",
        "按名称搜索工作区符号",
    ),
    ("quickAccess.group.recent", "最近打开"),
    ("quickAccess.group.files", "文件结果"),
    ("quickAccess.group.recentCommands", "最近使用"),
    ("quickAccess.group.otherCommands", "其他命令"),
    ("quickAccess.loading", "正在加载文件列表…"),
    (
        "quickAccess.line.prompt",
        "当前行: {line}，字符: {column}。键入要导航到的行号(介于 1 到 {max})。",
    ),
    ("quickAccess.line.goto", "转到行 {line} 和字符 {column}。"),
    ("quickAccess.line.gotoLine", "转到行 {line}。"),
    (
        "quickAccess.line.noEditor",
        "若要转到某一行，请先打开文本编辑器。",
    ),
    ("quickAccess.line.indexing", "正在建立行索引，完成后跳转"),
    (
        "quickAccess.symbols.unavailable",
        "活动编辑器无法提供大纲信息。",
    ),
    (
        "quickAccess.workspaceSymbols.unavailable",
        "工作区符号搜索尚不可用。可以用 {key} 按文件名搜索。",
    ),
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
    for (key, text) in STRINGS.iter().copied().chain(commands) {
        match table.register(key, Arc::from(text)) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 本模块命令的 GPUI action；由窗口（宿主）处理，键位服务按 when 解析后分发。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = quick_access, no_json)]
pub struct QuickAccessAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(QuickAccessAction(cmd)) as Box<dyn Action>)
}

/// 命令面板里不列出的命令：列表、快速输入、通知等内部导航命令（VS Code 里它们没有进入命令面板菜单）。
pub fn hidden_in_palette(id: &str) -> bool {
    const HIDDEN_PREFIXES: &[&str] = &[
        "list.",
        "rove.list.",
        "quickInput.",
        "history.",
        "notification.",
        "notifications.",
        "workbench.statusBar.",
        "breadcrumbs.",
        "outline.",
        "editor.createFoldingRangeFromSelection",
    ];
    const SHOWN: &[&str] = &["breadcrumbs.toggle", "breadcrumbs.focus", "outline.focus"];
    !SHOWN.contains(&id) && HIDDEN_PREFIXES.iter().any(|p| id.starts_with(p))
}

/// `search.quickOpen.*` 与 `workbench.quickOpen.*`（读取快照；无效值回退到 VS Code 默认值）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickOpenSettings {
    pub include_history: bool,
    pub history_sort_recency: bool,
    pub include_symbols: bool,
    pub close_on_focus_lost: bool,
    pub preserve_input: bool,
}

impl Default for QuickOpenSettings {
    fn default() -> Self {
        Self {
            include_history: true,
            history_sort_recency: false,
            include_symbols: false,
            close_on_focus_lost: true,
            preserve_input: false,
        }
    }
}

impl QuickOpenSettings {
    pub fn read(settings: &dyn Settings) -> Self {
        let d = Self::default();
        let scope = Scope::default();
        let boolean = |key: &str, default: bool| match settings.read(key, &scope) {
            Ok(Value::Bool(v)) => v,
            _ => default,
        };
        Self {
            include_history: boolean("search.quickOpen.includeHistory", d.include_history),
            history_sort_recency: matches!(
                settings.read("search.quickOpen.history.filterSortOrder", &scope),
                Ok(Value::String(v)) if &*v == "recency"
            ),
            include_symbols: boolean("search.quickOpen.includeSymbols", d.include_symbols),
            close_on_focus_lost: boolean(
                "workbench.quickOpen.closeOnFocusLost",
                d.close_on_focus_lost,
            ),
            preserve_input: boolean("workbench.quickOpen.preserveInput", d.preserve_input),
        }
    }
}
