//! #149：自动更新的窗口装配。
//!
//! 决策在 `crate::update::UpdateService`（不依赖 gpui），联网在 `rove-update`（核心层）；
//! 这里只负责四件事：
//!
//! 1. **把检查排到首帧之后**：`render` 画完第一帧才 [`Workbench::start_update_checks`]，
//!    所以冷启动预算完全不受影响（#149 验收第 6 条）。`default` 模式之后每 6 小时再查一次。
//! 2. **把 [`Step`] 翻译成反馈**：状态栏临时消息（第 1 级）或带按钮的通知（第 4 级）。
//!    **不弹模态框**（UX_SPEC §6.1 把它留给数据丢失与安全确认），**不加状态栏常驻条目**。
//! 3. **「重启以更新」**：先走 #50 的热退出把会话与未保存的修改存好 → 关窗口 →
//!    拉起安装包（静默参数来自清单）→ 安装器装完再把 Rove 拉起来。
//! 4. **C# 索引组件跟着一起升级**（第 4 节）：安装包下好之后调
//!    [`crate::update::component::follow_update`]（装了才下，没装不下），
//!    下次启动的首帧之后调 [`crate::update::component::cleanup`] 删掉旧版本目录。
//!
//! 检查与下载都在**后台执行器**上跑：它们会阻塞在网络 I/O 上，绝不能占 UI 线程。

use super::*;
use crate::update::{registry as update_registry, Step, UpdateService};
use rove_update::{mode::UpdateMode, Outcome};

/// 窗口的更新状态。
#[derive(Default)]
pub(super) struct Updates {
    /// 只有应用入口会装（测试入口 `Workbench::new` 不装，单元测试不联网、不碰用户目录）。
    pub(super) service: Option<UpdateService>,
    /// 已经排过「首帧之后的第一次检查」。
    scheduled: bool,
}

impl Workbench {
    /// 应用入口调用：按 `rove.update.mode` 建更新服务，并把上次留下的待安装版本提示出来。
    pub(crate) fn start_updates(&mut self, cx: &mut Context<Self>) {
        let mut service = UpdateService::open(&*self.settings);
        // 启动时先看有没有「下载好了还没装」的版本（用户上次点了「稍后」）。
        let step = service.on_start();
        self.updates.service = Some(service);
        self.apply_update_step(step, cx);
    }

    /// 测试与演练用：直接给一个造好的服务（不联网）。
    #[cfg(test)]
    pub(crate) fn set_update_service(&mut self, service: UpdateService) {
        self.updates.service = Some(service);
    }

    pub(crate) fn update_service(&self) -> Option<&UpdateService> {
        self.updates.service.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn update_service_mut(&mut self) -> Option<&mut UpdateService> {
        self.updates.service.as_mut()
    }

    /// 测试用：直接把一步决策翻译成反馈（不经过网络）。
    #[cfg(test)]
    pub(crate) fn apply_update_step_for_test(&mut self, step: Step, cx: &mut Context<Self>) {
        self.apply_update_step(step, cx);
    }

    /// 首帧画完之后才排第一次检查（冷启动预算不受影响）。
    pub(super) fn start_update_checks(&mut self, cx: &mut Context<Self>) {
        if self.updates.scheduled {
            return;
        }
        let Some(service) = &self.updates.service else {
            return;
        };
        let mode = service.mode();
        self.updates.scheduled = true;
        // 首帧之后顺手把不再使用的旧组件版本删掉（#149 第 4 节）：纯本地文件操作，
        // 和检查一样排在首帧之后，冷启动预算不受影响。
        self.cleanup_old_components(cx);
        let Some(delay) = mode.startup_check() else {
            // `none` / `manual`：启动时不检查。
            return;
        };
        self.schedule_update_check(delay, mode, cx);
    }

    /// 启动时清掉不再使用的旧组件版本（#149 第 4 节）。删不掉说明进程还占着文件，
    /// 留到下一次启动再说。
    fn cleanup_old_components(&mut self, cx: &mut Context<Self>) {
        let Some(service) = &self.updates.service else {
            return;
        };
        if !service.mode().may_connect() {
            // `none`：更新整个关着，不去动用户自己装的组件。
            return;
        }
        let roots = service.component_roots().clone();
        let keep = service.installed_component().map(str::to_string);
        let log = service.updater().log().clone();
        cx.background_executor()
            .spawn(async move {
                let removed = crate::update::component::cleanup(&roots, keep.as_deref());
                if removed > 0 {
                    log.line(&format!("component cleanup: 删掉 {removed} 个旧版本目录"));
                }
            })
            .detach();
    }

    fn schedule_update_check(
        &mut self,
        delay: std::time::Duration,
        mode: UpdateMode,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            loop {
                if this
                    .update(cx, |view, cx| view.run_update_check(false, cx))
                    .is_err()
                {
                    break;
                }
                // `default` 模式之后每 6 小时再查一次；其余模式只查这一次。
                let Some(interval) = mode.repeat_interval() else {
                    break;
                };
                cx.background_executor().timer(interval).await;
            }
        })
        .detach();
    }

    /// 跑一次检查（在后台执行器上）。`manual` 为真表示是「检查更新」命令触发的。
    pub(crate) fn run_update_check(&mut self, manual: bool, cx: &mut Context<Self>) {
        let Some(service) = &self.updates.service else {
            return;
        };
        if service.state().is_busy() && !manual {
            return;
        }
        let updater = service.updater().clone();
        cx.spawn(async move |this, cx| {
            // 网络 I/O 放后台线程，UI 线程一帧都不等。
            let outcome = cx
                .background_executor()
                .spawn(async move { updater.check() })
                .await;
            let _ = this.update(cx, |view, cx| view.finish_update_check(outcome, manual, cx));
        })
        .detach();
    }

    fn finish_update_check(&mut self, outcome: Outcome, manual: bool, cx: &mut Context<Self>) {
        let Some(service) = self.updates.service.as_mut() else {
            return;
        };
        let step = service.on_check_finished(&outcome, manual);
        self.apply_update_step(step.clone(), cx);
        if step == Step::Download {
            self.start_update_download(cx);
        }
    }

    /// 下载服务记下的那一版（`UpdateService::available`），**不重新检查**。
    fn start_update_download(&mut self, cx: &mut Context<Self>) {
        let Some(manifest) = self
            .updates
            .service
            .as_ref()
            .and_then(|service| service.available().cloned())
        else {
            return;
        };
        self.run_update_download(manifest, cx);
    }

    /// 后台下载安装包并校验。
    fn run_update_download(&mut self, manifest: rove_update::Manifest, cx: &mut Context<Self>) {
        let Some(service) = &self.updates.service else {
            return;
        };
        let updater = service.updater().clone();
        let roots = service.component_roots().clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let result = updater.download(&manifest);
                    // 安装包下好了，C# 索引组件跟着一起升级（#149 第 4 节）：
                    // **装了才下，没装不下**；组件这一步失败只写日志，不影响主程序这条主线。
                    if result.is_ok() {
                        crate::update::component::follow_update(&updater, &manifest, &roots);
                    }
                    result
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                let Some(service) = view.updates.service.as_mut() else {
                    return;
                };
                let step = service.on_download_finished(result);
                view.apply_update_step(step, cx);
            });
        })
        .detach();
    }

    /// 命令分发：`update.checkForUpdates` / `update.restartToUpdate`。
    pub(super) fn update_command(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            update_registry::CHECK_FOR_UPDATES => {
                let Some(service) = self.updates.service.as_mut() else {
                    return;
                };
                let step = service.on_command_check();
                let connects =
                    service.mode().may_connect() && !service.state().is_ready_to_install();
                self.apply_update_step(step, cx);
                if connects {
                    self.run_update_check(true, cx);
                }
            }
            update_registry::RESTART_TO_UPDATE => self.restart_to_update(cx),
            _ => {}
        }
    }

    /// 提示条 / 通知上的按钮。
    pub(crate) fn update_feedback_action(
        &mut self,
        key: &str,
        action: &str,
        cx: &mut Context<Self>,
    ) {
        let _ = key;
        match action {
            update_registry::ACTION_RESTART => self.restart_to_update(cx),
            update_registry::ACTION_DOWNLOAD => {
                let Some(service) = self.updates.service.as_mut() else {
                    return;
                };
                // 用户确认后才下载（`start` / `manual` 模式）：下的就是提示里那一版。
                let step = service.on_accept_download();
                if step == Step::Download {
                    self.start_update_download(cx);
                }
            }
            // 「稍后」：什么都不做。下次启动还会再提示一次（issue 第 3 节）。
            update_registry::ACTION_LATER => {}
            _ => {}
        }
    }

    /// 「重启以更新」：保存会话 → 退出 → 静默安装 → 重新启动。
    fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.updates.service.as_ref() else {
            return;
        };
        if !service.can_restart_to_update() {
            return;
        }
        let Some(pending) = service.pending().cloned() else {
            return;
        };
        let Some(paths) = service.updater().paths().cloned() else {
            return;
        };
        let step = service.on_restart_requested();
        self.apply_update_step(step, cx);

        // #50：走和关窗口完全一样的那条路——热退出把会话与未保存的修改存好、写下干净的
        // 退出标记。`files.hotExit` 关着且有脏文档时它会先问用户，这时**先不装**：
        // 等用户处理完再点一次「重启以更新」。
        if !self.request_window_close(cx) {
            return;
        }
        let installer = pending.installer_path(&paths);
        let exe = std::env::current_exe().unwrap_or_default();
        if let Err(error) = rove_update::install::launch(&pending, &installer, &exe) {
            let Some(service) = self.updates.service.as_mut() else {
                return;
            };
            let step = service.on_install_failed(&error.reason());
            self.apply_update_step(step, cx);
            return;
        }
        // 安装器要等 rove.exe 退出才能替换文件：和 #49 关窗口走同一条收尾路径。
        if let Some(window) = self.exit_window_handle() {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    /// 把一步决策翻译成界面反馈。
    fn apply_update_step(&mut self, step: Step, cx: &mut Context<Self>) {
        match step {
            Step::Nothing | Step::Download => {}
            Step::Status { key, args } => {
                let _ = shell::view::report(
                    self,
                    update_registry::KEY_STATUS,
                    Reason::Receipt,
                    message(key, &args),
                    Arc::from([]),
                    cx,
                );
            }
            Step::Prompt {
                key,
                args,
                actions,
                warning,
            } => {
                let feedback_key = if warning {
                    update_registry::KEY_FAILED
                } else {
                    update_registry::KEY_PROMPT
                };
                let actions: Vec<crate::feedback::FeedbackAction> = actions
                    .iter()
                    .map(|action| crate::feedback::FeedbackAction {
                        id: Arc::from(*action),
                        label: Message::key(action_label(action)),
                    })
                    .collect();
                let severity = if warning {
                    crate::feedback::Severity::Warning
                } else {
                    crate::feedback::Severity::Info
                };
                let _ = shell::view::report(
                    self,
                    feedback_key,
                    Reason::Global(severity),
                    message(key, &args),
                    Arc::from(actions),
                    cx,
                );
            }
        }
        cx.notify();
    }
}

impl Workbench {
    /// 收尾用的窗口句柄（#49 关窗口流程记下来的那一个）。
    fn exit_window_handle(&self) -> Option<gpui::AnyWindowHandle> {
        self.editing.window
    }
}

/// 按钮 ID → 已登记的标签 key。
fn action_label(action: &str) -> &'static str {
    match action {
        update_registry::ACTION_RESTART => "update.action.restart",
        update_registry::ACTION_DOWNLOAD => "update.action.download",
        _ => "update.action.later",
    }
}

fn message(key: &str, args: &[(&'static str, String)]) -> Message {
    Message {
        key: key.into(),
        args: args
            .iter()
            .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
            .collect(),
    }
}
