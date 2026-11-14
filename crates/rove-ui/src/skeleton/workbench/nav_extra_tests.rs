//! #60 的两个编辑器入口的集成测试（真实 GPUI 窗口）：
//!
//! - **转到类型定义**（`editor.action.goToTypeDefinition`，无默认键位）：单候选直接跳转、多候选开速览、
//!   非 C# 文件上下文键为假且命令什么都不做、无结果时光标处提示。
//! - **转到工作区中的符号**：`Ctrl+T` 与快速打开的 `#` 是同一份数据、同一套排序（逐条断言两者相同），
//!   近似说明只有一行且在顶部，选中后经打开文档服务打开并定位（`.md` 打开源码）。
//!
//! 运行：`cargo test -p rove-ui nav_extra`（文件内的用例名以 `type_definition_` / `workspace_symbols_` 开头）。
use super::*;
use crate::nav::registry as nav_reg;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Span, Utf8Column};
use rove_config::{SettingsOptions, SettingsService};
use rove_lang_cs::query::{
    CsQueries, Hover, NavigationKind, Precision, QueryPoint, QueryResult, SemanticToken, Target,
    WorkspaceSymbol,
};
use rove_lang_cs::router::testing::{
    find_word, TestIndex, HERO_CS, KIND_FIELD, KIND_LOCAL, KIND_PARAMETER, KIND_PROPERTY,
    KIND_TYPE, KIND_TYPE_PARAMETER, PLAYER_CS,
};
use rove_lang_cs::router::{hash_bytes, ManualSemantic, SemanticSource};
use rove_lang_cs::CsIndex;
use rove_syntax::Symbol;

/// 覆盖验收里点名的五种写法：`var` 变量、字段、参数、属性、泛型类型参数。
const WRITES_CS: &str = "namespace Game
{
    public class Box<T>
    {
        public T Item;
    }

    public class Writes
    {
        private Player field;
        public Player Property { get; set; }

        void Use(Player parameter)
        {
            var local = new Player();
            var box = new Box<Player>();
        }

        private int count;
    }
}
";

/// `Box<T>` 的第二个 partial 部分：让类型定义有两个候选。
const WRITES2_CS: &str = "namespace Game
{
    public partial class Box<T>
    {
        public int Count;
    }
}
";

const NOTES_MD: &str = "# Notes\n\nPlayer notes.\n";

const FILES: [(&str, &str); 4] = [
    ("Assets/Hero.cs", HERO_CS),
    ("Assets/Player.cs", PLAYER_CS),
    ("Assets/Writes.cs", WRITES_CS),
    ("Assets/Writes2.cs", WRITES2_CS),
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

/// 索引里有类型、也有用到这些类型的字段 / 属性 / 参数 / 局部变量（#38 的索引不记录声明类型，
/// 类型定义查询靠“到声明处读回写出来的类型名”回答，所以声明处的那几个 `Player` 引用是关键）。
fn build_index() -> TestIndex {
    let mut t = TestIndex::default();
    let f: Vec<u32> = FILES
        .iter()
        .map(|(p, text)| t.file(p, hash_bytes(text.as_bytes())))
        .collect();
    let (hero, player, writes, writes2) = (f[0], f[1], f[2], f[3]);
    let player_t = t.symbol("Game|T:Game.Player", "Player", KIND_TYPE, None);
    let box_t = t.symbol("Game|T:Game.Box`1", "Box", KIND_TYPE, None);
    let item_t = t.symbol("Game|T:Game.Box`1.T", "T", KIND_TYPE_PARAMETER, None);
    let (line, col) = find_word(PLAYER_CS, "Player", 0);
    t.def(player_t, player, line, col);
    let (line, col) = find_word(HERO_CS, "Player", 0);
    t.reference(player_t, hero, line, col);
    // Writes.cs 里 Player 的五次出现（字段、属性、参数、两处 new）与 Box 的两次。
    for nth in 0..5 {
        let (line, col) = find_word(WRITES_CS, "Player", nth);
        t.reference(player_t, writes, line, col);
    }
    for nth in 0..2 {
        let (line, col) = find_word(WRITES_CS, "Box", nth);
        if nth == 0 {
            t.def(box_t, writes, line, col);
        } else {
            t.reference(box_t, writes, line, col);
        }
    }
    // partial 的第二部分：同一个符号的第二个定义处。
    let (line, col) = find_word(WRITES2_CS, "Box", 0);
    t.def(box_t, writes2, line, col);
    // 泛型类型参数 `T`：声明处与 `public T Item;` 里的引用。
    for nth in 0..2 {
        let (line, col) = find_word(WRITES_CS, "T", nth);
        if nth == 0 {
            t.def(item_t, writes, line, col);
        } else {
            t.reference(item_t, writes, line, col);
        }
    }
    // 验收点名的五种写法各一个符号，定义处就是声明处的名字。
    let members = [
        (
            t.symbol("Game|F:Game.Box`1.Item", "Item", KIND_FIELD, None),
            "Item",
        ),
        (
            t.symbol("Game|F:Game.Writes.field", "field", KIND_FIELD, None),
            "field",
        ),
        (
            t.symbol(
                "Game|P:Game.Writes.Property",
                "Property",
                KIND_PROPERTY,
                None,
            ),
            "Property",
        ),
        (
            t.symbol("Game|F:Game.Writes.count", "count", KIND_FIELD, None),
            "count",
        ),
        (t.local("parameter", KIND_PARAMETER, writes, 0), "parameter"),
        (t.local("local", KIND_LOCAL, writes, 1), "local"),
        (t.local("box", KIND_LOCAL, writes, 2), "box"),
    ];
    for (symbol, name) in members {
        let (line, col) = find_word(WRITES_CS, name, 0);
        t.def(symbol, writes, line, col);
    }
    t
}

/// 工作区符号查询替身（#1 契约的 `CsQueries`，只实现工作区符号这一项）。
struct FakeSymbols {
    symbols: Vec<WorkspaceSymbol>,
    precision: Precision,
}

fn symbol(name: &str, container: &str, path: &str, line: u64, col: u64) -> WorkspaceSymbol {
    let span = Span::new(pos(line, col), pos(line, col + name.len() as u64)).unwrap();
    WorkspaceSymbol {
        name: name.into(),
        container: container.into(),
        target: Target {
            resource: Resource::Workspace(WorkspacePath::new(path).unwrap()),
            range: span.clone(),
            selection: span,
        },
    }
}

impl CsQueries for FakeSymbols {
    fn navigate(
        &self,
        _: NavigationKind,
        _: &QueryPoint,
        _: &Cancellation,
    ) -> CoreResult<QueryResult<Arc<[Target]>>> {
        Err(Error::Unsupported)
    }
    fn hover(&self, _: &QueryPoint, _: &Cancellation) -> CoreResult<QueryResult<Option<Hover>>> {
        Err(Error::Unsupported)
    }
    fn file_symbols(
        &self,
        _: &Resource,
        _: Version,
        _: &Cancellation,
    ) -> CoreResult<QueryResult<Arc<[Symbol]>>> {
        Err(Error::Unsupported)
    }
    fn workspace_symbols(
        &self,
        query: &str,
        _: &Cancellation,
    ) -> CoreResult<QueryResult<Arc<[WorkspaceSymbol]>>> {
        // 模拟 L1 的前缀筛选；**顺序就是这里给出的顺序**，界面不再排序。
        let value: Arc<[WorkspaceSymbol]> = self
            .symbols
            .iter()
            .filter(|s| s.name.to_lowercase().contains(&query.to_lowercase()))
            .cloned()
            .collect();
        Ok(QueryResult {
            value,
            precision: self.precision,
            reason: None,
            version: None,
            index_generation: 1,
        })
    }
    fn semantic_tokens(
        &self,
        _: &Resource,
        _: Version,
        _: &Cancellation,
    ) -> CoreResult<QueryResult<Arc<[SemanticToken]>>> {
        Err(Error::Unsupported)
    }
    fn inactive_regions(
        &self,
        _: &Resource,
        _: Version,
        _: &Cancellation,
    ) -> CoreResult<QueryResult<Arc<[TextRange]>>> {
        Err(Error::Unsupported)
    }
}

struct Project {
    folder: tempfile::TempDir,
    index: Arc<CsIndex>,
    _user: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Project {
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
        std::fs::create_dir(folder.path().join("Assets")).unwrap();
        for (path, text) in FILES {
            std::fs::write(folder.path().join(path), text).unwrap();
        }
        std::fs::write(folder.path().join("Assets/notes.md"), NOTES_MD).unwrap();
        std::fs::write(folder.path().join("Assets/notes.txt"), "plain\n").unwrap();
        let file = folder.path().join("index.idx");
        build_index().write(&file).unwrap();
        let index = Arc::new(CsIndex::open(&file).unwrap());
        Self {
            folder,
            index,
            _user: user,
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
        let semantic: Arc<dyn SemanticSource> = Arc::new(ManualSemantic::ready(self.index.clone()));
        view.update(cx, |v, cx| v.attach_semantic(Some(semantic), cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.focus(&view.read(cx).focus));
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

/// 像命令面板那样运行一条命令（这个命令没有默认键位，只能这样触发）。
fn run_nav(view: &Entity<Workbench>, id: &'static str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |v, cx| v.nav_command(id, window, cx));
    });
    cx.run_until_parked();
    refresh(cx);
}

fn location(view: &Entity<Workbench>, cx: &VisualTestContext) -> (String, Position) {
    view.read_with(cx, |v, cx| {
        let path = match &v.editor.active().current().unwrap().resource {
            Resource::Workspace(p) => p.as_str().to_string(),
            other => format!("{other:?}"),
        };
        (path, v.code_view().read(cx).selection().active)
    })
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

fn editor_context(view: &Entity<Workbench>, cx: &VisualTestContext) -> String {
    view.read_with(cx, |v, _| v.nav_context())
}

// --------------------------------------------------------------- 转到类型定义

/// 单候选直接跳转；命令没有默认键位，走命令面板那条路径。
#[gpui::test]
fn type_definition_jumps_to_the_declared_type(cx: &mut TestAppContext) {
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    open_file(&view, "Assets/Hero.cs", cx);
    caret(&view, word(HERO_CS, "Player", 0), cx);
    run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
    let (path, at) = location(&view, cx);
    assert_eq!(path, "Assets/Player.cs");
    assert_eq!(at, word(PLAYER_CS, "Player", 0));
    assert_eq!(cursor_hint(&view, cx), None);
}

/// 上下文键：C# 文件里为真，非 C# 文件里为假且命令什么都不做。
#[gpui::test]
fn type_definition_provider_key_is_only_published_for_csharp(cx: &mut TestAppContext) {
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    open_file(&view, "Assets/Hero.cs", cx);
    assert!(editor_context(&view, cx).contains(nav_reg::context::HAS_TYPE_DEFINITION));
    open_file(&view, "Assets/notes.txt", cx);
    assert!(!editor_context(&view, cx).contains(nav_reg::context::HAS_TYPE_DEFINITION));
    caret(&view, pos(0, 1), cx);
    run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
    // 不跳转、不提示、不开空速览。
    assert_eq!(location(&view, cx).0, "Assets/notes.txt");
    assert_eq!(cursor_hint(&view, cx), None);
    assert!(view.read_with(cx, |v, _| v.peek_widget(v.editor.focused, false).is_none()));
}

/// 验收点名的五种写法各一例：光标落在 `var` 变量 / 字段 / 属性 / 参数 / 泛型类型参数所声明的
/// 那个**名字**上（不是写出来的类型名上），跳到它**声明类型**的定义处。
#[gpui::test]
fn type_definition_covers_var_field_parameter_property_and_type_argument(cx: &mut TestAppContext) {
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    let player = ("Assets/Player.cs", word(PLAYER_CS, "Player", 0));
    // `public T Item;`：字段的声明类型是泛型类型参数 `T`，跳到 `Box<T>` 里的 `T`。
    let type_parameter = ("Assets/Writes.cs", word(WRITES_CS, "T", 0));
    for (name, expected) in [
        ("local", player),        // var 变量（从 `new Player()` 推断）
        ("field", player),        // 字段
        ("Property", player),     // 属性
        ("parameter", player),    // 参数
        ("Item", type_parameter), // 泛型类型参数
    ] {
        open_file(&view, "Assets/Writes.cs", cx);
        caret(&view, word(WRITES_CS, name, 0), cx);
        run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
        assert_eq!(cursor_hint(&view, cx), None, "{name}");
        let (path, at) = location(&view, cx);
        assert_eq!((path.as_str(), at), (expected.0, expected.1), "{name}");
    }
    // 光标落在写出来的类型名上时也跳到同一处（和 F12 一样，#40 已有）。
    open_file(&view, "Assets/Writes.cs", cx);
    for nth in 0..5 {
        caret(&view, word(WRITES_CS, "Player", nth), cx);
        run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
        let (path, at) = location(&view, cx);
        assert_eq!((path.as_str(), at), player, "第 {nth} 例");
        open_file(&view, "Assets/Writes.cs", cx);
    }
}

/// 声明类型读不出来（`int` 这类预定义类型、推断不出的 `var`）：按 UX_SPEC §6.3 在光标处提示，
/// 而且是“确实没有”——索引就绪时不写“（语义索引构建中）”。
#[gpui::test]
fn type_definition_without_a_declared_type_shows_the_empty_state(cx: &mut TestAppContext) {
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    open_file(&view, "Assets/Writes.cs", cx);
    caret(&view, word(WRITES_CS, "count", 0), cx);
    run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
    assert_eq!(
        cursor_hint(&view, cx).as_deref(),
        Some("未找到 `count` 的类型定义")
    );
}

/// 近似（语义索引构建中）时空状态追加“（语义索引构建中）”。
#[gpui::test]
fn type_definition_empty_state_mentions_the_index_when_approximate(cx: &mut TestAppContext) {
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    open_file(&view, "Assets/Writes.cs", cx);
    view.update(cx, |v, cx| {
        v.attach_semantic(
            Some(Arc::new(ManualSemantic::unavailable(
                rove_lang_cs::router::SemanticStatus::Building { percent: Some(40) },
            )) as Arc<dyn SemanticSource>),
            cx,
        )
    });
    cx.run_until_parked();
    caret(&view, word(WRITES_CS, "count", 0), cx);
    run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
    assert_eq!(
        cursor_hint(&view, cx).as_deref(),
        Some("未找到 `count` 的类型定义（语义索引构建中）")
    );
    assert_eq!(location(&view, cx).0, "Assets/Writes.cs", "没有跳转");
    // 索引没就绪也不是什么都做不了：声明处写出的类型名交给 L1 解析（近似结果）。
    caret(&view, word(WRITES_CS, "local", 0), cx);
    run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
    assert_eq!(location(&view, cx).0, "Assets/Player.cs");
}

/// `editor.gotoLocation.multipleTypeDefinitions` 的取值生效：
/// 单候选直接跳转；`peek`（默认）多候选开速览；`goto` 直接跳到第一个并进入符号导航模式。
#[gpui::test]
fn type_definition_multiple_candidates_follow_the_setting(cx: &mut TestAppContext) {
    // `Player` 只有一个定义：单候选直接跳转（不开速览）。
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    open_file(&view, "Assets/Hero.cs", cx);
    caret(&view, word(HERO_CS, "Player", 0), cx);
    run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
    assert!(
        view.read_with(cx, |v, _| v.peek_widget(v.editor.focused, false).is_none()),
        "单候选直接跳转"
    );
    assert_eq!(location(&view, cx).0, "Assets/Player.cs");
    // 默认值是 peek（按 compat/SETTINGS.md 登记）。
    let settings = view.read_with(cx, |v, _| v.settings.clone());
    assert_eq!(
        crate::nav::settings::NavSettings::read(&*settings, Some("csharp"))
            .multiple_type_definitions,
        crate::nav::settings::GotoMode::Peek
    );
    // `Box<T>` 是 partial，两个定义处：多候选 → 速览窗口。
    open_file(&view, "Assets/Writes.cs", cx);
    caret(&view, word(WRITES_CS, "Box", 1), cx);
    run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
    let widget = view
        .read_with(cx, |v, _| v.peek_widget(v.editor.focused, false))
        .expect("多候选应打开速览窗口");
    assert_eq!(
        widget.read_with(cx, |w, _| w.results().map(|r| r.total())),
        Some(2)
    );
}

/// `goto`：多候选不开速览，直接跳到第一个。
#[gpui::test]
fn type_definition_goto_mode_jumps_without_peeking(cx: &mut TestAppContext) {
    let project = Project::new("{ \"editor.gotoLocation.multipleTypeDefinitions\": \"goto\" }");
    let (view, cx) = project.open(cx);
    open_file(&view, "Assets/Writes.cs", cx);
    caret(&view, word(WRITES_CS, "Box", 1), cx);
    run_nav(&view, nav_reg::GOTO_TYPE_DEFINITION, cx);
    assert!(
        view.read_with(cx, |v, _| v.peek_widget(v.editor.focused, false).is_none()),
        "goto 不开速览"
    );
    assert_eq!(location(&view, cx).1, word(WRITES_CS, "Box", 0));
}

// ------------------------------------------------------- 转到工作区中的符号

fn fake_symbols(precision: Precision) -> Arc<dyn CsQueries> {
    Arc::new(FakeSymbols {
        symbols: vec![
            symbol("PlayerController", "Game", "Assets/Writes.cs", 1, 4),
            symbol("Player", "Game.Core", "Assets/Player.cs", 0, 17),
            symbol("Playbook", "", "Assets/notes.md", 2, 0),
        ],
        precision,
    })
}

fn quick_rows(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(String, String)> {
    view.read_with(cx, |v, cx| {
        let q = v.quick.view.as_ref().expect("浮层已创建").read(cx);
        let a = q.service.active().expect("浮层已打开");
        a.list
            .display_rows(0..a.list.len())
            .iter()
            .map(|r| {
                (
                    r.label.to_string(),
                    r.detail.as_deref().unwrap_or("").into(),
                )
            })
            .collect()
    })
}

fn quick_separators(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> Vec<(usize, Option<String>)> {
    view.read_with(cx, |v, cx| {
        let q = v.quick.view.as_ref().expect("浮层已创建").read(cx);
        let a = q.service.active().expect("浮层已打开");
        a.list
            .group_starts()
            .iter()
            .map(|(i, label)| (*i, label.as_deref().map(str::to_owned)))
            .collect()
    })
}

fn type_query(cx: &mut VisualTestContext, text: &str) {
    cx.simulate_input(text);
    cx.run_until_parked();
    refresh(cx);
    // 后台查询返回后列表再刷新一次。
    cx.run_until_parked();
    refresh(cx);
}

/// `Ctrl+T` 与快速打开的 `#` 前缀：同一份数据、同一套排序（逐条断言两者相同）。
#[gpui::test]
fn workspace_symbols_ctrl_t_and_the_hash_prefix_return_the_same_sequence(cx: &mut TestAppContext) {
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    view.update(cx, |v, _| {
        v.set_workspace_symbol_queries(Some(fake_symbols(Precision::Exact)))
    });
    // 入口一：Ctrl+T（输入预置成 `#`）。
    cx.simulate_keystrokes("ctrl-t");
    cx.run_until_parked();
    refresh(cx);
    type_query(cx, "play");
    let by_ctrl_t = quick_rows(&view, cx);
    assert_eq!(
        by_ctrl_t
            .iter()
            .map(|(l, _)| l.as_str())
            .collect::<Vec<_>>(),
        vec!["PlayerController", "Player", "Playbook"],
        "顺序就是查询返回的顺序"
    );
    assert_eq!(by_ctrl_t[0].1, "Game · Assets/Writes.cs");
    assert_eq!(by_ctrl_t[2].1, "Assets/notes.md", "没有容器名时只显示路径");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    refresh(cx);
    // 入口二：Ctrl+P 后手敲 `#`。
    cx.simulate_keystrokes("ctrl-p");
    cx.run_until_parked();
    refresh(cx);
    type_query(cx, "#play");
    assert_eq!(quick_rows(&view, cx), by_ctrl_t);
}

/// 近似结果在列表顶部加一行克制的说明；精确结果没有任何标记。
#[gpui::test]
fn workspace_symbols_mark_approximate_results_with_one_line(cx: &mut TestAppContext) {
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    view.update(cx, |v, _| {
        v.set_workspace_symbol_queries(Some(fake_symbols(Precision::Exact)))
    });
    cx.simulate_keystrokes("ctrl-t");
    cx.run_until_parked();
    refresh(cx);
    type_query(cx, "play");
    assert!(quick_separators(&view, cx).is_empty(), "精确结果不加标记");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    refresh(cx);
    // L1 的结果是语法级的：近似。
    view.update(cx, |v, _| {
        v.set_workspace_symbol_queries(Some(fake_symbols(Precision::Syntax)))
    });
    cx.simulate_keystrokes("ctrl-t");
    cx.run_until_parked();
    refresh(cx);
    type_query(cx, "play");
    let separators = quick_separators(&view, cx);
    assert_eq!(separators.len(), 1, "只有一行说明");
    assert_eq!(separators[0].0, 0, "在列表顶部");
    assert_eq!(separators[0].1.as_deref(), Some("近似结果：语义索引构建中"));
    assert_eq!(quick_rows(&view, cx).len(), 3);
}

/// 选中后经打开文档服务打开并定位；`.md` 打开的是源码。
#[gpui::test]
fn workspace_symbols_open_and_reveal_through_the_open_service(cx: &mut TestAppContext) {
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    view.update(cx, |v, _| {
        v.set_workspace_symbol_queries(Some(fake_symbols(Precision::Exact)))
    });
    cx.simulate_keystrokes("ctrl-t");
    cx.run_until_parked();
    refresh(cx);
    type_query(cx, "playbook");
    assert_eq!(quick_rows(&view, cx).len(), 1);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    refresh(cx);
    let (path, selection, kind) = view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        let path = match &tab.resource {
            Resource::Workspace(p) => p.as_str().to_string(),
            other => format!("{other:?}"),
        };
        (path, tab.view.selection.clone(), tab.kind.clone())
    });
    assert_eq!(path, "Assets/notes.md");
    assert_eq!(selection, Some(Span::new(pos(2, 0), pos(2, 8)).unwrap()));
    assert_eq!(
        kind,
        crate::editors::settings::EditorKind::Text,
        ".md 打开源码"
    );
}

/// 不装替身：走真实的 #38 路由 + #37 的 L1 声明索引。没有匹配时只有空状态，不报错。
#[gpui::test]
fn workspace_symbols_use_the_real_router_and_say_nothing_found_when_empty(cx: &mut TestAppContext) {
    let project = Project::new("{}");
    let (view, cx) = project.open(cx);
    cx.simulate_keystrokes("ctrl-t");
    cx.run_until_parked();
    refresh(cx);
    type_query(cx, "zzzznotasymbol");
    assert!(quick_rows(&view, cx).is_empty());
    let empty = view.read_with(cx, |v, cx| {
        let q = v.quick.view.as_ref()?.read(cx);
        let m = q.service.active()?.config.empty.clone()?;
        Some(v.strings.text(&m).unwrap())
    });
    assert_eq!(empty.as_deref(), Some("未找到匹配的符号"));
    // 没有任何通知 / 提示条（工作区里没有可匹配的 C# 符号不是错误）。
    assert!(view.read_with(cx, |v, _| v.shell.feedback.cursor_hint().is_none()));
}
