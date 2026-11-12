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

// #36：C# 索引状态（状态栏条目、详情面板、失败通知、rove.index.* 命令）。
pub mod index_status;
pub mod outline;

// #21：当前文件查找条。
pub mod find;
