//! 更新状态：「关于」对话框与提示条看到的那一份（issue #149 第 3 节最后一条）。
//!
//! 「关于」对话框要显示：已是最新 / 正在下载 / 待重启安装 / 更新已关闭 /
//! 上次检查失败（时间与原因）。状态只在这里定义，**文案由 `rove-ui` 按
//! [`UpdateState::string_key`] 去登记过的字符串表里取**（不写死在核心层）。

use crate::{version::Version, UpdateError};

/// 当前的更新状态。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateState {
    /// `rove.update.mode = "none"`。
    Disabled,
    /// 还没检查过。
    #[default]
    Idle,
    /// 正在检查。
    Checking,
    /// 已是最新（带上次检查时间，UTC 字符串）。
    UpToDate { checked_at: String },
    /// 发现了新版本，等用户确认下载（`start` 模式）。
    Available { version: String },
    /// 正在下载。
    Downloading { version: String },
    /// 下载好了，等重启安装。
    ReadyToInstall { version: String },
    /// 上次检查失败：时间与原因。
    Failed { at: String, reason: String },
    /// 上次安装失败：时间、没装上的版本与原因（issue #235）。
    /// 这次安装请求已经结束（记录与下载都清掉了），用户可以明确重试。
    InstallFailed {
        at: String,
        version: String,
        reason: String,
    },
}

impl UpdateState {
    /// 界面字符串的 key（在 `rove-ui` 的登记表里）。
    pub fn string_key(&self) -> &'static str {
        match self {
            Self::Disabled => "update.state.disabled",
            Self::Idle => "update.state.idle",
            Self::Checking => "update.state.checking",
            Self::UpToDate { .. } => "update.state.upToDate",
            Self::Available { .. } => "update.state.available",
            Self::Downloading { .. } => "update.state.downloading",
            Self::ReadyToInstall { .. } => "update.state.ready",
            Self::Failed { .. } => "update.state.failed",
            Self::InstallFailed { .. } => "update.state.installFailed",
        }
    }

    /// 文案里的占位符参数（`{version}` / `{time}` / `{reason}`）。
    pub fn args(&self) -> Vec<(&'static str, String)> {
        match self {
            Self::UpToDate { checked_at } => vec![("time", checked_at.clone())],
            Self::Available { version }
            | Self::Downloading { version }
            | Self::ReadyToInstall { version } => vec![("version", version.clone())],
            Self::Failed { at, reason } => {
                vec![("time", at.clone()), ("reason", reason.clone())]
            }
            Self::InstallFailed {
                at,
                version,
                reason,
            } => vec![
                ("time", at.clone()),
                ("version", version.clone()),
                ("reason", reason.clone()),
            ],
            _ => Vec::new(),
        }
    }

    /// 有没有可以「重启以更新」的东西（决定 `update.restartToUpdate` 命令可不可用）。
    pub fn is_ready_to_install(&self) -> bool {
        matches!(self, Self::ReadyToInstall { .. })
    }

    /// 正忙（检查或下载中）：再点一次「检查更新」不该叠一次检查。
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Checking | Self::Downloading { .. })
    }

    /// 从一次检查的失败构造状态。
    pub fn from_error(error: &UpdateError, at: String) -> Self {
        match error {
            UpdateError::Disabled => Self::Disabled,
            other => Self::Failed {
                at,
                reason: other.reason(),
            },
        }
    }

    /// 待安装版本（界面上拿去拼「重启以更新到 x.y.z」）。
    pub fn pending_version(&self) -> Option<&str> {
        match self {
            Self::ReadyToInstall { version } | Self::Available { version } => Some(version),
            _ => None,
        }
    }
}

/// 一次检查的结论（engine 返回给界面）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckResult {
    /// 已是最新。
    UpToDate,
    /// 有新版本（带是否「必须更新」，来自 `min_version`）。
    Newer { version: Version, required: bool },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_maps_to_its_own_string_key() {
        let states = [
            UpdateState::Disabled,
            UpdateState::Idle,
            UpdateState::Checking,
            UpdateState::UpToDate {
                checked_at: "2026-11-14 10:00:00Z".into(),
            },
            UpdateState::Available {
                version: "0.1.1".into(),
            },
            UpdateState::Downloading {
                version: "0.1.1".into(),
            },
            UpdateState::ReadyToInstall {
                version: "0.1.1".into(),
            },
            UpdateState::Failed {
                at: "2026-11-14 10:00:00Z".into(),
                reason: "网络不可用".into(),
            },
            UpdateState::InstallFailed {
                at: "2026-11-14 10:00:00Z".into(),
                version: "0.1.1".into(),
                reason: "安装器退出码 3".into(),
            },
        ];
        let mut keys: Vec<&str> = states.iter().map(UpdateState::string_key).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "状态的 key 不能重复");
        assert!(keys.iter().all(|k| k.starts_with("update.state.")));
    }

    /// 「上次检查失败」要带得出时间与原因。
    #[test]
    fn a_failure_carries_the_time_and_the_reason() {
        let state = UpdateState::from_error(&UpdateError::Http(403), "2026-11-14 10:00:00Z".into());
        assert_eq!(
            state.args(),
            [
                ("time", "2026-11-14 10:00:00Z".to_string()),
                ("reason", "服务器返回 403".to_string())
            ]
        );
        // `none` 不是失败，是「更新已关闭」。
        assert_eq!(
            UpdateState::from_error(&UpdateError::Disabled, String::new()),
            UpdateState::Disabled
        );
    }

    /// 安装失败和检查失败是两回事：「关于」对话框要说清楚是哪个版本没装上（#235）。
    #[test]
    fn a_failed_install_names_the_version_that_did_not_make_it() {
        let state = UpdateState::InstallFailed {
            at: "2026-11-17 10:00:00Z".into(),
            version: "0.1.1".into(),
            reason: "安装器退出码 3".into(),
        };
        assert_eq!(state.string_key(), "update.state.installFailed");
        assert_eq!(
            state.args(),
            [
                ("time", "2026-11-17 10:00:00Z".to_string()),
                ("version", "0.1.1".to_string()),
                ("reason", "安装器退出码 3".to_string())
            ]
        );
        // 安装失败之后没有可以重启安装的东西了。
        assert!(!state.is_ready_to_install());
        assert!(!state.is_busy());
        assert_eq!(state.pending_version(), None);
    }

    #[test]
    fn only_a_downloaded_update_can_be_installed_on_restart() {
        assert!(UpdateState::ReadyToInstall {
            version: "0.1.1".into()
        }
        .is_ready_to_install());
        for state in [
            UpdateState::Idle,
            UpdateState::Disabled,
            UpdateState::Checking,
            UpdateState::Available {
                version: "0.1.1".into(),
            },
            UpdateState::Downloading {
                version: "0.1.1".into(),
            },
        ] {
            assert!(!state.is_ready_to_install(), "{state:?}");
        }
        assert!(UpdateState::Checking.is_busy());
        assert!(UpdateState::Downloading {
            version: "0.1.1".into()
        }
        .is_busy());
        assert!(!UpdateState::Idle.is_busy());
    }

    #[test]
    fn the_pending_version_is_available_for_the_prompt() {
        assert_eq!(
            UpdateState::ReadyToInstall {
                version: "0.1.1".into()
            }
            .pending_version(),
            Some("0.1.1")
        );
        assert_eq!(UpdateState::Idle.pending_version(), None);
    }
}
