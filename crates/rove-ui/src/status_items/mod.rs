//! 状态栏右侧的文件状态条目（#27，UX_SPEC §1.5、§3.1–§3.6）。
//!
//! 条目容器与登记机制是 #16 的 [`shell::status`](crate::shell::status)；本模块负责
//! **往里登记六个条目**，并把文档状态投影成它们的文本、悬停提示与颜色级别：
//!
//! | 条目 | 登记 ID | 显示 | 点击 |
//! |---|---|---|---|
//! | 光标位置 | [`registry::CURSOR`] | `行 12，列 34`（有选区时附加已选字符数） | `workbench.action.gotoLine` |
//! | 缩进 | [`registry::INDENT`] | `空格: 4` / `制表符长度: 4` | `changeEditorIndentation` |
//! | 编码 | [`registry::ENCODING`] | `UTF-8` / `GBK`（自动识别的在悬停里注明） | `workbench.action.editor.changeEncoding` |
//! | 换行符 | [`registry::EOL`] | `LF` / `CRLF` / `混合` | `workbench.action.editor.changeEOL` |
//! | 语言模式 | [`registry::LANGUAGE`] | 语言名 | `workbench.action.editor.changeLanguageMode` |
//! | 只读 / 超大文件 | [`registry::READ_ONLY`] | `只读` / `超大文件 · 只读` | 无（纯提示） |
//!
//! [`model`] 是纯数据（不碰 GPUI，可直接单元测试）：状态 → [`StatusEntry`](crate::shell::status::StatusEntry)
//! 与各选择器的条目列表。窗口装配（每帧同步、选择器浮层、编码重读、语言切换）在
//! `skeleton/workbench/status_host.rs`。
//!
//! 文字全部走 [`registry::STRINGS`]，颜色只用 [`StatusKind`](crate::shell::status::StatusKind)
//! 映射到的 `statusBarItem.*` 键，命令与设置项都经登记机制加入。

pub mod model;
pub mod registry;

#[cfg(test)]
mod tests;
