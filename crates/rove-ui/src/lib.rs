//! rove-ui

pub mod contributions;
pub mod documents;
pub mod feedback;
pub mod list_widget;
pub mod lists;

pub mod skeleton;

pub mod shell;

pub mod code_view;

pub(crate) mod editors;
pub use editors::settings::register as editor_setting_defaults;

pub mod keys;

pub mod app_settings;
