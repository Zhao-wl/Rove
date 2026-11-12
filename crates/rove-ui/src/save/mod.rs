//! 保存与外部修改的界面层（#49）。核心逻辑在 `rove_buffer::save`（不依赖 gpui）。
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`registry`] | 命令（VS Code ID）、界面字符串、反馈键与动作 ID、`SaveAction` | 仅 action |
//! | [`controller`] | [`SaveController`]：可保存文档、保存结果 → 五级反馈、反馈按钮的后续动作、关闭确认、外部修改的合并与去重 | 否 |
//!
//! 窗口装配见 `skeleton/workbench.rs` 的 `#49` 段。

pub mod controller;
pub mod registry;

pub use controller::{display_name, encoding_label, CloseCheck, Effect, SaveController};

#[cfg(test)]
mod tests;
