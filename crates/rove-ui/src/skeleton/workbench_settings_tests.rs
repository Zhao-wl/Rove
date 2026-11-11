//! #104 入口集成测试：共享 `settings.json` 改动后，不重启窗口即可生效。
//!
//! 用真实的 `rove_config::SettingsService`（临时用户目录、关闭轮询），改文件后调用
//! `reload()` 同步发布变更；文件监听线程本身由 rove-config 的测试覆盖。
use super::*;
use crate::shell::layout::SideBarLocation;
use gpui::{TestAppContext, VisualTestContext};
use rove_config::{SettingsOptions, SettingsService};
use rove_theme::ThemeKind;

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
        Self {
            user,
            folder: tempfile::tempdir().unwrap(),
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

    /// 模拟 VS Code 改了共享文件。
    fn change(&self, settings_json: &str, cx: &mut VisualTestContext) {
        std::fs::write(self.user.path().join("settings.json"), settings_json).unwrap();
        self.service.reload();
        cx.run_until_parked();
    }
}

#[gpui::test]
fn editor_font_size_reloads_into_code_views(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{ "editor.fontSize": 15 }"#);
    let (view, cx) = fx.open(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| assert_eq!(c.options().font_size, 15.0));
    fx.change(r#"{ "editor.fontSize": 21 }"#, cx);
    code.read_with(cx, |c, _| assert_eq!(c.options().font_size, 21.0));
}

#[gpui::test]
fn color_theme_reloads_workbench_and_code_views(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    let dark = code.read_with(cx, |c, _| c.palette().clone());
    view.read_with(cx, |v, _| {
        assert!(matches!(v.shell.theme_kind(), ThemeKind::Dark))
    });
    fx.change(r#"{ "workbench.colorTheme": "Light+" }"#, cx);
    // 主题在后台线程重新加载，完成后经 UI 线程刷新。
    let deadline = Instant::now() + Duration::from_secs(10);
    while !view.read_with(cx, |v, _| matches!(v.shell.theme_kind(), ThemeKind::Light)) {
        assert!(Instant::now() < deadline, "theme did not reload");
        std::thread::sleep(Duration::from_millis(10));
        cx.run_until_parked();
    }
    view.read_with(cx, |v, _| {
        assert_eq!(v.theme_service.current().id(), "Light+")
    });
    let light = code.read_with(cx, |c, _| c.palette().clone());
    assert_ne!(light.background, dark.background, "代码视图换了配色");
}

#[gpui::test]
fn files_exclude_rescans_the_workspace(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    std::fs::write(fx.folder.path().join("a.cs"), "class A {}").unwrap();
    std::fs::create_dir(fx.folder.path().join("skip")).unwrap();
    std::fs::write(fx.folder.path().join("skip/b.cs"), "class B {}").unwrap();
    let (view, cx) = fx.open(cx);
    view.read_with(cx, |v, _| assert_eq!(v.tree.as_ref().unwrap().count(), 2));
    let scans = view.read_with(cx, |v, _| v.scans);
    fx.change(r#"{ "files.exclude": { "**/skip": true } }"#, cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.scans, scans + 1);
        assert_eq!(v.tree.as_ref().unwrap().count(), 1);
    });
    // 与工作区无关的设置不重新扫描。
    fx.change(
        r#"{ "files.exclude": { "**/skip": true }, "editor.fontSize": 18 }"#,
        cx,
    );
    view.read_with(cx, |v, _| assert_eq!(v.scans, scans + 1));
}

#[gpui::test]
fn workbench_settings_apply_without_restart(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.sidebar_location(), SideBarLocation::Left)
    });
    fx.change(r#"{ "workbench.sideBar.location": "right" }"#, cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.shell.sidebar_location(), SideBarLocation::Right)
    });
    // 编辑器组的 workbench.editor.* 也重新读取。
    view.read_with(cx, |v, _| assert!(!v.editor.settings.close_on_delete));
    fx.change(
        r#"{ "workbench.sideBar.location": "right", "workbench.editor.closeOnFileDelete": true }"#,
        cx,
    );
    view.read_with(cx, |v, _| assert!(v.editor.settings.close_on_delete));
}
