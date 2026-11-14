//! 热退出与崩溃恢复的界面登记项（#50）：界面字符串与反馈键。核心逻辑在
//! [`rove_buffer::hotexit`]（不依赖 gpui），窗口装配在 `skeleton/workbench/hotexit_host.rs`。
//!
//! - 设置项 `files.hotExit` / `files.restoreUndoStack` 由 `rove_buffer::hotexit::settings::register`
//!   登记（`app_settings` 装配），这里只读；
//! - **没有命令**：热退出是行为，不是用户能触发的动作，所以不登记命令，也没有键位；
//! - 两条提示都是第 1 级反馈（状态栏临时消息，3 秒后消失，`UX_SPEC` §6.1）：**不弹通知、
//!   不弹模态框**（`UX_SPEC` §6.4 明确要求）。

use rove_base::{Error, Result};
use rove_i18n::StringTable;
use std::sync::Arc;

/// 反馈键：意外退出后恢复成功的状态栏提示（整次会话只出现一次）。
pub const KEY_RESTORED: &str = "hotexit.restored";
/// 反馈键：连续两次恢复失败，备份已被隔离（只提示一次，不再尝试）。
pub const KEY_QUARANTINED: &str = "hotexit.quarantined";

pub const STRINGS: &[(&str, &str)] = &[
    // UX_SPEC §6.4 的原文。
    ("hotexit.status.restored", "已恢复上次未保存的修改"),
    (
        "hotexit.status.quarantined",
        "无法恢复上次未保存的修改，备份已保留在 {dir}",
    ),
];

pub fn register_strings(table: &mut StringTable) -> Result<()> {
    for (key, text) in STRINGS.iter().copied() {
        match table.register(key, Arc::from(text)) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::{Message, Strings};

    #[test]
    fn strings_are_registered_idempotently_and_match_the_spec() {
        let mut table = StringTable::default();
        register_strings(&mut table).unwrap();
        register_strings(&mut table).unwrap();
        for (key, _) in STRINGS {
            assert!(table.get(key).is_some(), "{key}");
        }
        let message = Message {
            key: "hotexit.status.restored".into(),
            args: Arc::from([]),
        };
        // 逐字取自 UX_SPEC §6.4。
        assert_eq!(table.text(&message).unwrap(), "已恢复上次未保存的修改");
        // 不带“错误：”前缀，也不出现模态框 / 通知才有的措辞（UX_SPEC §6）。
        for (key, text) in STRINGS {
            assert!(!text.starts_with("错误"), "{key}");
            assert!(!text.contains("是否"), "{key}");
        }
    }
}
