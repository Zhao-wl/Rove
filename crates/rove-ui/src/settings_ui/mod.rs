//! Shared settings editor backed by the application settings registry.
pub mod model;
pub mod registry;
pub mod view;

pub(crate) struct SharedSettings(pub std::sync::Arc<rove_config::SettingsService>);
impl gpui::Global for SharedSettings {}

impl SharedSettings {
    pub(crate) fn install(
        service: std::sync::Arc<rove_config::SettingsService>,
        cx: &mut gpui::App,
    ) {
        cx.set_global(Self(service));
        cx.background_executor()
            .spawn(async {
                model::catalog_rows();
            })
            .detach();
    }
}
