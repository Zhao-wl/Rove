use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, FocusHandle, Window,
    WindowBounds, WindowOptions,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde_json::{json, Value};
use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Instant};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{w, Interface, PCWSTR, PWSTR},
    Win32::{
        Foundation::{HWND, RECT},
        UI::{
            Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_SHIFT},
            WindowsAndMessaging::GetClientRect,
        },
    },
};

#[derive(Default)]
struct State {
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    pending: bool,
    generation: u64,
    ready: bool,
    error: Option<String>,
    blocked: usize,
    route: Option<String>,
    seq: u64,
    pings: HashMap<u64, Instant>,
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn log(kind: &str, data: Value) {
    println!("{}", json!({"event":kind,"data":data}));
}
fn fail(state: &Rc<RefCell<State>>, error: impl std::fmt::Display) {
    let message = error.to_string();
    log("error", json!(message));
    let mut s = state.borrow_mut();
    s.pending = false;
    s.error = Some(message);
}
fn close(state: &Rc<RefCell<State>>) {
    let controller = {
        let mut s = state.borrow_mut();
        s.generation += 1;
        s.ready = false;
        s.pending = false;
        s.webview = None;
        s.pings.clear();
        s.controller.take()
    };
    if let Some(c) = controller {
        unsafe {
            let _ = c.Close();
        }
    }
    log("closed", json!({}));
}
fn create(state: &Rc<RefCell<State>>, hwnd: HWND) {
    {
        let mut s = state.borrow_mut();
        if s.pending || s.controller.is_some() {
            return;
        }
        s.pending = true;
        s.error = None;
    }
    let generation = state.borrow().generation;
    let start = Instant::now();
    let weak = Rc::downgrade(state);
    let profile = std::env::var("ROVE_WEBVIEW_PROFILE")
        .unwrap_or_else(|_| format!("{}/runs/profile", env!("CARGO_MANIFEST_DIR")));
    let profile = wide(&profile);
    let callback =
        CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |result, env| {
            let Some(state) = weak.upgrade() else {
                return Ok(());
            };
            if state.borrow().generation != generation {
                return Ok(());
            }
            if let Err(e) = result {
                fail(&state, e);
                return Ok(());
            }
            let Some(env) = env else {
                fail(&state, "missing environment");
                return Ok(());
            };
            log(
                "environment",
                json!({"ms":start.elapsed().as_secs_f64()*1000.0}),
            );
            let weak = Rc::downgrade(&state);
            let environment = env.clone();
            let callback = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                move |result, controller| {
                    let Some(state) = weak.upgrade() else {
                        if let Some(c) = controller {
                            unsafe {
                                let _ = c.Close();
                            }
                        }
                        return Ok(());
                    };
                    if state.borrow().generation != generation {
                        if let Some(c) = controller {
                            unsafe {
                                let _ = c.Close();
                            }
                        }
                        return Ok(());
                    }
                    if let Err(e) = result {
                        fail(&state, e);
                        return Ok(());
                    }
                    let Some(controller) = controller else {
                        fail(&state, "missing controller");
                        return Ok(());
                    };
                    unsafe {
                        if let Err(e) = configure(&state, &environment, &controller, hwnd, start) {
                            let _ = controller.Close();
                            fail(&state, e);
                        }
                    }
                    Ok(())
                },
            ));
            if let Err(e) = unsafe { env.CreateCoreWebView2Controller(hwnd, &callback) } {
                fail(&state, e)
            }
            Ok(())
        }));
    let browser = std::env::var("ROVE_WEBVIEW_BROWSER_FOLDER")
        .ok()
        .map(|s| wide(&s));
    let result = unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            browser
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            PCWSTR(profile.as_ptr()),
            None,
            &callback,
        )
    };
    if let Err(e) = result {
        fail(state, e)
    }
}
unsafe fn configure(
    state: &Rc<RefCell<State>>,
    env: &ICoreWebView2Environment,
    controller: &ICoreWebView2Controller,
    hwnd: HWND,
    start: Instant,
) -> windows::core::Result<()> {
    let web = controller.CoreWebView2()?;
    let settings = web.Settings()?;
    settings.SetIsScriptEnabled(false)?;
    settings.SetAreDevToolsEnabled(false)?;
    settings.SetAreDefaultContextMenusEnabled(false)?;
    settings.SetIsStatusBarEnabled(false)?;
    settings
        .cast::<ICoreWebView2Settings3>()?
        .SetAreBrowserAcceleratorKeysEnabled(false)?;
    let assets = wide(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"));
    web.cast::<ICoreWebView2_3>()?
        .SetVirtualHostNameToFolderMapping(
            w!("rove.local"),
            PCWSTR(assets.as_ptr()),
            COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS,
        )?;
    let mut token = 0;
    web.add_NavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
            if let Some(a) = args {
                let mut uri = PWSTR::null();
                a.Uri(&mut uri)?;
                let uri = CoTaskMemPWSTR::from(uri).to_string();
                if !uri.starts_with("https://rove.local/") {
                    a.SetCancel(true)?;
                    log("navigation_blocked", json!(uri));
                }
            }
            Ok(())
        })),
        &mut token,
    )?;
    web.add_NewWindowRequested(
        &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
            if let Some(a) = args {
                a.SetHandled(true)?;
            }
            Ok(())
        })),
        &mut token,
    )?;
    web.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
    let weak = Rc::downgrade(state);
    let env = env.clone();
    web.add_WebResourceRequested(
        &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            if let Some(a) = args {
                let request = a.Request()?;
                let mut uri = PWSTR::null();
                request.Uri(&mut uri)?;
                let uri = CoTaskMemPWSTR::from(uri).to_string();
                if !uri.starts_with("https://rove.local/") {
                    let response = env.CreateWebResourceResponse(
                        None,
                        403,
                        w!("Blocked by Rove"),
                        w!("Content-Type: text/plain"),
                    )?;
                    a.SetResponse(&response)?;
                    if let Some(s) = weak.upgrade() {
                        s.borrow_mut().blocked += 1;
                    }
                    log("resource_blocked", json!(uri));
                } else {
                    log("local_resource", json!(uri));
                }
            }
            Ok(())
        })),
        &mut token,
    )?;
    let weak = Rc::downgrade(state);
    web.add_WebMessageReceived(&WebMessageReceivedEventHandler::create(Box::new(move |_,args| {
        if let (Some(s),Some(a))=(weak.upgrade(),args){let mut raw=PWSTR::null();a.WebMessageAsJson(&mut raw)?;let text=CoTaskMemPWSTR::from(raw).to_string();
            if let Ok(m)=serde_json::from_str::<Value>(&text){
                if m["version"]!=1{return Ok(())}
                if m["type"]=="ready"{s.borrow_mut().ready=true;log("ready",json!({"ms":start.elapsed().as_secs_f64()*1000.0,"documentScriptBlocked":m["documentScriptBlocked"]}));}
                else if m["type"]=="pong"{if let Some(seq)=m["seq"].as_u64(){if let Some(sent)=s.borrow_mut().pings.remove(&seq){log("roundtrip",json!({"seq":seq,"ms":sent.elapsed().as_secs_f64()*1000.0}));}}}
                else {log("message",m);}
            }
        }Ok(())
    })),&mut token)?;
    let weak = Rc::downgrade(state);
    controller.add_AcceleratorKeyPressed(
        &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            if let Some(a) = args {
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
                a.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                    && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                {
                    return Ok(());
                }
                let mut key = 0;
                a.VirtualKey(&mut key)?;
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                let route = match (ctrl, shift, key) {
                    (true, false, 80) => Some("quickOpen"),
                    (true, true, 80) => Some("commandPalette"),
                    (true, true, 86) => Some("togglePreview"),
                    (_, _, 27) => Some("escape"),
                    (_, _, 9) => Some("tab"),
                    (true, _, 82) => Some("blockedRefresh"),
                    (true, _, 70) => Some("findDeferred"),
                    _ => None,
                };
                if let Some(route) = route {
                    a.SetHandled(true)?;
                    if let Some(s) = weak.upgrade() {
                        s.borrow_mut().route = Some(route.into());
                    }
                    log(
                        "key_route",
                        json!({"key":key,"ctrl":ctrl,"shift":shift,"route":route}),
                    );
                }
            }
            Ok(())
        })),
        &mut token,
    )?;
    let script = wide(include_str!("../assets/bridge.js"));
    let navigation = web.clone();
    web.AddScriptToExecuteOnDocumentCreated(
        PCWSTR(script.as_ptr()),
        &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(move |result, _| {
            result?;
            navigation.Navigate(w!("https://rove.local/index.html"))?;
            Ok(())
        })),
    )?;
    controller.SetIsVisible(true)?;
    let mut s = state.borrow_mut();
    s.controller = Some(controller.clone());
    s.webview = Some(web);
    s.pending = false;
    log(
        "controller",
        json!({"ms":start.elapsed().as_secs_f64()*1000.0,"hwnd":hwnd.0 as usize}),
    );
    Ok(())
}
struct Spike {
    state: Rc<RefCell<State>>,
    focus: FocusHandle,
    start: Instant,
    first: bool,
    overlay: bool,
    mitigate: bool,
    stage: u8,
    auto: bool,
    last_ping: Instant,
}
impl Drop for Spike {
    fn drop(&mut self) {
        close(&self.state)
    }
}
impl Render for Spike {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let RawWindowHandle::Win32(raw) = HasWindowHandle::window_handle(window).unwrap().as_raw()
        else {
            panic!("Windows only")
        };
        let hwnd = HWND(raw.hwnd.get() as *mut _);
        if !self.first {
            self.first = true;
            log(
                "first_render",
                json!({"ms":self.start.elapsed().as_secs_f64()*1000.0}),
            );
        }
        let elapsed = self.start.elapsed().as_secs_f64();
        if self.auto {
            let next = if elapsed > 16.0 {
                6
            } else if elapsed > 13.0 {
                5
            } else if elapsed > 11.0 {
                4
            } else if elapsed > 8.0 {
                3
            } else if elapsed > 5.0 {
                2
            } else if elapsed > 1.0 {
                1
            } else {
                0
            };
            if next != self.stage {
                self.stage = next;
                match next {
                    1 => create(&self.state, hwnd),
                    2 => {
                        self.overlay = true;
                        self.mitigate = false;
                        log("overlay_unmitigated", json!({}));
                    }
                    3 => {
                        self.mitigate = true;
                        log("overlay_hidden_webview", json!({}));
                    }
                    4 => {
                        self.overlay = false;
                        close(&self.state);
                    }
                    5 => create(&self.state, hwnd),
                    6 => cx.quit(),
                    _ => {}
                }
            }
        }
        if let Some(route) = self.state.borrow_mut().route.take() {
            match route.as_str() {
                "quickOpen" | "commandPalette" => {
                    self.overlay = true;
                    self.mitigate = true;
                }
                "escape" => self.overlay = false,
                _ => {}
            }
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
        }
        let (controller, web, ready, pending, blocked, error) = {
            let s = self.state.borrow();
            (
                s.controller.clone(),
                s.webview.clone(),
                s.ready,
                s.pending,
                s.blocked,
                s.error.clone(),
            )
        };
        if let Some(c) = controller {
            unsafe {
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                r.left = r.right / 2;
                let _ = c.SetBounds(r);
                let _ = c.NotifyParentWindowPositionChanged();
                let _ = c.SetIsVisible(!(self.overlay && self.mitigate));
            }
        }
        if ready && self.last_ping.elapsed().as_millis() >= 16 {
            self.last_ping = Instant::now();
            if let Some(web) = web {
                let seq = {
                    let mut s = self.state.borrow_mut();
                    s.seq += 1;
                    let seq = s.seq;
                    s.pings.insert(seq, Instant::now());
                    seq
                };
                let message = wide(&json!({"version":1,"type":"ping","seq":seq}).to_string());
                unsafe {
                    let _ = web.PostWebMessageAsJson(PCWSTR(message.as_ptr()));
                }
            }
        }
        let entity = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = entity.update(cx, |_, cx| cx.notify());
        });
        window.request_animation_frame();
        div().size_full().bg(rgb(0x181b20)).text_color(rgb(0xeeeeee)).track_focus(&self.focus)
            .on_key_down(cx.listener(move |this,event: &gpui::KeyDownEvent,window,cx| {match event.keystroke.key.as_str(){"o"=>create(&this.state,hwnd),"h"=>{this.overlay=!this.overlay;this.mitigate=true;},"u"=>{this.overlay=!this.overlay;this.mitigate=false;},"x"=>close(&this.state),"escape"=>this.overlay=false,"t"=>{if let Some(w)=this.state.borrow().webview.as_ref(){let m=wide(&json!({"version":1,"type":"theme","dark":false,"seq":0}).to_string());unsafe{let _=w.PostWebMessageAsJson(PCWSTR(m.as_ptr()));}}},_=>{}}window.focus(&this.focus);cx.notify();}))
            .child(div().p_6().w_1_2().child("Rove GPUI / fake editor").child(div().mt_4().child("O: open | H: mitigated popup | U: raw overlap | X: dispose | T: light theme")).child(div().mt_4().child(format!("Ready={ready}, creating={pending}, blocked={blocked}; {}",error.unwrap_or_default()))))
            .child(div().absolute().left(px(640.)).top(px(100.)).child(if ready{"Preview hidden while popup overlaps"}else{"Loading preview — skeleton"}))
            .when(self.overlay,|d|d.child(div().absolute().left(px(350.)).top(px(120.)).w(px(600.)).h(px(280.)).bg(rgb(0x334c65)).p_6().child("GPUI command palette / menu / hover / notification overlap fixture")))
    }
}
fn main() {
    let start = Instant::now();
    let auto = std::env::args().any(|a| a == "--auto");
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        let handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| Spike {
                        state: Rc::new(RefCell::new(State::default())),
                        focus: cx.focus_handle(),
                        start,
                        first: false,
                        overlay: false,
                        mitigate: false,
                        stage: 0,
                        auto,
                        last_ping: Instant::now(),
                    })
                },
            )
            .unwrap();
        handle
            .update(cx, |view, window, cx| {
                window.focus(&view.focus);
                cx.activate(true);
            })
            .unwrap();
    });
}
