//! #20 入口集成测试（工作台一侧）：只读的表现、折叠随标签切换恢复、主题切换后重新着色。
//!
//! 运行：`cargo test -p rove-ui readonly_` 与 `cargo test -p rove-ui folds_follow_tabs`。
use super::*;
use crate::documents::OpenOptions;
use gpui::{TestAppContext, VisualTestContext};
use rove_config::{SettingsOptions, SettingsService};

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

    fn file(&self, path: &str, text: &str) {
        let full = self.folder.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
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
}

fn open_file(view: &Entity<Workbench>, cx: &mut VisualTestContext, path: &str) {
    view.update(cx, |v, _| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
    });
    settle(cx);
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.update(|window, cx| {
            window.refresh();
            cx.refresh_windows();
        });
        cx.run_until_parked();
    }
}

fn focus_code(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<CodeView> {
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    code
}

/// Dispatch a command's GPUI action to the focused element (as the keybinding service does).
fn command(cx: &mut VisualTestContext, id: &str) {
    let action = code_view::commands::action_for(id).unwrap();
    cx.update(|window, cx| window.dispatch_action(action, cx));
    cx.run_until_parked();
}

fn locked(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, _| {
        v.editor
            .active()
            .current()
            .is_some_and(|tab| v.tab_read_only(tab))
    })
}

fn cursor_hint(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| {
        let hint = v.shell.feedback.cursor_hint()?;
        let args: Vec<(String, String)> = hint
            .message
            .args
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let args: Vec<(&str, String)> = args.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        Some(v.label(&hint.message.key, &args))
    })
}

#[gpui::test]
fn readonly_package_cache_shows_lock_hint_and_toggles(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    fx.file(
        "Library/PackageCache/com.unity.foo/Runtime/Foo.cs",
        "class Foo {}\n",
    );
    fx.file("Assets/Bar.cs", "class Bar {}\n");
    let (view, cx) = fx.open(cx);

    // 普通文件：加载器默认的 `Session` 原因不显示锁，输入也不提示。
    open_file(&view, cx, "Assets/Bar.cs");
    let code = focus_code(&view, cx);
    assert!(!locked(&view, cx));
    code.read_with(cx, |c, _| assert!(!c.is_read_only()));
    cx.simulate_keystrokes("x");
    assert_eq!(cursor_hint(&view, cx), None);

    // 包缓存目录下的文件：打开即只读，标签显示锁，原因来自 rove.files.extraReadonlyInclude。
    open_file(
        &view,
        cx,
        "Library/PackageCache/com.unity.foo/Runtime/Foo.cs",
    );
    let code = focus_code(&view, cx);
    assert!(locked(&view, cx));
    let reasons = view.read_with(cx, |v, cx| v.active_read_only(cx));
    assert_eq!(
        reasons,
        vec![ReadOnlyReason::Rule(Arc::from(
            rove_workspace::REASON_EXTRA_READONLY_INCLUDE
        ))]
    );
    // 尝试输入：光标处提示，带原因，并说明可以怎么做。
    code.update(cx, |c, cx| {
        c.set_selection(Selection::caret(code_view::selection::pos(0, 6)), None, cx)
    });
    cx.simulate_keystrokes("x");
    let hint = cursor_hint(&view, cx).expect("cursor hint");
    assert!(hint.starts_with("无法在只读编辑器中编辑。"), "{hint}");
    assert!(hint.contains("Unity 包缓存目录，修改会被覆盖。"), "{hint}");
    assert!(hint.contains("在会话中切换只读模式"), "{hint}");
    view.read_with(cx, |v, _| {
        let h = v.shell.feedback.cursor_hint().unwrap();
        assert_eq!(h.position, code_view::selection::pos(0, 6));
    });

    // “在会话中切换只读模式”：取消只读，锁消失；再切换回来。
    command(
        cx,
        "workbench.action.files.toggleActiveEditorReadonlyInSession",
    );
    settle(cx);
    assert!(!locked(&view, cx));
    code.read_with(cx, |c, _| assert!(!c.is_read_only()));
    command(
        cx,
        "workbench.action.files.toggleActiveEditorReadonlyInSession",
    );
    settle(cx);
    assert!(locked(&view, cx));
}

#[gpui::test]
fn readonly_toggle_does_not_apply_to_huge_files(cx: &mut TestAppContext) {
    // 超大文件阈值调小到 2 KB。
    let fx = Fixture::new(r#"{ "rove.largeFile.threshold": 2048 }"#);
    fx.file("Logs/Editor.log", &"line of log text\n".repeat(400));
    fx.file("Assets/A.cs", "class A {}\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Logs/Editor.log");
    let code = focus_code(&view, cx);
    assert!(locked(&view, cx));
    code.read_with(cx, |c, _| {
        assert_eq!(c.read_only(), &[ReadOnlyReason::HugeFile]);
        assert!(!c.folding_enabled(), "超大文件关闭全部折叠");
        assert!(c
            .disabled_features()
            .contains(&code_view::Feature::SyntaxHighlighting));
    });
    command(
        cx,
        "workbench.action.files.toggleActiveEditorReadonlyInSession",
    );
    settle(cx);
    assert!(locked(&view, cx), "超大文件不能临时取消只读");
    cx.simulate_keystrokes("x");
    let hint = cursor_hint(&view, cx).expect("cursor hint");
    assert!(hint.contains("超大文件"), "{hint}");
    assert!(!hint.contains("切换只读"), "{hint}");

    // 普通文件可以在会话中设为只读（原因为 Session）。
    open_file(&view, cx, "Assets/A.cs");
    focus_code(&view, cx);
    assert!(!locked(&view, cx));
    command(
        cx,
        "workbench.action.files.toggleActiveEditorReadonlyInSession",
    );
    settle(cx);
    assert!(locked(&view, cx));
    let reasons = view.read_with(cx, |v, cx| v.active_read_only(cx));
    assert_eq!(reasons, vec![ReadOnlyReason::Session]);
    let _ = &fx.user;
}

#[gpui::test]
fn folds_follow_tabs(cx: &mut TestAppContext) {
    // 两个标签都保留（不用预览标签替换）。
    let fx = Fixture::new(r#"{ "workbench.editor.enablePreview": false }"#);
    let body: String = (0..40)
        .map(|i| format!("    void M{i}()\n    {{\n        Call();\n    }}\n"))
        .collect();
    fx.file("Assets/A.cs", &format!("class A\n{{\n{body}}}\n"));
    fx.file("Assets/B.cs", "class B {}\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Assets/A.cs");
    let code = focus_code(&view, cx);
    code.read_with(cx, |c, _| assert!(c.folding_ready()));
    // 折叠 M0 与 M1 的方法体（`{` 行，缩进区间）。
    for line in [3u64, 7] {
        code.update(cx, |c, cx| {
            c.set_selection(
                Selection::caret(code_view::selection::pos(line, 4)),
                None,
                cx,
            )
        });
        command(cx, "editor.fold");
    }
    settle(cx);
    let folds = code.read_with(cx, |c, _| c.folds());
    assert_eq!(folds.len(), 2, "{folds:?}");
    // 标签的 ViewState 已同步。
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert_eq!(tab.view.folds.to_vec(), folds);
    });
    // 切到另一个标签再切回来：折叠恢复。
    open_file(&view, cx, "Assets/B.cs");
    code.read_with(cx, |c, _| assert!(c.folds().is_empty()));
    open_file(&view, cx, "Assets/A.cs");
    settle(cx);
    view.read_with(cx, |v, _| assert_eq!(v.editor.active().tabs.len(), 2));
    assert_eq!(code.read_with(cx, |c, _| c.folds()), folds);
    code.read_with(cx, |c, _| {
        assert!(c.fold_map().is_hidden(4) && c.fold_map().is_hidden(8));
        assert!(!c.fold_map().is_hidden(12));
    });
}

#[gpui::test]
fn theme_switch_recolours_tokens(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{ "workbench.colorTheme": "Dark Modern" }"#);
    fx.file("Assets/A.cs", "public class A {}\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Assets/A.cs");
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    let keyword = |code: &Entity<CodeView>, cx: &mut VisualTestContext| {
        code.read_with(cx, |c, _| {
            let m = c.merged_decorations(0).expect("decorated");
            m.runs.iter().find(|r| r.range == (0..6)).unwrap().style
        })
    };
    let dark = keyword(&code, cx);
    fx.change(r#"{ "workbench.colorTheme": "Light Modern" }"#, cx);
    for _ in 0..20 {
        settle(cx);
        if keyword(&code, cx) != dark {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let light = keyword(&code, cx);
    assert_ne!(
        light.foreground, dark.foreground,
        "主题切换后 token 颜色立即更新"
    );
    // 整形缓存不随主题切换增长。
    let entries = code.read_with(cx, |c, _| c.shape_cache_stats().entries);
    assert!(entries <= 8, "{entries}");
}
