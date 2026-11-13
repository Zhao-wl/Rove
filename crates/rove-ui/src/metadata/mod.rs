//! 元数据视图界面（#42）：外部符号（DLL 里的类型与成员）的只读视图。
//!
//! **元数据视图是一份普通文档**：资源是 `Resource::Virtual("rove-metadata:<类型键>")`，正文是 #34 在建索引时
//! 生成的签名 + XML 文档注释，作为只读 `TextSource` 经文档加载器（`skeleton/text.rs` 的 `VirtualSource`）交给
//! #17 的打开文档服务与 #19 的代码视图。所以标签复用、预览 / 固定、标签状态恢复、导航历史、面包屑、
//! `Ctrl+Shift+O`、`#region` 折叠、语法高亮与主题全部沿用既有实现，没有另造一条打开路径。
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`loader`] | `MetadataLoader`：把类型键翻译成文档内容（只读原因 = 元数据视图），并记下标题 / DLL 路径 | 否 |
//! | [`snippets`] | 速览预览里外部符号的片段提供方（#41 的 `ExternalSnippets` 接缝） | 否 |
//! | [`registry`] | 界面字符串 | 否 |
//! | [`tooltip`] | 标签悬停提示（DLL 完整路径）的小视图 | 是 |
//!
//! 内容与位置的核心逻辑（`rove.metadataView.showDocs`、行号换算、视图内位置 → 符号）在核心层
//! `rove_lang_cs::metadata_view` 与 `router/metadata.rs`；窗口装配在 `skeleton/workbench/metadata_host.rs`。

pub mod loader;
pub mod registry;
pub mod snippets;
pub mod tooltip;

pub use loader::{MetadataLoader, ViewInfo};
