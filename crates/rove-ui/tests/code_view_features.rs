//! 代码视图 #20 的入口集成测试：语法着色与主题、括号匹配、同名高亮、折叠、缩进参考线与空白字符、
//! 按文档级别关闭功能。只用 `rove_ui::code_view` 的公开接口，在 GPUI 测试窗口里驱动。
//!
//! 运行：`cargo test -p rove-ui --test code_view_features`
//! 只读的界面表现（锁形图标、光标处提示、会话切换）在工作台一侧测：`cargo test -p rove-ui readonly_`。

use gpui::{Context, Entity, Focusable, TestAppContext, VisualTestContext, Window};
use rove_base::*;
use rove_buffer::{RopeText, SizeClass, TextSource};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings, SettingsChange};
use rove_syntax::{Fold, FoldKind, Symbol, SyntaxSnapshot, Token};
use rove_theme::{Color, ColorRegistry, DefaultTheme, Theme, ThemeKind, TokenStyle};
use rove_ui::code_view::{
    self, folding::FoldSource, selection::pos, CodeView, CodeViewEvent, CodeViewServices, Feature,
    ScrollPosition, Selection,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

const KEYWORD_DARK: Color = Color(0x56, 0x9c, 0xd6, 255);
const KEYWORD_LIGHT: Color = Color(0x00, 0x00, 0xff, 255);
const COMMENT: Color = Color(0x6a, 0x99, 0x55, 255);

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

/// 测试主题：`keyword` 粗体、`comment` 斜体加删除线，其余为前景色。
struct TestTheme {
    colors: DefaultTheme,
    keyword: Color,
}
impl Theme for TestTheme {
    fn color(&self, id: &str) -> Result<Option<Color>> {
        self.colors.color(id)
    }
    fn token_style(&self, scopes: &[Arc<str>]) -> TokenStyle {
        let last = scopes.last().map_or("", |s| s.as_ref());
        if last.starts_with("keyword") {
            TokenStyle {
                foreground: Some(self.keyword),
                bold: true,
                ..Default::default()
            }
        } else if last.starts_with("comment") {
            TokenStyle {
                foreground: Some(COMMENT),
                italic: true,
                strikethrough: true,
                ..Default::default()
            }
        } else {
            TokenStyle::default()
        }
    }
    fn changes(&self) -> &dyn Events<Version> {
        &NoEvents
    }
}

fn theme(keyword: Color, kind: ThemeKind) -> Arc<dyn Theme> {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    Arc::new(TestTheme {
        colors: DefaultTheme { colors, kind },
        keyword,
    })
}

fn services(overrides: Vec<(&'static str, Value)>) -> CodeViewServices {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    CodeViewServices {
        settings: Arc::new(Overrides(DefaultSettings(settings), overrides)),
        theme: theme(KEYWORD_DARK, ThemeKind::Dark),
        theme_kind: ThemeKind::Dark,
    }
}

/// 伪造的语法快照：按正则式的简单规则产出 token（`keyword`：`class`/`public`/`void`；`comment`：`//` 起）
/// 和给定的语法折叠区间。
struct FakeSyntax {
    text: Arc<dyn TextSource>,
    version: Version,
    folds: Vec<(u64, u64)>,
    keywords: &'static [&'static str],
}
impl SyntaxSnapshot for FakeSyntax {
    fn version(&self) -> Version {
        self.version
    }
    fn tokens(&self, line: Line) -> Result<Arc<[Token]>> {
        let range = self.text.line_range(line)?;
        let mut s = String::new();
        self.text
            .visit_chunks(range.clone(), &mut |c| s.push_str(c))?;
        let base = range.start().0;
        let mut out = Vec::new();
        let tok = |a: usize, b: usize, scope: &str| Token {
            range: Span::new(ByteOffset(base + a as u64), ByteOffset(base + b as u64)).unwrap(),
            scopes: Arc::from([Arc::from("source.cs"), Arc::from(scope)]),
        };
        let code_end = s.find("//").unwrap_or(s.len());
        for kw in self.keywords {
            for (i, _) in s[..code_end].match_indices(kw) {
                out.push(tok(i, i + kw.len(), "keyword.other.cs"));
            }
        }
        if code_end < s.len() {
            out.push(tok(code_end, s.len(), "comment.line.cs"));
        }
        out.sort_by_key(|t| t.range.start());
        Ok(out.into())
    }
    fn folds(&self) -> Arc<[Fold]> {
        self.folds
            .iter()
            .map(|&(a, b)| Fold {
                lines: LineRange::new(Line(a), Line(b)).unwrap(),
                kind: FoldKind::Syntax,
            })
            .collect()
    }
    fn symbols(&self) -> Arc<[Symbol]> {
        Arc::from([])
    }
}

fn snapshot(text: &Arc<dyn TextSource>, keywords: &'static [&'static str]) -> Arc<FakeSyntax> {
    Arc::new(FakeSyntax {
        text: text.clone(),
        version: text.version(),
        folds: Vec::new(),
        keywords,
    })
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    language: Option<&str>,
    overrides: Vec<(&'static str, Value)>,
) -> (Entity<CodeView>, &'a mut VisualTestContext) {
    let source: Arc<dyn TextSource> = Arc::new(RopeText::from_text(text));
    let s = services(overrides);
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
    (view, cx)
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

fn text_of(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Arc<dyn TextSource> {
    view.read_with(cx, |v, _| v.text().unwrap().clone())
}

fn caret(view: &Entity<CodeView>, cx: &mut VisualTestContext, line: u64, col: u64) {
    view.update(cx, |v, cx| {
        v.set_selection(Selection::caret(pos(line, col)), None, cx)
    });
    settle(cx);
}

fn run(view: &Entity<CodeView>, cx: &mut VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        view.update(cx, |v, cx| assert!(v.execute(id, window, cx), "{id}"));
    });
    settle(cx);
}

/// 第 `line` 行 `range` 区间的合并样式。
fn style_at(
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
    line: u64,
    range: std::ops::Range<u32>,
) -> Option<code_view::TextStyle> {
    view.read_with(cx, |v, _| {
        v.merged_decorations(line)?
            .runs
            .iter()
            .find(|r| r.range.start <= range.start && range.end <= r.range.end)
            .map(|r| r.style)
    })
}

// ------------------------------------------------------------------ 1. 语法着色与主题

/// 5000 行 C#：快照在首帧之前交给视图，首个画出正文的帧就带样式（不出现先白后彩）。
#[gpui::test]
fn first_frame_is_highlighted(cx: &mut TestAppContext) {
    let body: String = (0..5000)
        .map(|i| format!("public class C{i} {{ }} // note {i}\n"))
        .collect();
    let source: Arc<dyn TextSource> = Arc::new(RopeText::from_text(&body));
    let syntax = snapshot(&source, &["public", "class"]);
    let s = services(vec![]);
    let (view, cx) = cx.add_window_view(move |_: &mut Window, cx: &mut Context<CodeView>| {
        let mut v = CodeView::new(s, cx);
        v.set_text(Some(source), false, cx);
        v.set_language(Some("csharp".into()), cx);
        v.set_syntax(Some(syntax), None, cx);
        v.record_frames(true);
        v
    });
    // 每一帧：只要某行已经画出正文（整形或占位），它的样式就已经在。
    for _ in 0..6 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            for line in 0..20 {
                if v.painted_text(line).is_some() {
                    let m = v.merged_decorations(line).expect("styled with text");
                    assert_eq!(m.runs[0].range, 0..6);
                    assert_eq!(m.runs[0].style.foreground, Some(KEYWORD_DARK));
                    assert_eq!(m.runs[0].style.bold, Some(true));
                }
            }
        });
    }
    let comment = style_at(&view, cx, 3, 21..25).unwrap();
    assert_eq!(comment.foreground, Some(COMMENT));
    assert_eq!(comment.italic, Some(true), "fontStyle 斜体");
    assert_eq!(comment.strikethrough, Some(COMMENT), "fontStyle 删除线");
    view.read_with(cx, |v, _| {
        assert!(v.is_line_shaped(0));
        assert_eq!(v.last_frame().placeholders, 0);
    });
}

/// 新快照带变化行集合时只有这些行失效；解析未完成（文本已换、快照未到）时沿用旧样式。
#[gpui::test]
fn only_changed_lines_are_invalidated(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        "class A\nclass B\nclass C\nclass D\n",
        Some("csharp"),
        vec![],
    );
    let text = text_of(&view, cx);
    view.update(cx, |v, cx| {
        v.set_syntax(Some(snapshot(&text, &["class"])), None, cx)
    });
    settle(cx);
    assert_eq!(
        view.update(cx, |v, _| v.take_syntax_invalidation()),
        Some(None),
        "首次结果整体替换"
    );
    for line in 0..4 {
        assert!(style_at(&view, cx, line, 0..5).is_some_and(|s| s.bold == Some(true)));
    }
    // 新快照只认 `C` 行为关键字（模拟编辑了第 2 行），变化行只有第 2 行。
    let next = Arc::new(FakeSyntax {
        text: text.clone(),
        version: text.version(),
        folds: Vec::new(),
        keywords: &["class C"],
    });
    let changed = [LineRange::new(Line(2), Line(3)).unwrap()];
    view.update(cx, |v, cx| v.set_syntax(Some(next), Some(&changed), cx));
    settle(cx);
    assert_eq!(
        view.update(cx, |v, _| v.take_syntax_invalidation()),
        Some(Some(std::iter::once(2..3).collect()))
    );
    // 第 0、1、3 行没有失效，仍是旧结果；第 2 行按新快照重算。
    for line in [0, 1, 3] {
        assert!(style_at(&view, cx, line, 0..5).is_some_and(|s| s.bold == Some(true)));
    }
    // `class C` 的 ` C` 只有新快照才认作关键字（`class` 部分可能被同名高亮切成另一段）。
    assert!(style_at(&view, cx, 2, 5..7).is_some_and(|s| s.bold == Some(true)));

    // 同一文档的新文本版本到达、快照还没跟上：旧样式继续绘制。
    let edited: Arc<dyn TextSource> = Arc::new(RopeText::from_text(
        "class A\nclass B\nclass C\nclass D\n// x\n",
    ));
    view.update(cx, |v, cx| v.set_text(Some(edited), true, cx));
    view.read_with(cx, |v, _| {
        assert!(v.syntax_snapshot().is_some());
    });
    settle(cx);
    assert!(style_at(&view, cx, 2, 5..7).is_some_and(|s| s.bold == Some(true)));
}

/// 主题切换：颜色与 fontStyle 立即更新，整形缓存不随切换增长。
#[gpui::test]
fn theme_switch_updates_colors_and_font_style(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "public class A {}\n", Some("csharp"), vec![]);
    let text = text_of(&view, cx);
    view.update(cx, |v, cx| {
        v.set_syntax(Some(snapshot(&text, &["public"])), None, cx)
    });
    settle(cx);
    assert_eq!(
        style_at(&view, cx, 0, 0..6).unwrap().foreground,
        Some(KEYWORD_DARK)
    );
    let entries = view.read_with(cx, |v, _| v.shape_cache_stats().entries);
    for i in 0..6 {
        let (c, k) = if i % 2 == 0 {
            (KEYWORD_LIGHT, ThemeKind::Light)
        } else {
            (KEYWORD_DARK, ThemeKind::Dark)
        };
        view.update(cx, |v, cx| v.set_theme(theme(c, k), k, cx));
        settle(cx);
        assert_eq!(style_at(&view, cx, 0, 0..6).unwrap().foreground, Some(c));
        assert_eq!(view.read_with(cx, |v, _| v.palette().background), {
            if matches!(k, ThemeKind::Light) {
                Color(255, 255, 255, 255)
            } else {
                Color(0x1e, 0x1e, 0x1e, 255)
            }
        });
    }
    let after = view.read_with(cx, |v, _| v.shape_cache_stats().entries);
    assert!(after <= entries.max(1), "{entries} → {after}");
}

/// `editor.maxTokenizationLineLength`：超长行按纯文本显示（[csharp] 默认 2500 由设置层给出，这里直接设）。
#[gpui::test]
fn long_lines_are_not_tokenized(cx: &mut TestAppContext) {
    let long = format!("class {}\nclass B\n", "x".repeat(100));
    let (view, cx) = open(
        cx,
        &long,
        Some("csharp"),
        vec![("editor.maxTokenizationLineLength", Value::Number(50.0))],
    );
    let text = text_of(&view, cx);
    view.update(cx, |v, cx| {
        v.set_syntax(Some(snapshot(&text, &["class"])), None, cx)
    });
    settle(cx);
    // 同名高亮仍可能给 `class` 加背景，但没有 token 样式。
    assert!(style_at(&view, cx, 0, 0..5).is_none_or(|s| s.bold.is_none()));
    assert!(style_at(&view, cx, 1, 0..5).is_some_and(|s| s.bold == Some(true)));
}

// ------------------------------------------------------------------ 2. 括号匹配

fn brackets(
    view: &Entity<CodeView>,
    cx: &mut VisualTestContext,
    line: u64,
    col: u64,
) -> Option<((u64, u32), (u64, u32))> {
    caret(view, cx, line, col);
    view.read_with(cx, |v, _| {
        v.bracket_match()
            .map(|m| ((m.open.0, m.open.1.start), (m.close.0, m.close.1.start)))
    })
}

#[gpui::test]
fn bracket_matching_csharp(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        "class A\n{\n    void M(int[] a) { }\n}\n",
        Some("csharp"),
        vec![],
    );
    // 光标在 `(` 右侧 / `)` 左侧。
    assert_eq!(brackets(&view, cx, 2, 11), Some(((2, 10), (2, 18))));
    assert_eq!(brackets(&view, cx, 2, 18), Some(((2, 10), (2, 18))));
    // 跨行的 `{ }`：光标在 `}` 右侧。
    assert_eq!(brackets(&view, cx, 3, 1), Some(((1, 0), (3, 0))));
    // 不挨着括号：always 高亮包围光标的一对。
    assert_eq!(brackets(&view, cx, 2, 6), Some(((1, 0), (3, 0))));
    // 高亮样式来自 editorBracketMatch.*。
    let s = style_at(&view, cx, 3, 0..1).unwrap();
    assert!(s.background.is_some() && s.border.is_some());
}

#[gpui::test]
fn bracket_matching_json_and_never(cx: &mut TestAppContext) {
    let (view, cx) = open(cx, "{\n  \"a\": [1, {\"b\": 2}]\n}\n", Some("json"), vec![]);
    assert_eq!(brackets(&view, cx, 1, 8), Some(((1, 7), (1, 19))));
    assert_eq!(brackets(&view, cx, 1, 20), Some(((1, 7), (1, 19))));
    assert_eq!(brackets(&view, cx, 0, 0), Some(((0, 0), (2, 0))));
    view.update(cx, |v, cx| {
        let mut o = v.options().clone();
        o.match_brackets = code_view::brackets::MatchBrackets::Never;
        v.set_options(o, cx);
    });
    assert_eq!(brackets(&view, cx, 1, 8), None);
}

#[gpui::test]
fn bracket_matching_xml(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        "<root>\n  <item a=\"1\"/>\n</root>\n",
        Some("xml"),
        vec![],
    );
    // `<` 左侧与 `>` 右侧。
    assert_eq!(brackets(&view, cx, 1, 2), Some(((1, 2), (1, 14))));
    assert_eq!(brackets(&view, cx, 1, 15), Some(((1, 2), (1, 14))));
    assert_eq!(brackets(&view, cx, 0, 6), Some(((0, 0), (0, 5))));
}

// ------------------------------------------------------------------ 3. 同名高亮

#[gpui::test]
fn occurrences_highlight_words_and_selection(cx: &mut TestAppContext) {
    let text = "int count = 0;\ncount++;\nrecount = count;\n";
    let (view, cx) = open(cx, text, Some("csharp"), vec![]);
    caret(&view, cx, 0, 6);
    view.read_with(cx, |v, _| {
        assert!(v.has_word_highlights());
        assert_eq!(v.occurrence_text().as_deref(), Some("count"));
    });
    let bg = view.read_with(cx, |v, _| v.palette().word_highlight_text_background);
    assert!(bg.is_some());
    // 第 2 行 `recount` 不是同一个词，`count` 是。
    assert_eq!(style_at(&view, cx, 1, 0..5).unwrap().background, bg);
    assert!(style_at(&view, cx, 2, 0..7).is_none_or(|s| s.background != bg));
    assert_eq!(style_at(&view, cx, 2, 10..15).unwrap().background, bg);
    // 光标在空白处：没有单词高亮。
    caret(&view, cx, 0, 11);
    view.read_with(cx, |v, _| assert!(!v.has_word_highlights()));

    // 选区相似匹配（不区分大小写，findOptions 默认）。
    view.update(cx, |v, cx| {
        v.set_selection(Selection::new(pos(0, 4), pos(0, 9)), None, cx)
    });
    settle(cx);
    let sel_bg = view.read_with(cx, |v, _| v.palette().selection_highlight_background);
    view.read_with(cx, |v, _| assert!(!v.has_word_highlights()));
    assert_eq!(style_at(&view, cx, 2, 2..7).unwrap().background, sel_bg);
}

#[gpui::test]
fn occurrences_highlight_off(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        "int count = count;\n",
        Some("csharp"),
        vec![("editor.occurrencesHighlight", Value::String("off".into()))],
    );
    caret(&view, cx, 0, 6);
    view.read_with(cx, |v, _| {
        assert!(!v.has_word_highlights());
        assert!(v.occurrence_text().is_none());
    });
    assert!(style_at(&view, cx, 0, 12..17).is_none_or(|s| s.background.is_none()));
}

// ------------------------------------------------------------------ 4. 折叠

const CS: &str = "\
using System;
#region Fields
int a;
int b;
#endregion
class A
{
    void M()
    {
        Call();
    }
    /* block
       comment */
    void N()
    {
        Call();
    }
}
";

fn folds(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Vec<(u64, u64)> {
    view.read_with(cx, |v, _| {
        v.folds().iter().map(|r| (r.start().0, r.end().0)).collect()
    })
}

fn open_cs(cx: &mut TestAppContext) -> (Entity<CodeView>, &mut VisualTestContext) {
    let (view, cx) = open(cx, CS, Some("csharp"), vec![]);
    view.read_with(cx, |v, _| assert!(v.folding_ready()));
    (view, cx)
}

#[gpui::test]
fn fold_sources_indentation_region_and_syntax(cx: &mut TestAppContext) {
    let (view, cx) = open_cs(cx);
    let ranges = view.read_with(cx, |v, _| {
        v.fold_ranges()
            .iter()
            .map(|r| (r.start, r.end, r.source))
            .collect::<Vec<_>>()
    });
    assert!(ranges.contains(&(1, 5, FoldSource::Region)), "{ranges:?}");
    assert!(
        ranges.contains(&(6, 17, FoldSource::Indentation)),
        "{ranges:?}"
    );
    assert!(
        ranges.contains(&(8, 10, FoldSource::Indentation)),
        "{ranges:?}"
    );
    assert!(
        ranges.contains(&(11, 13, FoldSource::Comment)),
        "{ranges:?}"
    );

    // 语法区间：快照给出时（auto 策略）取代缩进区间，标记区间保留。
    let text = text_of(&view, cx);
    let syntax = Arc::new(FakeSyntax {
        text: text.clone(),
        version: text.version(),
        folds: vec![(5, 18), (7, 11)],
        keywords: &[],
    });
    view.update(cx, |v, cx| v.set_syntax(Some(syntax), None, cx));
    settle(cx);
    let ranges = view.read_with(cx, |v, _| {
        v.fold_ranges()
            .iter()
            .map(|r| (r.start, r.end, r.source))
            .collect::<Vec<_>>()
    });
    assert!(ranges.contains(&(5, 18, FoldSource::Syntax)), "{ranges:?}");
    assert!(ranges.contains(&(7, 11, FoldSource::Syntax)), "{ranges:?}");
    assert!(ranges.contains(&(1, 5, FoldSource::Region)), "{ranges:?}");
    assert!(
        !ranges.iter().any(|r| r.2 == FoldSource::Indentation),
        "{ranges:?}"
    );

    // foldingStrategy = indentation：忽略语法区间。
    view.update(cx, |v, cx| {
        let mut o = v.options().clone();
        o.folding_strategy = code_view::folding::FoldingStrategy::Indentation;
        v.set_options(o, cx);
    });
    settle(cx);
    view.read_with(cx, |v, _| {
        assert!(!v
            .fold_ranges()
            .iter()
            .any(|r| r.source == FoldSource::Syntax));
    });
}

#[gpui::test]
fn fold_commands(cx: &mut TestAppContext) {
    let (view, cx) = open_cs(cx);
    let events = Rc::new(RefCell::new(Vec::new()));
    let e = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &CodeViewEvent, _| {
            if let CodeViewEvent::FoldsChanged(f) = ev {
                e.borrow_mut().push(f.len());
            }
        })
        .detach()
    });

    // fold / unfold / toggleFold：光标在方法体里，先折最内层，再往外。
    caret(&view, cx, 9, 8);
    run(&view, cx, "editor.fold");
    assert_eq!(folds(&view, cx), vec![(8, 10)]);
    view.read_with(cx, |v, _| {
        assert_eq!(v.selection().active.line.0, 8, "光标回到折叠首行");
        assert!(v.fold_map().is_hidden(9));
    });
    run(&view, cx, "editor.fold");
    assert_eq!(folds(&view, cx), vec![(6, 17), (8, 10)]);
    run(&view, cx, "editor.unfold");
    assert_eq!(folds(&view, cx), vec![(8, 10)]);
    // 光标已回到第 6 行（class 体首行）。
    run(&view, cx, "editor.toggleFold");
    assert_eq!(folds(&view, cx), vec![(6, 17), (8, 10)]);
    run(&view, cx, "editor.toggleFold");
    assert_eq!(folds(&view, cx), vec![(8, 10)]);
    assert!(!events.borrow().is_empty(), "折叠变化发出 FoldsChanged");

    // 递归：class 及其全部子区间。
    caret(&view, cx, 6, 0);
    run(&view, cx, "editor.unfoldAll");
    run(&view, cx, "editor.foldRecursively");
    assert_eq!(folds(&view, cx), vec![(6, 17), (8, 10), (11, 13), (14, 16)]);
    run(&view, cx, "editor.unfoldRecursively");
    assert!(folds(&view, cx).is_empty());
    run(&view, cx, "editor.toggleFoldRecursively");
    assert_eq!(folds(&view, cx).len(), 4);
    run(&view, cx, "editor.toggleFoldRecursively");
    assert!(folds(&view, cx).is_empty());

    // 全部折叠 / 展开。
    run(&view, cx, "editor.foldAll");
    assert_eq!(folds(&view, cx).len(), 5);
    run(&view, cx, "editor.unfoldAll");
    assert!(folds(&view, cx).is_empty());

    // 按级别：级别 1 是 #region 与 class，级别 2 是方法体与注释。光标所在区间除外。
    caret(&view, cx, 0, 0);
    run(&view, cx, "editor.foldLevel1");
    assert_eq!(folds(&view, cx), vec![(1, 5), (6, 17)]);
    run(&view, cx, "editor.unfoldAll");
    run(&view, cx, "editor.foldLevel2");
    assert_eq!(folds(&view, cx), vec![(8, 10), (11, 13), (14, 16)]);
    run(&view, cx, "editor.unfoldAll");
    for n in 3..=7 {
        run(&view, cx, &format!("editor.foldLevel{n}"));
        assert!(folds(&view, cx).is_empty(), "level {n}");
    }

    // 块注释、标记区间。
    run(&view, cx, "editor.foldAllBlockComments");
    assert_eq!(folds(&view, cx), vec![(11, 13)]);
    run(&view, cx, "editor.unfoldAll");
    run(&view, cx, "editor.foldAllMarkerRegions");
    assert_eq!(folds(&view, cx), vec![(1, 5)]);
    run(&view, cx, "editor.unfoldAllMarkerRegions");
    assert!(folds(&view, cx).is_empty());

    // 除所选之外。
    caret(&view, cx, 9, 0);
    run(&view, cx, "editor.foldAllExcept");
    assert_eq!(folds(&view, cx), vec![(1, 5), (11, 13), (14, 16)]);
    run(&view, cx, "editor.foldAll");
    caret(&view, cx, 0, 0);
    run(&view, cx, "editor.unfoldAllExcept");
    assert!(folds(&view, cx).is_empty());
    caret(&view, cx, 9, 0);
    run(&view, cx, "editor.foldAll");
    caret(&view, cx, 6, 0);
    run(&view, cx, "editor.unfoldAllExcept");
    assert_eq!(folds(&view, cx), vec![(6, 17)], "包含光标的区间保持原状");
    run(&view, cx, "editor.unfoldAll");

    // 手动区间：从选区创建（立即折叠），再删除。
    view.update(cx, |v, cx| {
        v.set_selection(Selection::new(pos(2, 0), pos(3, 6)), None, cx)
    });
    settle(cx);
    run(&view, cx, "editor.createFoldingRangeFromSelection");
    assert_eq!(folds(&view, cx), vec![(2, 4)]);
    view.read_with(cx, |v, _| {
        assert!(v
            .fold_ranges()
            .iter()
            .any(|r| (r.start, r.end, r.source) == (2, 4, FoldSource::Manual)));
    });
    caret(&view, cx, 2, 0);
    run(&view, cx, "editor.removeManualFoldingRanges");
    assert!(folds(&view, cx).is_empty());
    view.read_with(cx, |v, _| {
        assert!(!v
            .fold_ranges()
            .iter()
            .any(|r| r.source == FoldSource::Manual));
    });

    // 每个折叠命令都有默认键位，when 为 `editorTextFocus && foldingEnabled`。
    for id in code_view::folding::FOLD_COMMANDS {
        assert!(code_view::commands::action_for(id).is_some(), "{id}");
    }
}

/// 折叠后屏幕行与文档行不再一一对应：视口、命中测试与跳转都按映射换算，跳进折叠会先展开。
#[gpui::test]
fn folding_maps_screen_lines(cx: &mut TestAppContext) {
    let (view, cx) = open_cs(cx);
    caret(&view, cx, 6, 0);
    run(&view, cx, "editor.fold");
    view.read_with(cx, |v, _| {
        let vp = v.viewport().unwrap();
        assert_eq!(vp.first_line, 0);
        // 第 7–16 行（class 体）隐藏：屏幕上第 7 行是文档第 17 行。
        assert_eq!(v.fold_map().to_doc(7), 17);
        assert!(v.painted_text(17).is_some());
    });
    // 光标下移跳过折叠。
    run(&view, cx, "cursorDown");
    view.read_with(cx, |v, _| assert_eq!(v.selection().active.line.0, 17));
    // 跳到隐藏行：先展开。
    view.update(cx, |v, cx| {
        v.set_selection(
            Selection::caret(pos(9, 0)),
            Some(code_view::Reveal::Center),
            cx,
        )
    });
    settle(cx);
    assert!(folds(&view, cx).is_empty());
    // 已折叠首行的占位标记可点击展开（点击行尾之后、占位标记上）。
    caret(&view, cx, 8, 0);
    run(&view, cx, "editor.fold");
    let at = view
        .update(cx, |v, _| v.point_for_position(pos(8, 5)))
        .unwrap();
    let lh = view.read_with(cx, |v, _| v.line_height());
    let cw = view.read_with(cx, |v, _| v.font_size() * 0.6);
    let click = gpui::point(at.x + gpui::px(cw * 2.0), at.y + gpui::px(lh / 2.0));
    cx.simulate_click(click, gpui::Modifiers::default());
    settle(cx);
    assert!(folds(&view, cx).is_empty(), "点击占位标记展开");
}

/// `editor.folding` 关闭时没有 `foldingEnabled`，折叠命令不起作用。
#[gpui::test]
fn folding_disabled(cx: &mut TestAppContext) {
    let (view, cx) = open(
        cx,
        CS,
        Some("csharp"),
        vec![("editor.folding", Value::Bool(false))],
    );
    view.read_with(cx, |v, _| {
        assert!(!v.folding_enabled());
        assert!(v.fold_ranges().is_empty());
    });
    caret(&view, cx, 9, 0);
    run(&view, cx, "editor.fold");
    assert!(folds(&view, cx).is_empty());
}

// ------------------------------------------------------------------ 5. 缩进参考线与空白字符

/// 四种 `renderWhitespace` 取值（与参考线一起）都能绘制；具体位置由 `guides` 的单元测试断言。
#[gpui::test]
fn whitespace_and_guides_render(cx: &mut TestAppContext) {
    for mode in ["none", "boundary", "selection", "all", "trailing"] {
        let (view, cx) = open(
            cx,
            "class A\n{\n\tint  a; \n\n    void M() { }\n}\n",
            Some("csharp"),
            vec![("editor.renderWhitespace", Value::String(mode.into()))],
        );
        view.update(cx, |v, cx| {
            v.set_selection(Selection::new(pos(2, 0), pos(2, 9)), None, cx)
        });
        settle(cx);
        view.read_with(cx, |v, _| {
            let expected = match mode {
                "none" => code_view::guides::RenderWhitespace::None,
                "boundary" => code_view::guides::RenderWhitespace::Boundary,
                "selection" => code_view::guides::RenderWhitespace::Selection,
                "all" => code_view::guides::RenderWhitespace::All,
                _ => code_view::guides::RenderWhitespace::Trailing,
            };
            assert_eq!(v.options().render_whitespace, expected);
            assert!(v.options().guides_indentation);
            assert!(v.last_frame().glyphs > 0);
        });
    }
}

// ------------------------------------------------------------------ 7. 按级别关闭功能

#[gpui::test]
fn large_and_huge_levels(cx: &mut TestAppContext) {
    let (view, cx) = open_cs(cx);
    let text = text_of(&view, cx);
    let syntax = Arc::new(FakeSyntax {
        text: text.clone(),
        version: text.version(),
        folds: vec![(5, 18)],
        keywords: &["class"],
    });
    view.update(cx, |v, cx| {
        v.set_syntax(Some(syntax), None, cx);
        v.set_document_level(SizeClass::Large, cx);
    });
    caret(&view, cx, 5, 1);
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.disabled_features(),
            vec![
                Feature::OccurrencesHighlight,
                Feature::SyntaxFolding,
                Feature::BackgroundHighlighting
            ]
        );
        assert!(!v.has_word_highlights(), "大文件关闭同名高亮");
        assert!(v.folding_enabled(), "缩进折叠仍可用");
        assert!(v
            .fold_ranges()
            .iter()
            .all(|r| r.source != FoldSource::Syntax));
        assert!(v
            .fold_ranges()
            .iter()
            .any(|r| r.source == FoldSource::Indentation));
    });
    assert!(
        style_at(&view, cx, 5, 0..5).is_some_and(|s| s.bold == Some(true)),
        "高亮仍在"
    );

    view.update(cx, |v, cx| v.set_document_level(SizeClass::Huge, cx));
    settle(cx);
    view.read_with(cx, |v, _| {
        assert!(!v.folding_enabled());
        assert!(v.fold_ranges().is_empty());
        assert!(v.disabled_features().contains(&Feature::SyntaxHighlighting));
        assert!(v.disabled_features().contains(&Feature::Folding));
    });
    assert!(
        style_at(&view, cx, 5, 0..5).is_none_or(|s| s.bold.is_none()),
        "超大文件不着色"
    );
}

// ------------------------------------------------------------------ 折叠与滚动范围

/// 请求滚到很远处再读回被钳住的位置：就是代码视图当前认为的最大滚动量（`Metrics::max_top`，
/// 滚动条滑块与拖动定位都用同一份度量）。
fn max_top(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> f64 {
    let top = view.update(cx, |v, cx| {
        v.set_scroll_position(ScrollPosition { top: 1e12, x: 0.0 }, false, cx);
        v.scroll_position().top
    });
    view.update(cx, |v, cx| {
        v.set_scroll_position(ScrollPosition::default(), false, cx)
    });
    settle(cx);
    top
}

/// #22 的滚动行数估计不得抵消 #20 的折叠：滚动范围永远按**屏幕行**算。
///
/// 回归：工作台曾对所有文档（包括索引已建完的普通文件）无条件下发估计值，而估计值在索引建完时
/// 等于文档总行数，于是只要有折叠收起，内容高度就被抬回未折叠的行数——能滚过最后一行、
/// 滑块偏小、拖动滚动条定位的行与屏幕行对不上。
#[gpui::test]
fn folding_shrinks_the_scroll_range_even_with_a_line_estimate(cx: &mut TestAppContext) {
    let mut text = String::from("root\n");
    for i in 0..300 {
        text.push_str(&format!("    line {i}\n"));
    }
    let (view, cx) = open(cx, &text, None, vec![]);
    settle(cx);
    let doc_lines = view.read_with(cx, |v, _| v.text().unwrap().lines().known);

    let unfolded = max_top(&view, cx);
    assert!(unfolded > 0.0, "内容比视口高");

    run(&view, cx, "editor.foldAll");
    settle(cx);
    assert!(!folds(&view, cx).is_empty(), "缩进折叠已收起");
    let folded = max_top(&view, cx);
    assert!(folded < unfolded, "折叠后内容变短：{folded} < {unfolded}");

    // 索引建完后工作台不再下发估计值；即便下发，估计值也要经折叠映射，不能把隐藏行算回来。
    view.update(cx, |v, cx| v.set_scroll_line_estimate(Some(doc_lines), cx));
    settle(cx);
    assert_eq!(
        max_top(&view, cx),
        folded,
        "估计值不得把折叠隐藏的行算回滚动范围"
    );

    // 索引未建完：估计值超出已知行数的部分仍然扩大滚动范围（可以滚到未索引区域）。
    view.update(cx, |v, cx| {
        v.set_scroll_line_estimate(Some(doc_lines * 3), cx)
    });
    settle(cx);
    assert!(
        max_top(&view, cx) > folded,
        "未索引部分仍然按估计值扩大滚动范围"
    );
}
