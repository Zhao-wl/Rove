//! #27 入口集成测试（真实 GPUI 窗口）：六个条目的顺序与文字、随光标与标签同步、GBK 文件的
//! 「自动识别」与「通过编码重新打开」、混合换行符、超大文件与只读、语言模式选择器。
//!
//! 运行：`cargo test -p rove-ui status_ -j 1`。
use super::*;
use crate::{
    documents::OpenOptions,
    shell::status::{Alignment, StatusEntry},
    status_items::registry as ids,
};
use gpui::{TestAppContext, VisualTestContext};
use rove_config::{SettingsOptions, SettingsService};

struct Fixture {
    _user: tempfile::TempDir,
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

    fn path(&self, path: &str) -> PathBuf {
        self.folder.path().join(path)
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
    for _ in 0..3 {
        cx.update(|window, cx| {
            window.refresh();
            cx.refresh_windows();
        });
        cx.run_until_parked();
    }
}

fn open_file(view: &Entity<Workbench>, cx: &mut VisualTestContext, path: &str) {
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(path).unwrap()),
                OpenOptions {
                    preview_tab: false,
                    ..OpenOptions::default()
                },
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    cx.run_until_parked();
    refresh(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    refresh(cx);
}

/// 右侧条目，从左到右。只保留 #27 的条目：通知铃铛（#16，固定在最右）是否出现取决于有没有
/// 通知，与本 issue 无关；它和六个条目的相对顺序由 `status_items` 的登记表单元测试覆盖。
fn right_items(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(String, String)> {
    view.read_with(cx, |v, _| {
        v.shell
            .status_bar
            .items(&v.shell.contributions, Alignment::Right)
            .into_iter()
            .filter(|(id, _, _)| ids::ITEMS.iter().any(|(own, ..)| own == id))
            .map(|(id, _, entry)| (id.to_owned(), entry.text.clone()))
            .collect()
    })
}

fn entry(view: &Entity<Workbench>, cx: &mut VisualTestContext, id: &str) -> Option<StatusEntry> {
    view.read_with(cx, |v, _| v.shell.status_bar.get(id).cloned())
}

fn text_of(view: &Entity<Workbench>, cx: &mut VisualTestContext, id: &str) -> String {
    entry(view, cx, id).unwrap_or_default().text
}

fn tooltip_of(view: &Entity<Workbench>, cx: &mut VisualTestContext, id: &str) -> String {
    entry(view, cx, id)
        .and_then(|entry| entry.tooltip)
        .unwrap_or_default()
}

/// 点击一个状态栏条目：与 `render_status_bar` 的 `on_click` 走同一条路（条目的命令 →
/// `run_external_command`），选择器在下一帧打开。
fn click(view: &Entity<Workbench>, cx: &mut VisualTestContext, id: &str) {
    let command = view
        .read_with(cx, |v, _| {
            v.shell
                .status_bar
                .get(id)
                .and_then(|e| e.command.clone())
                .or_else(|| {
                    v.shell
                        .contributions
                        .get(id)
                        .and_then(|c| c.command.clone())
                })
        })
        .expect("条目有默认命令");
    view.update(cx, |v, cx| v.status_command(&command, &Value::Null, cx));
    cx.run_until_parked();
    refresh(cx);
}

fn pick_rows(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| {
        let q = v.quick.view.as_ref().expect("浮层已创建").read(cx);
        let a = q.service.active().expect("浮层已打开");
        a.list
            .display_rows(0..a.list.len())
            .iter()
            .map(|r| r.label.to_string())
            .collect()
    })
}

fn pick_open(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |v, cx| {
        v.quick
            .view
            .as_ref()
            .is_some_and(|q| q.read(cx).service.active().is_some())
    })
}

/// 选中标签为 `label` 的那一行并回车。
fn pick(view: &Entity<Workbench>, cx: &mut VisualTestContext, label: &str) {
    let index = pick_rows(view, cx)
        .iter()
        .position(|row| row == label)
        .unwrap_or_else(|| panic!("找不到 {label}：{:?}", pick_rows(view, cx)));
    for _ in 0..index {
        cx.simulate_keystrokes("down");
    }
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    refresh(cx);
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

fn gbk(text: &str) -> Vec<u8> {
    let encoding = rove_buffer::settings::rs_encoding(&rove_buffer::Encoding::Gbk).unwrap();
    let (bytes, _, had_errors) = encoding.encode(text);
    assert!(!had_errors, "测试数据必须能用 GBK 编码");
    bytes.into_owned()
}

#[gpui::test]
fn status_right_items_follow_vscode_order_and_text(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    fx.file("Assets/A.cs", "class A {}\n");
    let (view, cx) = fx.open(cx);
    // 没有活动编辑器：文件相关的条目全部隐藏（与 VS Code 一致）。
    assert_eq!(right_items(&view, cx), Vec::<(String, String)>::new());

    open_file(&view, cx, "Assets/A.cs");
    let items = right_items(&view, cx);
    let order: Vec<&str> = items.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        order,
        [
            ids::CURSOR,
            ids::INDENT,
            ids::ENCODING,
            ids::EOL,
            ids::LANGUAGE
        ],
        "可编辑的普通文件没有只读条目"
    );
    let texts: Vec<&str> = items.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(texts, ["行 1，列 1", "空格: 4", "UTF-8", "LF", "C#"]);
    // 悬停提示与点击命令都来自登记表。
    assert_eq!(tooltip_of(&view, cx, ids::CURSOR), "转到行/列");
    assert_eq!(
        entry(&view, cx, ids::LANGUAGE).unwrap().command.as_deref(),
        Some(ids::CHANGE_LANGUAGE_MODE)
    );
}

#[gpui::test]
fn status_cursor_item_follows_caret_and_selection(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    fx.file("Assets/A.cs", "class A {}\nvoid M() {}\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Assets/A.cs");
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, cx| {
        c.set_selection(
            crate::code_view::Selection::caret(crate::code_view::selection::pos(1, 5)),
            None,
            cx,
        )
    });
    refresh(cx);
    assert_eq!(text_of(&view, cx, ids::CURSOR), "行 2，列 6");
    // 有选区时附加已选字符数。
    code.update(cx, |c, cx| {
        c.set_selection(
            crate::code_view::Selection::new(
                crate::code_view::selection::pos(1, 0),
                crate::code_view::selection::pos(1, 4),
            ),
            None,
            cx,
        )
    });
    refresh(cx);
    assert_eq!(text_of(&view, cx, ids::CURSOR), "行 2，列 5（已选择 4）");
}

#[gpui::test]
fn status_gbk_file_is_detected_and_reopens_with_another_encoding(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    // 真实的 GBK 文件：注释里的中文用 GBK 编码（Unity 项目里很常见）。
    let source = "// 中文注释，用来验证自动识别\nclass A {}\n";
    fx.file("Assets/A.cs", gbk(source));
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Assets/A.cs");
    assert_eq!(text_of(&view, cx, ids::ENCODING), "GBK");
    assert!(
        tooltip_of(&view, cx, ids::ENCODING).contains("自动识别"),
        "{}",
        tooltip_of(&view, cx, ids::ENCODING)
    );
    assert!(document_text(&view, cx).contains("中文注释"));

    // 点击 → 「通过编码重新打开」→ UTF-8：显示乱码，悬停提示不再说“自动识别”。
    click(&view, cx, ids::ENCODING);
    assert!(pick_open(&view, cx));
    assert_eq!(pick_rows(&view, cx), ["通过编码重新打开"]);
    pick(&view, cx, "通过编码重新打开");
    assert!(pick_open(&view, cx), "第二层列出候选编码");
    pick(&view, cx, "UTF-8");
    assert_eq!(text_of(&view, cx, ids::ENCODING), "UTF-8");
    assert!(!tooltip_of(&view, cx, ids::ENCODING).contains("自动识别"));
    assert!(
        document_text(&view, cx).contains('\u{fffd}'),
        "用 UTF-8 读 GBK 字节是乱码"
    );

    // 再切回 GBK：内容恢复正常。
    click(&view, cx, ids::ENCODING);
    pick(&view, cx, "通过编码重新打开");
    pick(&view, cx, "GBK");
    assert_eq!(text_of(&view, cx, ids::ENCODING), "GBK");
    assert!(document_text(&view, cx).contains("中文注释"));
}

#[gpui::test]
fn status_mixed_line_endings_show_mixed_and_leave_the_file_untouched(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let bytes = b"class A {}\r\nvoid M() {}\nvoid N() {}\r\n".to_vec();
    fx.file("Assets/A.cs", &bytes);
    fx.file("Assets/B.cs", b"class B {}\r\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Assets/A.cs");
    assert_eq!(text_of(&view, cx, ids::EOL), "混合");
    assert!(tooltip_of(&view, cx, ids::EOL).contains("保持原样"));
    open_file(&view, cx, "Assets/B.cs");
    assert_eq!(text_of(&view, cx, ids::EOL), "CRLF");
    // 打开再关闭不改动文件（字节级比对）。
    view.update(cx, |v, _| {
        let group = v.editor.focused;
        let tabs: Vec<TabId> = v.editor.groups[&group].tabs.iter().map(|t| t.id).collect();
        for tab in tabs {
            v.editor.close_tab(group, tab);
        }
    });
    refresh(cx);
    assert_eq!(std::fs::read(fx.path("Assets/A.cs")).unwrap(), bytes);
    assert_eq!(right_items(&view, cx), Vec::<(String, String)>::new());
}

#[gpui::test]
fn status_huge_and_read_only_files_show_the_mark_with_reasons(cx: &mut TestAppContext) {
    // 超大文件阈值调小到 2 KB（真正的 64 MB 文件不适合进单元测试）。
    let fx = Fixture::new(r#"{ "rove.largeFile.threshold": 2048 }"#);
    fx.file("Logs/Editor.log", "line of log text\n".repeat(400));
    fx.file(
        "Library/PackageCache/com.unity.foo/Runtime/Foo.cs",
        "class Foo {}\n",
    );
    fx.file("Assets/A.cs", "class A {}\n");
    let (view, cx) = fx.open(cx);

    open_file(&view, cx, "Logs/Editor.log");
    assert_eq!(text_of(&view, cx, ids::READ_ONLY), "超大文件 · 只读");
    let tooltip = tooltip_of(&view, cx, ids::READ_ONLY);
    assert!(
        tooltip.contains("超大文件只能以只读方式打开。"),
        "{tooltip}"
    );
    assert!(tooltip.contains("语法高亮"), "{tooltip}");
    assert!(tooltip.contains("软换行"), "{tooltip}");
    // 只读是警告态，颜色来自登记过的 statusBarItem.warning* 键。
    assert_eq!(
        entry(&view, cx, ids::READ_ONLY).unwrap().kind,
        crate::shell::status::StatusKind::Warning
    );

    open_file(
        &view,
        cx,
        "Library/PackageCache/com.unity.foo/Runtime/Foo.cs",
    );
    assert_eq!(text_of(&view, cx, ids::READ_ONLY), "只读");
    assert!(tooltip_of(&view, cx, ids::READ_ONLY).contains("Unity 包缓存目录，修改会被覆盖。"));

    open_file(&view, cx, "Assets/A.cs");
    assert!(
        entry(&view, cx, ids::READ_ONLY).is_none(),
        "可写文件没有标记"
    );
}

#[gpui::test]
fn status_items_follow_tab_switches_within_one_frame_budget(cx: &mut TestAppContext) {
    let fx = Fixture::new(r#"{ "workbench.editor.enablePreview": false }"#);
    fx.file("Assets/A.cs", "class A {}\n".repeat(200));
    fx.file("Assets/data.json", "{\r\n  \"a\": 1\r\n}\r\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Assets/A.cs");
    open_file(&view, cx, "Assets/data.json");
    assert_eq!(
        right_items(&view, cx)
            .iter()
            .map(|(_, t)| t.clone())
            .collect::<Vec<_>>(),
        ["行 1，列 1", "空格: 2", "UTF-8", "CRLF", "JSON"]
    );
    // 切回第一个标签：每一项都同步。
    view.update(cx, |v, _| v.editor.cycle_tab(false));
    refresh(cx);
    assert_eq!(text_of(&view, cx, ids::LANGUAGE), "C#");
    assert_eq!(text_of(&view, cx, ids::EOL), "LF");

    // 一次完整重算的耗时（强制让缓存失效）。机器负载较高时只看量级。
    let elapsed = cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.status.invalidate();
            let start = Instant::now();
            v.sync_status(window, cx);
            start.elapsed()
        })
    });
    eprintln!("STATUS_SYNC_FULL_RECOMPUTE {elapsed:?}");
    assert!(
        elapsed < Duration::from_millis(16),
        "状态栏同步 {elapsed:?} 超过一帧预算"
    );
}

#[gpui::test]
fn status_language_picker_switches_an_asmdef_and_rehighlights(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    fx.file(
        "Assets/Game.asmdef",
        "{\n  \"name\": \"Game\",\n  \"references\": []\n}\n",
    );
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Assets/Game.asmdef");
    // `.asmdef` 默认就是 JSON（#12 的扩展名表）。
    assert_eq!(text_of(&view, cx, ids::LANGUAGE), "JSON");
    assert!(highlighted(&view, cx), "JSON 已高亮");

    // 切成纯文本：高亮消失。
    click(&view, cx, ids::LANGUAGE);
    assert!(pick_open(&view, cx));
    assert_eq!(pick_rows(&view, cx)[0], "JSON", "当前语言排在最前");
    pick(&view, cx, "纯文本");
    assert_eq!(text_of(&view, cx, ids::LANGUAGE), "纯文本");
    assert!(!highlighted(&view, cx), "纯文本没有语法着色");

    // 切回 JSON：立即重新高亮。
    click(&view, cx, ids::LANGUAGE);
    pick(&view, cx, "JSON");
    assert_eq!(text_of(&view, cx, ids::LANGUAGE), "JSON");
    assert!(highlighted(&view, cx), "切回 JSON 后重新着色");
}

/// 活动标签的语法快照里有没有 token（语言模式切换后的重新高亮）。
fn highlighted(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    refresh(cx);
    view.read_with(cx, |v, _| {
        let Some(syntax) = v
            .editor
            .active()
            .current()
            .and_then(Tab::opened)
            .and_then(|o| o.syntax.clone())
        else {
            return false;
        };
        (0..4).any(|line| {
            syntax
                .tokens(Line(line))
                .is_ok_and(|tokens| !tokens.is_empty())
        })
    })
}

#[gpui::test]
fn status_indent_picker_changes_this_files_indentation(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    fx.file("Assets/A.cs", "class A {\n\tvoid M() {\n\t}\n}\n");
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Assets/A.cs");
    // 文件用制表符缩进：`editor.detectIndentation` 默认开启，条目显示制表符。
    assert_eq!(text_of(&view, cx, ids::INDENT), "制表符长度: 4");
    assert!(tooltip_of(&view, cx, ids::INDENT).contains("检测"));

    click(&view, cx, ids::INDENT);
    assert_eq!(
        pick_rows(&view, cx),
        [
            "使用空格缩进",
            "使用制表符缩进",
            "从内容中检测缩进",
            // 改写文件内容的两项走 #46 的编辑核心，只在可编辑的文件上出现。
            "将缩进转换为空格",
            "将缩进转换为制表符"
        ]
    );
    pick(&view, cx, "使用空格缩进");
    assert!(pick_open(&view, cx), "接着选缩进宽度");
    pick(&view, cx, "2");
    assert_eq!(text_of(&view, cx, ids::INDENT), "空格: 2");
    // 制表位宽度同步到代码视图。
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| assert_eq!(c.options().tab_size, 2));

    // 「从内容中检测缩进」回到文件本身的缩进。
    click(&view, cx, ids::INDENT);
    pick(&view, cx, "从内容中检测缩进");
    assert_eq!(text_of(&view, cx, ids::INDENT), "制表符长度: 4");
}

// --------------------------------------------------------------- 真实 Unity 项目

/// 在中型 Unity 项目里找一个真实的非 UTF-8（需要自动识别编码）的 `.cs` 文件。
/// 目录遍历有预算上限，跳过 `Library` / `Temp` / `obj` / `Logs` / `.git`。
fn find_non_utf8_source(root: &std::path::Path) -> Option<PathBuf> {
    // 这类文件不一定在 `Assets` 下（也可能在工具或归档目录里），所以从项目根开始走；
    // 目录项预算兜底，避免在超大项目上一直扫下去。
    let mut budget = 150_000usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if budget == 0 {
                return None;
            }
            budget -= 1;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy().into_owned();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                if !matches!(&*name, "Library" | "Temp" | "obj" | "Logs" | ".git") {
                    stack.push(path);
                }
                continue;
            }
            if !name.ends_with(".cs") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            // 非法 UTF-8 的文本文件 = 需要自动识别编码的那一类（Unity 项目里通常是 GBK）。
            if !bytes.is_empty() && std::str::from_utf8(&bytes).is_err() {
                return Some(path);
            }
        }
    }
    None
}

/// 验收口径：真实 Unity 项目里的 GBK 源码文件。未设置环境变量时跳过（AGENTS.md §8）；
/// 项目里没有这类文件时也跳过并说明原因。文件路径不写进仓库，只把内容复制到临时工作区。
#[gpui::test]
fn status_encoding_on_a_real_unity_source_file(cx: &mut TestAppContext) {
    let project =
        rove_test_support::require_unity_project!(rove_test_support::UnityProject::Medium);
    let Some(source) = find_non_utf8_source(&project) else {
        eprintln!("已跳过：中型 Unity 项目里没有找到非 UTF-8 的 .cs 文件");
        return;
    };
    let bytes = std::fs::read(&source).unwrap();
    let fx = Fixture::new("{}");
    fx.file("Assets/Real.cs", &bytes);
    let (view, cx) = fx.open(cx);
    open_file(&view, cx, "Assets/Real.cs");
    let encoding = text_of(&view, cx, ids::ENCODING);
    eprintln!("真实 Unity 源码文件的编码识别结果：{encoding}");
    assert_ne!(encoding, "UTF-8", "非法 UTF-8 的文件会被自动识别成别的编码");
    assert!(
        tooltip_of(&view, cx, ids::ENCODING).contains("自动识别"),
        "{}",
        tooltip_of(&view, cx, ids::ENCODING)
    );
    let original = document_text(&view, cx);
    assert!(
        !original.contains('\u{fffd}'),
        "按识别出的编码读出来不是乱码"
    );

    // 通过编码重新打开 → UTF-8：显示乱码；再切回去：恢复原样。
    click(&view, cx, ids::ENCODING);
    pick(&view, cx, "通过编码重新打开");
    pick(&view, cx, "UTF-8");
    assert_eq!(text_of(&view, cx, ids::ENCODING), "UTF-8");
    assert!(document_text(&view, cx).contains('\u{fffd}'));
    click(&view, cx, ids::ENCODING);
    pick(&view, cx, "通过编码重新打开");
    pick(&view, cx, &encoding);
    assert_eq!(text_of(&view, cx, ids::ENCODING), encoding);
    assert_eq!(document_text(&view, cx), original);
}
