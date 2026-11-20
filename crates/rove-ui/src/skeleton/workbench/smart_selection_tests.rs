//! 从工作台打开文档并经真实命令桥验收扩选与括号操作。

use super::*;
use crate::code_view::smart_selection as ids;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{ByteOffset, Position, Span};

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
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    cx.update(|window, app| {
        code.update(app, |code, cx| {
            assert!(code.execute("cursorWordPartRight", window, cx));
            assert!(code.execute("cursorWordPartLeftSelect", window, cx));
        });
    });
    assert_eq!(selected(&view, cx).0, "Bar");
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
fn smart_selection_rebuilds_after_word_and_bracket_settings_change(cx: &mut TestAppContext) {
    let source = "heading\n(\n  foo.bar\n)\nending";
    let (_folder, view, cx) = open(cx, "notes.txt", source);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    code.update(cx, |code, cx| {
        let mut options = code.options().clone();
        options.word_separators = ".".into();
        options.smart_select_subwords = false;
        options.smart_select_whitespace = true;
        options.language_brackets = Some(vec![("(".into(), ")".into())]);
        code.set_options(options, cx);
    });
    let start = caret(&view, cx, source.find("bar").unwrap() + 1);
    cx.simulate_keystrokes("shift-alt-right");
    assert_eq!(selected(&view, cx).0, "bar");
    cx.simulate_keystrokes("shift-alt-left");
    assert_eq!(
        selected(&view, cx).1,
        crate::code_view::Selection::caret(start)
    );

    code.update(cx, |code, cx| {
        let mut options = code.options().clone();
        options.word_separators = "".into();
        code.set_options(options, cx);
    });
    for expected in ["foo.bar", "  foo.bar", "(\n  foo.bar\n)"] {
        cx.simulate_keystrokes("shift-alt-right");
        assert_eq!(selected(&view, cx).0, expected);
    }
    for _ in 0..3 {
        cx.simulate_keystrokes("shift-alt-left");
    }
    assert_eq!(
        selected(&view, cx).1,
        crate::code_view::Selection::caret(start)
    );

    code.update(cx, |code, cx| {
        let mut options = code.options().clone();
        options.language_brackets = Some(Vec::new());
        code.set_options(options, cx);
    });
    for expected in ["foo.bar", "  foo.bar", source] {
        cx.simulate_keystrokes("shift-alt-right");
        assert_eq!(selected(&view, cx).0, expected);
    }
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
fn smart_selection_brackets_follow_the_active_cursor_with_highlighting_off(
    cx: &mut TestAppContext,
) {
    let source = "class C { void M() { F(a)(b); G(c); } }";
    let (_folder, view, cx) = open(cx, "Program.cs", source);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    code.update(cx, |code, cx| {
        let mut options = code.options().clone();
        options.bracket_pair_colorization = true;
        options.highlight_active_bracket_pair = false;
        options.match_brackets = crate::code_view::brackets::MatchBrackets::Never;
        code.set_options(options, cx);
    });
    settle(cx);
    code.read_with(cx, |code, _| {
        assert!(!code.bracket_depths(0).is_empty());
        assert!(code.bracket_match().is_none());
    });

    let second = source.find("(b)").unwrap();
    let close = caret(&view, cx, second + 2);
    cx.simulate_keystrokes("ctrl-shift-\\");
    assert_eq!(selected(&view, cx).1.active.column.0, second as u64);
    cx.simulate_keystrokes("ctrl-shift-\\");
    assert_eq!(selected(&view, cx).1.active, close);
    execute(&view, cx, ids::SELECT_TO_BRACKET, Value::Null);
    assert_eq!(selected(&view, cx).0, "(b)");

    let anchor = caret(&view, cx, source.find("(a)").unwrap() + 1);
    let active = caret(&view, cx, source.find("(c)").unwrap() + 1);
    code.update(cx, |code, cx| {
        code.set_selection(crate::code_view::Selection::new(anchor, active), None, cx);
    });
    cx.simulate_keystrokes("ctrl-alt-backspace");
    settle(cx);
    assert_eq!(shown_text(&view, cx), source.replace("(c)", "c"));
    cx.simulate_keystrokes("ctrl-z");
    settle(cx);
    assert_eq!(shown_text(&view, cx), source);
}

#[gpui::test]
fn smart_selection_bracket_delete_leaves_strings_intact_without_current_syntax(
    cx: &mut TestAppContext,
) {
    let source = r#"class C { string s = "()"; }"#;
    let (_folder, view, cx) = open(cx, "Program.cs", source);
    caret(&view, cx, source.find("()").unwrap() + 1);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    cx.update(|window, app| {
        code.update(app, |code, cx| {
            assert!(code.current_syntax_version().is_some());
            assert!(code.execute(ids::REMOVE_BRACKETS, window, cx));
            assert!(code.current_syntax_version().is_some());
            code.set_syntax(None, None, cx);
            assert!(code.current_syntax_version().is_none());
            assert!(code.execute(ids::REMOVE_BRACKETS, window, cx));
        });
    });
    assert_eq!(shown_text(&view, cx), source);
}

#[gpui::test]
fn smart_selection_bracket_delete_rejects_a_gap_in_tokenization(cx: &mut TestAppContext) {
    let source = format!(
        "class C {{ int M() => F(G(\n/* {} ) */\n1), 2); }}",
        "x".repeat(2_501)
    );
    let (_folder, view, cx) = open(cx, "Program.cs", &source);
    caret(&view, cx, source.find("F(").unwrap() + 1);
    cx.simulate_keystrokes("ctrl-alt-backspace");
    settle(cx);
    assert_eq!(shown_text(&view, cx), source);
}

#[gpui::test]
fn smart_selection_bracket_fallback_honors_an_empty_override(cx: &mut TestAppContext) {
    let source = "class C { void M() { F(a); } }";
    let (_folder, view, cx) = open(cx, "Program.cs", source);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    code.update(cx, |code, cx| {
        let mut options = code.options().clone();
        options.language_brackets = Some(Vec::new());
        options.bracket_pair_colorization = false;
        options.highlight_active_bracket_pair = false;
        options.guides_bracket_pairs = crate::code_view::guides::GuideMode::Off;
        code.set_options(options, cx);
    });
    settle(cx);
    let start = caret(&view, cx, source.find("(a)").unwrap() + 1);
    cx.simulate_keystrokes("ctrl-shift-\\");
    assert_eq!(
        selected(&view, cx).1,
        crate::code_view::Selection::caret(start)
    );
    cx.simulate_keystrokes("ctrl-alt-backspace");
    settle(cx);
    assert_eq!(shown_text(&view, cx), source);
}

#[gpui::test]
fn smart_selection_language_change_discards_the_cached_ancestors(cx: &mut TestAppContext) {
    let source = "class C {\n    int M() { return value + 1; }\n}\n";
    let (_folder, view, cx) = open(cx, "Program.cs", source);
    caret(&view, cx, source.find("value").unwrap() + 1);
    cx.simulate_keystrokes("shift-alt-right");
    assert_eq!(selected(&view, cx).0, "value");

    view.update(cx, |workbench, cx| {
        let document = workbench.active_large_file_document().unwrap();
        workbench.set_language_mode(&document, "plaintext".into(), cx);
    });
    settle(cx);
    cx.simulate_keystrokes("shift-alt-right");
    assert_eq!(selected(&view, cx).0, "    int M() { return value + 1; }");
}

#[gpui::test]
fn smart_selection_long_line_uses_bounded_coordinate_fallback(cx: &mut TestAppContext) {
    let line = format!("  {}  ", "界".repeat(6_000));
    let source = format!("{line}\nlast");
    let (_folder, view, cx) = open(cx, "notes.txt", &source);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    code.update(cx, |code, cx| {
        let text = code.text().unwrap();
        let from = text.position(ByteOffset(2)).unwrap();
        let to = text.position(ByteOffset(5)).unwrap();
        code.set_selection(crate::code_view::Selection::new(from, to), None, cx);
        assert_eq!(code.occurrence_text().as_deref(), Some("界"));
    });
    let start = caret(&view, cx, 5);
    cx.simulate_keystrokes("shift-alt-right");
    assert_eq!(selected(&view, cx).0, line);
    cx.simulate_keystrokes("shift-alt-left");
    assert_eq!(
        selected(&view, cx).1,
        crate::code_view::Selection::caret(start)
    );

    code.update(cx, |code, cx| {
        let mut options = code.options().clone();
        options.smart_select_whitespace = false;
        code.set_options(options, cx);
    });
    cx.simulate_keystrokes("shift-alt-right");
    assert_eq!(selected(&view, cx).0, source);
}

#[gpui::test]
fn smart_selection_bounds_bracket_highlighting_on_an_oversized_row(cx: &mut TestAppContext) {
    let source = format!(r#"{{"value":"{}"}}"#, "x".repeat(70 * 1024));
    let (_folder, view, cx) = open(cx, "Settings.json", &source);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    code.update(cx, |code, cx| {
        let mut options = code.options().clone();
        options.highlight_active_bracket_pair = true;
        options.match_brackets = crate::code_view::brackets::MatchBrackets::Always;
        code.set_options(options, cx);
        code.set_document_level(rove_buffer::SizeClass::Huge, cx);
    });
    caret(&view, cx, 0);
    cx.simulate_keystrokes("shift-alt-right");
    assert_eq!(selected(&view, cx).0, source);
    assert!(code.read_with(cx, |code, _| code.bracket_match().is_none()));
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
