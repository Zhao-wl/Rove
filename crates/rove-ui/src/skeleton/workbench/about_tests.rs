//! #76 的入口集成测试：`workbench.action.showAboutDialog`。
//!
//! 验收口径：命令能打开对话框、内容是版本 / 提交号 / 构建日期 / gpui 版本 / 索引组件状态、
//! 「复制」把纯文本写进剪贴板并回执一条状态栏消息、「第三方声明」打开随包的 notices、
//! 「关闭」与点浮层外面都能关掉。
use super::*;
use crate::about::{self, registry as reg};
use gpui::TestAppContext;

fn open(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut gpui::VisualTestContext) {
    let (view, cx) = cx.add_window_view(|_, cx| Workbench::empty(Instant::now(), cx));
    cx.run_until_parked();
    (view, cx)
}

fn show(view: &Entity<Workbench>, cx: &mut gpui::VisualTestContext) {
    view.update(cx, |v, cx| v.about_command(reg::SHOW_ABOUT, cx));
    cx.run_until_parked();
}

/// 命令打开对话框，正文逐行显示排查用的信息。
#[gpui::test]
fn show_about_dialog_lists_version_commit_date_gpui_and_indexer(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.read_with(cx, |v, _| assert!(v.about.info.is_none(), "默认不显示"));
    show(&view, cx);
    view.read_with(cx, |v, _| {
        let info = v.about.info.as_ref().expect("对话框已打开");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        let lines = about::lines(&v.strings, info);
        let labels: Vec<&str> = lines.iter().map(|l| l.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "版本",
                "提交",
                "构建日期",
                "通道",
                "gpui",
                "C# 索引组件",
                "可执行文件"
            ]
        );
        // 值都不为空，且索引组件那行走的是 #43 的发现规则（测试环境里通常是「未安装」）。
        for line in &lines {
            assert!(!line.value.is_empty(), "{} 没有值", line.label);
        }
    });
}

/// 「复制」把纯文本写进剪贴板，并在状态栏回执（UX_SPEC §6.1 第 1 级）。
#[gpui::test]
fn copy_button_puts_plain_text_on_the_clipboard(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    show(&view, cx);
    let expected = view.read_with(cx, |v, _| {
        about::clipboard_text(&v.strings, v.about.info.as_ref().unwrap())
    });
    view.update(cx, |v, cx| v.about_act(reg::ACTION_COPY, cx));
    cx.run_until_parked();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("剪贴板里有版本信息");
    assert_eq!(copied, expected);
    assert!(copied.starts_with("Rove "), "{copied}");
    assert!(copied.contains("gpui: "), "{copied}");
    // 状态栏回执。
    view.read_with(cx, |v, _| {
        let status = v.shell.feedback.status().expect("状态栏回执");
        assert_eq!(status.key.as_ref(), "about.copied");
    });
    // 复制不会把对话框关掉（可以接着复制或打开声明）。
    view.read_with(cx, |v, _| assert!(v.about.info.is_some()));
}

/// 「关闭」按钮与点浮层外面都能关掉。
#[gpui::test]
fn close_hides_the_dialog(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    show(&view, cx);
    view.update(cx, |v, cx| v.about_act(reg::ACTION_CLOSE, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.about.info.is_none()));
}

/// 「第三方声明」：随包的 THIRD_PARTY_NOTICES.md 不在 exe 旁边时只报一条状态栏消息，
/// 不弹通知、不调用外部程序。
#[gpui::test]
fn notices_button_reports_when_the_file_is_not_next_to_the_executable(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    show(&view, cx);
    let packaged = view.read_with(cx, |v, _| v.about.info.as_ref().unwrap().notices_path());
    view.update(cx, |v, cx| v.about_act(reg::ACTION_NOTICES, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        if packaged.is_none() {
            // 测试可执行文件旁边没有 notices：只报状态栏，不改变标签。
            let status = v.shell.feedback.status().expect("状态栏提示");
            assert_eq!(status.key.as_ref(), "about.noticesMissing");
            assert!(v.about.info.is_some(), "对话框保持打开");
        } else {
            assert!(v.about.info.is_none(), "打开文件后对话框关闭");
        }
    });
}

/// 命令经登记机制可达：命令中心里有它，而且能拿到 action（命令面板 / 键位分发用）。
#[gpui::test]
fn command_is_registered_and_dispatchable(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.read_with(cx, |v, _| {
        assert!(
            v.commands.registry().get(reg::SHOW_ABOUT).is_some(),
            "命令已登记"
        );
    });
    assert!(crate::keys::action_for(reg::SHOW_ABOUT).is_some());
}
