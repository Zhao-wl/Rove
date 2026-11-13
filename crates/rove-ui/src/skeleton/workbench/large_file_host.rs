//! #22：超大文件与超长行界面的窗口装配。
//!
//! 模型在 `crate::large_file`（不依赖 GPUI），这里只把它接到窗口上：
//!
//! 1. **提示条**：打开超大文件时经 #16 的反馈层报一条 `Reason::FileContext`（§6.1 第 3 级，
//!    可关闭、可“不再提示”），**从不弹通知**；大文件级别不报提示条。
//! 2. **状态栏**：每帧把“文件大小标记 + 被关闭的功能”与“行索引 N%”写进状态栏条目；
//!    索引建完后进度条目消失。被关闭的功能取自 #20 的 `CodeView::disabled_features`。
//! 3. **滚动**：把 [`IndexProgress::estimated_lines`] 交给代码视图当内容高度
//!    （#19 的 `set_scroll_line_estimate`），未索引部分按字节线性外推。
//! 4. **转到行的等待**：#24 的快速输入框把目标行交给这里；已索引直接跳，未索引进入等待
//!    （后台轮询行索引），索引到达后自动跳转，期间可取消。
//! 5. **流式查找**：#21 的查找条已经按块回报进度与新匹配；这里只在文档换页时告诉它文件级别
//!    （超大文件不提供替换），并在行索引推进时让它重新搜索新索引出来的行。
//! 6. **超长行**：给每个代码视图挂一层截断标记装饰（#19 的行内覆盖物），点击标记或执行
//!    `rove.editor.openLongLine` 时在只读临时标签里打开这一行。

use super::*;
use crate::large_file::{
    goto, index_status_entry,
    long_line::{self, OPEN_LONG_LINE},
    notice,
    progress::IndexProgress,
    registry as lf, status_entry, LongLineSource, TruncationMarks,
};
use crate::{
    code_view::{priority, LayerId},
    feedback::FeedbackAction,
};
use std::{collections::BTreeSet, rc::Rc};

/// 后台等待“转到行”的兜底上限：索引线程异常退出时不至于永远挂着。
const GOTO_TIMEOUT: Duration = Duration::from_secs(600);

/// 窗口里的 #22 状态。
pub(super) struct LargeFiles {
    /// 超长行临时标签的内容来源（串在 #42 的元数据加载器前面）。
    pub(super) long_lines: Arc<LongLineSource>,
    /// 已经报过提示条的文档（一个文档只报一次；“不再提示”由反馈层按去重键记住）。
    noticed: BTreeSet<DocumentId>,
    /// 当前写进状态栏的两条内容（避免每帧重复 set）。
    shown_size: Option<String>,
    shown_index: Option<String>,
    /// 进行中的“转到行”等待（同一时刻至多一个）。
    pending_goto: Option<goto::PendingGoto>,
    /// 等待期间给界面看的进度（#24 在输入框下方显示）。
    goto_progress: Option<IndexProgress>,
    /// 每个文档上一次看到的已知行数：变多了就让查找条重新搜索。
    last_known: BTreeMap<DocumentId, u64>,
    /// 每个代码视图上一次挂截断标记时的 (文档, 快照, 渲染上限)：换文档、换快照或
    /// `editor.stopRenderingLineAfter` 改了都要重挂（标记文字与列都随之变化）。
    attached: BTreeMap<(EditorGroupId, bool), (DocumentId, Version, Option<usize>)>,
}

impl LargeFiles {
    pub(super) fn new(next: Arc<dyn crate::skeleton::text::VirtualSource>) -> Self {
        Self {
            long_lines: Arc::new(LongLineSource::chained(next)),
            noticed: BTreeSet::new(),
            shown_size: None,
            shown_index: None,
            pending_goto: None,
            goto_progress: None,
            last_known: BTreeMap::new(),
            attached: BTreeMap::new(),
        }
    }
}

/// 截断标记装饰层的 ID（每个代码视图一层）。
pub(super) const MARKS_LAYER: &str = "rove.largeFile.truncation";

impl Workbench {
    /// 超长行临时标签的内容来源（测试用）。
    pub fn long_line_source(&self) -> &Arc<LongLineSource> {
        &self.large_files.long_lines
    }

    /// 每帧调用：提示条、状态栏、滚动估计、流式查找与截断标记。
    pub(super) fn sync_large_files(&mut self, cx: &mut Context<Self>) {
        self.sync_large_file_notice(cx);
        self.sync_large_file_views(cx);
        self.sync_large_file_status(cx);
        self.refresh_goto_progress();
        self.poll_line_index(cx);
    }

    /// 逐个代码视图：滚动估计、截断标记装饰层、告诉查找条文件级别（超大文件不提供替换）。
    fn sync_large_file_views(&mut self, cx: &mut Context<Self>) {
        let slots: Vec<(
            EditorGroupId,
            bool,
            Entity<CodeView>,
            Entity<crate::find::host::FindHost>,
        )> = self
            .views
            .iter()
            .flat_map(|(group, views)| {
                std::iter::once((*group, false, &views.primary))
                    .chain(views.secondary.as_ref().map(|s| (*group, true, s)))
            })
            .map(|(group, secondary, slot)| {
                (group, secondary, slot.code.clone(), slot.find.clone())
            })
            .collect();
        for (group, secondary, code, find) in slots {
            // 滚动条按“已索引按行 + 未索引按字节外推”的估计值算内容高度（UX_SPEC §3.2）。
            let shown = code.read(cx).text().map(|text| {
                (
                    text.version(),
                    IndexProgress::of(&text.lines()).estimated_lines(),
                )
            });
            code.update(cx, |code, cx| {
                code.set_scroll_line_estimate(shown.map(|(_, lines)| lines), cx)
            });
            let Some(document) = self.shown_document(group, secondary) else {
                // 这个视图没有文档了：撤掉装饰层，别让它拖着上一份文本的 Arc 不放。
                if self
                    .large_files
                    .attached
                    .remove(&(group, secondary))
                    .is_some()
                {
                    code.update(cx, |code, cx| {
                        code.remove_decorations(&LayerId(MARKS_LAYER.into()), cx);
                    });
                }
                continue;
            };
            find.update(cx, |find, cx| {
                find.set_document_size(document.metadata.size, cx)
            });
            let limit = code.read(cx).render_limit();
            let key = (document.id, document.text.version(), limit);
            if self.large_files.attached.get(&(group, secondary)) == Some(&key) {
                continue;
            }
            self.large_files.attached.insert((group, secondary), key);
            self.attach_truncation_marks(&code, cx);
        }
    }

    /// 打开超大文件时报一条提示条（§6.1 第 3 级）：可关闭、可“不再提示”，从不弹通知。
    fn sync_large_file_notice(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.active_large_file_document() else {
            return;
        };
        if self.large_files.noticed.contains(&document.id) {
            return;
        }
        let Some(n) = notice::notice_for(document.metadata.size, document.text.byte_len()) else {
            // 普通文件与大文件都不报提示条，但也要记下来，免得每帧重算。
            self.large_files.noticed.insert(document.id);
            return;
        };
        self.large_files.noticed.insert(document.id);
        let message = Message {
            key: n.key.into(),
            args: Arc::from([(Arc::from("size"), Arc::from(n.size.as_str()))]),
        };
        let key = format!("{}{}", notice::BAR_KEY_PREFIX, document.id.0);
        let _ = shell::view::report(
            self,
            &key,
            Reason::FileContext {
                document: document.id,
                reason: notice::SUPPRESS_KEY.into(),
            },
            message,
            Arc::from([] as [FeedbackAction; 0]),
            cx,
        );
    }

    /// 状态栏：`超大文件 · 只读`（悬停列出被关闭的功能）与 `行索引 N%`（建完后消失）。
    fn sync_large_file_status(&mut self, cx: &mut Context<Self>) {
        let document = self.active_large_file_document();
        let size = document.as_ref().map(|d| d.metadata.size);
        let disabled = self.code_view().read(cx).disabled_features();

        let entry = size.and_then(|size| status_entry(&self.strings, size, &disabled));
        let text = entry.as_ref().map(|e| e.text.clone());
        if self.large_files.shown_size != text {
            self.large_files.shown_size = text;
            let shell = &mut self.shell;
            match entry {
                Some(entry) => {
                    let _ = shell
                        .status_bar
                        .set(&shell.contributions, lf::SIZE_STATUS, entry);
                }
                None => {
                    shell.status_bar.remove(lf::SIZE_STATUS);
                }
            }
        }
        let progress = document
            .as_ref()
            .map(|d| IndexProgress::of(&d.text.lines()));
        let entry = progress.and_then(|p| index_status_entry(&self.strings, p));
        let text = entry.as_ref().map(|e| e.text.clone());
        if self.large_files.shown_index != text {
            self.large_files.shown_index = text;
            let shell = &mut self.shell;
            match entry {
                Some(entry) => {
                    let _ = shell
                        .status_bar
                        .set(&shell.contributions, lf::INDEX_STATUS, entry);
                }
                None => {
                    shell.status_bar.remove(lf::INDEX_STATUS);
                }
            }
        }
    }

    /// 行索引在后台线程推进，#8 只提供阻塞等待、没有事件可订阅：索引未建完时每帧读一次
    /// 已知行数（O(1)）并请求下一帧。行数变多时让可见的查找条把新索引出来的行也搜一遍。
    fn poll_line_index(&mut self, cx: &mut Context<Self>) {
        let documents: Vec<Arc<Document>> = self
            .views
            .values()
            .flat_map(|v| std::iter::once(&v.primary).chain(&v.secondary))
            .filter_map(|slot| slot.shown)
            .filter_map(|shown| self.document_of(shown.document))
            .collect();
        let indexing: Vec<_> = documents
            .iter()
            .filter(|d| !d.text.lines().complete)
            .collect();
        let mut grown = false;
        for document in &indexing {
            let known = document.text.lines().known;
            let last = self.large_files.last_known.insert(document.id, known);
            grown |= last.is_some_and(|last| last < known);
        }
        if grown {
            // 流式查找：之前只扫描了已知的行（`SearchOutput::partial`），重新搜一遍。
            let finds: Vec<_> = self
                .views
                .values()
                .flat_map(|v| std::iter::once(&v.primary).chain(&v.secondary))
                .map(|slot| slot.find.clone())
                .collect();
            for find in finds {
                find.update(cx, |find, cx| find.reindexed(cx));
            }
        }
        if indexing.is_empty() {
            // 索引建完：不再请求下一帧，窗口回到空闲（进度条目已在状态栏那一步移除）。
            self.large_files.last_known.retain(|id, _| {
                documents
                    .iter()
                    .any(|d| d.id == *id && !d.text.lines().complete)
            });
            return;
        }
        // 索引未建完：进度、滚动估计每帧都在变，请求下一帧。读一次 `lines()` 是 O(1)。
        cx.notify();
    }

    fn document_of(&self, id: DocumentId) -> Option<Arc<Document>> {
        self.editor
            .groups
            .values()
            .flat_map(|pane| pane.tabs.iter())
            .filter_map(Tab::opened)
            .find(|opened| opened.document.id == id)
            .map(|opened| opened.document.clone())
    }

    pub(super) fn active_large_file_document(&self) -> Option<Arc<Document>> {
        self.editor
            .active()
            .current()
            .and_then(Tab::opened)
            .map(|opened| opened.document.clone())
    }

    // ---------------------------------------------------------------- 转到行的等待

    /// 等待中的“转到行”（#24 在输入框下方显示提示与取消按钮）。
    pub fn pending_goto(&self) -> Option<(u64, u8)> {
        let pending = self.large_files.pending_goto.as_ref()?;
        Some((
            pending.line,
            self.large_files.goto_progress.map_or(0, |p| p.percent()),
        ))
    }

    /// 等待中时刷新进度（#24 的提示里显示 `正在建立行索引，完成后跳转（45%）`）。
    fn refresh_goto_progress(&mut self) {
        let Some(pending) = self.large_files.pending_goto.as_ref() else {
            return;
        };
        let Some(document) = self.document_of(pending.document) else {
            return;
        };
        self.large_files.goto_progress = Some(IndexProgress::of(&document.text.lines()));
    }

    /// 取消等待（Esc、关闭快速输入框或点“取消”）。
    pub fn cancel_pending_goto(&mut self) {
        if let Some(pending) = self.large_files.pending_goto.take() {
            pending.cancel();
        }
        self.large_files.goto_progress = None;
    }

    /// 转到行（1 起始）。目标行已索引时立即返回 `true`（调用方照常打开）；
    /// 尚未索引时返回 `false` 并进入等待，索引到达后自动跳转。
    pub(super) fn goto_line_or_wait(
        &mut self,
        document: &Arc<Document>,
        line: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        self.cancel_pending_goto();
        if goto::resolve_now(&*document.text, line).is_some() {
            return true;
        }
        let pending = goto::PendingGoto::new(document.id, line);
        self.large_files.pending_goto = Some(pending.clone());
        self.large_files.goto_progress = Some(IndexProgress::of(&document.text.lines()));
        let text = document.text.clone();
        let cancel = pending.cancel.clone();
        let id = document.id;
        cx.spawn(async move |view, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { goto::wait_for_line(&*text, line, &cancel, GOTO_TIMEOUT) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.finish_pending_goto(id, line, outcome, cx);
            });
        })
        .detach();
        cx.notify();
        false
    }

    fn finish_pending_goto(
        &mut self,
        document: DocumentId,
        line: u64,
        outcome: goto::WaitOutcome,
        cx: &mut Context<Self>,
    ) {
        let still_waiting = self
            .large_files
            .pending_goto
            .as_ref()
            .is_some_and(|p| p.document == document && p.line == line && !p.is_cancelled());
        self.large_files.pending_goto = None;
        self.large_files.goto_progress = None;
        if !still_waiting {
            return;
        }
        let target = match outcome {
            goto::WaitOutcome::Ready(line) => line,
            goto::WaitOutcome::OutOfRange { last } => last,
            goto::WaitOutcome::Cancelled | goto::WaitOutcome::TimedOut => return,
        };
        let Some(document) = self.document_of(document) else {
            return;
        };
        let options = crate::documents::OpenOptions {
            source: crate::documents::OpenSource::GoToLine,
            preserve_focus: false,
            location: Some(rove_base::Position {
                line: target,
                column: rove_base::Utf8Column(0),
            }),
            ..Default::default()
        };
        let resource = document.resource.clone();
        if let Err(e) = self
            .open_documents()
            .open(resource, options, Cancellation::default())
        {
            eprintln!("gotoLine: {e}");
        }
        cx.notify();
    }

    // ---------------------------------------------------------------- 超长行

    /// 给一个代码视图挂上截断标记装饰层（切换文档或换快照后重挂）。
    pub(super) fn attach_truncation_marks(
        &mut self,
        code: &Entity<CodeView>,
        cx: &mut Context<Self>,
    ) {
        // 颜色只用 `THEME_COLORS` 里已有的 `descriptionForeground`（不新造颜色键）。
        let color = self
            .theme
            .color(long_line::MARK_COLOR)
            .ok()
            .flatten()
            .or_else(|| {
                rove_theme::defaults::static_defaults(long_line::MARK_COLOR)
                    .and_then(|d| d.for_kind(self.theme_service.current().kind()))
            });
        let (text, limit) = {
            let view = code.read(cx);
            (view.text().cloned(), view.render_limit())
        };
        let Some(text) = text else {
            code.update(cx, |code, cx| {
                code.remove_decorations(&LayerId(MARKS_LAYER.into()), cx);
            });
            return;
        };
        let marks = Rc::new(TruncationMarks::new(text, limit, color, &self.strings));
        code.update(cx, |code, cx| {
            code.remove_decorations(&LayerId(MARKS_LAYER.into()), cx);
            let _ = code.add_decorations(MARKS_LAYER, priority::FIND_MATCH + 1, marks, cx);
        });
    }

    /// `rove.editor.openLongLine`：在只读临时标签里打开插入符所在的超长行。
    pub(super) fn open_long_line(&mut self, cx: &mut Context<Self>) {
        let code = self.code_view().clone();
        let line = code.read(cx).selection().active.line.0;
        self.open_long_line_at(line, cx);
    }

    pub(super) fn open_long_line_at(&mut self, line: u64, cx: &mut Context<Self>) {
        let Some(document) = self.active_large_file_document() else {
            return;
        };
        // 已经在临时标签里了：不再嵌套打开（VS Code 的临时编辑器也不会套娃）。
        if matches!(&document.resource, Resource::Virtual(name)
            if long_line::parse_resource(name).is_some())
        {
            return;
        }
        let truncated = self
            .code_view()
            .read(cx)
            .is_line_truncated(line)
            .unwrap_or(false);
        if !truncated {
            let position = self.code_view().read(cx).selection().active;
            let _ = shell::view::report(
                self,
                "largeFile.longLine.none",
                Reason::NoResult {
                    document: document.id,
                    position,
                },
                Message::key(long_line::NO_LONG_LINE_KEY),
                Arc::from([] as [FeedbackAction; 0]),
                cx,
            );
            return;
        }
        let source = self.large_files.long_lines.clone();
        let name = match source.prepare(
            document.id,
            &*document.text,
            line,
            long_line::FALLBACK_WRAP_COLUMNS,
            document.metadata.language.clone(),
        ) {
            Ok(name) => name,
            Err(e) => {
                eprintln!("openLongLine: {e}");
                return;
            }
        };
        let options = crate::documents::OpenOptions {
            source: crate::documents::OpenSource::CodeNavigation,
            preserve_focus: false,
            ..Default::default()
        };
        if let Err(e) =
            self.open_documents()
                .open(Resource::Virtual(name), options, Cancellation::default())
        {
            eprintln!("openLongLine: {e}");
        }
        cx.notify();
    }

    /// 代码视图报告点击了行内覆盖物：是截断标记就打开这一行。
    pub(super) fn truncation_overlay_clicked(&mut self, line: u64, cx: &mut Context<Self>) {
        self.open_long_line_at(line, cx);
    }

    /// 本 issue 负责的命令（`rove.editor.openLongLine`）。
    pub(super) fn large_file_command(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        if id != OPEN_LONG_LINE {
            return false;
        }
        self.open_long_line(cx);
        true
    }
}
