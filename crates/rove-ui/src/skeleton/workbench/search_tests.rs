//! #25 入口集成测试（真实 GPUI 窗口，只通过键位驱动）：`Ctrl+Shift+F` → 输入查询 → 回车 → 断言结果分组与
//! 首条位置；`F4` 打开并选中匹配；`.md` 结果打开源码；三个开关的 `alt+c/w/r`；`delete` 消除；
//! 空状态的“已排除 Unity 生成的目录”与“包含排除的文件再搜索一次”；`.gitignore` 不影响结果。
use super::*;
use crate::search::{
    model::Target,
    view::{SearchView, State},
};
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Utf8Column};
use rove_config::{SettingsOptions, SettingsService};

const PLAYER: &str = "namespace Game\n{\n    public class Player\n    {\n        public void Update()\n        {\n        }\n\n        public void Move(float speed) { Move(speed); }\n    }\n}\n";

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

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
        let folder = tempfile::tempdir().unwrap();
        let write = |rel: &str, text: &str| {
            let path = folder.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("src/Player.cs", PLAYER);
        write("src/Enemy.cs", "class Enemy { void Move() {} }\n");
        write("docs/Guide.md", "# Guide\n\nCall `Move` to walk.\n");
        write("Library/Cache.cs", "needle in the Unity cache\n");
        write(".gitignore", "src/\ndocs/\n");
        Self {
            _user: user,
            folder,
            service,
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
        cx.update(|window, cx| window.focus(&view.read(cx).focus));
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

fn search_view(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<SearchView> {
    view.read_with(cx, |v, _| v.search.clone())
}

/// `Ctrl+Shift+F`，输入查询并回车，等后台搜索结束。
fn find_in_files(view: &Entity<Workbench>, query: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("ctrl-shift-f");
    refresh(cx);
    let search = search_view(view, cx);
    let focused = cx.update(|window, cx| search.read(cx).query.focus_handle(cx).is_focused(window));
    assert!(focused, "Ctrl+Shift+F 把焦点交给查询框");
    cx.simulate_input(query);
    cx.simulate_keystrokes("enter");
    wait(view, cx);
}

fn wait(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let search = search_view(view, cx);
    for _ in 0..200 {
        cx.run_until_parked();
        if search.read_with(cx, |s, _| *s.state() != State::Searching) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    refresh(cx);
}

/// 结果行：(深度, 标签)。
fn rows(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(u32, String)> {
    let search = search_view(view, cx);
    search.read_with(cx, |s, _| {
        let rows = s.rows();
        crate::lists::ListModel::rows(&*rows, 0..rows.len())
            .unwrap()
            .iter()
            .map(|r| (r.depth, r.label.to_string()))
            .collect()
    })
}

fn active(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<(String, EditorKind)> {
    view.read_with(cx, |v, _| {
        v.editor.active().current().map(|t| {
            let name = match &t.resource {
                Resource::Workspace(p) => p.as_str().to_owned(),
                other => format!("{other:?}"),
            };
            (name, t.kind.clone())
        })
    })
}

fn selection(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> (Position, Position) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    // 选区方向由 #17 决定（插入符在起点，便于看到匹配开头）；这里只比较覆盖的区间。
    code.read_with(cx, |c, _| {
        let s = c.selection();
        (s.anchor.min(s.active), s.anchor.max(s.active))
    })
}

#[gpui::test]
fn ctrl_shift_f_groups_results_by_file_and_f4_opens_the_first_match(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (view, cx) = fixture.open(cx);
    find_in_files(&view, "Move", cx);
    let search = search_view(&view, cx);
    let summary = search.read_with(cx, |s, _| s.state().clone());
    let State::Done(summary) = summary else {
        panic!("搜索应已完成：{summary:?}");
    };
    // .gitignore 写了 src/ 与 docs/，结果不受影响；Library 被 Rove 的附加排除去掉。
    assert_eq!(summary.files_with_matches, 3);
    assert_eq!(
        rows(&view, cx),
        vec![
            (0, "Guide.md".to_string()),
            (1, "Call `Move` to walk.".to_string()),
            (0, "Enemy.cs".to_string()),
            (1, "class Enemy { void Move() {} }".to_string()),
            (0, "Player.cs".to_string()),
            (
                1,
                "public void Move(float speed) { Move(speed); }".to_string()
            ),
            (
                1,
                "public void Move(float speed) { Move(speed); }".to_string()
            ),
        ]
    );
    // F4：第一处匹配，在编辑器中打开并选中；.md 打开的是源码（文本编辑器），不是预览。
    cx.simulate_keystrokes("f4");
    refresh(cx);
    refresh(cx);
    assert_eq!(
        active(&view, cx),
        Some(("docs/Guide.md".to_string(), EditorKind::Text))
    );
    assert_eq!(selection(&view, cx), (pos(2, 6), pos(2, 10)));
    // 再按两次 F4：Enemy.cs，然后 Player.cs 第 9 行的第一处。
    cx.simulate_keystrokes("f4");
    refresh(cx);
    cx.simulate_keystrokes("f4");
    refresh(cx);
    refresh(cx);
    assert_eq!(
        active(&view, cx).map(|a| a.0),
        Some("src/Player.cs".to_string())
    );
    assert_eq!(selection(&view, cx), (pos(8, 20), pos(8, 24)));
    // Shift+F4 回到上一处。
    cx.simulate_keystrokes("shift-f4");
    refresh(cx);
    refresh(cx);
    assert_eq!(
        active(&view, cx).map(|a| a.0),
        Some("src/Enemy.cs".to_string())
    );
}

#[gpui::test]
fn toggles_remove_and_the_unity_exclusion_empty_state(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (view, cx) = fixture.open(cx);
    find_in_files(&view, "needle", cx);
    let search = search_view(&view, cx);
    let state = search.read_with(cx, |s, _| s.state().clone());
    let State::Done(summary) = state else {
        panic!("{state:?}");
    };
    assert!(
        summary.excluded_by_rove_defaults,
        "Library 只因 Rove 的附加排除而被排除"
    );
    assert!(rows(&view, cx).is_empty());
    // “包含排除的文件再搜索一次”。
    search.update(cx, |s, cx| s.search_without_excludes(cx));
    wait(&view, cx);
    assert_eq!(rows(&view, cx)[0], (0, "Cache.cs".to_string()));
    assert!(!search.read_with(cx, |s, _| s.toggles.use_exclude_settings));

    // alt+c / alt+w / alt+r 在搜索视图里切换开关并重新搜索。
    search.update(cx, |s, cx| s.set_query("move", cx));
    wait(&view, cx);
    let all = search.read_with(cx, |s, _| s.results().match_count());
    assert_eq!(all, 4);
    cx.simulate_keystrokes("alt-c");
    wait(&view, cx);
    assert!(search.read_with(cx, |s, _| s.toggles.case_sensitive));
    assert_eq!(search.read_with(cx, |s, _| s.results().match_count()), 0);
    cx.simulate_keystrokes("alt-c alt-r");
    wait(&view, cx);
    search.update(cx, |s, cx| s.set_query("M[a-z]+e\\(", cx));
    wait(&view, cx);
    assert!(search.read_with(cx, |s, _| s.toggles.regex));
    assert_eq!(search.read_with(cx, |s, _| s.results().match_count()), 3);
    cx.simulate_keystrokes("alt-w");
    wait(&view, cx);
    assert!(search.read_with(cx, |s, _| s.toggles.whole_word));

    // delete：焦点在结果列表的文件行上时消除整个文件。
    cx.simulate_keystrokes("alt-w alt-r");
    search.update(cx, |s, cx| s.set_query("Move", cx));
    wait(&view, cx);
    let list = search.read_with(cx, |s, _| s.list.clone());
    cx.update(|window, cx| window.focus(&list.focus_handle(cx)));
    list.update(cx, |l, cx| {
        l.widget.set_focus(Some(0));
        cx.notify();
    });
    refresh(cx);
    let first = search.read_with(cx, |s, _| s.rows().target(0));
    assert!(matches!(first, Some(Target::File(_))));
    cx.simulate_keystrokes("delete");
    refresh(cx);
    assert_eq!(search.read_with(cx, |s, _| s.results().file_count()), 2);
    // escape 在结果列表里（搜索已结束）不做事；clearSearchResults 清空。
    search.update(cx, |s, cx| s.set_query("", cx));
    refresh(cx);
    assert!(rows(&view, cx).is_empty());
    assert_eq!(search.read_with(cx, |s, _| s.state().clone()), State::Idle);
}

// #51：搜索视图里的 `Ctrl+Shift+L`（`addCursorsAtSearchResults`）：打开聚焦的那个文件，
// 并在它的每一处匹配上放一个光标。
#[gpui::test]
fn add_cursors_at_search_results_opens_the_file_with_one_cursor_per_match(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let (view, cx) = fixture.open(cx);
    find_in_files(&view, "Move", cx);
    // F4 在匹配之间走，直到聚焦到有两处匹配的 Player.cs。
    for _ in 0..6 {
        if active(&view, cx).map(|(name, _)| name).as_deref() == Some("src/Player.cs") {
            break;
        }
        cx.simulate_keystrokes("f4");
        refresh(cx);
        refresh(cx);
    }
    assert_eq!(
        active(&view, cx).map(|(name, _)| name).as_deref(),
        Some("src/Player.cs")
    );
    let search = search_view(&view, cx);
    cx.update(|window, cx| {
        search.update(cx, |s, cx| {
            s.command(crate::search::registry::ADD_CURSORS, window, cx)
        })
    });
    refresh(cx);
    refresh(cx);
    assert_eq!(
        active(&view, cx).map(|(name, _)| name).as_deref(),
        Some("src/Player.cs")
    );
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    let selections = code.read_with(cx, |c, _| c.selections());
    assert_eq!(selections.len(), 2, "Player.cs 的两处 Move 各一个光标");
    assert!(code.read_with(cx, |c, _| c.has_multiple_selections()));
    for s in &selections {
        assert_eq!(s.start().line, s.end().line);
        assert_eq!(s.end().column.0 - s.start().column.0, 4, "选中的是 Move");
    }
}
