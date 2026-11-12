//! 全局搜索引擎（#25）：`search.*` 语义、并行流式结果、50 ms 内取消、GBK 等非 UTF-8 文件。
//!
//! 核心层，不依赖 gpui；**不读取 `.gitignore`**，运行时不联网。界面（搜索视图）在 `rove-ui::search`。
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`query`] | [`SearchQuery`]（文本、正则 / 大小写 / 全词、包含 / 排除的文件、使用排除设置）、匹配器编译（`search.smartCase`）、glob 展开 |
//! | [`settings`] | `search.*` 登记与读取：[`SearchConfig`]（引擎）、[`settings::ViewSettings`]（视图） |
//! | [`engine`] | [`search`]：遍历 + 并行搜索 + 流式批次 + 取消 + 上限 + 编码识别 |
//!
//! 线程：[`search`] 是阻塞调用，由界面放到后台执行器；结果批次在调用它的线程上回调，
//! 回调只应入队并迅速返回（ARCHITECTURE §3）。

pub mod engine;
pub mod query;
pub mod settings;

pub use engine::{search, FileMatches, LineMatch, SearchSummary};
pub use query::SearchQuery;
pub use settings::SearchConfig;
