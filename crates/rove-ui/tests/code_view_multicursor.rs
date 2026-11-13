//! 多光标、列选择与行操作的入口集成测试（#51）。只用 `rove_ui::code_view` 的公开接口，在 GPUI
//! 测试窗口里驱动；编辑入口是一个 `RefCell<EditSession>`（窗口里的宿主借出 #49 的 `TextFile`）。
//!
//! 运行：`cargo test -p rove-ui --test code_view_multicursor`

use gpui::{
    Context, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    TestAppContext, VisualTestContext, Window,
};
use rove_base::{ByteOffset, Span, Value};
use rove_buffer::edit::{EditBuffer, EditOptions, EditSession};
use rove_buffer::{Eol, LineEndings, ReadOnlyReason, TextSource};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings};
use rove_theme::{ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{
    self, selection::pos, CodeView, CodeViewEvent, CodeViewServices, EditHost, Reveal, Selection,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

// ------------------------------------------------------------------ 夹具

/// 只覆盖若干键的设置（`editor.multiCursorModifier` 等）。
struct Overrides(DefaultSettings, Vec<(&'static str, Value)>);

impl Settings for Overrides {
    fn read(&self, key: &str, scope: &Scope) -> rove_base::Result<Value> {
        match self.1.iter().find(|(k, _)| *k == key) {
            Some((_, v)) => Ok(v.clone()),
            None => self.0.read(key, scope),
        }
    }
    fn write(
        &self,
        k: Arc<str>,
        s: Scope,
        v: Value,
        c: rove_base::Cancellation,
    ) -> rove_base::Task<()> {
        self.0.write(k, s, v, c)
    }
    fn changes(&self) -> &dyn rove_base::Events<rove_config::SettingsChange> {
        self.0.changes()
    }
}

fn services(overrides: Vec<(&'static str, Value)>) -> CodeViewServices {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    rove_buffer::edit::settings::register(&mut settings).unwrap();
    CodeViewServices {
        settings: Arc::new(Overrides(DefaultSettings(settings), overrides)),
        theme: Arc::new(DefaultTheme {
            colors,
            kind: ThemeKind::Dark,
        }),
        theme_kind: ThemeKind::Dark,
    }
}

struct Fixture<'a> {
    view: Entity<CodeView>,
    session: Rc<RefCell<EditSession>>,
    events: Rc<RefCell<Vec<CodeViewEvent>>>,
    cx: &'a mut VisualTestContext,
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
    eol: LineEndings,
    overrides: Vec<(&'static str, Value)>,
) -> Fixture<'a> {
    let session = Rc::new(RefCell::new(EditSession::new(
        EditBuffer::new(text, eol),
        EditOptions::default(),
    )));
    let snapshot: Arc<dyn TextSource> = session.borrow().snapshot();
    let s = services(overrides);
    let host: Rc<dyn EditHost> = session.clone();
    let events: Rc<RefCell<Vec<CodeViewEvent>>> = Rc::default();
    let sink = events.clone();
    let (view, cx) = cx.add_window_view(move |_: &mut Window, cx: &mut Context<CodeView>| {
        let mut v = CodeView::new(s, cx);
        v.set_text(Some(snapshot), false, cx);
        v.set_edit_host(Some(host), cx);
        v
    });
    cx.update(|window, cx| {
        rove_ui::keys::init(cx);
        window.focus(&view.read(cx).focus_handle(cx));
        cx.subscribe(&view, move |_, e: &CodeViewEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach();
    });
    settle(cx);
    Fixture {
        view,
        session,
        events,
        cx,
    }
}

fn lf<'a>(text: &str, cx: &'a mut TestAppContext) -> Fixture<'a> {
    open(cx, text, LineEndings::Uniform(Eol::Lf), Vec::new())
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

impl Fixture<'_> {
    fn contents(&self) -> String {
        self.session.borrow().contents()
    }
    fn caret(&mut self, line: u64, col: u64) {
        self.view.update(self.cx, |v, cx| {
            v.set_selection(Selection::caret(pos(line, col)), None, cx)
        });
    }
    fn set(&mut self, selections: Vec<Selection>) {
        self.view
            .update(self.cx, |v, cx| v.set_selections(selections, None, cx));
    }
    fn selections(&mut self) -> Vec<Selection> {
        self.view.update(self.cx, |v, _| v.selections())
    }
    fn primary(&mut self) -> Selection {
        self.view.update(self.cx, |v, _| v.selection())
    }
    fn run(&mut self, id: &str) -> bool {
        let handled = self
            .cx
            .update(|window, cx| self.view.update(cx, |v, cx| v.execute(id, window, cx)));
        settle(self.cx);
        handled
    }
    fn click(&mut self, line: u64, col: u64, modifiers: Modifiers) {
        let target = self
            .view
            .update(self.cx, |v, _| v.point_for_position(pos(line, col)))
            .expect("位置在视口内");
        self.cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: target,
            modifiers,
            click_count: 1,
            first_mouse: false,
        });
        self.cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: target,
            modifiers,
            click_count: 1,
        });
        settle(self.cx);
    }
    fn drag(&mut self, from: (u64, u64), to: (u64, u64), modifiers: Modifiers) {
        let (a, b) = self.view.update(self.cx, |v, _| {
            (
                v.point_for_position(pos(from.0, from.1)),
                v.point_for_position(pos(to.0, to.1)),
            )
        });
        let (a, b) = (a.expect("起点在视口内"), b.expect("终点在视口内"));
        self.cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: a,
            modifiers,
            click_count: 1,
            first_mouse: false,
        });
        self.cx.simulate_event(gpui::MouseMoveEvent {
            position: b,
            pressed_button: Some(MouseButton::Left),
            modifiers,
        });
        self.cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: b,
            modifiers,
            click_count: 1,
        });
        settle(self.cx);
    }
    fn link_clicks(&self) -> usize {
        self.events
            .borrow()
            .iter()
            .filter(|e| matches!(e, CodeViewEvent::LinkClicked { .. }))
            .count()
    }
    fn edit_count(&self) -> usize {
        self.events
            .borrow()
            .iter()
            .filter(|e| matches!(e, CodeViewEvent::Edited(_)))
            .count()
    }
}

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Default::default()
    }
}

fn ctrl() -> Modifiers {
    Modifiers {
        control: true,
        ..Default::default()
    }
}

fn shift_alt() -> Modifiers {
    Modifiers {
        alt: true,
        shift: true,
        ..Default::default()
    }
}

fn carets(items: &[(u64, u64)]) -> Vec<Selection> {
    items
        .iter()
        .map(|(l, c)| Selection::caret(pos(*l, *c)))
        .collect()
}

// ------------------------------------------------------------------ 1. 添加与移除光标

#[gpui::test]
fn alt_click_adds_a_cursor_and_clicking_it_again_removes_it(cx: &mut TestAppContext) {
    let mut f = lf("one\ntwo\nthree\n", cx);
    f.caret(0, 1);
    assert!(!f.view.update(f.cx, |v, _| v.has_multiple_selections()));
    f.click(1, 2, alt());
    assert_eq!(f.selections(), carets(&[(1, 2), (0, 1)]), "新光标是主选区");
    assert!(f.view.update(f.cx, |v, _| v.has_multiple_selections()));
    f.click(2, 3, alt());
    assert_eq!(f.selections(), carets(&[(2, 3), (1, 2), (0, 1)]));
    // 再点同一个光标：删掉它（VS Code 行为）。
    f.click(1, 2, alt());
    assert_eq!(f.selections(), carets(&[(2, 3), (0, 1)]));
    // 普通点击回到单光标（列 0 会落在行号栏上，这里点行内）。
    f.click(0, 2, Modifiers::default());
    assert_eq!(f.selections(), carets(&[(0, 2)]));
    assert!(!f.view.update(f.cx, |v, _| v.has_multiple_selections()));
}

#[gpui::test]
fn alt_drag_adds_a_whole_selection(cx: &mut TestAppContext) {
    let mut f = lf("alpha\nbeta\n", cx);
    f.caret(0, 0);
    f.drag((1, 1), (1, 4), alt());
    assert_eq!(
        f.selections(),
        vec![
            Selection::new(pos(1, 1), pos(1, 4)),
            Selection::caret(pos(0, 0))
        ]
    );
}

#[gpui::test]
fn keyboard_adds_cursors_above_below_and_at_line_ends(cx: &mut TestAppContext) {
    let mut f = lf("one\ntwo\nthree\nfour\n", cx);
    f.caret(1, 1);
    f.run("editor.action.insertCursorBelow");
    assert_eq!(
        f.selections(),
        carets(&[(1, 1), (2, 1)]),
        "保留原光标、在它下面加一个（VS Code 的顺序，主选区不变）"
    );
    f.run("editor.action.insertCursorAbove");
    // 两个光标各向上复制一个：1、2 行上面多出 0、1 行，1 与原来的合并。
    let mut lines: Vec<u64> = f.selections().iter().map(|s| s.active.line.0).collect();
    lines.sort_unstable();
    assert_eq!(lines, vec![0, 1, 2]);
    // shift+alt+i：选区覆盖的每一行行尾一个光标。
    f.set(vec![Selection::new(pos(0, 0), pos(2, 2))]);
    f.run("editor.action.insertCursorAtEndOfEachLineSelected");
    let mut got: Vec<(u64, u64)> = f
        .selections()
        .iter()
        .map(|s| (s.active.line.0, s.active.column.0))
        .collect();
    got.sort_unstable();
    assert_eq!(got, vec![(0, 3), (1, 3), (2, 2)]);
    // escape 去掉次光标。
    assert!(f.run("removeSecondaryCursors"));
    assert_eq!(f.selections().len(), 1);
    // 没有次光标时这条命令不消费按键（`editorHasMultipleSelections` 不成立，escape 另有去处）。
    assert!(!f.run("removeSecondaryCursors"));
}

#[gpui::test]
fn escape_only_removes_secondary_cursors_when_the_context_key_is_set(cx: &mut TestAppContext) {
    let mut f = lf("one\ntwo\n", cx);
    f.set(vec![
        Selection::new(pos(0, 0), pos(0, 3)),
        Selection::caret(pos(1, 1)),
    ]);
    // `editorHasMultipleSelections && textInputFocus` 成立 → escape 解析成 removeSecondaryCursors。
    f.cx.simulate_keystrokes("escape");
    settle(f.cx);
    assert_eq!(f.selections().len(), 1);
    assert_eq!(
        f.primary(),
        Selection::new(pos(0, 0), pos(0, 3)),
        "主选区连同它的选中范围保留"
    );
    // 只剩一个选区时 escape 落到 cancelSelection，折叠成插入符。
    f.cx.simulate_keystrokes("escape");
    settle(f.cx);
    assert_eq!(f.primary(), Selection::caret(pos(0, 3)));
}

#[gpui::test]
fn multi_cursor_modifier_swaps_add_cursor_and_go_to_definition(cx: &mut TestAppContext) {
    // 默认 alt：Alt+点击加光标、Ctrl+点击跳转。
    {
        let mut f = lf("one\ntwo\nthree\n", cx);
        f.caret(0, 0);
        f.click(1, 1, alt());
        assert_eq!(f.selections().len(), 2, "Alt+点击加光标");
        assert_eq!(f.link_clicks(), 0, "Alt+点击不跳转");
        f.click(2, 1, ctrl());
        assert_eq!(f.selections().len(), 1, "Ctrl+点击只是移动插入符");
        assert_eq!(f.link_clicks(), 1, "Ctrl+点击触发转到定义");
    }
    // ctrlCmd：两者互换。
    let mut f = open(
        cx,
        "one\ntwo\nthree\n",
        LineEndings::Uniform(Eol::Lf),
        vec![(
            "editor.multiCursorModifier",
            Value::String("ctrlCmd".into()),
        )],
    );
    f.caret(0, 0);
    f.click(1, 1, ctrl());
    assert_eq!(f.selections().len(), 2, "ctrlCmd 下 Ctrl+点击加光标");
    assert_eq!(f.link_clicks(), 0, "ctrlCmd 下 Ctrl+点击不跳转");
    f.click(2, 1, alt());
    assert_eq!(f.selections().len(), 1);
    assert_eq!(f.link_clicks(), 1, "ctrlCmd 下 Alt+点击转到定义");
}

// ------------------------------------------------------------------ 2. 按匹配添加选区

#[gpui::test]
fn ctrl_d_selects_the_word_then_adds_matches_and_wraps(cx: &mut TestAppContext) {
    let mut f = lf("foo bar\nfoo baz\nqux foo\n", cx);
    f.caret(0, 1);
    // 第一次：选中光标处的单词。
    f.run("editor.action.addSelectionToNextFindMatch");
    assert_eq!(f.selections(), vec![Selection::new(pos(0, 0), pos(0, 3))]);
    // 第二次、第三次：依次加上后面的匹配。
    f.run("editor.action.addSelectionToNextFindMatch");
    assert_eq!(f.primary(), Selection::new(pos(1, 0), pos(1, 3)));
    assert_eq!(f.selections().len(), 2);
    f.run("editor.action.addSelectionToNextFindMatch");
    assert_eq!(f.primary(), Selection::new(pos(2, 4), pos(2, 7)));
    assert_eq!(f.selections().len(), 3);
    // 到达末尾：循环回到第一处，与已有选区合并，数量不变。
    f.run("editor.action.addSelectionToNextFindMatch");
    assert_eq!(f.selections().len(), 3, "全部选中后不再增加");
    // 向上：上一处匹配。
    let before = f.selections().len();
    f.run("editor.action.addSelectionToPreviousFindMatch");
    assert_eq!(f.selections().len(), before);
}

#[gpui::test]
fn ctrl_k_ctrl_d_moves_the_selection_instead_of_adding(cx: &mut TestAppContext) {
    let mut f = lf("foo bar\nfoo baz\nqux foo\n", cx);
    f.caret(0, 1);
    f.run("editor.action.addSelectionToNextFindMatch");
    assert_eq!(f.selections().len(), 1);
    f.run("editor.action.moveSelectionToNextFindMatch");
    assert_eq!(f.selections().len(), 1, "跳过当前匹配，不新增光标");
    assert_eq!(f.primary(), Selection::new(pos(1, 0), pos(1, 3)));
    f.run("editor.action.moveSelectionToPreviousFindMatch");
    assert_eq!(f.primary(), Selection::new(pos(0, 0), pos(0, 3)));
}

#[gpui::test]
fn select_highlights_and_change_all_select_every_occurrence(cx: &mut TestAppContext) {
    let mut f = lf("foo bar\nfoo baz\nfoobar foo\n", cx);
    f.caret(0, 1);
    f.run("editor.action.selectHighlights");
    // 整词匹配：`foobar` 不算。
    let mut got: Vec<(u64, u64)> = f
        .selections()
        .iter()
        .map(|s| (s.start().line.0, s.start().column.0))
        .collect();
    got.sort_unstable();
    assert_eq!(got, vec![(0, 0), (1, 0), (2, 7)]);
    // ctrl+f2：改全部——选中之后输入一次，三处一起改，一次撤销全部回退。
    f.caret(0, 1);
    f.run("editor.action.changeAll");
    assert_eq!(f.selections().len(), 3);
    f.cx.simulate_input("X");
    settle(f.cx);
    assert_eq!(f.contents(), "X bar\nX baz\nfoobar X\n");
    f.run("undo");
    assert_eq!(f.contents(), "foo bar\nfoo baz\nfoobar foo\n");
}

// ------------------------------------------------------------------ 3. 列选择

#[gpui::test]
fn column_select_commands_align_on_tabs_and_full_width_characters(cx: &mut TestAppContext) {
    // 第 0 行 Tab（tabSize 4）、第 1 行全角、第 2 行普通，长度不一。
    let mut f = lf("\tabcd\n中文ab\nxy\n", cx);
    f.caret(0, 1); // Tab 之后，可见列 4
    f.run("cursorColumnSelectRight");
    f.run("cursorColumnSelectRight");
    f.run("cursorColumnSelectDown");
    f.run("cursorColumnSelectDown");
    let mut got: Vec<(u64, u64, u64)> = f
        .selections()
        .iter()
        .map(|s| (s.start().line.0, s.start().column.0, s.end().column.0))
        .collect();
    got.sort_unstable();
    // 第 0 行：可见列 4..6 → Tab 之后的 "ab"（字节 1..3）；第 1 行：“中文”各占两列、各 3 字节，
    // 可见列 4..6 同样是 "ab"（字节 6..8）；第 2 行只有 2 列，整段在可见列 4 之前 → 被跳过。
    assert_eq!(got, vec![(0, 1, 3), (1, 6, 8)]);
}

#[gpui::test]
fn shift_alt_drag_makes_a_column_selection(cx: &mut TestAppContext) {
    let mut f = lf("abcdef\nabcdef\nabcdef\n", cx);
    f.caret(0, 0);
    f.drag((0, 1), (2, 4), shift_alt());
    let mut got: Vec<(u64, u64, u64)> = f
        .selections()
        .iter()
        .map(|s| (s.start().line.0, s.start().column.0, s.end().column.0))
        .collect();
    got.sort_unstable();
    assert_eq!(got, vec![(0, 1, 4), (1, 1, 4), (2, 1, 4)]);
}

#[gpui::test]
fn column_selection_mode_turns_shift_arrows_into_column_select(cx: &mut TestAppContext) {
    let mut f = open(
        cx,
        "abcdef\nabcdef\nabcdef\n",
        LineEndings::Uniform(Eol::Lf),
        vec![("editor.columnSelection", Value::Bool(true))],
    );
    f.caret(0, 1);
    // `editorColumnSelection && textInputFocus` 成立时 shift+down / shift+right 走列选择。
    f.cx.simulate_keystrokes("shift-down shift-right");
    settle(f.cx);
    let mut got: Vec<(u64, u64, u64)> = f
        .selections()
        .iter()
        .map(|s| (s.start().line.0, s.start().column.0, s.end().column.0))
        .collect();
    got.sort_unstable();
    assert_eq!(got, vec![(0, 1, 2), (1, 1, 2)]);
}

// ------------------------------------------------------------------ 4. 行操作

#[gpui::test]
fn line_operations_each_produce_one_undo_unit(cx: &mut TestAppContext) {
    let cases: &[(&str, &str, (u64, u64), &str)] = &[
        (
            "editor.action.moveLinesDownAction",
            "a\nb\nc\n",
            (0, 0),
            "b\na\nc\n",
        ),
        (
            "editor.action.moveLinesUpAction",
            "a\nb\nc\n",
            (1, 0),
            "b\na\nc\n",
        ),
        (
            "editor.action.copyLinesDownAction",
            "a\nb\n",
            (0, 0),
            "a\na\nb\n",
        ),
        (
            "editor.action.copyLinesUpAction",
            "a\nb\n",
            (0, 0),
            "a\na\nb\n",
        ),
        ("editor.action.deleteLines", "a\nb\nc\n", (1, 0), "a\nc\n"),
        (
            "editor.action.duplicateSelection",
            "a\nb\n",
            (0, 0),
            "a\na\nb\n",
        ),
        ("editor.action.joinLines", "a\nb\nc\n", (0, 0), "a b\nc\n"),
        (
            "editor.action.sortLinesAscending",
            "c\nb\na\n",
            (0, 0),
            "a\nb\nc\n",
        ),
        (
            "editor.action.sortLinesDescending",
            "a\nb\nc\n",
            (0, 0),
            "c\nb\na\n",
        ),
        ("editor.action.transpose", "abc\n", (0, 1), "bac\n"),
        ("editor.action.transposeLetters", "abc\n", (0, 2), "bac\n"),
        (
            "editor.action.trimTrailingWhitespace",
            "a   \nb\t\n",
            (0, 0),
            "a\nb\n",
        ),
        (
            "editor.action.transformToUppercase",
            "hello world\n",
            (0, 1),
            "HELLO world\n",
        ),
        (
            "editor.action.transformToLowercase",
            "HELLO world\n",
            (0, 1),
            "hello world\n",
        ),
        (
            "editor.action.transformToTitlecase",
            "hello\n",
            (0, 1),
            "Hello\n",
        ),
        (
            "editor.action.transformToCamelcase",
            "foo_bar\n",
            (0, 1),
            "fooBar\n",
        ),
        (
            "editor.action.transformToPascalcase",
            "foo_bar\n",
            (0, 1),
            "FooBar\n",
        ),
        (
            "editor.action.transformToSnakecase",
            "fooBar\n",
            (0, 1),
            "foo_bar\n",
        ),
        (
            "editor.action.transformToKebabcase",
            "fooBar\n",
            (0, 1),
            "foo-bar\n",
        ),
    ];
    for (id, before, caret, after) in cases {
        let mut f = lf(before, cx);
        f.caret(caret.0, caret.1);
        assert!(f.run(id), "{id}");
        assert_eq!(&f.contents(), after, "{id}");
        assert_eq!(f.edit_count(), 1, "{id}：一条命令只发一次编辑事件");
        f.run("undo");
        assert_eq!(&f.contents(), before, "{id}：一次撤销全部回退");
    }
}

#[gpui::test]
fn move_lines_stops_at_the_file_edges_and_moves_whole_selections(cx: &mut TestAppContext) {
    let mut f = lf("a\nb\nc", cx);
    f.caret(0, 0);
    f.run("editor.action.moveLinesUpAction");
    assert_eq!(f.contents(), "a\nb\nc", "首行上移什么也不做");
    f.caret(2, 0);
    f.run("editor.action.moveLinesDownAction");
    assert_eq!(f.contents(), "a\nb\nc", "末行下移什么也不做");
    // 选区跨多行：整块一起移动，选区跟着走。
    f.set(vec![Selection::new(pos(0, 0), pos(1, 1))]);
    f.run("editor.action.moveLinesDownAction");
    assert_eq!(f.contents(), "c\na\nb");
    assert_eq!(f.primary(), Selection::new(pos(1, 0), pos(2, 1)));
}

#[gpui::test]
fn line_operations_with_several_cursors_are_one_transaction(cx: &mut TestAppContext) {
    let mut f = lf("a\nb\nc\nd\ne\n", cx);
    f.set(carets(&[(1, 0), (3, 0)]));
    f.run("editor.action.deleteLines");
    assert_eq!(f.contents(), "a\nc\ne\n");
    assert_eq!(f.edit_count(), 1, "两个光标共用一个事务");
    f.run("undo");
    assert_eq!(f.contents(), "a\nb\nc\nd\ne\n");
}

#[gpui::test]
fn mixed_line_endings_survive_move_and_copy_byte_for_byte(cx: &mut TestAppContext) {
    // 第 0 行 CRLF、第 1 行 LF、第 2 行 CR、末行无终止符。
    let original = "aa\r\nbb\ncc\rdd";
    let mut f = open(
        cx,
        original,
        LineEndings::Mixed { preferred: Eol::Lf },
        Vec::new(),
    );
    f.caret(1, 0);
    f.run("editor.action.moveLinesDownAction");
    assert_eq!(
        f.contents(),
        "aa\r\ncc\rbb\ndd",
        "被移动的两行各自带着自己的换行符；其余字节不动"
    );
    f.run("undo");
    assert_eq!(f.contents(), original, "撤销逐字节回到原文");
    f.caret(0, 0);
    f.run("editor.action.copyLinesDownAction");
    assert_eq!(f.contents(), "aa\r\naa\r\nbb\ncc\rdd");
    assert!(
        f.contents().ends_with("bb\ncc\rdd"),
        "未改动的行尾一个字节都没变"
    );
}

// ------------------------------------------------------------------ 5. 多光标下的输入与剪贴板

#[gpui::test]
fn ten_cursors_type_one_character_and_a_single_undo_reverts_all(cx: &mut TestAppContext) {
    let text: String = (0..10).map(|i| format!("line{i}\n")).collect();
    let mut f = lf(&text, cx);
    f.set(carets(&(0..10).map(|i| (i, 4)).collect::<Vec<_>>()));
    assert_eq!(f.selections().len(), 10);
    f.cx.simulate_input("!");
    settle(f.cx);
    let expected: String = (0..10).map(|i| format!("line!{i}\n")).collect();
    assert_eq!(f.contents(), expected, "10 个光标同时插入");
    assert_eq!(f.edit_count(), 1);
    f.run("undo");
    assert_eq!(f.contents(), text, "一次撤销全部回退");
    assert_eq!(f.selections().len(), 10, "撤销后光标回到编辑之前");
}

#[gpui::test]
fn multi_cursor_paste_spreads_or_repeats(cx: &mut TestAppContext) {
    // spread：剪贴板行数与光标数相同时按行分配。
    let mut f = lf("a\nb\n", cx);
    f.set(carets(&[(0, 1), (1, 1)]));
    f.cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("X\nY\n".into())));
    f.run("editor.action.clipboardPasteAction");
    assert_eq!(f.contents(), "aX\nbY\n");

    // full：每个光标粘贴全文。
    let mut f = open(
        cx,
        "a\nb\n",
        LineEndings::Uniform(Eol::Lf),
        vec![("editor.multiCursorPaste", Value::String("full".into()))],
    );
    f.session
        .borrow_mut()
        .set_options(multi_cursor_paste_full_options());
    f.set(carets(&[(0, 1), (1, 1)]));
    f.cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("X\nY".into())));
    f.run("editor.action.clipboardPasteAction");
    assert_eq!(f.contents(), "aX\nY\nbX\nY\n");
}

/// `editor.multiCursorPaste = full` 的 #46 选项（粘贴的分配语义在会话里）。
fn multi_cursor_paste_full_options() -> EditOptions {
    let mut registry = SettingRegistry::default();
    rove_buffer::edit::settings::register(&mut registry).unwrap();
    EditOptions::read(
        &Overrides(
            DefaultSettings(registry),
            vec![("editor.multiCursorPaste", Value::String("full".into()))],
        ),
        &Scope::default(),
    )
}

#[gpui::test]
fn multi_cursor_limit_truncates_without_hanging(cx: &mut TestAppContext) {
    let text: String = (0..20).map(|i| format!("line{i}\n")).collect();
    let mut f = open(
        cx,
        &text,
        LineEndings::Uniform(Eol::Lf),
        vec![("editor.multiCursorLimit", Value::Number(3.0))],
    );
    f.set(carets(&(0..20).map(|i| (i, 0)).collect::<Vec<_>>()));
    assert_eq!(f.selections().len(), 3, "超过上限时按创建顺序截断");
    f.cx.simulate_input("#");
    settle(f.cx);
    assert!(f.contents().starts_with("#line0\n#line1\n#line2\nline3\n"));
}

#[gpui::test]
fn read_only_documents_reject_every_command(cx: &mut TestAppContext) {
    let original = "a\nb\nc\n";
    let mut f = lf(original, cx);
    f.view.update(f.cx, |v, cx| {
        v.set_read_only(Arc::from([ReadOnlyReason::Permissions]), cx)
    });
    f.set(carets(&[(0, 0), (1, 0)]));
    for id in [
        "editor.action.moveLinesDownAction",
        "editor.action.copyLinesDownAction",
        "editor.action.deleteLines",
        "editor.action.duplicateSelection",
        "editor.action.joinLines",
        "editor.action.sortLinesAscending",
        "editor.action.transpose",
        "editor.action.transposeLetters",
        "editor.action.trimTrailingWhitespace",
        "editor.action.transformToUppercase",
    ] {
        f.run(id);
        assert_eq!(f.contents(), original, "{id} 在只读文档上必须什么都不改");
    }
    assert_eq!(f.edit_count(), 0);
    let blocked = f
        .events
        .borrow()
        .iter()
        .filter(|e| matches!(e, CodeViewEvent::EditBlocked { .. }))
        .count();
    assert!(blocked >= 10, "每条命令都给出了只读提示");
    // 多光标与列选择本身（不改文本）在只读文档上照样可用。
    f.run("editor.action.insertCursorBelow");
    assert!(f.view.update(f.cx, |v, _| v.has_multiple_selections()));
}

// ------------------------------------------------------------------ 6. 绘制：1000 个光标的每帧 CPU

#[gpui::test]
fn a_thousand_cursors_do_not_blow_the_frame_budget(cx: &mut TestAppContext) {
    let text: String = (0..1200).map(|i| format!("line {i} of text\n")).collect();
    let mut f = lf(&text, cx);
    let frame_cpu = |f: &mut Fixture<'_>| {
        let mut total = std::time::Duration::ZERO;
        for _ in 0..20 {
            f.cx.update(|window, _| window.refresh());
            f.cx.run_until_parked();
            total += f.view.update(f.cx, |v, _| v.last_frame().cpu);
        }
        total / 20
    };
    f.caret(0, 0);
    let one = frame_cpu(&mut f);
    f.set(carets(&(0..1000).map(|i| (i, 2)).collect::<Vec<_>>()));
    assert_eq!(f.selections().len(), 1000);
    let many = frame_cpu(&mut f);
    // 只有视口里的光标参与绘制，所以 1000 个光标的每帧 CPU 与 1 个光标同量级。
    println!(
        "frame cpu：1 个光标 {:?} / 1000 个光标 {:?}（机器负载较高）",
        one, many
    );
    assert!(
        many <= one * 5 + std::time::Duration::from_millis(2),
        "1000 个光标的每帧 CPU 退化过大：{one:?} → {many:?}"
    );
    // 1000 个光标同时输入仍然是一个事务、一次撤销。
    f.cx.simulate_input("!");
    settle(f.cx);
    assert_eq!(f.edit_count(), 1);
    f.run("undo");
    assert_eq!(f.contents(), text);
}

// ------------------------------------------------------------------ 7. 选区接口

#[gpui::test]
fn set_selections_merges_overlapping_and_keeps_the_primary_first(cx: &mut TestAppContext) {
    let mut f = lf("hello world\n", cx);
    f.view.update(f.cx, |v, cx| {
        v.set_selections(
            vec![
                Selection::new(pos(0, 0), pos(0, 5)),
                Selection::new(pos(0, 3), pos(0, 8)),
                Selection::caret(pos(0, 10)),
            ],
            Some(Reveal::Nearest),
            cx,
        )
    });
    assert_eq!(
        f.selections(),
        vec![
            Selection::new(pos(0, 0), pos(0, 8)),
            Selection::caret(pos(0, 10))
        ],
        "重叠的两个选区合并成一个，主选区仍在下标 0"
    );
    // 单选区接口回到单光标。
    f.caret(0, 2);
    assert_eq!(f.selections(), vec![Selection::caret(pos(0, 2))]);
}

#[gpui::test]
fn arrow_keys_move_every_cursor(cx: &mut TestAppContext) {
    let mut f = lf("abcd\nabcd\nabcd\n", cx);
    f.set(carets(&[(0, 1), (2, 1)]));
    f.run("cursorRight");
    assert_eq!(f.selections(), carets(&[(0, 2), (2, 2)]));
    f.run("cursorDown");
    let mut lines: Vec<u64> = f.selections().iter().map(|s| s.active.line.0).collect();
    lines.sort_unstable();
    assert_eq!(lines, vec![1, 3]);
}

/// 行操作之后视图里显示的文本与会话一致（回归：视图必须换上新快照）。
#[gpui::test]
fn the_view_shows_the_text_after_a_line_operation(cx: &mut TestAppContext) {
    let mut f = lf("one\ntwo\nthree\n", cx);
    f.caret(0, 0);
    f.run("editor.action.moveLinesDownAction");
    let shown = f.view.update(f.cx, |v, _| {
        let t = v.text().unwrap();
        let mut out = String::new();
        t.visit_chunks(
            Span::new(ByteOffset(0), ByteOffset(t.byte_len())).unwrap(),
            &mut |c| out.push_str(c),
        )
        .unwrap();
        out
    });
    assert_eq!(shown, f.contents());
    assert_eq!(shown, "two\none\nthree\n");
}
