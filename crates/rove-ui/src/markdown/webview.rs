//! WebView2 后端（#29），按 #6 报告（`docs/M0_WEBVIEW_SPIKE_REPORT.md`）推荐的方案实现：
//! GPUI 拥有顶层 HWND，每个窗口一个 controller（Hwnd 模式）。Rove 自有的子窗口按浮层区域裁切。
//!
//! 相对原型补上了报告 §5.1 列出的缺口：
//! - 运行时联网：环境参数关闭后台网络、组件更新、SmartScreen 等；设置里关闭信誉检查、自动填充；
//!   页面的所有资源请求经 `WebResourceRequested`：打包资源从内存返回，本地文件走盘符映射，
//!   远程请求返回 403；顶层/子框架导航、新窗口、下载、权限请求、外部协议一律拒绝；
//! - 回收由宿主的定时任务驱动（不依赖渲染帧），`Close()` 不在输入回调里调用；
//! - 隐藏时 `TrySuspend()`，重新显示时 `Resume()`；
//! - COM 回调里不持有 `RefCell` 借用去调 COM（先复制出接口再调用），避免重入 panic；
//! - 显式 `CoInitializeEx`（STA）并检查套间；
//! - 显式设置 `RasterizationScale`（窗口缩放系数）并关闭自动检测，物理像素 bounds 由宿主给出；
//! - 创建完成时按当前可见状态设置初值，不会在弹出层打开时闪一下。
use super::{
    backend::{Backend, BackendEvent, EventSender, Lifecycle},
    page,
};
use gpui::{Bounds, Keystroke, Modifiers, Pixels};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::RefCell, collections::BTreeSet, path::PathBuf, rc::Rc, time::Instant};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{Interface, HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::{HWND, RECT, RPC_E_CHANGED_MODE},
        Graphics::Gdi::{
            CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ, HRGN, RGN_DIFF,
            RGN_ERROR,
        },
        Storage::FileSystem::GetDriveTypeW,
        System::{
            Com::{CoInitializeEx, COINIT_APARTMENTTHREADED},
            WindowsProgramming::{DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOVABLE},
        },
        UI::{
            Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_MENU, VK_SHIFT},
            Shell::SHCreateMemStream,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, SWP_NOACTIVATE,
                SWP_NOZORDER, SW_HIDE, SW_SHOWNA, WINDOW_EX_STYLE, WS_CHILD, WS_CLIPCHILDREN,
                WS_CLIPSIBLINGS,
            },
        },
    },
};

/// 关闭 WebView2（Chromium）自带的后台联网：组件/实验配置更新、崩溃上报、域名可靠性、同步、
/// 安全浏览与 SmartScreen、预取、翻译等。页面资源另由 `WebResourceRequested` 拦截。
const BROWSER_ARGUMENTS: &str = "--disable-background-networking --disable-component-update \
--disable-domain-reliability --disable-client-side-phishing-detection --disable-sync \
--disable-default-apps --disable-breakpad --no-pings --no-first-run --metrics-recording-only \
--disable-features=msSmartScreenProtection,msEdgeEnhancedSecurityMode,Translate,OptimizationHints,\
MediaRouter,NetworkTimeServiceQuerying,AutofillServerCommunication,InterestFeedContentSuggestions,\
CalculateNativeWinOcclusion,msEdgeTranslate,msAutofillEdgeServiceCommunication";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[derive(Default)]
struct Inner {
    environment: Option<ICoreWebView2Environment>,
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    creating: bool,
    /// 每次创建 / 回收递增；迟到的回调发现代次不符就丢弃结果。
    generation: u64,
    bounds: Option<RECT>,
    visible: bool,
    suspended: bool,
    scale: f64,
    zoom: f64,
    focused: bool,
    /// 已建立映射的盘符（资源请求按它放行）与需要映射的盘符（实例创建前也会记下）。
    drives: BTreeSet<char>,
    wanted: BTreeSet<char>,
    background: Option<COREWEBVIEW2_COLOR>,
    nonce: String,
}

/// Rove 自己持有 controller 的父窗口。区域裁切不改变 WebView2 视口尺寸或鼠标坐标。
struct PreviewWindow {
    hwnd: HWND,
    bounds: RECT,
    holes: Vec<RECT>,
    clip_size: (i32, i32),
}

impl PreviewWindow {
    fn new(parent: HWND) -> windows::core::Result<Self> {
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("Static"),
                PCWSTR::null(),
                WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(parent),
                None,
                None,
                None,
            )?
        };
        Ok(Self {
            hwnd,
            bounds: RECT::default(),
            holes: Vec::new(),
            clip_size: (0, 0),
        })
    }

    fn layout(&mut self, bounds: RECT, occlusions: &[Bounds<Pixels>], scale: f32) -> Option<RECT> {
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        if width <= 0 || height <= 0 {
            return None;
        }
        let holes: Vec<_> = occlusions
            .iter()
            .filter_map(|overlay| {
                let left = (f32::from(overlay.left()) * scale).floor() as i32 - bounds.left;
                let top = (f32::from(overlay.top()) * scale).floor() as i32 - bounds.top;
                let right = (f32::from(overlay.right()) * scale).ceil() as i32 - bounds.left;
                let bottom = (f32::from(overlay.bottom()) * scale).ceil() as i32 - bounds.top;
                let rect = RECT {
                    left: left.max(0),
                    top: top.max(0),
                    right: right.min(width),
                    bottom: bottom.min(height),
                };
                (rect.left < rect.right && rect.top < rect.bottom).then_some(rect)
            })
            .collect();
        if self.holes != holes || (!holes.is_empty() && self.clip_size != (width, height)) {
            if !self.apply_region(width, height, &holes) {
                // 当前帧隐藏窗口以免挡住浮层，下次绘制重试。
                return None;
            }
            self.holes = holes;
            self.clip_size = (width, height);
        }
        if self.bounds != bounds {
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    None,
                    bounds.left,
                    bounds.top,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )
                .ok()?;
            }
            self.bounds = bounds;
        }
        Some(RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        })
    }

    fn apply_region(&self, width: i32, height: i32, holes: &[RECT]) -> bool {
        if holes.is_empty() {
            return unsafe { SetWindowRgn(self.hwnd, None, true) } != 0;
        }
        let Some(region) = Region::new(0, 0, width, height) else {
            return false;
        };
        for hole in holes {
            let Some(cut) = Region::new(hole.left, hole.top, hole.right, hole.bottom) else {
                return false;
            };
            let result =
                unsafe { CombineRgn(Some(region.0), Some(region.0), Some(cut.0), RGN_DIFF) };
            if result == RGN_ERROR {
                return false;
            }
        }
        if unsafe { SetWindowRgn(self.hwnd, Some(region.0), true) } == 0 {
            return false;
        }
        // 成功后区域句柄由系统接管。失败时由 Region 释放。
        std::mem::forget(region);
        true
    }
}

impl Drop for PreviewWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

struct Region(HRGN);

impl Region {
    fn new(left: i32, top: i32, right: i32, bottom: i32) -> Option<Self> {
        let handle = unsafe { CreateRectRgn(left, top, right, bottom) };
        if handle.is_invalid() {
            None
        } else {
            Some(Self(handle))
        }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ((self.0).0));
        }
    }
}

/// 窗口拥有的 WebView2 实例。
pub struct WebView2Backend {
    inner: Rc<RefCell<Inner>>,
    events: EventSender,
    hwnd: Option<HWND>,
    preview_window: Option<PreviewWindow>,
}

impl WebView2Backend {
    pub fn new(events: EventSender) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner {
                scale: 1.0,
                zoom: 1.0,
                ..Default::default()
            })),
            events,
            hwnd: None,
            preview_window: None,
        }
    }

    /// 本机是否有可用的 WebView2 运行时（读注册表与安装目录，不创建实例）。
    /// `ROVE_WEBVIEW_BROWSER_FOLDER` 可指向一个固定版本运行时目录；指向不存在的目录可模拟“没有运行时”。
    pub fn runtime_version() -> std::result::Result<String, String> {
        let folder = browser_folder();
        let folder = folder.as_ref().map(|f| HSTRING::from(f.as_os_str()));
        let mut version = PWSTR::null();
        let result = unsafe {
            GetAvailableCoreWebView2BrowserVersionString(
                folder
                    .as_ref()
                    .map_or(PCWSTR::null(), |f| PCWSTR(f.as_ptr())),
                &mut version,
            )
        };
        let version = CoTaskMemPWSTR::from(version).to_string();
        match result {
            Ok(()) if !version.is_empty() => Ok(version),
            Ok(()) => Err("WebView2 runtime not found".into()),
            Err(e) => Err(e.to_string()),
        }
    }
}

fn browser_folder() -> Option<PathBuf> {
    std::env::var_os("ROVE_WEBVIEW_BROWSER_FOLDER").map(PathBuf::from)
}

fn user_data_folder() -> PathBuf {
    std::env::var_os("ROVE_WEBVIEW_USER_DATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Rove")))
        .unwrap_or_else(std::env::temp_dir)
        .join("WebView2")
}

fn send(events: &EventSender, event: BackendEvent) {
    let _ = events.try_send(event);
}

fn hwnd_of(window: &gpui::Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(raw) => Some(HWND(raw.hwnd.get() as *mut _)),
        _ => None,
    }
}

impl Backend for WebView2Backend {
    fn lifecycle(&self) -> Lifecycle {
        let inner = self.inner.borrow();
        if inner.controller.is_some() {
            Lifecycle::Alive
        } else if inner.creating {
            Lifecycle::Creating
        } else {
            Lifecycle::Absent
        }
    }

    fn create(&mut self, window: &mut gpui::Window) {
        if self.lifecycle() != Lifecycle::Absent {
            return;
        }
        let Some(hwnd) = hwnd_of(window) else {
            send(
                &self.events,
                BackendEvent::Failed("no window handle".into()),
            );
            return;
        };
        self.hwnd = Some(hwnd);
        // 报告 §5.1：不依赖“GPUI 的窗口线程恰好是 STA”这一未写明的前提。
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if com == RPC_E_CHANGED_MODE {
            send(
                &self.events,
                BackendEvent::Failed("UI thread is not a single-threaded apartment".into()),
            );
            return;
        }
        if let Err(error) = Self::runtime_version() {
            send(&self.events, BackendEvent::RuntimeMissing(error));
            return;
        }
        if self.preview_window.is_none() {
            match PreviewWindow::new(hwnd) {
                Ok(preview) => self.preview_window = Some(preview),
                Err(error) => {
                    send(&self.events, BackendEvent::Failed(error.to_string()));
                    return;
                }
            }
        }
        let hwnd = self.preview_window.as_ref().unwrap().hwnd;
        let generation = {
            let mut inner = self.inner.borrow_mut();
            inner.creating = true;
            inner.generation += 1;
            inner.nonce = page::nonce();
            inner.generation
        };
        let start = Instant::now();
        let environment = self.inner.borrow().environment.clone();
        if let Some(environment) = environment {
            create_controller(
                &self.inner,
                &self.events,
                &environment,
                hwnd,
                generation,
                start,
            );
            return;
        }
        let weak = Rc::downgrade(&self.inner);
        let events = self.events.clone();
        let folder = browser_folder().map(|f| HSTRING::from(f.as_os_str()));
        let data = HSTRING::from(user_data_folder().as_os_str());
        let options = CoreWebView2EnvironmentOptions::default();
        unsafe {
            options.set_additional_browser_arguments(BROWSER_ARGUMENTS.into());
            options.set_language("zh-CN".into());
        }
        let options: ICoreWebView2EnvironmentOptions = options.into();
        let callback_events = events.clone();
        let handler =
            CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |result, env| {
                let Some(inner) = weak.upgrade() else {
                    return Ok(());
                };
                if inner.borrow().generation != generation {
                    return Ok(());
                }
                let env = match (result, env) {
                    (Ok(()), Some(env)) => env,
                    (Err(e), _) => {
                        inner.borrow_mut().creating = false;
                        send(&callback_events, BackendEvent::Failed(e.to_string()));
                        return Ok(());
                    }
                    (Ok(()), None) => {
                        inner.borrow_mut().creating = false;
                        send(
                            &callback_events,
                            BackendEvent::Failed("no environment".into()),
                        );
                        return Ok(());
                    }
                };
                inner.borrow_mut().environment = Some(env.clone());
                create_controller(&inner, &callback_events, &env, hwnd, generation, start);
                Ok(())
            }));
        let result = unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                folder
                    .as_ref()
                    .map_or(PCWSTR::null(), |f| PCWSTR(f.as_ptr())),
                PCWSTR(data.as_ptr()),
                &options,
                &handler,
            )
        };
        if let Err(error) = result {
            self.inner.borrow_mut().creating = false;
            send(&events, BackendEvent::Failed(error.to_string()));
        }
    }

    fn post(&mut self, message: &serde_json::Value) {
        let webview = self.inner.borrow().webview.clone();
        if let Some(webview) = webview {
            let text = wide(&message.to_string());
            unsafe {
                let _ = webview.PostWebMessageAsJson(PCWSTR(text.as_ptr()));
            }
        }
    }

    fn layout(
        &mut self,
        bounds: Option<Bounds<Pixels>>,
        scale: f32,
        occlusions: &[Bounds<Pixels>],
    ) {
        let Some(controller) = self.inner.borrow().controller.clone() else {
            return;
        };
        let Some(preview) = self.preview_window.as_mut() else {
            return;
        };
        let rect = bounds.and_then(|bounds| {
            let rect = RECT {
                left: (f32::from(bounds.left()) * scale).round() as i32,
                top: (f32::from(bounds.top()) * scale).round() as i32,
                right: (f32::from(bounds.right()) * scale).round() as i32,
                bottom: (f32::from(bounds.bottom()) * scale).round() as i32,
            };
            preview.layout(rect, occlusions, scale)
        });
        let visible = rect.is_some();
        let (webview, rect, visible_changed, scale_changed, resume) = {
            let mut inner = self.inner.borrow_mut();
            let visible_changed = inner.visible != visible;
            inner.visible = visible;
            let scale_changed = (inner.scale - scale as f64).abs() > f64::EPSILON;
            inner.scale = scale as f64;
            let rect = rect.filter(|r| inner.bounds != Some(*r));
            if let Some(r) = rect {
                inner.bounds = Some(r);
            }
            let resume = visible && inner.suspended;
            if resume {
                inner.suspended = false;
            }
            (
                inner.webview.clone(),
                rect,
                visible_changed,
                scale_changed,
                resume,
            )
        };
        unsafe {
            if scale_changed {
                if let Ok(c3) = controller.cast::<ICoreWebView2Controller3>() {
                    let _ = c3.SetRasterizationScale(scale as f64);
                }
            }
            if let Some(rect) = rect {
                let _ = controller.SetBounds(rect);
            }
            if visible_changed {
                if resume {
                    if let Some(w3) = webview.and_then(|w| w.cast::<ICoreWebView2_3>().ok()) {
                        let _ = w3.Resume();
                    }
                }
                let _ = controller.SetIsVisible(visible);
                let _ = ShowWindow(preview.hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
            }
            if visible {
                // 父窗口移动不一定改变客户区尺寸（报告 §9）。
                let _ = controller.NotifyParentWindowPositionChanged();
            }
        }
    }

    fn set_zoom(&mut self, factor: f64) {
        let controller = {
            let mut inner = self.inner.borrow_mut();
            if (inner.zoom - factor).abs() < f64::EPSILON {
                return;
            }
            inner.zoom = factor;
            inner.controller.clone()
        };
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.SetZoomFactor(factor);
            }
        }
    }

    fn focus(&mut self, reverse: bool) {
        let controller = self.inner.borrow().controller.clone();
        if let Some(controller) = controller {
            let reason = if reverse {
                COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS
            } else {
                COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC
            };
            unsafe {
                let _ = controller.MoveFocus(reason);
            }
        }
    }

    fn has_focus(&self) -> bool {
        self.inner.borrow().focused
    }

    fn release_focus(&mut self) {
        // 报告 §14：关闭或离开预览后显式把 Win32 焦点还给宿主，否则按键仍会进网页。
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
        }
        self.inner.borrow_mut().focused = false;
    }

    fn suspend(&mut self) {
        let webview = {
            let mut inner = self.inner.borrow_mut();
            if inner.visible || inner.suspended || inner.controller.is_none() {
                return;
            }
            inner.suspended = true;
            inner.webview.clone()
        };
        if let Some(w3) = webview.and_then(|w| w.cast::<ICoreWebView2_3>().ok()) {
            unsafe {
                let _ = w3.TrySuspend(None::<&ICoreWebView2TrySuspendCompletedHandler>);
            }
        }
    }

    fn dispose(&mut self) {
        let (controller, environment) = {
            let mut inner = self.inner.borrow_mut();
            inner.generation += 1;
            inner.creating = false;
            inner.webview = None;
            inner.bounds = None;
            inner.visible = false;
            inner.suspended = false;
            inner.focused = false;
            inner.drives.clear();
            inner.wanted.clear();
            (inner.controller.take(), inner.environment.take())
        };
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.Close();
            }
        }
        // 先关闭 controller，再销毁父窗口。迟到的回调由 generation 拒绝。
        self.preview_window = None;
        // 释放 environment：最后一个引用释放后浏览器进程退出，内存回落到基础阅读器的量级。
        drop(environment);
    }

    fn map_drives(&mut self, drives: &BTreeSet<char>) {
        self.inner
            .borrow_mut()
            .wanted
            .extend(drives.iter().copied());
        apply_drives(&self.inner);
    }

    fn set_background(&mut self, rgba: (u8, u8, u8, u8)) {
        let color = COREWEBVIEW2_COLOR {
            R: rgba.0,
            G: rgba.1,
            B: rgba.2,
            A: 255,
        };
        let controller = {
            let mut inner = self.inner.borrow_mut();
            inner.background = Some(color);
            inner.controller.clone()
        };
        if let Some(c2) = controller.and_then(|c| c.cast::<ICoreWebView2Controller2>().ok()) {
            unsafe {
                let _ = c2.SetDefaultBackgroundColor(color);
            }
        }
    }

    fn uses_native_child_window(&self) -> bool {
        self.preview_window.is_some() && self.inner.borrow().controller.is_some()
    }

    fn occlusion_count(&self) -> usize {
        if !self.inner.borrow().visible {
            return 0;
        }
        self.preview_window
            .as_ref()
            .map_or(0, |preview| preview.holes.len())
    }

    fn process_ids(&self) -> Vec<u32> {
        let webview = self.inner.borrow().webview.clone();
        let mut ids = Vec::new();
        if let Some(webview) = webview {
            let mut pid = 0u32;
            if unsafe { webview.BrowserProcessId(&mut pid) }.is_ok() && pid != 0 {
                ids.push(pid);
            }
        }
        ids
    }
}

impl Drop for WebView2Backend {
    fn drop(&mut self) {
        self.dispose();
    }
}

fn create_controller(
    inner: &Rc<RefCell<Inner>>,
    events: &EventSender,
    environment: &ICoreWebView2Environment,
    hwnd: HWND,
    generation: u64,
    start: Instant,
) {
    let weak = Rc::downgrade(inner);
    let callback_events = events.clone();
    let env = environment.clone();
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result, controller| {
            let Some(inner) = weak.upgrade() else {
                if let Some(c) = controller {
                    unsafe {
                        let _ = c.Close();
                    }
                }
                return Ok(());
            };
            // 创建期间实例已被回收（或窗口要求重建）：迟到的 controller 直接关闭，不挂到窗口上。
            if inner.borrow().generation != generation {
                if let Some(c) = controller {
                    unsafe {
                        let _ = c.Close();
                    }
                }
                return Ok(());
            }
            let controller = match (result, controller) {
                (Ok(()), Some(c)) => c,
                (Err(e), _) => {
                    inner.borrow_mut().creating = false;
                    send(&callback_events, BackendEvent::Failed(e.to_string()));
                    return Ok(());
                }
                (Ok(()), None) => {
                    inner.borrow_mut().creating = false;
                    send(
                        &callback_events,
                        BackendEvent::Failed("no controller".into()),
                    );
                    return Ok(());
                }
            };
            match unsafe { configure(&inner, &callback_events, &env, &controller) } {
                Ok(()) => send(
                    &callback_events,
                    BackendEvent::Created {
                        ms: start.elapsed().as_secs_f64() * 1000.0,
                    },
                ),
                Err(e) => {
                    unsafe {
                        let _ = controller.Close();
                    }
                    let mut state = inner.borrow_mut();
                    state.creating = false;
                    state.controller = None;
                    state.webview = None;
                    drop(state);
                    send(&callback_events, BackendEvent::Failed(e.to_string()));
                }
            }
            Ok(())
        },
    ));
    if let Err(error) = unsafe { environment.CreateCoreWebView2Controller(hwnd, &handler) } {
        inner.borrow_mut().creating = false;
        send(events, BackendEvent::Failed(error.to_string()));
    }
}

/// A mapped network drive also has a drive letter. Check before passing a path to
/// a subsystem that could access the filesystem.
pub(crate) fn is_local_drive(drive: u8) -> bool {
    let folder = HSTRING::from(format!("{}:\\", (drive as char).to_ascii_uppercase()));
    let kind = unsafe { GetDriveTypeW(PCWSTR(folder.as_ptr())) };
    matches!(
        kind,
        DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_CDROM | DRIVE_RAMDISK
    )
}

/// 为需要的盘符建立 `https://<盘符>.rove-file.local/` → `<盘符>:\` 映射（DENY_CORS：页面脚本不能读取）。
fn apply_drives(inner: &Rc<RefCell<Inner>>) {
    let (webview, missing) = {
        let state = inner.borrow();
        let Some(webview) = state.webview.clone() else {
            return;
        };
        let missing: Vec<char> = state.wanted.difference(&state.drives).copied().collect();
        (webview, missing)
    };
    let Ok(webview) = webview.cast::<ICoreWebView2_3>() else {
        return;
    };
    for drive in missing {
        let host = HSTRING::from(format!("{drive}{}", super::render::FILE_HOST_SUFFIX));
        let folder = HSTRING::from(format!("{}:\\", drive.to_ascii_uppercase()));
        if !is_local_drive(drive as u8) {
            continue;
        }
        let mapped = unsafe {
            webview.SetVirtualHostNameToFolderMapping(
                &host,
                &folder,
                COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS,
            )
        };
        if mapped.is_ok() {
            inner.borrow_mut().drives.insert(drive);
        }
    }
}

fn uri_of(get: impl FnOnce(&mut PWSTR) -> windows::core::Result<()>) -> String {
    let mut raw = PWSTR::null();
    if get(&mut raw).is_err() {
        return String::new();
    }
    CoTaskMemPWSTR::from(raw).to_string()
}

fn is_file_host(uri: &str) -> Option<char> {
    let rest = uri.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    let drive = host.strip_suffix(super::render::FILE_HOST_SUFFIX)?;
    let mut chars = drive.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

unsafe fn respond(
    env: &ICoreWebView2Environment,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
    status: i32,
    mime: &str,
    body: &[u8],
) -> windows::core::Result<()> {
    let stream = SHCreateMemStream(Some(body));
    let headers = HSTRING::from(format!(
        "Content-Type: {mime}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff"
    ));
    let reason = HSTRING::from(if status == 200 {
        "OK"
    } else {
        "Blocked by Rove"
    });
    let response = env.CreateWebResourceResponse(stream.as_ref(), status, &reason, &headers)?;
    args.SetResponse(&response)
}

unsafe fn configure(
    inner: &Rc<RefCell<Inner>>,
    events: &EventSender,
    env: &ICoreWebView2Environment,
    controller: &ICoreWebView2Controller,
) -> windows::core::Result<()> {
    let web = controller.CoreWebView2()?;
    let settings = web.Settings()?;
    // 只执行 Rove 打包的脚本：文档脚本由 CSP（nonce）与 HTML 净化共同阻止。
    settings.SetIsScriptEnabled(true)?;
    settings.SetIsWebMessageEnabled(true)?;
    settings.SetAreDefaultScriptDialogsEnabled(false)?;
    settings.SetAreDevToolsEnabled(std::env::var_os("ROVE_WEBVIEW_DEVTOOLS").is_some())?;
    settings.SetAreDefaultContextMenusEnabled(false)?;
    settings.SetIsStatusBarEnabled(false)?;
    settings.SetIsZoomControlEnabled(false)?;
    settings.SetIsBuiltInErrorPageEnabled(false)?;
    if let Ok(s3) = settings.cast::<ICoreWebView2Settings3>() {
        s3.SetAreBrowserAcceleratorKeysEnabled(false)?;
    }
    if let Ok(s4) = settings.cast::<ICoreWebView2Settings4>() {
        let _ = s4.SetIsPasswordAutosaveEnabled(false);
        let _ = s4.SetIsGeneralAutofillEnabled(false);
    }
    if let Ok(s5) = settings.cast::<ICoreWebView2Settings5>() {
        let _ = s5.SetIsPinchZoomEnabled(false);
    }
    if let Ok(s6) = settings.cast::<ICoreWebView2Settings6>() {
        let _ = s6.SetIsSwipeNavigationEnabled(false);
    }
    // SmartScreen 信誉检查会联网（报告 §5.1（1））。
    if let Ok(s8) = settings.cast::<ICoreWebView2Settings8>() {
        let _ = s8.SetIsReputationCheckingRequired(false);
    }
    let (visible, bounds, scale, zoom, background) = {
        let state = inner.borrow();
        (
            state.visible,
            state.bounds,
            state.scale,
            state.zoom,
            state.background,
        )
    };
    if let Ok(c3) = controller.cast::<ICoreWebView2Controller3>() {
        let _ = c3.SetShouldDetectMonitorScaleChanges(false);
        let _ = c3.SetRasterizationScale(scale);
    }
    if let Some(color) = background {
        if let Ok(c2) = controller.cast::<ICoreWebView2Controller2>() {
            let _ = c2.SetDefaultBackgroundColor(color);
        }
    }
    let _ = controller.SetZoomFactor(zoom);
    let mut token = 0i64;

    // 顶层导航：只允许预览页本身；其余（例如漏过脚本拦截的链接）取消并交给宿主按链接处理。
    let navigation_events = events.clone();
    web.add_NavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let uri = uri_of(|p| args.Uri(p));
                if uri != page::INDEX_URL {
                    args.SetCancel(true)?;
                    send(&navigation_events, BackendEvent::Navigate(uri));
                }
            }
            Ok(())
        })),
        &mut token,
    )?;
    web.add_FrameNavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(|_, args| {
            if let Some(args) = args {
                args.SetCancel(true)?;
            }
            Ok(())
        })),
        &mut token,
    )?;
    let window_events = events.clone();
    web.add_NewWindowRequested(
        &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                args.SetHandled(true)?;
                let uri = uri_of(|p| args.Uri(p));
                send(&window_events, BackendEvent::Navigate(uri));
            }
            Ok(())
        })),
        &mut token,
    )?;
    web.add_PermissionRequested(
        &PermissionRequestedEventHandler::create(Box::new(|_, args| {
            if let Some(args) = args {
                args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
            }
            Ok(())
        })),
        &mut token,
    )?;
    if let Ok(w4) = web.cast::<ICoreWebView2_4>() {
        w4.add_DownloadStarting(
            &DownloadStartingEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    if let Ok(w18) = web.cast::<ICoreWebView2_18>() {
        w18.add_LaunchingExternalUriScheme(
            &LaunchingExternalUriSchemeEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    if let Ok(w10) = web.cast::<ICoreWebView2_10>() {
        w10.add_BasicAuthenticationRequested(
            &BasicAuthenticationRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    let failed_events = events.clone();
    web.add_ProcessFailed(
        &ProcessFailedEventHandler::create(Box::new(move |_, args| {
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED;
            if let Some(args) = args {
                args.ProcessFailedKind(&mut kind)?;
            }
            send(
                &failed_events,
                BackendEvent::Failed(format!("process failed ({})", kind.0)),
            );
            Ok(())
        })),
        &mut token,
    )?;

    // 资源：打包资源从内存返回；本地文件主机交给盘符映射；远程请求返回 403。
    web.AddWebResourceRequestedFilter(&HSTRING::from("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
    let weak = Rc::downgrade(inner);
    let resource_env = env.clone();
    let resource_events = events.clone();
    web.add_WebResourceRequested(
        &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let request = args.Request()?;
            let uri = uri_of(|p| request.Uri(p));
            let Some(inner) = weak.upgrade() else {
                return respond(&resource_env, &args, 403, "text/plain", b"blocked");
            };
            if let Some(path) = uri.strip_prefix(super::render::ASSET_ORIGIN) {
                let path = path.split(['?', '#']).next().unwrap_or_default();
                if path == "index.html" {
                    let html = page::index_html(&inner.borrow().nonce);
                    return respond(
                        &resource_env,
                        &args,
                        200,
                        "text/html; charset=utf-8",
                        html.as_bytes(),
                    );
                }
                return match page::asset(path) {
                    Some((mime, bytes)) => respond(&resource_env, &args, 200, mime, &bytes),
                    None => respond(&resource_env, &args, 404, "text/plain", b"not found"),
                };
            }
            if let Some(drive) = is_file_host(&uri) {
                if inner.borrow().drives.contains(&drive) {
                    return Ok(());
                }
                return respond(&resource_env, &args, 403, "text/plain", b"blocked");
            }
            let lower = uri.to_ascii_lowercase();
            if lower.starts_with("data:")
                || lower.starts_with("blob:")
                || lower.starts_with("about:")
            {
                return Ok(());
            }
            send(&resource_events, BackendEvent::RemoteBlocked(uri));
            respond(&resource_env, &args, 403, "text/plain", b"blocked")
        })),
        &mut token,
    )?;

    // 页面消息：只接受预览页来源、≤ 64 KiB 以外的渲染消息不限（正文 HTML 只从宿主发往页面）。
    let message_events = events.clone();
    let message_inner = Rc::downgrade(inner);
    let generation = inner.borrow().generation;
    web.add_WebMessageReceived(
        &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            if message_inner
                .upgrade()
                .is_none_or(|state| state.borrow().generation != generation)
            {
                return Ok(());
            }
            let source = uri_of(|p| args.Source(p));
            if source != page::INDEX_URL {
                return Ok(());
            }
            let text = uri_of(|p| args.WebMessageAsJson(p));
            if text.len() > 64 * 1024 {
                return Ok(());
            }
            if let Ok(message) = serde_json::from_str::<serde_json::Value>(&text) {
                if message["version"] == 1 {
                    send(&message_events, BackendEvent::Message(message));
                }
            }
            Ok(())
        })),
        &mut token,
    )?;

    let key_events = events.clone();
    controller.add_AcceleratorKeyPressed(
        &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
            args.KeyEventKind(&mut kind)?;
            if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
            {
                return Ok(());
            }
            let mut vk = 0u32;
            args.VirtualKey(&mut vk)?;
            let modifiers = Modifiers {
                control: GetKeyState(VK_CONTROL.0 as i32) < 0,
                alt: GetKeyState(VK_MENU.0 as i32) < 0,
                shift: GetKeyState(VK_SHIFT.0 as i32) < 0,
                ..Default::default()
            };
            if let Some(keystroke) = forwarded_key(vk, modifiers) {
                args.SetHandled(true)?;
                send(&key_events, BackendEvent::Key(keystroke));
            }
            Ok(())
        })),
        &mut token,
    )?;
    let weak = Rc::downgrade(inner);
    let focus_events = events.clone();
    controller.add_GotFocus(
        &FocusChangedEventHandler::create(Box::new(move |_, _| {
            if let Some(inner) = weak.upgrade() {
                inner.borrow_mut().focused = true;
            }
            send(&focus_events, BackendEvent::Focus(true));
            Ok(())
        })),
        &mut token,
    )?;
    let weak = Rc::downgrade(inner);
    let blur_events = events.clone();
    controller.add_LostFocus(
        &FocusChangedEventHandler::create(Box::new(move |_, _| {
            if let Some(inner) = weak.upgrade() {
                inner.borrow_mut().focused = false;
            }
            send(&blur_events, BackendEvent::Focus(false));
            Ok(())
        })),
        &mut token,
    )?;

    if let Some(rect) = bounds {
        controller.SetBounds(rect)?;
    }
    // 报告 §5.1：按当前可见状态设置初值（弹出层打开或预览已离开时不闪）。
    controller.SetIsVisible(visible)?;
    {
        let mut state = inner.borrow_mut();
        state.controller = Some(controller.clone());
        state.webview = Some(web.clone());
        state.creating = false;
        state.suspended = false;
    }
    apply_drives(inner);
    web.Navigate(&HSTRING::from(page::INDEX_URL))?;
    Ok(())
}

/// 网页里按下的哪些键交给 Rove 的键位服务：Ctrl/Alt 组合（复制、全选除外，它们由网页原生处理）、
/// F1–F12 与 Esc。方向键、翻页等仍由网页自己滚动。
pub fn forwarded_key(vk: u32, modifiers: Modifiers) -> Option<Keystroke> {
    let key: String = match vk {
        0x41..=0x5A => ((vk as u8 - 0x41 + b'a') as char).to_string(),
        0x30..=0x39 => ((vk as u8 - 0x30 + b'0') as char).to_string(),
        0x70..=0x7B => format!("f{}", vk - 0x6F),
        0x1B => "escape".into(),
        0x0D => "enter".into(),
        0x20 => "space".into(),
        0x09 => "tab".into(),
        0x08 => "backspace".into(),
        0x2E => "delete".into(),
        0x2D => "insert".into(),
        0x24 => "home".into(),
        0x23 => "end".into(),
        0x21 => "pageup".into(),
        0x22 => "pagedown".into(),
        0x25 => "left".into(),
        0x26 => "up".into(),
        0x27 => "right".into(),
        0x28 => "down".into(),
        0xBA => ";".into(),
        0xBB => "=".into(),
        0xBC => ",".into(),
        0xBD => "-".into(),
        0xBE => ".".into(),
        0xBF => "/".into(),
        0xC0 => "`".into(),
        0xDB => "[".into(),
        0xDC => "\\".into(),
        0xDD => "]".into(),
        0xDE => "'".into(),
        _ => return None,
    };
    let function = key.starts_with('f') && key.len() > 1 && key[1..].parse::<u32>().is_ok();
    let chord = modifiers.control || modifiers.alt;
    let forward = function || key == "escape" || chord;
    if !forward {
        return None;
    }
    // 网页原生的复制与全选。
    if modifiers.control
        && !modifiers.alt
        && !modifiers.shift
        && matches!(key.as_str(), "c" | "a" | "insert")
    {
        return None;
    }
    // 只按着 Ctrl/Alt 时的方向键、翻页留给网页滚动。
    if !function
        && matches!(
            key.as_str(),
            "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown"
        )
        && !modifiers.alt
    {
        return None;
    }
    Some(Keystroke {
        modifiers,
        key,
        key_char: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Default::default()
        }
    }

    #[test]
    fn rove_keys_are_forwarded_and_native_keys_stay_in_the_page() {
        let toggle = forwarded_key(
            0x56,
            Modifiers {
                control: true,
                shift: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(toggle.key, "v");
        assert!(toggle.modifiers.control && toggle.modifiers.shift);
        assert_eq!(forwarded_key(0x50, ctrl()).unwrap().key, "p");
        assert_eq!(forwarded_key(0x70, Modifiers::default()).unwrap().key, "f1");
        assert_eq!(
            forwarded_key(0x1B, Modifiers::default()).unwrap().key,
            "escape"
        );
        // 复制、全选、普通字符、方向键留给网页。
        assert!(forwarded_key(0x43, ctrl()).is_none());
        assert!(forwarded_key(0x41, ctrl()).is_none());
        assert!(forwarded_key(0x41, Modifiers::default()).is_none());
        assert!(forwarded_key(0x28, Modifiers::default()).is_none());
        assert!(forwarded_key(0x22, ctrl()).is_none());
        assert!(forwarded_key(0x10, ctrl()).is_none());
    }

    #[test]
    fn native_preview_region_excludes_only_overlays_and_can_be_reset() {
        use windows::Win32::{
            Graphics::Gdi::{GetWindowRgn, PtInRegion},
            UI::WindowsAndMessaging::WS_POPUP,
        };

        // 标准隐藏窗口，无需 COM 和 WebView2，不抢平台焦点。
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                windows::core::w!("Static"),
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                300,
                300,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let parent = PreviewWindow {
            hwnd,
            bounds: RECT::default(),
            holes: Vec::new(),
            clip_size: (0, 0),
        };
        let mut preview = PreviewWindow::new(parent.hwnd).unwrap();
        let bounds = RECT {
            left: 100,
            top: 50,
            right: 300,
            bottom: 250,
        };
        let overlay = Bounds::new(
            gpui::point(gpui::px(100.0), gpui::px(50.0)),
            gpui::size(gpui::px(40.0), gpui::px(24.0)),
        );
        let viewport = preview.layout(bounds, &[overlay], 1.25).unwrap();
        assert_eq!((viewport.right, viewport.bottom), (200, 200));
        let region = Region::new(0, 0, 0, 0).unwrap();
        unsafe {
            assert_ne!(GetWindowRgn(preview.hwnd, region.0), RGN_ERROR);
            assert!(!PtInRegion(region.0, 50, 20).as_bool());
            assert!(PtInRegion(region.0, 150, 20).as_bool());
        }
        assert_eq!(preview.layout(bounds, &[], 1.25), Some(viewport));
        unsafe {
            assert_eq!(GetWindowRgn(preview.hwnd, region.0), RGN_ERROR);
        }
    }

    #[test]
    fn file_hosts_are_recognised() {
        assert_eq!(is_file_host("https://c.rove-file.local/a.png"), Some('c'));
        assert_eq!(is_file_host("https://evil.com/c.rove-file.local"), None);
        assert_eq!(is_file_host("https://cc.rove-file.local/a"), None);
    }
}
