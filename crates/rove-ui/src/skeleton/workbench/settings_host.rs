use super::Workbench;
use crate::{
    documents::OpenOptions,
    settings_ui::{
        registry as reg,
        view::{SettingsView, SettingsViewEvent},
        SharedSettings,
    },
};
use gpui::{Context, Window};
use rove_base::{Cancellation, Resource};
use rove_config::Scope;

impl Workbench {
    pub(super) fn open_preferred_settings(&mut self, cx: &mut Context<Self>) {
        let mode = reg::MODE.read(self.settings.as_ref(), &Scope::default());
        if cx.has_global::<SharedSettings>() && mode.is_ok_and(|mode| mode == "ui") {
            self.open_settings_view(cx);
        } else {
            self.open_settings_json(None, cx);
        }
    }

    pub(super) fn settings_command(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            reg::OPEN_UI => self.open_settings_view(cx),
            reg::OPEN_JSON | reg::SWITCH_JSON | reg::FOCUS_FILE => {
                self.open_settings_json(None, cx)
            }
            _ => {
                if let Some(view) = &self.settings_view {
                    view.update(cx, |view, cx| view.navigate(command, window, cx));
                }
            }
        }
    }

    fn open_settings_view(&mut self, cx: &mut Context<Self>) {
        let Some(shared) = cx
            .try_global::<SharedSettings>()
            .map(|binding| binding.0.clone())
        else {
            self.open_settings_json(None, cx);
            return;
        };
        if self.settings_view.is_none() {
            let theme = self.theme.clone();
            let strings = self.strings.clone();
            let view = cx.new(|cx| SettingsView::new(shared, theme, strings, cx));
            cx.subscribe(&view, |workbench, _, event: &SettingsViewEvent, cx| {
                let SettingsViewEvent::OpenJson(position) = *event;
                workbench.open_settings_json(position, cx);
            })
            .detach();
            self.settings_view = Some(view);
        }
        self.settings_focus_requested = true;
        let resource = Resource::Virtual(reg::RESOURCE.into());
        if let Err(error) = self.open_documents().open(
            resource,
            OpenOptions {
                preview_tab: false,
                ..Default::default()
            },
            Cancellation::default(),
        ) {
            eprintln!("settings: {error}");
        }
        cx.notify();
    }

    pub(super) fn sync_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.editor.active().current() else {
            return;
        };
        if !matches!(&tab.resource, Resource::Virtual(id) if id.as_ref() == reg::RESOURCE) {
            return;
        }
        let Some(document) = tab.opened().map(|opened| opened.document.id) else {
            return;
        };
        if !self.settings_notice_shown {
            self.settings_notice_shown = true;
            self.report_shared_notice(document, cx);
        }
        if std::mem::take(&mut self.settings_focus_requested) {
            if let Some(view) = &self.settings_view {
                view.read(cx).focus_search(window, cx);
            }
        }
    }
}
