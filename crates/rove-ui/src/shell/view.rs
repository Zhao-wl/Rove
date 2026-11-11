//! 工作台的 GPUI 部分（#16）：标题栏与命令中心、菜单栏、活动栏、主侧边栏、面板、状态栏、
//! 通知 toast / 通知中心、编辑器提示条、光标提示、模态框，以及命令分发与布局持久化。
//!
//! 宿主（当前是 `skeleton::Workbench`）实现 [`ShellHost`]，提供编辑区、视图工厂与文档身份；
//! 工作台状态都在 [`Shell`] 里。视图按需创建：面板默认隐藏时不调用面板视图工厂。
use super::{
    catalog::{self, *},
    feedback::{FeedbackHub, Reason, Severity, NEVER_SHOW_AGAIN, TOAST_AUTO_HIDE_MS},
    icons::{self, LabelPart},
    layout::{
        write_json, LayoutState, LayoutStore, PanelLocation, SideBarLocation, WindowPlacement,
        MIN_PANEL_SIZE, MIN_SIDEBAR_WIDTH,
    },
    menus::{visible_items, visible_menus, MenuId, MenuRegistry},
    status::{Alignment, StatusBar, StatusEntry, StatusKind},
};
use crate::{
    contributions::{Contribution, ContributionRegistry, Placement},
    feedback::{FeedbackAction, FeedbackService},
};
use gpui::{
    deferred, div, prelude::*, px, rgba, AnyElement, AnyView, App, Context, FocusHandle, Hsla,
    KeyBinding, KeyContext, KeyDownEvent, MouseButton, MouseMoveEvent, Pixels, SharedString,
    Subscription, Window, WindowBounds, WindowControlArea,
};
use rove_base::{DocumentId, Value};
use rove_commands::CommandRegistry;
use rove_config::{DefaultSettings, Key, Scope, SettingRegistry, Settings};
use rove_i18n::{Message, Strings};
use rove_theme::{Color, ColorRegistry, Theme, ThemeKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub const TITLE_BAR_HEIGHT: f32 = 35.0;
pub const ACTIVITY_BAR_WIDTH: f32 = 48.0;
pub const PART_TITLE_HEIGHT: f32 = 35.0;
pub const SECTION_HEADER_HEIGHT: f32 = 22.0;
pub const STATUS_BAR_HEIGHT: f32 = 22.0;
pub const UI_FONT_SIZE: f32 = 13.0;
const SAVE_DEBOUNCE_MS: u64 = 400;
/// VS Code 的缩放级别范围与步长：每级 ×1.2。
pub const MIN_ZOOM: f64 = -8.0;
pub const MAX_ZOOM: f64 = 8.0;

/// 宿主需要提供的东西。
pub trait ShellHost: Sized + 'static {
    fn shell(&self) -> &Shell;
    fn shell_mut(&mut self) -> &mut Shell;
    fn strings(&self) -> &dyn Strings;
    fn theme(&self) -> &dyn Theme;
    fn commands(&self) -> &CommandRegistry;
    /// 编辑区（编辑器组由 #17 提供）。
    fn render_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement;
    /// 视图工厂；返回 None 表示这个工厂还没有装配（视图不显示）。
    fn render_view(
        &mut self,
        factory_key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement>;
    /// 小节标题覆盖（例如“文件夹”小节显示文件夹名）。
    fn section_title(&self, _id: &str) -> Option<String> {
        None
    }
    /// 当前活动编辑器的文档；提示条与光标提示只显示在它上面。
    fn active_document(&self) -> Option<DocumentId>;
    fn editor_focus(&self) -> FocusHandle;
    /// 工作台没有实现的命令交给宿主的命令执行桥（#14）。
    fn run_external_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>);
    /// 缩放变化后宿主需要重新整形可见行（不重新扫描、不重新解析）。
    fn zoom_changed(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    TitleBar,
    ActivityBar,
    SideBar,
    Editor,
    Panel,
    StatusBar,
}

#[derive(Clone, Copy, Debug)]
enum Sash {
    SideBar { start: f32, size: f32 },
    Panel { start: f32, size: f32 },
}

/// 工作台状态；只由 UI 线程访问。
pub struct Shell {
    pub contributions: ContributionRegistry,
    pub menus: MenuRegistry,
    pub status_bar: StatusBar,
    pub feedback: FeedbackHub,
    pub layout: LayoutState,
    settings: Arc<dyn Settings>,
    overrides: BTreeMap<&'static str, Value>,
    colors: ColorRegistry,
    kind: ThemeKind,
    store: LayoutStore,
    save_generation: u64,
    status_timer: u64,
    toast_timers: BTreeMap<Arc<str>, u64>,
    zoom_level: f64,
    pub(crate) title: String,
    sidebar_focus: FocusHandle,
    panel_focus: FocusHandle,
    status_focus: FocusHandle,
    activity_focus: FocusHandle,
    notification_focus: FocusHandle,
    modal_focus: FocusHandle,
    title_focus: FocusHandle,
    status_index: usize,
    toast_index: usize,
    open_menu: Option<MenuId>,
    menubar_revealed: bool,
    drag: Option<Sash>,
    problems: (u32, u32),
    implemented: BTreeSet<String>,
    _subscriptions: Vec<Subscription>,
}

type Snapshot = (
    Option<PathBuf>,
    serde_json::Value,
    Option<PathBuf>,
    serde_json::Value,
);

/// 装配用：工作台自带的贡献、菜单与设置。各 issue 在 `Shell::new` 之后向
/// `shell.contributions` / `shell.menus` 追加登记（只在装配期，不在绘制中登记）。
pub fn default_contributions() -> ContributionRegistry {
    let mut r = ContributionRegistry::default();
    register_contributions(&mut r).expect("工作台贡献登记");
    r
}
pub fn default_menus() -> MenuRegistry {
    let mut r = MenuRegistry::default();
    register_menus(&mut r).expect("工作台菜单登记");
    r
}
/// #13 的设置服务合并前，工作台用契约里的 DefaultSettings 读 VS Code 默认值。
pub fn default_settings() -> Arc<dyn Settings> {
    let mut r = SettingRegistry::default();
    register_settings(&mut r).expect("工作台设置登记");
    Arc::new(DefaultSettings(r))
}

impl Shell {
    pub fn new<T: ShellHost>(
        root: &Path,
        contributions: ContributionRegistry,
        settings: Arc<dyn Settings>,
        store: LayoutStore,
        cx: &mut Context<T>,
    ) -> Self {
        let mut colors = ColorRegistry::default();
        register_colors(&mut colors).expect("工作台颜色登记");
        let layout = store.load();
        let global = store.load_global();
        let mut feedback = FeedbackHub::default();
        feedback.restore_suppressed(
            global
                .get("feedback.suppressed")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(Arc::from)),
        );
        feedback.set_do_not_disturb(
            global
                .get("notifications.doNotDisturb")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        );
        let title = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        let kind = Key::<String>::new("workbench.colorTheme")
            .read(settings.as_ref(), &Scope::default())
            .map(|name| {
                if name.to_lowercase().contains("light") {
                    ThemeKind::Light
                } else {
                    ThemeKind::Dark
                }
            })
            .unwrap_or(ThemeKind::Dark);
        let zoom_level = Key::<f64>::new("window.zoomLevel")
            .read(settings.as_ref(), &Scope::default())
            .unwrap_or(0.0)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        let mut shell = Self {
            contributions,
            menus: default_menus(),
            status_bar: StatusBar::default(),
            feedback,
            layout,
            settings,
            overrides: BTreeMap::new(),
            colors,
            kind,
            store,
            save_generation: 0,
            status_timer: 0,
            toast_timers: BTreeMap::new(),
            zoom_level,
            title,
            sidebar_focus: cx.focus_handle(),
            panel_focus: cx.focus_handle(),
            status_focus: cx.focus_handle(),
            activity_focus: cx.focus_handle(),
            notification_focus: cx.focus_handle(),
            modal_focus: cx.focus_handle(),
            title_focus: cx.focus_handle(),
            status_index: 0,
            toast_index: 0,
            open_menu: None,
            menubar_revealed: false,
            drag: None,
            problems: (0, 0),
            implemented: BTreeSet::new(),
            _subscriptions: Vec::new(),
        };
        if shell.layout.panel_location.is_none() {
            shell.layout.panel_location = Some(PanelLocation::from_setting(
                &shell.setting_str("workbench.panel.defaultLocation"),
            ));
        }
        shell
    }

    /// 窗口打开后调用：跟踪窗口位置与大小，关闭前保存布局。
    pub fn attach_window<T: ShellHost>(host: &mut T, window: &mut Window, cx: &mut Context<T>) {
        let bounds = cx.observe_window_bounds(window, |host: &mut T, window, cx| {
            host.shell_mut().record_window(window);
            host.shell_mut().schedule_save(cx);
        });
        host.shell_mut()._subscriptions.push(bounds);
        let entity = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            if let Some(entity) = entity.upgrade() {
                entity.update(cx, |host, _| {
                    host.shell_mut().record_window(window);
                    host.shell_mut().save_now();
                });
            }
            true
        });
    }

    /// 其他 issue 实现了自己的命令后在装配期声明，菜单才会显示对应的项。
    pub fn mark_implemented(&mut self, id: impl Into<String>) {
        self.implemented.insert(id.into());
    }

    // ------------------------------------------------------------- 设置

    fn setting(&self, key: &'static str) -> Option<Value> {
        self.overrides
            .get(key)
            .cloned()
            .or_else(|| self.settings.read(key, &Scope::default()).ok())
    }
    pub fn setting_str(&self, key: &'static str) -> String {
        match self.setting(key) {
            Some(Value::String(s)) => s.to_string(),
            _ => String::new(),
        }
    }
    pub fn setting_bool(&self, key: &'static str) -> bool {
        matches!(self.setting(key), Some(Value::Bool(true)))
    }
    /// 切换类命令：在后台写用户设置（#13 合并后生效，失败不弹通知）；
    /// 当前窗口立即使用新值，不等写入完成，也不因写入不支持而失效。
    fn set_setting<T: ShellHost>(&mut self, key: &'static str, value: Value, cx: &mut Context<T>) {
        let write = self.settings.write(
            key.into(),
            Scope::default(),
            value.clone(),
            Default::default(),
        );
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = write.await {
                    if e != rove_base::Error::Unsupported {
                        eprintln!("settings: 无法写入 {key}: {e}");
                    }
                }
            })
            .detach();
        self.overrides.insert(key, value);
    }

    pub fn sidebar_location(&self) -> SideBarLocation {
        SideBarLocation::from_setting(&self.setting_str("workbench.sideBar.location"))
    }
    pub fn panel_location(&self) -> PanelLocation {
        self.layout.panel_location.unwrap_or(PanelLocation::Bottom)
    }
    pub fn custom_title_bar(&self) -> bool {
        self.setting_str("window.titleBarStyle") != "native"
    }
    pub fn status_bar_visible(&self) -> bool {
        self.setting_bool("workbench.statusBar.visible")
    }

    // ------------------------------------------------------------- 缩放

    pub fn zoom_level(&self) -> f64 {
        self.zoom_level
    }
    /// 与 VS Code 相同：缩放因子 = 1.2^level。
    pub fn zoom(&self) -> f32 {
        1.2f64.powf(self.zoom_level) as f32
    }
    fn z(&self, v: f32) -> Pixels {
        px(v * self.zoom())
    }

    // ------------------------------------------------------------- 颜色

    pub fn set_theme_kind(&mut self, kind: ThemeKind) {
        self.kind = kind;
    }
    /// 先查当前主题；主题里没有登记的工作台颜色键回退到本模块登记的默认值。
    pub fn color(&self, theme: &dyn Theme, key: &str) -> Option<Hsla> {
        let c = match theme.color(key) {
            Ok(c) => c,
            Err(_) => self.colors.get(key).and_then(|d| d.for_kind(self.kind)),
        };
        c.map(hsla)
    }

    // ------------------------------------------------------------- 状态栏条目

    pub fn set_problem_counts(&mut self, errors: u32, warnings: u32) {
        self.problems = (errors, warnings);
    }

    // ------------------------------------------------------------- 布局

    pub fn active_container(&self) -> String {
        self.layout
            .active_container
            .clone()
            .filter(|c| {
                self.contributions
                    .get(c)
                    .is_some_and(|v| matches!(v.placement, Placement::ActivityBar))
            })
            .unwrap_or_else(|| EXPLORER.to_owned())
    }
    pub fn active_panel_view(&self) -> String {
        self.layout
            .active_panel_view
            .clone()
            .filter(|c| {
                self.contributions
                    .get(c)
                    .is_some_and(|v| matches!(v.placement, Placement::Panel))
            })
            .unwrap_or_else(|| PROBLEMS.to_owned())
    }
    pub fn record_window(&mut self, window: &Window) {
        let (bounds, maximized) = match window.window_bounds() {
            WindowBounds::Windowed(b) => (b, false),
            WindowBounds::Maximized(b) | WindowBounds::Fullscreen(b) => (b, true),
        };
        self.layout.window = Some(WindowPlacement {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
            maximized,
        });
    }
    fn snapshot(&self) -> Snapshot {
        let global = serde_json::json!({
            "feedback.suppressed": self.feedback.suppressed().map(|s| s.to_string()).collect::<Vec<_>>(),
            "notifications.doNotDisturb": self.feedback.do_not_disturb(),
        });
        (
            self.store.layout_path(),
            self.layout.to_json(),
            self.store.global_path(),
            global,
        )
    }
    /// 布局变化后调用：防抖后在后台执行器写文件。
    pub fn schedule_save<T: ShellHost>(&mut self, cx: &mut Context<T>) {
        if self.store.layout_path().is_none() {
            return;
        }
        self.save_generation += 1;
        let generation = self.save_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SAVE_DEBOUNCE_MS))
                .await;
            let Ok(Some(snapshot)) = this.update(cx, |host, _| {
                let shell = host.shell();
                (shell.save_generation == generation).then(|| shell.snapshot())
            }) else {
                return;
            };
            cx.background_executor()
                .spawn(async move { write_snapshot(snapshot) })
                .await;
        })
        .detach();
    }
    /// 关闭窗口时同步写入（几百字节）。
    pub fn save_now(&mut self) {
        self.save_generation += 1;
        write_snapshot(self.snapshot());
    }

    // ------------------------------------------------------------- 焦点

    pub fn focused_part(&self, window: &Window, cx: &App) -> Part {
        for (handle, part) in [
            (&self.sidebar_focus, Part::SideBar),
            (&self.panel_focus, Part::Panel),
            (&self.status_focus, Part::StatusBar),
            (&self.activity_focus, Part::ActivityBar),
            (&self.title_focus, Part::TitleBar),
        ] {
            if handle.contains_focused(window, cx) {
                return part;
            }
        }
        Part::Editor
    }
    fn part_visible(&self, part: Part) -> bool {
        match part {
            Part::SideBar => self.layout.sidebar_visible,
            Part::Panel => self.layout.panel_visible,
            Part::StatusBar => self.status_bar_visible(),
            Part::TitleBar => false,
            Part::ActivityBar | Part::Editor => true,
        }
    }
    pub fn sidebar_focus(&self) -> &FocusHandle {
        &self.sidebar_focus
    }
    pub fn panel_focus(&self) -> &FocusHandle {
        &self.panel_focus
    }
    pub fn status_focus(&self) -> &FocusHandle {
        &self.status_focus
    }
    pub fn notification_focus(&self) -> &FocusHandle {
        &self.notification_focus
    }
    pub fn status_index(&self) -> usize {
        self.status_index
    }
}

fn write_snapshot((layout_path, layout, global_path, global): Snapshot) {
    for (path, value) in [(layout_path, layout), (global_path, global)] {
        if let Some(path) = path {
            if let Err(e) = write_json(&path, &value) {
                eprintln!("layout: 无法保存 {}: {e}", path.display());
            }
        }
    }
}

fn hsla(c: Color) -> Hsla {
    rgba(((c.0 as u32) << 24) | ((c.1 as u32) << 16) | ((c.2 as u32) << 8) | c.3 as u32).into()
}

// ===================================================================== 反馈接入

/// 显示一条反馈并按需启动计时（状态栏消息 3 秒、toast 自动收起）。
pub fn report<T: ShellHost>(
    host: &mut T,
    key: &str,
    reason: Reason,
    message: Message,
    actions: Arc<[FeedbackAction]>,
    cx: &mut Context<T>,
) -> rove_base::Result<()> {
    host.shell_mut()
        .feedback
        .report(key, reason, message, actions)?;
    ensure_timers(host, cx);
    cx.notify();
    Ok(())
}

/// 为当前的状态栏消息与 toast 启动计时；重复调用是幂等的（绘制时也会调用，
/// 覆盖直接用 `FeedbackService::show` 的调用方）。
pub fn ensure_timers<T: ShellHost>(host: &mut T, cx: &mut Context<T>) {
    let shell = host.shell_mut();
    if let Some(status) = shell.feedback.status() {
        if status.generation != shell.status_timer {
            shell.status_timer = status.generation;
            let (generation, ms) = (status.generation, status.duration_ms);
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(ms as u64))
                    .await;
                let _ = this.update(cx, |host, cx| {
                    if host.shell_mut().feedback.expire_status(generation) {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }
    let pending: Vec<_> = shell
        .feedback
        .toasts()
        .filter(|n| n.actions.is_empty() && !matches!(n.severity, Severity::Error))
        .filter(|n| shell.toast_timers.get(&n.key) != Some(&n.generation))
        .map(|n| (n.key.clone(), n.generation))
        .collect();
    for (key, generation) in pending {
        shell.toast_timers.insert(key.clone(), generation);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(TOAST_AUTO_HIDE_MS as u64))
                .await;
            let _ = this.update(cx, |host, cx| {
                if host.shell_mut().feedback.expire_toast(&key, generation) {
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

// ===================================================================== 命令

macro_rules! shell_actions {
    ($($action:ident => $id:literal),* $(,)?) => {
        gpui::actions!(workbench, [$($action),*, CancelModal]);
        /// 把 KEYBINDINGS.md 的默认键位装配成 GPUI KeyBinding（#14 合并后由其键位系统替代）。
        pub fn key_bindings() -> Vec<KeyBinding> {
            std::iter::once(KeyBinding::new("escape", CancelModal, Some(CONTEXT_MODAL)))
                .chain(DEFAULT_KEYS
                .iter()
                .filter_map(|(keys, command, context)| match *command {
                    $($id => Some(KeyBinding::new(keys, $action, *context)),)*
                    _ => None,
                }))
                .collect()
        }
        /// 在宿主根元素上挂所有工作台命令的处理器。
        pub fn on_actions<T: ShellHost, E: InteractiveElement>(element: E, cx: &mut Context<T>) -> E {
            element
                $(.on_action(cx.listener(|host: &mut T, _: &$action, window, cx| run_command(host, $id, window, cx))))*
        }
    };
}
shell_actions! {
    ToggleSidebarVisibility => "workbench.action.toggleSidebarVisibility",
    FocusSideBar => "workbench.action.focusSideBar",
    TogglePanel => "workbench.action.togglePanel",
    ViewProblems => "workbench.actions.view.problems",
    FocusNextPart => "workbench.action.focusNextPart",
    FocusPreviousPart => "workbench.action.focusPreviousPart",
    ViewExplorer => "workbench.view.explorer",
    ViewSearch => "workbench.view.search",
    ZoomIn => "workbench.action.zoomIn",
    ZoomOut => "workbench.action.zoomOut",
    CloseWindow => "workbench.action.closeWindow",
    ShowNotifications => "notifications.showList",
    StatusClearFocus => "workbench.statusBar.clearFocus",
    StatusFocusFirst => "workbench.statusBar.focusFirst",
    StatusFocusLast => "workbench.statusBar.focusLast",
    StatusFocusNext => "workbench.statusBar.focusNext",
    StatusFocusPrevious => "workbench.statusBar.focusPrevious",
    NotificationClear => "notification.clear",
    NotificationCollapse => "notification.collapse",
    NotificationExpand => "notification.expand",
    NotificationToggle => "notification.toggle",
    FocusFirstToast => "notifications.focusFirstToast",
    FocusLastToast => "notifications.focusLastToast",
    FocusNextToast => "notifications.focusNextToast",
    FocusPreviousToast => "notifications.focusPreviousToast",
    HideToasts => "notifications.hideToasts",
    HideList => "notifications.hideList",
}

/// 执行一条命令：工作台实现的在这里处理，其余交给宿主。
pub fn run_command<T: ShellHost>(host: &mut T, id: &str, window: &mut Window, cx: &mut Context<T>) {
    if !catalog::is_implemented(id) {
        host.run_external_command(id, window, cx);
        return;
    }
    let editor = host.editor_focus();
    let part = host.shell().focused_part(window, cx);
    let shell = host.shell_mut();
    shell.open_menu = None;
    let mut layout_changed = false;
    match id {
        "workbench.action.toggleSidebarVisibility" => {
            shell.layout.sidebar_visible = !shell.layout.sidebar_visible;
            if !shell.layout.sidebar_visible && part == Part::SideBar {
                window.focus(&editor);
            }
            layout_changed = true;
        }
        "workbench.action.focusSideBar" => {
            shell.layout.sidebar_visible = true;
            window.focus(&shell.sidebar_focus.clone());
            layout_changed = true;
        }
        "workbench.action.toggleSidebarPosition" => {
            let next = match shell.sidebar_location() {
                SideBarLocation::Left => SideBarLocation::Right,
                SideBarLocation::Right => SideBarLocation::Left,
            };
            shell.set_setting(
                "workbench.sideBar.location",
                Value::String(next.as_setting().into()),
                cx,
            );
        }
        "workbench.action.togglePanel" => {
            shell.layout.panel_visible = !shell.layout.panel_visible;
            if shell.layout.panel_visible {
                window.focus(&shell.panel_focus.clone());
            } else if part == Part::Panel {
                window.focus(&editor);
            }
            layout_changed = true;
        }
        "workbench.action.closePanel" => {
            shell.layout.panel_visible = false;
            if part == Part::Panel {
                window.focus(&editor);
            }
            layout_changed = true;
        }
        "workbench.action.focusPanel" => {
            shell.layout.panel_visible = true;
            window.focus(&shell.panel_focus.clone());
            layout_changed = true;
        }
        "workbench.actions.view.problems" => {
            // 与 VS Code 相同：问题视图已在面板中获得焦点时再按一次就收起面板。
            if shell.layout.panel_visible
                && part == Part::Panel
                && shell.active_panel_view() == PROBLEMS
            {
                shell.layout.panel_visible = false;
                window.focus(&editor);
            } else {
                shell.layout.panel_visible = true;
                shell.layout.active_panel_view = Some(PROBLEMS.into());
                window.focus(&shell.panel_focus.clone());
            }
            layout_changed = true;
        }
        "workbench.actions.view.toggleProblems" => {
            if shell.layout.panel_visible && shell.active_panel_view() == PROBLEMS {
                shell.layout.panel_visible = false;
                if part == Part::Panel {
                    window.focus(&editor);
                }
            } else {
                shell.layout.panel_visible = true;
                shell.layout.active_panel_view = Some(PROBLEMS.into());
            }
            layout_changed = true;
        }
        "workbench.action.focusNextPart" | "workbench.action.focusPreviousPart" => {
            // VS Code 的顺序：编辑器 → 面板 → 状态栏 → 活动栏 → 侧边栏 → 编辑器，跳过隐藏的部分。
            const ORDER: [Part; 5] = [
                Part::Editor,
                Part::Panel,
                Part::StatusBar,
                Part::ActivityBar,
                Part::SideBar,
            ];
            let forward = id.ends_with("NextPart");
            let start = ORDER.iter().position(|p| *p == part).unwrap_or(0);
            let mut i = start;
            let next = loop {
                i = if forward {
                    (i + 1) % ORDER.len()
                } else {
                    (i + ORDER.len() - 1) % ORDER.len()
                };
                if shell.part_visible(ORDER[i]) || i == start {
                    break ORDER[i];
                }
            };
            focus_part(shell, next, &editor, window);
        }
        "workbench.action.focusActivityBar" => {
            focus_part(shell, Part::ActivityBar, &editor, window)
        }
        "workbench.action.focusStatusBar" => focus_part(shell, Part::StatusBar, &editor, window),
        "workbench.action.toggleStatusbarVisibility" => {
            let visible = !shell.status_bar_visible();
            shell.set_setting("workbench.statusBar.visible", Value::Bool(visible), cx);
            if !visible && part == Part::StatusBar {
                window.focus(&editor);
            }
        }
        "workbench.statusBar.focusFirst" => shell.status_index = 0,
        "workbench.statusBar.focusLast" => {
            shell.status_index = shell
                .status_bar
                .focus_order(&shell.contributions)
                .len()
                .saturating_sub(1)
        }
        "workbench.statusBar.focusNext" | "workbench.statusBar.focusPrevious" => {
            let n = shell
                .status_bar
                .focus_order(&shell.contributions)
                .len()
                .max(1);
            shell.status_index = if id.ends_with("Next") {
                (shell.status_index + 1) % n
            } else {
                (shell.status_index + n - 1) % n
            };
        }
        "workbench.statusBar.clearFocus" => window.focus(&editor),
        "workbench.view.explorer" | "workbench.view.search" => {
            let container = if id.ends_with("explorer") {
                EXPLORER
            } else {
                SEARCH
            };
            shell.layout.active_container = Some(container.into());
            shell.layout.sidebar_visible = true;
            window.focus(&shell.sidebar_focus.clone());
            layout_changed = true;
        }
        "workbench.action.zoomIn" | "workbench.action.zoomOut" | "workbench.action.zoomReset" => {
            let configured = Key::<f64>::new("window.zoomLevel")
                .read(shell.settings.as_ref(), &Scope::default())
                .unwrap_or(0.0);
            // window.zoomPerWindow 为 true（默认）时只改当前窗口，不写回设置，与 VS Code 相同。
            shell.zoom_level = match id {
                "workbench.action.zoomIn" => shell.zoom_level.floor() + 1.0,
                "workbench.action.zoomOut" => shell.zoom_level.ceil() - 1.0,
                _ => configured,
            }
            .clamp(MIN_ZOOM, MAX_ZOOM);
            if !shell.setting_bool("window.zoomPerWindow") {
                shell.set_setting("window.zoomLevel", Value::Number(shell.zoom_level), cx);
            }
            host.zoom_changed(window, cx);
            cx.notify();
            return;
        }
        "workbench.action.closeWindow" => {
            shell.record_window(window);
            shell.save_now();
            window.remove_window();
        }
        "notifications.showList" => shell.feedback.set_center_visible(true),
        "notifications.hideList" => {
            shell.feedback.set_center_visible(false);
            if shell.notification_focus.contains_focused(window, cx) {
                window.focus(&editor);
            }
        }
        "notifications.toggleList" => {
            let v = !shell.feedback.center_visible();
            shell.feedback.set_center_visible(v);
        }
        "notifications.hideToasts" => {
            shell.feedback.hide_toasts();
            if shell.notification_focus.contains_focused(window, cx) {
                window.focus(&editor);
            }
        }
        "notifications.clearAll" => shell.feedback.clear_all_notifications(),
        "notifications.toggleDoNotDisturbMode" => {
            let v = !shell.feedback.do_not_disturb();
            shell.feedback.set_do_not_disturb(v);
            layout_changed = true;
        }
        "notifications.focusToasts" | "notifications.focusFirstToast" => {
            shell.toast_index = 0;
            window.focus(&shell.notification_focus.clone());
        }
        "notifications.focusLastToast" => {
            shell.toast_index = shell.visible_notifications().len().saturating_sub(1);
        }
        "notifications.focusNextToast" | "notifications.focusPreviousToast" => {
            let n = shell.visible_notifications().len().max(1);
            shell.toast_index = if id.ends_with("NextToast") {
                (shell.toast_index + 1).min(n - 1)
            } else {
                shell.toast_index.saturating_sub(1)
            };
        }
        "notification.clear" => {
            if let Some(key) = shell.focused_notification() {
                let _ = shell.feedback.dismiss(&key);
            }
        }
        "notification.expand" | "notification.collapse" | "notification.toggle" => {
            if let Some(key) = shell.focused_notification() {
                let _ = match id {
                    "notification.expand" => shell.feedback.set_expanded(&key, true),
                    "notification.collapse" => shell.feedback.set_expanded(&key, false),
                    _ => shell.feedback.toggle_expanded(&key),
                };
            }
        }
        "notification.acceptPrimaryAction" => {
            if let Some(n) = shell.visible_notifications().get(shell.toast_index) {
                if let Some(a) = n.actions.first() {
                    let (key, action) = (n.key.clone(), a.id.clone());
                    let _ = shell.feedback.act(&key, &action);
                }
            }
        }
        _ => unreachable!("catalog 登记了但没有处理的命令：{id}"),
    }
    if layout_changed {
        shell.schedule_save(cx);
    }
    cx.notify();
}

fn focus_part(shell: &mut Shell, part: Part, editor: &FocusHandle, window: &mut Window) {
    let handle = match part {
        Part::Editor | Part::TitleBar => editor.clone(),
        Part::SideBar => shell.sidebar_focus.clone(),
        Part::Panel => shell.panel_focus.clone(),
        Part::StatusBar => {
            shell.status_index = 0;
            shell.status_focus.clone()
        }
        Part::ActivityBar => shell.activity_focus.clone(),
    };
    window.focus(&handle);
}

impl Shell {
    /// 通知中心打开时是全部通知，否则是 toast。
    fn visible_notifications(&self) -> Vec<&super::feedback::Notification> {
        if self.feedback.center_visible() {
            self.feedback.notifications().iter().collect()
        } else {
            self.feedback.toasts().collect()
        }
    }
    fn focused_notification(&self) -> Option<Arc<str>> {
        self.visible_notifications()
            .get(self.toast_index)
            .map(|n| n.key.clone())
    }
    /// 菜单里实际显示的项数（用于“菜单中不存在未实现的项”的断言）。
    pub fn visible_menu_items(&self, commands: &CommandRegistry, menu: MenuId) -> Vec<String> {
        visible_items(&self.menus, menu, commands, &|id| {
            catalog::is_implemented(id) || self.external_implemented(id)
        })
        .into_iter()
        .map(|(id, _)| id.to_owned())
        .collect()
    }
    /// 由其他 issue 实现、并通过 [`Shell::mark_implemented`] 声明的命令。
    fn external_implemented(&self, id: &str) -> bool {
        self.implemented.contains(id)
    }
}

// ===================================================================== 渲染

struct Palette<'a> {
    shell: &'a Shell,
    theme: &'a dyn Theme,
}
impl Palette<'_> {
    fn get(&self, key: &str) -> Option<Hsla> {
        self.shell.color(self.theme, key)
    }
    fn or(&self, key: &str, fallback: &str) -> Hsla {
        self.get(key)
            .or_else(|| self.get(fallback))
            .unwrap_or(gpui::transparent_black())
    }
}

fn text<T: ShellHost>(host: &T, message: &Message) -> String {
    host.strings().text(message).unwrap_or_else(|e| {
        eprintln!("strings: {} {e}", message.key);
        message.key.to_string()
    })
}
fn label<T: ShellHost>(host: &T, key: &str, args: &[(&str, String)]) -> String {
    text(
        host,
        &Message {
            key: key.into(),
            args: args
                .iter()
                .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
                .collect(),
        },
    )
}

/// 按 `$(icon)` 语法绘制标签：图标用 codicon 字体，文字用界面字体。
pub fn icon_label(text: &str, size: Pixels) -> gpui::Div {
    let mut row = div().flex().flex_row().items_center().gap(size * 0.25);
    for part in icons::parse_label(text) {
        row = match part {
            LabelPart::Text(t) => row.child(div().child(t)),
            LabelPart::Icon { glyph, .. } => row.child(icon(glyph, size * (16.0 / 13.0))),
        };
    }
    row
}
/// 单个 codicon 图标。
pub fn icon(glyph: char, size: Pixels) -> gpui::Div {
    div()
        .font_family(icons::FONT_FAMILY)
        .text_size(size)
        .line_height(size)
        .child(glyph.to_string())
}
fn named_icon(name: &str, size: Pixels) -> gpui::Div {
    icons::codepoint(name).map_or_else(div, |g| icon(g, size))
}

struct Tooltip {
    text: SharedString,
    bg: Hsla,
    fg: Hsla,
    border: Hsla,
    size: Pixels,
}
impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(self.bg)
            .text_color(self.fg)
            .border_1()
            .border_color(self.border)
            .text_size(self.size)
            .child(self.text.clone())
    }
}
fn tooltip(text: String, p: &Palette) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let (bg, fg, border, size) = (
        p.or("menu.background", "notifications.background"),
        p.or("menu.foreground", "notifications.foreground"),
        p.or("menu.border", "notifications.border"),
        p.shell.z(UI_FONT_SIZE),
    );
    let text: SharedString = text.into();
    move |_, cx| {
        cx.new(|_| Tooltip {
            text: text.clone(),
            bg,
            fg,
            border,
            size,
        })
        .into()
    }
}

/// 打开文件超过 100 ms 时显示在标签栏下方的细进度条（#17 调用）。`visible` 由调用方按
/// [`super::progress::ProgressDelay`] 判断；这里只负责绘制。
pub fn progress_bar(shell: &Shell, theme: &dyn Theme, visible: bool) -> gpui::Div {
    let color = shell
        .color(theme, "progressBar.background")
        .unwrap_or(gpui::transparent_black());
    let bar = div().h(px(2.0)).w_full().flex_shrink_0();
    if visible {
        bar.child(div().h_full().w(gpui::relative(0.3)).bg(color))
    } else {
        bar
    }
}

/// 整个工作台。宿主在自己的 `Render::render` 里调用。
pub fn render<T: ShellHost>(host: &mut T, window: &mut Window, cx: &mut Context<T>) -> AnyElement {
    ensure_timers(host, cx);
    let shell = host.shell();
    let custom_title = shell.custom_title_bar();
    let status_visible = shell.status_bar_visible();
    let sidebar_visible = shell.layout.sidebar_visible;
    let panel_visible = shell.layout.panel_visible;
    let sidebar_right = shell.sidebar_location() == SideBarLocation::Right;
    let zoom = shell.zoom();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let active = window.is_window_active();
    let border = if active {
        p.get("window.activeBorder")
    } else {
        p.get("window.inactiveBorder")
    };
    let fg = p.or("sideBar.foreground", "statusBar.foreground");
    let mut key_context = KeyContext::new_with_defaults();
    key_context.add("Workbench");
    if shell.feedback.modal().is_some() {
        // 模态框打开时只认它的 Esc，其他 Esc 键位（隐藏 toast 等）让位。
        key_context.add(CONTEXT_MODAL);
    } else {
        if shell.feedback.toasts().next().is_some() {
            key_context.add(CONTEXT_TOASTS);
        }
        if shell.feedback.center_visible() {
            key_context.add(CONTEXT_CENTER);
        }
    }

    let title_bar = custom_title.then(|| render_title_bar(host, window, cx));
    let activity = render_activity_bar(host, cx);
    let sidebar = sidebar_visible.then(|| render_sidebar(host, window, cx));
    let editor_area = render_editor_area(host, panel_visible, window, cx);
    let status = status_visible.then(|| render_status_bar(host, window, cx));
    let overlays = render_notifications(host, window, cx);
    let modal = render_modal(host, window, cx);

    let mut middle = div().flex().flex_row().flex_1().min_h_0().w_full();
    let sash = sidebar.as_ref().map(|_| sash_vertical(host, cx, true));
    let mut side: Vec<AnyElement> = vec![activity];
    if let Some(sidebar) = sidebar {
        side.push(sidebar);
        side.push(sash.unwrap());
    }
    if sidebar_right {
        side.reverse();
        middle = middle.child(editor_area).children(side);
    } else {
        middle = middle.children(side).child(editor_area);
    }

    let root = div()
        .id("workbench")
        .size_full()
        .flex()
        .flex_col()
        .relative()
        .text_size(px(UI_FONT_SIZE * zoom))
        .text_color(fg)
        .key_context(key_context)
        .when_some(border, |d, c| d.border_1().border_color(c))
        // Esc 关闭模态框等同取消：发出 Dismissed，绝不当作接受。
        .on_action(cx.listener(|host: &mut T, _: &CancelModal, window, cx| {
            if let Some(key) = host.shell().feedback.modal().map(|m| m.key.clone()) {
                let _ = host.shell_mut().feedback.dismiss(&key);
                window.focus(&host.editor_focus());
                cx.notify();
            }
        }))
        .on_mouse_move(
            cx.listener(|host: &mut T, e: &MouseMoveEvent, _, cx| drag_sash(host, e, cx)),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|host: &mut T, _, _, cx| {
                if host.shell_mut().drag.take().is_some() {
                    host.shell_mut().schedule_save(cx);
                    cx.notify();
                }
            }),
        )
        .children(title_bar)
        .child(middle)
        .children(status)
        .children(overlays)
        .children(modal);
    on_actions(root, cx).into_any_element()
}

fn drag_sash<T: ShellHost>(host: &mut T, e: &MouseMoveEvent, cx: &mut Context<T>) {
    let shell = host.shell_mut();
    let Some(drag) = shell.drag else { return };
    if e.pressed_button != Some(MouseButton::Left) {
        shell.drag = None;
        return;
    }
    let zoom = shell.zoom();
    let x: f32 = e.position.x.into();
    let y: f32 = e.position.y.into();
    match drag {
        Sash::SideBar { start, size } => {
            let delta = (x - start) / zoom;
            let delta = if shell.sidebar_location() == SideBarLocation::Right {
                -delta
            } else {
                delta
            };
            shell.layout.sidebar_width = (size + delta).max(MIN_SIDEBAR_WIDTH);
        }
        Sash::Panel { start, size } => {
            let delta = match shell.panel_location() {
                PanelLocation::Bottom => start - y,
                PanelLocation::Top => y - start,
                PanelLocation::Left => x - start,
                PanelLocation::Right => start - x,
            } / zoom;
            shell.layout.panel_size = (size + delta).max(MIN_PANEL_SIZE);
        }
    }
    cx.notify();
}

fn sash_vertical<T: ShellHost>(host: &T, cx: &mut Context<T>, sidebar: bool) -> AnyElement {
    let _ = host;
    div()
        .id(if sidebar {
            "sash-sidebar"
        } else {
            "sash-panel-v"
        })
        .w(px(4.0))
        .h_full()
        .flex_shrink_0()
        .ml(px(-2.0))
        .mr(px(-2.0))
        .cursor_col_resize()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |host: &mut T, e: &gpui::MouseDownEvent, _, _| {
                let shell = host.shell_mut();
                let x: f32 = e.position.x.into();
                shell.drag = Some(if sidebar {
                    Sash::SideBar {
                        start: x,
                        size: shell.layout.sidebar_width,
                    }
                } else {
                    Sash::Panel {
                        start: x,
                        size: shell.layout.panel_size,
                    }
                });
            }),
        )
        .into_any_element()
}

fn sash_horizontal<T: ShellHost>(cx: &mut Context<T>) -> AnyElement {
    div()
        .id("sash-panel")
        .h(px(4.0))
        .w_full()
        .flex_shrink_0()
        .mt(px(-2.0))
        .mb(px(-2.0))
        .cursor_row_resize()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|host: &mut T, e: &gpui::MouseDownEvent, _, _| {
                let shell = host.shell_mut();
                shell.drag = Some(Sash::Panel {
                    start: e.position.y.into(),
                    size: shell.layout.panel_size,
                });
            }),
        )
        .into_any_element()
}

// --------------------------------------------------------------------- 标题栏

fn render_title_bar<T: ShellHost>(
    host: &mut T,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyElement {
    let shell = host.shell();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let active = window.is_window_active();
    let (bg, fg) = if active {
        (
            p.or("titleBar.activeBackground", "sideBar.background"),
            p.or("titleBar.activeForeground", "sideBar.foreground"),
        )
    } else {
        (
            p.or("titleBar.inactiveBackground", "titleBar.activeBackground"),
            p.or("titleBar.inactiveForeground", "titleBar.activeForeground"),
        )
    };
    let height = shell.z(TITLE_BAR_HEIGHT);
    let menubar = render_menubar(host, cx);
    let shell = host.shell();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let center = if shell.setting_bool("window.commandCenter") {
        let (cbg, cborder, cfg) = if active {
            (
                p.get("commandCenter.background"),
                p.get("commandCenter.border"),
                p.or("commandCenter.foreground", "titleBar.activeForeground"),
            )
        } else {
            (
                p.get("commandCenter.background"),
                p.get("commandCenter.inactiveBorder"),
                p.or(
                    "commandCenter.inactiveForeground",
                    "titleBar.inactiveForeground",
                ),
            )
        };
        let hover_bg = p.get("commandCenter.activeBackground");
        let hover_fg = p.or("commandCenter.activeForeground", "commandCenter.foreground");
        let name = shell.title.clone();
        let text_label = label(host, "commandCenter.placeholder", &[("name", name.clone())]);
        let tip = label(host, "commandCenter.tooltip", &[("name", name)]);
        div()
            .id("command-center")
            .h(shell.z(22.0))
            .w(shell.z(400.0))
            .min_w(shell.z(160.0))
            .flex_shrink()
            .flex()
            .items_center()
            .justify_center()
            .rounded(shell.z(6.0))
            .text_color(cfg)
            .when_some(cbg, |d, c| d.bg(c))
            .when_some(cborder, |d, c| d.border_1().border_color(c))
            .hover(move |s| {
                let s = s.text_color(hover_fg);
                match hover_bg {
                    Some(c) => s.bg(c),
                    None => s,
                }
            })
            .cursor_pointer()
            .tooltip(tooltip(tip, &p))
            .child(icon_label(
                &format!("$(search) {text_label}"),
                shell.z(UI_FONT_SIZE),
            ))
            .on_click(cx.listener(|host: &mut T, _, window, cx| {
                run_command(host, "workbench.action.quickOpen", window, cx)
            }))
            .into_any_element()
    } else {
        div().child(shell.title.clone()).into_any_element()
    };
    let layout_controls = shell
        .setting_bool("workbench.layoutControl.enabled")
        .then(|| render_layout_controls(host, cx));
    let controls = render_window_controls(host, window);
    div()
        .id("titlebar")
        .debug_selector(|| "titlebar".into())
        .track_focus(&host.shell().title_focus.clone())
        .h(height)
        .w_full()
        .flex_shrink_0()
        .flex()
        .flex_row()
        .items_center()
        .bg(bg)
        .text_color(fg)
        .when_some(p.get("titleBar.border"), |d, c| {
            d.border_b_1().border_color(c)
        })
        .child(menubar)
        .child(
            div()
                .flex_1()
                .h_full()
                .window_control_area(WindowControlArea::Drag),
        )
        .child(center)
        .child(
            div()
                .flex_1()
                .h_full()
                .window_control_area(WindowControlArea::Drag),
        )
        .children(layout_controls)
        .children(controls)
        .into_any_element()
}

fn render_layout_controls<T: ShellHost>(host: &T, cx: &mut Context<T>) -> AnyElement {
    let shell = host.shell();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let hover = p
        .get("toolbar.hoverBackground")
        .or(p.get("menubar.selectionBackground"));
    let size = shell.z(16.0);
    let right = shell.sidebar_location() == SideBarLocation::Right;
    let sidebar_icon = match (right, shell.layout.sidebar_visible) {
        (false, true) => "layout-sidebar-left",
        (false, false) => "layout-sidebar-left-off",
        (true, true) => "layout-sidebar-right",
        (true, false) => "layout-sidebar-right-off",
    };
    let panel_icon = if shell.layout.panel_visible {
        "layout-panel"
    } else {
        "layout-panel-off"
    };
    let button = |id: &'static str, icon_name: &str, tip: String, command: &'static str| {
        div()
            .id(id)
            .w(shell.z(28.0))
            .h(shell.z(22.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(shell.z(4.0))
            .when_some(hover, |d, c| d.hover(move |s| s.bg(c)))
            .tooltip(tooltip(tip, &p))
            .child(named_icon(icon_name, size))
            .on_click(cx.listener(move |host: &mut T, _, window, cx| {
                run_command(host, command, window, cx)
            }))
    };
    let key = |c: &str| catalog::key_label(c).unwrap_or_default();
    div()
        .flex()
        .flex_row()
        .gap(shell.z(2.0))
        .px(shell.z(4.0))
        .child(button(
            "layout-sidebar",
            sidebar_icon,
            label(
                host,
                "layout.toggleSidebar",
                &[("key", key("workbench.action.toggleSidebarVisibility"))],
            ),
            "workbench.action.toggleSidebarVisibility",
        ))
        .child(button(
            "layout-panel",
            panel_icon,
            label(
                host,
                "layout.togglePanel",
                &[("key", key("workbench.action.togglePanel"))],
            ),
            "workbench.action.togglePanel",
        ))
        .into_any_element()
}

fn render_window_controls<T: ShellHost>(host: &T, window: &Window) -> Option<AnyElement> {
    let shell = host.shell();
    if shell.setting_str("window.controlsStyle") == "hidden" {
        return None;
    }
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let hover = p
        .get("toolbar.hoverBackground")
        .or(p.get("menubar.selectionBackground"));
    let maximized = window.is_maximized();
    let size = shell.z(16.0);
    let button =
        |id: &'static str, name: &str, area: WindowControlArea, tip: String, close: bool| {
            div()
                .id(id)
                .w(shell.z(46.0))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .window_control_area(area)
                .when(close, |d| {
                    d.hover(|s| s.bg(rgba(0xe81123ff)).text_color(gpui::white()))
                })
                .when(!close, |d| match hover {
                    Some(c) => d.hover(move |s| s.bg(c)),
                    None => d,
                })
                .tooltip(tooltip(tip, &p))
                .child(named_icon(name, size))
        };
    Some(
        div()
            .h_full()
            .flex()
            .flex_row()
            .child(button(
                "window-min",
                "chrome-minimize",
                WindowControlArea::Min,
                label(host, "window.minimize", &[]),
                false,
            ))
            .child(button(
                "window-max",
                if maximized {
                    "chrome-restore"
                } else {
                    "chrome-maximize"
                },
                WindowControlArea::Max,
                label(
                    host,
                    if maximized {
                        "window.restore"
                    } else {
                        "window.maximize"
                    },
                    &[],
                ),
                false,
            ))
            .child(button(
                "window-close",
                "chrome-close",
                WindowControlArea::Close,
                label(host, "window.close", &[]),
                true,
            ))
            .into_any_element(),
    )
}

// --------------------------------------------------------------------- 菜单

fn render_menubar<T: ShellHost>(host: &T, cx: &mut Context<T>) -> AnyElement {
    let shell = host.shell();
    let visibility = shell.setting_str("window.menuBarVisibility");
    let implemented = |id: &str| catalog::is_implemented(id) || shell.external_implemented(id);
    let menus = visible_menus(&shell.menus, host.commands(), &implemented);
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let hidden = match visibility.as_str() {
        "hidden" => true,
        "toggle" => !shell.menubar_revealed && shell.open_menu.is_none(),
        _ => false,
    };
    let mut bar = div()
        .id("menubar")
        .h_full()
        .flex()
        .flex_row()
        .items_center()
        .pl(shell.z(4.0));
    if hidden {
        return bar.into_any_element();
    }
    let selection_bg = p.get("menubar.selectionBackground");
    let selection_fg = p.get("menubar.selectionForeground");
    let open = shell.open_menu;
    let compact = visibility == "compact";
    let entries: Vec<(MenuId, String)> = if compact {
        vec![(MenuId::File, String::new())]
    } else {
        menus
            .iter()
            .map(|m| (*m, label(host, m.title_key(), &[])))
            .collect()
    };
    for (menu, title) in entries {
        let is_open = open == Some(menu) || (compact && open.is_some());
        let shell = host.shell();
        let mut button = div()
            .id(SharedString::from(format!("menubar-{menu:?}")))
            .relative()
            .h(shell.z(26.0))
            .px(shell.z(8.0))
            .flex()
            .items_center()
            .rounded(shell.z(4.0))
            .when_some(selection_bg, |d, c| d.hover(move |s| s.bg(c)))
            .when(is_open, |d| match (selection_bg, selection_fg) {
                (Some(bg), Some(fg)) => d.bg(bg).text_color(fg),
                (Some(bg), None) => d.bg(bg),
                _ => d,
            })
            .on_click(cx.listener(move |host: &mut T, _, _, cx| {
                let shell = host.shell_mut();
                shell.open_menu = if shell.open_menu.is_some() {
                    None
                } else {
                    Some(menu)
                };
                cx.notify();
            }))
            .on_hover(cx.listener(move |host: &mut T, hovered: &bool, _, cx| {
                let shell = host.shell_mut();
                if *hovered
                    && shell.open_menu.is_some()
                    && !compact
                    && shell.open_menu != Some(menu)
                {
                    shell.open_menu = Some(menu);
                    cx.notify();
                }
            }));
        button = if compact {
            button.child(named_icon("menu", shell.z(16.0)))
        } else {
            button.child(title)
        };
        if is_open {
            let menus_to_show = if compact { menus.clone() } else { vec![menu] };
            button = button.child(render_menu_dropdown(host, &menus_to_show, cx));
        }
        bar = bar.child(button);
    }
    bar.into_any_element()
}

fn render_menu_dropdown<T: ShellHost>(
    host: &T,
    menus: &[MenuId],
    cx: &mut Context<T>,
) -> AnyElement {
    let shell = host.shell();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let implemented = |id: &str| catalog::is_implemented(id) || shell.external_implemented(id);
    let sel_bg = p.or("menu.selectionBackground", "menubar.selectionBackground");
    let sel_fg = p.or("menu.selectionForeground", "menu.foreground");
    let sep = p.or("menu.separatorBackground", "menu.border");
    let mut list = div()
        .id("menu-dropdown")
        .occlude()
        .absolute()
        .top(shell.z(28.0))
        .left_0()
        .min_w(shell.z(240.0))
        .py(shell.z(4.0))
        .flex()
        .flex_col()
        .bg(p.or("menu.background", "notifications.background"))
        .text_color(p.or("menu.foreground", "notifications.foreground"))
        .border_1()
        .border_color(p.or("menu.border", "notifications.border"))
        .rounded(shell.z(6.0))
        .shadow_md()
        .on_mouse_down_out(cx.listener(|host: &mut T, _, _, cx| {
            host.shell_mut().open_menu = None;
            cx.notify();
        }));
    for (mi, menu) in menus.iter().enumerate() {
        if menus.len() > 1 {
            if mi > 0 {
                list = list.child(div().h(px(1.0)).mx(shell.z(8.0)).my(shell.z(4.0)).bg(sep));
            }
            list = list.child(
                div()
                    .px(shell.z(12.0))
                    .h(shell.z(22.0))
                    .flex()
                    .items_center()
                    .opacity(0.7)
                    .child(label(host, menu.title_key(), &[])),
            );
        }
        let mut last_group: Option<Arc<str>> = None;
        for (id, item) in visible_items(&shell.menus, *menu, host.commands(), &implemented) {
            if last_group.as_ref().is_some_and(|g| g != &item.group) {
                list = list.child(div().h(px(1.0)).mx(shell.z(8.0)).my(shell.z(4.0)).bg(sep));
            }
            last_group = Some(item.group.clone());
            let title = match &item.title {
                Some(m) => text(host, m),
                None => host
                    .commands()
                    .get(&item.command)
                    .map(|c| text(host, &Message::key(c.title_key.clone())))
                    .unwrap_or_default(),
            };
            let command = item.command.clone();
            list = list.child(
                div()
                    .id(SharedString::from(id.to_owned()))
                    .mx(shell.z(4.0))
                    .px(shell.z(12.0))
                    .h(shell.z(24.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(shell.z(24.0))
                    .rounded(shell.z(4.0))
                    .hover(move |s| s.bg(sel_bg).text_color(sel_fg))
                    .child(title)
                    .children(catalog::key_label(&command).map(|k| div().opacity(0.8).child(k)))
                    .on_click(cx.listener(move |host: &mut T, _, window, cx| {
                        host.shell_mut().open_menu = None;
                        run_command(host, &command, window, cx);
                    })),
            );
        }
    }
    deferred(list).with_priority(1).into_any_element()
}

// --------------------------------------------------------------------- 活动栏

fn render_activity_bar<T: ShellHost>(host: &T, cx: &mut Context<T>) -> AnyElement {
    let shell = host.shell();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let active = shell.active_container();
    let sidebar_visible = shell.layout.sidebar_visible;
    let right = shell.sidebar_location() == SideBarLocation::Right;
    let fg = p.or("activityBar.foreground", "sideBar.foreground");
    let inactive = p.or("activityBar.inactiveForeground", "activityBar.foreground");
    let indicator = p.get("activityBar.activeBorder");
    let active_bg = p.get("activityBar.activeBackground");
    let mut containers: Vec<(&str, &Contribution)> = shell
        .contributions
        .entries()
        .filter(|(_, c)| matches!(c.placement, Placement::ActivityBar))
        .collect();
    containers.sort_by(|a, b| a.1.order.cmp(&b.1.order).then(a.0.cmp(b.0)));
    let size = shell.z(24.0);
    let mut items = div().flex().flex_col();
    for (id, c) in containers {
        let is_active = sidebar_visible && id == active;
        let id_owned: Arc<str> = id.into();
        let tip = {
            let title = text(host, &c.title);
            match c.command.as_deref().and_then(catalog::key_label) {
                Some(k) => format!("{title} ({k})"),
                None => title,
            }
        };
        let glyph = c.icon.as_deref().and_then(icons::codepoint);
        items = items.child(
            div()
                .id(SharedString::from(format!("activity-{id}")))
                .relative()
                .w(shell.z(ACTIVITY_BAR_WIDTH))
                .h(shell.z(ACTIVITY_BAR_WIDTH))
                .flex()
                .items_center()
                .justify_center()
                .text_color(if is_active { fg } else { inactive })
                .hover(move |s| s.text_color(fg))
                .when(is_active, |d| match active_bg {
                    Some(c) => d.bg(c),
                    None => d,
                })
                .when(is_active && indicator.is_some(), |d| {
                    let bar = div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .w(px(2.0))
                        .bg(indicator.unwrap());
                    d.child(if right { bar.right_0() } else { bar.left_0() })
                })
                .tooltip(tooltip(tip, &p))
                .child(match glyph {
                    Some(g) => icon(g, size),
                    None => div().child(text(host, &c.title)),
                })
                .on_click(cx.listener(move |host: &mut T, _, window, cx| {
                    let shell = host.shell_mut();
                    // 与 VS Code 相同：点击当前活动的容器会收起侧边栏。
                    if shell.layout.sidebar_visible && shell.active_container() == id_owned.as_ref()
                    {
                        shell.layout.sidebar_visible = false;
                        window.focus(&host.editor_focus());
                    } else {
                        shell.layout.sidebar_visible = true;
                        shell.layout.active_container = Some(id_owned.to_string());
                    }
                    host.shell_mut().schedule_save(cx);
                    cx.notify();
                })),
        );
    }
    let open = shell.open_menu == Some(MenuId::Manage);
    let mut manage = div()
        .id("activity-manage")
        .relative()
        .w(shell.z(ACTIVITY_BAR_WIDTH))
        .h(shell.z(ACTIVITY_BAR_WIDTH))
        .flex()
        .items_center()
        .justify_center()
        .text_color(if open { fg } else { inactive })
        .hover(move |s| s.text_color(fg))
        .tooltip(tooltip(label(host, "activityBar.manage", &[]), &p))
        .child(named_icon("settings-gear", size))
        .on_click(cx.listener(|host: &mut T, _, _, cx| {
            let shell = host.shell_mut();
            shell.open_menu = if shell.open_menu == Some(MenuId::Manage) {
                None
            } else {
                Some(MenuId::Manage)
            };
            cx.notify();
        }));
    if open {
        let implemented = |id: &str| catalog::is_implemented(id) || shell.external_implemented(id);
        if !visible_items(&shell.menus, MenuId::Manage, host.commands(), &implemented).is_empty() {
            manage = manage.child(
                div()
                    .absolute()
                    .bottom(shell.z(ACTIVITY_BAR_WIDTH - 28.0))
                    .left(shell.z(ACTIVITY_BAR_WIDTH))
                    .child(render_menu_dropdown(host, &[MenuId::Manage], cx)),
            );
        }
    }
    div()
        .id("activitybar")
        .debug_selector(|| "activitybar".into())
        .track_focus(&shell.activity_focus.clone())
        .w(shell.z(ACTIVITY_BAR_WIDTH))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .justify_between()
        .bg(p.or("activityBar.background", "sideBar.background"))
        .when_some(p.get("activityBar.border"), |d, c| {
            if right {
                d.border_l_1().border_color(c)
            } else {
                d.border_r_1().border_color(c)
            }
        })
        .child(items)
        .child(manage)
        .into_any_element()
}

// --------------------------------------------------------------------- 侧边栏

fn render_sidebar<T: ShellHost>(
    host: &mut T,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyElement {
    let container = host.shell().active_container();
    let mut sections: Vec<(String, Contribution)> = host
        .shell()
        .contributions
        .entries()
        .filter(|(_, c)| matches!(&c.placement, Placement::Sidebar { container: k } if k.as_ref() == container))
        .map(|(id, c)| (id.to_owned(), c.clone()))
        .collect();
    sections.sort_by(|a, b| a.1.order.cmp(&b.1.order).then(a.0.cmp(&b.0)));
    let title = host
        .shell()
        .contributions
        .get(&container)
        .map(|c| text(host, &c.title))
        .unwrap_or_default();
    let mut body = div().flex().flex_col().flex_1().min_h_0();
    for (id, c) in sections {
        let collapsed = host.shell().layout.collapsed_sections.contains(&id);
        let content = (!collapsed)
            .then(|| host.render_view(&c.factory_key, window, cx))
            .flatten();
        let shell = host.shell();
        let p = Palette {
            shell,
            theme: host.theme(),
        };
        let header_title = host
            .section_title(&id)
            .unwrap_or_else(|| text(host, &c.title));
        let chevron = if collapsed {
            "chevron-right"
        } else {
            "chevron-down"
        };
        let id_owned = id.clone();
        let header = div()
            .id(SharedString::from(format!("section-{id}")))
            .h(shell.z(SECTION_HEADER_HEIGHT))
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(shell.z(2.0))
            .px(shell.z(2.0))
            .cursor_pointer()
            .font_weight(gpui::FontWeight::BOLD)
            .text_size(shell.z(11.0))
            .bg(p.or("sideBarSectionHeader.background", "sideBar.background"))
            .text_color(p.or("sideBarSectionHeader.foreground", "sideBar.foreground"))
            .when_some(p.get("sideBarSectionHeader.border"), |d, c| {
                d.border_t_1().border_color(c)
            })
            .child(named_icon(chevron, shell.z(16.0)))
            .child(header_title.to_uppercase())
            .on_click(cx.listener(move |host: &mut T, _, _, cx| {
                let shell = host.shell_mut();
                if !shell.layout.collapsed_sections.remove(&id_owned) {
                    shell.layout.collapsed_sections.insert(id_owned.clone());
                }
                shell.schedule_save(cx);
                cx.notify();
            }));
        let section = div()
            .flex()
            .flex_col()
            .when(!collapsed, |d| d.flex_1().min_h_0())
            .child(header)
            .children(content.map(|c| div().flex().flex_col().flex_1().min_h_0().child(c)));
        body = body.child(section);
    }
    let shell = host.shell();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let right = shell.sidebar_location() == SideBarLocation::Right;
    div()
        .id("sidebar")
        .debug_selector(|| "sidebar".into())
        .track_focus(&shell.sidebar_focus.clone())
        .w(shell.z(shell.layout.sidebar_width))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .bg(p.or("sideBar.background", "editor.background"))
        .text_color(p.or("sideBar.foreground", "editor.foreground"))
        .when_some(p.get("sideBar.border"), |d, c| {
            if right {
                d.border_l_1().border_color(c)
            } else {
                d.border_r_1().border_color(c)
            }
        })
        .child(
            div()
                .h(shell.z(PART_TITLE_HEIGHT))
                .flex_shrink_0()
                .flex()
                .items_center()
                .px(shell.z(20.0))
                .text_size(shell.z(11.0))
                .bg(p.or("sideBarTitle.background", "sideBar.background"))
                .text_color(p.or("sideBarTitle.foreground", "sideBar.foreground"))
                .child(title),
        )
        .child(body)
        .into_any_element()
}

// --------------------------------------------------------------------- 编辑区与面板

fn render_editor_area<T: ShellHost>(
    host: &mut T,
    panel_visible: bool,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyElement {
    let editor = host.render_editor(window, cx);
    let bars = render_editor_bars(host, cx);
    let hint = render_cursor_hint(host);
    let editor = div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .children(bars)
        .child(div().flex_1().min_h_0().child(editor))
        .children(hint);
    // 面板默认隐藏；隐藏时不调用面板视图工厂（按需创建）。
    if !panel_visible {
        return div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(editor)
            .into_any_element();
    }
    let panel = render_panel(host, window, cx);
    let location = host.shell().panel_location();
    let sash = if location.is_horizontal() {
        sash_vertical(host, cx, false)
    } else {
        sash_horizontal(cx)
    };
    let container = div().flex().flex_1().min_w_0().h_full();
    let container = if location.is_horizontal() {
        container.flex_row()
    } else {
        container.flex_col()
    };
    let container = match location {
        PanelLocation::Bottom | PanelLocation::Right => {
            container.child(editor).child(sash).child(panel)
        }
        PanelLocation::Top | PanelLocation::Left => {
            container.child(panel).child(sash).child(editor)
        }
    };
    container.into_any_element()
}

fn render_panel<T: ShellHost>(
    host: &mut T,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyElement {
    let active = host.shell().active_panel_view();
    let mut views: Vec<(String, Contribution)> = host
        .shell()
        .contributions
        .entries()
        .filter(|(_, c)| matches!(c.placement, Placement::Panel))
        .map(|(id, c)| (id.to_owned(), c.clone()))
        .collect();
    views.sort_by(|a, b| a.1.order.cmp(&b.1.order).then(a.0.cmp(&b.0)));
    let content = views
        .iter()
        .find(|(id, _)| *id == active)
        .and_then(|(_, c)| host.render_view(&c.factory_key, window, cx));
    let shell = host.shell();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let show_labels = shell.setting_bool("workbench.panel.showLabels");
    let location = shell.panel_location();
    let active_fg = p.or("panelTitle.activeForeground", "sideBar.foreground");
    let inactive_fg = p.or("panelTitle.inactiveForeground", "sideBar.foreground");
    let active_border = p.get("panelTitle.activeBorder");
    let mut tabs = div().flex().flex_row().h_full().gap(shell.z(16.0));
    for (id, c) in &views {
        let is_active = *id == active;
        let title = text(host, &c.title);
        let glyph = c.icon.as_deref().and_then(icons::codepoint);
        let id_owned = id.clone();
        tabs = tabs.child(
            div()
                .id(SharedString::from(format!("panel-tab-{id}")))
                .h_full()
                .flex()
                .items_center()
                .text_size(shell.z(11.0))
                .text_color(if is_active { active_fg } else { inactive_fg })
                .when(is_active, |d| match active_border {
                    Some(c) => d.border_b_1().border_color(c),
                    None => d,
                })
                .tooltip(tooltip(title.clone(), &p))
                .child(match (show_labels, glyph) {
                    (false, Some(g)) => icon(g, shell.z(16.0)),
                    _ => div().child(title.to_uppercase()),
                })
                .on_click(cx.listener(move |host: &mut T, _, _, cx| {
                    host.shell_mut().layout.active_panel_view = Some(id_owned.clone());
                    host.shell_mut().schedule_save(cx);
                    cx.notify();
                })),
        );
    }
    let close = div()
        .id("panel-close")
        .w(shell.z(22.0))
        .h(shell.z(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(shell.z(4.0))
        .tooltip(tooltip(label(host, "panel.close", &[]), &p))
        .child(named_icon("close", shell.z(16.0)))
        .on_click(cx.listener(|host: &mut T, _, window, cx| {
            run_command(host, "workbench.action.closePanel", window, cx)
        }));
    let size = shell.z(shell.layout.panel_size);
    div()
        .id("panel")
        .debug_selector(|| "panel".into())
        .track_focus(&shell.panel_focus.clone())
        .flex()
        .flex_col()
        .flex_shrink_0()
        .map(|d| {
            if location.is_horizontal() {
                d.w(size).h_full()
            } else {
                d.h(size).w_full()
            }
        })
        .bg(p.or("panel.background", "editor.background"))
        .when_some(p.get("panel.border"), |d, c| match location {
            PanelLocation::Bottom => d.border_t_1().border_color(c),
            PanelLocation::Top => d.border_b_1().border_color(c),
            PanelLocation::Left => d.border_r_1().border_color(c),
            PanelLocation::Right => d.border_l_1().border_color(c),
        })
        .child(
            div()
                .h(shell.z(PART_TITLE_HEIGHT))
                .flex_shrink_0()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(shell.z(8.0))
                .child(tabs)
                .child(close),
        )
        .child(div().flex_1().min_h_0().px(shell.z(20.0)).children(content))
        .into_any_element()
}

/// 问题面板的空状态（内容由 R60 填充）。
pub fn render_problems_empty<T: ShellHost>(host: &T) -> AnyElement {
    div()
        .text_size(host.shell().z(UI_FONT_SIZE))
        .child(label(host, "problems.empty", &[]))
        .into_any_element()
}

fn render_editor_bars<T: ShellHost>(host: &T, cx: &mut Context<T>) -> Vec<AnyElement> {
    let Some(document) = host.active_document() else {
        return Vec::new();
    };
    let shell = host.shell();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let bg = p.or("notifications.background", "sideBar.background");
    let border = p.or("notifications.border", "panel.border");
    let link = p.or("notificationLink.foreground", "progressBar.background");
    shell
        .feedback
        .editor_bars(document)
        .map(|bar| {
            let mut row = div()
                .id(SharedString::from(format!("editor-bar-{}", bar.key)))
                .w_full()
                .min_h(shell.z(26.0))
                .px(shell.z(12.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(shell.z(12.0))
                .bg(bg)
                .border_b_1()
                .border_color(border)
                .child(div().flex_1().child(icon_label(
                    &format!("$(info) {}", text(host, &bar.message)),
                    shell.z(UI_FONT_SIZE),
                )));
            for action in bar.actions.iter() {
                let (key, id) = (bar.key.clone(), action.id.clone());
                row = row.child(
                    div()
                        .id(SharedString::from(format!("bar-{}-{}", bar.key, action.id)))
                        .text_color(link)
                        .cursor_pointer()
                        .child(text(host, &action.label))
                        .on_click(cx.listener(move |host: &mut T, _, _, cx| {
                            let _ = host.shell_mut().feedback.act(&key, &id);
                            cx.notify();
                        })),
                );
            }
            if bar.suppress_key.is_some() {
                let key = bar.key.clone();
                row = row.child(
                    div()
                        .id(SharedString::from(format!("bar-never-{}", bar.key)))
                        .text_color(link)
                        .cursor_pointer()
                        .child(label(host, NEVER_SHOW_AGAIN, &[]))
                        .on_click(cx.listener(move |host: &mut T, _, _, cx| {
                            let _ = host.shell_mut().feedback.never_show_again(&key);
                            host.shell_mut().schedule_save(cx);
                            cx.notify();
                        })),
                );
            }
            if bar.dismissible {
                let key = bar.key.clone();
                row = row.child(
                    div()
                        .id(SharedString::from(format!("bar-close-{}", bar.key)))
                        .cursor_pointer()
                        .child(named_icon("close", shell.z(16.0)))
                        .on_click(cx.listener(move |host: &mut T, _, _, cx| {
                            let _ = host.shell_mut().feedback.dismiss(&key);
                            cx.notify();
                        })),
                );
            }
            row.into_any_element()
        })
        .collect()
}

fn render_cursor_hint<T: ShellHost>(host: &T) -> Option<AnyElement> {
    let shell = host.shell();
    let hint = shell.feedback.cursor_hint()?;
    if Some(hint.document) != host.active_document() {
        return None;
    }
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    // 精确定位到光标需要代码视图的坐标（#19）；在那之前显示在编辑区左上角。
    Some(
        div()
            .absolute()
            .top(shell.z(8.0))
            .left(shell.z(64.0))
            .px(shell.z(8.0))
            .py(shell.z(4.0))
            .bg(p.or("notifications.background", "sideBar.background"))
            .border_1()
            .border_color(p.or("notificationsInfoIcon.foreground", "notifications.border"))
            .child(text(host, &hint.message))
            .into_any_element(),
    )
}

// --------------------------------------------------------------------- 状态栏

fn status_kind_colors(p: &Palette, kind: StatusKind) -> (Option<Hsla>, Option<Hsla>, Option<Hsla>) {
    let prefix = match kind {
        StatusKind::Standard => return (None, None, p.get("statusBarItem.hoverBackground")),
        StatusKind::Prominent => "statusBarItem.prominent",
        StatusKind::Warning => "statusBarItem.warning",
        StatusKind::Error => "statusBarItem.error",
    };
    (
        p.get(&format!("{prefix}Background")),
        p.get(&format!("{prefix}Foreground")),
        p.get(&format!("{prefix}HoverBackground")),
    )
}

/// 状态栏左侧临时消息的纯文本（测试与无障碍用）。
pub fn status_message_text<T: ShellHost>(host: &T) -> Option<String> {
    host.shell()
        .feedback
        .status()
        .map(|s| icons::strip_icons(&text(host, &s.message)))
}

fn update_builtin_status_items<T: ShellHost>(host: &mut T) {
    let (errors, warnings) = host.shell().problems;
    let problems = StatusEntry {
        text: label(
            host,
            "status.problems.text",
            &[
                ("errors", errors.to_string()),
                ("warnings", warnings.to_string()),
            ],
        ),
        tooltip: Some(if errors + warnings == 0 {
            label(host, "status.problems.none", &[])
        } else {
            label(
                host,
                "status.problems.tooltip",
                &[
                    ("errors", errors.to_string()),
                    ("warnings", warnings.to_string()),
                ],
            )
        }),
        command: None,
        kind: StatusKind::Standard,
    };
    let shell = host.shell();
    let unread = shell.feedback.notifications().len();
    let dnd = shell.feedback.do_not_disturb();
    let show_bell = !(shell.setting_str("workbench.notifications.position") == "top-right"
        && shell.setting_bool("workbench.notifications.showInTitleBar"));
    let bell = StatusEntry {
        text: if dnd {
            "$(bell-slash)".into()
        } else if unread > 0 {
            "$(bell-dot)".into()
        } else {
            "$(bell)".into()
        },
        tooltip: Some(if dnd {
            label(host, "status.notifications.dnd", &[])
        } else if unread > 0 {
            label(
                host,
                "status.notifications.some",
                &[("count", unread.to_string())],
            )
        } else {
            label(host, "status.notifications.none", &[])
        }),
        command: None,
        kind: StatusKind::Standard,
    };
    let shell = host.shell_mut();
    let contributions = std::mem::take(&mut shell.contributions);
    let _ = shell
        .status_bar
        .set(&contributions, STATUS_PROBLEMS, problems);
    if show_bell {
        let _ = shell
            .status_bar
            .set(&contributions, STATUS_NOTIFICATIONS, bell);
    } else {
        shell.status_bar.remove(STATUS_NOTIFICATIONS);
    }
    shell.contributions = contributions;
}

fn render_status_bar<T: ShellHost>(
    host: &mut T,
    window: &mut Window,
    cx: &mut Context<T>,
) -> AnyElement {
    update_builtin_status_items(host);
    let message = status_message_text(host);
    let shell = host.shell();
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let focused = shell.status_focus.contains_focused(window, cx);
    let focus_order = shell.status_bar.focus_order(&shell.contributions);
    let focused_id = focused
        .then(|| focus_order.get(shell.status_index).copied())
        .flatten();
    let focus_border = p.or("statusBarItem.focusBorder", "statusBar.focusBorder");
    let fg = p.or("statusBar.foreground", "sideBar.foreground");
    let size = shell.z(12.0);
    let item = |id: &str, c: &Contribution, e: &StatusEntry| {
        let (bg, kind_fg, hover) = status_kind_colors(&p, e.kind);
        let command = e.command.clone().or_else(|| c.command.clone());
        let tip = e.tooltip.clone().unwrap_or_else(|| text(host, &c.title));
        div()
            .id(SharedString::from(format!("status-{id}")))
            .h_full()
            .px(shell.z(5.0))
            .flex()
            .items_center()
            .flex_shrink_0()
            .when_some(bg, |d, c| d.bg(c))
            .when_some(kind_fg, |d, c| d.text_color(c))
            .when_some(hover, |d, c| d.hover(move |s| s.bg(c)))
            .when(focused_id == Some(id), |d| {
                d.border_1().border_color(focus_border)
            })
            .when(command.is_some(), |d| d.cursor_pointer())
            .tooltip(tooltip(tip, &p))
            .child(icon_label(&e.text, size))
            .on_click(cx.listener(move |host: &mut T, _, window, cx| {
                if let Some(command) = &command {
                    run_command(host, command, window, cx);
                }
            }))
    };
    let left: Vec<_> = shell
        .status_bar
        .items(&shell.contributions, Alignment::Left)
        .into_iter()
        .map(|(id, c, e)| item(id, c, e))
        .collect();
    let right: Vec<_> = shell
        .status_bar
        .items(&shell.contributions, Alignment::Right)
        .into_iter()
        .map(|(id, c, e)| item(id, c, e))
        .collect();
    let focus_order: Vec<String> = focus_order.into_iter().map(str::to_owned).collect();
    div()
        .id("statusbar")
        .debug_selector(|| "statusbar".into())
        .track_focus(&shell.status_focus.clone())
        .key_context(if focused {
            CONTEXT_STATUS_BAR
        } else {
            "StatusBar"
        })
        .h(shell.z(STATUS_BAR_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .flex()
        .flex_row()
        .justify_between()
        .overflow_hidden()
        .text_size(size)
        .bg(p.or("statusBar.background", "sideBar.background"))
        .text_color(fg)
        .when_some(p.get("statusBar.border"), |d, c| {
            d.border_t_1().border_color(c)
        })
        .on_key_down(
            cx.listener(move |host: &mut T, e: &KeyDownEvent, window, cx| {
                // Enter / Space 执行聚焦条目的命令（与 VS Code 状态栏键盘操作相同）。
                if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                    let shell = host.shell();
                    let command = focus_order.get(shell.status_index).and_then(|id| {
                        shell
                            .status_bar
                            .get(id)
                            .and_then(|e| e.command.clone())
                            .or_else(|| shell.contributions.get(id).and_then(|c| c.command.clone()))
                    });
                    if let Some(command) = command {
                        run_command(host, &command, window, cx);
                        cx.stop_propagation();
                    }
                }
            }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .h_full()
                .pl(shell.z(5.0))
                .children(message.map(|m| {
                    div()
                        .id("status-message")
                        .h_full()
                        .px(shell.z(5.0))
                        .flex()
                        .items_center()
                        .child(m)
                }))
                .children(left),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .h_full()
                .pr(shell.z(5.0))
                .children(right),
        )
        .into_any_element()
}

// --------------------------------------------------------------------- 通知

fn severity_icon(severity: &Severity) -> (&'static str, &'static str) {
    match severity {
        Severity::Info => ("info", "notificationsInfoIcon.foreground"),
        Severity::Warning => ("warning", "notificationsWarningIcon.foreground"),
        Severity::Error => ("error", "notificationsErrorIcon.foreground"),
    }
}

fn render_notifications<T: ShellHost>(
    host: &T,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Option<AnyElement> {
    let shell = host.shell();
    let center = shell.feedback.center_visible();
    let list: Vec<_> = if center {
        shell.feedback.notifications().iter().collect()
    } else {
        shell.feedback.toasts().collect()
    };
    if list.is_empty() && !center {
        return None;
    }
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    let focused = shell.notification_focus.contains_focused(window, cx);
    let bg = p.or("notifications.background", "sideBar.background");
    let fg = p.or("notifications.foreground", "sideBar.foreground");
    let border = p.or(
        if center {
            "notificationCenter.border"
        } else {
            "notificationToast.border"
        },
        "notifications.border",
    );
    let link = p.or("notificationLink.foreground", "progressBar.background");
    let size = shell.z(UI_FONT_SIZE);
    let mut column = div()
        .flex()
        .flex_col()
        .gap(if center { px(0.0) } else { shell.z(8.0) });
    if center {
        column = column.child(
            div()
                .h(shell.z(35.0))
                .px(shell.z(8.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .bg(p.or("notificationCenterHeader.background", "sideBar.background"))
                .text_color(p.or("notificationCenterHeader.foreground", "sideBar.foreground"))
                .child(
                    label(
                        host,
                        if list.is_empty() {
                            "notifications.centerEmpty"
                        } else {
                            "notifications.center"
                        },
                        &[],
                    )
                    .to_uppercase(),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(shell.z(4.0))
                        .child(header_button(
                            host,
                            "notif-clear-all",
                            "clear-all",
                            "notifications.clearAll",
                            &p,
                            cx,
                        ))
                        .child(header_button(
                            host,
                            "notif-dnd",
                            if shell.feedback.do_not_disturb() {
                                "bell-slash"
                            } else {
                                "bell"
                            },
                            "notifications.toggleDoNotDisturbMode",
                            &p,
                            cx,
                        ))
                        .child(header_button(
                            host,
                            "notif-hide",
                            "chevron-down",
                            "notifications.hideList",
                            &p,
                            cx,
                        )),
                ),
        );
    }
    for (i, n) in list.iter().enumerate() {
        let (icon_name, icon_color) = severity_icon(&n.severity);
        let message = text(host, &n.message);
        let key = n.key.clone();
        let mut card = div()
            .id(SharedString::from(format!("notification-{}", n.key)))
            .w_full()
            .p(shell.z(10.0))
            .flex()
            .flex_col()
            .gap(shell.z(8.0))
            .bg(bg)
            .text_color(fg)
            .when(!center, |d| {
                d.border_1()
                    .border_color(border)
                    .rounded(shell.z(4.0))
                    .shadow_lg()
            })
            .when(center && i > 0, |d| d.border_t_1().border_color(border))
            .when(focused && i == shell.toast_index, |d| {
                d.border_1()
                    .border_color(p.or("focusBorder", "statusBar.focusBorder"))
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(shell.z(8.0))
                    .child(
                        named_icon(icon_name, shell.z(16.0))
                            .text_color(p.or(icon_color, "notifications.foreground")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .when(!n.expanded, |d| d.max_h(shell.z(66.0)).overflow_hidden())
                            .child(message),
                    )
                    .when(n.count > 1, |d| {
                        d.child(
                            div()
                                .px(shell.z(6.0))
                                .rounded(shell.z(8.0))
                                .bg(p.or("activityBarBadge.background", "progressBar.background"))
                                .text_color(
                                    p.or("activityBarBadge.foreground", "notifications.foreground"),
                                )
                                .text_size(shell.z(11.0))
                                .child(label(
                                    host,
                                    "notifications.count",
                                    &[("count", n.count.to_string())],
                                )),
                        )
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("notification-close-{}", n.key)))
                            .cursor_pointer()
                            .tooltip(tooltip(label(host, "notifications.close", &[]), &p))
                            .child(named_icon("close", shell.z(16.0)))
                            .on_click(cx.listener(move |host: &mut T, _, _, cx| {
                                let _ = host.shell_mut().feedback.dismiss(&key);
                                cx.notify();
                            })),
                    ),
            );
        if !n.actions.is_empty() {
            let mut buttons = div().flex().flex_row().justify_end().gap(shell.z(8.0));
            for (ai, action) in n.actions.iter().enumerate() {
                let (key, id) = (n.key.clone(), action.id.clone());
                let primary = ai == 0;
                buttons = buttons.child(
                    div()
                        .id(SharedString::from(format!(
                            "notification-{}-{}",
                            n.key, action.id
                        )))
                        .px(shell.z(10.0))
                        .py(shell.z(2.0))
                        .rounded(shell.z(2.0))
                        .cursor_pointer()
                        .when(primary, |d| {
                            d.bg(p.or("button.background", "progressBar.background"))
                                .text_color(p.get("button.foreground").unwrap_or(gpui::white()))
                        })
                        .when(!primary, |d| {
                            d.border_1().border_color(border).text_color(link)
                        })
                        .child(text(host, &action.label))
                        .on_click(cx.listener(move |host: &mut T, _, _, cx| {
                            let _ = host.shell_mut().feedback.act(&key, &id);
                            cx.notify();
                        })),
                );
            }
            card = card.child(buttons);
        }
        column = column.child(card);
    }
    let position = shell.setting_str("workbench.notifications.position");
    let bottom = shell.z(STATUS_BAR_HEIGHT + 8.0);
    let wrapper = div()
        .id("notifications")
        .track_focus(&shell.notification_focus.clone())
        .key_context(if focused {
            CONTEXT_NOTIFICATION
        } else {
            "Notifications"
        })
        .occlude()
        .absolute()
        .w(shell.z(450.0))
        .text_size(size)
        .when(center, |d| {
            d.border_1().border_color(border).bg(bg).shadow_lg()
        })
        .map(|d| match position.as_str() {
            "bottom-left" => d.bottom(bottom).left(shell.z(8.0)),
            "top-right" => d.top(shell.z(TITLE_BAR_HEIGHT + 8.0)).right(shell.z(8.0)),
            _ => d.bottom(bottom).right(shell.z(8.0)),
        })
        .child(column);
    Some(deferred(wrapper).with_priority(2).into_any_element())
}

fn header_button<T: ShellHost>(
    host: &T,
    id: &'static str,
    icon_name: &str,
    command: &'static str,
    p: &Palette,
    cx: &mut Context<T>,
) -> AnyElement {
    let shell = host.shell();
    let title = host
        .commands()
        .get(command)
        .map(|c| text(host, &Message::key(c.title_key.clone())))
        .unwrap_or_default();
    div()
        .id(id)
        .w(shell.z(22.0))
        .h(shell.z(22.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .tooltip(tooltip(title, p))
        .child(named_icon(icon_name, shell.z(16.0)))
        .on_click(
            cx.listener(move |host: &mut T, _, window, cx| run_command(host, command, window, cx)),
        )
        .into_any_element()
}

fn render_modal<T: ShellHost>(
    host: &T,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Option<AnyElement> {
    let shell = host.shell();
    let modal = shell.feedback.modal()?;
    let p = Palette {
        shell,
        theme: host.theme(),
    };
    if !shell.modal_focus.contains_focused(window, cx) {
        let handle = shell.modal_focus.clone();
        window.defer(cx, move |window, _| window.focus(&handle));
    }
    let mut buttons = div().flex().flex_row().justify_end().gap(shell.z(8.0));
    for (i, action) in modal.actions.iter().enumerate() {
        let (key, id) = (modal.key.clone(), action.id.clone());
        buttons = buttons.child(
            div()
                .id(SharedString::from(format!("modal-{}", action.id)))
                .px(shell.z(14.0))
                .py(shell.z(4.0))
                .rounded(shell.z(2.0))
                .cursor_pointer()
                .when(i == 0, |d| {
                    d.bg(p.or("button.background", "progressBar.background"))
                        .text_color(p.get("button.foreground").unwrap_or(gpui::white()))
                })
                .when(i > 0, |d| {
                    d.border_1()
                        .border_color(p.or("notifications.border", "panel.border"))
                })
                .child(text(host, &action.label))
                .on_click(cx.listener(move |host: &mut T, _, _, cx| {
                    let _ = host.shell_mut().feedback.act(&key, &id);
                    cx.notify();
                })),
        );
    }
    Some(
        deferred(
            div()
                .id("modal-backdrop")
                .occlude()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x0000004d))
                .child(
                    div()
                        .id("modal")
                        .track_focus(&shell.modal_focus.clone())
                        .w(shell.z(440.0))
                        .p(shell.z(20.0))
                        .flex()
                        .flex_col()
                        .gap(shell.z(16.0))
                        .bg(p.or("notifications.background", "sideBar.background"))
                        .text_color(p.or("notifications.foreground", "sideBar.foreground"))
                        .border_1()
                        .border_color(p.or("notifications.border", "panel.border"))
                        .rounded(shell.z(6.0))
                        .shadow_lg()
                        .child(div().font_weight(gpui::FontWeight::BOLD).child(label(
                            host,
                            "feedback.dialogTitle",
                            &[],
                        )))
                        .child(text(host, &modal.message))
                        .child(buttons),
                ),
        )
        .with_priority(3)
        .into_any_element(),
    )
}
