//! 查找条宿主（#21）：包住一个 [`CodeView`]，在它右上角浮出查找条，并把查找模型、后台搜索、
//! 匹配装饰、导航与历史接到一起。每个编辑器组的代码视图各有一个宿主；查找选项与上次的查找文本
//! 在窗口内共享（[`FindShared`]），所以切换文件、切换组后保持一致（与 VS Code 一致）。
//!
//! # 线程与帧预算
//!
//! 输入、点击只更新轻量状态并登记后台搜索；搜索在后台执行器里按块进行（可取消），结果经代次校验后
//! 回到界面。匹配装饰是代码视图的一个装饰层，只为可见行与预取行按需生成（见 [`Decor`]）。
//! 匹配列表完整时“上一个 / 下一个”在内存里二分，列表被截断（超过上限）或结果过期时才回到引擎。

use super::engine::{self, FindOptions, Match, MATCHES_LIMIT};
use super::registry::{self as reg, FindAction, FindColors};
use super::state::{
    plan_open, AutoFindInSelection, CountLabel, FindModel, OpenContext, SearchState, SeedMode,
};
use crate::code_view::{
    decorations::{priority, DecorationProvider, InlineDecoration, LineDecorations, TextStyle},
    view::hsla,
    CodeView, CodeViewEvent, LayerId, Reveal, Selection,
};
use crate::list_widget::history::InputHistory;
use crate::list_widget::text_input::{TextInput, TextInputColors, TextInputEvent};
use crate::shell::icons;
use gpui::{
    div, prelude::*, px, AnyView, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    IntoElement, KeyContext, MouseButton, Render, SharedString, Window,
};
use rove_base::{Cancellation, Position, Span, TextRange};
use rove_config::{Key, Scope, Settings};
use rove_i18n::{Message, Strings};
use rove_theme::{Theme, ThemeKind};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// 查找条宽度（VS Code `FIND_WIDGET_INITIAL_WIDTH`）。
const WIDTH: f32 = 419.0;
/// 查找条一行的高度（含上下内边距），也是 `editor.find.addExtraSpaceOnTop` 留出的空间。
const ROW_HEIGHT: f32 = 33.0;
const HISTORY_LIMIT: usize = 100;

/// `editor.find.*` 设置的当前值。
#[derive(Clone, Debug, PartialEq)]
pub struct FindSettings {
    pub find_on_type: bool,
    pub cursor_move_on_type: bool,
    pub loop_around: bool,
    pub close_on_result: bool,
    pub add_extra_space_on_top: bool,
    pub seed: SeedMode,
    pub auto_in_selection: AutoFindInSelection,
    pub history_workspace: bool,
    pub word_separators: Arc<str>,
}

impl FindSettings {
    pub fn read(settings: &dyn Settings) -> Self {
        let scope = Scope::default();
        let flag = |key: &'static str, default: bool| {
            Key::<bool>::new(key)
                .read(settings, &scope)
                .unwrap_or(default)
        };
        let text = |key: &'static str, default: &str| {
            Key::<String>::new(key)
                .read(settings, &scope)
                .unwrap_or_else(|_| default.to_string())
        };
        Self {
            find_on_type: flag("editor.find.findOnType", true),
            cursor_move_on_type: flag("editor.find.cursorMoveOnType", true),
            loop_around: flag("editor.find.loop", true),
            close_on_result: flag("editor.find.closeOnResult", false),
            add_extra_space_on_top: flag("editor.find.addExtraSpaceOnTop", true),
            seed: SeedMode::parse(&text("editor.find.seedSearchStringFromSelection", "always")),
            auto_in_selection: AutoFindInSelection::parse(&text(
                "editor.find.autoFindInSelection",
                "never",
            )),
            history_workspace: text("editor.find.history", "workspace") == "workspace",
            word_separators: text("editor.wordSeparators", "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?")
                .into(),
        }
    }
}

/// 窗口内所有查找条共享的状态：三个选项开关、上次的查找文本与查找历史。
pub struct FindShared {
    pub options: FindOptions,
    pub search: String,
    pub history: Vec<String>,
    history_path: Option<PathBuf>,
    loaded: bool,
}

pub type SharedFind = Rc<RefCell<FindShared>>;

impl FindShared {
    /// `history_path` 为 `None` 时历史只保存在内存里（测试、没有用户数据目录）。
    pub fn new(history_path: Option<PathBuf>) -> SharedFind {
        Rc::new(RefCell::new(Self {
            options: FindOptions::default(),
            search: String::new(),
            history: Vec::new(),
            history_path,
            loaded: false,
        }))
    }

    /// 首次打开查找条时读取历史文件（几百字节）。
    fn ensure_loaded(&mut self) {
        if std::mem::replace(&mut self.loaded, true) {
            return;
        }
        let Some(path) = &self.history_path else {
            return;
        };
        let parsed = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| {
                v.get("search")?.as_array().map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
            });
        if let Some(entries) = parsed {
            self.history = entries;
        }
    }

    /// 记一条历史（去重、移到末尾、截断）；返回需要写盘的内容。
    fn add_history(&mut self, search: &str) -> Option<(PathBuf, serde_json::Value)> {
        if search.is_empty() {
            return None;
        }
        self.history.retain(|e| e != search);
        self.history.push(search.to_string());
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
        let path = self.history_path.clone()?;
        Some((path, serde_json::json!({ "search": self.history })))
    }
}

/// 匹配装饰的数据：只存引用，不为每个匹配生成对象。
struct Decor {
    matches: Arc<Vec<Match>>,
    current: Option<usize>,
    scope: Option<TextRange>,
    colors: FindColors,
}

struct DecorProvider(Rc<RefCell<Decor>>);

impl DecorationProvider for DecorProvider {
    /// 只在某行进入视口 / 预取范围且缓存失效时被调用：二分定位本行的匹配，
    /// 所以 1 GB 文件上几十万个匹配也不会一次性生成装饰。
    fn decorate(&self, line: u64, text: &str, out: &mut LineDecorations) {
        let d = self.0.borrow();
        let c = &d.colors;
        let len = text.len() as u32;
        if let Some(scope) = &d.scope {
            let (s, e) = (scope.start(), scope.end());
            if (s.line.0..=e.line.0).contains(&line) {
                let from = if line == s.line.0 {
                    s.column.0 as u32
                } else {
                    0
                };
                let to = if line == e.line.0 {
                    e.column.0 as u32
                } else {
                    len
                };
                if from < to.min(len) {
                    out.inline.push(InlineDecoration {
                        range: from..to.min(len),
                        style: TextStyle {
                            background: c.range_background,
                            border: c.range_border,
                            ..Default::default()
                        },
                    });
                }
            }
        }
        let first = d.matches.partition_point(|m| m.end.line.0 < line);
        for (offset, m) in d.matches[first..].iter().enumerate() {
            if m.start.line.0 > line {
                break;
            }
            let from = if m.start.line.0 == line {
                m.start.column.0 as u32
            } else {
                0
            };
            let to = if m.end.line.0 == line {
                m.end.column.0 as u32
            } else {
                len
            };
            let (from, to) = (from.min(len), to.min(len));
            if from >= to {
                continue;
            }
            // 当前匹配与其他匹配用两套颜色键，不混用。
            let style = if d.current == Some(first + offset) {
                TextStyle {
                    background: c.match_background,
                    foreground: c.match_foreground,
                    border: c.match_border,
                    ..Default::default()
                }
            } else {
                TextStyle {
                    background: c.highlight_background,
                    foreground: c.highlight_foreground,
                    border: c.highlight_border,
                    ..Default::default()
                }
            };
            out.inline.push(InlineDecoration {
                range: from..to,
                style,
            });
        }
    }
}

/// 打开查找条的方式（`actions.find`、`actions.findWithSelection`、`f3` 在条关闭时）。
#[derive(Clone, Copy)]
struct OpenOptions {
    /// 把焦点交给查找输入框。
    focus_input: bool,
    /// 按 `autoFindInSelection` 更新搜索范围。
    update_scope: bool,
    seed: Seed,
}

/// 查找文本的种子来源。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seed {
    /// 按 `editor.find.seedSearchStringFromSelection`。
    Setting,
    /// 总是取选区 / 插入符所在单词（`actions.findWithSelection`）。
    Always,
    /// 不取种子，沿用已有的查找文本。
    Never,
}

/// 搜索结果回来之后要做的事。
#[derive(Clone, Copy, Debug, PartialEq)]
enum After {
    Nothing,
    /// 输入时光标跟随：从该位置起选中下一个匹配。
    MoveFrom(Position),
    /// 导航命令在结果过期时先重新搜索，再导航。
    Navigate {
        forward: bool,
    },
}

/// 查找条上要显示的校验 / 状态信息。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Problem {
    Regex(String),
    TooLarge,
}

pub struct FindHost {
    code: Entity<CodeView>,
    input: Entity<TextInput>,
    shared: SharedFind,
    settings: Arc<dyn Settings>,
    strings: Arc<dyn Strings>,
    colors: FindColors,
    cfg: FindSettings,
    model: FindModel,
    decor: Rc<RefCell<Decor>>,
    layer: Option<LayerId>,
    visible: bool,
    /// 搜索时文本的版本；版本变了匹配列表就过期。
    searched_version: Option<rove_base::Version>,
    after: After,
    problem: Option<Problem>,
    nav_cancel: Cancellation,
    pending_close: bool,
    pending_research: bool,
    /// 文档的大小级别（#8 分级）：超大文件只读，不提供替换（#22）。
    size: rove_buffer::SizeClass,
    /// 查找文本改了但还没搜索（`editor.find.findOnType` 关闭时等回车）。
    dirty: bool,
    _subs: Vec<gpui::Subscription>,
}

/// 查找条对外的事件（工作台据此维护状态栏等）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindEvent {
    /// 查找条显示 / 隐藏了。
    VisibilityChanged(bool),
}

impl EventEmitter<FindEvent> for FindHost {}

impl FindHost {
    pub fn new(
        code: Entity<CodeView>,
        shared: SharedFind,
        settings: Arc<dyn Settings>,
        strings: Arc<dyn Strings>,
        theme: &Arc<dyn Theme>,
        kind: ThemeKind,
        cx: &mut Context<Self>,
    ) -> Self {
        let colors = FindColors::resolve(theme.as_ref(), kind);
        let input = cx.new(|cx| TextInput::new(cx, input_colors(&colors)));
        let cfg = FindSettings::read(&*settings);
        let decor = Rc::new(RefCell::new(Decor {
            matches: Arc::new(Vec::new()),
            current: None,
            scope: None,
            colors: colors.clone(),
        }));
        let layer = code.update(cx, |code, cx| {
            code.add_decorations(
                "find.matches",
                priority::FIND_MATCH,
                Rc::new(DecorProvider(decor.clone())),
                cx,
            )
            .ok()
        });
        let mut subs = Vec::new();
        subs.push(cx.subscribe(&input, |this, _, event: &TextInputEvent, cx| {
            if let TextInputEvent::Changed { text, .. } = event {
                this.input_changed(text, cx);
            }
        }));
        subs.push(
            cx.subscribe(&code, |this, _, event: &CodeViewEvent, cx| match event {
                CodeViewEvent::SelectionChanged { selection, .. } => {
                    this.selection_changed(selection, cx)
                }
                CodeViewEvent::TextChanged => this.text_changed(cx),
                _ => {}
            }),
        );
        input.update(cx, |input, _| {
            input.set_placeholder(
                strings
                    .text(&Message::key("find.placeholder"))
                    .unwrap_or_default(),
            );
        });
        Self {
            code,
            input,
            shared,
            settings,
            strings,
            colors,
            model: FindModel::new(cfg.word_separators.clone()),
            cfg,
            decor,
            layer,
            visible: false,
            searched_version: None,
            after: After::Nothing,
            problem: None,
            nav_cancel: Cancellation::default(),
            pending_close: false,
            pending_research: false,
            size: rove_buffer::SizeClass::Normal,
            dirty: false,
            _subs: subs,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn model(&self) -> &FindModel {
        &self.model
    }

    /// 文档的大小级别（#22 在工作台装配时告知）。超大文件只读，查找条不提供替换。
    pub fn set_document_size(&mut self, size: rove_buffer::SizeClass, cx: &mut Context<Self>) {
        if self.size != size {
            self.size = size;
            cx.notify();
        }
    }

    pub fn document_size(&self) -> rove_buffer::SizeClass {
        self.size
    }

    /// 查找条是否提供替换：M1 本来就没有替换，#48 接入时**必须**遵守这个判断
    /// （超大文件上替换按钮不存在，UX_SPEC §3.2）。
    pub fn replace_allowed(&self) -> bool {
        crate::large_file::allows_replace(self.size)
    }

    /// 行索引又前进了一段（#22）：之前只扫描了已知的行，重新搜索把新索引出来的行也算进去。
    /// 下一帧生效（事件回调里没有窗口）。
    pub fn reindexed(&mut self, cx: &mut Context<Self>) {
        if self.visible {
            self.pending_research = true;
            cx.notify();
        }
    }

    pub fn code(&self) -> &Entity<CodeView> {
        &self.code
    }

    pub fn input_text(&self, cx: &App) -> String {
        self.input.read(cx).text().to_string()
    }

    pub fn input_focus(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }

    /// 查找条自己的 GPUI 动作入口，也供测试与 `rove-app` 直接按命令 ID 调用。
    pub fn execute(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match id {
            reg::FIND => self.open(
                OpenOptions {
                    focus_input: true,
                    update_scope: true,
                    seed: Seed::Setting,
                },
                window,
                cx,
            ),
            reg::FIND_WITH_SELECTION => self.open(
                OpenOptions {
                    focus_input: false,
                    update_scope: false,
                    seed: Seed::Always,
                },
                window,
                cx,
            ),
            reg::CLOSE => self.close(window, cx),
            reg::NEXT => self.navigate(true, window, cx),
            reg::PREVIOUS => self.navigate(false, window, cx),
            reg::NEXT_SELECTION => self.navigate_selection(true, window, cx),
            reg::PREVIOUS_SELECTION => self.navigate_selection(false, window, cx),
            reg::TOGGLE_CASE => self.toggle(window, cx, |o| o.case_sensitive = !o.case_sensitive),
            reg::TOGGLE_REGEX => self.toggle(window, cx, |o| o.regex = !o.regex),
            reg::TOGGLE_WHOLE_WORD => self.toggle(window, cx, |o| o.whole_word = !o.whole_word),
            reg::TOGGLE_IN_SELECTION => self.toggle_in_selection(cx),
            _ => return false,
        }
        true
    }

    /// 以给定的查找文本打开查找条，不取选区种子也不抢焦点（基准、嵌入方使用）。
    pub fn show(&mut self, search: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.shared.borrow_mut().search = search.to_string();
        self.open(
            OpenOptions {
                focus_input: false,
                update_scope: false,
                seed: Seed::Never,
            },
            window,
            cx,
        );
    }

    /// 主题变了：重算颜色并让装饰失效。
    pub fn set_theme(&mut self, theme: &Arc<dyn Theme>, kind: ThemeKind, cx: &mut Context<Self>) {
        self.colors = FindColors::resolve(theme.as_ref(), kind);
        self.decor.borrow_mut().colors = self.colors.clone();
        let colors = input_colors(&self.colors);
        self.input.update(cx, |input, cx| {
            input.colors = colors;
            cx.notify();
        });
        self.refresh_decorations(cx);
    }

    /// 设置热加载。
    pub fn reload_settings(&mut self, cx: &mut Context<Self>) {
        self.cfg = FindSettings::read(&*self.settings);
        self.model
            .set_word_separators(self.cfg.word_separators.clone());
        self.apply_top_inset(cx);
        if self.visible {
            self.research(After::Nothing, cx);
        }
    }

    // ------------------------------------------------------------------ 打开与关闭

    fn open(&mut self, opts: OpenOptions, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg = FindSettings::read(&*self.settings);
        self.model
            .set_word_separators(self.cfg.word_separators.clone());
        let was_visible = self.visible;
        let (selection, selected, word) = {
            let code = self.code.read(cx);
            let selection = code.selection();
            let single_line = selection.start().line == selection.end().line;
            (
                selection,
                (single_line && !selection.is_empty())
                    .then(|| code.selected_text())
                    .flatten(),
                code.word_at_caret(),
            )
        };
        let range = Span::new(selection.start(), selection.end()).expect("selection is ordered");
        let seed_mode = match opts.seed {
            Seed::Always => SeedMode::Always,
            Seed::Never => SeedMode::Never,
            Seed::Setting => self.cfg.seed,
        };
        let plan = plan_open(
            seed_mode,
            self.cfg.auto_in_selection,
            &OpenContext {
                selection: range,
                selected_text: selected.as_deref(),
                word: word.as_deref(),
            },
        );
        {
            let mut shared = self.shared.borrow_mut();
            shared.ensure_loaded();
            self.model.set_options(shared.options);
        }
        // 种子：正则模式下转义，保证按字面查找。
        let search = match plan.seed {
            Some(seed) if self.model.options().regex => escape_regex(&seed),
            Some(seed) => seed,
            None if was_visible => self.input.read(cx).text().to_string(),
            None => self.shared.borrow().search.clone(),
        };
        self.model.set_search(&search);
        if opts.update_scope {
            if let Some(scope) = plan.scope {
                self.model.set_in_selection(Some(scope));
            }
        }
        // 输入框与历史。
        let history = self.cfg.history_workspace.then(|| {
            let mut h = InputHistory::new(HISTORY_LIMIT);
            for entry in &self.shared.borrow().history {
                h.add(entry);
            }
            h
        });
        self.input.update(cx, |input, cx| {
            input.history = history;
            input.set_text(&search, cx);
            if opts.focus_input {
                input.select_all(cx);
            }
        });
        self.shared.borrow_mut().search = search;
        self.set_visible(true, cx);
        self.apply_top_inset(cx);
        if opts.focus_input {
            window.focus(&self.input.read(cx).focus_handle(cx));
        }
        self.research(After::Nothing, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.record_history(cx);
        let input_focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        self.visible = false;
        self.pending_close = false;
        self.after = After::Nothing;
        self.nav_cancel.cancel();
        // 关闭时范围一并清除（VS Code 同）：下次打开按 autoFindInSelection 重新决定。
        self.model.set_in_selection(None);
        self.model.clear();
        self.problem = None;
        self.searched_version = None;
        self.sync_decor(cx);
        self.apply_top_inset(cx);
        cx.emit(FindEvent::VisibilityChanged(false));
        if input_focused {
            let focus = self.code.read(cx).focus_handle(cx);
            window.focus(&focus);
        }
        cx.notify();
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible != visible {
            self.visible = visible;
            cx.emit(FindEvent::VisibilityChanged(visible));
        }
        cx.notify();
    }

    fn apply_top_inset(&mut self, cx: &mut Context<Self>) {
        let inset = if self.visible && self.cfg.add_extra_space_on_top {
            ROW_HEIGHT
        } else {
            0.0
        };
        self.code
            .update(cx, |code, cx| code.set_top_inset(inset, cx));
    }

    // ------------------------------------------------------------------ 搜索

    fn input_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.model.set_search(text);
        self.shared.borrow_mut().search = text.to_string();
        if !self.cfg.find_on_type {
            // 不输入即查找：等按下回车。
            self.dirty = true;
            return;
        }
        let after = if self.cfg.cursor_move_on_type {
            After::MoveFrom(self.code.read(cx).selection().start())
        } else {
            After::Nothing
        };
        self.research(after, cx);
    }

    fn selection_changed(&mut self, selection: &Selection, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.model
            .set_current_from_selection(Some((selection.start(), selection.end())));
        self.sync_decor(cx);
        cx.notify();
    }

    fn text_changed(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        // 换了文档或快照：范围作废，匹配重新计算（渲染时在窗口里执行）。
        self.model.set_in_selection(None);
        self.pending_research = true;
        cx.notify();
    }

    /// 取消上一次搜索，按当前条件在后台重新搜索；结果回来后做 `after`。
    fn research(&mut self, after: After, cx: &mut Context<Self>) {
        self.after = after;
        self.dirty = false;
        self.problem = None;
        let Some(text) = self.code.read(cx).text().cloned() else {
            self.model.clear();
            self.sync_decor(cx);
            return;
        };
        let version = text.version();
        let Some(job) = self.model.start_search() else {
            // 没有查找文本或正则非法。
            self.problem = self
                .model
                .error()
                .map(|engine::QueryError::InvalidRegex(reason)| Problem::Regex(reason.clone()));
            self.searched_version = Some(version);
            self.sync_decor(cx);
            cx.notify();
            return;
        };
        let generation = job.generation;
        let (tx, rx) = async_channel::unbounded();
        let source = text.clone();
        cx.background_executor()
            .spawn(async move {
                let progress_tx = tx.clone();
                let result = job.run(&*source, &mut |p, batch| {
                    let _ = progress_tx.try_send(SearchMessage::Progress(p, batch.to_vec()));
                });
                let _ = tx.try_send(SearchMessage::Done(result));
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(message) = rx.recv().await {
                let done = matches!(message, SearchMessage::Done(_));
                let alive = this
                    .update(cx, |host, cx| {
                        host.search_message(generation, version, message, cx)
                    })
                    .is_ok();
                if done || !alive {
                    break;
                }
            }
        })
        .detach();
        self.sync_decor(cx);
        cx.notify();
    }

    fn search_message(
        &mut self,
        generation: u64,
        version: rove_base::Version,
        message: SearchMessage,
        cx: &mut Context<Self>,
    ) {
        match message {
            SearchMessage::Progress(p, batch) => {
                // #22：流式查找——先接上新找到的匹配，再更新“已找到 N 处 · 已搜索 X%”。
                let published = self.model.publish_partial(generation, &batch);
                let advanced = self.model.progress(generation, p);
                if published {
                    let selection = self.code.read(cx).selection();
                    self.model
                        .set_current_from_selection(Some((selection.start(), selection.end())));
                    // 装饰仍然只为可见行生成（#19 按行调用提供方），不会一次性铺满全部匹配。
                    self.sync_decor(cx);
                }
                if published || advanced {
                    cx.notify();
                }
            }
            SearchMessage::Done(result) => {
                let too_large = matches!(result, Err(rove_base::Error::Unsupported));
                if !self.model.finish(generation, result) {
                    return;
                }
                self.searched_version = Some(version);
                self.problem = too_large.then_some(Problem::TooLarge);
                let selection = self.code.read(cx).selection();
                self.model
                    .set_current_from_selection(Some((selection.start(), selection.end())));
                self.sync_decor(cx);
                match std::mem::replace(&mut self.after, After::Nothing) {
                    After::Nothing => {}
                    After::MoveFrom(from) => self.step(from, true, cx),
                    After::Navigate { forward } => {
                        let sel = self.code.read(cx).selection();
                        let from = if forward { sel.end() } else { sel.start() };
                        self.step(from, forward, cx);
                    }
                }
                cx.notify();
            }
        }
    }

    // ------------------------------------------------------------------ 导航

    fn navigate(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            // `f3` 在条关闭时打开它（不抢焦点），再导航。
            self.open(
                OpenOptions {
                    focus_input: false,
                    update_scope: false,
                    seed: Seed::Setting,
                },
                window,
                cx,
            );
            if self.model.search().is_empty() {
                return;
            }
        }
        self.record_history(cx);
        let typed = self.input.read(cx).text().to_string();
        if self.dirty || typed != self.model.search() || self.stale(cx) {
            // 没有开 `findOnType`，或文本换了：先搜索，再导航。
            self.model.set_search(&typed);
            self.research(After::Navigate { forward }, cx);
            return;
        }
        let sel = self.code.read(cx).selection();
        let from = if forward { sel.end() } else { sel.start() };
        self.step(from, forward, cx);
    }

    /// `ctrl+f3` / `ctrl+shift+f3`：以当前选区（或插入符所在单词）为查找文本，找下一个 / 上一个。
    fn navigate_selection(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (text, sel) = {
            let code = self.code.read(cx);
            let sel = code.selection();
            let text = if sel.is_empty() {
                code.word_at_caret()
            } else if sel.start().line == sel.end().line {
                code.selected_text()
            } else {
                None
            };
            (text, sel)
        };
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return;
        };
        let was_visible = self.visible;
        if !was_visible {
            self.open(
                OpenOptions {
                    focus_input: false,
                    update_scope: false,
                    seed: Seed::Setting,
                },
                window,
                cx,
            );
        }
        let search = if self.model.options().regex {
            escape_regex(&text)
        } else {
            text
        };
        self.model.set_search(&search);
        self.shared.borrow_mut().search = search.clone();
        self.input
            .update(cx, |input, cx| input.set_text(&search, cx));
        self.record_history(cx);
        self.research(After::Navigate { forward }, cx);
        let _ = sel;
    }

    /// 匹配列表是否可能与文本不符（文本换了版本，或还没搜过）。
    fn stale(&self, cx: &App) -> bool {
        let version = self.code.read(cx).text().map(|t| t.version());
        version != self.searched_version
    }

    /// 从 `from` 起选中下一个 / 上一个匹配。列表完整时在内存里二分，否则回到引擎在后台查找。
    fn step(&mut self, from: Position, forward: bool, cx: &mut Context<Self>) {
        if self.model.searcher().is_none() {
            return;
        }
        let complete = !self.model.limit_exceeded()
            && matches!(self.model.state(), SearchState::Idle)
            && !self.stale(cx);
        if complete {
            let matches = self.model.matches().clone();
            if matches.is_empty() {
                return;
            }
            let step = in_memory_step(&matches, from, forward, self.cfg.loop_around);
            if let Some(m) = step {
                self.select(m, cx);
            }
            return;
        }
        let (Some(text), Some(searcher)) = (
            self.code.read(cx).text().cloned(),
            self.model.searcher().cloned(),
        ) else {
            return;
        };
        self.nav_cancel.cancel();
        self.nav_cancel = Cancellation::default();
        let cancel = self.nav_cancel.clone();
        let scope = self.model.scope().cloned();
        let wrap = self.cfg.loop_around;
        let task = cx.background_executor().spawn(async move {
            engine::find_next(
                &*text,
                &searcher,
                scope.as_ref(),
                from,
                forward,
                wrap,
                &cancel,
            )
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Some(step)) = task.await {
                let _ = this.update(cx, |host, cx| host.select(step.found, cx));
            }
        })
        .detach();
    }

    /// 选中一个匹配并居中显示（#19 的滚动定位接口）。
    fn select(&mut self, m: Match, cx: &mut Context<Self>) {
        self.code.update(cx, |code, cx| {
            code.set_selection(
                Selection::new(m.start, m.end),
                Some(Reveal::CenterIfOutside),
                cx,
            )
        });
        self.model
            .set_current_from_selection(Some((m.start, m.end)));
        self.sync_decor(cx);
        if self.cfg.close_on_result {
            self.pending_close = true;
        }
        cx.notify();
    }

    // ------------------------------------------------------------------ 选项

    fn toggle(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut FindOptions),
    ) {
        let mut options = self.shared.borrow().options;
        change(&mut options);
        self.shared.borrow_mut().options = options;
        self.model.set_options(options);
        if self.visible {
            let from = self.code.read(cx).selection().start();
            self.research(After::MoveFrom(from), cx);
        }
        cx.notify();
    }

    fn toggle_in_selection(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        if self.model.in_selection() {
            self.model.set_in_selection(None);
        } else {
            let sel = self.code.read(cx).selection();
            if sel.is_empty() {
                return;
            }
            self.model
                .set_in_selection(Some(Span::new(sel.start(), sel.end()).expect("ordered")));
        }
        self.research(After::Nothing, cx);
    }

    // ------------------------------------------------------------------ 历史、装饰

    fn record_history(&mut self, cx: &mut Context<Self>) {
        if !self.cfg.history_workspace {
            return;
        }
        let text = self.model.search().to_string();
        self.input.update(cx, |input, _| {
            if let Some(h) = input.history.as_mut() {
                h.add(&text);
            }
        });
        let write = self.shared.borrow_mut().add_history(&text);
        if let Some((path, value)) = write {
            cx.background_executor()
                .spawn(async move {
                    if let Err(e) = crate::shell::layout::write_json(&path, &value) {
                        eprintln!("find history: {e}");
                    }
                })
                .detach();
        }
    }

    /// 把模型的匹配 / 当前项 / 范围交给装饰层并让它按需重算。
    fn sync_decor(&mut self, cx: &mut Context<Self>) {
        {
            let mut d = self.decor.borrow_mut();
            d.matches = self.model.matches().clone();
            d.current = self.model.current();
            d.scope = self.model.scope().cloned();
        }
        self.refresh_decorations(cx);
    }

    fn refresh_decorations(&mut self, cx: &mut Context<Self>) {
        if let Some(layer) = self.layer.clone() {
            self.code
                .update(cx, |code, cx| code.invalidate_decorations(&layer, None, cx));
        }
    }

    // ------------------------------------------------------------------ 绘制

    fn count_text(&self) -> String {
        let text = |key: &str, args: &[(&str, String)]| {
            let message = Message {
                key: key.into(),
                args: args
                    .iter()
                    .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
                    .collect::<Vec<_>>()
                    .into(),
            };
            self.strings.text(&message).unwrap_or_default()
        };
        match self.model.count_label() {
            CountLabel::NoResults => text("find.count.none", &[]),
            CountLabel::Searching { found, percent } => match percent {
                Some(p) => text(
                    "find.count.searching",
                    &[("found", found.to_string()), ("percent", p.to_string())],
                ),
                None => text("find.count.found", &[("found", found.to_string())]),
            },
            CountLabel::Matches {
                over_limit: true, ..
            } => text(
                "find.count.overLimit",
                &[("limit", MATCHES_LIMIT.to_string())],
            ),
            CountLabel::Matches {
                index: Some(index),
                total,
                ..
            } => text(
                "find.count.of",
                &[("index", index.to_string()), ("total", total.to_string())],
            ),
            CountLabel::Matches { total, .. } => {
                text("find.count.total", &[("total", total.to_string())])
            }
        }
    }

    fn problem_text(&self) -> Option<String> {
        let message = |key: &str, args: Vec<(Arc<str>, Arc<str>)>| {
            self.strings
                .text(&Message {
                    key: key.into(),
                    args: args.into(),
                })
                .unwrap_or_default()
        };
        self.problem.as_ref().map(|p| match p {
            Problem::Regex(reason) => message(
                "find.error.regex",
                vec![(Arc::from("reason"), Arc::from(reason.as_str()))],
            ),
            Problem::TooLarge => message("find.error.tooLarge", Vec::new()),
        })
    }

    fn tooltip(&self, label_key: &str, command: &str, cx: &App) -> String {
        let label = self
            .strings
            .text(&Message::key(label_key))
            .unwrap_or_default();
        match crate::keys::key_label(command, cx) {
            Some(key) => self
                .strings
                .text(&Message {
                    key: "find.tooltip.withKey".into(),
                    args: vec![
                        (Arc::from("label"), Arc::from(label.as_str())),
                        (Arc::from("key"), Arc::from(key.as_str())),
                    ]
                    .into(),
                })
                .unwrap_or(label),
            None => label,
        }
    }

    fn render_widget(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.colors.clone();
        let paint =
            |c: Option<rove_theme::Color>| c.map(hsla).unwrap_or_else(gpui::transparent_black);
        let options = self.model.options();
        let in_selection = self.model.in_selection();
        let can_step = self.model.has_matches();
        let error = self.problem.is_some();
        let toggle = |id: &'static str,
                      icon: &str,
                      on: bool,
                      tip: String,
                      command: &'static str,
                      cx: &mut Context<Self>| {
            let glyph = icons::codepoint(icon).unwrap_or('?');
            let (active_bg, active_border, active_fg, hover) = (
                paint(c.option_active_background),
                paint(c.option_active_border),
                paint(c.option_active_foreground),
                paint(c.option_hover_background),
            );
            let fg = paint(c.widget_foreground);
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
                .text_color(if on { active_fg } else { fg })
                .child(crate::shell::view::icon(glyph, px(16.0)))
                .tooltip(simple_tooltip(tip, &c));
            if on {
                el = el.bg(active_bg).border_color(active_border);
            } else {
                el = el.hover(move |s| s.bg(hover));
            }
            el.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |host, _, window, cx| {
                    host.execute(command, window, cx);
                    cx.stop_propagation();
                }),
            )
        };
        let button = |id: &'static str,
                      icon: &str,
                      enabled: bool,
                      tip: String,
                      command: &'static str,
                      cx: &mut Context<Self>| {
            let glyph = icons::codepoint(icon).unwrap_or('?');
            let hover = paint(c.hover_background.or(c.option_hover_background));
            let fg = paint(c.widget_foreground);
            let mut el = div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .w(px(22.0))
                .h(px(22.0))
                .rounded(px(5.0))
                .text_color(fg)
                .child(crate::shell::view::icon(glyph, px(16.0)))
                .tooltip(simple_tooltip(tip, &c));
            if enabled {
                el = el.cursor_pointer().hover(move |s| s.bg(hover));
            } else {
                el = el.opacity(0.4);
            }
            el.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |host, _, window, cx| {
                    if enabled {
                        host.execute(command, window, cx);
                    }
                    cx.stop_propagation();
                }),
            )
        };
        let tip = |this: &Self, key: &str, command: &str, cx: &App| this.tooltip(key, command, cx);
        let toggles = div()
            .flex()
            .items_center()
            .gap(px(1.0))
            .pr(px(2.0))
            .child(toggle(
                "find-case",
                "case-sensitive",
                options.case_sensitive,
                tip(self, "find.toggle.case", reg::TOGGLE_CASE, cx),
                reg::TOGGLE_CASE,
                cx,
            ))
            .child(toggle(
                "find-word",
                "whole-word",
                options.whole_word,
                tip(self, "find.toggle.wholeWord", reg::TOGGLE_WHOLE_WORD, cx),
                reg::TOGGLE_WHOLE_WORD,
                cx,
            ))
            .child(toggle(
                "find-regex",
                "regex",
                options.regex,
                tip(self, "find.toggle.regex", reg::TOGGLE_REGEX, cx),
                reg::TOGGLE_REGEX,
                cx,
            ));
        let input_box = div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .h(px(24.0))
            .px(px(4.0))
            .bg(paint(c.input_background))
            .border_1()
            .border_color(if error {
                paint(c.error_border)
            } else {
                paint(c.input_border)
            })
            .text_size(px(13.0))
            .text_color(paint(c.input_foreground))
            .child(self.input.clone())
            .child(toggles);
        let count = div()
            .flex_shrink_0()
            .min_w(px(69.0))
            .px(px(4.0))
            .text_size(px(12.0))
            .text_color(paint(c.widget_foreground))
            .child(self.count_text());
        let row = div()
            .flex()
            .items_center()
            .gap(px(3.0))
            .h(px(ROW_HEIGHT - 8.0))
            .child(input_box)
            .child(count)
            .child(toggle(
                "find-in-selection",
                "list-selection",
                in_selection,
                tip(
                    self,
                    "find.toggle.inSelection",
                    reg::TOGGLE_IN_SELECTION,
                    cx,
                ),
                reg::TOGGLE_IN_SELECTION,
                cx,
            ))
            .child(button(
                "find-previous",
                "arrow-up",
                can_step,
                tip(self, "find.button.previous", reg::PREVIOUS, cx),
                reg::PREVIOUS,
                cx,
            ))
            .child(button(
                "find-next",
                "arrow-down",
                can_step,
                tip(self, "find.button.next", reg::NEXT, cx),
                reg::NEXT,
                cx,
            ))
            .child(button(
                "find-close",
                "close",
                true,
                tip(self, "find.button.close", reg::CLOSE, cx),
                reg::CLOSE,
                cx,
            ));
        let mut context = KeyContext::default();
        context.add("FindWidget");
        context.add("editorFocus");
        context.add(reg::context::WIDGET_VISIBLE);
        context.add(reg::context::INPUT_FOCUSSED);
        let mut widget = div()
            .id("find-widget")
            .occlude()
            .key_context(context)
            .absolute()
            .top(px(0.0))
            .right(px(18.0))
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .p(px(4.0))
            .bg(paint(c.widget_background))
            .text_color(paint(c.widget_foreground))
            .border_1()
            .border_color(paint(c.widget_border))
            .shadow_md()
            .child(row);
        if let Some(message) = self.problem_text() {
            widget = widget.child(
                div()
                    .mt(px(2.0))
                    .px(px(6.0))
                    .py(px(2.0))
                    .text_size(px(12.0))
                    .bg(paint(c.error_background))
                    .border_1()
                    .border_color(paint(c.error_border))
                    .text_color(paint(c.error_foreground))
                    .child(message),
            );
        }
        widget
    }
}

fn input_colors(colors: &FindColors) -> TextInputColors {
    let paint = |c: Option<rove_theme::Color>, fallback: Hsla| c.map(hsla).unwrap_or(fallback);
    let fg = paint(colors.input_foreground, gpui::white());
    TextInputColors {
        text: fg,
        placeholder: paint(colors.input_placeholder, fg),
        caret: fg,
        selection: paint(colors.focus_border, fg).opacity(0.4),
    }
}

/// 一次后台搜索回报给界面的消息。
enum SearchMessage {
    /// 进度 + 这一块新找到的匹配（#22 的流式查找：结果逐步出现）。
    Progress(engine::Progress, Vec<engine::Match>),
    Done(rove_base::Result<engine::SearchOutput>),
}

/// 正则元字符转义（正则模式下用选区做种子时按字面查找）。
fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for c in text.chars() {
        if "\\^$.*+?()[]{}|/-".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// 在完整的匹配列表里导航：下一个 = 起点 ≥ `from` 的第一个，上一个 = 起点 < `from` 的最后一个；
/// 到头时按 `wrap` 循环。范围内查找时列表本身已限制在范围内。
pub fn in_memory_step(
    matches: &[Match],
    from: Position,
    forward: bool,
    wrap: bool,
) -> Option<Match> {
    if matches.is_empty() {
        return None;
    }
    if forward {
        let i = matches.partition_point(|m| m.start < from);
        match matches.get(i) {
            Some(m) => Some(*m),
            None if wrap => matches.first().copied(),
            None => None,
        }
    } else {
        let i = matches.partition_point(|m| m.start < from);
        match i.checked_sub(1) {
            Some(i) => Some(matches[i]),
            None if wrap => matches.last().copied(),
            None => None,
        }
    }
}

struct SimpleTooltip {
    text: SharedString,
    bg: Hsla,
    fg: Hsla,
    border: Hsla,
}

impl Render for SimpleTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(self.bg)
            .text_color(self.fg)
            .border_1()
            .border_color(self.border)
            .text_size(px(12.0))
            .child(self.text.clone())
    }
}

fn simple_tooltip(
    text: String,
    c: &FindColors,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let paint = |c: Option<rove_theme::Color>| c.map(hsla).unwrap_or_else(gpui::transparent_black);
    let (bg, fg, border) = (
        paint(c.widget_background),
        paint(c.widget_foreground),
        paint(c.widget_border),
    );
    let text: SharedString = text.into();
    move |_, cx| {
        cx.new(|_| SimpleTooltip {
            text: text.clone(),
            bg,
            fg,
            border,
        })
        .into()
    }
}

impl Render for FindHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 事件回调里没有窗口：需要窗口的收尾（重新搜索、`closeOnResult`）在这里完成。
        if std::mem::take(&mut self.pending_research) && self.visible {
            let typed = self.model.search().to_string();
            self.model.set_search(&typed);
            self.research(After::Nothing, cx);
        }
        if std::mem::take(&mut self.pending_close) {
            self.close(window, cx);
        }
        let mut root = div()
            .relative()
            .size_full()
            .on_action(cx.listener(|host, action: &FindAction, window, cx| {
                host.execute(action.0, window, cx);
            }))
            .child(self.code.clone());
        if self.visible {
            let mut context = KeyContext::default();
            context.add(reg::context::WIDGET_VISIBLE);
            root = root.key_context(context).child(self.render_widget(cx));
        }
        root
    }
}
