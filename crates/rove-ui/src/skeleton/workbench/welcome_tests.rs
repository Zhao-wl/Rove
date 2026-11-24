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
    _app_data: tempfile::TempDir,
    user: PathBuf,
    state: tempfile::TempDir,
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Fixture {
    fn new(settings_json: &str) -> Self {
        Self::with_shared_files(Some(settings_json))
    }

    fn with_shared_files(settings_json: Option<&str>) -> Self {
        let app_data = tempfile::tempdir().unwrap();
        let user = app_data.path().join("Code").join("User");
        if let Some(settings_json) = settings_json {
            std::fs::create_dir_all(&user).unwrap();
            std::fs::write(user.join("settings.json"), settings_json).unwrap();
            std::fs::write(user.join("keybindings.json"), "[\n  // 用户的键位\n]\n").unwrap();
        }
        let mut options = SettingsOptions::new(&user, app_data.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        Self {
            _app_data: app_data,
            user,
            state: tempfile::tempdir().unwrap(),
            folder: tempfile::tempdir().unwrap(),
            service,
        }
    }

    fn settings_text(&self) -> String {
        std::fs::read_to_string(self.user.join("settings.json")).unwrap()
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
        let user = self.user.clone();
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

fn assert_editable_shared_file(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    path: PathBuf,
) {
    view.read_with(cx, |v, cx| {
        let tab = v.editor.active().current().unwrap();
        assert_eq!(tab.resource, Resource::File(path));
        assert!(tab.opened().is_some());
        assert!(v.code_view().read(cx).is_editable());
    });
}

#[gpui::test]
fn opening_missing_shared_files_creates_editable_json(cx: &mut TestAppContext) {
    let fx = Fixture::with_shared_files(None);
    let (view, cx) = fx.open(cx);
    assert!(!fx.user.exists());

    let settings = fx.user.join("settings.json");
    let keybindings = fx.user.join("keybindings.json");
    run(&view, cx, reg::OPEN_SETTINGS);
    assert_editable_shared_file(&view, cx, settings.clone());
    assert_eq!(std::fs::read_to_string(settings).unwrap(), "{}\n");
    assert!(!keybindings.exists());

    run(&view, cx, reg::OPEN_KEYBINDINGS);
    assert_editable_shared_file(&view, cx, keybindings.clone());
    assert_eq!(std::fs::read_to_string(keybindings).unwrap(), "[]\n");
}

#[gpui::test]
fn opening_missing_shared_files_keeps_the_command_target_and_focus(cx: &mut TestAppContext) {
    let fx = Fixture::with_shared_files(None);
    let (view, cx) = fx.open(cx);
    let settings = Resource::File(fx.user.join("settings.json"));
    let keybindings = Resource::File(fx.user.join("keybindings.json"));
    let (left, right) = view.update_in(cx, |v, window, cx| {
        let left = v.editor.focused;
        let right = v.editor.split(Direction::Right).unwrap();

        v.editor.focus(left);
        v.welcome_command(reg::OPEN_SETTINGS, window, cx);
        let tab = v.editor.active().current().unwrap();
        assert_eq!(tab.resource, settings);
        assert!(matches!(tab.content, Content::Loading));

        v.editor.focus(right);
        v.welcome_command(reg::OPEN_KEYBINDINGS, window, cx);
        let tab = v.editor.active().current().unwrap();
        assert_eq!(tab.resource, keybindings);
        assert!(matches!(tab.content, Content::Loading));

        v.editor.focus(left);
        (left, right)
    });
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert_eq!(v.editor.focused, left);
        for (group, resource) in [(left, settings), (right, keybindings.clone())] {
            let tab = v.editor.groups[&group].current().unwrap();
            assert_eq!(tab.resource, resource);
            assert!(tab.opened().is_some());
        }
        let doc = v.editor.active().current().unwrap().opened().unwrap();
        assert_eq!(v.shell.feedback.editor_bars(doc.document.id).count(), 1);
    });

    view.update(cx, |v, cx| {
        v.editor.focus(right);
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert_eq!(tab.resource, keybindings);
        let doc = tab.opened().unwrap();
        assert_eq!(v.shell.feedback.editor_bars(doc.document.id).count(), 1);
    });
}

#[gpui::test]
fn retrying_a_failed_shared_open_retries_initialization(cx: &mut TestAppContext) {
    let fx = Fixture::with_shared_files(None);
    let blocked_parent = fx.user.parent().unwrap();
    std::fs::write(blocked_parent, "a file where the Code directory should be").unwrap();
    let (view, cx) = fx.open(cx);

    run(&view, cx, reg::OPEN_SETTINGS);
    let (group, tab) = view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert!(matches!(tab.content, Content::Failed(_)));
        (v.editor.focused, tab.id)
    });

    std::fs::remove_file(blocked_parent).unwrap();
    view.update(cx, |v, cx| {
        v.editor.retry(group, tab);
        cx.notify();
    });
    cx.run_until_parked();
    assert_editable_shared_file(&view, cx, fx.user.join("settings.json"));
}

#[gpui::test]
fn opening_shared_settings_shows_the_notice_bar_once_then_never_again(cx: &mut TestAppContext) {
    let fx = Fixture::new("{\n  // 用户的注释\n  \"editor.fontSize\": 15\n}\n");
    let (view, cx) = fx.open(cx);
    run(&view, cx, reg::OPEN_SETTINGS);
    // 打开的是与 VS Code 共用的那一份文件。
    let shared = fx.user.join("settings.json");
    assert!(
        tabs(&view, cx)
            .iter()
            .any(|r| matches!(r, Resource::File(p) if *p == shared)),
        "{:?}",
        tabs(&view, cx)
    );
    assert_editable_shared_file(&view, cx, shared);
    assert_eq!(
        fx.settings_text(),
        "{\n  // 用户的注释\n  \"editor.fontSize\": 15\n}\n"
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
    let shared = fx.user.join("keybindings.json");
    assert!(tabs(&view, cx)
        .iter()
        .any(|r| matches!(r, Resource::File(p) if *p == shared)));
    assert_editable_shared_file(&view, cx, shared.clone());
    assert_eq!(
        std::fs::read_to_string(shared).unwrap(),
        "[\n  // 用户的键位\n]\n"
    );
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

/// 齿轮菜单里全部可用的项（#28 的四项 + #76 的「关于」 + #149 的「检查更新」）。
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
            // #149：检查更新（排在「关于」前面；状态栏不加常驻条目，菜单是唯一的常驻入口）。
            crate::update::registry::CHECK_FOR_UPDATES.to_string(),
            // #76：关于 Rove（版本、提交号、构建日期、gpui 版本、索引组件状态）。
            crate::about::registry::SHOW_ABOUT.to_string(),
        ]
    );
}

#[gpui::test]
fn graphical_settings_opens_via_preferences_filters_and_preserves_the_shared_file(
    cx: &mut TestAppContext,
) {
    use crate::settings_ui::{registry as settings, SharedSettings};

    let source = "{\n  // preserve this comment\n\n  \"editor.fontSize\": 15,\n  \"editor.wordWrap\": \"off\",\n  \"editor.minimap.enabled\": false\n}\n";
    let fx = Fixture::new(source);
    let (workbench, cx) = fx.open(cx);
    let service = fx.service.clone();
    workbench.update_in(cx, |view, window, cx| {
        cx.set_global(SharedSettings(service));
        window.focus(&view.code_view().focus_handle(cx));
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-,");
    cx.run_until_parked();
    assert!(tabs(&workbench, cx).contains(&Resource::Virtual(settings::RESOURCE.into())));
    workbench.read_with(cx, |view, _| {
        let current = view
            .editor
            .active()
            .current()
            .expect("active settings editor");
        assert_eq!(
            current.resource,
            Resource::Virtual(settings::RESOURCE.into())
        );
        assert!(
            current.opened().is_some(),
            "the graphical settings document must finish opening"
        );
    });
    assert_eq!(fx.settings_text(), source);
    let editor = workbench.read_with(cx, |view, _| view.settings_view.clone().unwrap());
    workbench.update_in(cx, |view, _, _| {
        view.editor
            .split(crate::editors::Direction::Right)
            .expect("open another editor group");
        assert!(view.editor.active().current().is_none());
    });
    run(&workbench, cx, reg::OPEN_SETTINGS);
    assert_eq!(
        tabs(&workbench, cx)
            .iter()
            .filter(|tab| crate::editors::settings::is_settings_resource(tab))
            .count(),
        1
    );
    workbench.update_in(cx, |view, window, cx| {
        assert!(crate::editors::settings::is_settings_resource(
            &view
                .editor
                .active()
                .current()
                .expect("settings tab")
                .resource
        ));
        assert!(editor.read(cx).search_focus(cx).is_focused(window));
    });
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("font size @modified");
    cx.simulate_keystrokes("enter");
    editor.update_in(cx, |view, window, cx| {
        assert!(!view.search_focus(cx).is_focused(window));
    });
    cx.simulate_keystrokes("up");
    editor.update_in(cx, |view, window, cx| {
        assert!(view.search_focus(cx).is_focused(window));
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    editor.update(cx, |view, cx| {
        assert!(view.select_result_for_test("editor.wordWrap", cx));
    });
    workbench.update_in(cx, |view, window, cx| {
        view.settings_command(settings::PREVIOUS_SEARCH, window, cx);
    });
    cx.run_until_parked();
    editor.update(cx, |view, cx| {
        assert!(!view.select_result_for_test("editor.wordWrap", cx));
        assert!(view.select_result_for_test("editor.fontSize", cx));
    });
    workbench.update_in(cx, |view, window, cx| {
        view.settings_command(settings::FOCUS_CONTROL, window, cx)
    });
    editor.update_in(cx, |view, window, cx| {
        view.edit_text_for_test("101", window, cx)
    });
    assert_eq!(fx.settings_text(), source);
    editor.update_in(cx, |view, window, cx| {
        view.edit_text_for_test("16", window, cx)
    });
    let wait_for_text = |cx: &mut VisualTestContext, expected: &str| {
        for _ in 0..200 {
            cx.run_until_parked();
            if fx.settings_text() == expected {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fx.settings_text(), expected);
    };
    let after_number = source.replace(": 15,", ": 16,");
    wait_for_text(cx, &after_number);
    assert_eq!(
        fx.service
            .read("editor.fontSize", &Scope::default())
            .unwrap(),
        Value::Number(16.0)
    );

    editor.update(cx, |view, cx| {
        view.set_query_for_test("editor.wordWrap", cx);
        assert!(view.select_result_for_test("editor.wordWrap", cx));
    });
    workbench.update_in(cx, |view, window, cx| {
        view.settings_command(settings::FOCUS_CONTROL, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("down enter");
    let after_enum = after_number.replace("\"off\"", "\"on\"");
    wait_for_text(cx, &after_enum);

    editor.update(cx, |view, cx| {
        view.set_query_for_test("editor.minimap.enabled", cx);
        assert!(view.select_result_for_test("editor.minimap.enabled", cx));
    });
    workbench.update_in(cx, |view, window, cx| {
        view.settings_command(settings::FOCUS_CONTROL, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("space");
    let after_bool = after_enum.replace(": false", ": true");
    wait_for_text(cx, &after_bool);

    // Queue two edits before the background executor has polled either write.
    editor.update(cx, |view, cx| {
        view.write_value("editor.fontSize".into(), Value::Number(17.0), cx);
        view.write_value("editor.fontSize".into(), Value::Number(18.0), cx);
    });
    let first_queued = after_bool.replace(": 16,", ": 17,");
    let last_queued = after_bool.replace(": 16,", ": 18,");
    wait_for_text(cx, &last_queued);

    editor.update(cx, |view, cx| {
        view.toggle_checkbox("editor.minimap.enabled".into(), cx);
        view.toggle_checkbox("editor.minimap.enabled".into(), cx);
        view.write_value("editor.fontSize".into(), Value::Number(19.0), cx);
    });
    let between_toggles = last_queued.replace(": true", ": false");
    let after_toggles = last_queued.replace(": 18,", ": 19,");
    wait_for_text(cx, &after_toggles);

    let backups: Vec<_> = std::fs::read_dir(fx._app_data.path().join("backup"))
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    for prior in [
        source,
        after_number.as_str(),
        after_enum.as_str(),
        after_bool.as_str(),
        first_queued.as_str(),
        last_queued.as_str(),
        between_toggles.as_str(),
    ] {
        assert!(backups.iter().any(|backup| backup == prior));
    }

    workbench.update_in(cx, |view, window, cx| {
        view.settings_command(settings::OPEN_JSON, window, cx)
    });
    cx.run_until_parked();
    assert_editable_shared_file(&workbench, cx, fx.user.join("settings.json"));
}
