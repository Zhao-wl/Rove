//! #28 欢迎页、最近打开、启动参数与首选项入口在骨架窗口中的装配。
//!
//! 这里只做装配，内容模型在 `crate::welcome`：
//!
//! 1. **欢迎页**：登记成编辑器类型 `rove.welcomePage`，资源是虚拟文档 `rove-welcome`，
//!    经 #17 的 `OpenDocuments` 在编辑器组里打开（标签复用、可固定、可关闭都是 #17 的既有语义）。
//! 2. **最近**：列表存在 Rove 自己的状态目录（`%APPDATA%\Rove\recent.json`），每项显示完整路径、可移除；
//!    「更多…」打开 `workbench.action.openRecent`（`Ctrl+R`）的快速选择（复用 #18）。
//! 3. **首选项入口**：齿轮菜单的四项；`Ctrl+,` / `Ctrl+K Ctrl+S` 打开与 VS Code **共用的那一份**文件，
//!    首次打开时在编辑器顶部显示第 3 级提示条（#16 的反馈层负责「可关闭」「不再提示」与持久化）。
//! 4. **颜色主题**：`Ctrl+K Ctrl+T` 的快速选择在移动选中项时实时预览（只换内存中的主题，不写文件），
//!    `Esc` 还原，`Enter` 才通过 #13 写一次 `workbench.colorTheme`。

use super::*;
use crate::{
    documents::OpenTarget,
    list_widget::{
        matcher::MatchType,
        options::ListOptions,
        quick_input::{QuickInputService, QuickPickConfig, QuickPickEvent},
        quick_input_view::{QuickInputView, QuickInputViewEvent},
        StaticList,
    },
    lists::{ItemId, QuickInputId, QuickInputOptions, Row},
    skeleton::text::{VirtualContent, VirtualSource},
    welcome::{
        recent::{RecentKind, RecentList},
        registry as reg,
        themes::ThemePicker,
        view as page,
    },
};
use rove_buffer::ReadOnlyReason;

/// 欢迎页的虚拟文档内容来源：没有磁盘文件，正文是空的（绘制完全由本模块负责）。
pub(super) struct WelcomeSource;

impl VirtualSource for WelcomeSource {
    fn load(&self, name: &str) -> Option<CoreResult<VirtualContent>> {
        (name == reg::RESOURCE).then(|| {
            Ok(VirtualContent {
                text: String::new(),
                language: "plaintext".into(),
                // 虚拟文档里唯一「会话中也解除不了」的只读原因：欢迎页不是可编辑文本。
                read_only: vec![ReadOnlyReason::Metadata],
            })
        })
    }
}

/// 浮层当前在选什么。
#[derive(Clone, Copy, PartialEq, Eq)]
enum PickMode {
    Theme,
    Recent,
}

/// 窗口里 #28 的状态。
#[derive(Default)]
pub(super) struct Welcome {
    pub(super) recent: RecentList,
    recent_path: Option<PathBuf>,
    /// 与 VS Code 共用的两份文件（#13 解析的位置）。
    settings_path: Option<PathBuf>,
    keybindings_path: Option<PathBuf>,
    /// 打开后要显示共用提示条的文件：等它的标签就绪后才知道 DocumentId。
    pending_notice: Option<PathBuf>,
    /// 浮层（颜色主题 / 打开最近）。
    view: Option<Entity<QuickInputView>>,
    id: Option<QuickInputId>,
    mode: Option<PickMode>,
    theme: Option<ThemePicker>,
    /// 「打开最近」列表里每一项对应的条目。
    picks: Vec<(PathBuf, RecentKind)>,
    /// 最近一次「把这个文件夹作为工作区打开」的请求（一个窗口一个文件夹：实际是新开一个窗口）。
    /// `bool` 是「新窗口出来后关掉本窗口」（本窗口是空窗口时）。
    folder_request: Option<(PathBuf, bool)>,
    /// `window.openFilesInNewWindow = "on"`：这些文件要在新窗口里打开（工作区仍是本窗口的那个）。
    file_requests: Vec<PathBuf>,
    subscriptions: Vec<gpui::Subscription>,
    pick_events: Option<rove_base::Subscription>,
    /// 接受事件在浮层关闭（`Closed`）之前**同步**写进这里，所以关闭时能分清「确认」与「取消」。
    accepted: Arc<std::sync::Mutex<Option<QuickPickEvent>>>,
    focus_editor: bool,
}

impl Welcome {
    pub(super) fn new(store: &LayoutStore) -> Self {
        let (settings_path, keybindings_path) = if cfg!(test) {
            // 单元测试不碰真实的用户目录；测试用 `set_shared_paths` 指定临时目录。
            (None, None)
        } else {
            shared_paths(std::env::args_os().skip(1))
        };
        Self {
            recent: RecentList::load(store.recent_path().as_deref()),
            recent_path: store.recent_path(),
            settings_path,
            keybindings_path,
            ..Default::default()
        }
    }
}

/// 与 VS Code 共用的 `settings.json` / `keybindings.json`（FEATURES §13 规则 1）。
pub fn shared_paths(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    let (arg, _) = rove_config::paths::split_user_dir_arg(args);
    let dir = rove_config::paths::resolve_user_dir(arg, |k| std::env::var_os(k));
    (
        dir.as_ref()
            .map(|d| d.join(rove_config::paths::SETTINGS_FILE)),
        dir.as_ref()
            .map(|d| d.join(rove_config::paths::KEYBINDINGS_FILE)),
    )
}

fn row(id: &str, label: &str, detail: Option<&str>) -> Row {
    Row {
        id: ItemId(id.into()),
        label: label.into(),
        detail: detail.map(Into::into),
        depth: 0,
        expandable: false,
        expanded: false,
        disabled: false,
    }
}

impl Workbench {
    // ------------------------------------------------------------------ 欢迎页

    /// 欢迎页的虚拟资源。
    pub(super) fn welcome_resource() -> Resource {
        Resource::Virtual(reg::RESOURCE.into())
    }

    /// 在编辑器组里打开欢迎页（#17 的打开文档服务）。已经打开时只是激活那个标签。
    pub fn open_welcome(&mut self, cx: &mut Context<Self>) {
        let options = OpenOptions {
            target: OpenTarget::Active,
            source: OpenSource::Other,
            preview_tab: false,
            ..Default::default()
        };
        if let Err(e) =
            self.open_documents()
                .open(Self::welcome_resource(), options, Cancellation::default())
        {
            eprintln!("welcome: {e}");
        }
        cx.notify();
    }

    /// 应用启动时的编辑器（`workbench.startupEditor`）：勾选了「启动时显示欢迎页」就打开欢迎页；
    /// 没有打开文件夹的空窗口**总是**显示欢迎页（UX_SPEC §6.3）。
    /// 由 `launch` 在窗口装配好之后调用一次；窗口构造函数本身不打开任何编辑器。
    pub fn open_startup_editor(&mut self, cx: &mut Context<Self>) {
        if crate::welcome::view::open_at_startup(&*self.settings, self.has_folder) {
            self.open_welcome(cx);
        }
    }

    /// 标签标题：「欢迎使用」。
    pub(super) fn welcome_tab_title(&self, resource: &Resource) -> Option<String> {
        crate::editors::settings::is_welcome_resource(resource)
            .then(|| self.label("welcome.tabTitle", &[]))
    }

    /// 测试与 `launch` 用：指定共用文件的位置与最近列表的存放位置。
    pub fn set_shared_paths(
        &mut self,
        settings: Option<PathBuf>,
        keybindings: Option<PathBuf>,
        recent: Option<PathBuf>,
    ) {
        self.welcome.settings_path = settings;
        self.welcome.keybindings_path = keybindings;
        self.welcome.recent = RecentList::load(recent.as_deref());
        self.welcome.recent_path = recent;
    }

    /// 最近打开列表（测试与欢迎页绘制用）。
    pub fn recent_entries(&self) -> &RecentList {
        &self.welcome.recent
    }

    /// 记一次打开并立即落盘（启动、打开文件夹 / 文件时调用）。
    pub fn note_recent(&mut self, path: impl Into<PathBuf>, kind: RecentKind) {
        self.welcome.recent.add(path, kind);
        let path = self.welcome.recent_path.clone();
        self.welcome.recent.save(path.as_deref());
    }

    pub(super) fn remove_recent(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.welcome.recent.entries().get(index).cloned() else {
            return;
        };
        self.welcome.recent.remove(&entry.path);
        let path = self.welcome.recent_path.clone();
        self.welcome.recent.save(path.as_deref());
        cx.notify();
    }

    /// 打开最近列表里的一项：文件夹作为工作区打开，文件在当前窗口打开。
    pub(super) fn open_recent_entry(
        &mut self,
        path: PathBuf,
        kind: RecentKind,
        cx: &mut Context<Self>,
    ) {
        match kind {
            RecentKind::Folder => self.open_folder(path, cx),
            RecentKind::File => self.open_path(path, cx),
        }
    }

    // ------------------------------------------------------------------ 命令

    pub(super) fn is_welcome_command(id: &str) -> bool {
        reg::COMMANDS.iter().any(|(cmd, _, _)| *cmd == id)
    }

    pub(super) fn welcome_command(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match id {
            reg::SHOW_WELCOME => self.open_welcome(cx),
            reg::OPEN_FILE => self.prompt_for_path(false, cx),
            reg::OPEN_FOLDER => self.prompt_for_path(true, cx),
            reg::OPEN_RECENT => self.show_recent_picker(window, cx),
            reg::SELECT_THEME => self.show_theme_picker(window, cx),
            reg::OPEN_SETTINGS => {
                let path = self.welcome.settings_path.clone();
                self.open_shared_file(path, cx);
            }
            reg::OPEN_KEYBINDINGS => {
                let path = self.welcome.keybindings_path.clone();
                self.open_shared_file(path, cx);
            }
            _ => {}
        }
    }

    /// 系统的打开文件 / 文件夹对话框。
    fn prompt_for_path(&mut self, directories: bool, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: !directories,
            directories,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if directories {
                    view.open_folder(path, cx);
                } else {
                    view.open_path(path, cx);
                }
            });
        })
        .detach();
    }

    /// 打开一个文件：按 `window.openFilesInNewWindow` 决定复用当前窗口还是开新窗口
    /// （VS Code 的默认值 `off` 是复用）。
    pub fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.open_in_new_window(false) {
            if path.is_file() {
                self.note_recent(path.clone(), RecentKind::File);
            }
            self.welcome.file_requests.push(path);
            cx.notify();
            return;
        }
        self.open_path_here(path, cx);
    }

    /// 在**当前**窗口打开一个文件（不存在的路径走 UX_SPEC §6.4 的错误占位页，不弹模态框）。
    fn open_path_here(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let resource = match self.workspace_relative(&path) {
            Some(relative) => Resource::Workspace(relative),
            None => Resource::File(path.clone()),
        };
        if path.is_file() {
            self.note_recent(path, RecentKind::File);
        }
        if let Err(e) = self.open_documents().open(
            resource,
            OpenOptions {
                preview_tab: false,
                ..Default::default()
            },
            Cancellation::default(),
        ) {
            eprintln!("welcome: {e}");
        }
        cx.notify();
    }

    /// 把一个文件夹作为工作区打开。**一个窗口一个文件夹**（FEATURES §6，不做多根工作区）：
    /// 新文件夹总是装进一个新窗口；当前窗口是空窗口（没有打开文件夹）时，新窗口出来后关掉它。
    /// `window.openFoldersInNewWindow = "off"` 时也只是复用空窗口的位置，不会往现有工作区里塞第二个根。
    pub fn open_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.note_recent(path.clone(), RecentKind::Folder);
        let replace = !self.has_folder && !self.open_in_new_window(true);
        // 真正开窗口要有 `Window`，留到下一帧的 `sync_welcome` 做；请求本身可以断言（见 `welcome_tests.rs`）。
        self.welcome.folder_request = Some((path, replace));
        cx.notify();
    }

    /// 最近一次「打开文件夹」的目标（测试与装配用）。
    pub fn folder_request(&self) -> Option<&std::path::Path> {
        self.welcome
            .folder_request
            .as_ref()
            .map(|(p, _)| p.as_path())
    }

    /// 把工作区内的绝对路径换成工作区相对资源（标签复用、资源管理器定位都按相对资源走）。
    fn workspace_relative(&self, path: &std::path::Path) -> Option<WorkspacePath> {
        if !self.has_folder {
            return None;
        }
        let relative = path.strip_prefix(&self.root).ok()?;
        WorkspacePath::new(&relative.to_string_lossy().replace('\\', "/")).ok()
    }

    /// `window.openFilesInNewWindow` / `window.openFoldersInNewWindow`：`on` 开新窗口，
    /// `off` 复用当前窗口，`default` 对文件复用、对文件夹开新窗口（与 VS Code 相同）。
    pub(super) fn open_in_new_window(&self, folder: bool) -> bool {
        let key = if folder {
            "window.openFoldersInNewWindow"
        } else {
            "window.openFilesInNewWindow"
        };
        let value = Key::<String>::new(key)
            .read(&*self.settings, &Scope::default())
            .unwrap_or_default();
        match value.as_str() {
            "on" => true,
            "off" => false,
            _ => folder,
        }
    }

    // ------------------------------------------------------------------ 共用的设置与键位文件

    fn open_shared_file(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(path) = path else {
            let _ = shell::view::report(
                self,
                "welcome.noUserDir",
                Reason::Global(crate::feedback::Severity::Warning),
                Message::key("app.noUserDir"),
                Arc::from([]),
                cx,
            );
            return;
        };
        self.welcome.pending_notice = Some(path.clone());
        if let Err(e) = self.open_documents().open(
            Resource::File(path),
            OpenOptions {
                preview_tab: false,
                presentation: crate::documents::Presentation::Source,
                ..Default::default()
            },
            Cancellation::default(),
        ) {
            eprintln!("welcome: {e}");
        }
        cx.notify();
    }

    /// 共用文件的标签就绪后显示第 3 级提示条（UX_SPEC §6.1）。「不再提示」与持久化由 #16 负责。
    fn sync_shared_notice(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.welcome.pending_notice.clone() else {
            return;
        };
        let document = self.editor.groups.values().find_map(|pane| {
            pane.tabs
                .iter()
                .find(|tab| matches!(&tab.resource, Resource::File(p) if *p == path))
                .and_then(Tab::opened)
                .map(|o| o.document.id)
        });
        let Some(document) = document else { return };
        self.welcome.pending_notice = None;
        let _ = shell::view::report(
            self,
            reg::SHARED_FILE_BAR,
            Reason::FileContext {
                document,
                reason: reg::SHARED_FILE_BAR.into(),
            },
            Message::key("welcome.sharedFile"),
            Arc::from([]),
            cx,
        );
    }

    // ------------------------------------------------------------------ 快速选择（主题 / 最近）

    fn ensure_pick_view(&mut self, cx: &mut Context<Self>) -> Entity<QuickInputView> {
        if let Some(view) = &self.welcome.view {
            return view.clone();
        }
        let theme = self.theme.clone();
        let strings: Arc<dyn Strings> = Arc::new(appearance::strings().unwrap());
        let options = ListOptions::read(&*self.settings);
        let view =
            cx.new(|cx| QuickInputView::new(QuickInputService::new(options), theme, strings, cx));
        let closed = cx.subscribe(&view, |this, _, event: &QuickInputViewEvent, cx| {
            let QuickInputViewEvent::Closed(id) = event;
            if this.welcome.id == Some(*id) {
                this.welcome.id = None;
                this.welcome_closed(cx);
            }
        });
        let slot = self.welcome.accepted.clone();
        let events = view
            .read(cx)
            .service
            .pick_events()
            .subscribe(Arc::new(move |e| {
                if matches!(e, QuickPickEvent::Accepted { .. }) {
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                }
            }));
        self.welcome.subscriptions = vec![closed];
        self.welcome.pick_events = Some(events);
        self.welcome.view = Some(view.clone());
        view
    }

    #[allow(clippy::too_many_arguments)]
    fn show_pick(
        &mut self,
        mode: PickMode,
        placeholder: &'static str,
        rows: Vec<Row>,
        focus: Option<usize>,
        empty: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.ensure_pick_view(cx);
        *self
            .welcome
            .accepted
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        let model = StaticList::new(rows, Version(1));
        let result = view.update(cx, |q, cx| {
            q.show(
                QuickInputOptions {
                    placeholder: Message::key(placeholder),
                    initial: "".into(),
                    items: Arc::new(model),
                    multi_select: false,
                },
                QuickPickConfig {
                    filter: Some(MatchType::Fuzzy),
                    empty: empty.map(Message::key),
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        match result {
            Ok(id) => {
                self.welcome.id = Some(id);
                self.welcome.mode = Some(mode);
                if let Some(index) = focus {
                    view.update(cx, |q, cx| {
                        if let Some(active) = q.service.active_mut() {
                            active.list.set_focus(Some(index));
                            active.list.reveal(index);
                        }
                        cx.notify();
                    });
                }
            }
            Err(e) => eprintln!("welcome: {e}"),
        }
        cx.notify();
    }

    fn show_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.theme_service.current().id().to_string();
        let picker = ThemePicker::new(self.theme_service.catalog(), &current);
        let rows = picker
            .items
            .iter()
            .map(|item| row(&item.id, &item.label, None))
            .collect();
        let focus = picker.initial_index();
        self.welcome.theme = Some(picker);
        self.show_pick(
            PickMode::Theme,
            "welcome.theme.placeholder",
            rows,
            focus,
            None,
            window,
            cx,
        );
    }

    fn show_recent_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picks: Vec<(PathBuf, RecentKind)> = self
            .welcome
            .recent
            .entries()
            .iter()
            .map(|e| (e.path.clone(), e.kind))
            .collect();
        let rows = picks
            .iter()
            .enumerate()
            .map(|(i, (path, _))| row(&i.to_string(), &path.display().to_string(), None))
            .collect();
        self.welcome.picks = picks;
        self.show_pick(
            PickMode::Recent,
            "welcome.recent.placeholder",
            rows,
            Some(0),
            Some("welcome.recent.empty"),
            window,
            cx,
        );
    }

    /// `Enter`：确认选中项（在浮层关闭之前处理，见 [`Welcome::accepted`]）。
    fn welcome_picked(&mut self, event: QuickPickEvent, cx: &mut Context<Self>) {
        let QuickPickEvent::Accepted { id, items, .. } = event else {
            return;
        };
        if self.welcome.id != Some(id) && self.welcome.id.is_some() {
            return;
        }
        match self.welcome.mode {
            Some(PickMode::Theme) => {
                let chosen = items.first().map(|i| i.0.clone());
                let write = self.welcome.theme.as_mut().and_then(|picker| {
                    let index = chosen
                        .as_ref()
                        .and_then(|id| picker.items.iter().position(|i| i.id == *id));
                    picker.accept(index)
                });
                if let Some(theme) = write {
                    self.write_color_theme(theme, cx);
                }
            }
            Some(PickMode::Recent) => {
                let chosen = items
                    .first()
                    .and_then(|i| i.0.parse::<usize>().ok())
                    .and_then(|index| self.welcome.picks.get(index).cloned());
                if let Some((path, kind)) = chosen {
                    self.open_recent_entry(path, kind, cx);
                }
            }
            None => {}
        }
        cx.notify();
    }

    /// 浮层关闭：先处理「确认」，没有确认过的主题预览还原成打开时的那一个（`Esc`、失去焦点都走这里）。
    fn welcome_closed(&mut self, cx: &mut Context<Self>) {
        let accepted = self
            .welcome
            .accepted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(event) = accepted {
            self.welcome_picked(event, cx);
        }
        if let (Some(PickMode::Theme), Some(picker)) =
            (self.welcome.mode, self.welcome.theme.as_mut())
        {
            if picker.cancel().is_some() {
                // 还原 = 按设置重新构建，等于回到用户真正选择的主题。
                let service = self.theme_service.clone();
                cx.background_executor()
                    .spawn(async move {
                        let _ = service.reload();
                    })
                    .detach();
            }
        }
        self.welcome.mode = None;
        self.welcome.theme = None;
        self.welcome.focus_editor = true;
        cx.notify();
    }

    /// #13 的保留格式写入：只改 `workbench.colorTheme` 一个键，写入前备份。
    fn write_color_theme(&mut self, theme: Arc<str>, cx: &mut Context<Self>) {
        self.write_setting("workbench.colorTheme", Value::String(theme), cx);
    }

    /// 「启动时显示欢迎页」：只改 `workbench.startupEditor` 一个键（#13 的保留格式写入）。
    pub(super) fn set_show_on_startup(&mut self, checked: bool, cx: &mut Context<Self>) {
        self.write_setting(reg::STARTUP_EDITOR, page::startup_value(checked), cx);
        cx.notify();
    }

    fn write_setting(&mut self, key: &'static str, value: Value, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        let task = settings.write(
            Arc::from(key),
            Scope::default(),
            value,
            Cancellation::default(),
        );
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = task.await {
                    eprintln!("welcome: 无法写入 {key}：{e}");
                }
            })
            .detach();
    }

    /// 每帧：打开文件夹的新窗口、主题预览跟随选中项、共用提示条、关闭后的焦点归还。
    pub(super) fn sync_welcome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_shared_notice(cx);
        if !cfg!(test) {
            if let Some((path, replace)) = self.welcome.folder_request.take() {
                open_new_window(path, Vec::new(), self.settings.clone(), cx);
                if replace {
                    window.remove_window();
                    return;
                }
            }
            if !self.welcome.file_requests.is_empty() {
                let files = std::mem::take(&mut self.welcome.file_requests);
                open_new_window(self.root.clone(), files, self.settings.clone(), cx);
            }
        }
        if self.welcome.mode == Some(PickMode::Theme) {
            let focused = self
                .welcome
                .view
                .as_ref()
                .and_then(|v| v.read(cx).service.active())
                .and_then(|a| a.list.focused_id());
            if let (Some(id), Some(picker)) = (focused, self.welcome.theme.as_mut()) {
                let index = picker.items.iter().position(|i| &*i.id == id.0.as_ref());
                if let Some(preview) = index.and_then(|i| picker.focus(i)) {
                    if let Err(e) = self.theme_service.preview(&preview) {
                        eprintln!("welcome: 无法预览主题 {preview}：{e}");
                    }
                }
            }
        }
        if std::mem::take(&mut self.welcome.focus_editor) {
            window.focus(&self.focus);
        }
    }

    /// 浮层元素（位置与宽度和 #24 的快速打开一致）。
    pub(super) fn welcome_overlay(&self, cx: &App) -> Option<AnyElement> {
        let view = self.welcome.view.as_ref()?;
        view.read(cx).service.active()?;
        Some(
            gpui::deferred(
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

    // ------------------------------------------------------------------ 启动参数

    /// `rove.exe <路径>`：文件在本窗口打开，不存在的路径同样打开标签（内容区是错误占位页）。
    pub fn open_startup_targets(
        &mut self,
        targets: &[crate::welcome::startup::Target],
        cx: &mut Context<Self>,
    ) {
        use crate::welcome::startup::Target;
        for target in targets {
            match target {
                Target::File(path) => self.open_path_here(path.clone(), cx),
                Target::Missing(path) => {
                    // 不记进最近列表：它根本没能打开。
                    if let Err(e) = self.open_documents().open(
                        Resource::File(path.clone()),
                        OpenOptions {
                            preview_tab: false,
                            ..Default::default()
                        },
                        Cancellation::default(),
                    ) {
                        eprintln!("welcome: {e}");
                    }
                }
                Target::Folder(_) => {}
            }
        }
        cx.notify();
    }

    // ------------------------------------------------------------------ 绘制

    /// 欢迎页正文。
    pub(super) fn render_welcome(
        &self,
        group: EditorGroupId,
        tab: TabId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tile = self.ui_color("welcomePage.tileBackground");
        let hover = self.ui_color("welcomePage.tileHoverBackground");
        let background = self.ui_color("welcomePage.background");
        // 只用登记过的 welcomePage.* 与工作台已登记的标签前景色（#16 的 P0 键）。
        let muted = self.ui_color("tab.inactiveForeground");
        let link = self.ui_color("tab.activeForeground");
        let mut start = div().flex().flex_col().gap_2();
        for (i, (text, command)) in page::START_TILES.iter().enumerate() {
            let command = *command;
            let keys = crate::keys::key_label(command, cx).unwrap_or_default();
            start = start.child(
                div()
                    .id(SharedString::from(format!("welcome-start-{i}")))
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .bg(tile)
                    .hover(move |s| s.bg(hover))
                    .child(self.label(text, &[]))
                    .child(div().text_color(muted).child(keys))
                    .on_click(
                        cx.listener(move |v, _, window, cx| v.welcome_command(command, window, cx)),
                    ),
            );
        }
        let mut recent = div().flex().flex_col().gap_1();
        let rows = page::recent_rows(&self.welcome.recent);
        if rows.is_empty() {
            recent = recent.child(
                div()
                    .text_color(muted)
                    .child(self.label("welcome.noRecent", &[])),
            );
        }
        for item in rows {
            let index = item.index;
            let path = item.path.clone();
            recent = recent.child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .id(SharedString::from(format!("welcome-recent-{index}")))
                            .cursor_pointer()
                            .text_color(link)
                            .hover(move |s| s.bg(hover))
                            // 显示完整路径（UX_SPEC §1.6）。
                            .child(path)
                            .on_click(cx.listener(move |v, _, _, cx| {
                                let entry = v.welcome.recent.entries().get(index).cloned();
                                if let Some(entry) = entry {
                                    v.open_recent_entry(entry.path, entry.kind, cx);
                                }
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("welcome-remove-{index}")))
                            .cursor_pointer()
                            .text_color(muted)
                            .child(self.label("welcome.remove", &[]))
                            .on_click(cx.listener(move |v, _, _, cx| v.remove_recent(index, cx))),
                    ),
            );
        }
        recent = recent.child(
            div()
                .id(SharedString::from(format!(
                    "welcome-more-{}-{tab}",
                    group.0
                )))
                .cursor_pointer()
                .text_color(link)
                .child(self.label("welcome.more", &[]))
                .on_click(
                    cx.listener(|v, _, window, cx| v.welcome_command(reg::OPEN_RECENT, window, cx)),
                ),
        );
        let checked = page::show_on_startup(&*self.settings);
        let checkbox = div()
            .id(SharedString::from(format!(
                "welcome-startup-{}-{tab}",
                group.0
            )))
            .flex()
            .gap_2()
            .cursor_pointer()
            .child(self.label(
                if checked {
                    "list.checkbox.checked"
                } else {
                    "list.checkbox.unchecked"
                },
                &[],
            ))
            .child(self.label("welcome.showOnStartup", &[]))
            .on_click(cx.listener(move |v, _, _, cx| v.set_show_on_startup(!checked, cx)));
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .gap_4()
            .p_8()
            .bg(background)
            .child(div().text_xl().child(self.label("welcome.heading", &[])))
            .child(
                div()
                    .text_color(muted)
                    .child(self.label("welcome.subtitle", &[])),
            )
            .child(div().child(self.label("welcome.section.start", &[])))
            .child(start)
            .child(div().child(self.label("welcome.section.recent", &[])))
            .child(recent)
            .child(checkbox)
            .into_any_element()
    }
}

#[cfg(test)]
impl Workbench {
    /// 浮层里当前的行标签。
    pub(super) fn pick_rows(&self, cx: &App) -> Vec<String> {
        let Some(view) = self.welcome.view.as_ref() else {
            return Vec::new();
        };
        let Some(active) = view.read(cx).service.active() else {
            return Vec::new();
        };
        (0..active.list.len())
            .filter_map(|i| active.list.row(i).map(|r| r.label.to_string()))
            .collect()
    }

    /// 把选中项移到 `id` 那一行（等价于按方向键），再跑一次 `sync_welcome`（实时预览）。
    pub(super) fn focus_pick(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.welcome.view.clone() {
            view.update(cx, |q, cx| {
                if let Some(active) = q.service.active_mut() {
                    let _ = active.list.focus_id(&ItemId(id.into()));
                }
                cx.notify();
            });
        }
        self.sync_welcome(window, cx);
    }

    /// `Esc`。
    pub(super) fn close_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.welcome.view.clone() {
            view.update(cx, |q, cx| q.run_command("quickInput.hide", window, cx));
        }
    }

    /// `Enter`。
    pub(super) fn accept_pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.welcome.view.clone() {
            view.update(cx, |q, cx| q.run_command("quickInput.accept", window, cx));
        }
    }
}

/// 开一个新窗口（`window.openFoldersInNewWindow` / `window.openFilesInNewWindow`）。
fn open_new_window(
    root: PathBuf,
    files: Vec<PathBuf>,
    settings: Arc<dyn Settings>,
    cx: &mut Context<Workbench>,
) {
    let store = LayoutStore::for_workspace(&root);
    let started = Instant::now();
    let opened = cx.open_window(Default::default(), |window, cx| {
        cx.new(|cx| {
            let mut view = Workbench::with_settings(root, started, store, settings, cx)
                .expect("valid editor settings");
            Shell::attach_window(&mut view, window, cx);
            edit_host::attach_close_confirmation(&mut view, window, cx);
            for file in files {
                view.open_path_here(file, cx);
            }
            view
        })
    });
    if let Err(e) = opened {
        eprintln!("welcome: 无法打开新窗口：{e}");
    }
}
