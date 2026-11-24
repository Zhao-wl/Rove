//! #26 入口集成测试（真实 GPUI 窗口）：打开 `.cs` → 大纲节点与面包屑文本；光标移动时跟随；
//! 键盘在大纲中定位；`Ctrl+Shift+O`；面包屑键位与选择器；不支持的文件显示空状态；设置热加载。
use super::*;
use crate::outline::{registry as reg, view::Status};
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Utf8Column};
use rove_config::{SettingsOptions, SettingsService};

const PLAYER: &str = "namespace Game\n{\n    public class Player\n    {\n        public void Update()\n        {\n        }\n\n        public void Move(float speed)\n        {\n        }\n    }\n}\n";

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

struct Fixture {
    user: tempfile::TempDir,
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Fixture {
    fn new(settings_json: &str) -> Self {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("settings.json"), settings_json).unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir(folder.path().join("src")).unwrap();
        std::fs::write(folder.path().join("src/Player.cs"), PLAYER).unwrap();
        std::fs::write(folder.path().join("src/Enemy.cs"), "class Enemy {}\n").unwrap();
        std::fs::write(folder.path().join("notes.txt"), "plain text\n").unwrap();
        Self {
            user,
            folder,
            service,
        }
    }

    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let root: PathBuf = self.folder.path().into();
        let settings: Arc<dyn Settings> = self.service.clone();
        let (view, cx) = cx.add_window_view(|_, cx| {
            Workbench::with_settings(root, Instant::now(), LayoutStore::disabled(), settings, cx)
                .unwrap()
        });
        cx.run_until_parked();
        (view, cx)
    }

    fn change(&self, settings_json: &str, cx: &mut VisualTestContext) {
        std::fs::write(self.user.path().join("settings.json"), settings_json).unwrap();
        self.service.reload();
        cx.run_until_parked();
        refresh(cx);
    }
}

#[gpui::test]
fn structured_problems_follow_the_settings_view_service(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{"editor.tabSize": 4}"#);
    cx.update(|cx| crate::settings_ui::SharedSettings::install(fx.service.clone(), cx));
    let (view, cx) = fx.open(cx);
    view.read_with(cx, |v, _| assert_eq!(v.settings_error_counts(), (0, 0)));

    fx.change(r#"{"editor.tabSize": }"#, cx);
    view.read_with(cx, |v, _| assert_eq!(v.settings_error_counts(), (1, 0)));

    fx.change(r#"{"editor.tabSize": 2}"#, cx);
    view.read_with(cx, |v, _| assert_eq!(v.settings_error_counts(), (0, 0)));
}

fn refresh(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

fn open_file(
    view: &Entity<Workbench>,
    path: &str,
    at: Option<Position>,
    cx: &mut VisualTestContext,
) {
    view.update(cx, |v, cx| {
        let options = OpenOptions {
            selection: at.and_then(|p| TextRange::new(p, p).ok()),
            ..Default::default()
        };
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                options,
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    cx.run_until_parked();
    // 第一帧发出符号请求，第二帧显示结果。
    refresh(cx);
    refresh(cx);
}

/// 大纲列表的可见行（缩进表示深度）。
fn outline_rows(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| {
        let w = &v.outline.list.read(cx).widget;
        w.display_rows(0..w.len())
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), r.label))
            .collect()
    })
}

fn selected_label(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, cx| {
        let w = &v.outline.list.read(cx).widget;
        let key = w.selected_ids().first()?.0.clone();
        let (_, symbols) = v.outline.shown.as_ref()?;
        Some(
            symbols
                .node(symbols.find_key(&key)?)?
                .symbol
                .name
                .to_string(),
        )
    })
}

/// 焦点组的面包屑文字。
fn breadcrumb_text(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    view.update(cx, |v, _| {
        let group = v.editor.focused;
        let Some((_, symbols, segments)) = v.group_segments(group) else {
            return Vec::new();
        };
        segments
            .iter()
            .map(|s| match s {
                crate::outline::model::Segment::File { label, .. } => label.to_string(),
                crate::outline::model::Segment::Symbol(id) => symbols
                    .as_ref()
                    .unwrap()
                    .node(id.to_owned())
                    .unwrap()
                    .symbol
                    .name
                    .to_string(),
            })
            .collect()
    })
}

fn cursor(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Position {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| c.selection().active)
}

fn move_cursor(view: &Entity<Workbench>, at: Position, cx: &mut VisualTestContext) {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, cx| {
        c.set_selection(code_view::Selection::caret(at), None, cx)
    });
    cx.run_until_parked();
    refresh(cx);
}

#[gpui::test]
fn opening_csharp_shows_outline_and_breadcrumbs(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    // 没有编辑器：空状态。
    view.read_with(cx, |v, _| assert_eq!(v.outline.status, Status::NoEditor));
    open_file(&view, "src/Player.cs", Some(pos(5, 8)), cx);
    view.read_with(cx, |v, _| assert_eq!(v.outline.status, Status::Ready));
    assert_eq!(
        outline_rows(&view, cx),
        ["Game", "  Player", "    Update()", "    Move(float)"]
    );
    // 面包屑：文件路径段 + 光标所在的符号路径。
    assert_eq!(
        breadcrumb_text(&view, cx),
        ["src", "Player.cs", "Game", "Player", "Update()"]
    );
    // 跟随光标：大纲选中光标所在的方法。
    assert_eq!(selected_label(&view, cx).as_deref(), Some("Update()"));

    // 光标移到 Move 内：面包屑与大纲随之更新。
    move_cursor(&view, pos(9, 9), cx);
    assert_eq!(
        breadcrumb_text(&view, cx),
        ["src", "Player.cs", "Game", "Player", "Move(float)"]
    );
    assert_eq!(selected_label(&view, cx).as_deref(), Some("Move(float)"));

    // 关闭跟随光标：大纲选择不再变化。
    view.update_in(cx, |v, window, cx| {
        v.outline_command(reg::OUTLINE_FOLLOW_CURSOR, window, cx)
    });
    move_cursor(&view, pos(5, 9), cx);
    assert_eq!(selected_label(&view, cx).as_deref(), Some("Move(float)"));
    assert_eq!(breadcrumb_text(&view, cx).last().unwrap(), "Update()");

    // 不支持的文件类型：空状态文案（UX_SPEC §6.3），面包屑只剩路径。
    open_file(&view, "notes.txt", None, cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.outline.status, Status::Unsupported);
        assert_eq!(
            v.label("outline.empty", &[]),
            "活动编辑器无法提供大纲信息。"
        );
    });
    assert_eq!(breadcrumb_text(&view, cx), ["notes.txt"]);
}

#[gpui::test]
fn keyboard_navigation_in_outline_reveals_symbol(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "src/Player.cs", Some(pos(0, 0)), cx);
    // `outline.focus` 把焦点交给大纲；回车经 OpenDocuments 定位到符号名称并把焦点还给编辑器。
    view.update_in(cx, |v, window, cx| {
        v.outline_command(reg::OUTLINE_FOCUS, window, cx)
    });
    refresh(cx);
    cx.simulate_keystrokes("home down down down enter");
    cx.run_until_parked();
    refresh(cx);
    assert_eq!(cursor(&view, cx), pos(8, 20));
    assert_eq!(breadcrumb_text(&view, cx).last().unwrap(), "Move(float)");
    let in_editor = view.update_in(cx, |v, window, cx| {
        v.code_view().focus_handle(cx).contains_focused(window, cx)
    });
    assert!(in_editor, "回车后焦点回到编辑器");
    // 全部折叠。
    view.update_in(cx, |v, window, cx| {
        v.outline_command(reg::OUTLINE_COLLAPSE, window, cx)
    });
    assert_eq!(outline_rows(&view, cx), ["Game"]);
}

#[gpui::test]
fn ctrl_shift_o_goes_to_symbol(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "src/Player.cs", Some(pos(0, 0)), cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_keystrokes("ctrl-shift-o");
    cx.run_until_parked();
    let count = view.read_with(cx, |v, cx| {
        let goto = v.outline.goto.as_ref().expect("quick input shown");
        goto.view.read(cx).service.active().unwrap().list.len()
    });
    assert_eq!(count, 4);
    cx.simulate_input("move");
    cx.run_until_parked();
    let labels = view.read_with(cx, |v, cx| {
        let goto = v.outline.goto.as_ref().unwrap();
        let list = &goto.view.read(cx).service.active().unwrap().list;
        list.display_rows(0..list.len())
            .iter()
            .map(|r| r.label.to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(labels, ["Move(float)"]);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    refresh(cx);
    assert_eq!(cursor(&view, cx), pos(8, 20));
    view.read_with(cx, |v, cx| {
        assert!(v
            .outline
            .goto
            .as_ref()
            .unwrap()
            .view
            .read(cx)
            .service
            .active()
            .is_none())
    });
}

#[gpui::test]
fn breadcrumb_keys_and_picker(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "src/Player.cs", Some(pos(5, 8)), cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    // breadcrumbs.focus（ctrl+shift+;）：聚焦最后一段。
    view.update_in(cx, |v, window, cx| {
        v.outline_command(reg::FOCUS, window, cx)
    });
    refresh(cx);
    view.read_with(cx, |v, _| {
        assert!(v.outline.bar_active);
        assert_eq!(v.outline.bar.focused, 4);
    });
    // left / right 由键位服务按 breadcrumbsActive 分发。
    cx.simulate_keystrokes("left left");
    view.read_with(cx, |v, _| assert_eq!(v.outline.bar.focused, 2));
    cx.simulate_keystrokes("right");
    view.read_with(cx, |v, _| assert_eq!(v.outline.bar.focused, 3));
    // enter（selectFocused）：弹出 Player 的同级选择器；down 进入列表的下一项。
    cx.simulate_keystrokes("right enter");
    refresh(cx);
    let items = view.read_with(cx, |v, _| v.outline.picker.as_ref().map(|p| p.items.len()));
    assert_eq!(items, Some(2), "Update() 与 Move(float)");
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    refresh(cx);
    assert_eq!(cursor(&view, cx), pos(8, 20));
    view.read_with(cx, |v, _| assert!(v.outline.picker.is_none()));
    // 文件段的选择器列出目录条目，选中另一个文件即打开。
    view.update_in(cx, |v, window, cx| {
        let group = v.editor.focused;
        v.open_picker(group, 1, window, cx)
    });
    refresh(cx);
    let labels = view.read_with(cx, |v, _| {
        v.outline
            .picker
            .as_ref()
            .unwrap()
            .items
            .iter()
            .map(|i| format!("{i:?}"))
            .collect::<Vec<_>>()
    });
    assert_eq!(labels.len(), 2, "{labels:?}");
    cx.simulate_keystrokes("home enter");
    cx.run_until_parked();
    refresh(cx);
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert_eq!(
            tab.resource,
            Resource::Workspace(WorkspacePath::new("src/Enemy.cs").unwrap())
        );
    });
    // escape（selectEditor）离开面包屑。
    view.update_in(cx, |v, window, cx| {
        v.outline_command(reg::FOCUS, window, cx)
    });
    refresh(cx);
    cx.simulate_keystrokes("escape");
    refresh(cx);
    view.read_with(cx, |v, _| assert!(!v.outline.bar_active));
}

#[gpui::test]
fn outline_and_breadcrumb_settings_reload(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{ "breadcrumbs.enabled": false }"#);
    let (view, cx) = fx.open(cx);
    open_file(&view, "src/Player.cs", Some(pos(5, 8)), cx);
    view.update(cx, |v, cx| {
        let group = v.editor.focused;
        assert!(v.render_breadcrumbs(group, cx).is_none());
    });
    fx.change(
        r#"{ "breadcrumbs.filePath": "off", "outline.showMethods": false, "outline.collapseItems": "alwaysCollapse" }"#,
        cx,
    );
    assert_eq!(breadcrumb_text(&view, cx), ["Game", "Player", "Update()"]);
    assert_eq!(outline_rows(&view, cx), ["Game"]);
    fx.change(
        r#"{ "breadcrumbs.symbolPath": "last", "breadcrumbs.showMethods": false }"#,
        cx,
    );
    assert_eq!(breadcrumb_text(&view, cx), ["src", "Player.cs", "Player"]);
    assert_eq!(
        outline_rows(&view, cx),
        ["Game", "  Player", "    Update()", "    Move(float)"]
    );
}

#[gpui::test]
fn context_basket_outline_menu_collects_the_entire_method(cx: &mut TestAppContext) {
    let fixture = Fixture::new("{}");
    let (view, cx) = fixture.open(cx);
    open_file(&view, "src/Player.cs", Some(pos(9, 8)), cx);
    view.update_in(cx, |view, window, cx| {
        view.basket.use_test_state();
        view.outline_command(reg::OUTLINE_FOCUS, window, cx);
    });
    refresh(cx);
    cx.simulate_keystrokes("home down down down shift-f10");
    super::basket_tests::click_basket_menu(cx, 0);
    view.read_with(cx, |view, _| {
        let [item] = view.basket.contents.items() else {
            panic!("one method is expected");
        };
        assert_eq!(item.path.as_str(), "src/Player.cs");
        assert_eq!((item.lines.first(), item.lines.last()), (9, 11));
        assert!(item
            .symbol
            .as_deref()
            .is_some_and(|name| name.starts_with("Move")));
    });
}

#[gpui::test]
fn context_basket_editor_caret_collects_method_class_and_plain_line(cx: &mut TestAppContext) {
    let fixture = Fixture::new("{}");
    let (view, cx) = fixture.open(cx);
    view.update(cx, |view, _| view.basket.use_test_state());
    for (path, position, expected) in [
        ("src/Player.cs", pos(9, 8), (9, 11)),
        ("src/Player.cs", pos(2, 19), (3, 12)),
        ("notes.txt", pos(0, 0), (1, 1)),
    ] {
        open_file(&view, path, Some(position), cx);
        view.update(cx, |view, cx| {
            view.command(crate::context_basket::registry::CLEAR, cx);
            view.command(crate::context_basket::registry::ADD, cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            let [item] = view.basket.contents.items() else {
                panic!("one entry is expected");
            };
            assert_eq!(item.path.as_str(), path);
            assert_eq!((item.lines.first(), item.lines.last()), expected);
            assert_eq!(item.symbol.is_none(), path == "notes.txt");
        });
    }
}

#[gpui::test]
fn structured_outline_loads_values_and_expands_on_demand(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    std::fs::write(
        fx.folder.path().join("config.json"),
        r#"{"items":[{"name":"alpha"}],"count":3}"#,
    )
    .unwrap();
    let (view, cx) = fx.open(cx);
    open_file(&view, "config.json", Some(pos(0, 6)), cx);
    refresh(cx);
    let items = view.read_with(cx, |v, cx| {
        assert_eq!(v.outline.status, Status::Ready);
        let w = &v.outline.list.read(cx).widget;
        let rows = w.display_rows(0..w.len());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label.as_ref(), "items");
        assert_eq!(rows[0].detail.as_deref(), Some("[1]"));
        assert!(rows[0].expandable);
        rows[0].id.clone()
    });
    view.update(cx, |v, cx| {
        v.outline.list.update(cx, |list, _| {
            assert!(list.widget.expand_id(&items, true).unwrap());
        });
        v.structured_outline_event(
            &crate::list_widget::widget::ListEvent::Expanded {
                id: items,
                expanded: true,
            },
            cx,
        );
    });
    refresh(cx);
    view.read_with(cx, |v, cx| {
        let w = &v.outline.list.read(cx).widget;
        let rows = w.display_rows(0..w.len());
        assert_eq!(rows[0].label.as_ref(), "items");
        assert_eq!(rows[1].label.as_ref(), "0");
        assert_eq!(rows[1].depth, 1);
        assert!(rows[1].expandable);
    });
}

#[gpui::test]
fn structured_json_errors_are_published_on_the_document(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    std::fs::write(fx.folder.path().join("bad.json"), "{\"value\": [ }").unwrap();
    let (view, cx) = fx.open(cx);
    open_file(&view, "bad.json", None, cx);
    refresh(cx);
    view.read_with(cx, |v, cx| {
        let (group, doc, _) = v.focused_text().unwrap();
        let entry = v.structured.documents.get(&doc.id).unwrap();
        assert!(!entry.problems.is_empty());
        let location = entry.problems[0].range.start();
        assert_eq!(location.line, Line(0));
        assert!(v
            .listed_problem_positions(cx)
            .contains(&(doc.resource.clone(), location)));
        let slot = &v.views[&group].primary;
        assert_eq!(
            slot.structured_mark.map(|(d, revision, _)| (d, revision)),
            Some((doc.id, doc.text.version()))
        );
        let decorated = slot
            .code
            .read(cx)
            .merged_decorations(location.line.0)
            .unwrap();
        assert!(decorated.runs.iter().any(|run| matches!(
            run.style.underline,
            Some(crate::code_view::decorations::TextDecoration::Wavy(_))
        )));
    });
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    for key in ["alt-f8", "f8", "shift-alt-f8", "shift-f8"] {
        cx.simulate_keystrokes(key);
        refresh(cx);
        refresh(cx);
        view.read_with(cx, |v, _| assert!(v.structured.marker.is_some(), "{key}"));
        cx.simulate_keystrokes("escape");
        refresh(cx);
        view.read_with(cx, |v, _| assert!(v.structured.marker.is_none(), "{key}"));
    }
    cx.simulate_keystrokes("ctrl-shift-m");
    refresh(cx);
    view.read_with(cx, |v, _| assert!(v.shell.layout.panel_visible));
    fx.change(r#"{"problems.visibility":false}"#, cx);
    view.read_with(cx, |v, _| {
        let (group, _, _) = v.focused_text().unwrap();
        assert!(!v.shell.problems_visible);
        assert!(v.views[&group].primary.structured_mark.is_none());
        assert!(!v.outline.structure_style.borrow().badges);
    });
}

#[gpui::test]
fn structured_format_command_creates_one_undo_step(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let original = r#"{"x":[1,2]}"#;
    std::fs::write(fx.folder.path().join("format.json"), original).unwrap();
    let (view, cx) = fx.open(cx);
    open_file(&view, "format.json", None, cx);
    refresh(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_keystrokes("shift-alt-f");
    refresh(cx);
    let read = |cx: &mut VisualTestContext| {
        view.read_with(cx, |v, _| {
            let (_, doc, _) = v.focused_text().unwrap();
            let mut text = String::new();
            doc.text
                .visit_chunks(
                    rove_base::ByteRange::new(
                        rove_base::ByteOffset(0),
                        rove_base::ByteOffset(doc.text.byte_len()),
                    )
                    .unwrap(),
                    &mut |part| text.push_str(part),
                )
                .unwrap();
            text
        })
    };
    assert!(read(cx).contains('\n'));
    cx.simulate_keystrokes("ctrl-z");
    refresh(cx);
    assert_eq!(read(cx), original);

    cx.simulate_keystrokes("shift-alt-f");
    refresh(cx);
    let pretty = read(cx);
    view.update(cx, |v, cx| {
        assert!(v.structured_command(crate::outline::structure::MINIFY, cx));
    });
    refresh(cx);
    assert_eq!(read(cx), original);
    cx.simulate_keystrokes("ctrl-z");
    refresh(cx);
    assert_eq!(read(cx), pretty);
}

#[gpui::test]
fn structured_paste_formats_in_context_and_preserves_outer_bytes(cx: &mut TestAppContext) {
    let fx = Fixture::new(
        r#"{"editor.formatOnPaste":true,"editor.tabSize":2,"editor.detectIndentation":false}"#,
    );
    let xml_start = r#"<root xml:space="pr&#101;serve">"#;
    let xml = format!("{xml_start}A</root>");
    let xml_paste = "<block><a/><b/></block>";
    let expected_xml = format!("{xml_start}{xml_paste}A</root>");
    let json = "{\n  \"outer\": {\n    \n  }\n}\n";
    let json_paste = "\"chosen\":[1,2]";
    let expected_json = "{\n  \"outer\": {\n    \"chosen\": [\n      1,\n      2\n    ]\n  }\n}\n";
    let cases = [
        (
            "paste.xml",
            xml.as_str(),
            pos(0, xml_start.len() as u64),
            xml_paste,
            expected_xml.as_str(),
        ),
        ("paste.json", json, pos(2, 4), json_paste, expected_json),
    ];
    for (file, source, _, _, _) in cases {
        std::fs::write(fx.folder.path().join(file), source).unwrap();
    }

    let (view, cx) = fx.open(cx);
    let read = |cx: &mut VisualTestContext| {
        view.read_with(cx, |v, _| {
            let (_, doc, _) = v.focused_text().unwrap();
            let mut text = String::new();
            doc.text
                .visit_chunks(
                    rove_base::ByteRange::new(
                        rove_base::ByteOffset(0),
                        rove_base::ByteOffset(doc.text.byte_len()),
                    )
                    .unwrap(),
                    &mut |part| text.push_str(part),
                )
                .unwrap();
            text
        })
    };
    for (file, source, cursor, pasted, expected) in cases {
        open_file(&view, file, Some(cursor), cx);
        refresh(cx);
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
        cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(pasted.into())));
        cx.simulate_keystrokes("ctrl-v");
        refresh(cx);
        assert_eq!(read(cx), expected, "{file}");

        cx.simulate_keystrokes("ctrl-z");
        refresh(cx);
        assert_eq!(read(cx), source, "{file}");
    }
}

#[gpui::test]
fn structured_format_on_save_preserves_utf16_bom_and_existing_line_endings(
    cx: &mut TestAppContext,
) {
    let fx = Fixture::new(r#"{"editor.formatOnSave":true,"files.insertFinalNewline":false}"#);
    let disk = fx.folder.path().join("encoded.json");
    let source = "{\r\n  \"中\":1,\n  \"array\":[1,2]\r}\r\n";
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(source.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&disk, &bytes).unwrap();
    let (view, cx) = fx.open(cx);
    open_file(&view, "encoded.json", Some(pos(1, 9)), cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.simulate_input("0");
    refresh(cx);
    view.update(cx, |v, cx| {
        let (_, document, _) = v.focused_text().unwrap();
        v.start_save(document.id, rove_buffer::save::SaveRequest::default(), cx);
    });
    refresh(cx);
    refresh(cx);
    let after = std::fs::read(&disk).unwrap();
    assert_ne!(after, bytes);
    assert_eq!(&after[..2], &[0xFF, 0xFE]);
    let after = String::from_utf16(
        &after[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(after.starts_with("{\r\n"));
    assert!(after.contains("\"中\": 10,\n"), "{after:?}");
    assert!(after.ends_with("]\r}\r\n"), "{after:?}");
}
