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
