//! 保存与外部修改的登记项（#49）：命令（VS Code ID）、界面字符串、反馈键与动作 ID。
//!
//! - 设置项 `files.eol` / `insertFinalNewline` / `trim*` / `saveConflictResolution` / `autoSave*` 由
//!   `rove_buffer::save::settings::register` 登记（`app_settings` 装配），这里只读；
//! - 默认键位（`ctrl+s`、`ctrl+shift+s`、`ctrl+k s`、`ctrl+k ctrl+shift+s`）来自 #14 生成的 VS Code
//!   默认键位表，不重复登记；
//! - **“比较”按钮不登记、不显示**：对比视图（P2）完成前隐藏（[`ACTION_COMPARE`] 只是预留的 ID）；
//! - 不存在任何提权（以管理员身份保存）入口。

use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use std::sync::Arc;

pub const SAVE: &str = "workbench.action.files.save";
pub const SAVE_AS: &str = "workbench.action.files.saveAs";
pub const SAVE_ALL: &str = "workbench.action.files.saveAll";
/// `ctrl+k s` 在 VS Code 默认键位表里绑定的是这个短 ID（与 `saveAll` 同义）。
pub const SAVE_ALL_SHORT: &str = "saveAll";
pub const SAVE_FILES: &str = "workbench.action.files.saveFiles";
pub const SAVE_ALL_IN_GROUP: &str = "workbench.files.action.saveAllInGroup";
/// Rove 没有保存时格式化：等同于 [`SAVE`]，但命令必须存在。
pub const SAVE_WITHOUT_FORMATTING: &str = "workbench.action.files.saveWithoutFormatting";
pub const REVERT: &str = "workbench.action.files.revert";
/// 保存冲突通知里“放弃本地修改”的命令形式。
pub const REVERT_LOCAL_CHANGES: &str = "workbench.files.action.revertLocalChanges";
/// 保存冲突通知里“覆盖”的命令形式。
pub const ACCEPT_LOCAL_CHANGES: &str = "workbench.files.action.acceptLocalChanges";
/// 状态栏换行符选择器的“统一为 LF / CRLF”（参数 `"\n"` / `"\r\n"`）。
pub const CHANGE_EOL: &str = "workbench.action.editor.changeEOL";

/// (命令 ID, 标题字符串 key, 中文标题)。
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (SAVE, "save.command.save", "保存"),
    (SAVE_AS, "save.command.saveAs", "另存为..."),
    (SAVE_ALL, "save.command.saveAll", "全部保存"),
    (SAVE_ALL_SHORT, "save.command.saveAll", "全部保存"),
    (SAVE_FILES, "save.command.saveFiles", "保存所有文件"),
    (
        SAVE_ALL_IN_GROUP,
        "save.command.saveAllInGroup",
        "保存组中的全部文件",
    ),
    (
        SAVE_WITHOUT_FORMATTING,
        "save.command.saveWithoutFormatting",
        "保存但不格式化",
    ),
    (REVERT, "save.command.revert", "还原文件"),
    (
        REVERT_LOCAL_CHANGES,
        "save.command.revertLocalChanges",
        "放弃本地修改并还原为磁盘上的内容",
    ),
    (
        ACCEPT_LOCAL_CHANGES,
        "save.command.acceptLocalChanges",
        "使用本地修改覆盖磁盘上的内容",
    ),
    (CHANGE_EOL, "save.command.changeEol", "更改行尾序列"),
];

pub const CATEGORY_KEY: &str = "category.file";

// 反馈键前缀（后接文档 ID）：同一文档同一原因只有一条，再次触发时更新次数（UX_SPEC §6.1）。
pub const KEY_BUSY: &str = "save.busy:";
pub const KEY_READ_ONLY: &str = "save.readOnly:";
pub const KEY_CONFLICT: &str = "save.conflict:";
pub const KEY_FAILED: &str = "save.failed:";
pub const KEY_UNENCODABLE: &str = "save.unencodable:";
pub const KEY_CLOSE: &str = "save.close:";
pub const KEY_EXTERNAL: &str = "save.external:";

// 反馈按钮的动作 ID。
pub const ACTION_RETRY: &str = "save.retry";
pub const ACTION_SAVE_AS: &str = "save.saveAs";
pub const ACTION_OVERWRITE: &str = "save.overwrite";
/// 预留：对比视图（P2）完成前不出现在任何通知里。
pub const ACTION_COMPARE: &str = "save.compare";
pub const ACTION_DISCARD: &str = "save.discard";
pub const ACTION_USE_UTF8: &str = "save.useUtf8";
pub const ACTION_REPLACE: &str = "save.replaceUnencodable";
pub const ACTION_CANCEL: &str = "save.cancel";
pub const ACTION_SAVE: &str = "save.save";
pub const ACTION_DONT_SAVE: &str = "save.dontSave";
pub const ACTION_RELOAD: &str = "save.reload";
pub const ACTION_KEEP: &str = "save.keepLocal";

/// 界面字符串：先说发生了什么，再说可以怎么做，不带“错误：”前缀（UX_SPEC §6）。
pub const STRINGS: &[(&str, &str)] = &[
    (CATEGORY_KEY, "文件"),
    ("save.notify.busy", "无法保存 {name}：文件被其他程序占用。"),
    ("save.notify.readOnly", "无法保存 {name}：文件是只读的。"),
    (
        "save.notify.conflict",
        "无法保存 {name}：磁盘上的文件已被修改。",
    ),
    (
        "save.notify.accessDenied",
        "无法保存 {name}：没有写入权限。",
    ),
    (
        "save.notify.directoryMissing",
        "无法保存 {name}：所在的文件夹不存在。",
    ),
    (
        "save.notify.documentReadOnly",
        "无法保存 {name}：它在编辑器中是只读的。可以另存为。",
    ),
    ("save.notify.failed", "无法保存 {name}。"),
    (
        "save.modal.unencodable",
        "有 {count} 个字符无法用 {encoding} 保存。",
    ),
    (
        "save.modal.close",
        "是否要保存对 {name} 的更改？如果不保存，更改将丢失。",
    ),
    ("save.bar.external", "{name} 在磁盘上已被修改。"),
    ("save.label.deleted", "已删除"),
    ("save.action.retry", "重试"),
    ("save.action.saveAs", "另存为..."),
    ("save.action.overwrite", "覆盖"),
    ("save.action.compare", "比较"),
    ("save.action.discard", "放弃本地修改"),
    ("save.action.useUtf8", "改用 UTF-8 保存"),
    ("save.action.replace", "仍然保存（替换为 ?）"),
    ("save.action.cancel", "取消"),
    ("save.action.save", "保存"),
    ("save.action.dontSave", "不保存"),
    ("save.action.reload", "重新加载（丢弃本地修改）"),
    ("save.action.keep", "保留本地修改"),
    ("save.status.saved", "已保存 {name}"),
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

/// 本模块命令的 GPUI action；由窗口（宿主）处理。
///
/// 尚未接入 `keys::action_for`：当前代码视图只读（可编辑视图属于 #47），窗口里没有可保存的修改，
/// 按“未实现的功能不得注册成可执行菜单项”，这些命令只登记、不进命令面板。#47 接入可编辑视图时
/// 在 `keys::action_for` 追加 `crate::save::registry::action_for`，并在窗口里把它分发给
/// [`super::SaveController`]。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = save, no_json)]
pub struct SaveAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(SaveAction(cmd)) as Box<dyn Action>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::{Message, Strings};

    #[test]
    fn registers_commands_and_strings_idempotently() {
        let mut commands = CommandRegistry::default();
        register_commands(&mut commands).unwrap();
        register_commands(&mut commands).unwrap();
        for (id, _, _) in COMMANDS {
            assert!(commands.get(id).is_some(), "{id}");
            assert!(action_for(id).is_some());
        }
        let mut strings = StringTable::default();
        register_strings(&mut strings).unwrap();
        register_strings(&mut strings).unwrap();
        for (_, key, _) in COMMANDS {
            assert!(strings.get(key).is_some());
        }
        let message = Message {
            key: "save.modal.unencodable".into(),
            args: Arc::from([
                (Arc::from("count"), Arc::from("2")),
                (Arc::from("encoding"), Arc::from("GBK")),
            ]),
        };
        assert_eq!(
            strings.text(&message).unwrap(),
            "有 2 个字符无法用 GBK 保存。"
        );
    }

    #[test]
    fn no_elevation_entry_and_no_hardcoded_error_prefix() {
        for (key, text) in STRINGS {
            assert!(!text.contains("管理员"), "{key}");
            assert!(!text.starts_with("错误"), "{key}");
        }
        for (id, _, title) in COMMANDS {
            assert!(!id.to_lowercase().contains("sudo") && !id.contains("Elevat"));
            assert!(!title.contains("管理员"));
        }
    }
}
