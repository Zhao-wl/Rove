//! 实机测量从打开设置命令到搜索框获得焦点的时间。
use super::{LayoutStore, Workbench};
use crate::{keys::RunCommand, settings_ui::SharedSettings, shell::Shell};
use gpui::{
    px, size, App, AppContext, Application, Bounds, Focusable, WindowBounds, WindowOptions,
};
use rove_base::{Resource, Value};
use rove_config::{SettingsOptions, SettingsService};
use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires a native Windows window"]
fn settings_open_in_a_native_window() {
    let fixture = tempfile::tempdir().expect("settings probe fixture");
    let workspace = fixture.path().join("workspace");
    let user = fixture.path().join("user");
    std::fs::create_dir_all(&workspace).expect("workspace directory");
    std::fs::create_dir_all(&user).expect("user directory");
    std::fs::write(
        user.join("settings.json"),
        r#"{"workbench.settings.editor":"ui","window.autoDetectColorScheme":false}"#,
    )
    .expect("fixture settings");
    let mut options = SettingsOptions::new(&user, fixture.path().join("backup"));
    options.watch = None;
    let service = Arc::new(SettingsService::open(
        options,
        crate::app_settings::registry().unwrap(),
    ));
    let elapsed = Arc::new(Mutex::new(None));
    let measured = elapsed.clone();
    Application::new().run(move |cx: &mut App| {
        cx.set_global(SharedSettings(service.clone()));
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(crate::shell::icons::FONT)])
            .expect("codicon font");
        crate::keys::init(cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1200.0), px(840.0)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |window, cx| {
                    cx.new(|cx| {
                        let mut workbench = Workbench::with_settings(
                            workspace.clone(),
                            Instant::now(),
                            LayoutStore::disabled(),
                            service.clone(),
                            cx,
                        )
                        .expect("settings workbench");
                        Shell::attach_window(&mut workbench, window, cx);
                        window.focus(&workbench.code_view().focus_handle(cx));
                        workbench
                    })
                },
            )
            .expect("settings probe window");
        cx.spawn(async move |cx| {
            let _fixture = fixture;
            let executor = cx.background_executor().clone();
            executor.timer(Duration::from_millis(250)).await;
            let (started, previous_focus) = window
                .update(cx, |_, window, cx| {
                    let previous_focus = window.focused(cx);
                    let started = Instant::now();
                    window.dispatch_action(
                        Box::new(RunCommand {
                            id: crate::welcome::registry::OPEN_SETTINGS.into(),
                            args: Value::Null,
                        }),
                        cx,
                    );
                    (started, previous_focus)
                })
                .expect("dispatch open settings");
            let opened = loop {
                let changed = window
                    .update(cx, |view, window, cx| {
                        let ready = view.settings_view.is_some()
                            && !view.settings_focus_requested
                            && view.editor.active().current().is_some_and(|tab| {
                                matches!(&tab.resource, Resource::Virtual(id)
                                    if id.as_ref() == crate::settings_ui::registry::RESOURCE)
                            });
                        ready
                            && window.focused(cx).is_some_and(|focus| {
                                previous_focus
                                    .as_ref()
                                    .is_none_or(|previous| focus != *previous)
                            })
                    })
                    .expect("read window focus");
                if changed || started.elapsed() >= Duration::from_secs(5) {
                    break changed;
                }
                executor.timer(Duration::from_millis(1)).await;
            };
            *measured.lock().unwrap() = opened.then(|| started.elapsed());
            cx.update(|cx| cx.quit()).expect("quit settings probe");
        })
        .detach();
    });
    let elapsed = elapsed
        .lock()
        .unwrap()
        .expect("settings search did not receive focus");
    println!(
        "SETTINGS_PROBE elapsed_ms={:.3}",
        elapsed.as_secs_f64() * 1_000.0
    );
    assert!(elapsed < Duration::from_millis(100), "{elapsed:?}");
}
