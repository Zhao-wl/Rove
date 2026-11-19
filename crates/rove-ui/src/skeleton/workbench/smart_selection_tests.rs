//! 从工作台打开文档并经真实命令桥验收扩选与括号操作。

use super::*;
use crate::code_view::smart_selection as ids;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{ByteOffset, Span};
use rove_buffer::TextSource;

fn open<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
    source: &str,
) -> (
    tempfile::TempDir,
    Entity<Workbench>,
    &'a mut VisualTestContext,
) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join(name), source).unwrap();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Workbench::with_store(
            folder.path().into(),
            Instant::now(),
            LayoutStore::disabled(),
            cx,
        )
    });
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(name).unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    settle(cx);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    settle(cx);
    (folder, view, cx)
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|window, app| {
            window.refresh();
            app.refresh_windows();
        });
    }
    cx.run_until_parked();
}

fn caret(view: &Entity<Workbench>, cx: &mut VisualTestContext, offset: usize) -> Position {
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    code.update(cx, |code, cx| {
        let position = code
            .text()
            .unwrap()
            .position(ByteOffset(offset as u64))
            .unwrap();
        code.set_selection(crate::code_view::Selection::caret(position), None, cx);
        position
    })
}

fn selected(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> (String, crate::code_view::Selection) {
    view.read_with(cx, |view, app| {
        let code = view.code_view().read(app);
        let selection = code.selection();
        let text = code.text().unwrap();
        let span = Span::new(
            text.offset(selection.start()).unwrap(),
            text.offset(selection.end()).unwrap(),
        )
        .unwrap();
        let mut out = String::new();
        text.visit_chunks(span, &mut |chunk| out.push_str(chunk))
            .unwrap();
        (out, selection)
    })
}

fn shown_text(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |view, app| {
        let text = view.code_view().read(app).text().unwrap();
        let mut out = String::new();
        text.visit_chunks(
            Span::new(ByteOffset(0), ByteOffset(text.byte_len())).unwrap(),
            &mut |chunk| out.push_str(chunk),
        )
        .unwrap();
        out
    })
}

fn execute(view: &Entity<Workbench>, cx: &mut VisualTestContext, command: &str, args: Value) {
    view.update(cx, |view, cx| {
        view.run_invocation(
            Invocation {
                id: command.into(),
                args,
            },
            cx,
        )
    });
    settle(cx);
}

#[gpui::test]
fn smart_selection_keys_round_trip_the_real_csharp_ancestors(cx: &mut TestAppContext) {
    let source = "class C { int M() { return (value + 42) * 2; } }";
    let (_folder, view, cx) = open(cx, "Program.cs", source);
    let start = caret(&view, cx, source.find("value").unwrap() + 1);
    let expected = [
        "value",
        "value + 42",
        "(value + 42)",
        "(value + 42) * 2",
        "return (value + 42) * 2;",
        "{ return (value + 42) * 2; }",
        "int M() { return (value + 42) * 2; }",
        "{ int M() { return (value + 42) * 2; } }",
        source,
    ];
    for step in expected {
        cx.simulate_keystrokes("shift-alt-right");
        assert_eq!(selected(&view, cx).0, step);
    }
    for step in [
        "{ int M() { return (value + 42) * 2; } }",
        "int M() { return (value + 42) * 2; }",
        "{ return (value + 42) * 2; }",
        "return (value + 42) * 2;",
        "(value + 42) * 2",
        "(value + 42)",
        "value + 42",
        "value",
        "",
    ] {
        cx.simulate_keystrokes("shift-alt-left");
        assert_eq!(selected(&view, cx).0, step);
    }
    assert_eq!(
        selected(&view, cx).1,
        crate::code_view::Selection::caret(start)
    );

    cx.simulate_keystrokes("shift-alt-right");
    let moved = caret(&view, cx, source.find("42").unwrap() + 1);
    cx.simulate_keystrokes("shift-alt-left");
    assert_eq!(
        selected(&view, cx).1,
        crate::code_view::Selection::caret(moved)
    );

    caret(&view, cx, source.find("value").unwrap() + 1);
    cx.simulate_keystrokes("shift-alt-right");
    assert_eq!(selected(&view, cx).0, "value");
    cx.simulate_input("another");
    settle(cx);
    let after_edit = selected(&view, cx).1;
    cx.simulate_keystrokes("shift-alt-left");
    assert_eq!(selected(&view, cx).1, after_edit);
    assert_eq!(shown_text(&view, cx), source.replace("value", "another"));
}

#[gpui::test]
fn smart_selection_fallback_uses_subwords_line_whitespace_and_utf8_boundaries(
    cx: &mut TestAppContext,
) {
    let source = "\tfooBarBaz 中文😀  \nending";
    let (_folder, view, cx) = open(cx, "notes.txt", source);
    caret(&view, cx, source.find("Bar").unwrap() + 1);
    for step in ["Bar", "fooBarBaz", "\tfooBarBaz 中文😀  ", source] {
        cx.simulate_keystrokes("shift-alt-right");
        assert_eq!(selected(&view, cx).0, step);
    }

    let at = caret(&view, cx, source.find("Bar").unwrap() + 1);
    execute(&view, cx, "cursorWordPartRight", Value::Null);
    execute(&view, cx, "cursorWordPartLeftSelect", Value::Null);
    assert_eq!(selected(&view, cx).0, "Bar");
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    let hit = code.update(cx, |code, _| {
        let mut hit = code.point_for_position(at).unwrap();
        hit.y += gpui::px(code.line_height() / 2.0);
        hit
    });
    cx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position: hit,
        modifiers: gpui::Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_mouse_up(hit, gpui::MouseButton::Left, gpui::Modifiers::none());
    assert_eq!(selected(&view, cx).0, "fooBarBaz");

    code.update(cx, |code, cx| {
        let mut options = code.options().clone();
        options.smart_select_subwords = false;
        options.smart_select_whitespace = false;
        code.set_options(options, cx);
    });
    caret(&view, cx, source.find("Bar").unwrap() + 1);
    cx.simulate_keystrokes("shift-alt-right");
    assert_eq!(selected(&view, cx).0, "fooBarBaz");
    cx.simulate_keystrokes("shift-alt-right");
    assert_eq!(selected(&view, cx).0, "fooBarBaz 中文😀");

    let unicode = caret(&view, cx, source.find('😀').unwrap());
    cx.simulate_keystrokes("shift-alt-right");
    cx.simulate_keystrokes("shift-alt-left");
    assert_eq!(
        selected(&view, cx).1,
        crate::code_view::Selection::caret(unicode)
    );
}

#[gpui::test]
fn smart_selection_bracket_commands_select_jump_and_undo_once(cx: &mut TestAppContext) {
    let source = "class C { int M() { return F(G(3)); } }";
    let (_folder, view, cx) = open(cx, "Program.cs", source);
    let open_at = source.find("(3)").unwrap();
    let inside = caret(&view, cx, open_at + 1);
    execute(&view, cx, ids::SELECT_TO_BRACKET, Value::Bool(false));
    assert_eq!(selected(&view, cx).0, "3");
    caret(&view, cx, open_at + 1);
    execute(&view, cx, ids::SELECT_TO_BRACKET, Value::Null);
    assert_eq!(selected(&view, cx).0, "(3)");
    caret(&view, cx, open_at + 1);
    cx.simulate_keystrokes("ctrl-shift-\\");
    assert_eq!(selected(&view, cx).1.active.column.0, inside.column.0 + 1);
    cx.simulate_keystrokes("ctrl-shift-\\");
    assert_eq!(selected(&view, cx).1.active.column.0, inside.column.0 - 1);

    cx.simulate_keystrokes("ctrl-alt-backspace");
    settle(cx);
    assert_eq!(shown_text(&view, cx), source.replace("(3)", "3"));
    cx.simulate_keystrokes("ctrl-z");
    settle(cx);
    assert_eq!(shown_text(&view, cx), source);
}

#[gpui::test]
fn smart_selection_readonly_keeps_selection_and_rejects_edits(cx: &mut TestAppContext) {
    let source = "class C { int M() { return (3); } }";
    let (_folder, view, cx) = open(cx, "Program.cs", source);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    code.update(cx, |code, cx| {
        code.set_read_only(Arc::from([rove_buffer::ReadOnlyReason::Session]), cx)
    });
    let open_at = source.find("(3)").unwrap();
    caret(&view, cx, open_at + 1);
    cx.simulate_keystrokes("shift-alt-right");
    assert!(!selected(&view, cx).0.is_empty());
    caret(&view, cx, open_at + 1);
    cx.simulate_keystrokes("ctrl-shift-\\");
    assert_eq!(selected(&view, cx).1.active.column.0, (open_at + 2) as u64);
    cx.simulate_keystrokes("ctrl-alt-backspace");
    settle(cx);
    assert_eq!(shown_text(&view, cx), source);
}
