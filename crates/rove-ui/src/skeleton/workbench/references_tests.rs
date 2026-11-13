//! #60 侧边「引用」视图的入口集成测试（真实 GPUI 窗口，键位 / 命令驱动）：
//! `Shift+Alt+F12` → 面板里的分组与计数；`F4` / `Shift+F4` 逐条导航并打开；
//! `references.preferredLocation` 的两个取值；12 条 `references-view.*` 命令逐条可用；
//! 层级相关的 6 条只登记、不出现在命令面板；两个上下文键的取值；4.7 万条结果。
//!
//! 运行：`cargo test -p rove-ui references_`。查询数据来自替身（[`Fake`]），C# 文件是真实的临时工作区文件。
use super::*;
use crate::documents::OpenOptions;
use crate::peek::{Availability, PeekKind, PeekQueries};
use crate::references::registry as reg;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Span, Utf8Column};
use rove_config::{SettingsOptions, SettingsService};
use rove_lang_cs::query::{Hover, NavigationKind, Precision, QueryPoint, QueryResult, Target};
use rove_lang_cs::router::{RoutedTarget, TargetRole};

const PLAYER: &str =
    "class Player\n{\n    public void Move() { }\n    void Update() { Move(); Move(); }\n}\n";
const ENEMY: &str =
    "class Enemy\n{\n    void Chase(Player p)\n    {\n        p.Move();\n    }\n}\n";

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

fn routed(path: &str, line: u64, col: u64, precision: Precision) -> RoutedTarget {
    let range = Span::new(pos(line, col), pos(line, col + 4)).unwrap();
    RoutedTarget {
        target: Target {
            resource: Resource::Workspace(WorkspacePath::new(path).unwrap()),
            range: range.clone(),
            selection: range,
        },
        precision,
        reason: None,
        role: TargetRole::Reference,
    }
}

/// `Move` 的四处引用：Enemy.cs 一处、Player.cs 三处。
fn move_references() -> Vec<RoutedTarget> {
    vec![
        routed("Assets/Enemy.cs", 4, 10, Precision::Exact),
        routed("Assets/Player.cs", 2, 16, Precision::Exact),
        routed("Assets/Player.cs", 3, 20, Precision::Exact),
        routed("Assets/Player.cs", 3, 28, Precision::Exact),
    ]
}

#[derive(Default)]
struct Fake {
    references: Vec<RoutedTarget>,
    implementations: Vec<RoutedTarget>,
}

impl PeekQueries for Fake {
    fn navigate(
        &self,
        kind: NavigationKind,
        point: &QueryPoint,
        _: &Cancellation,
    ) -> CoreResult<QueryResult<Arc<[RoutedTarget]>>> {
        let value: Arc<[RoutedTarget]> = match kind {
            NavigationKind::References => self.references.clone().into(),
            NavigationKind::Implementations => self.implementations.clone().into(),
            _ => Arc::from([]),
        };
        let precision = if value.iter().all(|t| t.precision == Precision::Exact) {
            Precision::Exact
        } else {
            Precision::Approximate
        };
        Ok(QueryResult {
            value,
            precision,
            reason: None,
            version: Some(point.version),
            index_generation: 1,
        })
    }
    fn hover(
        &self,
        point: &QueryPoint,
        _: &Cancellation,
    ) -> CoreResult<QueryResult<Option<Hover>>> {
        Ok(QueryResult {
            value: None,
            precision: Precision::Exact,
            reason: None,
            version: Some(point.version),
            index_generation: 1,
        })
    }
    fn availability(&self, _: PeekKind) -> Availability {
        Availability::Available
    }
}

struct Fixture {
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Fixture {
    fn new(settings_json: &str) -> Self {
        let user = tempfile::tempdir().unwrap();
        let json = settings_json.replacen('{', "{ \"rove.index.enabled\": false,", 1);
        std::fs::write(user.path().join("settings.json"), json).unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        let folder = tempfile::tempdir().unwrap();
        for (path, text) in [("Assets/Player.cs", PLAYER), ("Assets/Enemy.cs", ENEMY)] {
            let full = folder.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        // 用户目录只在构造设置服务时用到，保留到进程结束即可。
        std::mem::forget(user);
        Self { folder, service }
    }

    fn open<'a>(
        &self,
        fake: Fake,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let root: PathBuf = self.folder.path().into();
        let settings: Arc<dyn Settings> = self.service.clone();
        let queries: Arc<dyn PeekQueries> = Arc::new(fake);
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut v = Workbench::with_settings(
                root,
                Instant::now(),
                LayoutStore::disabled(),
                settings,
                cx,
            )
            .unwrap();
            v.set_peek_queries(Some(queries));
            v
        });
        cx.run_until_parked();
        open_file(&view, "Assets/Player.cs", cx);
        (view, cx)
    }
}

fn refresh(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.update(|window, cx| {
            window.refresh();
            cx.refresh_windows();
        });
        cx.run_until_parked();
    }
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
    focus_code(view, cx);
}

fn focus_code(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    refresh(cx);
}

fn caret(view: &Entity<Workbench>, at: Position, cx: &mut VisualTestContext) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, cx| {
        c.set_selection(code_view::Selection::caret(at), None, cx)
    });
    refresh(cx);
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    refresh(cx);
}

/// 直接运行一条命令（命令面板与菜单走的也是这条路径）。
fn run(view: &Entity<Workbench>, id: &'static str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |v, cx| v.references_command(id, window, cx));
    });
    cx.run_until_parked();
    refresh(cx);
}

fn panel(view: &Entity<Workbench>, cx: &VisualTestContext) -> (bool, String) {
    view.read_with(cx, |v, _| {
        (v.shell.layout.panel_visible, v.shell.active_panel_view())
    })
}

fn groups(view: &Entity<Workbench>, cx: &VisualTestContext) -> Vec<(String, usize)> {
    view.read_with(cx, |v, _| v.references_groups())
}

fn selected(view: &Entity<Workbench>, cx: &VisualTestContext) -> Option<usize> {
    view.read_with(cx, |v, _| v.references_state().selected)
}

fn active_tab(view: &Entity<Workbench>, cx: &VisualTestContext) -> (String, Option<TextRange>) {
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        let path = match &tab.resource {
            Resource::Workspace(p) => p.as_str().to_string(),
            other => format!("{other:?}"),
        };
        (path, tab.view.selection.clone())
    })
}

/// 全局上下文键（`f4` / `shift+f4` 的 when 用到的两个键就在这里求值）。
fn global_context(key: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| {
        let keys = crate::keys::keys(cx);
        rove_commands::ContextKeys::get(keys.context.as_ref(), key)
            .is_some_and(|v| matches!(v, Value::Bool(true)))
    })
}

#[gpui::test]
fn references_shift_alt_f12_fills_the_panel_view_with_groups_and_counts(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    assert_eq!(panel(&view, cx), (false, "workbench.panel.markers".into()));
    caret(&view, pos(3, 22), cx);
    press("shift-alt-f12", cx);
    // 面板打开，活动视图是「引用」。
    assert_eq!(panel(&view, cx), (true, reg::VIEW.to_string()));
    // 分组与计数来自 #41 的分组索引（文件 → 计数 + 偏移）。
    assert_eq!(
        groups(&view, cx),
        vec![("Enemy.cs".into(), 1), ("Player.cs".into(), 3)]
    );
    assert!(!view.read_with(cx, |v, _| v.references_loading()));
}

#[gpui::test]
fn references_f4_and_shift_f4_walk_the_results_and_open_them(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-alt-f12", cx);
    focus_code(&view, cx);
    press("f4", cx);
    assert_eq!(selected(&view, cx), Some(0));
    let (path, selection) = active_tab(&view, cx);
    assert_eq!(path, "Assets/Enemy.cs", "第一条在 Enemy.cs");
    assert_eq!(selection.unwrap().start().line, Line(4));
    press("f4", cx);
    assert_eq!(selected(&view, cx), Some(1));
    assert_eq!(active_tab(&view, cx).0, "Assets/Player.cs");
    // 首尾循环。
    press("f4", cx);
    press("f4", cx);
    assert_eq!(selected(&view, cx), Some(3));
    press("f4", cx);
    assert_eq!(selected(&view, cx), Some(0));
    press("shift-f4", cx);
    assert_eq!(selected(&view, cx), Some(3));
}

#[gpui::test]
fn references_context_keys_follow_the_result(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    assert!(!global_context(reg::context::HAS_RESULT, cx));
    assert!(!global_context(reg::context::CAN_NAVIGATE, cx));
    caret(&view, pos(3, 22), cx);
    press("shift-alt-f12", cx);
    assert!(global_context(reg::context::HAS_RESULT, cx));
    assert!(global_context(reg::context::CAN_NAVIGATE, cx));
    // 历史列表模式下不能逐条导航。
    run(&view, reg::PICK_FROM_HISTORY, cx);
    assert!(global_context(reg::context::HAS_RESULT, cx));
    assert!(!global_context(reg::context::CAN_NAVIGATE, cx));
    // 清除后两个键都为假。
    run(&view, reg::CLEAR, cx);
    assert!(!global_context(reg::context::HAS_RESULT, cx));
    assert!(!global_context(reg::context::CAN_NAVIGATE, cx));
}

/// `references.preferredLocation` 的两个取值：默认 `peek` 开速览，`view` 在侧边视图展示。
#[gpui::test]
fn preferred_location_decides_where_shift_f12_shows_its_results(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    assert!(
        view.read_with(cx, |v, _| v.peek_widget(v.editor.focused, false).is_some()),
        "默认值 peek：开速览窗口"
    );
    assert_eq!(panel(&view, cx).1, "workbench.panel.markers");

    let f = Fixture::new("{ \"references.preferredLocation\": \"view\" }");
    let (view, cx) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    assert!(
        view.read_with(cx, |v, _| v.peek_widget(v.editor.focused, false).is_none()),
        "view：不开速览"
    );
    assert_eq!(panel(&view, cx), (true, reg::VIEW.to_string()));
    assert_eq!(
        groups(&view, cx),
        vec![("Enemy.cs".into(), 1), ("Player.cs".into(), 3)]
    );
}

/// §7 附录中的 12 条 `references-view.*` 命令逐条可用（都有 action、都能跑完不 panic）。
#[gpui::test]
fn the_twelve_appendix_commands_all_run(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx) = f.open(
        Fake {
            references: move_references(),
            implementations: vec![routed("Assets/Enemy.cs", 4, 10, Precision::Exact)],
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-alt-f12", cx);
    run(&view, reg::NEXT, cx);
    assert_eq!(selected(&view, cx), Some(0));

    // copy / copyAll / copyPath
    run(&view, reg::COPY, cx);
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|i| i.text())),
        Some("Assets/Enemy.cs:5:11".into())
    );
    run(&view, reg::COPY_ALL, cx);
    let all = cx
        .update(|_, cx| cx.read_from_clipboard().and_then(|i| i.text()))
        .unwrap();
    assert_eq!(all.lines().count(), 4);
    run(&view, reg::COPY_PATH, cx);
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|i| i.text())),
        Some("Assets/Enemy.cs".into())
    );

    // removeReferenceItem：消除后分组与计数重建。
    run(&view, reg::REMOVE_REFERENCE_ITEM, cx);
    assert_eq!(groups(&view, cx), vec![("Player.cs".into(), 3)]);

    // refresh / refind：重新运行同一个查询点，结果回到 4 条。
    run(&view, reg::REFRESH, cx);
    assert_eq!(
        groups(&view, cx),
        vec![("Enemy.cs".into(), 1), ("Player.cs".into(), 3)]
    );
    run(&view, reg::REFIND, cx);
    assert_eq!(groups(&view, cx).len(), 2);

    // findImplementations
    focus_code(&view, cx);
    caret(&view, pos(3, 22), cx);
    run(&view, reg::FIND_IMPLEMENTATIONS, cx);
    assert_eq!(groups(&view, cx), vec![("Enemy.cs".into(), 1)]);
    assert_eq!(
        view.read_with(cx, |v, _| v.references_state().entry().map(|e| e.kind)),
        Some(PeekKind::Implementation)
    );

    // tree.focus / tree.resetViewLocation：视图回到面板并显示。
    run(&view, reg::CLEAR, cx);
    run(&view, reg::TREE_RESET_VIEW_LOCATION, cx);
    assert_eq!(panel(&view, cx), (true, reg::VIEW.to_string()));
    run(&view, reg::TREE_FOCUS, cx);
    assert_eq!(panel(&view, cx), (true, reg::VIEW.to_string()));

    // pickFromHistory / clearHistory
    assert!(view.read_with(cx, |v, _| !v.references_state().history.is_empty()));
    run(&view, reg::PICK_FROM_HISTORY, cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.references_state().mode),
        crate::references::Mode::History
    );
    run(&view, reg::CLEAR_HISTORY, cx);
    assert!(view.read_with(cx, |v, _| v.references_state().history.is_empty()));
    assert!(!view.read_with(cx, |v, _| v.references_state().has_result()));
}

/// 层级相关的 6 条只登记：命令表里有、命令面板里没有（没有 action、也没有标记为已实现）。
#[gpui::test]
fn the_six_hierarchy_commands_are_registered_but_hidden(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx) = f.open(Fake::default(), cx);
    for (id, _, _) in reg::HIERARCHY {
        assert!(
            view.read_with(cx, |v, _| v.commands.registry().get(id).is_some()),
            "{id} 应已登记"
        );
        assert!(
            !cx.update(|_, cx| crate::keys::is_implemented(id, cx)),
            "{id} 不应出现在命令面板里"
        );
        // 行为为空：跑一遍不 panic、也不改变任何状态。
        run(&view, id, cx);
        assert!(!view.read_with(cx, |v, _| v.references_state().has_result()));
    }
    // 12 条实现的命令都在命令面板里。
    for (id, _, _) in reg::COMMANDS {
        assert!(
            cx.update(|_, cx| crate::keys::is_implemented(id, cx)),
            "{id} 应可用"
        );
    }
}

/// 4.7 万条结果：视图可用，树里初始只有文件节点 + 第一个文件的结果行（不为 4.7 万条各建一个节点）。
/// 200 个真实文件 × 235 条 = 47 000 条。
#[gpui::test]
fn forty_seven_thousand_results_stay_usable(cx: &mut TestAppContext) {
    const FILES: usize = 200;
    const PER_FILE: usize = 235;
    let f = Fixture::new("{}");
    let line = "class C { void M() { } }\n";
    for i in 0..FILES {
        std::fs::write(
            f.folder.path().join(format!("Assets/f{i:04}.cs")),
            line.repeat(PER_FILE + 5),
        )
        .unwrap();
    }
    let references: Vec<RoutedTarget> = (0..FILES * PER_FILE)
        .map(|i| {
            routed(
                &format!("Assets/f{:04}.cs", i % FILES),
                (i / FILES) as u64,
                0,
                Precision::Exact,
            )
        })
        .collect();
    let (view, cx) = f.open(
        Fake {
            references,
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    let started = Instant::now();
    press("shift-alt-f12", cx);
    let elapsed = started.elapsed();
    let groups = groups(&view, cx);
    assert_eq!(groups.len(), FILES);
    assert_eq!(
        groups.iter().map(|(_, n)| n).sum::<usize>(),
        FILES * PER_FILE
    );
    assert!(global_context(reg::context::CAN_NAVIGATE, cx));
    // 初始只展开第一个文件（> 100 条）：树里是文件节点 + 第一个文件的结果行。
    let rows = view.read_with(cx, |v, cx| v.references_tree_rows(cx));
    assert_eq!(rows, FILES + PER_FILE, "没有把 4.7 万条展平成界面节点");
    // 逐条导航仍然是 O(1)。
    run(&view, reg::NEXT, cx);
    assert_eq!(selected(&view, cx), Some(0));
    // 只防“构建树时展平 4.7 万条”；具体数字见 PR 描述（机器负载较高）。
    assert!(elapsed.as_secs() < 20, "{elapsed:?}");
}

#[gpui::test]
fn an_empty_result_says_so_without_opening_a_tree(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx) = f.open(Fake::default(), cx);
    caret(&view, pos(3, 22), cx);
    press("shift-alt-f12", cx);
    assert_eq!(panel(&view, cx), (true, reg::VIEW.to_string()));
    assert!(groups(&view, cx).is_empty());
    assert!(!global_context(reg::context::HAS_RESULT, cx));
    let hint = view.read_with(cx, |v, _| {
        v.references_state().entry().map(|e| e.empty_hint().key)
    });
    assert_eq!(hint.as_deref(), Some("references.empty"));
}
