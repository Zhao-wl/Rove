//! #59 入口集成测试（真实 GPUI 窗口 + 真实索引读取端 + 真实主题引擎）：
//! 打开 `.cs` → 断言某个字段的 token 类型、修饰符与**最终颜色**；三值开关与 `[csharp]` 覆盖；
//! 关闭时一次查询都不发；只让被覆盖的行失效；不生效的 `#if` 区域整行置灰；大文件关闭并登记原因。

use super::*;
use crate::code_view::Feature;
use gpui::{TestAppContext, VisualTestContext};
use rove_lang_cs::router::testing::{find_word, TestIndex, KIND_FIELD, KIND_METHOD, KIND_TYPE};
use rove_lang_cs::router::{hash_bytes, ManualSemantic, SemanticSource};
use rove_lang_cs::CsIndex;
use rove_theme::Color;

const PATH: &str = "Assets/Unit.cs";
const SRC: &str = "namespace Game
{
    public class Unit
    {
        public const int MaxHealth = 100;
        private readonly string _name;
        private int _health;

        public void Hit(int amount)
        {
            _health -= amount;
        }
#if UNITY_EDITOR
        void EditorOnly() { }
#endif
    }
}
";

/// 不生效的预处理区域：`#if UNITY_EDITOR` 的三行（0 起算）。
fn inactive_lines() -> (u32, u32) {
    let start = SRC[..SRC.find("#if UNITY_EDITOR").unwrap()]
        .matches('\n')
        .count() as u32;
    (start, start + 3)
}

fn build_index() -> TestIndex {
    let mut t = TestIndex::default();
    let f = t.file(PATH, hash_bytes(SRC.as_bytes()));
    let unit = t.symbol("Game|T:Game.Unit", "Unit", KIND_TYPE, None);
    let max = t.symbol("Game|F:Game.Unit.MaxHealth", "MaxHealth", KIND_FIELD, None);
    let name = t.symbol("Game|F:Game.Unit._name", "_name", KIND_FIELD, None);
    let health = t.symbol("Game|F:Game.Unit._health", "_health", KIND_FIELD, None);
    let hit = t.symbol(
        "Game|M:Game.Unit.Hit(System.Int32)",
        "Hit",
        KIND_METHOD,
        None,
    );
    for (sym, word, defs, refs) in [
        (unit, "Unit", 1, 0),
        (max, "MaxHealth", 1, 0),
        (name, "_name", 1, 0),
        (health, "_health", 1, 1),
        (hit, "Hit", 1, 0),
    ] {
        for nth in 0..defs + refs {
            let (line, col) = find_word(SRC, word, nth);
            if nth < defs {
                t.def(sym, f, line, col);
            } else {
                t.reference(sym, f, line, col);
            }
        }
    }
    let (start, end) = inactive_lines();
    t.inactive.push((f, start, end));
    t
}

struct Project {
    folder: tempfile::TempDir,
    index: Arc<CsIndex>,
}

impl Project {
    fn new() -> Self {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir(folder.path().join("Assets")).unwrap();
        std::fs::write(folder.path().join(PATH), SRC).unwrap();
        let file = folder.path().join("index.idx");
        build_index().write(&file).unwrap();
        let index = Arc::new(CsIndex::open(&file).unwrap());
        Self { folder, index }
    }

    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let root: PathBuf = self.folder.path().into();
        let (view, cx) = cx.add_window_view(|_, cx| Workbench::new(root, Instant::now(), cx));
        cx.run_until_parked();
        let semantic: Arc<dyn SemanticSource> = Arc::new(ManualSemantic::ready(self.index.clone()));
        view.update(cx, |v, cx| v.attach_semantic(Some(semantic), cx));
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
}

/// 某一行某一列（源字节列）上最终合并出来的样式。
fn style_at(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    (line, col): (u32, u32),
) -> Option<crate::code_view::TextStyle> {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| {
        let merged = c.merged_decorations(u64::from(line))?;
        merged
            .runs
            .iter()
            .find(|r| r.range.start <= col && col < r.range.end)
            .map(|r| r.style)
    })
}

fn line_opacity(view: &Entity<Workbench>, cx: &mut VisualTestContext, line: u64) -> Option<u8> {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| c.merged_decorations(line)?.line.opacity)
}

/// 本窗口当前主题下某个语义 token 的颜色（测试与实现用同一条解析路径，但各自独立调用）。
fn expected_color(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    ty: &str,
    mods: &[&str],
) -> Option<Color> {
    view.read_with(cx, |v, _| {
        v.resolved_theme()
            .semantic_token_style(ty, mods, "csharp")
            .foreground
    })
}

#[gpui::test]
fn csharp_semantic_override_applies_before_the_first_query(cx: &mut TestAppContext) {
    use rove_config::{SettingsOptions, SettingsService};

    let project = Project::new();
    for enabled in [false, true] {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(
            user.path().join("settings.json"),
            serde_json::json!({
                "[csharp]": { "editor.semanticHighlighting.enabled": enabled },
            })
            .to_string(),
        )
        .unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let settings: Arc<dyn Settings> = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        let root = project.folder.path().to_path_buf();
        let (view, window) = cx.add_window_view(|_, cx| {
            Workbench::with_settings(root, Instant::now(), LayoutStore::disabled(), settings, cx)
                .unwrap()
        });
        let source: Arc<dyn SemanticSource> =
            Arc::new(ManualSemantic::ready(project.index.clone()));
        view.update(window, |view, cx| view.attach_semantic(Some(source), cx));
        open_file(&view, PATH, window);
        view.read_with(window, |view, _| {
            assert_eq!(view.semantic_highlighting_on(), enabled);
            assert_eq!(view.semantic_query_count() > 0, enabled);
        });
    }
}

#[gpui::test]
fn opening_a_cs_file_colours_a_field_by_its_semantic_token(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(cx);
    open_file(&view, PATH, cx);
    cx.run_until_parked();
    refresh(cx);

    let group = view.read_with(cx, |v, _| v.editor.focused);
    let token_at = |cx: &mut VisualTestContext, (line, col): (u32, u32)| {
        view.read_with(cx, |v, _| v.semantic_token_at(group, false, line, col))
    };

    // `const` 字段（C# 里 `const` 就是字段）：token 类型 variable、修饰符
    // declaration + static + readonly，最终颜色由回退表的
    // `variable.readonly → variable.other.constant` 给出。
    let konst = find_word(SRC, "MaxHealth", 0);
    assert_eq!(
        token_at(cx, konst),
        Some((
            "variable".to_owned(),
            vec![
                "declaration".to_owned(),
                "static".to_owned(),
                "readonly".to_owned()
            ]
        ))
    );
    let const_style = style_at(&view, cx, konst).expect("常量应有装饰");
    let const_expected = expected_color(
        &view,
        cx,
        "variable",
        &["declaration", "static", "readonly"],
    )
    .expect("variable.readonly 走回退表一定有颜色");
    assert_eq!(const_style.foreground, Some(const_expected));

    // 普通字段 `_health`：token 类型 field、声明处带 declaration、引用处没有。
    let decl = find_word(SRC, "_health", 0);
    let use_ = find_word(SRC, "_health", 1);
    assert_eq!(
        token_at(cx, decl),
        Some(("field".to_owned(), vec!["declaration".to_owned()]))
    );
    assert_eq!(token_at(cx, use_), Some(("field".to_owned(), Vec::new())));
    // `readonly` 字段的修饰符也取到了（索引里没有，由本文件的语法树补）。
    assert_eq!(
        token_at(cx, find_word(SRC, "_name", 0)),
        Some((
            "field".to_owned(),
            vec!["declaration".to_owned(), "readonly".to_owned()]
        ))
    );
    // VS Code 的 `tokenClassificationRegistry` 里没有 `field` 这一条回退规则，主题也没写
    // `semanticTokenColors.field`：这一列保持语法高亮的颜色，与 VS Code 一致。
    assert_eq!(
        expected_color(&view, cx, "field", &["declaration"]),
        None,
        "回退表里没有 field"
    );

    // 类名：class → `entity.name.type.class`。
    let ty = find_word(SRC, "Unit", 0);
    let class_style = style_at(&view, cx, ty).expect("类名应有装饰");
    let class_expected =
        expected_color(&view, cx, "class", &["declaration"]).expect("class 有回退");
    assert_eq!(class_style.foreground, Some(class_expected));

    // 只让**真正被语义样式覆盖的行**失效：没有样式的 token（field）与空行、`{`、`}` 都不在集合里。
    let invalid = view.read_with(cx, |v, _| v.semantic_invalidated_lines(group, false));
    let covered: Vec<u64> = invalid.iter().flat_map(|r| r.clone()).collect();
    assert!(covered.contains(&u64::from(ty.0)), "{invalid:?}");
    assert!(covered.contains(&u64::from(konst.0)), "{invalid:?}");
    assert!(!covered.contains(&0), "命名空间那一行没有 token，不该失效");
    assert!(!covered.contains(&1), "只有 `{{` 的行不该失效");
    assert!(
        !covered.contains(&u64::from(decl.0)),
        "field 没有样式，不必让这一行失效：{invalid:?}"
    );
}

#[gpui::test]
fn switching_csharp_documents_clears_old_semantics_when_the_index_is_unavailable(
    cx: &mut TestAppContext,
) {
    use rove_lang_cs::router::SemanticStatus;

    let project = Project::new();
    const OTHER: &str = "Assets/Other.cs";
    std::fs::write(project.folder.path().join(OTHER), SRC).unwrap();
    let source = Arc::new(ManualSemantic::ready(project.index.clone()));
    let (view, cx) = project.open(cx);
    view.update(cx, |view, cx| {
        view.attach_semantic(Some(source.clone()), cx);
        view.set_semantic_setting(semantic_host::SemanticHighlighting::On, cx);
    });
    open_file(&view, PATH, cx);
    let line = u64::from(inactive_lines().0);
    assert!(line_opacity(&view, cx, line).is_some());

    source.set(None, SemanticStatus::Building { percent: None });
    open_file(&view, OTHER, cx);
    assert_eq!(line_opacity(&view, cx, line), None);
}

#[gpui::test]
fn losing_the_index_clears_semantic_decorations_in_the_open_document(cx: &mut TestAppContext) {
    use rove_lang_cs::router::SemanticStatus;

    let project = Project::new();
    let source = Arc::new(ManualSemantic::ready(project.index.clone()));
    let (view, cx) = project.open(cx);
    view.update(cx, |view, cx| {
        view.attach_semantic(Some(source.clone()), cx);
        view.set_semantic_setting(semantic_host::SemanticHighlighting::On, cx);
    });
    open_file(&view, PATH, cx);
    let line = u64::from(inactive_lines().0);
    assert!(line_opacity(&view, cx, line).is_some());

    source.set(None, SemanticStatus::Building { percent: None });
    view.update(cx, |view, cx| view.semantic_index_changed(cx));
    refresh(cx);
    assert_eq!(line_opacity(&view, cx, line), None);
}

#[gpui::test]
fn inactive_preprocessor_lines_are_dimmed(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(cx);
    open_file(&view, PATH, cx);
    cx.run_until_parked();
    refresh(cx);
    let (start, end) = inactive_lines();
    for line in start..end {
        assert_eq!(
            line_opacity(&view, cx, u64::from(line)),
            Some(0xaa),
            "第 {line} 行应置灰"
        );
    }
    assert_eq!(line_opacity(&view, cx, u64::from(end)), None);
}

/// 三值开关的读取：按 `[csharp]` 的语言作用域读，所以语言级覆盖自动生效（#13 负责作用域合并）。
#[test]
fn the_switch_reads_three_values_through_the_csharp_scope() {
    use rove_base::{Events, NoEvents, Result as CoreResult, Task, Value};
    use rove_config::{Scope, Settings, SettingsChange};

    /// 全局 `true`、`[csharp]` 覆盖为 `value`（设置层按作用域解析后给出的就是覆盖值）。
    struct Scoped(Option<Value>);
    impl Settings for Scoped {
        fn read(&self, key: &str, scope: &Scope) -> CoreResult<Value> {
            if key != "editor.semanticHighlighting.enabled" {
                return Err(rove_base::Error::NotFound);
            }
            match (&self.0, scope.language.as_deref()) {
                (Some(v), Some("csharp")) => Ok(v.clone()),
                _ => Ok(Value::Bool(true)),
            }
        }
        fn write(&self, _: Arc<str>, _: Scope, _: Value, _: Cancellation) -> Task<()> {
            Box::pin(async { Err(rove_base::Error::Unsupported) })
        }
        fn changes(&self) -> &dyn Events<SettingsChange> {
            &NoEvents
        }
    }
    use super::semantic_host::SemanticHighlighting as S;
    // 没有语言级覆盖：全局 `true`。
    assert_eq!(S::read(&Scoped(None)), S::On);
    // `[csharp]` 覆盖为 false / true / configuredByTheme。
    assert_eq!(S::read(&Scoped(Some(Value::Bool(false)))), S::Off);
    assert_eq!(S::read(&Scoped(Some(Value::Bool(true)))), S::On);
    assert_eq!(
        S::read(&Scoped(Some(Value::String("configuredByTheme".into())))),
        S::ConfiguredByTheme
    );
    // 未登记 / 读不到：默认 `configuredByTheme`。
    let defaults = crate::app_settings::defaults_only();
    assert_eq!(S::read(&*defaults), S::ConfiguredByTheme);
    // 三个取值的解析。
    assert!(S::On.resolve(false));
    assert!(!S::Off.resolve(true));
    assert!(S::ConfiguredByTheme.resolve(true));
    assert!(!S::ConfiguredByTheme.resolve(false));
}

#[gpui::test]
fn turning_the_switch_off_stops_every_query(cx: &mut TestAppContext) {
    use super::semantic_host::SemanticHighlighting as S;
    let project = Project::new();
    let (view, cx) = project.open(cx);
    open_file(&view, PATH, cx);
    cx.run_until_parked();
    // 默认 `configuredByTheme`，内置主题 Dark 2026 打开了 `semanticHighlighting`。
    assert!(view.read_with(cx, |v, _| v.semantic_highlighting_on()));
    assert!(
        view.read_with(cx, |v, _| v.semantic_query_count()) > 0,
        "开着时应发起过查询"
    );
    let konst = find_word(SRC, "MaxHealth", 0);
    let const_expected = expected_color(
        &view,
        cx,
        "variable",
        &["declaration", "static", "readonly"],
    )
    .unwrap();
    assert_eq!(
        style_at(&view, cx, konst).and_then(|s| s.foreground),
        Some(const_expected)
    );

    // 关掉：不再发起任何查询，装饰也清掉。
    view.update(cx, |v, cx| v.set_semantic_setting(S::Off, cx));
    cx.run_until_parked();
    refresh(cx);
    let after = view.read_with(cx, |v, _| v.semantic_query_count());
    open_file(&view, PATH, cx);
    cx.run_until_parked();
    refresh(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.semantic_query_count()),
        after,
        "关闭后不得再发起任何语义查询"
    );
    assert!(!view.read_with(cx, |v, _| v.semantic_highlighting_on()));
    assert_ne!(
        style_at(&view, cx, konst).and_then(|s| s.foreground),
        Some(const_expected),
        "关闭后不再有语义颜色"
    );
}

#[gpui::test]
fn a_large_document_switches_semantic_highlighting_off(cx: &mut TestAppContext) {
    let project = Project::new();
    let (view, cx) = project.open(cx);
    open_file(&view, PATH, cx);
    cx.run_until_parked();
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert!(code.read_with(cx, |c, _| c.semantic_enabled()));
    code.update(cx, |c, cx| {
        c.set_document_level(rove_buffer::SizeClass::Large, cx)
    });
    cx.run_until_parked();
    assert!(!code.read_with(cx, |c, _| c.semantic_enabled()));
    assert!(code
        .read_with(cx, |c, _| c.disabled_features())
        .contains(&Feature::SemanticHighlighting));
    // 关闭原因有登记过的界面字符串。
    assert!(!Feature::SemanticHighlighting.label_key().is_empty());
}
