//! #50：窗口级入口测试——真实 GPUI 窗口里“编辑 → 持续备份 → 意外退出 → 重启恢复”。
//!
//! 覆盖 issue 验收标准里属于界面层的部分：
//!
//! - `files.hotExit` 三种取值的关闭确认（`off` 询问、`onExit` 退出应用不询问但关标签询问、
//!   `onExitAndWindowClose` 都不询问）；
//! - 意外退出后重启：内容回来、标签顺序与活动标签保持、状态栏出现“已恢复上次未保存的修改”；
//! - 正常退出后重启：内容同样回来，但**不出现**那条提示；
//! - 备份在保存与关闭之后消失；
//! - 磁盘文件在备份期间被删除：恢复成“已删除”状态的标签。
//!
//! 核心层的强杀进程、编码往返、撤销栈、隔离与崩溃日志在 `rove-buffer` 的 `tests/hotexit.rs`。
//!
//! 运行：`cargo test -p rove-ui hotexit_tests`
use super::*;
use crate::documents::OpenOptions;
use crate::save::registry as ids;
use gpui::{TestAppContext, VisualTestContext};
use rove_buffer::hotexit::{BackupStore, HotExitMode};
use rove_config::{SettingsOptions, SettingsService};

/// 一个“Rove 进程”的替身：同一个用户目录、同一个工作区、同一个状态根目录可以重复开窗，
/// 第二次开窗就是“重启”。
struct Fixture {
    _user: tempfile::TempDir,
    folder: tempfile::TempDir,
    state: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Fixture {
    fn new(hot_exit: HotExitMode) -> Self {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(
            user.path().join("settings.json"),
            format!("{{ \"files.hotExit\": \"{}\" }}", hot_exit.as_setting()),
        )
        .unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        Self {
            _user: user,
            folder: tempfile::tempdir().unwrap(),
            state: tempfile::tempdir().unwrap(),
            service,
        }
    }

    fn file(&self, path: &str, text: &str) -> PathBuf {
        let full = self.folder.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, text).unwrap();
        full
    }

    /// 开一个窗口（= 启动一次 Rove）。装配顺序与 `Workbench::with_settings` 的应用入口一致：
    /// 窗口建好后立刻接上热退出（恢复在这里发生）。
    fn launch<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let root: PathBuf = self.folder.path().into();
        let settings: Arc<dyn Settings> = self.service.clone();
        let state: PathBuf = self.state.path().into();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut view = Workbench::with_settings(
                root,
                Instant::now(),
                LayoutStore::disabled(),
                settings,
                cx,
            )
            .unwrap();
            view.start_hot_exit_in(&state, cx);
            view
        });
        cx.run_until_parked();
        (view, cx)
    }

    fn store(&self) -> BackupStore {
        BackupStore::new(self.state.path(), self.folder.path())
    }
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..6 {
        cx.update(|window, cx| {
            window.refresh();
            cx.refresh_windows();
        });
        cx.run_until_parked();
    }
}

fn open_file(view: &Entity<Workbench>, cx: &mut VisualTestContext, path: &str) -> Entity<CodeView> {
    view.update(cx, |v, _| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                OpenOptions {
                    preview_tab: false,
                    ..OpenOptions::default()
                },
                Cancellation::default(),
            )
            .unwrap();
    });
    settle(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    settle(cx);
    code
}

fn shown_text(code: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    code.read_with(cx, |c, _| {
        let t = c.text().unwrap();
        let mut out = String::new();
        t.visit_chunks(
            rove_base::Span::new(
                rove_base::ByteOffset(0),
                rove_base::ByteOffset(t.byte_len()),
            )
            .unwrap(),
            &mut |s| out.push_str(s),
        )
        .unwrap();
        out
    })
}

/// 窗口里每个标签的（文件名, 是否活动, 是否脏），按标签顺序。
fn tabs(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(String, bool, bool)> {
    view.read_with(cx, |v, _| {
        v.editor
            .groups
            .values()
            .flat_map(|pane| {
                pane.tabs.iter().map(move |tab| {
                    let name = match &tab.resource {
                        Resource::Workspace(p) => p.as_str().to_string(),
                        Resource::File(p) => p.file_name().unwrap().to_string_lossy().into_owned(),
                        Resource::Virtual(v) => v.to_string(),
                    };
                    (name, pane.active == Some(tab.id), tab.dirty)
                })
            })
            .collect()
    })
}

/// 把窗口里到期的备份立刻写出来（测试不等 2 秒的去抖）。
fn back_up_now(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| v.flush_due_backups(cx));
    settle(cx);
}

/// 模拟意外退出：直接丢掉窗口，不写任何退出标记（与被 `Stop-Process -Force` 杀掉等价）。
fn crash(cx: &mut VisualTestContext) {
    settle(cx);
}

/// 模拟正常退出：走窗口关闭按钮的完整流程（#49 的确认 + #50 的备份与退出标记）。
fn quit(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    let closed = view.update(cx, |v, cx| v.request_window_close(cx));
    settle(cx);
    closed
}

// ------------------------------------------------------------------------- 崩溃恢复

#[gpui::test]
fn a_crash_restores_every_dirty_document_with_its_tab_order_and_a_status_message(
    cx: &mut TestAppContext,
) {
    let fx = Fixture::new(HotExitMode::OnExit);
    fx.file("a.txt", "aaa\n");
    fx.file("b.txt", "bbb\n");
    fx.file("c.txt", "ccc\n");

    {
        let (view, cx) = fx.launch(cx);
        for name in ["a.txt", "b.txt", "c.txt"] {
            let code = open_file(&view, cx, name);
            cx.simulate_input("X");
            settle(cx);
            assert!(shown_text(&code, cx).starts_with('X'), "{name} 已编辑");
        }
        // 活动标签回到 b.txt（恢复时要保持它）。
        open_file(&view, cx, "b.txt");
        back_up_now(&view, cx);
        assert_eq!(
            tabs(&view, cx),
            vec![
                ("a.txt".into(), false, true),
                ("b.txt".into(), true, true),
                ("c.txt".into(), false, true),
            ]
        );
        let (writes, bytes) = view.read_with(cx, |v, _| v.hot_exit_io_stats());
        assert_eq!(writes, 3, "三个脏文档各一份备份");
        assert!(bytes > 0);
        // 意外退出：没有退出标记。
        crash(cx);
    }
    assert!(!fx.store().session().clean_exit);
    assert_eq!(fx.store().backups().len(), 3);
    // 磁盘上的文件一个字节都没动（备份不是保存）。
    assert_eq!(
        std::fs::read_to_string(fx.folder.path().join("a.txt")).unwrap(),
        "aaa\n"
    );

    // 重启：内容全部回来，标签顺序与活动标签保持，状态栏提示一次。
    let (view, cx) = fx.launch(cx);
    assert_eq!(
        tabs(&view, cx),
        vec![
            ("a.txt".into(), false, true),
            ("b.txt".into(), true, true),
            ("c.txt".into(), false, true),
        ],
        "标签顺序与活动标签保持，三个都带脏点"
    );
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert_eq!(shown_text(&code, cx), "Xbbb\n", "活动标签显示恢复后的内容");
    assert_eq!(
        view.read_with(cx, |v, _| v.status()),
        "已恢复上次未保存的修改",
        "UX_SPEC §6.4 的状态栏提示"
    );
    // 第 1 级反馈：不弹模态框、不弹通知。
    view.read_with(cx, |v, _| {
        assert!(v.shell.feedback.modal().is_none());
        assert!(v.shell.feedback.notifications().is_empty());
    });
    // 消费完的备份被清掉。
    assert!(fx.store().backups().is_empty());
}

#[gpui::test]
fn a_clean_exit_restores_silently(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExit);
    fx.file("a.txt", "aaa\n");
    {
        let (view, cx) = fx.launch(cx);
        let _code = open_file(&view, cx, "a.txt");
        cx.simulate_input("X");
        settle(cx);
        // 正常退出：热退出接管关闭确认，备份与退出标记一起落盘。
        assert!(quit(&view, cx), "onExit 下退出应用不询问");
    }
    assert!(fx.store().session().clean_exit, "写下了干净的退出标记");
    assert_eq!(fx.store().backups().len(), 1);

    let (view, cx) = fx.launch(cx);
    assert_eq!(tabs(&view, cx), vec![("a.txt".into(), true, true)]);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert_eq!(shown_text(&code, cx), "Xaaa\n", "内容同样回来");
    assert_ne!(
        view.read_with(cx, |v, _| v.status()),
        "已恢复上次未保存的修改",
        "正常退出后不出现那条提示"
    );
}

// ------------------------------------------------------------------------- 三种取值

#[gpui::test]
fn hot_exit_off_asks_before_closing_the_window_and_keeps_no_backup(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::Off);
    fx.file("a.txt", "aaa\n");
    let (view, cx) = fx.launch(cx);
    let _code = open_file(&view, cx, "a.txt");
    cx.simulate_input("X");
    settle(cx);
    back_up_now(&view, cx);
    assert!(fx.store().backups().is_empty(), "off 时一份备份都不写");

    assert!(!quit(&view, cx), "off 时关窗口仍然询问");
    view.read_with(cx, |v, _| {
        assert!(v.shell.feedback.modal().is_some(), "弹的是 #49 的模态框");
    });
}

#[gpui::test]
fn hot_exit_on_exit_skips_the_window_prompt_but_not_the_tab_prompt(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExit);
    fx.file("a.txt", "aaa\n");
    let (view, cx) = fx.launch(cx);
    let _code = open_file(&view, cx, "a.txt");
    cx.simulate_input("X");
    settle(cx);

    // 关闭单个标签：仍然询问（VS Code 同样只在窗口 / 应用级别跳过）。
    view.update(cx, |v, cx| {
        v.command("workbench.action.closeActiveEditor", cx)
    });
    settle(cx);
    view.read_with(cx, |v, _| {
        assert!(v.shell.feedback.modal().is_some(), "关标签要询问");
    });
    let doc = view.read_with(cx, |v, _| {
        v.editor
            .active()
            .current()
            .unwrap()
            .opened()
            .unwrap()
            .document
            .id
    });
    let key = format!("{}{}", ids::KEY_CLOSE, doc.0);
    view.update(cx, |v, cx| {
        v.save_feedback_action(&key, ids::ACTION_CANCEL, cx)
    });
    settle(cx);
    // 取消之后把模态框收掉，这样下面能直接看出关窗口有没有再弹一个。
    view.update(cx, |v, _| {
        let _ = crate::feedback::FeedbackService::dismiss(&mut v.shell.feedback, &key);
    });
    settle(cx);

    // 关闭窗口（= 退出应用）：不询问。
    assert!(quit(&view, cx));
    view.read_with(cx, |v, _| assert!(v.shell.feedback.modal().is_none()));
    assert_eq!(fx.store().backups().len(), 1);
}

#[gpui::test]
fn hot_exit_on_exit_and_window_close_never_asks(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExitAndWindowClose);
    fx.file("a.txt", "aaa\n");
    let (view, cx) = fx.launch(cx);
    let _code = open_file(&view, cx, "a.txt");
    cx.simulate_input("X");
    settle(cx);
    assert!(quit(&view, cx), "关任意窗口都不询问");
    view.read_with(cx, |v, _| assert!(v.shell.feedback.modal().is_none()));
    assert!(fx.store().session().clean_exit);
}

// ------------------------------------------------------------------------- 清理

#[gpui::test]
fn saving_or_closing_a_document_removes_its_backup(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExit);
    let disk = fx.file("a.txt", "aaa\n");
    let (view, cx) = fx.launch(cx);
    let _code = open_file(&view, cx, "a.txt");
    cx.simulate_input("X");
    settle(cx);
    back_up_now(&view, cx);
    assert!(fx.store().backup_path(&disk).is_file());

    // 保存后：备份消失。
    let action = ids::action_for(ids::SAVE).unwrap();
    cx.update(|window, cx| window.dispatch_action(action, cx));
    settle(cx);
    assert_eq!(std::fs::read_to_string(&disk).unwrap(), "Xaaa\n");
    assert!(!fx.store().backup_path(&disk).exists(), "保存后备份消失");

    // 再改一次并备份，然后“不保存”地关掉标签：备份同样消失。
    cx.simulate_input("Y");
    settle(cx);
    back_up_now(&view, cx);
    assert!(fx.store().backup_path(&disk).is_file());
    let doc = view.read_with(cx, |v, _| {
        v.editor
            .active()
            .current()
            .unwrap()
            .opened()
            .unwrap()
            .document
            .id
    });
    view.update(cx, |v, cx| {
        v.command("workbench.action.closeActiveEditor", cx)
    });
    settle(cx);
    let key = format!("{}{}", ids::KEY_CLOSE, doc.0);
    view.update(cx, |v, cx| {
        v.save_feedback_action(&key, ids::ACTION_DONT_SAVE, cx)
    });
    settle(cx);
    assert!(!fx.store().backup_path(&disk).exists(), "关闭后备份消失");
}

// ------------------------------------------------------------------------- 已删除

#[gpui::test]
fn a_file_deleted_while_backed_up_restores_as_a_deleted_tab(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExit);
    let disk = fx.file("gone.txt", "aaa\n");
    {
        let (view, cx) = fx.launch(cx);
        let _code = open_file(&view, cx, "gone.txt");
        cx.simulate_input("X");
        settle(cx);
        back_up_now(&view, cx);
        crash(cx);
    }
    // 备份之后、重启之前，别的程序把文件删了。
    std::fs::remove_file(&disk).unwrap();

    let (view, cx) = fx.launch(cx);
    assert_eq!(tabs(&view, cx).len(), 1);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert_eq!(shown_text(&code, cx), "Xaaa\n", "内容保留");
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert!(v.tab_deleted(tab), "标签显示“已删除”");
        assert!(tab.dirty);
    });
    // 保存视为新建写入（`ctrl+s` 经代码视图分发，所以先把焦点放回去）。
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    settle(cx);
    let action = ids::action_for(ids::SAVE).unwrap();
    cx.update(|window, cx| window.dispatch_action(action, cx));
    settle(cx);
    assert_eq!(std::fs::read_to_string(&disk).unwrap(), "Xaaa\n");
}
