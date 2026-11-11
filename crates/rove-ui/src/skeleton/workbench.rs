use super::{
    appearance::{self, DarkTheme},
    commands::{Commands, StatusFeedback},
    editor::{Pending, SingleEditor},
    text::{default_settings, FileDocuments},
    workspace::Folder,
};
use crate::{
    contributions::ContributionRegistry,
    documents::{OpenDocuments, OpenOptions, OpenSource},
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
use rove_workspace::{EntryKind, Workspace};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc, time::Instant};
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
    editor: SingleEditor,
    theme: Arc<dyn Theme>,
    strings: StringTable,
    contributions: ContributionRegistry,
    commands: Arc<dyn CommandExecutor>,
    feedback: StatusFeedback,
    focus: FocusHandle,
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
    pub fn new(root: PathBuf, started: Instant, cx: &mut Context<Self>) -> Self {
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
        let mut view = Self {
            tree: None,
            editor: SingleEditor::new(
                Arc::new(FileDocuments::new(root.clone(), default_settings())),
                rove_syntax::highlight::TreeSitterSyntax::default(),
                schedule,
            ),
            theme: Arc::new(DarkTheme::new().unwrap()),
            strings: appearance::strings().unwrap(),
            contributions: appearance::contributions().unwrap(),
            commands: Arc::new(Commands::new().unwrap()),
            feedback: StatusFeedback::default(),
            focus: cx.focus_handle(),
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
        self.feedback.clear();
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

    fn command(&mut self, id: &str, cx: &mut Context<Self>) {
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
    fn on_scroll(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = match e.delta {
            ScrollDelta::Lines(p) => p.y as f64,
            ScrollDelta::Pixels(p) => p.y.to_f64() / LINE_HEIGHT as f64,
        };
        let count = self
            .editor
            .current
            .as_ref()
            .map_or(0, |o| o.document.text.lines().known);
        let rows = ((self.viewport_height - 56.0) / LINE_HEIGHT).max(1.0) as f64;
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
        let visible = ((self.viewport_height - 56.0).max(0.0) / LINE_HEIGHT).ceil() as usize + 2;
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
    // SKELETON(status): fixed filename/line-count/encoding content; replaced by #27.
    fn status(&self) -> String {
        if let Some(m) = self.feedback.message.as_ref() {
            return self.strings.text(m).unwrap();
        }
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .on_scroll_wheel(cx.listener(Self::on_scroll))
                            .child(CodeElement { view: cx.entity() }),
                    ),
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
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for row in &rows {
                let y = bounds.top() + px((row.number as f64 - scroll) as f32 * LINE_HEIGHT);
                let _ = row.gutter.paint(
                    point(bounds.left() + px(8.0), y),
                    px(LINE_HEIGHT),
                    window,
                    cx,
                );
                let _ = row.shape.paint(
                    point(bounds.left() + px(64.0), y),
                    px(LINE_HEIGHT),
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
        cx.bind_keys([
            KeyBinding::new("ctrl-p", QuickOpen, None),
            KeyBinding::new("ctrl-shift-p", ShowCommands, None),
        ]);
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
                |_, cx| cx.new(|cx| Workbench::new(root, started, cx)),
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
        cx.simulate_click(point(px(80.0), px(88.0)), Modifiers::default());
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
