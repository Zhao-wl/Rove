//! #41 速览窗口的窗口装配：每个代码视图最多挂一个速览窗口（窗口栈的根），挂在代码视图的
//! 嵌入区域里；命令、上下文键、打开结果与光标处提示都在这里。
//!
//! - 打开：`editor.action.goToReferences` / `peekDefinition` / `peekImplementation` /
//!   `referenceSearch.trigger` 从主编辑器发出 → 先问能力开关（[`Availability`]）→ 窗口立即出现，
//!   查询在后台；结果为空时关闭窗口并按 UX_SPEC §6.3 在光标处提示。
//! - 打开结果：`Enter` / 双击 / `Ctrl+Enter` 经 #17 的打开文档服务（来源 `CodeNavigation`，带选区，
//!   所以 `.md` 也打开源码并定位）。
//! - 路由：每次打开速览时取 #40 装配的 `CsRouter`（[`Workbench::cs_router`]：带 L1、同步打开中的
//!   缓冲区，所以未保存修改的位置经映射）。#40 换掉路由（索引装上或换掉、L1 建好）后，下一次速览
//!   自然用新的；窗口打开期间的嵌套速览沿用打开时的那个。
use super::*;
use crate::peek::{
    registry::{self as peek_reg, context as peek_ctx, PeekAction},
    zone::zone_overlay,
    Availability, ExternalSnippets, Marker, PeekEvent, PeekKind, PeekQueries, PeekRequest,
    PeekServices, PeekWidget,
};
use gpui::Div;
use rove_commands::ContextKeys;
use rove_lang_cs::query::QueryPoint;
use std::rc::Rc;

/// 一个代码视图上打开着的速览窗口。
struct OpenPeek {
    widget: Entity<PeekWidget>,
    /// 嵌入区域所在的文档行（打开时的插入符行）。
    anchor: u64,
    /// 打开时显示的文档；换了文档就关闭。
    document: DocumentId,
    _events: gpui::Subscription,
}

/// 窗口内的速览状态。
#[derive(Default)]
pub(super) struct Peeks {
    open: BTreeMap<(EditorGroupId, bool), OpenPeek>,
    /// 显式替换的查询服务（测试替身）；`None` 时用 #40 的 `CsRouter`。
    queries: Option<Arc<dyn PeekQueries>>,
    /// 外部符号片段的登记点（#42 注册真正的提供方）。
    external: Arc<ExternalSnippets>,
}

fn kind_of(id: &str) -> Option<PeekKind> {
    match id {
        peek_reg::GO_TO_REFERENCES | peek_reg::REFERENCE_SEARCH_TRIGGER => {
            Some(PeekKind::References)
        }
        peek_reg::PEEK_DEFINITION => Some(PeekKind::Definition),
        peek_reg::PEEK_IMPLEMENTATION => Some(PeekKind::Implementation),
        _ => None,
    }
}

impl Workbench {
    /// 用别的查询服务代替 #40 的 `CsRouter`（测试替身）；`None` 恢复用路由。
    pub fn set_peek_queries(&mut self, queries: Option<Arc<dyn PeekQueries>>) {
        self.peek.queries = queries;
    }

    /// 本次速览用的查询服务：替身，否则 #40 当前的路由（每次现取，路由被换掉后自然是新的）。
    fn peek_queries(&self) -> Arc<dyn PeekQueries> {
        self.peek
            .queries
            .clone()
            .unwrap_or_else(|| self.cs_router().clone() as Arc<dyn PeekQueries>)
    }

    /// 外部符号片段的登记点：#42 在这里注册元数据视图的片段提供方。
    pub fn peek_external_snippets(&self) -> Arc<ExternalSnippets> {
        self.peek.external.clone()
    }

    /// 某个代码视图上打开着的速览窗口（窗口栈的根）。
    pub fn peek_widget(&self, group: EditorGroupId, secondary: bool) -> Option<Entity<PeekWidget>> {
        self.peek
            .open
            .get(&(group, secondary))
            .map(|p| p.widget.clone())
    }

    fn peek_services(&self) -> Rc<PeekServices> {
        Rc::new(PeekServices {
            queries: Some(self.peek_queries()),
            documents: self.editor.loader(),
            settings: self.settings.clone(),
            theme: self.theme.clone(),
            theme_kind: self.theme_service.current().kind(),
            strings: self.find_strings.clone(),
            external: self.peek.external.clone(),
            syntax: Arc::new(TreeSitterSyntax::default().with_settings(self.settings.clone())),
        })
    }

    fn slot_code(&self, group: EditorGroupId, secondary: bool) -> Option<Entity<CodeView>> {
        self.group_code(group, secondary).cloned()
    }

    /// 给一个代码视图的外框加上速览：上下文键、命令与嵌入区域里的窗口。
    pub(super) fn peek_viewport(
        &self,
        group: EditorGroupId,
        secondary: bool,
        viewport: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut context = gpui::KeyContext::default();
        let csharp = self
            .shown_document(group, secondary)
            .is_some_and(|d| &*d.metadata.language == "csharp");
        if csharp {
            context.add(peek_ctx::HAS_REFERENCES);
            context.add(peek_ctx::HAS_DEFINITION);
            context.add(peek_ctx::HAS_IMPLEMENTATION);
        }
        let open = self.peek.open.get(&(group, secondary));
        if open.is_some() {
            context.add(peek_ctx::VISIBLE);
        }
        let overlay = open
            .zip(self.slot_code(group, secondary))
            .map(|(p, code)| zone_overlay(code, p.anchor, p.widget.clone().into_any_element()));
        viewport
            .key_context(context)
            .on_action(cx.listener(move |v, a: &PeekAction, window, cx| {
                v.peek_command(group, secondary, a.0, window, cx)
            }))
            .children(overlay)
    }

    fn peek_command(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(kind) = kind_of(id) {
            self.open_peek_at(group, secondary, kind, window, cx);
            return;
        }
        // 焦点在主编辑器里：F4 / Esc / Ctrl+K F2 等作用于这个编辑器上的速览。
        let Some(widget) = self.peek_widget(group, secondary) else {
            cx.propagate();
            return;
        };
        if !widget.update(cx, |w, cx| w.execute(id, window, cx)) {
            cx.propagate();
        }
    }

    /// 在代码视图的插入符处打开速览。
    fn open_peek_at(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        kind: PeekKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(document), Some(code)) = (
            self.shown_document(group, secondary),
            self.slot_code(group, secondary),
        ) else {
            return;
        };
        let (caret, word) = code.read_with(cx, |c, _| (c.selection().active, c.word_at_caret()));
        let symbol: Arc<str> = word
            .unwrap_or_else(|| self.label("peek.symbolHere", &[]))
            .into();
        if self.peek_queries().availability(kind) == Availability::Unavailable {
            let message = Message::key("peek.unavailable.implementations");
            self.peek_hint(document.id, caret, message, cx);
            return;
        }
        self.show_peek(
            group,
            secondary,
            &document,
            PeekRequest {
                kind,
                point: QueryPoint {
                    resource: document.resource.clone(),
                    position: caret,
                    version: document.text.version(),
                },
                symbol,
                marker: Marker::Auto,
                preset: None,
            },
            window,
            cx,
        );
    }

    /// #40 的入口：多候选定义等已经有结果的情况，在活动编辑器的插入符处直接打开速览
    /// （`request.preset` 带结果，`request.marker` 带“近似 · N 个候选”之类的文案）。
    pub fn open_peek_with(
        &mut self,
        request: PeekRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let group = self.editor.focused;
        let Some(document) = self.shown_document(group, false) else {
            return;
        };
        self.show_peek(group, false, &document, request, window, cx);
    }

    fn show_peek(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        document: &Document,
        request: PeekRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(code) = self.slot_code(group, secondary) else {
            return;
        };
        self.close_peek(group, secondary, cx);
        let services = self.peek_services();
        let line_height = code.read(cx).line_height();
        let anchor = request.point.position.line.0;
        let widget = cx.new(|cx| PeekWidget::new(services, request, line_height, window, cx));
        let events = cx.subscribe_in(&widget, window, move |v, _, e: &PeekEvent, window, cx| {
            v.peek_event(group, secondary, e.clone(), window, cx)
        });
        let height = widget.read(cx).height();
        code.update(cx, |c, cx| {
            c.set_view_zone(anchor, height, cx);
            crate::peek::reveal_zone(c, anchor, cx);
        });
        widget.update(cx, |w, cx| w.take_focus(window, cx));
        self.peek.open.insert(
            (group, secondary),
            OpenPeek {
                widget,
                anchor,
                document: document.id,
                _events: events,
            },
        );
        cx.notify();
    }

    /// 关闭并释放（结果数据、预览文档、后台工作随窗口实体一起释放）。
    pub(super) fn close_peek(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.peek.open.remove(&(group, secondary)) {
            if let Some(code) = self.slot_code(group, secondary) {
                code.update(cx, |c, cx| c.set_view_zone(open.anchor, 0.0, cx));
            }
            cx.notify();
        }
    }

    fn peek_event(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        event: PeekEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus_code = |v: &Self, window: &mut Window, cx: &mut Context<Self>| {
            if let Some(code) = v.slot_code(group, secondary) {
                window.focus(&code.focus_handle(cx));
            }
        };
        match event {
            PeekEvent::Open {
                resource,
                selection,
                side,
            } => {
                if !side {
                    self.close_peek(group, secondary, cx);
                    focus_code(self, window, cx);
                }
                let options = OpenOptions {
                    target: if side {
                        crate::documents::OpenTarget::Beside
                    } else {
                        crate::documents::OpenTarget::Group(group)
                    },
                    source: OpenSource::CodeNavigation,
                    preserve_focus: side,
                    selection: Some(selection),
                    ..OpenOptions::default()
                };
                if let Err(e) =
                    self.open_documents()
                        .open(resource, options, Cancellation::default())
                {
                    eprintln!("peek: {e}");
                }
                cx.notify();
            }
            PeekEvent::Close => {
                self.close_peek(group, secondary, cx);
                focus_code(self, window, cx);
            }
            PeekEvent::Empty(message) => {
                self.close_peek(group, secondary, cx);
                focus_code(self, window, cx);
                self.peek_hint_at_caret(group, secondary, message, cx);
            }
            PeekEvent::Hint(message) => self.peek_hint_at_caret(group, secondary, message, cx),
            PeekEvent::Resized(height) => {
                if let (Some(open), Some(code)) = (
                    self.peek.open.get(&(group, secondary)),
                    self.slot_code(group, secondary),
                ) {
                    let anchor = open.anchor;
                    code.update(cx, |c, cx| c.set_view_zone(anchor, height, cx));
                }
            }
        }
    }

    fn peek_hint_at_caret(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        message: Message,
        cx: &mut Context<Self>,
    ) {
        let (Some(document), Some(code)) = (
            self.shown_document(group, secondary),
            self.slot_code(group, secondary),
        ) else {
            return;
        };
        let caret = code.read(cx).selection().active;
        self.peek_hint(document.id, caret, message, cx);
    }

    /// UX_SPEC §6.1 第 2 级：光标处提示。
    fn peek_hint(
        &mut self,
        document: DocumentId,
        position: rove_base::Position,
        message: Message,
        cx: &mut Context<Self>,
    ) {
        let _ = shell::view::report(
            self,
            "peek.noResult",
            Reason::NoResult { document, position },
            message,
            Arc::from([]),
            cx,
        );
    }

    /// 每帧：代码视图换了文档（切换标签、关闭）时关闭它上面的速览。
    pub(super) fn sync_peeks(&mut self, cx: &mut Context<Self>) {
        let stale: Vec<_> = self
            .peek
            .open
            .iter()
            .filter(|((group, secondary), open)| {
                self.views
                    .get(group)
                    .and_then(|v| {
                        if *secondary {
                            v.secondary.as_ref()
                        } else {
                            Some(&v.primary)
                        }
                    })
                    .and_then(|slot| slot.shown)
                    .is_none_or(|shown| shown.document != open.document)
            })
            .map(|(key, _)| *key)
            .collect();
        for (group, secondary) in stale {
            self.close_peek(group, secondary, cx);
        }
    }

    /// `editor.stablePeek` 发布为全局上下文键（四条 `closeReferenceSearch` 键位的 when 共用）；
    /// 设置变了时各窗口重读。
    pub(super) fn peek_settings_changed(&mut self, cx: &mut Context<Self>) {
        let stable = crate::peek::registry::PeekSettings::read(&*self.settings).stable;
        if let Some(keys) = cx.try_global::<crate::keys::Keys>() {
            let _ = keys
                .context
                .set(peek_ctx::STABLE_PEEK.into(), Some(Value::Bool(stable)));
        }
        let widgets: Vec<_> = self.peek.open.values().map(|p| p.widget.clone()).collect();
        for w in widgets {
            w.update(cx, |w, cx| w.reload_settings(cx));
        }
    }

    pub(super) fn peek_theme_changed(&mut self, cx: &mut Context<Self>) {
        let kind = self.theme_service.current().kind();
        let widgets: Vec<_> = self.peek.open.values().map(|p| p.widget.clone()).collect();
        for w in widgets {
            let theme = self.theme.clone();
            w.update(cx, |w, cx| w.set_theme(theme, kind, cx));
        }
    }

    /// 默认高度（行），供测试断言。
    #[cfg(test)]
    pub(super) fn peek_default_lines() -> f32 {
        crate::peek::widget::DEFAULT_HEIGHT_LINES
    }
}
