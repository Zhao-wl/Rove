//! 超大文件与超长行界面的登记（#22）：界面字符串、状态栏条目、颜色键。
//!
//! 按 #1 的登记机制加入，**不写死**：
//! - 命令 `rove.editor.openLongLine` 的 ID 与标题已由 #14 的 `rove_commands::builtin` 登记，
//!   这里只引用 [`long_line::OPEN_LONG_LINE`](super::long_line::OPEN_LONG_LINE)；
//! - 设置项 `editor.stopRenderingLineAfter`、`editor.largeFileOptimizations`、
//!   `rove.largeFile.threshold` 由 #19/#8 登记，本模块只读取，不另外定义 `rove.*` 替代项；
//! - 颜色键只用 `THEME_COLORS` 中已有的 `descriptionForeground`，不新造。

use crate::contributions::{Contribution, ContributionRegistry, Placement};
use crate::shell::catalog::STATUS_FACTORY;
use rove_base::{Error, Result};
use rove_i18n::{Message, StringTable};
use rove_theme::ColorRegistry;
use std::sync::Arc;

/// 行索引进度（`行索引 45%`），建完后条目消失。本条目属于 #22。
pub const INDEX_STATUS: &str = "rove.status.lineIndex";

/// 状态栏右侧位置：VS Code 的 priority，数值越大越靠左。
/// 排在编码、换行符（#27，100 起）右边。文件大小标记是 #27 只读标记条目的一个状态（#185）。
pub const INDEX_ORDER: i32 = 94;

/// 本模块用到的颜色键（均为已有的 VS Code 颜色键，不新造）。
pub const COLORS: &[&str] = &[super::long_line::MARK_COLOR];

pub fn register_contributions(r: &mut ContributionRegistry) -> Result<()> {
    match r.register(
        INDEX_STATUS,
        Contribution {
            title: Message::key("largeFile.status.indexName"),
            placement: Placement::StatusRight,
            order: INDEX_ORDER,
            icon: None,
            command: None,
            when: None,
            factory_key: STATUS_FACTORY.into(),
        },
    ) {
        Ok(()) | Err(Error::Duplicate(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

const STRINGS: &[(&str, &str)] = &[
    // --- 提示条（UX_SPEC §3.2 原文） ---
    (
        super::notice::NOTICE_HUGE,
        "超大文件（{size}）：只读，已关闭语法高亮等功能。",
    ),
    // --- 状态栏 ---
    ("largeFile.status.indexName", "行索引进度"),
    (super::progress::STATUS_KEY, "行索引 {percent}%"),
    (
        super::progress::STATUS_TOOLTIP_KEY,
        "正在后台建立行索引（{percent}%）。文件已经可以阅读和查找，索引建完后行号与滚动条才完全准确。",
    ),
    // --- 转到行的等待（界面在 #24） ---
    (
        super::goto::WAIT_HINT_KEY,
        "正在建立行索引，完成后跳转（{percent}%）",
    ),
    (super::goto::CANCEL_KEY, "取消"),
    (super::goto::CANCEL_HINT_KEY, "按 {key} 取消等待"),
    // --- 超长行 ---
    (
        super::long_line::MARK_KEY,
        "…（已截断，共 {chars} 个字符）",
    ),
    (super::long_line::TAB_TITLE_KEY, "超长行 {line}：{name}"),
    (
        super::long_line::TAB_NOTE_KEY,
        "只读的临时视图：第 {line} 行共 {chars} 个字符，已按每行 {columns} 个字符断开显示。",
    ),
    (
        super::long_line::NO_LONG_LINE_KEY,
        "当前行没有被截断，不需要在临时标签中查看。",
    ),
];

pub fn register_strings(t: &mut StringTable) -> Result<()> {
    for (key, text) in STRINGS {
        match t.register(*key, Arc::from(*text)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_string_key_used_by_the_module_is_registered() {
        let mut table = StringTable::default();
        register_strings(&mut table).unwrap();
        // 重复登记不报错（与其他模块共享的键）。
        register_strings(&mut table).unwrap();
        let keys: Vec<&str> = STRINGS.iter().map(|(k, _)| *k).collect();
        for key in [
            super::super::notice::NOTICE_HUGE,
            super::super::progress::STATUS_KEY,
            super::super::goto::WAIT_HINT_KEY,
            super::super::long_line::MARK_KEY,
            super::super::long_line::TAB_TITLE_KEY,
        ] {
            assert!(keys.contains(&key), "{key} 未登记");
        }
    }

    #[test]
    fn status_items_register_on_the_right_side() {
        let mut r = ContributionRegistry::default();
        crate::shell::catalog::register_contributions(&mut r).unwrap();
        register_contributions(&mut r).unwrap();
        // 幂等。
        register_contributions(&mut r).unwrap();
        let c = r.get(INDEX_STATUS).expect("已登记");
        assert!(matches!(c.placement, Placement::StatusRight));
    }
}
