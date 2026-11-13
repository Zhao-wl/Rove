//! 欢迎页、最近打开、启动参数与首选项入口（#28）。
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`registry`] | 命令（VS Code ID）、界面字符串、颜色键、齿轮菜单项、GPUI action | 仅 action |
//! | [`recent`] | 最近打开的文件夹与文件：去重、上限、JSON 读写（Rove 自己的状态目录） | 否 |
//! | [`startup`] | 启动参数 `rove.exe <路径>` 的分类（文件 / 文件夹 / 不存在）与长路径处理 | 否 |
//! | [`themes`] | 颜色主题选择器的候选与“预览 / 还原 / 确认”状态机 | 否 |
//! | [`view`] | 欢迎页的绘制（开始、最近、启动时显示欢迎页） | 是 |
//!
//! 宿主（骨架 `Workbench`，见 `skeleton/workbench/welcome_host.rs`）负责：把欢迎页作为编辑器类型
//! 经 #17 的 `OpenDocuments` 打开、齿轮菜单命令的实现、主题选择器的浮层（复用 #18）、
//! 经 #13 的保留格式写入 `workbench.startupEditor` 与 `workbench.colorTheme`。
pub mod recent;
pub mod registry;
pub mod startup;
pub mod themes;
pub mod view;
