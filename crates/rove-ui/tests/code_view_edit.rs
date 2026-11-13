//! 代码视图编辑交互与输入法的入口集成测试（#47）。只用 `rove_ui::code_view` 的公开接口，在 GPUI
//! 测试窗口里驱动；编辑入口是一个 `RefCell<EditSession>`（窗口里的宿主借出 #49 的 `TextFile`）。
//!
//! 运行：`cargo test -p rove-ui --test code_view_edit`

use gpui::{
    Context, Entity, EntityInputHandler, Focusable, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, TestAppContext, VisualTestContext, Window,
};
use rove_base::*;
use rove_buffer::edit::{EditBuffer, EditOptions, EditSession, MoveWord, WordClassifier};
use rove_buffer::{Eol, LineEndings, ReadOnlyReason, TextSource};
use rove_config::{DefaultSettings, SettingRegistry};
use rove_theme::{ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{
    self, selection::pos, CodeView, CodeViewEvent, CodeViewServices, EditHost, Selection,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

// ------------------------------------------------------------------ 夹具

fn services() -> CodeViewServices {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    rove_buffer::edit::settings::register(&mut settings).unwrap();
    CodeViewServices {
        settings: Arc::new(DefaultSettings(settings)),
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

fn open<'a>(cx: &'a mut TestAppContext, text: &str, eol: LineEndings) -> Fixture<'a> {
    let session = Rc::new(RefCell::new(EditSession::new(
        EditBuffer::new(text, eol),
        EditOptions::default(),
    )));
    let snapshot: Arc<dyn TextSource> = session.borrow().snapshot();
    let s = services();
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
    open(cx, text, LineEndings::Uniform(Eol::Lf))
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
    fn shown(&mut self) -> String {
        self.view.update(self.cx, |v, _| {
            let t = v.text().unwrap();
            let mut out = String::new();
            t.visit_chunks(
                Span::new(ByteOffset(0), ByteOffset(t.byte_len())).unwrap(),
                &mut |c| out.push_str(c),
            )
            .unwrap();
            out
        })
    }
    fn select(&mut self, s: Selection) {
        self.view
            .update(self.cx, |v, cx| v.set_selection(s, None, cx));
    }
    fn caret(&mut self, line: u64, col: u64) {
        self.select(Selection::caret(pos(line, col)));
    }
    fn selection(&mut self) -> Selection {
        self.view.update(self.cx, |v, _| v.selection())
    }
    fn run(&mut self, id: &str) {
        self.cx.update(|window, cx| {
            self.view
                .update(cx, |v, cx| assert!(v.execute(id, window, cx), "{id}"));
        });
        settle(self.cx);
    }
    fn mark(&mut self, text: &str, cursor: usize) {
        let view = self.view.clone();
        self.cx.update(|window, cx| {
            view.update(cx, |v, cx| {
                v.replace_and_mark_text_in_range(None, text, Some(cursor..cursor), window, cx)
            })
        });
    }
    fn commit(&mut self, text: &str) {
        let view = self.view.clone();
        self.cx.update(|window, cx| {
            view.update(cx, |v, cx| v.replace_text_in_range(None, text, window, cx))
        });
        settle(self.cx);
    }
    fn edits(&self) -> Vec<rove_ui::code_view::EditNotice> {
        self.events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                CodeViewEvent::Edited(n) => Some(n.clone()),
                _ => None,
            })
            .collect()
    }
}

// ------------------------------------------------------------------ 入口：输入一个字符

#[gpui::test]
fn typing_a_character_edits_the_buffer_notifies_and_is_undoable(cx: &mut TestAppContext) {
    let mut f = lf("hello\nworld\n", cx);
    f.caret(0, 5);
    f.cx.simulate_input("!");
    settle(f.cx);
    assert_eq!(f.contents(), "hello!\nworld\n");
    assert_eq!(f.shown(), "hello!\nworld\n", "视图显示会话的新快照");
    assert_eq!(f.selection(), Selection::caret(pos(0, 6)));
    let edits = f.edits();
    assert_eq!(edits.len(), 1, "一次输入一个编辑事件");
    let notice = &edits[0].0;
    let combined = notice.combined.clone().unwrap();
    assert_eq!(
        (
            combined.range.start().0,
            combined.range.end().0,
            &*combined.inserted
        ),
        (5, 5, "!"),
        "交给语法增量解析的编辑与实际改动一致"
    );
    assert_eq!(combined.version, notice.before.version());
    assert_eq!(
        notice.after.version(),
        f.session.borrow().snapshot().version()
    );
    let ie = notice.edits[0].input_edits[0];
    assert_eq!(
        (ie.start_byte.0, ie.old_end_byte.0, ie.new_end_byte.0),
        (5, 5, 6)
    );
    // 视图里的行已经是新文本（没有占位）。
    assert_eq!(
        f.view.update(f.cx, |v, _| v.painted_text(0)).as_deref(),
        Some("hello!")
    );
    f.run("undo");
    assert_eq!(f.contents(), "hello\nworld\n");
    assert_eq!(f.shown(), "hello\nworld\n");
    assert_eq!(f.selection(), Selection::caret(pos(0, 5)));
    f.run("redo");
    assert_eq!(f.contents(), "hello!\nworld\n");
}

#[gpui::test]
fn enter_keeps_indentation_and_trims_auto_whitespace(cx: &mut TestAppContext) {
    let mut f = lf("    foo\n", cx);
    f.caret(0, 7);
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "    foo\n    \n");
    assert_eq!(f.selection(), Selection::caret(pos(1, 4)));
    // 只有自动缩进的空白行再回车：那一行清空（editor.trimAutoWhitespace）。
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "    foo\n\n    \n");
}

// ------------------------------------------------------------------ 编辑基础命令（参数化）

struct Case {
    id: &'static str,
    text: &'static str,
    select: (u64, u64, u64, u64),
    expect: &'static str,
    caret: (u64, u64),
}

const fn case(
    id: &'static str,
    text: &'static str,
    select: (u64, u64, u64, u64),
    expect: &'static str,
    caret: (u64, u64),
) -> Case {
    Case {
        id,
        text,
        select,
        expect,
        caret,
    }
}

const CASES: &[Case] = &[
    case("deleteLeft", "abc", (0, 2, 0, 2), "ac", (0, 1)),
    case("deleteLeft", "        x", (0, 6, 0, 6), "      x", (0, 4)),
    case("deleteRight", "abc", (0, 1, 0, 1), "ac", (0, 1)),
    case("deleteWordLeft", "foo bar", (0, 7, 0, 7), "foo ", (0, 4)),
    case("deleteWordRight", "foo bar", (0, 0, 0, 0), " bar", (0, 0)),
    case(
        "deleteWordStartLeft",
        "foo bar",
        (0, 7, 0, 7),
        "foo ",
        (0, 4),
    ),
    case("deleteWordEndRight", "foo bar", (0, 3, 0, 3), "foo", (0, 3)),
    case("deleteWordPartLeft", "fooBar", (0, 6, 0, 6), "foo", (0, 3)),
    case("deleteWordPartRight", "fooBar", (0, 0, 0, 0), "Bar", (0, 0)),
    case("deleteAllLeft", "abc def", (0, 4, 0, 4), "def", (0, 0)),
    case("deleteAllRight", "abc def", (0, 3, 0, 3), "abc", (0, 3)),
    case("deleteAllRight", "abc\ndef", (0, 3, 0, 3), "abcdef", (0, 3)),
    case("tab", "x", (0, 0, 0, 0), "    x", (0, 4)),
    case("tab", "ab", (0, 2, 0, 2), "ab  ", (0, 4)),
    case("tab", "a\nb\n", (0, 0, 1, 1), "    a\n    b\n", (1, 5)),
    case("outdent", "    a\n\tb", (0, 0, 1, 0), "a\n\tb", (1, 0)),
    case(
        "editor.action.outdentLines",
        "    a",
        (0, 4, 0, 4),
        "a",
        (0, 0),
    ),
    case(
        "editor.action.indentLines",
        "a",
        (0, 0, 0, 0),
        "    a",
        (0, 4),
    ),
    case("lineBreakInsert", "ab", (0, 1, 0, 1), "a\nb", (0, 1)),
    case(
        "editor.action.insertLineAfter",
        "  ab\nc",
        (0, 1, 0, 1),
        "  ab\n  \nc",
        (1, 2),
    ),
    case(
        "editor.action.insertLineBefore",
        "  ab",
        (0, 3, 0, 3),
        "  \n  ab",
        (0, 2),
    ),
    case(
        "editor.action.indentationToTabs",
        "    a\n        b",
        (0, 0, 0, 0),
        "\ta\n\t\tb",
        (0, 1),
    ),
    case(
        "editor.action.indentationToSpaces",
        "\ta",
        (0, 0, 0, 0),
        "    a",
        (0, 4),
    ),
];

#[gpui::test]
fn editing_basics_commands_follow_vscode(cx: &mut TestAppContext) {
    for c in CASES {
        let mut cx2 = cx.clone();
        let mut f = lf(c.text, &mut cx2);
        let (a, b, d, e) = c.select;
        f.select(Selection::new(pos(a, b), pos(d, e)));
        f.run(c.id);
        assert_eq!(f.contents(), c.expect, "{} on {:?}", c.id, c.text);
        assert_eq!(f.shown(), c.expect, "{}：视图同步", c.id);
        assert_eq!(
            f.selection().active,
            pos(c.caret.0, c.caret.1),
            "{} 的插入符",
            c.id
        );
    }
}

#[gpui::test]
fn indentation_conversion_is_one_undo_step(cx: &mut TestAppContext) {
    let mut f = lf("    a\n        b\n", cx);
    f.run("editor.action.indentationToTabs");
    assert_eq!(f.contents(), "\ta\n\t\tb\n");
    f.run("undo");
    assert_eq!(f.contents(), "    a\n        b\n");
}

#[gpui::test]
fn word_navigation_lands_where_the_core_model_says(cx: &mut TestAppContext) {
    // CJK、驼峰、下划线、emoji（含 ZWJ）混排行：界面层的落点与 #46 的对拍实现一致。
    let line = "fooBar foo_bar 中文abc 👨‍👩‍👧x  end";
    let mut f = lf(line, cx);
    let text: Arc<dyn TextSource> = f.session.borrow().snapshot();
    let classifier = WordClassifier::new(&EditOptions::default().word_separators);
    for (id, m) in [
        ("cursorWordRight", MoveWord::Right),
        ("cursorWordEndRight", MoveWord::EndRight),
        ("cursorWordStartRight", MoveWord::StartRight),
        ("cursorWordPartRight", MoveWord::PartRight),
    ] {
        let mut at = pos(0, 0);
        f.caret(0, 0);
        for _ in 0..12 {
            let expect =
                rove_buffer::edit::word::move_word(&*text, &classifier, at, m, false).unwrap();
            f.run(id);
            assert_eq!(f.selection().active, expect, "{id} 从 {at:?}");
            at = expect;
        }
    }
    for (id, m) in [
        ("cursorWordLeft", MoveWord::Left),
        ("cursorWordStartLeft", MoveWord::StartLeft),
        ("cursorWordEndLeft", MoveWord::EndLeft),
        ("cursorWordPartLeft", MoveWord::PartLeft),
    ] {
        let mut at = pos(0, line.len() as u64);
        f.caret(0, line.len() as u64);
        for _ in 0..12 {
            let expect =
                rove_buffer::edit::word::move_word(&*text, &classifier, at, m, false).unwrap();
            f.run(id);
            assert_eq!(f.selection().active, expect, "{id} 从 {at:?}");
            at = expect;
        }
    }
    // *Select 变体只移动 active。
    f.caret(0, 0);
    f.run("cursorWordRightSelect");
    let s = f.selection();
    assert_eq!(s.anchor, pos(0, 0));
    assert!(s.active.column.0 > 0);
    f.run("cursorLineEnd");
    assert_eq!(f.selection(), Selection::caret(pos(0, line.len() as u64)));
    f.run("cursorLineStartSelect");
    assert_eq!(
        f.selection(),
        Selection::new(pos(0, line.len() as u64), pos(0, 0))
    );
}

#[gpui::test]
fn cursor_undo_and_selection_anchor(cx: &mut TestAppContext) {
    let mut f = lf("one two three", cx);
    f.caret(0, 0);
    f.run("cursorWordEndRight");
    f.run("cursorWordEndRight");
    assert_eq!(f.selection().active, pos(0, 7));
    f.run("cursorUndo");
    assert_eq!(f.selection().active, pos(0, 3));
    f.run("cursorRedo");
    assert_eq!(f.selection().active, pos(0, 7));

    f.caret(0, 4);
    f.run("editor.action.setSelectionAnchor");
    assert_eq!(
        f.view.update(f.cx, |v, _| v.selection_anchor()),
        Some(pos(0, 4))
    );
    f.caret(0, 13);
    f.run("editor.action.selectFromAnchorToCursor");
    assert_eq!(f.selection(), Selection::new(pos(0, 4), pos(0, 13)));
    assert_eq!(f.view.update(f.cx, |v, _| v.selection_anchor()), None);
    f.run("editor.action.setSelectionAnchor");
    f.caret(0, 0);
    f.run("editor.action.goToSelectionAnchor");
    assert_eq!(f.selection(), Selection::caret(pos(0, 13)));
    f.run("editor.action.cancelSelectionAnchor");
    assert_eq!(f.view.update(f.cx, |v, _| v.selection_anchor()), None);
}

// ------------------------------------------------------------------ 剪贴板

#[gpui::test]
fn cut_and_paste_keep_mixed_line_endings(cx: &mut TestAppContext) {
    // 混合换行（主换行符 LF）：粘贴 CRLF 内容按主换行符归一，其余行尾不动（字节级比对）。
    let mut f = open(cx, "a\r\nb\nc\n", LineEndings::Mixed { preferred: Eol::Lf });
    f.select(Selection::new(pos(1, 0), pos(1, 1)));
    f.run("editor.action.clipboardCutAction");
    assert_eq!(f.contents(), "a\r\n\nc\n");
    let clip = f.cx.update(|_, cx| cx.read_from_clipboard()).unwrap();
    assert_eq!(clip.text().as_deref(), Some("b"));
    f.cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("x\r\ny".into())));
    f.caret(2, 1);
    f.run("editor.action.clipboardPasteAction");
    assert_eq!(f.contents().as_bytes(), b"a\r\n\ncx\ny\n");
    // 无选区剪切整行，粘贴时插到当前行之上。
    f.caret(0, 0);
    f.run("editor.action.clipboardCutAction");
    assert_eq!(f.contents().as_bytes(), b"\ncx\ny\n");
    f.caret(1, 0);
    f.run("editor.action.clipboardPasteAction");
    // 粘贴的整行按文档主换行符（LF）归一，其余行尾不动。
    assert_eq!(f.contents().as_bytes(), b"\na\ncx\ny\n");
}

// ------------------------------------------------------------------ 改写模式

#[gpui::test]
fn overtype_replaces_following_characters_but_not_the_line_break(cx: &mut TestAppContext) {
    let mut f = lf("abcd\nef", cx);
    f.run("editor.action.toggleOvertypeInsertMode");
    assert!(f.view.update(f.cx, |v, _| v.overtype()));
    f.caret(0, 1);
    f.cx.simulate_input("XY");
    settle(f.cx);
    assert_eq!(f.contents(), "aXYd\nef");
    f.cx.simulate_input("ZW");
    settle(f.cx);
    assert_eq!(f.contents(), "aXYZW\nef", "行尾不吞换行");
    // 粘贴遵循 editor.overtypeOnPaste（默认开）。
    f.caret(1, 0);
    f.cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("Q".into())));
    f.run("editor.action.clipboardPasteAction");
    assert_eq!(f.contents(), "aXYZW\nQf");
    // 输入法提交在改写模式下按整串覆盖（不是逐字符）。
    f.caret(0, 0);
    f.mark("ni", 2);
    f.commit("你好");
    assert_eq!(f.contents(), "你好YZW\nQf");
    f.run("editor.action.toggleOvertypeInsertMode");
    f.caret(0, 0);
    f.cx.simulate_input("1");
    settle(f.cx);
    assert_eq!(f.contents(), "1你好YZW\nQf");
}

// ------------------------------------------------------------------ 输入法

#[gpui::test]
fn composition_is_inline_and_commits_as_one_undo_step(cx: &mut TestAppContext) {
    let mut f = lf("前缀😀\n", cx);
    let end = "前缀😀".len() as u64;
    f.caret(0, end);
    f.mark("n", 1);
    f.mark("ni'h", 4);
    f.mark("ni'hao", 6);
    // 组合期间：缓冲区不变、没有事务、不进撤销栈、没有编辑事件。
    assert_eq!(f.contents(), "前缀😀\n");
    assert!(!f.session.borrow().can_undo());
    assert!(f.edits().is_empty());
    let c = f
        .view
        .update(f.cx, |v, _| v.composition().cloned())
        .unwrap();
    assert_eq!(
        (c.text.as_str(), c.cursor, c.start),
        ("ni'hao", 6, pos(0, end))
    );
    // 输入法看到的区域：组合串的 UTF-16 区间相对组合串，不按整篇文档换算。
    let view = f.view.clone();
    let (marked, selected) = f.cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            (
                v.marked_text_range(window, cx),
                v.selected_text_range(false, window, cx).map(|s| s.range),
            )
        })
    });
    assert_eq!(marked, Some(4..10), "“前缀”2 + 😀 2 个 UTF-16 单元之后");
    assert_eq!(selected, Some(10..10));
    // 候选框跟随：组合串内的光标位置有坐标。
    let bounds = f.cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.bounds_for_range(10..10, gpui::Bounds::default(), window, cx)
        })
    });
    assert!(bounds.is_some());
    f.commit("你好");
    assert_eq!(f.contents(), "前缀😀你好\n");
    assert_eq!(f.view.update(f.cx, |v, _| v.composition().cloned()), None);
    assert_eq!(f.selection(), Selection::caret(pos(0, end + 6)));
    assert_eq!(f.edits().len(), 1, "提交是一次编辑");
    f.run("undo");
    assert_eq!(f.contents(), "前缀😀\n", "一次撤销整串");
}

#[gpui::test]
fn cancelled_composition_leaves_nothing_and_does_not_delete_the_selection(cx: &mut TestAppContext) {
    let mut f = lf("abc", cx);
    f.select(Selection::new(pos(0, 0), pos(0, 1)));
    f.mark("x", 1);
    // Esc / 切换输入法：输入法发来空的组合串，再发一个空的提交。
    f.mark("", 0);
    f.commit("");
    assert_eq!(f.contents(), "abc");
    assert_eq!(f.view.update(f.cx, |v, _| v.composition().cloned()), None);
    assert!(f.edits().is_empty());
}

#[gpui::test]
fn clicking_elsewhere_mid_composition_commits_in_place(cx: &mut TestAppContext) {
    let mut f = lf("first\nsecond\n", cx);
    f.caret(0, 5);
    f.mark("zhong", 5);
    let target = f
        .view
        .update(f.cx, |v, _| v.point_for_position(pos(1, 3)))
        .unwrap();
    f.cx.simulate_event(MouseDownEvent {
        button: MouseButton::Left,
        position: target,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    f.cx.simulate_event(MouseUpEvent {
        button: MouseButton::Left,
        position: target,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    settle(f.cx);
    // 组合串在原位置提交（没有残片、不会重复提交），插入符到点击处。
    assert_eq!(f.contents(), "firstzhong\nsecond\n");
    assert_eq!(f.view.update(f.cx, |v, _| v.composition().cloned()), None);
    assert_eq!(f.selection(), Selection::caret(pos(1, 3)));
    // 输入法之后发来的取消（空串）不改任何东西。
    f.mark("", 0);
    f.commit("");
    assert_eq!(f.contents(), "firstzhong\nsecond\n");
}

#[gpui::test]
fn undo_during_composition_cancels_it_instead_of_undoing(cx: &mut TestAppContext) {
    let mut f = lf("", cx);
    f.cx.simulate_input("a");
    settle(f.cx);
    f.mark("ni", 2);
    f.run("undo");
    assert_eq!(f.contents(), "a", "不撤销上一次提交");
    assert_eq!(f.view.update(f.cx, |v, _| v.composition().cloned()), None);
}

// ------------------------------------------------------------------ 只读

#[gpui::test]
fn read_only_documents_reject_edits_with_a_reason(cx: &mut TestAppContext) {
    let mut f = lf("abc", cx);
    let reasons: Arc<[ReadOnlyReason]> = Arc::from([ReadOnlyReason::Rule(
        rove_workspace::REASON_READONLY_INCLUDE.into(),
    )]);
    f.view
        .update(f.cx, |v, cx| v.set_read_only(reasons.clone(), cx));
    f.caret(0, 1);
    f.cx.simulate_input("x");
    settle(f.cx);
    f.run("deleteLeft");
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "abc");
    let blocked = f
        .events
        .borrow()
        .iter()
        .filter(|e| matches!(e, CodeViewEvent::EditBlocked { reasons: r, .. } if **r == *reasons))
        .count();
    assert_eq!(blocked, 3, "输入、删除、回车各提示一次原因");
    // 输入法不进入组合态。
    f.mark("ni", 2);
    assert_eq!(f.view.update(f.cx, |v, _| v.composition().cloned()), None);
    // 单词导航不要求可编辑（when 是 textInputFocus）。
    f.run("cursorWordRight");
    assert_eq!(f.selection().active, pos(0, 3));
}
