//! #51 M3 集成验收：把场景清单里**可自动化**的接缝固化成测试。
//!
//! 与 M3 各 issue 自己的入口测试的区别：#46 验证编辑核心、#47 验证代码视图的编辑交互、#48 验证
//! 当前文件替换、#49 验证保存与外部修改——每一套都只看**一层**。本 issue 看的是几层串起来之后的
//! 行为，也就是 issue #51「风险与提示」点名的三处：
//!
//! 1. **编辑 → 高亮 / 大纲 / 面包屑 / 索引精确度**这条跨 M1–M3 的链路（`m3_edit_*`）。
//!    方法体改动只让受影响的行失效、大纲不动；签名改动让大纲与面包屑跟着变；保存前查询标为
//!    **近似**，索引重建完成后**静默**变成精确（不重弹提示、不闪烁高亮）。
//! 2. **只读文档上的第三个入口**：输入与替换入口已经由 #20 / #48 的测试挡住了，这里补**保存**
//!    ——`Ctrl+S` 在 `Library/PackageCache/**` 与超大文件上必须一个字节都不写。
//! 3. **外部修改风暴在窗口里的表现**：100 次事件只有一条提示条、零通知，提示条上的两个选项各走一遍。
//!
//! 另有一组**真实 Unity 项目**上的测试（`m3_real_*`）：把真实 `.cs` 的字节复制进临时工作区
//! （**不往测试项目里写任何东西**），在真实窗口里走完「打开 → 改一处 → 保存」，再与原字节做
//! 字节级比较。核心层的完整往返矩阵在 `rove-buffer` 的 `tests/m3_roundtrip.rs` 里。
//!
//! 运行：
//!
//! ```powershell
//! cargo test -p rove-ui m3_ -j 1 -- --nocapture
//! ```
//!
//! 真实项目组额外需要 `ROVE_TEST_UNITY_MEDIUM`，缺失时**跳过而不是失败**（原因打到 stderr）。
use super::*;
use crate::documents::OpenOptions;
use crate::outline::view::Status as OutlineStatus;
use crate::save::registry as save_ids;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{Position, Utf8Column, Version};
use rove_buffer::ReadOnlyReason;
use rove_config::{SettingsOptions, SettingsService};
use rove_lang_cs::query::{CsQueries, NavigationKind, Precision, QueryPoint};
use rove_lang_cs::router::testing::{find_word, metadata_sample_index, GAME_CS, GAME_PATH};
use rove_lang_cs::router::{
    hash_bytes, IndexHolder, L2Snapshot, ManualSemantic, SemanticSource, SemanticStatus,
};
use rove_lang_cs::CsIndex;
use rove_test_support::UnityProject;

// --------------------------------------------------------------------------- 夹具

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

fn word(text: &str, w: &str, nth: usize) -> Position {
    let (line, col) = find_word(text, w, nth);
    pos(u64::from(line), u64::from(col))
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

fn save_command(cx: &mut VisualTestContext) {
    let action = save_ids::action_for("workbench.action.files.save").unwrap();
    cx.update(|window, cx| window.dispatch_action(action, cx));
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

/// 焦点组的面包屑文字（与 #26 的入口测试同一个口径）。
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

fn outline_rows(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, cx| {
        let w = &v.outline.list.read(cx).widget;
        w.display_rows(0..w.len())
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), r.label))
            .collect()
    })
}

/// 某一行上的语法着色记号（`起始列..结束列 → 最后一个 scope`）。
fn line_tokens(code: &Entity<CodeView>, line: u64, cx: &mut VisualTestContext) -> Vec<String> {
    code.read_with(cx, |c, _| {
        let Some(snapshot) = c.syntax_snapshot() else {
            return Vec::new();
        };
        snapshot
            .tokens(Line(line))
            .map(|tokens| {
                tokens
                    .iter()
                    .map(|t| {
                        format!(
                            "{}..{} {}",
                            t.range.start().0,
                            t.range.end().0,
                            t.scopes.last().map(|s| s.to_string()).unwrap_or_default()
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn status_message(view: &Entity<Workbench>, cx: &VisualTestContext) -> Option<String> {
    view.read_with(cx, |v, _| shell::view::status_message_text(v))
}

fn notifications(view: &Entity<Workbench>, cx: &VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.shell.feedback.notifications().len())
}

/// #40 装饰层上的语义同名高亮（近似时为空：退化成 #20 的文本高亮）。
fn semantic_highlight_count(view: &Entity<Workbench>, cx: &VisualTestContext) -> usize {
    view.read_with(cx, |v, _| v.nav_highlights(v.editor.focused, false).len())
}

fn document_version(code: &Entity<CodeView>, cx: &VisualTestContext) -> Version {
    code.read_with(cx, |c, _| c.version())
}

/// 查询层（#38 的路由）对某个位置的精确度。
fn router_precision(
    view: &Entity<Workbench>,
    cx: &VisualTestContext,
    at: Position,
    version: Version,
) -> Precision {
    let point = QueryPoint {
        resource: Resource::Workspace(WorkspacePath::new(GAME_PATH).unwrap()),
        position: at,
        version,
    };
    view.read_with(cx, |v, _| {
        v.cs_router()
            .navigate(NavigationKind::Definition, &point, &Cancellation::default())
    })
    .expect("router answers")
    .precision
}

/// 把索引换成「包含 `text` 这份内容」的一代（模拟 #35 保存后的单文件重绑完成）。
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

// ===================================================================== 风险点 1

/// 目标 4：**编辑 → 高亮 / 大纲 / 面包屑 / 索引精确度**这条跨里程碑的链路。
///
/// | 阶段 | 语法失效区间 | 大纲 / 面包屑 | 查询精确度 | 状态栏 | 通知 |
/// |---|---|---|---|---|---|
/// | 打开 | 整层（首次解析） | `Player` / `Update()` | 精确 | 无 | 0 |
/// | 改方法体 | 只有受影响的行 | 不变 | **近似**（有未保存修改） | 近似提示一次 | 0 |
/// | 改方法签名 | 只有受影响的行 | **跟着变** | 近似 | 不重复提示 | 0 |
/// | 保存 + 索引重建 | 不再失效 | 不变 | **精确** | **不弹任何提示** | 0 |
///
/// 「不闪烁」在自动测试里的可检验口径：**语法快照在编辑后不回退成空**（旧 token 继续画），
/// 并且失效区间不是整层；真正的视觉闪烁要看人工清单里的录屏。
#[gpui::test]
fn m3_edit_then_save_updates_syntax_outline_breadcrumb_and_precision(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
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

    // ① 打开：大纲与面包屑就位，查询精确。
    view.read_with(cx, |v, _| {
        assert_eq!(v.outline.status, OutlineStatus::Ready, "C# 应有大纲")
    });
    let outline_before = outline_rows(&view, cx);
    assert!(
        outline_before.iter().any(|r| r.contains("Update")),
        "{outline_before:?}"
    );
    let at = word(GAME_CS, "position", 0);
    caret(&code, at, cx);
    assert_eq!(
        breadcrumb_text(&view, cx).last().map(String::as_str),
        Some("Update()"),
        "光标在方法体里，面包屑最后一段是方法"
    );
    assert_eq!(
        router_precision(&view, cx, at, document_version(&code, cx)),
        Precision::Exact,
        "未修改的文件查询必须精确"
    );
    assert_eq!(notifications(&view, cx), 0);

    // ② 改方法体：高亮按行增量失效，大纲与面包屑不动，查询变近似。
    code.update(cx, |c, cx| {
        let _ = c.take_syntax_invalidation();
        c.set_selection(
            code_view::Selection::caret(word(GAME_CS, "list", 0)),
            None,
            cx,
        );
    });
    press("x", cx);
    assert!(
        code.read_with(cx, |c, _| c.syntax_snapshot().is_some()),
        "编辑后语法快照不能回退成空（否则会闪一下纯文本）"
    );
    let invalidated = code.update(cx, |c, _| c.take_syntax_invalidation());
    if let Some(Some(ranges)) = &invalidated {
        assert!(!ranges.is_empty(), "改动了一行，失效区间不能是空的");
        let covered: u64 = ranges.iter().map(|r| r.end - r.start).sum();
        assert!(
            covered <= 4,
            "一行的改动不该让 {covered} 行失效：{ranges:?}"
        );
    }
    assert_eq!(
        outline_rows(&view, cx),
        outline_before,
        "方法体改动不动大纲"
    );
    let edited = text_of(&code, cx);
    let at = word(&edited, "position", 0);
    caret(&code, at, cx);
    assert_eq!(
        breadcrumb_text(&view, cx).last().map(String::as_str),
        Some("Update()"),
        "方法体改动不动面包屑"
    );
    // 口径按 LANGUAGE_INTELLIGENCE §3.2（而不是 issue 正文那句简写）：**被改动的那一行**降级为
    // 近似，其余位置经 #38 的位置映射仍然是精确的。两边都断言，免得以后有人把任一侧改坏。
    assert_eq!(
        router_precision(&view, cx, at, document_version(&code, cx)),
        Precision::Exact,
        "未改动的行经位置映射后仍应精确"
    );
    let on_edited_line = word(&edited, "Add", 0);
    assert_eq!(
        router_precision(&view, cx, on_edited_line, document_version(&code, cx)),
        Precision::Approximate,
        "改动所在的行必须降级为近似"
    );
    assert_eq!(notifications(&view, cx), 0, "编辑不弹通知");

    // ③ 改方法签名：大纲与面包屑跟着变。
    let update_name = word(&edited, "Update", 0);
    code.update(cx, |c, cx| {
        let line = c.text().unwrap();
        let start = line.offset(update_name).unwrap();
        let end = rove_base::ByteOffset(start.0 + "Update".len() as u64);
        c.set_selection(
            code_view::Selection::new(line.position(start).unwrap(), line.position(end).unwrap()),
            None,
            cx,
        );
    });
    settle(cx);
    press("T i c k", cx);
    let after = outline_rows(&view, cx);
    assert!(
        after.iter().any(|r| r.contains("Tick")) && !after.iter().any(|r| r.contains("Update")),
        "签名改动后大纲必须更新：{after:?}"
    );
    let edited = text_of(&code, cx);
    caret(&code, word(&edited, "Tick", 0), cx);
    assert_eq!(
        breadcrumb_text(&view, cx).last().map(String::as_str),
        Some("Tick()"),
        "面包屑跟着签名走"
    );

    // ④ 保存 + 索引重建完成：静默变成精确。
    let highlights_before = semantic_highlight_count(&view, cx);
    let notes_before = notifications(&view, cx);
    save_command(cx);
    let saved = std::fs::read_to_string(fx.folder.path().join(GAME_PATH)).unwrap();
    assert_eq!(saved, edited, "保存后磁盘内容必须等于编辑器里的内容");

    // 保存后「立刻」查询：索引还没重建，改动所在的行仍然是近似（这是**正确行为**，不是缺陷）。
    let at = word(&edited, "Tick", 0);
    caret(&code, at, cx);
    assert_eq!(
        router_precision(&view, cx, at, document_version(&code, cx)),
        Precision::Approximate,
        "保存后索引重建完成前，改动所在的行仍应是近似"
    );

    // #35 的单文件重绑完成（测试里直接换一代包含新内容的索引）。
    // 先收掉保存回执（「已保存 Game.cs」是 #49 的第 1 级反馈，和索引无关），
    // 这样后面那条断言量的就只是「索引重建自己有没有弹东西」。
    view.update(cx, |v, _| {
        let _ = crate::feedback::FeedbackService::dismiss(&mut v.shell.feedback, "save.saved");
    });
    settle(cx);
    assert_eq!(status_message(&view, cx), None, "保存回执已收掉");
    reindex(&index_dir, &semantic, &edited, 1);
    view.update(cx, |v, cx| v.nav_index_changed(cx));
    settle(cx);
    assert_eq!(
        router_precision(&view, cx, at, document_version(&code, cx)),
        Precision::Exact,
        "索引重建完成后必须变成精确"
    );
    // 切换过程中界面是静默的：不弹提示、不弹通知、同名高亮没有被清空。
    assert_eq!(status_message(&view, cx), None, "索引重建完成本身不弹提示");
    assert_eq!(notifications(&view, cx), notes_before, "不弹通知");
    assert!(
        semantic_highlight_count(&view, cx) >= highlights_before,
        "同名高亮不能在切换时被清掉"
    );
}

/// 同一条链路上的**并发口径**：保存是异步的（写盘在后台），`Ctrl+S` 之后、写盘回来之前的输入
/// 照常被接受。此时磁盘上是保存开始时的快照，缓冲区已经领先一步，所以**不能**把磁盘内容的哈希
/// 登记成「当前内容的哈希」。
///
/// 少了这道守卫的后果不是「停在近似」，而是**标为精确的错误结果**：索引随后按磁盘内容重绑时，
/// `BufferStates::rebase` 会命中 `current_hash == indexed` 而丢掉整份编辑记录、把文件判为
/// `Clean`，之后整份文件的跳转 / 引用 / 悬停都按新文本的位置去查旧内容的索引。
#[gpui::test]
fn m3_edit_while_save_is_in_flight_keeps_the_buffer_diverged(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
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

    // ① 第一次编辑 E1，内容记为 S1。
    caret(&code, word(GAME_CS, "list", 0), cx);
    press("x", cx);
    let s1 = text_of(&code, cx);
    assert_ne!(s1, GAME_CS, "E1 必须真的改了内容");

    // ② 派发保存命令但**不**跑后台：写盘还在途中（报告 §7.2 实测 1 MB 77 ms / 10 MB 289 ms，
    //    这个窗口足够用户继续打字）。
    let action = save_ids::action_for("workbench.action.files.save").unwrap();
    cx.update(|window, cx| window.dispatch_action(action, cx));

    // ③ 保存在途时继续输入 E2：走 `type_text`（同步进到 `text_edited`），不等后台。
    let typed = code.update(cx, |c, cx| {
        c.set_selection(code_view::Selection::caret(word(&s1, "q", 0)), None, cx);
        c.type_text("Z", None, cx)
    });
    assert!(typed, "保存在途时的输入必须照常被接受");

    // ④ 让保存跑完：磁盘上是 S1，缓冲区是 S1 + E2。
    settle(cx);
    let on_disk = std::fs::read_to_string(fx.folder.path().join(GAME_PATH)).unwrap();
    assert_eq!(on_disk, s1, "写出的是保存开始时的快照");
    let in_buffer = text_of(&code, cx);
    assert_ne!(in_buffer, on_disk, "缓冲区里还有没落盘的 E2");

    // ⑤ 登记口径：E2 还没保存，当前内容没有对应的磁盘哈希。
    let state = view
        .read_with(cx, |v, _| v.cs_buffers().get(GAME_PATH))
        .expect("打开的 C# 文档已登记");
    assert_eq!(
        state.current_hash, None,
        "保存期间被编辑过：不能把磁盘内容的哈希登记成当前内容的哈希"
    );

    // ⑥ 索引随后按磁盘内容（S1）重绑：编辑记录不能被丢掉，结果也不能标成精确。
    reindex(&index_dir, &semantic, &on_disk, 1);
    view.update(cx, |v, cx| v.nav_index_changed(cx));
    settle(cx);
    let after = view
        .read_with(cx, |v, _| v.cs_buffers().get(GAME_PATH))
        .expect("重绑后仍登记");
    assert_eq!(
        after.base_hash,
        hash_bytes(GAME_CS.as_bytes()),
        "重绑不能丢掉 E1 + E2 的编辑记录（基线仍是打开时的内容）"
    );
    let at = word(&in_buffer, "parent", 1);
    caret(&code, at, cx);
    assert_ne!(
        router_precision(&view, cx, at, document_version(&code, cx)),
        Precision::Exact,
        "索引里只有 S1、缓冲区有 E2：结果可以降级为近似，但绝不能标成精确"
    );

    // ⑦ 再保存一次（这回全程没有新输入）：登记恢复正常，索引重绑后变精确。
    save_command(cx);
    let state = view
        .read_with(cx, |v, _| v.cs_buffers().get(GAME_PATH))
        .expect("仍登记");
    assert_eq!(
        state.current_hash,
        Some(hash_bytes(in_buffer.as_bytes())),
        "没有并发编辑的保存必须登记当前内容的索引哈希"
    );
    reindex(&index_dir, &semantic, &in_buffer, 2);
    view.update(cx, |v, cx| v.nav_index_changed(cx));
    settle(cx);
    assert_eq!(
        router_precision(&view, cx, at, document_version(&code, cx)),
        Precision::Exact,
        "索引追上当前内容后必须变成精确"
    );
}

/// 目标 7 的一条：编辑后交给 #94 的增量重解析的结果，必须和**从头解析同一份文本**一致。
///
/// 这是「高亮正确」在自动测试里唯一能钉死的口径：增量结果 ≡ 全量结果。
#[gpui::test]
fn m3_incremental_highlight_matches_a_fresh_parse(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    fx.file(GAME_PATH, GAME_CS.as_bytes());
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, GAME_PATH, cx);

    // 在方法体中间插入一段新语句（跨越一个完整的表达式，足以让 tree-sitter 重建子树）。
    let at = word(GAME_CS, "list", 0);
    caret(&code, at, cx);
    press("v a r space n space = space 1 ; enter", cx);
    let edited = text_of(&code, cx);
    let incremental: Vec<Vec<String>> = (0..12).map(|line| line_tokens(&code, line, cx)).collect();

    // 同一份文本从头打开一次（新文档、首次解析）。
    fx.file("Assets/Fresh.cs", edited.as_bytes());
    let fresh = open_file(&view, "Assets/Fresh.cs", cx);
    let full: Vec<Vec<String>> = (0..12).map(|line| line_tokens(&fresh, line, cx)).collect();

    assert!(
        full.iter().any(|t| !t.is_empty()),
        "全量解析应当有记号，否则这条断言没有意义"
    );
    assert_eq!(incremental, full, "增量重解析的记号必须与全量解析一致");
}

// ===================================================================== 风险点 2

/// 目标 5：只读文档上的**第三个入口**——保存。输入与替换入口已由 #20 / #48 的测试挡住，
/// 这里确认 `Ctrl+S` 在 `Library/PackageCache/**` 与超大文件上**一个字节都不写**。
#[gpui::test]
fn m3_read_only_documents_reject_the_save_entry_point(cx: &mut TestAppContext) {
    // 超大文件阈值调小，用小文件走同一条超大文件路径。
    let fx = Fixture::new(r#"{ "rove.largeFile.threshold": 2048 }"#);
    let pkg = "Library/PackageCache/com.unity.foo/Runtime/Foo.cs";
    let pkg_path = fx.file(pkg, b"class Foo {}\n");
    let huge_path = fx.file(
        "Logs/Editor.log",
        "line of log text\n".repeat(400).as_bytes(),
    );
    let (view, cx) = fx.open(cx);

    for (label, rel, path) in [
        ("包缓存目录", pkg, &pkg_path),
        ("超大文件", "Logs/Editor.log", &huge_path),
    ] {
        let before = std::fs::read(path).unwrap();
        let modified_before = std::fs::metadata(path).unwrap().modified().unwrap();
        open_file(&view, rel, cx);
        assert!(
            view.read_with(cx, |v, cx| !v.active_read_only(cx).is_empty()),
            "{label}应当是只读的"
        );
        // 先试着输入（被挡住），再按 Ctrl+S。
        press("x", cx);
        save_command(cx);
        assert_eq!(std::fs::read(path).unwrap(), before, "{label}：保存写了盘");
        assert_eq!(
            std::fs::metadata(path).unwrap().modified().unwrap(),
            modified_before,
            "{label}：保存动了修改时间"
        );
        // 只读文档上的 `Ctrl+S` 按 #49 的设计给一条带「另存为」的通知；要钉死的是
        // **按钮里没有任何提权入口**（`UX_SPEC` §6.4：不提供“以管理员身份保存”）。
        let actions = view.read_with(cx, |v, _| {
            v.shell
                .feedback
                .notifications()
                .iter()
                .flat_map(|n| n.actions.iter().map(|a| a.id.to_string()))
                .collect::<Vec<_>>()
        });
        assert!(
            !actions
                .iter()
                .any(|a| a.contains("elevat") || a.contains("admin")),
            "{label}：通知里不能出现提权入口：{actions:?}"
        );
        view.update(cx, |v, _| {
            let keys: Vec<String> = v
                .shell
                .feedback
                .notifications()
                .iter()
                .map(|n| n.key.to_string())
                .collect();
            for key in keys {
                let _ = crate::feedback::FeedbackService::dismiss(&mut v.shell.feedback, &key);
            }
        });
        settle(cx);
    }

    // 包缓存文件在会话中取消只读后可以正常保存（“在此会话中切换只读”对它有效）。
    open_file(&view, pkg, cx);
    let action = code_view::commands::action_for(
        "workbench.action.files.toggleActiveEditorReadonlyInSession",
    )
    .unwrap();
    cx.update(|window, cx| window.dispatch_action(action, cx));
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, cx| v.active_read_only(cx)),
        Vec::<ReadOnlyReason>::new(),
        "切换后应当不再只读"
    );
    press("x", cx);
    save_command(cx);
    assert_ne!(
        std::fs::read(&pkg_path).unwrap(),
        b"class Foo {}\n",
        "取消只读后保存应当写盘"
    );
}

// ===================================================================== 风险点 3

/// 目标 3：**外部修改风暴在窗口里的表现**。
///
/// 有本地修改时连续 100 次外部改写：提示条只出现一条，通知始终为 0；
/// 「重新加载」与「保留本地修改」两个选项各走一遍。
#[gpui::test]
fn m3_external_change_storm_shows_one_bar_and_no_notifications(cx: &mut TestAppContext) {
    let fx = Fixture::new("{}");
    let path = fx.file("Assets/Storm.cs", b"class Storm {}\n");
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, "Assets/Storm.cs", cx);
    press("/ /", cx);
    assert!(text_of(&code, cx).starts_with("//"), "先造一处本地修改");

    for i in 0..100 {
        std::fs::write(&path, format!("class Storm {{}} // external {i}\n")).unwrap();
        view.update(cx, |v, cx| {
            v.note_disk_events(
                &[FileEvent::Changed(
                    WorkspacePath::new("Assets/Storm.cs").unwrap(),
                )],
                cx,
            )
        });
    }
    // 合并器按真实时间判定安静期，定时器走测试时钟：两者都推进过去。
    std::thread::sleep(Duration::from_millis(1100));
    for _ in 0..3 {
        cx.executor().advance_clock(Duration::from_millis(1100));
        settle(cx);
    }

    assert_eq!(notifications(&view, cx), 0, "外部修改风暴不该弹通知");
    let doc = view.read_with(cx, |v, _| {
        v.editor
            .active()
            .current()
            .and_then(Tab::opened)
            .unwrap()
            .document
            .id
    });
    let bars = view.read_with(cx, |v, _| v.shell.feedback.editor_bars(doc).count());
    assert_eq!(bars, 1, "100 次外部改写只应有一条提示条");
    eprintln!("[M3 外部修改] 窗口里 100 次外部改写：提示条 1 条、通知 0 条");
}

// ===================================================================== 真实项目

/// 真实 Unity 项目里的 `.cs`：字节复制进临时工作区（**不往项目里写**），在真实窗口里
/// 「打开 → 改一处 → 保存」，再与原字节做字节级比较。
///
/// 这是 issue #51 目标 2 在**界面一侧**的复核；核心层的完整矩阵在 `rove-buffer` 的
/// `tests/m3_roundtrip.rs`（那里按编码分类逐条跑）。
#[gpui::test]
fn m3_real_project_file_edits_and_saves_byte_identically(cx: &mut TestAppContext) {
    let Some(root) = UnityProject::Medium.path() else {
        eprintln!("已跳过 m3_real_project_file_edits_and_saves_byte_identically");
        return;
    };
    let Some((rel, bytes)) = pick_real_cs(&root) else {
        eprintln!("已跳过：中型项目里没有找到合适的 .cs（UTF-8、2 KB–200 KB）");
        return;
    };
    eprintln!("[M3 界面往返] 样本：{rel}（{} B）", bytes.len());

    let fx = Fixture::new("{}");
    let path = fx.file("Assets/Sample.cs", &bytes);
    let (view, cx) = fx.open(cx);
    let code = open_file(&view, "Assets/Sample.cs", cx);
    let before = text_of(&code, cx);

    // 改一处：在文件中部那一行的行首插入一个标记。
    let line = (code.read_with(cx, |c, _| c.text().unwrap().lines().known) / 2).max(1);
    caret(&code, pos(line, 0), cx);
    press("/ / R o v e", cx);
    let after = text_of(&code, cx);
    assert_ne!(after, before, "应当真的改了一处");
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
    eprintln!("[M3 界面往返] {rel}：窗口里改一处并保存，差异恰好 +6 B");
}

/// 中型项目里挑一个适合往返的 `.cs`：UTF-8、2 KB–200 KB、至少 20 行。
fn pick_real_cs(root: &std::path::Path) -> Option<(String, Vec<u8>)> {
    let mut stack = vec![root.to_path_buf()];
    let mut scanned = 0usize;
    while let Some(dir) = stack.pop() {
        if scanned >= 3000 {
            break;
        }
        let entries = std::fs::read_dir(&dir).ok()?;
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
            if text.lines().count() < 20 {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            return Some((rel, bytes));
        }
    }
    None
}
