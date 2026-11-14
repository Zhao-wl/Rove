//! 自动更新（issue #149）：**全程序唯一允许发起网络请求的地方**
//! （`docs/PROJECT_CHARTER.md` §2.1，2026-11-13 修订）。
//!
//! 设计的第一目标是「让人放心」，所以：
//!
//! 1. **联网代码只有这一个 crate**，而且只有 [`http`] 一个模块；别的模块都是纯计算，
//!    可以脱离网络单独测试。`cargo xtask check-deps` 保证没有第二个 crate 声明网络相关的
//!    `windows` feature，也保证 HTTP / TLS 栈不回到依赖图里。
//! 2. **请求一眼能看清**：地址写死在 [`endpoint`]，一次检查最多三个 GET（清单、签名、
//!    安装包），没有自定义请求头、没有 cookie、没有设备标识，User-Agent 固定为 `Rove/<版本>`。
//!    每个请求都在 [`log`] 里留一行。
//! 3. **`rove.update.mode = "none"` 时一个连接都不建**：[`mode::UpdateMode::may_connect`]
//!    为假时 [`Updater`] 根本不会走到 [`http`]（[`Updater::check`] 直接返回
//!    [`Outcome::Disabled`]），传输层也不会被构造。
//! 4. **不信任下载到的任何东西**：清单必须通过内嵌公钥的 ed25519 验签（[`signature`]），
//!    安装包必须同时对上 `size` 与 `sha256`（[`download`]）。任何一条不过都当作「没有更新」，
//!    旧版本原样可用。
//!
//! # 模块地图
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`version`] | 版本号解析与比较（解析不了 = 没有更新） |
//! | [`manifest`] | `latest.json` 的格式（由本 issue 定义，#76 生成） |
//! | [`signature`] | 内嵌公钥与 ed25519 分离签名验证 |
//! | [`endpoint`] | 固定的清单地址；测试覆盖只在调试构建里生效 |
//! | [`http`] | WinHTTP 传输（唯一的网络出口） |
//! | [`paths`] | `%LOCALAPPDATA%\Rove\updates\` 的布局与待安装记录 |
//! | [`download`] | 下载 + `size` / `sha256` 校验 |
//! | [`mode`] | `rove.update.mode` 的四个取值与调度节奏 |
//! | [`state`] | 「关于」对话框与提示条看到的状态 |
//! | [`install`] | 静默安装与重启（参数来自清单，与 #76 对齐） |
//! | [`component`] | C# 索引组件（#43）的跟随更新 |
//! | [`zip`] | 组件包解压（只读 zip，不引入新依赖） |
//! | [`log`] | `%LOCALAPPDATA%\Rove\logs\update.log` |

pub mod component;
pub mod download;
pub mod endpoint;
pub mod http;
pub mod install;
pub mod log;
pub mod manifest;
pub mod mode;
pub mod paths;
pub mod signature;
pub mod state;
pub mod version;
pub mod zip;

mod engine;

pub use engine::{Outcome, Updater};
pub use manifest::Manifest;
pub use mode::UpdateMode;
pub use state::UpdateState;
pub use version::Version;

use std::fmt;

/// 更新流程里会发生的失败。分成两类（[`UpdateError::is_silent`]）：
///
/// - **静默**：网络不通、超时、404 / 403（限流）、没有更新。只写日志，不打扰用户，
///   下次按计划再试（issue #149 第 5 节）。
/// - **要说一声**：验签 / 校验失败、安装器出错、磁盘空间不足。发通知，并保证旧版本不受影响。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// 更新已关闭（`rove.update.mode = "none"`）。
    Disabled,
    /// 网络层失败（连不上、超时、DNS、代理）。静默。
    Network(String),
    /// HTTP 状态码不是 200。静默（404 = 还没有发布，403 = 限流）。
    Http(u32),
    /// 清单不是合法的 JSON，或者字段对不上。
    Manifest(String),
    /// 签名验证没通过（包括签名文件格式不对）。
    Signature(String),
    /// 版本号解析不了。
    Version(String),
    /// 下载内容与清单里的 `size` / `sha256` 对不上。
    Integrity(String),
    /// 磁盘空间不足或写文件失败。
    Storage(String),
    /// 安装器调用失败或退出码非 0。
    Install(String),
    /// 找不到要用的目录（`%LOCALAPPDATA%` 不存在等）。
    Unavailable(String),
}

impl UpdateError {
    /// 只写日志、不打扰用户的失败（issue #149 第 5 节第一条）。
    pub fn is_silent(&self) -> bool {
        matches!(
            self,
            UpdateError::Disabled | UpdateError::Network(_) | UpdateError::Http(_)
        )
    }

    /// 日志与「关于」对话框里显示的原因（短句，不含用户数据）。
    pub fn reason(&self) -> String {
        match self {
            UpdateError::Disabled => "更新已关闭".into(),
            UpdateError::Network(e) => format!("网络不可用（{e}）"),
            UpdateError::Http(code) => format!("服务器返回 {code}"),
            UpdateError::Manifest(e) => format!("更新清单无法解析（{e}）"),
            UpdateError::Signature(e) => format!("更新清单签名验证失败（{e}）"),
            UpdateError::Version(e) => format!("版本号无法解析（{e}）"),
            UpdateError::Integrity(e) => format!("下载内容校验失败（{e}）"),
            UpdateError::Storage(e) => format!("无法写入下载目录（{e}）"),
            UpdateError::Install(e) => format!("安装失败（{e}）"),
            UpdateError::Unavailable(e) => format!("更新不可用（{e}）"),
        }
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason())
    }
}

impl std::error::Error for UpdateError {}

pub type Result<T> = std::result::Result<T, UpdateError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// 第 5 节：网络类失败静默重试，校验类失败必须让用户知道。
    #[test]
    fn only_network_failures_are_silent() {
        assert!(UpdateError::Network("timeout".into()).is_silent());
        assert!(UpdateError::Http(403).is_silent());
        assert!(UpdateError::Disabled.is_silent());
        assert!(!UpdateError::Signature("bad".into()).is_silent());
        assert!(!UpdateError::Integrity("sha256".into()).is_silent());
        assert!(!UpdateError::Install("exit 1".into()).is_silent());
        assert!(!UpdateError::Storage("disk full".into()).is_silent());
    }

    /// 原因是给人看的短句，不应该空着。
    #[test]
    fn every_failure_has_a_reason() {
        for error in [
            UpdateError::Disabled,
            UpdateError::Network("x".into()),
            UpdateError::Http(404),
            UpdateError::Manifest("x".into()),
            UpdateError::Signature("x".into()),
            UpdateError::Version("x".into()),
            UpdateError::Integrity("x".into()),
            UpdateError::Storage("x".into()),
            UpdateError::Install("x".into()),
            UpdateError::Unavailable("x".into()),
        ] {
            assert!(!error.reason().is_empty(), "{error:?}");
            assert_eq!(error.to_string(), error.reason());
        }
    }
}
