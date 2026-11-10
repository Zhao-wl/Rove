use super::{
    appearance::{self, DarkTheme},
    commands::{Commands, StatusFeedback},
    text::{default_settings, FileDocuments},
    workspace::Folder,
};
use crate::{
    contributions::ContributionRegistry,
    documents::{EditorGroupId, OpenDocuments, OpenOptions, OpenSource},
    editors::{
        actions as editor_actions,
        settings::{EditorKind, EditorSettings},
        Axis, Content, Direction, EditorService, Layout, OpenJob, Tab, TabId,
    },
    feedback::{Feedback, FeedbackLevel, FeedbackService},
    lists::{ItemId, ListController, ListModel, Navigation, Row},
};
use gpui::{
    actions, div, point, prelude::*, px, relative, rgba, size, uniform_list, App, Application,
    Bounds, ContentMask, Context, Element, ElementId, Entity, FocusHandle, Focusable, Font,
    FontFeatures, FontStyle, FontWeight, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    KeyBinding, LayoutId, Pixels, Render, ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString,
    Style, TextRun, TitlebarOptions, Window, WindowBounds, WindowOptions,
};
use rove_base::{
    Cancellation, Error, Line, Resource, Result as CoreResult, Value, Version, WorkspacePath,
};
use rove_commands::{CommandExecutor, Invocation};
use rove_i18n::{Message, StringTable, Strings};
use rove_theme::{Color, Theme};
use rove_workspace::{EntryKind, FileEvent, Workspace};
use std::{
    collections::{BTreeSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

mod editor_panes;
#[cfg(test)]
mod editor_tests;
const LINE_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 14.0;
fn color(c: Color) -> Hsla {
    rgba(((c.0 as u32) << 24) | ((c.1 as u32) << 16) | ((c.2 as u32) << 8) | c.3 as u32).into()
}

// SKELETON(explorer): fixed expandable file tree; replaced by #23.
// SKELETON(lists): virtualized tree data/control; replaced by #18.
struct FileTree {
    workspace: Arc<dyn Workspace>,
    rows: Vec<Row>,
    expanded: BTreeSet<ItemId>,
    revision: Version,
}
impl FileTree {
    fn new(workspace: Arc<dyn Workspace>) -> CoreResult<Self> {
        let mut t = Self {
            workspace,
            rows: vec![],
            expanded: BTreeSet::new(),
            revision: Version(0),
        };
        t.rebuild()?;
        Ok(t)
    }
    fn rebuild(&mut self) -> CoreResult<()> {
        self.rows.clear();
        let roots = self.workspace.children(None, 0..usize::MAX)?;
        let mut pending: Vec<_> = roots.items.iter().rev().cloned().map(|e| (e, 0)).collect();
        while let Some((e, depth)) = pending.pop() {
            let id = ItemId(e.path.as_str().into());
            let expandable = matches!(e.kind, EntryKind::Directory);
            let expanded = self.expanded.contains(&id);
            self.rows.push(Row {
                id,
                label: e.path.as_str().rsplit('/').next().unwrap().into(),
                detail: None,
                depth,
                expandable,
                expanded,
                disabled: false,
            });
            if expandable && expanded {
                let children = self.workspace.children(Some(&e.path), 0..usize::MAX)?;
                pending.extend(children.items.iter().rev().cloned().map(|e| (e, depth + 1)));
            }
        }
        Ok(())
    }
}
impl ListModel for FileTree {
    fn revision(&self) -> Version {
        self.revision
    }
    fn count(&self) -> usize {
        self.rows.len()
    }
    fn rows(&self, r: std::ops::Range<usize>) -> CoreResult<Arc<[Row]>> {
        if r.start > r.end || r.start > self.rows.len() {
            return Err(Error::OutOfBounds);
        }
        Ok(Arc::from(&self.rows[r.start..r.end.min(self.rows.len())]))
    }
}
impl ListController for FileTree {
    fn navigate(&mut self, _: Navigation) -> CoreResult<()> {
        Err(Error::Unsupported)
    }
    fn filter(&mut self, _: Arc<str>, _: Cancellation) -> CoreResult<()> {
        Err(Error::Unsupported)
    }
    fn selection(&self) -> Arc<[ItemId]> {
        Arc::from([])
    }
    fn expand(&mut self, id: &ItemId, expanded: bool) -> CoreResult<()> {
        if !self.rows.iter().any(|r| &r.id == id && r.expandable) {
            return Err(Error::NotFound);
        }
        if expanded {
            self.expanded.insert(id.clone());
        } else {
            self.expanded.remove(id);
        }
        self.revision.0 += 1;
        self.rebuild()
    }
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

// SKELETON(workbench): fixed containers and assembly; replaced by #16.
pub struct Workbench {
    tree: Option<FileTree>,
    editor: EditorService,
    root: PathBuf,
    prepared: VecDeque<PreparedViewport>,
    trace_switch: bool,
    switch_started: Option<(TabId, Instant)>,
    #[cfg(test)]
    painted: Vec<PreparedLine>,
    theme: Arc<dyn Theme>,
    strings: StringTable,
    contributions: ContributionRegistry,
    commands: Commands,
    feedback: StatusFeedback,
    focus: FocusHandle,
    scan_failed: bool,
    workspace_events: Option<rove_base::Subscription>,
    started: Instant,
    first_frame: bool,
}
impl Workbench {
    pub fn new(root: PathBuf, started: Instant, cx: &mut Context<Self>) -> Self {
        Self::with_settings(root, started, EditorSettings::default(), cx)
    }

    fn with_settings(
        root: PathBuf,
        started: Instant,
        settings: EditorSettings,
        cx: &mut Context<Self>,
    ) -> Self {
        let owner = cx.entity().downgrade();
        let app = cx.to_async();
        let background = cx.background_executor().clone();
        let schedule = move |job: OpenJob| {
            let OpenJob { id, task } = job;
            let read = background.spawn(task);
            let progress = background.timer(Duration::from_millis(100));
            let loading_owner = owner.clone();
            app.spawn(async move |cx| {
                progress.await;
                let _ = loading_owner.update(cx, |view, cx| {
                    if view.editor.slow(id) {
                        cx.notify();
                    }
                });
            })
            .detach();
            let owner = owner.clone();
            app.spawn(async move |cx| {
                let result = read.await;
                let _ = owner.update(cx, |view, cx| {
                    if view.editor.finish(id, result) {
                        cx.notify();
                    }
                });
            })
            .detach();
        };
        let size = settings.size;
        let mut view = Self {
            tree: None,
            editor: EditorService::new(
                Arc::new(
                    FileDocuments::new(root.clone(), default_settings()).with_size_policy(size),
                ),
                Arc::new(rove_syntax::highlight::TreeSitterSyntax::default()),
                settings,
                schedule,
            ),
            root: root.clone(),
            trace_switch: std::env::var_os("ROVE_EDITOR_SWITCH_TRACE").is_some(),
            switch_started: None,
            prepared: VecDeque::new(),
            #[cfg(test)]
            painted: Vec::new(),
            theme: Arc::new(DarkTheme::new().unwrap()),
            strings: appearance::strings().unwrap(),
            contributions: appearance::contributions().unwrap(),
            commands: Commands::new().unwrap(),
            feedback: StatusFeedback::default(),
            focus: cx.focus_handle(),
            scan_failed: false,
            workspace_events: None,
            started,
            first_frame: false,
        };
        // Validate the two factories assembled by this fixed skeleton host.
        for (id, placement) in [
            (
                "workbench.view.explorer",
                crate::contributions::Placement::Sidebar {
                    container: "workbench.view.explorer".into(),
                },
            ),
            (
                "rove.status.file",
                crate::contributions::Placement::StatusLeft,
            ),
        ] {
            let contribution = view.contributions.get(id).expect("registered contribution");
            assert_eq!(contribution.factory_key.as_ref(), id);
            match (&contribution.placement, placement) {
                (
                    crate::contributions::Placement::Sidebar { container: actual },
                    crate::contributions::Placement::Sidebar { container },
                ) => assert_eq!(actual, &container),
                (
                    crate::contributions::Placement::StatusLeft,
                    crate::contributions::Placement::StatusLeft,
                ) => {}
                _ => panic!("unexpected skeleton contribution placement"),
            }
            view.strings
                .text(&contribution.title)
                .expect("registered contribution title");
        }
        view.scan(root, cx);
        view
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
        let task = cx.background_executor().spawn(Folder::root(root));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                match result.and_then(FileTree::new) {
                    Ok(tree) => {
                        view.watch_workspace(&tree.workspace, cx);
                        view.tree = Some(tree);
                    }
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
    fn watch_workspace(&mut self, workspace: &Arc<dyn Workspace>, cx: &mut Context<Self>) {
        if !self.editor.settings.close_on_delete {
            return;
        }
        let (send, receive) = async_channel::unbounded();
        self.workspace_events = Some(workspace.changes().subscribe(Arc::new(move |batch| {
            let _ = send.try_send(batch);
        })));
        cx.spawn(async move |this, cx| {
            while let Ok(first) = receive.recv().await {
                let mut removed = BTreeSet::new();
                for batch in
                    std::iter::once(first).chain(std::iter::from_fn(|| receive.try_recv().ok()))
                {
                    for event in batch.events.iter() {
                        if let FileEvent::Removed(path) | FileEvent::Renamed { from: path, .. } =
                            event
                        {
                            removed.insert(path.clone());
                        }
                    }
                }
                if this
                    .update(cx, |view, cx| {
                        let mut changed = false;
                        for path in removed {
                            changed |= view.editor.file_removed(&Resource::Workspace(path));
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Builds a window using the registered editor preferences from a supplied settings service.
    pub fn with_editor_settings(
        root: PathBuf,
        started: Instant,
        settings: &dyn rove_config::Settings,
        cx: &mut Context<Self>,
    ) -> CoreResult<Self> {
        Ok(Self::with_settings(
            root,
            started,
            EditorSettings::read(settings)?,
            cx,
        ))
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
        self.feedback.clear();
        let preserve_focus = self.editor.settings.explorer_preserve_focus;
        let options = OpenOptions {
            source: OpenSource::Explorer,
            preserve_focus,
            ..Default::default()
        };
        if let Err(error) = self
            .open_documents()
            .open(resource, options, Cancellation::default())
        {
            eprintln!("open {path}: {error}");
        }
        cx.notify();
    }

    /// The installed, window-owned service. Subscribe to events before opening.
    pub fn open_documents(&mut self) -> &mut dyn OpenDocuments {
        &mut self.editor
    }

    /// Installs a placeholder for a registered editor type until its view factory is available.
    pub fn register_editor_placeholder(
        &mut self,
        id: &str,
        label_key: &'static str,
    ) -> CoreResult<()> {
        self.strings.text(&Message::key(label_key))?;
        self.editor
            .settings
            .editors
            .register_placeholder(id, label_key)
    }

    fn note_switch(&mut self, previous: Option<TabId>, start: Instant) {
        if self.trace_switch {
            self.switch_started = None;
            if let Some(tab) = self.editor.active().current() {
                if previous != Some(tab.id)
                    && self.prepared.iter().any(|cache| cache.key.tab == tab.id)
                {
                    self.switch_started = Some((tab.id, start));
                }
            }
        }
    }

    fn switch_tab(&mut self, group: EditorGroupId, tab: TabId) {
        let start = Instant::now();
        let previous = self.editor.active().active;
        self.editor.activate(group, tab, true);
        self.note_switch(previous, start);
    }

    /// Shared by the command palette and shortcut dispatch.
    pub fn command_registry(&self) -> &rove_commands::CommandRegistry {
        &self.commands.registry
    }

    fn command(&mut self, id: &str, cx: &mut Context<Self>) {
        let start = Instant::now();
        let previous = self.editor.active().active;
        if self.commands.registry.get(id).is_some() && editor_actions::execute(&mut self.editor, id)
        {
            self.note_switch(previous, start);
            cx.notify();
            return;
        }
        let task = self.commands.execute(
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
                    let _ = view.feedback.show(Feedback {
                        key: "command.placeholder".into(),
                        level: FeedbackLevel::Status { duration_ms: 3000 },
                        message: Message::key(key),
                        actions: Arc::from([]),
                    });
                    let generation = view.feedback.generation;
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(3000))
                            .await;
                        let _ = this.update(cx, |view, cx| {
                            if view.feedback.generation == generation {
                                let _ = view.feedback.dismiss("command.placeholder");
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn on_scroll(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        e: &ScrollWheelEvent,
        cx: &mut Context<Self>,
    ) {
        let delta = match e.delta {
            ScrollDelta::Lines(p) => p.y as f64,
            ScrollDelta::Pixels(p) => p.y.to_f64() / LINE_HEIGHT as f64,
        };
        let Some(pane) = self.editor.groups.get_mut(&group) else {
            return;
        };
        let rows = pane.rows;
        let Some(tab) = pane.current_mut() else {
            return;
        };
        let count = tab
            .opened()
            .map_or(0, |opened| opened.document.text.lines().known);
        let view = if secondary {
            let Some(view) = tab.secondary.as_mut() else {
                return;
            };
            view
        } else {
            &mut tab.view
        };
        view.scroll = (view.scroll - delta).clamp(0.0, (count as f64 - rows).max(0.0));
        cx.notify();
    }

    fn viewport_key(
        &self,
        group: EditorGroupId,
        secondary: bool,
        rows: usize,
    ) -> Option<ViewportKey> {
        let tab = self.editor.groups.get(&group)?.current()?;
        let opened = tab.opened()?;
        if tab.kind != EditorKind::Text {
            return None;
        }
        let state = if secondary {
            tab.secondary.as_ref()?
        } else {
            &tab.view
        };
        Some(ViewportKey {
            tab: tab.id,
            secondary,
            document: opened.document.id,
            version: opened.document.text.version(),
            first: state.scroll.floor() as usize,
            rows,
        })
    }

    // SKELETON(code-view): visible-line preparation/scrolling, replaced by #19.
    // SKELETON(code-features): scope/color projection, replaced by #20.
    fn prepare(
        &self,
        group: EditorGroupId,
        key: ViewportKey,
        window: &mut Window,
    ) -> CoreResult<Vec<PreparedLine>> {
        let mut prepared = Vec::new();
        let Some(opened) = self
            .editor
            .groups
            .get(&group)
            .and_then(|g| g.current())
            .and_then(Tab::opened)
        else {
            return Ok(prepared);
        };
        let text = &opened.document.text;
        let first = key.first;
        let visible = key.rows + 2;
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
            let tokens = opened
                .syntax
                .as_ref()
                .map(|syntax| syntax.tokens(Line(line as u64)))
                .transpose()?
                .unwrap_or_default();
            for token in tokens.iter() {
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
                px(FONT_SIZE),
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
            let shape =
                window
                    .text_system()
                    .shape_line(value.clone().into(), px(FONT_SIZE), &runs, None);
            prepared.push(PreparedLine {
                number: line,
                #[cfg(test)]
                text: value,
                #[cfg(test)]
                spans,
                shape,
                gutter,
            });
        }
        Ok(prepared)
    }
    // SKELETON(status): fixed filename/line-count/encoding content; replaced by #27.
    fn status(&self) -> String {
        if let Some(m) = self.feedback.message.as_ref() {
            return self.strings.text(m).unwrap();
        }
        if self.scan_failed {
            return self.label("workbench.scanFailed", &[]);
        }
        if self
            .editor
            .active()
            .current()
            .is_some_and(Tab::show_progress)
            || self.tree.is_none()
        {
            return self.label("common.loading", &[]);
        }
        if let Some(o) = self.editor.active().current().and_then(Tab::opened) {
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
}
impl Focusable for Workbench {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let title = self
            .strings
            .text(
                &self
                    .contributions
                    .get("workbench.view.explorer")
                    .unwrap()
                    .title,
            )
            .unwrap();
        let rows = self.tree.as_ref().map_or(0, ListModel::count);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(self.ui_color("editor.background"))
            .text_color(self.ui_color("editor.foreground"))
            .text_size(px(FONT_SIZE))
            .font_family("Consolas")
            .track_focus(&self.focus)
            .key_context(editor_actions::context(&self.editor).as_str())
            .on_action(
                cx.listener(|v, action: &editor_actions::EditorAction, _, cx| {
                    v.command(action.0, cx)
                }),
            )
            .on_action(
                cx.listener(|v, _: &QuickOpen, _, cx| v.command("workbench.action.quickOpen", cx)),
            )
            .on_action(cx.listener(|v, _: &ShowCommands, _, cx| {
                v.command("workbench.action.showCommands", cx)
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .w(px(280.0))
                            .h_full()
                            .flex()
                            .flex_col()
                            .bg(self.ui_color("sideBar.background"))
                            .text_color(self.ui_color("sideBar.foreground"))
                            .child(div().h(px(28.0)).px_2().child(title))
                            .child(
                                uniform_list(
                                    "file-tree",
                                    rows,
                                    cx.processor(|v, range, _, cx| {
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
                                                    .h(px(24.0))
                                                    .pl(px(10.0 + row.depth as f32 * 16.0))
                                                    .cursor_pointer()
                                                    .overflow_hidden()
                                                    .child(label)
                                                    .on_click(cx.listener(move |v, _, _, cx| {
                                                        v.activate(row.clone(), cx)
                                                    }))
                                            })
                                            .collect::<Vec<_>>()
                                    }),
                                )
                                .flex_1()
                                .min_h_0(),
                            ),
                    )
                    .child(self.editor_grid(cx)),
            )
            .child(
                div()
                    .id("status")
                    .h(px(28.0))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .bg(self.ui_color("statusBar.background"))
                    .text_color(self.ui_color("statusBar.foreground"))
                    .px_2()
                    .child(self.status()),
            )
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct ViewportKey {
    tab: TabId,
    secondary: bool,
    document: rove_base::DocumentId,
    version: Version,
    first: usize,
    rows: usize,
}

struct PreparedViewport {
    key: ViewportKey,
    lines: Arc<[PreparedLine]>,
}

struct CodeElement {
    view: Entity<Workbench>,
    group: EditorGroupId,
    secondary: bool,
}
impl IntoElement for CodeElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for CodeElement {
    type RequestLayoutState = ();
    type PrepaintState = usize;
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
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> usize {
        let rows = (bounds.size.height.to_f64() / f64::from(LINE_HEIGHT))
            .floor()
            .max(1.0) as usize;
        let view = self.view.read(cx);
        let key = view.viewport_key(self.group, self.secondary, rows);
        let needs_rows = !self.secondary
            && view
                .editor
                .groups
                .get(&self.group)
                .is_some_and(|g| g.rows != rows as f64);
        let needs_cache =
            key.is_some_and(|key| !view.prepared.iter().any(|cache| cache.key == key));
        if needs_rows || needs_cache {
            let view = self.view.downgrade();
            let group = self.group;
            let secondary = self.secondary;
            window.defer(cx, move |window, cx| {
                let _ = view.update(cx, |v, cx| {
                    if !secondary {
                        if let Some(pane) = v.editor.groups.get_mut(&group) {
                            pane.rows = rows as f64;
                        }
                    }
                    if let Some(key) =
                        key.filter(|key| v.viewport_key(group, secondary, rows) == Some(*key))
                    {
                        if !v.prepared.iter().any(|cache| cache.key == key) {
                            let lines = v.prepare(group, key, window).unwrap_or_else(|error| {
                                eprintln!("prepare: {error}");
                                Vec::new()
                            });
                            v.prepared.retain(|cache| {
                                cache.key.tab != key.tab || cache.key.secondary != secondary
                            });
                            v.prepared.push_back(PreparedViewport {
                                key,
                                lines: lines.into(),
                            });
                            if v.prepared.len() > 32 {
                                v.prepared.pop_front();
                            }
                            cx.notify();
                        }
                    }
                });
            });
        }
        rows
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        rows: &mut usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        let v = self.view.read(cx);
        let Some(key) = v.viewport_key(self.group, self.secondary, *rows) else {
            return;
        };
        let Some(cache) = v.prepared.iter().find(|cache| cache.key == key) else {
            return;
        };
        let tab = v.editor.groups[&self.group].current().unwrap();
        let view = if self.secondary {
            tab.secondary.as_ref().unwrap()
        } else {
            &tab.view
        };
        let scroll = view.scroll;
        let selection = view.selection.clone();
        let selection_color = v.ui_color("editor.selectionBackground");
        let rows = cache.lines.clone();
        let active_primary = v.editor.focused == self.group && !self.secondary;
        let trace = v
            .switch_started
            .filter(|(id, _)| active_primary && *id == key.tab);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for row in rows.iter() {
                let y = bounds.top() + px((row.number as f64 - scroll) as f32 * LINE_HEIGHT);
                let _ = row.gutter.paint(
                    point(bounds.left() + px(8.0), y),
                    px(LINE_HEIGHT),
                    window,
                    cx,
                );
                if let Some(selection) = &selection {
                    let (start, end) = (selection.start(), selection.end());
                    let line = Line(row.number as u64);
                    if line >= start.line && line <= end.line {
                        let len = row.shape.len();
                        let left = if line == start.line {
                            start.column.0 as usize
                        } else {
                            0
                        };
                        let right = if line == end.line {
                            end.column.0 as usize
                        } else {
                            len
                        };
                        let x = row.shape.x_for_index(left.min(len));
                        let width = row.shape.x_for_index(right.min(len)) - x;
                        if width > px(0.0) {
                            window.paint_quad(gpui::fill(
                                Bounds {
                                    origin: point(bounds.left() + px(64.0) + x, y),
                                    size: size(width, px(LINE_HEIGHT)),
                                },
                                selection_color,
                            ));
                        }
                    }
                }
                let _ = row.shape.paint(
                    point(bounds.left() + px(64.0), y),
                    px(LINE_HEIGHT),
                    window,
                    cx,
                );
            }
        });
        if let Some((_, start)) = trace {
            println!(
                "EDITOR_SWITCH_TO_PAINT_MS {:.3}",
                start.elapsed().as_secs_f64() * 1000.0
            );
            self.view.update(cx, |v, _| v.switch_started = None);
        }
        #[cfg(test)]
        if active_primary {
            self.view.update(cx, |v, _| v.painted = rows.to_vec());
        }
    }
}
/// Launch the actual GPUI application. Path is session input, never persisted in repository data.
pub fn launch(root: PathBuf, started: Instant) {
    let settings =
        crate::editors::startup_settings::load(crate::editor_setting_defaults().unwrap());
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("ctrl-p", QuickOpen, None),
            KeyBinding::new("ctrl-shift-p", ShowCommands, None),
        ]);
        cx.bind_keys(editor_actions::bindings());
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some(
                            appearance::message(&appearance::strings().unwrap(), "app.title", &[])
                                .into(),
                        ),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        Workbench::with_editor_settings(root, started, &settings, cx)
                            .expect("valid editor settings")
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
        cx.simulate_click(point(px(80.0), px(40.0)), Modifiers::default());
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
            let opened = v.editor.active().current().and_then(Tab::opened).unwrap();
            assert_eq!(opened.document.text.lines().known, 3);
            let line = v
                .painted
                .iter()
                .find(|l| l.number == 0)
                .expect("first line painted");
            assert_eq!(line.text, "public class First {}");
            let keyword = line.spans.iter().find(|s| s.range == (0..6)).unwrap();
            assert_eq!(keyword.scope.as_ref(), "storage.modifier.public.cs");
            assert_eq!(keyword.color, appearance::KEYWORD);
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
        cx.simulate_click(point(px(80.0), px(40.0)), Modifiers::default());
        view.read_with(cx, |v, _| assert_eq!(v.tree.as_ref().unwrap().count(), 3));
        cx.simulate_click(point(px(80.0), px(64.0)), Modifiers::default());
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert_eq!(
                v.editor
                    .active()
                    .current()
                    .and_then(Tab::opened)
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
        cx.simulate_click(point(px(80.0), px(88.0)), Modifiers::default());
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert_eq!(v.painted.first().unwrap().text, "public class B {}");
            assert_eq!(v.editor.active().current().unwrap().view.scroll, 0.0);
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
