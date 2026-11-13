//! #60 侧边「引用」视图在骨架窗口中的装配：面板视图、16 条 `references-view.*` 命令、
//! 两个上下文键，以及 `references.preferredLocation` 为 `"view"` 时 `Shift+F12` 的去向。
//!
//! - 查询：经 #1 的 C# 导航查询接口（#38 的 `CsRouter`，与 #41 速览同一条后台流水线
//!   [`crate::peek::source::run`]），在后台执行器上跑；结果是 #41 的分组索引（文件 → 计数 + 偏移）。
//! - 结果树：#18 的虚拟化树 + #41 的 [`ResultsTree`] 数据源，展开某个文件时才读那个文件的片段。
//! - 打开结果：一律经 #17 的打开文档服务（`OpenSource::CodeNavigation`，带选区，所以 `.md` 也打开源码）。
use super::*;
use crate::{
    documents::OpenTarget,
    list_widget::{
        options::ListOptions,
        tree::{RowKind, TreeModel},
        view::{ListView, RowContext, RowRenderer},
        widget::ListEvent,
        StaticList,
    },
    lists::{ItemId, Row},
    peek::{
        model::{Node, PeekKind, PeekResults},
        results_tree::ResultsTree,
        snippets, source,
    },
    references::{
        model::{Entry, Mode},
        registry::{self as reg, context as ref_ctx, ReferencesAction},
        ReferencesState,
    },
};
use gpui::ClipboardItem;
use rove_commands::ContextKeys;
use rove_lang_cs::query::QueryPoint;
use std::rc::Rc;

/// 结果行高（与速览的结果树一致）。
const ROW_HEIGHT: f32 = 22.0;

/// 窗口里引用视图的状态。
#[derive(Default)]
pub(super) struct References {
    pub(super) state: ReferencesState,
    /// 当前结果树的数据源（片段按需读入）。
    tree: Option<Arc<ResultsTree>>,
    list: Option<Entity<ListView>>,
    /// `pickFromHistory` 的历史列表。
    history_list: Option<Entity<ListView>>,
    /// 正在跑的查询代次：新的查找作废旧的回调。
    epoch: u64,
    loading: bool,
    subscriptions: Vec<gpui::Subscription>,
    /// 上一次发布的 (`reference-list.hasResult`, `references-view.canNavigate`)。
    published: Option<(bool, bool)>,
}

impl Workbench {
    /// 测试与其他装配读取当前状态。
    pub fn references_state(&self) -> &ReferencesState {
        &self.references.state
    }

    /// #41 的装配调用的**唯一判定**：`editor.action.goToReferences` 是否改在本视图中展示。
    pub(super) fn references_prefer_view(&self) -> bool {
        reg::opens_in_view(&*self.settings)
    }

    /// 键位服务与命令面板分发来的 `references-view.*`。
    pub(super) fn references_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            reg::VIEW | reg::TREE_FOCUS | reg::TREE_RESET_VIEW_LOCATION => {
                self.show_references_panel(cx);
                if id != reg::VIEW {
                    self.focus_references(window, cx);
                }
            }
            reg::FIND_REFERENCES => self.find_references(PeekKind::References, window, cx),
            reg::FIND_IMPLEMENTATIONS => self.find_references(PeekKind::Implementation, window, cx),
            reg::NEXT => self.step_reference(true, cx),
            reg::PREV => self.step_reference(false, cx),
            reg::CLEAR => {
                self.references.state.clear();
                self.references.tree = None;
                self.references.list = None;
                cx.notify();
            }
            reg::CLEAR_HISTORY => {
                self.references.state.clear_history();
                self.references.tree = None;
                self.references.list = None;
                self.references.history_list = None;
                cx.notify();
            }
            reg::COPY => self.copy_reference(false, cx),
            reg::COPY_ALL => self.copy_reference(true, cx),
            reg::COPY_PATH => self.copy_reference_path(cx),
            reg::PICK_FROM_HISTORY => {
                if self.references.state.pick_from_history() {
                    self.show_references_panel(cx);
                    self.build_history_list(cx);
                    self.focus_references(window, cx);
                }
            }
            reg::REFIND => {
                // 历史列表里聚焦的那一条（不在历史模式时就是当前这一条）重新运行。
                let index = match self.references.state.mode {
                    Mode::History => self.references.state.history_focus,
                    Mode::Results => self.references.state.current.unwrap_or(0),
                };
                self.rerun_reference(index, cx);
            }
            reg::REFRESH => {
                let index = self.references.state.current.unwrap_or(0);
                self.rerun_reference(index, cx);
            }
            reg::REMOVE_REFERENCE_ITEM => self.remove_reference_item(cx),
            // 调用层级与类型层级（P2）只登记命令，行为为空。
            _ => {}
        }
        cx.notify();
    }

    /// 面板里显示本视图（`UX_SPEC` §1.4：与「问题」并列）。
    pub(super) fn show_references_panel(&mut self, cx: &mut Context<Self>) {
        let shell = &mut self.shell;
        shell.layout.panel_visible = true;
        shell.layout.active_panel_view = Some(reg::VIEW.into());
        shell.schedule_save(cx);
    }

    fn focus_references(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list = match self.references.state.mode {
            Mode::History => self.references.history_list.clone(),
            Mode::Results => self.references.list.clone(),
        };
        if let Some(list) = list {
            window.focus(&list.focus_handle(cx));
        }
    }

    // ------------------------------------------------------------------ 查找

    /// `shift+alt+f12`（引用）与 `references-view.findImplementations`（实现）：
    /// 在焦点代码视图的插入符处查找，结果放进本视图。
    pub(super) fn find_references(
        &mut self,
        kind: PeekKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let group = self.editor.focused;
        let secondary = self
            .group_code(group, true)
            .is_some_and(|code| code.focus_handle(cx).contains_focused(window, cx));
        let (Some(document), Some(code)) = (
            self.shown_document(group, secondary),
            self.group_code(group, secondary).cloned(),
        ) else {
            return;
        };
        let (caret, word) = code.read_with(cx, |c, _| (c.selection().active, c.word_at_caret()));
        let point = QueryPoint {
            resource: document.resource.clone(),
            position: caret,
            version: document.text.version(),
        };
        let symbol: Arc<str> = word
            .unwrap_or_else(|| self.label("peek.symbolHere", &[]))
            .into();
        self.run_reference_query(kind, symbol, point, cx);
    }

    /// 重新运行历史里的第 `index` 条（`refind` / `refresh`）。
    fn rerun_reference(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.references.state.history.get(index) else {
            return;
        };
        let (kind, symbol, point) = (entry.kind, entry.symbol.clone(), entry.point.clone());
        self.run_reference_query(kind, symbol, point, cx);
    }

    fn run_reference_query(
        &mut self,
        kind: PeekKind,
        symbol: Arc<str>,
        point: QueryPoint,
        cx: &mut Context<Self>,
    ) {
        self.show_references_panel(cx);
        self.references.epoch += 1;
        self.references.loading = true;
        let epoch = self.references.epoch;
        let queries = self.peek_queries();
        let documents = self.editor.loader();
        let task = cx.background_executor().spawn({
            let point = point.clone();
            async move {
                source::run(
                    queries,
                    documents,
                    kind,
                    point,
                    rove_base::Cancellation::default(),
                )
                .await
            }
        });
        cx.spawn(async move |this, cx| {
            let loaded = task.await;
            let _ = this.update(cx, |v, cx| {
                if v.references.epoch != epoch {
                    return;
                }
                v.references.loading = false;
                match loaded {
                    Ok(loaded) => {
                        let entry = Entry {
                            kind,
                            symbol,
                            point,
                            results: loaded.results.clone(),
                            precision: loaded.precision,
                        };
                        v.references.state.show(entry);
                        v.build_results_tree(loaded.snippets, cx);
                    }
                    Err(e) if source::is_silent(&e) => {}
                    Err(e) => {
                        // 查询失败：按“空结果”呈现，口径与速览一致（不能确定时追加“语义索引构建中”）。
                        let entry = Entry {
                            kind,
                            symbol,
                            point,
                            results: Arc::new(PeekResults::default()),
                            precision: source::error_precision(&e),
                        };
                        v.references.state.show(entry);
                        v.references.tree = None;
                        v.references.list = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------ 结果树

    fn build_results_tree(
        &mut self,
        loaded: Vec<(usize, Arc<snippets::FileSnippets>)>,
        cx: &mut Context<Self>,
    ) {
        let Some(results) = self.references.state.results().cloned() else {
            return;
        };
        self.references.subscriptions.clear();
        if results.is_empty() {
            self.references.tree = None;
            self.references.list = None;
            return;
        }
        let (send, receive) = async_channel::unbounded::<usize>();
        let tree = Arc::new(ResultsTree::new(results.clone(), loaded, send));
        let mut model = TreeModel::new(tree.clone());
        for file in results.initially_expanded() {
            let _ = model.set_expanded(&ItemId(Node::File(file).id()), true);
        }
        let mut options = ListOptions::read(&*self.settings);
        options.open_mode = crate::list_widget::options::OpenMode::SingleClick;
        let mut widget = ListWidget::tree(model, RowHeight::Fixed(ROW_HEIGHT), options);
        widget.supports_find = false;
        let theme = self.theme.clone();
        let strings = self.find_strings.clone();
        let renderer = self.results_renderer(tree.clone());
        let list = cx.new(|cx| {
            let mut view = ListView::new(widget, theme, strings, cx);
            view.set_renderer(renderer);
            view
        });
        let events = cx.subscribe(&list, |v, _, event: &ListEvent, cx| {
            v.references_list_event(event, cx)
        });
        self.references.subscriptions.push(events);
        self.listen_for_reference_expansions(receive, &tree, cx);
        self.references.tree = Some(tree);
        self.references.list = Some(list);
    }

    /// 展开了片段还没读的文件：后台读完后推给树（与速览同一套做法）。
    fn listen_for_reference_expansions(
        &self,
        receive: async_channel::Receiver<usize>,
        tree: &Arc<ResultsTree>,
        cx: &mut Context<Self>,
    ) {
        let tree = Arc::downgrade(tree);
        let documents = self.editor.loader();
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            while let Ok(file) = receive.recv().await {
                let Some(t) = tree.upgrade() else { break };
                if t.is_loaded(file) {
                    continue;
                }
                let (results, documents) = (t.results.clone(), documents.clone());
                drop(t);
                let cancel = rove_base::Cancellation::default();
                let loaded = background
                    .spawn(
                        async move { snippets::load(&results, file, &*documents, &cancel).await },
                    )
                    .await;
                let Ok(loaded) = loaded else { continue };
                let applied = this.update(cx, |v, cx| {
                    v.deliver_reference_snippets(file, Arc::new(loaded), cx)
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn deliver_reference_snippets(
        &mut self,
        file: usize,
        snippets: Arc<snippets::FileSnippets>,
        cx: &mut Context<Self>,
    ) {
        let (Some(tree), Some(list)) = (&self.references.tree, &self.references.list) else {
            return;
        };
        tree.store(file, snippets);
        let Some(items) = tree.items(file) else {
            return;
        };
        list.update(cx, |v, cx| {
            v.widget.update_tree(|t| {
                let _ = t.set_children(Some(&ItemId(Node::File(file).id())), items);
            });
            cx.notify();
        });
    }

    fn results_renderer(&self, tree: Arc<ResultsTree>) -> RowRenderer {
        let file_fg = self.ui_color("sideBar.foreground");
        let line_fg = self.ui_color("descriptionForeground");
        Rc::new(move |ctx: &RowContext, _, _| {
            if ctx.kind == RowKind::Loading {
                return div()
                    .text_color(line_fg)
                    .child(ctx.loading_text.clone().unwrap_or_default())
                    .into_any_element();
            }
            let twistie = |expandable: bool, expanded: bool| {
                let key = if !expandable {
                    None
                } else if expanded {
                    Some("list.twistie.expanded")
                } else {
                    Some("list.twistie.collapsed")
                };
                let text: SharedString = key
                    .and_then(|k| ctx.strings.text(&Message::key(k)).ok())
                    .unwrap_or_default()
                    .into();
                div().w(px(16.)).flex_shrink_0().child(text)
            };
            match Node::parse(&ctx.row.id.0) {
                Some(Node::File(file)) => {
                    let count = tree.results.files.get(file).map_or(0, |f| f.count);
                    div()
                        .flex()
                        .items_center()
                        .w_full()
                        .gap(px(4.))
                        .min_w_0()
                        .overflow_hidden()
                        .child(twistie(ctx.row.expandable, ctx.row.expanded))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(file_fg)
                                .child(SharedString::from(ctx.row.label.to_string())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_color(line_fg)
                                .child(SharedString::from(
                                    ctx.row.detail.as_deref().unwrap_or("").to_string(),
                                )),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .px(px(5.))
                                .text_color(line_fg)
                                .child(SharedString::from(count.to_string())),
                        )
                        .into_any_element()
                }
                Some(Node::Item(k)) => {
                    let line = tree.snippet(k).map_or(0, |s| s.line + 1);
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_w_0()
                        .overflow_hidden()
                        .pl(px(16.))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(line_fg)
                                .child(SharedString::from(line.to_string())),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .child(SharedString::from(ctx.row.label.to_string())),
                        )
                        .into_any_element()
                }
                None => div().into_any_element(),
            }
        })
    }

    fn references_list_event(&mut self, event: &ListEvent, cx: &mut Context<Self>) {
        match event {
            ListEvent::FocusChanged(Some(id)) => {
                if let Some(Node::Item(k)) = Node::parse(&id.0) {
                    self.references.state.selected = Some(k);
                }
            }
            ListEvent::Open { ids, .. } => {
                let Some(id) = ids.first() else { return };
                match Node::parse(&id.0) {
                    Some(Node::Item(k)) => {
                        self.references.state.selected = Some(k);
                        self.open_reference(k, cx);
                    }
                    _ => return,
                }
            }
            _ => return,
        }
        cx.notify();
    }

    // ------------------------------------------------------------------ 历史列表

    fn build_history_list(&mut self, cx: &mut Context<Self>) {
        let rows: Vec<Row> = self
            .references
            .state
            .history
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let label = self.strings.text(&e.title()).unwrap_or_default();
                Row {
                    id: ItemId(i.to_string().into()),
                    label: label.into(),
                    detail: None,
                    depth: 0,
                    expandable: false,
                    expanded: false,
                    disabled: false,
                }
            })
            .collect();
        let focus = self.references.state.history_focus;
        let options = ListOptions::read(&*self.settings);
        let theme = self.theme.clone();
        let strings = self.find_strings.clone();
        let list = cx.new(|cx| {
            let mut widget = ListWidget::new(
                Arc::new(StaticList::new(rows, Version(0))),
                RowHeight::Fixed(ROW_HEIGHT),
                options,
            );
            widget.set_focus(Some(focus));
            widget.take_events();
            ListView::new(widget, theme, strings, cx)
        });
        let events = cx.subscribe(&list, |v, _, event: &ListEvent, cx| match event {
            ListEvent::FocusChanged(Some(id)) => {
                if let Ok(i) = id.0.parse::<usize>() {
                    v.references.state.history_focus = i;
                }
            }
            ListEvent::Open { ids, .. } => {
                if let Some(i) = ids.first().and_then(|i| i.0.parse::<usize>().ok()) {
                    v.references.state.restore(i);
                    v.rebuild_current_tree(cx);
                    cx.notify();
                }
            }
            _ => {}
        });
        self.references.subscriptions.push(events);
        self.references.history_list = Some(list);
    }

    /// 从历史里恢复一条结果时重建树（片段重新按需读取）。
    fn rebuild_current_tree(&mut self, cx: &mut Context<Self>) {
        self.build_results_tree(Vec::new(), cx);
    }

    // ------------------------------------------------------------------ 导航与命令

    fn step_reference(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(item) = self.references.state.step(forward) else {
            return;
        };
        if let Some(list) = self.references.list.clone() {
            let id = ItemId(Node::Item(item).id());
            list.update(cx, |v, cx| {
                let _ = v.widget.focus_id(&id);
                v.widget.select_ids(std::slice::from_ref(&id));
                v.widget.take_events();
                cx.notify();
            });
        }
        self.open_reference(item, cx);
    }

    /// 打开一条结果：经 #17 的打开文档服务，带选区（`.md` 也打开源码并定位）。
    fn open_reference(&mut self, item: usize, cx: &mut Context<Self>) {
        let Some((resource, selection)) = self.references.state.location(item) else {
            return;
        };
        let group = self.editor.focused;
        let options = OpenOptions {
            target: OpenTarget::Group(group),
            source: OpenSource::CodeNavigation,
            preserve_focus: true,
            selection: Some(selection),
            ..OpenOptions::default()
        };
        if let Err(e) = self
            .open_documents()
            .open(resource, options, Cancellation::default())
        {
            eprintln!("references: {e}");
        }
        cx.notify();
    }

    fn remove_reference_item(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.references.state.selected else {
            return;
        };
        if self.references.state.remove_item(item) {
            self.rebuild_current_tree(cx);
            cx.notify();
        }
    }

    fn copy_reference(&mut self, all: bool, cx: &mut Context<Self>) {
        let Some(results) = self.references.state.results().cloned() else {
            return;
        };
        let items: Vec<usize> = if all {
            (0..results.total()).collect()
        } else {
            self.references.state.selected.into_iter().collect()
        };
        if items.is_empty() {
            return;
        }
        let text = items
            .into_iter()
            .filter_map(|i| crate::references::model::copy_args(&results, i))
            .map(|args| self.label("references.copy.item", &args))
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn copy_reference_path(&mut self, cx: &mut Context<Self>) {
        let (Some(results), Some(item)) = (
            self.references.state.results().cloned(),
            self.references.state.selected,
        ) else {
            return;
        };
        if let Some(path) = crate::references::model::copy_path(&results, item) {
            cx.write_to_clipboard(ClipboardItem::new_string(path));
        }
    }

    // ------------------------------------------------------------------ 绘制与每帧

    pub(super) fn render_references(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let fg = self.ui_color("sideBar.foreground");
        let description = self.ui_color("descriptionForeground");
        let message = |text: String| {
            div()
                .px_4()
                .py_1()
                .text_color(description)
                .child(SharedString::from(text))
                .into_any_element()
        };
        let header = self.references.state.entry().map(|entry| {
            let title = self.strings.text(&entry.title()).unwrap_or_default();
            let marker = entry
                .marker()
                .and_then(|m| self.strings.text(&m).ok())
                .unwrap_or_default();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px_4()
                .py_1()
                .flex_shrink_0()
                .text_color(fg)
                .child(SharedString::from(title))
                .when(!marker.is_empty(), |d| {
                    d.child(
                        div()
                            .text_color(description)
                            .child(SharedString::from(marker)),
                    )
                })
                .into_any_element()
        });
        let body = match self.references.state.mode {
            Mode::History => match self.references.history_list.clone() {
                Some(list) => div().flex_1().min_h_0().child(list).into_any_element(),
                None => message(self.label("references.history.empty", &[])),
            },
            Mode::Results => {
                if self.references.loading {
                    message(self.label("references.loading", &[]))
                } else if let Some(list) = self.references.list.clone() {
                    div().flex_1().min_h_0().child(list).into_any_element()
                } else if let Some(entry) = self.references.state.entry() {
                    let hint = entry.empty_hint();
                    message(self.strings.text(&hint).unwrap_or_default())
                } else {
                    message(self.label("references.noResult", &[]))
                }
            }
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .children(header)
            .child(body)
            .into_any_element()
    }

    /// 每帧：两个上下文键跟着状态走（取值变了才写，避免每帧通知）。
    pub(super) fn sync_references(&mut self, cx: &mut Context<Self>) {
        let now = (
            self.references.state.has_result(),
            self.references.state.can_navigate(),
        );
        if self.references.published == Some(now) {
            return;
        }
        self.references.published = Some(now);
        if let Some(keys) = cx.try_global::<crate::keys::Keys>() {
            let _ = keys
                .context
                .set(ref_ctx::HAS_RESULT.into(), Some(Value::Bool(now.0)));
            let _ = keys
                .context
                .set(ref_ctx::CAN_NAVIGATE.into(), Some(Value::Bool(now.1)));
        }
    }

    pub(super) fn references_action_handler(
        &mut self,
        action: &ReferencesAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.references_command(action.0, window, cx);
    }

    /// 测试读取：当前结果树里每个文件的 (文件名, 结果数)。
    #[cfg(test)]
    pub(super) fn references_groups(&self) -> Vec<(String, usize)> {
        self.references
            .state
            .results()
            .map(|r| {
                r.files
                    .iter()
                    .map(|f| (f.label.to_string(), f.count))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn references_loading(&self) -> bool {
        self.references.loading
    }

    /// 测试读取：结果树里当前的行数（展开前应当只有文件节点）。
    #[cfg(test)]
    pub(super) fn references_tree_rows(&self, cx: &App) -> usize {
        self.references
            .list
            .as_ref()
            .map_or(0, |l| l.read(cx).widget.len())
    }
}
