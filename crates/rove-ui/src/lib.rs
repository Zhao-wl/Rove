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
// #27：状态栏右侧的文件状态条目（行列、缩进、编码、换行符、语言模式、只读 / 超大文件）。
pub mod status_items;

// #21：当前文件查找条。
pub mod find;
// #24：快速打开与命令面板（`:` `@` `#` `>` 前缀、命令中心入口）。
pub mod quick_access;

// #49：保存与外部修改（命令、反馈映射、关闭确认、外部修改提示）。核心在 rove_buffer::save。
pub mod save;
// #50：热退出与崩溃恢复的界面登记项（状态栏提示）。核心在 rove_buffer::hotexit。
pub mod hotexit;
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
// #29：Markdown 预览（WebView2，按需创建；渲染、净化、预览实例与命令）。
pub mod markdown;
// #28：欢迎页、最近打开、启动参数与首选项入口（设置、键位、颜色主题、齿轮菜单）。
pub mod welcome;
// #22：超大文件与超长行的界面（提示条、状态栏、行索引进度、转到行等待、流式查找、截断标记）。
pub mod large_file;

// #74：Rove 不联网。装给 gpui 的 HTTP 客户端拒绝一切请求并记日志。
pub mod net;
// #60：侧边「引用」视图（D3 的备选形态）与 `references.preferredLocation` 的一处开关。
pub mod references;
// #60：转到工作区中的符号（`Ctrl+T` 与快速打开的 `#`，同一份数据、同一套排序、同一个实现）。
pub mod workspace_symbols;
// #76：「关于」对话框（版本、提交号、构建日期、gpui 版本、C# 索引组件状态，可一键复制）。
pub mod about;
// #149：自动更新的界面一侧（命令、提示条与通知、「关于」里的更新状态）。
// 联网部分全在核心层的 `rove-update` 里，这里不发任何请求。
pub mod update;
