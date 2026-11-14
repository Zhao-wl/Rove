//! #28 的入口集成测试：欢迎页、最近打开、启动参数、共用文件提示条与颜色主题选择器。
//!
//! 共用设置文件用真实的 `rove_config::SettingsService`（临时用户目录、关闭轮询），
//! 所以「只改一个键、注释与格式不变」是在真实文件上验证的。
use super::*;
use crate::{
    feedback::FeedbackService,
    shell::feedback::NEVER_SHOW_AGAIN,
    welcome::{recent::RecentKind, registry as reg, view as page},
};
use gpui::{TestAppContext, VisualTestContext};
use rove_config::{SettingsOptions, SettingsService};

struct Fixture {
    user: tempfile::TempDir,
    state: tempfile::TempDir,
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Fixture {
    fn new(settings_json: &str) -> Self {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("settings.json"), settings_json).unwrap();
        std::fs::write(user.path().join("keybindings.json"), "[]\n").unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        Self {
            user,
            state: tempfile::tempdir().unwrap(),
            folder: tempfile::tempdir().unwrap(),
            service,
        }
    }

    fn settings_text(&self) -> String {
        std::fs::read_to_string(self.user.path().join("settings.json")).unwrap()
    }

    /// 打开了文件夹的窗口。
    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        self.open_root(cx, self.folder.path().into())
    }

    /// 空窗口（没有打开文件夹）。
    fn open_empty<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        self.open_root(cx, PathBuf::new())
    }

    fn open_root<'a>(
        &self,
        cx: &'a mut TestAppContext,
        root: PathBuf,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let settings: Arc<dyn Settings> = self.service.clone();
        let user = self.user.path().to_path_buf();
        let state = self.state.path().to_path_buf();
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut view = Workbench::with_settings(
                root,
                Instant::now(),
                LayoutStore::disabled(),
                settings,
                cx,
            )
            .unwrap();
            view.set_shared_paths(
                Some(user.join("settings.json")),
                Some(user.join("keybindings.json")),
                Some(state.join("recent.json")),
            );
            // `launch` 在窗口装配好之后做的事（#28）。
            view.open_startup_editor(cx);
            view
        });
        cx.run_until_parked();
        (view, cx)
    }
}

fn run(view: &Entity<Workbench>, cx: &mut VisualTestContext, command: &'static str) {
    view.update_in(cx, |v, window, cx| v.welcome_command(command, window, cx));
    cx.run_until_parked();
}

/// 当前窗口里打开着的全部标签。
fn tabs(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<Resource> {
    view.read_with(cx, |v, _| {
        v.editor
            .groups
            .values()
            .flat_map(|pane| pane.tabs.iter().map(|t| t.resource.clone()))
            .collect()
    })
}

fn welcome_tab_open(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| {
        v.editor.groups.values().any(|pane| {
            pane.tabs.iter().any(|t| {
                crate::editors::settings::is_welcome(&t.kind)
                    && crate::editors::settings::is_welcome_resource(&t.resource)
            })
        })
    })
}

// ---------------------------------------------------------------- 欢迎页与空窗口

#[gpui::test]
fn empty_window_opens_the_welcome_page_and_an_empty_explorer(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open_empty(cx);
    assert!(welcome_tab_open(&view, cx), "空窗口显示欢迎页");
    view.read_with(cx, |v, cx| {
        // 资源管理器显示“尚未打开文件夹。”的空状态（#23 的空状态，没有行）。
        assert!(!v.explorer.read(cx).has_workspace());
        assert_eq!(v.explorer.read(cx).rows(cx).len(), 0);
        // 标签标题是登记过的「欢迎使用」。
        assert_eq!(
            v.welcome_tab_title(&Workbench::welcome_resource()).unwrap(),
            "欢迎使用"
        );
    });
}

#[gpui::test]
fn startup_editor_none_skips_the_welcome_page_but_never_in_an_empty_window(
    cx: &mut TestAppContext,
) {
    let fx = Fixture::new(r#"{ "workbench.startupEditor": "none" }"#);
    let (view, cx) = fx.open(cx);
    assert!(!welcome_tab_open(&view, cx), "取消勾选后不显示欢迎页");
    // 空窗口始终显示（UX_SPEC §6.3）。
    let (empty, cx) = fx.open_empty(cx);
    assert!(welcome_tab_open(&empty, cx));
}

#[gpui::test]
fn the_welcome_page_can_be_reopened_by_command(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{ "workbench.startupEditor": "none" }"#);
    let (view, cx) = fx.open(cx);
    assert!(!welcome_tab_open(&view, cx));
    run(&view, cx, reg::SHOW_WELCOME);
    assert!(welcome_tab_open(&view, cx));
    // 再次执行只是激活同一个标签，不会开第二个。
    run(&view, cx, reg::SHOW_WELCOME);
    assert_eq!(
        tabs(&view, cx)
            .iter()
            .filter(|r| crate::editors::settings::is_welcome_resource(r))
            .count(),
        1
    );
}

// ---------------------------------------------------------------- 最近打开

#[gpui::test]
fn recent_list_shows_full_paths_is_removable_and_survives_a_restart(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open_empty(cx);
    let other = fx.folder.path().join("sub");
    std::fs::create_dir_all(&other).unwrap();
    view.update(cx, |v, _| {
        v.note_recent(fx.folder.path(), RecentKind::Folder);
        v.note_recent(other.clone(), RecentKind::Folder);
    });
    view.read_with(cx, |v, _| {
        let rows = page::recent_rows(v.recent_entries());
        assert_eq!(rows.len(), 2);
        // 完整路径，最近的在最前。
        assert_eq!(rows[0].path, other.display().to_string());
        assert!(rows[0].is_folder);
    });
    // 重启：同一个状态文件里还在。
    let (restarted, cx) = fx.open_empty(cx);
    restarted.read_with(cx, |v, _| {
        assert_eq!(v.recent_entries().entries().len(), 2);
        assert_eq!(v.recent_entries().entries()[0].path, other);
    });
    // 从列表中移除后也落盘。
    restarted.update(cx, |v, cx| v.remove_recent(0, cx));
    let (again, cx) = fx.open_empty(cx);
    again.read_with(cx, |v, _| {
        assert_eq!(v.recent_entries().entries().len(), 1);
        assert_eq!(v.recent_entries().entries()[0].path, fx.folder.path());
    });
}

/// 空窗口 → 欢迎页 → 从「最近」打开一个文件夹。
#[gpui::test]
fn opening_a_recent_folder_from_the_welcome_page(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open_empty(cx);
    assert!(welcome_tab_open(&view, cx));
    let folder: PathBuf = fx.folder.path().into();
    view.update(cx, |v, cx| {
        v.note_recent(folder.clone(), RecentKind::Folder);
        let entry = v.recent_entries().entries()[0].clone();
        v.open_recent_entry(entry.path, entry.kind, cx);
    });
    // 一个窗口一个文件夹：请求记录下来，真正的窗口装配在 `sync_welcome` 里完成。
    view.read_with(cx, |v, _| {
        assert_eq!(v.folder_request(), Some(folder.as_path()))
    });
}

/// 「更多…」打开 `Ctrl+R` 的快速选择，里面是最近列表。
#[gpui::test]
fn open_recent_quick_pick_lists_the_recent_entries(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    let folder: PathBuf = fx.folder.path().into();
    view.update(cx, |v, _| v.note_recent(folder.clone(), RecentKind::Folder));
    run(&view, cx, reg::OPEN_RECENT);
    view.read_with(cx, |v, cx| {
        let rows = v.pick_rows(cx);
        assert_eq!(rows, vec![folder.display().to_string()]);
    });
    // `Ctrl+R` 是 VS Code 默认键位表里的键（#14 生成），不是写死的。
    cx.update(|_, cx| {
        assert_eq!(
            crate::keys::key_label(reg::OPEN_RECENT, cx).as_deref(),
            Some("Ctrl+R")
        )
    });
}

/// `window.openFilesInNewWindow` / `window.openFoldersInNewWindow` 决定复用还是开新窗口。
#[gpui::test]
fn new_window_settings_decide_whether_the_current_window_is_reused(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{ "window.openFilesInNewWindow": "on" }"#);
    let file = fx.folder.path().join("Player.cs");
    std::fs::write(
        &file,
        "class Player {}
",
    )
    .unwrap();
    let (view, cx) = fx.open(cx);
    view.read_with(cx, |v, _| {
        assert!(v.open_in_new_window(false), "文件：on 表示开新窗口");
        // 文件夹的默认值 `default` 等于开新窗口（与 VS Code 相同）。
        assert!(v.open_in_new_window(true));
    });
    // on：不在当前窗口开标签（真正的新窗口装配在 `sync_welcome` 里，测试不开窗口）。
    view.update(cx, |v, cx| v.open_path(file.clone(), cx));
    cx.run_until_parked();
    assert!(!tabs(&view, cx)
        .iter()
        .any(|r| matches!(r, Resource::File(p) if *p == file)));

    // off：复用当前窗口。
    let fx = Fixture::new(r#"{ "window.openFilesInNewWindow": "off" }"#);
    let file = fx.folder.path().join("Player.cs");
    std::fs::write(
        &file,
        "class Player {}
",
    )
    .unwrap();
    let (view, cx) = fx.open(cx);
    view.read_with(cx, |v, _| assert!(!v.open_in_new_window(false)));
    view.update(cx, |v, cx| v.open_path(file.clone(), cx));
    cx.run_until_parked();
    assert!(tabs(&view, cx).iter().any(|r| matches!(
        r,
        Resource::Workspace(p) if p.as_str().ends_with("Player.cs")
    )));
}

// ---------------------------------------------------------------- 启动参数

#[gpui::test]
fn startup_arguments_open_files_and_show_an_error_page_for_missing_paths(cx: &mut TestAppContext) {
    use crate::welcome::startup::{plan, Target};
    let fx = Fixture::new("{}");
    let file = fx.folder.path().join("Player.cs");
    std::fs::write(&file, "class Player {}\n").unwrap();
    let missing = fx.folder.path().join("Gone.cs");
    let startup = plan([
        fx.folder.path().as_os_str().to_owned(),
        file.as_os_str().to_owned(),
        missing.as_os_str().to_owned(),
    ]);
    assert_eq!(startup.folder.as_deref(), Some(fx.folder.path()));
    assert!(matches!(startup.files[0], Target::File(_)));
    assert!(matches!(startup.files[1], Target::Missing(_)));

    let (view, cx) = fx.open(cx);
    view.update(cx, |v, cx| v.open_startup_targets(&startup.files, cx));
    cx.run_until_parked();
    // 文件打开成功；不存在的路径照常有标签，内容是错误占位页（UX_SPEC §6.4）。
    view.read_with(cx, |v, _| {
        let states: Vec<_> = v
            .editor
            .groups
            .values()
            .flat_map(|p| p.tabs.iter())
            .filter(|t| !crate::editors::settings::is_welcome_resource(&t.resource))
            .map(|t| (t.resource.clone(), matches!(t.content, Content::Failed(_))))
            .collect();
        assert_eq!(states.len(), 2, "{states:?}");
        assert!(states.iter().any(|(_, failed)| !failed), "{states:?}");
        assert!(states.iter().any(|(_, failed)| *failed), "{states:?}");
    });
}

// ---------------------------------------------------------------- 首选项入口

#[gpui::test]
fn opening_shared_settings_shows_the_notice_bar_once_then_never_again(cx: &mut TestAppContext) {
    let fx = Fixture::new("{\n  // 用户的注释\n  \"editor.fontSize\": 15\n}\n");
    let (view, cx) = fx.open(cx);
    run(&view, cx, reg::OPEN_SETTINGS);
    // 打开的是与 VS Code 共用的那一份文件。
    let shared = fx.user.path().join("settings.json");
    assert!(
        tabs(&view, cx)
            .iter()
            .any(|r| matches!(r, Resource::File(p) if *p == shared)),
        "{:?}",
        tabs(&view, cx)
    );
    // 提示条只出现一次。
    let bars = view.read_with(cx, |v, _| v.shell.feedback.all_editor_bars().len());
    assert_eq!(bars, 1);
    run(&view, cx, reg::OPEN_SETTINGS);
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.feedback.all_editor_bars().len(), 1)
    });
    // 「不再提示」之后不再出现（状态记在 Rove 自己的状态目录，不写进共用 settings.json）。
    view.update(cx, |v, _| {
        v.shell
            .feedback
            .act(reg::SHARED_FILE_BAR, NEVER_SHOW_AGAIN)
            .unwrap();
        FeedbackService::dismiss(&mut v.shell.feedback, reg::SHARED_FILE_BAR).ok();
    });
    run(&view, cx, reg::OPEN_SETTINGS);
    view.read_with(cx, |v, _| {
        assert!(v.shell.feedback.all_editor_bars().is_empty());
        assert!(v
            .shell
            .feedback
            .suppressed()
            .any(|k| k.as_ref() == reg::SHARED_FILE_BAR));
    });
    assert!(
        !fx.settings_text().contains("welcome"),
        "{}",
        fx.settings_text()
    );
}

#[gpui::test]
fn opening_shared_keybindings_also_shows_the_notice(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    run(&view, cx, reg::OPEN_KEYBINDINGS);
    let shared = fx.user.path().join("keybindings.json");
    assert!(tabs(&view, cx)
        .iter()
        .any(|r| matches!(r, Resource::File(p) if *p == shared)));
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.feedback.all_editor_bars().len(), 1)
    });
}

#[gpui::test]
fn unchecking_show_on_startup_changes_only_one_key(cx: &mut TestAppContext) {
    let fx =
        Fixture::new("{\n  // 字号\n  \"editor.fontSize\": 15,\n  \"files.eol\": \"\\n\"\n}\n");
    let (view, cx) = fx.open(cx);
    assert!(view.read_with(cx, |v, _| page::show_on_startup(&*v.settings)));
    view.update(cx, |v, cx| v.set_show_on_startup(false, cx));
    cx.run_until_parked();
    fx.service.reload();
    cx.run_until_parked();
    let text = fx.settings_text();
    // 注释、已有的键与格式都没变，只多了一个键。
    assert!(text.contains("// 字号"), "{text}");
    assert!(text.contains("\"editor.fontSize\": 15"), "{text}");
    assert!(text.contains("\"files.eol\""), "{text}");
    assert!(
        text.contains("\"workbench.startupEditor\": \"none\""),
        "{text}"
    );
    assert!(view.read_with(cx, |v, _| !page::show_on_startup(&*v.settings)));
    // 重启后不再显示欢迎页。
    let (restarted, cx) = fx.open(cx);
    assert!(!welcome_tab_open(&restarted, cx));
}

// ---------------------------------------------------------------- 颜色主题

#[gpui::test]
fn theme_picker_previews_without_writing_restores_on_escape_and_writes_on_enter(
    cx: &mut TestAppContext,
) {
    let fx = Fixture::new("{\n  // 我的设置\n  \"editor.fontSize\": 15\n}\n");
    let (view, cx) = fx.open(cx);
    let original = view.read_with(cx, |v, _| v.theme_service.current().id().to_string());
    run(&view, cx, reg::SELECT_THEME);
    // 移动选中项：实时预览，但不写文件。
    view.update_in(cx, |v, window, cx| v.focus_pick("Light 2026", window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.theme_service.current().id(), "Light 2026")
    });
    assert!(!fx.settings_text().contains("colorTheme"), "预览不写文件");
    // Esc：还原。
    view.update_in(cx, |v, window, cx| v.close_pick(window, cx));
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.theme_service.current().id(), original)
    });

    // 再来一次，这次按 Enter：写入 workbench.colorTheme（保留注释与格式）。
    run(&view, cx, reg::SELECT_THEME);
    view.update_in(cx, |v, window, cx| v.focus_pick("Light 2026", window, cx));
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| v.accept_pick(window, cx));
    cx.run_until_parked();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fx.settings_text().contains("workbench.colorTheme") {
        assert!(
            Instant::now() < deadline,
            "没有写入主题：{}",
            fx.settings_text()
        );
        std::thread::sleep(Duration::from_millis(10));
        cx.run_until_parked();
    }
    let text = fx.settings_text();
    assert!(text.contains("// 我的设置"), "{text}");
    assert!(text.contains("\"editor.fontSize\": 15"), "{text}");
    assert!(
        text.contains("\"workbench.colorTheme\": \"Light 2026\""),
        "{text}"
    );
    view.read_with(cx, |v, _| {
        assert_eq!(v.theme_service.current().id(), "Light 2026")
    });
}

// ---------------------------------------------------------------- 水印与齿轮菜单

#[gpui::test]
fn watermark_follows_the_user_keybindings_file(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    let lines = |cx: &mut VisualTestContext| {
        view.read_with(cx, |v, cx| {
            let label = |k: &str| v.label(k, &[]);
            let keys = |c: &str| crate::keys::key_label(c, cx);
            page::watermark_lines(&label, &keys)
        })
    };
    let before = lines(cx);
    assert!(before.iter().any(|l| l.contains("Ctrl+P")), "{before:?}");
    // 用户在共用 keybindings.json 中覆盖了快速打开的键位。
    cx.update(|_, cx| {
        crate::keys::keys(cx)
            .bindings
            .set_user_text(r#"[{ "key": "ctrl+alt+o", "command": "workbench.action.quickOpen" }]"#)
            .unwrap()
    });
    let after = lines(cx);
    assert!(after.iter().any(|l| l.contains("Ctrl+Alt+O")), "{after:?}");
    assert_ne!(before, after);
}

/// 齿轮菜单里全部可用的项（#28 的四项 + #76 追加的「关于」）。
/// 以后再有 issue 往齿轮菜单里加项，就在这张期望表里追加一行。
#[gpui::test]
fn the_gear_menu_lists_every_usable_item(cx: &mut TestAppContext) {
    use crate::shell::menus::{visible_items, MenuId};
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    let ids: Vec<String> = view.read_with(cx, |v, cx| {
        let implemented = |id: &str| crate::keys::is_implemented(id, cx);
        visible_items(
            &v.shell.menus,
            MenuId::Manage,
            v.commands.registry(),
            &implemented,
        )
        .into_iter()
        .map(|(_, item)| item.command.to_string())
        .collect()
    });
    assert_eq!(
        ids,
        vec![
            "workbench.action.showCommands".to_string(),
            reg::OPEN_SETTINGS.to_string(),
            reg::OPEN_KEYBINDINGS.to_string(),
            reg::SELECT_THEME.to_string(),
            // #76：关于 Rove（版本、提交号、构建日期、gpui 版本、索引组件状态）。
            crate::about::registry::SHOW_ABOUT.to_string(),
        ]
    );
}
