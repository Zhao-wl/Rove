//! `rove.update.mode`：更新的四个取值与它们决定的节奏（issue #149 第 3 节）。
//!
//! | 取值 | 行为 |
//! |---|---|
//! | `none` | 从不联网；「检查更新」命令提示更新已关闭 |
//! | `manual` | 只在执行「检查更新」命令时检查 |
//! | `start` | 启动时检查一次，有新版本时提示，**用户确认后**再下载 |
//! | `default`（默认） | 启动后延迟检查（不占冷启动预算），之后每 6 小时检查一次；有新版本时后台下载，下载好后用提示条提示「重启以更新到 x.y.z」 |
//!
//! 键名带 `rove.` 前缀：settings.json 是和 VS Code 共用的，VS Code 自己有一个
//! `update.mode`，两者不能互相影响（`FEATURES.md` §13）。设置项本身登记在
//! `rove-config` 的内置目录里（`docs/compat/SETTINGS.md` §4），这里只负责解释取值。

use std::time::Duration;

/// 设置键（登记在内置设置目录里，**不写死在界面上**）。
pub const SETTING_KEY: &str = "rove.update.mode";

/// 启动后第一次检查的延迟：**必须排在首帧之后**，否则会占冷启动预算
/// （#149 验收：冷启动预算不受影响）。
pub const STARTUP_DELAY: Duration = Duration::from_secs(20);
/// `default` 模式的检查间隔。
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum UpdateMode {
    /// 从不联网。
    None,
    /// 只有命令触发。
    Manual,
    /// 启动时检查一次，下载前先问。
    Start,
    /// 启动后延迟检查 + 每 6 小时一次 + 后台下载。
    #[default]
    Default,
}

impl UpdateMode {
    /// 设置值 → 模式。认不出来的值按默认处理（设置层会另外报格式错误）。
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "none" => Self::None,
            "manual" => Self::Manual,
            "start" => Self::Start,
            "default" => Self::Default,
            _ => return None,
        })
    }

    /// 写回设置文件时用的字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Manual => "manual",
            Self::Start => "start",
            Self::Default => "default",
        }
    }

    /// 四个合法取值（设置项登记与测试都用它，不在别处再抄一遍）。
    pub const ALL: [UpdateMode; 4] = [
        UpdateMode::None,
        UpdateMode::Manual,
        UpdateMode::Start,
        UpdateMode::Default,
    ];

    /// **本模式下是否允许建立任何连接**。`none` 为假时 [`crate::Updater`] 连传输层都不会构造。
    pub fn may_connect(self) -> bool {
        !matches!(self, Self::None)
    }

    /// 启动后多久做第一次检查；`None` 表示启动时不检查。
    pub fn startup_check(self) -> Option<Duration> {
        match self {
            // `start`：启动时检查一次。仍然排在首帧之后，只是不等满 20 秒。
            Self::Start => Some(Duration::from_secs(2)),
            Self::Default => Some(STARTUP_DELAY),
            Self::None | Self::Manual => None,
        }
    }

    /// 周期检查的间隔；`None` 表示不周期检查。
    pub fn repeat_interval(self) -> Option<Duration> {
        matches!(self, Self::Default).then_some(CHECK_INTERVAL)
    }

    /// 发现新版本后是否直接后台下载（`start` 要先问用户）。
    pub fn downloads_in_background(self) -> bool {
        matches!(self, Self::Default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验收：四个取值各有行为，而且能和设置值互相转换。
    #[test]
    fn the_four_values_round_trip() {
        for mode in UpdateMode::ALL {
            assert_eq!(UpdateMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(UpdateMode::parse("auto"), None);
        assert_eq!(UpdateMode::parse("None"), None);
        assert_eq!(UpdateMode::parse(""), None);
        // 默认值是 `default`（MILESTONE_v0.1 §6 的表）。
        assert_eq!(UpdateMode::default(), UpdateMode::Default);
        assert_eq!(UpdateMode::default().as_str(), "default");
    }

    /// `none` 是唯一一个连连接都不允许建立的取值。
    #[test]
    fn only_none_forbids_connecting() {
        assert!(!UpdateMode::None.may_connect());
        for mode in [UpdateMode::Manual, UpdateMode::Start, UpdateMode::Default] {
            assert!(mode.may_connect(), "{mode:?}");
        }
    }

    /// 启动检查、周期检查、后台下载三件事各归各的模式。
    #[test]
    fn each_mode_has_its_own_schedule() {
        assert_eq!(UpdateMode::None.startup_check(), None);
        assert_eq!(UpdateMode::None.repeat_interval(), None);
        assert!(!UpdateMode::None.downloads_in_background());

        assert_eq!(UpdateMode::Manual.startup_check(), None);
        assert_eq!(UpdateMode::Manual.repeat_interval(), None);
        assert!(!UpdateMode::Manual.downloads_in_background());

        // `start`：启动检查，但下载要先问。
        assert!(UpdateMode::Start.startup_check().is_some());
        assert_eq!(UpdateMode::Start.repeat_interval(), None);
        assert!(!UpdateMode::Start.downloads_in_background());

        assert_eq!(UpdateMode::Default.startup_check(), Some(STARTUP_DELAY));
        assert_eq!(UpdateMode::Default.repeat_interval(), Some(CHECK_INTERVAL));
        assert!(UpdateMode::Default.downloads_in_background());
    }

    /// 冷启动预算：任何模式的第一次检查都必须排在首帧之后（这里用「不为零」把关，
    /// 真正把它挂到首帧之后的是 `rove-ui` 的接线）。
    #[test]
    fn no_mode_checks_during_cold_start() {
        for mode in UpdateMode::ALL {
            if let Some(delay) = mode.startup_check() {
                assert!(delay >= Duration::from_secs(1), "{mode:?} 的首次检查太早");
            }
        }
        assert_eq!(CHECK_INTERVAL, Duration::from_secs(21_600));
    }
}
