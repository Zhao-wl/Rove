//! 大纲（#26）在骨架窗口中的装配：活动文档 → 后台符号分析 → 大纲小节、面包屑、转到符号。
//! 定位一律经 #17 的 `OpenDocuments`（`OpenSource::Outline` / `Breadcrumb`）。
use super::*;
use crate::{
    documents::OpenTarget,
    list_widget::{
        matcher::{matcher_for, MatchType},
        options::ListOptions,
        quick_input::{QuickInputService, QuickPickConfig},
        quick_input_view::{QuickInputView, QuickInputViewEvent},
        view::ListView,
        widget::ListEvent,
        StaticList,
    },
    lists::{QuickInput, QuickInputEvent, QuickInputOptions, Row},
    outline::{
        goto,
        model::{self, Segment},
        registry::{self as reg, OUTLINE_VIEW},
        view::{self as ov, CursorPath, Goto, Outline, Picker, PickerItem, Status},
    },
};
use gpui::{deferred, MouseButton};
use rove_base::Position;
use rove_buffer::Document;
use rove_commands::ContextKeys;
use rove_syntax::symbols::{availability, Availability, DocumentSymbols, SymbolId, SymbolUpdate};
use std::collections::HashSet;

const BAR_HEIGHT: f32 = 22.0;
const PICKER_WIDTH: f32 = 300.0;
const PICKER_ROWS: usize = 12;

impl Workbench {
    pub(super) fn new_outline(
        theme: Arc<dyn Theme>,
        settings: &Arc<dyn Settings>,
        cx: &mut Context<Self>,
    ) -> Outline {
        let strings: Arc<dyn Strings> = Arc::new(appearance::strings().unwrap());
        let outline = Outline::new(
            theme,
            strings,
            ListOptions::read(&**settings),
            &**settings,
            cx,
        );
        cx.subscribe(&outline.list, |this, _, event: &ListEvent, cx| {
            this.outline_list_event(event, cx)
        })
        .detach();
        publish_breadcrumbs_enabled(outline.crumbs.enabled, cx);
        outline
    }

    /// 焦点组的活动文本编辑器：(组, 文档, 光标)。
    fn focused_text(&self) -> Option<(EditorGroupId, Arc<Document>, Position)> {
        let group = self.editor.focused;
        let tab = self.editor.groups.get(&group)?.current()?;
        if tab.kind != EditorKind::Text {
            return None;
        }
        let opened = tab.opened()?;
        Some((group, opened.document.clone(), tab.view.cursor))
    }

    /// 每帧调用（只读缓存与轻量状态）：让大纲跟上活动编辑器，必要时在后台请求新版本的符号。
    pub(super) fn sync_outline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.outline.bar_active = self.outline.bar.focus.contains_focused(window, cx)
            || self
                .outline
                .picker
                .as_ref()
                .is_some_and(|p| p.list.focus_handle(cx).contains_focused(window, cx));
        if std::mem::take(&mut self.outline.focus_picker) {
            if let Some(picker) = &self.outline.picker {
                window.focus(&picker.list.focus_handle(cx));
            }
        }
        if self.outline.focus_editor {
            self.outline.focus_editor = false;
            self.focus_group_code(self.editor.focused, window, cx);
        }
        let open: HashSet<DocumentId> = self
            .editor
            .groups
            .values()
            .flat_map(|g| g.tabs.iter())
            .filter_map(|t| t.opened().map(|o| o.document.id))
            .collect();
        self.outline.retain_documents(&open);
        let Some((group, document, cursor)) = self.focused_text() else {
            self.cancel_symbols();
            self.outline.clear(Status::NoEditor, cx);
            return;
        };
        match availability(&document.metadata.language, document.metadata.size) {
            Availability::Available => {}
            other => {
                self.cancel_symbols();
                let status = if other == Availability::TooLarge {
                    Status::TooLarge
                } else {
                    Status::Unsupported
                };
                self.outline.clear(status, cx);
                return;
            }
        }
        let version = document.text.version();
        let cached = self.outline.cached(document.id).cloned();
        match &cached {
            Some(symbols) => self.outline.show(document.id, symbols.clone(), cx),
            None if self
                .outline
                .shown
                .as_ref()
                .is_none_or(|(d, _)| *d != document.id) =>
            {
                self.outline.clear(Status::Loading, cx)
            }
            None => {}
        }
        let current = cached.as_ref().is_some_and(|s| s.version() == version);
        let requested = self
            .outline
            .pending
            .as_ref()
            .is_some_and(|(d, v, _)| *d == document.id && *v == version)
            || self.outline.failed == Some((document.id, version));
        if !current && !requested {
            self.request_symbols(document.clone(), cx);
        }
        // 跟随光标：路径变化时选中最深的可见节点。
        if let Some(symbols) = cached {
            let path = self.cursor_path(group, &document, &symbols, cursor);
            let key = (document.id, symbols.version(), path.clone());
            if self.outline.follow_cursor && self.outline.revealed.as_ref() != Some(&key) {
                self.outline.reveal_path(&path, cx);
                self.outline.revealed = Some(key);
            }
        }
    }

    fn cancel_symbols(&mut self) {
        if let Some((_, _, cancel)) = self.outline.pending.take() {
            cancel.cancel();
        }
    }

    fn request_symbols(&mut self, document: Arc<Document>, cx: &mut Context<Self>) {
        self.cancel_symbols();
        let cancel = Cancellation::default();
        let (id, version) = (document.id, document.text.version());
        self.outline.pending = Some((id, version, cancel.clone()));
        let previous = self.outline.cached(id).cloned();
        let task = self.outline.service.analyze(
            document.text.clone(),
            document.metadata.language.clone(),
            document.metadata.size,
            previous,
            cancel,
        );
        let task = cx.background_executor().spawn(task);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| view.symbols_ready(id, version, result, cx));
        })
        .detach();
    }

    fn symbols_ready(
        &mut self,
        document: DocumentId,
        version: Version,
        result: CoreResult<SymbolUpdate>,
        cx: &mut Context<Self>,
    ) {
        if self
            .outline
            .pending
            .as_ref()
            .is_some_and(|(d, v, _)| *d == document && *v == version)
        {
            self.outline.pending = None;
        }
        match result {
            Ok(update) => {
                let newer = self
                    .outline
                    .cached(document)
                    .is_none_or(|c| c.version() <= update.symbols.version());
                if newer {
                    self.outline.store(document, update.symbols);
                }
            }
            Err(Error::Cancelled | Error::Stale) => {}
            Err(e) => {
                eprintln!("outline: {e}");
                self.outline.failed = Some((document, version));
            }
        }
        cx.notify();
    }

    /// 某组光标所在的符号路径：复用上一条路径（`path_from`），光标不动时直接返回缓存。
    fn cursor_path(
        &mut self,
        group: EditorGroupId,
        document: &Document,
        symbols: &DocumentSymbols,
        cursor: Position,
    ) -> Vec<SymbolId> {
        let previous = self
            .outline
            .paths
            .get(&group)
            .filter(|p| p.document == document.id && p.version == symbols.version());
        if let Some(p) = previous.filter(|p| p.position == cursor) {
            return p.path.clone();
        }
        let hint = previous.map(|p| p.path.as_slice()).unwrap_or_default();
        let path = symbols.path_from(hint, cursor);
        self.outline.paths.insert(
            group,
            CursorPath {
                document: document.id,
                version: symbols.version(),
                position: cursor,
                path: path.clone(),
            },
        );
        path
    }

    /// 某组的面包屑数据：(文档, 符号, 段)。只读：以该组上一次的路径为起点查找（`path_from`）。
    pub(super) fn group_segments(
        &self,
        group: EditorGroupId,
    ) -> Option<(Arc<Document>, Option<Arc<DocumentSymbols>>, Vec<Segment>)> {
        let tab = self.editor.groups.get(&group)?.current()?;
        if tab.kind != EditorKind::Text {
            return None;
        }
        let document = tab.opened()?.document.clone();
        let cursor = tab.view.cursor;
        let symbols = (availability(&document.metadata.language, document.metadata.size)
            == Availability::Available)
            .then(|| self.outline.cached(document.id).cloned())
            .flatten();
        let path = match &symbols {
            Some(s) => {
                let previous = self
                    .outline
                    .paths
                    .get(&group)
                    .filter(|p| p.document == document.id && p.version == s.version());
                match previous {
                    Some(p) if p.position == cursor => p.path.clone(),
                    Some(p) => s.path_from(&p.path, cursor),
                    None => s.path_at(cursor),
                }
            }
            None => Vec::new(),
        };
        let segments = self
            .outline
            .segments(&document.resource, symbols.as_deref(), &path);
        Some((document, symbols, segments))
    }

    /// 大纲列表事件：单击/回车定位，展开状态按文档保留。
    fn outline_list_event(&mut self, event: &ListEvent, cx: &mut Context<Self>) {
        let Some((document, _)) = self.outline.shown.clone() else {
            return;
        };
        match event {
            ListEvent::Expanded { id, expanded } => {
                self.outline
                    .note_expanded(document, id.0.clone(), *expanded)
            }
            ListEvent::SelectionChanged(ids) if ids.len() == 1 => {
                let key = ids[0].0.clone();
                self.outline.last_nav = Some(key.clone());
                self.reveal_key(
                    document,
                    &key,
                    true,
                    OpenSource::Outline,
                    OpenTarget::Active,
                    cx,
                );
            }
            ListEvent::Open {
                ids,
                preserve_focus,
                side_by_side,
                ..
            } => {
                let Some(key) = ids.first().map(|i| i.0.clone()) else {
                    return;
                };
                if *preserve_focus && self.outline.last_nav.as_ref() == Some(&key) {
                    return;
                }
                let target = if *side_by_side {
                    OpenTarget::Beside
                } else {
                    OpenTarget::Active
                };
                self.reveal_key(
                    document,
                    &key,
                    *preserve_focus,
                    OpenSource::Outline,
                    target,
                    cx,
                );
            }
            _ => {}
        }
    }

    fn reveal_key(
        &mut self,
        document: DocumentId,
        key: &str,
        preserve_focus: bool,
        source: OpenSource,
        target: OpenTarget,
        cx: &mut Context<Self>,
    ) {
        let Some(symbols) = self.outline.cached(document).cloned() else {
            return;
        };
        if let Some(id) = symbols.find_key(key) {
            self.reveal_symbol(document, &symbols, id, preserve_focus, source, target, cx);
        }
    }

    /// 经打开文档服务定位到符号：选区收拢到符号名称的起点（与 VS Code 大纲相同）。
    #[allow(clippy::too_many_arguments)]
    fn reveal_symbol(
        &mut self,
        document: DocumentId,
        symbols: &DocumentSymbols,
        id: SymbolId,
        preserve_focus: bool,
        source: OpenSource,
        target: OpenTarget,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = symbols.node(id) else { return };
        let Some(resource) = self
            .editor
            .groups
            .values()
            .flat_map(|g| g.tabs.iter())
            .find(|t| t.opened().is_some_and(|o| o.document.id == document))
            .map(|t| t.resource.clone())
        else {
            return;
        };
        let at = node.symbol.selection.start();
        let options = OpenOptions {
            source,
            target,
            preserve_focus: true,
            selection: TextRange::new(at, at).ok(),
            ..Default::default()
        };
        if let Err(e) = self
            .open_documents()
            .open(resource, options, Cancellation::default())
        {
            eprintln!("outline: {e}");
        }
        if !preserve_focus {
            self.outline.focus_editor = true;
        }
        cx.notify();
    }

    // ------------------------------------------------------------------ 大纲小节

    pub(super) fn render_outline(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let fg = self.ui_color("sideBar.foreground");
        let icon_fg = self.ui_color("icon.foreground");
        let active_bg = self.ui_color("inputOption.activeBackground");
        let size = px(16.0 * self.shell.zoom());
        let mut toolbar = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(4.0))
            .px(px(8.0))
            .flex_shrink_0();
        for (command, icon) in ov::toolbar_commands() {
            let on = command == reg::OUTLINE_FOLLOW_CURSOR && self.outline.follow_cursor;
            toolbar = toolbar.child(
                div()
                    .id(SharedString::from(format!("outline-{command}")))
                    .cursor_pointer()
                    .rounded_sm()
                    .when(on, |d| d.bg(active_bg))
                    .child(ov::codicon(icon, size, Some(icon_fg)))
                    .on_click(
                        cx.listener(move |v, _, window, cx| v.outline_command(command, window, cx)),
                    ),
            );
        }
        let message = |key: &str| {
            div()
                .px_4()
                .py_1()
                .text_color(fg)
                .child(self.label(key, &[]))
                .into_any_element()
        };
        let body = match self.outline.status {
            Status::NoEditor | Status::Unsupported => message("outline.empty"),
            Status::TooLarge => message("outline.tooLarge"),
            Status::Loading => message("outline.loading"),
            Status::Ready => {
                let empty = self
                    .outline
                    .shown
                    .as_ref()
                    .is_none_or(|(_, s)| s.is_empty());
                if empty {
                    message("outline.noSymbols")
                } else {
                    let truncated = self
                        .outline
                        .shown
                        .as_ref()
                        .is_some_and(|(_, s)| s.truncated());
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .child(div().flex_1().min_h_0().child(self.outline.list.clone()))
                        .when(truncated, |d| {
                            d.child(div().px_4().text_color(fg).child(self.label(
                                "outline.truncated",
                                &[("count", rove_syntax::symbols::DATA_SYMBOL_LIMIT.to_string())],
                            )))
                        })
                        .into_any_element()
                }
            }
        };
        div()
            .id("outline")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(toolbar)
            .child(body)
            .into_any_element()
    }

    // ------------------------------------------------------------------ 面包屑

    /// 根节点 key context 追加的面包屑上下文键。
    pub(super) fn breadcrumbs_context(&self) -> &'static str {
        let possible = self.focused_text().is_some();
        match (possible, self.outline.crumbs.enabled) {
            (true, true) => " breadcrumbsPossible breadcrumbsVisible",
            (true, false) => " breadcrumbsPossible",
            _ => "",
        }
    }

    /// 编辑器组标签栏下方的面包屑（`breadcrumbs.enabled`）。
    pub(super) fn render_breadcrumbs(
        &self,
        group: EditorGroupId,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.outline.crumbs.enabled {
            return None;
        }
        let (_, symbols, segments) = self.group_segments(group)?;
        let zoom = self.shell.zoom();
        let height = px(BAR_HEIGHT * zoom);
        let icon_size = px(14.0 * zoom);
        let active = self.outline.bar.group == Some(group);
        let focused = (active && self.outline.bar_active).then_some(self.outline.bar.focused);
        let picker_segment = self
            .outline
            .picker
            .as_ref()
            .filter(|p| p.group == group)
            .map(|p| p.segment);
        let fg = self.ui_color("breadcrumb.foreground");
        let focus_fg = self.ui_color("breadcrumb.focusForeground");
        let selection_fg = self.ui_color("breadcrumb.activeSelectionForeground");
        let icon_fg = self.ui_color("icon.foreground");
        let mut bar = div()
            .id(element_id("breadcrumbs", group, 0))
            .h(height)
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .px(px(8.0 * zoom))
            .gap(px(2.0 * zoom))
            .text_size(px(13.0 * zoom))
            .bg(self.ui_color("breadcrumb.background"))
            .text_color(fg)
            .overflow_hidden();
        if active {
            bar = bar
                .track_focus(&self.outline.bar.focus)
                .key_context("breadcrumbs breadcrumbsActive breadcrumbsVisible");
        }
        let count = segments.len();
        for (index, segment) in segments.into_iter().enumerate() {
            let (label, icon): (String, Option<gpui::Div>) = match &segment {
                Segment::File { label, is_file, .. } => (
                    label.to_string(),
                    self.outline.crumbs.icons.then(|| {
                        ov::codicon(
                            if *is_file { "file" } else { "folder" },
                            icon_size,
                            Some(icon_fg),
                        )
                    }),
                ),
                Segment::Symbol(id) => {
                    let Some(node) = symbols.as_ref().and_then(|s| s.node(*id)) else {
                        continue;
                    };
                    (
                        (self.outline.label)(&node.symbol).to_string(),
                        self.outline
                            .crumbs
                            .icons
                            .then(|| ov::kind_icon(self.theme.as_ref(), &node.symbol, icon_size)),
                    )
                }
            };
            let color = if picker_segment == Some(index) {
                selection_fg
            } else if focused == Some(index) {
                focus_fg
            } else {
                fg
            };
            let mut item = div()
                .id(element_id("crumb", group, index as u64))
                .relative()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(3.0 * zoom))
                .flex_shrink_0()
                .cursor_pointer()
                .text_color(color)
                .children(icon)
                .child(label)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |v, _, window, cx| {
                    v.open_picker(group, index, window, cx);
                    cx.notify();
                }));
            if picker_segment == Some(index) {
                if let Some(picker) = &self.outline.picker {
                    let rows = picker.items.len().clamp(1, PICKER_ROWS) as f32;
                    item = item.child(deferred(
                        div()
                            .id("breadcrumb-picker")
                            .absolute()
                            .top(height)
                            .left_0()
                            .w(px(PICKER_WIDTH * zoom))
                            .h(px(rows * 22.0 * zoom + 4.0))
                            .p(px(2.0))
                            .bg(self.ui_color("breadcrumbPicker.background"))
                            .border_1()
                            .border_color(self.ui_color("widget.border"))
                            .occlude()
                            .child(picker.list.clone()),
                    ));
                }
            }
            bar = bar.child(item);
            if index + 1 < count {
                bar = bar.child(ov::codicon("chevron-right", icon_size, Some(fg)));
            }
        }
        Some(bar.into_any_element())
    }

    /// 选择器的条目：符号段列出同级符号（`breadcrumbs.symbolSortOrder`），文件段列出所在目录。
    fn picker_items(
        &self,
        segment: &Segment,
        symbols: Option<&DocumentSymbols>,
    ) -> Vec<PickerItem> {
        match segment {
            Segment::Symbol(id) => symbols
                .map(|s| {
                    model::sibling_symbols(
                        s,
                        &self.outline.crumbs.filter,
                        *id,
                        self.outline.crumbs.sort,
                    )
                    .into_iter()
                    .map(PickerItem::Symbol)
                    .collect()
                })
                .unwrap_or_default(),
            Segment::File { dir, .. } => self.directory_items(dir),
        }
    }

    fn directory_items(&self, dir: &str) -> Vec<PickerItem> {
        let Some(workspace) = &self.workspace else {
            return Vec::new();
        };
        let path = (!dir.is_empty())
            .then(|| WorkspacePath::new(dir).ok())
            .flatten();
        if !dir.is_empty() && path.is_none() {
            return Vec::new();
        }
        match workspace.children(path.as_ref(), 0..usize::MAX) {
            Ok(page) => page
                .items
                .iter()
                .map(|e| {
                    PickerItem::Path(
                        e.path.as_str().into(),
                        !matches!(e.kind, EntryKind::Directory),
                    )
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn picker_rows(&self, items: &[PickerItem], symbols: Option<&DocumentSymbols>) -> Vec<Row> {
        items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let label: String = match item {
                    PickerItem::Symbol(id) => symbols
                        .and_then(|s| s.node(*id))
                        .map(|n| (self.outline.label)(&n.symbol).to_string())
                        .unwrap_or_default(),
                    PickerItem::Path(path, _) => path.rsplit('/').next().unwrap_or_default().into(),
                };
                crate::list_widget::plain_row(&i.to_string(), &label)
            })
            .collect()
    }

    /// 打开某段的选择器并把焦点交给它（`breadcrumbs.selectFocused`、点击段）。
    pub(super) fn open_picker(
        &mut self,
        group: EditorGroupId,
        segment: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, symbols, segments)) = self.group_segments(group) else {
            return;
        };
        let Some(seg) = segments.get(segment) else {
            return;
        };
        let items = self.picker_items(seg, symbols.as_deref());
        let current = match seg {
            Segment::Symbol(id) => items
                .iter()
                .position(|i| matches!(i, PickerItem::Symbol(s) if s == id)),
            Segment::File { dir, label, .. } => {
                let full = if dir.is_empty() {
                    label.to_string()
                } else {
                    format!("{dir}/{label}")
                };
                items
                    .iter()
                    .position(|i| matches!(i, PickerItem::Path(p, _) if p.as_ref() == full))
            }
        };
        let rows = self.picker_rows(&items, symbols.as_deref());
        let list = self.picker_list(rows, current, cx);
        window.focus(&list.focus_handle(cx));
        self.outline.bar.group = Some(group);
        self.outline.bar.focused = segment;
        self.outline.picker = Some(Picker {
            group,
            segment,
            list,
            items,
        });
    }

    fn picker_list(
        &mut self,
        rows: Vec<Row>,
        current: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Entity<ListView> {
        let theme = self.theme.clone();
        let strings: Arc<dyn Strings> = Arc::new(appearance::strings().unwrap());
        let options = ListOptions::read(&*self.settings);
        let list = cx.new(|cx| {
            let mut widget = ListWidget::new(
                Arc::new(StaticList::new(rows, Version(0))),
                RowHeight::Fixed(22.0),
                options,
            );
            match current {
                Some(i) => {
                    widget.set_focus(Some(i));
                    widget.reveal(i);
                }
                None => widget.focus_first(),
            }
            widget.take_events();
            ListView::new(widget, theme, strings, cx)
        });
        cx.subscribe(&list, |this, _, event: &ListEvent, cx| {
            if let ListEvent::Open {
                ids, side_by_side, ..
            } = event
            {
                if let Some(index) = ids.first().and_then(|i| i.0.parse::<usize>().ok()) {
                    this.picker_accept(index, *side_by_side, cx);
                }
            }
        })
        .detach();
        list
    }

    /// 接受选择器中的一项：符号定位，文件打开，文件夹进入下一层。
    fn picker_accept(&mut self, index: usize, side: bool, cx: &mut Context<Self>) {
        let Some(picker) = &self.outline.picker else {
            return;
        };
        let group = picker.group;
        let Some(item) = picker.items.get(index).cloned() else {
            return;
        };
        let target = if side {
            OpenTarget::Beside
        } else {
            OpenTarget::Group(group)
        };
        match item {
            PickerItem::Symbol(id) => {
                let Some(document) = self
                    .editor
                    .groups
                    .get(&group)
                    .and_then(|g| g.current())
                    .and_then(Tab::opened)
                    .map(|o| o.document.id)
                else {
                    return;
                };
                if let Some(symbols) = self.outline.cached(document).cloned() {
                    self.outline.picker = None;
                    self.reveal_symbol(
                        document,
                        &symbols,
                        id,
                        false,
                        OpenSource::Breadcrumb,
                        target,
                        cx,
                    );
                }
            }
            PickerItem::Path(path, true) => {
                self.outline.picker = None;
                if let Ok(path) = WorkspacePath::new(&path) {
                    let options = OpenOptions {
                        source: OpenSource::Breadcrumb,
                        target,
                        ..Default::default()
                    };
                    if let Err(e) = self.open_documents().open(
                        Resource::Workspace(path),
                        options,
                        Cancellation::default(),
                    ) {
                        eprintln!("breadcrumbs: {e}");
                    }
                }
                self.outline.focus_editor = true;
            }
            PickerItem::Path(dir, false) => {
                let items = self.directory_items(&dir);
                let rows = self.picker_rows(&items, None);
                let list = self.picker_list(rows, None, cx);
                if let Some(picker) = &mut self.outline.picker {
                    picker.items = items;
                    picker.list = list;
                }
                self.outline.focus_picker = true;
            }
        }
        cx.notify();
    }

    /// 选择器中聚焦的一项（`revealFocused*` 在选择器里时使用）。
    fn picker_focused(&self, cx: &App) -> Option<usize> {
        self.outline
            .picker
            .as_ref()
            .and_then(|p| p.list.read(cx).widget.focus())
    }

    // ------------------------------------------------------------------ 命令

    pub(super) fn outline_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused_group = self.editor.focused;
        let segment_count = |v: &mut Self| {
            v.group_segments(v.outline.bar.group.unwrap_or(focused_group))
                .map_or(0, |(_, _, s)| s.len())
        };
        let in_picker = self
            .outline
            .picker
            .as_ref()
            .is_some_and(|p| p.list.focus_handle(cx).contains_focused(window, cx));
        match id {
            reg::GOTO_SYMBOL => self.show_goto(window, cx),
            reg::FOCUS | reg::FOCUS_AND_SELECT => {
                if !self.outline.crumbs.enabled {
                    return;
                }
                self.outline.bar.group = Some(focused_group);
                let count = segment_count(self);
                self.outline.bar.focused = count.saturating_sub(1);
                self.outline.picker = None;
                window.focus(&self.outline.bar.focus);
                if id == reg::FOCUS_AND_SELECT && count > 0 {
                    self.open_picker(focused_group, count - 1, window, cx);
                }
            }
            reg::FOCUS_NEXT
            | reg::FOCUS_PREVIOUS
            | reg::FOCUS_NEXT_WITH_PICKER
            | reg::FOCUS_PREVIOUS_WITH_PICKER => {
                let count = segment_count(self);
                if count == 0 {
                    return;
                }
                let at = self.outline.bar.focused.min(count - 1);
                let next = if matches!(id, reg::FOCUS_NEXT | reg::FOCUS_NEXT_WITH_PICKER) {
                    (at + 1).min(count - 1)
                } else {
                    at.saturating_sub(1)
                };
                self.outline.bar.focused = next;
                let group = self.outline.bar.group.unwrap_or(focused_group);
                if matches!(
                    id,
                    reg::FOCUS_NEXT_WITH_PICKER | reg::FOCUS_PREVIOUS_WITH_PICKER
                ) {
                    self.open_picker(group, next, window, cx);
                } else {
                    self.outline.picker = None;
                    window.focus(&self.outline.bar.focus);
                }
            }
            reg::SELECT_FOCUSED => {
                let group = self.outline.bar.group.unwrap_or(focused_group);
                self.open_picker(group, self.outline.bar.focused, window, cx);
            }
            reg::REVEAL_FOCUSED | reg::REVEAL_FOCUSED_ASIDE => {
                let side = id == reg::REVEAL_FOCUSED_ASIDE;
                if in_picker {
                    if let Some(index) = self.picker_focused(cx) {
                        self.picker_accept(index, side, cx);
                    }
                    return;
                }
                let group = self.outline.bar.group.unwrap_or(focused_group);
                let Some((document, symbols, segments)) = self.group_segments(group) else {
                    return;
                };
                match (segments.get(self.outline.bar.focused), symbols) {
                    (Some(Segment::Symbol(sid)), Some(symbols)) => {
                        let target = if side {
                            OpenTarget::Beside
                        } else {
                            OpenTarget::Group(group)
                        };
                        self.reveal_symbol(
                            document.id,
                            &symbols,
                            *sid,
                            true,
                            OpenSource::Breadcrumb,
                            target,
                            cx,
                        );
                    }
                    (Some(Segment::File { .. }), _) => {
                        self.open_picker(group, self.outline.bar.focused, window, cx)
                    }
                    _ => {}
                }
            }
            reg::SELECT_EDITOR => {
                self.outline.picker = None;
                self.focus_group_code(self.outline.bar.group.unwrap_or(focused_group), window, cx);
            }
            reg::TOGGLE_TO_ON => self.write_breadcrumbs_enabled(true, cx),
            reg::TOGGLE => {
                let enabled = !self.outline.crumbs.enabled;
                self.write_breadcrumbs_enabled(enabled, cx)
            }
            reg::COPY_PATH => {
                if let Some((group, document, cursor)) = self.focused_text() {
                    if let Some(symbols) = self.outline.cached(document.id).cloned() {
                        let path = self.cursor_path(group, &document, &symbols, cursor);
                        let text = model::symbol_path_text(
                            &symbols,
                            &path,
                            &self.outline.crumbs.separator,
                        );
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    }
                }
            }
            reg::OUTLINE_FOCUS => {
                self.shell.layout.collapsed_sections.remove(OUTLINE_VIEW);
                if !self.shell.layout.sidebar_visible {
                    shell::view::run_command(
                        self,
                        "workbench.action.toggleSidebarVisibility",
                        window,
                        cx,
                    );
                }
                window.focus(&self.outline.list.focus_handle(cx));
            }
            reg::OUTLINE_COLLAPSE => {
                if let Some((document, _)) = self.outline.shown.clone() {
                    let expanded: Vec<Arc<str>> = self.outline.list.update(cx, |v, _| {
                        let ids = v
                            .widget
                            .tree_model()
                            .map(|t| {
                                (0..t.count())
                                    .filter(|i| t.can_collapse(*i))
                                    .filter_map(|i| t.id_at(i))
                                    .map(|i| i.0)
                                    .collect()
                            })
                            .unwrap_or_default();
                        v.widget.update_tree(|t| t.collapse_all());
                        v.widget.take_events();
                        ids
                    });
                    for key in expanded {
                        self.outline.note_expanded(document, key, false);
                    }
                }
            }
            reg::OUTLINE_FOLLOW_CURSOR => {
                self.outline.follow_cursor = !self.outline.follow_cursor;
                self.outline.revealed = None;
            }
            _ => {}
        }
        cx.notify();
    }

    fn write_breadcrumbs_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let task = self.settings.write(
            "breadcrumbs.enabled".into(),
            Scope::default(),
            Value::Bool(enabled),
            Cancellation::default(),
        );
        let task = cx.background_executor().spawn(task);
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                // 设置文件不可写（例如测试用的只读设置）：本窗口照样切换，重载时以文件为准。
                eprintln!("breadcrumbs.enabled: {e}");
                let _ = this.update(cx, |v, cx| {
                    v.outline.crumbs.enabled = enabled;
                    publish_breadcrumbs_enabled(enabled, cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 设置热加载：`outline.*` / `breadcrumbs.*` 变化时重读。
    pub(super) fn outline_settings_changed(
        &mut self,
        keys: &BTreeSet<Arc<str>>,
        cx: &mut Context<Self>,
    ) {
        if keys
            .iter()
            .any(|k| k.starts_with("outline.") || k.starts_with("breadcrumbs."))
        {
            let theme = self.theme.clone();
            self.outline.reload_settings(&*self.settings, theme, cx);
            publish_breadcrumbs_enabled(self.outline.crumbs.enabled, cx);
        }
    }

    // ------------------------------------------------------------------ 转到文件中的符号

    fn show_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.outline.goto.is_none() {
            let theme = self.theme.clone();
            let strings: Arc<dyn Strings> = Arc::new(appearance::strings().unwrap());
            let options = ListOptions::read(&*self.settings);
            let view = cx
                .new(|cx| QuickInputView::new(QuickInputService::new(options), theme, strings, cx));
            let (send, receive) = async_channel::unbounded::<QuickInputEvent>();
            let events = view.read(cx).service.events().subscribe(Arc::new(move |e| {
                let _ = send.try_send(e);
            }));
            cx.spawn(async move |this, cx| {
                while let Ok(event) = receive.recv().await {
                    if this.update(cx, |v, cx| v.goto_event(event, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
            cx.subscribe(&view, |this, _, event: &QuickInputViewEvent, cx| {
                // 接受事件经通道异步到达，这里不清除 `goto.id`（接受时按 ID 匹配）。
                let QuickInputViewEvent::Closed(_) = event;
                this.outline.focus_editor = true;
                cx.notify();
            })
            .detach();
            self.outline.goto = Some(Goto {
                view,
                id: None,
                document: None,
                revision: 0,
                _events: events,
            });
        }
        let document = self.focused_text().map(|(_, d, _)| d);
        let symbols = document
            .as_ref()
            .and_then(|d| self.outline.cached(d.id).cloned());
        let placeholder = match (&document, &symbols) {
            (None, _) => "gotoSymbol.noEditor",
            (Some(_), Some(s)) if !s.is_empty() => "gotoSymbol.placeholder",
            _ => "gotoSymbol.noSymbols",
        };
        let (list, separators) = self.goto_items(symbols.as_deref(), "");
        let goto = self.outline.goto.as_mut().unwrap();
        goto.document = document.map(|d| d.id);
        let view = goto.view.clone();
        let result = view.update(cx, |q, cx| {
            q.show(
                QuickInputOptions {
                    placeholder: Message::key(placeholder),
                    initial: "".into(),
                    items: Arc::new(list),
                    multi_select: false,
                },
                QuickPickConfig {
                    filter: None,
                    separators,
                    empty: Some(Message::key("gotoSymbol.noResults")),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        match result {
            Ok(id) => self.outline.goto.as_mut().unwrap().id = Some(id),
            Err(e) => eprintln!("gotoSymbol: {e}"),
        }
    }

    /// 候选与分组分隔行（`:` 前缀按种类分组）。排序与 #24 的 `@` 共用 [`goto::symbol_picks`]。
    fn goto_items(
        &mut self,
        symbols: Option<&DocumentSymbols>,
        query: &str,
    ) -> (goto::PickList, Vec<(usize, Option<Arc<str>>)>) {
        let revision = self.outline.goto.as_mut().map_or(0, |g| {
            g.revision += 1;
            g.revision
        });
        let Some(symbols) = symbols else {
            return (
                goto::PickList::new(&goto::SymbolPicks::default(), Version(revision)),
                Vec::new(),
            );
        };
        let matcher = matcher_for(MatchType::Fuzzy);
        let label = |id: SymbolId| {
            symbols
                .node(id)
                .map(|n| (self.outline.label)(&n.symbol))
                .unwrap_or_default()
        };
        let picks = goto::symbol_picks(symbols, query, matcher.as_ref(), &label);
        let separators = picks
            .groups
            .iter()
            .map(|(start, kind, count)| {
                let kind_label = self.label(&format!("symbolKind.{kind}"), &[]);
                let text = self.label(
                    "gotoSymbol.group",
                    &[("kind", kind_label), ("count", count.to_string())],
                );
                (*start, Some(Arc::from(text)))
            })
            .collect();
        (goto::PickList::new(&picks, Version(revision)), separators)
    }

    fn goto_event(&mut self, event: QuickInputEvent, cx: &mut Context<Self>) {
        let Some(goto) = &self.outline.goto else {
            return;
        };
        let (Some(active), Some(document)) = (goto.id, goto.document) else {
            return;
        };
        match event {
            QuickInputEvent::Changed { id, text } if id == active => {
                let symbols = self.outline.cached(document).cloned();
                let (list, separators) = self.goto_items(symbols.as_deref(), &text);
                let view = self.outline.goto.as_ref().unwrap().view.clone();
                view.update(cx, |q, cx| {
                    let _ = q.service.update(id, Arc::new(list));
                    if let Some(a) = q.service.active_mut() {
                        a.list.set_separators(separators);
                    }
                    cx.notify();
                });
            }
            QuickInputEvent::Accepted { id, items, .. } if id == active => {
                if let Some(key) = items.first() {
                    let key = key.0.clone();
                    self.reveal_key(
                        document,
                        &key,
                        false,
                        OpenSource::Other,
                        OpenTarget::Active,
                        cx,
                    );
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// 快速输入浮层（标题栏下方居中）。
    pub(super) fn goto_overlay(&self, cx: &App) -> Option<AnyElement> {
        let goto = self.outline.goto.as_ref()?;
        goto.view.read(cx).service.active()?;
        Some(
            deferred(
                div()
                    .absolute()
                    .top(px(40.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w(px(crate::list_widget::quick_input_view::WIDTH))
                            .occlude()
                            .child(goto.view.clone()),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

fn element_id(part: &str, group: EditorGroupId, n: u64) -> SharedString {
    SharedString::from(format!("{part}-{}-{n}", group.0))
}

/// `config.breadcrumbs.enabled` 上下文键（`breadcrumbs.toggleToOn` 的 when 用到它）。
fn publish_breadcrumbs_enabled(enabled: bool, cx: &mut App) {
    if let Some(keys) = cx.try_global::<crate::keys::Keys>() {
        let _ = keys.context.set(
            "config.breadcrumbs.enabled".into(),
            Some(Value::Bool(enabled)),
        );
    }
}
