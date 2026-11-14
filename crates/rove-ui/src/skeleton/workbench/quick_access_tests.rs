//! #24 入口集成测试（真实 GPUI 窗口）：`Ctrl+P` → 输入 → 选中 → 文档已打开并定位；四个前缀；命令面板只列
//! 已实现命令并显示键位；`Ctrl+Enter` 在旁边打开；`Esc`；命令中心入口；历史加权；设置热读取。
use super::*;
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
    _user: tempfile::TempDir,
    folder: tempfile::TempDir,
    service: Arc<rove_config::SettingsService>,
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
        std::fs::write(
            folder.path().join("notes.txt"),
            "one\ntwo\nthree\nfour\nfive\n",
        )
        .unwrap();
        Self {
            _user: user,
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
        cx.update(|window, cx| window.focus(&view.read(cx).focus));
        (view, cx)
    }
}

fn refresh(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

fn open_file(view: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    cx.run_until_parked();
    refresh(cx);
    refresh(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
}

fn cursor(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Position {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| c.selection().active)
}

fn active_resource(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| {
        v.editor.active().current().map(|t| match &t.resource {
            Resource::Workspace(p) => p.as_str().to_owned(),
            other => format!("{other:?}"),
        })
    })
}

/// 浮层当前的行：(标签, 描述, 右侧文本)。
fn rows(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(String, String, String)> {
    view.read_with(cx, |v, cx| {
        let q = v.quick.view.as_ref().expect("浮层已创建").read(cx);
        let a = q.service.active().expect("浮层已打开");
        a.list
            .display_rows(0..a.list.len())
            .iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    r.label.to_string(),
                    r.detail.as_deref().unwrap_or("").to_owned(),
                    a.decor
                        .get(i)
                        .and_then(|d| d.trailing.as_deref())
                        .unwrap_or("")
                        .to_owned(),
                )
            })
            .collect()
    })
}

fn is_open(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, cx| {
        v.quick
            .view
            .as_ref()
            .is_some_and(|q| q.read(cx).service.active().is_some())
    })
}

fn empty_message(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, cx| {
        let q = v.quick.view.as_ref()?.read(cx);
        let m = q.service.active()?.config.empty.clone()?;
        Some(v.strings.text(&m).unwrap())
    })
}

#[gpui::test]
fn ctrl_p_types_selects_and_opens_with_the_line(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    assert!(!is_open(&view, cx));
    cx.simulate_keystrokes("ctrl-p");
    cx.run_until_parked();
    assert!(is_open(&view, cx));
    // 输入文件名与行号：每次按键同步刷新列表
    cx.simulate_input("plyr:6");
    cx.run_until_parked();
    let found = rows(&view, cx);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "Player.cs");
    assert_eq!(found[0].1, "src");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    refresh(cx);
    refresh(cx);
    assert!(!is_open(&view, cx));
    assert_eq!(active_resource(&view, cx).as_deref(), Some("src/Player.cs"));
    assert_eq!(cursor(&view, cx), pos(5, 0));
}

#[gpui::test]
fn arrows_enter_and_escape_work_from_the_keyboard(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input(".cs");
    cx.run_until_parked();
    let found = rows(&view, cx);
    assert_eq!(found.len(), 2, "两个 .cs 文件：{found:?}");
    // ↓ 选第二项，Enter 打开它
    let second = found[1].0.clone();
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    refresh(cx);
    let opened = active_resource(&view, cx).unwrap();
    assert!(opened.ends_with(&second), "{opened} / {second}");
    // Esc 关闭
    cx.simulate_keystrokes("ctrl-p");
    assert!(is_open(&view, cx));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!is_open(&view, cx));
}

#[gpui::test]
fn ctrl_enter_opens_beside(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "notes.txt", cx);
    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("enemy");
    cx.run_until_parked();
    cx.simulate_modifiers_change(gpui::Modifiers::control());
    cx.simulate_keystrokes("ctrl-enter");
    cx.simulate_modifiers_change(gpui::Modifiers::default());
    cx.run_until_parked();
    refresh(cx);
    refresh(cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.editor.groups.len(), 2, "在旁边打开：新编辑器组")
    });
    assert_eq!(active_resource(&view, cx).as_deref(), Some("src/Enemy.cs"));
}

#[gpui::test]
fn empty_query_lists_recent_files_first(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "src/Player.cs", cx);
    open_file(&view, "notes.txt", cx);
    cx.simulate_keystrokes("ctrl-p");
    cx.run_until_parked();
    let found = rows(&view, cx);
    let labels: Vec<_> = found.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(labels, ["notes.txt", "Player.cs"], "最近打开的在前");
    let groups = view.read_with(cx, |v, cx| {
        let a = v
            .quick
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .service
            .active()
            .unwrap();
        a.list.group_starts().to_vec()
    });
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].1.as_deref(), Some("最近打开"));
    cx.simulate_keystrokes("escape");

    // search.quickOpen.includeHistory = false：空查询没有历史
    let off = Fixture::new(r#"{ "search.quickOpen.includeHistory": false }"#);
    let (view, cx) = off.open(cx);
    open_file(&view, "notes.txt", cx);
    cx.simulate_keystrokes("ctrl-p");
    cx.run_until_parked();
    assert!(rows(&view, cx).is_empty());
}

#[gpui::test]
fn colon_goes_to_line_and_column_with_prompt_and_ctrl_g(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "notes.txt", cx);
    // Ctrl+G 与 `Ctrl+P` 输入 `:` 共用实现
    cx.simulate_keystrokes("ctrl-g");
    cx.run_until_parked();
    assert!(is_open(&view, cx));
    let prompt = empty_message(&view, cx).unwrap();
    assert!(
        prompt.contains("当前行: 1") && prompt.contains("1 到 6"),
        "{prompt}"
    );
    cx.simulate_input("4:2");
    cx.run_until_parked();
    let found = rows(&view, cx);
    assert_eq!(found[0].0, "转到行 4 和字符 2。");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    refresh(cx);
    assert_eq!(cursor(&view, cx), pos(3, 1));

    // 超出范围：只提示，Enter 不关闭
    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input(":99");
    cx.run_until_parked();
    assert!(rows(&view, cx).is_empty());
    assert!(empty_message(&view, cx).unwrap().contains("1 到 6"));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(cursor(&view, cx), pos(3, 1));
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn at_lists_symbols_and_degrades_without_an_outline(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open_file(&view, "src/Player.cs", cx);
    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("@move");
    cx.run_until_parked();
    let found = rows(&view, cx);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "Move(float)");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    refresh(cx);
    assert_eq!(cursor(&view, cx), pos(8, 20));

    // 纯文本没有符号：显示 UX_SPEC §6.3 的提示，不报错、不留空白
    open_file(&view, "notes.txt", cx);
    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("@x");
    cx.run_until_parked();
    assert!(rows(&view, cx).is_empty());
    assert_eq!(
        empty_message(&view, cx).as_deref(),
        Some("活动编辑器无法提供大纲信息。")
    );
    cx.simulate_keystrokes("escape");
}

/// #60：`#` 前缀改成真正的工作区符号搜索，M1 的“尚不可用”降级提示已删除。
/// （与 `Ctrl+T` 同一份数据、同一套排序的断言在 `nav_extra_tests` 里。）
#[gpui::test]
fn hash_searches_workspace_symbols_and_the_unavailable_hint_is_gone(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("#zzzznotasymbol");
    cx.run_until_parked();
    refresh(cx);
    cx.run_until_parked();
    assert!(rows(&view, cx).is_empty());
    assert_eq!(
        empty_message(&view, cx).as_deref(),
        Some("未找到匹配的符号")
    );
    assert!(
        view.read_with(cx, |v, _| v
            .strings
            .get("quickAccess.workspaceSymbols.unavailable")
            .is_none()),
        "降级提示的字符串不应再登记"
    );
}

#[gpui::test]
fn command_palette_lists_implemented_commands_with_their_keys(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    cx.simulate_keystrokes("ctrl-shift-p");
    cx.run_until_parked();
    assert!(is_open(&view, cx));
    let all = rows(&view, cx);
    assert!(all.len() > 10, "已实现的命令：{}", all.len());
    // 键位来自键位服务：快速打开显示 Ctrl+P（默认还绑定了 Ctrl+E，用户未改）
    let open = all
        .iter()
        .find(|r| r.0 == "转到文件...")
        .expect("转到文件...");
    assert_eq!(open.2, "Ctrl+P");
    // 内部导航命令与尚未实现的命令不列出
    assert!(!all.iter().any(|r| r.0.contains("list.focus")));
    let ids = view.read_with(cx, |v, cx| {
        let a = v
            .quick
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .service
            .active()
            .unwrap();
        a.list
            .display_rows(0..a.list.len())
            .iter()
            .map(|r| r.id.0.to_string())
            .collect::<Vec<_>>()
    });
    assert!(ids
        .iter()
        .all(|id| !id.starts_with("list.") && !id.starts_with("quickInput.")));
    assert!(!ids.iter().any(|id| id == "workbench.action.debug.start"));
    // 过滤并运行：切换侧边栏
    cx.simulate_input("侧边栏");
    cx.run_until_parked();
    let before = view.read_with(cx, |v, _| v.shell.layout.sidebar_visible);
    let found = rows(&view, cx);
    assert!(!found.is_empty(), "{found:?}");
    let toggle = found
        .iter()
        .position(|r| r.2 == "Ctrl+B")
        .expect("切换侧边栏可见性 Ctrl+B");
    cx.simulate_keystrokes(&format!("{}enter", "down ".repeat(toggle)));
    cx.run_until_parked();
    refresh(cx);
    refresh(cx);
    assert_ne!(
        view.read_with(cx, |v, _| v.shell.layout.sidebar_visible),
        before
    );
    assert!(!is_open(&view, cx));

    // 最近使用分组：刚运行过的排在最前
    cx.simulate_keystrokes("f1");
    cx.run_until_parked();
    let first = rows(&view, cx).remove(0);
    assert_eq!(first.2, "Ctrl+B");
    let groups = view.read_with(cx, |v, cx| {
        let a = v
            .quick
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .service
            .active()
            .unwrap();
        a.list
            .group_starts()
            .iter()
            .map(|g| g.1.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].as_deref(), Some("最近使用"));
    assert_eq!(groups[1].as_deref(), Some("其他命令"));
}

#[gpui::test]
fn user_keybinding_overrides_show_in_the_palette(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let user = tempfile::tempdir().unwrap();
    let keybindings = user.path().join("keybindings.json");
    std::fs::write(
        &keybindings,
        r#"[ { "key": "ctrl+b", "command": "-workbench.action.toggleSidebarVisibility" },
             { "key": "ctrl+alt+b", "command": "workbench.action.toggleSidebarVisibility" } ]"#,
    )
    .unwrap();
    cx.update(|cx| crate::keys::watch_user_file(keybindings, cx));
    let (view, cx) = fx.open(cx);
    cx.simulate_keystrokes("ctrl-shift-p");
    cx.simulate_input("侧边栏可见");
    cx.run_until_parked();
    let found = rows(&view, cx);
    assert!(found.iter().any(|r| r.2 == "Ctrl+Alt+B"), "{found:?}");
    assert!(!found.iter().any(|r| r.2 == "Ctrl+B"), "{found:?}");
}

#[gpui::test]
fn command_center_click_opens_quick_open(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    // 标题栏命令中心的 on_click 调用的就是这个入口。
    view.update_in(cx, |v, window, cx| {
        shell::view::run_command(v, "workbench.action.quickOpen", window, cx)
    });
    cx.run_until_parked();
    assert!(is_open(&view, cx));
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn files_listing_shows_loading_until_the_index_is_ready(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    // 模拟“工作区扫描未完成”：没有文件列表时立即可用，底部显示加载提示。
    view.update_in(cx, |v, window, cx| {
        v.quick.index = None;
        v.quick_access_command("workbench.action.quickOpen", window, cx);
    });
    cx.run_until_parked();
    let loading = view.read_with(cx, |v, cx| {
        let a = v
            .quick
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .service
            .active()
            .unwrap();
        a.list.loading_tail() && a.config.loading_tail.is_some()
    });
    assert!(loading);
    // 索引就绪后按当前输入刷新
    view.update(cx, |v, cx| v.quick_workspace_ready(cx));
    cx.run_until_parked();
    cx.simulate_input("enemy");
    cx.run_until_parked();
    assert_eq!(rows(&view, cx).len(), 1);
    let loading = view.read_with(cx, |v, cx| {
        v.quick
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .service
            .active()
            .unwrap()
            .list
            .loading_tail()
    });
    assert!(!loading);
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn new_files_appear_without_reopening(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    view.update(cx, |v, cx| {
        let created = WorkspacePath::new("src/Boss.cs").unwrap();
        v.quick_files_changed(
            vec![(u64::MAX, Arc::from([FileEvent::Created(created)]))],
            cx,
        );
    });
    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("boss");
    cx.run_until_parked();
    let found = rows(&view, cx);
    assert_eq!(found.first().map(|r| r.0.as_str()), Some("Boss.cs"));
}

#[gpui::test]
fn preserve_input_restores_the_previous_text(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{ "workbench.quickOpen.preserveInput": true }"#);
    let (view, cx) = fx.open(cx);
    cx.simulate_keystrokes("ctrl-p");
    cx.simulate_input("enemy");
    cx.run_until_parked();
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("ctrl-p");
    cx.run_until_parked();
    let text = view.read_with(cx, |v, cx| {
        v.quick
            .view
            .as_ref()
            .unwrap()
            .read(cx)
            .service
            .active()
            .unwrap()
            .text
            .clone()
    });
    assert_eq!(text, "enemy");
    assert_eq!(rows(&view, cx).len(), 1);
}

/// #30：浮层（快速打开 / 命令面板）在 100% / 125% / 150% / 200% 下位置与尺寸都是同一组逻辑像素，
/// 并且完整落在窗口里——不越界、不被裁切。副屏上的人工复核见 `docs/DPI_CHECKLIST.md`。
#[gpui::test]
fn quick_input_overlay_keeps_its_place_at_every_scale(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    cx.simulate_keystrokes("ctrl-shift-p");
    cx.run_until_parked();
    assert!(is_open(&view, cx));

    let mut baseline: Option<gpui::Bounds<gpui::Pixels>> = None;
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        cx.simulate_scale_factor_change(scale);
        cx.update(|window, cx| {
            window.refresh();
            cx.refresh_windows();
        });
        cx.run_until_parked();
        assert_eq!(cx.update(|window, _| window.scale_factor()), scale);
        assert!(is_open(&view, cx), "{scale}×：浮层不应被缩放变更关掉");

        let b = cx.debug_bounds("quick-input").expect("浮层可见");
        let viewport = cx.update(|window, _| window.viewport_size());
        assert!(
            f32::from(b.origin.x) >= 0.0 && f32::from(b.origin.y) >= 0.0,
            "{scale}×：浮层越过了窗口左上角 {b:?}"
        );
        assert!(
            b.right() <= viewport.width && b.bottom() <= viewport.height,
            "{scale}×：浮层被窗口裁切 {b:?} / {viewport:?}"
        );
        assert!(
            (f32::from(b.size.width) - crate::list_widget::quick_input_view::WIDTH).abs() < 1.0,
            "{scale}×：浮层宽度应当是固定的逻辑像素值，实际 {:?}",
            b.size.width
        );
        match baseline {
            None => baseline = Some(b),
            Some(base) => {
                for (a, c, what) in [
                    (b.origin.x, base.origin.x, "x"),
                    (b.origin.y, base.origin.y, "y"),
                    (b.size.width, base.size.width, "宽"),
                    (b.size.height, base.size.height, "高"),
                ] {
                    let d = (f32::from(a) - f32::from(c)).abs();
                    // GPUI 把布局对齐到物理像素网格，允许不到 1 逻辑像素的差。
                    assert!(d < 1.0, "{scale}×：浮层的{what}差了 {d} 逻辑像素");
                }
            }
        }
    }
}
