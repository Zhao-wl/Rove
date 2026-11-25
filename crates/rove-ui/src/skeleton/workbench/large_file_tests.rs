//! #22 入口集成测试（真实 GPUI 测试窗口）：
//! 超大文件打开 → 提示条与状态栏 → 索引进度 → 转到行等待 → 流式查找 → 超长行截断与临时标签。
//!
//! 测试把 `rove.largeFile.threshold` 调小（几十 KB），用小文件走完**同一条**超大文件路径；
//! 1 GB / 50 MB 的真实数据由 `cargo xtask bench` 的测试数据在人工验收时复跑（见 PR）。
use super::*;
use crate::large_file::{goto, long_line, notice, progress::IndexProgress, registry as lf};
use crate::status_items::registry::READ_ONLY;
// #27 删掉了骨架的固定状态栏内容，`workbench.rs` 不再导入 `StatusEntry`。
use crate::shell::status::StatusEntry;
use gpui::{TestAppContext, VisualTestContext};
use rove_base::{ByteOffset, ByteRange, Span};
use rove_config::{SettingsOptions, SettingsService};

/// 阈值调到 4 KB：下面的文件都按“超大文件”处理。
const SETTINGS: &str =
    r#"{ "rove.largeFile.threshold": 4096, "editor.stopRenderingLineAfter": 40 }"#;

struct Fixture {
    folder: tempfile::TempDir,
    service: Arc<SettingsService>,
    _user: tempfile::TempDir,
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
        // 约 90 KB、6000 行的“日志”文件：超过 4 KB 阈值 → 超大文件、只读、内存映射。
        let big: String = (0..6000).map(|i| format!("line {i:05} needle\n")).collect();
        std::fs::write(folder.path().join("big.log"), &big).unwrap();
        // 单行压缩 JSON：整份文件只有一行。
        let json = format!("{{\"a\":\"{}\"}}", "x".repeat(20_000));
        std::fs::write(folder.path().join("min.json"), &json).unwrap();
        std::fs::write(folder.path().join("small.txt"), "a\nb\n").unwrap();
        Self {
            folder,
            service,
            _user: user,
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
}

fn refresh(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        refresh(cx);
    }
}

fn open_file(view: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) {
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
    refresh(cx);
}

fn document(view: &Entity<Workbench>, cx: &VisualTestContext) -> Arc<Document> {
    view.read_with(cx, |v, _| v.active_large_file_document().unwrap())
}

fn status(view: &Entity<Workbench>, id: &str, cx: &VisualTestContext) -> Option<StatusEntry> {
    view.read_with(cx, |v, _| v.shell.status_bar.get(id).cloned())
}

/// 打开超大文件：没有任何通知，只有提示条与状态栏（UX_SPEC §6.1）。
#[gpui::test]
fn opening_a_huge_file_shows_a_bar_and_status_but_never_a_notification(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);

    let document = document(&view, cx);
    assert_eq!(document.metadata.size, rove_buffer::SizeClass::Huge);
    assert!(document
        .metadata
        .read_only
        .contains(&rove_buffer::ReadOnlyReason::HugeFile));

    // 提示条：文案与 §3.2 一致、可关闭、可“不再提示”。
    let bars = view.read_with(cx, |v, _| v.shell.feedback.all_editor_bars().to_vec());
    assert_eq!(bars.len(), 1, "只有一条提示条");
    let bar = &bars[0];
    assert!(bar.dismissible);
    assert_eq!(bar.suppress_key.as_deref(), Some(notice::SUPPRESS_KEY));
    let text = view.read_with(cx, |v, _| v.strings.text(&bar.message).unwrap().to_string());
    assert!(
        text.starts_with("超大文件（") && text.contains("只读"),
        "{text}"
    );
    assert!(text.contains("KB"), "体积按实际大小格式化：{text}");

    // 全程没有通知、没有模态框。
    view.read_with(cx, |v, _| {
        assert!(v.shell.feedback.notifications().is_empty());
        assert!(v.shell.feedback.modal().is_none());
    });

    // 状态栏：超大文件 · 只读，悬停列出被关闭的功能。
    let entry = status(&view, READ_ONLY, cx).expect("只读标记条目");
    assert_eq!(entry.text, "超大文件 · 只读");
    let tooltip = entry.tooltip.unwrap();
    for feature in ["语法高亮", "折叠", "缩略图"] {
        assert!(tooltip.contains(feature), "悬停缺少 {feature}：{tooltip}");
    }

    // 普通文件没有提示条、也没有大小标记。
    open_file(&view, "small.txt", cx);
    assert!(status(&view, READ_ONLY, cx).is_none());
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.shell.feedback.all_editor_bars().len(),
            1,
            "提示条按文档保留"
        );
        assert!(v.shell.feedback.notifications().is_empty());
    });
}

/// “不再提示”之后，再开别的超大文件不再出现提示条（去重键一致）。
#[gpui::test]
fn never_show_again_suppresses_later_huge_files(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);
    let key = view.read_with(cx, |v, _| v.shell.feedback.all_editor_bars()[0].key.clone());
    view.update(cx, |v, _| {
        v.shell.feedback.never_show_again(&key).unwrap();
    });
    settle(cx);
    view.read_with(cx, |v, _| {
        assert!(v.shell.feedback.all_editor_bars().is_empty());
        assert!(v
            .shell
            .feedback
            .suppressed()
            .any(|k| &**k == notice::SUPPRESS_KEY));
    });
    open_file(&view, "min.json", cx);
    view.read_with(cx, |v, _| {
        assert!(
            v.shell.feedback.all_editor_bars().is_empty(),
            "“不再提示”后不再出现"
        );
        assert!(v.shell.feedback.notifications().is_empty());
    });
}

/// 行索引：进度条目在索引期间显示、建完后消失；滚动估计永远不小于已知行数。
#[gpui::test]
fn line_index_progress_appears_only_while_indexing(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);
    settle(cx);

    let document = document(&view, cx);
    let count = document.text.lines();
    // 这个文件不大，索引一般已经建完：条目应当不存在。
    if count.complete {
        assert!(
            status(&view, lf::INDEX_STATUS, cx).is_none(),
            "建完后条目消失"
        );
    }
    let progress = IndexProgress::of(&count);
    assert!(progress.estimated_lines() >= count.known.max(1));
    let estimate = view.read_with(cx, |v, cx| v.code_view().read(cx).scroll_line_estimate());
    if count.complete {
        // 索引建完时估计值就是真实行数：不再下发（继续下发会把折叠隐藏的行算回内容高度，
        // 见 `tests/code_view_features.rs` 的 `folding_shrinks_the_scroll_range_...`）。
        assert_eq!(progress.estimated_lines(), count.known);
        assert_eq!(estimate, None, "索引建完后不再下发估计值");
    } else {
        // 索引未建完：滚动条用的是估计值，不是“已知行数”。
        assert_eq!(estimate, Some(progress.estimated_lines()));
    }
}

/// 转到行：已索引的行立即生效；未索引的行进入等待、可取消。
#[gpui::test]
fn goto_line_waits_for_the_index_and_can_be_cancelled(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);
    settle(cx);
    let document = document(&view, cx);

    // 已索引：立刻可跳，不进入等待。
    let immediate = view.update(cx, |v, cx| v.goto_line_or_wait(&document, 3, cx));
    assert!(immediate);
    assert!(view.read_with(cx, |v, _| v.pending_goto().is_none()));

    // 构造一个“索引未建完”的文本源，验证等待与取消。
    let waiting = Arc::new(Pending::new(100));
    let pending_doc = Arc::new(Document {
        id: DocumentId(99_999),
        resource: Resource::Virtual("pending".into()),
        metadata: document.metadata.clone(),
        text: waiting.clone(),
    });
    let immediate = view.update(cx, |v, cx| v.goto_line_or_wait(&pending_doc, 90, cx));
    assert!(!immediate, "未索引的行要等待");
    let (line, _) = view.read_with(cx, |v, _| v.pending_goto().expect("等待中"));
    assert_eq!(line, 90);
    view.update(cx, |v, _| v.cancel_pending_goto());
    assert!(
        view.read_with(cx, |v, _| v.pending_goto().is_none()),
        "可取消"
    );
    settle(cx);

    // 取消后不会再跳：解析立即可得的结论仍然正确。
    assert_eq!(
        goto::resolve_now(&*waiting, 1),
        Some(goto::WaitOutcome::Ready(Line(0)))
    );
    assert_eq!(goto::resolve_now(&*waiting, 90), None);
}

/// #232：旧请求的回调不得清掉新请求。用可控的回调顺序代替真实延迟：
/// 先等 A、再切到 B，A 的回调先到——界面继续显示 B 的等待，B 就绪后跳到 B；
/// 取消后立刻重发同一行也是独立的请求，旧任务既不提前跳转、也不清除新任务。
#[gpui::test]
fn a_stale_goto_callback_neither_jumps_nor_clears_the_newer_request(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);
    settle(cx);
    let document = document(&view, cx);
    // 同一个文档（编号一致，跳转能找到它），但行索引停在第 10 行。
    let pending_doc = Arc::new(Document {
        text: Arc::new(Pending::new(100)),
        ..(*document).clone()
    });
    let line_of_caret = |view: &Entity<Workbench>, cx: &mut VisualTestContext| {
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        code.read_with(cx, |c, _| c.selection().active.line.0)
    };
    let start = line_of_caret(&view, cx);

    // 测试里的回调由手动调用送达；后台的真实等待永远等不到第 10 行以后，
    // 所以在 `refresh`（会跑后台任务）之前一律先取消它，否则要轮询到超时。
    // A（第 50 行）等待中，切到 B（第 80 行）。
    assert!(!view.update(cx, |v, cx| v.goto_line_or_wait(&pending_doc, 50, cx)));
    let a = view.read_with(cx, |v, _| v.pending_goto_request().unwrap());
    assert!(!view.update(cx, |v, cx| v.goto_line_or_wait(&pending_doc, 80, cx)));
    let b = view.read_with(cx, |v, _| v.pending_goto_request().unwrap());
    assert_ne!(a.id, b.id);
    assert!(a.is_cancelled() && !b.is_cancelled());
    // A 的回调先到：它是已取消的旧请求，不跳转、不动 B 的等待。
    view.update(cx, |v, cx| {
        v.finish_pending_goto(a.id, goto::WaitOutcome::Ready(Line(49)), cx)
    });
    assert_eq!(
        view.read_with(cx, |v, _| v.pending_goto().map(|p| p.0)),
        Some(80)
    );
    assert_eq!(line_of_caret(&view, cx), start, "旧请求没有跳转");
    // B 就绪：跳到 B，等待结束。
    view.update(cx, |v, cx| {
        v.finish_pending_goto(b.id, goto::WaitOutcome::Ready(Line(79)), cx)
    });
    b.cancel();
    refresh(cx);
    assert!(view.read_with(cx, |v, _| v.pending_goto().is_none()));
    assert_eq!(line_of_caret(&view, cx), 79);

    // 取消后立即重发同一行：两次请求各自独立。
    assert!(!view.update(cx, |v, cx| v.goto_line_or_wait(&pending_doc, 60, cx)));
    let old = view.read_with(cx, |v, _| v.pending_goto_request().unwrap());
    view.update(cx, |v, _| v.cancel_pending_goto());
    assert!(!view.update(cx, |v, cx| v.goto_line_or_wait(&pending_doc, 60, cx)));
    let new = view.read_with(cx, |v, _| v.pending_goto_request().unwrap());
    assert_ne!(old.id, new.id);
    view.update(cx, |v, cx| {
        v.finish_pending_goto(old.id, goto::WaitOutcome::Ready(Line(59)), cx)
    });
    assert_eq!(
        view.read_with(cx, |v, _| v.pending_goto().map(|p| p.0)),
        Some(60)
    );
    assert_eq!(line_of_caret(&view, cx), 79, "旧任务没有提前跳转");
    view.update(cx, |v, cx| {
        v.finish_pending_goto(new.id, goto::WaitOutcome::Ready(Line(59)), cx)
    });
    new.cancel();
    refresh(cx);
    assert!(view.read_with(cx, |v, _| v.pending_goto().is_none()));
    assert_eq!(line_of_caret(&view, cx), 59);
}

/// 等待期间界面上看得见、也取消得掉（UX_SPEC §6.2）：快速输入框保持打开、显示进度，
/// `quickInput.hide`（Esc 的命令）取消等待。
#[gpui::test]
fn the_goto_line_wait_is_visible_and_cancellable(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);
    settle(cx);

    // 索引未建完的文档上转到未索引的行：进入等待。
    let document = document(&view, cx);
    let waiting = Arc::new(Pending::new(100));
    let pending_doc = Arc::new(Document {
        id: DocumentId(99_999),
        resource: Resource::Virtual("pending".into()),
        metadata: document.metadata.clone(),
        text: waiting.clone(),
    });
    assert!(!view.update(cx, |v, cx| v.goto_line_or_wait(&pending_doc, 90, cx)));

    // 下一帧：快速输入框（回车时已经关掉了）重新打开，里面是等待提示 + 进度 + 取消办法。
    // 这里直接驱动每帧的装配，不跑后台执行器——等待任务一跑就会把 600 s 的轮询走完。
    cx.update(|window, cx| view.update(cx, |v, cx| v.sync_quick_access(window, cx)));
    let percent = view.read_with(cx, |v, _| v.pending_goto().expect("等待中").1);
    let rows: Vec<(String, String)> = view.read_with(cx, |v, cx| {
        let q = v.quick.view.as_ref().expect("浮层已创建").read(cx);
        let a = q.service.active().expect("等待期间浮层开着");
        a.list
            .display_rows(0..a.list.len())
            .iter()
            .map(|r| {
                (
                    r.label.to_string(),
                    r.detail.as_deref().unwrap_or("").to_owned(),
                )
            })
            .collect()
    });
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].0.contains("90"), "{rows:?}");
    assert!(rows[0].0.contains("正在建立行索引"), "{rows:?}");
    assert!(
        rows[0].0.contains(&percent.to_string()),
        "提示里有进度：{rows:?}"
    );
    assert!(rows[0].1.contains("取消"), "有取消办法：{rows:?}");

    // Esc：`quickInput.hide` 关掉浮层 → 取消等待（`ARCHITECTURE` §39）。
    let quick = view.read_with(cx, |v, _| v.quick.view.clone().unwrap());
    cx.update(|window, cx| quick.update(cx, |q, cx| q.run_command("quickInput.hide", window, cx)));
    assert!(
        view.read_with(cx, |v, _| v.pending_goto().is_none()),
        "Esc 取消了等待"
    );
    cx.update(|window, cx| view.update(cx, |v, cx| v.sync_quick_access(window, cx)));
    assert!(
        view.read_with(cx, |v, cx| v.quick.view.as_ref().is_none_or(|q| q
            .read(cx)
            .service
            .active()
            .is_none())),
        "浮层已关闭"
    );
    settle(cx);
    assert!(view.read_with(cx, |v, _| v.pending_goto().is_none()));
}

/// 流式查找：超大文件上显示“已找到 N 处 · 已搜索 X%”，结果可用，替换按钮不存在。
#[gpui::test]
fn streaming_find_on_a_huge_file_has_no_replace(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);
    settle(cx);

    cx.simulate_keystrokes("ctrl-f");
    cx.run_until_parked();
    refresh(cx);
    cx.simulate_input("needle");
    cx.run_until_parked();
    settle(cx);

    let find = view.read_with(cx, |v, _| v.views[&v.editor.focused].primary.find.clone());
    find.read_with(cx, |h, _| {
        assert!(h.is_visible());
        assert_eq!(h.document_size(), rove_buffer::SizeClass::Huge);
        assert!(!h.replace_allowed(), "超大文件上没有替换");
        assert_eq!(h.model().matches().len(), 6000, "每行一处");
    });
    // 查找条里没有替换入口：#48 的 `ctrl+h` 在超大文件上也不展开替换行（必须遵守 replace_allowed）。
    cx.simulate_keystrokes("ctrl-h");
    cx.run_until_parked();
    refresh(cx);
    find.read_with(cx, |h, cx| {
        assert!(!h.can_replace(cx), "超大文件上不提供替换");
        assert!(!h.replace_shown(cx), "替换行不显示");
    });
}

/// 超长行：行尾显示截断标记、字符数正确，点击后在只读临时标签里断行显示。
#[gpui::test]
fn long_lines_are_truncated_and_open_in_a_read_only_tab(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "min.json", cx);
    settle(cx);

    let document = document(&view, cx);
    let chars = long_line::line_chars(&*document.text, Line(0)).unwrap();
    assert_eq!(chars, 20_008, "外面 8 个字符");

    // 渲染上限 40（settings.json）：第 0 行被截断，标记文字带正确的字符数。
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    assert_eq!(
        code.read_with(cx, |c, _| c.is_line_truncated(0)),
        Some(true)
    );
    let overlays = code.read_with(cx, |c, _| {
        c.merged_decorations(0)
            .map(|m| m.overlays.iter().map(|o| o.text.to_string()).collect())
            .unwrap_or_else(Vec::new)
    });
    let mark = overlays
        .iter()
        .find(|t| t.contains("已截断"))
        .unwrap_or_else(|| panic!("没有截断标记：{overlays:?}"));
    assert!(mark.contains(&chars.to_string()), "字符数不对：{mark}");

    // 执行 rove.editor.openLongLine：临时标签是只读的虚拟文档，内容已断行。
    view.update(cx, |v, cx| v.open_long_line_at(0, cx));
    settle(cx);
    let tab = view.read_with(cx, |v, _| v.active_large_file_document().unwrap());
    assert!(
        matches!(&tab.resource, Resource::Virtual(name) if long_line::parse_resource(name).is_some()),
        "{:?}",
        tab.resource
    );
    assert!(!tab.metadata.read_only.is_empty(), "临时标签只读");
    let lines = tab.text.lines().known;
    assert_eq!(
        lines,
        (chars as usize).div_ceil(long_line::FALLBACK_WRAP_COLUMNS) as u64,
        "按固定宽度断行（#57 合并前的临时实现）"
    );

    // 同一行重复打开复用同一个标签（照搬 VS Code 对临时编辑器的做法）。
    let before = view.read_with(cx, |v, _| v.editor.active().tabs.len());
    open_file(&view, "min.json", cx);
    view.update(cx, |v, cx| v.open_long_line_at(0, cx));
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.editor.active().tabs.len()),
        before
    );
    // 在临时标签里再执行一次不会套娃。
    view.update(cx, |v, cx| v.open_long_line_at(0, cx));
    settle(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.editor.active().tabs.len()),
        before
    );
}

fn install_truncation_test_action(view: &Entity<Workbench>, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        v.register_truncation_action(
            crate::large_file::TruncationAction {
                command: "workbench.action.pinEditor",
                label_key: "editors.command.pinEditor",
                applies: |code| code.language.as_deref() == Some("json"),
            },
            cx,
        );
    });
    settle(cx);
}

fn truncation_points(
    code: &Entity<CodeView>,
    cx: &mut VisualTestContext,
) -> ([gpui::Point<gpui::Pixels>; 2], f64) {
    code.update(cx, |code, cx| {
        let mut p = code.scroll_position();
        p.x = f64::MAX;
        code.set_scroll_position(p, false, cx);
    });
    settle(cx);
    code.update(cx, |code, _| {
        let (entry, shaped) = code.line_geometry(0).unwrap();
        let marker = &entry.merged.as_ref().unwrap().overlays[0];
        let xs = code.xs_for(&entry, shaped.as_deref());
        let segment = code.visual_map().span(0) - 1;
        let shift = code
            .segment_geometry(0, &entry, &xs, segment)
            .map_or(0.0, |(shift, _)| shift as f32);
        let start = entry.display.x_for_source(&xs, marker.column) + shift;
        let origin = code.layout.text.origin;
        let metrics = code.metrics().unwrap();
        let scroll = code.scroll_position();
        let y = origin.y
            + gpui::px(
                ((f64::from(segment) - scroll.top + 0.5) * metrics.line_height
                    + metrics.padding_top) as f32,
            );
        let points: Vec<_> = code
            .overlay_parts(marker, start)
            .map(|part| {
                gpui::point(
                    origin.x
                        + gpui::px(
                            part.x + f32::from(part.shaped.layout.width) / 2.0 - scroll.x as f32,
                        ),
                    y,
                )
            })
            .collect();
        assert_eq!(points.len(), 2, "both runs must be shaped");
        assert!(points.iter().all(|point| code.layout.text.contains(point)));
        assert!(points[0].x < points[1].x);
        assert_eq!(
            code.overlay_at_point(points[0]),
            Some((0, marker.text.clone()))
        );
        assert_eq!(
            code.overlay_at_point(points[1]),
            Some((0, marker.action.as_ref().unwrap().text.clone()))
        );
        if segment > 0 {
            let first_row = gpui::point(
                points[1].x,
                y - gpui::px((f64::from(segment) * metrics.line_height) as f32),
            );
            assert_eq!(code.overlay_at_point(first_row), None);
        }
        ([points[0], points[1]], scroll.x)
    })
}

#[gpui::test]
fn adjacent_truncation_action_is_reachable_and_preserves_the_source_group(cx: &mut TestAppContext) {
    let f = Fixture::new(
        r#"{ "rove.largeFile.threshold": 4096, "editor.stopRenderingLineAfter": 400 }"#,
    );
    std::fs::write(f.folder.path().join("long.txt"), "x".repeat(20_000)).unwrap();
    let (view, cx) = f.open(cx);
    open_file(&view, "min.json", cx);
    let source = document(&view, cx);
    let (source_group, code) = view.read_with(cx, |v, _| (v.editor.focused, v.code_view().clone()));
    code.read_with(cx, |code, _| {
        assert_eq!(code.is_line_truncated(0), Some(true))
    });
    install_truncation_test_action(&view, cx);

    let ([_, action_point], scroll) = truncation_points(&code, cx);
    assert!(
        scroll > 0.0,
        "both actions are reachable by horizontal scrolling"
    );
    let blank = code.update(cx, |code, _| {
        let blank = gpui::point(
            action_point.x,
            action_point.y + gpui::px((code.metrics().unwrap().line_height * 3.0) as f32),
        );
        assert!(code.layout.text.contains(&blank));
        assert_eq!(code.overlay_at_point(blank), None);
        blank
    });
    cx.simulate_click(blank, gpui::Modifiers::default());
    settle(cx);
    view.read_with(cx, |v, _| {
        assert!(!v.editor.active().current().unwrap().pinned)
    });
    cx.simulate_click(action_point, gpui::Modifiers::default());
    settle(cx);
    view.read_with(cx, |v, _| {
        assert!(v.editor.active().current().unwrap().pinned)
    });
    assert_eq!(document(&view, cx).id, source.id);

    let other = view.update(cx, |v, cx| {
        let group = v.editor.split(crate::editors::Direction::Right).unwrap();
        v.editor.focus(group);
        cx.notify();
        group
    });
    settle(cx);
    open_file(&view, "long.txt", cx);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.editor.focused, other);
        let overlays = &v
            .code_view()
            .read(cx)
            .merged_decorations(0)
            .unwrap()
            .overlays;
        assert_eq!(overlays.len(), 1);
        assert!(overlays[0].action.is_none());
    });

    let ([marker_point, _], _) = truncation_points(&code, cx);
    cx.simulate_click(marker_point, gpui::Modifiers::default());
    settle(cx);
    view.read_with(cx, |v, _| assert_eq!(v.editor.focused, source_group));
    let target = document(&view, cx);
    assert!(matches!(&target.resource, Resource::Virtual(name)
        if long_line::parse_resource(name) == Some((source.id.0, 0))));
    assert!(!target.metadata.read_only.is_empty());
}

#[gpui::test]
fn adjacent_truncation_action_hits_only_the_last_wrapped_segment(cx: &mut TestAppContext) {
    for mode in ["on", "wordWrapColumn"] {
        let f = Fixture::new(&format!(
            r#"{{ "editor.stopRenderingLineAfter": 360, "editor.wordWrap": "{mode}", "editor.wordWrapColumn": 65 }}"#
        ));
        std::fs::write(
            f.folder.path().join("min.json"),
            format!(
                "{{\"value\":\"{}\",\n\"after\":[\n{}0\n]}}",
                "x".repeat(4_000),
                "0,\n".repeat(64)
            ),
        )
        .unwrap();
        let (view, cx) = f.open(cx);
        open_file(&view, "min.json", cx);
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        code.read_with(cx, |code, _| {
            assert!(code.visual_map().span(0) > 1, "{mode}")
        });
        install_truncation_test_action(&view, cx);
        let ([_, action_point], _) = truncation_points(&code, cx);
        cx.simulate_click(action_point, gpui::Modifiers::default());
        settle(cx);
        view.read_with(cx, |v, _| {
            assert!(v.editor.active().current().unwrap().pinned, "{mode}")
        });

        let measured = code.read_with(cx, |c, _| c.overlay_width);
        assert!(measured > 0.0);
        code.update(cx, |c, cx| {
            let mut scroll = c.scroll_position();
            scroll.top = 30.0;
            c.set_scroll_position(scroll, false, cx);
        });
        settle(cx);
        code.read_with(cx, |c, _| {
            assert!(c.viewport().unwrap().first_line > 0);
            assert_eq!(
                c.overlay_width, measured,
                "scrolling must not clear the measured extent"
            );
        });
        open_file(&view, "small.txt", cx);
        code.read_with(cx, |c, _| assert_eq!(c.overlay_width, 0.0));
    }
}

/// `editor.stopRenderingLineAfter` 改成别的值后截断位置随之变化。
#[gpui::test]
fn the_render_limit_setting_moves_the_truncation_point(cx: &mut TestAppContext) {
    let f = Fixture::new(
        r#"{ "rove.largeFile.threshold": 4096, "editor.stopRenderingLineAfter": 10 }"#,
    );
    let (view, cx) = f.open(cx);
    open_file(&view, "min.json", cx);
    settle(cx);
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    let column_at = |cx: &VisualTestContext| {
        code.read_with(cx, |c, _| {
            c.merged_decorations(0)
                .and_then(|m| m.overlays.first().map(|o| o.column))
        })
    };
    assert_eq!(column_at(cx), Some(10));

    // 改成 25：标记后移。
    std::fs::write(
        f._user.path().join("settings.json"),
        r#"{ "rove.largeFile.threshold": 4096, "editor.stopRenderingLineAfter": 25 }"#,
    )
    .unwrap();
    f.service.reload();
    settle(cx);
    settle(cx);
    assert_eq!(column_at(cx), Some(25));
}

/// 外部在末尾追加内容：滚动位置保持、索引增量更新、进度显示正确。
#[gpui::test]
fn appending_to_the_file_keeps_the_scroll_position(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);
    settle(cx);

    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.update(cx, |c, cx| {
        c.set_scroll_position(
            crate::code_view::ScrollPosition {
                top: 1200.0,
                x: 0.0,
            },
            false,
            cx,
        )
    });
    settle(cx);
    let before = code.read_with(cx, |c, _| c.scroll_position().top);
    assert_eq!(before, 1200.0);

    let document = document(&view, cx);
    let text = document.text.clone();
    let count = text.lines();

    // #8 的增量：追加后重新映射，已有行号不变。
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(f.folder.path().join("big.log"))
        .unwrap();
    for i in 0..100 {
        writeln!(file, "appended {i}").unwrap();
    }
    drop(file);
    settle(cx);

    // 滚动位置没动（新的快照换进来也要保持；这里文档还没重载，但位置必须稳定）。
    assert_eq!(code.read_with(cx, |c, _| c.scroll_position().top), before);
    let (after, complete) = view.read_with(cx, |v, cx| {
        let code = v.code_view().read(cx);
        (
            code.scroll_line_estimate(),
            code.text().is_some_and(|t| t.lines().complete),
        )
    });
    match after {
        // 索引未建完：估计值不小于已知行数（滚动范围只扩不缩）。
        Some(lines) => assert!(lines >= count.known),
        // 索引建完：不再下发估计值，内容高度回到真实（且经折叠映射的）行数。
        None => assert!(complete, "只有索引建完才撤掉估计值"),
    }
}

/// 状态栏数据本身：超大文件的只读标记按 UX_SPEC §3.1 的整张表（数据来自 #8）列出被关闭的功能，
/// 不只是 #20 已经实现的那几项；体积标记只有这一条（#185 起不再有 `rove.status.fileSize`）。
#[gpui::test]
fn the_read_only_mark_lists_every_feature_the_size_level_disables(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);
    settle(cx);
    let disabled = view.read_with(cx, |v, cx| v.code_view().read(cx).disabled_features());
    assert!(!disabled.is_empty(), "超大文件关掉了功能");

    let entry = status(&view, READ_ONLY, cx).expect("只读标记条目");
    assert_eq!(entry.text, "超大文件 · 只读");
    let tooltip = entry.tooltip.unwrap();
    view.read_with(cx, |v, _| {
        for feature in rove_buffer::disabled_features(rove_buffer::SizeClass::Huge) {
            let text = v
                .strings
                .text(&Message::key(crate::status_items::model::feature_key(
                    *feature,
                )))
                .unwrap()
                .to_string();
            assert!(tooltip.contains(&text), "悬停缺少 {text}：{tooltip}");
        }
        assert!(
            v.shell.status_bar.get("rove.status.fileSize").is_none(),
            "体积标记只出现一次"
        );
    });
}

/// 行索引尚未建完的假文本源（测试“转到行”的等待）。
pub(super) struct Pending {
    inner: rove_buffer::RopeText,
}

impl Pending {
    pub(super) fn new(lines: u64) -> Self {
        Self {
            inner: rove_buffer::RopeText::from_text(&"x\n".repeat(lines as usize)),
        }
    }
}

impl rove_buffer::TextSource for Pending {
    fn version(&self) -> Version {
        self.inner.version()
    }
    fn byte_len(&self) -> u64 {
        self.inner.byte_len()
    }
    fn lines(&self) -> rove_buffer::LineCount {
        rove_buffer::LineCount {
            known: 10,
            indexed_bytes: rove_base::ByteOffset(20),
            total_bytes: self.inner.byte_len(),
            complete: false,
        }
    }
    fn line_range(&self, l: Line) -> rove_base::Result<rove_base::ByteRange> {
        self.inner.line_range(l)
    }
    fn visit_chunks(
        &self,
        r: rove_base::ByteRange,
        v: &mut dyn FnMut(&str),
    ) -> rove_base::Result<()> {
        self.inner.visit_chunks(r, v)
    }
    fn offset(&self, p: rove_base::Position) -> rove_base::Result<rove_base::ByteOffset> {
        self.inner.offset(p)
    }
    fn position(&self, o: rove_base::ByteOffset) -> rove_base::Result<rove_base::Position> {
        self.inner.position(o)
    }
    fn utf16_to_utf8(
        &self,
        l: Line,
        c: rove_base::Utf16Column,
    ) -> rove_base::Result<rove_base::Utf8Column> {
        self.inner.utf16_to_utf8(l, c)
    }
    fn utf8_to_utf16(
        &self,
        l: Line,
        c: rove_base::Utf8Column,
    ) -> rove_base::Result<rove_base::Utf16Column> {
        self.inner.utf8_to_utf16(l, c)
    }
}

#[track_caller]
pub(super) fn wait_tail(
    cx: &mut VisualTestContext,
    mut condition: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        refresh(cx);
        if condition(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "tail did not converge");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn disk_append(path: &std::path::Path, suffix: &str) {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(suffix.as_bytes()).unwrap();
    file.flush().unwrap();
}

fn decoded(document: &Document, range: ByteRange) -> String {
    let mut text = String::new();
    document
        .text
        .visit_chunks(range, &mut |chunk| text.push_str(chunk))
        .unwrap();
    text
}

fn tail_has_line(document: &Document, line: &str) -> bool {
    let last = document.text.lines().known.saturating_sub(2);
    document
        .text
        .line_range(Line(last))
        .is_ok_and(|range| decoded(document, range) == line)
}

#[gpui::test]
fn r66_follows_real_appends_pauses_resumes_and_keeps_the_rotated_path(cx: &mut TestAppContext) {
    use crate::follow_tail as tail;
    let fixture = Fixture::new(SETTINGS);
    let path = fixture.folder.path().join("big.log");
    let (view, cx) = fixture.open(cx);
    open_file(&view, "big.log", cx);
    let original = document(&view, cx);
    wait_tail(cx, |cx| {
        original.text.lines().complete && document(&view, cx).text.lines().complete
    });
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    let group = view.read_with(cx, |view, _| view.editor.focused);
    let button = match group.0 {
        0 => "editor-title-follow-0",
        1 => "editor-title-follow-1",
        _ => panic!("unexpected initial group"),
    };
    let button = cx
        .debug_bounds(button)
        .expect("follow title button")
        .center();
    cx.simulate_click(button, gpui::Modifiers::default());
    settle(cx);
    let menu = cx
        .debug_bounds("editor-title-follow-menu-action")
        .expect("follow title menu action")
        .center();
    cx.simulate_click(menu, gpui::Modifiers::default());
    wait_tail(cx, |cx| {
        code.read_with(cx, |code, _| code.scroll_position().top > 0.0)
    });
    assert_eq!(
        status(&view, tail::FOLLOW_STATUS, cx).unwrap().text,
        "跟随末尾"
    );
    assert!(status(&view, READ_ONLY, cx).is_some());
    let baseline_reopens = view.read_with(cx, |view, _| view.follow_tail.counts.reopens);

    disk_append(&path, "first append\n");
    wait_tail(cx, |cx| tail_has_line(&document(&view, cx), "first append"));
    let first_end = code.read_with(cx, |code, _| code.scroll_position().top);
    assert!(first_end > 5_000.0);

    let point = code.read_with(cx, |code, _| code.layout.text.center());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: point,
        delta: gpui::ScrollDelta::Lines(gpui::point(0.0, 8.0)),
        ..Default::default()
    });
    wait_tail(cx, |cx| {
        status(&view, tail::FOLLOW_STATUS, cx).is_some_and(|entry| entry.text == "已暂停跟随")
    });
    let paused = code.read_with(cx, |code, _| code.scroll_position().top);
    assert!(paused < first_end);

    disk_append(&path, "second append\n");
    wait_tail(cx, |cx| {
        tail_has_line(&document(&view, cx), "second append")
    });
    assert_eq!(
        code.read_with(cx, |code, _| code.scroll_position().top),
        paused
    );
    let resume = cx
        .debug_bounds("status-rove.status.followTail")
        .expect("follow status is rendered")
        .center();
    cx.simulate_click(resume, gpui::Modifiers::default());
    wait_tail(cx, |cx| {
        code.read_with(cx, |code, _| code.scroll_position().top > paused)
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.follow_tail.counts.appends, 2);
        assert_eq!(view.follow_tail.counts.reopens, baseline_reopens);
    });

    assert!(cx.update(|window, cx| code.focus_handle(cx).is_focused(window)));
    cx.simulate_keystrokes("ctrl-home");
    wait_tail(cx, |cx| {
        status(&view, tail::FOLLOW_STATUS, cx).is_some_and(|entry| entry.text == "已暂停跟随")
    });
    cx.simulate_keystrokes("ctrl-end");
    wait_tail(cx, |cx| {
        status(&view, tail::FOLLOW_STATUS, cx).is_some_and(|entry| entry.text == "跟随末尾")
    });

    // Rename the live Windows file while the previous snapshot is retained.
    // The replacement at the original path must decode its new UTF-16 BOM.
    std::fs::rename(&path, fixture.folder.path().join("archived.log")).unwrap();
    let mut replacement = vec![0xff, 0xfe];
    replacement.extend("rotated\n".encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&path, replacement).unwrap();
    wait_tail(cx, |cx| {
        let replacement = document(&view, cx);
        replacement.text.lines().complete
            && replacement.text.byte_len() == 8
            && tail_has_line(&replacement, "rotated")
    });
    let replacement = document(&view, cx);
    assert_eq!(replacement.id, original.id);
    assert_eq!(replacement.resource, original.resource);
    assert!(replacement.metadata.bom);
    assert_ne!(replacement.metadata.encoding, original.metadata.encoding);
    let retained_reasons: Vec<_> = original
        .metadata
        .read_only
        .iter()
        .filter(|reason| !matches!(reason, ReadOnlyReason::HugeFile))
        .cloned()
        .collect();
    assert_eq!(replacement.metadata.read_only.as_ref(), retained_reasons);
    assert!(view.read_with(cx, |view, _| view.follow_tail.counts.reopens) > 0);

    open_file(&view, "small.txt", cx);
    assert!(status(&view, tail::FOLLOW_STATUS, cx).is_none());
    open_file(&view, "big.log", cx);
    assert!(status(&view, tail::FOLLOW_STATUS, cx).is_none());
}

#[cfg(windows)]
#[gpui::test]
fn r66_tracks_a_fast_same_handle_truncation_that_preserves_both_samples(cx: &mut TestAppContext) {
    use std::io::Write as _;
    let fixture = Fixture::new(SETTINGS);
    let path = fixture.folder.path().join("big.log");
    let original = format!(
        "{}before rotation\n{}",
        "identical leading rows\n".repeat(450),
        "identical trailing rows\n".repeat(450),
    );
    let replacement =
        original.replace("before rotation", "after! rotation") + "final rotated row\n";
    assert_eq!(&original[..4096], &replacement[..4096]);
    assert_eq!(
        &original[original.len() - 4096..],
        &replacement[original.len() - 4096..original.len()]
    );
    std::fs::write(&path, original).unwrap();

    let (view, cx) = fixture.open(cx);
    open_file(&view, "big.log", cx);
    let old = document(&view, cx);
    assert_eq!(old.metadata.size, rove_buffer::SizeClass::Huge);
    view.update(cx, |view, cx| view.command(crate::follow_tail::FOLLOW, cx));
    wait_tail(cx, |cx| {
        status(&view, crate::follow_tail::FOLLOW_STATUS, cx).is_some()
    });

    let baseline_reopens = view.read_with(cx, |view, _| view.follow_tail.counts.reopens);
    // Even with a previous TextSource reference alive, Windows must let the
    // same writer handle truncate and write back a file longer than the old one.
    let mut writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    writer.set_len(0).unwrap();
    writer.write_all(replacement.as_bytes()).unwrap();
    writer.flush().unwrap();
    drop(writer);
    wait_tail(cx, |cx| {
        let current = document(&view, cx);
        current.text.lines().complete && tail_has_line(&current, "final rotated row")
    });
    let current = document(&view, cx);
    assert_eq!(current.id, old.id);
    assert_eq!(current.resource, old.resource);
    let range = Span::new(ByteOffset(0), ByteOffset(current.text.byte_len())).unwrap();
    assert_eq!(decoded(&current, range), replacement);
    view.read_with(cx, |view, _| {
        assert_eq!(view.follow_tail.counts.appends, 0);
        assert!(view.follow_tail.counts.reopens > baseline_reopens);
    });
}

#[gpui::test]
fn r66_formatted_copy_uses_the_real_truncation_action_and_preserves_disk(cx: &mut TestAppContext) {
    use crate::follow_tail as tail;
    let fixture = Fixture::new(
        r#"{ "rove.largeFile.threshold": 65536, "editor.stopRenderingLineAfter": 400 }"#,
    );
    let path = fixture.folder.path().join("min.json");
    let before = std::fs::read(&path).unwrap();
    let (view, cx) = fixture.open(cx);
    open_file(&view, "small.txt", cx);
    cx.simulate_keystrokes("ctrl-shift-p");
    cx.simulate_input("格式化后查看");
    settle(cx);
    view.read_with(cx, |view, app| {
        let palette = view.quick.view.as_ref().expect("command palette").read(app);
        let list = &palette.service.active().expect("opened palette").list;
        assert!(!list
            .display_rows(0..list.len())
            .iter()
            .any(|row| { row.label.contains("格式化后查看") }));
    });
    cx.simulate_keystrokes("escape");
    open_file(&view, "min.json", cx);
    let source = document(&view, cx);
    view.read_with(cx, |view, _| {
        assert!(view.follow_command_visible(tail::FORMATTED))
    });

    // Cancel from the same UI turn before a result can open, then use the line-end action.
    view.update(cx, |view, cx| {
        view.command(tail::FORMATTED, cx);
        assert!(view.shell.status_bar.get(tail::FORMAT_STATUS).is_some());
        assert!(view.cancel_formatted_preview(cx));
    });
    settle(cx);
    assert_eq!(document(&view, cx).id, source.id);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    let ([_, action], _) = truncation_points(&code, cx);
    cx.simulate_click(action, gpui::Modifiers::default());
    wait_tail(cx, |cx| document(&view, cx).id != source.id);
    let preview = document(&view, cx);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.formatted_tab_title(&preview.resource).as_deref(),
            Some("min.json（格式化后）")
        );
        assert!(view.open_document(source.id).is_some());
    });
    assert!(!preview.metadata.read_only.is_empty());
    assert!(status(&view, READ_ONLY, cx)
        .unwrap()
        .tooltip
        .unwrap()
        .contains("格式化后的只读副本"));
    let range = Span::new(ByteOffset(0), ByteOffset(preview.text.byte_len())).unwrap();
    assert!(decoded(&preview, range).starts_with("{\n    \"a\": \""));
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[gpui::test]
fn r66_ordinary_tail_appends_and_custom_command_bindings_work(cx: &mut TestAppContext) {
    use crate::follow_tail as tail;
    let fixture = Fixture::new(
        r#"{ "rove.largeFile.threshold": 65536, "editor.stopRenderingLineAfter": 400 }"#,
    );
    let path = fixture.folder.path().join("small.txt");
    std::fs::write(&path, "log line\r\n".repeat(150)).unwrap();
    let keybindings = fixture._user.path().join("keybindings.json");
    std::fs::write(
        &keybindings,
        r#"[{"key":"ctrl+alt+t","command":"rove.editor.followTail"},
            {"key":"ctrl+alt+j","command":"rove.editor.viewFormatted"}]"#,
    )
    .unwrap();
    cx.update(|cx| crate::keys::watch_user_file(keybindings, cx));
    let (view, cx) = fixture.open(cx);
    open_file(&view, "small.txt", cx);
    let initial = document(&view, cx);
    assert_ne!(initial.metadata.size, rove_buffer::SizeClass::Huge);
    cx.simulate_keystrokes("ctrl-alt-t");
    wait_tail(cx, |cx| {
        view.read_with(cx, |view, _| {
            view.follow_tail.has_ordinary_baseline(initial.id)
        })
    });
    disk_append(&path, "ordinary append\r\n");
    wait_tail(cx, |cx| {
        tail_has_line(&document(&view, cx), "ordinary append")
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.follow_tail.counts.appends, 1);
        assert_eq!(view.follow_tail.counts.reopens, 0);
        assert!(!view.editing.saves.borrow().is_dirty(initial.id));
    });
    std::fs::write(&path, "after truncation\n").unwrap();
    wait_tail(cx, |cx| {
        tail_has_line(&document(&view, cx), "after truncation")
    });
    assert_eq!(document(&view, cx).id, initial.id);
    assert_eq!(
        view.read_with(cx, |view, _| view.follow_tail.counts.reopens),
        1
    );

    view.update(cx, |view, cx| {
        view.command("workbench.action.closeActiveEditor", cx)
    });
    settle(cx);
    assert!(status(&view, tail::FOLLOW_STATUS, cx).is_none());
    open_file(&view, "min.json", cx);
    let source = document(&view, cx);
    cx.simulate_keystrokes("ctrl-alt-j");
    wait_tail(cx, |cx| document(&view, cx).id != source.id);
    let preview = document(&view, cx);
    assert!(view.read_with(cx, |view, _| {
        view.formatted_tab_title(&preview.resource).is_some()
    }));
}

#[gpui::test]
fn r66_external_parent_rotation_keeps_the_log_path_and_updates_other_tabs(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        r#"{ "rove.largeFile.threshold": 4096, "workbench.editor.closeOnFileDelete": true, "workbench.editor.enablePreview": false }"#,
    );
    let parent = fixture.folder.path().join("Logs");
    let archive = fixture.folder.path().join("Archive");
    std::fs::create_dir(&parent).unwrap();
    std::fs::write(parent.join("Editor.log"), "old log line\n".repeat(600)).unwrap();
    std::fs::write(parent.join("notes.txt"), "notes\n").unwrap();
    let (view, cx) = fixture.open(cx);
    open_file(&view, "Logs/notes.txt", cx);
    let notes = document(&view, cx).id;
    open_file(&view, "small.txt", cx);
    let removed = document(&view, cx).id;
    open_file(&view, "Logs/Editor.log", cx);
    let original = document(&view, cx);
    view.update(cx, |view, cx| view.command(crate::follow_tail::FOLLOW, cx));
    wait_tail(cx, |cx| {
        status(&view, crate::follow_tail::FOLLOW_STATUS, cx).is_some()
            && document(&view, cx).text.lines().complete
    });

    std::fs::rename(&parent, &archive).unwrap();
    std::fs::create_dir(&parent).unwrap();
    std::fs::write(parent.join("Editor.log"), "replacement log\n".repeat(600)).unwrap();
    std::fs::remove_file(fixture.folder.path().join("small.txt")).unwrap();
    let workspace = |path| WorkspacePath::new(path).unwrap();
    view.update(cx, |view, cx| {
        view.apply_explorer_workspace_events(
            vec![
                FileEvent::Renamed {
                    from: workspace("Logs"),
                    to: workspace("Archive"),
                },
                FileEvent::Removed(workspace("Logs/Editor.log")),
                FileEvent::Created(workspace("Logs/Editor.log")),
                FileEvent::Removed(workspace("small.txt")),
            ],
            cx,
        );
    });
    wait_tail(cx, |cx| {
        tail_has_line(&document(&view, cx), "replacement log")
    });
    let log = document(&view, cx);
    assert_eq!(log.id, original.id);
    assert_eq!(
        log.resource,
        Resource::Workspace(workspace("Logs/Editor.log"))
    );
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.open_document(notes).unwrap().resource,
            Resource::Workspace(workspace("Archive/notes.txt"))
        );
        assert!(view.open_document(removed).is_none());
    });
}

#[gpui::test]
fn r66_follow_status_and_readonly_notice_coexist_with_index_progress(cx: &mut TestAppContext) {
    let fixture = Fixture::new(SETTINGS);
    let (view, cx) = fixture.open(cx);
    open_file(&view, "big.log", cx);
    view.update(cx, |view, cx| view.command(crate::follow_tail::FOLLOW, cx));
    wait_tail(cx, |cx| {
        status(&view, crate::follow_tail::FOLLOW_STATUS, cx).is_some()
    });
    let current = document(&view, cx);

    view.update_in(cx, |view, window, cx| {
        // Drive the real status assembly with a stable 20/44-byte indexed
        // snapshot rather than racing the background index on a tiny fixture.
        let indexing = Arc::new(Document {
            text: Arc::new(Pending::new(22)),
            ..(*current).clone()
        });
        view.editor.replace_document(indexing, None);
        view.sync_views(cx);
        view.sync_large_files(cx);
        view.sync_follow_tail(cx);
        view.sync_status(window, cx);
        assert!(view
            .shell
            .status_bar
            .get(lf::INDEX_STATUS)
            .unwrap()
            .text
            .contains("45%"));
        assert_eq!(
            view.shell.status_bar.get(READ_ONLY).unwrap().text,
            "超大文件 · 只读"
        );
        assert_eq!(
            view.shell
                .status_bar
                .get(crate::follow_tail::FOLLOW_STATUS)
                .unwrap()
                .text,
            "跟随末尾"
        );
        assert_eq!(view.shell.feedback.all_editor_bars().len(), 1);
        view.editor.replace_document(current.clone(), None);
        view.sync_views(cx);
    });
}

#[gpui::test]
fn r66_preview_refuses_input_over_the_configured_threshold(cx: &mut TestAppContext) {
    let fixture = Fixture::new(SETTINGS);
    let (view, cx) = fixture.open(cx);
    open_file(&view, "min.json", cx);
    let original = document(&view, cx);
    view.update(cx, |view, cx| {
        view.command(crate::follow_tail::FORMATTED, cx)
    });
    settle(cx);
    assert_eq!(document(&view, cx).id, original.id);
    assert!(view
        .read_with(cx, |view, _| view.status())
        .contains("无法格式化后查看"));
}

#[gpui::test]
#[ignore = "requires ROVE_TAIL_JSON_FIXTURE pointing to a minified real Unity manifest"]
fn r66_real_unity_manifest_opens_a_formatted_readonly_copy(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        r#"{ "rove.largeFile.threshold": 65536, "editor.stopRenderingLineAfter": 400 }"#,
    );
    let path = PathBuf::from(std::env::var_os("ROVE_TAIL_JSON_FIXTURE").expect("real manifest"));
    let before = std::fs::read(&path).unwrap();
    assert!(!before.contains(&b'\n'));
    let input = fixture.folder.path().join("manifest.json");
    std::fs::write(&input, &before).unwrap();
    let (view, cx) = fixture.open(cx);
    open_file(&view, "manifest.json", cx);
    let source = document(&view, cx);
    let code = view.read_with(cx, |view, _| view.code_view().clone());
    let ([_, format], _) = truncation_points(&code, cx);
    cx.simulate_click(format, gpui::Modifiers::default());
    wait_tail(cx, |cx| document(&view, cx).id != source.id);
    let preview = document(&view, cx);
    assert!(preview
        .metadata
        .read_only
        .contains(&ReadOnlyReason::Metadata));
    assert!(preview.text.lines().known > 2);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.formatted_tab_title(&preview.resource).as_deref(),
            Some("manifest.json（格式化后）")
        );
        assert!(view.open_document(source.id).is_some());
    });
    let range = Span::new(ByteOffset(0), ByteOffset(preview.text.byte_len())).unwrap();
    let pretty = decoded(&preview, range);
    assert!(pretty.starts_with("{\n"));
    assert!(pretty.contains("\n    \"dependencies\": {"));
    let input_hash = rove_buffer::save::content_hash(&before);
    assert_eq!(std::fs::read(input).unwrap(), before);
    assert_eq!(std::fs::read(path).unwrap(), before);
    println!("R66_UNITY_MANIFEST source_bytes={} preview_bytes={} unchanged_disk_fnv64={input_hash:016x}", before.len(), preview.text.byte_len());
}
