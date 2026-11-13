//! #22 入口集成测试（真实 GPUI 测试窗口）：
//! 超大文件打开 → 提示条与状态栏 → 索引进度 → 转到行等待 → 流式查找 → 超长行截断与临时标签。
//!
//! 测试把 `rove.largeFile.threshold` 调小（几十 KB），用小文件走完**同一条**超大文件路径；
//! 1 GB / 50 MB 的真实数据由 `cargo xtask bench` 的测试数据在人工验收时复跑（见 PR）。
use super::*;
use crate::large_file::{
    goto, long_line, notice, progress::IndexProgress, registry as lf, status_entry,
};
use gpui::{TestAppContext, VisualTestContext};
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
    let entry = status(&view, lf::SIZE_STATUS, cx).expect("文件大小条目");
    assert_eq!(entry.text, "超大文件 · 只读");
    let tooltip = entry.tooltip.unwrap();
    for feature in ["语法高亮", "折叠", "缩略图"] {
        assert!(tooltip.contains(feature), "悬停缺少 {feature}：{tooltip}");
    }

    // 普通文件没有提示条、也没有大小标记。
    open_file(&view, "small.txt", cx);
    assert!(status(&view, lf::SIZE_STATUS, cx).is_none());
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

/// 状态栏数据本身：被关闭的功能来自 #20 的查询接口，普通文件没有条目。
#[gpui::test]
fn status_entry_lists_the_features_the_code_view_disabled(cx: &mut TestAppContext) {
    let f = Fixture::new(SETTINGS);
    let (view, cx) = f.open(cx);
    open_file(&view, "big.log", cx);
    settle(cx);
    let disabled = view.read_with(cx, |v, cx| v.code_view().read(cx).disabled_features());
    assert!(!disabled.is_empty(), "超大文件关掉了功能");

    view.read_with(cx, |v, _| {
        let entry = status_entry(&v.strings, rove_buffer::SizeClass::Huge, &disabled).unwrap();
        let tooltip = entry.tooltip.unwrap();
        // UX_SPEC §3.1 的整张表（数据来自 #8），不只是 #20 已经实现的那几项。
        for feature in rove_buffer::disabled_features(rove_buffer::SizeClass::Huge) {
            let text = v
                .strings
                .text(&Message::key(crate::large_file::notice::feature_key(
                    *feature,
                )))
                .unwrap()
                .to_string();
            assert!(tooltip.contains(&text), "悬停缺少 {text}：{tooltip}");
        }
        // `editor.largeFileOptimizations` 关掉时这一级不生效：#20 报空集合，这里也不列功能。
        let off = status_entry(&v.strings, rove_buffer::SizeClass::Large, &[]).unwrap();
        assert!(off.tooltip.unwrap().contains("没有"));
        assert!(status_entry(&v.strings, rove_buffer::SizeClass::Normal, &[]).is_none());
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
