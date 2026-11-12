//! #16 工作台的集成测试：在真实 GPUI 窗口（TestAppContext）里走键位、命令、反馈与布局持久化。
use super::*;
use crate::{
    feedback::{FeedbackAction, FeedbackService, Severity},
    shell::{
        catalog,
        feedback::{FeedbackHub, NEVER_SHOW_AGAIN},
        layout::SideBarLocation,
        menus::MenuId,
        view::{run_command, Part},
    },
};
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use std::time::Duration;

fn open_workbench<'a>(
    cx: &'a mut TestAppContext,
    folder: &std::path::Path,
    store: LayoutStore,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let folder = folder.to_path_buf();
    let (view, cx) = cx.add_window_view(move |_, cx| {
        Workbench::with_store(folder.clone(), Instant::now(), store, cx)
    });
    cx.update(|window, cx| {
        window.focus(&view.read(cx).focus);
    });
    cx.run_until_parked();
    (view, cx)
}

fn part(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Part {
    cx.update(|window, cx| view.read(cx).shell.focused_part(window, cx))
}

fn csharp_folder() -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("A.cs"), "public class A {}\n").unwrap();
    folder
}

fn open_first_file(cx: &mut VisualTestContext) {
    cx.simulate_click(point(px(128.0), px(104.0)), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

/// 重新绘制一帧，让 debug_bounds 与焦点反映最新状态。
fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

fn none() -> Arc<[FeedbackAction]> {
    Arc::from([])
}

#[gpui::test]
fn layout_keys_behave_like_vscode(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    // 面板默认隐藏，按需创建。
    view.read_with(cx, |v, _| {
        assert!(v.shell.layout.sidebar_visible);
        assert!(!v.shell.layout.panel_visible);
    });
    redraw(cx);
    assert!(cx.debug_bounds("panel").is_none());

    // ctrl+b：隐藏 / 显示主侧边栏。
    cx.simulate_keystrokes("ctrl-b");
    view.read_with(cx, |v, _| assert!(!v.shell.layout.sidebar_visible));
    cx.simulate_keystrokes("ctrl-b");
    view.read_with(cx, |v, _| assert!(v.shell.layout.sidebar_visible));

    // ctrl+0：聚焦侧边栏（隐藏时先显示）。
    cx.simulate_keystrokes("ctrl-b");
    cx.simulate_keystrokes("ctrl-0");
    view.read_with(cx, |v, _| assert!(v.shell.layout.sidebar_visible));
    assert_eq!(part(&view, cx), Part::SideBar);
    // 在侧边栏有焦点时 ctrl+b 隐藏侧边栏，焦点回到编辑器。
    cx.simulate_keystrokes("ctrl-b");
    assert_eq!(part(&view, cx), Part::Editor);
    cx.simulate_keystrokes("ctrl-b");

    // ctrl+j：打开面板并聚焦；再按一次收起。
    cx.simulate_keystrokes("ctrl-j");
    view.read_with(cx, |v, _| assert!(v.shell.layout.panel_visible));
    assert_eq!(part(&view, cx), Part::Panel);
    redraw(cx);
    assert!(cx.debug_bounds("panel").is_some());
    cx.simulate_keystrokes("ctrl-j");
    view.read_with(cx, |v, _| assert!(!v.shell.layout.panel_visible));
    assert_eq!(part(&view, cx), Part::Editor);

    // ctrl+shift+m：显示问题视图并聚焦；问题视图有焦点时再按收起面板。
    cx.simulate_keystrokes("ctrl-shift-m");
    view.read_with(cx, |v, _| {
        assert!(v.shell.layout.panel_visible);
        assert_eq!(v.shell.active_panel_view(), catalog::PROBLEMS);
    });
    assert_eq!(part(&view, cx), Part::Panel);
    cx.simulate_keystrokes("ctrl-shift-m");
    view.read_with(cx, |v, _| assert!(!v.shell.layout.panel_visible));

    // f6 / shift+f6：编辑器 → 状态栏 → 活动栏 → 侧边栏 → 编辑器（面板隐藏时跳过）。
    let mut seen = vec![part(&view, cx)];
    for _ in 0..4 {
        cx.simulate_keystrokes("f6");
        seen.push(part(&view, cx));
    }
    assert_eq!(
        seen,
        [
            Part::Editor,
            Part::StatusBar,
            Part::ActivityBar,
            Part::SideBar,
            Part::Editor
        ]
    );
    cx.simulate_keystrokes("shift-f6");
    assert_eq!(part(&view, cx), Part::SideBar);
    cx.simulate_keystrokes("shift-f6");
    assert_eq!(part(&view, cx), Part::ActivityBar);
}

#[gpui::test]
fn sidebar_location_right_moves_activity_bar_too(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    let left_activity = cx.debug_bounds("activitybar").unwrap();
    let left_sidebar = cx.debug_bounds("sidebar").unwrap();
    assert!(left_activity.origin.x < left_sidebar.origin.x);
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            run_command(v, "workbench.action.toggleSidebarPosition", window, cx)
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.sidebar_location(), SideBarLocation::Right)
    });
    redraw(cx);
    let activity = cx.debug_bounds("activitybar").unwrap();
    let sidebar = cx.debug_bounds("sidebar").unwrap();
    assert!(sidebar.origin.x > left_sidebar.origin.x);
    assert!(activity.origin.x > sidebar.origin.x, "活动栏在侧边栏外侧");
}

#[gpui::test]
fn zoom_scales_ui_without_rescan_or_reparse(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    open_first_file(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    let (scans, document, syntax) = view.read_with(cx, |v, _| {
        let o = v.editor.active().current().and_then(Tab::opened).unwrap();
        (
            v.scans,
            Arc::as_ptr(&o.document),
            Arc::as_ptr(o.syntax.as_ref().unwrap()) as *const (),
        )
    });
    let line_height = code.read_with(cx, |c, _| c.line_height());
    let status_before = cx.debug_bounds("statusbar").unwrap().size.height;
    cx.simulate_keystrokes("ctrl-=");
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.zoom_level(), 1.0);
        assert!((v.shell.zoom() - 1.2).abs() < 1e-6);
        let o = v.editor.active().current().and_then(Tab::opened).unwrap();
        assert_eq!(v.scans, scans, "缩放不得重新扫描");
        assert_eq!(Arc::as_ptr(&o.document), document, "缩放不得重新打开文档");
        assert_eq!(
            Arc::as_ptr(o.syntax.as_ref().unwrap()) as *const (),
            syntax,
            "缩放不得重新解析"
        );
    });
    // 代码视图（#19）随窗口缩放一起放大：行高按字号四舍五入，留一点余量。
    code.read_with(cx, |c, _| {
        let scaled = c.line_height();
        assert!(
            (scaled - line_height * 1.2).abs() < 1.0,
            "行高未随窗口缩放：{line_height} → {scaled}"
        );
        assert!(c.last_frame().visible_lines > 0, "缩放后仍只重新整形可见行");
    });
    redraw(cx);
    let status_after = cx.debug_bounds("statusbar").unwrap().size.height;
    assert!(status_after > status_before, "界面整体缩放");
    cx.simulate_keystrokes("ctrl--");
    cx.simulate_keystrokes("ctrl--");
    view.read_with(cx, |v, _| assert_eq!(v.shell.zoom_level(), -1.0));
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            run_command(v, "workbench.action.zoomReset", window, cx)
        })
    });
    view.read_with(cx, |v, _| assert_eq!(v.shell.zoom_level(), 0.0));
}

#[gpui::test]
fn layout_is_restored_per_workspace(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let data = tempfile::tempdir().unwrap();
    let store = LayoutStore::with_base(Some(data.path().into()), folder.path());
    {
        let (view, cx) = open_workbench(cx, folder.path(), store.clone());
        cx.simulate_keystrokes("ctrl-j");
        cx.simulate_keystrokes("ctrl-shift-f");
        cx.update(|window, cx| {
            view.update(cx, |v, cx| {
                v.shell.layout.sidebar_width = 333.0;
                v.shell.layout.panel_size = 222.0;
                run_command(v, "workbench.action.toggleSidebarVisibility", window, cx);
                v.shell.record_window(window);
                v.shell.save_now();
            })
        });
    }
    let restored = store.load();
    assert!(!restored.sidebar_visible);
    assert_eq!(restored.sidebar_width, 333.0);
    assert!(restored.panel_visible);
    assert_eq!(restored.panel_size, 222.0);
    assert_eq!(restored.active_container.as_deref(), Some(catalog::SEARCH));
    assert!(restored.window.is_some(), "窗口大小与位置已记录");
    let (view, cx) = open_workbench(cx, folder.path(), store);
    view.read_with(cx, |v, _| {
        assert!(!v.shell.layout.sidebar_visible);
        assert!(v.shell.layout.panel_visible);
        assert_eq!(v.shell.active_container(), catalog::SEARCH);
        assert_eq!(v.shell.layout.sidebar_width, 333.0);
    });
}

#[gpui::test]
fn menus_only_show_implemented_commands(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    view.read_with(cx, |v, _| {
        let commands = v.commands.registry();
        // #16 自己实现的命令，加上其他 issue 在装配期声明的（#24：快速打开、命令面板、转到行）。
        let implemented = |id: &str| {
            catalog::is_implemented(id)
                || crate::quick_access::registry::COMMANDS
                    .iter()
                    .any(|(cmd, _, _)| *cmd == id)
        };
        let mut shown = 0;
        let mut expected = 0;
        for menu in [
            MenuId::File,
            MenuId::Edit,
            MenuId::Selection,
            MenuId::View,
            MenuId::Go,
            MenuId::Help,
            MenuId::Manage,
        ] {
            let items = v.shell.visible_menu_items(commands, menu);
            shown += items.len();
            for id in &items {
                let item = v.shell.menus.get(id).unwrap();
                assert!(commands.get(&item.command).is_some());
                assert!(implemented(&item.command));
            }
        }
        // 菜单项数 == 已登记且已实现的命令被登记成菜单项的数量。
        for (_, item) in v.shell.menus.entries() {
            if commands.get(&item.command).is_some() && implemented(&item.command) {
                expected += 1;
            }
        }
        assert_eq!(shown, expected);
        // 命令面板（showCommands）由 #24 实现并在装配期声明：齿轮菜单里出现这一项。
        assert!(v
            .shell
            .menus
            .entries()
            .any(|(_, i)| i.command.as_ref() == "workbench.action.showCommands"));
        let manage = v.shell.visible_menu_items(commands, MenuId::Manage);
        assert!(manage.iter().any(|id| {
            v.shell.menus.get(id).unwrap().command.as_ref() == "workbench.action.showCommands"
        }));
    });
}

#[gpui::test]
fn status_message_disappears_after_three_seconds(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    view.update(cx, |v, cx| {
        shell::view::report(
            v,
            "copy.done",
            Reason::Receipt,
            Message::key("view.problems"),
            none(),
            cx,
        )
        .unwrap();
    });
    view.read_with(cx, |v, _| assert_eq!(v.status(), "问题"));
    cx.executor().advance_clock(Duration::from_millis(2900));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(v.status(), "问题"));
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(v.shell.feedback.status().is_none()));
}

#[gpui::test]
fn editor_bar_can_be_closed_and_suppressed(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    open_first_file(cx);
    let document = view.read_with(cx, |v, _| v.active_document().unwrap());
    let show = |v: &mut Workbench| {
        v.shell.feedback.report(
            "settings.shared",
            Reason::FileContext {
                document,
                reason: "settings.shared".into(),
            },
            Message::key("view.problems"),
            none(),
        )
    };
    view.update(cx, |v, _| show(v).unwrap());
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.feedback.editor_bars(document).count(), 1)
    });
    // 同一文件、同一原因不重复。
    view.update(cx, |v, _| show(v).unwrap());
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.feedback.editor_bars(document).count(), 1)
    });
    // 关闭。
    view.update(cx, |v, _| {
        v.shell.feedback.dismiss("settings.shared").unwrap()
    });
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.feedback.editor_bars(document).count(), 0)
    });
    // 再次出现后点“不再提示”。
    view.update(cx, |v, _| show(v).unwrap());
    view.update(cx, |v, _| {
        v.shell
            .feedback
            .act("settings.shared", NEVER_SHOW_AGAIN)
            .unwrap()
    });
    view.update(cx, |v, _| show(v).unwrap());
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.feedback.editor_bars(document).count(), 0);
        assert!(v
            .shell
            .feedback
            .suppressed()
            .any(|k| k.as_ref() == "settings.shared"));
    });
}

#[gpui::test]
fn same_cause_notification_counts_instead_of_repeating(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    view.update(cx, |v, cx| {
        for _ in 0..3 {
            shell::view::report(
                v,
                "index.failed",
                Reason::Global(Severity::Error),
                Message::key("view.problems"),
                none(),
                cx,
            )
            .unwrap();
        }
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let all = v.shell.feedback.notifications();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].count, 3);
        assert_eq!(v.shell.feedback.toasts().count(), 1);
    });
    redraw(cx);
    assert!(cx.debug_bounds("statusbar").is_some());
}

#[gpui::test]
fn do_not_disturb_hides_info_but_not_errors(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            run_command(v, "notifications.toggleDoNotDisturbMode", window, cx)
        })
    });
    view.update(cx, |v, cx| {
        for (key, severity) in [
            ("info", Severity::Info),
            ("warn", Severity::Warning),
            ("error", Severity::Error),
        ] {
            shell::view::report(
                v,
                key,
                Reason::Global(severity),
                Message::key("view.problems"),
                none(),
                cx,
            )
            .unwrap();
        }
    });
    view.read_with(cx, |v, _| {
        assert!(v.shell.feedback.do_not_disturb());
        let toasts: Vec<_> = v
            .shell
            .feedback
            .toasts()
            .map(|n| n.key.to_string())
            .collect();
        assert_eq!(toasts, ["error"]);
        // 不弹出，但仍在通知中心里（与 VS Code 相同）。
        assert_eq!(v.shell.feedback.notifications().len(), 3);
    });
}

#[gpui::test]
fn modal_only_for_data_loss(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    view.update(cx, |v, cx| {
        // 普通原因永远不会产生模态框。
        for reason in [
            Reason::Receipt,
            Reason::Background,
            Reason::Global(Severity::Error),
        ] {
            shell::view::report(v, "x", reason, Message::key("view.problems"), none(), cx).unwrap();
        }
        assert!(v.shell.feedback.modal().is_none());
        // 直接要求模态级别但没有声明数据丢失：拒绝。
        assert_eq!(
            v.shell.feedback.show(crate::feedback::Feedback {
                key: "save.conflict".into(),
                level: crate::feedback::FeedbackLevel::Modal,
                message: Message::key("view.problems"),
                actions: none(),
            }),
            Err(Error::InvalidInput)
        );
        assert!(v.shell.feedback.modal().is_none());
        let actions: Arc<[FeedbackAction]> = Arc::from([FeedbackAction {
            id: "save".into(),
            label: Message::key("common.retry"),
        }]);
        shell::view::report(
            v,
            "close.dirty",
            Reason::DataLoss,
            Message::key("view.problems"),
            actions,
            cx,
        )
        .unwrap();
        assert!(v.shell.feedback.modal().is_some());
    });
    redraw(cx);
    // Esc 关闭模态框：发出 Dismissed，不代表接受。
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |v, _| assert!(v.shell.feedback.modal().is_none()));
}

#[gpui::test]
fn cursor_hint_and_background_never_notify(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    open_first_file(cx);
    view.update(cx, |v, cx| {
        let document = v.active_document().unwrap();
        shell::view::report(
            v,
            "goto.none",
            Reason::NoResult {
                document,
                position: Default::default(),
            },
            Message::key("view.problems"),
            none(),
            cx,
        )
        .unwrap();
        shell::view::report(
            v,
            "index.progress",
            Reason::Background,
            Message::key("common.loading"),
            none(),
            cx,
        )
        .unwrap();
        assert!(v.shell.feedback.cursor_hint().is_some());
        assert!(v.shell.feedback.notifications().is_empty());
    });
}

#[gpui::test]
fn status_bar_items_from_registry_render_and_focus(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            run_command(v, "workbench.action.focusStatusBar", window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(part(&view, cx), Part::StatusBar);
    let order = view.read_with(cx, |v, _| {
        v.shell
            .status_bar
            .focus_order(&v.shell.contributions)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });
    assert!(order.iter().any(|i| i == catalog::STATUS_PROBLEMS));
    assert_eq!(order.last().unwrap(), catalog::STATUS_NOTIFICATIONS);
    cx.simulate_keystrokes("right");
    view.read_with(cx, |v, _| assert_eq!(v.shell.status_index(), 1));
    cx.simulate_keystrokes("end");
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.status_index(), order.len() - 1)
    });
    cx.simulate_keystrokes("escape");
    assert_eq!(part(&view, cx), Part::Editor);
    // 问题计数条目用 $(icon) 语法。
    view.read_with(cx, |v, _| {
        let entry = v.shell.status_bar.get(catalog::STATUS_PROBLEMS).unwrap();
        assert!(entry.text.contains("$(error)"));
        assert_eq!(crate::shell::icons::strip_icons(&entry.text), "0   0");
    });
}

#[test]
fn hub_is_usable_without_a_window() {
    let mut hub = FeedbackHub::default();
    hub.report(
        "a",
        Reason::Global(Severity::Info),
        Message::key("x"),
        none(),
    )
    .unwrap();
    assert_eq!(hub.toasts().count(), 1);
}

#[gpui::test]
fn top_right_notifications_have_a_title_bar_entry(cx: &mut TestAppContext) {
    let folder = csharp_folder();
    let (view, cx) = open_workbench(cx, folder.path(), LayoutStore::disabled());
    // 默认右下角：入口是状态栏铃铛。
    view.read_with(cx, |v, _| {
        assert!(!v.shell.notifications_in_title_bar());
        assert!(v
            .shell
            .status_bar
            .get(catalog::STATUS_NOTIFICATIONS)
            .is_some());
    });
    redraw(cx);
    assert!(cx.debug_bounds("titlebar-notifications").is_none());

    // 右上角 + showInTitleBar（默认 true）：入口移到标题栏，状态栏不再显示铃铛。
    view.update(cx, |v, cx| {
        v.shell.override_setting(
            "workbench.notifications.position",
            Value::String("top-right".into()),
        );
        shell::view::report(
            v,
            "index.failed",
            Reason::Global(Severity::Error),
            Message::key("view.problems"),
            none(),
            cx,
        )
        .unwrap();
    });
    redraw(cx);
    view.read_with(cx, |v, _| {
        assert!(v.shell.notifications_in_title_bar());
        assert!(v
            .shell
            .status_bar
            .get(catalog::STATUS_NOTIFICATIONS)
            .is_none());
    });
    let bell = cx
        .debug_bounds("titlebar-notifications")
        .expect("标题栏有通知入口");
    let title = cx.debug_bounds("titlebar").unwrap();
    assert!(bell.origin.y >= title.origin.y && bell.bottom() <= title.bottom());
    // 点击铃铛打开通知中心，再点关闭。
    cx.simulate_click(bell.center(), Modifiers::default());
    view.read_with(cx, |v, _| assert!(v.shell.feedback.center_visible()));
    redraw(cx);
    let bell = cx.debug_bounds("titlebar-notifications").unwrap();
    cx.simulate_click(bell.center(), Modifiers::default());
    view.read_with(cx, |v, _| assert!(!v.shell.feedback.center_visible()));

    // showInTitleBar 关掉后回到状态栏铃铛。
    view.update(cx, |v, _| {
        v.shell
            .override_setting("workbench.notifications.showInTitleBar", Value::Bool(false))
    });
    redraw(cx);
    view.read_with(cx, |v, _| {
        assert!(!v.shell.notifications_in_title_bar());
        assert!(v
            .shell
            .status_bar
            .get(catalog::STATUS_NOTIFICATIONS)
            .is_some());
    });
}
