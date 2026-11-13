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

/// 文件大小标记（`超大文件 · 只读` / `大文件`）。
///
/// **临时承载**：按 issue，这应当是 #27 的“只读标记”条目的一个状态；#27 尚未合并，
/// 本 issue 先用自己的条目把数据显示出来，#27 合并时把 [`super::status_entry`] 的输出并进
/// 只读标记条目并删掉这一条（`docs/ARCHITECTURE.md` §39 已注明）。
pub const SIZE_STATUS: &str = "rove.status.fileSize";
/// 行索引进度（`行索引 45%`），建完后条目消失。本条目属于 #22。
pub const INDEX_STATUS: &str = "rove.status.lineIndex";

/// 状态栏右侧位置：VS Code 的 priority，数值越大越靠左。
/// 只读/大小标记排在编码、换行符（#27，100 起）右边。
pub const SIZE_ORDER: i32 = 95;
pub const INDEX_ORDER: i32 = 94;

/// 本模块用到的颜色键（均为已有的 VS Code 颜色键，不新造）。
pub const COLORS: &[&str] = &[super::long_line::MARK_COLOR];

pub fn register_contributions(r: &mut ContributionRegistry) -> Result<()> {
    for (id, title, order) in [
        (SIZE_STATUS, "largeFile.status.name", SIZE_ORDER),
        (INDEX_STATUS, "largeFile.status.indexName", INDEX_ORDER),
    ] {
        match r.register(
            id,
            Contribution {
                title: Message::key(title),
                placement: Placement::StatusRight,
                order,
                icon: None,
                command: None,
                when: None,
                factory_key: STATUS_FACTORY.into(),
            },
        ) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

const STRINGS: &[(&str, &str)] = &[
    // --- 提示条（UX_SPEC §3.2 原文） ---
    (
        super::notice::NOTICE_HUGE,
        "超大文件（{size}）：只读，已关闭语法高亮等功能。",
    ),
    // --- 状态栏 ---
    ("largeFile.status.name", "文件大小"),
    ("largeFile.status.indexName", "行索引进度"),
    (super::notice::STATUS_HUGE, "超大文件 · 只读"),
    (super::notice::STATUS_LARGE, "大文件"),
    (super::progress::STATUS_KEY, "行索引 {percent}%"),
    (
        super::progress::STATUS_TOOLTIP_KEY,
        "正在后台建立行索引（{percent}%）。文件已经可以阅读和查找，索引建完后行号与滚动条才完全准确。",
    ),
    ("largeFile.tooltip.disabled", "已关闭的功能：\n{features}"),
    (
        "largeFile.tooltip.noneDisabled",
        "这个文件没有因为体积关闭任何功能。",
    ),
    // --- 被关闭的功能（顺序与 rove_buffer::disabled_features 一致） ---
    ("largeFile.feature.minimap", "缩略图"),
    ("largeFile.feature.stickyScroll", "粘性滚动"),
    ("largeFile.feature.bracketPairColorization", "括号对着色"),
    ("largeFile.feature.occurrencesHighlight", "同名高亮"),
    ("largeFile.feature.semanticHighlighting", "语义高亮"),
    ("largeFile.feature.syntaxFolding", "基于语法的折叠"),
    ("largeFile.feature.syntaxHighlighting", "语法高亮"),
    ("largeFile.feature.folding", "折叠"),
    ("largeFile.feature.wordWrap", "软换行"),
    ("largeFile.feature.replace", "替换"),
    // --- 转到行的等待（界面在 #24） ---
    (
        super::goto::WAIT_HINT_KEY,
        "正在建立行索引，完成后跳转（{percent}%）",
    ),
    (super::goto::CANCEL_KEY, "取消"),
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
            super::super::notice::STATUS_HUGE,
            super::super::notice::STATUS_LARGE,
            super::super::progress::STATUS_KEY,
            super::super::goto::WAIT_HINT_KEY,
            super::super::long_line::MARK_KEY,
            super::super::long_line::TAB_TITLE_KEY,
        ] {
            assert!(keys.contains(&key), "{key} 未登记");
        }
        for feature in rove_buffer::disabled_features(rove_buffer::SizeClass::Huge) {
            let key = super::super::notice::feature_key(*feature);
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
        for id in [SIZE_STATUS, INDEX_STATUS] {
            let c = r.get(id).expect("已登记");
            assert!(matches!(c.placement, Placement::StatusRight));
        }
    }
}
