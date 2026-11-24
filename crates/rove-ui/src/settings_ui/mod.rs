//! Shared settings editor backed by the application settings registry.
pub mod model;
pub mod registry;
pub mod view;

pub(crate) struct SharedSettings(pub std::sync::Arc<rove_config::SettingsService>);
impl gpui::Global for SharedSettings {}
