//! #42 入口集成测试（真实 GPUI 窗口，键位驱动）：外部符号 `F12` → 元数据文档已打开、定位到成员、只读、标签复用；
//! 泛型 / 嵌套类型；找不到元数据的提示；`showDocs` 热切换；面包屑与 `Ctrl+Shift+O`；视图内的 `F12`（落到源码与
//! 另一个元数据视图）、`Alt+F12`、`Shift+F12`、`Alt+←`；速览预览里的外部符号片段。
//!
//! 索引用 #38 的测试写出器（`router::testing::metadata_sample_index`）现场生成，读取端是真实的 `CsIndex`。
//! 运行：`cargo test -p rove-ui metadata_`。
use super::*;
use crate::documents::OpenOptions;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Utf8Column};
use rove_lang_cs::metadata_view::{view_key, view_resource};
use rove_lang_cs::router::testing::{
    find_word, metadata_sample_index, ADD_KEY, ENUMERATOR_KEY, GAME_CS, GAME_PATH, LIST_KEY,
    PARENT_KEY, POSITION_KEY, TRANSFORM_KEY, TRANSFORM_VIEW, VECTOR3_KEY,
};
use rove_lang_cs::router::{ManualSemantic, SemanticSource, SemanticStatus};
use rove_lang_cs::CsIndex;

struct Project {
    folder: tempfile::TempDir,
    index: Arc<CsIndex>,
}

impl Project {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir(folder.path().join("Assets")).unwrap();
        std::fs::write(folder.path().join(GAME_PATH), GAME_CS).unwrap();
        let file = folder.path().join("index.idx");
        metadata_sample_index(GAME_CS).0.write(&file).unwrap();
        let index = Arc::new(CsIndex::open(&file).unwrap());
        Self { folder, index }
    }

    fn exact(&self) -> Arc<dyn SemanticSource> {
        Arc::new(ManualSemantic::ready(self.index.clone()))
    }

    fn open<'a>(
        &self,
        semantic: Arc<dyn SemanticSource>,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let root: PathBuf = self.folder.path().into();
        let (view, cx) = cx.add_window_view(|_, cx| Workbench::new(root, Instant::now(), cx));
        cx.run_until_parked();
        view.update(cx, |v, cx| v.attach_semantic(Some(semantic), cx));
        (view, cx)
    }

    /// 源码文件的 L1 声明索引（视图里按名字解析到源码类型用）。
    fn l1(&self) -> Arc<rove_lang_cs::l1::L1Index> {
        let l1 = Arc::new(rove_lang_cs::l1::L1Index::new(
            rove_lang_cs::l1::L1Options::new(self.folder.path()),
        ));
        l1.build(
            [WorkspacePath::new(GAME_PATH).unwrap()],
            &Cancellation::default(),
        )
        .unwrap();
        l1
    }
}

fn pos(line: u64, col: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(col),
    }
}

fn word(text: &str, w: &str, nth: usize) -> Position {
    let (line, col) = find_word(text, w, nth);
    pos(u64::from(line), u64::from(col))
}

/// 视图正文里某个声明片段中名字的位置（避开文档注释里的同名单词）。
fn declared(text: &str, anchor: &str, name: &str) -> Position {
    let at = text.find(anchor).unwrap() + anchor.find(name).unwrap();
    let line = text[..at].matches('\n').count() as u64;
    let col = text[..at].rfind('\n').map_or(at, |n| at - n - 1) as u64;
    pos(line, col)
}

fn refresh(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        refresh(cx);
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
    settle(cx);
    focus_code(view, cx);
}

fn focus_code(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    refresh(cx);
}

fn caret(view: &Entity<Workbench>, p: Position, cx: &mut VisualTestContext) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, cx| {
        c.set_selection(
            code_view::Selection::caret(p),
            Some(code_view::Reveal::CenterIfOutside),
            cx,
        )
    });
    cx.run_until_parked();
    settle(cx);
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    settle(cx);
}

/// 焦点组活动标签的资源与插入符。
fn location(view: &Entity<Workbench>, cx: &VisualTestContext) -> (Resource, Position) {
    view.read_with(cx, |v, cx| {
        (
            v.editor.active().current().unwrap().resource.clone(),
            v.code_view().read(cx).selection().active,
        )
    })
}

fn tab_count(view: &Entity<Workbench>, cx: &VisualTestContext) -> usize {
    view.read_with(cx, |v, _| {
        v.editor.groups.values().map(|g| g.tabs.len()).sum()
    })
}

fn status(view: &Entity<Workbench>, cx: &VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| shell::view::status_message_text(v))
}

fn cursor_hint(view: &Entity<Workbench>, cx: &VisualTestContext) -> Option<String> {
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

fn active_text(view: &Entity<Workbench>, cx: &VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        let doc = v
            .editor
            .active()
            .current()
            .and_then(Tab::opened)
            .unwrap()
            .document
            .clone();
        let text = &doc.text;
        let end = text.lines().known;
        let mut out = String::new();
        for line in 0..end {
            if let Ok(range) = text.line_range(Line(line)) {
                let _ = text.visit_chunks(range, &mut |s| out.push_str(s));
            }
            out.push('\n');
        }
        out
    })
}

fn breadcrumbs(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    view.update(cx, |v, _| {
        let group = v.editor.focused;
        let Some((_, symbols, segments)) = v.group_segments(group) else {
            return Vec::new();
        };
        segments
            .iter()
            .map(|s| match s {
                crate::outline::model::Segment::File { label, .. } => label.to_string(),
                crate::outline::model::Segment::Symbol(id) => symbols
                    .as_ref()
                    .unwrap()
                    .node(id.to_owned())
                    .unwrap()
                    .symbol
                    .name
                    .to_string(),
            })
            .collect()
    })
}

/// 在源码里对一个外部符号按 `F12`，落到元数据视图。
fn goto_external(view: &Entity<Workbench>, name: &str, nth: usize, cx: &mut VisualTestContext) {
    open_file(view, GAME_PATH, cx);
    caret(view, word(GAME_CS, name, nth), cx);
    press("f12", cx);
}

#[gpui::test]
fn metadata_f12_opens_a_read_only_view_at_the_member(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    goto_external(&view, "position", 0, cx);
    let (resource, caret_at) = location(&view, cx);
    assert_eq!(resource, view_resource(TRANSFORM_KEY));
    // 定位到 `position` 属性的名字上。
    assert_eq!(
        caret_at,
        declared(TRANSFORM_VIEW, "Vector3 position", "position")
    );
    // 标题 `Transform [元数据]`，悬停提示是 DLL 完整路径。
    let (title, tip) = view.read_with(cx, |v, _| {
        (
            v.metadata_tab_title(&view_resource(TRANSFORM_KEY)),
            v.metadata_tab_tooltip(&view_resource(TRANSFORM_KEY)),
        )
    });
    assert_eq!(title.as_deref(), Some("Transform [元数据]"));
    assert_eq!(
        tip.as_deref(),
        Some("C:/Unity/Managed/UnityEngine.CoreModule.dll")
    );
    // 只读：标签带锁形图标，原因是元数据视图。
    view.read_with(cx, |v, cx| {
        assert_eq!(v.active_read_only(cx), vec![ReadOnlyReason::Metadata]);
        assert!(v.tab_read_only(v.editor.active().current().unwrap()));
    });
    // 与普通 `.cs` 一致：C# 语言（语法高亮与主题）；正文只有签名和文档注释。
    view.read_with(cx, |v, _| {
        let opened = v.editor.active().current().and_then(Tab::opened).unwrap();
        assert_eq!(&*opened.document.metadata.language, "csharp");
        assert!(opened.syntax.is_some(), "元数据视图按 C# 语法着色");
    });
    // 尝试输入：光标处提示只读原因，不提“在会话中切换只读模式”（对元数据视图无效）。
    cx.simulate_keystrokes("x");
    let hint = cursor_hint(&view, cx).expect("cursor hint");
    assert!(hint.starts_with("无法在只读编辑器中编辑。"), "{hint}");
    assert!(hint.contains("这是元数据视图"), "{hint}");
    assert!(!hint.contains("在会话中切换只读模式"), "{hint}");
    assert_eq!(active_text(&view, cx).trim_end(), TRANSFORM_VIEW.trim_end());
    // “在此会话中切换只读”无效。
    let action = crate::code_view::commands::action_for(
        "workbench.action.files.toggleActiveEditorReadonlyInSession",
    )
    .unwrap();
    cx.update(|window, cx| window.dispatch_action(action, cx));
    settle(cx);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.active_read_only(cx), vec![ReadOnlyReason::Metadata]);
    });
    // Alt+←：退回源码。
    focus_code(&view, cx);
    press("alt-left", cx);
    assert_eq!(
        location(&view, cx).0,
        Resource::Workspace(WorkspacePath::new(GAME_PATH).unwrap())
    );
}

#[gpui::test]
fn metadata_header_region_is_expanded_and_collapsible(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    goto_external(&view, "position", 0, cx);
    assert!(active_text(&view, cx).starts_with("#region 程序集 UnityEngine.CoreModule, Version="));
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    // 文件头 `#region … #endregion`（0..=2 行）默认展开；折叠区间由 #20 的 `#region` 标记折叠提供。
    code.read_with(cx, |c, _| {
        assert!(c.folds().is_empty(), "默认展开");
        assert!(
            c.fold_ranges().iter().any(|r| r.start == 0),
            "文件头 #region 可折叠"
        );
    });
}

#[gpui::test]
fn metadata_tab_is_reused_for_the_same_type(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    goto_external(&view, "position", 0, cx);
    let tabs = tab_count(&view, cx);
    let meta = view_resource(TRANSFORM_KEY);
    // 把标签固定：再跳到同一类型的其他成员时只滚动定位，不新开标签，也不变回预览标签。
    view.update(cx, |v, _| {
        let group = v.editor.focused;
        let id = v.editor.active().current().unwrap().id;
        v.editor.pin(group, id, true);
    });
    for (name, nth, anchor, member) in [
        ("parent", 1, "Transform parent", "parent"),
        ("Translate", 0, "void Translate", "Translate"),
        ("position", 1, "Vector3 position", "position"),
    ] {
        open_file(&view, GAME_PATH, cx);
        caret(&view, word(GAME_CS, name, nth), cx);
        press("f12", cx);
        let (resource, at) = location(&view, cx);
        assert_eq!(resource, meta, "{name}");
        assert_eq!(at, declared(TRANSFORM_VIEW, anchor, member), "{name}");
        assert_eq!(tab_count(&view, cx), tabs, "{name}: 始终只有一个元数据标签");
        view.read_with(cx, |v, _| {
            let tab = v.editor.active().current().unwrap();
            assert_eq!(tab.resource, meta);
            assert!(tab.pinned && !tab.preview);
        });
    }
}

#[gpui::test]
fn metadata_generic_and_nested_types(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    // `List<T>.Add`：标题与定位正确。
    goto_external(&view, "Add", 0, cx);
    let (resource, at) = location(&view, cx);
    assert_eq!(resource, view_resource(LIST_KEY));
    assert_eq!(
        at,
        declared(rove_lang_cs::router::testing::LIST_VIEW, "void Add", "Add")
    );
    let title = view.read_with(cx, |v, _| v.metadata_tab_title(&resource));
    assert_eq!(title.as_deref(), Some("List<T> [元数据]"));
    // 嵌套类型：打开外层类型的视图并定位到嵌套类型那一段，不新开标签。
    let tabs = tab_count(&view, cx);
    view.update(cx, |v, cx| {
        let group = v.editor.focused;
        let target = rove_lang_cs::query::Target {
            resource: Resource::Virtual(format!("rove-cs-metadata:{ENUMERATOR_KEY}").into()),
            range: rove_base::Span::new(pos(0, 0), pos(0, 0)).unwrap(),
            selection: rove_base::Span::new(pos(0, 0), pos(0, 0)).unwrap(),
        };
        v.open_metadata_targets(group, "Enumerator", &[target], false, cx);
    });
    cx.run_until_parked();
    settle(cx);
    let (resource, at) = location(&view, cx);
    assert_eq!(resource, view_resource(LIST_KEY));
    assert_eq!(
        at,
        declared(
            rove_lang_cs::router::testing::LIST_VIEW,
            "struct Enumerator",
            "Enumerator"
        )
    );
    assert_eq!(tab_count(&view, cx), tabs);
}

#[gpui::test]
fn metadata_missing_symbol_shows_a_status_hint_and_no_empty_tab(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, GAME_PATH, cx);
    let tabs = tab_count(&view, cx);
    let missing = rove_lang_cs::query::Target {
        resource: Resource::Virtual("rove-cs-metadata:Nowhere|T:Nowhere.Missing".into()),
        range: rove_base::Span::new(pos(0, 0), pos(0, 0)).unwrap(),
        selection: rove_base::Span::new(pos(0, 0), pos(0, 0)).unwrap(),
    };
    view.update(cx, |v, cx| {
        let group = v.editor.focused;
        v.open_metadata_targets(group, "Missing", &[missing.clone()], false, cx);
    });
    cx.run_until_parked();
    settle(cx);
    assert_eq!(
        status(&view, cx).as_deref(),
        Some("未找到 `Missing` 的元数据")
    );
    assert_eq!(tab_count(&view, cx), tabs, "不打开空标签");
    assert_eq!(
        location(&view, cx).0,
        Resource::Workspace(WorkspacePath::new(GAME_PATH).unwrap())
    );
    // 索引没有就绪：同样的提示。
    view.update(cx, |v, cx| {
        v.attach_semantic(
            Some(Arc::new(ManualSemantic::unavailable(
                SemanticStatus::Building { percent: Some(10) },
            ))),
            cx,
        );
        let group = v.editor.focused;
        let known = rove_lang_cs::query::Target {
            resource: Resource::Virtual(format!("rove-cs-metadata:{POSITION_KEY}").into()),
            ..missing
        };
        v.open_metadata_targets(group, "position", &[known], false, cx);
    });
    cx.run_until_parked();
    settle(cx);
    assert_eq!(
        status(&view, cx).as_deref(),
        Some("未找到 `position` 的元数据")
    );
    assert_eq!(tab_count(&view, cx), tabs);
}

#[gpui::test]
fn metadata_show_docs_toggles_without_reopening_the_tab(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    goto_external(&view, "position", 0, cx);
    let tab = view.read_with(cx, |v, _| v.editor.active().current().unwrap().id);
    assert!(active_text(&view, cx).contains("/// <summary>The world space position"));
    let line_with_docs = location(&view, cx).1.line.0;
    // `rove.metadataView.showDocs = false`：文档注释立即隐藏，同一个标签，光标仍在 `position` 上。
    view.update(cx, |v, cx| v.set_show_docs(false, cx));
    settle(cx);
    let hidden = active_text(&view, cx);
    assert!(!hidden.contains("///"), "{hidden}");
    assert!(hidden.contains("public Vector3 position"));
    assert_eq!(
        view.read_with(cx, |v, _| v.editor.active().current().unwrap().id),
        tab
    );
    let at = location(&view, cx).1;
    assert_eq!(at.line.0, line_with_docs - 2);
    let current = hidden.lines().nth(at.line.0 as usize).unwrap().to_owned();
    assert!(current.contains("position"), "{current}");
    // 改回：恢复。
    view.update(cx, |v, cx| v.set_show_docs(true, cx));
    settle(cx);
    assert!(active_text(&view, cx).contains("/// <summary>The world space position"));
    assert_eq!(location(&view, cx).1.line.0, line_with_docs);
    // 隐藏文档时的跳转同样定位到成员（行号按显示坐标换算）。
    view.update(cx, |v, cx| v.set_show_docs(false, cx));
    settle(cx);
    open_file(&view, GAME_PATH, cx);
    caret(&view, word(GAME_CS, "parent", 1), cx);
    press("f12", cx);
    let at = location(&view, cx).1;
    let text = active_text(&view, cx);
    assert!(text
        .lines()
        .nth(at.line.0 as usize)
        .unwrap()
        .contains("parent"));
}

#[gpui::test]
fn metadata_breadcrumbs_and_symbol_navigation(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    goto_external(&view, "position", 0, cx);
    settle(cx);
    // 程序集 › 命名空间 › 类型 › 成员。
    assert_eq!(
        breadcrumbs(&view, cx),
        vec![
            "UnityEngine.CoreModule",
            "UnityEngine",
            "Transform",
            "position"
        ]
    );
    // 程序集这一段没有目录可列（`rove-metadata:` 不是工作区路径）。
    view.update(cx, |v, _| {
        let group = v.editor.focused;
        let (_, _, segments) = v.group_segments(group).unwrap();
        let items = v.picker_items(&segments[0], None);
        assert!(items.is_empty());
    });
    // `Ctrl+Shift+O`：转到文件中的符号，在视图内可用。
    press("ctrl-shift-o", cx);
    cx.simulate_input("parent");
    cx.run_until_parked();
    press("enter", cx);
    assert_eq!(
        location(&view, cx).1,
        declared(TRANSFORM_VIEW, "Transform parent", "parent")
    );
}

#[gpui::test]
fn metadata_navigation_inside_a_view(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    let l1 = project.l1();
    view.update(cx, |v, _| v.attach_l1(Some(l1)));
    goto_external(&view, "position", 0, cx);
    assert_eq!(location(&view, cx).0, view_resource(TRANSFORM_KEY));
    // 视图内有定义 / 引用提供方（规则与 #40 对齐）。
    let ctx = view.read_with(cx, |v, _| {
        crate::editors::actions::context(&v.editor) + &v.nav_context()
    });
    assert!(ctx.contains("editorHasDefinitionProvider"), "{ctx}");
    assert!(ctx.contains("editorHasReferenceProvider"), "{ctx}");
    let tabs = tab_count(&view, cx);
    // F12：签名里的 `Vector3` → 另一个元数据视图。
    caret(
        &view,
        declared(TRANSFORM_VIEW, "Vector3 position", "Vector3"),
        cx,
    );
    press("f12", cx);
    assert_eq!(location(&view, cx).0, view_resource(VECTOR3_KEY));
    assert_eq!(tab_count(&view, cx), tabs + 1);
    // Alt+←：回到 Transform 视图（导航历史），再回到源码。
    press("alt-left", cx);
    assert_eq!(location(&view, cx).0, view_resource(TRANSFORM_KEY));
    // F12：签名里的 `Player`（源码类型）→ 落到源码。
    caret(
        &view,
        declared(TRANSFORM_VIEW, "Player Owner", "Player"),
        cx,
    );
    press("f12", cx);
    let (resource, at) = location(&view, cx);
    assert_eq!(
        resource,
        Resource::Workspace(WorkspacePath::new(GAME_PATH).unwrap())
    );
    assert_eq!(at, word(GAME_CS, "Player", 0));
    press("alt-left", cx);
    assert_eq!(location(&view, cx).0, view_resource(TRANSFORM_KEY));
}

#[gpui::test]
fn metadata_peek_and_references_inside_a_view(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    goto_external(&view, "position", 0, cx);
    let group = view.read_with(cx, |v, _| v.editor.focused);
    // Shift+F12：列出工作区里使用这个 API 的全部位置（速览窗口），条数与源码里同一个符号的引用查询一致。
    caret(
        &view,
        declared(TRANSFORM_VIEW, "Vector3 position", "position"),
        cx,
    );
    press("shift-f12", cx);
    let widget = view
        .read_with(cx, |v, _| v.peek_widget(group, false))
        .expect("references peek");
    let total = widget.read_with(cx, |w, _| w.results().unwrap().total());
    assert_eq!(total, 3, "Game.cs 里 position 的三处使用");
    let files: Vec<String> = widget.read_with(cx, |w, _| {
        w.results()
            .unwrap()
            .files
            .iter()
            .map(|f| f.label.to_string())
            .collect()
    });
    assert_eq!(files, vec!["Game.cs".to_string()]);
    // 与索引里这个符号的引用数（#38 的引用查询的数据来源）一致。
    let symbol = metadata_sample_index(GAME_CS).1.position;
    assert_eq!(project.index.reference_count(symbol), total);
    press("escape", cx);
    assert!(view
        .read_with(cx, |v, _| v.peek_widget(group, false))
        .is_none());
    // Alt+F12：在视图内开速览（定义）。
    caret(
        &view,
        declared(TRANSFORM_VIEW, "Vector3 position", "Vector3"),
        cx,
    );
    press("alt-f12", cx);
    let widget = view
        .read_with(cx, |v, _| v.peek_widget(group, false))
        .expect("definition peek in the view");
    assert_eq!(widget.read_with(cx, |w, _| w.results().unwrap().total()), 1);
}

#[gpui::test]
fn metadata_refreshes_when_the_index_is_replaced(cx: &mut TestAppContext) {
    let project = Project::new();
    let semantic = Arc::new(ManualSemantic::ready(project.index.clone()));
    let (view, cx) = project.open(semantic.clone() as Arc<dyn SemanticSource>, cx);
    goto_external(&view, "position", 0, cx);
    let before = active_text(&view, cx);
    assert!(before.contains("Transform : Component"));
    // 索引原子替换：新的视图内容（多一个属性）显示到同一个标签里，不用重开。
    let other = project.folder.path().join("index2.idx");
    let (mut index, s) = metadata_sample_index(GAME_CS);
    let changed = TRANSFORM_VIEW.replace("Transform : Component", "Transform : Behaviour");
    index
        .views
        .iter_mut()
        .find(|v| v.type_symbol == s.transform)
        .unwrap()
        .text = changed;
    index.write(&other).unwrap();
    let replaced: Arc<dyn rove_lang_cs::router::IndexHolder> =
        Arc::new(CsIndex::open(&other).unwrap());
    semantic.set(
        Some(rove_lang_cs::router::L2Snapshot {
            holder: replaced,
            generation: 2,
            trusted: true,
        }),
        SemanticStatus::Ready,
    );
    let tabs = tab_count(&view, cx);
    settle(cx);
    let after = active_text(&view, cx);
    assert!(after.contains("Transform : Behaviour"), "{after}");
    assert_eq!(tab_count(&view, cx), tabs);
}

#[gpui::test]
fn metadata_peek_preview_shows_the_view_snippet(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    // 登记了元数据提供方后，#41 的外部符号片段不再是保底的一行签名。
    let registry = view.read_with(cx, |v, _| v.peek_external_snippets());
    assert!(!registry.is_fallback());
    let target = rove_lang_cs::query::Target {
        resource: Resource::Virtual(format!("rove-cs-metadata:{POSITION_KEY}").into()),
        range: rove_base::Span::new(pos(0, 0), pos(0, 0)).unwrap(),
        selection: rove_base::Span::new(pos(0, 0), pos(0, 0)).unwrap(),
    };
    let resolved = registry.resolve(&target, None, "x".into(), &Cancellation::default());
    match resolved {
        crate::peek::external::Resolved::Provided(s) => {
            assert_eq!(&*s.language, "csharp");
            assert!(s.text.contains("public Vector3 position"));
            assert_eq!(
                s.focus.unwrap().start(),
                declared(TRANSFORM_VIEW, "Vector3 position", "position")
            );
        }
        other => panic!("{other:?}"),
    }
    let _ = (PARENT_KEY, ADD_KEY, view_key(&view_resource(TRANSFORM_KEY)));
}
