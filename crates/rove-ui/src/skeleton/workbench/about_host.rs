//! #76：「关于」对话框的窗口装配。
//!
//! 模型在 `crate::about`（不依赖 GPUI），这里只把它接到窗口上：
//!
//! 1. `workbench.action.showAboutDialog`（命令面板、齿轮菜单的「关于」）打开浮层；
//! 2. 「复制」把 `about::clipboard_text` 写进剪贴板，并在状态栏回执一条（UX_SPEC §6.1 第 1 级）；
//! 3. 「第三方声明」在编辑器里打开随包的 `THIRD_PARTY_NOTICES.md`（与 `rove.exe` 同目录），
//!    找不到时只报一条状态栏消息——**不调用外部程序、不联网**；
//! 4. 「关闭」或点浮层外面关掉。
//!
//! 浮层而不是模态框：UX_SPEC §6.1 把模态框保留给数据丢失与安全确认。

use super::*;
use crate::about::{self, registry as about_registry, AboutInfo};

/// 窗口里的「关于」状态：`None` 表示没打开。
#[derive(Default)]
pub(super) struct About {
    pub(super) info: Option<AboutInfo>,
}

impl Workbench {
    /// 命令分发：目前只有「关于 Rove」。
    pub(super) fn about_command(&mut self, id: &str, cx: &mut Context<Self>) {
        if id == about_registry::SHOW_ABOUT {
            self.about.info = Some(AboutInfo::current());
            cx.notify();
        }
    }

    /// 对话框里的按钮。
    pub(super) fn about_act(&mut self, action: &str, cx: &mut Context<Self>) {
        let Some(info) = self.about.info.clone() else {
            return;
        };
        match action {
            about_registry::ACTION_COPY => {
                let text = about::clipboard_text(&self.strings, &info);
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                let _ = shell::view::report(
                    self,
                    "about.copied",
                    Reason::Receipt,
                    Message::key("about.copied"),
                    Arc::from([]),
                    cx,
                );
            }
            about_registry::ACTION_NOTICES => match info.notices_path() {
                Some(path) => {
                    self.about.info = None;
                    // 用编辑器打开随包的声明文件；不调用外部程序，也不进「最近打开」。
                    if let Err(e) = self.open_documents().open(
                        Resource::File(path),
                        OpenOptions {
                            preview_tab: false,
                            ..Default::default()
                        },
                        Cancellation::default(),
                    ) {
                        eprintln!("about: {e}");
                    }
                }
                None => {
                    let _ = shell::view::report(
                        self,
                        "about.noticesMissing",
                        Reason::Receipt,
                        Message::key("about.noticesMissing"),
                        Arc::from([]),
                        cx,
                    );
                }
            },
            about_registry::ACTION_CLOSE => self.about.info = None,
            _ => {}
        }
        cx.notify();
    }

    /// 浮层。没有打开时返回 `None`（不绘制任何东西）。
    pub(super) fn about_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let info = self.about.info.as_ref()?;
        let lines = about::lines(&self.strings, info);
        let background = self.ui_color("editorWidget.background");
        let foreground = self.ui_color("editorWidget.foreground");
        let border = self.ui_color("editorWidget.border");
        let muted = self.ui_color("descriptionForeground");
        let accent = self.ui_color("button.background");
        let accent_text = self.ui_color("button.foreground");
        // 浮层尺寸跟随窗口缩放（#16 的 zoom），与模态框同一口径。
        let zoom = self.shell.zoom();
        let scale = move |v: f32| gpui::px(v * zoom);

        let mut body = div().flex().flex_col().gap(scale(4.0));
        for line in lines {
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(scale(8.0))
                    .child(
                        div()
                            .w(scale(96.0))
                            .flex_none()
                            .text_color(muted)
                            .child(SharedString::from(line.label)),
                    )
                    .child(div().flex_1().child(SharedString::from(line.value))),
            );
        }

        let button = |id: &'static str, label: String, primary: bool| {
            div()
                .id(SharedString::from(format!("about-{id}")))
                .px(scale(14.0))
                .py(scale(4.0))
                .rounded(scale(2.0))
                .cursor_pointer()
                .when(primary, |d| d.bg(accent).text_color(accent_text))
                .when(!primary, |d| d.border_1().border_color(border))
                .child(SharedString::from(label))
                .on_click(cx.listener(move |view: &mut Self, _, _, cx| view.about_act(id, cx)))
        };

        Some(
            gpui::deferred(
                div()
                    .id("about-backdrop")
                    .occlude()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui::rgba(0x0000004d))
                    .on_click(cx.listener(|view: &mut Self, _, _, cx| {
                        view.about_act(about_registry::ACTION_CLOSE, cx)
                    }))
                    .child(
                        div()
                            .id("about")
                            .occlude()
                            .w(scale(480.0))
                            .p(scale(20.0))
                            .flex()
                            .flex_col()
                            .gap(scale(12.0))
                            .bg(background)
                            .text_color(foreground)
                            .border_1()
                            .border_color(border)
                            .rounded(scale(6.0))
                            .shadow_lg()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child(SharedString::from(self.label("about.title", &[]))),
                            )
                            .child(
                                div()
                                    .text_color(muted)
                                    .child(SharedString::from(self.label("about.tagline", &[]))),
                            )
                            .child(body)
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .justify_end()
                                    .gap(scale(8.0))
                                    .child(button(
                                        about_registry::ACTION_COPY,
                                        self.label("about.copy", &[]),
                                        true,
                                    ))
                                    .child(button(
                                        about_registry::ACTION_NOTICES,
                                        self.label("about.notices", &[]),
                                        false,
                                    ))
                                    .child(button(
                                        about_registry::ACTION_CLOSE,
                                        self.label("about.close", &[]),
                                        false,
                                    )),
                            ),
                    ),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
}
