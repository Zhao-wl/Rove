//! #40 入口集成测试（真实 GPUI 窗口，键位 / 鼠标驱动）：`F12` → 文档已打开并定位、历史可回退；
//! `ctrl+f12` 在接口 / 虚方法 / 抽象方法上；`partial` 多定义与符号导航模式；`Ctrl+悬停` 与 `Ctrl+点击`；
//! 悬停卡片；语义同名高亮的区间集合与 `F7`；近似呈现的五种场景；空状态；非 C# 文件。
//!
//! 索引用 #38 的测试写出器（`router::testing::TestIndex`）按源码现场生成，读取端是真实的 `CsIndex`。
use super::nav_host::Phase;
use super::*;
use crate::nav::model::HighlightKind;
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use rove_base::{Position, Utf8Column};
use rove_lang_cs::router::testing::{
    find_word, TestIndex, HERO_CS, KIND_FIELD, KIND_METHOD, KIND_TYPE, PLAYER_CS,
};
use rove_lang_cs::router::{hash_bytes, ManualSemantic, SemanticSource, SemanticStatus};
use rove_lang_cs::CsIndex;

const SHAPES_CS: &str = "namespace Game
{
    public interface IShape
    {
        void Draw();
    }

    public class Circle : IShape
    {
        public void Draw() { }
    }

    public class Square : IShape
    {
        public void Draw() { }
    }

    public abstract class Base
    {
        public abstract void Run();
        public virtual void Stop() { }
    }

    public class Impl : Base
    {
        public override void Run() { }
        public override void Stop() { }
    }

    public partial class Big
    {
        void Use(IShape s) { s.Draw(); }
    }
}
";

const BIG2_CS: &str = "namespace Game
{
    public partial class Big
    {
        Big Make() => new Big();
    }
}
";

/// `M0_LANG_SPIKE_REPORT` §3 的五个高频同名名称：两个类各声明一个，第一个类调用自己的两次。
const NAMES: [&str; 5] = ["Init", "Update", "Dispose", "Get", "Value"];
const NAMES_CS: &str = "namespace Game
{
    public class First
    {
        public void Init() { }
        public void Update() { }
        public void Dispose() { }
        public void Get() { }
        public void Value() { }
        void Use()
        {
            Init(); Update(); Dispose(); Get(); Value();
            Init(); Update(); Dispose(); Get(); Value();
        }
    }

    public class Second
    {
        public void Init() { }
        public void Update() { }
        public void Dispose() { }
        public void Get() { }
        public void Value() { }
    }
}
";

const FILES: [(&str, &str); 5] = [
    ("Assets/Big2.cs", BIG2_CS),
    ("Assets/Hero.cs", HERO_CS),
    ("Assets/Names.cs", NAMES_CS),
    ("Assets/Player.cs", PLAYER_CS),
    ("Assets/Shapes.cs", SHAPES_CS),
];

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

fn word(text: &str, w: &str, nth: usize) -> Position {
    let (l, c) = find_word(text, w, nth);
    pos(l as u64, c as u64)
}

/// 示例项目的 L2 索引（文件按路径排序，位置靠搜索单词得到）。
fn build_index() -> TestIndex {
    let mut t = TestIndex::default();
    let f: Vec<u32> = FILES
        .iter()
        .map(|(p, text)| t.file(p, hash_bytes(text.as_bytes())))
        .collect();
    let (big2, hero, names, player, shapes) = (f[0], f[1], f[2], f[3], f[4]);
    let ty = |t: &mut TestIndex, k: &str, n: &str| {
        t.symbol(&format!("Game|T:Game.{k}"), n, KIND_TYPE, None)
    };
    let m = |t: &mut TestIndex, k: &str, n: &str| {
        t.symbol(&format!("Game|M:Game.{k}"), n, KIND_METHOD, None)
    };
    // Hero / Player（与 #38 的示例项目相同）。
    let player_t = ty(&mut t, "Player", "Player");
    let health = t.symbol("Game|F:Game.Player.Health", "Health", KIND_FIELD, None);
    let move_ = m(&mut t, "Player.Move(System.Int32)", "Move");
    let hero_t = ty(&mut t, "Hero", "Hero");
    for (sym, file, text, w, defs, refs) in [
        (player_t, player, PLAYER_CS, "Player", 1, 0),
        (health, player, PLAYER_CS, "Health", 1, 1),
        (move_, player, PLAYER_CS, "Move", 1, 0),
        (hero_t, hero, HERO_CS, "Hero", 1, 0),
        (player_t, hero, HERO_CS, "Player", 0, 1),
        (move_, hero, HERO_CS, "Move", 0, 1),
        (health, hero, HERO_CS, "Health", 0, 1),
    ] {
        occurrences(&mut t, sym, file, text, w, 0, defs, refs);
    }
    t.relations.push((hero_t, player_t, 1));
    t.docs
        .push((player_t, "<summary>A player.</summary>".into()));
    // 接口、抽象方法、虚方法与 partial。
    let ishape = ty(&mut t, "IShape", "IShape");
    let ishape_draw = m(&mut t, "IShape.Draw", "Draw");
    let circle = ty(&mut t, "Circle", "Circle");
    let circle_draw = m(&mut t, "Circle.Draw", "Draw");
    let square = ty(&mut t, "Square", "Square");
    let square_draw = m(&mut t, "Square.Draw", "Draw");
    let base = ty(&mut t, "Base", "Base");
    let base_run = m(&mut t, "Base.Run", "Run");
    let base_stop = m(&mut t, "Base.Stop", "Stop");
    let imp = ty(&mut t, "Impl", "Impl");
    let impl_run = m(&mut t, "Impl.Run", "Run");
    let impl_stop = m(&mut t, "Impl.Stop", "Stop");
    let big = ty(&mut t, "Big", "Big");
    // Shapes.cs 中各单词的出现顺序：IShape ×3（声明、两次基类型），Draw ×4（接口、Circle、Square、调用）。
    occurrences(&mut t, ishape, shapes, SHAPES_CS, "IShape", 0, 1, 3);
    occurrences(&mut t, ishape_draw, shapes, SHAPES_CS, "Draw", 0, 1, 0);
    occurrences(&mut t, circle_draw, shapes, SHAPES_CS, "Draw", 1, 1, 0);
    occurrences(&mut t, square_draw, shapes, SHAPES_CS, "Draw", 2, 1, 0);
    occurrences(&mut t, ishape_draw, shapes, SHAPES_CS, "Draw", 3, 0, 1);
    occurrences(&mut t, circle, shapes, SHAPES_CS, "Circle", 0, 1, 0);
    occurrences(&mut t, square, shapes, SHAPES_CS, "Square", 0, 1, 0);
    occurrences(&mut t, base, shapes, SHAPES_CS, "Base", 0, 1, 1);
    occurrences(&mut t, base_run, shapes, SHAPES_CS, "Run", 0, 1, 0);
    occurrences(&mut t, impl_run, shapes, SHAPES_CS, "Run", 1, 1, 0);
    occurrences(&mut t, base_stop, shapes, SHAPES_CS, "Stop", 0, 1, 0);
    occurrences(&mut t, impl_stop, shapes, SHAPES_CS, "Stop", 1, 1, 0);
    occurrences(&mut t, imp, shapes, SHAPES_CS, "Impl", 0, 1, 0);
    occurrences(&mut t, big, shapes, SHAPES_CS, "Big", 0, 1, 0);
    occurrences(&mut t, big, big2, BIG2_CS, "Big", 0, 1, 2);
    t.relations.extend([
        (circle, ishape, 1),
        (square, ishape, 1),
        (imp, base, 1),
        (circle_draw, ishape_draw, 2),
        (square_draw, ishape_draw, 2),
        (impl_run, base_run, 3),
        (impl_stop, base_stop, 3),
    ]);
    // 五个高频名称：First 的声明 + 两次调用，Second 的声明。
    for n in NAMES {
        let first = m(&mut t, &format!("First.{n}"), n);
        let second = m(&mut t, &format!("Second.{n}"), n);
        occurrences(&mut t, first, names, NAMES_CS, n, 0, 1, 2);
        occurrences(&mut t, second, names, NAMES_CS, n, 3, 1, 0);
    }
    t
}

/// `text` 中单词 `w` 从第 `from` 个起的 `defs` 个定义、`refs` 个引用。
#[allow(clippy::too_many_arguments)]
fn occurrences(
    t: &mut TestIndex,
    sym: u32,
    file: u32,
    text: &str,
    w: &str,
    from: usize,
    defs: usize,
    refs: usize,
) {
    for nth in from..from + defs + refs {
        let (line, col) = find_word(text, w, nth);
        if nth < from + defs {
            t.def(sym, file, line, col);
        } else {
            t.reference(sym, file, line, col);
        }
    }
}

struct Project {
    folder: tempfile::TempDir,
    index: Arc<CsIndex>,
}

impl Project {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir(folder.path().join("Assets")).unwrap();
        for (path, text) in FILES {
            std::fs::write(folder.path().join(path), text).unwrap();
        }
        std::fs::write(folder.path().join("Assets/notes.txt"), "Player notes\n").unwrap();
        let file = folder.path().join("index.idx");
        build_index().write(&file).unwrap();
        let index = Arc::new(CsIndex::open(&file).unwrap());
        Self { folder, index }
    }

    /// 精确的语义索引（L2 就绪）。
    fn exact(&self) -> Arc<dyn SemanticSource> {
        Arc::new(ManualSemantic::ready(self.index.clone()))
    }

    /// 语义索引构建中：查询走 L0 / L1，结果近似。
    fn building() -> Arc<dyn SemanticSource> {
        Arc::new(ManualSemantic::unavailable(SemanticStatus::Building {
            percent: Some(40),
        }))
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

    /// 同一个项目的 L1 快速声明索引（真实解析）。
    fn l1(&self) -> Arc<rove_lang_cs::l1::L1Index> {
        let l1 = Arc::new(rove_lang_cs::l1::L1Index::new(
            rove_lang_cs::l1::L1Options::new(self.folder.path()),
        ));
        l1.build(
            FILES.iter().map(|(p, _)| WorkspacePath::new(p).unwrap()),
            &Cancellation::default(),
        )
        .unwrap();
        l1
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

/// 固定标签（不是预览）打开：再打开别的文件时不会被顶掉。
fn open_pinned(view: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                OpenOptions {
                    preview_tab: false,
                    ..Default::default()
                },
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

fn caret(view: &Entity<Workbench>, p: Position, cx: &mut VisualTestContext) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, cx| {
        c.set_selection(
            code_view::Selection::caret(p),
            Some(Reveal::CenterIfOutside),
            cx,
        )
    });
    cx.run_until_parked();
    refresh(cx);
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    refresh(cx);
    refresh(cx);
}

/// 焦点组的活动文档与插入符。
fn location(view: &Entity<Workbench>, cx: &VisualTestContext) -> (String, Position) {
    view.read_with(cx, |v, cx| {
        let path = match &v.editor.active().current().unwrap().resource {
            Resource::Workspace(p) => p.as_str().to_string(),
            other => format!("{other:?}"),
        };
        (path, v.code_view().read(cx).selection().active)
    })
}

fn status(view: &Entity<Workbench>, cx: &VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| shell::view::status_message_text(v))
}

fn dismiss_status(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    view.update(cx, |v, _| {
        for key in ["nav.status", "nav.symbols", "keybindings.chord"] {
            let _ = crate::feedback::FeedbackService::dismiss(&mut v.shell.feedback, key);
        }
    });
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

/// 焦点组上速览窗口的结果数（没有窗口为 `None`）。#134：多候选时按设置打开速览窗口。
fn peek_total(view: &Entity<Workbench>, cx: &VisualTestContext) -> Option<usize> {
    let widget = view.read_with(cx, |v, _| v.peek_widget(v.editor.focused, false))?;
    widget.read_with(cx, |w, _| w.results().map(|r| r.total()))
}

/// 速览窗口的（标题, 近似标记）。
fn peek_title(view: &Entity<Workbench>, cx: &VisualTestContext) -> (String, Option<String>) {
    let widget = view
        .read_with(cx, |v, _| v.peek_widget(v.editor.focused, false))
        .expect("peek widget");
    widget.read_with(cx, |w, _| w.title_text())
}

fn context(view: &Entity<Workbench>, cx: &VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        editor_actions::context(&v.editor) + &v.nav_context()
    })
}

/// 焦点代码视图上 #40 装饰层的区间：（行, 列区间, 背景色）。
fn highlights(view: &Entity<Workbench>, cx: &VisualTestContext) -> Vec<(u64, u64, HighlightKind)> {
    view.read_with(cx, |v, _| {
        let group = v.editor.focused;
        v.nav_highlights(group, false)
    })
}

#[gpui::test]
fn f12_opens_the_definition_and_history_returns(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/Hero.cs", cx);
    // C# 文件：有定义提供方。
    assert!(context(&view, cx).contains("editorHasDefinitionProvider"));
    assert!(!context(&view, cx).contains("canNavigateBack"));
    let call = word(HERO_CS, "Move", 0);
    caret(&view, call, cx);
    press("f12", cx);
    assert_eq!(
        location(&view, cx),
        ("Assets/Player.cs".into(), word(PLAYER_CS, "Move", 0))
    );
    // 目标行在视口内；精确结果没有任何提示。
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| {
        let vp = c.viewport().unwrap();
        let line = word(PLAYER_CS, "Move", 0).line.0;
        assert!(vp.first_line <= line && line <= vp.last_line);
    });
    assert_eq!(status(&view, cx), None);
    assert_eq!(cursor_hint(&view, cx), None);
    // F12 → 画面完成的样本已记录（查询 + 打开 + 定位 + 一帧）。
    let samples = view.read_with(cx, |v, _| v.nav.samples.clone());
    assert_eq!(samples.len(), 1, "{samples:?}");
    // 导航历史：后退回到调用处，前进再到定义。
    assert!(context(&view, cx).contains("canNavigateBack"));
    press("alt-left", cx);
    assert_eq!(location(&view, cx), ("Assets/Hero.cs".into(), call));
    assert!(context(&view, cx).contains("canNavigateForward"));
    press("alt-right", cx);
    assert_eq!(location(&view, cx).0, "Assets/Player.cs");
    // 目标标签已关闭：后退时重新打开。
    view.update(cx, |v, cx| {
        let group = v.editor.focused;
        let hero = v.editor.groups[&group]
            .tabs
            .iter()
            .find(
                |t| matches!(&t.resource, Resource::Workspace(p) if p.as_str() == "Assets/Hero.cs"),
            )
            .unwrap()
            .id;
        let _ = v.editor.close_tab(group, hero);
        cx.notify();
    });
    refresh(cx);
    press("alt-left", cx);
    assert_eq!(location(&view, cx), ("Assets/Hero.cs".into(), call));
}

/// #132：目标文件已经在另一个标签里打开时，`F12` 同步定位（不再多走一次后台任务），
/// 而且目标帧一帧到位——跳转后画的每一帧都是正式字形，没有等宽占位或空白行。
#[gpui::test]
fn f12_to_an_open_file_paints_the_target_without_placeholders(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    // 两个都用固定标签打开：目标文件已在同组的另一个标签里（#132 的场景）。
    open_pinned(&view, "Assets/Player.cs", cx);
    open_pinned(&view, "Assets/Hero.cs", cx);
    let call = word(HERO_CS, "Move", 0);
    caret(&view, call, cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    // 从这里开始记录每一帧：跳转落位的那一帧也在内。
    code.update(cx, |c, _| {
        c.record_frames(true);
        c.take_frame_samples();
    });
    press("f12", cx);
    let target = word(PLAYER_CS, "Move", 0);
    assert_eq!(location(&view, cx), ("Assets/Player.cs".into(), target));
    // 打开已打开的文件是同步的：这一轮里代码视图始终是同一个实体，帧样本连续。
    let frames = code.update(cx, |c, _| {
        let frames = c.take_frame_samples();
        c.record_frames(false);
        frames
    });
    assert!(!frames.is_empty(), "跳转后至少画了一帧");
    assert!(
        frames.iter().all(|f| f.placeholders == 0 && f.missing == 0),
        "目标帧不能有等宽占位或空白行：{frames:?}"
    );
    assert!(
        frames.iter().any(|f| f.first_line <= target.line.0
            && target.line.0 < f.first_line + u64::from(f.visible_lines)),
        "记录到了目标行所在的帧：{frames:?}"
    );
}

#[gpui::test]
fn f12_beside_crosses_groups_and_history_follows(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/Hero.cs", cx);
    let first = view.read_with(cx, |v, _| v.editor.focused);
    caret(&view, word(HERO_CS, "Player", 0), cx);
    press("ctrl-k f12", cx);
    let (second, path) = view.read_with(cx, |v, _| {
        let resource = v.editor.active().current().unwrap().resource.clone();
        (v.editor.focused, resource)
    });
    assert_ne!(first, second, "在旁边的组打开");
    assert!(matches!(path, Resource::Workspace(p) if p.as_str() == "Assets/Player.cs"));
    focus_code(&view, cx);
    press("alt-left", cx);
    view.read_with(cx, |v, _| assert_eq!(v.editor.focused, first));
    assert_eq!(location(&view, cx).0, "Assets/Hero.cs");
}

#[gpui::test]
fn ctrl_f12_goes_to_implementations(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/Shapes.cs", cx);
    // 抽象方法：唯一的重写，直接跳转。
    caret(&view, word(SHAPES_CS, "Run", 0), cx);
    press("ctrl-f12", cx);
    assert_eq!(location(&view, cx).1, word(SHAPES_CS, "Run", 1));
    // 虚方法。
    caret(&view, word(SHAPES_CS, "Stop", 0), cx);
    press("ctrl-f12", cx);
    assert_eq!(location(&view, cx).1, word(SHAPES_CS, "Stop", 1));
    // 接口方法：两个实现 → 速览窗口（#134），插入符留在原处，不进入符号导航模式。
    caret(&view, word(SHAPES_CS, "Draw", 0), cx);
    press("ctrl-f12", cx);
    let peek = view
        .read_with(cx, |v, _| v.nav.last_peek.clone())
        .expect("peek request");
    assert_eq!(peek.targets.len(), 2);
    assert!(peek.annotation.is_none(), "精确结果的标题没有任何标记");
    assert_eq!(peek_total(&view, cx), Some(2));
    assert_eq!(peek_title(&view, cx), ("Draw — 2 个实现".to_string(), None));
    assert_eq!(location(&view, cx).1, word(SHAPES_CS, "Draw", 0));
    assert!(!context(&view, cx).contains("hasSymbols"));
    // 没有实现：光标处提示。
    press("escape", cx);
    assert_eq!(peek_total(&view, cx), None);
    caret(&view, word(SHAPES_CS, "Draw", 1), cx);
    press("ctrl-f12", cx);
    assert_eq!(
        cursor_hint(&view, cx).as_deref(),
        Some("未找到 `Draw` 的实现")
    );
}

#[gpui::test]
fn partial_class_definitions_enter_symbol_navigation(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/Big2.cs", cx);
    let reference = word(BIG2_CS, "Big", 1);
    caret(&view, reference, cx);
    press("f12", cx);
    // multipleDefinitions = peek（默认，#134）：打开速览窗口，不跳转、不进入符号导航模式。
    let peek = view
        .read_with(cx, |v, _| v.nav.last_peek.clone())
        .expect("peek request");
    assert_eq!(peek.targets.len(), 2);
    assert_eq!(peek_total(&view, cx), Some(2));
    assert_eq!(location(&view, cx), ("Assets/Big2.cs".into(), reference));
    assert!(!context(&view, cx).contains("hasSymbols"));
    press("escape", cx);
    assert_eq!(peek_total(&view, cx), None);
    // multipleDefinitions = goto：跳到第一个并进入符号导航模式，不请求速览。
    view.update(cx, |v, _| {
        v.nav.last_peek = None;
        v.nav.settings.multiple_definitions = crate::nav::settings::GotoMode::Goto;
    });
    caret(&view, reference, cx);
    press("f12", cx);
    assert!(view.read_with(cx, |v, _| v.nav.last_peek.is_none()));
    assert_eq!(peek_total(&view, cx), None);
    assert!(context(&view, cx).contains("hasSymbols"));
    let first = location(&view, cx);
    assert_eq!(status(&view, cx).as_deref(), Some("第 1 / 共 2 个符号"));
    // hasSymbols 下 F12 在候选之间循环。
    press("f12", cx);
    let second = location(&view, cx);
    assert_ne!(first, second);
    assert_eq!(status(&view, cx).as_deref(), Some("第 2 / 共 2 个符号"));
    press("f12", cx);
    assert_eq!(location(&view, cx), first);
    assert_eq!(status(&view, cx).as_deref(), Some("第 1 / 共 2 个符号"));
    // Esc 退出模式。
    press("escape", cx);
    assert!(!context(&view, cx).contains("hasSymbols"));
    // multipleDefinitions = gotoAndPeek：先跳到第一个候选，再在那里打开速览。
    view.update(cx, |v, _| {
        v.nav.last_peek = None;
        v.nav.settings.multiple_definitions = crate::nav::settings::GotoMode::GotoAndPeek;
    });
    open_file(&view, "Assets/Big2.cs", cx);
    caret(&view, reference, cx);
    press("f12", cx);
    assert!(view.read_with(cx, |v, _| v.nav.last_peek.is_some()));
    let landed = location(&view, cx);
    assert_ne!(landed, ("Assets/Big2.cs".into(), reference));
    assert_eq!(peek_total(&view, cx), Some(2));
    assert!(!context(&view, cx).contains("hasSymbols"));
}

/// 鼠标移到某个位置所在单词的中间（窗口坐标）。
fn point_at(
    view: &Entity<Workbench>,
    p: Position,
    cx: &mut VisualTestContext,
) -> gpui::Point<gpui::Pixels> {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, _| {
        let a = c.point_for_position(p).unwrap();
        let b = c.point_for_position(pos(p.line.0, p.column.0 + 1)).unwrap();
        gpui::point(a.x + (b.x - a.x) * 0.5, a.y + px(c.line_height() / 2.0))
    })
}

/// 某个空行上（不在任何单词上）的一点。
fn blank_point(
    view: &Entity<Workbench>,
    line: u64,
    cx: &mut VisualTestContext,
) -> gpui::Point<gpui::Pixels> {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, _| {
        let a = c.point_for_position(pos(line, 0)).unwrap();
        gpui::point(a.x + px(40.0), a.y + px(c.line_height() / 2.0))
    })
}

fn hover_phase(view: &Entity<Workbench>, cx: &VisualTestContext) -> Phase {
    view.read_with(cx, |v, _| v.nav.hover.phase.clone())
}

#[gpui::test]
fn ctrl_hover_underlines_and_ctrl_click_jumps(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/Hero.cs", cx);
    let target = word(HERO_CS, "Move", 0);
    let at = point_at(&view, target, cx);
    cx.simulate_mouse_move(at, None, Modifiers::control());
    cx.run_until_parked();
    refresh(cx);
    // 下划线（editorLink.activeForeground）落在这个单词上。
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    let link = view.read_with(cx, |v, _| v.nav.colors_link());
    code.read_with(cx, |c, _| {
        let merged = c.merged_decorations(target.line.0).expect("line prepared");
        assert!(merged.runs.iter().any(|r| r.range
            == (target.column.0 as u32..target.column.0 as u32 + 4)
            && r.style.underline == link
            && link.is_some()));
    });
    // 定义预览：到达 editor.hover.delay 后出现，内容是定义处的源码。
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    match hover_phase(&view, cx) {
        Phase::Shown(content) => {
            assert!(
                content.preview[0].contains("public void Move(int speed)"),
                "{content:?}"
            );
            assert!(!content.approximate);
        }
        other => panic!("{other:?}"),
    }
    // 松开 Ctrl：下划线与定义预览都收起（指针仍在单词上，普通悬停重新开始等待 editor.hover.delay）。
    cx.simulate_modifiers_change(Modifiers::none());
    cx.run_until_parked();
    refresh(cx);
    assert_eq!(hover_phase(&view, cx), Phase::Waiting);
    assert!(!view.read_with(cx, |v, _| v.nav.hover.definition));
    code.read_with(cx, |c, _| {
        let merged = c.merged_decorations(target.line.0).unwrap();
        assert!(merged.runs.iter().all(|r| r.style.underline.is_none()));
    });
    // Ctrl+点击跳转。
    cx.simulate_click(at, Modifiers::control());
    cx.run_until_parked();
    refresh(cx);
    refresh(cx);
    assert_eq!(
        location(&view, cx),
        ("Assets/Player.cs".into(), word(PLAYER_CS, "Move", 0))
    );
    // editor.definitionLinkOpensInPeek = true：改为速览。
    view.update(cx, |v, _| {
        v.nav.settings.definition_link_opens_in_peek = true
    });
    open_file(&view, "Assets/Hero.cs", cx);
    let at = point_at(&view, target, cx);
    cx.simulate_click(at, Modifiers::control());
    cx.run_until_parked();
    refresh(cx);
    let peek = view
        .read_with(cx, |v, _| v.nav.last_peek.clone())
        .expect("peek");
    assert_eq!(peek.targets.len(), 1);
    // 速览窗口真的打开了（单个结果），原编辑器没有跳转。
    assert_eq!(peek_total(&view, cx), Some(1));
    assert_eq!(location(&view, cx).0, "Assets/Hero.cs");
}

#[gpui::test]
fn hover_card_respects_delay_enabled_and_sticky(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/Hero.cs", cx);
    let player = word(HERO_CS, "Player", 0);
    let at = point_at(&view, player, cx);
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.run_until_parked();
    // editor.hover.delay（300 ms）之前没有卡片。
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(hover_phase(&view, cx), Phase::Waiting);
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    refresh(cx);
    let content = match hover_phase(&view, cx) {
        Phase::Shown(c) => c,
        other => panic!("{other:?}"),
    };
    assert_eq!(&*content.signature, "Player");
    assert_eq!(content.doc.summary, "A player.");
    // 精确结果：没有任何精确度标记。
    assert!(!content.approximate);
    assert!(context(&view, cx).contains("editorHoverVisible"));
    // sticky（默认）：离开单词后等 hidingDelay 才收起。
    let blank = blank_point(&view, 5, cx);
    cx.simulate_mouse_move(blank, None, Modifiers::none());
    cx.run_until_parked();
    assert!(matches!(hover_phase(&view, cx), Phase::Shown(_)));
    cx.executor().advance_clock(Duration::from_millis(350));
    cx.run_until_parked();
    assert_eq!(hover_phase(&view, cx), Phase::Hidden);
    // sticky = false：立即收起。
    view.update(cx, |v, _| v.nav.settings.hover_sticky = false);
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.executor().advance_clock(Duration::from_millis(350));
    cx.run_until_parked();
    assert!(matches!(hover_phase(&view, cx), Phase::Shown(_)));
    cx.simulate_mouse_move(blank, None, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(hover_phase(&view, cx), Phase::Hidden);
    // enabled = off：鼠标悬停不出卡片，ctrl+k ctrl+i 仍然可以。
    view.update(cx, |v, _| {
        v.nav.settings.hover_enabled = crate::nav::settings::HoverEnabled::Off
    });
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(hover_phase(&view, cx), Phase::Hidden);
    caret(&view, player, cx);
    press("ctrl-k ctrl-i", cx);
    assert!(matches!(hover_phase(&view, cx), Phase::Shown(_)));
    // 第二次把焦点移进卡片：editorHoverFocused 下的方向键滚动卡片。
    press("ctrl-k ctrl-i", cx);
    cx.update(|window, cx| {
        let focused = view.read(cx).nav.hover.focus.clone().unwrap();
        assert!(focused.is_focused(window));
    });
    press("escape", cx);
    assert_eq!(hover_phase(&view, cx), Phase::Hidden);
}

#[gpui::test]
fn semantic_occurrences_count_only_the_same_symbol(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/Names.cs", cx);
    for name in NAMES {
        // 在 First 的第一次调用处：同一个符号 3 处（声明 + 两次调用），按名称命中是 4 处。
        caret(&view, word(NAMES_CS, name, 1), cx);
        let h = highlights(&view, cx);
        assert_eq!(h.len(), 3, "{name}: {h:?}");
        let text = NAMES_CS.matches(name).count();
        assert_eq!(text, 4, "{name}");
        // 声明处按“写”，调用处按“读”。
        assert_eq!(
            h.iter()
                .filter(|(_, _, k)| *k == HighlightKind::Write)
                .count(),
            1
        );
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        code.read_with(cx, |c, _| {
            assert_eq!(c.semantic_word_highlights(), Some(true));
            assert!(c.has_word_highlights());
        });
        assert!(context_has_word_highlights(&view, cx));
    }
    // F7 / Shift+F7 在同一符号的出现之间循环。
    let decl = word(NAMES_CS, "Init", 0);
    let call1 = word(NAMES_CS, "Init", 1);
    let call2 = word(NAMES_CS, "Init", 2);
    caret(&view, call1, cx);
    press("f7", cx);
    assert_eq!(location(&view, cx).1, call2);
    press("f7", cx);
    assert_eq!(location(&view, cx).1, decl);
    press("shift-f7", cx);
    assert_eq!(location(&view, cx).1, call2);
    // 光标移到非单词处：没有高亮，hasWordHighlights 为假。
    caret(&view, pos(0, 0), cx);
    caret(&view, pos(1, 0), cx);
    assert!(highlights(&view, cx).is_empty());
    assert!(!context_has_word_highlights(&view, cx));
}

fn context_has_word_highlights(view: &Entity<Workbench>, cx: &VisualTestContext) -> bool {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| c.has_word_highlights())
}

#[gpui::test]
fn approximate_results_follow_ux_spec(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(Project::building(), cx);
    let l1 = project.l1();
    view.update(cx, |v, _| v.attach_l1(Some(l1)));
    open_file(&view, "Assets/Shapes.cs", cx);
    // ① 单个候选：直接跳转，状态栏“近似结果：语义索引未就绪”。
    caret(&view, word(SHAPES_CS, "Base", 1), cx);
    press("f12", cx);
    assert_eq!(location(&view, cx).1, word(SHAPES_CS, "Base", 0));
    assert_eq!(
        status(&view, cx).as_deref(),
        Some("近似结果：语义索引未就绪")
    );
    // 同一符号的重复近似跳转只提示一次。
    dismiss_status(&view, cx);
    caret(&view, word(SHAPES_CS, "Base", 1), cx);
    press("f12", cx);
    assert_eq!(location(&view, cx).1, word(SHAPES_CS, "Base", 0));
    assert_eq!(status(&view, cx), None);
    // ② 多个候选：速览标题“近似 · N 个候选”。
    caret(&view, word(SHAPES_CS, "Draw", 3), cx);
    press("f12", cx);
    let peek = view
        .read_with(cx, |v, _| v.nav.last_peek.clone())
        .expect("peek");
    let (key, args) = peek.annotation.clone().expect("approximate annotation");
    let title = view.read_with(cx, |v, _| v.label(key, &args));
    assert_eq!(title, format!("近似 · {} 个候选", peek.targets.len()));
    assert!(peek.targets.len() >= 2);
    // 速览窗口的标题标记就是 #40 传入的文案。
    assert_eq!(peek_title(&view, cx).1, Some(title));
    press("escape", cx);
    // ③ 查找引用的速览标题“近似”（文案由 #40 传给 #41）。
    let (key, args) = crate::nav::model::references_annotation(true).unwrap();
    assert_eq!(view.read_with(cx, |v, _| v.label(key, &args)), "近似");
    assert!(crate::nav::model::references_annotation(false).is_none());
    // ④ 悬停：底部“近似结果”。
    open_file(&view, "Assets/Player.cs", cx);
    let local = word(PLAYER_CS, "step", 1);
    caret(&view, local, cx);
    press("ctrl-k ctrl-i", cx);
    match hover_phase(&view, cx) {
        Phase::Shown(c) => assert!(c.approximate, "{c:?}"),
        other => panic!("{other:?}"),
    }
    // ⑤ 同名高亮：退化为文本高亮（#20 的文本层），没有任何提示。
    dismiss_status(&view, cx);
    caret(&view, word(PLAYER_CS, "Health", 0), cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| {
        assert_eq!(c.semantic_word_highlights(), None);
        assert!(c.has_word_highlights());
    });
    assert!(highlights(&view, cx).is_empty());
    assert_eq!(status(&view, cx), None);
    assert_eq!(cursor_hint(&view, cx), None);
}

#[gpui::test]
fn empty_state_hints_at_the_cursor(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/Shapes.cs", cx);
    caret(&view, word(SHAPES_CS, "void", 0), cx);
    press("f12", cx);
    assert_eq!(
        cursor_hint(&view, cx).as_deref(),
        Some("未找到 `void` 的定义")
    );
    // 近似时追加“（语义索引构建中）”。
    view.update(cx, |v, cx| v.attach_semantic(Some(Project::building()), cx));
    caret(&view, word(SHAPES_CS, "void", 0), cx);
    press("f12", cx);
    assert_eq!(
        cursor_hint(&view, cx).as_deref(),
        Some("未找到 `void` 的定义（语义索引构建中）")
    );
}

#[gpui::test]
fn non_csharp_files_have_no_definition_provider(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/notes.txt", cx);
    let ctx = context(&view, cx);
    for key in crate::nav::registry::context::PROVIDERS {
        assert!(!ctx.contains(key), "{key}: {ctx}");
    }
    caret(&view, pos(0, 1), cx);
    press("f12", cx);
    // 不弹错误、不显示空速览、不跳转。
    assert_eq!(cursor_hint(&view, cx), None);
    assert_eq!(status(&view, cx), None);
    assert!(view.read_with(cx, |v, _| v.nav.last_peek.is_none()));
    assert_eq!(location(&view, cx).0, "Assets/notes.txt");
    // 命令面板 / 命令直接执行同样什么都不做。
    view.update_in(cx, |v, window, cx| {
        v.nav_command(crate::nav::registry::REVEAL_DEFINITION, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(cursor_hint(&view, cx), None);
    // 文本同名高亮仍是 #20 的实现。
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| assert_eq!(c.semantic_word_highlights(), None));
}

#[gpui::test]
fn open_buffers_are_registered_with_the_router(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(project.exact(), cx);
    open_file(&view, "Assets/Hero.cs", cx);
    refresh(cx);
    let state = view
        .read_with(cx, |v, _| v.cs_buffers().get("Assets/Hero.cs"))
        .expect("registered");
    assert_eq!(state.base_hash, hash_bytes(HERO_CS.as_bytes()));
    // 关闭最后一个视图：注销。
    view.update(cx, |v, cx| {
        let group = v.editor.focused;
        let tab = v.editor.active().current().unwrap().id;
        let _ = v.editor.close_tab(group, tab);
        cx.notify();
    });
    refresh(cx);
    assert!(view
        .read_with(cx, |v, _| v.cs_buffers().get("Assets/Hero.cs"))
        .is_none());
}
