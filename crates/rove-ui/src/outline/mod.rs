//! 大纲（#26）：资源管理器中的“大纲”小节、编辑器上方的面包屑、转到文件中的符号（`Ctrl+Shift+O`）。
//!
//! 三者共用 `rove_syntax::symbols` 产出的同一份 [`DocumentSymbols`](rove_syntax::symbols::DocumentSymbols)：
//! 符号在后台按文档版本提取（`SymbolService`），窗口按文档缓存最近的结果；新版本到达前继续显示旧结果。
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`model`] | `outline.*` / `breadcrumbs.*` 设置读取、按种类过滤、大纲树数据源（节点 ID = 稳定键）、面包屑段与选择器条目、图标与 `symbolIcon.*` 颜色键 | 否 |
//! | [`goto`] | 转到符号与快速打开 `@`（#24）共用的候选与排序 [`goto::symbol_picks`] | 否 |
//! | [`registry`] | 命令（VS Code ID）、界面字符串、视图小节登记与命令的 GPUI action | 仅 action |
//! | [`view`] | 窗口状态 [`view::Outline`]：大纲列表（#18 `ListView`）、缓存、面包屑键盘状态、选择器、快速输入 | 是 |
//!
//! 宿主（骨架 `Workbench`，见 `skeleton/workbench/outline.rs`）负责：取活动编辑器的文档与光标、在后台执行
//! 符号分析、绘制面包屑与小节、经 #17 的 `OpenDocuments` 定位（`OpenSource::Outline` / `Breadcrumb`）。
//!
//! 性能：光标移动时面包屑与“跟随光标”只做 `DocumentSymbols::path_from`（复用上一条路径，
//! O(深度 · log 兄弟数)），不遍历整棵树；基准 `cargo xtask bench --filter outline/`。
pub mod goto;
pub mod model;
pub mod registry;
pub mod view;

#[cfg(test)]
mod tests;
