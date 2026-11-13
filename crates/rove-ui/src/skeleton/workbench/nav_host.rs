//! #40 的窗口装配：C# 导航交互（见 `crate::nav` 的模块文档）。
//!
//! - **查询**一律经 `CsRouter`（#38，实现 #1 的 `CsQueries`）在后台执行器上执行，结果按“请求代次 +
//!   文档版本”回到窗口，过期结果丢弃；
//! - **跳转**一律经 #17 的打开文档服务（`OpenSource::CodeNavigation`，自动进入导航历史，目标行按
//!   `Reveal::CenterIfOutside` 置入视口）；
//! - **装饰**（语义同名高亮、`Ctrl+悬停` 下划线）是每个代码视图上的一层装饰（优先级
//!   `SYMBOL_HIGHLIGHT`），按行二分取出，结果只在查询完成时整层或按行失效，不进入绘制关键路径；
//! - **悬停卡片**画在工作台的浮层里（`gpui::anchored`，`editor.hover.above` 决定先上后下）。
//!
//! #41（速览窗口）合并后在 [`Workbench::open_peek`] 里打开窗口；在那之前多候选按 `goto` 处理
//! （跳到第一个并进入符号导航模式）。#42（元数据视图）合并后在 [`Workbench::open_metadata`] 里打开。

use super::*;
use crate::code_view::{priority, InlineDecoration, LayerId, LineDecorations, TextStyle};
use crate::documents::OpenTarget;
use crate::nav::{
    hover::{parse_xml_doc, HoverContent, HoverMove, HoverScroll, PREVIEW_LINES},
    model::{
        self, is_csharp, peek_annotation, plan, ApproxNotices, HighlightKind, NavKind, NavSample,
        PeekRequest, Plan, SymbolHighlights, SymbolNavigation,
    },
    registry::{self as nav_registry, context as nav_context, NavColors},
    semantic::{BufferChange, LiveSemantic, OpenBuffer, OpenBuffers},
    settings::{GotoMode, HoverEnabled, NavSettings, Occurrences},
    view::{self as nav_view, CardStyle},
};
use gpui::{anchored, deferred, Corner, EntityId, Pixels, Point};
use rove_base::{Line, Position};
use rove_buffer::SizeClass;
use rove_lang_cs::l1::{L1Change, L1Index, L1Options};
use rove_lang_cs::metadata_view::MetadataViews;
use rove_lang_cs::query::{NavigationKind, Precision, QueryPoint, QueryResult, Target};
use rove_lang_cs::router::{hash_bytes, CsRouter, RoutedTarget, SemanticSource};
use std::cell::RefCell;
use std::rc::Rc;

/// 悬停卡片的最大尺寸（缩放前，像素）。
const CARD_MAX_WIDTH: f32 = 500.0;
const CARD_MAX_HEIGHT: f32 = 250.0;
/// 300 ms 内没有结果时显示“正在加载…”（UX_SPEC §6.2）。
const LOADING_AFTER: Duration = Duration::from_millis(300);
/// 装饰层 ID（每个代码视图一层）。
const LAYER: &str = "rove.nav.symbols";
/// 状态栏消息的去重键。
const STATUS_KEY: &str = "nav.status";
const SYMBOLS_KEY: &str = "nav.symbols";
const HINT_KEY: &str = "nav.noResult";

/// 一个代码视图的语义同名高亮与链接下划线。
struct SlotState {
    /// 当前高亮属于哪个文档的哪个版本。
    document: Option<(DocumentId, Version)>,
    highlights: SymbolHighlights,
    /// `Ctrl+悬停` 的下划线：（行, 字节列区间）。
    link: Option<(u64, std::ops::Range<u32>)>,
    styles: [TextStyle; 3],
    link_style: TextStyle,
}

struct Slot {
    code: EntityId,
    layer: LayerId,
    state: Rc<RefCell<SlotState>>,
    /// 高亮请求的代次（光标移动、文本变化、索引变化都会作废旧请求）。
    epoch: u64,
}

/// 悬停卡片锚定的单词。
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Anchor {
    pub(super) group: EditorGroupId,
    pub(super) secondary: bool,
    pub(super) document: DocumentId,
    pub(super) version: Version,
    pub(super) resource: Resource,
    pub(super) line: u64,
    pub(super) start: u64,
    pub(super) end: u64,
    /// 单词起点的左上角（窗口坐标）与行高。
    pub(super) point: Point<Pixels>,
    pub(super) line_height: f32,
}

impl Anchor {
    fn same_word(&self, other: &Anchor) -> bool {
        self.group == other.group
            && self.secondary == other.secondary
            && self.document == other.document
            && self.version == other.version
            && self.line == other.line
            && self.start == other.start
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Phase {
    Hidden,
    /// 等待 `editor.hover.delay` 或查询结果。
    Waiting,
    /// 300 ms 内没有结果：显示“正在加载…”。
    Loading,
    Shown(HoverContent),
}

pub(super) struct HoverState {
    pub(super) phase: Phase,
    pub(super) anchor: Option<Anchor>,
    /// `Ctrl+悬停` 的定义预览（而不是普通悬停）。
    pub(super) definition: bool,
    epoch: u64,
    pub(super) over_card: bool,
    pub(super) scroll: HoverScroll,
    pub(super) focus: Option<FocusHandle>,
    /// 焦点在卡片里（`showHover` 第二次）：指针移开不收起。
    pub(super) keyboard: bool,
}

/// 一次 `F12` 的计时（`ROVE_NAV_TRACE=1` 时输出，测试与基准读取样本）。
#[derive(Clone, Debug)]
struct Pending {
    started: Instant,
    query_ms: f64,
    resource: Resource,
    position: Position,
}

/// 窗口内的导航状态。
pub(super) struct Nav {
    pub(super) router: Arc<CsRouter>,
    /// #37 的 L1 快速声明索引（应用入口在工作区扫描后建立；测试按需注入）。
    l1: Option<Arc<L1Index>>,
    pub(super) semantic: Arc<LiveSemantic>,
    /// #42：元数据视图的内容来源（路由重建时沿用同一个实例，`showDocs` 状态在里面）。
    pub(super) views: Arc<MetadataViews>,
    /// #134：等下一帧（有窗口句柄时）打开的速览请求。
    pending_peek: Option<PendingPeek>,
    buffers: OpenBuffers,
    pub(super) settings: NavSettings,
    colors: NavColors,
    epoch: u64,
    pub(super) symbols: Option<SymbolNavigation>,
    notices: ApproxNotices,
    pub(super) hover: HoverState,
    link: Option<(EditorGroupId, bool)>,
    slots: BTreeMap<(EditorGroupId, bool), Slot>,
    /// 最近一次交给速览窗口的请求（#41 合并前只记录）。
    pub(super) last_peek: Option<PeekRequest>,
    /// 语义来源的（快照代次, 状态）：变化时静默刷新同名高亮。
    semantic_key: Option<(Option<u64>, rove_lang_cs::router::SemanticStatus)>,
    trace: bool,
    pending: Option<Pending>,
    pub(super) samples: Vec<NavSample>,
}

impl Nav {
    pub(super) fn new(
        root: &std::path::Path,
        settings: &dyn Settings,
        theme: &dyn Theme,
        kind: rove_theme::ThemeKind,
    ) -> Self {
        let semantic = Arc::new(LiveSemantic::default());
        let buffers = Arc::new(rove_lang_cs::router::BufferStates::new());
        let views = Arc::new(MetadataViews::new(
            semantic.clone() as Arc<dyn SemanticSource>,
            Workbench::read_show_docs(settings),
        ));
        let router = Arc::new(
            CsRouter::new(semantic.clone() as Arc<dyn SemanticSource>, buffers)
                .with_root(root)
                .with_metadata(views.clone()),
        );
        Self {
            router,
            l1: None,
            semantic,
            views,
            pending_peek: None,
            buffers: OpenBuffers::default(),
            settings: NavSettings::read(settings, Some("csharp")),
            colors: NavColors::resolve(theme, kind),
            epoch: 0,
            symbols: None,
            notices: ApproxNotices::default(),
            hover: HoverState {
                phase: Phase::Hidden,
                anchor: None,
                definition: false,
                epoch: 0,
                over_card: false,
                scroll: HoverScroll::default(),
                focus: None,
                keyboard: false,
            },
            link: None,
            slots: BTreeMap::new(),
            last_peek: None,
            semantic_key: None,
            trace: std::env::var_os("ROVE_NAV_TRACE").is_some(),
            pending: None,
            samples: Vec::new(),
        }
    }
}

#[cfg(test)]
impl Nav {
    pub(super) fn colors_link(&self) -> Option<rove_theme::Color> {
        self.colors.link
    }
}

#[cfg(test)]
impl Workbench {
    /// 某个视图当前的语义同名高亮：（行, 起始列, 种类）。
    pub(super) fn nav_highlights(
        &self,
        group: EditorGroupId,
        secondary: bool,
    ) -> Vec<(u64, u64, HighlightKind)> {
        self.nav
            .slots
            .get(&(group, secondary))
            .map(|s| {
                s.state
                    .borrow()
                    .highlights
                    .ranges
                    .iter()
                    .map(|(r, k)| (r.start().line.0, r.start().column.0, *k))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// `editor.wordHighlight*` 三组样式（读、写、文本）与链接样式；主题缺键时用 VS Code 默认值。
fn styles(theme: &dyn Theme, kind: rove_theme::ThemeKind) -> ([TextStyle; 3], TextStyle) {
    let get = |key: &str| match theme.color(key) {
        Ok(c) => c,
        Err(_) => rove_theme::defaults::static_defaults(key).and_then(|d| d.for_kind(kind)),
    };
    let style = |bg: &str, border: &str| TextStyle {
        background: get(bg),
        border: get(border),
        ..Default::default()
    };
    let link = get("editorLink.activeForeground");
    (
        [
            style(
                "editor.wordHighlightBackground",
                "editor.wordHighlightBorder",
            ),
            style(
                "editor.wordHighlightStrongBackground",
                "editor.wordHighlightStrongBorder",
            ),
            style(
                "editor.wordHighlightTextBackground",
                "editor.wordHighlightTextBorder",
            ),
        ],
        TextStyle {
            foreground: link,
            underline: link,
            ..Default::default()
        },
    )
}

fn kind_index(k: HighlightKind) -> usize {
    match k {
        HighlightKind::Read => 0,
        HighlightKind::Write => 1,
        HighlightKind::Text => 2,
    }
}

/// #134：等窗口句柄的速览请求（`PeekRequest` 要在有 `Window` 的地方才能建窗口，所以登记后在下一帧的
/// [`Workbench::sync_nav`] 里打开）。
struct PendingPeek {
    request: PeekRequest,
    group: EditorGroupId,
    secondary: bool,
    /// `gotoAndPeek`：先跳转，等目标已经显示、插入符在目标位置后再在那里开速览。
    wait: Option<(Resource, Position)>,
    /// 发起时的导航代次；新的跳转作废它。
    epoch: u64,
}

/// 一次跳转的发起点。
pub(super) struct Origin {
    group: EditorGroupId,
    secondary: bool,
    document: Arc<Document>,
    position: Position,
    name: String,
}

impl Workbench {
    // ------------------------------------------------------------------ 装配

    /// 打开中的 C# 缓冲区状态（#38 的 `BufferStates`）。编辑（M3 编辑事务）与保存（#49）的装配处在这里调用
    /// `edited` / `saved`；打开、重新载入与关闭由 #40 按打开的标签自动同步。
    pub fn cs_buffers(&self) -> &Arc<rove_lang_cs::router::BufferStates> {
        self.nav.router.buffers()
    }

    /// C# 导航查询的唯一入口（#38 的 `CsRouter`）。
    pub fn cs_router(&self) -> &Arc<CsRouter> {
        &self.nav.router
    }

    /// 换上 L1 快速声明索引（#37）：路由重建，`BufferStates` 与语义来源沿用。
    pub(crate) fn attach_l1(&mut self, l1: Option<Arc<L1Index>>) {
        let buffers = self.nav.router.buffers().clone();
        let mut router = CsRouter::new(
            self.nav.semantic.clone() as Arc<dyn SemanticSource>,
            buffers,
        )
        .with_root(self.root.clone())
        .with_metadata(self.nav.views.clone());
        if let Some(l1) = &l1 {
            router = router.with_l1(l1.clone());
        }
        self.nav.router = Arc::new(router);
        self.nav.l1 = l1;
    }

    /// 工作区扫描完成：应用入口（已启动 #36 生命周期）在后台建 L1，有缓存时先载入缓存、只解析变化的文件。
    /// `rove.index.enabled = false` 时不写缓存目录（只在内存里建）。
    pub(super) fn nav_workspace_ready(&mut self, cx: &mut Context<Self>) {
        if self.index.lifecycle().is_none() {
            return;
        }
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        let config = LifecycleConfig::from_settings(&*self.settings);
        let mut options = L1Options::new(self.root.clone());
        if let (true, Ok(cache), false) = (config.enabled, &config.cache_root, cfg!(test)) {
            options = options.with_cache_dir(
                rove_lang_cs::lifecycle::layout::ProjectCache::new(cache, &self.root).l1_dir(),
            );
        }
        let l1 = Arc::new(L1Index::new(options));
        self.attach_l1(Some(l1.clone()));
        cx.background_executor()
            .spawn(async move {
                let files = match workspace.files(0..usize::MAX) {
                    Ok(page) => page
                        .items
                        .iter()
                        .map(|e| e.path.clone())
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        eprintln!("l1: {e}");
                        return;
                    }
                };
                if let Err(e) = l1.build_and_save(files, &Cancellation::default()) {
                    eprintln!("l1: {e}");
                }
            })
            .detach();
    }

    /// #15 的文件事件 → L1 增量更新（后台）；`RescanRequired` 按新的文件列表重建。
    pub(super) fn nav_files_changed(&mut self, events: &[FileEvent], cx: &mut Context<Self>) {
        let Some(l1) = self.nav.l1.clone() else {
            return;
        };
        if events
            .iter()
            .any(|e| matches!(e, FileEvent::RescanRequired))
        {
            self.nav_workspace_ready(cx);
            return;
        }
        let changes: Vec<L1Change> = events
            .iter()
            .filter_map(|e| match e {
                FileEvent::Created(p) | FileEvent::Changed(p) => {
                    Some(L1Change::Upserted(p.clone()))
                }
                FileEvent::Removed(p) => Some(L1Change::Removed(p.clone())),
                FileEvent::Renamed { from, to } => Some(L1Change::Renamed {
                    from: from.clone(),
                    to: to.clone(),
                }),
                FileEvent::RescanRequired => None,
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = l1.apply_changes(&changes, &Cancellation::default()) {
                    eprintln!("l1: {e}");
                }
            })
            .detach();
    }

    /// 焦点组活动标签的项目相对路径（基准等待跳转完成）。
    pub fn active_path(&self) -> Option<String> {
        match &self.editor.active().current()?.resource {
            Resource::Workspace(p) => Some(p.as_str().to_owned()),
            _ => None,
        }
    }

    /// `F12` → 目标行画面完成的计时样本（测试与 `cargo xtask bench --filter nav/` 读取）。
    pub fn navigation_samples(&self) -> &[NavSample] {
        &self.nav.samples
    }

    /// 换上（或换掉）语义来源：#36 的生命周期（应用入口自动装配），基准与测试注入固定索引。
    pub fn attach_semantic(
        &mut self,
        source: Option<Arc<dyn SemanticSource>>,
        cx: &mut Context<Self>,
    ) {
        self.nav.semantic.set(source);
        self.nav_index_changed(cx);
    }

    /// 生命周期换了（#36 按设置重启）：语义来源跟着换。
    pub(super) fn nav_index_attached(&mut self, cx: &mut Context<Self>) {
        let source = self.index.shared().map(|l| l as Arc<dyn SemanticSource>);
        self.attach_semantic(source, cx);
    }

    /// 索引状态变化：快照代次或状态种类变了就静默刷新同名高亮（不弹提示、不清空，结果到达后替换）。
    pub(super) fn nav_index_changed(&mut self, cx: &mut Context<Self>) {
        let key = (
            self.nav.semantic.snapshot().map(|s| s.generation),
            self.nav.semantic.status(),
        );
        if self.nav.semantic_key == Some(key) {
            return;
        }
        self.nav.semantic_key = Some(key);
        let slots: Vec<_> = self.nav.slots.keys().copied().collect();
        for (group, secondary) in slots {
            self.refresh_highlights(group, secondary, true, false, cx);
        }
    }

    pub(super) fn nav_settings_changed(
        &mut self,
        keys: &BTreeSet<Arc<str>>,
        cx: &mut Context<Self>,
    ) {
        if keys.iter().any(|k| NavSettings::is_key(k)) {
            self.nav.settings = NavSettings::read(&*self.settings, Some("csharp"));
            let slots: Vec<_> = self.nav.slots.keys().copied().collect();
            for (group, secondary) in slots {
                self.refresh_highlights(group, secondary, true, false, cx);
            }
        }
    }

    pub(super) fn nav_theme_changed(&mut self, cx: &mut Context<Self>) {
        let kind = self.theme_service.current().kind();
        self.nav.colors = NavColors::resolve(self.theme.as_ref(), kind);
        let (s, link) = styles(self.theme.as_ref(), kind);
        for slot in self.nav.slots.values() {
            let mut state = slot.state.borrow_mut();
            state.styles = s;
            state.link_style = link;
        }
        for (group, secondary) in self.nav.slots.keys().copied().collect::<Vec<_>>() {
            self.invalidate_slot(group, secondary, None, cx);
        }
    }

    /// 每帧调用：把打开中的 C# 文档同步给 `BufferStates`（只比较标签集合；读盘算哈希在后台），
    /// 并在 `F12` 的目标画面提交后记录耗时。
    pub(super) fn sync_nav(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current: Vec<OpenBuffer> = self
            .editor
            .groups
            .values()
            .flat_map(|g| g.tabs.iter())
            .filter_map(Tab::opened)
            .filter(|o| is_csharp(&o.document.metadata.language))
            .filter_map(|o| match &o.document.resource {
                Resource::Workspace(path) => Some(OpenBuffer {
                    path: path.clone(),
                    document: o.document.id,
                    version: o.document.text.version(),
                    text: o.document.text.clone(),
                }),
                _ => None,
            })
            .collect();
        for change in self.nav.buffers.diff(current) {
            self.apply_buffer_change(change, cx);
        }
        self.trace_paint(window, cx);
        self.flush_pending_peek(window, cx);
        // #42：索引原子替换后重新渲染打开着的元数据视图。
        self.sync_metadata(cx);
    }

    fn apply_buffer_change(&mut self, change: BufferChange, cx: &mut Context<Self>) {
        let (buffer, reload) = match change {
            BufferChange::Closed(path) => {
                self.nav.router.buffers().closed(&path);
                return;
            }
            BufferChange::Opened(b) => (b, false),
            BufferChange::Reloaded(b) => (b, true),
        };
        // 磁盘原始字节的哈希（与索引文件表同一算法）在后台计算。
        let file = self.root.join(buffer.path.as_str());
        let task = cx
            .background_executor()
            .spawn(async move { std::fs::read(file).map(|bytes| hash_bytes(&bytes)) });
        cx.spawn(async move |this, cx| {
            let Ok(hash) = task.await else { return };
            let _ = this.update(cx, |view, cx| {
                // 计算期间文档已关闭或又换了快照：这次结果作废（新的变化会再触发一次）。
                let still = view
                    .editor
                    .groups
                    .values()
                    .flat_map(|g| g.tabs.iter())
                    .filter_map(Tab::opened)
                    .any(|o| {
                        o.document.id == buffer.document
                            && o.document.text.version() == buffer.version
                    });
                if !still {
                    return;
                }
                let states = view.nav.router.buffers();
                let reloaded = reload
                    && states
                        .reloaded(&buffer.path, buffer.text.clone(), hash)
                        .is_ok();
                if !reloaded {
                    states.opened(&buffer.path, buffer.text.clone(), hash);
                }
                // 登记前的高亮可能来自“未打开文件”的路径：刷新一次。
                let slots: Vec<_> = view.nav.slots.keys().copied().collect();
                for (group, secondary) in slots {
                    view.refresh_highlights(group, secondary, true, false, cx);
                }
            });
        })
        .detach();
    }

    /// 编辑器区域的上下文键：C# 文件发布“有提供方”的键（D2：其他语言一个都不发布），符号导航模式发布
    /// `hasSymbols`，悬停卡片可见时发布 `editorHoverVisible`。
    pub(super) fn nav_context(&self) -> String {
        let mut out = String::new();
        let csharp = self
            .editor
            .active()
            .current()
            .filter(|tab| tab.kind == EditorKind::Text)
            .and_then(Tab::opened)
            .is_some_and(|o| is_csharp(&o.document.metadata.language));
        if csharp {
            for key in nav_context::PROVIDERS {
                out.push(' ');
                out.push_str(key);
            }
        }
        if self.nav.symbols.is_some() {
            out.push(' ');
            out.push_str(nav_context::HAS_SYMBOLS);
        }
        if !matches!(self.nav.hover.phase, Phase::Hidden) {
            out.push(' ');
            out.push_str(nav_context::HOVER_VISIBLE);
        }
        out
    }

    // ------------------------------------------------------------------ 代码视图事件

    pub(super) fn nav_code_event(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        event: &CodeViewEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            CodeViewEvent::PointerMoved {
                position,
                point,
                ctrl,
            } => self.pointer_moved(group, secondary, *position, *point, *ctrl, cx),
            CodeViewEvent::PointerLeft => {
                self.clear_link(cx);
                self.pointer_off_word(cx);
            }
            CodeViewEvent::LinkClicked { position, side } => {
                self.clear_link(cx);
                self.hide_hover(cx);
                let peek = self.nav.settings.definition_link_opens_in_peek;
                self.navigate(
                    NavKind::Definition,
                    group,
                    secondary,
                    Some(*position),
                    *side,
                    peek,
                    cx,
                );
            }
            CodeViewEvent::SelectionChanged { source, .. } => {
                if *source == crate::code_view::SelectionSource::Keyboard {
                    self.hide_hover(cx);
                }
                self.refresh_highlights(group, secondary, false, false, cx);
            }
            CodeViewEvent::TextChanged => {
                self.hide_hover(cx);
                self.refresh_highlights(group, secondary, true, false, cx);
            }
            CodeViewEvent::ViewportChanged(_)
                if self
                    .nav
                    .hover
                    .anchor
                    .as_ref()
                    .is_some_and(|a| a.group == group && a.secondary == secondary) =>
            {
                self.hide_hover(cx);
            }
            _ => {}
        }
    }

    /// 键位服务分发来的本模块命令。
    pub(super) fn nav_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (group, secondary) = self.focused_slot(window, cx);
        match id {
            nav_registry::REVEAL_DEFINITION => self.navigate(
                NavKind::Definition,
                group,
                secondary,
                None,
                false,
                false,
                cx,
            ),
            nav_registry::REVEAL_DEFINITION_ASIDE => {
                self.navigate(NavKind::Definition, group, secondary, None, true, false, cx)
            }
            nav_registry::GOTO_DECLARATION => self.navigate(
                NavKind::Declaration,
                group,
                secondary,
                None,
                false,
                false,
                cx,
            ),
            nav_registry::GOTO_IMPLEMENTATION => self.navigate(
                NavKind::Implementation,
                group,
                secondary,
                None,
                false,
                false,
                cx,
            ),
            nav_registry::NEXT_SYMBOL => self.next_symbol(cx),
            nav_registry::NEXT_SYMBOL_CANCEL => self.cancel_symbol_navigation(cx),
            nav_registry::SHOW_HOVER => self.show_hover_command(group, secondary, window, cx),
            nav_registry::SCROLL_UP_HOVER => self.scroll_hover(HoverMove::Up, cx),
            nav_registry::SCROLL_DOWN_HOVER => self.scroll_hover(HoverMove::Down, cx),
            nav_registry::SCROLL_LEFT_HOVER => self.scroll_hover(HoverMove::Left, cx),
            nav_registry::SCROLL_RIGHT_HOVER => self.scroll_hover(HoverMove::Right, cx),
            nav_registry::PAGE_UP_HOVER => self.scroll_hover(HoverMove::PageUp, cx),
            nav_registry::PAGE_DOWN_HOVER => self.scroll_hover(HoverMove::PageDown, cx),
            nav_registry::GOTO_TOP_HOVER => self.scroll_hover(HoverMove::Top, cx),
            nav_registry::GOTO_BOTTOM_HOVER => self.scroll_hover(HoverMove::Bottom, cx),
            // 悬停只有一级详细程度（#40 只登记这两个命令）。
            nav_registry::INCREASE_HOVER_VERBOSITY | nav_registry::DECREASE_HOVER_VERBOSITY => {}
            nav_registry::WORD_HIGHLIGHT_NEXT => self.step_highlight(group, secondary, true, cx),
            nav_registry::WORD_HIGHLIGHT_PREV => self.step_highlight(group, secondary, false, cx),
            nav_registry::WORD_HIGHLIGHT_TRIGGER => {
                self.refresh_highlights(group, secondary, true, true, cx)
            }
            _ => {}
        }
        cx.notify();
    }

    /// 有键盘焦点的代码视图：焦点组的主视图，组内拆分时焦点在副视图上则为副视图。
    fn focused_slot(&self, window: &Window, cx: &App) -> (EditorGroupId, bool) {
        let group = self.editor.focused;
        let secondary = self
            .group_code(group, true)
            .is_some_and(|code| code.focus_handle(cx).contains_focused(window, cx));
        (group, secondary)
    }

    // ------------------------------------------------------------------ 转到定义 / 声明 / 实现

    fn origin(
        &self,
        group: EditorGroupId,
        secondary: bool,
        position: Option<Position>,
        cx: &App,
    ) -> Option<Origin> {
        let document = self.shown_document(group, secondary)?;
        if !is_csharp(&document.metadata.language) {
            return None;
        }
        let code = self.group_code(group, secondary)?.read(cx);
        let position = position.unwrap_or_else(|| code.selection().active);
        let name = code
            .word_range_at(position)
            .map(|(_, _, w)| w)
            .unwrap_or_default();
        Some(Origin {
            group,
            secondary,
            document,
            position,
            name,
        })
    }

    /// `F12` / `ctrl+k f12` / `goToDeclaration` / `ctrl+f12` / `Ctrl+点击`。非 C# 文件什么都不做（不弹错误、
    /// 不打开空速览）。`peek`：`Ctrl+点击` 且 `editor.definitionLinkOpensInPeek` 为真。
    #[allow(clippy::too_many_arguments)]
    fn navigate(
        &mut self,
        kind: NavKind,
        group: EditorGroupId,
        secondary: bool,
        position: Option<Position>,
        aside: bool,
        peek: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(origin) = self.origin(group, secondary, position, cx) else {
            return;
        };
        let started = Instant::now();
        // 新的跳转结束上一次的符号导航模式。
        self.nav.symbols = None;
        self.nav.epoch += 1;
        let epoch = self.nav.epoch;
        let point = QueryPoint {
            resource: origin.document.resource.clone(),
            position: origin.position,
            version: origin.document.text.version(),
        };
        let router = self.nav.router.clone();
        let query = kind.query();
        let task = cx.background_executor().spawn(async move {
            let t = Instant::now();
            let result = router.navigate_detailed(query, &point, &Cancellation::default());
            (result, t.elapsed().as_secs_f64() * 1000.0)
        });
        cx.spawn(async move |this, cx| {
            let (result, query_ms) = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.nav.epoch != epoch {
                    return;
                }
                view.apply_navigation(kind, origin, result, aside, peek, started, query_ms, cx);
                cx.notify();
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_navigation(
        &mut self,
        kind: NavKind,
        origin: Origin,
        result: CoreResult<QueryResult<Arc<[RoutedTarget]>>>,
        aside: bool,
        peek: bool,
        started: Instant,
        query_ms: f64,
        cx: &mut Context<Self>,
    ) {
        let result = match result {
            Ok(r) => r,
            Err(Error::Cancelled) => return,
            // 未就绪、版本过期、不支持的资源：不能确定，按“近似的空结果”提示。
            Err(_) => QueryResult {
                value: Arc::from([]),
                precision: Precision::Approximate,
                reason: None,
                version: None,
                index_generation: 0,
            },
        };
        let mode = if peek {
            GotoMode::Peek
        } else {
            kind.mode(&self.nav.settings)
        };
        match plan(kind, &result, mode) {
            Plan::NoResult { key } => {
                let message = Message {
                    key: key.into(),
                    args: Arc::from([("name".into(), origin.name.as_str().into())]),
                };
                let _ = shell::view::report(
                    self,
                    HINT_KEY,
                    Reason::NoResult {
                        document: origin.document.id,
                        position: origin.position,
                    },
                    message,
                    Arc::from([]),
                    cx,
                );
            }
            Plan::Metadata { targets } => self.open_metadata(&origin, &targets, aside, cx),
            Plan::Jump {
                target,
                approximate,
            } => {
                if peek {
                    let request = PeekRequest {
                        kind,
                        origin: (origin.document.resource.clone(), origin.position),
                        symbol: origin.name.as_str().into(),
                        targets: Arc::from([target.clone()]),
                        annotation: peek_annotation(approximate, 1),
                    };
                    if self.open_peek(request, &origin, None, cx) {
                        return;
                    }
                }
                self.jump(&target, aside, origin.group, Some((started, query_ms)));
                if approximate {
                    self.approximate_notice(&target, cx);
                }
            }
            Plan::Multiple {
                mode,
                targets,
                approximate,
            } => {
                let request = PeekRequest {
                    kind,
                    origin: (origin.document.resource.clone(), origin.position),
                    symbol: origin.name.as_str().into(),
                    targets: targets.clone(),
                    annotation: peek_annotation(approximate, targets.len()),
                };
                let peeked = match mode {
                    GotoMode::Peek => self.open_peek(request, &origin, None, cx),
                    GotoMode::GotoAndPeek => {
                        self.jump(&targets[0], aside, origin.group, Some((started, query_ms)));
                        let at = (targets[0].resource.clone(), targets[0].selection.start());
                        self.open_peek(request, &origin, Some(at), cx);
                        return;
                    }
                    GotoMode::Goto => false,
                };
                if peeked {
                    return;
                }
                // `goto`（以及速览窗口不可用时）：跳到第一个，进入符号导航模式。
                self.jump(&targets[0], aside, origin.group, Some((started, query_ms)));
                self.nav.symbols = Some(SymbolNavigation::new(targets.clone()));
                if approximate {
                    self.approximate_notice(&targets[0], cx);
                } else {
                    self.symbol_status(cx);
                }
            }
        }
    }

    /// 经 #17 的打开文档服务打开目标并把插入符放到符号名的起点（VS Code `collapseToStart`）；
    /// 目标行按 `Reveal::CenterIfOutside` 置入视口；跳转前的位置自动压入导航历史。
    fn jump(
        &mut self,
        target: &Target,
        aside: bool,
        group: EditorGroupId,
        timing: Option<(Instant, f64)>,
    ) {
        let position = target.selection.start();
        let options = OpenOptions {
            target: if aside {
                OpenTarget::Beside
            } else {
                OpenTarget::Group(group)
            },
            source: OpenSource::CodeNavigation,
            location: Some(position),
            ..Default::default()
        };
        if let Err(e) =
            self.open_documents()
                .open(target.resource.clone(), options, Cancellation::default())
        {
            eprintln!("goto definition: {e}");
            return;
        }
        if let Some((started, query_ms)) = timing {
            self.nav.pending = Some(Pending {
                started,
                query_ms,
                resource: target.resource.clone(),
                position,
            });
        }
    }

    /// 近似的单候选跳转：状态栏临时提示 3 秒（§6.1 第 1 级），同一符号只提示一次。
    fn approximate_notice(&mut self, target: &Target, cx: &mut Context<Self>) {
        if !self.nav.notices.first_time(target) {
            return;
        }
        let _ = shell::view::report(
            self,
            STATUS_KEY,
            Reason::Receipt,
            Message::key("nav.approximateJump"),
            Arc::from([]),
            cx,
        );
    }

    /// 多候选（或 `Ctrl+点击` 且 `editor.definitionLinkOpensInPeek`）交给速览窗口（#41）。返回 true 表示速览已经接手
    /// （调用方不再跳转）。
    ///
    /// 建速览窗口需要 `Window`，而查询结果是在后台任务回到窗口时处理的，手里没有它：这里只登记请求，
    /// 下一帧的 [`Workbench::sync_nav`] 在有窗口句柄的地方打开（[`Workbench::flush_pending_peek`]）。
    /// `wait`：`gotoAndPeek` 先跳转，等目标已经显示、插入符到位后才在那里开速览。
    pub(super) fn open_peek(
        &mut self,
        request: PeekRequest,
        origin: &Origin,
        wait: Option<(Resource, Position)>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.nav.last_peek = Some(request.clone());
        self.nav.pending_peek = Some(PendingPeek {
            request,
            group: origin.group,
            secondary: origin.secondary,
            wait,
            epoch: self.nav.epoch,
        });
        cx.notify();
        true
    }

    /// 打开登记的速览：#40 的 `PeekRequest`（多候选的目标列表与近似标记）→ #41 的 `PeekRequest`（带现成结果，
    /// 不再查询）。目标文档已经换掉、窗口已关闭、又发起了新的跳转时放弃。
    fn flush_pending_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.nav.pending_peek.take() else {
            return;
        };
        if pending.epoch != self.nav.epoch {
            return;
        }
        let (group, secondary, at) = match &pending.wait {
            None => (pending.group, pending.secondary, pending.request.origin.1),
            Some((resource, position)) => {
                // gotoAndPeek：焦点组的活动文档是目标、插入符在目标位置才开；否则下一帧再看。
                let group = self.editor.focused;
                let ready = self.shown_document(group, false).is_some_and(|d| {
                    model::resource_key(&d.resource) == model::resource_key(resource)
                }) && self
                    .group_code(group, false)
                    .is_some_and(|c| c.read(cx).selection().active == *position);
                if !ready {
                    self.nav.pending_peek = Some(pending);
                    return;
                }
                (group, false, *position)
            }
        };
        let Some(document) = self.shown_document(group, secondary) else {
            return;
        };
        if pending.wait.is_none() && document.resource != pending.request.origin.0 {
            return;
        }
        let request = pending.request;
        let kind = match request.kind {
            NavKind::Definition | NavKind::Declaration => crate::peek::PeekKind::Definition,
            NavKind::Implementation => crate::peek::PeekKind::Implementation,
        };
        let approximate = request.annotation.is_some();
        let routed: Vec<RoutedTarget> = request
            .targets
            .iter()
            .map(|target| RoutedTarget {
                target: target.clone(),
                precision: if approximate {
                    Precision::Approximate
                } else {
                    Precision::Exact
                },
                reason: None,
                role: rove_lang_cs::router::TargetRole::Definition,
            })
            .collect();
        let marker = crate::peek::Marker::Custom(request.annotation.map(|(key, args)| {
            Message {
                key: key.into(),
                args: args
                    .iter()
                    .map(|(k, v)| ((*k).into(), v.as_str().into()))
                    .collect(),
            }
        }));
        let symbol: Arc<str> = if request.symbol.is_empty() {
            self.label("peek.symbolHere", &[]).into()
        } else {
            request.symbol
        };
        let peek = crate::peek::PeekRequest {
            kind,
            point: QueryPoint {
                resource: document.resource.clone(),
                position: at,
                version: document.text.version(),
            },
            symbol,
            marker,
            preset: Some(Arc::new(crate::peek::PeekResults::group(&routed))),
        };
        self.show_peek(group, secondary, &document, peek, window, cx);
    }

    /// 只有外部符号的目标：打开元数据视图（#42，见 `metadata_host.rs`）。
    pub(super) fn open_metadata(
        &mut self,
        origin: &Origin,
        targets: &[Target],
        aside: bool,
        cx: &mut Context<Self>,
    ) {
        self.open_metadata_targets(origin.group, &origin.name, targets, aside, cx);
    }

    // ------------------------------------------------------------------ 符号导航模式

    fn symbol_status(&mut self, cx: &mut Context<Self>) {
        let Some(nav) = &self.nav.symbols else { return };
        let (index, total) = nav.position();
        let message = Message {
            key: "nav.symbolPosition".into(),
            args: Arc::from([
                ("index".into(), index.to_string().into()),
                ("total".into(), total.to_string().into()),
            ]),
        };
        let _ = shell::view::report(
            self,
            SYMBOLS_KEY,
            Reason::Receipt,
            message,
            Arc::from([]),
            cx,
        );
    }

    /// `editor.gotoNextSymbolFromResult`（`f12`，when `hasSymbols`）：到末尾后回到第一个。
    fn next_symbol(&mut self, cx: &mut Context<Self>) {
        let Some(nav) = self.nav.symbols.as_mut() else {
            return;
        };
        let target = nav.advance();
        let group = self.editor.focused;
        self.jump(&target, false, group, Some((Instant::now(), 0.0)));
        self.symbol_status(cx);
    }

    /// `editor.gotoNextSymbolFromResult.cancel`（`escape`）。
    fn cancel_symbol_navigation(&mut self, _cx: &mut Context<Self>) {
        self.nav.symbols = None;
        let _ = crate::feedback::FeedbackService::dismiss(&mut self.shell.feedback, SYMBOLS_KEY);
    }

    // ------------------------------------------------------------------ 计时

    /// `F12` 的目标位置已经出现在焦点组的代码视图里：下一帧提交后记一个样本。
    fn trace_paint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.nav.pending.as_ref() else {
            return;
        };
        let group = self.editor.focused;
        let shown = self.shown_document(group, false).is_some_and(|d| {
            model::resource_key(&d.resource) == model::resource_key(&pending.resource)
        });
        let at = self
            .group_code(group, false)
            .is_some_and(|c| c.read(cx).selection().active == pending.position);
        if !(shown && at) {
            return;
        }
        let pending = self.nav.pending.take().unwrap();
        // 目标位置进入了这一帧：记一个样本。这一帧的代码视图绘制（prepaint → paint）结束即“画面完成”；
        // 它的 CPU 耗时与占位行数在下一帧回调里从代码视图的帧统计读出。
        self.nav.samples.push(NavSample {
            query_ms: pending.query_ms,
            frame_ms: pending.started.elapsed().as_secs_f64() * 1000.0,
            paint_cpu_ms: None,
            placeholders: None,
            next_frame_ms: None,
            full_ms: None,
        });
        let index = self.nav.samples.len() - 1;
        let trace = self.nav.trace;
        let this = cx.entity().downgrade();
        let watch = this.clone();
        window.on_next_frame(move |window, cx| {
            let next = pending.started.elapsed().as_secs_f64() * 1000.0;
            let _ = this.update(cx, |v, cx| {
                let stats = v.code_view().read(cx).last_frame();
                if let Some(sample) = v.nav.samples.get_mut(index) {
                    sample.paint_cpu_ms = Some(stats.cpu.as_secs_f64() * 1000.0);
                    sample.placeholders = Some(stats.placeholders + stats.missing);
                    sample.next_frame_ms = Some(next);
                    if stats.placeholders + stats.missing == 0 {
                        sample.full_ms = Some(next);
                    }
                    if trace {
                        println!(
                            "NAV_F12_TO_PAINT_MS {:.3} QUERY_MS {:.3} NEXT_FRAME_MS {next:.3} PLACEHOLDERS {}",
                            sample.frame_ms + sample.paint_cpu_ms.unwrap_or(0.0),
                            sample.query_ms,
                            stats.placeholders + stats.missing
                        );
                    }
                }
            });
            // 目标帧里有等宽占位（整形缓存未命中，空闲切片随后替换）：继续看后面的帧，记下第一帧
            // 全部是正式整形结果的时刻（上界，含等待垂直同步）。
            watch_full(watch, index, pending.started, 0, window);
        });
    }

    // ------------------------------------------------------------------ 语义同名高亮

    /// 代码视图上的装饰层（视图重建后重新登记）。
    fn slot(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        cx: &mut Context<Self>,
    ) -> Option<(Entity<CodeView>, Rc<RefCell<SlotState>>)> {
        let code = self.group_code(group, secondary)?.clone();
        let id = code.entity_id();
        if let Some(slot) = self.nav.slots.get(&(group, secondary)) {
            if slot.code == id {
                return Some((code, slot.state.clone()));
            }
        }
        let (s, link_style) = styles(self.theme.as_ref(), self.theme_service.current().kind());
        let state = Rc::new(RefCell::new(SlotState {
            document: None,
            highlights: SymbolHighlights::default(),
            link: None,
            styles: s,
            link_style,
        }));
        let provider = state.clone();
        let layer = code.update(cx, |c, cx| {
            c.add_decorations(
                LAYER,
                priority::SYMBOL_HIGHLIGHT,
                Rc::new(move |line: u64, _: &str, out: &mut LineDecorations| {
                    let s = provider.borrow();
                    for (range, kind) in s.highlights.on_line(line) {
                        out.inline.push(InlineDecoration {
                            range,
                            style: s.styles[kind_index(kind)],
                        });
                    }
                    if let Some((l, range)) = &s.link {
                        if *l == line {
                            out.inline.push(InlineDecoration {
                                range: range.clone(),
                                style: s.link_style,
                            });
                        }
                    }
                }),
                cx,
            )
        });
        let layer = match layer {
            Ok(layer) => layer,
            Err(e) => {
                eprintln!("nav decorations: {e}");
                return None;
            }
        };
        self.nav.slots.insert(
            (group, secondary),
            Slot {
                code: id,
                layer,
                state: state.clone(),
                epoch: 0,
            },
        );
        Some((code, state))
    }

    fn invalidate_slot(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        lines: Option<Vec<u64>>,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.nav.slots.get(&(group, secondary)) else {
            return;
        };
        let Some(code) = self.group_code(group, secondary).cloned() else {
            return;
        };
        let layer = slot.layer.clone();
        code.update(cx, |c, cx| match lines {
            Some(lines) if lines.len() <= 64 => {
                for l in lines {
                    c.invalidate_decorations(&layer, Some(l..l + 1), cx);
                }
            }
            _ => c.invalidate_decorations(&layer, None, cx),
        });
    }

    /// 换一组高亮：只让新旧两组涉及的行失效。
    fn set_highlights(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        document: Option<(DocumentId, Version)>,
        highlights: SymbolHighlights,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.nav.slots.get(&(group, secondary)) else {
            return;
        };
        let lines: Vec<u64> = {
            let mut s = slot.state.borrow_mut();
            if s.highlights == highlights && s.document == document {
                return;
            }
            let mut lines: Vec<u64> = s
                .highlights
                .ranges
                .iter()
                .chain(&highlights.ranges)
                .map(|(r, _)| r.start().line.0)
                .collect();
            lines.sort_unstable();
            lines.dedup();
            s.highlights = highlights;
            s.document = document;
            lines
        };
        if !lines.is_empty() {
            self.invalidate_slot(group, secondary, Some(lines), cx);
        }
    }

    /// 光标移动、文本变化、设置或索引变化后更新某个视图的同名高亮。
    ///
    /// C# 文件：光标仍在当前高亮的某一处时不重查；否则按 `editor.occurrencesHighlightDelay` 去抖后在后台
    /// 查询引用，只保留本文件的结果。精确结果按读 / 写区分颜色；近似或查询失败时退化为文本高亮（#20 的
    /// 文本层），**不加任何提示**。非 C# 文件、大文件与超大文件交给 #20 的文本实现。
    /// `force`：忽略“光标仍在当前高亮内”（文本、索引、设置变化）；`trigger`：`wordHighlight.trigger`，
    /// `editor.occurrencesHighlight` 为 `off` 时也高亮一次。
    fn refresh_highlights(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        force: bool,
        trigger: bool,
        cx: &mut Context<Self>,
    ) {
        let document = self.shown_document(group, secondary);
        let semantic = document.as_ref().is_some_and(|d| {
            is_csharp(&d.metadata.language)
                && d.metadata.size == SizeClass::Normal
                // 元数据视图（#42）的“引用”是整个工作区的使用位置，不做本文件的语义同名高亮。
                && rove_lang_cs::metadata_view::view_key(&d.resource).is_none()
        });
        if !semantic {
            // 只在曾经接管过的视图上还原（非 C# 视图不必登记装饰层）。
            if self.nav.slots.contains_key(&(group, secondary)) {
                self.set_highlights(group, secondary, None, SymbolHighlights::default(), cx);
                if let Some(code) = self.group_code(group, secondary).cloned() {
                    code.update(cx, |c, cx| c.set_semantic_word_highlights(None, cx));
                }
            }
            return;
        }
        let document = document.unwrap();
        let Some((code, state)) = self.slot(group, secondary, cx) else {
            return;
        };
        let key = (document.id, document.text.version());
        let selection = code.read(cx).selection();
        let off = self.nav.settings.occurrences == Occurrences::Off && !trigger;
        let word = (!off && selection.is_empty())
            .then(|| code.read(cx).word_range_at(selection.active))
            .flatten();
        let Some(_) = word else {
            // 没有单词（或有选区、或关闭了同名高亮）：清空；文本层同样不高亮单词。
            if let Some(slot) = self.nav.slots.get_mut(&(group, secondary)) {
                slot.epoch += 1;
            }
            self.set_highlights(group, secondary, Some(key), SymbolHighlights::default(), cx);
            code.update(cx, |c, cx| c.set_semantic_word_highlights(Some(false), cx));
            return;
        };
        let current = {
            let s = state.borrow();
            s.document == Some(key) && s.highlights.contains(selection.active)
        };
        if current && !force && !trigger {
            return;
        }
        let Some(slot) = self.nav.slots.get_mut(&(group, secondary)) else {
            return;
        };
        slot.epoch += 1;
        let epoch = slot.epoch;
        if !current {
            // 光标离开了旧符号：旧高亮立即消失；模式（语义 / 文本）保持到新结果到达，不闪烁。
            self.set_highlights(group, secondary, Some(key), SymbolHighlights::default(), cx);
            if code.read(cx).semantic_word_highlights() == Some(true) {
                code.update(cx, |c, cx| c.set_semantic_word_highlights(Some(false), cx));
            }
        }
        let delay = if trigger {
            0
        } else {
            self.nav.settings.occurrences_delay_ms
        };
        let point = QueryPoint {
            resource: document.resource.clone(),
            position: selection.active,
            version: key.1,
        };
        let router = self.nav.router.clone();
        let executor = cx.background_executor().clone();
        let task = cx.background_executor().spawn(async move {
            if delay > 0 {
                executor.timer(Duration::from_millis(delay)).await;
            }
            router.navigate_detailed(NavigationKind::References, &point, &Cancellation::default())
        });
        let resource = document.resource.clone();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                if view
                    .nav
                    .slots
                    .get(&(group, secondary))
                    .is_none_or(|s| s.epoch != epoch)
                    || view
                        .shown_document(group, secondary)
                        .map(|d| (d.id, d.text.version()))
                        != Some(key)
                {
                    return;
                }
                let exact = result.as_ref().is_ok_and(|r| {
                    r.precision == Precision::Exact
                        && r.value.iter().all(|t| t.precision == Precision::Exact)
                });
                let Some(code) = view.group_code(group, secondary).cloned() else {
                    return;
                };
                if exact {
                    let result = result.unwrap();
                    let highlights = SymbolHighlights::from_references(&resource, &result.value);
                    let has = !highlights.is_empty();
                    view.set_highlights(group, secondary, Some(key), highlights, cx);
                    code.update(cx, |c, cx| c.set_semantic_word_highlights(Some(has), cx));
                } else {
                    // 近似：退化为文本高亮，没有提示。
                    view.set_highlights(
                        group,
                        secondary,
                        Some(key),
                        SymbolHighlights::default(),
                        cx,
                    );
                    code.update(cx, |c, cx| c.set_semantic_word_highlights(None, cx));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `editor.action.wordHighlight.next` / `prev`（`f7` / `shift+f7`）：在同名高亮之间跳转（循环）。
    /// 语义高亮生效时用它的区间；文本模式按整词匹配扫描本文件（只在执行命令时扫描）。
    fn step_highlight(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        forward: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(code) = self.group_code(group, secondary).cloned() else {
            return;
        };
        let caret = code.read(cx).selection().active;
        let semantic = self
            .nav
            .slots
            .get(&(group, secondary))
            .map(|s| s.state.borrow().highlights.clone())
            .filter(|h| !h.is_empty() && code.read(cx).semantic_word_highlights().is_some());
        let highlights = match semantic {
            Some(h) => h,
            None => {
                let Some((_, _, word)) = code.read(cx).word_range_at(caret) else {
                    return;
                };
                text_highlights(&code, &word, cx)
            }
        };
        if let Some(range) = highlights.step(caret, forward) {
            code.update(cx, |c, cx| {
                c.set_selection(
                    crate::code_view::Selection::caret(range.start()),
                    Some(Reveal::CenterIfOutside),
                    cx,
                )
            });
        }
    }

    // ------------------------------------------------------------------ 悬停与 Ctrl+悬停

    #[allow(clippy::too_many_arguments)]
    fn pointer_moved(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        position: Position,
        point: Point<Pixels>,
        ctrl: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(code) = self.group_code(group, secondary).cloned() else {
            return;
        };
        let document = self
            .shown_document(group, secondary)
            .filter(|d| is_csharp(&d.metadata.language));
        let Some(document) = document else {
            self.clear_link(cx);
            self.pointer_off_word(cx);
            return;
        };
        let line = position.line.0;
        // 指针真的落在单词上（不是行尾空白钳制过来的位置）。
        let word = code.update(cx, |c, _| {
            let (a, b, _) = c.word_range_at(position)?;
            let start = c.point_for_position(model::pos(line, a))?;
            let end = c.point_for_position(model::pos(line, b))?;
            (point.x >= start.x && point.x < end.x).then_some((a, b, start, c.line_height()))
        });
        let Some((a, b, start, line_height)) = word else {
            self.clear_link(cx);
            self.pointer_off_word(cx);
            return;
        };
        let anchor = Anchor {
            group,
            secondary,
            document: document.id,
            version: document.text.version(),
            resource: document.resource.clone(),
            line,
            start: a,
            end: b,
            point: start,
            line_height,
        };
        if ctrl {
            self.set_link(group, secondary, line, a as u32..b as u32, cx);
            self.show_hover(anchor, true, false, cx);
            return;
        }
        self.clear_link(cx);
        if self.nav.settings.hover_enabled == HoverEnabled::On {
            self.show_hover(anchor, false, false, cx);
        } else {
            self.pointer_off_word(cx);
        }
    }

    fn set_link(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        line: u64,
        range: std::ops::Range<u32>,
        cx: &mut Context<Self>,
    ) {
        if self.nav.link != Some((group, secondary)) {
            self.clear_link(cx);
        }
        let Some((code, state)) = self.slot(group, secondary, cx) else {
            return;
        };
        let old = state.borrow().link.clone();
        if old.as_ref() == Some(&(line, range.clone())) {
            return;
        }
        state.borrow_mut().link = Some((line, range));
        self.nav.link = Some((group, secondary));
        let mut lines = vec![line];
        lines.extend(old.map(|(l, _)| l));
        self.invalidate_slot(group, secondary, Some(lines), cx);
        code.update(cx, |c, cx| c.set_link_cursor(true, cx));
    }

    fn clear_link(&mut self, cx: &mut Context<Self>) {
        let Some((group, secondary)) = self.nav.link.take() else {
            return;
        };
        let old = self
            .nav
            .slots
            .get(&(group, secondary))
            .and_then(|s| s.state.borrow_mut().link.take());
        if let Some((line, _)) = old {
            self.invalidate_slot(group, secondary, Some(vec![line]), cx);
        }
        if let Some(code) = self.group_code(group, secondary).cloned() {
            code.update(cx, |c, cx| c.set_link_cursor(false, cx));
        }
        // 松开 Ctrl：定义预览随之收起。
        if self.nav.hover.definition {
            self.hide_hover(cx);
        }
    }

    /// 在 `anchor` 处显示悬停（`definition`：`Ctrl+悬停` 的定义预览）。同一个单词上不重复查询。
    /// `immediate`：键盘命令，不等 `editor.hover.delay`。
    fn show_hover(
        &mut self,
        anchor: Anchor,
        definition: bool,
        immediate: bool,
        cx: &mut Context<Self>,
    ) {
        let hover = &mut self.nav.hover;
        if hover.phase != Phase::Hidden
            && hover.definition == definition
            && hover.anchor.as_ref().is_some_and(|a| a.same_word(&anchor))
        {
            return;
        }
        hover.epoch += 1;
        let epoch = hover.epoch;
        hover.anchor = Some(anchor.clone());
        hover.definition = definition;
        hover.scroll = HoverScroll::default();
        // 已经显示着卡片时（换了单词）保持显示旧内容直到新结果到达，避免闪烁。
        if !matches!(hover.phase, Phase::Shown(_)) {
            hover.phase = Phase::Waiting;
        }
        let delay = if immediate {
            0
        } else {
            self.nav.settings.hover_delay_ms
        };
        let long_line = self.nav.settings.hover_long_line_warning
            && self
                .group_code(anchor.group, anchor.secondary)
                .and_then(|c| c.read(cx).text().cloned())
                .and_then(|t| t.line_range(Line(anchor.line)).ok())
                .is_some_and(|r| {
                    (r.end().0 - r.start().0) as usize
                        > self.nav.settings.max_tokenization_line_length
                });
        let point = QueryPoint {
            resource: anchor.resource.clone(),
            position: model::pos(anchor.line, anchor.start),
            version: anchor.version,
        };
        let router = self.nav.router.clone();
        let root = self.root.clone();
        let executor = cx.background_executor().clone();
        let started = cx.background_executor().spawn(async move {
            if delay > 0 {
                executor.timer(Duration::from_millis(delay)).await;
            }
        });
        cx.spawn(async move |this, cx| {
            started.await;
            let Ok(task) = this.update(cx, |view, cx| {
                if view.nav.hover.epoch != epoch {
                    return None;
                }
                // 300 ms 内没有结果：显示“正在加载…”（只有索引冷读取时才会出现）。
                let timer = cx.background_executor().timer(LOADING_AFTER);
                cx.spawn(async move |this, cx| {
                    timer.await;
                    let _ = this.update(cx, |view, cx| {
                        if view.nav.hover.epoch == epoch && view.nav.hover.phase == Phase::Waiting {
                            view.nav.hover.phase = Phase::Loading;
                            cx.notify();
                        }
                    });
                })
                .detach();
                Some(cx.background_executor().spawn(async move {
                    hover_content(&router, &point, definition, &root, long_line)
                }))
            }) else {
                return;
            };
            let Some(task) = task else { return };
            let content = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.nav.hover.epoch != epoch {
                    return;
                }
                view.nav.hover.phase = match content {
                    Some(c) if !c.is_empty() => Phase::Shown(c),
                    _ => Phase::Hidden,
                };
                if view.nav.hover.phase == Phase::Hidden {
                    view.nav.hover.anchor = None;
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// 指针离开了单词：`editor.hover.sticky` 时等 `hidingDelay`，鼠标移进卡片就保留；否则立即收起。
    fn pointer_off_word(&mut self, cx: &mut Context<Self>) {
        let hover = &self.nav.hover;
        if hover.phase == Phase::Hidden || hover.over_card {
            return;
        }
        if hover.keyboard {
            return;
        }
        if !self.nav.settings.hover_sticky || !matches!(hover.phase, Phase::Shown(_)) {
            self.hide_hover(cx);
            return;
        }
        let epoch = hover.epoch;
        let timer = cx.background_executor().timer(Duration::from_millis(
            self.nav.settings.hover_hiding_delay_ms,
        ));
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |view, cx| {
                if view.nav.hover.epoch == epoch && !view.nav.hover.over_card {
                    view.hide_hover(cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn hide_hover(&mut self, cx: &mut Context<Self>) {
        let hover = &mut self.nav.hover;
        if hover.phase == Phase::Hidden && hover.anchor.is_none() {
            return;
        }
        hover.epoch += 1;
        hover.phase = Phase::Hidden;
        hover.anchor = None;
        hover.definition = false;
        hover.over_card = false;
        hover.keyboard = false;
        cx.notify();
    }

    /// `editor.action.showHover`（`ctrl+k ctrl+i`）：在插入符处显示悬停（不受 `editor.hover.enabled` 影响）；
    /// 卡片已经显示时把焦点移进卡片（`editorHoverFocused`，卡片内的方向键、翻页键滚动）。
    fn show_hover_command(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.nav.hover.phase, Phase::Shown(_)) {
            let focus = self
                .nav
                .hover
                .focus
                .get_or_insert_with(|| cx.focus_handle())
                .clone();
            window.focus(&focus);
            self.nav.hover.keyboard = true;
            return;
        }
        let Some(code) = self.group_code(group, secondary).cloned() else {
            return;
        };
        let Some(document) = self
            .shown_document(group, secondary)
            .filter(|d| is_csharp(&d.metadata.language))
        else {
            return;
        };
        let caret = code.read(cx).selection().active;
        let found = code.update(cx, |c, _| {
            let (a, b, _) = c.word_range_at(caret)?;
            let start = c.point_for_position(model::pos(caret.line.0, a))?;
            Some((a, b, start, c.line_height()))
        });
        let Some((a, b, start, line_height)) = found else {
            return;
        };
        let anchor = Anchor {
            group,
            secondary,
            document: document.id,
            version: document.text.version(),
            resource: document.resource.clone(),
            line: caret.line.0,
            start: a,
            end: b,
            point: start,
            line_height,
        };
        self.show_hover(anchor, false, true, cx);
    }

    fn scroll_hover(&mut self, m: HoverMove, cx: &mut Context<Self>) {
        let zoom = self.shell.zoom();
        let line = 18.0 * zoom;
        let visible = (CARD_MAX_WIDTH * zoom, CARD_MAX_HEIGHT * zoom);
        // 内容尺寸按行数估算（卡片内容是少量文字行）。
        let rows = match &self.nav.hover.phase {
            Phase::Shown(c) => {
                c.signature.lines().count()
                    + c.preview.len()
                    + c.doc.summary.lines().count()
                    + c.doc.params.len()
                    + usize::from(!c.doc.returns.is_empty())
                    + c.doc.exceptions.len()
                    + c.doc.remarks.lines().count()
                    + 2
            }
            _ => return,
        };
        let content = (visible.0 * 2.0, rows as f32 * line);
        self.nav
            .hover
            .scroll
            .apply(m, line, visible.1 - line, content, visible);
        cx.notify();
    }

    /// 悬停卡片浮层（没有卡片时为 `None`）。
    pub(super) fn nav_overlay(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let hover = &self.nav.hover;
        let anchor = hover.anchor.clone()?;
        let (content, loading) = match &hover.phase {
            Phase::Hidden | Phase::Waiting => return None,
            Phase::Loading => (None, true),
            Phase::Shown(c) => (Some(c.clone()), false),
        };
        let zoom = self.shell.zoom();
        let (code_font, code_size) = self
            .group_code(anchor.group, anchor.secondary)
            .map(|c| {
                let c = c.read(cx);
                (
                    SharedString::from(c.options().font_family.to_string()),
                    c.font_size(),
                )
            })
            .unwrap_or_else(|| ("Consolas".into(), 14.0 * zoom));
        let style = CardStyle {
            colors: self.nav.colors.clone(),
            code_font,
            code_size,
            ui_size: shell::view::UI_FONT_SIZE * zoom,
            max_width: CARD_MAX_WIDTH * zoom,
            max_height: CARD_MAX_HEIGHT * zoom,
        };
        let focus = self
            .nav
            .hover
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let scroll = self.nav.hover.scroll;
        let body = nav_view::card_body(content.as_ref(), loading, &self.strings, &style);
        let status = nav_view::status_row(content.as_ref(), &self.strings, &style);
        let card = nav_view::frame(&style)
            .id("nav-hover")
            .track_focus(&focus)
            .key_context(nav_context::HOVER_FOCUSED)
            .on_hover(cx.listener(|v, hovered: &bool, _, cx| {
                v.nav.hover.over_card = *hovered;
                if !*hovered {
                    v.pointer_off_word(cx);
                }
            }))
            .on_key_down(cx.listener(|v, e: &gpui::KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    v.hide_hover(cx);
                    let group = v.editor.focused;
                    v.focus_group_code(group, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div().flex().flex_col().min_h_0().overflow_hidden().child(
                    div()
                        .flex()
                        .flex_col()
                        .mt(px(-scroll.y))
                        .ml(px(-scroll.x))
                        .children(body),
                ),
            )
            .children(status);
        let (corner, at) = if self.nav.settings.hover_above {
            (Corner::BottomLeft, anchor.point)
        } else {
            (
                Corner::TopLeft,
                point(anchor.point.x, anchor.point.y + px(anchor.line_height)),
            )
        };
        Some(
            deferred(
                anchored()
                    .anchor(corner)
                    .position(at)
                    .snap_to_window_with_margin(px(4.0))
                    .child(card),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

/// 文本模式的全部整词出现（`wordHighlight.next/prev` 用；只在执行命令时扫描）。
fn text_highlights(code: &Entity<CodeView>, word: &str, cx: &App) -> SymbolHighlights {
    let c = code.read(cx);
    let Some(text) = c.text().cloned() else {
        return SymbolHighlights::default();
    };
    let count = text.lines().known;
    let mut ranges = Vec::new();
    for line in 0..count {
        let Some(t) = crate::code_view::columns::read_line(&*text, line) else {
            continue;
        };
        for (i, _) in t.match_indices(word) {
            let p = model::pos(line, i as u64);
            if c.word_range_at(p)
                .is_some_and(|(a, b, _)| a == i as u64 && b == (i + word.len()) as u64)
            {
                if let Ok(r) = TextRange::new(p, model::pos(line, (i + word.len()) as u64)) {
                    ranges.push((r, HighlightKind::Text));
                }
            }
        }
    }
    SymbolHighlights { ranges }
}

/// 后台计算卡片内容。`definition`：`Ctrl+悬停` 的定义预览（定义处开始的几行源码，加上签名与文档）。
fn hover_content(
    router: &CsRouter,
    point: &QueryPoint,
    definition: bool,
    root: &std::path::Path,
    long_line: bool,
) -> Option<HoverContent> {
    let cancel = Cancellation::default();
    let mut content = HoverContent {
        long_line,
        ..Default::default()
    };
    if let Ok(hover) = router.hover_detailed(point, &cancel) {
        content.approximate = hover.precision != Precision::Exact;
        if let Some(h) = hover.value {
            content.signature = h.signature;
            content.doc = parse_xml_doc(&h.documentation);
        }
    }
    if definition {
        let result = router
            .navigate_detailed(NavigationKind::Definition, point, &cancel)
            .ok()?;
        let target = result
            .value
            .iter()
            .find(|t| !model::is_metadata(&t.target))
            .cloned();
        if let Some(t) = target {
            content.approximate |=
                t.precision != Precision::Exact || result.precision != Precision::Exact;
            content.preview = preview_lines(router, root, &t.target);
            if !content.preview.is_empty() {
                // 预览已经包含声明，签名不重复显示。
                content.signature = Arc::from("");
            }
        }
    }
    Some(content)
}

/// 定义处开始的最多 [`PREVIEW_LINES`] 行（打开中的缓冲区优先，否则读盘）。
fn preview_lines(router: &CsRouter, root: &std::path::Path, target: &Target) -> Vec<String> {
    let Resource::Workspace(path) = &target.resource else {
        return Vec::new();
    };
    let first = target.range.start().line.0;
    let last = target
        .range
        .end()
        .line
        .0
        .max(first)
        .min(first + PREVIEW_LINES as u64 - 1);
    if let Some(buffer) = router.buffers().get(path.as_str()) {
        return (first..=last)
            .filter_map(|l| crate::code_view::columns::read_line(&*buffer.text, l))
            .collect();
    }
    let Ok(text) = std::fs::read_to_string(root.join(path.as_str())) else {
        return Vec::new();
    };
    text.lines()
        .skip(first as usize)
        .take((last - first + 1) as usize)
        .map(str::to_owned)
        .collect()
}

/// 等到代码视图某一帧没有占位行，记下 `full_ms`（最多看 30 帧）。
fn watch_full(
    this: gpui::WeakEntity<Workbench>,
    index: usize,
    started: Instant,
    frames: u32,
    window: &mut Window,
) {
    if frames >= 30 {
        return;
    }
    window.on_next_frame(move |window, cx| {
        let done = this
            .update(cx, |v, cx| {
                let stats = v.code_view().read(cx).last_frame();
                let Some(sample) = v.nav.samples.get_mut(index) else {
                    return true;
                };
                if sample.full_ms.is_some() {
                    return true;
                }
                if stats.placeholders + stats.missing == 0 {
                    sample.full_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                    return true;
                }
                false
            })
            .unwrap_or(true);
        if !done {
            watch_full(this, index, started, frames + 1, window);
        }
    });
}
