//! 超大文件与超长行的界面（#22，`UX_SPEC` §3.1–§3.3、§6.1、§6.2）。
//!
//! #8 提供了内存映射、分块行索引与增量索引，#19 提供了 f64 滚动模型与单行渲染上限挂钩，
//! #21 提供了带进度的查找引擎与状态机；本模块补齐用户看到的那一层：
//!
//! | 用途 | 入口 |
//! |---|---|
//! | 超大文件提示条（可关闭、可“不再提示”） | [`notice::notice_for`]、[`LargeFileState::notice`] |
//! | 状态栏 `超大文件 · 只读` 与被关闭功能的悬停 | [`status_entry`] |
//! | 行索引进度与滚动条的字节→行外推 | [`progress::IndexProgress`]、[`index_status_entry`] |
//! | 转到行的等待与取消 | [`goto::PendingGoto`]、[`goto::wait_for_line`] |
//! | 流式查找（进度、部分结果、隐藏替换） | [`allows_replace`]、`find::state::FindModel::publish_partial` |
//! | 超长行截断标记与只读临时标签 | [`TruncationMarks`]、[`long_line`] |
//!
//! **全程不弹通知**（§6.1）：提示条走 #16 的 `Reason::FileContext`（第 3 级），
//! 索引进度走 `Reason::Background`（只进状态栏）。
//!
//! 除 [`host`] 外本模块不依赖 GPUI。

pub mod goto;
pub mod long_line;
pub mod notice;
pub mod progress;
pub mod registry;

pub mod source;
#[cfg(test)]
mod tests;

pub use source::LongLineSource;

use crate::code_view::{
    decorations::{DecorationProvider, LineDecorations, Overlay, TextStyle},
    Feature,
};
use crate::shell::status::{StatusEntry, StatusKind};
use rove_base::Result;
use rove_buffer::{SizeClass, TextSource};
use rove_i18n::{Message, Strings};
use rove_theme::Color;
use std::cell::RefCell;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

pub use progress::IndexProgress;

/// 一次性登记本模块的界面字符串、状态栏条目与颜色键（装配时调用）。
///
/// 命令 `rove.editor.openLongLine` 与三个设置项由 #14 / #19 / #8 登记，这里只引用。
pub fn register(
    contributions: &mut crate::contributions::ContributionRegistry,
    colors: &mut rove_theme::ColorRegistry,
    strings: &mut rove_i18n::StringTable,
) -> Result<()> {
    registry::register_contributions(contributions)?;
    registry::register_colors(colors)?;
    registry::register_strings(strings)
}

fn label(strings: &dyn Strings, key: &str, args: &[(&str, String)]) -> String {
    let message = Message {
        key: key.into(),
        args: args
            .iter()
            .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
            .collect(),
    };
    rove_i18n::tr!(strings, &message).unwrap_or_else(|_| key.to_string())
}

/// 状态栏的文件大小标记（`超大文件 · 只读`）与悬停提示。
///
/// 两个数据源各管一半，与 `docs/ARCHITECTURE.md` 的分工一致：
/// - **这一级生效了吗**：看 #20 的
///   [`CodeView::disabled_features`](crate::code_view::CodeView::disabled_features)。
///   `editor.largeFileOptimizations` 为 false 时大文件级别不生效，视图报空集合，这里也不列功能。
/// - **关掉了哪些功能**：用 #8 的 [`rove_buffer::disabled_features`]（`UX_SPEC` §3.1 的表），
///   界面字符串由本模块登记（[`notice::feature_key`]）。#20 只实现了其中一部分，
///   但状态栏要照 §3.1 把这一级关掉的功能都列出来。
///
/// 普通文件没有条目。
pub fn status_entry(
    strings: &dyn Strings,
    size: SizeClass,
    disabled: &[Feature],
) -> Option<StatusEntry> {
    let key = notice::status_key(size)?;
    let in_effect = !disabled.is_empty();
    let tooltip = if in_effect {
        let features = rove_buffer::disabled_features(size)
            .iter()
            .map(|f| format!("· {}", label(strings, notice::feature_key(*f), &[])))
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        label(
            strings,
            "largeFile.tooltip.disabled",
            &[("features", features)],
        )
    } else {
        label(strings, "largeFile.tooltip.noneDisabled", &[])
    };
    Some(StatusEntry {
        text: label(strings, key, &[]),
        tooltip: Some(tooltip),
        command: None,
        kind: StatusKind::Standard,
    })
}

/// 状态栏的 `行索引 N%`；索引建完后返回 `None`（条目消失）。
pub fn index_status_entry(strings: &dyn Strings, p: IndexProgress) -> Option<StatusEntry> {
    if !p.is_indexing() {
        return None;
    }
    let percent = p.percent().to_string();
    Some(StatusEntry {
        text: label(
            strings,
            progress::STATUS_KEY,
            &[("percent", percent.clone())],
        ),
        tooltip: Some(label(
            strings,
            progress::STATUS_TOOLTIP_KEY,
            &[("percent", percent)],
        )),
        command: None,
        kind: StatusKind::Standard,
    })
}

/// 超大文件上不提供替换（M1 本来就没有替换，M3 的 #48 也不得在超大文件上出现）。
pub fn allows_replace(size: SizeClass) -> bool {
    size != SizeClass::Huge
}

/// 超长行的截断标记提供方（#19 的行内覆盖物接口）。
///
/// 只为**被截断**的行产出一条灰色覆盖物，放在最后一个渲染出来的字符之后；
/// 字符数由 [`long_line::CharCounts`] 按行缓存，整行只扫描一次。
pub struct TruncationMarks {
    text: Arc<dyn TextSource>,
    /// 渲染上限（字符数）：`None` 表示不限，这时不会有截断。
    limit: Option<usize>,
    color: Option<Color>,
    template: String,
    counts: RefCell<long_line::CharCounts>,
}

impl TruncationMarks {
    pub fn new(
        text: Arc<dyn TextSource>,
        limit: Option<usize>,
        color: Option<Color>,
        strings: &dyn Strings,
    ) -> Self {
        Self {
            text,
            limit,
            color,
            template: label(strings, long_line::MARK_KEY, &[("chars", "{chars}".into())]),
            counts: RefCell::new(long_line::CharCounts::default()),
        }
    }

    /// 截断点：最后一个会被渲染出来的字符之后的源字节列。
    ///
    /// 逐字素簇累计字符数，与 #19 的 [`DisplayLine::build`](crate::code_view::columns::DisplayLine)
    /// **同一套规则**（按字素簇截断、按字符数计上限），否则标记会压在最后一个字形上或留出空档。
    /// 行没有超过上限时返回 `None`。
    fn cut_at(&self, source: &str) -> Option<u32> {
        let limit = self.limit?;
        let mut chars = 0usize;
        for (offset, cluster) in source.grapheme_indices(true) {
            if chars >= limit {
                return Some(offset as u32);
            }
            chars += cluster.chars().count();
        }
        None
    }

    /// 某一行的标记文字（行未被截断时为 `None`）；测试与临时标签说明用。
    pub fn mark_text(&self, line: u64, source: &str) -> Option<String> {
        self.cut_at(source)?;
        let chars = self
            .counts
            .borrow_mut()
            .get(&*self.text, line)
            .unwrap_or_else(|| source.chars().count() as u64);
        Some(self.template.replace("{chars}", &chars.to_string()))
    }
}

impl DecorationProvider for TruncationMarks {
    fn decorate(&self, line: u64, text: &str, out: &mut LineDecorations) {
        let Some(column) = self.cut_at(text) else {
            return;
        };
        let Some(mark) = self.mark_text(line, text) else {
            return;
        };
        out.overlays.push(Overlay {
            column,
            text: mark.into(),
            style: TextStyle {
                foreground: self.color,
                ..Default::default()
            },
        });
    }
}
