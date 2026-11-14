//! 注释切换、自动缩进、自动闭合与自动包围的入口集成测试（#53）。
//!
//! 和 #47 的 `code_view_edit` 一样在 GPUI 测试窗口里驱动真实视图，编辑入口是 `RefCell<EditSession>`。
//! 自动行为依赖语言配置（#12），所以每个夹具都设了语言 ID。
//!
//! 运行：`cargo test -p rove-ui --test code_view_autoedit`

use gpui::{Context, Entity, Focusable, TestAppContext, VisualTestContext, Window};
use rove_buffer::edit::{EditBuffer, EditOptions, EditSession};
use rove_buffer::{Eol, LineEndings, ReadOnlyReason, SizeClass, TextSource};
use rove_config::{DefaultSettings, SettingRegistry};
use rove_theme::{ColorRegistry, DefaultTheme, ThemeKind};
use rove_ui::code_view::{
    self, selection::pos, CodeView, CodeViewEvent, CodeViewServices, EditHost, Selection,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

// ------------------------------------------------------------------ 夹具

/// 默认设置加上几个用户值（`editor.autoClosingBrackets` 等取值的集成验证）。
struct Overrides {
    defaults: DefaultSettings,
    user: Vec<(&'static str, rove_base::Value)>,
}

impl rove_config::Settings for Overrides {
    fn read(&self, key: &str, scope: &rove_config::Scope) -> rove_base::Result<rove_base::Value> {
        match self.user.iter().find(|(k, _)| *k == key) {
            Some((_, v)) => Ok(v.clone()),
            None => self.defaults.read(key, scope),
        }
    }
    fn write(
        &self,
        _: Arc<str>,
        _: rove_config::Scope,
        _: rove_base::Value,
        _: rove_base::Cancellation,
    ) -> rove_base::Task<()> {
        Box::pin(async { Err(rove_base::Error::Unsupported) })
    }
    fn changes(&self) -> &dyn rove_base::Events<rove_config::SettingsChange> {
        &rove_base::NoEvents
    }
}

fn services(user: Vec<(&'static str, rove_base::Value)>) -> CodeViewServices {
    let mut settings = SettingRegistry::default();
    let mut colors = ColorRegistry::default();
    let mut commands = rove_commands::CommandRegistry::default();
    let mut strings = rove_i18n::StringTable::default();
    code_view::register(&mut settings, &mut colors, &mut commands, &mut strings).unwrap();
    rove_buffer::edit::settings::register(&mut settings).unwrap();
    CodeViewServices {
        settings: Arc::new(Overrides {
            defaults: DefaultSettings(settings),
            user,
        }),
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

fn open<'a>(cx: &'a mut TestAppContext, language: &str, text: &str) -> Fixture<'a> {
    open_with(cx, language, text, Vec::new())
}

fn open_with<'a>(
    cx: &'a mut TestAppContext,
    language: &str,
    text: &str,
    user: Vec<(&'static str, rove_base::Value)>,
) -> Fixture<'a> {
    let session = Rc::new(RefCell::new(EditSession::new(
        EditBuffer::new(text, LineEndings::Uniform(Eol::Lf)),
        EditOptions::default(),
    )));
    let snapshot: Arc<dyn TextSource> = session.borrow().snapshot();
    let s = services(user);
    let host: Rc<dyn EditHost> = session.clone();
    let events: Rc<RefCell<Vec<CodeViewEvent>>> = Rc::default();
    let sink = events.clone();
    let language: Arc<str> = Arc::from(language);
    let (view, cx) = cx.add_window_view(move |_: &mut Window, cx: &mut Context<CodeView>| {
        let mut v = CodeView::new(s, cx);
        v.set_text(Some(snapshot), false, cx);
        v.set_language(Some(language), cx);
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
    fn select(&mut self, s: Selection) {
        self.view
            .update(self.cx, |v, cx| v.set_selection(s, None, cx));
        settle(self.cx);
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
    fn type_text(&mut self, text: &str) {
        self.cx.simulate_input(text);
        settle(self.cx);
    }
    fn edit_count(&self) -> usize {
        self.events
            .borrow()
            .iter()
            .filter(|e| matches!(e, CodeViewEvent::Edited(_)))
            .count()
    }
}

// ------------------------------------------------------------------ 一、注释切换

#[gpui::test]
fn comment_line_toggles_a_csharp_selection(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "int a;\nint b;\n");
    f.select(Selection::new(pos(0, 0), pos(1, 6)));
    f.run("editor.action.commentLine");
    assert_eq!(f.contents(), "// int a;\n// int b;\n");
    f.run("editor.action.commentLine");
    assert_eq!(f.contents(), "int a;\nint b;\n");
}

#[gpui::test]
fn mixed_selection_comments_everything_and_is_one_undo_unit(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "// a\nb\nc\n");
    f.select(Selection::new(pos(0, 0), pos(2, 1)));
    f.run("editor.action.commentLine");
    assert_eq!(
        f.contents(),
        "// // a\n// b\n// c\n",
        "只要有一行未注释就全部注释"
    );
    f.run("undo");
    assert_eq!(f.contents(), "// a\nb\nc\n", "三行的注释是一个撤销单元");
}

#[gpui::test]
fn add_and_remove_comment_line_force_the_direction(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "// a\nb\n");
    f.select(Selection::new(pos(0, 0), pos(1, 1)));
    f.run("editor.action.addCommentLine");
    assert_eq!(f.contents(), "// // a\n// b\n");
    f.run("editor.action.removeCommentLine");
    assert_eq!(f.contents(), "// a\nb\n");
}

#[gpui::test]
fn comment_prefix_goes_at_the_common_indent(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "        a();\n    b();\n");
    f.select(Selection::new(pos(0, 0), pos(1, 8)));
    f.run("editor.action.commentLine");
    assert_eq!(f.contents(), "    //     a();\n    // b();\n");
}

#[gpui::test]
fn yaml_uses_its_own_line_comment(cx: &mut TestAppContext) {
    let mut f = open(cx, "yaml", "a: 1\n");
    f.caret(0, 0);
    f.run("editor.action.commentLine");
    assert_eq!(f.contents(), "# a: 1\n");
}

#[gpui::test]
fn xml_without_a_line_comment_falls_back_to_block_comment(cx: &mut TestAppContext) {
    let mut f = open(cx, "xml", "<a/>\n");
    f.select(Selection::new(pos(0, 0), pos(0, 4)));
    f.run("editor.action.commentLine");
    assert_eq!(f.contents(), "<!-- <a/> -->\n");
    f.run("editor.action.blockComment");
    assert_eq!(f.contents(), "<a/>\n");
}

#[gpui::test]
fn block_comment_wraps_a_csharp_selection(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "int a;\n");
    f.select(Selection::new(pos(0, 0), pos(0, 6)));
    f.run("editor.action.blockComment");
    assert_eq!(f.contents(), "/* int a; */\n");
}

// ------------------------------------------------------------------ 二、自动缩进与回车规则

#[gpui::test]
fn enter_after_an_open_brace_indents(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "    if (x) {\n");
    f.caret(0, 12);
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "    if (x) {\n        \n");
    assert_eq!(f.selection(), Selection::caret(pos(1, 8)));
}

#[gpui::test]
fn enter_between_braces_is_indent_outdent_in_one_undo_unit(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "if (x) {}\n");
    f.caret(0, 8);
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "if (x) {\n    \n}\n");
    assert_eq!(f.selection(), Selection::caret(pos(1, 4)));
    f.run("undo");
    assert_eq!(f.contents(), "if (x) {}\n", "回车 + 自动缩进是一个撤销单元");
}

#[gpui::test]
fn enter_continues_a_csharp_doc_comment(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "    /// <summary>\n");
    f.caret(0, 17);
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "    /// <summary>\n    /// \n");
    assert_eq!(f.selection(), Selection::caret(pos(1, 8)));
    f.run("undo");
    assert_eq!(
        f.contents(),
        "    /// <summary>\n",
        "回车 + `///` 续写是一个撤销单元"
    );
}

#[gpui::test]
fn enter_uses_indentation_rules_in_json(cx: &mut TestAppContext) {
    let mut f = open(cx, "json", "{\n");
    f.caret(0, 1);
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "{\n    \n");
}

#[gpui::test]
fn typing_a_closing_brace_realigns_the_line(cx: &mut TestAppContext) {
    let mut f = open(cx, "json", "{\n        \n");
    f.caret(1, 8);
    f.type_text("}");
    assert_eq!(
        f.contents(),
        "{\n}\n",
        "decreaseIndentPattern 命中时重新对齐"
    );
    assert_eq!(f.selection(), Selection::caret(pos(1, 1)));
    f.run("undo");
    assert_eq!(
        f.contents(),
        "{\n        \n",
        "重新缩进与输入的字符是一个撤销单元"
    );
}

#[gpui::test]
fn auto_whitespace_is_trimmed_when_the_caret_leaves(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "    if (x) {\nfoo\n");
    f.caret(0, 12);
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "    if (x) {\n        \nfoo\n");
    // 光标离开那一行后再编辑：自动插入的空白一起清掉。
    f.caret(2, 3);
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "    if (x) {\n\nfoo\n\n");
}

// ------------------------------------------------------------------ 三、自动闭合与自动包围

#[gpui::test]
fn typing_an_open_bracket_closes_it(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "f\n");
    f.caret(0, 1);
    f.type_text("(");
    assert_eq!(f.contents(), "f()\n");
    assert_eq!(f.selection(), Selection::caret(pos(0, 2)));
    f.run("undo");
    assert_eq!(f.contents(), "f\n", "自动闭合与输入是一个撤销单元");
}

#[gpui::test]
fn auto_close_before_blocks_a_following_word(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "fabc\n");
    f.caret(0, 1);
    f.type_text("(");
    assert_eq!(f.contents(), "f(abc\n", "后面是字母时不自动闭合");
}

#[gpui::test]
fn quotes_do_not_close_after_a_word_character(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "don\n");
    f.caret(0, 3);
    f.type_text("'");
    assert_eq!(f.contents(), "don'\n");
}

#[gpui::test]
fn only_the_auto_inserted_close_is_overtyped(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "f\n");
    f.caret(0, 1);
    f.type_text("(");
    assert_eq!(f.contents(), "f()\n");
    // 自己补的那个 `)`：越过而不是再插一个。
    f.type_text(")");
    assert_eq!(f.contents(), "f()\n");
    assert_eq!(f.selection(), Selection::caret(pos(0, 3)));

    // 手工输入的 `)` 不被越过（最容易做错的一条）。
    let mut f = open(cx, "csharp", "f\n");
    f.caret(0, 1);
    f.type_text(")");
    assert_eq!(f.contents(), "f)\n");
    f.caret(0, 1);
    f.type_text(")");
    assert_eq!(f.contents(), "f))\n", "不是编辑器补的闭合符，照常插入");
}

#[gpui::test]
fn caret_moving_away_forgets_the_auto_closed_pair(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "f\n");
    f.caret(0, 1);
    f.type_text("(");
    assert_eq!(f.contents(), "f()\n");
    // 光标离开再回来：那对括号不再算“编辑器补的”。
    f.caret(0, 0);
    f.caret(0, 2);
    f.type_text(")");
    assert_eq!(f.contents(), "f())\n");
}

#[gpui::test]
fn backspace_deletes_the_auto_inserted_close(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "f\n");
    f.caret(0, 1);
    f.type_text("(");
    assert_eq!(f.contents(), "f()\n");
    f.run("deleteLeft");
    assert_eq!(f.contents(), "f\n", "一起删掉相邻的右括号");

    // 手工输入的一对不一起删。
    let mut f = open(cx, "csharp", "f()\n");
    f.caret(0, 2);
    f.run("deleteLeft");
    assert_eq!(f.contents(), "f)\n");
}

#[gpui::test]
fn selection_is_surrounded(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "abc\n");
    f.select(Selection::new(pos(0, 0), pos(0, 3)));
    f.type_text("\"");
    assert_eq!(f.contents(), "\"abc\"\n");
    assert_eq!(
        f.selection(),
        Selection::new(pos(0, 1), pos(0, 4)),
        "包围后选区仍然是原来那段文本"
    );
    f.run("undo");
    assert_eq!(f.contents(), "abc\n", "两处插入是一个撤销单元");
}

#[gpui::test]
fn whitespace_only_selection_is_replaced_not_surrounded(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "a   b\n");
    f.select(Selection::new(pos(0, 1), pos(0, 4)));
    f.type_text("(");
    // VS Code：选区非空时不自动闭合，只有空白的选区也不包围——直接被输入的字符替换掉。
    assert_eq!(f.contents(), "a(b\n");
}

#[gpui::test]
fn overtype_mode_does_not_auto_close(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "ab\n");
    f.caret(0, 0);
    f.run("editor.action.toggleOvertypeInsertMode");
    f.type_text("(");
    assert_eq!(f.contents(), "(b\n", "改写模式下只覆盖，不补闭合符");
}

// ------------------------------------------------------------------ 四、设置项取值

#[gpui::test]
fn auto_closing_brackets_never_disables_the_pair(cx: &mut TestAppContext) {
    let mut f = open_with(
        cx,
        "csharp",
        "f\n",
        vec![(
            "editor.autoClosingBrackets",
            rove_base::Value::String("never".into()),
        )],
    );
    f.caret(0, 1);
    f.type_text("(");
    assert_eq!(f.contents(), "f(\n");
}

#[gpui::test]
fn auto_closing_brackets_always_ignores_auto_close_before(cx: &mut TestAppContext) {
    let mut f = open_with(
        cx,
        "csharp",
        "fabc\n",
        vec![(
            "editor.autoClosingBrackets",
            rove_base::Value::String("always".into()),
        )],
    );
    f.caret(0, 1);
    f.type_text("(");
    assert_eq!(f.contents(), "f()abc\n");
}

#[gpui::test]
fn auto_surround_never_replaces_the_selection(cx: &mut TestAppContext) {
    let mut f = open_with(
        cx,
        "csharp",
        "abc\n",
        vec![(
            "editor.autoSurround",
            rove_base::Value::String("never".into()),
        )],
    );
    f.select(Selection::new(pos(0, 0), pos(0, 3)));
    f.type_text("\"");
    assert_eq!(f.contents(), "\"\n");
}

#[gpui::test]
fn comments_insert_space_can_be_turned_off(cx: &mut TestAppContext) {
    let mut f = open_with(
        cx,
        "csharp",
        "a\n",
        vec![("editor.comments.insertSpace", rove_base::Value::Bool(false))],
    );
    f.caret(0, 0);
    f.run("editor.action.commentLine");
    assert_eq!(f.contents(), "//a\n");
}

#[gpui::test]
fn ignore_empty_lines_can_be_turned_off(cx: &mut TestAppContext) {
    let mut f = open_with(
        cx,
        "csharp",
        "a\n\nb\n",
        vec![(
            "editor.comments.ignoreEmptyLines",
            rove_base::Value::Bool(false),
        )],
    );
    f.select(Selection::new(pos(0, 0), pos(2, 1)));
    f.run("editor.action.commentLine");
    assert_eq!(f.contents(), "// a\n// \n// b\n");
}

#[gpui::test]
fn auto_indent_none_inserts_a_bare_newline(cx: &mut TestAppContext) {
    let mut f = open_with(
        cx,
        "csharp",
        "    if (x) {\n",
        vec![("editor.autoIndent", rove_base::Value::String("none".into()))],
    );
    f.caret(0, 12);
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "    if (x) {\n\n");
}

#[gpui::test]
fn auto_indent_on_paste_reindents_the_pasted_block(cx: &mut TestAppContext) {
    let mut f = open_with(
        cx,
        "json",
        "{\n",
        vec![("editor.autoIndentOnPaste", rove_base::Value::Bool(true))],
    );
    f.cx.update(|_, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("\"a\": [\n\"b\"\n]".into()))
    });
    f.caret(0, 1);
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    f.run("editor.action.clipboardPasteAction");
    assert_eq!(f.contents(), "{\n    \"a\": [\n        \"b\"\n    ]\n");
    f.run("undo");
    assert_eq!(f.contents(), "{\n    \n", "粘贴与重新缩进是一个撤销单元");
}

// ------------------------------------------------------------------ 五、只读与超大文件

#[gpui::test]
fn read_only_documents_reject_every_automatic_command(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "int a;\n");
    let reasons: Arc<[ReadOnlyReason]> = Arc::from([ReadOnlyReason::Rule(
        rove_workspace::REASON_READONLY_INCLUDE.into(),
    )]);
    f.view
        .update(f.cx, |v, cx| v.set_read_only(reasons.clone(), cx));
    f.caret(0, 3);
    f.run("editor.action.commentLine");
    f.run("editor.action.blockComment");
    f.type_text("(");
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "int a;\n", "只读文档上没有任何修改");
    assert_eq!(f.edit_count(), 0);
    let blocked = f
        .events
        .borrow()
        .iter()
        .filter(|e| matches!(e, CodeViewEvent::EditBlocked { .. }))
        .count();
    assert!(blocked >= 3, "每条被拒绝的命令都提示了原因");
}

#[gpui::test]
fn huge_documents_do_not_run_automatic_edits(cx: &mut TestAppContext) {
    let mut f = open(cx, "csharp", "f\n");
    // 超大文件：关掉全部自动行为（它本来也是只读的，这里单独验证这道闸）。
    f.view
        .update(f.cx, |v, cx| v.set_document_level(SizeClass::Huge, cx));
    f.caret(0, 1);
    assert!(!f.view.update(f.cx, |v, _| v.auto_edit_allowed()));
    f.type_text("(");
    assert_eq!(f.contents(), "f(\n", "只插入输入的字符，不自动闭合");
    f.cx.simulate_keystrokes("enter");
    settle(f.cx);
    assert_eq!(f.contents(), "f(\n\n", "回车不带自动缩进");
}
