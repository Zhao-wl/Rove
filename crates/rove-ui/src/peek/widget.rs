//! 速览窗口（GPUI）：标题栏、左侧只读预览（#19 的代码视图）、右侧结果树（#18 的树组件）。
//!
//! 窗口本身不知道自己挂在哪里：宿主（主编辑器或上一层速览的预览）用 [`super::zone`] 把它摆进
//! 嵌入区域，并处理它发出的 [`PeekEvent`]。嵌套速览（在预览里 `Alt+F12`）由窗口自己在预览的
//! 嵌入区域里再开一层，形成窗口栈；`Esc` 逐层关闭。
//!
//! # 线程
//!
//! 查询、分组、读片段、读预览文档、预览的语法分析都在后台执行器上；回到界面后按代次丢弃过期结果。
//! 窗口关闭（实体释放）时取消全部后台工作，并释放结果数据与预览文档。

use super::external::{ExternalSnippets, Resolved};
use super::model::{self, Marker, Node, PeekKind, PeekResults};
use super::registry::{self as reg, context, DefaultFocus, PeekAction, PeekSettings};
use super::results_tree::ResultsTree;
use super::snippets;
use super::source::{self, Loaded, PeekQueries};
use super::zone::zone_overlay;
use crate::code_view::{
    decorations::{DecorationProvider, InlineDecoration, LineDecorations, TextStyle},
    view::hsla,
    CodeView, CodeViewServices, Reveal, Selection,
};
use crate::list_widget::{
    options::ListOptions,
    tree::{RowKind, TreeModel},
    view::{ListView, RowContext, RowRenderer},
    widget::{ListEvent, ListWidget, RowHeight},
};
use crate::lists::ItemId;
use gpui::{
    canvas, div, prelude::*, px, relative, App, Context, Entity, EventEmitter, FocusHandle,
    Focusable, HighlightStyle, Hsla, KeyContext, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Render, SharedString, StyledText, Subscription, Window,
};
use rove_base::{Cancellation, Events, Resource, Result, TextRange, Version};
use rove_buffer::{Document, Documents, ReadOnlyReason, RopeText, TextSource};
use rove_config::Settings;
use rove_i18n::{Message, Strings};
use rove_lang_cs::query::{Hover, Precision, QueryPoint};
use rove_syntax::SyntaxService;
use rove_theme::{Color, Theme, ThemeKind, TokenStyle};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// VS Code `ReferenceWidget` 的默认布局：高 18 行，预览占 70%。
pub const DEFAULT_HEIGHT_LINES: f32 = 18.0;
pub const DEFAULT_RATIO: f32 = 0.7;
const MIN_HEIGHT_LINES: f32 = 4.0;
const MAX_HEIGHT_LINES: f32 = 60.0;
/// 300 ms 内没有结果才显示“正在加载…”（UX_SPEC §6.2）。
pub const LOADING_DELAY: Duration = Duration::from_millis(300);
const ROW_HEIGHT: f32 = 22.0;
const SASH: f32 = 4.0;
const LAYER: &str = "peek.matches";

/// 窗口需要的服务（窗口栈各层共享）。
pub struct PeekServices {
    pub queries: Option<Arc<dyn PeekQueries>>,
    pub documents: Arc<dyn Documents>,
    pub settings: Arc<dyn Settings>,
    pub theme: Arc<dyn Theme>,
    pub theme_kind: ThemeKind,
    pub strings: Arc<dyn Strings>,
    pub external: Arc<ExternalSnippets>,
    /// 预览的语法分析（与主编辑器同一个 tree-sitter 服务；无状态调用，不动主编辑器的基线）。
    pub syntax: Arc<dyn SyntaxService>,
}

impl PeekServices {
    fn text(&self, message: &Message) -> String {
        self.strings
            .text(message)
            .unwrap_or_else(|_| message.key.to_string())
    }
}

/// 一次打开速览的请求。
#[derive(Clone)]
pub struct PeekRequest {
    pub kind: PeekKind,
    pub point: QueryPoint,
    /// 标题里的符号名（查询点处的单词）。
    pub symbol: Arc<str>,
    /// 近似标记；#40 在多候选定义时传入自己的文案。
    pub marker: Marker,
    /// 已经算好的结果（#40 多候选定义时直接交给速览，不再查询）。
    pub preset: Option<Arc<PeekResults>>,
}

/// 窗口发给宿主的事件。
#[derive(Clone, Debug)]
pub enum PeekEvent {
    /// 在编辑器中打开并定位（来源“跳转 / 引用”）。`side` 为 false 时宿主随后关闭整个速览。
    Open {
        resource: Resource,
        selection: TextRange,
        side: bool,
    },
    /// 关闭这一层（`Esc`、关闭按钮）。
    Close,
    /// 关闭这一层，并在光标处提示（结果为空、实现不可用、查询失败）。
    Empty(Message),
    /// 光标处提示，不关闭。
    Hint(Message),
    /// 用户拖动了底边：新的高度（像素）。
    Resized(f32),
}

/// 主题适配：结果树用 `peekViewResult.*`、预览用 `peekViewEditor*`（VS Code 同样这样映射）。
struct PeekTheme {
    inner: Arc<dyn Theme>,
    kind: ThemeKind,
}

impl PeekTheme {
    fn mapped(id: &str) -> Option<&'static str> {
        Some(match id {
            "list.activeSelectionBackground"
            | "list.inactiveSelectionBackground"
            | "list.focusBackground" => "peekViewResult.selectionBackground",
            "list.activeSelectionForeground"
            | "list.inactiveSelectionForeground"
            | "list.focusForeground" => "peekViewResult.selectionForeground",
            "sideBar.foreground" => "peekViewResult.lineForeground",
            "editor.background" => "peekViewEditor.background",
            "editorGutter.background" => "peekViewEditorGutter.background",
            _ => return None,
        })
    }
}

impl Theme for PeekTheme {
    fn color(&self, id: &str) -> Result<Option<Color>> {
        match Self::mapped(id) {
            Some(key) => {
                match self.inner.color(key) {
                    Ok(c) => Ok(c),
                    Err(_) => Ok(rove_theme::defaults::static_defaults(key)
                        .and_then(|d| d.for_kind(self.kind))),
                }
            }
            None => self.inner.color(id),
        }
    }
    fn token_style(&self, scopes: &[Arc<str>]) -> TokenStyle {
        self.inner.token_style(scopes)
    }
    fn changes(&self) -> &dyn Events<Version> {
        self.inner.changes()
    }
}

/// 窗口的调色板。
#[derive(Clone, Copy)]
struct Colors {
    border: Hsla,
    title_bg: Hsla,
    title_label: Hsla,
    title_description: Hsla,
    result_bg: Hsla,
    file_fg: Hsla,
    line_fg: Hsla,
    result_match: Option<Hsla>,
    editor_match: Option<Color>,
    editor_match_border: Option<Color>,
}

impl Colors {
    fn resolve(theme: &dyn Theme, kind: ThemeKind) -> Self {
        let get = |key: &str| match theme.color(key) {
            Ok(c) => c,
            Err(_) => rove_theme::defaults::static_defaults(key).and_then(|d| d.for_kind(kind)),
        };
        let h = |key: &str, fallback: Color| hsla(get(key).unwrap_or(fallback));
        Self {
            border: h("peekView.border", Color(0x35, 0x94, 0xd2, 255)),
            title_bg: h("peekViewTitle.background", Color(0x25, 0x25, 0x26, 255)),
            title_label: h(
                "peekViewTitleLabel.foreground",
                Color(0xff, 0xff, 0xff, 255),
            ),
            title_description: h(
                "peekViewTitleDescription.foreground",
                Color(0xcc, 0xcc, 0xcc, 0xb3),
            ),
            result_bg: h("peekViewResult.background", Color(0x25, 0x25, 0x26, 255)),
            file_fg: h(
                "peekViewResult.fileForeground",
                Color(0xff, 0xff, 0xff, 255),
            ),
            line_fg: h(
                "peekViewResult.lineForeground",
                Color(0xbb, 0xbb, 0xbb, 255),
            ),
            result_match: get("peekViewResult.matchHighlightBackground").map(hsla),
            editor_match: get("peekViewEditor.matchHighlightBackground"),
            editor_match_border: get("peekViewEditor.matchHighlightBorder"),
        }
    }
}

/// 预览里匹配区间的装饰（只为进入视口的行按需生成：二分定位本行的结果）。
struct Matches {
    /// 当前预览文件里的结果区间，按位置排序。
    ranges: RefCell<Vec<TextRange>>,
    style: RefCell<TextStyle>,
}

impl DecorationProvider for Matches {
    fn decorate(&self, line: u64, text: &str, out: &mut LineDecorations) {
        let ranges = self.ranges.borrow();
        let style = *self.style.borrow();
        let first = ranges.partition_point(|r| r.end().line.0 < line);
        for r in ranges[first..]
            .iter()
            .take_while(|r| r.start().line.0 <= line)
        {
            let start = if r.start().line.0 == line {
                r.start().column.0 as u32
            } else {
                0
            };
            let end = if r.end().line.0 == line {
                r.end().column.0 as u32
            } else {
                text.len() as u32
            };
            if end > start {
                out.inline.push(InlineDecoration {
                    range: start..end,
                    style,
                });
            }
        }
    }
}

enum Phase {
    Loading,
    Ready,
}

/// 预览当前显示的内容。
#[derive(Clone, PartialEq)]
enum Shown {
    File(usize),
    External(usize),
}

struct Nested {
    anchor: u64,
    widget: Entity<PeekWidget>,
    _events: Subscription,
}

#[derive(Clone, Copy)]
enum Drag {
    Height {
        start_y: f32,
        start: f32,
    },
    Ratio {
        start_x: f32,
        start: f32,
        width: f32,
    },
}

pub struct PeekWidget {
    services: Rc<PeekServices>,
    request: PeekRequest,
    settings: PeekSettings,
    colors: Colors,
    phase: Phase,
    loading_visible: bool,
    results: Option<Arc<PeekResults>>,
    tree: Option<Arc<ResultsTree>>,
    precision: Precision,
    hover: Option<Hover>,
    list: Option<Entity<ListView>>,
    preview: Entity<CodeView>,
    preview_theme: Arc<dyn Theme>,
    matches: Rc<Matches>,
    shown: Option<Shown>,
    /// 预览文档（只在显示时持有，关闭窗口即释放）。
    document: Option<Arc<Document>>,
    preview_generation: u64,
    current: Option<usize>,
    /// 跳到的结果所在文件还没读片段：读好后再把树的焦点放上去。
    pending_focus: Option<usize>,
    line_height: f32,
    height_lines: f32,
    ratio: f32,
    body_width: f32,
    nested: Option<Nested>,
    focus: FocusHandle,
    drag: Option<Drag>,
    cancel: Cancellation,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PeekEvent> for PeekWidget {}

impl Drop for PeekWidget {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Focusable for PeekWidget {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PeekWidget {
    /// 新窗口立即出现（加载状态），查询在后台进行。`line_height` 是宿主编辑器的行高。
    pub fn new(
        services: Rc<PeekServices>,
        request: PeekRequest,
        line_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = PeekSettings::read(&*services.settings);
        let colors = Colors::resolve(&*services.theme, services.theme_kind);
        let preview_theme: Arc<dyn Theme> = Arc::new(PeekTheme {
            inner: services.theme.clone(),
            kind: services.theme_kind,
        });
        let matches = Rc::new(Matches {
            ranges: RefCell::new(Vec::new()),
            style: RefCell::new(TextStyle {
                background: colors.editor_match,
                border: colors.editor_match_border,
                ..TextStyle::default()
            }),
        });
        let preview = {
            let (settings, theme, kind) = (
                services.settings.clone(),
                preview_theme.clone(),
                services.theme_kind,
            );
            let provider: Rc<dyn DecorationProvider> = matches.clone();
            cx.new(|cx| {
                let mut code = CodeView::new(
                    CodeViewServices {
                        settings,
                        theme,
                        theme_kind: kind,
                    },
                    cx,
                );
                // 左侧预览只读（UX_SPEC §2.3 与 VS Code 的差异项）。
                code.set_read_only(Arc::from([ReadOnlyReason::Session]), cx);
                let _ = code.add_decorations(
                    LAYER,
                    crate::code_view::priority::FIND_MATCH - 1,
                    provider,
                    cx,
                );
                code
            })
        };
        // 只读预览里尝试输入只会发出 `EditBlocked`；没有订阅者，内容不变，也不在主编辑器上弹提示。
        let mut widget = Self {
            services,
            request,
            settings,
            colors,
            phase: Phase::Loading,
            loading_visible: false,
            results: None,
            tree: None,
            precision: Precision::Exact,
            hover: None,
            list: None,
            preview,
            preview_theme,
            matches,
            shown: None,
            document: None,
            preview_generation: 0,
            current: None,
            pending_focus: None,
            line_height,
            height_lines: DEFAULT_HEIGHT_LINES,
            ratio: DEFAULT_RATIO,
            body_width: 0.0,
            nested: None,
            focus: cx.focus_handle(),
            drag: None,
            cancel: Cancellation::default(),
            _subscriptions: Vec::new(),
        };
        widget.start(window, cx);
        widget
    }

    // ------------------------------------------------------------------ 只读访问（测试与宿主）

    pub fn kind(&self) -> PeekKind {
        self.request.kind
    }
    pub fn results(&self) -> Option<&Arc<PeekResults>> {
        self.results.as_ref()
    }
    pub fn is_loading(&self) -> bool {
        matches!(self.phase, Phase::Loading)
    }
    pub fn loading_visible(&self) -> bool {
        self.loading_visible
    }
    pub fn current(&self) -> Option<usize> {
        self.current
    }
    pub fn preview(&self) -> &Entity<CodeView> {
        &self.preview
    }
    pub fn list(&self) -> Option<&Entity<ListView>> {
        self.list.as_ref()
    }
    pub fn nested(&self) -> Option<&Entity<PeekWidget>> {
        self.nested.as_ref().map(|n| &n.widget)
    }
    /// 窗口栈深度（本层算 1）。
    pub fn depth(&self, cx: &App) -> usize {
        1 + self.nested().map_or(0, |n| n.read(cx).depth(cx))
    }
    /// 窗口高度（像素）。
    pub fn height(&self) -> f32 {
        (self.height_lines * self.line_height).round()
    }
    /// 标题（符号名 + 结果数）与近似标记，已解析成文字。
    pub fn title_text(&self) -> (String, Option<String>) {
        let s = &self.services;
        match &self.results {
            Some(r) => (
                s.text(&model::title(
                    self.request.kind,
                    &self.request.symbol,
                    r.total(),
                )),
                model::marker_text(&self.request.marker, self.request.kind, r).map(|m| s.text(&m)),
            ),
            None => (self.request.symbol.to_string(), None),
        }
    }
    /// 预览里显示的是外部符号片段时的全文（测试用）。
    pub fn preview_text(&self, cx: &App) -> Option<String> {
        let code = self.preview.read(cx);
        let text = code.text()?;
        let n = text.lines().known;
        Some(
            (0..n)
                .filter_map(|l| snippets::line_text(&**text, l))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
    pub fn preview_resource(&self) -> Option<Resource> {
        self.document.as_ref().map(|d| d.resource.clone())
    }

    // ------------------------------------------------------------------ 查询

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cancel = self.cancel.clone();
        if let Some(preset) = self.request.preset.clone() {
            let documents = self.services.documents.clone();
            let task = cx.background_executor().spawn(async move {
                let mut loaded = Vec::new();
                for file in preset.initially_expanded() {
                    let s = snippets::load(&preset, file, &*documents, &cancel).await?;
                    loaded.push((file, Arc::new(s)));
                }
                Ok(Loaded {
                    precision: preset.precision(),
                    reason: None,
                    results: preset,
                    snippets: loaded,
                    hover: None,
                })
            });
            self.await_results(task, window, cx);
            return;
        }
        let Some(queries) = self.services.queries.clone() else {
            let empty =
                model::empty_hint(self.request.kind, &self.request.symbol, Precision::Exact);
            cx.defer_in(window, move |_, _, cx| cx.emit(PeekEvent::Empty(empty)));
            return;
        };
        let task = cx.background_executor().spawn(source::run(
            queries,
            self.services.documents.clone(),
            self.request.kind,
            self.request.point.clone(),
            cancel,
        ));
        self.await_results(task, window, cx);
    }

    fn await_results(
        &mut self,
        task: gpui::Task<Result<Loaded>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let timer = cx.background_executor().timer(LOADING_DELAY);
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |w, cx| {
                if w.is_loading() {
                    w.loading_visible = true;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |w, window, cx| w.apply(result, window, cx));
        })
        .detach();
    }

    fn apply(&mut self, result: Result<Loaded>, window: &mut Window, cx: &mut Context<Self>) {
        let loaded = match result {
            Ok(l) => l,
            Err(e) if source::is_silent(&e) => return,
            Err(e) => {
                cx.emit(PeekEvent::Empty(model::empty_hint(
                    self.request.kind,
                    &self.request.symbol,
                    source::error_precision(&e),
                )));
                return;
            }
        };
        if loaded.results.is_empty() {
            cx.emit(PeekEvent::Empty(model::empty_hint(
                self.request.kind,
                &self.request.symbol,
                loaded.precision,
            )));
            return;
        }
        self.phase = Phase::Ready;
        self.loading_visible = false;
        self.precision = loaded.precision;
        self.hover = loaded.hover;
        let results = loaded.results;
        let (send, receive) = async_channel::unbounded::<usize>();
        let tree = Arc::new(ResultsTree::new(results.clone(), loaded.snippets, send));
        self.build_list(&tree, window, cx);
        self.listen_for_expansions(receive, &tree, cx);
        self.results = Some(results.clone());
        self.tree = Some(tree);
        // 初始选中：查询点处的那一条（引用）或第一条；它所在的文件没展开时选第一条。
        let first = 0;
        let nearest = (self.request.kind == PeekKind::References)
            .then(|| results.nearest(&self.request.point.resource, self.request.point.position))
            .flatten()
            .filter(|k| results.initially_expanded().contains(&results.file_of(*k)))
            .unwrap_or(first);
        self.select(nearest, window, cx);
        match self.settings.default_focus {
            DefaultFocus::Tree => self.focus_tree(window, cx),
            DefaultFocus::Editor => self.focus_preview(window, cx),
        }
        cx.notify();
    }

    fn build_list(&mut self, tree: &Arc<ResultsTree>, window: &mut Window, cx: &mut Context<Self>) {
        let mut model = TreeModel::new(tree.clone());
        for file in tree.results.initially_expanded() {
            let _ = model.set_expanded(&ItemId(Node::File(file).id()), true);
        }
        let mut options = ListOptions::read(&*self.services.settings);
        // 单击预览、双击打开（VS Code 引用树的行为），不随 workbench.list.openMode 变化。
        options.open_mode = crate::list_widget::options::OpenMode::SingleClick;
        let mut widget = ListWidget::tree(model, RowHeight::Fixed(ROW_HEIGHT), options);
        widget.supports_find = false;
        let theme: Arc<dyn Theme> = Arc::new(PeekTheme {
            inner: self.services.theme.clone(),
            kind: self.services.theme_kind,
        });
        let strings = self.services.strings.clone();
        let renderer = self.renderer(tree.clone());
        let list = cx.new(|cx| {
            let mut view = ListView::new(widget, theme, strings, cx);
            view.set_renderer(renderer);
            view
        });
        let events = cx.subscribe_in(&list, window, |w, _, e: &ListEvent, window, cx| {
            w.list_event(e, window, cx)
        });
        self._subscriptions.push(events);
        self.list = Some(list);
    }

    /// 展开了片段还没读的文件：后台读完后推给树。
    fn listen_for_expansions(
        &self,
        receive: async_channel::Receiver<usize>,
        tree: &Arc<ResultsTree>,
        cx: &mut Context<Self>,
    ) {
        let tree = Arc::downgrade(tree);
        let documents = self.services.documents.clone();
        let cancel = self.cancel.clone();
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            while let Ok(file) = receive.recv().await {
                let Some(t) = tree.upgrade() else { break };
                if t.is_loaded(file) {
                    continue;
                }
                let (results, documents, c) =
                    (t.results.clone(), documents.clone(), cancel.clone());
                drop(t);
                let loaded = background
                    .spawn(async move { snippets::load(&results, file, &*documents, &c).await })
                    .await;
                let Ok(loaded) = loaded else { continue };
                let applied = this.update(cx, |w, cx| w.deliver(file, Arc::new(loaded), cx));
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn deliver(
        &mut self,
        file: usize,
        snippets: Arc<snippets::FileSnippets>,
        cx: &mut Context<Self>,
    ) {
        let (Some(tree), Some(list)) = (&self.tree, &self.list) else {
            return;
        };
        tree.store(file, snippets);
        let Some(items) = tree.items(file) else {
            return;
        };
        let pending = self.pending_focus.take();
        list.update(cx, |v, cx| {
            v.widget.update_tree(|t| {
                let _ = t.set_children(Some(&ItemId(Node::File(file).id())), items);
            });
            if let Some(k) = pending {
                let _ = v.widget.focus_id(&ItemId(Node::Item(k).id()));
                v.widget.select_ids(&[ItemId(Node::Item(k).id())]);
            }
            cx.notify();
        });
    }

    // ------------------------------------------------------------------ 结果树

    fn renderer(&self, tree: Arc<ResultsTree>) -> RowRenderer {
        let colors = self.colors;
        Rc::new(move |ctx: &RowContext, _, _| {
            if ctx.kind == RowKind::Loading {
                return div()
                    .text_color(colors.line_fg)
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
                    let count = ctx
                        .strings
                        .text(&Message {
                            key: "peek.fileCount".into(),
                            args: Arc::from([("count".into(), count.to_string().into())]),
                        })
                        .unwrap_or_else(|_| count.to_string());
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
                                .text_color(colors.file_fg)
                                .child(SharedString::from(ctx.row.label.to_string())),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_color(colors.line_fg)
                                .child(SharedString::from(
                                    ctx.row.detail.as_deref().unwrap_or("").to_string(),
                                )),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .px(px(5.))
                                .rounded(px(9.))
                                .text_color(colors.line_fg)
                                .child(count),
                        )
                        .into_any_element()
                }
                Some(Node::Item(k)) => {
                    let snippet = tree.snippet(k);
                    let line = snippet.as_ref().map_or(0, |s| s.line + 1);
                    let line = ctx
                        .strings
                        .text(&Message {
                            key: "peek.lineNumber".into(),
                            args: Arc::from([("line".into(), line.to_string().into())]),
                        })
                        .unwrap_or_else(|_| line.to_string());
                    let label: SharedString = ctx.row.label.to_string().into();
                    let mut text = StyledText::new(label.clone());
                    if let (Some(s), Some(bg)) = (&snippet, colors.result_match) {
                        let h = s.highlight.clone();
                        if h.end <= label.len()
                            && label.is_char_boundary(h.start)
                            && label.is_char_boundary(h.end)
                            && h.start < h.end
                        {
                            text = text.with_highlights([(
                                h,
                                HighlightStyle {
                                    background_color: Some(bg),
                                    ..Default::default()
                                },
                            )]);
                        }
                    }
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
                                .text_color(colors.line_fg)
                                .child(SharedString::from(line)),
                        )
                        .child(div().min_w_0().overflow_hidden().child(text))
                        .into_any_element()
                }
                None => div().into_any_element(),
            }
        })
    }

    fn list_event(&mut self, event: &ListEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ListEvent::FocusChanged(Some(id)) => match Node::parse(&id.0) {
                Some(Node::Item(k)) => self.show(k, window, cx),
                // 选中文件节点：预览定位到该文件的第一条结果。
                Some(Node::File(f)) => {
                    if let Some(start) = self
                        .results
                        .as_ref()
                        .and_then(|r| r.files.get(f))
                        .map(|g| g.start)
                    {
                        self.show(start, window, cx);
                    }
                }
                None => {}
            },
            ListEvent::Open {
                ids,
                pinned,
                side_by_side,
                ..
            } => {
                let Some(k) = ids.first().and_then(|id| self.item_of(id)) else {
                    return;
                };
                if *side_by_side {
                    self.open(k, true, cx);
                } else if *pinned {
                    // 双击结果：与 `Enter` 相同。
                    self.open(k, false, cx);
                } else {
                    self.show(k, window, cx);
                }
            }
            _ => {}
        }
    }

    /// 树节点对应的结果：结果行本身，或文件节点的第一条。
    fn item_of(&self, id: &ItemId) -> Option<usize> {
        match Node::parse(&id.0)? {
            Node::Item(k) => Some(k),
            Node::File(f) => self.results.as_ref()?.files.get(f).map(|g| g.start),
        }
    }

    /// 选中第 `k` 条：树的焦点与选择、预览。
    fn select(&mut self, k: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(results) = self.results.clone() else {
            return;
        };
        let file = results.file_of(k);
        let ready = self.tree.as_ref().is_some_and(|t| t.is_loaded(file));
        if let Some(list) = &self.list {
            list.update(cx, |v, cx| {
                let file_id = ItemId(Node::File(file).id());
                let _ = v.widget.expand_id(&file_id, true);
                let id = ItemId(Node::Item(k).id());
                if ready {
                    let _ = v.widget.focus_id(&id);
                    v.widget.select_ids(&[id]);
                }
                cx.notify();
            });
        }
        self.pending_focus = (!ready).then_some(k);
        self.show(k, window, cx);
    }

    // ------------------------------------------------------------------ 预览

    fn show(&mut self, k: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(results) = self.results.clone() else {
            return;
        };
        let Some(item) = results.items.get(k) else {
            return;
        };
        self.current = Some(k);
        let file = item.file as usize;
        let group = &results.files[file];
        let wanted = if group.is_external() {
            Shown::External(k)
        } else {
            Shown::File(file)
        };
        if self.shown.as_ref() == Some(&wanted) {
            self.reveal_current(cx);
            return;
        }
        self.shown = Some(wanted.clone());
        self.preview_generation += 1;
        let generation = self.preview_generation;
        let cancel = self.cancel.clone();
        match wanted {
            Shown::File(file) => {
                let resource = group.resource.clone();
                let documents = self.services.documents.clone();
                let task = cx.background_executor().spawn(async move {
                    match documents.find(&resource) {
                        Some(d) => Ok(d),
                        None => documents.open(resource, cancel).await,
                    }
                });
                *self.matches.ranges.borrow_mut() = results.items[results.files[file].items()]
                    .iter()
                    .map(|i| i.range.clone())
                    .collect();
                cx.spawn_in(window, async move |this, cx| {
                    let document = task.await;
                    let _ = this.update_in(cx, |w, _, cx| {
                        if w.preview_generation == generation {
                            if let Ok(d) = document {
                                w.set_preview_document(d, cx);
                            }
                        }
                    });
                })
                .detach();
            }
            Shown::External(k) => {
                let target = rove_lang_cs::query::Target {
                    resource: group.resource.clone(),
                    range: item.range.clone(),
                    selection: item.range.clone(),
                };
                let key = rove_lang_cs::router::metadata_symbol_key(&group.resource)
                    .unwrap_or_default()
                    .to_string();
                let signature = source::external_signature(self.hover.as_ref(), &key);
                let (external, hover) = (self.services.external.clone(), self.hover.clone());
                let task = cx.background_executor().spawn(async move {
                    external.resolve(&target, hover.as_ref(), signature, &cancel)
                });
                self.matches.ranges.borrow_mut().clear();
                let _ = k;
                cx.spawn_in(window, async move |this, cx| {
                    let resolved = task.await;
                    let _ = this.update_in(cx, |w, _, cx| {
                        if w.preview_generation == generation {
                            w.set_preview_external(resolved, cx);
                        }
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }

    fn set_preview_document(&mut self, document: Arc<Document>, cx: &mut Context<Self>) {
        let text = document.text.clone();
        let language = document.metadata.language.clone();
        self.document = Some(document);
        self.set_preview_text(text, language, cx);
        self.reveal_current(cx);
    }

    fn set_preview_external(&mut self, resolved: Resolved, cx: &mut Context<Self>) {
        self.document = None;
        let (text, language, focus) = match resolved {
            Resolved::Provided(s) => (s.text.to_string(), s.language, s.focus),
            Resolved::Fallback { signature } => (
                self.services.text(&Message {
                    key: "peek.external".into(),
                    args: Arc::from([("signature".into(), signature.to_string().into())]),
                }),
                Arc::from("plaintext"),
                None,
            ),
        };
        let source: Arc<dyn TextSource> = Arc::new(RopeText::new(text));
        self.set_preview_text(source, language, cx);
        if let Some(focus) = focus {
            *self.matches.ranges.borrow_mut() = vec![focus.clone()];
            self.preview.update(cx, |p, cx| {
                p.invalidate_decorations(&crate::code_view::LayerId(LAYER.into()), None, cx);
                p.set_selection(
                    Selection::new(focus.start(), focus.end()),
                    Some(Reveal::Center),
                    cx,
                );
            });
        }
    }

    fn set_preview_text(
        &mut self,
        text: Arc<dyn TextSource>,
        language: Arc<str>,
        cx: &mut Context<Self>,
    ) {
        let generation = self.preview_generation;
        self.preview.update(cx, |p, cx| {
            p.set_text(Some(text.clone()), false, cx);
            p.set_language(Some(language.clone()), cx);
        });
        // 预览走与主编辑器相同的语法着色路径（同一个 tree-sitter 服务、同一个主题）。
        let analysis = self
            .services
            .syntax
            .analyze(text, language, self.cancel.clone());
        cx.spawn(async move |this, cx| {
            let Ok(update) = analysis.await else { return };
            let _ = this.update(cx, |w, cx| {
                if w.preview_generation == generation {
                    w.preview
                        .update(cx, |p, cx| p.set_syntax(Some(update.snapshot), None, cx));
                }
            });
        })
        .detach();
    }

    /// 预览定位到当前结果，并高亮匹配区间。
    fn reveal_current(&mut self, cx: &mut Context<Self>) {
        let (Some(k), Some(results)) = (self.current, self.results.clone()) else {
            return;
        };
        if !matches!(self.shown, Some(Shown::File(_))) || self.document.is_none() {
            return;
        }
        let range = results.items[k].range.clone();
        self.preview.update(cx, |p, cx| {
            p.invalidate_decorations(&crate::code_view::LayerId(LAYER.into()), None, cx);
            p.set_selection(
                Selection::new(range.start(), range.end()),
                Some(Reveal::Center),
                cx,
            );
        });
    }

    // ------------------------------------------------------------------ 命令

    /// 处理一条速览命令；`false` 表示本层不处理（交给外层）。
    pub fn execute(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(nested) = self.nested.as_ref().map(|n| n.widget.clone()) {
            // 焦点不在内层时，作用于最内层的窗口（主编辑器里按 F4 / Esc 等）。
            if nested.update(cx, |n, cx| n.execute(id, window, cx)) {
                return true;
            }
        }
        match id {
            reg::CLOSE => {
                if !self.settings.can_close_by_escape() {
                    return true;
                }
                cx.emit(PeekEvent::Close);
            }
            reg::NEXT | reg::NEXT_FROM_EMBEDDED => self.step(true, window, cx),
            reg::PREVIOUS | reg::PREVIOUS_FROM_EMBEDDED => self.step(false, window, cx),
            reg::REVEAL | reg::OPEN_TO_SIDE => {
                let focused = self
                    .list
                    .as_ref()
                    .and_then(|l| l.read(cx).widget.focused_id())
                    .and_then(|id| self.item_of(&id));
                if let Some(k) = focused.or(self.current) {
                    self.open(k, id == reg::OPEN_TO_SIDE, cx);
                }
            }
            reg::TOGGLE_FOCUS => {
                if self.preview.focus_handle(cx).contains_focused(window, cx) {
                    self.focus_tree(window, cx);
                } else {
                    self.focus_preview(window, cx);
                }
            }
            _ => return false,
        }
        true
    }

    fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(results) = &self.results else { return };
        if let Some(k) = results.step(self.current, forward) {
            let in_preview = self.preview.focus_handle(cx).contains_focused(window, cx);
            self.select(k, window, cx);
            if in_preview {
                self.focus_preview(window, cx);
            }
        }
    }

    fn open(&mut self, k: usize, side: bool, cx: &mut Context<Self>) {
        let Some(results) = &self.results else { return };
        let Some(item) = results.items.get(k) else {
            return;
        };
        let resource = results.files[item.file as usize].resource.clone();
        cx.emit(PeekEvent::Open {
            resource,
            selection: item.range.clone(),
            side,
        });
    }

    fn focus_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.list {
            Some(list) => window.focus(&list.focus_handle(cx)),
            None => window.focus(&self.focus),
        }
    }

    fn focus_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.preview.focus_handle(cx));
    }

    /// 把焦点放进窗口（宿主打开窗口后调用）：结果到达前放在窗口根上（`Esc` 仍可关闭）。
    pub fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(n) = self.nested.as_ref().map(|n| n.widget.clone()) {
            n.update(cx, |n, cx| n.take_focus(window, cx));
            return;
        }
        match (&self.phase, self.settings.default_focus) {
            (Phase::Loading, _) => window.focus(&self.focus),
            (Phase::Ready, DefaultFocus::Tree) => self.focus_tree(window, cx),
            (Phase::Ready, DefaultFocus::Editor) => self.focus_preview(window, cx),
        }
    }

    // ------------------------------------------------------------------ 嵌套速览

    /// 在预览的插入符处再开一层（`Alt+F12` 等）。返回是否开了。
    fn open_nested(&mut self, kind: PeekKind, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(document) = self.document.clone() else {
            return false;
        };
        let (point, symbol, line_height) = {
            let p = self.preview.read(cx);
            let caret = p.selection().active;
            (
                QueryPoint {
                    resource: document.resource.clone(),
                    position: caret,
                    version: p.version(),
                },
                p.word_at_caret(),
                p.line_height(),
            )
        };
        let symbol: Arc<str> = symbol
            .unwrap_or_else(|| self.services.text(&Message::key("peek.symbolHere")))
            .into();
        if let Some(previous) = self.nested.take() {
            self.preview
                .update(cx, |p, cx| p.set_view_zone(previous.anchor, 0.0, cx));
        }
        let request = PeekRequest {
            kind,
            point: point.clone(),
            symbol,
            marker: Marker::Auto,
            preset: None,
        };
        let services = self.services.clone();
        let widget = cx.new(|cx| PeekWidget::new(services, request, line_height, window, cx));
        let anchor = point.position.line.0;
        let height = widget.read(cx).height();
        let events = cx.subscribe_in(&widget, window, |w, _, e: &PeekEvent, window, cx| {
            w.nested_event(e.clone(), window, cx)
        });
        self.preview.update(cx, |p, cx| {
            p.set_view_zone(anchor, height, cx);
            super::reveal_zone(p, anchor, cx);
        });
        widget.update(cx, |n, cx| n.take_focus(window, cx));
        self.nested = Some(Nested {
            anchor,
            widget,
            _events: events,
        });
        cx.notify();
        true
    }

    fn close_nested(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(n) = self.nested.take() {
            self.preview
                .update(cx, |p, cx| p.set_view_zone(n.anchor, 0.0, cx));
            self.focus_preview(window, cx);
            cx.notify();
        }
    }

    fn nested_event(&mut self, event: PeekEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            PeekEvent::Close => self.close_nested(window, cx),
            PeekEvent::Empty(m) => {
                self.close_nested(window, cx);
                cx.emit(PeekEvent::Hint(m));
            }
            PeekEvent::Resized(h) => {
                if let Some(n) = &self.nested {
                    let anchor = n.anchor;
                    self.preview
                        .update(cx, |p, cx| p.set_view_zone(anchor, h, cx));
                }
            }
            other => cx.emit(other),
        }
    }

    // ------------------------------------------------------------------ 绘制

    fn key_context(&self) -> KeyContext {
        let mut k = KeyContext::default();
        k.add("PeekWidget");
        k.add(context::VISIBLE);
        k
    }

    fn preview_context(&self) -> KeyContext {
        let mut k = KeyContext::default();
        k.add(context::IN_EDITOR);
        k.add(context::EMBEDDED);
        let csharp = self
            .document
            .as_ref()
            .is_some_and(|d| &*d.metadata.language == "csharp");
        if csharp && self.services.queries.is_some() {
            k.add(context::HAS_DEFINITION);
            k.add(context::HAS_REFERENCES);
            k.add(context::HAS_IMPLEMENTATION);
        }
        k
    }

    fn on_preview_action(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let kind = match id {
            reg::PEEK_DEFINITION => PeekKind::Definition,
            reg::GO_TO_REFERENCES | reg::REFERENCE_SEARCH_TRIGGER => PeekKind::References,
            reg::PEEK_IMPLEMENTATION => PeekKind::Implementation,
            _ => {
                if !self.execute(id, window, cx) {
                    cx.propagate();
                }
                return;
            }
        };
        if let Some(q) = &self.services.queries {
            if q.availability(kind) == source::Availability::Unavailable {
                cx.emit(PeekEvent::Hint(Message::key(
                    "peek.unavailable.implementations",
                )));
                return;
            }
        }
        if !self.open_nested(kind, window, cx) {
            cx.propagate();
        }
    }

    fn title_bar(&self) -> impl IntoElement {
        let (title, marker) = self.title_text();
        let description = self
            .current
            .zip(self.results.as_ref())
            .map(|(k, r)| {
                let f = &r.files[r.file_of(k)];
                if f.description.is_empty() {
                    f.label.to_string()
                } else {
                    format!("{}/{}", f.description, f.label)
                }
            })
            .unwrap_or_default();
        let c = self.colors;
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .h(px((self.line_height * 1.2).ceil()))
            .flex_shrink_0()
            .bg(c.title_bg)
            .overflow_hidden()
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(c.title_label)
                    .child(SharedString::from(title)),
            )
            .children(marker.map(|m| {
                div()
                    .flex_shrink_0()
                    .text_color(c.title_description)
                    .child(SharedString::from(m))
            }))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(c.title_description)
                    .child(SharedString::from(description)),
            )
    }

    fn drag_listeners(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let active = self.drag.is_some();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                if !active {
                    return;
                }
                let e = entity.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                    let _ = e.update(cx, |w, cx| {
                        w.drag_to(f32::from(ev.position.x), f32::from(ev.position.y), cx)
                    });
                });
                let e = entity.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                    let _ = e.update(cx, |w, cx| {
                        w.drag = None;
                        cx.notify();
                    });
                });
            },
        )
        .absolute()
        .size_0()
    }

    fn drag_to(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        match self.drag {
            Some(Drag::Height { start_y, start }) => {
                let lines = (start + (y - start_y) / self.line_height)
                    .clamp(MIN_HEIGHT_LINES, MAX_HEIGHT_LINES);
                if (lines - self.height_lines).abs() > f32::EPSILON {
                    self.height_lines = lines;
                    cx.emit(PeekEvent::Resized(self.height()));
                    cx.notify();
                }
            }
            Some(Drag::Ratio {
                start_x,
                start,
                width,
            }) if width > 0.0 => {
                self.ratio = (start + (x - start_x) / width).clamp(0.15, 0.85);
                cx.notify();
            }
            _ => {}
        }
    }
}

impl Render for PeekWidget {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors;
        let frame = (self.line_height / 9.0).round().max(1.0);
        let mut preview = div()
            .id("peek-preview")
            .key_context(self.preview_context())
            .relative()
            .h_full()
            .w(relative(self.ratio))
            .overflow_hidden()
            .on_action(
                cx.listener(|w, a: &PeekAction, window, cx| w.on_preview_action(a.0, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|w, e: &MouseDownEvent, window, cx| {
                    if e.click_count == 2 && w.settings.can_close_by_escape() {
                        // 双击预览内容：在编辑器中打开插入符处（代码视图先处理这次点击）。
                        cx.spawn_in(window, async move |this, cx| {
                            let _ = this.update(cx, |w, cx| w.open_at_caret(cx));
                        })
                        .detach();
                    }
                }),
            )
            .child(self.preview.clone());
        if let Some(n) = &self.nested {
            preview = preview.child(zone_overlay(
                self.preview.clone(),
                n.anchor,
                n.widget.clone().into_any_element(),
            ));
        }
        let results: gpui::AnyElement = match (&self.list, self.loading_visible) {
            (Some(list), _) => list.clone().into_any_element(),
            (None, true) => div()
                .p(px(8.))
                .text_color(c.line_fg)
                .child(SharedString::from(
                    self.services.text(&Message::key("peek.loading")),
                ))
                .into_any_element(),
            (None, false) => div().into_any_element(),
        };
        let start_ratio = self.ratio;
        let body_width = self.body_width;
        let body = div()
            .flex()
            .flex_1()
            .min_h_0()
            .child(preview)
            .child(
                div()
                    .id("peek-sash-v")
                    .w(px(SASH))
                    .h_full()
                    .flex_shrink_0()
                    .bg(c.border)
                    .cursor_col_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |w, e: &MouseDownEvent, _, cx| {
                            w.drag = Some(Drag::Ratio {
                                start_x: f32::from(e.position.x),
                                start: start_ratio,
                                width: body_width,
                            });
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .bg(c.result_bg)
                    .child(results),
            )
            .child({
                let entity = cx.entity().downgrade();
                canvas(
                    move |bounds, _, cx| {
                        let _ =
                            entity.update(cx, |w, _| w.body_width = f32::from(bounds.size.width));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            });
        let start_lines = self.height_lines;
        div()
            .id("peek-widget")
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .occlude()
            .border_t(px(frame))
            .border_b(px(frame))
            .border_color(c.border)
            .bg(c.result_bg)
            .on_action(cx.listener(|w, a: &PeekAction, window, cx| {
                if !w.execute(a.0, window, cx) {
                    cx.propagate();
                }
            }))
            .child(self.title_bar())
            .child(body)
            .child(
                div()
                    .id("peek-sash-h")
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(-frame))
                    .h(px(SASH))
                    .cursor_row_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |w, e: &MouseDownEvent, _, cx| {
                            w.drag = Some(Drag::Height {
                                start_y: f32::from(e.position.y),
                                start: start_lines,
                            });
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(self.drag_listeners(cx))
    }
}

impl PeekWidget {
    /// 双击预览：在编辑器中打开预览文档的插入符处（`editor.stablePeek` 为 true 时不处理）。
    fn open_at_caret(&mut self, cx: &mut Context<Self>) {
        let Some(document) = &self.document else {
            return;
        };
        let s = self.preview.read(cx).selection();
        let Ok(selection) = TextRange::new(s.start(), s.end()) else {
            return;
        };
        cx.emit(PeekEvent::Open {
            resource: document.resource.clone(),
            selection,
            side: false,
        });
    }

    /// 测试：直接双击预览的效果（不经鼠标事件）。
    #[doc(hidden)]
    pub fn double_click_preview(&mut self, cx: &mut Context<Self>) {
        if self.settings.can_close_by_escape() {
            self.open_at_caret(cx);
        }
    }

    /// 主题变了：重新解析调色板。
    pub fn set_theme(&mut self, theme: Arc<dyn Theme>, kind: ThemeKind, cx: &mut Context<Self>) {
        let services = Rc::new(PeekServices {
            queries: self.services.queries.clone(),
            documents: self.services.documents.clone(),
            settings: self.services.settings.clone(),
            theme: theme.clone(),
            theme_kind: kind,
            strings: self.services.strings.clone(),
            external: self.services.external.clone(),
            syntax: self.services.syntax.clone(),
        });
        self.services = services;
        self.colors = Colors::resolve(&*theme, kind);
        *self.matches.style.borrow_mut() = TextStyle {
            background: self.colors.editor_match,
            border: self.colors.editor_match_border,
            ..TextStyle::default()
        };
        self.preview_theme = Arc::new(PeekTheme { inner: theme, kind });
        let pt = self.preview_theme.clone();
        self.preview.update(cx, |p, cx| p.set_theme(pt, kind, cx));
        if let Some(n) = self.nested.as_ref().map(|n| n.widget.clone()) {
            let theme = self.services.theme.clone();
            n.update(cx, |n, cx| n.set_theme(theme, kind, cx));
        }
        cx.notify();
    }

    /// 设置变了（`editor.stablePeek`、`editor.peekWidgetDefaultFocus`）。
    pub fn reload_settings(&mut self, cx: &mut Context<Self>) {
        self.settings = PeekSettings::read(&*self.services.settings);
        if let Some(n) = self.nested.as_ref().map(|n| n.widget.clone()) {
            n.update(cx, |n, cx| n.reload_settings(cx));
        }
    }
}
