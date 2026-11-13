//! 快速打开与命令面板（#24）在骨架窗口中的装配：浮层、四个前缀、历史、文件列表索引、打开与定位。
//! 打开与定位一律经 #17 的 `OpenDocuments`（`OpenSource::QuickOpen` / `GoToLine` / `Other`）。
use super::*;
use crate::{
    documents::OpenTarget,
    list_widget::{
        options::ListOptions,
        quick_input::{QuickInputService, QuickPickConfig, QuickPickEvent},
        quick_input_view::{QuickInputTextChanged, QuickInputView, QuickInputViewEvent},
    },
    lists::{QuickInput, QuickInputId, QuickInputOptions},
    outline::goto,
    quick_access::{
        commands::{palette_picks, CommandEntry, CommandHistory, PaletteLabels},
        files::{FileIndex, FileLabels, HistorySort, Search, SearchOptions},
        lines::{self, LineState},
        matcher::FuzzyMatcher,
        parse::{self, FileTarget, Mode},
        registry::{self as reg, QuickAccessAction, QuickOpenSettings},
        Picks,
    },
};
use gpui::deferred;
use rove_base::{Line, Position, Utf16Column, Utf8Column};
use rove_buffer::Document;

/// 最近打开的文件最多记多少个。
const HISTORY_LIMIT: usize = 100;

/// 窗口里快速打开的状态。
#[derive(Default)]
pub(super) struct QuickAccess {
    pub(super) view: Option<Entity<QuickInputView>>,
    pub(super) id: Option<QuickInputId>,
    /// 工作区文件列表（后台建好后才有）。
    pub(super) index: Option<FileIndex>,
    pub(super) building: bool,
    /// 建索引期间收到的文件事件：(工作区版本, 事件)。
    pub(super) pending_events: Vec<(u64, Arc<[FileEvent]>)>,
    pub(super) search: Search,
    /// 最近打开的文件，最近的在前。
    pub(super) history: Vec<WorkspacePath>,
    pub(super) recorded: Option<WorkspacePath>,
    pub(super) commands: CommandHistory,
    /// 命令面板的候选（每次打开时重新计算）。
    pub(super) entries: Option<Vec<CommandEntry>>,
    pub(super) revision: u64,
    /// `workbench.quickOpen.preserveInput`：上一次的输入。
    pub(super) last_input: String,
    /// 关闭后把焦点还给编辑器 / 运行选中的命令（需要窗口，留到下一帧处理）。
    pub(super) focus_editor: bool,
    pub(super) run: Option<Arc<str>>,
    pub(super) seen_focus: bool,
    pub(super) placeholder: &'static str,
    /// 打开时的活动文档（`:` 与 `@` 的目标）。
    pub(super) document: Option<DocumentId>,
    pub(super) matcher: Option<Arc<FuzzyMatcher>>,
    pub(super) subscriptions: Vec<gpui::Subscription>,
    pub(super) pick_events: Option<rove_base::Subscription>,
}

/// 一次计算的结果。
struct Output {
    picks: Picks,
    placeholder: &'static str,
    /// 列表为空时显示的文字（提示或“没有匹配的结果”）。
    empty: Option<Message>,
    loading: bool,
}

fn msg(key: &str, args: &[(&str, String)]) -> Message {
    Message {
        key: key.into(),
        args: args
            .iter()
            .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
            .collect(),
    }
}

impl Workbench {
    // ------------------------------------------------------------------ 命令入口

    pub(super) fn quick_access_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            reg::QUICK_OPEN => {
                // 已经打开时再按 `Ctrl+P`：选中下一项（VS Code 的 `quickOpenNavigateNext`）。
                if self.quick_active(cx) {
                    self.quick_navigate_next(window, cx);
                } else {
                    let initial = self.quick_initial();
                    self.show_quick_access(initial, window, cx);
                }
            }
            reg::SHOW_COMMANDS => self.show_quick_access(">".into(), window, cx),
            reg::GOTO_LINE => self.show_quick_access(":".into(), window, cx),
            _ => {}
        }
    }

    fn quick_initial(&self) -> String {
        if QuickOpenSettings::read(&*self.settings).preserve_input {
            self.quick.last_input.clone()
        } else {
            String::new()
        }
    }

    fn quick_active(&self, cx: &App) -> bool {
        self.quick
            .view
            .as_ref()
            .is_some_and(|v| v.read(cx).service.active().is_some())
    }

    fn quick_navigate_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.quick.view.clone() {
            view.update(cx, |q, cx| q.run_command("quickInput.next", window, cx));
        }
    }

    // ------------------------------------------------------------------ 浮层

    fn ensure_quick_view(&mut self, cx: &mut Context<Self>) -> Entity<QuickInputView> {
        if let Some(view) = &self.quick.view {
            return view.clone();
        }
        let theme = self.theme.clone();
        let strings: Arc<dyn Strings> = Arc::new(appearance::strings().unwrap());
        let options = ListOptions::read(&*self.settings);
        let view =
            cx.new(|cx| QuickInputView::new(QuickInputService::new(options), theme, strings, cx));
        // 输入变化：同步重算列表（每次按键到列表刷新的路径上没有异步等待）。
        let changed = cx.subscribe(&view, |this, _, event: &QuickInputTextChanged, cx| {
            if this.quick.id == Some(event.id) {
                this.quick.last_input = event.text.to_string();
                this.quick_refresh(&event.text, cx);
            }
        });
        let closed = cx.subscribe(&view, |this, _, event: &QuickInputViewEvent, cx| {
            let QuickInputViewEvent::Closed(id) = event;
            if this.quick.id == Some(*id) {
                this.quick.id = None;
            }
            this.quick.focus_editor = true;
            cx.notify();
        });
        // 接受（含修饰键）经通道回到 UI 线程。
        let (send, receive) = async_channel::unbounded::<QuickPickEvent>();
        let events = view
            .read(cx)
            .service
            .pick_events()
            .subscribe(Arc::new(move |e| {
                let _ = send.try_send(e);
            }));
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receive.recv().await {
                if this.update(cx, |v, cx| v.quick_picked(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        self.quick.subscriptions = vec![changed, closed];
        self.quick.pick_events = Some(events);
        self.quick.view = Some(view.clone());
        view
    }

    fn show_quick_access(&mut self, initial: String, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.ensure_quick_view(cx);
        self.quick.document = self.focused_text().map(|(_, d, _)| d.id);
        self.quick.search.reset();
        self.quick.entries = None;
        self.quick.seen_focus = false;
        let out = self.quick_compute(&initial, cx);
        self.quick.revision += 1;
        let list = out.picks.model(self.quick.revision);
        let placeholder = out.placeholder;
        let result = view.update(cx, |q, cx| {
            q.show(
                QuickInputOptions {
                    placeholder: Message::key(placeholder),
                    initial: initial.as_str().into(),
                    items: Arc::new(list),
                    multi_select: false,
                },
                QuickPickConfig {
                    filter: None,
                    separators: out.picks.separators.clone(),
                    loading_tail: out.loading.then(|| Message::key("quickAccess.loading")),
                    empty: out.empty.clone(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        match result {
            Ok(id) => {
                self.quick.id = Some(id);
                self.quick.placeholder = placeholder;
                self.quick.last_input = initial;
                view.update(cx, |q, cx| {
                    let _ = q.service.set_decor(id, out.picks.decor);
                    cx.notify();
                });
            }
            Err(e) => eprintln!("quickAccess: {e}"),
        }
        cx.notify();
    }

    /// 输入变化后刷新列表、分组、高亮与提示。
    fn quick_refresh(&mut self, text: &str, cx: &mut Context<Self>) {
        let (Some(view), Some(id)) = (self.quick.view.clone(), self.quick.id) else {
            return;
        };
        let out = self.quick_compute(text, cx);
        self.quick.revision += 1;
        let list = out.picks.model(self.quick.revision);
        let placeholder = out.placeholder;
        let change_placeholder = self.quick.placeholder != placeholder;
        self.quick.placeholder = placeholder;
        let placeholder_text = change_placeholder.then(|| self.label(placeholder, &[]));
        view.update(cx, |q, cx| {
            let _ = q.service.update(id, Arc::new(list));
            let _ = q.service.set_decor(id, out.picks.decor);
            if let Some(a) = q.service.active_mut() {
                a.list.set_separators(out.picks.separators);
                a.list.set_loading_tail(out.loading);
                a.config.loading_tail = out.loading.then(|| Message::key("quickAccess.loading"));
                a.config.empty = out.empty;
                // 每次输入后焦点回到第一项
                a.list.set_focus(None);
                a.list.focus_first();
            }
            if let Some(text) = placeholder_text {
                q.input().update(cx, |i, _| i.set_placeholder(text));
            }
            cx.notify();
        });
        cx.notify();
    }

    // ------------------------------------------------------------------ 计算

    fn quick_compute(&mut self, text: &str, cx: &App) -> Output {
        match parse::parse(text) {
            Mode::Files { query, .. } => self.quick_files(query),
            Mode::Commands(filter) => self.quick_commands(filter, cx),
            Mode::Line(rest) => self.quick_line(rest),
            Mode::Symbols(rest) => self.quick_symbols(rest),
            Mode::WorkspaceSymbols(_) => {
                let key = crate::keys::key_label(reg::QUICK_OPEN, cx)
                    .unwrap_or_else(|| "Ctrl+P".to_string());
                Output {
                    picks: Picks::default(),
                    placeholder: "quickAccess.workspaceSymbols.placeholder",
                    empty: Some(msg(
                        "quickAccess.workspaceSymbols.unavailable",
                        &[("key", key)],
                    )),
                    loading: false,
                }
            }
        }
    }

    fn quick_files(&mut self, query: &str) -> Output {
        let settings = QuickOpenSettings::read(&*self.settings);
        // 工作区还在扫描（或文件列表还在建）：先可用，列表底部显示“正在加载文件列表…”
        let loading = match &self.quick.index {
            Some(index) => !index.complete,
            None => !self.scan_failed && self.has_folder,
        };
        let picks = match &self.quick.index {
            Some(index) => {
                let history = self.quick.history.clone();
                let results = self.quick.search.run(
                    index,
                    query,
                    &history,
                    SearchOptions {
                        include_history: settings.include_history,
                        history_sort: if settings.history_sort_recency {
                            HistorySort::Recency
                        } else {
                            HistorySort::Default
                        },
                    },
                );
                let labels = FileLabels {
                    recent: self.label("quickAccess.group.recent", &[]).into(),
                    files: self.label("quickAccess.group.files", &[]).into(),
                };
                results.picks(index, &labels)
            }
            None => Picks::default(),
        };
        Output {
            picks,
            placeholder: "quickAccess.files.placeholder",
            empty: None,
            loading,
        }
    }

    fn command_entries(&self, cx: &App) -> Vec<CommandEntry> {
        let mut entries = Vec::new();
        for (id, command) in self.commands.registry().entries() {
            if reg::hidden_in_palette(id) || !crate::keys::is_implemented(id, cx) {
                continue;
            }
            let title = Message::key(command.title_key.clone());
            let Ok(mut label) = self.strings.text(&title) else {
                continue;
            };
            if let Some(category) = &command.category_key {
                if let Ok(category) = self.strings.text(&Message::key(category.clone())) {
                    label = format!("{category}: {label}");
                }
            }
            entries.push(CommandEntry {
                id: id.into(),
                label: label.into(),
                // 实际生效的键位（含用户在共用 keybindings.json 里的覆盖）
                keybinding: crate::keys::key_label(id, cx).map(Into::into),
            });
        }
        entries
    }

    fn quick_commands(&mut self, filter: &str, cx: &App) -> Output {
        // 命令列表与键位在一次打开期间不变：第一次需要时算好。
        if self.quick.entries.is_none() {
            self.quick.entries = Some(self.command_entries(cx));
        }
        let entries = self.quick.entries.clone().unwrap_or_default();
        let labels = PaletteLabels {
            recent: self.label("quickAccess.group.recentCommands", &[]).into(),
            other: self.label("quickAccess.group.otherCommands", &[]).into(),
        };
        Output {
            picks: palette_picks(&entries, &self.quick.commands, filter, &labels),
            placeholder: "quickAccess.commands.placeholder",
            empty: None,
            loading: false,
        }
    }

    /// 转到行/列：活动文本编辑器的文档。
    fn quick_line(&mut self, rest: &str) -> Output {
        let placeholder = "quickAccess.line.placeholder";
        let Some((_, document, cursor)) = self.focused_text() else {
            return Output {
                picks: Picks::default(),
                placeholder,
                empty: Some(Message::key("quickAccess.line.noEditor")),
                loading: false,
            };
        };
        let count = document.text.lines();
        let known = count.known.max(1);
        match line_state(rest, &count) {
            LineState::Go { line, column } => {
                let (key, args) = match column {
                    Some(c) => (
                        "quickAccess.line.goto",
                        vec![("line", line.to_string()), ("column", c.to_string())],
                    ),
                    None => (
                        "quickAccess.line.gotoLine",
                        vec![("line", line.to_string())],
                    ),
                };
                let mut label = self.label(key, &args);
                if !count.complete && line > known {
                    label = format!("{label} {}", self.label("quickAccess.line.indexing", &[]));
                }
                let mut picks = Picks::default();
                picks.push(
                    crate::lists::Row {
                        id: crate::lists::ItemId("goto".into()),
                        label: label.into(),
                        detail: None,
                        depth: 0,
                        expandable: false,
                        expanded: false,
                        disabled: false,
                    },
                    Default::default(),
                );
                Output {
                    picks,
                    placeholder,
                    empty: None,
                    loading: false,
                }
            }
            LineState::Prompt => Output {
                picks: Picks::default(),
                placeholder,
                empty: Some(msg(
                    "quickAccess.line.prompt",
                    &[
                        ("line", (cursor.line.0 + 1).to_string()),
                        ("column", (cursor.column.0 + 1).to_string()),
                        ("max", known.to_string()),
                    ],
                )),
                loading: false,
            },
        }
    }

    /// `@`：活动编辑器的符号，与“转到文件中的符号”共用候选与排序（#26 的 `goto::symbol_picks`）。
    fn quick_symbols(&mut self, rest: &str) -> Output {
        let placeholder = "quickAccess.symbols.placeholder";
        let unavailable = |placeholder| Output {
            picks: Picks::default(),
            placeholder,
            empty: Some(Message::key("quickAccess.symbols.unavailable")),
            loading: false,
        };
        let Some((_, document, _)) = self.focused_text() else {
            return unavailable(placeholder);
        };
        let Some(symbols) = self.outline.cached(document.id).cloned() else {
            return unavailable(placeholder);
        };
        if symbols.is_empty() {
            return unavailable(placeholder);
        }
        let matcher = self
            .quick
            .matcher
            .get_or_insert_with(|| Arc::new(FuzzyMatcher::default()))
            .clone();
        let label = |id| {
            symbols
                .node(id)
                .map(|n| (self.outline.label)(&n.symbol))
                .unwrap_or_default()
        };
        let found = goto::symbol_picks(&symbols, rest, matcher.as_ref(), &label);
        let mut picks = Picks::default();
        let mut groups = found.groups.iter().peekable();
        for (i, p) in found.items.iter().enumerate() {
            if let Some((start, kind, count)) = groups.peek().copied() {
                if *start == i {
                    let kind_label = self.label(&format!("symbolKind.{kind}"), &[]);
                    let text = self.label(
                        "gotoSymbol.group",
                        &[("kind", kind_label), ("count", count.to_string())],
                    );
                    picks.separators.push((i, Some(Arc::from(text))));
                    groups.next();
                }
            }
            picks.push(
                crate::lists::Row {
                    id: crate::lists::ItemId(p.key.clone()),
                    label: p.label.clone(),
                    detail: p.container.clone(),
                    depth: 0,
                    expandable: false,
                    expanded: false,
                    disabled: false,
                },
                crate::list_widget::quick_input::RowDecor {
                    label_highlights: p.highlights.clone(),
                    detail_highlights: Vec::new(),
                    trailing: None,
                },
            );
        }
        Output {
            picks,
            placeholder,
            empty: Some(Message::key("gotoSymbol.noResults")),
            loading: false,
        }
    }

    // ------------------------------------------------------------------ 接受

    fn quick_picked(&mut self, event: QuickPickEvent, cx: &mut Context<Self>) {
        let QuickPickEvent::Accepted {
            id,
            items,
            text,
            in_background,
            modifiers,
        } = event
        else {
            return;
        };
        if self.quick.id.is_some_and(|active| active != id) && !in_background {
            return;
        }
        let beside = modifiers.ctrl;
        let item = items.first().map(|i| i.0.clone());
        match parse::parse(&text) {
            Mode::Files { target, .. } => {
                if let Some(path) = item.and_then(|p| WorkspacePath::new(&p).ok()) {
                    self.quick_open_file(path, target, beside, cx);
                }
            }
            Mode::Commands(_) => {
                if let Some(command) = item {
                    self.quick.commands.record(&command);
                    self.quick.run = Some(command);
                }
            }
            Mode::Line(rest) => {
                if item.is_some() {
                    self.quick_goto_line(rest, cx);
                }
            }
            Mode::Symbols(_) => {
                if let (Some(key), Some(document)) = (item, self.quick.document) {
                    let target = if beside {
                        OpenTarget::Beside
                    } else {
                        OpenTarget::Active
                    };
                    self.reveal_key(document, &key, false, OpenSource::Other, target, cx);
                }
            }
            Mode::WorkspaceSymbols(_) => {}
        }
        self.quick.focus_editor = true;
        cx.notify();
    }

    fn quick_open_file(
        &mut self,
        path: WorkspacePath,
        target: Option<FileTarget>,
        beside: bool,
        cx: &mut Context<Self>,
    ) {
        self.record_history(&path);
        let location = target.filter(|t| t.line > 0).map(|t| Position {
            line: Line(u64::from(t.line) - 1),
            column: Utf8Column(u64::from(t.column.saturating_sub(1))),
        });
        let options = OpenOptions {
            source: OpenSource::QuickOpen,
            target: if beside {
                OpenTarget::Beside
            } else {
                OpenTarget::Active
            },
            preserve_focus: false,
            location,
            ..Default::default()
        };
        if let Err(e) =
            self.open_documents()
                .open(Resource::Workspace(path), options, Cancellation::default())
        {
            eprintln!("quickOpen: {e}");
        }
        cx.notify();
    }

    fn quick_goto_line(&mut self, rest: &str, cx: &mut Context<Self>) {
        let Some((group, document, _)) = self.focused_text() else {
            return;
        };
        let LineState::Go { line, column } = line_state(rest, &document.text.lines()) else {
            return;
        };
        let line0 = Line(line - 1);
        let column = column.map_or(Utf8Column(0), |c| line_column(&document, line0, c));
        let Some(resource) = self
            .editor
            .groups
            .get(&group)
            .and_then(|g| g.current())
            .map(|t| t.resource.clone())
        else {
            return;
        };
        let options = OpenOptions {
            source: OpenSource::GoToLine,
            preserve_focus: false,
            location: Some(Position {
                line: line0,
                column,
            }),
            ..Default::default()
        };
        if let Err(e) = self
            .open_documents()
            .open(resource, options, Cancellation::default())
        {
            eprintln!("gotoLine: {e}");
        }
        cx.notify();
    }

    // ------------------------------------------------------------------ 历史

    fn record_history(&mut self, path: &WorkspacePath) {
        let history = &mut self.quick.history;
        history.retain(|p| p != path);
        history.insert(0, path.clone());
        history.truncate(HISTORY_LIMIT);
    }

    // ------------------------------------------------------------------ 每帧

    /// 每帧调用：记录活动文件、失去焦点时关闭、关闭后还焦点或运行命令。
    pub(super) fn sync_quick_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 活动编辑器进入“最近打开”
        let active = self
            .editor
            .groups
            .get(&self.editor.focused)
            .and_then(|g| g.current())
            .and_then(|t| match &t.resource {
                Resource::Workspace(p) => Some(p.clone()),
                _ => None,
            });
        if self.quick.recorded != active {
            if let Some(path) = &active {
                self.record_history(path);
            }
            self.quick.recorded = active;
        }

        // `workbench.quickOpen.closeOnFocusLost`
        if let (Some(view), true) = (self.quick.view.clone(), self.quick.id.is_some()) {
            let focused = {
                let v = view.read(cx);
                v.focus_handle(cx).contains_focused(window, cx)
                    || v.input().focus_handle(cx).contains_focused(window, cx)
            };
            if focused {
                self.quick.seen_focus = true;
            } else if self.quick.seen_focus
                && QuickOpenSettings::read(&*self.settings).close_on_focus_lost
            {
                view.update(cx, |q, cx| q.run_command("quickInput.hide", window, cx));
            }
        }

        if std::mem::take(&mut self.quick.focus_editor) {
            self.quick_focus_editor(window, cx);
        }
        if let Some(command) = self.quick.run.take() {
            self.quick_focus_editor(window, cx);
            let action = crate::keys::action_for(&command).unwrap_or_else(|| {
                Box::new(crate::keys::RunCommand {
                    id: SharedString::from(command.to_string()),
                    args: Value::Null,
                })
            });
            window.dispatch_action(action, cx);
        }
    }

    /// 浮层关闭后把焦点还给编辑器。没有打开文本编辑器时代码视图不会绘制，它的焦点句柄不在绘制树里
    /// （按键与 action 都找不到路径），这时回到编辑器区域本身。
    fn quick_focus_editor(&self, window: &mut Window, cx: &App) {
        if self.focused_text().is_some() {
            self.focus_group_code(self.editor.focused, window, cx);
        } else {
            window.focus(&self.focus);
        }
    }

    /// 快速输入浮层（标题栏下方居中）。
    pub(super) fn quick_overlay(&self, cx: &App) -> Option<AnyElement> {
        let view = self.quick.view.as_ref()?;
        view.read(cx).service.active()?;
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
                            .child(view.clone()),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    pub(super) fn quick_action_handler(
        &mut self,
        action: &QuickAccessAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.quick_access_command(action.0, window, cx);
    }

    // ------------------------------------------------------------------ 工作区文件列表

    /// 工作区就绪（或需要重新扫描）：后台建文件索引。
    pub(super) fn quick_workspace_ready(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        self.quick.building = true;
        let task = cx
            .background_executor()
            .spawn(async move { FileIndex::from_workspace(&*workspace) });
        cx.spawn(async move |this, cx| {
            let index = task.await;
            let _ = this.update(cx, |v, cx| v.quick_index_ready(index, cx));
        })
        .detach();
    }

    fn quick_index_ready(&mut self, mut index: FileIndex, cx: &mut Context<Self>) {
        self.quick.building = false;
        let pending = std::mem::take(&mut self.quick.pending_events);
        let mut rescan = false;
        for (revision, events) in pending {
            if revision > index.revision {
                rescan |= index.apply(&events);
            }
        }
        self.quick.index = Some(index);
        self.quick.search.reset();
        if rescan {
            self.quick_workspace_ready(cx);
        }
        self.quick_refresh_current(cx);
    }

    /// 工作区文件事件（文件增删改名）。
    pub(super) fn quick_files_changed(
        &mut self,
        batches: Vec<(u64, Arc<[FileEvent]>)>,
        cx: &mut Context<Self>,
    ) {
        let mut rescan = false;
        for (revision, events) in batches {
            if self.quick.building {
                self.quick.pending_events.push((revision, events));
            } else if let Some(index) = &mut self.quick.index {
                if revision > index.revision {
                    rescan |= index.apply(&events);
                    index.revision = revision;
                }
            }
        }
        if rescan {
            self.quick_workspace_ready(cx);
        } else {
            self.quick_refresh_current(cx);
        }
    }

    /// 浮层打开时按当前输入重新计算（文件列表变了）。
    fn quick_refresh_current(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.quick.view.clone() else {
            return;
        };
        let text = view.read(cx).service.active().map(|a| a.text.clone());
        if let Some(text) = text {
            self.quick.search.reset();
            self.quick_refresh(&text, cx);
        }
    }
}

/// `lines::state`：未建完行索引时，超出已知行数的行号照样接受（打开服务会等索引）。
fn line_state(rest: &str, count: &rove_buffer::LineCount) -> LineState {
    let state = lines::state(rest, count.known.max(1));
    if count.complete {
        return state;
    }
    match state {
        LineState::Prompt => {
            // 行号超出已知范围：按正数接受
            let first = rest
                .split([',', ':', '#'])
                .next()
                .and_then(|p| p.trim().parse::<u64>().ok())
                .filter(|n| *n > count.known);
            match first {
                Some(line) => LineState::Go {
                    line,
                    column: rest
                        .split([',', ':', '#'])
                        .nth(1)
                        .and_then(|p| p.trim().parse::<u64>().ok())
                        .filter(|c| *c > 0),
                },
                None => LineState::Prompt,
            }
        }
        go => go,
    }
}

/// VS Code 的列按 UTF-16 码元数，转成这一行的 UTF-8 字节列；越界时落在行尾。
fn line_column(document: &Document, line: Line, column: u64) -> Utf8Column {
    let column = column.saturating_sub(1);
    if let Ok(c) = document.text.utf16_to_utf8(line, Utf16Column(column)) {
        return c;
    }
    match document.text.line_range(line) {
        Ok(range) => {
            let start = range.start().0;
            Utf8Column(range.end().0.saturating_sub(start))
        }
        Err(_) => Utf8Column(0),
    }
}
