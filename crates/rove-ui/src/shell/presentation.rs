//! #70：窗口阅读布局。Zen 的临时状态不会覆盖工作区的常规布局。
use super::{layout::LayoutState, Shell};
use crate::editors::settings::Tabs;
use gpui::Window;

#[derive(Default)]
pub(super) struct Presentation {
    centered: bool,
    zen: Option<ZenSession>,
    pending_full_screen: Option<bool>,
}

struct ZenSession {
    previous_layout: LayoutState,
    centered: bool,
    entered_full_screen: bool,
    hide_activity_bar: bool,
    hide_status_bar: bool,
    hide_line_numbers: bool,
    tabs: Tabs,
}

impl Presentation {
    fn target_full_screen(&mut self, window: &Window) -> bool {
        let actual = window.is_fullscreen();
        if self.pending_full_screen == Some(actual) {
            self.pending_full_screen = None;
        }
        self.pending_full_screen.unwrap_or(actual)
    }

    pub(super) fn in_zen(&self) -> bool {
        self.zen.is_some()
    }

    pub(super) fn centered(&self) -> bool {
        self.zen.as_ref().map_or(self.centered, |zen| zen.centered)
    }

    pub(super) fn toggle_centered(&mut self) {
        if let Some(zen) = &mut self.zen {
            zen.centered = !zen.centered;
        } else {
            self.centered = !self.centered;
        }
    }

    pub(super) fn hides_activity_bar(&self) -> bool {
        self.zen.as_ref().is_some_and(|zen| zen.hide_activity_bar)
    }

    pub(super) fn hides_status_bar(&self) -> bool {
        self.zen.as_ref().is_some_and(|zen| zen.hide_status_bar)
    }

    pub(super) fn layout_to_save(&self, current: &LayoutState, restore_zen: bool) -> LayoutState {
        let mut layout = self
            .zen
            .as_ref()
            .map_or(current, |zen| &zen.previous_layout)
            .clone();
        layout.window = current.window;
        layout.zen_mode = restore_zen && self.in_zen();
        layout
    }
}

impl Shell {
    pub(super) fn restore_presentation(&mut self, window: &mut Window) {
        if self.layout.zen_mode && self.setting_bool("zenMode.restore") {
            self.enter_zen(window);
        }
    }

    pub(super) fn toggle_full_screen(&mut self, window: &Window) {
        let full_screen = self.presentation.target_full_screen(window);
        if !full_screen {
            self.record_window(window);
        }
        // On Windows GPUI queues the change. A Zen command arriving before the platform
        // has applied the toggle must see the queued state.
        self.presentation.pending_full_screen = Some(!full_screen);
        window.toggle_fullscreen();
    }

    pub(super) fn enter_zen(&mut self, window: &mut Window) {
        let entered_full_screen = self.setting_bool("zenMode.fullScreen")
            && !self.presentation.target_full_screen(window);
        let tabs = match self.setting_str("zenMode.showTabs").as_str() {
            "none" => Tabs::None,
            "single" => Tabs::Single,
            _ => Tabs::Multiple,
        };
        if entered_full_screen {
            self.toggle_full_screen(window);
        }
        self.presentation.zen = Some(ZenSession {
            previous_layout: self.layout.clone(),
            centered: self.presentation.centered || self.setting_bool("zenMode.centerLayout"),
            entered_full_screen,
            hide_activity_bar: self.setting_bool("zenMode.hideActivityBar"),
            hide_status_bar: self.setting_bool("zenMode.hideStatusBar"),
            hide_line_numbers: self.setting_bool("zenMode.hideLineNumbers"),
            tabs,
        });
        self.layout.sidebar_visible = false;
        self.layout.panel_visible = false;
        self.feedback
            .set_zen_silent(self.setting_bool("zenMode.silentNotifications"));
    }

    pub(super) fn exit_zen(&mut self, window: &Window) {
        let Some(zen) = self.presentation.zen.take() else {
            return;
        };
        let window_placement = self.layout.window;
        self.layout = zen.previous_layout;
        self.layout.window = window_placement;
        self.layout.zen_mode = false;
        self.feedback.set_zen_silent(false);
        if zen.entered_full_screen && self.presentation.target_full_screen(window) {
            self.toggle_full_screen(window);
        }
    }

    pub(crate) fn zen_context(&self) -> Option<&'static str> {
        (self.presentation.in_zen() && self.feedback.modal().is_none()).then_some("inZenMode")
    }

    pub(crate) fn zen_tabs(&self) -> Option<Tabs> {
        self.presentation.zen.as_ref().map(|zen| zen.tabs)
    }

    pub(crate) fn zen_hides_line_numbers(&self) -> bool {
        self.presentation
            .zen
            .as_ref()
            .is_some_and(|zen| zen.hide_line_numbers)
    }
}
