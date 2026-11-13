//! 资源管理器的窗口状态与绘制（#23，GPUI）：#18 `ListView` 的包装。
//!
//! 数据来自 [`ExplorerSource`]（#15 的工作区），渲染、虚拟化、缩进、缩进参考线、粘性滚动、键盘导航与
//! 查找过滤全部复用 #18 的树组件；这里只做数据适配、行渲染（图标、紧凑文件夹分段、装饰）、
//! 资源管理器自己的命令与上下文键。
use super::{
    decorations::{
        decorate, Badge, DecorationColor, DecorationProvider, ProblemDecorations,
        ReadOnlyDecorations,
    },
    model::{
        chain_paths, is_error_id, AutoReveal, ExplorerSettings, ExplorerSource, Refresh,
        SETTING_KEYS,
    },
    registry::{self, context as ck, ExplorerAction},
};
use crate::{
    list_widget::{
        options::ListOptions,
        tree::{Children, TreeModel, TreeSource},
        view::{
            default_row, highlighted_label, hsla, EmptyState, ListView, ListViewEvent, RowContext,
            RowRenderer,
        },
        widget::{ListEvent, ListWidget, RowHeight},
    },
    lists::{ItemId, Row},
    outline::view::codicon,
};
use gpui::{
    div, prelude::*, px, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    KeyContext, Render, SharedString, Window,
};
use rove_base::WorkspacePath;
use rove_config::Settings;
use rove_i18n::{Message, Strings};
use rove_theme::Theme;
use rove_workspace::{FileEvent, Workspace};
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// 行高（与 #2 骨架一致；缩放由窗口整体处理）。
pub const ROW_HEIGHT: f32 = 24.0;
const ICON: f32 = 16.0;

/// 资源管理器对宿主的请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExplorerEvent {
    /// 打开文件（经 #17 的打开文档服务）。
    Open {
        path: WorkspacePath,
        preserve_focus: bool,
        /// false = 预览标签（单击）。
        pinned: bool,
        side: bool,
    },
    /// 空状态按钮的命令 ID（`workbench.action.files.openFolder`）。
    Command(Arc<str>),
}

/// 没有打开文件夹时的空树。
struct NoFolder;
impl TreeSource for NoFolder {
    fn children(&self, _: Option<&ItemId>) -> Children {
        Children::Ready(Arc::from([]))
    }
}

/// 行渲染读取的共享状态（渲染闭包在 GPUI 的单线程里运行）。
struct RenderState {
    providers: Vec<Arc<dyn DecorationProvider>>,
    badges: bool,
    colors: bool,
    /// 紧凑文件夹行当前聚焦的段：(行 ID, 段下标)。
    segment: Option<(Arc<str>, usize)>,
}

pub struct ExplorerView {
    list: Entity<ListView>,
    render: Rc<RefCell<RenderState>>,
    source: Option<Arc<ExplorerSource>>,
    workspace: Option<Arc<dyn Workspace>>,
    settings: ExplorerSettings,
    strings: Arc<dyn Strings>,
    problems: Arc<ProblemDecorations>,
    /// 紧凑文件夹行内聚焦的段：(行 ID, 段下标)；行不是当前聚焦行时视为最后一段。
    segment: Option<(Arc<str>, usize)>,
    /// 视图还没有尺寸时推迟的“滚动到可见”。
    pending_reveal: Option<ItemId>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<ExplorerEvent> for ExplorerView {}

/// 文件图标（codicon 名称）：按扩展名区分常见类型；Seti 文件图标主题属于 R68。
pub fn file_icon(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "cs" | "rs" | "js" | "ts" | "jsx" | "tsx" | "py" | "c" | "cc" | "cpp" | "h" | "hpp"
        | "java" | "go" | "lua" | "sh" | "ps1" | "bat" | "cmd" | "shader" | "cginc" | "hlsl"
        | "glsl" | "compute" | "xml" | "yaml" | "yml" | "toml" | "uxml" | "uss" | "asmdef"
        | "asmref" | "csproj" | "sln" | "html" | "css" | "ini" | "cfg" => "file-code",
        "json" => "json",
        "md" | "markdown" | "txt" | "log" | "rtf" => "file-text",
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "psd" | "tga" | "tif" | "tiff" | "webp"
        | "svg" | "ico" | "exr" | "hdr" | "wav" | "mp3" | "ogg" | "mp4" | "mov" | "avi" => {
            "file-media"
        }
        "pdf" => "file-pdf",
        "zip" | "7z" | "rar" | "gz" | "tar" | "unitypackage" => "file-zip",
        "dll" | "exe" | "so" | "bin" | "pdb" | "obj" | "a" | "lib" => "file-binary",
        _ => "file",
    }
}

fn color_of(theme: &dyn Theme, key: &str) -> Option<Hsla> {
    theme.color(key).ok().flatten().map(hsla)
}

fn row_renderer(state: Rc<RefCell<RenderState>>, theme: Arc<dyn Theme>) -> RowRenderer {
    Rc::new(move |ctx: &RowContext, window: &mut Window, cx: &mut App| {
        use crate::list_widget::tree::RowKind;
        if ctx.kind != RowKind::Item {
            return default_row(ctx, window, cx);
        }
        let row = ctx.row;
        let state = state.borrow();
        // 读取失败的提示行：一句话原因，不是文件。
        if is_error_id(&row.id.0) {
            return div()
                .pl(px(ICON + 4.0))
                .text_color(
                    ctx.colors
                        .deemphasized_fg
                        .unwrap_or_else(gpui::transparent_black),
                )
                .child(SharedString::from(row.label.to_string()))
                .into_any_element();
        }
        let is_dir = row.expandable;
        let paths = if is_dir {
            chain_paths(&row.id.0, &row.label)
        } else {
            vec![row.id.0.to_string()]
        };
        let decoration = paths
            .last()
            .and_then(|p| WorkspacePath::new(p).ok())
            .and_then(|p| decorate(&state.providers, &p, is_dir));
        let name_color = decoration
            .and_then(|d| d.color)
            .filter(|_| state.colors)
            .and_then(|c| {
                color_of(
                    &*theme,
                    match c {
                        DecorationColor::Error => "list.errorForeground",
                        DecorationColor::Warning => "list.warningForeground",
                        DecorationColor::Deemphasized => "list.deemphasizedForeground",
                    },
                )
            });
        let mut left = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .flex_1()
            .overflow_hidden();
        let twistie = if is_dir {
            if row.expanded {
                "chevron-down"
            } else {
                "chevron-right"
            }
        } else {
            ""
        };
        left = left.child(codicon(twistie, px(ICON), None));
        let name = row.label.rsplit('/').next().unwrap_or(&row.label);
        let (icon, key) = if is_dir {
            (
                if row.expanded {
                    "folder-opened"
                } else {
                    "folder"
                },
                "symbolIcon.folderForeground",
            )
        } else {
            (file_icon(name), "symbolIcon.fileForeground")
        };
        left = left.child(codicon(icon, px(ICON), color_of(&*theme, key)));
        let segments = if is_dir {
            row.label.split('/').count()
        } else {
            1
        };
        let mut label = div().flex().items_center().min_w_0().overflow_hidden();
        if segments > 1 && ctx.highlights.is_empty() {
            // 紧凑文件夹：每一段单独显示，聚焦的段高亮（`home` / `end` / `left` / `right` 在段间移动）。
            let focused = state
                .segment
                .as_ref()
                .filter(|(id, _)| *id == row.id.0)
                .map(|(_, i)| *i);
            for (i, part) in row.label.split('/').enumerate() {
                if i > 0 {
                    label = label.child(SharedString::from("/"));
                }
                let mut segment = div()
                    .flex_shrink_0()
                    .child(SharedString::from(part.to_owned()));
                if ctx.focused && ctx.list_focused && focused == Some(i) {
                    segment = segment
                        .border_b_1()
                        .border_color(ctx.colors.focus_outline.unwrap_or_else(gpui::white));
                }
                label = label.child(segment);
            }
        } else {
            label = label.child(highlighted_label(ctx));
        }
        if let Some(color) = name_color {
            label = label.text_color(color);
        }
        left = left.child(label);
        let mut el = div().flex().items_center().w_full().child(left);
        if let (true, Some(Badge::Lock)) = (state.badges, decoration.and_then(|d| d.badge)) {
            el = el.child(codicon(
                "lock-small",
                px(ICON),
                color_of(&*theme, "list.deemphasizedForeground"),
            ));
        } else if let (true, Some(Badge::Count(n))) =
            (state.badges, decoration.and_then(|d| d.badge))
        {
            let mut badge = div()
                .flex_shrink_0()
                .pl(px(6.))
                .child(SharedString::from(n.to_string()));
            if let Some(color) = name_color {
                badge = badge.text_color(color);
            }
            el = el.child(badge);
        }
        el.into_any_element()
    })
}

impl ExplorerView {
    pub fn new(
        theme: Arc<dyn Theme>,
        strings: Arc<dyn Strings>,
        settings: &dyn Settings,
        cx: &mut Context<Self>,
    ) -> Self {
        let explorer = ExplorerSettings::read(settings);
        let render = Rc::new(RefCell::new(RenderState {
            providers: Vec::new(),
            badges: explorer.badges,
            colors: explorer.colors,
            segment: None,
        }));
        let options = ListOptions::read(settings);
        let renderer = row_renderer(render.clone(), theme.clone());
        let list = cx.new(|cx| {
            let widget = ListWidget::tree(
                TreeModel::new(Arc::new(NoFolder)),
                RowHeight::Fixed(ROW_HEIGHT),
                options,
            );
            let mut view = ListView::new(widget, theme, strings.clone(), cx);
            view.set_renderer(renderer);
            view
        });
        let subscriptions = vec![
            cx.subscribe(&list, |this, _, event: &ListEvent, cx| {
                this.list_event(event, cx)
            }),
            cx.subscribe(&list, |_, _, event: &ListViewEvent, cx| {
                if let ListViewEvent::EmptyAction(command) = event {
                    cx.emit(ExplorerEvent::Command(command.clone()));
                }
            }),
        ];
        let mut view = Self {
            list,
            render,
            source: None,
            workspace: None,
            settings: explorer,
            strings,
            problems: Arc::new(ProblemDecorations::default()),
            segment: None,
            pending_reveal: None,
            _subscriptions: subscriptions,
        };
        view.show_empty_state(true, cx);
        view
    }

    pub fn settings(&self) -> &ExplorerSettings {
        &self.settings
    }

    /// 问题面板（R60）把错误 / 警告数写进这里；合并前为空。
    pub fn problems(&self) -> &Arc<ProblemDecorations> {
        &self.problems
    }

    pub fn list(&self) -> &Entity<ListView> {
        &self.list
    }

    pub fn has_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    pub fn source(&self) -> Option<&Arc<ExplorerSource>> {
        self.source.as_ref()
    }

    fn show_empty_state(&mut self, on: bool, cx: &mut Context<Self>) {
        let empty = on.then(|| EmptyState {
            message: Message::key("explorer.empty"),
            actions: vec![(
                Message::key("explorer.openFolder"),
                registry::OPEN_FOLDER.into(),
            )],
        });
        self.list.update(cx, |list, cx| {
            list.set_empty_state(empty);
            cx.notify();
        });
    }

    /// 展开着的目录（链尾路径），重建树之后按路径还原。
    fn expanded_paths(&self, cx: &App) -> Vec<WorkspacePath> {
        let list = self.list.read(cx);
        let rows = list.widget.display_rows(0..list.widget.len());
        rows.iter()
            .filter(|r| r.expandable && r.expanded)
            .filter_map(|r| chain_paths(&r.id.0, &r.label).pop())
            .filter_map(|p| WorkspacePath::new(&p).ok())
            .collect()
    }

    /// 换工作区（首次、`files.exclude` 变化后重新扫描，或 `None` = 没有打开文件夹）。
    /// 展开状态、选中与焦点按路径保留。
    pub fn set_workspace(&mut self, workspace: Option<Arc<dyn Workspace>>, cx: &mut Context<Self>) {
        let expanded = self.expanded_paths(cx);
        let Some(workspace) = workspace else {
            self.source = None;
            self.workspace = None;
            self.render.borrow_mut().providers.clear();
            self.list.update(cx, |list, cx| {
                list.widget
                    .update_tree(|t| *t = TreeModel::new(Arc::new(NoFolder)));
                list.widget.take_events();
                cx.notify();
            });
            self.show_empty_state(true, cx);
            return;
        };
        let source = Arc::new(ExplorerSource::new(
            workspace.clone(),
            self.settings.clone(),
            self.strings.clone(),
        ));
        self.render.borrow_mut().providers = vec![
            Arc::new(ReadOnlyDecorations(workspace.clone())),
            self.problems.clone(),
        ];
        self.source = Some(source.clone());
        self.workspace = Some(workspace);
        self.show_empty_state(false, cx);
        self.list.update(cx, |list, cx| {
            list.widget
                .update_tree(|t| *t = TreeModel::new(source.clone()));
            for path in &expanded {
                if let Some(plan) = source.reveal_plan(path) {
                    for id in plan.expand.iter().chain(std::iter::once(&plan.target)) {
                        let _ = list.widget.expand_id(id, true);
                    }
                }
            }
            list.widget.take_events();
            cx.notify();
        });
        cx.notify();
    }

    /// 设置变化：排序、紧凑文件夹等影响数据的设置重建树，徽标与颜色只影响绘制。
    pub fn reload_settings(&mut self, settings: &dyn Settings, cx: &mut Context<Self>) {
        let new = ExplorerSettings::read(settings);
        let rebuild = new.compact_folders != self.settings.compact_folders
            || new.sort != self.settings.sort
            || new.lexicographic != self.settings.lexicographic
            || new.reverse != self.settings.reverse;
        {
            let mut render = self.render.borrow_mut();
            render.badges = new.badges;
            render.colors = new.colors;
        }
        self.settings = new;
        let options = ListOptions::read(settings);
        self.list
            .update(cx, |list, cx| list.set_options(options, cx));
        if rebuild {
            let workspace = self.workspace.clone();
            self.set_workspace(workspace, cx);
        }
        cx.notify();
    }

    /// 这批设置键里有没有资源管理器关心的。
    pub fn cares_about(keys: &std::collections::BTreeSet<Arc<str>>) -> bool {
        keys.iter().any(|k| {
            SETTING_KEYS.contains(&&**k)
                || k.starts_with("workbench.list.")
                || k.starts_with("workbench.tree.")
        })
    }

    /// 文件事件的增量应用：只重新装填受影响的节点，展开状态与选中项按 ID 保留。
    pub fn files_changed(&mut self, events: &[FileEvent], cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        match source.plan_refresh(events) {
            Refresh::Full => {
                let workspace = self.workspace.clone();
                self.set_workspace(workspace, cx);
            }
            Refresh::Nodes(nodes) if nodes.is_empty() => {}
            Refresh::Nodes(nodes) => {
                self.list.update(cx, |list, cx| {
                    list.widget.update_tree(|tree| {
                        for node in &nodes {
                            let id = node.as_ref().map(|p| ItemId(p.as_str().into()));
                            let items = source.items(node.as_ref());
                            // 节点已被丢弃（父目录同时被刷新或删除）：忽略。
                            let _ = tree.set_children(id.as_ref(), items);
                        }
                    });
                    list.widget.take_events();
                    cx.notify();
                });
            }
        }
        cx.notify();
    }

    /// 定位：展开路径上的目录并选中该文件（紧凑文件夹按路径解析到正确的节点）。
    /// `scroll` 为 false（`explorer.autoReveal = focusNoScroll`）时只选中不滚动。
    /// 目标已是唯一选中项时什么也不做（由树自己打开的文件不会再抖动焦点）。
    pub fn reveal(&mut self, path: &WorkspacePath, scroll: bool, cx: &mut Context<Self>) -> bool {
        let Some(plan) = self.source.as_ref().and_then(|s| s.reveal_plan(path)) else {
            return false;
        };
        let mut pending = None;
        let changed = self.list.update(cx, |list, cx| {
            for id in &plan.expand {
                let _ = list.widget.expand_id(id, true);
            }
            let Some(index) = list
                .widget
                .tree_model()
                .and_then(|t| t.index_of(&plan.target))
            else {
                return false;
            };
            let same = list.widget.selected_ids().as_ref() == [plan.target.clone()]
                && list.widget.focus() == Some(index);
            if !same {
                list.widget.select_ids(std::slice::from_ref(&plan.target));
                list.widget.set_focus(Some(index));
            }
            if scroll {
                if list.widget.viewport() > 0.0 {
                    list.widget.reveal(index);
                } else {
                    pending = Some(plan.target.clone());
                }
            }
            list.widget.take_events();
            cx.notify();
            !same
        });
        if changed {
            self.segment = None;
        }
        self.sync_segment(cx);
        if pending.is_some() {
            self.pending_reveal = pending;
        }
        cx.notify();
        true
    }

    /// 活动编辑器换了：设置允许且路径没有被 `explorer.autoRevealExclude` 排除时在树中定位。
    pub fn auto_reveal(&mut self, path: &WorkspacePath, cx: &mut Context<Self>) {
        let mode = self.settings.auto_reveal;
        if mode == AutoReveal::Off || self.settings.auto_reveal_exclude.is_excluded(path.as_str()) {
            return;
        }
        self.reveal(path, mode != AutoReveal::FocusNoScroll, cx);
    }

    /// 一个路径当前的装饰（徽标与颜色）：绘制与测试共用。
    pub fn decoration(
        &self,
        path: &WorkspacePath,
        is_dir: bool,
    ) -> Option<super::decorations::Decoration> {
        decorate(&self.render.borrow().providers, path, is_dir)
    }

    /// 可见行的 (缩进, 标签, 是否展开) —— 测试与调试用。
    pub fn rows(&self, cx: &App) -> Vec<(u32, String, bool)> {
        let list = self.list.read(cx);
        list.widget
            .display_rows(0..list.widget.len())
            .iter()
            .map(|r| (r.depth, r.label.to_string(), r.expanded))
            .collect()
    }

    /// 选中项的 ID（工作区路径）。
    pub fn selected(&self, cx: &App) -> Vec<String> {
        self.list
            .read(cx)
            .widget
            .selected_ids()
            .iter()
            .map(|id| id.0.to_string())
            .collect()
    }

    /// 紧凑文件夹行内当前聚焦的段。
    pub fn focused_segment(&self, cx: &App) -> usize {
        self.focused_row(cx).map_or(0, |row| self.segment_of(&row))
    }

    /// 某行当前聚焦的段（不是记录的那一行时从最后一段开始，与 VS Code 一致）。
    fn segment_of(&self, row: &Row) -> usize {
        let last = Self::segments(row) - 1;
        match &self.segment {
            Some((id, at)) if *id == row.id.0 => (*at).min(last),
            _ => last,
        }
    }

    /// 焦点所在的行。
    pub fn focused_row(&self, cx: &App) -> Option<Row> {
        let list = self.list.read(cx);
        list.widget.focus().and_then(|i| list.widget.row(i))
    }

    /// 聚焦行的路径（紧凑文件夹：当前聚焦段的目录）。
    pub fn focused_path(&self, cx: &App) -> Option<WorkspacePath> {
        let row = self.focused_row(cx)?;
        if is_error_id(&row.id.0) {
            return None;
        }
        if !row.expandable {
            return WorkspacePath::new(&row.id.0).ok();
        }
        let paths = chain_paths(&row.id.0, &row.label);
        let at = self.segment_of(&row).min(paths.len() - 1);
        WorkspacePath::new(&paths[at]).ok()
    }

    fn segments(row: &Row) -> usize {
        if row.expandable {
            row.label.split('/').count()
        } else {
            1
        }
    }

    /// 把当前段写回渲染状态。
    fn sync_segment(&mut self, cx: &mut Context<Self>) {
        let state = self
            .focused_row(cx)
            .map(|row| (row.id.0.clone(), self.segment_of(&row)));
        self.render.borrow_mut().segment = state;
        self.list.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn list_event(&mut self, event: &ListEvent, cx: &mut Context<Self>) {
        match event {
            ListEvent::Open {
                ids,
                preserve_focus,
                pinned,
                side_by_side,
            } => {
                for id in ids.iter().filter(|id| !is_error_id(&id.0)) {
                    if let Ok(path) = WorkspacePath::new(&id.0) {
                        cx.emit(ExplorerEvent::Open {
                            path,
                            preserve_focus: *preserve_focus,
                            pinned: *pinned,
                            side: *side_by_side,
                        });
                    }
                }
            }
            // 焦点换到另一行：紧凑文件夹从最后一段开始（与 VS Code 一致）。
            ListEvent::FocusChanged(_) => {
                self.segment = None;
                self.sync_segment(cx);
            }
            _ => {}
        }
    }

    /// 命令（键位服务分发的 action，或宿主转发）。
    pub fn command(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            registry::OPEN_AND_PASS_FOCUS => self.open_focused(false, true, false, cx),
            registry::OPEN_TO_SIDE => self.open_focused(false, true, true, cx),
            registry::OPEN_PRESERVE_FOCUS => self.open_focused(true, false, false, cx),
            registry::FIRST_COMPRESSED
            | registry::LAST_COMPRESSED
            | registry::NEXT_COMPRESSED
            | registry::PREVIOUS_COMPRESSED => {
                let Some(row) = self.focused_row(cx) else {
                    return;
                };
                let last = Self::segments(&row) - 1;
                let at = self.segment_of(&row);
                let next = match id {
                    registry::FIRST_COMPRESSED => 0,
                    registry::LAST_COMPRESSED => last,
                    registry::NEXT_COMPRESSED => (at + 1).min(last),
                    _ => at.saturating_sub(1),
                };
                self.segment = Some((row.id.0.clone(), next));
                self.sync_segment(cx);
            }
            _ => {}
        }
    }

    fn open_focused(
        &mut self,
        preserve_focus: bool,
        pinned: bool,
        side: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.focused_row(cx) else {
            return;
        };
        if row.expandable || is_error_id(&row.id.0) {
            return;
        }
        if let Ok(path) = WorkspacePath::new(&row.id.0) {
            cx.emit(ExplorerEvent::Open {
                path,
                preserve_focus,
                pinned,
                side,
            });
        }
    }

    /// VS Code 上下文键：`filesExplorerFocus` 等（焦点在树内时生效）。
    fn key_context(&self, cx: &App) -> KeyContext {
        let mut k = KeyContext::default();
        k.add("ExplorerView");
        k.add(ck::FILES_EXPLORER_FOCUS);
        k.add(ck::VIEWLET_FOCUS);
        k.add(ck::FOLDERS_VIEW_VISIBLE);
        // 单文件夹工作区不显示根行，所以 `explorerResourceIsRoot` 恒为 false（不加入上下文）。
        if let Some(row) = self.focused_row(cx) {
            if row.expandable {
                k.add(ck::RESOURCE_IS_FOLDER);
            }
            let segments = Self::segments(&row);
            if segments > 1 {
                let at = self.segment_of(&row);
                k.add(ck::COMPRESSED_FOCUS);
                if at == 0 {
                    k.add(ck::COMPRESSED_FIRST_FOCUS);
                }
                if at == segments - 1 {
                    k.add(ck::COMPRESSED_LAST_FOCUS);
                }
            }
            let readonly = self
                .focused_path(cx)
                .zip(self.workspace.as_ref())
                .and_then(|(path, ws)| ws.rules(&path).ok())
                .is_some_and(|rules| !rules.read_only.is_empty());
            if readonly {
                k.add(ck::RESOURCE_READONLY);
            }
        }
        k
    }
}

impl Focusable for ExplorerView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.list.focus_handle(cx)
    }
}

impl Render for ExplorerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(id) = self.pending_reveal.take() {
            let ready = self.list.update(cx, |list, _| {
                let at = list.widget.tree_model().and_then(|t| t.index_of(&id));
                match at {
                    Some(i) if list.widget.viewport() > 0.0 => {
                        list.widget.reveal(i);
                        true
                    }
                    Some(_) => false,
                    None => true,
                }
            });
            if !ready {
                self.pending_reveal = Some(id);
            }
        }
        let context = self.key_context(cx);
        div()
            .size_full()
            .key_context(context)
            .on_action(
                cx.listener(|this, action: &ExplorerAction, _, cx| this.command(action.0, cx)),
            )
            .child(self.list.clone())
    }
}
