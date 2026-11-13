//! #29 入口集成测试（真实 GPUI 测试窗口 + 测试后端；GPUI 测试窗口没有 HWND，真实 WebView2 由
//! `examples/markdown_probe.rs` 验收）：打开 `.md` → 预览模式（实例按需创建、渲染请求带 data-line）→
//! `ctrl+shift+v` 切到源码 → 再切回预览；`ctrl+k v` 并排；标题栏按钮；用户的 `workbench.editorAssociations`；
//! 搜索结果打开源码；没有运行时；远程资源始终拦截；主题切换不重建；弹出层隐藏；空闲回收；双击定位；
//! 双向滚动同步。
use super::*;
use crate::editors::settings::is_markdown_preview;
use gpui::{TestAppContext, VisualTestContext};
use rove_config::{SettingsOptions, SettingsService};

const DOC: &str = "# 标题\n\n第一段。\n\n![远程](https://example.com/a.png)\n\n```mermaid\ngraph TD; A-->B\n```\n\n$E=mc^2$\n";

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
        std::fs::write(folder.path().join("README.md"), DOC).unwrap();
        let long: String = (0..2400).map(|i| format!("段落 {i}\n\n")).collect();
        std::fs::write(folder.path().join("Long.md"), long).unwrap();
        std::fs::write(folder.path().join("Other.md"), "# Other\n").unwrap();
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
            let editor = EditorSettings::read(&*settings).unwrap();
            Workbench::build(
                root,
                Instant::now(),
                LayoutStore::disabled(),
                settings,
                editor,
                cx,
            )
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

fn open(view: &Entity<Workbench>, path: &str, options: OpenOptions, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                options,
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    settle(cx);
}

fn explorer() -> OpenOptions {
    OpenOptions {
        source: OpenSource::Explorer,
        ..Default::default()
    }
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            cx.refresh_windows();
        });
    }
    cx.run_until_parked();
}

fn active_kind(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> (String, bool) {
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        let path = match &tab.resource {
            Resource::Workspace(p) => p.as_str().to_string(),
            other => format!("{other:?}"),
        };
        (path, is_markdown_preview(&tab.kind))
    })
}

fn renders(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<serde_json::Value> {
    view.read_with(cx, |v, _| {
        v.markdown
            .fake
            .borrow()
            .posted
            .iter()
            .filter(|m| m["type"] == "render")
            .cloned()
            .collect()
    })
}

fn focus_editor(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let v = view.read(cx);
        v.focus_group_code(v.editor.focused, window, cx);
    });
    settle(cx);
}

/// 入口：打开 `.md` → 预览（第一次打开才创建实例）→ `ctrl+shift+v` 源码 → 再切回预览。
#[gpui::test]
fn opening_markdown_shows_preview_and_toggles_to_source(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    // 冷启动后、第一次打开预览之前，进程里没有实例。
    view.read_with(cx, |v, _| {
        assert_eq!(v.markdown.fake.borrow().created, 0);
        assert_eq!(v.markdown_snapshot().lifecycle, Some("absent"));
    });
    open(&view, "README.md", explorer(), cx);
    assert_eq!(active_kind(&view, cx), ("README.md".into(), true));
    let snapshot = view.read_with(cx, |v, _| v.markdown_snapshot());
    assert_eq!(snapshot.lifecycle, Some("alive"));
    assert!(snapshot.ready && snapshot.rendered, "{snapshot:?}");
    assert!(snapshot.visible);
    view.read_with(cx, |v, _| {
        let log = v.markdown.fake.borrow();
        assert_eq!(log.created, 1);
        // 主题先于正文推送。
        assert_eq!(log.posted[0]["type"], "theme");
        assert!(log.layout.flatten().is_some(), "实例放在预览区域上");
    });
    let render = renders(&view, cx).pop().unwrap();
    let html = render["html"].as_str().unwrap();
    assert!(html.contains("data-line=\"0\""));
    assert!(html.contains("class=\"code-line mermaid\""));
    assert!(html.contains("rove-math"));
    // 远程图片被阻止，提示条显示数量。
    assert_eq!(snapshot.remote, 1);
    view.read_with(cx, |v, _| {
        let bars = v.shell.feedback.all_editor_bars();
        assert_eq!(bars.len(), 1);
        assert_eq!(
            v.label(&bars[0].message.key, &[("count", "1".into())]),
            "已阻止 1 个远程资源。"
        );
    });
    // 预览获得焦点（键位上下文 activeCustomEditorId），Ctrl+Shift+V 切到源码。
    cx.update(|window, cx| {
        let focus = view.read(cx).markdown.focus.clone().unwrap();
        window.focus(&focus);
    });
    settle(cx);
    cx.simulate_keystrokes("ctrl-shift-v");
    settle(cx);
    assert_eq!(active_kind(&view, cx), ("README.md".into(), false));
    view.read_with(cx, |v, _| assert!(!v.markdown_snapshot().visible));
    // 源码里再按一次切回预览（when：editorFocus && resourceLangId == markdown），实例复用。
    focus_editor(&view, cx);
    cx.simulate_keystrokes("ctrl-shift-v");
    settle(cx);
    assert_eq!(active_kind(&view, cx), ("README.md".into(), true));
    view.read_with(cx, |v, _| assert_eq!(v.markdown.fake.borrow().created, 1));
}

/// `ctrl+k v`：源码旁边并排打开预览；标题栏按钮切换。
#[gpui::test]
fn preview_to_side_and_title_button(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{ "workbench.editorAssociations": { "*.md": "default" } }"#);
    let (view, cx) = fx.open(cx);
    // 用户设置了 editorAssociations：以用户设置为准，双击文件树打开源码。
    open(&view, "README.md", explorer(), cx);
    assert_eq!(active_kind(&view, cx), ("README.md".into(), false));
    view.read_with(cx, |v, _| assert_eq!(v.markdown.fake.borrow().created, 0));
    focus_editor(&view, cx);
    cx.simulate_keystrokes("ctrl-k v");
    settle(cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.editor.groups.len(), 2, "并排打开");
        let previews = v
            .editor
            .groups
            .values()
            .filter(|g| g.current().is_some_and(|t| is_markdown_preview(&t.kind)))
            .count();
        assert_eq!(previews, 1);
        assert_eq!(v.markdown.fake.borrow().created, 1);
    });
    // 标题栏按钮：源码组显示“显示预览”，预览组显示“显示源码”。
    view.read_with(cx, |v, _| {
        let entries: Vec<_> = v
            .shell
            .contributions
            .entries()
            .filter(|(_, c)| matches!(c.placement, crate::contributions::Placement::EditorTitle))
            .map(|(id, _)| id.to_string())
            .collect();
        assert_eq!(entries.len(), 2);
    });
    let preview_group = view.read_with(cx, |v, _| {
        *v.editor
            .groups
            .iter()
            .find(|(_, g)| g.current().is_some_and(|t| is_markdown_preview(&t.kind)))
            .unwrap()
            .0
    });
    view.update(cx, |v, cx| {
        v.editor.focus(preview_group);
        v.command(crate::markdown::registry::TOGGLE_PREVIEW, cx);
    });
    settle(cx);
    assert_eq!(active_kind(&view, cx), ("README.md".into(), false));
}

/// 搜索结果、跳转（带位置）打开 `.md` 时是源码并定位到匹配行。
#[gpui::test]
fn located_opens_show_source(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open(
        &view,
        "README.md",
        OpenOptions {
            source: OpenSource::Search,
            location: Some(rove_base::Position {
                line: Line(2),
                column: rove_base::Utf8Column(0),
            }),
            ..Default::default()
        },
        cx,
    );
    assert_eq!(active_kind(&view, cx), ("README.md".into(), false));
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.editor.active().current().unwrap().view.cursor.line,
            Line(2)
        );
        assert_eq!(v.markdown.fake.borrow().created, 0);
    });
}

/// 没有 WebView2 运行时：`.md` 改为源码，只通知一次，带官方下载页按钮，没有任何下载。
#[gpui::test]
fn missing_runtime_falls_back_to_source_once(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    view.update(cx, |v, _| {
        let sender = v.markdown.sender_for_test();
        let log = v.markdown.fake.clone();
        v.markdown
            .replace_backend(Box::new(crate::markdown::backend::FakeBackend::new(
                sender, log, true,
            )));
    });
    open(&view, "README.md", explorer(), cx);
    assert_eq!(active_kind(&view, cx), ("README.md".into(), false));
    open(&view, "Other.md", explorer(), cx);
    assert_eq!(active_kind(&view, cx), ("Other.md".into(), false));
    view.read_with(cx, |v, _| {
        let notes = v.shell.feedback.notifications();
        assert_eq!(notes.len(), 1);
        assert_eq!(
            notes[0].key.as_ref(),
            crate::markdown::registry::RUNTIME_MISSING
        );
        assert_eq!(notes[0].count, 1, "只弹一次");
        assert_eq!(
            notes[0].actions[0].id.as_ref(),
            crate::markdown::registry::ACTION_DOWNLOAD_PAGE
        );
        assert!(v.markdown_snapshot().runtime_missing);
    });
}

/// 旧版设置为 true 时，资源仍只来自本地，提示条没有放行动作。
#[gpui::test]
fn legacy_remote_setting_keeps_preview_offline(cx: &mut TestAppContext) {
    let fx = Fixture::new(
        r#"{ "rove.markdown.allowRemoteResources": true,
             "markdown.styles": ["https://example.com/remote.css", "local.css"] }"#,
    );
    std::fs::write(
        fx.folder.path().join("README.md"),
        format!(
            "{DOC}\n![本地](local.png)\n\n<video src=\"https://example.com/video.mp4\"></video>\n"
        ),
    )
    .unwrap();
    let (view, cx) = fx.open(cx);
    open(&view, "README.md", explorer(), cx);
    let all = renders(&view, cx);
    let render = all.last().unwrap();
    let html = render["html"].as_str().unwrap();
    assert!(!html.contains("example.com"));
    assert!(html.contains("rove-file.local"));
    assert!(html.contains("blocked-image.svg"));
    assert!(html.contains("mermaid"));
    assert!(html.contains("rove-math"));
    let styles = render["styles"].as_array().unwrap();
    assert_eq!(styles.len(), 1);
    assert!(styles[0].as_str().unwrap().ends_with("/local.css"));
    view.read_with(cx, |v, _| {
        let bars = v.shell.feedback.all_editor_bars();
        assert_eq!(bars.len(), 1);
        assert!(bars[0].actions.is_empty());
    });
}

/// 主题切换只推变量；弹出层打开时隐藏实例。
#[gpui::test]
fn theme_and_overlay_keep_preview_instance(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open(&view, "README.md", explorer(), cx);
    // 主题切换：推 theme 消息，不重建。
    fx.change(r#"{ "workbench.colorTheme": "Light 2026" }"#, cx);
    settle(cx);
    view.read_with(cx, |v, _| {
        let log = v.markdown.fake.borrow();
        assert_eq!(log.created, 1);
        let last_theme = log
            .posted
            .iter()
            .rev()
            .find(|m| m["type"] == "theme")
            .unwrap();
        assert_eq!(last_theme["kind"], "vscode-light");
    });
    // 命令面板打开：实例隐藏；关闭后恢复。
    cx.update(|window, cx| {
        let focus = view.read(cx).markdown.focus.clone().unwrap();
        window.focus(&focus);
    });
    settle(cx);
    cx.simulate_keystrokes("ctrl-shift-p");
    settle(cx);
    view.read_with(cx, |v, _| {
        assert!(!v.markdown_snapshot().visible);
        assert_eq!(v.markdown.fake.borrow().layout, Some(None));
    });
    cx.simulate_keystrokes("escape");
    settle(cx);
    view.read_with(cx, |v, _| {
        assert!(v.markdown_snapshot().visible);
        assert!(v.markdown.fake.borrow().layout.flatten().is_some());
    });
}

/// 所有预览关闭后经过 `rove.markdown.webviewIdleTimeout` 销毁实例；超时前重新打开则复用。
#[gpui::test]
fn idle_timeout_disposes_the_instance(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{ "rove.markdown.webviewIdleTimeout": 5 }"#);
    let (view, cx) = fx.open(cx);
    open(&view, "README.md", explorer(), cx);
    view.update(cx, |v, cx| {
        v.editor.close_tabs(None, false);
        cx.notify();
    });
    settle(cx);
    view.read_with(cx, |v, _| {
        assert!(v.markdown_snapshot().idle_pending);
        assert!(v.markdown.fake.borrow().suspended >= 1);
        assert_eq!(v.markdown.fake.borrow().disposed, 0);
    });
    cx.executor().advance_clock(Duration::from_secs(3));
    // 超时前重新打开：复用，不重建。
    open(&view, "Other.md", explorer(), cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.markdown.fake.borrow().created, 1);
        assert!(!v.markdown_snapshot().idle_pending);
    });
    view.update(cx, |v, cx| {
        v.editor.close_tabs(None, false);
        cx.notify();
    });
    settle(cx);
    cx.executor().advance_clock(Duration::from_secs(6));
    settle(cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.markdown.fake.borrow().disposed, 1);
        assert_eq!(v.markdown_snapshot().lifecycle, Some("absent"));
    });
    // 回收后再打开：重新创建。
    open(&view, "README.md", explorer(), cx);
    view.read_with(cx, |v, _| assert_eq!(v.markdown.fake.borrow().created, 2));
}

/// 双击预览定位到源码行；并排时双向滚动同步（≥ 2000 行的文档）。
#[gpui::test]
fn double_click_and_scroll_sync(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let (view, cx) = fx.open(cx);
    open(
        &view,
        "Long.md",
        OpenOptions {
            presentation: crate::documents::Presentation::Source,
            ..explorer()
        },
        cx,
    );
    focus_editor(&view, cx);
    cx.simulate_keystrokes("ctrl-k v");
    settle(cx);
    let document = view.read_with(cx, |v, _| v.markdown.requested.unwrap().document);
    // 预览滚到第 3000 行 → 源码视图跟随。
    view.update(cx, |v, cx| {
        let sender = v.markdown.sender_for_test();
        let _ = sender.try_send(crate::markdown::backend::BackendEvent::Message(
            serde_json::json!({
                "version": 1, "type": "scroll", "documentId": document.0, "line": 3000.0,
            }),
        ));
        cx.notify();
    });
    settle(cx);
    let source_group = view.read_with(cx, |v, _| {
        *v.editor
            .groups
            .iter()
            .find(|(_, g)| g.current().is_some_and(|t| !is_markdown_preview(&t.kind)))
            .unwrap()
            .0
    });
    let code = view.read_with(cx, |v, _| {
        v.group_code(source_group, false).unwrap().clone()
    });
    code.read_with(cx, |c, _| {
        assert!(
            (c.scroll_position().top - 3000.0).abs() < 1.0,
            "{:?}",
            c.scroll_position()
        );
    });
    // 源码滚动 → 预览收到 scrollTo（预览自己引起的那次视口变化不回推）。
    let before = view.read_with(cx, |v, _| {
        v.markdown
            .fake
            .borrow()
            .posted
            .iter()
            .filter(|m| m["type"] == "scrollTo")
            .count()
    });
    code.update(cx, |c, cx| {
        c.set_scroll_position(
            crate::code_view::ScrollPosition {
                top: 4200.0,
                x: 0.0,
            },
            false,
            cx,
        )
    });
    settle(cx);
    view.read_with(cx, |v, _| {
        let log = v.markdown.fake.borrow();
        let scrolls: Vec<_> = log
            .posted
            .iter()
            .filter(|m| m["type"] == "scrollTo")
            .collect();
        assert!(scrolls.len() > before);
        assert!((scrolls.last().unwrap()["line"].as_f64().unwrap() - 4200.0).abs() < 1.0);
    });
    // 双击预览第 1234 行 → 预览标签就地换成源码并定位。
    let preview_group = view.read_with(cx, |v, _| v.markdown.owner.unwrap().0);
    view.update(cx, |v, cx| {
        v.editor.focus(preview_group);
        let sender = v.markdown.sender_for_test();
        let _ = sender.try_send(crate::markdown::backend::BackendEvent::Message(
            serde_json::json!({
                "version": 1, "type": "dblclick", "documentId": document.0, "line": 1234.0,
            }),
        ));
        cx.notify();
    });
    settle(cx);
    view.read_with(cx, |v, _| {
        let tab = v.editor.groups[&preview_group].current().unwrap();
        assert!(!is_markdown_preview(&tab.kind));
        assert_eq!(tab.view.cursor.line, Line(1234));
    });
}
