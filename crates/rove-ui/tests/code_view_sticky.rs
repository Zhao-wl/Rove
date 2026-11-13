//! 粘性滚动（#55）的入口集成测试：三档模型、`maxLineCount`、横向滚动、点击跳转与视口换算、
//! 键盘与上下文键、按文档级别关闭、登记机制。只用 `rove_ui::code_view` 的公开接口，在 GPUI
//! 测试窗口里驱动。
//!
//! 运行：`cargo test -p rove-ui --test code_view_sticky`

use gpui::{Context, Entity, Focusable, TestAppContext, VisualTestContext, Window};
use rove_base::*;
use rove_buffer::{RopeText, SizeClass, TextSource};
use rove_commands::{
    keybindings::KeybindingService, keys::parse_keybinding, CommandRegistry, ContextSnapshot,
    KeyResolution,
};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings, SettingsChange};
use rove_syntax::symbols::{extract, DocumentSymbols};
use rove_theme::{Color, ColorDefaults, ColorRegistry, DefaultTheme, Theme, ThemeKind};
use rove_ui::code_view::{
    self, scroll::ScrollPosition, sticky::StickyModel, CodeView, CodeViewServices, Feature,
};
use std::sync::{Arc, Mutex};

// ------------------------------------------------------------------ 夹具

/// 记录写回的设置（`editor.action.toggleStickyScroll` 要写回 `editor.stickyScroll.enabled`）。
struct Overrides {
    base: DefaultSettings,
    overrides: Vec<(&'static str, Value)>,
    written: Mutex<Vec<(String, Value)>>,
}

impl Settings for Overrides {
    fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
        if let Some(v) = self
            .written
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(k, _)| k == key)
        {
            return Ok(v.1.clone());
        }
        match self.overrides.iter().find(|(k, _)| *k == key) {
            Some((_, v)) => Ok(v.clone()),
            None => self.base.read(key, scope),
        }
    }
    fn write(&self, key: Arc<str>, _: Scope, value: Value, _: Cancellation) -> Task<()> {
        self.written.lock().unwrap().push((key.to_string(), value));
        Box::pin(async { Ok(()) })
    }
    fn changes(&self) -> &dyn Events<SettingsChange> {
        self.base.changes()
    }
}

fn registries() -> (
    SettingRegistry,
    ColorRegistry,
    CommandRegistry,
    rove_i18n::StringTable,
) {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    (settings, colors, commands, strings)
}

fn services(
    overrides: Vec<(&'static str, Value)>,
    kind: ThemeKind,
) -> (CodeViewServices, Arc<Overrides>) {
    let (settings, colors, _, _) = registries();
    let settings = Arc::new(Overrides {
        base: DefaultSettings(settings),
        overrides,
        written: Mutex::new(Vec::new()),
    });
    let services = CodeViewServices {
        settings: settings.clone(),
        theme: Arc::new(DefaultTheme { colors, kind }),
        theme_kind: kind,
    };
    (services, settings)
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    language: Option<&str>,
    overrides: Vec<(&'static str, Value)>,
) -> (Entity<CodeView>, &'a mut VisualTestContext, Arc<Overrides>) {
    let source: Arc<dyn TextSource> = Arc::new(RopeText::from_text(text));
    let (s, settings) = services(overrides, ThemeKind::Dark);
    let language = language.map(Arc::from);
    let (view, cx) = cx.add_window_view(move |_: &mut Window, cx: &mut Context<CodeView>| {
        let mut v = CodeView::new(s, cx);
        v.set_text(Some(source), false, cx);
        v.set_language(language, cx);
        v
    });
    cx.update(|window, cx| {
        rove_ui::keys::init(cx);
        window.focus(&view.read(cx).focus_handle(cx));
    });
    settle(cx);
    (view, cx, settings)
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

fn symbols_of(text: &str, language: &str) -> Arc<DocumentSymbols> {
    let roots = extract(language, text, &Cancellation::default()).unwrap();
    Arc::new(DocumentSymbols::new(
        Version(1),
        language.into(),
        roots.into(),
    ))
}

/// 滚到某一屏幕行（无折叠时即文档行）并等一帧。
fn scroll_to(view: &Entity<CodeView>, cx: &mut VisualTestContext, line: u64) {
    view.update(cx, |v, cx| {
        v.set_scroll_position(
            ScrollPosition {
                top: line as f64,
                x: v.scroll_position().x,
            },
            false,
            cx,
        )
    });
    settle(cx);
}

fn sticky(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Vec<u64> {
    view.read_with(cx, |v, _| v.sticky_lines().iter().map(|l| l.line).collect())
}

fn run(view: &Entity<CodeView>, cx: &mut VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        view.update(cx, |v, cx| assert!(v.execute(id, window, cx), "{id}"));
    });
    settle(cx);
}

fn line_of(text: &str, needle: &str) -> u64 {
    text.lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("找不到 {needle}")) as u64
}

fn line_after(text: &str, from: u64, needle: &str) -> u64 {
    text.lines()
        .enumerate()
        .skip(from as usize)
        .find(|(_, l)| l.contains(needle))
        .map(|(i, _)| i as u64)
        .unwrap_or_else(|| panic!("{from} 之后找不到 {needle}"))
}

/// 约 5,000 行 Unity 风格的 C#：命名空间 → 25 个类 → 每个类 8 个字段 + 19 个方法（方法体 6 行）。
fn csharp_5000() -> String {
    let mut s = String::from("using UnityEngine;\n\nnamespace Game.Generated\n{\n");
    for c in 0..25 {
        s.push_str(&format!(
            "    public class Component{c} : MonoBehaviour\n    {{\n"
        ));
        for f in 0..8 {
            s.push_str(&format!("        [SerializeField] private int field{f};\n"));
        }
        for m in 0..19 {
            s.push_str(&format!(
                "        public void Method{m}(int value)\n        {{\n"
            ));
            for k in 0..6 {
                s.push_str(&format!("            var step{k} = value * {k};\n"));
            }
            s.push_str("        }\n\n");
        }
        s.push_str("    }\n\n");
    }
    s.push_str("}\n");
    s
}

/// 很宽的 C#：一个方法，方法体每行 300 多列，用来测水平滚动。
fn csharp_wide() -> String {
    let pad = "x".repeat(300);
    let mut s =
        String::from("namespace Wide\n{\n    public class Host\n    {\n        public void Run(int value)\n        {\n");
    for k in 0..40 {
        s.push_str(&format!("            var step{k} = value; // {pad}\n"));
    }
    s.push_str("        }\n    }\n}\n");
    s
}

// ------------------------------------------------------------------ 1. 大纲模型

/// 在 5,000 行 C# 里滚动：粘性区依次显示命名空间 → 类型 → 方法，并且随滚动换成下一个方法。
#[gpui::test]
fn outline_model_shows_namespace_type_and_method(cx: &mut TestAppContext) {
    let text = csharp_5000();
    let lines = text.lines().count() as u64;
    assert!((4_900..5_400).contains(&lines), "{lines} 行");
    let symbols = symbols_of(&text, "csharp");
    let (view, cx, _) = open(cx, &text, Some("csharp"), vec![]);
    view.update(cx, |v, cx| v.set_symbols(Some(symbols), cx));
    settle(cx);

    let namespace = line_of(&text, "namespace Game.Generated");
    let class = line_of(&text, "class Component7");
    let method = line_after(&text, class, "void Method5(");
    scroll_to(&view, cx, method + 3);
    assert_eq!(sticky(&view, cx), vec![namespace, class, method]);
    assert_eq!(
        view.read_with(cx, |v, _| v.sticky_model()),
        Some(StickyModel::Outline)
    );
    // 粘性区高度 = 行数 × 行高。
    view.read_with(cx, |v, _| {
        assert_eq!(v.sticky_height(), 3.0 * v.line_height());
    });

    // 滚到下一个方法：最内层换成它，外面两级不变。
    let next = line_after(&text, method + 1, "void Method6(");
    scroll_to(&view, cx, next + 3);
    assert_eq!(sticky(&view, cx), vec![namespace, class, next]);

    // 类的字段区（还没进任何方法）：只剩命名空间与类型。
    scroll_to(&view, cx, class + 3);
    assert_eq!(sticky(&view, cx), vec![namespace, class]);

    // 文首没有粘性行。
    scroll_to(&view, cx, 0);
    assert!(sticky(&view, cx).is_empty());
}

/// `editor.stickyScroll.maxLineCount` 为 1 / 3 / 5 时保留最内层的 1 / 3 / 5 级。
#[gpui::test]
fn max_line_count_keeps_the_innermost_levels(cx: &mut TestAppContext) {
    let mut text = String::from(
        "namespace Outer\n{\n    namespace Inner\n    {\n        public class Host\n        {\n            public class Nested\n            {\n                public void Deep(int value)\n                {\n",
    );
    for k in 0..20 {
        text.push_str(&format!("                    var step{k} = value;\n"));
    }
    text.push_str("                }\n            }\n        }\n    }\n}\n");
    let text = text.as_str();
    let symbols = symbols_of(text, "csharp");
    let (view, cx, _) = open(cx, text, Some("csharp"), vec![]);
    view.update(cx, |v, cx| v.set_symbols(Some(symbols), cx));
    settle(cx);
    let deep = line_of(text, "void Deep(");
    let all = [
        line_of(text, "namespace Outer"),
        line_of(text, "namespace Inner"),
        line_of(text, "class Host"),
        line_of(text, "class Nested"),
        deep,
    ];
    for max in [5u64, 3, 1] {
        view.update(cx, |v, cx| {
            let mut o = v.options().clone();
            o.sticky_scroll_max_line_count = max as usize;
            v.set_options(o, cx);
        });
        scroll_to(&view, cx, deep + 6);
        let expected: Vec<u64> = all[all.len() - max as usize..].to_vec();
        assert_eq!(sticky(&view, cx), expected, "maxLineCount = {max}");
    }
}

// ------------------------------------------------------------------ 2. 三档模型与回退

/// 没有符号树但有 `#region` 的文件：折叠提供方模型给出区间链的起始行。
#[gpui::test]
fn folding_provider_model_uses_region_markers(cx: &mut TestAppContext) {
    let text = "// generated\n#region Outer\nvalue 1\n#region Inner\nvalue 2\nvalue 3\nvalue 4\nvalue 5\n#endregion\nvalue 6\n#endregion\nvalue 7\n";
    let (view, cx, _) = open(
        cx,
        text,
        Some("csharp"),
        vec![(
            "editor.stickyScroll.defaultModel",
            Value::String("foldingProviderModel".into()),
        )],
    );
    let outer = line_of(text, "#region Outer");
    let inner = line_of(text, "#region Inner");
    scroll_to(&view, cx, inner + 2);
    assert_eq!(sticky(&view, cx), vec![outer, inner]);
    assert_eq!(
        view.read_with(cx, |v, _| v.sticky_model()),
        Some(StickyModel::FoldingProvider)
    );
}

/// YAML：缩进模型按缩进层级回溯。
#[gpui::test]
fn indentation_model_follows_indentation(cx: &mut TestAppContext) {
    let text = "root:\n  child:\n    key1: 1\n    key2: 2\n    key3: 3\n    key4: 4\n    key5: 5\n  other: 6\n";
    let (view, cx, _) = open(
        cx,
        text,
        Some("yaml"),
        vec![(
            "editor.stickyScroll.defaultModel",
            Value::String("indentationModel".into()),
        )],
    );
    scroll_to(&view, cx, line_of(text, "key3:"));
    assert_eq!(
        sticky(&view, cx),
        vec![line_of(text, "root:"), line_of(text, "child:")]
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.sticky_model()),
        Some(StickyModel::Indentation)
    );
}

/// 默认的 `outlineModel` 在没有符号树时按固定顺序回退：折叠提供方模型 → 缩进模型。
#[gpui::test]
fn outline_model_falls_back_in_a_fixed_order(cx: &mut TestAppContext) {
    let text = "root:\n  child:\n    key1: 1\n    key2: 2\n    key3: 3\n    key4: 4\n    key5: 5\n";
    let (view, cx, _) = open(cx, text, Some("yaml"), vec![]);
    scroll_to(&view, cx, line_of(text, "key3:"));
    // YAML 没有 `#region`，折叠区间也来自缩进，所以两档给出同一条链。
    assert_eq!(
        sticky(&view, cx),
        vec![line_of(text, "root:"), line_of(text, "child:")]
    );
    assert!(matches!(
        view.read_with(cx, |v, _| v.sticky_model()),
        Some(StickyModel::FoldingProvider) | Some(StickyModel::Indentation)
    ));
}

// ------------------------------------------------------------------ 3. 交互与视口换算

/// 点击粘性行跳转并定位：目标行落在粘性区下面，不被遮挡。
#[gpui::test]
fn clicking_a_sticky_line_reveals_it_below_the_band(cx: &mut TestAppContext) {
    let text = csharp_5000();
    let symbols = symbols_of(&text, "csharp");
    let (view, cx, _) = open(cx, &text, Some("csharp"), vec![]);
    view.update(cx, |v, cx| v.set_symbols(Some(symbols), cx));
    settle(cx);
    let class = line_of(&text, "class Component7");
    let method = line_after(&text, class, "void Method5(");
    scroll_to(&view, cx, method + 3);
    let before = sticky(&view, cx);
    assert_eq!(before.len(), 3);

    // 点击最内层（方法声明行）。
    view.update(cx, |v, cx| v.go_to_sticky_line(2, cx));
    settle(cx);
    let first = view.read_with(cx, |v, _| v.viewport().unwrap().first_line);
    let band = view.read_with(cx, |v, _| v.sticky_lines().len() as u64);
    assert!(
        method >= first + band,
        "目标行 {method} 要在粘性区（首行 {first} + {band} 行）下面"
    );
    // 插入符落在该行。
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection().active.line.0, method);
    });
}

/// `editor.stickyScroll.scrollWithEditor`：为真时粘性行跟着正文横向滚动，为假时不动。
#[gpui::test]
fn scroll_with_editor_controls_the_horizontal_offset(cx: &mut TestAppContext) {
    let text = csharp_wide();
    let symbols = symbols_of(&text, "csharp");
    for value in [true, false] {
        let (view, cx, _) = open(
            cx,
            &text,
            Some("csharp"),
            vec![("editor.stickyScroll.scrollWithEditor", Value::Bool(value))],
        );
        view.update(cx, |v, cx| v.set_symbols(Some(symbols.clone()), cx));
        settle(cx);
        let method = line_of(&text, "void Run(");
        scroll_to(&view, cx, method + 6);
        assert_eq!(sticky(&view, cx).len(), 3, "命名空间 → 类型 → 方法");
        view.update(cx, |v, cx| {
            v.set_scroll_position(
                ScrollPosition {
                    top: v.scroll_position().top,
                    x: 40.0,
                },
                false,
                cx,
            )
        });
        settle(cx);
        let (x, offset) = view.read_with(cx, |v, _| {
            (v.scroll_position().x, v.sticky_content_offset())
        });
        assert_eq!(x, 40.0, "行足够宽，水平滚动没有被钳制");
        if value {
            assert_eq!(offset, 40.0, "scrollWithEditor = true 时跟着横向滚动");
        } else {
            assert_eq!(offset, 0.0, "scrollWithEditor = false 时不动");
        }
    }
}

/// 键盘全套：`focusStickyScroll` → `↓` / `↑` → `Enter`，以及 `Esc` 退回编辑器。
#[gpui::test]
fn keyboard_navigates_the_band_and_publishes_the_context_key(cx: &mut TestAppContext) {
    let text = csharp_5000();
    let symbols = symbols_of(&text, "csharp");
    let (view, cx, _) = open(cx, &text, Some("csharp"), vec![]);
    view.update(cx, |v, cx| v.set_symbols(Some(symbols), cx));
    settle(cx);
    let namespace = line_of(&text, "namespace Game.Generated");
    let class = line_of(&text, "class Component9");
    let method = line_after(&text, class, "void Method2(");
    scroll_to(&view, cx, method + 3);
    assert_eq!(sticky(&view, cx), vec![namespace, class, method]);
    view.read_with(cx, |v, _| assert!(!v.sticky_focused()));

    run(&view, cx, "editor.action.focusStickyScroll");
    view.read_with(cx, |v, _| {
        assert!(v.sticky_focused());
        assert_eq!(v.sticky_focus(), Some(0));
    });
    run(&view, cx, "editor.action.selectNextStickyScrollLine");
    run(&view, cx, "editor.action.selectNextStickyScrollLine");
    view.read_with(cx, |v, _| assert_eq!(v.sticky_focus(), Some(2)));
    run(&view, cx, "editor.action.selectPreviousStickyScrollLine");
    view.read_with(cx, |v, _| assert_eq!(v.sticky_focus(), Some(1)));
    // Enter：跳到聚焦的那一行，焦点退回编辑器。
    run(&view, cx, "editor.action.goToFocusedStickyScrollLine");
    view.read_with(cx, |v, _| {
        assert!(!v.sticky_focused());
        assert_eq!(v.selection().active.line.0, class);
    });
    // Esc：焦点退回编辑器。
    run(&view, cx, "editor.action.focusStickyScroll");
    view.read_with(cx, |v, _| assert!(v.sticky_focused()));
    run(&view, cx, "editor.action.selectEditor");
    view.read_with(cx, |v, _| assert!(!v.sticky_focused()));
}

/// `stickyScrollFocused` 的三条键位压过正文的 `↑` / `↓`，且与 `treestickyScrollFocused` 互不影响。
#[test]
fn sticky_keybindings_do_not_collide_with_the_tree_key() {
    let service = KeybindingService::with_builtin_defaults();
    let table = service.table();
    let commands = rove_ui::keys::default_commands().unwrap();
    let registry = commands.registry();
    let context = |keys: &[&str]| -> ContextSnapshot {
        keys.iter()
            .map(|k| (Arc::from(*k), Value::Bool(true)))
            .collect()
    };
    let hit = |key: &str, keys: &[&str]| -> Option<String> {
        match table.resolve(&parse_keybinding(key).unwrap(), &context(keys), registry) {
            KeyResolution::Command(i) => Some(i.id.to_string()),
            _ => None,
        }
    };
    // 焦点在粘性区：`↑` / `↓` / `Enter` / `Esc` 全走粘性滚动。
    let sticky = &["editorTextFocus", "textInputFocus", "stickyScrollFocused"];
    assert_eq!(
        hit("down", sticky).as_deref(),
        Some("editor.action.selectNextStickyScrollLine")
    );
    assert_eq!(
        hit("up", sticky).as_deref(),
        Some("editor.action.selectPreviousStickyScrollLine")
    );
    assert_eq!(
        hit("enter", sticky).as_deref(),
        Some("editor.action.goToFocusedStickyScrollLine")
    );
    assert_eq!(
        hit("escape", sticky).as_deref(),
        Some("editor.action.selectEditor")
    );
    // 没有这个键时还是正文的光标移动。
    assert_eq!(
        hit("down", &["editorTextFocus", "textInputFocus"]).as_deref(),
        Some("cursorDown")
    );
    // 列表的 `treestickyScrollFocused` 是另一个键：不会触发编辑器的粘性滚动命令。
    let tree = &["listFocus", "treestickyScrollFocused"];
    for key in ["down", "up", "enter"] {
        let id = hit(key, tree).unwrap_or_default();
        assert!(
            !id.contains("StickyScrollLine"),
            "{key} 在树的粘性区里解析成了 {id}"
        );
    }
}

/// `editor.action.toggleStickyScroll` 切换开关并写回 `editor.stickyScroll.enabled`。
#[gpui::test]
fn toggle_command_writes_the_setting_back(cx: &mut TestAppContext) {
    let text = csharp_5000();
    let symbols = symbols_of(&text, "csharp");
    let (view, cx, settings) = open(cx, &text, Some("csharp"), vec![]);
    view.update(cx, |v, cx| v.set_symbols(Some(symbols), cx));
    settle(cx);
    let class = line_of(&text, "class Component2");
    let method = line_after(&text, class, "void Method1(");
    scroll_to(&view, cx, method + 3);
    assert_eq!(sticky(&view, cx).len(), 3);

    run(&view, cx, "editor.action.toggleStickyScroll");
    assert!(sticky(&view, cx).is_empty());
    view.read_with(cx, |v, _| {
        assert!(!v.sticky_enabled());
        assert_eq!(v.sticky_height(), 0.0);
    });
    assert_eq!(
        settings.written.lock().unwrap().as_slice(),
        [(
            "editor.stickyScroll.enabled".to_string(),
            Value::Bool(false)
        )]
    );
    // 再切回来：粘性行回来。
    run(&view, cx, "editor.action.toggleStickyScroll");
    settle(cx);
    assert_eq!(sticky(&view, cx).len(), 3);
    assert_eq!(settings.written.lock().unwrap().len(), 2);
}

// ------------------------------------------------------------------ 4. 级别与登记

/// 大文件与超大文件关闭粘性滚动，关闭原因可以被查询到（#22 的状态栏悬停提示）。
#[gpui::test]
fn large_and_huge_documents_turn_sticky_scroll_off(cx: &mut TestAppContext) {
    let text = csharp_5000();
    let symbols = symbols_of(&text, "csharp");
    let (view, cx, _) = open(cx, &text, Some("csharp"), vec![]);
    view.update(cx, |v, cx| v.set_symbols(Some(symbols), cx));
    settle(cx);
    let class = line_of(&text, "class Component5");
    let method = line_after(&text, class, "void Method3(");
    scroll_to(&view, cx, method + 3);
    assert_eq!(sticky(&view, cx).len(), 3);
    view.read_with(cx, |v, _| {
        assert!(v.disabled_features().is_empty());
    });
    for level in [SizeClass::Large, SizeClass::Huge] {
        view.update(cx, |v, cx| v.set_document_level(level, cx));
        scroll_to(&view, cx, method + 3);
        view.read_with(cx, |v, _| {
            assert!(!v.sticky_enabled(), "{level:?}");
            assert_eq!(v.sticky_height(), 0.0);
            assert!(
                v.disabled_features().contains(&Feature::StickyScroll),
                "{level:?} 的关闭原因要能查到"
            );
        });
        assert!(sticky(&view, cx).is_empty());
    }
    // 关闭原因的界面字符串经登记机制注册。
    let (_, _, _, strings) = registries();
    assert!(strings.get(Feature::StickyScroll.label_key()).is_some());
}

/// 命令、设置项、颜色键、界面字符串都经由登记机制加入，深色与浅色主题都能取到五个颜色键。
#[test]
fn commands_settings_colors_and_strings_are_registered() {
    let (settings, colors, commands, strings) = registries();
    for key in [
        "editor.stickyScroll.enabled",
        "editor.stickyScroll.maxLineCount",
        "editor.stickyScroll.defaultModel",
        "editor.stickyScroll.scrollWithEditor",
    ] {
        assert!(settings.get(key).is_some(), "{key}");
    }
    for id in [
        "editor.action.toggleStickyScroll",
        "editor.action.focusStickyScroll",
        "editor.action.selectNextStickyScrollLine",
        "editor.action.selectPreviousStickyScrollLine",
        "editor.action.goToFocusedStickyScrollLine",
        "editor.action.selectEditor",
    ] {
        let command = commands.get(id).unwrap_or_else(|| panic!("{id} 未登记"));
        assert!(strings.get(&command.title_key).is_some(), "{id} 的标题");
        assert!(rove_ui::keys::action_for(id).is_some(), "{id} 的 action");
    }
    let keys = [
        "editorStickyScroll.background",
        "editorStickyScroll.border",
        "editorStickyScroll.shadow",
        "editorStickyScrollGutter.background",
        "editorStickyScrollHover.background",
    ];
    for key in keys {
        assert!(colors.get(key).is_some(), "{key}");
    }
    // 深色与浅色主题都解析得出（`border` 在两套默认主题里都不设色，取 `None`）。
    for kind in [ThemeKind::Dark, ThemeKind::Light] {
        let (_, colors, _, _) = registries();
        let theme = DefaultTheme { colors, kind };
        let palette = code_view::colors::Palette::resolve(&theme, kind);
        assert!(palette.sticky_hover_background.is_some(), "{kind:?} 悬停色");
        assert_eq!(
            Some(palette.sticky_background),
            theme.color("editorStickyScroll.background").unwrap(),
            "{kind:?} 背景取自登记的键"
        );
    }
    // 用户主题覆盖登记的键时以主题为准。
    let mut custom = ColorRegistry::default();
    let red = Some(Color(255, 0, 0, 255));
    custom
        .register(
            "editorStickyScroll.background",
            ColorDefaults {
                dark: red,
                light: red,
                high_contrast_dark: red,
                high_contrast_light: red,
            },
        )
        .unwrap();
    code_view::colors::register(&mut custom).unwrap();
    let theme = DefaultTheme {
        colors: custom,
        kind: ThemeKind::Dark,
    };
    let palette = code_view::colors::Palette::resolve(&theme, ThemeKind::Dark);
    assert_eq!(Some(palette.sticky_background), red);
}
