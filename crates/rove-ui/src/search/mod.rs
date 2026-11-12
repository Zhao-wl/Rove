//! 全局搜索视图（#25）。引擎在核心层 `rove-search`（不依赖 gpui），这里只有界面。
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`model`] | 结果模型：按文件分组、排序（`search.sortOrder`）、折叠（`search.collapseResults`）、删除；展平快照 `ResultRows` 实现 #18 的 `ListModel` | 否 |
//! | [`registry`] | 命令（`search.action.*`、`toggleSearch*`）、界面字符串、颜色键、视图小节、`SearchAction` | 仅 action |
//! | [`view`] | `SearchView`：输入框与开关、后台流式搜索与取消、进度条、摘要与空状态、虚拟化结果列表 | 是 |
//!
//! 宿主（骨架的 `Workbench`，见 `skeleton/workbench/search_host.rs`）负责：在侧边栏小节里绘制视图、
//! 把键位服务分发的 `SearchAction` 转给视图、经 #17 的 `OpenDocuments` 打开结果、发布全局上下文键
//! `searchViewletVisible` / `hasSearchResult`。
pub mod model;
pub mod registry;
pub mod view;
