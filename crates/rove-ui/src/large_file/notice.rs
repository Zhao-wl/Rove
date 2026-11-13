//! 超大文件的提示条与状态栏数据（#22，`UX_SPEC` §3.2）。
//!
//! 本文件只产出**数据**（文案 key + 参数、去重键），不依赖 GPUI，也不自己决定反馈级别：
//! 级别由 #16 的 [`Reason::FileContext`](crate::shell::feedback::Reason) 映射成“编辑器顶部提示条”，
//! 所以**永远不会弹通知**（§6.1）。大文件级别不产生提示条，只在状态栏标记。

use rove_buffer::{ReducedFeature, SizeClass};

/// 提示条的“不再提示”去重键：同一台机器只要用户点过一次，之后所有超大文件都不再提示。
pub const SUPPRESS_KEY: &str = "rove.largeFile.huge";
/// 提示条的反馈 key（每个文档一条）。
pub const BAR_KEY_PREFIX: &str = "rove.largeFile.bar.";

/// 体积格式化：按 1024 进位，1 位小数，整数时不带小数点（“1.2 GB”“64 MB”）。
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    // 1 位小数；四舍五入后正好是整数时省掉 “.0”。
    let rounded = (value * 10.0).round() / 10.0;
    if (rounded - rounded.round()).abs() < f64::EPSILON {
        format!("{} {}", rounded.round() as u64, UNITS[unit])
    } else {
        format!("{rounded:.1} {}", UNITS[unit])
    }
}

/// 打开一个文档时要显示的提示条（没有则为 `None`）。
///
/// 只有**超大文件**有提示条；大文件级别按 §3.2 只在状态栏标记，`editor.largeFileOptimizations`
/// 为 false 时连大文件级别都不成立（分级已由 #8 的 `SizePolicy` 处理，这里只看结果）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// 界面字符串 key。
    pub key: &'static str,
    /// 已格式化的体积，填进 `{size}`。
    pub size: String,
}

/// 提示条文案 key（`UX_SPEC` §3.2 的原文）。
pub const NOTICE_HUGE: &str = "largeFile.notice.huge";

pub fn notice_for(size: SizeClass, bytes: u64) -> Option<Notice> {
    match size {
        SizeClass::Huge => Some(Notice {
            key: NOTICE_HUGE,
            size: format_size(bytes),
        }),
        SizeClass::Normal | SizeClass::Large => None,
    }
}

/// 状态栏“超大文件 · 只读”条目的文案 key。
pub const STATUS_HUGE: &str = "largeFile.status.huge";
/// 状态栏“大文件”条目的文案 key（大文件可编辑，所以不带“只读”）。
pub const STATUS_LARGE: &str = "largeFile.status.large";

/// 状态栏条目的文案 key（普通文件没有条目）。
pub fn status_key(size: SizeClass) -> Option<&'static str> {
    match size {
        SizeClass::Huge => Some(STATUS_HUGE),
        SizeClass::Large => Some(STATUS_LARGE),
        SizeClass::Normal => None,
    }
}

/// 被关闭的功能在悬停提示里的字符串 key（内容来自 #8 的 `disabled_features`，
/// 与 #20 `CodeView::disabled_features` 对同一级别给出的集合一致）。
pub fn feature_key(feature: ReducedFeature) -> &'static str {
    match feature {
        ReducedFeature::Minimap => "largeFile.feature.minimap",
        ReducedFeature::StickyScroll => "largeFile.feature.stickyScroll",
        ReducedFeature::BracketPairColorization => "largeFile.feature.bracketPairColorization",
        ReducedFeature::OccurrencesHighlight => "largeFile.feature.occurrencesHighlight",
        ReducedFeature::SemanticHighlighting => "largeFile.feature.semanticHighlighting",
        ReducedFeature::SyntaxFolding => "largeFile.feature.syntaxFolding",
        ReducedFeature::SyntaxHighlighting => "largeFile.feature.syntaxHighlighting",
        ReducedFeature::Folding => "largeFile.feature.folding",
        ReducedFeature::WordWrap => "largeFile.feature.wordWrap",
        ReducedFeature::Replace => "largeFile.feature.replace",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_formatted_like_the_spec() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(64 * 1024 * 1024), "64 MB");
        // UX_SPEC §3.2 的例子：1.2 GB
        assert_eq!(format_size(1_288_490_189), "1.2 GB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1 GB");
        assert_eq!(format_size(1536 * 1024 * 1024), "1.5 GB");
    }

    #[test]
    fn only_huge_files_get_a_notice_bar() {
        assert_eq!(
            notice_for(SizeClass::Huge, 1_288_490_189),
            Some(Notice {
                key: NOTICE_HUGE,
                size: "1.2 GB".into(),
            })
        );
        assert_eq!(notice_for(SizeClass::Large, 30 << 20), None);
        assert_eq!(notice_for(SizeClass::Normal, 10), None);
    }

    #[test]
    fn status_marks_both_large_levels_but_not_normal() {
        assert_eq!(status_key(SizeClass::Huge), Some(STATUS_HUGE));
        assert_eq!(status_key(SizeClass::Large), Some(STATUS_LARGE));
        assert_eq!(status_key(SizeClass::Normal), None);
    }

    #[test]
    fn every_reduced_feature_has_a_distinct_string_key() {
        let keys: Vec<_> = rove_buffer::disabled_features(SizeClass::Huge)
            .iter()
            .map(|f| feature_key(*f))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len());
        assert_eq!(keys.len(), 10);
    }
}
