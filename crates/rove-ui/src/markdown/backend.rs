//! 预览实例的后端接口（#29）。窗口只持有一个后端；生产实现是 WebView2（[`super::webview`]），
//! 测试用 [`FakeBackend`]（GPUI 的测试窗口没有真实 HWND）。
//!
//! 后端只在 UI 线程上调用。它的回调（WebView2 的 COM 事件）**不直接改界面**：只把
//! [`BackendEvent`] 送进通道，由窗口的前台任务在正常的 GPUI 上下文里处理（不重入、不阻塞输入回调）。
use gpui::{Bounds, Keystroke, Pixels};
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

/// 后端发给窗口的事件。
#[derive(Clone, Debug)]
pub enum BackendEvent {
    /// 控制器已创建（`ms` 为从请求到创建完成的耗时）。
    Created { ms: f64 },
    /// 运行时不可用（没有安装 WebView2）。
    RuntimeMissing(String),
    /// 创建失败或渲染进程崩溃。
    Failed(String),
    /// 页面脚本发来的消息（已检查来源与大小，`version == 1`）。
    Message(serde_json::Value),
    /// 网页获得 / 失去键盘焦点。
    Focus(bool),
    /// 网页里按下了应由 Rove 处理的键（Ctrl/Alt 组合、F 键、Esc）。
    Key(Keystroke),
    /// 网页试图导航或打开新窗口（已取消），按链接点击处理。
    Navigate(String),
    /// 远程资源请求被拦截。
    RemoteBlocked(String),
}

/// 事件的接收端；后端持有发送端。
pub type EventSender = async_channel::Sender<BackendEvent>;

/// 实例的生命周期状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    /// 没有实例（从未创建，或已回收）。
    Absent,
    /// 正在异步创建。
    Creating,
    /// 控制器已存在。
    Alive,
}

/// 窗口拥有的预览后端。
pub trait Backend {
    fn lifecycle(&self) -> Lifecycle;
    /// 第一次打开预览时调用：异步创建实例，不阻塞 UI。已有实例或正在创建时什么也不做。
    fn create(&mut self, window: &mut gpui::Window);
    /// 投递一条 JSON 消息给页面（`PostWebMessageAsJson`，非阻塞）。
    fn post(&mut self, message: &serde_json::Value);
    /// 显示在窗口客户区的逻辑坐标 `bounds`（乘 `scale` 得到物理像素），`None` 表示隐藏。
    fn layout(&mut self, bounds: Option<Bounds<Pixels>>, scale: f32);
    /// 网页缩放（跟随 `window.zoomLevel`）。
    fn set_zoom(&mut self, factor: f64);
    /// 把键盘焦点交给网页（`MoveFocus`）；`reverse` 为 Shift+Tab 进入。
    fn focus(&mut self, reverse: bool);
    /// 网页当前是否持有 Win32 键盘焦点。
    fn has_focus(&self) -> bool;
    /// 把 Win32 键盘焦点还给宿主窗口。
    fn release_focus(&mut self);
    /// 隐藏期间挂起渲染进程以降低内存（`TrySuspend`）；重新显示时自动恢复。
    fn suspend(&mut self);
    /// 销毁实例并释放浏览器进程。同步调用（WebView2 实测 29–459 ms），只能在定时器任务或窗口关闭时调用，
    /// 不得在输入回调里调用。
    fn dispose(&mut self);
    /// 为这些盘符建立本地文件主机映射（`https://<盘符>.rove-file.local/`）。
    fn map_drives(&mut self, drives: &BTreeSet<char>);
    /// 当前显示的文档是否放行远程资源。
    /// 实例就绪前的底色（跟随编辑器背景，避免白屏闪烁）。
    fn set_background(&mut self, _rgba: (u8, u8, u8, u8)) {}
    /// 属于该实例的进程 ID（测量用；没有实例时为空）。
    fn process_ids(&self) -> Vec<u32> {
        Vec::new()
    }
}

/// [`FakeBackend`] 记下的调用。
#[derive(Clone, Debug, Default)]
pub struct FakeLog {
    pub created: u32,
    pub disposed: u32,
    pub suspended: u32,
    pub posted: Vec<serde_json::Value>,
    pub layout: Option<Option<Bounds<Pixels>>>,
    pub visible_changes: u32,
    pub focus_requests: u32,
    pub released: u32,
    pub drives: BTreeSet<char>,
    pub zoom: f64,
}

/// 测试后端：创建后立即“就绪”，收到 `render` 时回一条 `rendered`。可以设置成“运行时缺失”。
pub struct FakeBackend {
    pub log: Rc<RefCell<FakeLog>>,
    events: EventSender,
    lifecycle: Lifecycle,
    missing: bool,
    focused: bool,
    visible: bool,
}

impl FakeBackend {
    pub fn new(events: EventSender, log: Rc<RefCell<FakeLog>>, missing: bool) -> Self {
        Self {
            log,
            events,
            lifecycle: Lifecycle::Absent,
            missing,
            focused: false,
            visible: false,
        }
    }
}

impl Backend for FakeBackend {
    fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }
    fn create(&mut self, _: &mut gpui::Window) {
        if self.lifecycle != Lifecycle::Absent {
            return;
        }
        if self.missing {
            let _ = self
                .events
                .try_send(BackendEvent::RuntimeMissing("0x80070002".into()));
            return;
        }
        self.log.borrow_mut().created += 1;
        self.lifecycle = Lifecycle::Alive;
        let _ = self.events.try_send(BackendEvent::Created { ms: 0.0 });
        let _ = self.events.try_send(BackendEvent::Message(
            serde_json::json!({"version": 1, "type": "ready"}),
        ));
    }
    fn post(&mut self, message: &serde_json::Value) {
        self.log.borrow_mut().posted.push(message.clone());
        if message["type"] == "render" {
            let html = message["html"].as_str().unwrap_or_default();
            let stats = serde_json::json!({
                "mermaid": html.matches("class=\"code-line mermaid\"").count(),
                "math": html.matches("class=\"rove-math").count(),
                "documentScript": false,
            });
            let _ = self
                .events
                .try_send(BackendEvent::Message(serde_json::json!({
                    "version": 1,
                    "type": "rendered",
                    "documentId": message["documentId"],
                    "seq": message["seq"],
                    "stats": stats,
                })));
        }
    }
    fn layout(&mut self, bounds: Option<Bounds<Pixels>>, _: f32) {
        let mut log = self.log.borrow_mut();
        if self.visible != bounds.is_some() {
            log.visible_changes += 1;
        }
        self.visible = bounds.is_some();
        log.layout = Some(bounds);
    }
    fn set_zoom(&mut self, factor: f64) {
        self.log.borrow_mut().zoom = factor;
    }
    fn focus(&mut self, _: bool) {
        self.focused = true;
        self.log.borrow_mut().focus_requests += 1;
    }
    fn has_focus(&self) -> bool {
        self.focused
    }
    fn release_focus(&mut self) {
        self.focused = false;
        self.log.borrow_mut().released += 1;
    }
    fn suspend(&mut self) {
        self.log.borrow_mut().suspended += 1;
    }
    fn dispose(&mut self) {
        if self.lifecycle == Lifecycle::Absent {
            return;
        }
        self.lifecycle = Lifecycle::Absent;
        self.focused = false;
        self.visible = false;
        self.log.borrow_mut().disposed += 1;
    }
    fn map_drives(&mut self, drives: &BTreeSet<char>) {
        self.log.borrow_mut().drives.extend(drives.iter().copied());
    }
}

/// 不支持 WebView2 的平台：总是“运行时缺失”。
pub struct UnavailableBackend(pub EventSender);

impl Backend for UnavailableBackend {
    fn lifecycle(&self) -> Lifecycle {
        Lifecycle::Absent
    }
    fn create(&mut self, _: &mut gpui::Window) {
        let _ = self
            .0
            .try_send(BackendEvent::RuntimeMissing("unsupported platform".into()));
    }
    fn post(&mut self, _: &serde_json::Value) {}
    fn layout(&mut self, _: Option<Bounds<Pixels>>, _: f32) {}
    fn set_zoom(&mut self, _: f64) {}
    fn focus(&mut self, _: bool) {}
    fn has_focus(&self) -> bool {
        false
    }
    fn release_focus(&mut self) {}
    fn suspend(&mut self) {}
    fn dispose(&mut self) {}
    fn map_drives(&mut self, _: &BTreeSet<char>) {}
}
