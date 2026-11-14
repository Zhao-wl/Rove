//! 「关于」对话框要显示的信息（issue #76 第 4 节）。
//!
//! 排查用户问题时这是第一手信息，所以要能**一键复制**成纯文本。信息的来源：
//!
//! | 行 | 来源 |
//! |---|---|
//! | 版本 | `CARGO_PKG_VERSION`（根 `Cargo.toml` 的 `[workspace.package] version`，唯一来源） |
//! | 提交号 / 构建日期 / 通道 | `rove-ui/build.rs` 注入的 `ROVE_BUILD_*`（发布流水线会设，本地构建问 git） |
//!
//! 「构建日期」取的是**提交日期**而不是跑构建的那一刻：同一个提交构建出的产物因此一致
//! （可复现构建），排查问题时「版本 + 提交号 + 日期」也能一一对上。
//! | gpui 版本 | 同上，从根 `Cargo.toml` 的依赖声明读出来 |
//! | C# 索引组件 | `rove_lang_cs::lifecycle::component::discover`（#43 的发现规则，不启动进程） |
//! | 可执行文件 | `std::env::current_exe()` |
//!
//! 这里**不读网络、不写磁盘**，只读进程自己的信息与组件目录。

use rove_lang_cs::lifecycle::component::{self, Discovery, Roots};
use std::path::PathBuf;

/// 版本号的唯一来源：工作区的 `version`。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("ROVE_BUILD_COMMIT");
pub const BUILD_DATE: &str = env!("ROVE_BUILD_DATE");
pub const CHANNEL: &str = env!("ROVE_BUILD_CHANNEL");
pub const GPUI_VERSION: &str = env!("ROVE_GPUI_VERSION");
/// 随包分发的第三方声明文件名（#76 第 6 节：随安装包与便携版一起分发）。
pub const NOTICES_FILE: &str = "THIRD_PARTY_NOTICES.md";

/// C# 索引组件的状态（#43 的三种结果）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Indexer {
    /// 已安装且兼容；带版本号（用环境变量覆盖的开发组件没有清单，为 `None`）。
    Installed(Option<String>),
    /// 装了但协议 / 索引格式对不上；带组件版本与方向。
    Incompatible {
        version: Option<String>,
        component_too_old: bool,
    },
    NotInstalled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AboutInfo {
    pub version: String,
    pub commit: String,
    pub build_date: String,
    pub channel: String,
    pub gpui_version: String,
    pub indexer: Indexer,
    pub executable: Option<PathBuf>,
    /// #149：更新状态（已是最新 / 正在下载 / 待重启安装 / 更新已关闭 / 上次检查失败）。
    /// `None` 表示这次装配没有更新服务（例如入口测试），那一行就不显示。
    pub update: Option<rove_update::UpdateState>,
}

impl AboutInfo {
    /// 当前进程的真实信息。
    pub fn current() -> Self {
        Self::with_indexer(Indexer::discover(&Roots::current()))
    }

    /// 组件状态由调用方给出（测试里用假的组件目录）。
    pub fn with_indexer(indexer: Indexer) -> Self {
        Self {
            version: VERSION.into(),
            commit: COMMIT.into(),
            build_date: BUILD_DATE.into(),
            channel: CHANNEL.into(),
            gpui_version: GPUI_VERSION.into(),
            indexer,
            executable: std::env::current_exe().ok(),
            update: None,
        }
    }

    /// #149：带上更新状态那一行。
    pub fn with_update(mut self, state: rove_update::UpdateState) -> Self {
        self.update = Some(state);
        self
    }

    /// 提交号只显示前 12 位（完整值在复制出来的文本里）。
    pub fn short_commit(&self) -> String {
        self.commit.chars().take(12).collect()
    }

    /// 随包的第三方声明：和 `rove.exe` 放在同一个目录（便携版与安装目录都是这样）。
    pub fn notices_path(&self) -> Option<PathBuf> {
        let path = self.executable.as_ref()?.parent()?.join(NOTICES_FILE);
        path.is_file().then_some(path)
    }
}

impl Indexer {
    pub fn discover(roots: &Roots) -> Self {
        match component::discover(roots) {
            Discovery::Found(found) => {
                Indexer::Installed(found.version.map(component::version_text))
            }
            Discovery::Incompatible(why) => Indexer::Incompatible {
                version: why.component_version.clone(),
                component_too_old: matches!(why.direction, component::Direction::ComponentTooOld),
            },
            Discovery::NotInstalled => Indexer::NotInstalled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_info_is_injected_by_the_build_script() {
        let info = AboutInfo::with_indexer(Indexer::NotInstalled);
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(!info.commit.is_empty());
        assert!(!info.build_date.is_empty());
        assert!(
            info.gpui_version.starts_with("0.2"),
            "gpui 版本应来自根 Cargo.toml，实际 {}",
            info.gpui_version
        );
        assert!(info.short_commit().len() <= 12);
    }

    /// 没装组件时是「未安装」；#43 的发现规则不变，这里只是读它。
    #[test]
    fn indexer_status_follows_the_component_discovery_rules() {
        let empty = Roots {
            env_override: None,
            dirs: vec![std::env::temp_dir().join("rove-about-no-such-dir")],
        };
        assert_eq!(Indexer::discover(&empty), Indexer::NotInstalled);
    }
}
