//! #150 v0.1 发布候选的集成验收：**提前条目叠在 M1–M3 上**之后的接缝。
//!
//! 和各自 issue 的入口测试的区别：#52 只看多光标与行操作、#53 只看自动编辑、#55 只看粘性滚动、
//! #58 只看软换行、#60 只看三个导航入口——每一套都在**单独的代码视图**里，而且互不打开。
//! R77 §2 的场景 6 要求的是「粘性滚动 + 软换行 + 编辑 + 索引增量更新同时进行」，
//! 本模块固化的就是这种同时打开时的接缝：
//!
//! 1. `v01_sticky_scroll_agrees_with_the_breadcrumb_while_soft_wrap_is_on`
//!    （场景 3 + 4 的判据：粘性区与面包屑是同一条符号路径，软换行不改变这个结论）；
//! 2. `v01_a_huge_document_switches_off_sticky_scroll_and_soft_wrap_together`
//!    与 `v01_a_large_document_only_switches_off_sticky_scroll`
//!    （场景 3「超大文件下按分级关闭」、场景 4「超大文件下禁用」是**同一张分级表**的两行）；
//! 3. `v01_multicursor_line_edit_with_wrap_on_undoes_as_one_unit`
//!    （场景 1 的判据「多光标下的撤销是一个单元」在软换行打开时仍然成立）；
//! 4. `v01_comment_toggle_on_a_gbk_file_saves_with_no_extra_difference`
//!    （场景 2 的判据「GBK 文件编辑后保存零额外差异」，改动由 `language-configuration.json`
//!    驱动的注释切换产生，不是手敲字符）；
//! 5. `v01_editing_with_sticky_and_wrap_on_goes_approximate_then_exact`
//!    （场景 6：粘性滚动与软换行开着时，编辑 → 保存 → 索引重绑这条 M2/M3 链路不受影响）；
//! 6. `v01_real_project_sticky_wrap_and_edit_hold_together`
//!    （真实 Unity 项目的 `.cs` 复制进临时工作区后走完同一条路，**不往测试项目里写任何东西**）。
//!
//! 运行：
//!
//! ```powershell
//! cargo test -p rove-ui v01_ -j 1 -- --nocapture
//! ```
//!
//! 真实项目那一条需要 `ROVE_TEST_UNITY_MEDIUM`，缺失时**跳过而不是失败**（原因打到 stderr）。
use super::*;
use crate::code_view::scroll::ScrollPosition;
use crate::documents::OpenOptions;
use crate::save::registry as save_ids;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Utf8Column};
use rove_buffer::SizeClass;
use rove_config::{SettingsOptions, SettingsService};
use rove_lang_cs::query::{CsQueries, NavigationKind, Precision, QueryPoint};
use rove_lang_cs::router::testing::{find_word, metadata_sample_index, GAME_CS, GAME_PATH};
use rove_lang_cs::router::{
    IndexHolder, L2Snapshot, ManualSemantic, SemanticSource, SemanticStatus,
};
use rove_lang_cs::CsIndex;
use rove_test_support::UnityProject;

// --------------------------------------------------------------------------- 夹具

/// 粘性滚动 + 软换行同时打开（场景 3 / 4 / 6 的公共设置）。
const STICKY_AND_WRAP: &str = r#"{ "editor.stickyScroll.enabled": true, "editor.wordWrap": "on" }"#;

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

    fn file(&self, path: &str, bytes: &[u8]) -> PathBuf {
        let full = self.folder.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, bytes).unwrap();
        full
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
}

fn refresh(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..6 {
        refresh(cx);
    }
}

fn open_file(view: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> Entity<CodeView> {
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
    settle(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    settle(cx);
    code
}

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

fn caret(code: &Entity<CodeView>, at: Position, cx: &mut VisualTestContext) {
    code.update(cx, |c, cx| {
        c.set_selection(code_view::Selection::caret(at), None, cx)
    });
    settle(cx);
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    settle(cx);
}

/// 用命令 id 驱动代码视图（和 #52 / #55 / #58 的入口测试同一条路径）。
fn run_command(code: &Entity<CodeView>, id: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        code.update(cx, |c, cx| {
            assert!(c.execute(id, window, cx), "命令 `{id}` 没有执行");
        });
    });
    settle(cx);
}

fn save_command(cx: &mut VisualTestContext) {
    let action = save_ids::action_for("workbench.action.files.save").unwrap();
    cx.update(|window, cx| window.dispatch_action(action, cx));
    settle(cx);
}

fn scroll_to(code: &Entity<CodeView>, line: u64, cx: &mut VisualTestContext) {
    code.update(cx, |c, cx| {
        c.set_scroll_position(
            ScrollPosition {
                top: line as f64,
                x: c.scroll_position().x,
            },
            false,
            cx,
        )
    });
    settle(cx);
}

fn text_of(code: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    code.read_with(cx, |c, _| {
        let t = c.text().unwrap();
        let mut out = String::new();
        t.visit_chunks(
            rove_base::Span::new(
                rove_base::ByteOffset(0),
                rove_base::ByteOffset(t.byte_len()),
            )
            .unwrap(),
            &mut |s| out.push_str(s),
        )
        .unwrap();
        out
    })
}

/// 焦点组的面包屑文字（口径同 #26 与 M3 验收）。
fn breadcrumb_text(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    view.update(cx, |v, _| {
        let group = v.editor.focused;
        let Some((_, symbols, segments)) = v.group_segments(group) else {
            return Vec::new();
        };
        segments
            .iter()
            .map(|s| match s {
                crate::outline::model::Segment::File { label, .. } => label.to_string(),
                crate::outline::model::Segment::Symbol(id) => symbols
                    .as_ref()
                    .unwrap()
                    .node(id.to_owned())
                    .unwrap()
                    .symbol
                    .name
                    .to_string(),
            })
            .collect()
    })
}

/// 粘性区每一行的**文本**（粘性行是文档行号，这里取出该行的源码，便于与面包屑对照）。
fn sticky_line_texts(code: &Entity<CodeView>, cx: &mut VisualTestContext) -> Vec<String> {
    let text = text_of(code, cx);
    let lines: Vec<&str> = text.lines().collect();
    code.read_with(cx, |c, _| {
        c.sticky_lines()
            .iter()
            .map(|sticky| {
                lines
                    .get(sticky.line as usize)
                    .map(|line| line.trim().to_string())
                    .unwrap_or_default()
            })
            .collect()
    })
}

/// 查询层（#38 的路由）对某个位置的精确度。
fn router_precision(
    view: &Entity<Workbench>,
    cx: &VisualTestContext,
    path: &str,
    at: Position,
    version: Version,
) -> Precision {
    let point = QueryPoint {
        resource: Resource::Workspace(WorkspacePath::new(path).unwrap()),
        position: at,
        version,
    };
    view.read_with(cx, |v, _| {
        v.cs_router()
            .navigate(NavigationKind::Definition, &point, &Cancellation::default())
    })
    .expect("查询路由要给出答复")
    .precision
}

/// 把索引换成「包含 `text` 这份内容」的一代（口径同 M3 验收：模拟保存后的单文件重绑完成）。
fn reindex(
    folder: &std::path::Path,
    semantic: &Arc<ManualSemantic>,
    text: &str,
    generation: u64,
) -> Arc<CsIndex> {
    let file = folder.join(format!("index{generation}.idx"));
    metadata_sample_index(text).0.write(&file).unwrap();
    let index = Arc::new(CsIndex::open(&file).unwrap());
    semantic.set(
        Some(L2Snapshot {
            holder: index.clone() as Arc<dyn IndexHolder>,
            generation,
            trusted: true,
        }),
        SemanticStatus::Ready,
    );
    index
}

/// 深层嵌套的 C#：命名空间 → 类 → 内部类 → 方法，方法体里有很长的一行（软换行要有活干）。
fn nested_csharp() -> String {
    let long = "x".repeat(400);
    let mut text = String::from("using UnityEngine;\n\nnamespace Game.Deep\n{\n");
    text.push_str("    public class Outer : MonoBehaviour\n    {\n");
    text.push_str("        public class Inner\n        {\n");
    for method in 0..6 {
        text.push_str(&format!(
            "            public void Method{method}(int value)\n            {{\n"
        ));
        for step in 0..8 {
            text.push_str(&format!(
                "                var step{step} = value * {step};\n"
            ));
        }
        text.push_str(&format!("                // {long}\n"));
        text.push_str("            }\n\n");
    }
    text.push_str("        }\n    }\n}\n");
    text
}

fn line_of(text: &str, needle: &str) -> u64 {
    text.lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("找不到 {needle}")) as u64
}

// ===================================================================== 场景 3 + 4

/// 场景 3 的判据「与面包屑一致」在**软换行打开**时仍然成立：
/// 粘性区自外向内就是面包屑里那条符号路径（命名空间 → 类 → 内部类 → 方法）。
///
/// 这正是 #55 与 #58 之间的接缝：粘性行是**文档行**，软换行之后一条文档行会占多条屏幕行，
/// 两边用错坐标系的话，粘性区显示的就不是面包屑里的那几级。
#[gpui::test]
fn v01_sticky_scroll_agrees_with_the_breadcrumb_while_soft_wrap_is_on(cx: &mut TestAppContext) {
    let text = nested_csharp();
    let fx = Fixture::new(STICKY_AND_WRAP);
    fx.file("Assets/Deep.cs", text.as_bytes());
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, "Assets/Deep.cs", cx);

    assert!(
        code.read_with(cx, |c, _| c.word_wrap_enabled()),
        "editor.wordWrap = on 应当生效"
    );
    assert!(
        code.read_with(cx, |c, _| c.sticky_enabled()),
        "普通大小的文档上粘性滚动应当可用"
    );

    // 滚到第 3 个方法的方法体里，并把插入符放在同一行：面包屑与粘性区应当说同一件事。
    // **坐标系**：`set_scroll_position` 的 top 是屏幕行，软换行之后它和文档行不是一回事，
    // 所以这里先把文档位置换算成屏幕行（`visual_line_of`）再滚——这正是两个功能的接缝。
    let method = line_of(&text, "public void Method3(");
    let inside = pos(method + 4, 20);
    let visual = code.update(cx, |c, _| c.visual_line_of(inside));
    assert!(
        visual > method + 4,
        "方法体上方有超长行，屏幕行号应当大于文档行号（{visual} vs {}）",
        method + 4
    );
    scroll_to(&code, visual, cx);
    caret(&code, inside, cx);

    let sticky = sticky_line_texts(&code, cx);
    let crumbs = breadcrumb_text(&view, cx);
    assert!(
        !sticky.is_empty(),
        "方法体里应当有粘性行；面包屑是 {crumbs:?}"
    );
    // 面包屑前面是目录与文件名，后面才是符号；粘性区只有符号。符号名按 `(` 之前的部分比较
    //（面包屑写成 `Method3(int)`，源码里是 `public void Method3(int value)`）。
    let symbols: Vec<&str> = crumbs
        .iter()
        .map(|crumb| crumb.split('(').next().unwrap_or(crumb))
        .filter(|name| text.contains(*name))
        .collect();
    assert_eq!(
        sticky.len(),
        symbols.len(),
        "视口首行与插入符在同一行时，粘性区级数应当与面包屑的符号级数相同：粘性 {sticky:?} 面包屑 {crumbs:?}"
    );
    for (line, name) in sticky.iter().zip(&symbols) {
        assert!(
            line.contains(name),
            "粘性行 `{line}` 与面包屑 `{name}` 对不上（粘性 {sticky:?} 面包屑 {crumbs:?}）"
        );
    }

    // 软换行开着时，列号仍然按**文档列**算（#58 的判据）：插入符所在的文档位置不变。
    assert_eq!(
        code.read_with(cx, |c, _| c.selection().active),
        pos(method + 4, 20),
        "软换行不改变插入符的文档坐标"
    );
}

/// 超大文件：粘性滚动与软换行**一起**关掉（`UX_SPEC` §3.1 的分级表），
/// 而且 `alt+z`（`editor.action.toggleWordWrap`）也打不开。
#[gpui::test]
fn v01_a_huge_document_switches_off_sticky_scroll_and_soft_wrap_together(cx: &mut TestAppContext) {
    let text = nested_csharp();
    // 阈值调小，让一个小文件走完**同一条**超大文件路径（口径同 #22 的入口测试）。
    let fx = Fixture::new(
        r#"{ "editor.stickyScroll.enabled": true, "editor.wordWrap": "on", "rove.largeFile.threshold": 4096 }"#,
    );
    fx.file("Assets/Deep.cs", text.as_bytes());
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, "Assets/Deep.cs", cx);

    assert_eq!(
        code.read_with(cx, |c, _| c.document_level()),
        SizeClass::Huge,
        "阈值 4 KB 下这份文档应当被判为超大文件"
    );
    assert!(
        !code.read_with(cx, |c, _| c.word_wrap_enabled()),
        "超大文件禁用软换行（场景 4 判据）"
    );
    assert!(
        !code.read_with(cx, |c, _| c.sticky_enabled()),
        "超大文件关闭粘性滚动（场景 3 判据）"
    );
    assert!(
        code.read_with(cx, |c, _| c.sticky_lines().is_empty()),
        "关闭之后不应还留着粘性行"
    );

    // alt+z 在超大文件上不生效：会话覆盖不写、软换行仍然是关的。
    code.update(cx, |c, cx| c.toggle_word_wrap(cx));
    settle(cx);
    assert!(!code.read_with(cx, |c, _| c.word_wrap_enabled()));
    assert_eq!(code.read_with(cx, |c, _| c.word_wrap_override()), None);
}

/// 大文件（不是超大文件）：**只**关粘性滚动，软换行照常可用——
/// 「按分级关闭」指的就是这两行不一样。
///
/// 20 MB / 30 万行的真文件不适合放进单元测试，这里直接把文档级别设成 `Large`
/// （分级本身由 `rove-buffer` 的 `SizePolicy::classify` 测试覆盖）。
#[gpui::test]
fn v01_a_large_document_only_switches_off_sticky_scroll(cx: &mut TestAppContext) {
    let text = nested_csharp();
    let fx = Fixture::new(STICKY_AND_WRAP);
    fx.file("Assets/Deep.cs", text.as_bytes());
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, "Assets/Deep.cs", cx);

    code.update(cx, |c, cx| c.set_document_level(SizeClass::Large, cx));
    settle(cx);
    assert!(
        !code.read_with(cx, |c, _| c.sticky_enabled()),
        "大文件关闭粘性滚动"
    );
    assert!(
        code.read_with(cx, |c, _| c.word_wrap_enabled()),
        "大文件**不**关软换行"
    );
}

// ===================================================================== 场景 1 + 4

/// 场景 1 的判据「多光标下的撤销是一个单元」在软换行打开时仍然成立：
/// `ctrl+shift+l` 选中全部同名出现处 → 一次输入改掉所有 → **一次** `ctrl+z` 全部回退。
#[gpui::test]
fn v01_multicursor_line_edit_with_wrap_on_undoes_as_one_unit(cx: &mut TestAppContext) {
    let text = nested_csharp();
    let fx = Fixture::new(STICKY_AND_WRAP);
    fx.file("Assets/Deep.cs", text.as_bytes());
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, "Assets/Deep.cs", cx);
    let before = text_of(&code, cx);
    assert!(code.read_with(cx, |c, _| c.word_wrap_enabled()));

    // 选中第一个 `value`，再用 selectHighlights 把同名出现处全部选上。
    let (line, column) = find_word(&before, "value", 0);
    caret(&code, pos(u64::from(line), u64::from(column)), cx);
    run_command(&code, "editor.action.addSelectionToNextFindMatch", cx);
    run_command(&code, "editor.action.selectHighlights", cx);
    let carets = code.read_with(cx, |c, _| c.selections().len());
    assert!(carets > 1, "应当有多个光标，实际 {carets}");

    press("Z", cx);
    let edited = text_of(&code, cx);
    assert_ne!(edited, before, "多光标输入应当真的改了文本");

    press("ctrl-z", cx);
    assert_eq!(
        text_of(&code, cx),
        before,
        "多光标下的一次输入必须作为一个撤销单元整体回退"
    );
}

// ===================================================================== 场景 2

/// 场景 2 的判据「GBK 文件编辑后保存零额外差异」：改动由**注释切换**
///（`editor.action.commentLine`，行为来自 `language-configuration.json`）产生，
/// 保存之后文件仍然是 GBK，且差异恰好是加上去的 `//`。
#[gpui::test]
fn v01_comment_toggle_on_a_gbk_file_saves_with_no_extra_difference(cx: &mut TestAppContext) {
    let source = "namespace Game\n{\n    // 中文注释：位置同步\n    public class Player\n    {\n        public void Update()\n        {\n            var p = transform.position;\n        }\n    }\n}\n";
    let (gbk, _, unmappable) = encoding_rs::GBK.encode(source);
    assert!(!unmappable, "测试文本必须能用 GBK 表示");
    let gbk = gbk.into_owned();

    let fx = Fixture::new(STICKY_AND_WRAP);
    let path = fx.file("Assets/Gbk.cs", &gbk);
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, "Assets/Gbk.cs", cx);
    let text = text_of(&code, cx);
    assert!(text.contains("中文注释"), "GBK 文件应当按 GBK 解码");

    // 把带中文的那一行注释掉：改动来自语言配置里的 lineComment。
    let line = line_of(&text, "var p = transform.position;");
    caret(&code, pos(line, 0), cx);
    run_command(&code, "editor.action.commentLine", cx);
    let after = text_of(&code, cx);
    assert_ne!(after, text, "注释切换应当真的改了一行");

    save_command(cx);
    let saved = std::fs::read(&path).unwrap();
    let (decoded, _, had_errors) = encoding_rs::GBK.decode(&saved);
    assert!(!had_errors, "保存后的文件必须仍然是合法 GBK");
    assert_eq!(decoded, after, "保存回来的内容应当与缓冲区一致");
    assert_eq!(
        saved.len() - gbk.len(),
        "// ".len(),
        "差异必须恰好是注释标记本身（零额外差异）：{} → {}",
        gbk.len(),
        saved.len()
    );

    // 再切一次注释并保存：应当回到原来的字节，一个字节都不多。
    run_command(&code, "editor.action.commentLine", cx);
    save_command(cx);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        gbk,
        "注释切回去之后应当与原文件逐字节相同"
    );
}

// ===================================================================== 场景 6

/// 场景 6：粘性滚动与软换行开着、在方法体里编辑时，M2 / M3 的那条链路不受影响——
/// 有未保存修改时查询标为**近似**，保存并等索引重绑完成后静默变回**精确**，
/// 而且粘性区与面包屑仍然一致、通知数始终是 0。
#[gpui::test]
fn v01_editing_with_sticky_and_wrap_on_goes_approximate_then_exact(cx: &mut TestAppContext) {
    let fx = Fixture::new(STICKY_AND_WRAP);
    fx.file(GAME_PATH, GAME_CS.as_bytes());
    let index_dir = fx.folder.path().to_path_buf();
    let base = {
        let file = index_dir.join("index0.idx");
        metadata_sample_index(GAME_CS).0.write(&file).unwrap();
        Arc::new(CsIndex::open(&file).unwrap())
    };
    let semantic = Arc::new(ManualSemantic::ready(base));
    let (view, cx) = fx.open(cx);
    let source = semantic.clone() as Arc<dyn SemanticSource>;
    view.update(cx, |v, cx| v.attach_semantic(Some(source), cx));
    let code = open_file(&view, GAME_PATH, cx);

    let (line, column) = find_word(GAME_CS, "position", 0);
    let at = pos(u64::from(line), u64::from(column));
    caret(&code, at, cx);
    assert_eq!(
        router_precision(
            &view,
            cx,
            GAME_PATH,
            at,
            code.read_with(cx, |c, _| c.version())
        ),
        Precision::Exact,
        "刚打开、没有修改：精确"
    );
    let crumbs_before = breadcrumb_text(&view, cx);

    // 在方法体里加一行（软换行与粘性滚动都开着）。
    caret(&code, pos(u64::from(line), 0), cx);
    press("/ / R o v e enter", cx);
    let edited = text_of(&code, cx);
    assert!(edited.contains("//Rove"), "应当真的改了一处");
    assert_eq!(
        router_precision(
            &view,
            cx,
            GAME_PATH,
            pos(u64::from(line) + 1, u64::from(column)),
            code.read_with(cx, |c, _| c.version())
        ),
        Precision::Approximate,
        "有未保存修改：近似"
    );

    // 保存 + 索引重绑：精确度静默回到 Exact，期间不弹通知。
    save_command(cx);
    reindex(&index_dir, &semantic, &edited, 1);
    settle(cx);
    assert_eq!(
        router_precision(
            &view,
            cx,
            GAME_PATH,
            pos(u64::from(line) + 1, u64::from(column)),
            code.read_with(cx, |c, _| c.version())
        ),
        Precision::Exact,
        "保存 + 重绑之后应当回到精确"
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.shell.feedback.notifications().len()),
        0,
        "整条链路不应弹任何通知"
    );
    assert_eq!(
        breadcrumb_text(&view, cx).first(),
        crumbs_before.first(),
        "面包屑的文件段不应因为这次编辑而变"
    );
    assert!(
        code.read_with(cx, |c, _| c.word_wrap_enabled() && c.sticky_enabled()),
        "编辑与重绑之后，软换行与粘性滚动仍然开着"
    );
}

// ===================================================================== 真实项目

/// 真实 Unity 项目（**中型**）里的 `.cs` 复制进临时工作区后走完场景 3 / 4 / 6 的主干。
/// `ROVE_TEST_UNITY_MEDIUM` 没设置时跳过。
#[gpui::test]
fn v01_real_medium_project_sticky_wrap_and_edit_hold_together(cx: &mut TestAppContext) {
    real_project_scenario(UnityProject::Medium, "中型", cx);
}

/// 同上，**大型**项目（R77 §2 要求两个项目各走一遍）。
/// `ROVE_TEST_UNITY_LARGE` 没设置时跳过。
#[gpui::test]
fn v01_real_large_project_sticky_wrap_and_edit_hold_together(cx: &mut TestAppContext) {
    real_project_scenario(UnityProject::Large, "大型", cx);
}

/// 真实 Unity 项目里的 `.cs` 复制进临时工作区后走完场景 3 / 4 / 6 的主干：
/// 粘性区与面包屑一致 → 改一处 → 保存 → 除改动处外逐字节不变。
///
/// **不往测试项目里写任何东西**（只读取字节）。
fn real_project_scenario(project: UnityProject, label: &str, cx: &mut TestAppContext) {
    let Some(root) = project.path() else {
        eprintln!("已跳过 v0.1 场景 3/4/6（{label}项目）：环境变量未设置或路径不存在");
        return;
    };
    let Some((rel, bytes)) = pick_nested_cs(&root) else {
        eprintln!(
            "已跳过：{label}项目里没有找到合适的 .cs（UTF-8、2 KB–200 KB、含嵌套类型与方法）"
        );
        return;
    };
    eprintln!(
        "[v0.1 场景 3/4/6｜{label}] 样本：{rel}（{} B）",
        bytes.len()
    );

    let fx = Fixture::new(STICKY_AND_WRAP);
    let path = fx.file("Assets/Sample.cs", &bytes);
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, "Assets/Sample.cs", cx);
    let text = text_of(&code, cx);
    assert!(code.read_with(cx, |c, _| c.word_wrap_enabled()));

    // 滚到文件中部（先把文档行换算成屏幕行，软换行下两者不同），让粘性区算出一条作用域链。
    let middle = (text.lines().count() as u64 / 2).max(1);
    let visual = code.update(cx, |c, _| c.visual_line_of(pos(middle, 0)));
    scroll_to(&code, visual, cx);
    let sticky = sticky_line_texts(&code, cx);
    assert!(!sticky.is_empty(), "文件中部应当有粘性行");

    // 判据「与面包屑一致」的可检验口径：把插入符放进**最内层粘性行所在的作用域**，
    // 面包屑给出的符号链必须和粘性区逐级相同。
    //（粘性区描述的是视口顶部那一行所在的作用域，插入符在别处时两者本来就可以不同。）
    let innermost = code.read_with(cx, |c, _| c.sticky_lines().last().copied().unwrap());
    // 插入符放在声明行的**第一个非空白字符**上：符号区间从声明本身开始，
    // 行首缩进处不属于任何符号（见 §5.1 的记录）。
    let indent = text
        .lines()
        .nth(innermost.line as usize)
        .map(|line| line.len() - line.trim_start().len())
        .unwrap_or(0) as u64;
    caret(&code, pos(innermost.line, indent), cx);
    let crumbs = breadcrumb_text(&view, cx);
    let symbols: Vec<&str> = crumbs
        .iter()
        .map(|crumb| crumb.split('(').next().unwrap_or(crumb))
        .filter(|name| text.contains(*name))
        .collect();
    assert_eq!(
        sticky.len(),
        symbols.len(),
        "粘性区级数应当与面包屑的符号级数相同：粘性 {sticky:?} 面包屑 {crumbs:?}"
    );
    for (line, name) in sticky.iter().zip(&symbols) {
        assert!(
            line.contains(name),
            "粘性行 `{line}` 与面包屑 `{name}` 对不上（粘性 {sticky:?} 面包屑 {crumbs:?}）"
        );
    }
    eprintln!("[v0.1 场景 3/4/6｜{label}] 粘性 {sticky:?}；面包屑 {crumbs:?}");

    // 改一处并保存：除改动处外逐字节不变（软换行与粘性滚动开着也不例外）。
    press("/ / R o v e", cx);
    save_command(cx);
    let saved = std::fs::read(&path).unwrap();
    let common_prefix = bytes.iter().zip(&saved).take_while(|(a, b)| a == b).count();
    let common_suffix = bytes
        .iter()
        .rev()
        .zip(saved.iter().rev())
        .take(bytes.len().min(saved.len()))
        .take_while(|(a, b)| a == b)
        .count();
    assert!(
        common_prefix + common_suffix >= bytes.len(),
        "改动处之外的字节也变了（前缀 {common_prefix} 后缀 {common_suffix} 原长 {}）",
        bytes.len()
    );
    assert_eq!(
        saved.len() - bytes.len(),
        "//Rove".len(),
        "长度变化必须恰好等于插入的字节数"
    );
    eprintln!("[v0.1 场景 3/4/6｜{label}] {rel}：粘性 + 软换行 + 编辑 + 保存，差异恰好 +6 B");
}

/// 中型项目里挑一个带嵌套结构的 `.cs`：UTF-8、2 KB–200 KB、至少 60 行、至少两级大括号缩进。
fn pick_nested_cs(root: &std::path::Path) -> Option<(String, Vec<u8>)> {
    let mut stack = vec![root.to_path_buf()];
    let mut scanned = 0usize;
    while let Some(dir) = stack.pop() {
        if scanned >= 3000 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if !matches!(
                    name.as_ref(),
                    "Library" | "Temp" | "obj" | "Logs" | "Build" | ".git" | ".vs"
                ) {
                    stack.push(path);
                }
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("cs") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() < 2048 || meta.len() > 200 * 1024 {
                continue;
            }
            scanned += 1;
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            if text.lines().count() < 60 {
                continue;
            }
            // 至少有一行缩进到第三层（命名空间 → 类 → 方法体），粘性区才有三级可看。
            if !text.lines().any(|line| line.starts_with("            ")) {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            return Some((rel, bytes));
        }
    }
    None
}
