use super::{
    appearance,
    commands::Commands,
    editor::{Pending, SingleEditor},
    text::{default_settings, FileDocuments},
};
use crate::{
    documents::{OpenDocuments, OpenOptions, OpenSource},
    list_widget::{
        options::ListOptions,
        tree::{ChildCount, Children, TreeItem, TreeModel, TreeSource},
        widget::{ListWidget, RowHeight},
    },
    lists::{ItemId, ListController, ListModel, Row},
    shell::{
        self,
        catalog::{CONTAINER_FACTORY, FILE_VIEW, PROBLEMS, STATUS_FACTORY},
        feedback::Reason,
        layout::LayoutStore,
        status::StatusEntry,
        Shell, ShellHost,
    },
};
use gpui::{
    actions, div, point, prelude::*, px, relative, rgba, size, uniform_list, AnyElement, App,
    Application, Bounds, ContentMask, Context, Element, ElementId, Entity, FocusHandle, Focusable,
    Font, FontFeatures, FontStyle, FontWeight, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, KeyBinding, LayoutId, Pixels, Render, ScrollDelta, ScrollWheelEvent, ShapedLine,
    SharedString, Style, TextRun, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use rove_base::{
    Cancellation, DocumentId, Error, Line, Resource, Result as CoreResult, Value, WorkspacePath,
};
use rove_commands::{CommandExecutor, CommandRegistry, Invocation};
use rove_config::{Key, Scope};
use rove_i18n::{Message, StringTable, Strings};
use rove_theme::{Color, Theme};
use rove_workspace::{EntryKind, Workspace};
use std::{borrow::Cow, path::PathBuf, sync::Arc, time::Instant};
const LINE_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 14.0;
fn color(c: Color) -> Hsla {
    rgba(((c.0 as u32) << 24) | ((c.1 as u32) << 16) | ((c.2 as u32) << 8) | c.3 as u32).into()
}

// SKELETON(explorer): fixed expandable file tree; replaced by #23.
// Tree data/control use the #18 list component (lazy children, stable ItemId); only this
// adapter and the fixed rendering below remain skeleton.
struct WorkspaceTree(Arc<dyn Workspace>);
impl TreeSource for WorkspaceTree {
    fn children(&self, parent: Option<&ItemId>) -> Children {
        let path = parent.and_then(|p| WorkspacePath::new(&p.0).ok());
        match self.0.children(path.as_ref(), 0..usize::MAX) {
            Ok(page) => Children::Ready(
                page.items
                    .iter()
                    .map(|e| TreeItem {
                        id: ItemId(e.path.as_str().into()),
                        label: e.path.as_str().rsplit('/').next().unwrap().into(),
                        detail: None,
                        children: if matches!(e.kind, EntryKind::Directory) {
                            ChildCount::Unknown
                        } else {
                            ChildCount::None
                        },
                    })
                    .collect(),
            ),
            Err(e) => {
                eprintln!("workspace: {e}");
                Children::Ready(Arc::from([]))
            }
        }
    }
}
type FileTree = ListWidget;
fn file_tree(workspace: Arc<dyn Workspace>) -> CoreResult<FileTree> {
    // Surface a failing root listing as a scan failure, as before.
    workspace.children(None, 0..1)?;
    Ok(ListWidget::tree(
        TreeModel::new(Arc::new(WorkspaceTree(workspace))),
        RowHeight::Fixed(24.0),
        ListOptions::default(),
    ))
}

#[cfg(test)]
#[derive(Clone)]
struct ColoredSpan {
    range: std::ops::Range<usize>,
    scope: Arc<str>,
    color: Color,
}
#[derive(Clone)]
struct PreparedLine {
    number: usize,
    #[cfg(test)]
    text: String,
    #[cfg(test)]
    spans: Vec<ColoredSpan>,
    shape: ShapedLine,
    gutter: ShapedLine,
}
// SKELETON(commands): temporary GPUI action bridge and hardcoded keys, replaced by #14.
actions!(skeleton_actions, [QuickOpen, ShowCommands]);

/// 骨架窗口：工作台（标题栏、活动栏、侧边栏、面板、状态栏、反馈）由 #16 的 `shell` 提供，
/// 这里装配骨架的文件树、单编辑区与代码视图。
pub struct Workbench {
    tree: Option<FileTree>,
    editor: SingleEditor,
    theme: Arc<dyn Theme>,
    strings: StringTable,
    shell: Shell,
    commands: Arc<Commands>,
    focus: FocusHandle,
    /// 工作区扫描次数（缩放等界面操作不得触发重新扫描）。
    scans: u32,
    scan_failed: bool,
    error_path: Option<String>,
    scroll: f64,
    prepared: Vec<PreparedLine>,
    painted: Vec<PreparedLine>,
    dirty: bool,
    preparing: bool,
    viewport_height: f32,
    started: Instant,
    first_frame: bool,
}
impl Workbench {
    /// 布局按工作区持久化到用户数据目录；单元测试不写用户目录。
    pub fn new(root: PathBuf, started: Instant, cx: &mut Context<Self>) -> Self {
        let store = if cfg!(test) {
            LayoutStore::disabled()
        } else {
            LayoutStore::for_workspace(&root)
        };
        Self::with_store(root, started, store, cx)
    }
    pub fn with_store(
        root: PathBuf,
        started: Instant,
        store: LayoutStore,
        cx: &mut Context<Self>,
    ) -> Self {
        let owner = cx.entity().downgrade();
        let app = cx.to_async();
        let background = cx.background_executor().clone();
        let schedule = move |job: Pending| {
            let Pending { id, resource, task } = job;
            let read = background.spawn(task);
            let owner = owner.clone();
            app.spawn(async move |cx| {
                let result = read.await;
                let _ = owner.update(cx, |view, cx| {
                    match view.editor.complete(id, result) {
                        Ok(true) => {
                            view.scroll = view
                                .editor
                                .current
                                .as_ref()
                                .and_then(|opened| opened.position)
                                .map_or(0.0, |position| position.line.0 as f64);
                            view.editor.ready(id);
                            view.error_path = None;
                            view.dirty = true;
                            view.prepared.clear();
                        }
                        Ok(false) => return,
                        Err(Error::Cancelled) => {}
                        Err(error) => {
                            eprintln!("open {resource:?}: {error}");
                            if view.editor.current.is_none() {
                                let path = match &resource {
                                    Resource::Workspace(path) => path.as_str().to_owned(),
                                    Resource::File(path) => path.display().to_string(),
                                    Resource::Virtual(name) => name.to_string(),
                                };
                                view.error_path = Some(path);
                                view.prepared.clear();
                                view.painted.clear();
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        };
        let shell = Shell::new(
            &root,
            appearance::contributions().unwrap(),
            shell::view::default_settings(),
            store,
            cx,
        );
        let mut view = Self {
            tree: None,
            editor: SingleEditor::new(
                Arc::new(FileDocuments::new(root.clone(), default_settings())),
                rove_syntax::highlight::TreeSitterSyntax::default(),
                schedule,
            ),
            theme: rove_theme::ThemeService::new(default_settings()),
            strings: appearance::strings().unwrap(),
            shell,
            commands: Arc::new(Commands::new().unwrap()),
            focus: cx.focus_handle(),
            scans: 0,
            scan_failed: false,
            error_path: None,
            scroll: 0.0,
            prepared: vec![],
            painted: vec![],
            dirty: true,
            preparing: false,
            viewport_height: 0.0,
            started,
            first_frame: false,
        };
        // 装配校验：每个贡献的工厂、标题与命令都必须存在（#2/#16 的职责）。
        for (id, c) in view.shell.contributions.entries() {
            assert!(
                KNOWN_FACTORIES.contains(&c.factory_key.as_ref()),
                "{id}: 未装配的工厂 {}",
                c.factory_key
            );
            view.strings
                .text(&c.title)
                .expect("registered contribution title");
            if let Some(command) = &c.command {
                assert!(
                    view.commands.registry.get(command).is_some(),
                    "{id}: 未登记的命令 {command}"
                );
            }
        }
        view.scan(root, cx);
        view
    }
    fn line_height(&self) -> f32 {
        LINE_HEIGHT * self.shell.zoom()
    }
    fn font_size(&self) -> f32 {
        FONT_SIZE * self.shell.zoom()
    }
    fn label(&self, key: &str, args: &[(&str, String)]) -> String {
        appearance::message(&self.strings, key, args)
    }
    fn ui_color(&self, key: &str) -> Hsla {
        color(
            self.theme
                .color(key)
                .expect("registered color")
                .expect("Dark 2026 value"),
        )
    }
    fn scan(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.scans += 1;
        // SKELETON: build the workspace through the #15 implementation. The explorer still waits
        // for the initial snapshot before building its rows; streaming (root in the first frame)
        // is available from `rove_workspace::Folder::start` and belongs to #23's explorer.
        let task = cx.background_executor().spawn(async move {
            let config = rove_workspace::WorkspaceConfig::from_settings(
                &rove_workspace::default_settings(),
            )?;
            let folder = rove_workspace::Folder::scan_root(root, config)?;
            Ok(folder as Arc<dyn Workspace>)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                match result.and_then(file_tree) {
                    Ok(tree) => view.tree = Some(tree),
                    Err(e) => {
                        eprintln!("workspace: {e}");
                        view.scan_failed = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn activate(&mut self, row: Row, cx: &mut Context<Self>) {
        if row.expandable {
            if let Some(tree) = self.tree.as_mut() {
                if let Err(e) = ListController::expand(tree, &row.id, !row.expanded) {
                    eprintln!("expand: {e}");
                }
            }
            cx.notify();
            return;
        }
        let path = row.id.0.to_string();
        let resource = Resource::Workspace(WorkspacePath::new(&path).expect("workspace path"));
        self.open(resource, path, cx);
    }
    /// The installed service schedules its own work.
    pub fn open_documents(&mut self) -> &mut dyn OpenDocuments {
        &mut self.editor
    }

    fn open(&mut self, resource: Resource, path: String, cx: &mut Context<Self>) {
        self.error_path = None;
        let _ = crate::feedback::FeedbackService::dismiss(
            &mut self.shell.feedback,
            "command.placeholder",
        );
        if let Err(error) = self.open_documents().open(
            resource,
            OpenOptions {
                source: OpenSource::Explorer,
                ..Default::default()
            },
            Cancellation::default(),
        ) {
            eprintln!("open: {error}");
            self.error_path = Some(path);
        }
        cx.notify();
    }

    /// 工作台没有实现的命令交给骨架的命令执行器（#14 的执行桥合并后替换）；
    /// 占位命令返回标题，作为状态栏临时消息显示（3 秒后消失）。
    fn command(&mut self, id: &str, cx: &mut Context<Self>) {
        let executor: Arc<dyn CommandExecutor> = self.commands.clone();
        let task = executor.execute(
            Invocation {
                id: id.into(),
                args: Value::Null,
            },
            Cancellation::default(),
        );
        let task = cx.background_executor().spawn(task);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                if let Ok(Value::String(key)) = result {
                    let _ = shell::view::report(
                        view,
                        "command.placeholder",
                        Reason::Receipt,
                        Message::key(key),
                        Arc::from([]),
                        cx,
                    );
                }
            });
        })
        .detach();
    }
    fn on_scroll(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = match e.delta {
            ScrollDelta::Lines(p) => p.y as f64,
            ScrollDelta::Pixels(p) => p.y.to_f64() / self.line_height() as f64,
        };
        let count = self
            .editor
            .current
            .as_ref()
            .map_or(0, |o| o.document.text.lines().known);
        let rows = ((self.viewport_height - 56.0) / self.line_height()).max(1.0) as f64;
        self.scroll = (self.scroll - delta).clamp(0.0, (count as f64 - rows).max(0.0));
        self.dirty = true;
        cx.notify();
    }
    // SKELETON(code-view): visible-line preparation/scrolling, replaced by #19.
    // SKELETON(code-features): scope/color projection, replaced by #20.
    fn prepare(&mut self, window: &mut Window) -> CoreResult<()> {
        self.prepared.clear();
        let Some(opened) = self.editor.current.as_ref() else {
            return Ok(());
        };
        let text = &opened.document.text;
        let visible =
            ((self.viewport_height - 56.0).max(0.0) / self.line_height()).ceil() as usize + 2;
        let first = self.scroll.floor() as usize;
        let font = Font {
            family: "Consolas".into(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight: FontWeight::default(),
            style: FontStyle::default(),
        };
        let default = self.theme.color("editor.foreground")?.unwrap();
        for line in first..(first + visible).min(text.lines().known as usize) {
            let range = text.line_range(Line(line as u64))?;
            let mut value = String::new();
            text.visit_chunks(range.clone(), &mut |s| value.push_str(s))?;
            #[cfg(test)]
            let mut spans = vec![];
            let mut runs = vec![];
            let mut consumed = 0;
            for token in opened.syntax.tokens(Line(line as u64))?.iter() {
                let start = (token.range.start().0.max(range.start().0) - range.start().0) as usize;
                let end = (token.range.end().0.min(range.end().0) - range.start().0) as usize;
                if start >= end || start < consumed || end > value.len() {
                    continue;
                }
                if start > consumed {
                    runs.push(TextRun {
                        len: start - consumed,
                        font: font.clone(),
                        color: color(default),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    });
                }
                let c = self
                    .theme
                    .token_style(&token.scopes)
                    .foreground
                    .unwrap_or(default);
                runs.push(TextRun {
                    len: end - start,
                    font: font.clone(),
                    color: color(c),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                });
                #[cfg(test)]
                spans.push(ColoredSpan {
                    range: start..end,
                    scope: token.scopes.last().cloned().unwrap_or_default(),
                    color: c,
                });
                consumed = end;
            }
            if consumed < value.len() {
                runs.push(TextRun {
                    len: value.len() - consumed,
                    font: font.clone(),
                    color: color(default),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                });
            }
            let number = self.label("line.number", &[("line", (line + 1).to_string())]);
            let gutter = window.text_system().shape_line(
                number.clone().into(),
                px(self.font_size()),
                &[TextRun {
                    len: number.len(),
                    font: font.clone(),
                    color: self.ui_color("editorLineNumber.foreground"),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            let shape = window.text_system().shape_line(
                value.clone().into(),
                px(self.font_size()),
                &runs,
                None,
            );
            self.prepared.push(PreparedLine {
                number: line,
                #[cfg(test)]
                text: value,
                #[cfg(test)]
                spans,
                shape,
                gutter,
            });
        }
        Ok(())
    }
    /// 状态栏左侧最显眼的文字：临时消息优先，否则是骨架的文件状态条目（测试用）。
    #[cfg(test)]
    fn status(&self) -> String {
        shell::view::status_message_text(self).unwrap_or_else(|| self.file_status())
    }
    // SKELETON(status): fixed filename/line-count/encoding content; replaced by #27.
    fn file_status(&self) -> String {
        if let Some(path) = &self.error_path {
            return self.label("workbench.failed", &[("path", path.clone())]);
        }
        if self.scan_failed {
            return self.label("workbench.scanFailed", &[]);
        }
        if self.editor.loading || self.tree.is_none() {
            return self.label("common.loading", &[]);
        }
        if let Some(o) = &self.editor.current {
            let name = match &o.document.resource {
                Resource::Workspace(p) => p.as_str().to_string(),
                Resource::File(p) => p.display().to_string(),
                Resource::Virtual(p) => p.to_string(),
            };
            self.label(
                "workbench.status",
                &[
                    ("name", name),
                    ("lines", o.document.text.lines().known.to_string()),
                    ("encoding", self.label("encoding.utf8", &[])),
                ],
            )
        } else {
            self.label("workbench.empty", &[])
        }
    }
    fn render_file_tree(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.tree.is_none() {
            let key = if self.scan_failed {
                "workbench.scanFailed"
            } else {
                "common.loading"
            };
            return div().px_2().child(self.label(key, &[])).into_any_element();
        }
        let rows = self.tree.as_ref().map_or(0, ListModel::count);
        let row_height = 24.0 * self.shell.zoom();
        uniform_list(
            "file-tree",
            rows,
            cx.processor(move |v, range, _, cx| {
                let rows = v
                    .tree
                    .as_ref()
                    .and_then(|t| ListModel::rows(t, range).ok())
                    .unwrap_or_else(|| Arc::from([]));
                rows.iter()
                    .map(|row| {
                        let label = if row.expandable {
                            v.label(
                                if row.expanded {
                                    "tree.open"
                                } else {
                                    "tree.closed"
                                },
                                &[("name", row.label.to_string())],
                            )
                        } else {
                            row.label.to_string()
                        };
                        let row = row.clone();
                        div()
                            .id(SharedString::from(row.id.0.to_string()))
                            .h(px(row_height))
                            .pl(px(10.0 + row.depth as f32 * 16.0))
                            .cursor_pointer()
                            .overflow_hidden()
                            .child(label)
                            .on_click(cx.listener(move |v, _, _, cx| v.activate(row.clone(), cx)))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .min_h_0()
        .into_any_element()
    }
}
/// 骨架装配的工厂：视图容器与状态栏条目由工作台通用绘制，文件夹小节是骨架文件树，问题面板是空状态。
const KNOWN_FACTORIES: [&str; 4] = [CONTAINER_FACTORY, STATUS_FACTORY, FILE_VIEW, PROBLEMS];
impl ShellHost for Workbench {
    fn shell(&self) -> &Shell {
        &self.shell
    }
    fn shell_mut(&mut self) -> &mut Shell {
        &mut self.shell
    }
    fn strings(&self) -> &dyn Strings {
        &self.strings
    }
    fn theme(&self) -> &dyn Theme {
        self.theme.as_ref()
    }
    fn commands(&self) -> &CommandRegistry {
        &self.commands.registry
    }
    fn render_editor(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("editor")
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus)
            .bg(self.ui_color("editor.background"))
            .text_color(self.ui_color("editor.foreground"))
            .font_family("Consolas")
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(CodeElement { view: cx.entity() })
            .into_any_element()
    }
    fn render_view(
        &mut self,
        factory_key: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match factory_key {
            FILE_VIEW => Some(self.render_file_tree(cx)),
            PROBLEMS => Some(shell::view::render_problems_empty(self)),
            _ => None,
        }
    }
    fn section_title(&self, id: &str) -> Option<String> {
        (id == FILE_VIEW).then(|| self.shell.title.clone())
    }
    fn active_document(&self) -> Option<DocumentId> {
        self.editor.current.as_ref().map(|o| o.document.id)
    }
    fn editor_focus(&self) -> FocusHandle {
        self.focus.clone()
    }
    fn run_external_command(&mut self, id: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.command(id, cx);
    }
    fn zoom_changed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        // 只重新整形可见行：文档、语法快照与工作区扫描结果都不变。
        self.prepared.clear();
        self.dirty = true;
        cx.notify();
    }
}
impl Focusable for Workbench {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height: f32 = window.viewport_size().height.into();
        if height != self.viewport_height {
            self.viewport_height = height;
            self.dirty = true;
        }
        if self.dirty && !self.preparing {
            self.dirty = false;
            self.preparing = true;
            let view = cx.entity();
            window.defer(cx, move |window, cx| {
                view.update(cx, |v, cx| {
                    v.preparing = false;
                    if let Err(e) = v.prepare(window) {
                        eprintln!("prepare: {e}");
                    }
                    cx.notify();
                });
            });
        }
        if !self.first_frame {
            self.first_frame = true;
            let start = self.started;
            window.on_next_frame(move |_, _| {
                println!(
                    "FIRSTFRAME_SINCE_PROCESS_MS {:.3}",
                    start.elapsed().as_secs_f64() * 1000.0
                );
            });
        }
        let status = StatusEntry {
            text: self.file_status(),
            ..Default::default()
        };
        let shell = &mut self.shell;
        let _ = shell
            .status_bar
            .set(&shell.contributions, appearance::STATUS_FILE, status);
        let workbench = shell::view::render(self, window, cx);
        div()
            .size_full()
            .font_family("Segoe UI")
            .on_action(
                cx.listener(|v, _: &QuickOpen, _, cx| v.command("workbench.action.quickOpen", cx)),
            )
            .on_action(cx.listener(|v, _: &ShowCommands, _, cx| {
                v.command("workbench.action.showCommands", cx)
            }))
            .child(workbench)
    }
}
struct CodeElement {
    view: Entity<Workbench>,
}
impl IntoElement for CodeElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for CodeElement {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let v = self.view.read(cx);
        let rows = v.prepared.clone();
        let scroll = v.scroll;
        let (line_height, zoom) = (v.line_height(), v.shell.zoom());
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for row in &rows {
                let y = bounds.top() + px((row.number as f64 - scroll) as f32 * line_height);
                let _ = row.gutter.paint(
                    point(bounds.left() + px(8.0 * zoom), y),
                    px(line_height),
                    window,
                    cx,
                );
                let _ = row.shape.paint(
                    point(bounds.left() + px(64.0 * zoom), y),
                    px(line_height),
                    window,
                    cx,
                );
            }
        });
        self.view.update(cx, |v, _| v.painted = rows);
    }
}
/// Launch the actual GPUI application. Path is session input, never persisted in repository data.
pub fn launch(root: PathBuf, started: Instant) {
    Application::new().run(move |cx: &mut App| {
        if let Err(e) = cx
            .text_system()
            .add_fonts(vec![Cow::Borrowed(shell::icons::FONT)])
        {
            eprintln!("codicons: {e}");
        }
        cx.bind_keys([
            KeyBinding::new("ctrl-p", QuickOpen, None),
            KeyBinding::new("ctrl-shift-p", ShowCommands, None),
        ]);
        cx.bind_keys(shell::view::key_bindings());
        // 布局文件只有几百字节：首帧前读取，用来恢复窗口位置、大小与最大化状态。
        let store = LayoutStore::for_workspace(&root);
        let layout = store.load();
        let settings = shell::view::default_settings();
        let read = |key: &'static str| {
            Key::<String>::new(key)
                .read(settings.as_ref(), &Scope::default())
                .unwrap_or_default()
        };
        let custom_title = read("window.titleBarStyle") != "native";
        let restore = read("window.restoreWindows") != "none";
        let saved = layout
            .window
            .filter(|_| restore)
            .map(|w| {
                (
                    Bounds::new(point(px(w.x), px(w.y)), size(px(w.width), px(w.height))),
                    w.maximized,
                )
            })
            .filter(|(b, _)| cx.displays().iter().any(|d| d.bounds().intersects(b)));
        let window_bounds = match saved {
            Some((b, true)) => WindowBounds::Maximized(b),
            Some((b, false)) => WindowBounds::Windowed(b),
            None => WindowBounds::Windowed(Bounds::centered(None, size(px(1200.0), px(800.0)), cx)),
        };
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(window_bounds),
                    titlebar: Some(TitlebarOptions {
                        title: Some(
                            appearance::message(&appearance::strings().unwrap(), "app.title", &[])
                                .into(),
                        ),
                        appears_transparent: custom_title,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        let mut view = Workbench::with_store(root, started, store, cx);
                        Shell::attach_window(&mut view, window, cx);
                        view
                    })
                },
            )
            .expect("create window");
        window
            .update(cx, |v, window, cx| {
                window.focus(&v.focus);
                cx.activate(true);
            })
            .unwrap();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}

#[cfg(test)]
#[path = "workbench_shell_tests.rs"]
mod shell_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};
    #[gpui::test]
    fn walking_skeleton_opens_csharp_through_real_window(cx: &mut TestAppContext) {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join("First.cs"),
            "public class First {}\n// 中文😀\n",
        )
        .unwrap();
        let (view, cx) =
            cx.add_window_view(|_, cx| Workbench::new(folder.path().into(), Instant::now(), cx));
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert_eq!(v.tree.as_ref().unwrap().count(), 1));
        cx.simulate_click(point(px(128.0), px(104.0)), Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            cx.refresh_windows();
        });
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert!(v
                .editor
                .find(&Resource::Workspace(
                    WorkspacePath::new("First.cs").unwrap()
                ))
                .is_some());
            let opened = v.editor.current.as_ref().unwrap();
            assert_eq!(opened.document.text.lines().known, 3);
            let line = v
                .painted
                .iter()
                .find(|l| l.number == 0)
                .expect("first line painted");
            assert_eq!(line.text, "public class First {}");
            let keyword = line.spans.iter().find(|s| s.range == (0..6)).unwrap();
            assert_eq!(keyword.scope.as_ref(), "storage.modifier.public.cs");
            // Dark 2026（#10 正式主题引擎）：include 链中 dark_vs 的 `storage.modifier`
            // 比 2026-dark 的 `storage` 更具体，与 VS Code 一致取 #569cd6。
            assert_eq!(keyword.color, Color(0x56, 0x9c, 0xd6, 255));
            assert_eq!(line.shape.len(), line.text.len());
            assert!(v.status().contains("UTF-8"));
        });
    }
    #[gpui::test]
    fn directory_expansion_replacement_and_virtual_scroll(cx: &mut TestAppContext) {
        let folder = tempfile::tempdir().unwrap();
        std::fs::create_dir(folder.path().join("src")).unwrap();
        std::fs::write(
            folder.path().join("src/a.cs"),
            "public class A {}\n".repeat(500),
        )
        .unwrap();
        std::fs::write(folder.path().join("src/b.cs"), "public class B {}\n").unwrap();
        let (view, cx) =
            cx.add_window_view(|_, cx| Workbench::new(folder.path().into(), Instant::now(), cx));
        cx.run_until_parked();
        cx.simulate_click(point(px(128.0), px(104.0)), Modifiers::default());
        view.read_with(cx, |v, _| assert_eq!(v.tree.as_ref().unwrap().count(), 3));
        cx.simulate_click(point(px(128.0), px(128.0)), Modifiers::default());
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert_eq!(
                v.editor
                    .current
                    .as_ref()
                    .unwrap()
                    .document
                    .text
                    .lines()
                    .known,
                501
            );
            assert!(!v.painted.is_empty());
            assert!(v.painted.len() < 100);
        });
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(400.0), px(100.0)),
            delta: ScrollDelta::Lines(point(0.0, -20.0)),
            modifiers: Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert_eq!(v.painted.first().unwrap().number, 20));
        cx.simulate_click(point(px(128.0), px(152.0)), Modifiers::default());
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert_eq!(v.painted.first().unwrap().text, "public class B {}");
            assert_eq!(v.scroll, 0.0);
            assert!(v.status().contains("b.cs"));
        });
    }
    #[gpui::test]
    fn registered_key_shows_temporary_status(cx: &mut TestAppContext) {
        let folder = tempfile::tempdir().unwrap();
        let (view, cx) =
            cx.add_window_view(|_, cx| Workbench::new(folder.path().into(), Instant::now(), cx));
        cx.run_until_parked();
        cx.update(|window, cx| {
            cx.bind_keys([KeyBinding::new("ctrl-p", QuickOpen, None)]);
            window.focus(&view.read(cx).focus);
        });
        cx.simulate_keystrokes("ctrl-p");
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert_eq!(v.status(), v.label("commands.quickOpen", &[]))
        });
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(3));
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert_eq!(v.status(), v.label("workbench.empty", &[]))
        });
    }
}
