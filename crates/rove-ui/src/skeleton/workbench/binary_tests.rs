//! #209 入口集成测试：二进制文件（或使用了不支持的文本编码的文件）的错误页与「仍然打开」。
//!
//! 覆盖 UX_SPEC §3.5 要求的整条链路：**拒绝 → 错误页提供「仍然打开」→ 按 lossy 文本显示**，
//! 并核对「仍然打开」之后状态栏的编码条目与只读表现和 #27 的「通过编码重新打开」一致。
//! 另有一条跑在真实 Unity 项目的 `.png` 上（`ROVE_TEST_UNITY_MEDIUM` 未设置时跳过，
//! 所以 CI 上 `cargo test --workspace` 不受影响）。
//!
//! 运行：`cargo test -p rove-ui binary_ -j 1`。
use super::editor_panes::{can_open_anyway, error_page_key};
use super::*;
use crate::{documents::OpenOptions, status_items::registry as ids};
use gpui::{TestAppContext, VisualTestContext};
use rove_config::{SettingsOptions, SettingsService};
use rove_test_support::UnityProject;

/// 合成的二进制文件：第一行是纯 ASCII（断言好写），后面带 NUL 字节，所以普通打开会被
/// `rove-buffer` 按「二进制」拒绝，而「仍然打开」之后字节原样可见。
const BINARY: &[u8] = b"PNG-BINARY-FIXTURE\r\n\x00\x01\x02hello\x00world\n";
const FIRST_LINE: &str = "PNG-BINARY-FIXTURE";

struct Fixture {
    _user: tempfile::TempDir,
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
}

impl Fixture {
    fn new() -> Self {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("settings.json"), "{}").unwrap();
        let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
        options.watch = None;
        let service = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        Self {
            _user: user,
            folder: tempfile::tempdir().unwrap(),
            service,
        }
    }

    fn file(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let full = self.folder.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, bytes).unwrap();
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
        settle(cx);
        cx.update(|window, cx| window.focus(&view.read(cx).focus));
        (view, cx)
    }
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

/// 和资源管理器 / 快速打开一样走 `OpenDocuments`，不绕过服务。
fn open_file(view: &Entity<Workbench>, cx: &mut VisualTestContext, resource: Resource) {
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                resource,
                OpenOptions {
                    preview_tab: false,
                    ..OpenOptions::default()
                },
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    settle(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    settle(cx);
}

/// 活动标签的失败原因（`None` 表示没有失败）。
fn failure(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<Error> {
    view.read_with(cx, |v, _| match &v.editor.active().current()?.content {
        Content::Failed(error) => Some(error.clone()),
        _ => None,
    })
}

fn active_tab(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> (EditorGroupId, TabId) {
    view.read_with(cx, |v, _| {
        (v.editor.focused, v.editor.active().current().unwrap().id)
    })
}

/// 点「仍然打开」走的就是错误页按钮的那条路（`Workbench::editor` 上的 `open_anyway`）。
fn click_open_anyway(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    let (group, tab) = active_tab(view, cx);
    view.update(cx, |v, cx| {
        v.editor.open_anyway(group, tab);
        cx.notify();
    });
    settle(cx);
}

fn status_text(view: &Entity<Workbench>, cx: &mut VisualTestContext, id: &str) -> String {
    view.read_with(cx, |v, _| {
        v.shell
            .status_bar
            .get(id)
            .map(|e| e.text.clone())
            .unwrap_or_default()
    })
}

fn document_text(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        let text = &v
            .editor
            .active()
            .current()
            .and_then(Tab::opened)
            .unwrap()
            .document
            .text;
        let range = rove_base::Span::new(
            rove_base::ByteOffset(0),
            rove_base::ByteOffset(text.byte_len()),
        )
        .unwrap();
        let mut out = String::new();
        text.visit_chunks(range, &mut |chunk| out.push_str(chunk))
            .unwrap();
        out
    })
}

// ---------------------------------------------------------------- 整条链路

/// 拒绝 → 错误页（文案与「仍然打开」）→ 仍然打开 → 正文可见。
#[gpui::test]
fn binary_file_is_rejected_then_open_anyway_shows_its_text(cx: &mut TestAppContext) {
    let fx = Fixture::new();
    fx.file("Assets/Textures/Player.png", BINARY);
    let (view, cx) = fx.open(cx);
    open_file(
        &view,
        cx,
        Resource::Workspace(WorkspacePath::new("Assets/Textures/Player.png").unwrap()),
    );

    // 1. 普通打开被 rove-buffer 的二进制判定拒绝，标签上是错误页而不是一屏乱码。
    assert_eq!(failure(&view, cx), Some(Error::Unsupported));
    let resource = view.read_with(cx, |v, _| {
        v.editor.active().current().unwrap().resource.clone()
    });

    // 2. 错误页的文案是 UX_SPEC §3.5 的那一句，并且提供「仍然打开」。
    assert_eq!(
        error_page_key(&Error::Unsupported, &resource),
        "editors.fileBinary"
    );
    assert!(can_open_anyway(&Error::Unsupported, &resource));
    let shown = view.read_with(cx, |v, _| {
        (
            v.label("editors.fileBinary", &[]),
            v.label("editors.openAnyway", &[]),
        )
    });
    assert_eq!(shown.0, "此文件是二进制文件或使用了不支持的文本编码。");
    assert_eq!(shown.1, "仍然打开");

    // 3. 「仍然打开」：同一个标签换成就绪的文本文档，字节原样按 lossy 显示。
    click_open_anyway(&view, cx);
    assert_eq!(failure(&view, cx), None);
    assert_eq!(document_text(&view, cx), String::from_utf8_lossy(BINARY));

    // 4. 正文真的画了出来（错误页已经让位给代码视图）。
    let painted = view.read_with(cx, |v, cx| v.code_view().read(cx).painted_text(0));
    assert_eq!(painted.as_deref(), Some(FIRST_LINE));
}

/// 「仍然打开」之后状态栏的文件状态条目与只读表现与 #27 的「通过编码重新打开」一致：
/// 编码条目照常显示识别结果，文档是普通的可编辑文档（不额外加只读原因）。
#[gpui::test]
fn binary_open_anyway_keeps_the_encoding_entry_and_stays_editable(cx: &mut TestAppContext) {
    let fx = Fixture::new();
    fx.file("Assets/Textures/Player.png", BINARY);
    let (view, cx) = fx.open(cx);
    open_file(
        &view,
        cx,
        Resource::Workspace(WorkspacePath::new("Assets/Textures/Player.png").unwrap()),
    );
    // 失败的标签没有活动文本编辑器，六个条目全部移除（#27 的既有行为）。
    assert_eq!(status_text(&view, cx, ids::ENCODING), "");

    click_open_anyway(&view, cx);
    assert_eq!(status_text(&view, cx, ids::ENCODING), "UTF-8");
    // 夹具里 CRLF 与 LF 都有，换行符条目照常给出「混合」——和普通文本文件同一条判定。
    assert_eq!(status_text(&view, cx, ids::EOL), "混合");
    view.read_with(cx, |v, cx| {
        assert!(
            v.active_read_only(cx).is_empty(),
            "「仍然打开」不额外加只读原因：{:?}",
            v.active_read_only(cx)
        );
        assert!(
            !v.tab_read_only(v.editor.active().current().unwrap()),
            "标签上不该有锁形图标"
        );
    });
}

/// 「仍然打开」是标签上的决定：之后的「重试」继续按强制打开读，不会退回错误页。
#[gpui::test]
fn retry_after_open_anyway_keeps_forcing(cx: &mut TestAppContext) {
    let fx = Fixture::new();
    fx.file("Assets/Textures/Player.png", BINARY);
    let (view, cx) = fx.open(cx);
    open_file(
        &view,
        cx,
        Resource::Workspace(WorkspacePath::new("Assets/Textures/Player.png").unwrap()),
    );
    click_open_anyway(&view, cx);
    assert_eq!(failure(&view, cx), None);

    let (group, tab) = active_tab(&view, cx);
    view.update(cx, |v, cx| {
        v.editor.retry(group, tab);
        cx.notify();
    });
    settle(cx);
    assert_eq!(failure(&view, cx), None);
    assert_eq!(document_text(&view, cx), String::from_utf8_lossy(BINARY));
}

/// 虚拟资源的「不受支持」不是二进制：错误页用通用文案，也不提供「仍然打开」（没有字节可读）。
#[gpui::test]
fn virtual_resources_keep_the_generic_unsupported_page() {
    let virtual_resource = Resource::Virtual("rove-cs-metadata:Unknown".into());
    assert_eq!(
        error_page_key(&Error::Unsupported, &virtual_resource),
        "editors.fileUnsupported"
    );
    assert!(!can_open_anyway(&Error::Unsupported, &virtual_resource));
    // 其他错误分支不受影响（只有 `Unsupported` 能「仍然打开」）。
    let file = Resource::File(PathBuf::from("C:/x/Player.png"));
    assert!(!can_open_anyway(&Error::NotFound, &file));
    assert_eq!(
        error_page_key(&Error::NotFound, &file),
        "editors.fileMissing"
    );
    assert_eq!(
        error_page_key(&Error::OutOfBounds, &file),
        "editors.invalidLocation"
    );
}

// ---------------------------------------------------------------- 真实项目

/// #31 场景 3 的现象：真实 Unity 项目里 `Ctrl+P` 会列出 `.png`，打开后走错误页。
/// 这里把那条路走完：错误页 → 「仍然打开」→ 正文可见。
#[gpui::test]
fn binary_png_in_a_real_unity_project_opens_anyway(cx: &mut TestAppContext) {
    let Some(root) = UnityProject::Medium.path() else {
        eprintln!("[#209] 未设置 ROVE_TEST_UNITY_MEDIUM，跳过真实项目用例");
        return;
    };
    let Some(png) = find_png(&root, 40_000) else {
        eprintln!("[#209] 真实项目里没找到合适的 .png，跳过");
        return;
    };
    let user = tempfile::tempdir().unwrap();
    std::fs::write(user.path().join("settings.json"), "{}").unwrap();
    let mut options = SettingsOptions::new(user.path(), user.path().join("backup"));
    options.watch = None;
    let service = Arc::new(SettingsService::open(
        options,
        crate::app_settings::registry().unwrap(),
    ));
    let settings: Arc<dyn Settings> = service.clone();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Workbench::with_settings(
            root.clone(),
            Instant::now(),
            LayoutStore::disabled(),
            settings,
            cx,
        )
        .unwrap()
    });
    settle(cx);
    cx.update(|window, cx| window.focus(&view.read(cx).focus));
    open_file(&view, cx, Resource::File(png.clone()));

    assert_eq!(
        failure(&view, cx),
        Some(Error::Unsupported),
        "真实 .png 应当先被拒绝：{}",
        png.display()
    );
    let resource = view.read_with(cx, |v, _| {
        v.editor.active().current().unwrap().resource.clone()
    });
    assert_eq!(
        error_page_key(&Error::Unsupported, &resource),
        "editors.fileBinary"
    );
    assert!(can_open_anyway(&Error::Unsupported, &resource));

    click_open_anyway(&view, cx);
    assert_eq!(failure(&view, cx), None, "「仍然打开」之后不应再失败");
    assert!(
        !document_text(&view, cx).is_empty(),
        "「仍然打开」之后正文不应为空"
    );
    let painted = view.read_with(cx, |v, cx| v.code_view().read(cx).painted_text(0));
    assert!(painted.is_some(), "「仍然打开」之后首行应当画出来");
    eprintln!(
        "[#209] 真实项目的 {} 走通了「拒绝 → 仍然打开 → 显示」",
        png.file_name().unwrap().to_string_lossy()
    );
}

/// 在项目里找一个不太大的 `.png`（只读遍历，最多看 `budget` 个条目）。
fn find_png(root: &std::path::Path, budget: usize) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > budget {
                return None;
            }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.len() > 64
                && meta.len() < 4 * 1024 * 1024
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("png"))
            {
                return Some(path);
            }
        }
    }
    None
}
