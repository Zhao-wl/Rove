//! 侧边「引用」视图（#60，决策 D3 的**备选**形态；默认仍然是 #41 的速览窗口）。
//!
//! 位置按 `UX_SPEC` §1.4 放在**面板**里（与「问题」并列），通过 #16 的视图贡献点登记。
//! `references.preferredLocation` 决定 `Shift+F12` 用速览还是这个视图，判定只有
//! [`registry::opens_in_view`] 这**一处**，由 #41 的装配调用。
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`registry`] | 命令（VS Code ID）、界面字符串、面板视图贡献点、上下文键名、`references.preferredLocation` | 仅 action |
//! | [`model`] | 结果与历史、逐条导航、消除、复制文案、两个上下文键的取值 | 否 |
//!
//! 结果树复用 #41 已经做好的分组索引（[`crate::peek::PeekResults`]）与树数据源
//! （[`crate::peek::results_tree::ResultsTree`]）：数据层给出「文件 → 计数 + 偏移」，界面按需取段，
//! 4.7 万条结果下也不为每条结果建界面节点。查询经 #1 的 C# 导航查询接口（#38 的 `CsRouter`），
//! 打开结果经 #17 的打开文档服务；窗口装配在 `skeleton/workbench/references_host.rs`。

pub mod model;
pub mod registry;

pub use model::{Entry, Mode, ReferencesState};
pub use registry::{opens_in_view, preferred_location, PreferredLocation, ReferencesAction};
