//! #21 入口集成测试（真实 GPUI 窗口，只通过键位驱动）：`ctrl+f` 打开查找条并按选区取种子；
//! 输入即查找与光标跟随；`enter` / `f3` / `shift+f3` / `ctrl+f3` 导航与循环；四个开关；非法正则；
//! 在选区内查找与 `autoFindInSelection`；历史；`escape` 清除装饰；替换入口不存在。
use super::*;
use crate::find::{
    engine::FindOptions,
    host::FindHost,
    state::{CountLabel, SearchState},
};
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
        std::fs::write(folder.path().join("other.txt"), "other file with foo\n").unwrap();
        Self {
            folder,
            service,
            _user: user,
        }
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
        open_file(&view, "notes.txt", cx);
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
    // 键盘焦点放进代码视图。
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    refresh(cx);
}

fn host(view: &Entity<Workbench>, cx: &VisualTestContext) -> Entity<FindHost> {
    view.read_with(cx, |v, _| v.views[&v.editor.focused].primary.find.clone())
}

fn select(view: &Entity<Workbench>, a: Position, b: Position, cx: &mut VisualTestContext) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, cx| {
        c.set_selection(code_view::Selection::new(a, b), None, cx)
    });
    refresh(cx);
}

fn selection(view: &Entity<Workbench>, cx: &VisualTestContext) -> (Position, Position) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| (c.selection().start(), c.selection().end()))
}

/// 键入文字（真实输入事件，进入焦点输入框）并等后台搜索完成。
fn type_text(text: &str, cx: &mut VisualTestContext) {
    cx.simulate_input(text);
    cx.run_until_parked();
    refresh(cx);
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    refresh(cx);
}

fn label(view: &Entity<Workbench>, cx: &VisualTestContext) -> CountLabel {
    host(view, cx).read_with(cx, |h, _| h.model().count_label())
}

fn visible(view: &Entity<Workbench>, cx: &VisualTestContext) -> bool {
    host(view, cx).read_with(cx, |h, _| h.is_visible())
}

fn options(view: &Entity<Workbench>, cx: &VisualTestContext) -> FindOptions {
    host(view, cx).read_with(cx, |h, _| h.model().options())
}

fn input(view: &Entity<Workbench>, cx: &VisualTestContext) -> String {
    host(view, cx).read_with(cx, |h, cx| h.input_text(cx))
}

fn matches(view: &Entity<Workbench>, cx: &VisualTestContext) -> Vec<(u64, u64)> {
    host(view, cx).read_with(cx, |h, _| {
        h.model()
            .matches()
            .iter()
            .map(|m| (m.start.line.0, m.start.column.0))
            .collect()
    })
}

#[gpui::test]
fn ctrl_f_opens_with_seed_and_escape_clears(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    assert!(!visible(&view, cx));
    // 选中第一个 foo：按 seedSearchStringFromSelection=always 作为种子。
    select(&view, pos(0, 0), pos(0, 3), cx);
    press("ctrl-f", cx);
    assert!(visible(&view, cx));
    assert_eq!(input(&view, cx), "foo");
    // 默认不区分大小写：foo、foo、Foo、foo、foo。
    assert_eq!(matches(&view, cx), [(0, 0), (0, 8), (1, 0), (2, 0), (2, 7)]);
    assert_eq!(
        label(&view, cx),
        CountLabel::Matches {
            index: Some(1),
            total: 5,
            over_limit: false
        }
    );
    // 装饰：当前匹配与其他匹配都生成了装饰层数据。
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert!(code.read_with(cx, |c, _| c.merged_decorations(0)).is_some());
    // 焦点在查找输入框：escape 关闭并清除匹配，焦点回到代码视图。
    press("escape", cx);
    assert!(!visible(&view, cx));
    assert!(matches(&view, cx).is_empty());
    let code_focused = cx.update(|window, cx| code.focus_handle(cx).is_focused(window));
    assert!(code_focused);
    // 再次 escape 不会有任何反应。
    press("escape", cx);
    assert!(!visible(&view, cx));
}

#[gpui::test]
fn seed_follows_the_setting_and_word_under_caret(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    // 空选区：取插入符所在的单词。
    select(&view, pos(2, 2), pos(2, 2), cx);
    press("ctrl-f", cx);
    assert_eq!(input(&view, cx), "foobar");
    press("escape", cx);
    // 多行选区：不取种子，沿用上次的查找文本。
    select(&view, pos(0, 0), pos(1, 3), cx);
    press("ctrl-f", cx);
    assert_eq!(input(&view, cx), "foobar");
}

#[gpui::test]
fn seed_never_leaves_the_box_empty(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{"editor.find.seedSearchStringFromSelection": "never"}"#);
    let (view, cx) = fx.open(cx);
    select(&view, pos(0, 0), pos(0, 3), cx);
    press("ctrl-f", cx);
    assert_eq!(input(&view, cx), "");
}

#[gpui::test]
fn typing_finds_and_moves_the_cursor_then_enter_and_f3_navigate(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    select(&view, pos(0, 4), pos(0, 4), cx);
    press("ctrl-f", cx);
    // 种子是 bar；全选后键入 foo：输入即查找，光标跟随到第一个匹配（从起点 (0,4) 往后）。
    type_text("foo", cx);
    assert_eq!(matches(&view, cx).len(), 5);
    assert_eq!(selection(&view, cx), (pos(0, 8), pos(0, 11)));
    // 输入框里 enter / shift+enter。
    press("enter", cx);
    assert_eq!(selection(&view, cx), (pos(1, 0), pos(1, 3)));
    assert_eq!(
        label(&view, cx),
        CountLabel::Matches {
            index: Some(3),
            total: 5,
            over_limit: false
        }
    );
    press("shift-enter", cx);
    assert_eq!(selection(&view, cx), (pos(0, 8), pos(0, 11)));
    // 回到代码视图后 f3 / shift+f3 同样导航。
    press("escape", cx);
    press("f3", cx);
    // f3 在条关闭时重新打开它（不抢焦点）并移动。
    assert!(visible(&view, cx));
    assert_eq!(selection(&view, cx), (pos(1, 0), pos(1, 3)));
    press("shift-f3", cx);
    assert_eq!(selection(&view, cx), (pos(0, 8), pos(0, 11)));
}

#[gpui::test]
fn navigation_loops(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    select(&view, pos(2, 7), pos(2, 10), cx);
    press("ctrl-f", cx);
    assert_eq!(input(&view, cx), "foo");
    // 当前在最后一个匹配：下一个循环回第一个。
    press("enter", cx);
    assert_eq!(selection(&view, cx), (pos(0, 0), pos(0, 3)));
    press("shift-enter", cx);
    assert_eq!(selection(&view, cx), (pos(2, 7), pos(2, 10)));
}

#[gpui::test]
fn navigation_stops_at_the_ends_when_loop_is_off(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{"editor.find.loop": false}"#);
    let (view, cx) = fx.open(cx);
    select(&view, pos(2, 7), pos(2, 10), cx);
    press("ctrl-f", cx);
    press("enter", cx);
    assert_eq!(selection(&view, cx), (pos(2, 7), pos(2, 10)));
    select(&view, pos(0, 0), pos(0, 3), cx);
    press("shift-enter", cx);
    assert_eq!(selection(&view, cx), (pos(0, 0), pos(0, 3)));
}

#[gpui::test]
fn find_on_type_off_waits_for_enter(cx: &mut TestAppContext) {
    let fx =
        Fixture::new(r#"{"editor.find.findOnType": false, "editor.find.cursorMoveOnType": false}"#);
    let (view, cx) = fx.open(cx);
    press("ctrl-f", cx);
    type_text("baz", cx);
    // 输入时不搜索，也不移动光标。
    // （匹配仍是打开时按种子 foo 搜出来的。）
    assert!(!matches(&view, cx).contains(&(1, 4)));
    assert_eq!(selection(&view, cx), (pos(0, 0), pos(0, 0)));
    press("enter", cx);
    assert_eq!(matches(&view, cx), [(1, 4)]);
    assert_eq!(selection(&view, cx), (pos(1, 4), pos(1, 7)));
}

#[gpui::test]
fn selection_match_commands_search_the_selected_text(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    select(&view, pos(2, 0), pos(2, 6), cx);
    press("ctrl-f3", cx);
    assert_eq!(input(&view, cx), "foobar");
    assert!(visible(&view, cx));
    // 只有一处 foobar：循环回自己。
    assert_eq!(selection(&view, cx), (pos(2, 0), pos(2, 6)));
    select(&view, pos(0, 0), pos(0, 3), cx);
    press("ctrl-f3", cx);
    assert_eq!(input(&view, cx), "foo");
    assert_eq!(selection(&view, cx).0, pos(0, 8));
    press("ctrl-shift-f3", cx);
    assert_eq!(selection(&view, cx).0, pos(0, 0));
}

#[gpui::test]
fn toggles_change_results_and_persist_across_files(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    select(&view, pos(0, 0), pos(0, 3), cx);
    press("ctrl-f", cx);
    assert_eq!(matches(&view, cx).len(), 5);
    press("alt-c", cx);
    assert!(options(&view, cx).case_sensitive);
    assert_eq!(matches(&view, cx).len(), 4);
    press("alt-w", cx);
    assert!(options(&view, cx).whole_word);
    // foobar 不是全词：foo、foo、foo。
    assert_eq!(matches(&view, cx), [(0, 0), (0, 8), (2, 7)]);
    press("alt-r", cx);
    assert!(options(&view, cx).regex);
    // 切换文件：选项保持（同一窗口内），查找条仍然打开并重新搜索。
    open_file(&view, "other.txt", cx);
    assert!(visible(&view, cx));
    assert_eq!(
        options(&view, cx),
        FindOptions {
            case_sensitive: true,
            whole_word: true,
            regex: true
        }
    );
    assert_eq!(matches(&view, cx), [(0, 16)]);
    press("alt-c", cx);
    assert!(!options(&view, cx).case_sensitive);
}

#[gpui::test]
fn regex_groups_multiline_and_invalid_pattern(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    press("ctrl-f", cx);
    press("alt-r", cx);
    type_text("(foo)bar", cx);
    assert_eq!(matches(&view, cx), [(2, 0)]);
    // 跨行。
    press("ctrl-a", cx);
    type_text("baz\\nfoo", cx);
    assert_eq!(matches(&view, cx), [(1, 4)]);
    // 行首行尾语义。
    press("ctrl-a", cx);
    type_text("^Foo", cx);
    assert_eq!(matches(&view, cx), [(0, 0), (1, 0), (2, 0)]);
    press("alt-c", cx);
    assert_eq!(matches(&view, cx), [(1, 0)]);
    // 非法正则：显示校验错误、没有匹配（不是静默无结果）。
    press("ctrl-a", cx);
    type_text("(", cx);
    assert!(matches(&view, cx).is_empty());
    host(&view, cx).read_with(cx, |h, _| assert!(h.model().error().is_some()));
}

#[gpui::test]
fn find_in_selection_limits_matches(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    // 选中第 2~3 行，手动打开“在选区内查找”。
    select(&view, pos(1, 0), pos(2, 10), cx);
    press("ctrl-f", cx);
    type_text("foo", cx);
    assert_eq!(matches(&view, cx).len(), 5);
    select(&view, pos(1, 0), pos(2, 10), cx);
    // 选区在代码视图里改变后，焦点回到查找输入框（`alt+l` 在输入框里同样生效）。
    let find = host(&view, cx);
    cx.update(|window, cx| {
        let focus = find.read(cx).input_focus(cx);
        window.focus(&focus);
    });
    press("alt-l", cx);
    assert!(host(&view, cx).read_with(cx, |h, _| h.model().in_selection()));
    assert_eq!(matches(&view, cx), [(1, 0), (2, 0), (2, 7)]);
    press("alt-l", cx);
    assert_eq!(matches(&view, cx).len(), 5);
}

#[gpui::test]
fn auto_find_in_selection_multiline(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{"editor.find.autoFindInSelection": "multiline"}"#);
    let (view, cx) = fx.open(cx);
    select(&view, pos(1, 0), pos(2, 10), cx);
    press("ctrl-f", cx);
    assert!(host(&view, cx).read_with(cx, |h, _| h.model().in_selection()));
    press("escape", cx);
    select(&view, pos(0, 0), pos(0, 3), cx);
    press("ctrl-f", cx);
    assert!(!host(&view, cx).read_with(cx, |h, _| h.model().in_selection()));
}

#[gpui::test]
fn auto_find_in_selection_always(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{"editor.find.autoFindInSelection": "always"}"#);
    let (view, cx) = fx.open(cx);
    select(&view, pos(0, 0), pos(0, 3), cx);
    press("ctrl-f", cx);
    assert!(host(&view, cx).read_with(cx, |h, _| h.model().in_selection()));
}

#[gpui::test]
fn history_is_browsed_with_up_and_down(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    press("ctrl-f", cx);
    type_text("foo", cx);
    press("enter", cx);
    press("ctrl-a", cx);
    type_text("baz", cx);
    press("enter", cx);
    press("ctrl-a", cx);
    type_text("last", cx);
    press("up", cx);
    assert_eq!(input(&view, cx), "baz");
    press("up", cx);
    assert_eq!(input(&view, cx), "foo");
    press("down", cx);
    assert_eq!(input(&view, cx), "baz");
    // 历史驱动搜索。
    assert_eq!(matches(&view, cx), [(1, 4)]);
}

#[gpui::test]
fn counts_cover_none_single_many(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    press("ctrl-f", cx);
    type_text("zzz", cx);
    assert_eq!(label(&view, cx), CountLabel::NoResults);
    press("ctrl-a", cx);
    type_text("last", cx);
    assert!(matches!(
        label(&view, cx),
        CountLabel::Matches {
            total: 1,
            over_limit: false,
            ..
        }
    ));
    press("ctrl-a", cx);
    type_text("o", cx);
    assert!(matches!(
        label(&view, cx),
        CountLabel::Matches {
            total: 10,
            over_limit: false,
            ..
        }
    ));
    host(&view, cx).read_with(cx, |h, _| assert_eq!(h.model().state(), SearchState::Idle));
}

#[gpui::test]
fn find_with_selection_and_no_replace_entry_points(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    // `ctrl+h`（startFindReplaceAction）没有注册：什么也不会发生。
    press("ctrl-h", cx);
    assert!(!visible(&view, cx));
    // 命令登记表里没有任何替换命令。
    view.read_with(cx, |v, _| {
        for (id, _) in v.commands.registry().entries() {
            assert!(!id.to_lowercase().contains("replace"), "{id}");
        }
    });
    // actions.findWithSelection（无默认键位）：以选区为查找文本打开，不抢焦点。
    select(&view, pos(1, 4), pos(1, 7), cx);
    let find = host(&view, cx);
    cx.update(|window, cx| {
        find.update(cx, |h, cx| {
            h.execute(crate::find::registry::FIND_WITH_SELECTION, window, cx)
        });
    });
    refresh(cx);
    assert!(visible(&view, cx));
    assert_eq!(input(&view, cx), "baz");
}
