//! #44 M2 集成验收：把场景清单里**可自动化**的接缝固化成测试。
//!
//! 与 M2 各 issue 自己的入口测试的区别：那些验证的是**某一层在某一个状态下**的行为
//! （#38 的路由、#40 的交互、#41 的速览、#42 的元数据视图各有一套）；这里验证的是**状态之间的切换**
//! 与**几层串起来之后**的一致性，也就是 issue #44「风险与提示」点名的两处：
//!
//! 1. **精确度标记的传递链**：#38 打标记 → #40 决定提示 → #41 写进标题 → #42 决定是否打开视图。
//!    任何一环把标记丢了，界面都会「看起来正常」但其实在骗人。这里强制索引处于
//!    **构建中 / 已就绪 / 文件已修改 / 组件未安装**四种状态，逐链路断言。
//! 2. **索引原子替换的那一瞬间**：替换时可能有速览窗口开着、元数据视图开着、同名高亮正显示着。
//!    这里让三者同时开着再换索引，断言它们刷新而不是显示旧数据或崩溃。
//!
//! 另有一组**真实 Unity 项目**上的测试（`m2_real_*`）：用 `ROVE_TEST_UNITY_MEDIUM` 指向的项目和
//! 它的真实语义索引，逐点比较「界面按 `F12` 的落点」与「查询层直接回答的落点」，也就是 issue #44
//! 目标 2 要求的「界面与查询层的结果是否一致」。
//!
//! 运行：
//!
//! ```powershell
//! cargo test -p rove-ui m2_ -j 1 -- --nocapture
//! ```
//!
//! 真实项目组额外需要 `ROVE_TEST_UNITY_MEDIUM` 与该项目的 L2 索引文件
//! （默认 `target/nav-check/medium.idx`，可用 `ROVE_NAV_INDEX_MEDIUM` 指定；
//! 用 `cargo run --release -p rove-lang-cs --example nav_check -- --build` 生成）。
//! 缺任何一项都**跳过而不是失败**，并把原因打到 stderr，所以 CI 不受影响。
use super::*;
use crate::index_status::STATUS_ITEM;
use crate::nav::model::HighlightKind;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Utf8Column};
use rove_lang_cs::lifecycle::{IndexInfo, IndexState};
use rove_lang_cs::metadata_view::view_key;
use rove_lang_cs::query::{CsQueries, NavigationKind, Precision, QueryPoint};
use rove_lang_cs::router::testing::{find_word, metadata_sample_index, GAME_CS, GAME_PATH};
use rove_lang_cs::router::{
    IndexHolder, L2Snapshot, ManualSemantic, SemanticSource, SemanticStatus,
};
use rove_lang_cs::CsIndex;

// --------------------------------------------------------------------- 合成夹具

/// 第二个源码文件：L2 的示例索引里没有它，只有 L1 认得。降级路径（构建中 / 组件未安装）靠它做
/// **跨文件**跳转，而不是在声明处原地打转。
const USE_PATH: &str = "Assets/Use.cs";
const USE_CS: &str = "namespace Game
{
    public class Use
    {
        Player player;
    }
}
";

/// #42 的元数据示例项目：一个源码文件 + 三份外部类型的元数据视图。
/// 选它做接缝夹具，是因为同一份索引里同时有**源码符号**（高亮、速览）与**外部符号**（元数据视图），
/// 一次替换能把三条链路一起验到。
struct Project {
    folder: tempfile::TempDir,
    index: Arc<CsIndex>,
    semantic: Arc<ManualSemantic>,
}

impl Project {
    /// `ready` 为真时语义索引已就绪（精确），否则处于「构建中」（近似）。
    fn new(ready: bool) -> Self {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir(folder.path().join("Assets")).unwrap();
        std::fs::write(folder.path().join(GAME_PATH), GAME_CS).unwrap();
        std::fs::write(folder.path().join(USE_PATH), USE_CS).unwrap();
        let file = folder.path().join("index.idx");
        metadata_sample_index(GAME_CS).0.write(&file).unwrap();
        let index = Arc::new(CsIndex::open(&file).unwrap());
        let semantic = Arc::new(if ready {
            ManualSemantic::ready(index.clone())
        } else {
            ManualSemantic::unavailable(SemanticStatus::Building { percent: Some(40) })
        });
        Self {
            folder,
            index,
            semantic,
        }
    }

    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let root: PathBuf = self.folder.path().into();
        let (view, cx) = cx.add_window_view(|_, cx| Workbench::new(root, Instant::now(), cx));
        cx.run_until_parked();
        let source = self.semantic.clone() as Arc<dyn SemanticSource>;
        view.update(cx, |v, cx| v.attach_semantic(Some(source), cx));
        let l1 = self.l1();
        view.update(cx, |v, _| v.attach_l1(Some(l1)));
        (view, cx)
    }

    /// 语义索引就绪（精确层上线）。
    fn become_ready(&self) {
        self.semantic.set(
            Some(L2Snapshot {
                holder: self.index.clone() as Arc<dyn IndexHolder>,
                generation: 1,
                trusted: true,
            }),
            SemanticStatus::Ready,
        );
    }

    /// 原子替换成新一代索引（内容有差别，便于看出标签里换没换）。
    fn replace(&self, generation: u64) {
        let other = self.folder.path().join(format!("index{generation}.idx"));
        let (mut index, s) = metadata_sample_index(GAME_CS);
        let view = index
            .views
            .iter_mut()
            .find(|v| v.type_symbol == s.transform)
            .unwrap();
        view.text = view
            .text
            .replace("Transform : Component", "Transform : Behaviour");
        index.write(&other).unwrap();
        let replaced = Arc::new(CsIndex::open(&other).unwrap());
        self.semantic.set(
            Some(L2Snapshot {
                holder: replaced as Arc<dyn IndexHolder>,
                generation,
                trusted: true,
            }),
            SemanticStatus::Ready,
        );
    }

    fn l1(&self) -> Arc<rove_lang_cs::l1::L1Index> {
        let l1 = Arc::new(rove_lang_cs::l1::L1Index::new(
            rove_lang_cs::l1::L1Options::new(self.folder.path()),
        ));
        l1.build(
            [
                WorkspacePath::new(GAME_PATH).unwrap(),
                WorkspacePath::new(USE_PATH).unwrap(),
            ],
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

fn source_resource() -> Resource {
    Resource::Workspace(WorkspacePath::new(GAME_PATH).unwrap())
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

/// 状态栏临时消息（第 1 级反馈）。
fn status(view: &Entity<Workbench>, cx: &VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| shell::view::status_message_text(v))
}

fn dismiss_status(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    view.update(cx, |v, _| {
        for key in ["nav.status", "nav.symbols"] {
            let _ = crate::feedback::FeedbackService::dismiss(&mut v.shell.feedback, key);
        }
    });
}

/// 通知（第 4 级反馈）的条数：后台索引工作全程必须是 0（UX_SPEC §6.1 通用规则）。
fn notifications(view: &Entity<Workbench>, cx: &VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.shell.feedback.notifications().len())
}

fn tab_count(view: &Entity<Workbench>, cx: &VisualTestContext) -> usize {
    view.read_with(cx, |v, _| {
        v.editor.groups.values().map(|g| g.tabs.len()).sum()
    })
}

fn active_resource(view: &Entity<Workbench>, cx: &VisualTestContext) -> Resource {
    view.read_with(cx, |v, _| {
        v.editor.active().current().unwrap().resource.clone()
    })
}

fn active_line(view: &Entity<Workbench>, cx: &VisualTestContext) -> u64 {
    view.read_with(cx, |v, cx| v.code_view().read(cx).selection().active.line.0)
}

fn document_version(view: &Entity<Workbench>, cx: &VisualTestContext) -> Version {
    view.read_with(cx, |v, cx| v.code_view().read(cx).version())
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
        let mut out = String::new();
        for line in 0..text.lines().known {
            if let Ok(range) = text.line_range(Line(line)) {
                let _ = text.visit_chunks(range, &mut |s| out.push_str(s));
            }
            out.push('\n');
        }
        out
    })
}

/// #40 装饰层上的语义同名高亮（近似时为空：退化成 #20 的文本高亮）。
fn semantic_highlights(
    view: &Entity<Workbench>,
    cx: &VisualTestContext,
) -> Vec<(u64, u64, HighlightKind)> {
    view.read_with(cx, |v, _| v.nav_highlights(v.editor.focused, false))
}

/// 速览窗口的（近似标记, 结果数）。
fn peek_state(view: &Entity<Workbench>, cx: &VisualTestContext) -> Option<(Option<String>, usize)> {
    let widget = view.read_with(cx, |v, _| v.peek_widget(v.editor.focused, false))?;
    widget.read_with(cx, |w, _| {
        let marker = w.title_text().1;
        w.results().map(|r| (marker, r.total()))
    })
}

/// 查询层（#38 的路由）对某个位置的直接回答，用来和界面比。
fn ask_router(
    view: &Entity<Workbench>,
    cx: &VisualTestContext,
    kind: NavigationKind,
    resource: &Resource,
    at: Position,
    version: Version,
) -> rove_lang_cs::query::QueryResult<Arc<[rove_lang_cs::query::Target]>> {
    let point = QueryPoint {
        resource: resource.clone(),
        position: at,
        version,
    };
    view.read_with(cx, |v, _| {
        v.cs_router()
            .navigate(kind, &point, &Cancellation::default())
    })
    .expect("router answers")
}

// --------------------------------------------------------------- 场景 2 / 17

/// 场景 2（预读完成前后）+ 场景 17（近似呈现）：**近似 → 精确的切换在界面上静默完成**。
///
/// 逐条对照 issue #44 的验收标准「近似 → 精确的切换在界面上静默完成：不闪烁、不重弹提示」。
/// 断言的事件序列是：
///
/// | 阶段 | 状态栏临时消息 | 语义同名高亮 | 通知 | 标签 |
/// |---|---|---|---|---|
/// | 构建中，第一次 `F12` | 「近似结果：语义索引未就绪」 | 空（退化为文本高亮） | 0 | 不变 |
/// | 构建中，同一符号再 `F12` | 无（不重复提示） | 空 | 0 | 不变 |
/// | 切到就绪：没有任何按键 | **无**（不因为索引就绪而弹提示） | 自动补上 | 0 | 不变 |
/// | 就绪后 `F12` | **无**（精确结果没有任何额外标记） | 有 | 0 | 不变 |
#[gpui::test]
fn m2_approximate_to_exact_switches_silently(cx: &mut TestAppContext) {
    let project = Project::new(false);
    let (view, cx) = project.open(cx);
    open_file(&view, USE_PATH, cx);

    // ① 构建中：`F12` 仍然立即有结果（L1 跨文件落到类型声明），状态栏一行近似提示。
    caret(&view, word(USE_CS, "Player", 0), cx);
    press("f12", cx);
    assert_eq!(active_resource(&view, cx), source_resource());
    assert_eq!(
        active_line(&view, cx),
        u64::from(find_word(GAME_CS, "Player", 0).0),
        "构建中也要跳得到"
    );
    assert_eq!(
        status(&view, cx).as_deref(),
        Some("近似结果：语义索引未就绪"),
        "构建中的第一次跳转要提示一次"
    );
    assert_eq!(notifications(&view, cx), 0, "后台索引不弹通知");
    // 同名高亮退化为 #20 的文本高亮：装饰层是空的，没有任何额外提示。
    caret(&view, word(GAME_CS, "position", 0), cx);
    assert!(semantic_highlights(&view, cx).is_empty());
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| {
        assert_eq!(c.semantic_word_highlights(), None);
        assert!(c.has_word_highlights(), "退化为文本高亮，不是没有高亮");
    });

    // ② 同一符号再跳一次：不重复提示。
    dismiss_status(&view, cx);
    open_file(&view, USE_PATH, cx);
    caret(&view, word(USE_CS, "Player", 0), cx);
    press("f12", cx);
    assert_eq!(status(&view, cx), None, "同一符号的近似跳转只提示一次");

    // ③ 预读完成：**界面没有按键**，索引自己从构建中变成就绪。
    dismiss_status(&view, cx);
    open_file(&view, GAME_PATH, cx);
    let tabs = tab_count(&view, cx);
    caret(&view, word(GAME_CS, "position", 0), cx);
    project.become_ready();
    view.update(cx, |v, cx| v.nav_index_changed(cx));
    settle(cx);
    assert_eq!(status(&view, cx), None, "索引就绪本身不弹任何提示");
    assert_eq!(notifications(&view, cx), 0);
    assert_eq!(tab_count(&view, cx), tabs, "静默刷新，不重开标签");
    // 同名高亮静默补上：这一步是「静默刷新」最直接的证据——没有按键，装饰层自己变了。
    let after = semantic_highlights(&view, cx);
    assert!(
        !after.is_empty(),
        "索引就绪后同名高亮应自动变为语义高亮：{after:?}"
    );

    // ④ 就绪后跳转：精确结果没有任何额外标记。
    caret(&view, word(GAME_CS, "position", 0), cx);
    press("f12", cx);
    assert_eq!(status(&view, cx), None, "精确结果不加任何标记");
    assert_eq!(notifications(&view, cx), 0);
    // 落点是外部符号的元数据视图（#42），不是源码。
    let resource = active_resource(&view, cx);
    assert!(
        view_key(&resource).is_some(),
        "position 是外部符号，应落到元数据视图：{resource:?}"
    );
}

/// 场景 2 的另一半：**精确度标记在各层之间不丢**。
/// 同一个位置，索引构建中与就绪时各查一次，逐层比较：
/// 查询层（#38 的 `QueryResult::precision`）→ 速览标题标记（#41）→ 结果数一致。
#[gpui::test]
fn m2_precision_flag_survives_router_and_peek(cx: &mut TestAppContext) {
    let project = Project::new(false);
    let (view, cx) = project.open(cx);
    open_file(&view, GAME_PATH, cx);
    let at = word(GAME_CS, "position", 0);
    let resource = source_resource();

    // ① 构建中：查询层说「近似」，速览标题就要带标记。
    let version = document_version(&view, cx);
    let answer = ask_router(
        &view,
        cx,
        NavigationKind::References,
        &resource,
        at,
        version,
    );
    assert_ne!(answer.precision, Precision::Exact, "构建中必须是近似");
    caret(&view, at, cx);
    press("shift-f12", cx);
    if let Some((marker, _)) = peek_state(&view, cx) {
        assert_eq!(
            marker.as_deref(),
            Some("近似"),
            "查询层标了近似，速览标题就必须标"
        );
        press("escape", cx);
    }

    // ② 就绪：查询层说「精确」，速览标题就**不能**有任何标记，结果数必须一致。
    project.become_ready();
    view.update(cx, |v, cx| v.nav_index_changed(cx));
    settle(cx);
    let version = document_version(&view, cx);
    let answer = ask_router(
        &view,
        cx,
        NavigationKind::References,
        &resource,
        at,
        version,
    );
    assert_eq!(answer.precision, Precision::Exact);
    caret(&view, at, cx);
    press("shift-f12", cx);
    let (marker, total) = peek_state(&view, cx).expect("peek window");
    assert_eq!(marker, None, "精确结果不加任何标记");
    assert_eq!(total, answer.value.len(), "速览的结果数必须与查询层一致");
    assert_eq!(notifications(&view, cx), 0);
}

// ------------------------------------------------------------------- 风险点 2

/// 风险点 2：**索引原子替换的那一瞬间**，速览窗口、元数据视图、同名高亮同时开着。
/// 断言三者都刷新而不是显示旧数据，也不崩溃，标签数不变（不重开、不关掉）。
#[gpui::test]
fn m2_index_replacement_refreshes_peek_metadata_and_highlights(cx: &mut TestAppContext) {
    let project = Project::new(true);
    let (view, cx) = project.open(cx);
    open_file(&view, GAME_PATH, cx);

    // ① 打开元数据视图（外部符号 `position` → `Transform [元数据]`）。
    caret(&view, word(GAME_CS, "position", 0), cx);
    press("f12", cx);
    let metadata = active_resource(&view, cx);
    assert!(view_key(&metadata).is_some(), "{metadata:?}");
    let before = active_text(&view, cx);
    assert!(before.contains("Transform : Component"), "{before}");

    // ② 回到源码，开着速览窗口与同名高亮。
    open_file(&view, GAME_PATH, cx);
    caret(&view, word(GAME_CS, "position", 0), cx);
    press("shift-f12", cx);
    let open_peek = peek_state(&view, cx);
    assert!(open_peek.is_some(), "速览窗口应当开着");
    assert!(
        !semantic_highlights(&view, cx).is_empty(),
        "同名高亮应当正显示着"
    );
    let tabs = tab_count(&view, cx);

    // ③ 就在这个状态下原子替换索引。
    project.replace(2);
    view.update(cx, |v, cx| v.nav_index_changed(cx));
    settle(cx);
    assert_eq!(tab_count(&view, cx), tabs, "替换不改变标签集合");
    assert_eq!(notifications(&view, cx), 0, "替换不弹通知");

    // ④ 元数据标签里换进了新内容（不是旧快照），而且复用同一个标签。
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                metadata.clone(),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    settle(cx);
    let after = active_text(&view, cx);
    assert!(
        after.contains("Transform : Behaviour"),
        "元数据视图必须显示新索引的内容，不能是旧快照：{after}"
    );
    assert_eq!(tab_count(&view, cx), tabs, "同一类型复用同一个标签");

    // ⑤ 速览与同名高亮：没有崩溃，重新查询后仍然精确、结果数不变。
    open_file(&view, GAME_PATH, cx);
    caret(&view, word(GAME_CS, "position", 0), cx);
    press("shift-f12", cx);
    let reopened = peek_state(&view, cx).expect("替换后仍然能开速览");
    assert_eq!(
        Some(reopened.1),
        open_peek.map(|r| r.1),
        "替换前后同一符号的引用数一致"
    );
    assert_eq!(reopened.0, None, "仍然是精确结果");
    assert!(!semantic_highlights(&view, cx).is_empty());
}

// --------------------------------------------------------------------- 场景 7 / 16

/// 场景 7（组件未安装）+ 场景 16（状态栏）：**降级路径三件事同时成立**——
/// 状态栏是 `$(info) C# 基础导航`、不弹通知、导航仍然可用且标为近似。
#[gpui::test]
fn m2_component_missing_degrades_but_navigation_still_works(cx: &mut TestAppContext) {
    let project = Project::new(false);
    let (view, cx) = project.open(cx);
    // 语义组件没有安装：语义来源不可用，L1 仍在。
    project.semantic.set(None, SemanticStatus::NotInstalled);
    view.update(cx, |v, cx| {
        let epoch = v.index.attach(None, None);
        v.index_status_changed(
            epoch,
            IndexStatus {
                state: IndexState::NotInstalled,
                info: IndexInfo::default(),
                revision: 1,
            },
            cx,
        );
        v.nav_index_changed(cx);
    });
    open_file(&view, USE_PATH, cx);

    // ① 状态栏按 UX_SPEC §2.1 的「未安装」一行。
    let entry = view
        .read_with(cx, |v, _| v.shell.status_bar.get(STATUS_ITEM).cloned())
        .expect("status item");
    assert_eq!(entry.text, "$(info) C# 基础导航");
    assert_eq!(
        entry.tooltip.as_deref(),
        Some("跳转和引用是近似结果。安装语义组件后可以获得精确结果。")
    );
    assert_eq!(notifications(&view, cx), 0, "未安装不是失败，不弹通知");

    // ② 导航仍然可用：L1 跨文件解析到类型声明，并标成近似。
    caret(&view, word(USE_CS, "Player", 0), cx);
    press("f12", cx);
    assert_eq!(active_resource(&view, cx), source_resource());
    assert_eq!(
        active_line(&view, cx),
        u64::from(find_word(GAME_CS, "Player", 0).0),
        "未装组件时 F12 仍然落到类型声明"
    );
    assert_eq!(
        status(&view, cx).as_deref(),
        Some("近似结果：语义索引未就绪"),
        "降级结果必须标出来"
    );
    assert_eq!(notifications(&view, cx), 0);
}

// --------------------------------------------------------------------- 场景 6

/// 场景 6：**未保存的修改**。在界面里真的敲字（走 M3 的编辑事务 → #38 的 `BufferStates::edited`），
/// 然后在**被修改的那一行**上查定义：结果回退为近似、界面标出来、该文件进入后台重建队列
/// （`LANGUAGE_INTELLIGENCE` §3.2）。未修改行的位置映射由 #38 与 #41 的入口测试覆盖，这里不重复。
#[gpui::test]
fn m2_unsaved_edit_falls_back_to_approximate_and_requests_a_rebuild(cx: &mut TestAppContext) {
    let project = Project::new(true);
    let (view, cx) = project.open(cx);
    open_file(&view, GAME_PATH, cx);
    let resource = source_resource();
    let at = word(GAME_CS, "Player", 0);

    // 修改前：这一行上的查询是精确的。
    let version = document_version(&view, cx);
    let before = ask_router(
        &view,
        cx,
        NavigationKind::Definition,
        &resource,
        at,
        version,
    );
    assert_eq!(before.precision, Precision::Exact);
    caret(&view, at, cx);
    press("f12", cx);
    assert_eq!(status(&view, cx), None, "精确结果不加任何标记");

    // 在这一行的行首敲两个空格：这一行进入「已修改区域」。
    dismiss_status(&view, cx);
    caret(&view, pos(at.line.0, 0), cx);
    cx.simulate_input("  ");
    settle(cx);
    let edited_version = document_version(&view, cx);
    assert_ne!(edited_version, version, "编辑应当产生新的文档版本");
    let moved = pos(at.line.0, at.column.0 + 2);

    // 被修改行上的查询：回退为近似，原因是「已修改区域」。
    let after = ask_router(
        &view,
        cx,
        NavigationKind::Definition,
        &resource,
        moved,
        edited_version,
    );
    assert_ne!(
        after.precision,
        Precision::Exact,
        "被修改区域内的符号必须回退为近似"
    );
    assert!(
        matches!(
            after.reason,
            Some(rove_lang_cs::query::ApproximationReason::ModifiedRegion)
        ),
        "原因应当是已修改区域：{:?}",
        after.reason
    );

    // 界面必须把这件事标出来（UX_SPEC §2.2：近似结果永远要标明）。
    caret(&view, moved, cx);
    press("f12", cx);
    assert_eq!(
        status(&view, cx).as_deref(),
        Some("近似结果：语义索引未就绪"),
        "未保存修改导致的近似也要在状态栏标出来"
    );

    // 重建请求：**现状是只在保存之后才发**。`LANGUAGE_INTELLIGENCE` §3.2 的流程图写的是
    // 「回退并触发该文件的增量重建」，#38 的实现则在 `router/mod.rs::file_state` 里明确写了
    // 「未保存的编辑不请求——索引进程只读磁盘，重建也得不到缓冲区里的内容」。实现的理由成立，
    // 文档没跟上。本测试按**实现**断言，口径统一走 #213；改了哪一边，这里都会红。
    assert!(
        project.semantic.requested().is_empty(),
        "未保存的编辑不请求重建（与 LANGUAGE_INTELLIGENCE §3.2 的措辞不一致，见 #213）：{:?}",
        project.semantic.requested()
    );

    // 保存之后再查：这时磁盘内容与索引不同，才请求增量重建。
    let path = WorkspacePath::new(GAME_PATH).unwrap();
    view.read_with(cx, |v, _| {
        v.cs_buffers()
            .saved(&path, rove_lang_cs::router::hash_bytes(b"saved"))
            .unwrap()
    });
    let _ = ask_router(
        &view,
        cx,
        NavigationKind::Definition,
        &resource,
        moved,
        edited_version,
    );
    assert!(
        project.semantic.requested().iter().any(|p| p == GAME_PATH),
        "保存后必须请求增量重建：{:?}",
        project.semantic.requested()
    );
    assert_eq!(notifications(&view, cx), 0);
}

// ------------------------------------------------------- 真实 Unity 项目上的一致性

/// 真实项目根目录 + 真实 L2 索引；缺任何一项时返回 `None` 并打印跳过原因。
fn real_medium() -> Option<(PathBuf, PathBuf)> {
    let root = rove_test_support::UnityProject::Medium.path()?;
    let index = match std::env::var("ROVE_NAV_INDEX_MEDIUM") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/nav-check/medium.idx")
        }
    };
    if !index.is_file() {
        eprintln!(
            "已跳过：找不到中型项目的 L2 索引（{}）。先运行 \
             cargo run --release -p rove-lang-cs --example nav_check -- --build，\
             或用 ROVE_NAV_INDEX_MEDIUM 指定。",
            index.display()
        );
        return None;
    }
    Some((root, index))
}

/// issue #44 目标 2 的自动化部分：**界面按 `F12` 的落点必须与查询层的回答一致**。
///
/// 在真实中型 Unity 项目上取一个真实源码文件，把索引里记下的若干出现位置当成光标位置，
/// 先问查询层（`CsRouter`），再在真实窗口里按 `F12`，逐点比较落点。手工抽查里
/// 「`F12` 是否落到正确的行」靠人眼，这里把它变成可重复的断言；悬停签名与近似标记的**观感**
/// 仍然是人工事项（见 `docs/M2_ACCEPTANCE_REPORT.md` §7）。
#[gpui::test]
fn m2_real_project_ui_matches_the_query_layer(cx: &mut TestAppContext) {
    let Some((root, index_path)) = real_medium() else {
        return;
    };
    let index = Arc::new(CsIndex::open(&index_path).expect("open index"));
    let Some((path, points)) = sample_positions(&index, &root) else {
        eprintln!("已跳过：索引里没有可用的源码位置（索引可能比项目旧）。");
        return;
    };
    let semantic = Arc::new(ManualSemantic::ready(index)) as Arc<dyn SemanticSource>;
    let (view, cx) = cx.add_window_view(|_, cx| Workbench::new(root, Instant::now(), cx));
    cx.run_until_parked();
    view.update(cx, |v, cx| v.attach_semantic(Some(semantic), cx));
    settle(cx);
    open_file(&view, &path, cx);
    let resource = Resource::Workspace(WorkspacePath::new(&path).unwrap());
    let version = document_version(&view, cx);

    let mut checked = 0;
    for at in points {
        let answer = ask_router(
            &view,
            cx,
            NavigationKind::Definition,
            &resource,
            at,
            version,
        );
        // 只比较「查询层给出唯一一个源码落点」的位置：多候选会开速览，不是一次跳转。
        if answer.value.len() != 1 || answer.precision != Precision::Exact {
            continue;
        }
        let target = &answer.value[0];
        let Resource::Workspace(target_path) = &target.resource else {
            continue;
        };
        let expected = (
            target_path.as_str().to_owned(),
            target.selection.start().line.0,
        );
        caret(&view, at, cx);
        press("f12", cx);
        let actual = match active_resource(&view, cx) {
            Resource::Workspace(p) => (p.as_str().to_owned(), active_line(&view, cx)),
            other => panic!("F12 落到了非工作区资源：{other:?}"),
        };
        assert_eq!(actual, expected, "界面与查询层的落点不一致（位置 {at:?}）");
        checked += 1;
        if checked >= 20 {
            break;
        }
        // 回到原文件继续下一个点。
        open_file(&view, &path, cx);
    }
    eprintln!("真实中型项目 {path}：比对了 {checked} 个 F12 落点，界面与查询层一致");
    assert!(checked >= 5, "可比对的位置太少（{checked}）");
}

/// 从真实索引里挑一个 `Assets/` 下的小源码文件和其中若干个出现位置。
fn sample_positions(index: &CsIndex, root: &std::path::Path) -> Option<(String, Vec<Position>)> {
    for file in 0..index.files().len() as u32 {
        let Some(path) = index.file_path(file) else {
            continue;
        };
        if !path.starts_with("Assets/") {
            continue;
        }
        let on_disk = root.join(path);
        let Ok(meta) = std::fs::metadata(&on_disk) else {
            continue;
        };
        if meta.len() == 0 || meta.len() > (200 << 10) {
            continue;
        }
        // 索引比磁盘旧时位置会错，所以只取行数足够、且文件没被截短的样本。
        let Ok(text) = std::fs::read_to_string(&on_disk) else {
            continue;
        };
        let lines = text.lines().count() as u64;
        let range = index.file_occurrences(file);
        let points: Vec<Position> = range
            .clone()
            .filter_map(|i| index.occurrence(i))
            .filter(|o| u64::from(o.line) < lines)
            .map(|o| pos(u64::from(o.line), o.col.0))
            .take(40)
            .collect();
        if points.len() >= 10 {
            return Some((path.to_owned(), points));
        }
    }
    None
}
