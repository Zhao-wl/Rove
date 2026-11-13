//! 资源管理器的登记项（#23）：命令（VS Code ID）、界面字符串、上下文键与命令的 GPUI action。
//!
//! - 设置项（`explorer.*`、`workbench.editor.decorations.*`）在 `rove_config` 的内置目录里；
//! - 颜色键（`symbolIcon.fileForeground` / `folderForeground`、`list.*`、`sideBar*`）是 #10 登记的 P0 键，
//!   `gitDecoration.*` 按 THEME_COLORS 静默忽略；
//! - 视图小节 `workbench.explorer.fileView` 与容器由 #16 的 `shell::catalog` 登记，默认键位来自 #14 的
//!   VS Code 默认键位表（`enter`、`ctrl+enter`、`space`、`home` / `end` / `left` / `right`、`ctrl+shift+e`），
//!   这里都不重复登记；
//! - **不登记**文件操作：`renameFile`、`deleteFile`、`moveFileToTrash`、`filesExplorer.copy` / `cut` / `paste`、
//!   `copyFilePath`、`revealFileInOS`、新建与拖放（R62）。
use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use std::sync::Arc;

pub const OPEN_AND_PASS_FOCUS: &str = "explorer.openAndPassFocus";
pub const OPEN_TO_SIDE: &str = "explorer.openToSide";
pub const OPEN_PRESERVE_FOCUS: &str = "filesExplorer.openFilePreserveFocus";
pub const FIRST_COMPRESSED: &str = "firstCompressedFolder";
pub const LAST_COMPRESSED: &str = "lastCompressedFolder";
pub const NEXT_COMPRESSED: &str = "nextCompressedFolder";
pub const PREVIOUS_COMPRESSED: &str = "previousCompressedFolder";
pub const SHOW_ACTIVE_FILE: &str = "workbench.files.action.showActiveFileInExplorer";
/// 空状态按钮执行的命令（对话框由 #28 / #16 提供）。
pub const OPEN_FOLDER: &str = "workbench.action.files.openFolder";

/// VS Code 上下文键（默认键位的 when 子句用到）。
pub mod context {
    pub const FILES_EXPLORER_FOCUS: &str = "filesExplorerFocus";
    pub const VIEWLET_FOCUS: &str = "explorerViewletFocus";
    pub const FOLDERS_VIEW_VISIBLE: &str = "foldersViewVisible";
    pub const RESOURCE_IS_FOLDER: &str = "explorerResourceIsFolder";
    pub const RESOURCE_IS_ROOT: &str = "explorerResourceIsRoot";
    pub const RESOURCE_READONLY: &str = "explorerResourceReadonly";
    pub const COMPRESSED_FOCUS: &str = "explorerViewletCompressedFocus";
    pub const COMPRESSED_FIRST_FOCUS: &str = "explorerViewletCompressedFirstFocus";
    pub const COMPRESSED_LAST_FOCUS: &str = "explorerViewletCompressedLastFocus";
}

/// (命令 ID, 标题字符串 key, 中文标题)
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (OPEN_AND_PASS_FOCUS, "explorer.command.open", "打开"),
    (OPEN_TO_SIDE, "explorer.command.openToSide", "在侧边打开"),
    (
        OPEN_PRESERVE_FOCUS,
        "explorer.command.openPreserveFocus",
        "打开文件并保持焦点",
    ),
    (
        FIRST_COMPRESSED,
        "explorer.command.firstCompressed",
        "转到紧凑文件夹中的第一个文件夹",
    ),
    (
        LAST_COMPRESSED,
        "explorer.command.lastCompressed",
        "转到紧凑文件夹中的最后一个文件夹",
    ),
    (
        NEXT_COMPRESSED,
        "explorer.command.nextCompressed",
        "转到紧凑文件夹中的下一个文件夹",
    ),
    (
        PREVIOUS_COMPRESSED,
        "explorer.command.previousCompressed",
        "转到紧凑文件夹中的上一个文件夹",
    ),
    (
        SHOW_ACTIVE_FILE,
        "explorer.command.showActiveFile",
        "在资源管理器中显示活动文件",
    ),
];

/// 界面字符串。
pub const STRINGS: &[(&str, &str)] = &[
    ("explorer.empty", "尚未打开文件夹。"),
    ("explorer.openFolder", "打开文件夹"),
    ("explorer.loading", "正在加载…"),
    ("explorer.readFailed.permission", "没有权限读取此文件夹。"),
    (
        "explorer.readFailed.missing",
        "此文件夹已不存在或无法读取。",
    ),
    (
        "explorer.readFailed.tooLong",
        "路径太长，无法读取此文件夹。",
    ),
    ("explorer.readFailed.other", "无法读取此文件夹。"),
    ("explorer.badge.readonly", "只读"),
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

/// 本模块命令的 GPUI action；键位服务按 when 解析后分发，资源管理器（或宿主）处理。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = explorer, no_json)]
pub struct ExplorerAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(ExplorerAction(cmd)) as Box<dyn Action>)
}
