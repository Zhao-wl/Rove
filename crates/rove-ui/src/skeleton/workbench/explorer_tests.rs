//! #23 入口集成测试（真实 GPUI 窗口，真实的 #15 工作区与文件监听，只通过键位与点击驱动）：
//! 打开文件夹立即显示根目录且默认排除生效、紧凑文件夹与 `home` / `end` / `left` / `right`、
//! `enter` / `ctrl+enter` / `space` 与单击 / 双击的打开方式、`.md` 进入预览、自动定位与
//! `autoRevealExclude`、过滤时展开命中节点的父级并在清空后还原、外部批量增删后树与磁盘一致且
//! 展开状态与选中项保留、只读装饰、没有打开文件夹的空状态。
use super::*;
use crate::{
    documents::OpenEvent,
    explorer::{
        registry::{self, ExplorerAction},
        view::ExplorerEvent,
    },
    list_widget::{matcher::FindMode, view::ListView},
};
use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext,
};
use rove_config::{SettingsOptions, SettingsService};
use std::sync::Mutex;

struct Fixture {
    user: tempfile::TempDir,
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Fixture {
    fn new(settings_json: &str, files: &[&str]) -> Self {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("settings.json"), settings_json).unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        let folder = tempfile::tempdir().unwrap();
        for rel in files {
            let path = folder.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "class X {}\n").unwrap();
        }
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
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.folder.path().join(rel)
    }
}

/// 树当前的行：缩进两个空格 / 级，展开的目录以 `/` 结尾。
fn rows(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| {
        v.explorer
            .read(cx)
            .rows(cx)
            .into_iter()
            .map(|(depth, label, expanded)| {
                format!(
                    "{}{label}{}",
                    "  ".repeat(depth as usize),
                    if expanded { "/" } else { "" }
                )
            })
            .collect()
    })
}

fn selected(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| v.explorer.read(cx).selected(cx))
}

fn list(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<ListView> {
    view.read_with(cx, |v, cx| v.explorer.read(cx).list().clone())
}

/// 第 `i` 行的屏幕位置（#2 的骨架坐标：第一行在 y = 104，行高 24）。
fn row_at(i: usize) -> gpui::Point<gpui::Pixels> {
    point(px(128.0), px(104.0 + 24.0 * i as f32))
}

/// 让树获得键盘焦点并聚焦第 `i` 行。
fn focus_row(view: &Entity<Workbench>, i: usize, cx: &mut VisualTestContext) {
    let list = list(view, cx);
    list.update(cx, |l, cx| {
        l.widget.set_focus(Some(i));
        l.widget.set_selection(vec![i]);
        cx.notify();
    });
    cx.update(|window, cx| {
        let focus = list.read(cx).focus_handle(cx);
        window.focus(&focus);
    });
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn tabs(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(String, bool, bool)> {
    view.read_with(cx, |v, _| {
        v.editor
            .in_group(v.editor.focused)
            .into_iter()
            .map(|t| {
                let name = match &t.resource {
                    Resource::Workspace(p) => p.as_str().to_owned(),
                    other => format!("{other:?}"),
                };
                (name, t.preview, t.active)
            })
            .collect()
    })
}

fn wait_until(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    what: &str,
    done: impl Fn(&[String]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        let current = rows(view, cx);
        if done(&current) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: {current:?}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[gpui::test]
fn opening_a_folder_shows_the_root_at_once_without_unity_generated_directories(
    cx: &mut TestAppContext,
) {
    let fx = Fixture::new(
        "{}",
        &[
            "Assets/A.cs",
            "Library/PackageCache/pkg/P.cs",
            "Temp/t.cs",
            "obj/o.cs",
            "Logs/l.cs",
            "Readme.md",
        ],
    );
    let (view, cx) = fx.open(cx);
    assert_eq!(rows(&view, cx), ["Assets", "Readme.md"]);
    // 把 `**/Library` 改为 false 后 Library 出现（扫描随排除规则重新进行）。
    fx.change(
        r#"{ "rove.files.extraExclude": { "**/Library": false, "**/Temp": true, "**/obj": true, "**/Logs": true } }"#,
        cx,
    );
    wait_until(&view, cx, "Library 出现", |r| {
        r == ["Assets", "Library/PackageCache/pkg", "Readme.md"]
    });
    // 没有任何 Git 装饰：`.gitignore` 也不被读取。
    let git = fx.path(".gitignore");
    std::fs::write(&git, "Assets/\n").unwrap();
    view.read_with(cx, |v, cx| {
        let explorer = v.explorer.read(cx);
        let any = WorkspacePath::new("Assets/A.cs").unwrap();
        assert!(explorer.decoration(&any, false).is_none());
    });
}

#[gpui::test]
fn compressed_folders_navigate_inside_the_node_and_the_setting_turns_them_off(
    cx: &mut TestAppContext,
) {
    let fx = Fixture::new(
        "{}",
        &[
            "Assets/Scripts/Runtime/A.cs",
            "Assets/Scripts/Runtime/B.cs",
            "Z.txt",
        ],
    );
    let (view, cx) = fx.open(cx);
    wait_until(&view, cx, "紧凑文件夹", |r| {
        r == ["Assets/Scripts/Runtime", "Z.txt"]
    });
    focus_row(&view, 0, cx);
    let segment = |view: &Entity<Workbench>, cx: &mut VisualTestContext| {
        view.read_with(cx, |v, cx| {
            let explorer = v.explorer.read(cx);
            (
                explorer.focused_segment(cx),
                explorer.focused_path(cx).map(|p| p.as_str().to_owned()),
            )
        })
    };
    // 聚焦压缩节点时从最后一段开始。
    assert_eq!(
        segment(&view, cx),
        (2, Some("Assets/Scripts/Runtime".into()))
    );
    cx.simulate_keystrokes("left");
    assert_eq!(segment(&view, cx), (1, Some("Assets/Scripts".into())));
    cx.simulate_keystrokes("home");
    assert_eq!(segment(&view, cx), (0, Some("Assets".into())));
    // 已在第一段：`left` 回到普通的折叠（这里没有父节点，什么也不变）。
    cx.simulate_keystrokes("left");
    assert_eq!(segment(&view, cx).0, 0);
    assert_eq!(rows(&view, cx), ["Assets/Scripts/Runtime", "Z.txt"]);
    cx.simulate_keystrokes("right");
    assert_eq!(segment(&view, cx).0, 1);
    cx.simulate_keystrokes("end");
    assert_eq!(segment(&view, cx).0, 2);
    // 已在最后一段：`right` 展开节点。
    cx.simulate_keystrokes("right");
    assert_eq!(
        rows(&view, cx),
        ["Assets/Scripts/Runtime/", "  A.cs", "  B.cs", "Z.txt"]
    );
    // `explorer.compactFolders = false`：逐级显示，已展开的目录按路径保持展开。
    fx.change(r#"{ "explorer.compactFolders": false }"#, cx);
    assert_eq!(rows(&view, cx)[0], "Assets/");
    assert!(rows(&view, cx).contains(&"      A.cs".to_owned()));
}

#[gpui::test]
fn keys_and_clicks_open_files_through_the_open_service(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}", &["A.cs", "B.md", "C.cs", "D.cs"]);
    let (view, cx) = fx.open(cx);
    assert_eq!(rows(&view, cx), ["A.cs", "B.md", "C.cs", "D.cs"]);
    // 单击：预览标签，焦点留在树里。
    cx.simulate_click(row_at(0), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(tabs(&view, cx), [("A.cs".to_owned(), true, true)]);
    let in_tree = |view: &Entity<Workbench>, cx: &mut VisualTestContext| {
        let list = list(view, cx);
        cx.update(|window, cx| list.read(cx).focus_handle(cx).contains_focused(window, cx))
    };
    assert!(in_tree(&view, cx));
    // 双击转为正式标签。
    for count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: row_at(0),
            modifiers: Modifiers::default(),
            click_count: count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: row_at(0),
            modifiers: Modifiers::default(),
            click_count: count,
        });
    }
    cx.run_until_parked();
    assert_eq!(tabs(&view, cx), [("A.cs".to_owned(), false, true)]);
    // `space`：打开但保持焦点（预览）。
    focus_row(&view, 2, cx);
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    assert!(tabs(&view, cx).contains(&("C.cs".to_owned(), true, true)));
    assert!(in_tree(&view, cx));
    // `enter`：打开并移交焦点。
    focus_row(&view, 3, cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    // `enter` 打开的是正式标签（不是预览）；焦点移交由 #17 的打开服务按 `preserve_focus` 处理。
    assert!(tabs(&view, cx).contains(&("D.cs".to_owned(), false, true)));
    // `.md` 解析为 Markdown 预览（#29 合并前是占位的编辑器类型）。
    focus_row(&view, 1, cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert!(matches!(
            tab.kind,
            EditorKind::Placeholder { ref id, .. } if id.as_ref() == crate::editors::settings::MARKDOWN
        ));
        assert_eq!(tab.resource, Resource::Workspace(WorkspacePath::new("B.md").unwrap()));
    });
    // `ctrl+enter`：在侧边打开（第二个编辑器组）。
    focus_row(&view, 0, cx);
    let groups = view.read_with(cx, |v, _| v.editor.groups.len());
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(v.editor.groups.len(), groups + 1));
}

#[gpui::test]
fn the_active_editor_is_revealed_unless_auto_reveal_is_off_or_excluded(cx: &mut TestAppContext) {
    let fx = Fixture::new(
        "{}",
        &[
            "Assets/Scripts/Runtime/A.cs",
            "Assets/Scripts/Runtime/B.cs",
            "Assets/Skipped/S.cs",
            "Assets/Art/logo.txt",
            "Top.cs",
        ],
    );
    let (view, cx) = fx.open(cx);
    let open = |view: &Entity<Workbench>, cx: &mut VisualTestContext, rel: &str| {
        view.update(cx, |v, cx| {
            v.open_documents()
                .open(
                    Resource::Workspace(WorkspacePath::new(rel).unwrap()),
                    OpenOptions {
                        source: OpenSource::QuickOpen,
                        ..Default::default()
                    },
                    Cancellation::default(),
                )
                .unwrap();
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    };
    assert_eq!(rows(&view, cx), ["Assets", "Top.cs"]);
    open(&view, cx, "Assets/Scripts/Runtime/B.cs");
    // 紧凑文件夹：链上的目录一次展开，目标选中在正确的层级上。
    assert_eq!(selected(&view, cx), ["Assets/Scripts/Runtime/B.cs"]);
    let shown = rows(&view, cx);
    assert!(shown.iter().any(|r| r.trim() == "B.cs"), "{shown:?}");
    // `autoRevealExclude` 命中的路径不定位。
    fx.change(
        r#"{ "explorer.autoRevealExclude": { "**/Skipped": true } }"#,
        cx,
    );
    open(&view, cx, "Assets/Skipped/S.cs");
    assert_eq!(selected(&view, cx), ["Assets/Scripts/Runtime/B.cs"]);
    // `explorer.autoReveal = false`：切换活动编辑器时树不动。
    fx.change(r#"{ "explorer.autoReveal": false }"#, cx);
    open(&view, cx, "Assets/Art/logo.txt");
    assert_eq!(selected(&view, cx), ["Assets/Scripts/Runtime/B.cs"]);
    // 命令“在资源管理器中显示活动文件”无视设置，并展开到目标。
    cx.update(|window, cx| window.focus(&view.read(cx).focus));
    cx.update(|window, cx| {
        window.dispatch_action(Box::new(ExplorerAction(registry::SHOW_ACTIVE_FILE)), cx)
    });
    cx.run_until_parked();
    assert_eq!(selected(&view, cx), ["Assets/Art/logo.txt"]);
    // `ctrl+shift+e` 把焦点交给树。
    cx.simulate_keystrokes("ctrl-shift-e");
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let list = list(&view, cx);
    assert!(cx.update(|window, cx| list.read(cx).focus_handle(cx).contains_focused(window, cx)));
}

#[gpui::test]
fn filtering_expands_the_parents_of_matches_and_restores_the_tree_when_cleared(
    cx: &mut TestAppContext,
) {
    let fx = Fixture::new(
        "{}",
        &[
            "Assets/Scripts/Player.cs",
            "Assets/Scripts/Enemy.cs",
            "Assets/Art/logo.png",
            "Docs/readme.txt",
        ],
    );
    let (view, cx) = fx.open(cx);
    assert_eq!(rows(&view, cx), ["Assets", "Docs"]);
    let list = list(&view, cx);
    // 展开 Assets 与 Scripts（加载它们的子项），再折叠 Scripts：用户自己的展开状态。
    for (id, expanded) in [
        ("Assets", true),
        ("Assets/Scripts", true),
        ("Assets/Scripts", false),
    ] {
        list.update(cx, |l, _| {
            l.widget
                .expand_id(&crate::lists::ItemId(id.into()), expanded)
                .unwrap();
        });
    }
    let before = rows(&view, cx);
    assert_eq!(before, ["Assets/", "  Art", "  Scripts", "Docs"]);
    list.update(cx, |l, cx| {
        l.widget
            .set_filter_query(Some(("Player".into(), FindMode::Filter)));
        cx.notify();
    });
    assert_eq!(rows(&view, cx), ["Assets/", "  Scripts/", "    Player.cs"]);
    list.update(cx, |l, cx| {
        l.widget.set_filter_query(None);
        cx.notify();
    });
    assert_eq!(rows(&view, cx), before);
}

#[gpui::test]
fn external_bulk_changes_keep_the_tree_in_sync_with_disk_and_keep_marks(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}", &["Assets/keep.cs", "Other/x.cs", "Top.cs"]);
    let (view, cx) = fx.open(cx);
    assert_eq!(rows(&view, cx), ["Assets", "Other", "Top.cs"]);
    // 展开 Assets 并选中 keep.cs。
    cx.simulate_click(row_at(0), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(rows(&view, cx), ["Assets/", "  keep.cs", "Other", "Top.cs"]);
    cx.simulate_click(row_at(1), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(selected(&view, cx), ["Assets/keep.cs"]);
    // 脚本一次创建 500 个文件。
    for i in 0..500 {
        std::fs::write(fx.path(&format!("Assets/gen{i:03}.cs")), "x").unwrap();
    }
    wait_until(&view, cx, "500 个新文件", |r| r.len() == 504);
    let after_create = rows(&view, cx);
    assert_eq!(after_create[0], "Assets/", "展开状态保留");
    assert!(after_create.contains(&"  gen499.cs".to_owned()));
    assert_eq!(selected(&view, cx), ["Assets/keep.cs"], "选中项保留");
    // 再一次删除 500 个。
    for i in 0..500 {
        std::fs::remove_file(fx.path(&format!("Assets/gen{i:03}.cs"))).unwrap();
    }
    wait_until(&view, cx, "删除 500 个文件", |r| r.len() == 4);
    assert_eq!(rows(&view, cx), ["Assets/", "  keep.cs", "Other", "Top.cs"]);
    assert_eq!(selected(&view, cx), ["Assets/keep.cs"]);
    // 重命名一个已展开目录的父目录、删除一个已展开的目录：树最终与磁盘一致，不 panic。
    cx.simulate_click(row_at(2), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(rows(&view, cx)[2], "Other/");
    std::fs::rename(fx.path("Other"), fx.path("Other2")).unwrap();
    wait_until(&view, cx, "重命名", |r| {
        r.iter().any(|l| l.starts_with("Other2"))
    });
    std::fs::remove_dir_all(fx.path("Other2")).unwrap();
    wait_until(&view, cx, "删除目录", |r| {
        r == ["Assets/", "  keep.cs", "Top.cs"]
    });
}

#[gpui::test]
fn read_only_rules_decorate_files_and_there_is_no_git_decoration(cx: &mut TestAppContext) {
    let fx = Fixture::new(
        r#"{ "rove.files.extraExclude": { "**/Library": false, "**/Temp": true, "**/obj": true, "**/Logs": true } }"#,
        &["Library/PackageCache/pkg/P.cs", "Assets/A.cs"],
    );
    let (view, cx) = fx.open(cx);
    view.read_with(cx, |v, cx| {
        let explorer = v.explorer.read(cx);
        let decoration = |rel: &str| explorer.decoration(&WorkspacePath::new(rel).unwrap(), false);
        let lock = decoration("Library/PackageCache/pkg/P.cs").expect("只读装饰");
        assert_eq!(lock.badge, Some(crate::explorer::decorations::Badge::Lock));
        assert!(decoration("Assets/A.cs").is_none());
    });
    // `workbench.editor.decorations.badges = false` 只影响绘制，数据来源不变。
    fx.change(r#"{ "workbench.editor.decorations.badges": false }"#, cx);
    view.read_with(cx, |v, cx| assert!(!v.explorer.read(cx).settings().badges));
}

#[gpui::test]
fn without_a_folder_the_explorer_shows_the_empty_state_and_its_button_runs_open_folder(
    cx: &mut TestAppContext,
) {
    let fx = Fixture::new("{}", &["A.cs"]);
    let (view, cx) = fx.open(cx);
    let commands = Arc::new(Mutex::new(Vec::new()));
    let captured = commands.clone();
    let explorer = view.read_with(cx, |v, _| v.explorer.clone());
    let _subscription = cx.update(|_, app| {
        app.subscribe(&explorer, move |_, event: &ExplorerEvent, _| {
            captured.lock().unwrap().push(event.clone());
        })
    });
    explorer.update(cx, |e, cx| e.set_workspace(None, cx));
    cx.run_until_parked();
    assert!(rows(&view, cx).is_empty());
    view.read_with(cx, |v, cx| assert!(!v.explorer.read(cx).has_workspace()));
    // 空状态里的按钮：按钮发出的命令由宿主执行（对话框由 #28 / #16 提供）。
    list(&view, cx).update(cx, |_, cx| {
        cx.emit(crate::list_widget::view::ListViewEvent::EmptyAction(
            registry::OPEN_FOLDER.into(),
        ))
    });
    cx.run_until_parked();
    assert_eq!(
        *commands.lock().unwrap(),
        [ExplorerEvent::Command(registry::OPEN_FOLDER.into())]
    );
    // 重新给一个工作区，树立即恢复。
    let workspace = view.read_with(cx, |v, _| v.workspace.clone());
    explorer.update(cx, |e, cx| e.set_workspace(workspace, cx));
    assert_eq!(rows(&view, cx), ["A.cs"]);
}

#[gpui::test]
fn files_that_fail_to_open_do_not_break_the_tree(cx: &mut TestAppContext) {
    // 树里的文件在点击之前被删除：打开失败走 #17 的失败页，树仍然可用。
    let fx = Fixture::new("{}", &["A.cs", "B.cs"]);
    let (view, cx) = fx.open(cx);
    std::fs::remove_file(fx.path("A.cs")).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let _subscription = view.update(cx, |v, _| {
        v.open_documents()
            .events()
            .subscribe(Arc::new(move |event| {
                captured.lock().unwrap().push(event);
            }))
    });
    cx.simulate_click(row_at(0), Modifiers::default());
    cx.run_until_parked();
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .all(|e| matches!(e, OpenEvent::Ready { .. } | OpenEvent::Failed { .. })));
    assert!(!rows(&view, cx).is_empty());
}
