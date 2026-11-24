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

fn branch_commit(
    repo: &git2::Repository,
    author: &str,
    summary: &str,
    edits: &[(&str, Option<&str>)],
) -> git2::Oid {
    let root = repo.workdir().unwrap();
    let mut index = repo.index().unwrap();
    for (name, contents) in edits {
        let path = std::path::Path::new(name);
        let disk = root.join(path);
        if let Some(contents) = contents {
            std::fs::write(disk, contents).unwrap();
            index.add_path(path).unwrap();
        } else {
            std::fs::remove_file(disk).unwrap();
            index.remove_path(path).unwrap();
        }
    }
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let who = git2::Signature::now(author, &format!("{author}@example.test")).unwrap();
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    let parents = parent.iter().collect::<Vec<_>>();
    repo.commit(Some("HEAD"), &who, &who, summary, &tree, &parents)
        .unwrap()
}

#[gpui::test]
fn explorer_branch_menu_filters_real_commit_history(cx: &mut TestAppContext) {
    check_branch_menu_filters(cx, "main");
}

#[gpui::test]
fn explorer_branch_menu_uses_origin_default_without_main(cx: &mut TestAppContext) {
    check_branch_menu_filters(cx, "dev");
}

fn check_branch_menu_filters(cx: &mut TestAppContext, base_branch: &str) {
    let fixture = Fixture::new("{}", &["modified.txt", "deleted.txt"]);
    let repo = git2::Repository::init_opts(
        fixture.folder.path(),
        git2::RepositoryInitOptions::new().initial_head(base_branch),
    )
    .unwrap();
    let base = branch_commit(
        &repo,
        "Alice",
        "base",
        &[("modified.txt", Some("old")), ("deleted.txt", Some("old"))],
    );
    repo.branch("feature", &repo.find_commit(base).unwrap(), false)
        .unwrap();
    repo.set_head("refs/heads/feature").unwrap();
    if base_branch == "dev" {
        repo.reference("refs/remotes/origin/dev", base, false, "test default")
            .unwrap();
        repo.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/dev",
            false,
            "test default",
        )
        .unwrap();
        repo.find_branch("dev", git2::BranchType::Local)
            .unwrap()
            .delete()
            .unwrap();
    }
    branch_commit(
        &repo,
        "Alice",
        "add files",
        &[
            ("added.txt", Some("new")),
            ("temporary.txt", Some("temporary")),
        ],
    );
    branch_commit(
        &repo,
        "Bob",
        "modify and delete",
        &[
            ("modified.txt", Some("new")),
            ("deleted.txt", None),
            ("temporary.txt", None),
        ],
    );
    std::fs::write(fixture.path("working.txt"), "untracked").unwrap();
    let (view, cx) = fixture.open(cx);
    let click = |cx: &mut VisualTestContext, selector: &'static str| {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is missing"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
    };

    assert_eq!(
        rows(&view, cx),
        ["added.txt", "modified.txt", "working.txt"]
    );
    click(cx, "explorer-branch-filter");
    click(cx, "explorer-branch-toggle");
    assert_eq!(
        rows(&view, cx),
        ["added.txt", "deleted.txt", "modified.txt"]
    );

    click(cx, "explorer-branch-author");
    click(cx, "explorer-branch-option-1");
    assert_eq!(rows(&view, cx), ["added.txt"]);
    click(cx, "explorer-branch-commit");
    click(cx, "explorer-branch-option-1");
    assert_eq!(rows(&view, cx), ["added.txt", "temporary.txt"]);

    click(cx, "explorer-branch-toggle");
    assert_eq!(
        rows(&view, cx),
        ["added.txt", "modified.txt", "working.txt"]
    );
    click(cx, "explorer-branch-toggle");
    assert_eq!(rows(&view, cx), ["added.txt", "temporary.txt"]);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    // GPUI retains debug_bounds across frames; inspect the live menu state.
    assert!(!view.read_with(cx, |view, cx| view.explorer.read(cx).branch_menu_is_open()));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, cx| view.explorer.read(cx).branch_menu_is_open()));
}

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

/// #272：切回标签时，目标行恰好在可视区外一行（上方会被粘性目录行盖住），
/// 定位后整行可见、不被粘性区遮挡，并且选中的是 `.cs` 本身而不是同名前缀的 `.cs.meta`。
#[gpui::test]
fn switching_tabs_reveals_the_exact_file_fully_below_the_sticky_band(cx: &mut TestAppContext) {
    let mut files = vec![
        "Assets/Scripts/A.cs".to_owned(),
        "Assets/Scripts/A.cs.meta".to_owned(),
        "Assets/Scripts/B.cs".to_owned(),
    ];
    files.extend((0..120).map(|i| format!("Assets/Scripts/Z{i:03}.cs")));
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    let fx = Fixture::new("{}", &files);
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
    let list = list(&view, cx);
    // 焦点行完整可见，且不在粘性区下面。
    let assert_visible = |cx: &mut VisualTestContext, what: &str| {
        list.read_with(cx, |l, _| {
            let w = &l.widget;
            let i = w.focus().expect("focused row");
            let band: f32 = w.sticky_rows().iter().map(|&r| w.row_height(r)).sum();
            let (top, bottom) = (w.row_top(i), w.row_top(i) + w.row_height(i));
            assert!(w.viewport() > 0.0);
            assert!(
                top >= w.scroll_top() + band && bottom <= w.scroll_top() + w.viewport(),
                "{what}: row {i} [{top}, {bottom}) scroll {} band {band} viewport {}",
                w.scroll_top(),
                w.viewport()
            );
        });
    };

    open(&view, cx, "Assets/Scripts/A.cs");
    assert_eq!(selected(&view, cx), ["Assets/Scripts/A.cs"]);
    open(&view, cx, "Assets/Scripts/Z119.cs");
    assert_eq!(selected(&view, cx), ["Assets/Scripts/Z119.cs"]);
    assert_visible(cx, "below the viewport");

    // 把 A.cs 放到可视区上方恰好一行，再切回它的标签。
    list.update(cx, |l, cx| {
        let w = &mut l.widget;
        let a = w
            .tree_model()
            .and_then(|t| t.index_of(&crate::lists::ItemId("Assets/Scripts/A.cs".into())))
            .unwrap();
        let y = w.row_top(a) + w.row_height(a);
        w.set_scroll_top(y);
        cx.notify();
    });
    cx.run_until_parked();
    open(&view, cx, "Assets/Scripts/A.cs");
    assert_eq!(selected(&view, cx), ["Assets/Scripts/A.cs"]);
    assert_visible(cx, "one row above the viewport");

    // 反方向：目标在可视区下方。
    list.update(cx, |l, cx| {
        l.widget.set_scroll_top(0.0);
        cx.notify();
    });
    cx.run_until_parked();
    open(&view, cx, "Assets/Scripts/Z119.cs");
    assert_eq!(selected(&view, cx), ["Assets/Scripts/Z119.cs"]);
    assert_visible(cx, "below the viewport again");
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

#[gpui::test]
fn context_basket_explorer_menu_preserves_selected_files_and_follows_a_rename(
    cx: &mut TestAppContext,
) {
    use crate::context_basket::registry as basket;

    let fixture = Fixture::new("{}", &["Folder/A.cs", "B.cs"]);
    std::fs::write(fixture.path("Folder/A.cs"), "class A {}\n// two").unwrap();
    std::fs::write(fixture.path("B.cs"), "class B {}\n// two\n// three").unwrap();
    let (view, cx) = fixture.open(cx);
    view.update(cx, |view, _| view.basket.use_test_state());

    focus_row(&view, 0, cx);
    cx.simulate_keystrokes("right");
    wait_until(&view, cx, "expanded fixture folder", |rows| {
        rows == ["Folder/", "  A.cs", "B.cs"]
    });
    let list = list(&view, cx);
    list.update(cx, |view, cx| {
        view.widget.set_focus(Some(0));
        view.widget.set_selection(vec![0, 1, 2]);
        cx.notify();
    });

    cx.simulate_keystrokes("shift-f10");
    assert!(cx.debug_bounds(registry::NEW_FILE).is_some());
    assert!(cx.debug_bounds(registry::RENAME_FILE).is_some());
    let add = cx
        .debug_bounds(basket::ADD)
        .expect("basket entry in Explorer menu");
    cx.simulate_click(add.center(), Modifiers::default());
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        let [a, b] = view.basket.contents.items() else {
            panic!("the selected descendant and the sibling file must both be collected");
        };
        assert_eq!(
            (a.path.as_str(), a.lines.first(), a.lines.last()),
            ("Folder/A.cs", 1, 2)
        );
        assert_eq!(
            (b.path.as_str(), b.lines.first(), b.lines.last()),
            ("B.cs", 1, 3)
        );
    });

    focus_row(&view, 1, cx);
    cx.simulate_keystrokes("shift-f10");
    assert!(cx.debug_bounds(basket::ADD).is_some());
    let rename = cx
        .debug_bounds(registry::RENAME_FILE)
        .expect("rename entry in the same Explorer menu");
    cx.simulate_click(rename.center(), Modifiers::default());
    cx.run_until_parked();
    cx.simulate_input("Renamed");
    cx.simulate_keystrokes("enter");
    wait_until(&view, cx, "rename from the shared context menu", |rows| {
        rows == ["Folder/", "  Renamed.cs", "B.cs"]
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        let moved = view.read_with(cx, |view, _| {
            view.basket.contents.items().first().is_some_and(|item| {
                item.path.as_str() == "Folder/Renamed.cs"
                    && item.availability == crate::context_basket::model::Availability::Current
                    && item.lines.first() == 1
                    && item.lines.last() == 2
            })
        });
        if moved {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the basket did not follow the file rename"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(!fixture.path("Folder/A.cs").exists());
    assert!(fixture.path("Folder/Renamed.cs").is_file());
}

#[cfg(windows)]
#[gpui::test]
fn explorer_inline_create_rename_remaps_the_open_buffer_and_save_target(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        r#"{"workbench.editor.closeOnFileDelete": true, "explorer.copyRelativePathSeparator": "/"}"#,
        &["Seed.txt"],
    );
    let (view, cx) = fixture.open(cx);
    focus_row(&view, 0, cx);
    cx.update(|window, cx| {
        window.dispatch_action(Box::new(ExplorerAction(registry::NEW_FILE)), cx);
        window.refresh();
    });
    cx.run_until_parked();
    cx.simulate_input("Src/Foo.cs");
    cx.simulate_keystrokes("enter");
    wait_until(&view, cx, "new file added by the watcher", |rows| {
        rows.iter().any(|row| row.trim() == "Foo.cs")
    });
    assert!(fixture.path("Src/Foo.cs").is_file());
    let document = view.read_with(cx, |v, _| {
        v.editor
            .active()
            .current()
            .unwrap()
            .opened()
            .unwrap()
            .document
            .id
    });

    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| {
        window.focus(&code.read(cx).focus_handle(cx));
        window.refresh();
    });
    cx.run_until_parked();
    cx.simulate_input("edited");
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.editor.active().current().unwrap().dirty));

    let index = rows(&view, cx)
        .iter()
        .position(|row| row.trim() == "Foo.cs")
        .unwrap();
    focus_row(&view, index, cx);
    cx.simulate_keystrokes("f2");
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.simulate_input("foo");
    cx.simulate_keystrokes("enter");
    wait_until(&view, cx, "case-only rename from F2", |rows| {
        rows.iter().any(|row| row.trim() == "foo.cs")
    });
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert_eq!(
            tab.resource,
            Resource::Workspace(WorkspacePath::new("Src/foo.cs").unwrap())
        );
        assert_eq!(tab.opened().unwrap().document.id, document);
        assert!(tab.dirty);
        assert_eq!(
            v.editing.saves.borrow().file(document).unwrap().path(),
            Some(fixture.path("Src/foo.cs").as_path())
        );
    });

    cx.update(|window, cx| {
        window.dispatch_action(Box::new(ExplorerAction(registry::COPY_RELATIVE_PATH)), cx);
    });
    cx.run_until_parked();
    let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied.as_deref(), Some("Src/foo.cs"));

    cx.update(|window, cx| {
        window.focus(&code.read(cx).focus_handle(cx));
        window.refresh();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-s");
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(fixture.path("Src/foo.cs")).unwrap(),
        "edited"
    );
}

#[cfg(windows)]
fn wait_for_structured_problems(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    expected: Option<&Resource>,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let problems = view.read_with(cx, |v, cx| v.listed_problem_positions(cx));
        let ready = match expected {
            Some(resource) => {
                !problems.is_empty() && problems.iter().all(|(path, _)| path == resource)
            }
            None => problems.is_empty(),
        };
        if ready {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected structured problems for {expected:?}, got {problems:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(windows)]
fn open_structured_fixture(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    path: &WorkspacePath,
) -> DocumentId {
    let resource = Resource::Workspace(path.clone());
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                resource.clone(),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    wait_for_structured_problems(view, cx, Some(&resource));
    view.read_with(cx, |v, _| v.focused_text().unwrap().1.id)
}

#[cfg(windows)]
#[gpui::test]
fn explorer_structured_problems_follow_rename_move_undo_and_delete(cx: &mut TestAppContext) {
    use crate::explorer::decorations::DecorationProvider;

    let fixture = Fixture::new(
        r#"{"workbench.editor.closeOnFileDelete":true,"explorer.confirmDragAndDrop":false}"#,
        &[
            "Bad.json",
            "Target/Seed.txt",
            "Other.txt",
            "OtherTarget/Seed.txt",
        ],
    );
    std::fs::write(fixture.path("Bad.json"), r#"{"value": [ }"#).unwrap();
    let (view, cx) = fixture.open(cx);
    let original = WorkspacePath::new("Bad.json").unwrap();
    let document = open_structured_fixture(&view, cx, &original);
    let version = view.read_with(cx, |v, _| v.open_document(document).unwrap().text.version());

    let index = rows(&view, cx)
        .iter()
        .position(|row| row == "Bad.json")
        .unwrap();
    focus_row(&view, index, cx);
    cx.simulate_keystrokes("f2");
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.simulate_input("Renamed");
    cx.simulate_keystrokes("enter");
    let renamed = WorkspacePath::new("Renamed.json").unwrap();
    wait_for_structured_problems(&view, cx, Some(&Resource::Workspace(renamed.clone())));
    view.read_with(cx, |v, cx| {
        let explorer = v.explorer.read(cx);
        assert!(explorer.problems().decorate(&original, false).is_none());
        assert!(explorer.problems().decorate(&renamed, false).is_some());
    });

    let target = WorkspacePath::new("Target").unwrap();
    let explorer = view.read_with(cx, |v, _| v.explorer.clone());
    explorer.update(cx, |_, cx| {
        cx.emit(ExplorerEvent::MoveDropped {
            paths: vec![renamed.clone()],
            parent: Some(target.clone()),
        });
    });
    let moved = WorkspacePath::new("Target/Renamed.json").unwrap();
    let resource = Resource::Workspace(moved.clone());
    wait_for_structured_problems(&view, cx, Some(&resource));
    assert!(!fixture.path("Renamed.json").exists());
    assert!(fixture.path("Target/Renamed.json").is_file());
    view.read_with(cx, |v, cx| {
        let current = v.open_document(document).unwrap();
        assert_eq!(current.resource, resource);
        assert_eq!(current.text.version(), version);
        let explorer = v.explorer.read(cx);
        assert!(explorer.problems().decorate(&renamed, false).is_none());
        assert!(explorer.problems().decorate(&moved, false).is_some());
        assert!(explorer.problems().decorate(&target, true).is_some());
    });

    // A queued removal belongs to the previous occupant of the moved path.
    view.update(cx, |v, cx| {
        v.apply_explorer_workspace_events(vec![FileEvent::Removed(moved.clone())], cx);
    });
    wait_for_structured_problems(&view, cx, Some(&resource));

    let index = rows(&view, cx)
        .iter()
        .position(|row| row.trim_end_matches('/') == "Target")
        .unwrap();
    focus_row(&view, index, cx);
    cx.simulate_keystrokes("ctrl-z");
    let restored = Resource::Workspace(renamed.clone());
    wait_for_structured_problems(&view, cx, Some(&restored));
    assert!(!fixture.path("Target/Renamed.json").exists());
    assert!(fixture.path("Renamed.json").is_file());
    view.read_with(cx, |v, cx| {
        let current = v.open_document(document).unwrap();
        assert_eq!(current.resource, restored);
        assert_eq!(current.text.version(), version);
        let explorer = v.explorer.read(cx);
        assert!(explorer.problems().decorate(&target, true).is_none());
        assert!(explorer.problems().decorate(&renamed, false).is_some());
    });

    // Complete a real, unrelated operation before the old notification arrives.
    let other = WorkspacePath::new("Other.txt").unwrap();
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(other.clone()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    cx.run_until_parked();
    explorer.update(cx, |_, cx| {
        cx.emit(ExplorerEvent::MoveDropped {
            paths: vec![other],
            parent: Some(WorkspacePath::new("OtherTarget").unwrap()),
        });
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if tabs(&view, cx)
            .iter()
            .any(|(path, _, _)| path == "OtherTarget/Other.txt")
        {
            break;
        }
        assert!(Instant::now() < deadline, "unrelated move did not finish");
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(fixture.path("OtherTarget/Other.txt").is_file());

    let forward = FileEvent::Renamed {
        from: renamed.clone(),
        to: moved.clone(),
    };
    view.update(cx, |v, cx| {
        v.apply_explorer_workspace_events(vec![forward.clone()], cx);
    });
    view.read_with(cx, |v, _| {
        let current = v.open_document(document).unwrap();
        assert_eq!(
            current.resource, restored,
            "a late forward event cannot undo the undo"
        );
        assert_eq!(current.text.version(), version);
    });
    wait_for_structured_problems(&view, cx, Some(&restored));

    // An actual series of renames can leave the same path pair and existence pattern.
    // The replacement at the original path is a different filesystem object.
    let next = WorkspacePath::new("Target/Next.json").unwrap();
    view.update(cx, |v, cx| {
        let source = fixture.path("Renamed.json");
        let intermediate = fixture.path("Target/Renamed.json");
        std::fs::rename(&source, &intermediate).unwrap();
        std::fs::rename(&intermediate, fixture.path("Target/Next.json")).unwrap();
        std::fs::write(source, "{}").unwrap();
        v.apply_explorer_workspace_events(
            vec![
                forward,
                FileEvent::Renamed {
                    from: moved.clone(),
                    to: next.clone(),
                },
            ],
            cx,
        );
    });
    let recent = Resource::Workspace(next.clone());
    wait_for_structured_problems(&view, cx, Some(&recent));
    view.read_with(cx, |v, _| {
        let current = v.open_document(document).unwrap();
        assert_eq!(current.resource, recent);
        assert_eq!(current.text.version(), version);
    });
    std::fs::write(fixture.path("Unrelated.txt"), "external").unwrap();
    wait_until(
        &view,
        cx,
        "unrelated external creation is visible",
        |rows| rows.iter().any(|row| row == "Unrelated.txt"),
    );
    std::fs::remove_file(fixture.path("Target/Next.json")).unwrap();
    wait_for_structured_problems(&view, cx, None);
    assert!(!fixture.path("Target/Next.json").exists());
    view.read_with(cx, |v, cx| {
        assert!(v.open_document(document).is_none());
        let explorer = v.explorer.read(cx);
        assert!(explorer.problems().decorate(&next, false).is_none());
        assert!(explorer.problems().decorate(&target, true).is_none());
    });
}

#[cfg(windows)]
#[gpui::test]
#[ignore = "requires a Windows desktop with the Recycle Bin enabled for the test volume"]
fn explorer_structured_problems_clear_on_recycle_and_return_after_undo(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        r#"{"workbench.editor.closeOnFileDelete":true,"explorer.confirmDelete":false,"files.enableTrash":true}"#,
        &["Bad.json", "Seed.txt"],
    );
    std::fs::write(fixture.path("Bad.json"), r#"{"value": [ }"#).unwrap();
    let (view, cx) = fixture.open(cx);
    let path = WorkspacePath::new("Bad.json").unwrap();
    let document = open_structured_fixture(&view, cx, &path);
    focus_row(&view, 0, cx);
    cx.simulate_keystrokes("delete");
    wait_for_structured_problems(&view, cx, None);
    assert!(!fixture.path("Bad.json").exists());
    view.read_with(cx, |v, _| assert!(v.open_document(document).is_none()));

    focus_row(&view, 0, cx);
    cx.simulate_keystrokes("ctrl-z");
    wait_until(&view, cx, "structured file restored by undo", |rows| {
        rows.iter().any(|row| row == "Bad.json")
    });
    assert_eq!(
        std::fs::read_to_string(fixture.path("Bad.json")).unwrap(),
        r#"{"value": [ }"#
    );
    open_structured_fixture(&view, cx, &path);
}

#[gpui::test]
fn explorer_permanent_delete_requires_confirmation_even_when_recycle_prompt_is_off(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(
        r#"{"files.enableTrash": false, "explorer.confirmDelete": false}"#,
        &["Delete.txt"],
    );
    let (view, cx) = fixture.open(cx);
    focus_row(&view, 0, cx);
    cx.simulate_keystrokes("shift-delete");
    cx.run_until_parked();
    assert!(fixture.path("Delete.txt").exists());
    let key = view.read_with(cx, |v, _| {
        let modal = v.shell.feedback.modal().expect("permanent confirmation");
        assert_eq!(modal.message.key.as_ref(), "explorer.delete.permanent");
        assert!(modal.checkbox.is_none());
        modal.key.clone()
    });
    view.update(cx, |v, _| {
        v.shell
            .feedback
            .act(&key, registry::FILE_OP_CONFIRM)
            .unwrap()
    });
    wait_until(&view, cx, "confirmed deletion reaches the tree", |rows| {
        rows.is_empty()
    });
    assert!(!fixture.path("Delete.txt").exists());
}

#[cfg(windows)]
#[gpui::test]
#[ignore = "requires a Windows desktop with the Recycle Bin enabled for the test volume"]
fn explorer_create_rename_recycle_then_undo_through_the_workbench(cx: &mut TestAppContext) {
    let fixture = Fixture::new(r#"{"explorer.confirmDelete": false}"#, &["Seed.txt"]);
    let (view, cx) = fixture.open(cx);
    focus_row(&view, 0, cx);
    cx.update(|window, cx| {
        window.dispatch_action(Box::new(ExplorerAction(registry::NEW_FILE)), cx);
        window.refresh();
    });
    cx.run_until_parked();
    cx.simulate_input("Created.txt");
    cx.simulate_keystrokes("enter");
    wait_until(&view, cx, "created entry", |rows| {
        rows.iter().any(|row| row == "Created.txt")
    });
    let index = rows(&view, cx)
        .iter()
        .position(|row| row == "Created.txt")
        .unwrap();
    focus_row(&view, index, cx);
    cx.simulate_keystrokes("f2");
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.simulate_input("Renamed");
    cx.simulate_keystrokes("enter");
    wait_until(&view, cx, "renamed entry", |rows| {
        rows.iter().any(|row| row == "Renamed.txt")
    });
    let index = rows(&view, cx)
        .iter()
        .position(|row| row == "Renamed.txt")
        .unwrap();
    focus_row(&view, index, cx);
    cx.simulate_keystrokes("delete");
    wait_until(&view, cx, "recycled entry", |rows| rows == ["Seed.txt"]);
    assert!(!fixture.path("Renamed.txt").exists());
    view.read_with(cx, |v, _| assert!(v.shell.feedback.modal().is_none()));
    focus_row(&view, 0, cx);
    cx.simulate_keystrokes("ctrl-z");
    wait_until(&view, cx, "restored by undo", |rows| {
        rows.iter().any(|row| row == "Renamed.txt")
    });
    assert!(fixture.path("Renamed.txt").is_file());
}

#[cfg(windows)]
#[gpui::test]
#[ignore = "requires a Windows desktop with the Recycle Bin enabled for the test volume"]
fn explorer_batch_recycle_persists_the_confirmation_choice_without_reformatting(
    cx: &mut TestAppContext,
) {
    let settings = r#"{
    // Keep my existing comment and indentation.
    "explorer.confirmDelete": true,
    "files.enableTrash": true
}"#;
    let fixture = Fixture::new(settings, &["First.txt", "Second.txt", "Untouched.txt"]);
    let (view, cx) = fixture.open(cx);
    focus_row(&view, 0, cx);
    list(&view, cx).update(cx, |list, cx| {
        list.widget.set_selection(vec![0, 1]);
        cx.notify();
    });
    cx.simulate_keystrokes("delete");
    cx.run_until_parked();

    let key = view.read_with(cx, |v, _| {
        let modal = v
            .shell
            .feedback
            .modal()
            .expect("single recycle confirmation");
        assert_eq!(modal.message.key.as_ref(), "explorer.delete.recycle");
        assert!(modal
            .message
            .args
            .iter()
            .any(|(key, count)| { key.as_ref() == "count" && count.as_ref() == "2" }));
        assert!(modal.checkbox.is_some());
        modal.key.clone()
    });
    assert!(fixture.path("First.txt").exists());
    assert!(fixture.path("Second.txt").exists());
    view.update(cx, |v, _| {
        v.shell.feedback.toggle_modal_checkbox(&key);
        v.shell
            .feedback
            .act(&key, registry::FILE_OP_CONFIRM)
            .unwrap();
    });
    wait_until(&view, cx, "both items reach the recycle bin", |rows| {
        rows == ["Untouched.txt"]
    });
    view.read_with(cx, |v, _| assert!(v.shell.feedback.modal().is_none()));
    let stored = std::fs::read_to_string(fixture.user.path().join("settings.json")).unwrap();
    assert_eq!(
        stored,
        settings.replace("Delete\": true", "Delete\": false")
    );

    focus_row(&view, 0, cx);
    cx.simulate_keystrokes("ctrl-z");
    wait_until(&view, cx, "one undo restores both items", |rows| {
        rows == ["First.txt", "Second.txt", "Untouched.txt"]
    });
}
