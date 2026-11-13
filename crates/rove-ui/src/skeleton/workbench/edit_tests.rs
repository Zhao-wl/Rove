//! #47：窗口级入口测试——真实 GPUI 窗口里“打开 → 编辑 → 保存 → 外部修改 → 重载”，以及新建未命名
//! 文档、关闭确认、已删除徽标、#38 `BufferStates` 的编辑 / 保存登记。
//!
//! 运行：`cargo test -p rove-ui edit_tests`
use super::*;
use crate::documents::OpenOptions;
use crate::save::registry as ids;
use gpui::{TestAppContext, VisualTestContext};
use rove_config::{SettingsOptions, SettingsService};

struct Fixture {
    _user: tempfile::TempDir,
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Fixture {
    fn new() -> Self {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("settings.json"), "{}").unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        Self {
            _user: user,
            folder: tempfile::tempdir().unwrap(),
            service,
        }
    }

    fn file(&self, path: &str, text: &str) -> PathBuf {
        let full = self.folder.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, text).unwrap();
        full
    }

    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let root: PathBuf = self.folder.path().into();
        let settings: Arc<dyn Settings> = self.service.clone();
        let (view, cx) = cx.add_window_view(|_, cx| {
            Workbench::with_settings(root, Instant::now(), LayoutStore::disabled(), settings, cx)
                .unwrap()
        });
        cx.run_until_parked();
        (view, cx)
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
                OpenOptions::default(),
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

fn save_action(cx: &mut VisualTestContext, id: &str) {
    let action = ids::action_for(id).unwrap();
    cx.update(|window, cx| window.dispatch_action(action, cx));
    settle(cx);
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

fn active_tab<R>(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&Workbench, &Tab) -> R,
) -> R {
    view.read_with(cx, |v, _| f(v, v.editor.active().current().unwrap()))
}

fn active_doc(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> DocumentId {
    active_tab(view, cx, |_, t| t.opened().unwrap().document.id)
}

/// 外部修改：文件监听的事件（直接交给窗口，避免依赖系统监听的时序），等过合并的安静期。
fn external_change(view: &Entity<Workbench>, cx: &mut VisualTestContext, event: FileEvent) {
    view.update(cx, |v, cx| v.note_disk_events(&[event], cx));
    // 合并器按真实时间判定安静期（最长等待 1 s；系统监听也可能补报同一路径），定时器走测试时钟：
    // 两者都推进过去。
    std::thread::sleep(Duration::from_millis(1100));
    for _ in 0..3 {
        cx.executor().advance_clock(Duration::from_millis(1100));
        settle(cx);
    }
}

#[gpui::test]
fn edit_save_external_change_reload_through_real_window(cx: &mut TestAppContext) {
    let fx = Fixture::new();
    let lines: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let disk = fx.file("notes.txt", &lines);
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, cx, "notes.txt");
    assert!(
        code.read_with(cx, |c, _| c.is_editable()),
        "普通文件建好 TextFile 后可编辑"
    );

    // 编辑：滚到中部，在第 150 行行首输入。
    code.update(cx, |c, cx| {
        c.set_scroll_position(ScrollPosition { top: 100.0, x: 0.0 }, false, cx);
        c.set_selection(
            Selection::caret(rove_base::Position {
                line: Line(150),
                column: rove_base::Utf8Column(0),
            }),
            None,
            cx,
        );
    });
    settle(cx);
    cx.simulate_input("X");
    settle(cx);
    assert!(shown_text(&code, cx).contains("Xline 150\n"));
    assert!(active_tab(&view, cx, |_, t| t.dirty), "编辑后标签显示脏点");
    let doc = active_doc(&view, cx);
    let version = code.read_with(cx, |c, _| c.version());
    assert_eq!(
        active_tab(&view, cx, |_, t| t
            .opened()
            .unwrap()
            .document
            .text
            .version()),
        version,
        "打开文档服务里的文档换成了编辑后的快照"
    );

    // 保存：ctrl+s → SaveController（后台写盘）。
    save_action(cx, ids::SAVE);
    let written = std::fs::read_to_string(&disk).unwrap();
    assert!(written.contains("Xline 150\n"));
    assert!(!active_tab(&view, cx, |_, t| t.dirty), "保存后脏点消失");

    // 撤销回保存点之前又变脏，重做回到保存点又干净。
    let undo = code_view::commands::action_for("undo").unwrap();
    cx.update(|window, cx| window.dispatch_action(undo, cx));
    settle(cx);
    assert!(active_tab(&view, cx, |_, t| t.dirty));
    let redo = code_view::commands::action_for("redo").unwrap();
    cx.update(|window, cx| window.dispatch_action(redo, cx));
    settle(cx);
    assert!(!active_tab(&view, cx, |_, t| t.dirty));

    // 外部修改（本地没有修改）：静默重载，新快照交给代码视图，滚动位置不变。
    let top_before = code.read_with(cx, |c, _| c.scroll_position().top);
    let changed = written.replace("line 10\n", "line ten\n");
    std::fs::write(&disk, &changed).unwrap();
    external_change(
        &view,
        cx,
        FileEvent::Changed(WorkspacePath::new("notes.txt").unwrap()),
    );
    assert_eq!(shown_text(&code, cx), changed, "重载后视图显示磁盘内容");
    let top_after = code.read_with(cx, |c, _| c.scroll_position().top);
    assert!((top_before - top_after).abs() < 1e-6, "重载保持滚动位置");
    assert!(!active_tab(&view, cx, |_, t| t.dirty));
    assert_eq!(active_doc(&view, cx), doc, "文档身份不变");
    assert_eq!(
        view.read_with(cx, |v, _| v.shell.feedback.editor_bars(doc).count()),
        0,
        "本地无修改时不弹提示条"
    );

    // 本地有修改时外部修改：提示条（不静默覆盖本地修改）。
    cx.simulate_input("Y");
    settle(cx);
    std::fs::write(&disk, changed.replace("line 20\n", "line twenty\n")).unwrap();
    external_change(
        &view,
        cx,
        FileEvent::Changed(WorkspacePath::new("notes.txt").unwrap()),
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.shell.feedback.editor_bars(doc).count()),
        1
    );
    assert!(shown_text(&code, cx).contains("line 20\n"), "本地修改保留");

    // 文件被删除：标签显示“已删除”，内容保留。
    std::fs::remove_file(&disk).unwrap();
    external_change(
        &view,
        cx,
        FileEvent::Removed(WorkspacePath::new("notes.txt").unwrap()),
    );
    assert!(active_tab(&view, cx, |v, t| v.tab_deleted(t)));
    assert!(
        shown_text(&code, cx).contains("XYline 150\n")
            || shown_text(&code, cx).contains("YXline 150\n")
    );
}

#[gpui::test]
fn csharp_edits_and_saves_reach_buffer_states(cx: &mut TestAppContext) {
    let fx = Fixture::new();
    fx.file("Assets/A.cs", "class A\n{\n    void F() {}\n}\n");
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, cx, "Assets/A.cs");
    let path = "Assets/A.cs";
    let registered = view.read_with(cx, |v, _| v.nav.router.buffers().get(path).is_some());
    assert!(registered, "打开的 C# 文档已登记到 BufferStates");
    code.update(cx, |c, cx| {
        c.set_selection(
            Selection::caret(rove_base::Position {
                line: Line(0),
                column: rove_base::Utf8Column(0),
            }),
            None,
            cx,
        )
    });
    cx.simulate_input("public ");
    settle(cx);
    let version = code.read_with(cx, |c, _| c.version());
    let state = view
        .read_with(cx, |v, _| v.nav.router.buffers().get(path))
        .unwrap();
    assert_eq!(state.version, version, "编辑事务经 edited 登记");
    assert_eq!(
        state.current_hash, None,
        "未保存的修改：磁盘内容与缓冲区不同"
    );
    save_action(cx, ids::SAVE);
    let state = view
        .read_with(cx, |v, _| v.nav.router.buffers().get(path))
        .unwrap();
    assert!(state.current_hash.is_some(), "保存后经 saved 登记磁盘哈希");
    assert_eq!(state.version, code.read_with(cx, |c, _| c.version()));
}

#[gpui::test]
fn ctrl_n_opens_an_editable_untitled_document(cx: &mut TestAppContext) {
    let fx = Fixture::new();
    let (view, cx) = fx.open(cx);
    // 焦点在工作台（键位服务把 `ctrl+n` 派发到焦点路径上，窗口根元素处理 `SaveAction`）。
    cx.update(|window, cx| window.focus(&view.read(cx).focus));
    save_action(cx, ids::NEW_UNTITLED);
    let resource = active_tab(&view, cx, |_, t| t.resource.clone());
    assert!(matches!(resource, Resource::Virtual(ref r) if r.contains("Untitled-1")));
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert!(code.read_with(cx, |c, _| c.is_editable()));
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_input("hi");
    settle(cx);
    assert_eq!(shown_text(&code, cx), "hi");
    assert!(active_tab(&view, cx, |_, t| t.dirty));
    // 第二个未命名文档编号递增。
    save_action(cx, ids::NEW_UNTITLED);
    let resource = active_tab(&view, cx, |_, t| t.resource.clone());
    assert!(matches!(resource, Resource::Virtual(ref r) if r.contains("Untitled-2")));
}

#[gpui::test]
fn closing_a_dirty_tab_or_the_window_asks_first(cx: &mut TestAppContext) {
    let fx = Fixture::new();
    fx.file("a.txt", "abc\n");
    let (view, cx) = fx.open(cx);
    let _code = open_file(&view, cx, "a.txt");
    cx.simulate_input("x");
    settle(cx);
    let doc = active_doc(&view, cx);

    // 窗口关闭按钮：有未保存的修改时不关，弹“保存 / 不保存 / 取消”。
    let close_now = view.update(cx, |v, cx| v.window_should_close(cx));
    assert!(!close_now);
    assert!(view.read_with(cx, |v, _| v.shell.feedback.modal().is_some()));
    let key = format!("{}{}", ids::KEY_CLOSE, doc.0);
    view.update(cx, |v, cx| {
        v.save_feedback_action(&key, ids::ACTION_CANCEL, cx)
    });
    settle(cx);
    assert!(
        view.read_with(cx, |v, _| v.open_document(doc).is_some()),
        "取消：什么都不做"
    );

    // 关闭标签（ctrl+w）：同样先确认；“不保存”后标签关闭，磁盘不变。
    view.update(cx, |v, cx| {
        v.command("workbench.action.closeActiveEditor", cx)
    });
    settle(cx);
    assert!(view.read_with(cx, |v, _| v.open_document(doc).is_some()));
    view.update(cx, |v, cx| {
        v.save_feedback_action(&key, ids::ACTION_DONT_SAVE, cx)
    });
    settle(cx);
    assert!(view.read_with(cx, |v, _| v.open_document(doc).is_none()));
    assert_eq!(
        std::fs::read_to_string(fx.folder.path().join("a.txt")).unwrap(),
        "abc\n"
    );
}
