//! 资源管理器的登记项（#23）：命令（VS Code ID）、界面字符串、上下文键与命令的 GPUI action。
//!
//! - 设置项（`explorer.*`、`workbench.editor.decorations.*`）在 `rove_config` 的内置目录里；
//! - 颜色键（`symbolIcon.fileForeground` / `folderForeground`、`list.*`、`sideBar*`）是 #10 登记的 P0 键，
//!   `gitDecoration.*` 按 THEME_COLORS 静默忽略；
//! - 视图小节 `workbench.explorer.fileView` 与容器由 #16 的 `shell::catalog` 登记，默认键位来自 #14 的
//!   VS Code 默认键位表（`enter`、`ctrl+enter`、`space`、`home` / `end` / `left` / `right`、`ctrl+shift+e`），
//!   这里都不重复登记；
//! - #63 的文件操作命令与反馈同样由此登记。
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

pub const COPY_PATH: &str = "copyFilePath";
pub const COPY_RELATIVE_PATH: &str = "copyRelativeFilePath";
pub const COPY_ACTIVE_PATH: &str = "workbench.action.files.copyPathOfActiveFile";
pub const REVEAL_PATH: &str = "revealFileInOS";
pub const REVEAL_ACTIVE_PATH: &str = "workbench.action.files.revealActiveFileInWindows";
pub const REVEAL_FAILED: &str = "explorer.reveal.failed";

pub const NEW_FILE: &str = "explorer.newFile";
pub const NEW_FOLDER: &str = "explorer.newFolder";
pub const CREATE_FILE: &str = "workbench.files.action.createFileFromExplorer";
pub const CREATE_FOLDER: &str = "workbench.files.action.createFolderFromExplorer";
pub const RENAME_FILE: &str = "renameFile";
pub const REFRESH_FILES: &str = "workbench.files.action.refreshFilesExplorer";

pub fn is_name_command(id: &str) -> bool {
    matches!(
        id,
        NEW_FILE | NEW_FOLDER | CREATE_FILE | CREATE_FOLDER | RENAME_FILE
    )
}

pub const COPY_FILES: &str = "filesExplorer.copy";
pub const CUT_FILES: &str = "filesExplorer.cut";
pub const PASTE_FILES: &str = "filesExplorer.paste";
pub const CANCEL_CUT: &str = "filesExplorer.cancelCut";

pub fn is_clipboard_command(id: &str) -> bool {
    cfg!(windows) && matches!(id, COPY_FILES | CUT_FILES | PASTE_FILES | CANCEL_CUT)
}

pub const DELETE_FILE: &str = "deleteFile";
pub const MOVE_TO_TRASH: &str = "moveFileToTrash";
pub const UNDO_FILE_OPERATION: &str = "undo";
pub const FILE_OP_CONFIRM: &str = "explorer.operation.confirm";
pub const FILE_OP_CANCEL: &str = "explorer.operation.cancel";
pub const FILE_OP_REMEMBER: &str = "explorer.operation.remember";
pub const FILE_OP_FEEDBACK_PREFIX: &str = "explorer.fileOperation.";

#[derive(Clone, Copy)]
pub enum MenuWhen {
    Create,
    Item,
    Writable,
    Recycle,
    Permanent,
}

pub const FILE_CONTEXT_MENU: &[(&str, MenuWhen)] = &[
    (NEW_FILE, MenuWhen::Create),
    (NEW_FOLDER, MenuWhen::Create),
    (RENAME_FILE, MenuWhen::Writable),
    (MOVE_TO_TRASH, MenuWhen::Recycle),
    (DELETE_FILE, MenuWhen::Permanent),
    #[cfg(windows)]
    (COPY_FILES, MenuWhen::Item),
    #[cfg(windows)]
    (CUT_FILES, MenuWhen::Writable),
    #[cfg(windows)]
    (PASTE_FILES, MenuWhen::Create),
    (COPY_PATH, MenuWhen::Item),
    (COPY_RELATIVE_PATH, MenuWhen::Item),
    (REVEAL_PATH, MenuWhen::Item),
];

/// VS Code 上下文键（默认键位的 when 子句用到）。
pub mod context {
    pub const FILES_EXPLORER_FOCUS: &str = "filesExplorerFocus";
    pub const VIEWLET_FOCUS: &str = "explorerViewletFocus";
    pub const FOLDERS_VIEW_VISIBLE: &str = "foldersViewVisible";
    pub const RESOURCE_IS_FOLDER: &str = "explorerResourceIsFolder";
    pub const RESOURCE_IS_ROOT: &str = "explorerResourceIsRoot";
    pub const RESOURCE_READONLY: &str = "explorerResourceReadonly";
    pub const RESOURCE_TRASHABLE: &str = "explorerResourceMoveableToTrash";
    pub const RESOURCE_CUT: &str = "explorerResourceCut";
    pub const COMPRESSED_FOCUS: &str = "explorerViewletCompressedFocus";
    pub const COMPRESSED_FIRST_FOCUS: &str = "explorerViewletCompressedFirstFocus";
    pub const COMPRESSED_LAST_FOCUS: &str = "explorerViewletCompressedLastFocus";
}

/// (命令 ID, 标题字符串 key, 中文标题)
pub const COMMANDS: &[(&str, &str, &str)] = &[
    #[cfg(windows)]
    (COPY_FILES, "explorer.command.copyFiles", "复制"),
    #[cfg(windows)]
    (CUT_FILES, "explorer.command.cutFiles", "剪切"),
    #[cfg(windows)]
    (PASTE_FILES, "explorer.command.pasteFiles", "粘贴"),
    #[cfg(windows)]
    (CANCEL_CUT, "explorer.command.cancelCut", "取消剪切"),
    (DELETE_FILE, "explorer.command.delete", "永久删除"),
    (MOVE_TO_TRASH, "explorer.command.trash", "移到回收站"),
    (NEW_FILE, "explorer.command.newFile", "新建文件"),
    (NEW_FOLDER, "explorer.command.newFolder", "新建文件夹"),
    (
        CREATE_FILE,
        "explorer.command.createFile",
        "在资源管理器中新建文件",
    ),
    (
        CREATE_FOLDER,
        "explorer.command.createFolder",
        "在资源管理器中新建文件夹",
    ),
    (RENAME_FILE, "explorer.command.rename", "重命名"),
    (REFRESH_FILES, "explorer.command.refresh", "刷新资源管理器"),
    (COPY_PATH, "explorer.command.copyPath", "复制路径"),
    (
        COPY_RELATIVE_PATH,
        "explorer.command.copyRelativePath",
        "复制相对路径",
    ),
    (
        COPY_ACTIVE_PATH,
        "explorer.command.copyActivePath",
        "复制活动文件的路径",
    ),
    (
        REVEAL_PATH,
        "explorer.command.revealInOs",
        "在系统资源管理器中显示",
    ),
    (
        REVEAL_ACTIVE_PATH,
        "explorer.command.revealActiveInOs",
        "在系统资源管理器中显示活动文件",
    ),
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
    ("explorer.paste.native", "将系统剪贴板里的 {count} 个项目粘贴到当前目录吗？"),
    ("explorer.paste.replace", "此操作将移动或粘贴 {count} 个项目，并替换其中 {replace} 个同名项目。原项目会进入回收站。继续吗？"),
    ("explorer.drop.confirm", "将 {count} 个项目移动到所选目录吗？"),
    ("explorer.operation.continue", "继续"),
    ("explorer.delete.recycle", "确定将 {count} 个项目移到回收站吗？"),
    ("explorer.delete.permanent", "确定永久删除 {permanent} 个项目吗？此操作无法撤销。"),
    ("explorer.delete.fallback", "所选 {count} 个项目中有 {permanent} 个无法移到回收站。确认后将永久删除这些项目，且无法撤销。"),
    ("explorer.delete.recycleButton", "移到回收站"),
    ("explorer.delete.permanentButton", "永久删除"),
    ("explorer.operation.cancel", "取消"),
    ("explorer.operation.neverAsk", "不再询问"),
    ("explorer.operation.retry", "重试"),
    ("explorer.operation.failed", "未能完成文件操作。请检查路径、文件占用及权限后重试。"),
    ("explorer.operation.busy", "所选文件的保存正在进行。请等待保存结束后重试。"),
    ("explorer.operation.locked", "项目正被其他程序占用。请关闭占用它的程序后重试。"),
    ("explorer.operation.long", "项目的路径太长。请缩短名称或移动到更短的路径后重试。"),
    ("explorer.operation.denied", "没有权限更改所选项目。请检查访问权限后重试。"),
    ("explorer.operation.sourceRetained", "项目已复制，但未能移除源项目。源文件仍保留在原位置。"),
    ("explorer.undo.restore", "撤销上一次资源管理器文件操作吗？"),
    ("explorer.undo.restoreButton", "还原"),
    ("explorer.name.empty", "请输入文件或文件夹名称。"),
    (
        "explorer.name.device",
        "此名称是 Windows 保留的设备名。请换一个名称。",
    ),
    ("explorer.name.invalid", "名称包含不允许的字符或路径段。"),
    ("explorer.name.exists", "同名项目已存在。请换一个名称。"),
    (
        "explorer.name.denied",
        "当前目录没有写入权限。检查权限后按 Enter 重试。",
    ),
    ("explorer.name.readonly", "该路径受工作区只读规则保护。"),
    (
        "explorer.name.locked",
        "项目正在被其他程序使用。关闭它后按 Enter 重试。",
    ),
    ("explorer.name.long", "文件名或路径太长。请缩短后重试。"),
    (
        "explorer.name.other",
        "操作失败。请检查该位置，或按 Enter 重试。",
    ),
    (
        "explorer.name.recovery",
        "原项目仍在同一目录的临时名称下。请检查目录。",
    ),
    (
        REVEAL_FAILED,
        "无法在系统资源管理器中显示此文件。请检查该路径是否仍然可用。",
    ),
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
    ("explorer.branch.title", "分支筛选"),
    ("explorer.branch.only", "只看分支变更"),
    ("explorer.branch.author", "提交者"),
    ("explorer.branch.commit", "分支提交"),
    ("explorer.branch.allAuthors", "全部提交者"),
    ("explorer.branch.whole", "整个分支"),
    ("explorer.branch.parent", "相对父提交"),
    ("explorer.branch.base", "相对基准分支的分叉点 · 仅已提交"),
    ("explorer.branch.clear", "清除"),
    ("explorer.branch.files", "个文件"),
    ("explorer.branch.empty", "没有匹配的文件。"),
    (
        "explorer.branch.unavailable",
        "无法读取当前分支或比较基准。",
    ),
    ("explorer.branch.deleted", "该文件在所选范围内已删除。"),
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
