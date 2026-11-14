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
    attach_with(view, cx, mode, dir, StubTransport::new());
}

/// 同上，但自带一个造好响应的替身传输（仍然不联网），并把它返回给调用方做断言。
fn attach_with(
    view: &Entity<Workbench>,
    cx: &mut gpui::VisualTestContext,
    mode: UpdateMode,
    dir: &std::path::Path,
    transport: StubTransport,
) -> StubTransport {
    let spy = transport.clone();
    let updater = Updater::with_transport(
        Version::parse("0.1.0").unwrap(),
        mode,
        Some(UpdatePaths::new(dir.join("Rove"))),
        Arc::new(transport),
    );
    view.update(cx, |v, _| {
        v.set_update_service(UpdateService::with_updater(updater, false))
    });
    spy
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

/// 点「下载」下的是**提示里那一版**：服务记着上次检查的清单，不重新查一遍。
#[gpui::test]
fn accepting_a_download_uses_the_manifest_from_the_prompt(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (view, cx) = open(cx);
    attach(&view, cx, UpdateMode::Start, dir.path());
    view.update(cx, |v, cx| {
        let service = v.update_service_mut().unwrap();
        let step = service.on_check_finished(
            &Outcome::Available {
                manifest: Box::new(manifest("0.1.1")),
                required: false,
            },
            false,
        );
        v.apply_update_step_for_test(step, cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let service = v.update_service().unwrap();
        assert_eq!(
            service.available().map(|m| m.version.as_str()),
            Some("0.1.1")
        );
    });
    // 点「下载」：真的开始下这一版，而不是再弹一次「发现新版本」。
    // 测试里的传输是 StubTransport，安装包取不到（404），所以下载会立刻以失败收尾——
    // 关键是它**确实下过**：记下的那一版清单被消费掉了，状态也不再停在 Available。
    view.update(cx, |v, cx| {
        v.update_feedback_action(reg::KEY_PROMPT, reg::ACTION_DOWNLOAD, cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let service = v.update_service().unwrap();
        assert!(
            service.available().is_none(),
            "清单没有被消费，说明没开始下载"
        );
        assert!(
            !matches!(service.state(), UpdateState::Available { .. }),
            "状态不该还停在「发现新版本」：{:?}",
            service.state()
        );
    });
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

// ---------------------------------------------------- #149 第 4 节：索引组件跟随更新

use rove_lang_cs::lifecycle::component as cs;

/// 测试用的「安装包」内容（清单里的 size / sha256 照它算）。
const INSTALLER: &[u8] = b"setup";

fn component_url(version: &str) -> String {
    format!("https://a/rove-indexer-cs-{version}-win-x64.zip")
}

/// 带安装包与可选组件的清单（size / sha256 都是真算出来的，下载会真的校验通过）。
fn manifest_with_component(version: &str, component: Option<(&str, &[u8])>) -> Manifest {
    let components = match component {
        None => "[]".to_string(),
        Some((component_version, archive)) => format!(
            r#"[{{"id":"indexer-cs","version":"{component_version}","file":"rove-indexer-cs-{component_version}-win-x64.zip","url":"{}","size":{},"sha256":"{}"}}]"#,
            component_url(component_version),
            archive.len(),
            rove_update::download::sha256(archive)
        ),
    };
    Manifest::parse(
        format!(
            r#"{{"schema":1,"product":"rove","version":"{version}","installer":{{"file":"s.exe","url":"https://a/s.exe","size":{},"sha256":"{}","silent_args":["/VERYSILENT"]}},"components":{components}}}"#,
            INSTALLER.len(),
            rove_update::download::sha256(INSTALLER)
        )
        .as_bytes(),
    )
    .unwrap()
}

fn component_manifest(version: &str) -> String {
    format!(
        r#"{{"component":"indexer-cs","version":"{version}","protocol":{},"protocol_min":{},"index_format":{}}}"#,
        rove_lang_cs::lifecycle::resident::PROTOCOL,
        rove_lang_cs::lifecycle::resident::PROTOCOL_MIN,
        rove_lang_cs::lifecycle::validity::EXPECTED_FORMAT,
    )
}

/// 本机已经装着的组件版本（布局与 #43 的发现规则一致）。
fn install_component(root: &std::path::Path, version: &str) {
    let dir = root.join(version);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(cs::EXE_FILE), b"MZ").unwrap();
    std::fs::write(dir.join(cs::MANIFEST_FILE), component_manifest(version)).unwrap();
}

/// 一个最小的组件包（只有清单与可执行文件）。
fn component_package(version: &str) -> Vec<u8> {
    zip_of(&[
        (cs::MANIFEST_FILE, component_manifest(version).as_bytes()),
        (cs::EXE_FILE, b"MZ pretend"),
    ])
}

/// 把组件位置指到临时目录（**绝不碰真实的 `%LOCALAPPDATA%\Rove\components`**）。
fn point_components_at(
    view: &Entity<Workbench>,
    cx: &mut gpui::VisualTestContext,
    root: &std::path::Path,
) {
    let roots = cs::Roots {
        env_override: None,
        dirs: vec![root.to_path_buf()],
    };
    view.update(cx, |v, _| {
        v.update_service_mut().unwrap().set_component_roots(roots)
    });
}

fn dir_names(dir: &std::path::Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// 走一遍「发现新版本 → 用户点下载」的产品路径。
fn download_update(view: &Entity<Workbench>, cx: &mut gpui::VisualTestContext, manifest: Manifest) {
    view.update(cx, |v, cx| {
        let step = v.update_service_mut().unwrap().on_check_finished(
            &Outcome::Available {
                manifest: Box::new(manifest),
                required: false,
            },
            false,
        );
        v.apply_update_step_for_test(step, cx);
    });
    cx.run_until_parked();
    view.update(cx, |v, cx| {
        v.update_feedback_action(reg::KEY_PROMPT, reg::ACTION_DOWNLOAD, cx)
    });
    cx.run_until_parked();
}

/// 验收：**已安装 C# 索引组件时，组件随更新升级**。
///
/// 走的是产品自己的路径（提示 → 点「下载」→ `update_host` 的下载流程），
/// 不是直接调 `rove-update` 的库函数。
#[gpui::test]
fn an_installed_component_is_upgraded_with_the_app(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let components = dir.path().join("components").join("indexer-cs");
    install_component(&components, "1.2.0.0");
    let archive = component_package("1.3.0.0");

    let (view, cx) = open(cx);
    let transport = StubTransport::new()
        .with("https://a/s.exe", INSTALLER.to_vec())
        .with(&component_url("1.3.0.0"), archive.clone());
    let spy = attach_with(&view, cx, UpdateMode::Start, dir.path(), transport);
    point_components_at(&view, cx, &components);

    download_update(
        &view,
        cx,
        manifest_with_component("0.1.1", Some(("1.3.0.0", &archive))),
    );

    // 组件真的装进了 `<组件根>\1.3.0.0\`，旧版本还在（要等下次启动才清理）。
    assert!(components.join("1.3.0.0").join(cs::MANIFEST_FILE).is_file());
    assert!(components.join("1.3.0.0").join(cs::EXE_FILE).is_file());
    assert_eq!(dir_names(&components), ["1.2.0.0", "1.3.0.0"]);
    assert!(spy.requested().contains(&component_url("1.3.0.0")));
    // 主程序这条主线照常走完：待安装版本记下了，组件版本也记进了 pending.json。
    view.read_with(cx, |v, _| {
        let service = v.update_service().unwrap();
        assert!(service.can_restart_to_update());
        assert_eq!(
            service.pending().unwrap().component_version.as_deref(),
            Some("1.3.0.0")
        );
    });
}

/// 验收：**未安装时不下载**组件。
#[gpui::test]
fn a_component_that_is_not_installed_is_never_downloaded(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let components = dir.path().join("components").join("indexer-cs");
    std::fs::create_dir_all(&components).unwrap();
    let archive = component_package("1.3.0.0");

    let (view, cx) = open(cx);
    let transport = StubTransport::new()
        .with("https://a/s.exe", INSTALLER.to_vec())
        .with(&component_url("1.3.0.0"), archive.clone());
    let spy = attach_with(&view, cx, UpdateMode::Start, dir.path(), transport);
    point_components_at(&view, cx, &components);

    download_update(
        &view,
        cx,
        manifest_with_component("0.1.1", Some(("1.3.0.0", &archive))),
    );

    assert!(dir_names(&components).is_empty(), "没装组件却下了组件包");
    assert!(
        !spy.requested().contains(&component_url("1.3.0.0")),
        "没装组件却请求了组件包：{:?}",
        spy.requested()
    );
    // 主程序更新不受影响。
    view.read_with(cx, |v, _| {
        assert!(v.update_service().unwrap().can_restart_to_update())
    });
}

/// 验收：**旧版本在下次启动后清理**（排在首帧之后，冷启动预算不受影响）。
#[gpui::test]
fn old_component_versions_are_cleaned_up_on_the_next_start(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let components = dir.path().join("components").join("indexer-cs");
    install_component(&components, "1.2.0.0");
    install_component(&components, "1.3.0.0");
    std::fs::create_dir_all(components.join("1.3.0.0.staging")).unwrap();

    let (view, cx) = open(cx);
    // `manual`：启动时不检查，所以这个测试只看清理这一件事。
    attach(&view, cx, UpdateMode::Manual, dir.path());
    point_components_at(&view, cx, &components);
    view.update(cx, |v, cx| v.start_update_checks(cx));
    cx.run_until_parked();

    assert_eq!(dir_names(&components), ["1.3.0.0"]);
}

/// `none` 时更新整个关着：不碰用户自己装的组件。
#[gpui::test]
fn mode_none_leaves_the_components_alone(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let components = dir.path().join("components").join("indexer-cs");
    install_component(&components, "1.2.0.0");
    install_component(&components, "1.3.0.0");

    let (view, cx) = open(cx);
    attach(&view, cx, UpdateMode::None, dir.path());
    point_components_at(&view, cx, &components);
    view.update(cx, |v, cx| v.start_update_checks(cx));
    cx.run_until_parked();

    assert_eq!(dir_names(&components), ["1.2.0.0", "1.3.0.0"]);
}

/// 现造一个「存储」方式的 zip（与 `rove-update` 的测试同一段代码，这里独立一份，
/// 免得入口测试去依赖别人的私有实现）。
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in files {
        let offset = out.len() as u32;
        let crc = crc32(data);
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&[0u8; 4]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_at = out.len() as u32;
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&central_at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}
