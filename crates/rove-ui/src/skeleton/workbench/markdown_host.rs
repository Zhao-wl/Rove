//! #29：Markdown 预览的窗口装配。
//!
//! - **一个窗口一个实例**：第一次有预览要显示时才创建后端（WebView2），之前进程里没有任何 WebView2；
//!   多个预览标签共用它，实例显示在“拥有者”（聚焦组优先的、可见的预览标签）上，其余可见的预览显示一行
//!   提示，单击后接管。
//! - **生命周期**：预览不可见时隐藏并挂起；所有预览标签都关闭后启动 `rove.markdown.webviewIdleTimeout`
//!   的定时任务（GPUI 定时器，不依赖渲染帧），到期在前台任务里销毁实例；期间再打开预览则取消定时并复用。
//! - **遮挡**（#6 报告的 Hwnd + 相交隐藏）：快速打开 / 命令面板、转到符号、菜单、模态框、通知、悬停卡片
//!   或拖动标签期间隐藏实例，原位置显示骨架；关闭后恢复。
//! - **焦点**：网页获得焦点时 GPUI 焦点移到预览宿主（上下文键 `activeCustomEditorId`）；网页里按下的
//!   Ctrl/Alt 组合、F 键、Esc 交回键位服务；GPUI 焦点离开预览时把 Win32 焦点还给宿主窗口；
//!   Tab 走到网页边界时回到宿主。
//! - **渲染**：在后台执行器上渲染与净化，结果按序号投递；主题、字体变化只推变量，不重建实例。
use super::*;
use crate::{
    editors::settings::{is_markdown_preview, markdown_preview},
    feedback::{FeedbackAction, FeedbackEvent, FeedbackService},
    markdown::{
        backend::{Backend, BackendEvent, Lifecycle},
        page::{self, Fonts},
        registry::{self as md},
        render::{self, FrontMatter, RenderOptions, Target},
    },
};
use gpui::{canvas, Bounds, Pixels};
use rove_commands::ContextKeys as _;
use serde_json::{json, Value as Json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
    rc::Rc,
};

/// 当前在实例里显示（或正在渲染）的内容。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct RenderKey {
    tab: TabId,
    pub(super) document: DocumentId,
    version: Version,
    options: u64,
}

/// 预览状态的只读快照（测试与 `markdown_probe` 用）。
#[derive(Clone, Debug, Default)]
pub struct MarkdownSnapshot {
    pub lifecycle: Option<&'static str>,
    pub owner: Option<(EditorGroupId, u64)>,
    pub ready: bool,
    pub rendered: bool,
    pub render_sequence: u64,
    pub visible: bool,
    pub stats: Option<Json>,
    pub first_open_ms: Option<f64>,
    pub created_ms: Option<f64>,
    pub remote: usize,
    pub runtime_missing: bool,
    pub idle_pending: bool,
    pub process_ids: Vec<u32>,
    /// 最近一次 [`Workbench::markdown_probe`] 读回的预览顶部源码行。
    pub position: Option<f64>,
}

pub(super) struct MarkdownPreview {
    backend: Rc<RefCell<Box<dyn Backend>>>,
    #[cfg_attr(not(test), allow(dead_code))]
    sender: crate::markdown::backend::EventSender,
    receiver: Option<async_channel::Receiver<BackendEvent>>,
    feedback: Option<rove_base::Subscription>,
    pub(super) focus: Option<FocusHandle>,
    was_focused: bool,
    pub(super) owner: Option<(EditorGroupId, TabId)>,
    ready: bool,
    pending: Vec<Json>,
    seq: u64,
    pub(super) requested: Option<RenderKey>,
    rendered: Option<u64>,
    stats: Option<Json>,
    lines: HashMap<TabId, f64>,
    remote: BTreeMap<DocumentId, usize>,
    runtime_missing: bool,
    notified: bool,
    idle_generation: u64,
    idle_pending: bool,
    /// 这一帧实例是否显示（画布在 prepaint 时读取：遮挡判定在编辑区构建之后才知道）。
    visible: Rc<Cell<bool>>,
    /// 预览→编辑器滚动后，编辑器回报的视口不再回推（防回声）。
    echo: Option<(DocumentId, f64)>,
    fragment: Option<(Resource, String)>,
    options_generation: u64,
    open_started: Option<Instant>,
    first_open_ms: Option<f64>,
    created_ms: Option<f64>,
    external: Option<String>,
    position: Option<f64>,
    trace: bool,
    #[cfg(test)]
    pub(super) fake: Rc<RefCell<crate::markdown::backend::FakeLog>>,
}

fn new_backend(
    #[cfg_attr(not(test), allow(dead_code))] sender: crate::markdown::backend::EventSender,
    #[cfg(test)] log: Rc<RefCell<crate::markdown::backend::FakeLog>>,
) -> Box<dyn Backend> {
    #[cfg(test)]
    {
        Box::new(crate::markdown::backend::FakeBackend::new(
            sender, log, false,
        ))
    }
    #[cfg(all(not(test), windows))]
    {
        Box::new(crate::markdown::webview::WebView2Backend::new(sender))
    }
    #[cfg(all(not(test), not(windows)))]
    {
        Box::new(crate::markdown::backend::UnavailableBackend(sender))
    }
}

impl Default for MarkdownPreview {
    fn default() -> Self {
        let (sender, receiver) = async_channel::unbounded();
        #[cfg(test)]
        let fake = Rc::new(RefCell::new(Default::default()));
        let backend = new_backend(
            sender.clone(),
            #[cfg(test)]
            fake.clone(),
        );
        Self {
            backend: Rc::new(RefCell::new(backend)),
            sender,
            receiver: Some(receiver),
            feedback: None,
            focus: None,
            was_focused: false,
            owner: None,
            ready: false,
            pending: Vec::new(),
            seq: 0,
            requested: None,
            rendered: None,
            stats: None,
            lines: HashMap::new(),
            remote: BTreeMap::new(),
            runtime_missing: false,
            notified: false,
            idle_generation: 0,
            idle_pending: false,
            visible: Rc::new(Cell::new(false)),
            echo: None,
            fragment: None,
            options_generation: 0,
            open_started: None,
            first_open_ms: None,
            created_ms: None,
            external: None,
            position: None,
            trace: std::env::var_os("ROVE_MARKDOWN_TRACE").is_some(),
            #[cfg(test)]
            fake,
        }
    }
}

#[cfg(test)]
impl MarkdownPreview {
    pub(super) fn sender_for_test(&self) -> crate::markdown::backend::EventSender {
        self.sender.clone()
    }
    pub(super) fn replace_backend(&mut self, backend: Box<dyn Backend>) {
        *self.backend.borrow_mut() = backend;
    }
}

fn lifecycle_name(lifecycle: Lifecycle) -> &'static str {
    match lifecycle {
        Lifecycle::Absent => "absent",
        Lifecycle::Creating => "creating",
        Lifecycle::Alive => "alive",
    }
}

impl Workbench {
    fn md_key<T: rove_config::SettingValue>(&self, key: &'static str, fallback: T) -> T {
        Key::<T>::new(key)
            .read(&*self.settings, &Scope::default())
            .unwrap_or(fallback)
    }

    /// 预览状态快照（测试、`markdown_probe`）。
    pub fn markdown_snapshot(&self) -> MarkdownSnapshot {
        let m = &self.markdown;
        let backend = m.backend.borrow();
        MarkdownSnapshot {
            lifecycle: Some(lifecycle_name(backend.lifecycle())),
            owner: m.owner,
            ready: m.ready,
            rendered: m.rendered.is_some() && m.rendered == m.requested.map(|_| m.seq),
            render_sequence: m.seq,
            visible: m.visible.get(),
            stats: m.stats.clone(),
            first_open_ms: m.first_open_ms,
            created_ms: m.created_ms,
            remote: m
                .requested
                .and_then(|k| m.remote.get(&k.document).copied())
                .unwrap_or(0),
            runtime_missing: m.runtime_missing,
            idle_pending: m.idle_pending,
            process_ids: backend.process_ids(),
            position: m.position,
        }
    }

    /// 测量用（`examples/markdown_probe.rs`）：`where` 读回预览顶部对应的源码行（结果在
    /// [`MarkdownSnapshot::position`]）；`userScroll` 像用户一样把预览滚到某一行。
    pub fn markdown_probe(&mut self, kind: &str, line: f64) {
        self.markdown.position = None;
        let document = self.markdown.requested.map(|k| k.document.0);
        self.markdown_post(json!({
            "version": 1, "type": kind, "documentId": document, "line": line,
        }));
    }

    fn preview_tab(&self, group: EditorGroupId) -> Option<&Tab> {
        self.editor
            .groups
            .get(&group)?
            .current()
            .filter(|tab| is_markdown_preview(&tab.kind))
    }

    fn any_preview_tab(&self) -> bool {
        self.editor
            .groups
            .values()
            .flat_map(|g| &g.tabs)
            .any(|tab| is_markdown_preview(&tab.kind))
    }

    fn displayed_groups(&self) -> Vec<EditorGroupId> {
        match self.editor.maximized {
            Some(group) => vec![group],
            None => self.editor.groups.keys().copied().collect(),
        }
    }

    fn resource_path(&self, resource: &Resource) -> Option<PathBuf> {
        match resource {
            Resource::Workspace(p) => Some(self.root.join(p.as_str())),
            Resource::File(p) => Some(p.clone()),
            Resource::Virtual(_) => None,
        }
    }

    /// 每帧（`Render` 开头）：选拥有者、按需创建实例、请求渲染、空闲回收、上下文键与焦点。
    pub(super) fn sync_markdown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.markdown_listen(window, cx);
        let focus = self
            .markdown
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let candidates: Vec<(EditorGroupId, TabId)> = self
            .displayed_groups()
            .into_iter()
            .filter_map(|g| {
                self.preview_tab(g)
                    .filter(|t| t.opened().is_some())
                    .map(|t| (g, t.id))
            })
            .collect();
        let focused = self.editor.focused;
        let owner = candidates
            .iter()
            .find(|(g, _)| *g == focused)
            .or_else(|| {
                candidates
                    .iter()
                    .find(|(g, _)| Some(*g) == self.markdown.owner.map(|o| o.0))
            })
            .or_else(|| candidates.first())
            .copied();
        self.markdown.owner = owner;
        // VS Code 的上下文键：活动编辑器是 Markdown 预览（`markdown.togglePreview` 的 when）。
        let active_preview = self.preview_tab(focused).is_some();
        if let Some(keys) = cx.try_global::<crate::keys::Keys>() {
            let _ = keys.context.set(
                md::ACTIVE_CUSTOM_EDITOR_ID.into(),
                active_preview.then(|| Value::String(md::EDITOR_ID.into())),
            );
        }
        let lifecycle = self.markdown.backend.borrow().lifecycle();
        if let Some((_, tab)) = owner {
            if self.markdown.idle_pending {
                self.markdown.idle_pending = false;
                self.markdown.idle_generation += 1;
            }
            if lifecycle == Lifecycle::Absent && !self.markdown.runtime_missing {
                self.markdown.open_started.get_or_insert_with(Instant::now);
                self.markdown.ready = false;
                self.markdown.requested = None;
                let background = self.code_view().read(cx).palette().background;
                let mut backend = self.markdown.backend.borrow_mut();
                backend.set_background((background.0, background.1, background.2, background.3));
                backend.create(window);
            }
            self.markdown_request_render(tab, cx);
        } else {
            self.markdown.visible.set(false);
            let mut backend = self.markdown.backend.borrow_mut();
            backend.layout(None, window.scale_factor());
            backend.suspend();
            drop(backend);
            if !self.any_preview_tab()
                && lifecycle != Lifecycle::Absent
                && !self.markdown.idle_pending
            {
                self.markdown_start_idle(cx);
            }
        }
        // 焦点：GPUI 焦点刚进入预览 → 把 Win32 焦点交给网页；刚离开 → 还给宿主窗口。
        let now = focus.is_focused(window);
        if now && !self.markdown.was_focused && owner.is_some() {
            let mut backend = self.markdown.backend.borrow_mut();
            if !backend.has_focus() {
                backend.focus(false);
            }
        } else if !now && self.markdown.was_focused {
            let mut backend = self.markdown.backend.borrow_mut();
            if backend.has_focus() {
                backend.release_focus();
            }
        }
        self.markdown.was_focused = now;
    }

    /// `Render` 末尾：弹出层与预览相交时隐藏实例（#6 报告的 Hwnd + 相交隐藏）。
    pub(super) fn sync_markdown_overlays(
        &mut self,
        overlay: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shell = &self.shell;
        let covered = overlay
            || shell.menu_open()
            || shell.feedback.modal().is_some()
            || shell.feedback.toasts().next().is_some()
            || shell.feedback.center_visible()
            || cx.has_active_drag();
        let visible = self.markdown.owner.is_some() && !covered;
        self.markdown.visible.set(visible);
        let mut backend = self.markdown.backend.borrow_mut();
        backend.set_zoom(self.shell.zoom() as f64);
        if !visible {
            backend.layout(None, window.scale_factor());
        }
    }

    fn markdown_listen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(receiver) = self.markdown.receiver.take() {
            cx.spawn_in(window, async move |this, cx| {
                while let Ok(event) = receiver.recv().await {
                    if this
                        .update_in(cx, |view, window, cx| {
                            view.markdown_event(event, window, cx)
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        if self.markdown.feedback.is_none() {
            let (send, receive) = async_channel::unbounded::<FeedbackEvent>();
            self.markdown.feedback = Some(self.shell.feedback.events().subscribe(Arc::new(
                move |event| {
                    let _ = send.try_send(event);
                },
            )));
            cx.spawn(async move |this, cx| {
                while let Ok(event) = receive.recv().await {
                    let FeedbackEvent::Action { key, action } = event else {
                        continue;
                    };
                    if !key.starts_with("markdown.") {
                        continue;
                    }
                    if this
                        .update(cx, |view, cx| view.markdown_feedback(&key, &action, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
    }

    pub(super) fn markdown_feedback(&mut self, key: &str, action: &str, cx: &mut Context<Self>) {
        match action {
            md::ACTION_DOWNLOAD_PAGE => cx.open_url(md::WEBVIEW2_DOWNLOAD_PAGE),
            md::ACTION_OPEN_LINK => {
                if let Some(url) = self.markdown.external.take() {
                    cx.open_url(&url);
                }
                let _ = FeedbackService::dismiss(&mut self.shell.feedback, key);
            }
            md::ACTION_CANCEL => {
                self.markdown.external = None;
                let _ = FeedbackService::dismiss(&mut self.shell.feedback, key);
            }
            _ => {}
        }
        cx.notify();
    }

    fn markdown_start_idle(&mut self, cx: &mut Context<Self>) {
        self.markdown.idle_pending = true;
        self.markdown.idle_generation += 1;
        let generation = self.markdown.idle_generation;
        let seconds = self
            .md_key::<f64>("rove.markdown.webviewIdleTimeout", 300.0)
            .max(0.0);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(seconds))
                .await;
            let _ = this.update(cx, |view, cx| {
                let m = &mut view.markdown;
                if m.idle_generation != generation || !m.idle_pending {
                    return;
                }
                m.idle_pending = false;
                if view.any_preview_tab() {
                    return;
                }
                // 定时任务里同步 Close（不在输入回调里）；释放浏览器进程。
                view.markdown.backend.borrow_mut().dispose();
                let m = &mut view.markdown;
                m.ready = false;
                m.requested = None;
                m.rendered = None;
                m.pending.clear();
                if m.trace {
                    println!("MARKDOWN_DISPOSED");
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_options(&self) -> RenderOptions {
        let alert = |key: &str| self.label(key, &[]);
        RenderOptions {
            breaks: self.md_key("markdown.preview.breaks", false),
            linkify: self.md_key("markdown.preview.linkify", true),
            typographer: self.md_key("markdown.preview.typographer", false),
            math: self.md_key("markdown.math.enabled", true),
            front_matter: match self
                .md_key::<String>("markdown.preview.frontMatter", "table".into())
                .as_str()
            {
                "hide" => FrontMatter::Hide,
                "table" => FrontMatter::Table,
                _ => FrontMatter::CodeBlock,
            },
            alerts: render::AlertTitles {
                note: alert("markdown.alert.note"),
                tip: alert("markdown.alert.tip"),
                important: alert("markdown.alert.important"),
                warning: alert("markdown.alert.warning"),
                caution: alert("markdown.alert.caution"),
            },
        }
    }

    fn fonts(&self) -> Fonts {
        Fonts {
            family: self.md_key(
                "markdown.preview.fontFamily",
                "-apple-system, BlinkMacSystemFont, 'Segoe WPC', 'Segoe UI', system-ui, 'Ubuntu', 'Droid Sans', sans-serif".to_string(),
            ),
            size: self.md_key("markdown.preview.fontSize", 14.0),
            line_height: self.md_key("markdown.preview.lineHeight", 1.6),
            editor_family: self.md_key("editor.fontFamily", "Consolas, 'Courier New', monospace".to_string()),
            editor_size: self.md_key("editor.fontSize", 14.0),
        }
    }

    fn markdown_post(&mut self, message: Json) {
        if self.markdown.ready {
            self.markdown.backend.borrow_mut().post(&message);
        } else {
            self.markdown.pending.push(message);
        }
    }

    fn markdown_send_theme(&mut self) {
        let kind = self.theme_service.current().kind();
        let message = page::theme_message(self.theme.as_ref(), kind, &self.fonts());
        self.markdown_post(message);
    }

    /// 拥有者换了文档、文档有新版本、选项或放行变化时，在后台重新渲染。
    fn markdown_request_render(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let Some((group, _)) = self.markdown.owner else {
            return;
        };
        let Some(opened) = self.editor.groups[&group].tab(tab).and_then(Tab::opened) else {
            return;
        };
        let document = opened.document.clone();
        let options = self.render_options();
        let key = RenderKey {
            tab,
            document: document.id,
            version: document.text.version(),
            options: self.markdown.options_generation,
        };
        if self.markdown.requested == Some(key) {
            return;
        }
        let same_document = self
            .markdown
            .requested
            .is_some_and(|old| old.document == key.document && old.tab == key.tab);
        self.markdown.requested = Some(key);
        self.markdown.seq += 1;
        let seq = self.markdown.seq;
        let base = self
            .resource_path(&document.resource)
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let fragment = self
            .markdown
            .fragment
            .take_if(|(r, _)| *r == document.resource)
            .map(|(_, f)| f);
        let line = if same_document {
            None
        } else {
            self.markdown.lines.get(&tab).copied()
        };
        let mut styles = Vec::new();
        let mut drives = BTreeSet::new();
        let style_list: Value = self.md_key("markdown.styles", Value::Null);
        if let Value::Array(items) = style_list {
            for item in items.iter() {
                let Value::String(style) = item else { continue };
                let Target::Local(path, _) = render::classify(style, Some(&self.root)) else {
                    continue;
                };
                if let Some((drive, url)) = render::file_url(&path) {
                    drives.insert(drive);
                    styles.push(url);
                }
            }
        }
        let macros: Value = self.md_key("markdown.math.macros", Value::Null);
        let macros = to_json(&macros);
        let mark = self.md_key("markdown.preview.markEditorSelection", false);
        let text = document.text.clone();
        let task = cx.background_executor().spawn(async move {
            let mut source = String::new();
            if let Ok(range) = rove_base::Span::new(
                rove_base::ByteOffset(0),
                rove_base::ByteOffset(text.byte_len()),
            ) {
                let _ = text.visit_chunks(range, &mut |chunk| source.push_str(chunk));
            }
            render::render(&source, base.as_deref(), &options)
        });
        let id = document.id;
        cx.spawn(async move |this, cx| {
            let rendered = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.markdown.seq != seq {
                    return;
                }
                let mut all = rendered.drives.clone();
                all.extend(drives);
                view.markdown.backend.borrow_mut().map_drives(&all);
                view.markdown.remote.insert(id, rendered.remote);
                view.markdown_remote_bar(id, rendered.remote, cx);
                let mut message = json!({
                    "version": 1,
                    "type": "render",
                    "seq": seq,
                    "documentId": id.0,
                    "html": rendered.html,
                    "styles": styles,
                    "mathMacros": macros,
                    "markEditorSelection": mark,
                });
                if let Some(line) = line {
                    message["line"] = json!(line);
                }
                if let Some(fragment) = fragment {
                    message["fragment"] = json!(fragment);
                }
                view.markdown_post(message);
                cx.notify();
            });
        })
        .detach();
    }

    /// 预览上方的第 3 级提示条：已阻止 N 个远程资源。
    fn markdown_remote_bar(&mut self, document: DocumentId, remote: usize, cx: &mut Context<Self>) {
        let key = format!("{}:{}", md::REMOTE_BAR, document.0);
        if remote == 0 {
            let _ = FeedbackService::dismiss(&mut self.shell.feedback, &key);
            return;
        }
        let message = Message {
            key: "markdown.remoteBlocked".into(),
            args: Arc::from([("count".into(), remote.to_string().into())]),
        };
        let _ = shell::view::report(
            self,
            &key,
            Reason::FileContext {
                document,
                reason: md::REMOTE_BAR.into(),
            },
            message,
            Arc::from([]),
            cx,
        );
    }

    fn markdown_event(&mut self, event: BackendEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            BackendEvent::Created { ms } => {
                self.markdown.created_ms = Some(ms);
                if self.markdown.trace {
                    println!("MARKDOWN_CONTROLLER_MS {ms:.1}");
                }
            }
            BackendEvent::RuntimeMissing(error) => {
                self.markdown_unavailable(&error, true, cx);
            }
            BackendEvent::Failed(error) => {
                let created = self.markdown.created_ms.is_some();
                self.markdown.backend.borrow_mut().dispose();
                self.markdown.ready = false;
                self.markdown.requested = None;
                if !created {
                    self.markdown_unavailable(&error, false, cx);
                }
            }
            BackendEvent::Message(message) => self.markdown_message(message, window, cx),
            BackendEvent::Focus(true) => {
                if let Some((group, _)) = self.markdown.owner {
                    self.editor.focus(group);
                    self.focus_group = Some(group);
                }
                if let Some(focus) = self.markdown.focus.clone() {
                    window.focus(&focus);
                }
                self.markdown.was_focused = true;
            }
            BackendEvent::Focus(false) => {}
            BackendEvent::Key(keystroke) => {
                if let Some(focus) = self.markdown.focus.clone() {
                    if !focus.is_focused(window) {
                        window.focus(&focus);
                        self.markdown.was_focused = true;
                    }
                }
                window.dispatch_keystroke(keystroke, cx);
            }
            BackendEvent::Navigate(uri) => {
                if !uri.is_empty() && uri != "about:blank" {
                    self.markdown_link(&uri, cx);
                }
            }
            BackendEvent::RemoteBlocked(uri) => {
                if self.markdown.trace {
                    println!("MARKDOWN_REMOTE_BLOCKED {uri}");
                }
            }
        }
        cx.notify();
    }

    /// 没有 WebView2 运行时（或创建失败）：所有预览改为源码，以后 `.md` 也以源码打开；只通知一次，
    /// 附微软官方下载页（只在用户点击时交给系统浏览器打开，Rove 不下载任何东西）。
    fn markdown_unavailable(&mut self, error: &str, missing: bool, cx: &mut Context<Self>) {
        self.markdown.runtime_missing = true;
        self.editor.settings.editors.set_preview_available(false);
        let tabs: Vec<(EditorGroupId, TabId)> = self
            .editor
            .groups
            .iter()
            .flat_map(|(g, pane)| {
                pane.tabs
                    .iter()
                    .filter(|t| is_markdown_preview(&t.kind))
                    .map(|t| (*g, t.id))
                    .collect::<Vec<_>>()
            })
            .collect();
        let focused = self.editor.focused;
        for (group, tab) in tabs {
            self.editor.reopen_as(group, tab, EditorKind::Text, None);
        }
        self.editor.focus(focused);
        if self.markdown.notified {
            return;
        }
        self.markdown.notified = true;
        let (key, message, actions): (_, _, Arc<[FeedbackAction]>) = if missing {
            (
                md::RUNTIME_MISSING,
                Message::key("markdown.webviewMissing"),
                Arc::from([FeedbackAction {
                    id: md::ACTION_DOWNLOAD_PAGE.into(),
                    label: Message::key("markdown.openDownloadPage"),
                }]),
            )
        } else {
            (
                md::CREATE_FAILED,
                Message {
                    key: "markdown.webviewFailed".into(),
                    args: Arc::from([("error".into(), error.into())]),
                },
                Arc::from([]),
            )
        };
        eprintln!("markdown: WebView2 不可用：{error}");
        let _ = shell::view::report(
            self,
            key,
            Reason::Global(shell::feedback::Severity::Warning),
            message,
            actions,
            cx,
        );
    }

    fn markdown_message(&mut self, message: Json, window: &mut Window, cx: &mut Context<Self>) {
        let document = message["documentId"].as_u64().map(DocumentId);
        let current = self.markdown.requested.map(|k| k.document);
        match message["type"].as_str() {
            Some("ready") => {
                self.markdown.ready = true;
                if self.markdown.trace {
                    if let Some(start) = self.markdown.open_started {
                        println!(
                            "MARKDOWN_PAGE_READY_MS {:.1}",
                            start.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                }
                self.markdown_send_theme();
                let pending = std::mem::take(&mut self.markdown.pending);
                let mut backend = self.markdown.backend.borrow_mut();
                for message in pending {
                    backend.post(&message);
                }
            }
            Some("rendered") => {
                let seq = message["seq"].as_u64();
                if seq == Some(self.markdown.seq) {
                    self.markdown.rendered = seq;
                    self.markdown.stats = Some(message["stats"].clone());
                    if self.markdown.first_open_ms.is_none() {
                        if let Some(start) = self.markdown.open_started {
                            let ms = start.elapsed().as_secs_f64() * 1000.0;
                            self.markdown.first_open_ms = Some(ms);
                            if self.markdown.trace {
                                println!("MARKDOWN_FIRST_RENDER_MS {ms:.1}");
                            }
                        }
                    }
                    if self.markdown.trace {
                        println!("MARKDOWN_RENDERED {}", message["stats"]);
                    }
                }
            }
            Some("scroll") if document.is_some() && document == current => {
                let line = message["line"].as_f64().unwrap_or(0.0);
                if let Some((_, tab)) = self.markdown.owner {
                    self.markdown.lines.insert(tab, line);
                }
                if self.md_key("markdown.preview.scrollEditorWithPreview", true) {
                    self.markdown_scroll_editors(document.unwrap(), line, cx);
                }
            }
            Some("dblclick") if document.is_some() && document == current => {
                if self.md_key("markdown.preview.doubleClickToSwitchToEditor", true) {
                    let line = message["line"].as_f64().unwrap_or(0.0).max(0.0) as u32;
                    if let Some((group, tab)) = self.markdown.owner {
                        self.markdown.backend.borrow_mut().release_focus();
                        self.editor
                            .reopen_as(group, tab, EditorKind::Text, Some(line));
                        self.sync_views(cx);
                        self.focus_group_code(group, window, cx);
                    }
                }
            }
            Some("link") if document.is_some() && document == current => {
                if let Some(href) = message["href"].as_str() {
                    let href = href.to_string();
                    self.markdown_link(&href, cx);
                }
            }
            Some("position") => {
                self.markdown.position = message["line"].as_f64();
            }
            Some("focusExit") => {
                self.markdown.backend.borrow_mut().release_focus();
            }
            _ => {}
        }
    }

    /// 预览滚动 → 显示同一文档的代码视图跟着滚动（`markdown.preview.scrollEditorWithPreview`）。
    fn markdown_scroll_editors(&mut self, document: DocumentId, line: f64, cx: &mut Context<Self>) {
        let slots: Vec<Entity<CodeView>> = self
            .views
            .values()
            .flat_map(|v| std::iter::once(&v.primary).chain(&v.secondary))
            .filter(|slot| slot.shown.is_some_and(|s| s.document == document))
            .map(|slot| slot.code.clone())
            .collect();
        for code in slots {
            self.markdown.echo = Some((document, line));
            code.update(cx, |code, cx| {
                let x = code.scroll_position().x;
                code.set_scroll_position(ScrollPosition { top: line, x }, false, cx);
            });
        }
    }

    /// 编辑器滚动 / 选区变化 → 预览跟随（`markdown.preview.scrollPreviewWithEditor`、`markEditorSelection`）。
    pub(super) fn markdown_code_event(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        event: &CodeViewEvent,
    ) {
        let Some(current) = self.markdown.requested.map(|k| k.document) else {
            return;
        };
        let Some(document) = self.shown_document(group, secondary).map(|d| d.id) else {
            return;
        };
        if document != current {
            return;
        }
        match event {
            CodeViewEvent::ViewportChanged(viewport) => {
                if let Some((echo_doc, line)) = self.markdown.echo {
                    if echo_doc == document && (viewport.top - line).abs() < 1.0 {
                        self.markdown.echo = None;
                        return;
                    }
                }
                if self.md_key("markdown.preview.scrollPreviewWithEditor", true) {
                    self.markdown_post(json!({
                        "version": 1, "type": "scrollTo", "documentId": document.0, "line": viewport.top,
                    }));
                }
            }
            CodeViewEvent::SelectionChanged { selection, .. }
                if self.md_key("markdown.preview.markEditorSelection", false) =>
            {
                self.markdown_post(json!({
                    "version": 1, "type": "selection", "documentId": document.0,
                    "line": selection.active.line.0,
                }));
            }
            _ => {}
        }
    }

    /// 预览里点击的链接：`#片段` 已由页面自己滚动；相对路径的 `.md` 在预览中打开
    /// （`markdown.preview.openMarkdownLinks`、`markdown.links.openLocation`），其他本地文件以源码打开；
    /// 外部链接打开前确认（FEATURES §3），确认后交给系统浏览器。
    fn markdown_link(&mut self, href: &str, cx: &mut Context<Self>) {
        let base = self
            .markdown
            .owner
            .and_then(|(g, t)| self.editor.groups.get(&g)?.tab(t))
            .and_then(|t| self.resource_path(&t.resource))
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let lower = href.to_ascii_lowercase();
        let external = lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("mailto:");
        if external {
            self.markdown.external = Some(href.to_string());
            let message = Message {
                key: "markdown.openExternal".into(),
                args: Arc::from([("url".into(), href.into())]),
            };
            let _ = shell::view::report(
                self,
                md::EXTERNAL_LINK,
                Reason::SecurityConfirmation,
                message,
                Arc::from([
                    FeedbackAction {
                        id: md::ACTION_OPEN_LINK.into(),
                        label: Message::key("markdown.open"),
                    },
                    FeedbackAction {
                        id: md::ACTION_CANCEL.into(),
                        label: Message::key("markdown.cancel"),
                    },
                ]),
                cx,
            );
            return;
        }
        let Target::Local(path, fragment) = render::classify(href, base.as_deref()) else {
            return;
        };
        let resource = match path.strip_prefix(&self.root) {
            Ok(relative) => WorkspacePath::new(&relative.to_string_lossy().replace('\\', "/"))
                .map(Resource::Workspace)
                .unwrap_or(Resource::File(path.clone())),
            Err(_) => Resource::File(path.clone()),
        };
        let is_md = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"));
        let in_preview = is_md
            && self.md_key::<String>("markdown.preview.openMarkdownLinks", "inPreview".into())
                == "inPreview"
            && self.editor.settings.editors.preview_available();
        let beside =
            self.md_key::<String>("markdown.links.openLocation", "currentGroup".into()) == "beside";
        if let Some(fragment) = fragment.filter(|_| in_preview) {
            self.markdown.fragment = Some((resource.clone(), fragment));
        }
        let options = OpenOptions {
            target: if beside {
                crate::documents::OpenTarget::Beside
            } else {
                crate::documents::OpenTarget::Active
            },
            presentation: if in_preview {
                crate::documents::Presentation::Preview
            } else {
                crate::documents::Presentation::Source
            },
            ..Default::default()
        };
        if let Err(e) = self
            .open_documents()
            .open(resource, options, Cancellation::default())
        {
            eprintln!("markdown link {href}: {e}");
        }
        cx.notify();
    }

    pub(super) fn is_markdown_command(id: &str) -> bool {
        md::COMMANDS.iter().any(|(c, _, _)| *c == id)
    }

    /// `markdown.togglePreview` / `showPreview` / `showPreviewToSide` / `showSource` / `preview.refresh`。
    pub(super) fn markdown_command(&mut self, id: &str, cx: &mut Context<Self>) {
        let group = self.editor.focused;
        let Some(tab) = self.editor.active().current() else {
            return;
        };
        let tab_id = tab.id;
        let resource = tab.resource.clone();
        let preview = is_markdown_preview(&tab.kind);
        let markdown_source = !preview
            && tab
                .opened()
                .is_some_and(|o| &*o.document.metadata.language == "markdown");
        let top = self
            .group_code(group, false)
            .and_then(|c| c.read(cx).viewport())
            .map(|v| v.top);
        match id {
            md::TOGGLE_PREVIEW if preview => {
                let line = self.markdown.lines.get(&tab_id).map(|l| l.max(0.0) as u32);
                self.markdown.backend.borrow_mut().release_focus();
                self.editor.reopen_as(group, tab_id, EditorKind::Text, line);
            }
            md::TOGGLE_PREVIEW if markdown_source => {
                if !self.editor.settings.editors.preview_available() {
                    return;
                }
                if let Some(top) = top {
                    self.markdown.lines.insert(tab_id, top);
                }
                self.editor
                    .reopen_as(group, tab_id, markdown_preview(), None);
            }
            md::SHOW_PREVIEW | md::SHOW_PREVIEW_TO_SIDE if markdown_source => {
                let beside = id == md::SHOW_PREVIEW_TO_SIDE;
                let options = OpenOptions {
                    target: if beside {
                        crate::documents::OpenTarget::Beside
                    } else {
                        crate::documents::OpenTarget::Active
                    },
                    preserve_focus: beside,
                    presentation: crate::documents::Presentation::Preview,
                    ..Default::default()
                };
                if let Err(e) =
                    self.open_documents()
                        .open(resource, options, Cancellation::default())
                {
                    eprintln!("{id}: {e}");
                }
            }
            md::SHOW_SOURCE if preview => {
                let line = self.markdown.lines.get(&tab_id).copied().unwrap_or(0.0) as u32;
                let options = OpenOptions {
                    location: Some(rove_base::Position {
                        line: rove_base::Line(line as u64),
                        column: rove_base::Utf8Column(0),
                    }),
                    ..Default::default()
                };
                self.markdown.backend.borrow_mut().release_focus();
                if let Err(e) =
                    self.open_documents()
                        .open(resource, options, Cancellation::default())
                {
                    eprintln!("{id}: {e}");
                }
            }
            md::REFRESH => {
                self.markdown.options_generation += 1;
            }
            _ => return,
        }
        self.sync_views(cx);
        cx.notify();
    }

    /// 主题切换：只推变量，不重建实例。
    pub(super) fn markdown_theme_changed(&mut self) {
        if self.markdown.backend.borrow().lifecycle() != Lifecycle::Absent {
            let background = self.code_view_background();
            self.markdown
                .backend
                .borrow_mut()
                .set_background(background);
            self.markdown_send_theme();
        }
    }

    fn code_view_background(&self) -> (u8, u8, u8, u8) {
        let c = self
            .theme
            .color("editor.background")
            .ok()
            .flatten()
            .unwrap_or(rove_theme::Color(0x1e, 0x1e, 0x1e, 0xff));
        (c.0, c.1, c.2, c.3)
    }

    /// `markdown.*`、`rove.markdown.*`、`editor.font*` 变化：重新渲染并重推字体变量。
    pub(super) fn markdown_settings_changed(&mut self, keys: &BTreeSet<Arc<str>>) {
        if !self.markdown.runtime_missing {
            self.editor.settings.editors.set_preview_available(true);
        } else {
            self.editor.settings.editors.set_preview_available(false);
        }
        if keys.iter().any(|k| {
            k.starts_with("markdown.")
                || k.starts_with("rove.markdown.")
                || k.starts_with("editor.font")
        }) {
            self.markdown.options_generation += 1;
            if self.markdown.backend.borrow().lifecycle() != Lifecycle::Absent {
                self.markdown_send_theme();
            }
        }
    }

    /// 编辑器标题栏上 `Placement::EditorTitle` 登记的按钮（显示源码 / 显示预览）。
    pub(super) fn markdown_title_actions(
        &self,
        group: EditorGroupId,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(tab) = self.editor.groups.get(&group).and_then(|g| g.current()) else {
            return Vec::new();
        };
        let preview = is_markdown_preview(&tab.kind);
        let markdown = !preview
            && self.editor.settings.editors.preview_available()
            && tab
                .opened()
                .is_some_and(|o| &*o.document.metadata.language == "markdown");
        let mut values: BTreeMap<&str, Value> = BTreeMap::new();
        if preview {
            values.insert(
                md::ACTIVE_CUSTOM_EDITOR_ID,
                Value::String(md::EDITOR_ID.into()),
            );
        }
        if markdown {
            values.insert("resourceLangId", Value::String("markdown".into()));
        }
        let context = TitleContext(values);
        let size = px(14.0 * self.shell.zoom());
        self.shell
            .contributions
            .entries()
            .filter(|(_, c)| matches!(c.placement, crate::contributions::Placement::EditorTitle))
            .filter(|(_, c)| {
                c.when.as_ref().is_none_or(|w| {
                    rove_commands::when::WhenExpr::parse(&w.0)
                        .map(|e| e.evaluate(&context))
                        .unwrap_or(false)
                })
            })
            .filter_map(|(id, c)| {
                let command: &'static str = md::COMMANDS
                    .iter()
                    .find(|(cmd, _, _)| Some(*cmd) == c.command.as_deref())?
                    .0;
                let title = self.strings.text(&c.title).ok()?;
                let icon = format!("$({})", c.icon.as_deref().unwrap_or("open-preview"));
                Some(
                    div()
                        .id(SharedString::from(format!("{id}-{}", group.0)))
                        .h_full()
                        .px_1()
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(self.ui_color("tab.hoverBackground")))
                        .child(crate::shell::view::icon_label(&icon, size))
                        .tooltip(move |_, cx| cx.new(|_| TitleTooltip(title.to_string())).into())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.editor.focus(group);
                            view.markdown_command(command, cx);
                        }))
                        .into_any_element(),
                )
            })
            .collect()
    }

    /// 预览标签的编辑区：骨架占位、实例位置（画布在 prepaint 时把逻辑坐标交给后端）与焦点宿主。
    pub(super) fn markdown_viewport(
        &self,
        group: EditorGroupId,
        tab: TabId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = self.markdown.owner == Some((group, tab));
        let rendered = owner
            && self.markdown.rendered.is_some()
            && self.markdown.rendered == self.markdown.requested.map(|_| self.markdown.seq);
        let focus = self.markdown.focus.clone();
        let mut context = gpui::KeyContext::default();
        context.set(md::ACTIVE_CUSTOM_EDITOR_ID, md::EDITOR_ID);
        let mut root = div()
            .id(SharedString::from(format!("markdown-preview-{}", group.0)))
            .absolute()
            .inset_0()
            .key_context(context);
        if let Some(focus) = focus.filter(|_| owner) {
            root = root.track_focus(&focus).on_key_down(cx.listener(
                move |view, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "tab" {
                        view.markdown
                            .backend
                            .borrow_mut()
                            .focus(event.keystroke.modifiers.shift);
                        cx.stop_propagation();
                    }
                },
            ));
        }
        if !owner {
            return root
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(self.ui_color("descriptionForeground"))
                .child(self.label("markdown.otherGroup", &[]))
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.editor.focus(group);
                    view.focus_group = Some(group);
                    if let Some(focus) = view.markdown.focus.clone() {
                        window.focus(&focus);
                    }
                    cx.notify();
                }))
                .into_any_element();
        }
        let backend = self.markdown.backend.clone();
        let visible = self.markdown.visible.clone();
        let bar = self.ui_color("editorWidget.background");
        let skeleton = (!rendered || !visible.get()).then(|| {
            let line = |w: f32, h: f32| div().h(px(h)).w(relative(w)).rounded_sm().bg(bar);
            div()
                .absolute()
                .inset_0()
                .p(px(26.0))
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(line(0.45, 28.0))
                .child(line(0.9, 14.0))
                .child(line(0.82, 14.0))
                .child(line(0.86, 14.0))
                .child(line(0.6, 14.0))
                .child(line(0.88, 14.0))
                .child(line(0.75, 14.0))
        });
        root.on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |view, _, window, cx| {
                if let Some(focus) = view.markdown.focus.clone() {
                    window.focus(&focus);
                }
                cx.notify();
            }),
        )
        .children(skeleton)
        .child(
            canvas(
                move |bounds: Bounds<Pixels>, window, _| {
                    if visible.get() {
                        backend
                            .borrow_mut()
                            .layout(Some(bounds), window.scale_factor());
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .into_any_element()
    }

    /// 包住代码视图：Markdown 源码发布 `editorLangId` / `resourceLangId`（`ctrl+k v`、`ctrl+shift+v` 的 when）。
    pub(super) fn markdown_code_context(
        &self,
        group: EditorGroupId,
        secondary: bool,
        viewport: gpui::Div,
    ) -> gpui::Div {
        let markdown = self
            .shown_document(group, secondary)
            .is_some_and(|d| &*d.metadata.language == "markdown");
        if !markdown {
            return viewport;
        }
        let mut context = gpui::KeyContext::default();
        context.set("editorLangId", "markdown");
        context.set("resourceLangId", "markdown");
        // 另包一层：代码视图外框自己的 key context 归速览（#41）。
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .key_context(context)
            .child(viewport)
    }
}

fn to_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => json!(n),
        Value::String(s) => Json::String(s.to_string()),
        Value::Array(items) => Json::Array(items.iter().map(to_json).collect()),
        Value::Object(map) => {
            Json::Object(map.iter().map(|(k, v)| (k.clone(), to_json(v))).collect())
        }
    }
}

/// 标题栏按钮的 `when` 求值上下文。
struct TitleContext<'a>(BTreeMap<&'a str, Value>);
impl rove_commands::ContextKeys for TitleContext<'_> {
    fn get(&self, key: &str) -> Option<Value> {
        self.0.get(key).cloned()
    }
    fn set(&self, _: Arc<str>, _: Option<Value>) -> CoreResult<()> {
        Err(Error::Unsupported)
    }
    fn changes(&self) -> &dyn rove_base::Events<rove_commands::ContextChange> {
        &rove_base::NoEvents
    }
}

struct TitleTooltip(String);
impl Render for TitleTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().px_2().py_1().child(self.0.clone())
    }
}
