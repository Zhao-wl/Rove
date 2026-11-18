//! 超大文件的提示条与状态栏数据（#22，`UX_SPEC` §3.2）。
//!
//! 本文件只产出**数据**（文案 key + 参数、去重键），不依赖 GPUI，也不自己决定反馈级别：
//! 级别由 #16 的 [`Reason::FileContext`](crate::shell::feedback::Reason) 映射成“编辑器顶部提示条”，
//! 所以**永远不会弹通知**（§6.1）。大文件级别不产生提示条，只在状态栏标记。

use rove_buffer::SizeClass;

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
}
