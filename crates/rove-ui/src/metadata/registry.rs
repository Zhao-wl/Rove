//! #42 的界面字符串登记。命令与设置项没有新增：`F12` / `Alt+F12` / `Shift+F12` 是 #40 / #41 的命令，
//! `rove.metadataView.showDocs` 已由 #13 登记（本模块只读取）。

use rove_base::{Error, Result};
use rove_i18n::StringTable;
use std::sync::Arc;

/// 设置项：元数据视图是否显示文档注释（#13 登记，默认 true）。
pub const SHOW_DOCS: &str = "rove.metadataView.showDocs";

pub const STRINGS: &[(&str, &str)] = &[
    // 标签标题（UX_SPEC §4.1：`Transform [元数据]`）。
    ("metadata.tabTitle", "{name} [元数据]"),
    // 标签悬停提示：DLL 完整路径，没有路径时只有程序集名。
    ("metadata.tabTooltip", "{path}"),
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
