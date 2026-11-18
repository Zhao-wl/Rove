//! 括号对着色与活动括号对（#57 第一部分）的入口集成测试：打开一份 `.cs` 之后
//! 断言某一行各括号的着色深度、注释与字符串里的括号不参与配对、`editor.matchBrackets`
//! 的三个取值行为不变、按文档级别与括号数上限关闭，以及设置项 / 颜色键 / 界面字符串都经过登记。
//!
//! 运行：`cargo test -p rove-ui --test code_view_brackets`

use gpui::{Context, Entity, Focusable, TestAppContext, VisualTestContext, Window};
use rove_base::*;
use rove_buffer::{RopeText, SizeClass, TextSource};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings, SettingsChange};
use rove_syntax::{Fold, Symbol, SyntaxSnapshot, Token};
use rove_theme::{Color, ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{self, selection::pos, CodeView, CodeViewServices, Feature, Selection};
use std::sync::Arc;

// ------------------------------------------------------------------ 夹具

struct Overrides(DefaultSettings, Vec<(&'static str, Value)>);
impl Settings for Overrides {
    fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
        match self.1.iter().find(|(k, _)| *k == key) {
            Some((_, v)) => Ok(v.clone()),
            None => self.0.read(key, scope),
        }
    }
    fn write(&self, k: Arc<str>, s: Scope, v: Value, c: Cancellation) -> Task<()> {
        self.0.write(k, s, v, c)
    }
    fn changes(&self) -> &dyn Events<SettingsChange> {
        self.0.changes()
    }
}

fn registries() -> (
    SettingRegistry,
    ColorRegistry,
    rove_commands::CommandRegistry,
    rove_i18n::StringTable,
) {
    let mut settings = SettingRegistry::default();
    let mut colors = code_view::colors::theme_registry();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    rove_buffer::edit::settings::register(&mut settings).unwrap();
    (settings, colors, commands, strings)
}

fn services(overrides: Vec<(&'static str, Value)>) -> CodeViewServices {
    let (settings, colors, _, _) = registries();
    CodeViewServices {
        settings: Arc::new(Overrides(DefaultSettings(settings), overrides)),
        theme: Arc::new(DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        }),
        theme_kind: ThemeKind::Dark,
    }
}

/// 极简的 C# token：`//` 之后是注释，成对的 `"` 之间是字符串。
/// 真实运行时这份数据来自 #9 的 tree-sitter 集成；这里只需要同样的 scope。
struct FakeSyntax(Arc<dyn TextSource>);

impl SyntaxSnapshot for FakeSyntax {
    fn version(&self) -> Version {
        self.0.version()
    }
    fn tokens(&self, line: Line) -> Result<Arc<[Token]>> {
        let range = self.0.line_range(line)?;
        let mut text = String::new();
        self.0
            .visit_chunks(range.clone(), &mut |c| text.push_str(c))?;
        let base = range.start().0;
        let tok = |a: usize, b: usize, scope: &'static str| Token {
            range: Span::new(ByteOffset(base + a as u64), ByteOffset(base + b as u64)).unwrap(),
            scopes: Arc::from([Arc::from("source.cs"), Arc::from(scope)]),
        };
        let mut out = Vec::new();
        let comment = text.find("//").unwrap_or(text.len());
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < comment {
            if bytes[i] == b'"' {
                let end = text[i + 1..comment]
                    .find('"')
                    .map_or(comment, |j| i + 1 + j + 1);
                out.push(tok(i, end, "string.quoted.double.cs"));
                i = end;
                continue;
            }
            i += 1;
        }
        if comment < text.len() {
            out.push(tok(comment, text.len(), "comment.line.double-slash.cs"));
        }
        out.sort_by_key(|t| t.range.start());
        Ok(out.into())
    }
    fn folds(&self) -> Arc<[Fold]> {
        Arc::from([])
    }
    fn symbols(&self) -> Arc<[Symbol]> {
        Arc::from([])
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    overrides: Vec<(&'static str, Value)>,
    with_syntax: bool,
) -> (Entity<CodeView>, &'a mut VisualTestContext) {
    let source: Arc<dyn TextSource> = Arc::new(RopeText::from_text(text));
    let syntax: Arc<dyn SyntaxSnapshot> = Arc::new(FakeSyntax(source.clone()));
    let s = services(overrides);
    let (view, cx) = cx.add_window_view(move |_: &mut Window, cx: &mut Context<CodeView>| {
        let mut v = CodeView::new(s, cx);
        v.set_text(Some(source), false, cx);
        v.set_language(Some("csharp".into()), cx);
        if with_syntax {
            v.set_syntax(Some(syntax), None, cx);
        }
        v
    });
    cx.update(|window, cx| {
        rove_ui::keys::init(cx);
        window.focus(&view.read(cx).focus_handle(cx));
    });
    settle(cx);
    (view, cx)
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

fn caret(view: &Entity<CodeView>, cx: &mut VisualTestContext, line: u64, col: u64) {
    view.update(cx, |v, cx| {
        v.set_selection(Selection::caret(pos(line, col)), None, cx)
    });
    settle(cx);
}

/// 第 `line` 行上 `(列起点, 深度, 颜色键)`。
fn colors(
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
    line: u64,
) -> Vec<(u32, u32, &'static str)> {
    view.read_with(cx, |v, _| {
        v.bracket_colors(line)
            .into_iter()
            .map(|(range, level, key)| (range.start, level, key))
            .collect()
    })
}

/// 第 `line` 行 `range` 区间的合并前景色（装饰通道，验证着色确实提交到了 #19）。
fn foreground(
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
    line: u64,
    range: std::ops::Range<u32>,
) -> Option<Color> {
    view.read_with(cx, |v, _| {
        v.merged_decorations(line)?
            .runs
            .iter()
            .find(|r| r.range.start <= range.start && range.end <= r.range.end)
            .and_then(|r| r.style.foreground)
    })
}

const SOURCE: &str = concat!(
    "using System;\n",
    "namespace N\n",
    "{\n",
    "    public class C\n",
    "    {\n",
    "        public int M(int[] a) { return a[0] + (a[1] * 2); }\n",
    "        // ) } ] unbalanced in a comment\n",
    "        const string S = \"a { b ( c\";\n",
    "    }\n",
    "}\n",
);

// ------------------------------------------------------------------ 1. 打开 .cs 后的着色深度

#[gpui::test]
fn opening_a_cs_file_colors_brackets_by_depth(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SOURCE, vec![], true);
    // 第 5 行：`public int M(int[] a) { return a[0] + (a[1] * 2); }`
    // 它在 `namespace N { ... class C { ... } }` 里面，所以最外层是第 3 层。
    let line = colors(&view, cx, 5);
    assert_eq!(
        line,
        vec![
            // M(
            (20, 3, "editorBracketHighlight.foreground3"),
            // int[ ]
            (24, 4, "editorBracketHighlight.foreground4"),
            (25, 4, "editorBracketHighlight.foreground4"),
            // )
            (28, 3, "editorBracketHighlight.foreground3"),
            // 方法体 {
            (30, 3, "editorBracketHighlight.foreground3"),
            // a[0]
            (40, 4, "editorBracketHighlight.foreground4"),
            (42, 4, "editorBracketHighlight.foreground4"),
            // (a[1] * 2)
            (46, 4, "editorBracketHighlight.foreground4"),
            (48, 5, "editorBracketHighlight.foreground5"),
            (50, 5, "editorBracketHighlight.foreground5"),
            (55, 4, "editorBracketHighlight.foreground4"),
            // 方法体 }
            (58, 3, "editorBracketHighlight.foreground3"),
        ],
        "{line:?}"
    );
    // 着色确实经装饰通道提交，并且压过了 token 前景色。
    let depth3 = rove_theme::defaults::static_defaults("editorBracketHighlight.foreground3")
        .and_then(|d| d.for_kind(ThemeKind::Dark))
        .expect("深色主题有 foreground3");
    assert_eq!(foreground(&view, cx, 5, 20..21), Some(depth3));
}

#[gpui::test]
fn brackets_in_comments_and_strings_are_not_paired(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SOURCE, vec![], true);
    assert!(colors(&view, cx, 6).is_empty(), "注释里的括号不着色");
    assert!(colors(&view, cx, 7).is_empty(), "字符串里的括号不着色");
    // 没有它们捣乱，最后两行的 `}` 深度才是 2 和 1。
    assert_eq!(colors(&view, cx, 8)[0].1, 2);
    assert_eq!(colors(&view, cx, 9)[0].1, 1);
    // 全篇没有未配对的括号。
    for line in 0..10 {
        let hits = view.read_with(cx, |v, _| v.bracket_depths(line));
        assert!(
            hits.iter().all(|h| !h.unmatched),
            "第 {line} 行出现了未配对的括号：{hits:?}"
        );
    }
}

#[gpui::test]
fn without_tokens_the_tree_still_builds_and_recovers_when_they_arrive(cx: &mut TestAppContext) {
    // 解析未完成时先按已有内容着色（不闪烁），token 到达后重新配对。
    let (view, cx) = open(cx, SOURCE, vec![], false);
    assert!(
        !colors(&view, cx, 6).is_empty(),
        "没有 token 时注释里的括号也会被当成括号"
    );
    let source = view.read_with(cx, |v, _| v.text().unwrap().clone());
    let syntax: Arc<dyn SyntaxSnapshot> = Arc::new(FakeSyntax(source));
    view.update(cx, |v, cx| v.set_syntax(Some(syntax), None, cx));
    settle(cx);
    assert!(colors(&view, cx, 6).is_empty(), "token 到达后纠正");
}

// ------------------------------------------------------------------ 2. 设置项

#[gpui::test]
fn colorization_can_be_disabled(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        SOURCE,
        vec![("editor.bracketPairColorization.enabled", Value::Bool(false))],
        true,
    );
    assert!(colors(&view, cx, 5).is_empty());
    assert_eq!(foreground(&view, cx, 5, 20..21), None);
    // 活动括号对不受影响。
    caret(&view, cx, 5, 21);
    assert!(view.read_with(cx, |v, _| v.bracket_match().is_some()));
}

#[gpui::test]
fn independent_color_pool_per_bracket_type(cx: &mut TestAppContext) {
    let text = "class C { void M() { a[0] = (1); } }\n";
    let (view, cx) = open(cx, text, vec![], true);
    // 共用一个颜色池：`[`（第 22 列）在第 3 层。
    let shared = colors(&view, cx, 0);
    let hit = shared.iter().find(|c| c.0 == 22).expect("找得到 `[`");
    assert_eq!((hit.1, hit.2), (3, "editorBracketHighlight.foreground3"));

    let (view, cx) = open(
        cx,
        text,
        vec![(
            "editor.bracketPairColorization.independentColorPoolPerBracketType",
            Value::Bool(true),
        )],
        true,
    );
    // 每种括号各自从 1 开始：`[` 是第一对方括号，取 foreground1，深度本身不变。
    let pooled = colors(&view, cx, 0);
    let hit = pooled.iter().find(|c| c.0 == 22).expect("找得到 `[`");
    assert_eq!((hit.1, hit.2), (3, "editorBracketHighlight.foreground1"));
}

#[gpui::test]
fn user_can_override_the_colorized_pairs(cx: &mut TestAppContext) {
    let pair = |o: &str, c: &str| {
        Value::Array(Arc::from([Value::Array(Arc::from([
            Value::String(o.into()),
            Value::String(c.into()),
        ]))]))
    };
    let (view, cx) = open(
        cx,
        "class C { void M() { a[0]; } }\n",
        vec![("editor.language.colorizedBracketPairs", pair("[", "]"))],
        true,
    );
    let line = colors(&view, cx, 0);
    assert_eq!(
        line.iter().map(|c| c.0).collect::<Vec<_>>(),
        vec![22, 24],
        "只有方括号着色：{line:?}"
    );
}

// ------------------------------------------------------------------ 3. `editor.matchBrackets` 回归

/// 三个取值的语义必须和 #20 的就近扫描完全一样（判定换成了括号对树）。
#[gpui::test]
fn match_brackets_modes_are_unchanged(cx: &mut TestAppContext) {
    let text = "void M() { f(a, (b)); }\n";
    for (mode, column, expected) in [
        // 光标左边挨着 `(`。
        ("near", 13u64, Some((12u32..13, 19..20))),
        // 光标右边挨着 `)`。
        ("near", 18, Some((16..17, 18..19))),
        // 不挨着括号：near 不高亮。
        ("near", 15, None),
        // always 高亮包围光标的那一对。
        ("always", 15, Some((12..13, 19..20))),
        ("never", 13, None),
    ] {
        let (view, cx) = open(
            cx,
            text,
            vec![("editor.matchBrackets", Value::String(mode.into()))],
            true,
        );
        caret(&view, cx, 0, column);
        let m = view.read_with(cx, |v, _| v.bracket_match());
        assert_eq!(
            m.map(|m| (m.open.1, m.close.1)),
            expected,
            "matchBrackets = {mode}，列 {column}"
        );
    }
}

/// 换成括号对树之后，嵌套与未配对的情况比就近扫描更准。
#[gpui::test]
fn active_pair_is_more_accurate_than_the_nearest_scan(cx: &mut TestAppContext) {
    let text = "void M() { ) f(a, (b)); }\n";
    let (view, cx) = open(cx, text, vec![], true);
    // 多余的右括号旁边不高亮。
    caret(&view, cx, 0, 12);
    assert_eq!(view.read_with(cx, |v, _| v.bracket_match()), None);
    // 它后面那对括号照样配得上。
    caret(&view, cx, 0, 15);
    let m = view
        .read_with(cx, |v, _| v.bracket_match())
        .expect("有匹配");
    assert_eq!((m.open.1, m.close.1), (14..15, 21..22));
}

#[gpui::test]
fn highlight_active_bracket_pair_can_be_disabled(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        "void M() { }\n",
        vec![(
            "editor.guides.highlightActiveBracketPair",
            Value::Bool(false),
        )],
        true,
    );
    caret(&view, cx, 0, 7);
    assert_eq!(view.read_with(cx, |v, _| v.bracket_match()), None);
    // 着色不受影响。
    assert!(!colors(&view, cx, 0).is_empty());
}

/// 真实编辑（#47 的编辑入口）之后括号对树跟着增量更新。
#[gpui::test]
fn the_tree_follows_edits(cx: &mut TestAppContext) {
    use rove_buffer::edit::{EditBuffer, EditOptions, EditSession};
    use rove_buffer::{Eol, LineEndings};
    use std::cell::RefCell;
    use std::rc::Rc;

    let session = Rc::new(RefCell::new(EditSession::new(
        EditBuffer::new("a(b)\nc(d)\ne(f)\n", LineEndings::Uniform(Eol::Lf)),
        EditOptions::default(),
    )));
    let snapshot: Arc<dyn TextSource> = session.borrow().snapshot();
    let host: Rc<dyn rove_ui::code_view::EditHost> = session.clone();
    // 关掉自动闭合，这样输入的字符串就是最终文本。
    let s = services(vec![(
        "editor.autoClosingBrackets",
        Value::String("never".into()),
    )]);
    let (view, cx) = cx.add_window_view(move |_: &mut Window, cx: &mut Context<CodeView>| {
        let mut v = CodeView::new(s, cx);
        v.set_text(Some(snapshot), false, cx);
        v.set_language(Some("csharp".into()), cx);
        v.set_edit_host(Some(host), cx);
        v
    });
    cx.update(|window, cx| {
        rove_ui::keys::init(cx);
        window.focus(&view.read(cx).focus_handle(cx));
    });
    settle(cx);
    assert_eq!(colors(&view, cx, 1).len(), 2);
    assert_eq!(colors(&view, cx, 1)[0].1, 1);

    caret(&view, cx, 1, 2);
    cx.update(|window, cx| {
        view.update(cx, |v, cx| v.type_text("[x]", Some(window), cx));
    });
    settle(cx);
    // 第 1 行变成 `c([x]d)`：多了一对方括号，深度 2。
    let line = colors(&view, cx, 1);
    assert_eq!(
        line.iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(),
        vec![(1, 1), (2, 2), (4, 2), (6, 1)],
        "{line:?}"
    );
    assert!(view
        .read_with(cx, |v, _| v.bracket_depths(1))
        .iter()
        .all(|h| !h.unmatched));
    // 其余行不受影响。
    assert_eq!(colors(&view, cx, 2).len(), 2);
    assert_eq!(colors(&view, cx, 2)[0].1, 1);
}

// ------------------------------------------------------------------ 4. 关闭条件

#[gpui::test]
fn large_and_huge_files_turn_colorization_off(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, SOURCE, vec![], true);
    assert!(!colors(&view, cx, 5).is_empty());
    assert!(view.read_with(cx, |v, _| v.disabled_features()).is_empty());

    view.update(cx, |v, cx| v.set_document_level(SizeClass::Large, cx));
    settle(cx);
    assert!(colors(&view, cx, 5).is_empty(), "大文件关闭括号对着色");
    assert!(view
        .read_with(cx, |v, _| v.disabled_features())
        .contains(&Feature::BracketPairColorization));
    // 活动括号对回落到 #20 的就近扫描，仍然工作。
    caret(&view, cx, 5, 21);
    assert!(view.read_with(cx, |v, _| v.bracket_match().is_some()));

    view.update(cx, |v, cx| v.set_document_level(SizeClass::Huge, cx));
    settle(cx);
    assert!(colors(&view, cx, 5).is_empty());
    assert!(view
        .read_with(cx, |v, _| v.disabled_features())
        .contains(&Feature::BracketPairColorization));

    // 回到普通级别后恢复。
    view.update(cx, |v, cx| v.set_document_level(SizeClass::Normal, cx));
    settle(cx);
    assert!(!colors(&view, cx, 5).is_empty());
}

#[gpui::test]
fn too_many_brackets_turns_colorization_off_and_is_reported(cx: &mut TestAppContext) {
    let mut text = String::from("var x = new[] { ");
    for i in 0..400 {
        text.push_str(&format!("f({i}), "));
    }
    text.push_str("};\n");
    let (view, cx) = open(
        cx,
        &text,
        vec![(
            "rove.bracketPairColorization.maximumBrackets",
            Value::Number(100.0),
        )],
        true,
    );
    assert!(colors(&view, cx, 0).is_empty(), "整体关闭，而不是画错");
    assert!(view
        .read_with(cx, |v, _| v.disabled_features())
        .contains(&Feature::BracketPairColorization));
}

// ------------------------------------------------------------------ 5. 登记机制

#[gpui::test]
fn settings_colors_and_strings_are_registered(cx: &mut TestAppContext) {
    let _ = cx;
    let (settings, colors, _, strings) = registries();
    for key in [
        "editor.bracketPairColorization.enabled",
        "editor.bracketPairColorization.independentColorPoolPerBracketType",
        "editor.guides.highlightActiveBracketPair",
        "editor.language.brackets",
        "editor.language.colorizedBracketPairs",
        "rove.bracketPairColorization.maximumBrackets",
    ] {
        assert!(settings.get(key).is_some(), "设置项 {key} 未登记");
    }
    for key in code_view::bracket_pairs::FOREGROUND_KEYS
        .iter()
        .chain(std::iter::once(&code_view::bracket_pairs::UNEXPECTED_KEY))
    {
        assert!(colors.get(key).is_some(), "颜色键 {key} 未登记");
    }
    assert!(strings
        .get(Feature::BracketPairColorization.label_key())
        .is_some());
}
