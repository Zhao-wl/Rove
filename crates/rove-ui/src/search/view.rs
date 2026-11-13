//! 搜索视图（GPUI）：查询框 + 区分大小写 / 全字匹配 / 正则三个开关、“包含 / 排除的文件”、“使用排除设置”，
//! 进度条、结果摘要、按文件分组的虚拟化结果列表（#18 `ListView`）与空状态（UX_SPEC §6.2 / §6.3）。
//!
//! 搜索在后台执行器上调用 [`rove_search::search`]；批次经通道回到窗口线程，按代次丢弃过期结果。
//! 打开结果不在这里做：视图发出 [`SearchViewEvent::Open`]，宿主经 #17 的 `OpenDocuments`
//! （`OpenSource::Search`，带选区，`.md` 也打开源码）定位。
use super::{
    model::{self, parse_id, ResultRows, Results, Target},
    registry::{self as reg, context as ck},
};
use crate::{
    list_widget::{
        options::{ListOptions, OpenMode},
        text_input::{TextInput, TextInputColors, TextInputEvent},
        view::{hsla, ListView, RowContext, RowRenderer},
        widget::{ListEvent, ListWidget, RowHeight},
    },
    shell::icons,
};
use gpui::{
    div, prelude::*, px, App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    HighlightStyle, Hsla, KeyContext, SharedString, StyledText, Window,
};
use rove_base::{
    Cancellation, Error, Line, Position, Result as CoreResult, Span, TextRange, Utf8Column,
    WorkspacePath,
};
use rove_config::Settings;
use rove_i18n::{Message, Strings};
use rove_search::{settings::ViewSettings, FileMatches, SearchConfig, SearchQuery, SearchSummary};
use rove_theme::Theme;
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

/// 打开一处结果的请求（宿主经 `OpenDocuments` 执行）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    pub path: WorkspacePath,
    /// 匹配区间（UTF-8 列），打开后选中。
    pub selection: TextRange,
    pub side: bool,
    pub preserve_focus: bool,
    pub pinned: bool,
}

/// #52：`addCursorsAtSearchResults`——打开这个文件并在每一处匹配上放一个光标。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddCursorsRequest {
    pub path: WorkspacePath,
    /// 该文件里全部匹配的区间（UTF-8 列），按出现顺序。
    pub selections: Vec<TextRange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchViewEvent {
    Open(OpenRequest),
    /// “查看排除设置”链接。
    OpenSettings,
    /// #52：在结果处添加光标。
    AddCursors(AddCursorsRequest),
}

/// 视图的搜索状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Searching,
    Done(SearchSummary),
    Cancelled,
    InvalidPattern,
    Failed,
}

/// 三个开关与“使用排除设置”。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Toggles {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub use_exclude_settings: bool,
}

impl Default for Toggles {
    fn default() -> Self {
        Self {
            case_sensitive: false,
            whole_word: false,
            regex: false,
            use_exclude_settings: true,
        }
    }
}

enum Message_ {
    Batch(Vec<FileMatches>),
    Done(CoreResult<SearchSummary>),
}

/// 视图颜色（主题缺键时为 `None`，不设色）。
#[derive(Clone)]
struct Colors {
    fg: Option<Hsla>,
    info: Option<Hsla>,
    description: Option<Hsla>,
    link: Option<Hsla>,
    error: Option<Hsla>,
    progress: Option<Hsla>,
    badge_bg: Option<Hsla>,
    badge_fg: Option<Hsla>,
    match_bg: Option<Hsla>,
    input_bg: Option<Hsla>,
    input_fg: Option<Hsla>,
    input_border: Option<Hsla>,
    input_placeholder: Option<Hsla>,
    focus_border: Option<Hsla>,
    option_bg: Option<Hsla>,
    option_border: Option<Hsla>,
    option_fg: Option<Hsla>,
    option_hover: Option<Hsla>,
    error_bg: Option<Hsla>,
    error_border: Option<Hsla>,
    selection: Option<Hsla>,
}

impl Colors {
    fn resolve(theme: &dyn Theme) -> Self {
        let c = |key: &str| theme.color(key).ok().flatten().map(hsla);
        Self {
            fg: c("sideBar.foreground").or(c("foreground")),
            info: c("search.resultsInfoForeground").or(c("descriptionForeground")),
            description: c("descriptionForeground"),
            link: c("textLink.foreground"),
            error: c("errorForeground"),
            progress: c("progressBar.background"),
            badge_bg: c("badge.background"),
            badge_fg: c("badge.foreground"),
            match_bg: c("editor.findMatchHighlightBackground"),
            input_bg: c("input.background"),
            input_fg: c("input.foreground"),
            input_border: c("input.border"),
            input_placeholder: c("input.placeholderForeground"),
            focus_border: c("focusBorder"),
            option_bg: c("inputOption.activeBackground"),
            option_border: c("inputOption.activeBorder"),
            option_fg: c("inputOption.activeForeground"),
            option_hover: c("inputOption.hoverBackground"),
            error_bg: c("inputValidation.errorBackground"),
            error_border: c("inputValidation.errorBorder"),
            selection: c("editor.selectionBackground").or(c("list.activeSelectionBackground")),
        }
    }
}

fn paint(c: Option<Hsla>) -> Hsla {
    c.unwrap_or_else(gpui::transparent_black)
}

pub struct SearchView {
    root: PathBuf,
    settings: Arc<dyn Settings>,
    strings: Arc<dyn Strings>,
    theme: Arc<dyn Theme>,
    colors: Colors,
    pub query: Entity<TextInput>,
    pub includes: Entity<TextInput>,
    pub excludes: Entity<TextInput>,
    pub list: Entity<ListView>,
    focus: FocusHandle,
    pub toggles: Toggles,
    view: ViewSettings,
    results: Results,
    rows: Rc<RefCell<Arc<ResultRows>>>,
    state: State,
    generation: u64,
    cancel: Option<Cancellation>,
    /// 上一次真正发起的查询（刷新、限制到文件夹、“包含排除的文件再搜索一次”用）。
    last_query: Option<SearchQuery>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<SearchViewEvent> for SearchView {}

impl Focusable for SearchView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SearchView {
    pub fn new(
        root: PathBuf,
        settings: Arc<dyn Settings>,
        theme: Arc<dyn Theme>,
        strings: Arc<dyn Strings>,
        cx: &mut Context<Self>,
    ) -> Self {
        let colors = Colors::resolve(&*theme);
        let input_colors = TextInputColors {
            text: paint(colors.input_fg.or(colors.fg)),
            placeholder: paint(colors.input_placeholder.or(colors.description)),
            caret: paint(colors.input_fg.or(colors.fg)),
            selection: paint(colors.selection),
        };
        let text = |key: &str| {
            strings
                .text(&Message::key(key))
                .unwrap_or_else(|_| key.to_string())
        };
        let input = |placeholder: String, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut input = TextInput::new(cx, input_colors);
                input.set_placeholder(placeholder);
                input
            })
        };
        let query = input(text("search.placeholder"), cx);
        let includes = input(text("search.includes.placeholder"), cx);
        let excludes = input(text("search.excludes.placeholder"), cx);
        let view = ViewSettings::read(&*settings);
        let results = Results::new(view.sort_order, view.collapse);
        let rows = Rc::new(RefCell::new(results.rows()));
        let options = ListOptions {
            open_mode: OpenMode::SingleClick,
            ..ListOptions::read(&*settings)
        };
        let renderer = row_renderer(rows.clone(), colors.clone(), view.clone());
        let initial: Arc<dyn crate::lists::ListModel> = rows.borrow().clone();
        let list = cx.new(|cx| {
            let widget = ListWidget::new(initial, RowHeight::Fixed(22.0), options);
            let mut list = ListView::new(widget, theme.clone(), strings.clone(), cx);
            list.set_renderer(renderer);
            list
        });
        let mut subscriptions = vec![];
        for input in [&query, &includes, &excludes] {
            subscriptions.push(
                cx.subscribe(input, |v: &mut Self, _, e: &TextInputEvent, cx| {
                    if matches!(e, TextInputEvent::Changed { .. }) {
                        v.input_changed(cx);
                    }
                }),
            );
        }
        subscriptions.push(cx.subscribe(&list, |v: &mut Self, _, e: &ListEvent, cx| {
            v.list_event(e, cx)
        }));
        Self {
            root,
            settings,
            strings,
            theme,
            colors,
            query,
            includes,
            excludes,
            list,
            focus: cx.focus_handle(),
            toggles: Toggles::default(),
            view,
            results,
            rows,
            state: State::Idle,
            generation: 0,
            cancel: None,
            last_query: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn results(&self) -> &Results {
        &self.results
    }

    pub fn rows(&self) -> Arc<ResultRows> {
        self.rows.borrow().clone()
    }

    pub fn has_results(&self) -> bool {
        !self.results.is_empty()
    }

    /// 设置变化（`search.*`、`workbench.list.*`）：重读展示设置并重排；引擎设置在下次搜索时读取。
    pub fn reload_settings(&mut self, cx: &mut Context<Self>) {
        self.view = ViewSettings::read(&*self.settings);
        self.results
            .set_view_options(self.view.sort_order, self.view.collapse);
        let renderer = row_renderer(self.rows.clone(), self.colors.clone(), self.view.clone());
        self.list.update(cx, |l, _| l.set_renderer(renderer));
        self.refresh_rows(cx);
    }

    /// 主题变化：重新解析颜色。
    pub fn set_theme(&mut self, theme: Arc<dyn Theme>, cx: &mut Context<Self>) {
        self.colors = Colors::resolve(&*theme);
        self.theme = theme;
        self.reload_settings(cx);
    }

    /// 把焦点交给查询框并全选（`workbench.action.findInFiles`、显示视图）。
    pub fn focus_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.query.focus_handle(cx));
        self.query.update(cx, |q, cx| q.select_all(cx));
    }

    /// 用编辑器的选中内容作为查询（只取单行），并立即搜索。
    pub fn seed(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() || text.contains('\n') || text == self.query.read(cx).text() {
            return;
        }
        self.query.update(cx, |q, cx| q.set_text(text, cx));
        self.search_now(cx);
    }

    /// 编程式设置查询文本并立即搜索（测试与命令用）。
    pub fn set_query(&mut self, text: &str, cx: &mut Context<Self>) {
        self.query.update(cx, |q, cx| q.set_text(text, cx));
        self.search_now(cx);
    }

    fn current_query(&self, cx: &App) -> SearchQuery {
        SearchQuery {
            pattern: self.query.read(cx).text().to_string(),
            is_regex: self.toggles.regex,
            case_sensitive: self.toggles.case_sensitive,
            whole_word: self.toggles.whole_word,
            includes: self.includes.read(cx).text().to_string(),
            excludes: self.excludes.read(cx).text().to_string(),
            use_exclude_settings: self.toggles.use_exclude_settings,
        }
    }

    fn input_changed(&mut self, cx: &mut Context<Self>) {
        if !self.view.search_on_type {
            return;
        }
        // `search.searchOnTypeDebouncePeriod` 之后才发起；期间的新输入使旧的计划作废（代次）。
        self.generation += 1;
        let generation = self.generation;
        let delay = Duration::from_millis(self.view.debounce_ms);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |v, cx| {
                if v.generation == generation {
                    v.search_now(cx);
                }
            });
        })
        .detach();
    }

    /// 立即按当前输入搜索（取消进行中的搜索）。
    pub fn search_now(&mut self, cx: &mut Context<Self>) {
        let query = self.current_query(cx);
        self.run(query, cx);
    }

    fn run(&mut self, query: SearchQuery, cx: &mut Context<Self>) {
        self.cancel_running();
        self.generation += 1;
        let generation = self.generation;
        self.results.clear();
        if query.pattern.is_empty() {
            self.state = State::Idle;
            self.last_query = None;
            self.refresh_rows(cx);
            return;
        }
        let config = SearchConfig::from_settings(self.settings.clone());
        if rove_search::query::compile(&query, config.smart_case).is_err() {
            self.state = State::InvalidPattern;
            self.last_query = None;
            self.refresh_rows(cx);
            return;
        }
        self.state = State::Searching;
        self.last_query = Some(query.clone());
        self.refresh_rows(cx);
        let cancel = Cancellation::default();
        self.cancel = Some(cancel.clone());
        let (send, receive) = async_channel::unbounded::<Message_>();
        let root = self.root.clone();
        cx.background_executor()
            .spawn(async move {
                let batches = send.clone();
                let result = rove_search::search(&root, &query, &config, &cancel, |batch| {
                    let _ = batches.try_send(Message_::Batch(batch));
                });
                let _ = send.try_send(Message_::Done(result));
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(first) = receive.recv().await {
                let mut messages = vec![first];
                while let Ok(more) = receive.try_recv() {
                    messages.push(more);
                }
                let done = messages.iter().any(|m| matches!(m, Message_::Done(_)));
                let alive = this
                    .update(cx, |v, cx| v.apply(generation, messages, cx))
                    .unwrap_or(false);
                if done || !alive {
                    break;
                }
            }
        })
        .detach();
    }

    /// 应用一组到达的消息；返回 false 表示这次搜索已被取代，不必再收。
    fn apply(&mut self, generation: u64, messages: Vec<Message_>, cx: &mut Context<Self>) -> bool {
        if generation != self.generation || self.state != State::Searching {
            return false;
        }
        for message in messages {
            match message {
                Message_::Batch(files) => {
                    for file in files {
                        self.results.add(file);
                    }
                }
                Message_::Done(Ok(summary)) => {
                    self.cancel = None;
                    self.state = State::Done(summary);
                }
                Message_::Done(Err(Error::Cancelled)) => {
                    self.cancel = None;
                    self.state = State::Cancelled;
                }
                Message_::Done(Err(e)) => {
                    eprintln!("search: {e}");
                    self.cancel = None;
                    self.state = State::Failed;
                }
            }
        }
        self.refresh_rows(cx);
        true
    }

    fn cancel_running(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
    }

    /// `search.action.cancel`：停止进行中的搜索，已经出现的结果保留。
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.state == State::Searching {
            self.cancel_running();
            self.generation += 1;
            self.state = State::Cancelled;
            cx.notify();
        }
    }

    fn refresh_rows(&mut self, cx: &mut Context<Self>) {
        let rows = self.results.rows();
        *self.rows.borrow_mut() = rows.clone();
        self.list.update(cx, |l, cx| {
            l.widget.set_model(rows);
            if l.widget.focus().is_none() && !l.widget.is_empty() {
                l.widget.set_focus(Some(0));
            }
            cx.notify();
        });
        cx.notify();
    }

    fn focused_target(&self, cx: &App) -> Option<Target> {
        let list = self.list.read(cx);
        list.widget.focused_id().as_ref().and_then(parse_id)
    }

    fn match_selection(&self, target: &Target) -> Option<(WorkspacePath, TextRange)> {
        let rows = self.rows.borrow();
        match target {
            Target::Match(path, line, range) => {
                let file = rows.file(path)?;
                let l = file.lines.get(*line)?;
                let r = l.ranges.get(*range)?;
                Some((path.clone(), selection(l.line, r.start, r.end)))
            }
            Target::File(path) => {
                let file = rows.file(path)?;
                let l = file.lines.first()?;
                let r = l.ranges.first()?;
                Some((path.clone(), selection(l.line, r.start, r.end)))
            }
        }
    }

    fn open(
        &mut self,
        target: &Target,
        side: bool,
        preserve_focus: bool,
        pinned: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some((path, selection)) = self.match_selection(target) {
            cx.emit(SearchViewEvent::Open(OpenRequest {
                path,
                selection,
                side,
                preserve_focus,
                pinned,
            }));
        }
    }

    /// #52：`addCursorsAtSearchResults`。聚焦的是文件行还是匹配行都取该文件的全部匹配
    /// （VS Code 在选中整个文件时也是这个行为）。
    fn add_cursors_at_results(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.focused_target(cx) else {
            return;
        };
        let path = target_path(&target).clone();
        let selections: Vec<TextRange> = {
            let rows = self.rows.borrow();
            let Some(file) = rows.file(&path) else {
                return;
            };
            file.lines
                .iter()
                .flat_map(|l| {
                    l.ranges
                        .iter()
                        .map(move |r| selection(l.line, r.start, r.end))
                })
                .collect()
        };
        if selections.is_empty() {
            return;
        }
        cx.emit(SearchViewEvent::AddCursors(AddCursorsRequest {
            path,
            selections,
        }));
    }

    fn toggle_file(
        &mut self,
        path: &WorkspacePath,
        expanded: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = self.results.file(path) else {
            return;
        };
        let now = self.results.is_expanded(file);
        let want = expanded.unwrap_or(!now);
        if self.results.set_expanded(path, want) {
            self.refresh_rows(cx);
        }
    }

    fn list_event(&mut self, event: &ListEvent, cx: &mut Context<Self>) {
        if let ListEvent::Open {
            ids,
            preserve_focus,
            pinned,
            side_by_side,
        } = event
        {
            let Some(target) = ids.first().and_then(parse_id) else {
                return;
            };
            match &target {
                // 文件行：单击 / 回车展开或折叠（与 VS Code 树的 expandMode 一致）。
                Target::File(path) if !side_by_side => {
                    let path = path.clone();
                    self.toggle_file(&path, None, cx);
                }
                _ => self.open(&target, *side_by_side, *preserve_focus, *pinned, cx),
            }
        }
    }

    /// 宿主转来的命令（`search.action.*`、`toggleSearch*`）。
    pub fn command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            reg::TOGGLE_CASE => self.flip(|t| t.case_sensitive = !t.case_sensitive, cx),
            reg::TOGGLE_WORD => self.flip(|t| t.whole_word = !t.whole_word, cx),
            reg::TOGGLE_REGEX => self.flip(|t| t.regex = !t.regex, cx),
            reg::OPEN_RESULT | reg::OPEN_RESULT_TO_SIDE => {
                let side = id == reg::OPEN_RESULT_TO_SIDE;
                match self.focused_target(cx) {
                    Some(Target::File(path)) if !side => self.toggle_file(&path, None, cx),
                    Some(target) => self.open(&target, side, false, true, cx),
                    None => {}
                }
            }
            // #52：当前聚焦文件的全部匹配变成多光标。
            reg::ADD_CURSORS => self.add_cursors_at_results(cx),
            reg::NEXT_RESULT => self.step(true, cx),
            reg::PREVIOUS_RESULT => self.step(false, cx),
            reg::REMOVE => self.remove_focused(cx),
            reg::COPY_MATCH => {
                if let Some(text) = self.focused_target(cx).and_then(|t| self.copy_text(&t)) {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            reg::COPY_PATH => {
                if let Some(path) = self.focused_target(cx).map(|t| target_path(&t).clone()) {
                    let absolute = self
                        .root
                        .join(path.as_str().replace('/', std::path::MAIN_SEPARATOR_STR));
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        absolute.to_string_lossy().into_owned(),
                    ));
                }
            }
            reg::COPY_ALL => {
                let text = self.all_text();
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            reg::RESTRICT_TO_FOLDER => {
                if let Some(target) = self.focused_target(cx) {
                    let path = target_path(&target).as_str().to_string();
                    let folder = path.rfind('/').map_or("", |i| &path[..i]).to_string();
                    let include = if folder.is_empty() {
                        String::new()
                    } else {
                        format!("./{folder}")
                    };
                    self.includes.update(cx, |i, cx| i.set_text(&include, cx));
                    self.search_now(cx);
                }
            }
            reg::CANCEL => {
                self.cancel(cx);
                self.focus_query(window, cx);
            }
            reg::CLEAR => {
                self.query.update(cx, |q, cx| q.set_text("", cx));
                self.run(SearchQuery::new(""), cx);
                self.focus_query(window, cx);
            }
            reg::REFRESH => {
                if let Some(q) = self.last_query.clone() {
                    self.run(q, cx);
                }
            }
            reg::COLLAPSE_ALL | reg::EXPAND_ALL => {
                self.results.set_all_expanded(id == reg::EXPAND_ALL);
                self.refresh_rows(cx);
            }
            reg::FOCUS_FROM_RESULTS => self.focus_query(window, cx),
            reg::NEXT_INPUT | reg::PREVIOUS_INPUT => {
                self.cycle_input(id == reg::NEXT_INPUT, window, cx)
            }
            _ => {}
        }
        cx.notify();
    }

    fn flip(&mut self, f: impl FnOnce(&mut Toggles), cx: &mut Context<Self>) {
        f(&mut self.toggles);
        self.search_now(cx);
    }

    /// `ctrl+down` / `ctrl+up`：查询框 → 包含 → 排除 → 结果列表。
    fn cycle_input(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let handles = [
            self.query.focus_handle(cx),
            self.includes.focus_handle(cx),
            self.excludes.focus_handle(cx),
            self.list.focus_handle(cx),
        ];
        let current = handles.iter().position(|h| h.is_focused(window));
        let next = match (current, forward) {
            (Some(i), true) => (i + 1).min(handles.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
            (None, _) => 0,
        };
        window.focus(&handles[next]);
    }

    /// F4 / Shift+F4：焦点移到下一处 / 上一处匹配（循环，必要时展开文件），并在编辑器里预览。
    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.results.is_empty() {
            return;
        }
        let rows = self.rows();
        let total = rows.len();
        let start = self.list.read(cx).widget.focus();
        let mut i = start;
        let mut found = None;
        for _ in 0..total {
            let next = match (i, forward) {
                (None, true) => 0,
                (None, false) => total - 1,
                (Some(k), true) => (k + 1) % total,
                (Some(k), false) => (k + total - 1) % total,
            };
            i = Some(next);
            match rows.target(next) {
                Some(t @ Target::Match(..)) => {
                    found = Some(t);
                    break;
                }
                Some(Target::File(path)) if !rows.is_expanded_at(self.file_index(&path)) => {
                    // 折叠的文件：展开后它的第一 / 最后一处匹配。
                    self.results.set_expanded(&path, true);
                    self.refresh_rows(cx);
                    let rows = self.rows();
                    let file = rows.file(&path).cloned();
                    if let Some(file) = file {
                        let k = if forward {
                            0
                        } else {
                            file.count().saturating_sub(1)
                        };
                        if let Some((line, range)) = file.nth(k) {
                            found = Some(Target::Match(path.clone(), line, range));
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
        let Some(target) = found else {
            return;
        };
        let rows = self.rows();
        if let Some(index) = rows.index_of(&target) {
            self.list.update(cx, |l, cx| {
                l.widget.set_focus(Some(index));
                l.widget.set_selection(vec![index]);
                l.widget.reveal(index);
                cx.notify();
            });
        }
        self.open(&target, false, true, false, cx);
    }

    fn file_index(&self, path: &WorkspacePath) -> usize {
        self.results
            .files()
            .iter()
            .position(|f| &f.path == path)
            .unwrap_or(usize::MAX)
    }

    fn remove_focused(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.focused_target(cx) else {
            return;
        };
        let index = self.list.read(cx).widget.focus();
        if self.results.remove(&target) {
            self.refresh_rows(cx);
            let len = self.rows().len();
            self.list.update(cx, |l, cx| {
                let i = index
                    .map(|i| i.min(len.saturating_sub(1)))
                    .filter(|_| len > 0);
                l.widget.set_focus(i);
                l.widget.set_selection(i.into_iter().collect());
                cx.notify();
            });
        }
    }

    fn copy_text(&self, target: &Target) -> Option<String> {
        let rows = self.rows.borrow();
        match target {
            Target::Match(path, line, _) => {
                let l = rows.file(path)?.lines.get(*line)?;
                Some(format!("  {}: {}", l.line + 1, l.preview))
            }
            Target::File(path) => {
                let file = rows.file(path)?;
                let mut out = path.as_str().to_string();
                for l in &file.lines {
                    out.push_str(&format!("\n  {}: {}", l.line + 1, l.preview));
                }
                Some(out)
            }
        }
    }

    fn all_text(&self) -> String {
        self.results
            .files()
            .iter()
            .filter_map(|f| self.copy_text(&Target::File(f.path.clone())))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// 发布在视图根元素上的上下文键。
    fn context(&self, window: &Window, cx: &App) -> KeyContext {
        let mut k = KeyContext::default();
        k.add(ck::VIEWLET_FOCUS);
        k.set(
            ck::STATE,
            if self.state == State::Searching {
                "1"
            } else {
                "0"
            },
        );
        if self.list.focus_handle(cx).is_focused(window) {
            let rows = self.rows.borrow();
            let focus = self.list.read(cx).widget.focus();
            match focus.and_then(|i| rows.target(i)) {
                Some(Target::File(_)) => {
                    k.add(ck::FILE_OR_MATCH_FOCUS);
                    k.add(ck::FILE_FOCUS);
                    k.add(ck::FILE_OR_FOLDER_RESOURCE_FOCUS);
                    // 列表模式没有文件夹节点：在文件行上也允许“限制搜索文件夹”（限制到它所在的文件夹）。
                    k.add(ck::FOLDER_RESOURCE_FOCUS);
                }
                Some(Target::Match(..)) => {
                    k.add(ck::FILE_OR_MATCH_FOCUS);
                    k.add(ck::MATCH_FOCUS);
                    if focus == Some(1) {
                        k.add(ck::FIRST_MATCH_FOCUS);
                    }
                }
                None => {}
            }
            if focus == Some(0) {
                k.add(ck::FIRST_MATCH_FOCUS);
            }
        }
        k
    }

    fn text(&self, key: &str, args: &[(&str, String)]) -> String {
        let message = Message {
            key: key.into(),
            args: args
                .iter()
                .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
                .collect(),
        };
        self.strings
            .text(&message)
            .unwrap_or_else(|_| key.to_string())
    }

    /// 结果摘要 / 空状态 / 错误（UX_SPEC §6.3）。
    fn render_message(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let c = &self.colors;
        let line = |text: String, color: Option<Hsla>| div().text_color(paint(color)).child(text);
        let link = |id: &'static str,
                    text: String,
                    cx: &mut Context<Self>,
                    f: fn(&mut Self, &mut Context<Self>)| {
            div()
                .id(id)
                .cursor_pointer()
                .text_color(paint(c.link))
                .child(text)
                .on_click(cx.listener(move |v, _, _, cx| f(v, cx)))
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(12.0))
            .py(px(4.0));
        match &self.state {
            State::Idle => return None,
            State::InvalidPattern => {
                body = body.child(line(self.text("search.invalidRegex", &[]), c.error));
            }
            State::Failed => {
                body = body.child(line(self.text("search.noResults", &[]), c.info));
            }
            State::Searching if self.results.is_empty() => {
                body = body.child(line(self.text("search.searching", &[]), c.info));
            }
            state => {
                let summary = match state {
                    State::Done(s) => Some(s.clone()),
                    _ => None,
                };
                if self.results.is_empty() && matches!(state, State::Done(_)) {
                    body = body
                        .child(line(self.text("search.noResults", &[]), c.info))
                        .child(link(
                            "search-open-settings",
                            self.text("search.openSettings", &[]),
                            cx,
                            |_, cx| cx.emit(SearchViewEvent::OpenSettings),
                        ));
                    if summary
                        .as_ref()
                        .is_some_and(|s| s.excluded_by_rove_defaults)
                    {
                        body = body
                            .child(line(self.text("search.roveExcluded", &[]), c.info))
                            .child(link(
                                "search-without-excludes",
                                self.text("search.searchExcluded", &[]),
                                cx,
                                |v, cx| v.search_without_excludes(cx),
                            ));
                    }
                } else {
                    let matches = self.results.match_count().to_string();
                    let files = self.results.file_count();
                    let text = if files == 1 {
                        self.text("search.summary.one", &[("matches", matches)])
                    } else {
                        self.text(
                            "search.summary",
                            &[("matches", matches), ("files", files.to_string())],
                        )
                    };
                    body = body.child(line(text, c.info));
                    if summary.as_ref().is_some_and(|s| s.limit_hit) {
                        body = body.child(line(self.text("search.limitHit", &[]), c.info));
                    }
                    if *state == State::Cancelled {
                        body = body.child(line(self.text("search.cancelled", &[]), c.info));
                    }
                }
            }
        }
        Some(body.into_any_element())
    }

    /// “包含排除的文件再搜索一次”：关闭“使用排除设置”后重搜。
    pub fn search_without_excludes(&mut self, cx: &mut Context<Self>) {
        self.toggles.use_exclude_settings = false;
        self.search_now(cx);
    }

    fn toggle_button(
        &self,
        id: &'static str,
        icon: &str,
        on: bool,
        tip: String,
        command: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let c = &self.colors;
        let glyph = icons::codepoint(icon).unwrap_or('?');
        let hover = paint(c.option_hover);
        let mut el = div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(20.0))
            .h(px(20.0))
            .rounded(px(3.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_color(paint(if on { c.option_fg.or(c.fg) } else { c.fg }))
            .child(crate::shell::view::icon(glyph, px(16.0)))
            .tooltip(move |_, cx| cx.new(|_| Tip(tip.clone().into())).into())
            .on_click(cx.listener(move |v, _, window, cx| v.command(command, window, cx)));
        if on {
            el = el
                .bg(paint(c.option_bg))
                .border_color(paint(c.option_border));
        } else {
            el = el.hover(move |s| s.bg(hover));
        }
        el
    }

    fn tip(&self, label: &str, command: &str, cx: &App) -> String {
        let label = self.text(label, &[]);
        match crate::keys::key_label(command, cx) {
            Some(key) => self.text("search.tooltip.withKey", &[("label", label), ("key", key)]),
            None => label,
        }
    }

    fn input_box(
        &self,
        input: &Entity<TextInput>,
        invalid: bool,
        window: &Window,
        cx: &App,
    ) -> gpui::Div {
        let c = &self.colors;
        let focused = input.focus_handle(cx).is_focused(window);
        let border = if invalid {
            c.error_border
        } else if focused {
            c.focus_border
        } else {
            c.input_border
        };
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .h(px(24.0))
            .px(px(4.0))
            .bg(paint(if invalid {
                c.error_bg.or(c.input_bg)
            } else {
                c.input_bg
            }))
            .border_1()
            .border_color(paint(border))
            .child(div().flex_1().min_w_0().child(input.clone()))
    }
}

fn target_path(target: &Target) -> &WorkspacePath {
    match target {
        Target::File(p) | Target::Match(p, ..) => p,
    }
}

fn selection(line: u64, start: u32, end: u32) -> TextRange {
    let at = |column: u32| Position {
        line: Line(line),
        column: Utf8Column(u64::from(column)),
    };
    Span::new(at(start), at(end.max(start))).expect("ordered range")
}

/// 简单的悬停提示。
struct Tip(SharedString);
impl gpui::Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(6.0))
            .py(px(2.0))
            .bg(gpui::black())
            .text_color(gpui::white())
            .text_size(px(12.0))
            .child(self.0.clone())
    }
}

/// 结果行：文件行（展开箭头、文件名、目录、计数徽章）与匹配行（可选行号、预览、匹配高亮）。
fn row_renderer(
    rows: Rc<RefCell<Arc<ResultRows>>>,
    colors: Colors,
    view: ViewSettings,
) -> RowRenderer {
    Rc::new(move |ctx: &RowContext, _: &mut Window, _: &mut App| {
        let rows = rows.borrow();
        let target = parse_id(&ctx.row.id);
        let base = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .min_w_0()
            .overflow_hidden()
            .pl(px(ctx.indent));
        match target {
            Some(Target::File(path)) => {
                let Some(file) = rows.file(&path) else {
                    return base.into_any_element();
                };
                let chevron = if ctx.row.expanded {
                    "chevron-down"
                } else {
                    "chevron-right"
                };
                let glyph = icons::codepoint(chevron).unwrap_or(' ');
                let mut el = base
                    .child(crate::shell::view::icon(glyph, px(16.0)).flex_shrink_0())
                    .child(
                        div()
                            .flex_shrink_0()
                            .child(SharedString::from(file.name().to_string())),
                    );
                if !file.dir().is_empty() {
                    el = el.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_color(paint(colors.description))
                            .child(SharedString::from(file.dir().to_string())),
                    );
                } else {
                    el = el.child(div().flex_1());
                }
                if view.badges {
                    el = el.child(
                        div()
                            .flex_shrink_0()
                            .px(px(6.0))
                            .mr(px(8.0))
                            .rounded(px(9.0))
                            .text_size(px(11.0))
                            .bg(paint(colors.badge_bg))
                            .text_color(paint(colors.badge_fg))
                            .child(SharedString::from(file.count().to_string())),
                    );
                }
                el.into_any_element()
            }
            Some(Target::Match(path, line, range)) => {
                let Some(file) = rows.file(&path) else {
                    return base.into_any_element();
                };
                let Some(l) = file.lines.get(line) else {
                    return base.into_any_element();
                };
                let preview = SharedString::from(l.preview.to_string());
                let highlight = model::ResultRows::highlight(file, line, range).filter(|r| {
                    preview.is_char_boundary(r.start) && preview.is_char_boundary(r.end)
                });
                let styled = StyledText::new(preview).with_highlights(highlight.map(|r| {
                    (
                        r,
                        HighlightStyle {
                            background_color: colors.match_bg,
                            ..Default::default()
                        },
                    )
                }));
                let mut el = base.pl(px(ctx.indent + 20.0));
                if view.show_line_numbers {
                    el = el.child(
                        div()
                            .flex_shrink_0()
                            .text_color(paint(colors.description))
                            .child(SharedString::from(format!("{}:", l.line + 1))),
                    );
                }
                el.child(div().min_w_0().overflow_hidden().child(styled))
                    .into_any_element()
            }
            None => base.into_any_element(),
        }
    })
}

impl gpui::Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors.clone();
        let invalid = self.state == State::InvalidPattern;
        let toggles = self.toggles;
        let options = div()
            .flex()
            .flex_row()
            .gap(px(1.0))
            .child(self.toggle_button(
                "search-case",
                "case-sensitive",
                toggles.case_sensitive,
                self.tip("search.toggle.case", reg::TOGGLE_CASE, cx),
                reg::TOGGLE_CASE,
                cx,
            ))
            .child(self.toggle_button(
                "search-word",
                "whole-word",
                toggles.whole_word,
                self.tip("search.toggle.word", reg::TOGGLE_WORD, cx),
                reg::TOGGLE_WORD,
                cx,
            ))
            .child(self.toggle_button(
                "search-regex",
                "regex",
                toggles.regex,
                self.tip("search.toggle.regex", reg::TOGGLE_REGEX, cx),
                reg::TOGGLE_REGEX,
                cx,
            ));
        let query_row = div()
            .key_context(KeyContext::parse("inputBoxFocus searchInputBoxFocus").unwrap())
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .child(
                self.input_box(&self.query, invalid, window, cx)
                    .child(options),
            )
            .on_key_down(cx.listener(|v, e: &gpui::KeyDownEvent, _, cx| {
                let m = &e.keystroke.modifiers;
                if e.keystroke.key == "enter" && !m.control && !m.alt && !m.shift && !m.platform {
                    v.search_now(cx);
                    cx.stop_propagation();
                }
            }));
        let label = |text: String| {
            div()
                .text_size(px(11.0))
                .text_color(paint(c.fg))
                .child(text)
        };
        let glob_row = |input: &Entity<TextInput>, title: &str, this: &Self| {
            div()
                .key_context(KeyContext::parse("inputBoxFocus").unwrap())
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(label(this.text(title, &[])))
                .child(this.input_box(input, false, window, cx))
                .on_key_down(cx.listener(|v, e: &gpui::KeyDownEvent, _, cx| {
                    if e.keystroke.key == "enter" && !e.keystroke.modifiers.modified() {
                        v.search_now(cx);
                        cx.stop_propagation();
                    }
                }))
        };
        let use_excludes = self.toggles.use_exclude_settings;
        let exclude_toggle = div()
            .id("search-use-excludes")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .cursor_pointer()
            .text_size(px(11.0))
            .text_color(paint(c.fg))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(20.0))
                    .h(px(20.0))
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(paint(if use_excludes { c.option_border } else { None }))
                    .when(use_excludes, |d| d.bg(paint(c.option_bg)))
                    .child(crate::shell::view::icon(
                        icons::codepoint("exclude").unwrap_or('?'),
                        px(16.0),
                    )),
            )
            .child(self.text("search.useExcludeSettings", &[]))
            .on_click(cx.listener(|v, _, _, cx| {
                v.toggles.use_exclude_settings = !v.toggles.use_exclude_settings;
                v.search_now(cx);
            }));
        let progress = (self.state == State::Searching).then(|| {
            div()
                .id("search-progress")
                .h(px(2.0))
                .w_full()
                .flex_shrink_0()
                .bg(paint(c.progress))
        });
        let inputs = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(6.0))
            .px(px(12.0))
            .py(px(6.0))
            .child(query_row)
            .child(glob_row(&self.includes, "search.includes", self))
            .child(glob_row(&self.excludes, "search.excludes", self))
            .child(exclude_toggle);
        let list = div()
            .flex_1()
            .min_h_0()
            .child(self.list.clone())
            // 列表是展平的模型：左右方向键 / 切换展开在这里换成文件行的展开与折叠。
            .capture_action(
                cx.listener(|v, _: &crate::list_widget::keys::ListExpand, _, cx| {
                    v.expand_focused(Some(true), cx)
                }),
            )
            .capture_action(
                cx.listener(|v, _: &crate::list_widget::keys::ListCollapse, _, cx| {
                    v.expand_focused(Some(false), cx)
                }),
            )
            .capture_action(cx.listener(
                |v, _: &crate::list_widget::keys::ListToggleExpand, _, cx| {
                    v.expand_focused(None, cx)
                },
            ));
        let message = self.render_message(cx);
        let context = self.context(window, cx);
        div()
            .id("search-view")
            .key_context(context)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .text_color(paint(c.fg))
            .children(progress)
            .child(inputs)
            .children(message)
            .child(list)
    }
}

impl SearchView {
    /// 展平列表上的展开 / 折叠：文件行切换；匹配行上“折叠”把焦点移回它的文件行。
    fn expand_focused(&mut self, expanded: Option<bool>, cx: &mut Context<Self>) {
        match self.focused_target(cx) {
            Some(Target::File(path)) => {
                self.toggle_file(&path, expanded, cx);
                cx.stop_propagation();
            }
            Some(Target::Match(path, ..)) if expanded == Some(false) => {
                if let Some(i) = self.rows().file_row(&path) {
                    self.list.update(cx, |l, cx| {
                        l.widget.set_focus(Some(i));
                        l.widget.set_selection(vec![i]);
                        l.widget.reveal(i);
                        cx.notify();
                    });
                }
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}
