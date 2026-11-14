//! 自动更新的界面一侧（issue #149 第 3 节）。
//!
//! 核心逻辑（清单、验签、下载、状态机）在核心层的 `rove-update`；这里只做三件事：
//!
//! 1. [`registry`]：命令、界面字符串、齿轮菜单项（全部走 #1 的登记机制）；
//! 2. [`UpdateService`]：读设置、决定「现在该做什么」，**不碰 GPUI**，所以可以直接测；
//! 3. 窗口装配在 `skeleton/workbench/update_host.rs`：把 [`Step`] 翻译成提示条、通知与状态栏消息。
//!
//! 反馈分级按 `UX_SPEC` §6.1：
//!
//! | 情况 | 级别 |
//! |---|---|
//! | 「检查更新」的即时回执、已是最新、正在下载 | 1 状态栏临时消息 |
//! | 「重启以更新到 x.y.z」、发现新版本待确认 | 4 通知（带按钮，不会自动消失） |
//! | 校验失败、安装失败 | 4 通知（警告） |
//! | 网络不通、超时、404 / 403 | **什么都不显示**，只写日志 |
//!
//! 更新**永远不弹模态框**：模态框留给数据丢失与安全确认。

pub mod registry;

use rove_base::Error;
use rove_config::{Key, Scope, Settings};
use rove_update::{
    install, mode::UpdateMode, paths::Pending, state::UpdateState, Outcome, UpdateError, Updater,
    Version,
};
use std::{path::PathBuf, sync::Arc};

/// 版本号的唯一来源：工作区的 `version`（与「关于」对话框同一个）。
pub fn current_version() -> Version {
    Version::parse(crate::about::info::VERSION)
        .unwrap_or_else(|_| Version::parse("0.0.1").expect("兜底版本号"))
}

/// 从设置里读 `rove.update.mode`。读不到或取值认不出来时用默认值 `default`
/// （设置层会单独报格式问题，这里不重复报）。
pub fn mode_from_settings(settings: &dyn Settings) -> UpdateMode {
    match Key::<String>::new(rove_update::mode::SETTING_KEY).read(settings, &Scope::default()) {
        Ok(value) => UpdateMode::parse(&value).unwrap_or_default(),
        Err(Error::NotFound) => UpdateMode::default(),
        Err(_) => UpdateMode::default(),
    }
}

/// 界面接下来要做的一件事。窗口把它翻译成具体的反馈。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// 什么都不做（静默失败、已是最新且不是用户主动触发的检查）。
    Nothing,
    /// 第 1 级：状态栏临时消息。
    Status {
        key: &'static str,
        args: Vec<(&'static str, String)>,
    },
    /// 第 4 级：带按钮的通知。
    Prompt {
        key: &'static str,
        args: Vec<(&'static str, String)>,
        actions: Vec<&'static str>,
        warning: bool,
    },
    /// 开始后台下载（窗口把它丢到后台执行器上）。
    Download,
}

impl Step {
    fn status(key: &'static str, args: Vec<(&'static str, String)>) -> Self {
        Step::Status { key, args }
    }
}

/// 界面持有的更新服务。
pub struct UpdateService {
    updater: Arc<Updater>,
    state: UpdateState,
    /// 便携版不自己替换文件（#149「不包含」最后一条）。
    portable: bool,
    /// 下载好、等重启安装的那一个。
    pending: Option<Pending>,
    /// 已经发现、还没下载的那一版清单（`start` / `manual` 模式里用户点「下载」时直接用它，
    /// **不再重新查一遍**：那样既多一次请求，也可能拿到和提示里不一样的版本）。
    available: Option<Box<rove_update::Manifest>>,
}

impl UpdateService {
    /// 按设置与当前进程构造。
    pub fn open(settings: &dyn Settings) -> Self {
        let mode = mode_from_settings(settings);
        let updater = Updater::new(
            current_version(),
            mode,
            rove_update::paths::UpdatePaths::current(),
        );
        let exe = std::env::current_exe().unwrap_or_default();
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let portable = install::is_portable(&exe, local.as_deref());
        Self::with_updater(updater, portable)
    }

    /// 测试与演练用。
    pub fn with_updater(updater: Updater, portable: bool) -> Self {
        let state = if updater.mode().may_connect() {
            UpdateState::Idle
        } else {
            UpdateState::Disabled
        };
        Self {
            updater: Arc::new(updater),
            state,
            portable,
            pending: None,
            available: None,
        }
    }

    /// 已经发现、等用户确认下载的那一版清单。
    pub fn available(&self) -> Option<&rove_update::Manifest> {
        self.available.as_deref()
    }

    pub fn updater(&self) -> &Arc<Updater> {
        &self.updater
    }

    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    pub fn mode(&self) -> UpdateMode {
        self.updater.mode()
    }

    pub fn is_portable(&self) -> bool {
        self.portable
    }

    pub fn pending(&self) -> Option<&Pending> {
        self.pending.as_ref()
    }

    /// 启动时：上次留下的待安装版本还在吗？
    ///
    /// - 还比当前版本新 → 再提示一次（issue 第 3 节：用户忽略过也要再提示）；
    /// - 已经装上了 → `rove-update` 会把记录与下载清掉，这里什么都不显示。
    pub fn on_start(&mut self) -> Step {
        if let Some(pending) = self.updater.take_pending_after_start() {
            let version = pending.version.clone();
            self.pending = Some(pending);
            self.state = UpdateState::ReadyToInstall {
                version: version.clone(),
            };
            return Self::restart_prompt(&version);
        }
        Step::Nothing
    }

    /// 「检查更新」命令（用户主动触发：即使没有更新也要给个回执）。
    pub fn on_command_check(&mut self) -> Step {
        if !self.mode().may_connect() {
            self.state = UpdateState::Disabled;
            return Step::status("update.status.disabled", Vec::new());
        }
        if self.state.is_ready_to_install() {
            // 已经下载好了，直接提示重启，不必再查一遍。
            let version = self.state.pending_version().unwrap_or_default().to_string();
            return Self::restart_prompt(&version);
        }
        if self.state.is_busy() {
            return Step::status("update.status.checking", Vec::new());
        }
        self.state = UpdateState::Checking;
        Step::status("update.status.checking", Vec::new())
    }

    /// 一次检查有了结果。`manual` 为真表示这次检查是用户按命令触发的
    ///（决定「已是最新」要不要给回执）。
    pub fn on_check_finished(&mut self, outcome: &Outcome, manual: bool) -> Step {
        let now = rove_update::log::timestamp(std::time::SystemTime::now());
        match outcome {
            Outcome::Disabled => {
                self.state = UpdateState::Disabled;
                if manual {
                    Step::status("update.status.disabled", Vec::new())
                } else {
                    Step::Nothing
                }
            }
            Outcome::UpToDate => {
                self.state = UpdateState::UpToDate { checked_at: now };
                if manual {
                    Step::status(
                        "update.status.upToDate",
                        vec![("version", self.updater.current_version().to_string())],
                    )
                } else {
                    Step::Nothing
                }
            }
            Outcome::Failed { error } => {
                self.state = UpdateState::from_error(error, now);
                // 网络类失败静默（第 5 节：连续失败也不弹通知）。
                if error.is_silent() {
                    return Step::Nothing;
                }
                Self::failure(error)
            }
            Outcome::Available { manifest, required } => {
                let version = manifest.version.clone();
                // 便携版只提示，不下载、不替换自己的文件。
                if self.portable {
                    self.state = UpdateState::Available {
                        version: version.clone(),
                    };
                    return Step::status("update.status.portable", vec![("version", version)]);
                }
                // 清单里没有安装包（#76 在没装 Inno Setup 时会这样）：没什么可装的。
                if manifest.installer.is_none() {
                    self.state = UpdateState::Available {
                        version: version.clone(),
                    };
                    return Step::status("update.status.portable", vec![("version", version)]);
                }
                // 记住这一版清单：后台下载直接用它，`start` / `manual` 下用户点「下载」时
                // 也用它，**不再重新查一遍**（那样既多一次请求，也可能拿到和提示里不一样的版本）。
                self.available = Some(manifest.clone());
                if self.mode().downloads_in_background() {
                    self.state = UpdateState::Downloading {
                        version: version.clone(),
                    };
                    return Step::Download;
                }
                // `start` / `manual`：先问用户。
                self.state = UpdateState::Available {
                    version: version.clone(),
                };
                Step::Prompt {
                    key: if *required {
                        "update.prompt.required"
                    } else {
                        "update.prompt.available"
                    },
                    args: vec![("version", version)],
                    actions: vec![registry::ACTION_DOWNLOAD, registry::ACTION_LATER],
                    warning: *required,
                }
            }
        }
    }

    /// 用户在通知里点了「下载」。要下载的就是**提示里那一版**（上次检查记下的清单），
    /// 不重新检查：那样既多一次请求，也可能拿到和提示里不一样的版本。
    pub fn on_accept_download(&mut self) -> Step {
        let Some(manifest) = &self.available else {
            return Step::Nothing;
        };
        self.state = UpdateState::Downloading {
            version: manifest.version.clone(),
        };
        Step::Download
    }

    /// 下载结束。
    pub fn on_download_finished(&mut self, result: Result<Pending, UpdateError>) -> Step {
        // 下载结束之后这一版清单就没用了（成功 → 看 `pending`；失败 → 下次检查重新拿）。
        self.available = None;
        match result {
            Ok(pending) => {
                let version = pending.version.clone();
                self.pending = Some(pending);
                self.state = UpdateState::ReadyToInstall {
                    version: version.clone(),
                };
                Self::restart_prompt(&version)
            }
            Err(error) => {
                let now = rove_update::log::timestamp(std::time::SystemTime::now());
                self.state = UpdateState::from_error(&error, now);
                self.updater.discard_pending();
                self.pending = None;
                if error.is_silent() {
                    Step::Nothing
                } else {
                    Self::failure(&error)
                }
            }
        }
    }

    /// 「重启以更新」命令可不可用。
    pub fn can_restart_to_update(&self) -> bool {
        self.pending.is_some() && !self.portable
    }

    /// 用户选择现在重启安装。返回要显示的回执；真正的保存会话与退出由窗口做。
    pub fn on_restart_requested(&self) -> Step {
        if !self.can_restart_to_update() {
            return Step::Nothing;
        }
        Step::status("update.status.restarting", Vec::new())
    }

    /// 安装失败（安装器退出码非 0、重启后版本号没变）：通知并保留旧版本。
    pub fn on_install_failed(&mut self, reason: &str) -> Step {
        self.updater.discard_pending();
        self.pending = None;
        self.state = UpdateState::Failed {
            at: rove_update::log::timestamp(std::time::SystemTime::now()),
            reason: reason.to_string(),
        };
        Step::Prompt {
            key: "update.notify.installFailed",
            args: vec![("reason", reason.to_string())],
            actions: Vec::new(),
            warning: true,
        }
    }

    fn restart_prompt(version: &str) -> Step {
        Step::Prompt {
            key: "update.prompt.restart",
            args: vec![("version", version.to_string())],
            actions: vec![registry::ACTION_RESTART, registry::ACTION_LATER],
            warning: false,
        }
    }

    fn failure(error: &UpdateError) -> Step {
        Step::Prompt {
            key: "update.notify.failed",
            args: vec![("reason", error.reason())],
            actions: Vec::new(),
            warning: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_update::{http::StubTransport, manifest::Manifest, paths::UpdatePaths};

    fn manifest(version: &str, with_installer: bool) -> Manifest {
        let installer = if with_installer {
            format!(
                r#""installer":{{"file":"s.exe","url":"https://a/s.exe","size":1,"sha256":"{}","silent_args":["/VERYSILENT"]}}"#,
                "0".repeat(64)
            )
        } else {
            r#""installer":null"#.to_string()
        };
        Manifest::parse(
            format!(r#"{{"schema":1,"product":"rove","version":"{version}",{installer}}}"#)
                .as_bytes(),
        )
        .unwrap()
    }

    fn make_service(mode: UpdateMode, portable: bool) -> (tempfile::TempDir, UpdateService) {
        let dir = tempfile::tempdir().unwrap();
        let updater = Updater::with_transport(
            Version::parse("0.1.0").unwrap(),
            mode,
            Some(UpdatePaths::new(dir.path().join("Rove"))),
            Arc::new(StubTransport::new()),
        );
        (dir, UpdateService::with_updater(updater, portable))
    }

    /// 验收：`rove.update.mode` 四个取值各有行为。
    #[test]
    fn the_four_modes_behave_differently() {
        // none：命令只回一句「已关闭」，状态是「更新已关闭」。
        let (_d, mut none) = make_service(UpdateMode::None, false);
        assert_eq!(none.state(), &UpdateState::Disabled);
        assert_eq!(
            none.on_command_check(),
            Step::status("update.status.disabled", Vec::new())
        );
        assert!(none.updater().transport().is_none());

        // manual / start：发现新版本先问，不自动下载。
        for mode in [UpdateMode::Manual, UpdateMode::Start] {
            let (_d, mut service) = make_service(mode, false);
            let step = service.on_check_finished(
                &Outcome::Available {
                    manifest: Box::new(manifest("0.1.1", true)),
                    required: false,
                },
                true,
            );
            let Step::Prompt { key, actions, .. } = &step else {
                panic!("{mode:?} 应该先问用户，实际 {step:?}")
            };
            assert_eq!(*key, "update.prompt.available");
            assert_eq!(
                actions,
                &[registry::ACTION_DOWNLOAD, registry::ACTION_LATER]
            );
            // 点「下载」才开始下。
            assert_eq!(service.on_accept_download(), Step::Download);
        }

        // default：直接后台下载。
        let (_d, mut auto) = make_service(UpdateMode::Default, false);
        assert_eq!(
            auto.on_check_finished(
                &Outcome::Available {
                    manifest: Box::new(manifest("0.1.1", true)),
                    required: false,
                },
                false,
            ),
            Step::Download
        );
        assert_eq!(
            auto.state(),
            &UpdateState::Downloading {
                version: "0.1.1".into()
            }
        );
    }

    /// 网络类失败静默；校验类失败发通知。
    #[test]
    fn silent_failures_stay_silent() {
        let (_d, mut service) = make_service(UpdateMode::Default, false);
        for error in [
            UpdateError::Network("连不上".into()),
            UpdateError::Http(403),
            UpdateError::Http(404),
        ] {
            assert_eq!(
                service.on_check_finished(
                    &Outcome::Failed {
                        error: error.clone()
                    },
                    false
                ),
                Step::Nothing,
                "{error:?}"
            );
            // 但状态里留下了时间与原因，「关于」对话框看得到。
            assert!(matches!(service.state(), UpdateState::Failed { .. }));
        }
        let step = service.on_check_finished(
            &Outcome::Failed {
                error: UpdateError::Integrity("sha256".into()),
            },
            false,
        );
        let Step::Prompt { key, warning, .. } = step else {
            panic!("校验失败要发通知")
        };
        assert_eq!(key, "update.notify.failed");
        assert!(warning);
    }

    /// 下载完成 → 「重启以更新到 x.y.z」；失败 → 通知 + 丢掉下载。
    #[test]
    fn a_finished_download_offers_a_restart() {
        let (_d, mut service) = make_service(UpdateMode::Default, false);
        let pending = Pending {
            version: "0.1.1".into(),
            file: "s.exe".into(),
            sha256: "0".repeat(64),
            size: 1,
            silent_args: vec!["/VERYSILENT".into()],
            downloaded_at: String::new(),
            component_version: None,
        };
        let step = service.on_download_finished(Ok(pending));
        let Step::Prompt {
            key, args, actions, ..
        } = &step
        else {
            panic!("{step:?}")
        };
        assert_eq!(*key, "update.prompt.restart");
        assert_eq!(args[0].1, "0.1.1");
        assert_eq!(actions[0], registry::ACTION_RESTART);
        assert!(service.can_restart_to_update());

        let (_d, mut failing) = make_service(UpdateMode::Default, false);
        let step = failing.on_download_finished(Err(UpdateError::Integrity("体积不符".into())));
        assert!(matches!(step, Step::Prompt { warning: true, .. }));
        assert!(!failing.can_restart_to_update());
    }

    /// 便携版只提示，不下载（#149「不包含」最后一条）。
    #[test]
    fn a_portable_build_only_points_at_the_release_page() {
        let (_d, mut service) = make_service(UpdateMode::Default, true);
        let step = service.on_check_finished(
            &Outcome::Available {
                manifest: Box::new(manifest("0.1.1", true)),
                required: false,
            },
            true,
        );
        assert_eq!(
            step,
            Step::status(
                "update.status.portable",
                vec![("version", "0.1.1".to_string())]
            )
        );
        assert!(!service.can_restart_to_update());
    }

    /// 清单里没有安装包时也只提示，不会去下载一个不存在的东西。
    #[test]
    fn a_release_without_an_installer_is_only_announced() {
        let (_d, mut service) = make_service(UpdateMode::Default, false);
        let step = service.on_check_finished(
            &Outcome::Available {
                manifest: Box::new(manifest("0.1.1", false)),
                required: false,
            },
            false,
        );
        assert!(matches!(step, Step::Status { .. }), "{step:?}");
    }

    /// 用户主动检查时「已是最新」要给回执；后台检查则什么都不说。
    #[test]
    fn only_a_manual_check_reports_being_up_to_date() {
        let (_d, mut service) = make_service(UpdateMode::Default, false);
        assert_eq!(
            service.on_check_finished(&Outcome::UpToDate, false),
            Step::Nothing
        );
        let step = service.on_check_finished(&Outcome::UpToDate, true);
        assert!(matches!(
            step,
            Step::Status {
                key: "update.status.upToDate",
                ..
            }
        ));
    }

    /// 设置读不到或写错时按默认值 `default`。
    #[test]
    fn the_mode_falls_back_to_default() {
        let registry = rove_config::catalog::builtin_registry();
        let settings = rove_config::DefaultSettings(registry);
        assert_eq!(mode_from_settings(&settings), UpdateMode::Default);
    }

    /// 必须更新（`min_version`）时通知是警告级，文案也不同。
    #[test]
    fn a_required_update_is_a_warning() {
        let (_d, mut service) = make_service(UpdateMode::Start, false);
        let step = service.on_check_finished(
            &Outcome::Available {
                manifest: Box::new(manifest("0.2.0", true)),
                required: true,
            },
            false,
        );
        let Step::Prompt { key, warning, .. } = step else {
            panic!("应该是通知")
        };
        assert_eq!(key, "update.prompt.required");
        assert!(warning);
    }
}
