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
// #24：快速打开与命令面板（`:` `@` `#` `>` 前缀、命令中心入口）。
pub mod quick_access;

// #49：保存与外部修改（命令、反馈映射、关闭确认、外部修改提示）。核心在 rove_buffer::save。
pub mod save;
// #40：C# 导航交互（转到定义 / 实现、悬停、Ctrl+点击、语义同名高亮）。
pub mod nav;
// #25：全局搜索视图（引擎在核心层 `rove-search`）。
pub mod search;
// #41：速览窗口（速览定义、引用、实现；D3 默认方式）。
pub mod peek;
// #42：元数据视图界面（外部符号的只读视图；内容与位置的核心逻辑在 rove_lang_cs::metadata_view）。
pub mod metadata;
// #23：资源管理器（文件树、紧凑文件夹、定位当前文件、过滤、文件装饰）。
pub mod explorer;
