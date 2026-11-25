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
        Self::with_restore_undo_stack(hot_exit, true)
    }

    fn with_restore_undo_stack(hot_exit: HotExitMode, restore_undo_stack: bool) -> Self {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(
            user.path().join("settings.json"),
            format!(
                "{{ \"files.hotExit\": \"{}\", \"files.restoreUndoStack\": {restore_undo_stack} }}",
                hot_exit.as_setting()
            ),
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

fn click_caret(code: &Entity<CodeView>, cx: &mut VisualTestContext, target: Position) {
    let at = cx.update(|_, app| {
        code.update(app, |code, _| {
            code.point_for_position(target)
                .expect("visible caret target")
        })
    });
    cx.simulate_click(
        point(at.x + px(1.0), at.y + px(4.0)),
        gpui::Modifiers::default(),
    );
    settle(cx);
    assert_eq!(code.read_with(cx, |c, _| c.selection().active), target);
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
    let active_buffer = code.read_with(cx, |code, _| {
        code.edit_host()
            .and_then(|host| host.session())
            .map(|session| session.contents())
    });
    assert_eq!(active_buffer.as_deref(), Some("Xbbb\n"));
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
    // 恢复结束后不做编辑、不等后台写入，立刻再次异常退出。
    assert_eq!(fx.store().session().attempts, 0);
    assert_eq!(fx.store().backups().len(), 3);
    assert_eq!(view.read_with(cx, |v, _| v.hot_exit_io_stats()), (0, 0));
    crash(cx);

    let (view, cx) = fx.launch(cx);
    assert_eq!(tabs(&view, cx).len(), 3);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert_eq!(shown_text(&code, cx), "Xbbb\n");
}

#[gpui::test]
fn a_crash_restores_the_last_caret_move_without_an_undo_stack(cx: &mut TestAppContext) {
    let fx = Fixture::with_restore_undo_stack(HotExitMode::OnExit, false);
    let disk = fx.file("a.txt", "disk");
    let moved = Position {
        line: rove_base::Line(1),
        column: rove_base::Utf8Column(3),
    };
    {
        let (view, cx) = fx.launch(cx);
        let code = open_file(&view, cx, "a.txt");
        cx.simulate_input("X");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("汉YZ");
        settle(cx);
        assert_eq!(shown_text(&code, cx), "X\n汉YZdisk");
        back_up_now(&view, cx);
        let initial = fx.store().backups();
        assert_eq!(initial.len(), 1);
        assert_eq!(
            initial[0].cursor,
            Some(code.read_with(cx, |c, _| c.selection().active))
        );
        assert_ne!(initial[0].cursor, Some(moved));
        assert!(initial[0].undo.is_empty());

        cx.simulate_keystrokes("left left");
        settle(cx);
        assert_eq!(code.read_with(cx, |c, _| c.selection().active), moved);
        back_up_now(&view, cx);
        let backups = fx.store().backups();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].text, initial[0].text);
        assert_eq!(backups[0].cursor, Some(moved));
        crash(cx);
    }

    let (view, cx) = fx.launch(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert_eq!(shown_text(&code, cx), "X\n汉YZdisk");
    assert_eq!(code.read_with(cx, |c, _| c.selection().active), moved);
    assert_eq!(fx.store().backups().len(), 1);
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_input("!");
    settle(cx);
    assert_eq!(shown_text(&code, cx), "X\n汉!YZdisk");
    assert_eq!(std::fs::read(&disk).unwrap(), b"disk");
}

#[gpui::test]
fn a_split_document_restores_the_focused_or_most_recently_activated_caret(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExit);
    let disk = fx.file("a.txt", "abcdefgh");
    fx.file("other.txt", "clean");
    fx.file("left-other.txt", "also clean");
    {
        let (view, cx) = fx.launch(cx);
        let left_code = open_file(&view, cx, "a.txt");
        cx.simulate_input("X");
        settle(cx);
        let (left, document) = view.read_with(cx, |v, _| {
            (
                v.editor.focused,
                v.editor
                    .active()
                    .current()
                    .unwrap()
                    .opened()
                    .unwrap()
                    .document
                    .id,
            )
        });
        cx.simulate_keystrokes("ctrl-\\");
        settle(cx);
        let right_code = view.read_with(cx, |v, _| {
            assert_ne!(v.editor.focused, left);
            assert_eq!(
                v.editor
                    .active()
                    .current()
                    .unwrap()
                    .opened()
                    .unwrap()
                    .document
                    .id,
                document
            );
            v.code_view().clone()
        });
        cx.update(|window, cx| window.focus(&right_code.focus_handle(cx)));
        cx.simulate_keystrokes("right right");
        cx.simulate_input("!");
        cx.simulate_keystrokes("right");
        settle(cx);
        assert_eq!(
            left_code.read_with(cx, |c, _| c.selection().active.column.0),
            1
        );
        assert_eq!(
            right_code.read_with(cx, |c, _| c.selection().active.column.0),
            5
        );
        assert_eq!(shown_text(&right_code, cx), "Xab!cdefgh");
        back_up_now(&view, cx);
        let active = fx.store().backups();
        assert_eq!(active.len(), 1);
        assert!(active[0].active);
        assert_eq!(active[0].cursor.unwrap().column.0, 5);

        cx.simulate_keystrokes("left");
        settle(cx);
        open_file(&view, cx, "other.txt");
        back_up_now(&view, cx);
        let inactive = fx.store().backups();
        assert_eq!(inactive.len(), 1);
        assert!(!inactive[0].active);
        assert_eq!(inactive[0].cursor.unwrap().column.0, 4);

        // Clicking the left editor focuses its group but does not reactivate its tab.
        // Its caret must win after opening another tab despite the right tab having higher MRU.
        click_caret(
            &left_code,
            cx,
            Position {
                line: Line(0),
                column: rove_base::Utf8Column(2),
            },
        );
        view.read_with(cx, |v, _| assert_eq!(v.editor.focused, left));
        assert_eq!(
            left_code.read_with(cx, |c, _| c.selection().active.column.0),
            2
        );
        open_file(&view, cx, "left-other.txt");
        back_up_now(&view, cx);
        let left_recent = fx.store().backups();
        assert_eq!(left_recent.len(), 1);
        assert!(!left_recent[0].active);
        assert_eq!(left_recent[0].cursor.unwrap().column.0, 2);
        crash(cx);
    }

    let (view, cx) = fx.launch(cx);
    assert_eq!(tabs(&view, cx), vec![("a.txt".into(), true, true)]);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert_eq!(shown_text(&code, cx), "Xab!cdefgh");
    assert_eq!(code.read_with(cx, |c, _| c.selection().active.column.0), 2);
    back_up_now(&view, cx);
    assert_eq!(view.read_with(cx, |v, _| v.hot_exit_io_stats()), (0, 0));
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_input("?");
    settle(cx);
    assert_eq!(shown_text(&code, cx), "Xa?b!cdefgh");
    assert_eq!(std::fs::read(&disk).unwrap(), b"abcdefgh");
}

#[gpui::test]
fn the_in_group_second_view_caret_restores_in_the_primary_view(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExit);
    let disk = fx.file("a.txt", "abcdefgh");
    {
        let (view, cx) = fx.launch(cx);
        let primary = open_file(&view, cx, "a.txt");
        cx.simulate_input("X");
        settle(cx);
        view.update(cx, |view, cx| {
            view.command("workbench.action.splitEditorInGroup", cx)
        });
        settle(cx);
        let secondary = view.read_with(cx, |v, _| {
            v.group_code(v.editor.focused, true).unwrap().clone()
        });
        click_caret(
            &secondary,
            cx,
            Position {
                line: Line(0),
                column: rove_base::Utf8Column(3),
            },
        );
        cx.simulate_input("!");
        cx.simulate_keystrokes("right");
        settle(cx);
        assert_eq!(
            primary.read_with(cx, |c, _| c.selection().active.column.0),
            1
        );
        assert_eq!(
            secondary.read_with(cx, |c, _| c.selection().active.column.0),
            5
        );
        assert_eq!(shown_text(&secondary, cx), "Xab!cdefgh");
        back_up_now(&view, cx);
        let backups = fx.store().backups();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].active);
        assert_eq!(backups[0].cursor.unwrap().column.0, 5);
        crash(cx);
    }

    let (view, cx) = fx.launch(cx);
    let code = view.read_with(cx, |v, _| {
        assert!(v.group_code(v.editor.focused, true).is_none());
        v.code_view().clone()
    });
    assert_eq!(code.read_with(cx, |c, _| c.selection().active.column.0), 5);
    assert_eq!(shown_text(&code, cx), "Xab!cdefgh");
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_input("?");
    settle(cx);
    assert_eq!(shown_text(&code, cx), "Xab!c?defgh");
    assert_eq!(std::fs::read(&disk).unwrap(), b"abcdefgh");
}

#[gpui::test]
fn a_crash_restores_committed_quick_navigation_in_a_dirty_file(cx: &mut TestAppContext) {
    for (key, query) in [("ctrl-g", "3:2"), ("ctrl-p", "a.txt:3:2")] {
        let fx = Fixture::new(HotExitMode::OnExit);
        let disk = fx.file("a.txt", "aaa\nbbb\nccc\nddd");
        let target = Position {
            line: Line(2),
            column: rove_base::Utf8Column(1),
        };
        {
            let (view, cx) = fx.launch(cx);
            let code = open_file(&view, cx, "a.txt");
            cx.simulate_input("X");
            settle(cx);
            back_up_now(&view, cx);
            let original = fx.store().backups();
            assert_eq!(original.len(), 1);
            assert_ne!(original[0].cursor, Some(target));

            cx.simulate_keystrokes(key);
            settle(cx);
            cx.simulate_input(query);
            settle(cx);
            cx.simulate_keystrokes("enter");
            settle(cx);
            assert_eq!(
                code.read_with(cx, |c, _| c.selection().active),
                target,
                "{key}"
            );
            back_up_now(&view, cx);
            let updated = fx.store().backups();
            assert_eq!(updated.len(), 1);
            assert_eq!(updated[0].text, original[0].text);
            assert_eq!(updated[0].cursor, Some(target), "{key}");
            crash(cx);
        }

        let (view, cx) = fx.launch(cx);
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        assert_eq!(
            code.read_with(cx, |c, _| c.selection().active),
            target,
            "{key}"
        );
        back_up_now(&view, cx);
        assert_eq!(view.read_with(cx, |v, _| v.hot_exit_io_stats()), (0, 0));
        cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
        cx.simulate_input("!");
        settle(cx);
        assert_eq!(shown_text(&code, cx), "Xaaa\nbbb\nc!cc\nddd");
        assert_eq!(std::fs::read(&disk).unwrap(), b"aaa\nbbb\nccc\nddd");
    }
}

#[gpui::test]
fn committed_definition_and_history_locations_update_dirty_backups(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExit);
    fx.file("a.txt", "aaa\nbbb\nccc\nddd");
    let (view, cx) = fx.launch(cx);
    let code = open_file(&view, cx, "a.txt");
    cx.simulate_input("X");
    settle(cx);
    back_up_now(&view, cx);

    let first = Position {
        line: Line(2),
        column: rove_base::Utf8Column(1),
    };
    let second = Position {
        line: Line(3),
        column: rove_base::Utf8Column(1),
    };
    for position in [first, second] {
        view.update(cx, |workbench, ctx| {
            workbench
                .open_documents()
                .open(
                    Resource::Workspace(WorkspacePath::new("a.txt").unwrap()),
                    OpenOptions {
                        source: OpenSource::CodeNavigation,
                        location: Some(position),
                        ..OpenOptions::default()
                    },
                    Cancellation::default(),
                )
                .unwrap();
            ctx.notify();
        });
        settle(cx);
        assert_eq!(code.read_with(cx, |c, _| c.selection().active), position);
        back_up_now(&view, cx);
        assert_eq!(fx.store().backups()[0].cursor, Some(position));
    }

    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_keystrokes("alt-left");
    settle(cx);
    assert_eq!(code.read_with(cx, |c, _| c.selection().active), first);
    back_up_now(&view, cx);
    let backups = fx.store().backups();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].cursor, Some(first));
    assert_eq!(backups[0].text, "Xaaa\nbbb\nccc\nddd");
}

#[gpui::test]
fn a_split_document_edit_updates_the_backup_source_without_moving_the_caret(
    cx: &mut TestAppContext,
) {
    let fx = Fixture::new(HotExitMode::OnExit);
    fx.file("a.txt", "abcdefgh");
    fx.file("other.txt", "clean");
    let (view, cx) = fx.launch(cx);
    let left = open_file(&view, cx, "a.txt");
    cx.simulate_input("X");
    settle(cx);
    cx.simulate_keystrokes("ctrl-\\");
    settle(cx);
    let right = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&right.focus_handle(cx)));
    cx.simulate_keystrokes("right right");
    cx.simulate_input("!");
    settle(cx);
    back_up_now(&view, cx);
    let previous = fx.store().backups();
    assert_eq!(previous[0].cursor.unwrap().column.0, 4);

    let left_caret = Position {
        line: Line(0),
        column: rove_base::Utf8Column(1),
    };
    assert_eq!(left.read_with(cx, |c, _| c.selection().active), left_caret);
    click_caret(&left, cx, left_caret);
    cx.simulate_keystrokes("delete");
    settle(cx);
    assert_eq!(left.read_with(cx, |c, _| c.selection().active), left_caret);
    assert_eq!(shown_text(&left, cx), "Xb!cdefgh");
    open_file(&view, cx, "other.txt");
    back_up_now(&view, cx);
    let updated = fx.store().backups();
    assert_eq!(updated.len(), 1);
    assert!(!updated[0].active);
    assert_eq!(updated[0].cursor, Some(left_caret));
    assert_eq!(updated[0].text, "Xb!cdefgh");
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
        cx.simulate_keystrokes("right");
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
    assert_eq!(code.read_with(cx, |c, _| c.selection().active.column.0), 2);
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

#[gpui::test]
fn window_close_asks_about_dirty_untitled_document(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExit);
    let (view, cx) = fx.launch(cx);
    let action = ids::action_for(ids::NEW_UNTITLED).unwrap();
    cx.update(|window, cx| {
        window.focus(&view.read(cx).focus);
        window.dispatch_action(action, cx);
    });
    settle(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_input("未保存");
    settle(cx);

    assert!(!quit(&view, cx));
    assert!(view.read_with(cx, |v, _| v.shell.feedback.modal().is_some()));
    assert_eq!(shown_text(&code, cx), "未保存");
    assert!(tabs(&view, cx).iter().any(|(_, _, dirty)| *dirty));
    assert!(!fx.store().session().clean_exit);
}

#[gpui::test]
fn backup_write_failure_keeps_the_window_open_until_retried(cx: &mut TestAppContext) {
    let fx = Fixture::new(HotExitMode::OnExit);
    let first = fx.file("a.txt", "aaa\n");
    let second = fx.file("b.txt", "bbb\n");
    let (view, cx) = fx.launch(cx);
    let _ = open_file(&view, cx, "a.txt");
    cx.simulate_input("X");
    settle(cx);
    let code = open_file(&view, cx, "b.txt");
    cx.simulate_input("Y");
    settle(cx);

    let store = fx.store();
    let blocked = store.backup_path(&second);
    std::fs::create_dir_all(&blocked).unwrap();
    assert!(!quit(&view, cx));
    assert!(store.backup_path(&first).is_file());
    assert!(!store.session().clean_exit);
    assert_eq!(shown_text(&code, cx), "Ybbb\n");
    let doc = view.read_with(cx, |v, _| {
        assert!(v.shell.feedback.modal().is_some());
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
        v.save_feedback_action(&key, ids::ACTION_CANCEL, cx);
        let _ = crate::feedback::FeedbackService::dismiss(&mut v.shell.feedback, &key);
    });
    std::fs::remove_dir(&blocked).unwrap();
    assert!(quit(&view, cx));
    assert!(store.session().clean_exit);
    assert_eq!(store.backups().len(), 2);
}

// ------------------------------------------------------------------------- 清理

#[gpui::test]
fn old_save_keeps_the_written_backup_of_newer_edits(cx: &mut TestAppContext) {
    old_save_keeps_newer_backup(cx, true);
}

#[gpui::test]
fn old_save_keeps_the_queued_backup_of_newer_edits(cx: &mut TestAppContext) {
    old_save_keeps_newer_backup(cx, false);
}

fn old_save_keeps_newer_backup(cx: &mut TestAppContext, already_written: bool) {
    let fx = Fixture::new(HotExitMode::OnExit);
    let disk = fx.file("a.txt", "aaa\n");
    let store = fx.store();
    let newer;
    {
        let (view, cx) = fx.launch(cx);
        let code = open_file(&view, cx, "a.txt");
        cx.simulate_input("X");
        settle(cx);
        let saved = shown_text(&code, cx);
        let (document, job) = view.update(cx, |v, _| {
            let document = v
                .editor
                .active()
                .current()
                .unwrap()
                .opened()
                .unwrap()
                .document
                .id;
            let job = v
                .editing
                .saves
                .borrow_mut()
                .begin_save(
                    document,
                    &rove_buffer::save::SaveRequest::default(),
                    &rove_buffer::save::SaveOptions::default(),
                    &mut v.shell.feedback,
                    Instant::now(),
                )
                .unwrap();
            (document, job)
        });

        cx.simulate_input("Y");
        settle(cx);
        newer = shown_text(&code, cx);
        assert_ne!(saved, newer);
        if already_written {
            back_up_now(&view, cx);
            assert_eq!(store.backups()[0].text, newer);
        } else {
            assert!(store.backups().is_empty());
        }

        let result = job.run();
        view.update(cx, |v, cx| v.finish_save(document, result, cx));
        settle(cx);
        assert_eq!(std::fs::read_to_string(&disk).unwrap(), saved);
        assert!(tabs(&view, cx)[0].2);
        if !already_written {
            back_up_now(&view, cx);
        }
        let backups = store.backups();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].text, newer);
        crash(cx);
    }

    let (view, cx) = fx.launch(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert_eq!(shown_text(&code, cx), newer);
    assert!(tabs(&view, cx)[0].2);
    // 恢复时沿用备份里的磁盘标记。用户确认覆盖后，验证成功保存会清理备份。
    let (document, job) = view.update(cx, |v, _| {
        let document = v
            .editor
            .active()
            .current()
            .unwrap()
            .opened()
            .unwrap()
            .document
            .id;
        let job = v
            .editing
            .saves
            .borrow_mut()
            .begin_save(
                document,
                &rove_buffer::save::SaveRequest {
                    overwrite_conflict: true,
                    ..Default::default()
                },
                &rove_buffer::save::SaveOptions::default(),
                &mut v.shell.feedback,
                Instant::now(),
            )
            .unwrap();
        (document, job)
    });
    let result = job.run();
    view.update(cx, |v, cx| v.finish_save(document, result, cx));
    assert_eq!(std::fs::read_to_string(&disk).unwrap(), newer);
    assert!(store.backups().is_empty());
}

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
    assert_eq!(code.read_with(cx, |c, _| c.selection().active.column.0), 1);
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
    assert!(
        !fx.store().backup_path(&disk).exists(),
        "恢复后保存会清理原备份"
    );
}
