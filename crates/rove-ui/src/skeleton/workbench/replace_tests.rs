//! #48 入口集成测试（真实 GPUI 窗口，只通过键位与按钮命令驱动）：`ctrl+h` 展开替换行与展开状态
//! 的保持；替换一处并前进、全部替换、一次撤销 / 重做；正则捕获组与保留大小写；在选区内替换不越界；
//! 只读文档上没有替换入口；CRLF 与混合换行符文件逐字节还原。
//!
//! 运行：`cargo test -p rove-ui replace_tests -j 1`
use super::*;
use crate::find::host::FindHost;
use crate::find::state::CountLabel;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Utf8Column};
use rove_config::{SettingsOptions, SettingsService};

const NOTES: &str = "foo bar foo\nFoo baz\nfoobar foo\nlast line\n";

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

struct Fixture {
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
    _user: tempfile::TempDir,
}

impl Fixture {
    fn new(settings_json: &str) -> Self {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("settings.json"), settings_json).unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("notes.txt"), NOTES).unwrap();
        Self {
            folder,
            service,
            _user: user,
        }
    }

    fn file(&self, path: &str, text: &str) {
        let full = self.folder.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
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

fn refresh(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

fn open_file(view: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    cx.run_until_parked();
    refresh(cx);
    refresh(cx);
    focus_code(view, cx);
}

fn focus_code(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    refresh(cx);
}

fn host(view: &Entity<Workbench>, cx: &VisualTestContext) -> Entity<FindHost> {
    view.read_with(cx, |v, _| v.views[&v.editor.focused].primary.find.clone())
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    refresh(cx);
}

fn type_text(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    cx.run_until_parked();
    refresh(cx);
}

/// 当前文档的全文（视图显示的快照）。
fn text(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> String {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| {
        let t = c.text().expect("有文本");
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

fn matches(view: &Entity<Workbench>, cx: &VisualTestContext) -> usize {
    host(view, cx).read_with(cx, |h, _| h.model().matches().len())
}

fn selection(view: &Entity<Workbench>, cx: &VisualTestContext) -> (Position, Position) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| (c.selection().start(), c.selection().end()))
}

fn select(view: &Entity<Workbench>, a: Position, b: Position, cx: &mut VisualTestContext) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, cx| {
        c.set_selection(code_view::Selection::new(a, b), None, cx)
    });
    refresh(cx);
}

/// 查找串 + 替换串：`ctrl+f` 输入查找串，`ctrl+h` 展开替换行并输入替换串。
fn prepare(search: &str, replacement: &str, cx: &mut VisualTestContext) {
    press("ctrl-f", cx);
    type_text(search, cx);
    press("ctrl-h", cx);
    if !replacement.is_empty() {
        type_text(replacement, cx);
    }
}

#[gpui::test]
fn ctrl_h_expands_the_replace_row_and_the_state_is_kept(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "notes.txt", cx);
    // `ctrl+f`：替换行默认收起。
    press("ctrl-f", cx);
    host(&view, cx).read_with(cx, |h, cx| {
        assert!(h.is_visible());
        assert!(!h.replace_shown(cx), "ctrl+f 不展开替换行");
        assert!(h.can_replace(cx), "普通文件可写");
    });
    // `ctrl+h`：展开并把焦点放到替换框。
    press("ctrl-h", cx);
    host(&view, cx).read_with(cx, |h, cx| assert!(h.replace_shown(cx)));
    type_text("zzz", cx);
    host(&view, cx).read_with(cx, |h, cx| assert_eq!(h.replace_text(cx), "zzz"));
    // 关闭再用 `ctrl+f` 打开：同一窗口内展开状态保持。
    press("escape", cx);
    host(&view, cx).read_with(cx, |h, cx| assert!(!h.replace_shown(cx), "关闭后不显示"));
    press("ctrl-f", cx);
    host(&view, cx).read_with(cx, |h, cx| {
        assert!(h.replace_shown(cx), "展开状态保持");
        assert_eq!(h.replace_text(cx), "zzz", "替换串保持");
    });
}

#[gpui::test]
fn replace_one_advances_to_the_next_match_and_undo_restores(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "notes.txt", cx);
    prepare("foo", "qux", cx);
    assert_eq!(matches(&view, cx), 5);
    // 输入即查找已经选中第一处匹配：替换框里的 `enter` 替换它并前进到下一处。
    assert_eq!(selection(&view, cx), (pos(0, 0), pos(0, 3)));
    press("enter", cx);
    assert_eq!(
        text(&view, cx),
        "qux bar foo\nFoo baz\nfoobar foo\nlast line\n"
    );
    assert_eq!(
        selection(&view, cx),
        (pos(0, 8), pos(0, 11)),
        "前进到下一处"
    );
    assert_eq!(matches(&view, cx), 4);
    // 替换行展开时，查找框里的 `enter` 仍然是“查找下一个”（两个上下文键互斥）。
    let find = host(&view, cx);
    let focus = find.read_with(cx, |h, cx| h.input_focus(cx));
    cx.update(|window, _| window.focus(&focus));
    refresh(cx);
    press("enter", cx);
    assert_eq!(
        text(&view, cx),
        "qux bar foo\nFoo baz\nfoobar foo\nlast line\n",
        "查找框里的 enter 不替换"
    );
    assert_eq!(selection(&view, cx), (pos(1, 0), pos(1, 3)), "只是前进");
    // 再替换一处：`ctrl+shift+1` 在哪个框里都等价（当前匹配是第 2 行的 Foo）。
    press("ctrl-shift-1", cx);
    assert_eq!(
        text(&view, cx),
        "qux bar foo\nqux baz\nfoobar foo\nlast line\n"
    );
    // 两次替换是两个撤销单元。
    press("escape", cx);
    press("ctrl-z", cx);
    assert_eq!(
        text(&view, cx),
        "qux bar foo\nFoo baz\nfoobar foo\nlast line\n"
    );
    press("ctrl-z", cx);
    assert_eq!(text(&view, cx), NOTES);
}

#[gpui::test]
fn replace_all_is_a_single_undo_unit(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "notes.txt", cx);
    prepare("foo", "qux", cx);
    press("ctrl-alt-enter", cx);
    assert_eq!(
        text(&view, cx),
        "qux bar qux\nqux baz\nquxbar qux\nlast line\n"
    );
    // 替换后匹配清零，查找串保留。
    assert_eq!(matches(&view, cx), 0);
    host(&view, cx).read_with(cx, |h, cx| {
        assert_eq!(h.input_text(cx), "foo");
        assert_eq!(h.model().count_label(), CountLabel::NoResults);
    });
    // 一次撤销全部回退，一次重做全部恢复。
    press("escape", cx);
    press("ctrl-z", cx);
    assert_eq!(text(&view, cx), NOTES);
    press("ctrl-y", cx);
    assert_eq!(
        text(&view, cx),
        "qux bar qux\nqux baz\nquxbar qux\nlast line\n"
    );
}

#[gpui::test]
fn crlf_and_mixed_newlines_come_back_byte_identical(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let mixed = "foo one\r\nfoo two\nfoo three\r\n";
    fx.file("mixed.txt", mixed);
    let (view, cx) = fx.open(cx);
    open_file(&view, "mixed.txt", cx);
    prepare("foo", "bar baz", cx);
    press("ctrl-alt-enter", cx);
    assert_eq!(
        text(&view, cx),
        "bar baz one\r\nbar baz two\nbar baz three\r\n"
    );
    press("escape", cx);
    press("ctrl-z", cx);
    assert_eq!(text(&view, cx), mixed, "逐字节还原（含混合换行符）");
    press("ctrl-y", cx);
    assert_eq!(
        text(&view, cx),
        "bar baz one\r\nbar baz two\nbar baz three\r\n"
    );
}

#[gpui::test]
fn regex_replacement_expands_capture_groups(cx: &mut TestAppContext) {
    // 种子关掉：`ctrl+h` 不再用“当前匹配”覆盖刚输入的正则（VS Code 的种子规则本身见 #21）。
    let fx = Fixture::new(r#"{"editor.find.seedSearchStringFromSelection": "never"}"#);
    fx.file("code.txt", "int a = 1;\nint bb = 22;\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, "code.txt", cx);
    press("ctrl-f", cx);
    press("alt-r", cx); // toggleFindRegex
    type_text("(\\w+) = (\\d+)", cx);
    assert_eq!(matches(&view, cx), 2);
    press("ctrl-h", cx);
    type_text("$2 <- $1", cx);
    press("ctrl-alt-enter", cx);
    assert_eq!(text(&view, cx), "int 1 <- a;\nint 22 <- bb;\n");
    press("escape", cx);
    press("ctrl-z", cx);
    assert_eq!(text(&view, cx), "int a = 1;\nint bb = 22;\n");
}

#[gpui::test]
fn preserve_case_follows_each_match(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    fx.file("case.txt", "foo FOO Foo fOo\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, "case.txt", cx);
    prepare("foo", "bar", cx);
    assert_eq!(matches(&view, cx), 4, "默认不区分大小写");
    press("alt-p", cx); // togglePreserveCase
    host(&view, cx).read_with(cx, |h, _| assert!(h.preserve_case()));
    press("ctrl-alt-enter", cx);
    assert_eq!(text(&view, cx), "bar BAR Bar bar\n");
}

#[gpui::test]
fn replacing_in_selection_stays_inside_the_range(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    fx.file("scope.txt", "foo foo\nfoo foo\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, "scope.txt", cx);
    // 选中第一行，打开“在选定内容中查找”。
    select(&view, pos(0, 0), pos(0, 7), cx);
    press("ctrl-f", cx);
    press("alt-l", cx); // toggleFindInSelection
    type_text("foo", cx);
    assert_eq!(matches(&view, cx), 2, "范围内只有两处");
    press("ctrl-h", cx);
    type_text("quux", cx);
    press("ctrl-alt-enter", cx);
    assert_eq!(text(&view, cx), "quux quux\nfoo foo\n", "第二行不受影响");
    // 范围跟着变长：重新搜索后仍然只在第一行里找。
    host(&view, cx).read_with(cx, |h, _| {
        let scope = h.model().scope().expect("范围还在");
        assert_eq!(scope.start(), pos(0, 0));
        assert_eq!(scope.end(), pos(0, 9));
    });
    press("escape", cx);
    press("ctrl-z", cx);
    assert_eq!(text(&view, cx), "foo foo\nfoo foo\n");
}

#[gpui::test]
fn read_only_documents_have_no_replace_entry_point(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    fx.file(
        "Library/PackageCache/com.unity.foo/Runtime/Foo.cs",
        "foo bar foo\n",
    );
    let (view, cx) = fx.open(cx);
    open_file(
        &view,
        "Library/PackageCache/com.unity.foo/Runtime/Foo.cs",
        cx,
    );
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| assert!(c.is_read_only(), "包缓存目录只读"));
    press("ctrl-h", cx);
    host(&view, cx).read_with(cx, |h, cx| {
        assert!(h.is_visible(), "ctrl+h 仍然打开查找条");
        assert!(!h.can_replace(cx));
        assert!(!h.replace_shown(cx), "只读文档不显示替换行");
    });
    type_text("foo", cx);
    // 替换命令执行后文本不变。
    press("ctrl-alt-enter", cx);
    press("ctrl-shift-1", cx);
    assert_eq!(text(&view, cx), "foo bar foo\n");
}

#[gpui::test]
fn replace_all_rewrites_five_thousand_matches_in_one_undo_unit(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let mut big = String::with_capacity(4_500_000);
    for i in 0..100_000u32 {
        if i % 20 == 0 {
            big.push_str("    var target = GetComponent<TargetComponent>();\n");
        } else {
            big.push_str("    int value = Mathf.Max(1, speed * delta);\n");
        }
    }
    fx.file("big.txt", &big);
    let (view, cx) = fx.open(cx);
    open_file(&view, "big.txt", cx);
    prepare("TargetComponent", "RenamedComponent", cx);
    assert_eq!(matches(&view, cx), 5_000);
    let started = Instant::now();
    press("ctrl-alt-enter", cx);
    let elapsed = started.elapsed();
    assert_eq!(matches(&view, cx), 0, "替换后计数清零");
    assert!(
        text(&view, cx).contains("RenamedComponent")
            && !text(&view, cx).contains("TargetComponent")
    );
    press("escape", cx);
    press("ctrl-z", cx);
    assert_eq!(text(&view, cx), big, "一次撤销全部回退");
    eprintln!("5000 处替换（含重新搜索与刷新，debug 构建）：{elapsed:?}");
}

#[gpui::test]
fn replace_history_is_browsed_with_up_and_down(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "notes.txt", cx);
    prepare("foo", "one", cx);
    press("ctrl-alt-enter", cx);
    press("ctrl-a", cx);
    type_text("two", cx);
    press("up", cx);
    host(&view, cx).read_with(cx, |h, cx| assert_eq!(h.replace_text(cx), "one"));
    press("down", cx);
    host(&view, cx).read_with(cx, |h, cx| assert_eq!(h.replace_text(cx), "two"));
    // 查找历史与替换历史各自独立。
    host(&view, cx).read_with(cx, |h, cx| assert_eq!(h.input_text(cx), "foo"));
}
