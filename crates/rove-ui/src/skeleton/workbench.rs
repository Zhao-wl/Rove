use super::{appearance, text::FileDocuments};
use crate::{
    code_view::{
        self, CodeView, CodeViewEvent, CodeViewServices, Reveal, ScrollPosition, Selection,
    },
    documents::{EditorGroupId, OpenDocuments, OpenOptions, OpenSource},
    editors::{
        actions as editor_actions,
        settings::{EditorKind, EditorSettings},
        syntax::SyntaxJob,
        Axis, Content, Direction, EditorService, Layout, OpenJob, Tab, TabId, ViewState,
    },
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
    div, point, prelude::*, px, relative, rgba, size, uniform_list, AnyElement, App, Application,
    Bounds, Context, Entity, FocusHandle, Focusable, Hsla, IntoElement, Render, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};
#[cfg(test)]
use rove_base::Line;
use rove_base::{
    Cancellation, DocumentId, Error, Resource, Result as CoreResult, TextRange, Value, Version,
    WorkspacePath,
};
use rove_buffer::{Document, ReadOnlyReason};
use rove_commands::{center::CommandCenter, CommandExecutor, CommandRegistry, Invocation};
use rove_config::{Key, Scope, Settings, SettingsChange};
use rove_i18n::{Message, StringTable, Strings};
use rove_syntax::highlight::{SyntaxDocuments, TreeSitterSyntax};
use rove_theme::{Color, Theme};
use rove_workspace::{EntryKind, FileEvent, Workspace};
// #36：C# 索引生命周期与状态。
use crate::index_status::{self, IndexStatusController, Reaction};
use rove_lang_cs::lifecycle::{
    IndexLifecycle, IndexStatus, LifecycleConfig, ProcessBuilder, SourceChange,
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

mod editor_panes;
#[cfg(test)]
mod editor_tests;
#[cfg(test)]
mod find_tests;
mod outline_host;
#[cfg(test)]
mod outline_tests;
mod quick_access_host;
#[cfg(test)]
mod quick_access_tests;
// #40：C# 导航交互（转到定义 / 实现、悬停、Ctrl+点击、语义同名高亮）。
#[cfg(test)]
mod nav_cases_tests;
mod nav_host;
#[cfg(test)]
mod nav_tests;
#[cfg(test)]
#[path = "workbench_readonly_tests.rs"]
mod readonly_tests;
#[cfg(test)]
mod syntax_tests;
// 代码视图（#19）的字号与行高来自 editor.fontSize / editor.lineHeight，
// 窗口缩放（#16）在 `zoom_changed` 里乘到选项上；骨架不再自己定义常量。
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

/// What a code view currently displays: a tab's document at one text version.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Shown {
    tab: TabId,
    document: DocumentId,
    version: Version,
}

/// One code view of an editor group. Syntax colouring, bracket matching, occurrences and
/// folding live in the code view itself (#20); the host only hands it snapshots and state.
struct CodeSlot {
    code: Entity<CodeView>,
    /// #21：包住代码视图并在右上角浮出查找条。
    find: Entity<crate::find::host::FindHost>,
    shown: Option<Shown>,
    _events: gpui::Subscription,
}

/// Each editor group owns one code view; the in-group side-by-side editor
/// (`splitEditorInGroup`) adds a second one for the same tab.
struct GroupViews {
    primary: CodeSlot,
    secondary: Option<CodeSlot>,
}

/// See [`Workbench::read_only_of`].
fn read_only_for(
    overrides: &BTreeMap<DocumentId, bool>,
    document: &Document,
) -> Arc<[ReadOnlyReason]> {
    let base = code_view::presented_read_only(&document.metadata.read_only);
    let huge = base.contains(&ReadOnlyReason::HugeFile);
    match overrides.get(&document.id) {
        Some(false) if !huge => Arc::from([]),
        Some(true) if base.is_empty() => Arc::from([ReadOnlyReason::Session]),
        _ => base.into(),
    }
}

fn selection_of(state: &ViewState) -> Selection {
    match &state.selection {
        Some(range) if range.start() == state.cursor => Selection::new(range.end(), range.start()),
        Some(range) => Selection::new(range.start(), range.end()),
        None => Selection::caret(state.cursor),
    }
}

fn apply_zoom(code: &mut CodeView, zoom: f64, cx: &mut Context<CodeView>) {
    code.reload_options(cx);
    let mut options = code.options().clone();
    options.font_size *= zoom;
    // 0 表示按字号自动计算，乘以系数仍是 0。
    options.line_height_setting *= zoom;
    code.set_options(options, cx);
}

/// 骨架窗口：工作台（标题栏、活动栏、侧边栏、面板、状态栏、反馈）由 #16 的 `shell` 提供，
/// 编辑器组网格、标签与打开文档服务由 #17 的 `editors` 提供，每个组的正文是 #19 的代码视图。
pub struct Workbench {
    tree: Option<FileTree>,
    editor: EditorService,
    root: PathBuf,
    views: BTreeMap<EditorGroupId, GroupViews>,
    /// The group whose code view last received keyboard focus from the editor area.
    focus_group: Option<EditorGroupId>,
    trace_switch: bool,
    switch_started: Option<(TabId, Instant)>,
    theme: Arc<dyn Theme>,
    strings: StringTable,
    shell: Shell,
    /// #14 command table shared with the keybinding service (`crate::keys`).
    commands: Arc<CommandCenter>,
    focus: FocusHandle,
    /// 工作区扫描次数（缩放等界面操作不得触发重新扫描）。
    scans: u32,
    scan_failed: bool,
    workspace_events: Option<rove_base::Subscription>,
    started: Instant,
    first_frame: bool,
    /// #104：应用唯一的设置服务实例；工作台、主题、代码视图、打开服务与工作区扫描共用。
    settings: Arc<dyn Settings>,
    /// 同一个主题服务的具体类型：读取当前主题的明暗。
    theme_service: Arc<rove_theme::ThemeService>,
    settings_events: Option<rove_base::Subscription>,
    theme_events: Option<rove_base::Subscription>,
    /// #36：C# 索引的生命周期与状态栏条目。
    index: IndexStatusController,
    feedback_events: Option<rove_base::Subscription>,
    /// #26：大纲小节、面包屑与转到符号的状态（见 `outline_host.rs`）。
    outline: crate::outline::view::Outline,
    /// 最近一次扫描得到的工作区（面包屑的文件选择器列出目录条目）。
    workspace: Option<Arc<dyn Workspace>>,
    /// "Toggle read-only in this session" per document: `true` read-only, `false` writable.
    readonly_overrides: BTreeMap<DocumentId, bool>,
    /// #21：窗口内所有查找条共享的选项、上次的查找文本与历史。
    find_shared: crate::find::host::SharedFind,
    find_strings: Arc<dyn Strings>,
    /// #24：快速打开与命令面板的状态（见 `quick_access_host.rs`）。
    quick: quick_access_host::QuickAccess,
    /// #40：C# 导航（查询路由、悬停、同名高亮、符号导航模式，见 `nav_host.rs`）。
    nav: nav_host::Nav,
}
impl Workbench {
    /// 布局按工作区持久化到用户数据目录；单元测试不写用户目录。
    /// 使用只读的 VS Code 默认设置（测试与嵌入用）；应用入口用 [`Workbench::with_settings`]。
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
        let settings = crate::app_settings::defaults_only();
        Self::build(
            root,
            started,
            store,
            settings,
            EditorSettings::default(),
            cx,
        )
    }
    /// 用应用的设置服务建窗口（#104）：同一个实例传给工作台、主题、代码视图、打开服务与
    /// 工作区扫描，并订阅它的变更做热加载。
    pub fn with_settings(
        root: PathBuf,
        started: Instant,
        store: LayoutStore,
        settings: Arc<dyn Settings>,
        cx: &mut Context<Self>,
    ) -> CoreResult<Self> {
        let editor = EditorSettings::read(&*settings)?;
        let mut view = Self::build(root, started, store, settings, editor, cx);
        // #36：应用入口按设置启动 C# 索引（测试入口 `new` 不启动，条目不显示）。
        view.start_index(cx);
        Ok(view)
    }
    fn build(
        root: PathBuf,
        started: Instant,
        store: LayoutStore,
        services: Arc<dyn Settings>,
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
                        // Ready is reported once the location is in the group's code view.
                        view.sync_views(cx);
                        view.editor.flush_ready();
                        cx.notify();
                    }
                });
            })
            .detach();
        };
        let syntax_owner = cx.entity().downgrade();
        let syntax_app = cx.to_async();
        let syntax_background = cx.background_executor().clone();
        let schedule_syntax = move |job: SyntaxJob| {
            let SyntaxJob {
                document,
                generation,
                task,
            } = job;
            let analysis = syntax_background.spawn(task);
            let owner = syntax_owner.clone();
            syntax_app
                .spawn(async move |cx| {
                    let result = analysis.await;
                    let _ = owner.update(cx, |view, cx| {
                        let changed = result
                            .as_ref()
                            .ok()
                            .and_then(|update| update.changed_lines.clone());
                        if view.editor.finish_syntax(document, generation, result) {
                            view.refresh_syntax(document, changed, cx);
                        }
                    });
                })
                .detach();
        };
        let find_history = store.find_history_path();
        let shell = Shell::new(
            &root,
            appearance::contributions().unwrap(),
            services.clone(),
            store,
            cx,
        );
        // #10 的主题服务同时供给骨架与代码视图（#19）；登记表补上代码视图用到的 P1 颜色键（#20）。
        let theme_service = rove_theme::ThemeService::with_catalog(
            services.clone(),
            rove_theme::ThemeCatalog::builtin(),
            Arc::new(code_view::colors::theme_registry()),
        );
        let theme: Arc<dyn Theme> = theme_service.clone();
        let size = settings.size;
        let outline = Self::new_outline(theme.clone(), &services, cx);
        let nav = nav_host::Nav::new(
            &root,
            &*services,
            theme.as_ref(),
            theme_service.current().kind(),
        );
        let mut view = Self {
            tree: None,
            editor: EditorService::new(
                Arc::new(FileDocuments::new(root.clone(), services.clone()).with_size_policy(size)),
                // #94: one baseline per open document, configured by the shared settings.
                Arc::new(SyntaxDocuments::new(
                    TreeSitterSyntax::default().with_settings(services.clone()),
                )),
                settings,
                schedule,
                schedule_syntax,
            ),
            root: root.clone(),
            views: BTreeMap::new(),
            focus_group: None,
            trace_switch: std::env::var_os("ROVE_EDITOR_SWITCH_TRACE").is_some(),
            switch_started: None,
            theme,
            strings: appearance::strings().unwrap(),
            shell,
            commands: {
                crate::keys::init(cx);
                crate::keys::keys(cx).commands.clone()
            },
            focus: cx.focus_handle(),
            scans: 0,
            scan_failed: false,
            workspace_events: None,
            started,
            first_frame: false,
            settings: services,
            theme_service,
            settings_events: None,
            theme_events: None,
            index: IndexStatusController::default(),
            feedback_events: None,
            outline,
            workspace: None,
            readonly_overrides: BTreeMap::new(),
            find_shared: crate::find::host::FindShared::new(find_history),
            find_strings: Arc::new(appearance::strings().unwrap()),
            quick: Default::default(),
            nav,
        };
        // #24：快速打开、命令面板与转到行已实现：菜单与命令中心据此显示。
        for (id, _, _) in crate::quick_access::registry::COMMANDS {
            view.shell.mark_implemented(*id);
        }
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
                    view.commands.registry().get(command).is_some(),
                    "{id}: 未登记的命令 {command}"
                );
            }
        }
        view.shell
            .set_theme_kind(view.theme_service.current().kind());
        view.watch_settings(cx);
        view.watch_theme(cx);
        view.watch_feedback(cx);
        view.sync_views(cx);
        view.scan(root, cx);
        view
    }
    /// #104 热加载：设置服务（文件监听线程）发布的变更合并后回到 UI 线程处理。
    fn watch_settings(&mut self, cx: &mut Context<Self>) {
        let (send, receive) = async_channel::unbounded::<SettingsChange>();
        self.settings_events = Some(self.settings.changes().subscribe(Arc::new(move |change| {
            let _ = send.try_send(change);
        })));
        cx.spawn(async move |this, cx| {
            while let Ok(first) = receive.recv().await {
                let mut keys: BTreeSet<Arc<str>> = first.keys.iter().cloned().collect();
                while let Ok(more) = receive.try_recv() {
                    keys.extend(more.keys.iter().cloned());
                }
                if this
                    .update(cx, |view, cx| view.settings_changed(&keys, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
    fn settings_changed(&mut self, keys: &BTreeSet<Arc<str>>, cx: &mut Context<Self>) {
        // 工作台按键现读设置，只需放弃被外部改动的本窗口临时值。
        self.shell.settings_changed(keys);
        if keys.iter().any(|k| k.starts_with("workbench.editor.")) {
            match EditorSettings::read(&*self.settings) {
                Ok(settings) => self.editor.settings = settings,
                Err(e) => eprintln!("settings: 编辑器设置无效，沿用旧值：{e}"),
            }
        }
        self.outline_settings_changed(keys, cx);
        // #40：editor.gotoLocation.* / editor.hover.* / editor.occurrencesHighlight* 变化。
        self.nav_settings_changed(keys, cx);
        // 代码视图重读 editor.* 选项（字号、行高、字体……），再乘窗口缩放。
        let zoom = self.shell.zoom() as f64;
        for code in self.code_views() {
            code.update(cx, |code, cx| apply_zoom(code, zoom, cx));
        }
        // #21：editor.find.* 变化。
        if keys
            .iter()
            .any(|k| k.starts_with("editor.find.") || &**k == "editor.wordSeparators")
        {
            for find in self.find_hosts() {
                find.update(cx, |find, cx| find.reload_settings(cx));
            }
        }
        // #36：rove.index.enabled / cacheDir 变化：关闭旧的生命周期，按新设置重新打开。
        if self.index.lifecycle().is_some()
            && keys.iter().any(|k| LifecycleConfig::is_lifecycle_key(k))
        {
            self.start_index(cx);
        }
        // 排除与只读规则变化：按新规则重新扫描工作区。
        let mut workspace_keys = rove_config::SettingRegistry::default();
        if rove_workspace::register_settings(&mut workspace_keys).is_ok()
            && keys.iter().any(|k| workspace_keys.get(k).is_some())
        {
            self.scan(self.root.clone(), cx);
        }
        cx.notify();
    }
    /// 主题服务换了主题（`workbench.colorTheme` 等变化后在后台重新加载）：刷新明暗与代码视图配色。
    fn watch_theme(&mut self, cx: &mut Context<Self>) {
        let (send, receive) = async_channel::unbounded::<Version>();
        self.theme_events = Some(self.theme.changes().subscribe(Arc::new(move |v| {
            let _ = send.try_send(v);
        })));
        cx.spawn(async move |this, cx| {
            while receive.recv().await.is_ok() {
                while receive.try_recv().is_ok() {}
                if this.update(cx, |view, cx| view.theme_changed(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        let kind = self.theme_service.current().kind();
        self.shell.set_theme_kind(kind);
        // The code view recolours tokens (colour and fontStyle) itself; stale styles stay on
        // screen until the new ones are ready.
        for code in self.code_views() {
            let theme = self.theme.clone();
            code.update(cx, |code, cx| code.set_theme(theme, kind, cx));
        }
        for find in self.find_hosts() {
            let theme = self.theme.clone();
            find.update(cx, |find, cx| find.set_theme(&theme, kind, cx));
        }
        // #40：悬停卡片、链接与语义同名高亮的颜色。
        self.nav_theme_changed(cx);
        cx.notify();
    }
    fn find_hosts(&self) -> Vec<Entity<crate::find::host::FindHost>> {
        self.views
            .values()
            .flat_map(|views| std::iter::once(&views.primary).chain(&views.secondary))
            .map(|slot| slot.find.clone())
            .collect()
    }
    fn code_views(&self) -> Vec<Entity<CodeView>> {
        self.views
            .values()
            .flat_map(|views| std::iter::once(&views.primary).chain(&views.secondary))
            .map(|slot| slot.code.clone())
            .collect()
    }
    fn label(&self, key: &str, args: &[(&str, String)]) -> String {
        appearance::message(&self.strings, key, args)
    }
    /// 主题没有给这个键设色时（各主题覆盖的键不同，例如 Dark+ 没有 `editorGroupHeader.tabsBorder`）
    /// 先用工作台登记的默认值，再不行就透明，与 VS Code 一致；热切换主题后也不会因缺色崩溃。
    fn ui_color(&self, key: &str) -> Hsla {
        self.shell
            .color(self.theme.as_ref(), key)
            .unwrap_or_else(gpui::transparent_black)
    }
    fn scan(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.scans += 1;
        // SKELETON: build the workspace through the #15 implementation. The explorer still waits
        // for the initial snapshot before building its rows; streaming (root in the first frame)
        // is available from `rove_workspace::Folder::start` and belongs to #23's explorer.
        let settings = self.settings.clone();
        let task = cx.background_executor().spawn(async move {
            let config = rove_workspace::WorkspaceConfig::from_settings(&*settings)?;
            let folder = rove_workspace::Folder::scan_root(root, config)?;
            Ok(folder as Arc<dyn Workspace>)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                let tree = result.and_then(|workspace| {
                    view.watch_workspace(&workspace, cx);
                    view.workspace = Some(workspace.clone());
                    // #24：后台建快速打开的文件索引。
                    view.quick_workspace_ready(cx);
                    // #40：后台建 C# 的 L1 快速声明索引（导航的近似回退）。
                    view.nav_workspace_ready(cx);
                    file_tree(workspace)
                });
                match tree {
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
    /// `workbench.editor.closeOnFileDelete`: close tabs whose file was removed or renamed away.
    /// 总是订阅，事件到达时再看设置：`closeOnFileDelete` 可以热加载（#104）。
    fn watch_workspace(&mut self, workspace: &Arc<dyn Workspace>, cx: &mut Context<Self>) {
        let (send, receive) = async_channel::unbounded();
        self.workspace_events = Some(workspace.changes().subscribe(Arc::new(move |batch| {
            let _ = send.try_send(batch);
        })));
        cx.spawn(async move |this, cx| {
            while let Ok(first) = receive.recv().await {
                let mut removed = BTreeSet::new();
                let mut changes = Vec::new();
                // #24：快速打开的文件列表按事件增量更新。
                let mut quick = Vec::new();
                // #40：L1 快速声明索引的增量更新。
                let mut nav = Vec::new();
                for batch in
                    std::iter::once(first).chain(std::iter::from_fn(|| receive.try_recv().ok()))
                {
                    quick.push((batch.revision.0, batch.events.clone()));
                    nav.extend(batch.events.iter().cloned());
                    changes.extend(batch.events.iter().map(index_change));
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
                        // #36：源文件变化标记过期，项目配置变化触发重建。
                        view.index_files_changed(&changes);
                        view.quick_files_changed(quick, cx);
                        view.nav_files_changed(&nav, cx);
                        if !view.editor.settings.close_on_delete {
                            return;
                        }
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
        let _ = crate::feedback::FeedbackService::dismiss(
            &mut self.shell.feedback,
            "command.placeholder",
        );
        let options = OpenOptions {
            source: OpenSource::Explorer,
            preserve_focus: self.editor.settings.explorer_preserve_focus,
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
            self.switch_started = self
                .editor
                .active()
                .current()
                .filter(|tab| previous != Some(tab.id) && tab.opened().is_some())
                .map(|tab| (tab.id, start));
        }
    }

    fn switch_tab(&mut self, group: EditorGroupId, tab: TabId) {
        let start = Instant::now();
        let previous = self.editor.active().active;
        self.editor.activate(group, tab, true);
        self.note_switch(previous, start);
    }

    /// Shared by the command palette and shortcut dispatch.
    pub fn command_registry(&self) -> &CommandRegistry {
        self.commands.registry()
    }

    /// 编辑器组与标签命令由 #17 的服务直接执行；其余命令交给 #14 的命令表（`CommandCenter`），
    /// 已登记未实现的命令把标题作为状态栏临时消息显示（3 秒后消失）。
    fn command(&mut self, id: &str, cx: &mut Context<Self>) {
        self.run_invocation(
            Invocation {
                id: id.into(),
                args: Value::Null,
            },
            cx,
        )
    }

    fn run_invocation(&mut self, invocation: Invocation, cx: &mut Context<Self>) {
        let id = invocation.id.clone();
        let id = id.as_ref();
        let start = Instant::now();
        let previous = self.editor.active().active;
        if crate::index_status::is_index_command(id) {
            self.index_command(id, cx);
            return;
        }
        if self.commands.is_registered(id) && editor_actions::execute(&mut self.editor, id) {
            self.note_switch(previous, start);
            cx.notify();
            return;
        }
        // Registered but not implemented yet: the status bar shows the command title (the
        // placeholder titles say "尚未实现") instead of pretending it ran.
        let title = self
            .commands
            .registry()
            .get(id)
            .map(|c| c.title_key.clone());
        let executor: Arc<dyn CommandExecutor> = self.commands.clone();
        let task = executor.execute(invocation, Cancellation::default());
        let task = cx.background_executor().spawn(task);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let key = match (result, title) {
                (Ok(Value::String(key)), _) => Some(key),
                (Err(Error::Unsupported), title) => title,
                _ => None,
            };
            let _ = this.update(cx, |view, cx| {
                if let Some(key) = key {
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

    /// Chord feedback from the keybinding service: "waiting for the next key" / "not a command".
    fn chord_status(&mut self, status: &crate::keys::ChordStatus, cx: &mut Context<Self>) {
        use crate::keys::ChordStatus;
        let (key, keys) = match status {
            ChordStatus::Pending(keys) => ("keybindings.chordPending", keys),
            ChordStatus::Aborted(keys) => ("keybindings.chordAborted", keys),
            ChordStatus::Cleared => {
                let _ = crate::feedback::FeedbackService::dismiss(
                    &mut self.shell.feedback,
                    "keybindings.chord",
                );
                cx.notify();
                return;
            }
        };
        let message = Message {
            key: key.into(),
            args: Arc::from([("keys".into(), keys.as_ref().into())]),
        };
        let _ = shell::view::report(
            self,
            "keybindings.chord",
            Reason::Receipt,
            message,
            Arc::from([]),
            cx,
        );
    }

    // ------------------------------------------------------------- code views per group

    fn new_slot(&self, group: EditorGroupId, secondary: bool, cx: &mut Context<Self>) -> CodeSlot {
        let theme = self.theme.clone();
        let theme_kind = self.theme_service.current().kind();
        let settings = self.settings.clone();
        let zoom = self.shell.zoom() as f64;
        let code = cx.new(|cx| {
            let mut code = CodeView::new(
                CodeViewServices {
                    settings,
                    theme,
                    theme_kind,
                },
                cx,
            );
            if zoom != 1.0 {
                apply_zoom(&mut code, zoom, cx);
            }
            code
        });
        let events = cx.subscribe(&code, move |this, _, event: &CodeViewEvent, cx| {
            this.code_event(group, secondary, event, cx);
            // 面包屑与大纲“跟随光标”随光标更新（#26）。
            if matches!(event, CodeViewEvent::SelectionChanged { .. }) {
                cx.notify();
            }
        });
        let find = {
            let (code, shared) = (code.clone(), self.find_shared.clone());
            let (settings, strings) = (self.settings.clone(), self.find_strings.clone());
            let theme = self.theme.clone();
            cx.new(|cx| {
                crate::find::host::FindHost::new(
                    code, shared, settings, strings, &theme, theme_kind, cx,
                )
            })
        };
        CodeSlot {
            code,
            find,
            shown: None,
            _events: events,
        }
    }

    /// Writes the shown tab's live scroll and selection back to its `ViewState`.
    fn code_event(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        event: &CodeViewEvent,
        cx: &mut Context<Self>,
    ) {
        // #40：悬停、Ctrl+悬停 / 点击、语义同名高亮跟随指针、光标与文本。
        self.nav_code_event(group, secondary, event, cx);
        match event {
            CodeViewEvent::EditBlocked { position, reasons } => {
                return self.edit_blocked(group, secondary, *position, reasons, cx);
            }
            CodeViewEvent::ToggleReadonlyRequested => {
                return self.toggle_readonly_in_session(group, secondary, cx);
            }
            _ => {}
        }
        let Some(slot) = self.views.get(&group).and_then(|views| {
            if secondary {
                views.secondary.as_ref()
            } else {
                Some(&views.primary)
            }
        }) else {
            return;
        };
        let Some(shown) = slot.shown else { return };
        let Some(pane) = self.editor.groups.get_mut(&group) else {
            return;
        };
        if let CodeViewEvent::ViewportChanged(viewport) = event {
            if !secondary {
                pane.rows = (viewport.last_line.saturating_sub(viewport.first_line) + 1) as f64;
            }
        }
        let Some(tab) = pane.tab_mut(shown.tab) else {
            return;
        };
        let state = if secondary {
            match tab.secondary.as_mut() {
                Some(state) => state,
                None => return,
            }
        } else {
            &mut tab.view
        };
        match event {
            CodeViewEvent::SelectionChanged { selection, .. } => {
                state.cursor = selection.active;
                state.selection = if selection.is_empty() {
                    None
                } else {
                    TextRange::new(selection.start(), selection.end()).ok()
                };
            }
            CodeViewEvent::ViewportChanged(viewport) => {
                state.scroll = viewport.top;
                state.scroll_x = viewport.scroll_x;
            }
            CodeViewEvent::FoldsChanged(folds) => state.folds = folds.clone(),
            CodeViewEvent::TextChanged
            | CodeViewEvent::EditBlocked { .. }
            | CodeViewEvent::ToggleReadonlyRequested
            | CodeViewEvent::PointerMoved { .. }
            | CodeViewEvent::PointerLeft
            | CodeViewEvent::LinkClicked { .. } => {}
        }
    }

    /// The document a code view shows, if any.
    fn shown_document(&self, group: EditorGroupId, secondary: bool) -> Option<Arc<Document>> {
        let views = self.views.get(&group)?;
        let slot = if secondary {
            views.secondary.as_ref()?
        } else {
            &views.primary
        };
        let shown = slot.shown?;
        self.editor
            .groups
            .get(&group)?
            .tab(shown.tab)
            .and_then(Tab::opened)
            .map(|opened| opened.document.clone())
    }

    /// Read-only reasons shown for a document (UX_SPEC §3.4): huge file, metadata view, disk
    /// attribute and read-only rules, adjusted by "toggle read-only in this session".
    fn read_only_of(&self, document: &Document) -> Arc<[ReadOnlyReason]> {
        read_only_for(&self.readonly_overrides, document)
    }

    /// `workbench.action.files.toggleActiveEditorReadonlyInSession`: per document, never for
    /// huge files (they are read-only by construction).
    fn toggle_readonly_in_session(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.shown_document(group, secondary) else {
            return;
        };
        if document
            .metadata
            .read_only
            .contains(&ReadOnlyReason::HugeFile)
        {
            return;
        }
        let now = !self.read_only_of(&document).is_empty();
        self.readonly_overrides.insert(document.id, !now);
        self.refresh_read_only(cx);
    }

    /// Push each shown document's read-only state into its code views.
    fn refresh_read_only(&mut self, cx: &mut Context<Self>) {
        let slots: Vec<(EditorGroupId, bool)> = self
            .views
            .iter()
            .flat_map(|(g, v)| {
                std::iter::once((*g, false)).chain(v.secondary.as_ref().map(|_| (*g, true)))
            })
            .collect();
        for (group, secondary) in slots {
            let reasons = self
                .shown_document(group, secondary)
                .map(|d| self.read_only_of(&d))
                .unwrap_or_else(|| Arc::from([]));
            if let Some(code) = self.group_code(group, secondary).cloned() {
                code.update(cx, |code, cx| code.set_read_only(reasons, cx));
            }
        }
        cx.notify();
    }

    /// Typing into a read-only editor: a cursor hint with the reason (UX_SPEC §6.1 level 2).
    fn edit_blocked(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        position: rove_base::Position,
        reasons: &[ReadOnlyReason],
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.shown_document(group, secondary) else {
            return;
        };
        let reason = reasons
            .first()
            .map(|r| self.label(code_view::read_only_reason_key(r), &[]))
            .unwrap_or_default();
        let key = if reasons.contains(&ReadOnlyReason::HugeFile) {
            "codeView.readonly.cannotEdit"
        } else {
            "codeView.readonly.cannotEditToggle"
        };
        let message = Message {
            key: key.into(),
            args: Arc::from([("reason".into(), reason.into())]),
        };
        let _ = shell::view::report(
            self,
            "codeView.readonly",
            Reason::NoResult {
                document: document.id,
                position,
            },
            message,
            Arc::from([]),
            cx,
        );
    }

    /// Loads each group's active tab into that group's code view. The outgoing tab's state is
    /// already in its `ViewState` (see [`Self::code_event`]); the incoming tab's state is restored
    /// from its own, and a pending reveal from a completed open is applied last.
    fn sync_views(&mut self, cx: &mut Context<Self>) {
        let groups: Vec<_> = self.editor.groups.keys().copied().collect();
        self.views.retain(|group, _| groups.contains(group));
        for group in groups {
            if !self.views.contains_key(&group) {
                let primary = self.new_slot(group, false, cx);
                self.views.insert(
                    group,
                    GroupViews {
                        primary,
                        secondary: None,
                    },
                );
            }
            let side_by_side = self.editor.groups[&group]
                .current()
                .is_some_and(|tab| tab.secondary.is_some());
            match (side_by_side, self.views[&group].secondary.is_some()) {
                (true, false) => {
                    let slot = self.new_slot(group, true, cx);
                    self.views.get_mut(&group).unwrap().secondary = Some(slot);
                }
                (false, true) => self.views.get_mut(&group).unwrap().secondary = None,
                _ => {}
            }
            self.sync_slot(group, false, cx);
            if side_by_side {
                self.sync_slot(group, true, cx);
            }
        }
    }

    fn sync_slot(&mut self, group: EditorGroupId, secondary: bool, cx: &mut Context<Self>) {
        let Self {
            editor,
            views,
            readonly_overrides,
            ..
        } = self;
        let pane = editor.groups.get_mut(&group).unwrap();
        let views = views.get_mut(&group).unwrap();
        let slot = if secondary {
            views.secondary.as_mut().unwrap()
        } else {
            &mut views.primary
        };
        let tab = pane
            .current_mut()
            .filter(|tab| tab.kind == EditorKind::Text && tab.opened().is_some());
        let target = tab.as_ref().map(|tab| {
            let opened = tab.opened().unwrap();
            Shown {
                tab: tab.id,
                document: opened.document.id,
                version: opened.document.text.version(),
            }
        });
        let reveal = !secondary && tab.as_ref().is_some_and(|tab| tab.reveal);
        if slot.shown == target && !reveal {
            return;
        }
        let previous = std::mem::replace(&mut slot.shown, target);
        let Some(tab) = tab else {
            slot.code
                .update(cx, |code, cx| code.set_text(None, false, cx));
            return;
        };
        let opened = tab.opened().unwrap();
        let text = opened.document.text.clone();
        let language = opened.document.metadata.language.clone();
        let syntax = opened.syntax.clone();
        let level = opened.document.metadata.size;
        let document = opened.document.clone();
        let state = if secondary {
            tab.secondary.clone().unwrap_or_default()
        } else {
            tab.view.clone()
        };
        tab.reveal &= secondary;
        // A new snapshot of the document already shown keeps the live view state.
        let same = matches!((previous, target), (Some(p), Some(t)) if p.tab == t.tab && p.document == t.document);
        let changed = previous != target;
        let code = slot.code.clone();
        let read_only = read_only_for(readonly_overrides, &document);
        code.update(cx, |code, cx| {
            if changed {
                code.set_document_level(level, cx);
                code.set_text(Some(text), same, cx);
                code.set_language(Some(language), cx);
                code.set_syntax(syntax, None, cx);
                code.set_read_only(read_only, cx);
                if !same {
                    // Folds first: the saved scroll position is in screen lines.
                    code.set_folds(&state.folds, cx);
                    code.set_scroll_position(
                        ScrollPosition {
                            top: state.scroll,
                            x: state.scroll_x,
                        },
                        false,
                        cx,
                    );
                    if !reveal {
                        code.set_selection(selection_of(&state), None, cx);
                    }
                }
            }
            if reveal {
                code.set_selection(selection_of(&state), Some(Reveal::CenterIfOutside), cx);
            }
        });
    }

    /// A background analysis of `document` finished: hand the new snapshot to the code views
    /// showing it. `changed` is the analysis' changed-line set (new snapshot coordinates);
    /// only those lines are restyled, the rest keep their cached styles (#20).
    fn refresh_syntax(
        &mut self,
        document: DocumentId,
        changed: Option<Arc<[rove_base::LineRange]>>,
        cx: &mut Context<Self>,
    ) {
        let Self { editor, views, .. } = self;
        for slot in views
            .values()
            .flat_map(|views| std::iter::once(&views.primary).chain(&views.secondary))
        {
            let Some(shown) = slot.shown.filter(|shown| shown.document == document) else {
                continue;
            };
            let Some(opened) = editor
                .groups
                .values()
                .find_map(|pane| pane.tab(shown.tab))
                .and_then(Tab::opened)
                .filter(|opened| opened.document.text.version() == shown.version)
            else {
                continue;
            };
            let syntax = opened.syntax.clone();
            let changed = changed.clone();
            slot.code.update(cx, |code, cx| {
                code.set_syntax(syntax, changed.as_deref(), cx)
            });
        }
        cx.notify();
    }

    /// Whether a tab's document is presented read-only (lock icon on the tab).
    fn tab_read_only(&self, tab: &Tab) -> bool {
        tab.opened()
            .is_some_and(|opened| !self.read_only_of(&opened.document).is_empty())
    }

    /// Read-only reasons the focused code view shows (tests, status entries).
    pub fn active_read_only(&self, cx: &App) -> Vec<ReadOnlyReason> {
        self.code_view().read(cx).read_only().to_vec()
    }

    /// The focused group's code view (the active editor).
    pub fn code_view(&self) -> &Entity<CodeView> {
        &self.views[&self.editor.focused].primary.code
    }

    /// The code view of a group, if the group exists.
    fn group_code(&self, group: EditorGroupId, secondary: bool) -> Option<&Entity<CodeView>> {
        let views = self.views.get(&group)?;
        if secondary {
            views.secondary.as_ref().map(|slot| &slot.code)
        } else {
            Some(&views.primary.code)
        }
    }

    /// Moves keyboard focus into a group's code view (tab clicks, group focus changes).
    fn focus_group_code(&self, group: EditorGroupId, window: &mut Window, cx: &App) {
        match self.group_code(group, false) {
            Some(code) => window.focus(&code.focus_handle(cx)),
            None => window.focus(&self.focus),
        }
    }

    /// 状态栏左侧最显眼的文字：临时消息优先，否则是骨架的文件状态条目（测试用）。
    #[cfg(test)]
    fn status(&self) -> String {
        shell::view::status_message_text(self).unwrap_or_else(|| self.file_status())
    }
    // SKELETON(status): fixed filename/line-count/encoding content; replaced by #27.
    fn file_status(&self) -> String {
        if self.scan_failed {
            return self.label("workbench.scanFailed", &[]);
        }
        let current = self.editor.active().current();
        if current.is_some_and(Tab::show_progress) || self.tree.is_none() {
            return self.label("common.loading", &[]);
        }
        if let Some(tab) = current.filter(|tab| matches!(tab.content, Content::Failed(_))) {
            let path = match &tab.resource {
                Resource::Workspace(p) => p.as_str().to_string(),
                Resource::File(p) => p.display().to_string(),
                Resource::Virtual(p) => p.to_string(),
            };
            return self.label("workbench.failed", &[("path", path)]);
        }
        if let Some(o) = current.and_then(Tab::opened) {
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

// ------------------------------------------------------------- #36 C# 索引状态

/// #15 的文件事件 → 索引生命周期的变化（项目相对路径）。
fn index_change(event: &FileEvent) -> SourceChange {
    match event {
        FileEvent::Created(p) => SourceChange::Created(p.as_str().to_owned()),
        FileEvent::Changed(p) => SourceChange::Changed(p.as_str().to_owned()),
        FileEvent::Removed(p) => SourceChange::Removed(p.as_str().to_owned()),
        FileEvent::Renamed { from, to } => SourceChange::Renamed {
            from: from.as_str().to_owned(),
            to: to.as_str().to_owned(),
        },
        FileEvent::RescanRequired => SourceChange::Rescan,
    }
}

impl Workbench {
    /// 按设置（#104 的统一设置服务）启动索引生命周期。`rove.index.enabled = false` 时只显示
    /// “已禁用”，不创建缓存目录、不启动索引进程。替换旧的生命周期时旧的随之关闭。
    fn start_index(&mut self, cx: &mut Context<Self>) {
        let config = LifecycleConfig::from_settings(&*self.settings);
        let lifecycle = IndexLifecycle::start(
            self.root.clone(),
            config,
            Arc::new(ProcessBuilder::default()),
        );
        self.attach_index(lifecycle, cx);
    }

    /// 装上一个生命周期并订阅它的状态（测试用它注入假的索引进程）。
    pub(crate) fn attach_index(&mut self, lifecycle: IndexLifecycle, cx: &mut Context<Self>) {
        let (send, receive) = async_channel::unbounded::<IndexStatus>();
        let subscription = lifecycle.changes().subscribe(Arc::new(move |status| {
            let _ = send.try_send(status);
        }));
        let epoch = self.index.attach(Some(lifecycle), Some(subscription));
        // #40：查询路由的语义来源换成新的生命周期。
        self.nav_index_attached(cx);
        let current = self.index.status().clone();
        self.index_status_changed(epoch, current, cx);
        cx.spawn(async move |this, cx| {
            while let Ok(first) = receive.recv().await {
                let batch: Vec<_> = std::iter::once(first)
                    .chain(std::iter::from_fn(|| receive.try_recv().ok()))
                    .collect();
                let applied = this.update(cx, |view, cx| {
                    for status in batch {
                        view.index_status_changed(epoch, status, cx);
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// 状态变化：更新状态栏条目；进入失败时经反馈层报一次通知，恢复后收起。
    fn index_status_changed(&mut self, epoch: u64, status: IndexStatus, cx: &mut Context<Self>) {
        match self.index.apply(epoch, status) {
            Reaction::Notify(failure) => {
                let (key, reason, message, actions) =
                    index_status::failure_notification(&failure, &self.strings);
                let _ = shell::view::report(self, &key, reason, message, actions, cx);
            }
            Reaction::Recovered => {
                for key in index_status::failure_keys() {
                    let _ =
                        crate::feedback::FeedbackService::dismiss(&mut self.shell.feedback, &key);
                }
            }
            Reaction::None => {}
        }
        let entry = index_status::status_entry(
            self.index.status(),
            &self.strings,
            rove_lang_cs::lifecycle::meta::now_ms(),
        );
        let shell = &mut self.shell;
        match entry {
            Some(entry) => {
                let _ =
                    shell
                        .status_bar
                        .set(&shell.contributions, index_status::STATUS_ITEM, entry);
            }
            None => {
                shell.status_bar.remove(index_status::STATUS_ITEM);
            }
        }
        // #40：索引就绪 / 重建完成时静默刷新语义同名高亮。
        self.nav_index_changed(cx);
        cx.notify();
    }

    fn index_files_changed(&self, changes: &[SourceChange]) {
        if let Some(lifecycle) = self.index.lifecycle() {
            if !changes.is_empty() {
                lifecycle.files_changed(changes);
            }
        }
    }

    /// 通知按钮（“查看详情”“重试”）发出的动作就是命令 ID。
    fn watch_feedback(&mut self, cx: &mut Context<Self>) {
        use crate::feedback::{FeedbackEvent, FeedbackService};
        let (send, receive) = async_channel::unbounded::<FeedbackEvent>();
        self.feedback_events = Some(self.shell.feedback.events().subscribe(Arc::new(
            move |event| {
                let _ = send.try_send(event);
            },
        )));
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receive.recv().await {
                let FeedbackEvent::Action { key, action } = event else {
                    continue;
                };
                if !key.starts_with(index_status::FAILURE_KEY_PREFIX)
                    || !index_status::is_index_command(&action)
                {
                    continue;
                }
                if this
                    .update(cx, |view, cx| view.index_command(&action, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// `rove.index.rebuild` / `showDetails` / `openLog`。
    fn index_command(&mut self, id: &str, cx: &mut Context<Self>) {
        match id {
            index_status::REBUILD => {
                if let Some(lifecycle) = self.index.lifecycle() {
                    lifecycle.rebuild();
                }
            }
            index_status::SHOW_DETAILS => {
                let shell = &mut self.shell;
                shell.layout.panel_visible = true;
                shell.layout.active_panel_view = Some(index_status::DETAILS_VIEW.into());
                shell.schedule_save(cx);
            }
            index_status::OPEN_LOG => {
                let log = self
                    .index
                    .status()
                    .info
                    .log
                    .clone()
                    .filter(|path| path.is_file());
                match log {
                    Some(path) => {
                        if let Err(e) = self.open_documents().open(
                            Resource::File(path),
                            OpenOptions::default(),
                            Cancellation::default(),
                        ) {
                            eprintln!("index log: {e}");
                        }
                    }
                    None => {
                        let _ = shell::view::report(
                            self,
                            "rove.index.noLog",
                            Reason::Receipt,
                            Message::key("index.noLog"),
                            Arc::from([]),
                            cx,
                        );
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn render_index_details(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let status = self.index.status();
        let now = rove_lang_cs::lifecycle::meta::now_ms();
        let lines = index_status::details_lines(status, &self.strings, now);
        let actions = index_status::details_actions(&status.state);
        let link = self
            .shell
            .color(self.theme.as_ref(), "textLink.foreground")
            .or_else(|| {
                self.shell
                    .color(self.theme.as_ref(), "notificationLink.foreground")
            });
        let zoom = self.shell.zoom();
        let buttons: Vec<_> = actions
            .into_iter()
            .map(|(command, label)| {
                div()
                    .id(SharedString::from(format!("index-details-{command}")))
                    .cursor_pointer()
                    .when_some(link, |d, c| d.text_color(c))
                    .child(self.label(label, &[]))
                    .on_click(cx.listener(move |v, _, _, cx| v.index_command(command, cx)))
            })
            .collect();
        div()
            .id("index-details")
            .px(px(8.0 * zoom))
            .flex()
            .flex_col()
            .gap(px(4.0 * zoom))
            .text_size(px(shell::view::UI_FONT_SIZE * zoom))
            .children(lines.into_iter().map(|line| div().child(line)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(16.0 * zoom))
                    .children(buttons),
            )
            .into_any_element()
    }
}

/// 骨架装配的工厂：视图容器与状态栏条目由工作台通用绘制，文件夹小节是骨架文件树，问题面板是空状态。
const KNOWN_FACTORIES: [&str; 6] = [
    CONTAINER_FACTORY,
    STATUS_FACTORY,
    FILE_VIEW,
    PROBLEMS,
    crate::index_status::DETAILS_VIEW,
    crate::outline::registry::OUTLINE_VIEW,
];
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
        self.commands.registry()
    }
    fn render_editor(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("editor")
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus)
            // 主题可能不给 editor.* 设色（例如 Light+ 没有 editor.foreground）：用代码视图
            // 解析好的调色板（与 VS Code 一样回落到默认值），热切换主题时也不会缺色。
            .bg(color(self.code_view().read(cx).palette().background))
            .text_color(color(self.code_view().read(cx).palette().foreground))
            .child(self.editor_grid(cx))
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
            crate::index_status::DETAILS_VIEW => Some(self.render_index_details(cx)),
            crate::outline::registry::OUTLINE_VIEW => Some(self.render_outline(cx)),
            _ => None,
        }
    }
    fn section_title(&self, id: &str) -> Option<String> {
        (id == FILE_VIEW).then(|| self.shell.title.clone())
    }
    fn active_document(&self) -> Option<DocumentId> {
        self.editor
            .active()
            .current()
            .and_then(Tab::opened)
            .map(|o| o.document.id)
    }
    fn editor_focus(&self) -> FocusHandle {
        self.focus.clone()
    }
    fn run_external_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        // #24：快速打开、命令面板、转到行。
        if crate::quick_access::registry::COMMANDS
            .iter()
            .any(|(cmd, _, _)| *cmd == id)
        {
            self.quick_access_command(id, window, cx);
            return;
        }
        self.command(id, cx);
    }
    fn zoom_changed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        // #16 的窗口缩放（window.zoomLevel）同步到 #19 的代码视图：先按设置重读基准字号，
        // 再乘上窗口缩放系数。只重新整形可见行：文档、语法快照与工作区扫描结果都不变。
        let zoom = self.shell.zoom() as f64;
        for code in self.code_views() {
            code.update(cx, |code, cx| apply_zoom(code, zoom, cx));
        }
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
        // Groups, active tabs and reveals change through commands, clicks and the open service;
        // load them into the per-group code views before drawing.
        self.sync_views(cx);
        // A tab that finished loading in the background reports Ready once it is shown here.
        self.editor.flush_ready();
        // #26: outline, breadcrumbs and symbol requests follow the active editor.
        self.sync_outline(window, cx);
        // #24：最近打开、失去焦点时关闭、关闭后还焦点或运行选中的命令。
        self.sync_quick_access(window, cx);
        // #40：打开中的 C# 文档同步给查询路由的 BufferStates；F12 的画面计时。
        self.sync_nav(window, cx);
        if self.focus_group != Some(self.editor.focused) {
            let moved = self.focus_group.is_some();
            self.focus_group = Some(self.editor.focused);
            // Follow the focused group only while the keyboard is in the editor area.
            if moved && self.focus.contains_focused(window, cx) {
                self.focus_group_code(self.editor.focused, window, cx);
            }
        }
        if let Some((tab, start)) = self.switch_started {
            let shown = self.views[&self.editor.focused].primary.shown;
            if shown.is_some_and(|shown| shown.tab == tab) {
                self.switch_started = None;
                window.on_next_frame(move |_, _| {
                    println!(
                        "EDITOR_SWITCH_TO_PAINT_MS {:.3}",
                        start.elapsed().as_secs_f64() * 1000.0
                    );
                });
            }
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
        let goto = self.goto_overlay(cx);
        let quick = self.quick_overlay(cx);
        let hover = self.nav_overlay(cx);
        let context = editor_actions::context(&self.editor)
            + self.breadcrumbs_context()
            + &self.nav_context();
        div()
            .size_full()
            .font_family("Segoe UI")
            .key_context(context.as_str())
            .on_action(
                cx.listener(|v, action: &editor_actions::EditorAction, _, cx| {
                    v.command(action.0, cx)
                }),
            )
            // #14: keys resolved by the keybinding service for commands without a module action.
            .on_action(cx.listener(|v, a: &crate::keys::RunCommand, _, cx| {
                v.run_invocation(
                    Invocation {
                        id: a.id.as_ref().into(),
                        args: a.args.clone(),
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|v, a: &crate::keys::ChordStatus, _, cx| v.chord_status(a, cx)))
            // #36：rove.index.* 命令（键位服务分发来的 action）。
            .on_action(cx.listener(|v, _: &crate::index_status::Rebuild, _, cx| {
                v.index_command(crate::index_status::REBUILD, cx)
            }))
            .on_action(
                cx.listener(|v, _: &crate::index_status::ShowDetails, _, cx| {
                    v.index_command(crate::index_status::SHOW_DETAILS, cx)
                }),
            )
            .on_action(cx.listener(|v, _: &crate::index_status::OpenLog, _, cx| {
                v.index_command(crate::index_status::OPEN_LOG, cx)
            }))
            .on_action(cx.listener(
                |v, a: &crate::outline::registry::OutlineAction, window, cx| {
                    v.outline_command(a.0, window, cx)
                },
            ))
            .on_action(cx.listener(
                |v, a: &crate::quick_access::registry::QuickAccessAction, window, cx| {
                    v.quick_action_handler(a, window, cx)
                },
            ))
            // #40：C# 导航命令。
            .on_action(
                cx.listener(|v, a: &crate::nav::registry::NavAction, window, cx| {
                    v.nav_command(a.0, window, cx)
                }),
            )
            .child(workbench)
            .children(goto)
            .children(quick)
            .children(hover)
    }
}
/// Launch the actual GPUI application. Path is session input, never persisted in repository data.
///
/// `settings` 是应用唯一的设置服务实例（`crate::app_settings::open`），窗口设置、工作台、主题、
/// 代码视图、打开服务与工作区扫描都用它。
pub fn launch(root: PathBuf, settings: Arc<dyn Settings>, started: Instant) {
    Application::new().run(move |cx: &mut App| {
        if let Err(e) = cx
            .text_system()
            .add_fonts(vec![Cow::Borrowed(shell::icons::FONT)])
        {
            eprintln!("codicons: {e}");
        }
        // #14：所有按键经键位服务解析（生成的 VS Code 默认键位 + 用户 keybindings.json）。
        crate::keys::init(cx);
        if let Some(path) = crate::keys::user_keybindings_path(std::env::args_os().skip(1)) {
            crate::keys::watch_user_file(path, cx);
        }
        // 布局文件只有几百字节：首帧前读取，用来恢复窗口位置、大小与最大化状态。
        let store = LayoutStore::for_workspace(&root);
        let layout = store.load();
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
                        let mut view = Workbench::with_settings(root, started, store, settings, cx)
                            .expect("valid editor settings");
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
    use gpui::{point, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext};
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
            let opened = v.editor.active().current().and_then(Tab::opened).unwrap();
            assert_eq!(opened.document.text.lines().known, 3);
            let token = opened.syntax.as_ref().unwrap().tokens(Line(0)).unwrap();
            assert!(token.iter().any(|t| t.range.start().0 == 0
                && t.range.end().0 == 6
                && t.scopes.last().map(|s| s.as_ref()) == Some("storage.modifier.public.cs")));
            assert!(v.status().contains("UTF-8"));
        });
        // 代码视图已绘制首行：正文已整形，关键字区间按主题着色。
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        code.read_with(cx, |c, _| {
            let viewport = c.viewport().expect("code view laid out");
            assert_eq!(viewport.first_line, 0);
            assert!(c.last_frame().shaped >= 2, "{:?}", c.last_frame());
            assert_eq!(c.last_frame().placeholders, 0);
            let first = c.painted_text(0).expect("first line painted");
            assert_eq!(&*first, "public class First {}");
            let merged = c.merged_decorations(0).expect("syntax decorations");
            let keyword = merged.runs.iter().find(|r| r.range == (0..6)).unwrap();
            // Dark 2026（#10 正式主题引擎）：include 链中 dark_vs 的 `storage.modifier`
            // 比 2026-dark 的 `storage` 更具体，与 VS Code 一致取 #569cd6。
            assert_eq!(keyword.style.foreground, Some(Color(0x56, 0x9c, 0xd6, 255)));
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
        });
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        code.read_with(cx, |c, _| {
            let frame = c.last_frame();
            assert!(frame.visible_lines > 0);
            assert!(frame.visible_lines < 100, "只布局可见行：{frame:?}");
        });
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(400.0), px(100.0)),
            delta: ScrollDelta::Lines(point(0.0, -20.0)),
            modifiers: Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.run_until_parked();
        code.read_with(cx, |c, _| assert_eq!(c.viewport().unwrap().first_line, 20));
        cx.simulate_click(point(px(128.0), px(152.0)), Modifiers::default());
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert!(v.status().contains("b.cs"));
        });
        code.read_with(cx, |c, _| {
            assert_eq!(c.scroll_position().top, 0.0);
            assert_eq!(c.viewport().unwrap().first_line, 0);
            assert_eq!(&*c.painted_text(0).unwrap(), "public class B {}");
        });
    }
    /// #14 entry point: keybindings.json → real GPUI key events → command dispatch. Covers user
    /// overrides of workbench keys (formerly bound by each module), `-command` removal, unknown
    /// commands, chords with Esc and timeout, and a placeholder command still reporting status.
    #[gpui::test]
    fn keybindings_json_keys_dispatch_commands(cx: &mut TestAppContext) {
        let folder = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        let keybindings = user.path().join("keybindings.json");
        std::fs::write(
            &keybindings,
            r#"// shared with VS Code
            [
                { "key": "ctrl+p", "command": "-workbench.action.quickOpen" },
                { "key": "ctrl+k ctrl+o", "command": "rove.contextBasket.add" },
                { "key": "ctrl+b", "command": "-workbench.action.toggleSidebarVisibility" },
                { "key": "ctrl+alt+b", "command": "workbench.action.toggleSidebarVisibility" },
                { "key": "f5", "command": "workbench.action.debug.start" },
            ]"#,
        )
        .unwrap();
        cx.update(|cx| crate::keys::watch_user_file(keybindings, cx));
        let (view, cx) =
            cx.add_window_view(|_, cx| Workbench::new(folder.path().into(), Instant::now(), cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.focus(&view.read(cx).focus));
        let empty = |v: &Workbench| v.label("workbench.empty", &[]);
        // Default ctrl+p removed by the user file: nothing happens.
        cx.simulate_keystrokes("ctrl-p");
        view.read_with(cx, |v, _| assert_eq!(v.status(), empty(v)));
        // Unknown command in the file is ignored without an error.
        cx.simulate_keystrokes("f5");
        view.read_with(cx, |v, _| assert_eq!(v.status(), empty(v)));
        // A workbench key (#16) rebound by the user: the old key no longer toggles the side bar.
        cx.simulate_keystrokes("ctrl-b");
        view.read_with(cx, |v, _| assert!(v.shell.layout.sidebar_visible));
        cx.simulate_keystrokes("ctrl-alt-b");
        view.read_with(cx, |v, _| assert!(!v.shell.layout.sidebar_visible));
        cx.simulate_keystrokes("ctrl-alt-b");
        view.read_with(cx, |v, _| assert!(v.shell.layout.sidebar_visible));
        // Defaults still apply to keys the user did not touch (#16's ctrl+j toggles the panel).
        let panel = view.read_with(cx, |v, _| v.shell.layout.panel_visible);
        cx.simulate_keystrokes("ctrl-j");
        view.read_with(cx, |v, _| assert_ne!(v.shell.layout.panel_visible, panel));
        // User chord: pending status, then dispatch of a registered placeholder command.
        cx.simulate_keystrokes("ctrl-k");
        view.read_with(cx, |v, _| {
            assert_eq!(
                v.status(),
                v.label("keybindings.chordPending", &[("keys", "ctrl+k".into())])
            )
        });
        cx.simulate_keystrokes("ctrl-o");
        view.read_with(cx, |v, _| {
            assert_eq!(v.status(), v.label("command.rove.contextBasket.add", &[]))
        });
        // Escape aborts a pending chord.
        cx.simulate_keystrokes("ctrl-k escape");
        cx.update(|window, cx| {
            assert!(crate::keys::pending_chord(window, cx).is_empty());
            assert_eq!(
                view.read(cx).status(),
                view.read(cx).label(
                    "keybindings.chordAborted",
                    &[("keys", "ctrl+k escape".into())]
                )
            );
        });
        // A pending chord times out after 5 seconds; the next key starts fresh.
        cx.simulate_keystrokes("ctrl-k");
        cx.executor()
            .advance_clock(rove_commands::chord::CHORD_TIMEOUT);
        cx.run_until_parked();
        cx.update(|window, cx| assert!(crate::keys::pending_chord(window, cx).is_empty()));
        cx.simulate_keystrokes("ctrl-alt-b");
        view.read_with(cx, |v, _| assert!(!v.shell.layout.sidebar_visible));
        // Menus show the effective key (user override included).
        cx.update(|_, cx| {
            assert_eq!(
                crate::keys::key_label("workbench.action.toggleSidebarVisibility", cx).as_deref(),
                Some("Ctrl+Alt+B")
            )
        });
    }
}

#[cfg(test)]
#[path = "workbench_settings_tests.rs"]
mod settings_tests;

#[cfg(test)]
#[path = "workbench_index_tests.rs"]
mod index_tests;
