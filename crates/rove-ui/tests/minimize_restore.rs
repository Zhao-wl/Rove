#![cfg(target_os = "windows")]

use std::{cell::Cell, rc::Rc};

use gpui::{div, AppContext, Application, Context, Render, Window, WindowOptions};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        PostMessageW, PostQuitMessage, SIZE_MINIMIZED, SIZE_RESTORED, WM_PAINT, WM_SIZE,
    },
};

struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
    }
}

#[test]
fn duplicate_minimize_notifications_preserve_the_next_frame_after_restore() {
    let frame_requested = Rc::new(Cell::new(false));
    let next_frame = frame_requested.clone();

    Application::new().run(move |cx| {
        let window = cx
            .open_window(
                WindowOptions {
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                |_, cx| cx.new(|_| EmptyView),
            )
            .unwrap();
        let hwnd = window
            .update(cx, |_, window, _| {
                let RawWindowHandle::Win32(handle) =
                    HasWindowHandle::window_handle(window).unwrap().as_raw()
                else {
                    panic!("expected a native Windows window");
                };
                window.on_next_frame(move |_, _| next_frame.set(true));
                HWND(handle.hwnd.get() as _)
            })
            .unwrap();

        // Drive the real window procedure in queue order, without moving or activating
        // any window. The frame callback must survive both minimize notifications.
        unsafe {
            for size in [SIZE_MINIMIZED, SIZE_MINIMIZED, SIZE_RESTORED] {
                PostMessageW(
                    Some(hwnd),
                    WM_SIZE,
                    WPARAM(size as usize),
                    LPARAM(if size == SIZE_RESTORED {
                        (240 << 16) | 320
                    } else {
                        0
                    }),
                )
                .unwrap();
            }
            PostMessageW(Some(hwnd), WM_PAINT, WPARAM(0), LPARAM(0)).unwrap();
            PostQuitMessage(0);
        }
    });

    assert!(
        frame_requested.get(),
        "restored window never requested a frame"
    );
}
