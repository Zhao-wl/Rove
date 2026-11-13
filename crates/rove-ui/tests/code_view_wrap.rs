//! 软换行（#58）的入口集成测试：只用 `rove_ui::code_view` 的公开接口，在 GPUI 测试窗口里驱动。
//!
//! 运行：`cargo test -p rove-ui --test code_view_wrap -j 1`
//!
//! 测试窗口使用 GPUI 的测试文本系统：每个 BMP 字符宽 0.6 em，非 BMP 字符（emoji 等）宽 1.2 em。
//! 为了不依赖窗口宽度，大部分用例用 `editor.wordWrap: "wordWrapColumn"` 固定换行列。
//!
//! 「视觉行 ↔ 缓冲行」映射本身的单元测试（不依赖 GPUI）在
//! `crates/rove-ui/src/code_view/wrap.rs` 的 `tests` 里。

use gpui::{
    point, px, Context, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, TestAppContext,
    VisualTestContext, Window,
};
use rove_base::*;
use rove_buffer::{RopeText, SizeClass, TextSource};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings, SettingsChange};
use rove_theme::{ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{
    self, selection::pos, CodeView, CodeViewServices, Feature, Reveal, ScrollPosition,
};
use std::sync::Arc;
use std::sync::Mutex;

// ------------------------------------------------------------------ 测试夹具

/// 记录所有设置写入（`alt+z` 必须一次都不写）。
struct Overrides {
    base: DefaultSettings,
    values: Vec<(&'static str, Value)>,
    writes: Arc<Mutex<Vec<String>>>,
}

impl Settings for Overrides {
    fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
        match self.values.iter().find(|(k, _)| *k == key) {
            Some((_, v)) => Ok(v.clone()),
            None => self.base.read(key, scope),
        }
    }
    fn write(&self, k: Arc<str>, s: Scope, v: Value, c: Cancellation) -> Task<()> {
        self.writes.lock().unwrap().push(k.to_string());
        self.base.write(k, s, v, c)
    }
    fn changes(&self) -> &dyn Events<SettingsChange> {
        self.base.changes()
    }
}

fn services(
    overrides: Vec<(&'static str, Value)>,
    writes: Arc<Mutex<Vec<String>>>,
) -> CodeViewServices {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    CodeViewServices {
        settings: Arc::new(Overrides {
            base: DefaultSettings(settings),
            values: overrides,
            writes,
        }),
        theme: Arc::new(DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        }),
        theme_kind: ThemeKind::Dark,
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    overrides: Vec<(&'static str, Value)>,
) -> (Entity<CodeView>, &'a mut VisualTestContext) {
    let (view, cx, _) = open_recording(cx, text, overrides);
    (view, cx)
}

fn open_recording<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    overrides: Vec<(&'static str, Value)>,
) -> (
    Entity<CodeView>,
    &'a mut VisualTestContext,
    Arc<Mutex<Vec<String>>>,
) {
    let source: Arc<dyn TextSource> = Arc::new(RopeText::from_text(text));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let s = services(overrides, writes.clone());
    let (view, cx) = cx.add_window_view(move |_: &mut Window, cx: &mut Context<CodeView>| {
        let mut v = CodeView::new(s, cx);
        v.set_text(Some(source), false, cx);
        v
    });
    cx.update(|window, cx| {
        rove_ui::keys::init(cx);
        window.focus(&view.read(cx).focus_handle(cx));
    });
    settle(cx);
    (view, cx, writes)
}

/// 让空闲准备切片（含分帧重算换行点）全部跑完并重绘。
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..8 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

fn wrap_column(n: u32) -> Vec<(&'static str, Value)> {
    vec![
        ("editor.wordWrap", Value::String("wordWrapColumn".into())),
        ("editor.wordWrapColumn", Value::Number(n as f64)),
    ]
}

fn run(view: &Entity<CodeView>, cx: &mut VisualTestContext, id: &str) {
    view.update_in(cx, |v, window, cx| {
        assert!(v.execute(id, window, cx), "未识别的命令 {id}");
    });
    settle(cx);
}

/// 某条缓冲行占几条视觉行。
fn span(view: &Entity<CodeView>, cx: &mut VisualTestContext, line: u64) -> u32 {
    view.read_with(cx, |v, _| v.visual_map().span(line))
}

fn breaks(view: &Entity<CodeView>, cx: &mut VisualTestContext, line: u64) -> Vec<u32> {
    view.update(cx, |v, _| v.line_breaks(line))
}

// ------------------------------------------------------------------ 设置项逐项生效

/// 入口集成测试：开启软换行 → 视觉行数、换行点、命中测试都按视觉行工作。
#[gpui::test]
fn entry_wraps_long_lines_and_hit_tests_by_visual_line(cx: &mut TestAppContext) {
    let text = "alpha beta gamma delta epsilon zeta eta theta\nshort\n";
    let (view, cx) = open(cx, text, wrap_column(20));
    // 第 0 行按 20 列切成多段，第 1 行不换。
    assert!(span(&view, cx, 0) >= 3, "长行应当换行");
    assert_eq!(span(&view, cx, 1), 1);
    let total = view.read_with(cx, |v, _| v.visual_map().visual_count());
    assert_eq!(total, span(&view, cx, 0) as u64 + 2);
    // 上一帧画出来的视觉行：行 0 的续行段号递增，行号栏留空的就是这些行。
    let rows = view.read_with(cx, |v, _| v.visual_rows().to_vec());
    assert_eq!(rows[0], (0, 0));
    assert_eq!(rows[1], (0, 1));
    assert_eq!(rows[span(&view, cx, 0) as usize], (1, 0));
    assert_eq!(
        rows.iter().filter(|(l, s)| *l == 0 && *s == 0).count(),
        1,
        "一条缓冲行只有一条首视觉行会画行号"
    );

    // 续行的第一个字符在下一条视觉行、x 回到行首。
    let b = breaks(&view, cx, 0);
    let first = view
        .update(cx, |v, _| v.point_for_position(pos(0, 0)))
        .unwrap();
    let cont = view
        .update(cx, |v, _| v.point_for_position(pos(0, b[0] as u64)))
        .unwrap();
    let lh = view.read_with(cx, |v, _| v.line_height());
    assert!(
        (f32::from(cont.y - first.y) - lh).abs() < 0.5,
        "差一条视觉行"
    );
    assert!(f32::from(cont.x - first.x).abs() < 0.5, "续行从行首开始");

    // 命中测试：点续行的中间，落在续行的列上，不会落到首行。
    let at = point(cont.x + px(6.0), cont.y + px(lh / 2.0));
    let hit = view.update(cx, |v, _| v.position_for_point(at)).unwrap();
    assert_eq!(hit.line.0, 0);
    assert!(hit.column.0 >= b[0] as u64, "{hit:?} 应当落在续行里");
}

/// `editor.wordWrap` 的四个取值。
#[gpui::test]
fn word_wrap_modes(cx: &mut TestAppContext) {
    let text = format!("{}\n", "x".repeat(400));
    // off：不换行。
    let (view, cx) = open(cx, &text, vec![]);
    assert_eq!(span(&view, cx, 0), 1);
    assert!(view.read_with(cx, |v, _| !v.word_wrap_enabled()));

    // on：按视口宽度换行，段数与视口宽度 / 字符宽一致。
    let (view, cx) = open(
        cx,
        &text,
        vec![("editor.wordWrap", Value::String("on".into()))],
    );
    let (width, cell) = view.read_with(cx, |v, _| {
        (
            v.viewport().unwrap().width as f64,
            v.font_size() as f64 * 0.6,
        )
    });
    let columns = (width / cell).floor() as u32;
    assert_eq!(span(&view, cx, 0), 400_u32.div_ceil(columns));

    // wordWrapColumn：按设置的列换行，与视口宽度无关。
    let (view, cx) = open(cx, &text, wrap_column(50));
    assert_eq!(span(&view, cx, 0), 8);

    // bounded：取视口与 wordWrapColumn 的较小者。
    let (view, cx) = open(
        cx,
        &text,
        vec![
            ("editor.wordWrap", Value::String("bounded".into())),
            ("editor.wordWrapColumn", Value::Number(50.0)),
        ],
    );
    assert_eq!(span(&view, cx, 0), 8, "视口比 50 列宽，取 50");
}

/// `wrappingIndent` 四个取值、`wrappingStrategy` 两个取值、`wordBreak`、
/// `wrapOnEscapedLineFeeds`、`wordWrapIndicator` 逐项生效。
#[gpui::test]
fn wrapping_options_each_take_effect(cx: &mut TestAppContext) {
    let indented = format!("    {}\n", "ab ".repeat(40));
    let counts: Vec<u32> = ["none", "same", "indent", "deepIndent"]
        .iter()
        .map(|v| {
            let mut o = wrap_column(30);
            o.push(("editor.wrappingIndent", Value::String((*v).into())));
            let (view, cx) = open(cx, &indented, o);
            span(&view, cx, 0)
        })
        .collect();
    // 缩进越深，续行可用宽度越小，段数单调不减；且 deepIndent 严格多于 none。
    assert!(counts.windows(2).all(|w| w[0] <= w[1]), "{counts:?}");
    assert!(counts[3] > counts[0], "{counts:?}");

    // wrappingStrategy：simple 按等宽近似，advanced 按真实字形宽度。测试文本系统里
    // 非 BMP 字符宽 2 倍，两种策略对同一行给出不同的段数。
    let emoji = format!("{}\n", "🚀".repeat(60));
    let mut simple = wrap_column(30);
    simple.push(("editor.wrappingStrategy", Value::String("simple".into())));
    let (view, cx) = open(cx, &emoji, simple);
    let simple_count = span(&view, cx, 0);
    let mut advanced = wrap_column(30);
    advanced.push(("editor.wrappingStrategy", Value::String("advanced".into())));
    let (view, cx) = open(cx, &emoji, advanced);
    let advanced_count = span(&view, cx, 0);
    assert!(simple_count > 1 && advanced_count > 1);
    assert_ne!(
        simple_count, advanced_count,
        "真实字形宽度与等宽近似应当给出不同的切分（simple={simple_count}, advanced={advanced_count}）"
    );

    // wordBreak：keepAll 时 CJK 与相邻字符之间不断行。
    let cjk = "中文abcdef\n";
    let (view, cx) = open(cx, cjk, wrap_column(6));
    assert_eq!(breaks(&view, cx, 0), vec![6], "normal：在“文”之后断");
    let mut keep = wrap_column(6);
    keep.push(("editor.wordBreak", Value::String("keepAll".into())));
    let (view, cx) = open(cx, cjk, keep);
    assert_eq!(breaks(&view, cx, 0), vec![8], "keepAll：只能在溢出处硬断");

    // wrapOnEscapedLineFeeds：`\n` 字面量触发换行。
    let escaped = "a\\nb\\nc\n";
    let (view, cx) = open(cx, escaped, wrap_column(200));
    assert_eq!(span(&view, cx, 0), 1);
    let mut on = wrap_column(200);
    on.push(("editor.wrapOnEscapedLineFeeds", Value::Bool(true)));
    let (view, cx) = open(cx, escaped, on);
    assert_eq!(span(&view, cx, 0), 3);

    // wordWrapIndicator：开关只影响绘制，映射不变（开着也不会多算一条视觉行）。
    let mut ind = wrap_column(30);
    ind.push(("editor.wordWrapIndicator", Value::Bool(true)));
    let (view, cx) = open(cx, &indented, ind);
    let with = span(&view, cx, 0);
    let (view, cx) = open(cx, &indented, wrap_column(30));
    assert_eq!(with, span(&view, cx, 0));
}

// ------------------------------------------------------------------ 折叠、行号栏、滚动

/// 软换行 + 折叠：折叠后视觉行连续，展开后恢复。
#[gpui::test]
fn wrap_and_folding_compose(cx: &mut TestAppContext) {
    let long = "alpha beta gamma delta epsilon zeta eta theta iota";
    let text = format!("fn a() {{\n    {long}\n    {long}\n}}\nafter\n");
    let (view, cx) = open(cx, &text, wrap_column(24));
    let before: Vec<(u64, u32)> = view.read_with(cx, |v, _| {
        (0..v.visual_map().visual_count())
            .map(|l| v.visual_map().to_doc(l))
            .collect()
    });
    let spans: Vec<u32> = (0..5).map(|l| span(&view, cx, l)).collect();
    assert!(spans[1] > 1 && spans[2] > 1, "两条长行都换行了 {spans:?}");

    // 折叠 0..3（隐藏行 1、2 及其续行）。
    view.update(cx, |v, cx| {
        v.set_folds(&[LineRange::new(Line(0), Line(3)).unwrap()], cx)
    });
    settle(cx);
    let folded: Vec<(u64, u32)> = view.read_with(cx, |v, _| {
        (0..v.visual_map().visual_count())
            .map(|l| v.visual_map().to_doc(l))
            .collect()
    });
    assert_eq!(
        folded,
        vec![(0, 0), (3, 0), (4, 0), (5, 0)],
        "折叠后视觉行号连续，隐藏行的续行一并消失"
    );

    // 展开后恢复原来的序列。
    view.update(cx, |v, cx| v.set_folds(&[], cx));
    settle(cx);
    let after: Vec<(u64, u32)> = view.read_with(cx, |v, _| {
        (0..v.visual_map().visual_count())
            .map(|l| v.visual_map().to_doc(l))
            .collect()
    });
    assert_eq!(after, before);
}

/// `alt+z`（`editor.action.toggleWordWrap`）只影响当前编辑器会话，不写回设置；
/// 切换后视口仍停在同一条缓冲行上。
#[gpui::test]
fn toggle_word_wrap_is_session_only(cx: &mut TestAppContext) {
    let text: String = (0..400)
        .map(|i| format!("line {i} {}\n", "word ".repeat(80)))
        .collect();
    let (view, cx, writes) = open_recording(cx, &text, vec![]);
    assert!(view.read_with(cx, |v, _| !v.word_wrap_enabled()));
    view.update(cx, |v, cx| v.reveal_line(200, Reveal::Top, cx));
    settle(cx);
    let top_line = view.read_with(cx, |v, _| v.viewport().unwrap().first_line);
    assert_eq!(top_line, 200);

    run(&view, cx, "editor.action.toggleWordWrap");
    assert!(view.read_with(cx, |v, _| v.word_wrap_enabled()));
    assert_eq!(
        view.read_with(cx, |v, _| v.word_wrap_override()),
        Some(true)
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.viewport().unwrap().first_line),
        200,
        "切换后视口仍在同一条缓冲行上"
    );
    assert!(span(&view, cx, 200) > 1);

    run(&view, cx, "editor.action.toggleWordWrap");
    assert!(view.read_with(cx, |v, _| !v.word_wrap_enabled()));
    assert_eq!(
        view.read_with(cx, |v, _| v.viewport().unwrap().first_line),
        200
    );
    assert!(
        writes.lock().unwrap().is_empty(),
        "切换软换行不写回 settings.json，实际写了 {:?}",
        writes.lock().unwrap()
    );

    // 设置里开着时，alt+z 关掉它（同样不写回）。
    let (view, cx, writes) = open_recording(
        cx,
        &text,
        vec![("editor.wordWrap", Value::String("on".into()))],
    );
    assert!(view.read_with(cx, |v, _| v.word_wrap_enabled()));
    run(&view, cx, "editor.action.toggleWordWrap");
    assert!(view.read_with(cx, |v, _| !v.word_wrap_enabled()));
    assert!(writes.lock().unwrap().is_empty());
}

/// 滚动模型按视觉行：滚动条长度、`reveal_line`、水平滚动条都跟着换行走。
#[gpui::test]
fn scroll_model_counts_visual_lines(cx: &mut TestAppContext) {
    let text: String = (0..200)
        .map(|i| format!("line {i} {}\n", "word ".repeat(20)))
        .collect();
    let (view, cx) = open(cx, &text, wrap_column(40));
    let visual = view.read_with(cx, |v, _| v.visual_map().visual_count());
    assert!(
        visual > 400,
        "200 条长行应当占 400 条以上的视觉行：{visual}"
    );

    // 「哪些行的视觉行数变了」可以被下游取走（取过之后清空）。
    let changed = view.update(cx, |v, _| v.take_wrap_changes());
    assert!(changed.is_none() || !changed.unwrap().is_empty());
    assert_eq!(
        view.update(cx, |v, _| v.take_wrap_changes()),
        Some(Vec::new())
    );

    // 滚到某条缓冲行，视口首行就是它。
    view.update(cx, |v, cx| v.reveal_line(150, Reveal::Top, cx));
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.viewport().unwrap().first_line),
        150
    );
    // 软换行下没有横向滚动。
    view.update(cx, |v, cx| {
        v.set_scroll_position(
            ScrollPosition {
                top: 10.0,
                x: 500.0,
            },
            false,
            cx,
        )
    });
    settle(cx);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_position().x), 0.0);
}

// ------------------------------------------------------------------ 光标移动

/// `cursorUp` / `cursorDown` / `Home` / `End` / `PageUp` / `PageDown` 及 `*Select`
/// 按视觉行移动，`cursorLineStart` / `cursorLineEnd` 按缓冲行移动，期望列不漂移。
#[gpui::test]
fn caret_moves_by_visual_line(cx: &mut TestAppContext) {
    // 第 1 行换成多段，第 2 行很短（用来验证期望列不漂移）。
    let text = "first\naaaa bbbb cccc dddd eeee ffff\nxy\nlast line here\n";
    let (view, cx) = open(cx, text, wrap_column(10));
    let b = breaks(&view, cx, 1);
    assert!(b.len() >= 2, "第 1 行应当换成三段以上：{b:?}");

    // 从第 0 行下移：进入第 1 行的第一段。
    view.update(cx, |v, cx| {
        v.set_selection(code_view::Selection::caret(pos(0, 2)), None, cx)
    });
    run(&view, cx, "cursorDown");
    let p = view.read_with(cx, |v, _| v.selection().active);
    assert_eq!((p.line.0, p.column.0), (1, 2));
    // 再下移：还在第 1 行，但进了第二段（列落在续行里）。
    run(&view, cx, "cursorDown");
    let p = view.read_with(cx, |v, _| v.selection().active);
    assert_eq!(p.line.0, 1, "仍在同一条缓冲行");
    assert!(p.column.0 >= b[0] as u64, "进入了续行：{p:?}");
    // 上移回到第一段。
    run(&view, cx, "cursorUp");
    assert_eq!(view.read_with(cx, |v, _| v.selection().active.column.0), 2);

    // End / Home 按视觉行：End 落在本段末尾而不是整行末尾。
    run(&view, cx, "cursorEnd");
    let p = view.read_with(cx, |v, _| v.selection().active);
    assert!(p.column.0 < b[0] as u64 + 1 && p.column.0 > 2, "{p:?}");
    run(&view, cx, "cursorHome");
    assert_eq!(view.read_with(cx, |v, _| v.selection().active.column.0), 0);
    // cursorLineEnd 按**缓冲行**走，落在整行末尾。
    run(&view, cx, "cursorLineEnd");
    assert_eq!(
        view.read_with(cx, |v, _| v.selection().active.column.0),
        text.lines().nth(1).unwrap().len() as u64
    );

    // 期望列不漂移：经过短行再回到长行，列回到原来的位置。
    view.update(cx, |v, cx| {
        v.set_selection(code_view::Selection::caret(pos(1, 8)), None, cx)
    });
    let want = 8;
    for _ in 0..4 {
        run(&view, cx, "cursorDown");
    }
    let p = view.read_with(cx, |v, _| v.selection().active);
    assert_eq!(p.line.0, 3, "穿过第 1 行的续行与短行后到了第 3 行：{p:?}");
    assert_eq!(p.column.0, want, "期望列没有因为短行而漂移");

    // 从没换行的行下移到换行的行：落在它的**第一段**上，不会跳过续行。
    view.update(cx, |v, cx| {
        v.set_selection(code_view::Selection::caret(pos(0, 4)), None, cx)
    });
    run(&view, cx, "cursorDown");
    let p = view.read_with(cx, |v, _| v.selection().active);
    assert_eq!(p.line.0, 1);
    assert!(p.column.0 < b[0] as u64, "落在第一段里：{p:?}");

    // *Select 变体扩展选区。
    view.update(cx, |v, cx| {
        v.set_selection(code_view::Selection::caret(pos(1, 0)), None, cx)
    });
    run(&view, cx, "cursorDownSelect");
    let s = view.read_with(cx, |v, _| v.selection());
    assert_eq!(s.anchor, pos(1, 0));
    assert_eq!(s.active.line.0, 1);
    assert!(s.active.column.0 > 0, "向下扩选了一条视觉行");

    // PageDown / PageUp 也按视觉行（这里一页比全文还长，落到文末再回到文首）。
    run(&view, cx, "cursorPageDown");
    assert_eq!(
        view.read_with(cx, |v, _| v.selection().active.line.0),
        4,
        "一页比全文还长：落到最后一行（末尾换行符之后的空行）"
    );
    run(&view, cx, "cursorPageUp");
    assert_eq!(view.read_with(cx, |v, _| v.selection().active.line.0), 0);
}

// ------------------------------------------------------------------ 超长行与级别

/// `UX_SPEC` §3.3 的两个场景：单行 10 万字符、单行压缩 JSON。首屏要快。
#[gpui::test]
fn huge_single_line_files_open_fast(cx: &mut TestAppContext) {
    let long = format!("{}\n", "x".repeat(100_000));
    let started = std::time::Instant::now();
    let (view, cx) = open(cx, &long, wrap_column(120));
    let elapsed = started.elapsed();
    assert!(elapsed.as_millis() < 1000, "首屏耗时 {elapsed:?}");
    // 截断点（`editor.stopRenderingLineAfter` 默认 10000）之后不参与换行计算。
    assert_eq!(span(&view, cx, 0), 10_000_u32.div_ceil(120));

    let json = format!(
        "{{{}}}\n",
        (0..2000)
            .map(|i| format!("\"k{i}\":{i},"))
            .collect::<String>()
    );
    let started = std::time::Instant::now();
    let (view, cx) = open(cx, &json, wrap_column(100));
    assert!(started.elapsed().as_millis() < 1000);
    assert!(span(&view, cx, 0) > 1);
    // 压缩 JSON 在分隔符处断行，最后一段不为空。
    let b = breaks(&view, cx, 0);
    assert!(b.windows(2).all(|w| w[0] < w[1]));
}

/// 超大文件禁用软换行，原因可以查到。
#[gpui::test]
fn huge_files_disable_word_wrap(cx: &mut TestAppContext) {
    let text = format!("{}\nshort\n", "y".repeat(2000));
    let (view, cx) = open(cx, &text, wrap_column(40));
    assert!(view.read_with(cx, |v, _| v.word_wrap_enabled()));
    assert!(span(&view, cx, 0) > 1);

    view.update(cx, |v, cx| v.set_document_level(SizeClass::Huge, cx));
    settle(cx);
    assert!(view.read_with(cx, |v, _| !v.word_wrap_enabled()));
    assert_eq!(span(&view, cx, 0), 1);
    view.read_with(cx, |v, _| {
        assert!(
            v.disabled_features().contains(&Feature::WordWrap),
            "{:?}",
            v.disabled_features()
        );
        assert_eq!(Feature::WordWrap.label_key(), "codeView.feature.wordWrap");
    });
    // 超大文件下 alt+z 也不开。
    run(&view, cx, "editor.action.toggleWordWrap");
    assert!(view.read_with(cx, |v, _| !v.word_wrap_enabled()));

    // 大文件级别**不**禁用。
    view.update(cx, |v, cx| v.set_document_level(SizeClass::Large, cx));
    settle(cx);
    assert!(view.read_with(cx, |v, _| v.word_wrap_enabled()));
    assert!(span(&view, cx, 0) > 1);
}

/// 视口宽度变化时按帧重算换行点：重算期间仍按旧映射绘制，界面不卡死。
#[gpui::test]
fn viewport_width_change_recomputes_in_slices(cx: &mut TestAppContext) {
    let text: String = (0..20_000)
        .map(|i| format!("line {i} {}\n", "word ".repeat(12)))
        .collect();
    let (view, cx) = open(cx, &text, wrap_column(40));
    let before = view.read_with(cx, |v, _| v.visual_map().visual_count());
    assert!(before > 20_000);

    // 把换行列改小：整份映射重排。先确认“分帧”：一个切片之后还有剩余。
    view.update(cx, |v, cx| {
        let mut o = v.options().clone();
        o.wrap.column = 20;
        v.set_options(o, cx);
    });
    view.update_in(cx, |v, window, cx| {
        v.set_prepare_budget(std::time::Duration::from_micros(200));
        v.prepare_slice(window, cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let pending = view.read_with(cx, |v, _| v.wrap_pending_lines());
    assert!(pending > 0 || before > 0, "预算用完时应当留下未算的行");
    settle(cx);
    for _ in 0..200 {
        if view.read_with(cx, |v, _| v.wrap_pending_lines()) == 0 {
            break;
        }
        settle(cx);
    }
    let after = view.read_with(cx, |v, _| v.visual_map().visual_count());
    assert!(after > before, "换行列变小后视觉行更多：{before} → {after}");
    let (elapsed, lines) = view
        .read_with(cx, |v, _| v.wrap_recompute_stats())
        .expect("重算统计");
    assert!(lines >= 20_000, "重算覆盖了全部行：{lines}");
    eprintln!("20000 行重算换行点：{elapsed:?}（机器负载较高）");
}

/// 嵌入区域（#41 速览窗口）在软换行下落在缓冲行的最后一条视觉行之后。
#[gpui::test]
fn view_zones_follow_the_last_visual_line(cx: &mut TestAppContext) {
    let text = "short\nalpha beta gamma delta epsilon zeta\ntail\n";
    let (view, cx) = open(cx, text, wrap_column(12));
    let spans = span(&view, cx, 1);
    assert!(spans > 1);
    let lh = view.read_with(cx, |v, _| v.line_height());
    view.update(cx, |v, cx| v.set_view_zone(1, 3.0 * lh, cx));
    settle(cx);
    let zone = view
        .read_with(cx, |v, _| v.view_zone_bounds(1))
        .expect("嵌入区域可见");
    let last_row = view
        .update(cx, |v, _| {
            v.point_for_position(pos(1, text.lines().nth(1).unwrap().len() as u64))
        })
        .unwrap();
    assert!(
        (f32::from(zone.origin.y - last_row.y) - lh).abs() < 1.0,
        "嵌入区域紧跟在最后一条视觉行之下：zone={:?} last={:?}",
        zone.origin.y,
        last_row.y
    );
    // 其后的缓冲行被嵌入区域推下去。
    let tail = view
        .update(cx, |v, _| v.point_for_position(pos(2, 0)))
        .unwrap();
    assert!(f32::from(tail.y - zone.origin.y) >= 3.0 * lh - 1.0);
}

/// 鼠标点击（选区）在软换行下落在正确的视觉行与列上。
#[gpui::test]
fn mouse_selection_uses_visual_rows(cx: &mut TestAppContext) {
    let text = "aaaa bbbb cccc dddd eeee ffff gggg\nnext\n";
    let (view, cx) = open(cx, text, wrap_column(10));
    let b = breaks(&view, cx, 0);
    let target = pos(0, b[0] as u64 + 2);
    let at = view
        .update(cx, |v, _| v.point_for_position(target))
        .unwrap();
    let lh = view.read_with(cx, |v, _| v.line_height());
    cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: point(at.x + px(1.0), at.y + px(lh / 2.0)),
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    settle(cx);
    assert_eq!(view.read_with(cx, |v, _| v.selection().active), target);
}

/// 登记机制：命令、设置项、界面字符串都在注册表里，没有写死。
#[gpui::test]
fn command_setting_and_strings_are_registered(_cx: &mut TestAppContext) {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    for key in [
        "editor.wordWrap",
        "editor.wordWrapColumn",
        "editor.wordWrapIndicator",
        "editor.wrappingIndent",
        "editor.wrappingStrategy",
        "editor.wrapOnEscapedLineFeeds",
        "editor.wordBreak",
    ] {
        assert!(settings.get(key).is_some(), "{key} 没有登记");
    }
    assert_eq!(
        settings.get("editor.wordWrap").unwrap().default,
        Value::String("off".into())
    );
    assert_eq!(
        settings.get("editor.wordWrapColumn").unwrap().default,
        Value::Number(80.0)
    );
    let cmd = commands
        .get("editor.action.toggleWordWrap")
        .expect("命令没有登记");
    assert!(strings.get(&cmd.title_key).is_some());
    assert!(strings.get("codeView.feature.wordWrap").is_some());
}
