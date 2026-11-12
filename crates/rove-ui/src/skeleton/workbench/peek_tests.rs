//! #41 入口集成测试（真实 GPUI 窗口，只通过键位驱动）：`Shift+F12` / `Alt+F12` / `Ctrl+Shift+F12`
//! 三个入口；分组与计数、展开规则（100 / 101）；`↑↓`、`Enter`、`Ctrl+Enter`、`F4` / `Shift+F4`、
//! `Ctrl+K F2`、`Esc` 与 `editor.stablePeek`；`editor.peekWidgetDefaultFocus`；嵌套速览逐层返回；
//! 近似标记；外部符号的保底片段与注册提供方；空结果与实现不可用的光标处提示；`.md` 打开源码；
//! 加载提示；反复开关释放结果与预览。
//!
//! 运行：`cargo test -p rove-ui peek_`。查询数据来自替身（[`Fake`]），C# 文件是真实的临时工作区文件。
use super::*;
use crate::documents::OpenOptions;
use crate::lists::ItemId;
use crate::peek::{model::Node, ExternalSnippet, ExternalSnippetProvider, PeekQueries, PeekWidget};
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Span, Utf8Column};
use rove_config::{SettingsOptions, SettingsService};
use rove_lang_cs::query::{Hover, NavigationKind, Precision, QueryPoint, QueryResult, Target};
use rove_lang_cs::router::{RoutedTarget, TargetRole};
use std::sync::Mutex;

const PLAYER: &str =
    "class Player\n{\n    public void Move() { }\n    void Update() { Move(); Move(); }\n}\n";
const ENEMY: &str =
    "class Enemy\n{\n    void Chase(Player p)\n    {\n        p.Move();\n    }\n}\n";
const NOTES: &str = "# Notes\n\nCall `Move` every frame.\n";

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

fn routed(path: &str, line: u64, col: u64, len: u64, precision: Precision) -> RoutedTarget {
    let range = Span::new(pos(line, col), pos(line, col + len)).unwrap();
    RoutedTarget {
        target: Target {
            resource: if path.starts_with("rove-cs-metadata:") {
                Resource::Virtual(path.into())
            } else {
                Resource::Workspace(WorkspacePath::new(path).unwrap())
            },
            range: range.clone(),
            selection: range,
        },
        precision,
        reason: None,
        role: TargetRole::Reference,
    }
}

/// 查询替身：按种类返回固定结果，记录收到的查询点。
#[derive(Default)]
struct Fake {
    references: Vec<RoutedTarget>,
    definitions: Vec<RoutedTarget>,
    implementations: Vec<RoutedTarget>,
    hover: Option<Hover>,
    implementations_available: bool,
    /// 模拟“查询迟迟没有结果”：返回 `Cancelled`，窗口停在加载状态。
    never_finish: bool,
    points: Mutex<Vec<(NavigationKind, QueryPoint)>>,
}

impl PeekQueries for Fake {
    fn navigate(
        &self,
        kind: NavigationKind,
        point: &QueryPoint,
        _: &Cancellation,
    ) -> CoreResult<QueryResult<Arc<[RoutedTarget]>>> {
        self.points.lock().unwrap().push((kind, point.clone()));
        if self.never_finish {
            return Err(Error::Cancelled);
        }
        let value: Arc<[RoutedTarget]> = match kind {
            NavigationKind::References => self.references.clone().into(),
            NavigationKind::Definition => self.definitions.clone().into(),
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
            value: self.hover.clone(),
            precision: Precision::Exact,
            reason: None,
            version: Some(point.version),
            index_generation: 1,
        })
    }
    fn availability(&self, kind: crate::peek::PeekKind) -> crate::peek::Availability {
        if kind == crate::peek::PeekKind::Implementation && !self.implementations_available {
            crate::peek::Availability::Unavailable
        } else {
            crate::peek::Availability::Available
        }
    }
}

/// `Move` 的三处引用：Player.cs 的声明与两次调用、Enemy.cs 的一次调用。
fn move_references() -> Vec<RoutedTarget> {
    vec![
        routed("Assets/Enemy.cs", 4, 10, 4, Precision::Exact),
        routed("Assets/Player.cs", 2, 16, 4, Precision::Exact),
        routed("Assets/Player.cs", 3, 20, 4, Precision::Exact),
        routed("Assets/Player.cs", 3, 28, 4, Precision::Exact),
    ]
}

struct Fixture {
    user: tempfile::TempDir,
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Fixture {
    fn new(settings_json: &str) -> Self {
        let user = tempfile::tempdir().unwrap();
        // 测试不启动 C# 索引进程。
        let json = if settings_json.trim() == "{}" {
            "{ \"rove.index.enabled\": false }".to_string()
        } else {
            settings_json.replacen('{', "{ \"rove.index.enabled\": false,", 1)
        };
        std::fs::write(user.path().join("settings.json"), json).unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        let folder = tempfile::tempdir().unwrap();
        for (path, text) in [
            ("Assets/Player.cs", PLAYER),
            ("Assets/Enemy.cs", ENEMY),
            ("Docs/notes.md", NOTES),
        ] {
            let full = folder.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        Self {
            user,
            folder,
            service,
        }
    }

    fn file(&self, path: &str, text: &str) {
        let full = self.folder.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }

    fn open<'a>(
        &self,
        fake: Fake,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext, Arc<Fake>) {
        let root: PathBuf = self.folder.path().into();
        let settings: Arc<dyn Settings> = self.service.clone();
        let fake = Arc::new(fake);
        let queries: Arc<dyn PeekQueries> = fake.clone();
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
        (view, cx, fake)
    }

    fn change(&self, settings_json: &str, cx: &mut VisualTestContext) {
        let json = settings_json.replacen('{', "{ \"rove.index.enabled\": false,", 1);
        std::fs::write(self.user.path().join("settings.json"), json).unwrap();
        self.service.reload();
        cx.run_until_parked();
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

fn peek(view: &Entity<Workbench>, cx: &VisualTestContext) -> Option<Entity<PeekWidget>> {
    view.read_with(cx, |v, _| v.peek_widget(v.editor.focused, false))
}

fn widget(view: &Entity<Workbench>, cx: &VisualTestContext) -> Entity<PeekWidget> {
    peek(view, cx).expect("速览窗口应已打开")
}

fn cursor_hint(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| {
        let hint = v.shell.feedback.cursor_hint()?;
        let args: Vec<(String, String)> = hint
            .message
            .args
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let args: Vec<(&str, String)> = args.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        Some(v.label(&hint.message.key, &args))
    })
}

/// 当前焦点路径上的上下文键（与键位服务求值 when 的方式相同）。
fn context_has(key: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let stack = window.context_stack();
        let keys = crate::keys::keys(cx);
        let ctx = crate::keys::FocusContext {
            stack: &stack,
            base: keys.context.as_ref(),
        };
        rove_commands::ContextKeys::get(&ctx, key)
            .is_some_and(|v| !matches!(v, Value::Bool(false) | Value::Null))
    })
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

fn list_focused(w: &Entity<PeekWidget>, cx: &mut VisualTestContext) -> bool {
    let list = w.read_with(cx, |w, _| w.list().cloned()).expect("结果树");
    cx.update(|window, cx| list.focus_handle(cx).contains_focused(window, cx))
}

fn preview_focused(w: &Entity<PeekWidget>, cx: &mut VisualTestContext) -> bool {
    let preview = w.read_with(cx, |w, _| w.preview().clone());
    cx.update(|window, cx| preview.focus_handle(cx).contains_focused(window, cx))
}

#[gpui::test]
fn peek_shift_f12_shows_grouped_references_and_enter_opens_the_location(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, fake) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    let w = widget(&view, cx);
    // 查询点：当前文档、插入符位置。
    let (kind, point) = fake.points.lock().unwrap()[0].clone();
    assert!(matches!(kind, NavigationKind::References));
    assert_eq!(point.position, pos(3, 22));
    // 分组与计数：两个文件（Enemy 1、Player 3），总数 4，全部展开（≤ 100）。
    let (files, counts, total) = w.read_with(cx, |w, _| {
        let r = w.results().unwrap();
        (
            r.files
                .iter()
                .map(|f| f.label.to_string())
                .collect::<Vec<_>>(),
            r.files.iter().map(|f| f.count).collect::<Vec<_>>(),
            r.total(),
        )
    });
    assert_eq!(files, ["Enemy.cs", "Player.cs"]);
    assert_eq!(counts, [1, 3]);
    assert_eq!(total, 4);
    assert_eq!(
        w.read_with(cx, |w, _| w.title_text()),
        ("Move — 4 个引用".to_string(), None)
    );
    let rows = w.read_with(cx, |w, cx| w.list().unwrap().read(cx).widget.len());
    assert_eq!(rows, 2 + 4);
    // 初始选中查询点处的那一条（Player.cs 第 4 行第一个 Move），树有焦点（默认 tree）。
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(2));
    assert!(list_focused(&w, cx));
    assert!(context_has("referenceSearchVisible", cx));
    assert!(context_has("listFocus", cx));
    assert!(!context_has("inReferenceSearchEditor", cx));
    // 主编辑器留出了嵌入区域（18 行）。
    let zone = view.read_with(cx, |v, cx| v.code_view().read(cx).doc_zones.get(3));
    let lh = view.read_with(cx, |v, cx| v.code_view().read(cx).line_height());
    assert_eq!(zone, (Workbench::peek_default_lines() * lh).round() as f64);
    // 预览显示当前结果所在文件并选中匹配。
    let (preview_doc, selection) = w.read_with(cx, |w, cx| {
        (w.preview_resource(), w.preview().read(cx).selection())
    });
    assert_eq!(
        preview_doc,
        Some(Resource::Workspace(
            WorkspacePath::new("Assets/Player.cs").unwrap()
        ))
    );
    assert_eq!(
        (selection.start(), selection.end()),
        (pos(3, 20), pos(3, 24))
    );
    // ↑ 回到上一条（Player.cs 的声明），预览跟随。
    press("up", cx);
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(1));
    // ↑↑ 越过 Player.cs 的文件行到 Enemy.cs 的结果，Enter → 在编辑器中打开并定位，窗口关闭。
    press("up up", cx);
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(0));
    press("enter", cx);
    cx.run_until_parked();
    refresh(cx);
    assert!(peek(&view, cx).is_none(), "Enter 后关闭速览");
    let (path, selection) = active_tab(&view, cx);
    assert_eq!(path, "Assets/Enemy.cs");
    assert_eq!(selection, Some(Span::new(pos(4, 10), pos(4, 14)).unwrap()));
    assert_eq!(
        view.read_with(cx, |v, cx| v.code_view().read(cx).doc_zones.get(3)),
        0.0
    );
}

#[gpui::test]
fn peek_expansion_rule_100_expands_all_and_101_only_the_first_file(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let mut lines = String::new();
    for i in 0..60 {
        lines.push_str(&format!("    Move(); // {i}\n"));
    }
    for name in ["A", "B", "C"] {
        f.file(&format!("Assets/{name}.cs"), &lines);
    }
    let make = |total: usize| -> Vec<RoutedTarget> {
        (0..total)
            .map(|i| {
                let file = ["Assets/A.cs", "Assets/B.cs", "Assets/C.cs"][i % 3];
                routed(file, (i / 3) as u64, 4, 4, Precision::Exact)
            })
            .collect()
    };
    for (total, expect_all) in [(100usize, true), (101, false)] {
        let (view, cx, _) = f.open(
            Fake {
                references: make(total),
                ..Default::default()
            },
            cx,
        );
        caret(&view, pos(2, 18), cx);
        press("shift-f12", cx);
        let w = widget(&view, cx);
        let (expanded, counts, rows) = w.read_with(cx, |w, cx| {
            let list = w.list().unwrap().read(cx);
            let tree = list.widget.tree_model().unwrap();
            let expanded: Vec<bool> = (0..3)
                .map(|i| tree.is_expanded(&ItemId(Node::File(i).id())) == Some(true))
                .collect();
            let counts: Vec<usize> = w.results().unwrap().files.iter().map(|f| f.count).collect();
            (expanded, counts, list.widget.len())
        });
        if expect_all {
            assert_eq!(expanded, [true, true, true], "≤ 100：全部展开");
            assert_eq!(rows, 3 + 100);
        } else {
            assert_eq!(expanded, [true, false, false], "> 100：只展开第一个文件");
            assert_eq!(
                counts,
                [34, 34, 33],
                "折叠的文件旁显示数量（数据层一次算出）"
            );
            assert_eq!(rows, 3 + 34);
            // 折叠的文件行由渲染器显示数量：行数据里带着可展开标记与计数来源。
            let row = w.read_with(cx, |w, cx| {
                w.list().unwrap().read(cx).widget.row(1 + 34).unwrap()
            });
            assert_eq!(&*row.label, "B.cs");
            assert!(row.expandable && !row.expanded);
        }
        press("escape", cx);
        assert!(peek(&view, cx).is_none());
    }
}

#[gpui::test]
fn peek_escape_closes_unless_stable_peek(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, _) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    assert!(peek(&view, cx).is_some());
    press("escape", cx);
    assert!(peek(&view, cx).is_none(), "Esc（焦点在结果树）关闭");
    // 焦点在主编辑器里：`editorTextFocus && referenceSearchVisible` 那一条。
    press("shift-f12", cx);
    focus_code(&view, cx);
    assert!(context_has("referenceSearchVisible", cx));
    press("escape", cx);
    assert!(peek(&view, cx).is_none(), "Esc（焦点在主编辑器）关闭");
    // shift+escape 同样关闭。
    press("shift-f12", cx);
    press("shift-escape", cx);
    assert!(peek(&view, cx).is_none());

    f.change(r#"{ "editor.stablePeek": true }"#, cx);
    press("shift-f12", cx);
    let w = widget(&view, cx);
    press("escape", cx);
    assert!(
        peek(&view, cx).is_some(),
        "stablePeek：Esc 不关闭（结果树）"
    );
    press("ctrl-k f2", cx);
    assert!(preview_focused(&w, cx));
    press("escape", cx);
    assert!(peek(&view, cx).is_some(), "stablePeek：Esc 不关闭（预览）");
    w.update(cx, |w, cx| w.double_click_preview(cx));
    refresh(cx);
    assert!(peek(&view, cx).is_some(), "stablePeek：双击预览不关闭");
    assert_eq!(active_tab(&view, cx).0, "Assets/Player.cs");
}

#[gpui::test]
fn peek_f4_navigates_and_ctrl_k_f2_toggles_focus(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, _) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    let w = widget(&view, cx);
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(2));
    press("f4", cx);
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(3));
    press("f4", cx);
    assert_eq!(
        w.read_with(cx, |w, _| w.current()),
        Some(0),
        "末尾循环到开头"
    );
    press("shift-f4", cx);
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(3));
    // 结果树聚焦时 F12 / Shift+F12 也是下一个 / 上一个（KEYBINDINGS 导航一节）。
    press("f12", cx);
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(0));
    press("shift-f12", cx);
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(3));
    // Ctrl+K F2 切到预览：上下文键 inReferenceSearchEditor / isInEmbeddedEditor。
    press("ctrl-k f2", cx);
    assert!(preview_focused(&w, cx));
    assert!(context_has("inReferenceSearchEditor", cx));
    assert!(context_has("isInEmbeddedEditor", cx));
    // 焦点在预览里 F4 仍然导航，焦点留在预览。
    press("f4", cx);
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(0));
    assert!(preview_focused(&w, cx));
    press("ctrl-k f2", cx);
    assert!(list_focused(&w, cx));
    // 焦点在主编辑器里按 F4：作用于这个编辑器上的速览。
    focus_code(&view, cx);
    press("f4", cx);
    assert_eq!(w.read_with(cx, |w, _| w.current()), Some(1));
    // 预览只读：输入不改变内容。
    press("ctrl-k f2", cx);
    let before = w.read_with(cx, |w, cx| w.preview_text(cx));
    cx.simulate_input("xyz");
    refresh(cx);
    assert_eq!(w.read_with(cx, |w, cx| w.preview_text(cx)), before);
    assert!(w.read_with(cx, |w, cx| w.preview().read(cx).is_read_only()));
}

#[gpui::test]
fn peek_default_focus_editor_puts_focus_in_the_preview(cx: &mut TestAppContext) {
    let f = Fixture::new(r#"{ "editor.peekWidgetDefaultFocus": "editor" }"#);
    let (view, cx, _) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    let w = widget(&view, cx);
    assert!(preview_focused(&w, cx));
    assert!(context_has("inReferenceSearchEditor", cx));
}

#[gpui::test]
fn peek_ctrl_enter_opens_to_the_side_and_keeps_the_window(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, _) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    // 行：Enemy.cs、r0、Player.cs、r1、r2（焦点）、r3 → 上移三行到 Enemy.cs 的结果。
    press("up up up", cx);
    press("ctrl-enter", cx);
    cx.run_until_parked();
    refresh(cx);
    let groups = view.read_with(cx, |v, _| v.editor.groups.len());
    assert_eq!(groups, 2, "在侧边的组里打开");
    let first = view.read_with(cx, |v, _| *v.editor.groups.keys().next().unwrap());
    // 焦点留在原来的组（速览仍在），旁边的组显示并定位到结果。
    let (path, selection) = view.read_with(cx, |v, _| {
        let side = v
            .editor
            .groups
            .iter()
            .find(|(g, _)| **g != first)
            .unwrap()
            .1;
        let tab = side.current().unwrap();
        (format!("{:?}", tab.resource), tab.view.selection.clone())
    });
    assert!(path.contains("Assets/Enemy.cs"), "{path}");
    assert_eq!(selection, Some(Span::new(pos(4, 10), pos(4, 14)).unwrap()));
    assert!(
        view.read_with(cx, |v, _| v.peek_widget(first, false))
            .is_some(),
        "旁边打开不关闭速览"
    );
}

#[gpui::test]
fn peek_definition_nested_peek_and_layered_escape(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, fake) = f.open(
        Fake {
            definitions: vec![routed("Assets/Player.cs", 2, 16, 4, Precision::Exact)],
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    // Enemy.cs 里 `p.Move()` 上 Alt+F12。
    open_file(&view, "Assets/Enemy.cs", cx);
    caret(&view, pos(4, 11), cx);
    press("alt-f12", cx);
    let w = widget(&view, cx);
    assert_eq!(w.read_with(cx, |w, _| w.title_text()).0, "Move — 1 个定义");
    // 焦点到预览（定位在 Player.cs 的声明上），预览里再 Alt+F12：嵌套一层。
    press("ctrl-k f2", cx);
    assert!(preview_focused(&w, cx));
    assert!(context_has("editorHasDefinitionProvider", cx));
    press("alt-f12", cx);
    assert_eq!(w.read_with(cx, |w, cx| w.depth(cx)), 2);
    let (kind, point) = fake.points.lock().unwrap().last().cloned().unwrap();
    assert!(matches!(kind, NavigationKind::Definition));
    assert_eq!(
        point.resource,
        Resource::Workspace(WorkspacePath::new("Assets/Player.cs").unwrap())
    );
    // 内层的结果树有焦点：上下文键随层级——仍可见、焦点在内层的列表。
    let inner = w.read_with(cx, |w, _| w.nested().cloned()).unwrap();
    assert!(list_focused(&inner, cx));
    assert!(context_has("referenceSearchVisible", cx));
    // Esc 逐层返回：先关内层（焦点回到外层预览），再关外层。
    press("escape", cx);
    assert_eq!(w.read_with(cx, |w, cx| w.depth(cx)), 1);
    assert!(peek(&view, cx).is_some());
    assert!(preview_focused(&w, cx));
    assert!(context_has("inReferenceSearchEditor", cx));
    press("escape", cx);
    assert!(peek(&view, cx).is_none());
    assert!(!context_has("referenceSearchVisible", cx));
}

#[gpui::test]
fn peek_titles_mark_approximate_results_only(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let mut approx = move_references();
    approx[0].precision = Precision::Approximate;
    let (view, cx, _) = f.open(
        Fake {
            references: approx,
            definitions: vec![
                routed("Assets/Player.cs", 2, 16, 4, Precision::Approximate),
                routed("Assets/Enemy.cs", 2, 9, 5, Precision::Approximate),
            ],
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    let w = widget(&view, cx);
    assert_eq!(
        w.read_with(cx, |w, _| w.title_text()),
        ("Move — 4 个引用".to_string(), Some("近似".to_string()))
    );
    press("escape", cx);
    press("alt-f12", cx);
    let w = widget(&view, cx);
    assert_eq!(
        w.read_with(cx, |w, _| w.title_text()),
        (
            "Move — 2 个定义".to_string(),
            Some("近似 · 2 个候选".to_string())
        )
    );
}

struct MetadataView;
impl ExternalSnippetProvider for MetadataView {
    fn snippet(
        &self,
        target: &Target,
        _: Option<&Hover>,
        _: &Cancellation,
    ) -> Option<ExternalSnippet> {
        let key = rove_lang_cs::router::metadata_symbol_key(&target.resource)?;
        Some(ExternalSnippet {
            text: format!("// {key}\npublic class Transform : Component\n{{\n}}\n").into(),
            language: "csharp".into(),
            focus: Some(Span::new(pos(1, 13), pos(1, 22)).unwrap()),
        })
    }
}

#[gpui::test]
fn peek_external_symbol_shows_fallback_then_registered_snippet(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, _) = f.open(
        Fake {
            definitions: vec![routed(
                "rove-cs-metadata:UnityEngine|T:UnityEngine.Transform",
                0,
                0,
                0,
                Precision::Exact,
            )],
            hover: Some(Hover {
                signature: "public class Transform".into(),
                documentation: "".into(),
                range: Span::new(pos(0, 0), pos(0, 1)).unwrap(),
            }),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(0, 7), cx);
    // 保底：#42 未注册时显示 `外部符号：<签名>`。
    assert!(view.read_with(cx, |v, _| v.peek_external_snippets().is_fallback()));
    press("alt-f12", cx);
    let w = widget(&view, cx);
    assert_eq!(
        w.read_with(cx, |w, cx| w.preview_text(cx)).as_deref(),
        Some("外部符号：public class Transform")
    );
    assert_eq!(
        w.read_with(cx, |w, _| w.results().unwrap().files[0].label.to_string()),
        "Transform"
    );
    press("escape", cx);
    // 注册真正的提供方后显示元数据片段并定位。
    view.update(cx, |v, _| {
        v.peek_external_snippets().register(Arc::new(MetadataView))
    });
    press("alt-f12", cx);
    let w = widget(&view, cx);
    let text = w.read_with(cx, |w, cx| w.preview_text(cx)).unwrap();
    assert!(
        text.contains("public class Transform : Component"),
        "{text}"
    );
    let s = w.read_with(cx, |w, cx| w.preview().read(cx).selection());
    assert_eq!((s.start(), s.end()), (pos(1, 13), pos(1, 22)));
}

#[gpui::test]
fn peek_empty_results_and_unavailable_implementations_hint_without_a_window(
    cx: &mut TestAppContext,
) {
    let f = Fixture::new("{}");
    let (view, cx, fake) = f.open(Fake::default(), cx);
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    assert!(peek(&view, cx).is_none(), "空结果不留窗口");
    assert_eq!(
        cursor_hint(&view, cx).as_deref(),
        Some("未找到“Move”的引用")
    );
    // 实现关系不可用：不查询、不开窗口，光标处提示“未找到实现”。
    let queries = fake.points.lock().unwrap().len();
    press("ctrl-shift-f12", cx);
    assert!(peek(&view, cx).is_none());
    assert_eq!(cursor_hint(&view, cx).as_deref(), Some("未找到实现"));
    assert_eq!(fake.points.lock().unwrap().len(), queries);
}

#[gpui::test]
fn peek_implementations_when_available_and_approximate_empty_hint(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let mut empty_approx = Fake {
        implementations: vec![routed("Assets/Enemy.cs", 0, 6, 5, Precision::Exact)],
        implementations_available: true,
        ..Default::default()
    };
    empty_approx.references = vec![];
    let (view, cx, fake) = f.open(empty_approx, cx);
    caret(&view, pos(0, 8), cx);
    press("ctrl-shift-f12", cx);
    let w = widget(&view, cx);
    assert!(matches!(
        fake.points.lock().unwrap()[0].0,
        NavigationKind::Implementations
    ));
    assert_eq!(
        w.read_with(cx, |w, _| w.title_text()).0,
        "Player — 1 个实现"
    );
}

#[gpui::test]
fn peek_reveal_on_markdown_result_opens_the_source(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, _) = f.open(
        Fake {
            references: vec![routed("Docs/notes.md", 2, 6, 4, Precision::Approximate)],
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    press("enter", cx);
    cx.run_until_parked();
    refresh(cx);
    let (path, selection) = active_tab(&view, cx);
    assert_eq!(path, "Docs/notes.md");
    assert_eq!(selection, Some(Span::new(pos(2, 6), pos(2, 10)).unwrap()));
    let kind = view.read_with(cx, |v, _| v.editor.active().current().unwrap().kind.clone());
    assert_eq!(
        kind,
        crate::editors::settings::EditorKind::Text,
        "定位打开的是源码"
    );
}

#[gpui::test]
fn peek_shows_loading_after_300_ms_without_results(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, _) = f.open(
        Fake {
            never_finish: true,
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    let w = widget(&view, cx);
    assert!(
        w.read_with(cx, |w, _| w.is_loading()),
        "窗口立即出现，不等数据"
    );
    assert!(!w.read_with(cx, |w, _| w.loading_visible()));
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert!(!w.read_with(cx, |w, _| w.loading_visible()));
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    assert!(
        w.read_with(cx, |w, _| w.loading_visible()),
        "300 ms 后显示“正在加载…”"
    );
    press("escape", cx);
    assert!(peek(&view, cx).is_none());
}

#[gpui::test]
fn peek_closing_releases_results_and_preview_documents(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, _) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    let mut weak = Vec::new();
    for _ in 0..100 {
        press("shift-f12", cx);
        let w = widget(&view, cx);
        let results = w.read_with(cx, |w, _| Arc::downgrade(w.results().unwrap()));
        let preview = w.read_with(cx, |w, _| w.preview().downgrade());
        let list = w.read_with(cx, |w, _| w.list().unwrap().downgrade());
        weak.push((results, preview, list, w.downgrade()));
        drop(w);
        press("escape", cx);
    }
    cx.run_until_parked();
    for (results, preview, list, widget) in &weak {
        assert!(results.upgrade().is_none(), "结果数据已释放");
        assert!(preview.upgrade().is_none(), "预览已释放");
        assert!(list.upgrade().is_none(), "结果树已释放");
        assert!(widget.upgrade().is_none(), "窗口已释放");
    }
    assert!(
        view.read_with(cx, |v, cx| v.code_view().read(cx).doc_zones.is_empty()),
        "嵌入区域已移除"
    );
}

#[gpui::test]
fn peek_closes_when_the_editor_switches_document(cx: &mut TestAppContext) {
    let f = Fixture::new("{}");
    let (view, cx, _) = f.open(
        Fake {
            references: move_references(),
            ..Default::default()
        },
        cx,
    );
    caret(&view, pos(3, 22), cx);
    press("shift-f12", cx);
    assert!(peek(&view, cx).is_some());
    open_file(&view, "Assets/Enemy.cs", cx);
    assert!(peek(&view, cx).is_none());
}

#[test]
fn peek_keybindings_match_the_navigation_table() {
    // 导航一节里这些命令的每一行（键 + when）都必须原样出现在生效的默认键位表里。
    let doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/compat/KEYBINDINGS.md"),
    )
    .unwrap();
    let section = doc
        .split("### 导航")
        .nth(1)
        .unwrap()
        .split("\n### ")
        .next()
        .unwrap();
    let ids: Vec<&str> = crate::peek::registry::COMMANDS
        .iter()
        .map(|c| c.0)
        .collect();
    let defaults = rove_commands::keybindings::default_rules();
    let mut checked = 0;
    for row in section.lines().filter(|l| l.starts_with("| P")) {
        let cells: Vec<&str> = row.split(" | ").collect();
        let strip = |s: &str| s.trim().trim_matches('`').replace("\\|", "|");
        let (key, command, when) = (
            strip(cells[1]),
            strip(cells[2]),
            strip(cells[3].trim_end_matches('|')),
        );
        if !ids.contains(&command.as_str()) {
            continue;
        }
        checked += 1;
        assert!(
            defaults.iter().any(|d| {
                d.rule.command == command
                    && d.rule.key.as_deref() == Some(key.as_str())
                    && d.rule.when.as_deref().unwrap_or("") == when
            }),
            "{key} {command} {when}"
        );
    }
    assert_eq!(checked, 14, "导航一节里速览相关的 14 行");
}
