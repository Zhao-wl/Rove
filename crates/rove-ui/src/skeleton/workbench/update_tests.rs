//! #149 的入口集成测试：`update.checkForUpdates` / `update.restartToUpdate`。
//!
//! 测试里的更新服务挂的是 [`StubTransport`]（**绝不联网**），地址与真实构造一致。
//! 真实网络路径由 `rove-update` 的 `tests/rehearsal.rs` 用本机 HTTP 服务器覆盖。
//!
//! 验收口径：
//!
//! - 四个 `rove.update.mode` 取值在界面上的行为各不相同；
//! - `none` 时命令只回一句「更新已关闭」，**一个请求都不发**；
//! - 下载好之后出现带「重启」「稍后」按钮的通知（第 4 级），不是模态框；
//! - 「关于」对话框多一行更新状态；
//! - 更新**不往状态栏加常驻条目**。
use super::*;
use crate::update::{registry as reg, UpdateService};
use gpui::TestAppContext;
use rove_update::{
    http::StubTransport, manifest::Manifest, mode::UpdateMode, paths::UpdatePaths, Outcome,
    UpdateState, Updater, Version,
};

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut gpui::VisualTestContext) {
    let (view, cx) = cx.add_window_view(|_, cx| Workbench::empty(Instant::now(), cx));
    cx.run_until_parked();
    (view, cx)
}

/// 挂一个不联网的更新服务。
fn attach(
    view: &Entity<Workbench>,
    cx: &mut gpui::VisualTestContext,
    mode: UpdateMode,
    dir: &std::path::Path,
) {
    let updater = Updater::with_transport(
        Version::parse("0.1.0").unwrap(),
        mode,
        Some(UpdatePaths::new(dir.join("Rove"))),
        Arc::new(StubTransport::new()),
    );
    view.update(cx, |v, _| {
        v.set_update_service(UpdateService::with_updater(updater, false))
    });
}

fn manifest(version: &str) -> Manifest {
    Manifest::parse(
        format!(
            r#"{{"schema":1,"product":"rove","version":"{version}","installer":{{"file":"s.exe","url":"https://a/s.exe","size":1,"sha256":"{}","silent_args":["/VERYSILENT"]}}}}"#,
            "0".repeat(64)
        )
        .as_bytes(),
    )
    .unwrap()
}

/// 状态栏当前这条临时消息的文字。
fn status_text(view: &Entity<Workbench>, cx: &mut gpui::VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| {
        v.shell
            .feedback
            .status()
            .map(|s| rove_i18n::Strings::text(&v.strings, &s.message).unwrap())
    })
}

/// 当前这条通知的文字与按钮。
fn notification(
    view: &Entity<Workbench>,
    cx: &mut gpui::VisualTestContext,
) -> Option<(String, Vec<String>)> {
    view.read_with(cx, |v, _| {
        v.shell.feedback.notifications().first().map(|n| {
            (
                rove_i18n::Strings::text(&v.strings, &n.message).unwrap(),
                n.actions.iter().map(|a| a.id.to_string()).collect(),
            )
        })
    })
}

/// 验收：`none` 时「检查更新」只提示更新已关闭，而且**一个请求都没发**。
#[gpui::test]
fn mode_none_only_says_updates_are_off(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (view, cx) = open(cx);
    attach(&view, cx, UpdateMode::None, dir.path());
    view.update(cx, |v, cx| v.update_command(reg::CHECK_FOR_UPDATES, cx));
    cx.run_until_parked();
    assert_eq!(
        status_text(&view, cx).as_deref(),
        Some("更新已关闭（rove.update.mode）")
    );
    view.read_with(cx, |v, _| {
        let service = v.update_service().unwrap();
        assert_eq!(service.state(), &UpdateState::Disabled);
        // 传输层根本没有构造出来 → 不可能发请求。
        assert!(service.updater().transport().is_none());
    });
    // 没有通知、没有模态框。
    assert_eq!(notification(&view, cx), None);
}

/// 验收：`manual` 发现新版本时先问用户（通知 + 两个按钮），不是模态框。
#[gpui::test]
fn a_new_version_is_offered_as_a_notification(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (view, cx) = open(cx);
    attach(&view, cx, UpdateMode::Manual, dir.path());
    view.update(cx, |v, cx| {
        let service = v.update_service_mut().unwrap();
        let step = service.on_check_finished(
            &Outcome::Available {
                manifest: Box::new(manifest("0.1.1")),
                required: false,
            },
            true,
        );
        v.apply_update_step_for_test(step, cx);
    });
    cx.run_until_parked();
    let (text, actions) = notification(&view, cx).expect("有通知");
    assert_eq!(text, "发现新版本 Rove 0.1.1");
    assert_eq!(actions, [reg::ACTION_DOWNLOAD, reg::ACTION_LATER]);
    // UX_SPEC §6.1：更新不弹模态框。
    view.read_with(cx, |v, _| assert!(v.shell.feedback.modal().is_none()));
}

/// 验收：下载好之后提示「重启以更新到 x.y.z」，按钮是「重启」与「稍后」。
#[gpui::test]
fn a_downloaded_update_offers_a_restart(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (view, cx) = open(cx);
    attach(&view, cx, UpdateMode::Default, dir.path());
    view.update(cx, |v, cx| {
        let service = v.update_service_mut().unwrap();
        let step = service.on_download_finished(Ok(rove_update::paths::Pending {
            version: "0.1.1".into(),
            file: "s.exe".into(),
            sha256: "0".repeat(64),
            size: 1,
            silent_args: vec!["/VERYSILENT".into()],
            downloaded_at: String::new(),
            component_version: None,
        }));
        v.apply_update_step_for_test(step, cx);
    });
    cx.run_until_parked();
    let (text, actions) = notification(&view, cx).expect("有通知");
    assert_eq!(text, "已下载 Rove 0.1.1，重启以更新");
    assert_eq!(actions, [reg::ACTION_RESTART, reg::ACTION_LATER]);
    view.read_with(cx, |v, _| {
        assert!(v.update_service().unwrap().can_restart_to_update());
        assert_eq!(
            v.update_service().unwrap().state(),
            &UpdateState::ReadyToInstall {
                version: "0.1.1".into()
            }
        );
    });

    // 点「稍后」什么都不做，待安装版本还在（下次启动会再提示一次）。
    view.update(cx, |v, cx| {
        v.update_feedback_action(reg::KEY_PROMPT, reg::ACTION_LATER, cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.update_service().unwrap().can_restart_to_update())
    });
}

/// 验收：「关于」对话框显示更新状态。
#[gpui::test]
fn the_about_dialog_shows_the_update_state(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (view, cx) = open(cx);
    attach(&view, cx, UpdateMode::None, dir.path());
    view.update(cx, |v, cx| {
        v.about_command(crate::about::registry::SHOW_ABOUT, cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let info = v.about.info.as_ref().expect("对话框已打开");
        assert_eq!(info.update, Some(UpdateState::Disabled));
        let lines = crate::about::lines(&v.strings, info);
        let update = lines
            .iter()
            .find(|l| l.label == "更新")
            .expect("有更新那一行");
        assert_eq!(update.value, "已关闭（rove.update.mode = none）");
    });
}

/// 验收：更新不往状态栏加常驻条目（只用提示条与通知）。
#[gpui::test]
fn updates_add_no_status_bar_item(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (view, cx) = open(cx);
    attach(&view, cx, UpdateMode::Default, dir.path());
    view.read_with(cx, |v, _| {
        assert!(
            !v.shell
                .contributions
                .entries()
                .any(|(id, _)| id.starts_with("update.")),
            "更新不应该登记状态栏条目或视图"
        );
    });
}

/// 「检查更新」在命令面板与齿轮菜单里都可用（走登记机制，不写死）。
#[gpui::test]
fn the_check_command_is_implemented_and_in_the_gear_menu(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.read_with(cx, |_, cx| {
        assert!(crate::keys::is_implemented(reg::CHECK_FOR_UPDATES, cx));
        assert!(crate::keys::is_implemented(reg::RESTART_TO_UPDATE, cx));
    });
    view.read_with(cx, |v, _| {
        assert!(v
            .shell
            .menus
            .entries()
            .any(|(_, item)| item.command.as_ref() == reg::CHECK_FOR_UPDATES));
    });
}
