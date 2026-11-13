//! 资源管理器（#23）：“文件夹”小节里的文件树、紧凑文件夹、定位当前文件、过滤与文件装饰。
//!
//! 只读：文件操作（新建、重命名、删除、复制路径、拖放）属于 R62，本模块一律不登记。
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`model`] | `explorer.*` 设置读取、排序、紧凑文件夹（单链压缩）、树数据源（节点 ID = 工作区路径）、文件事件的增量刷新计划、定位计划 | 否 |
//! | [`decorations`] | 装饰接口与两类来源：只读标记（#15 的只读规则）、错误 / 警告计数（问题面板，R60 前为空） | 否 |
//! | [`registry`] | 命令（VS Code ID）、界面字符串、上下文键名与命令的 GPUI action | 仅 action |
//! | [`view`] | 窗口状态 [`view::ExplorerView`]：#18 `ListView` 的包装、行渲染（图标、紧凑文件夹分段、装饰）、命令、空状态 | 是 |
//!
//! 宿主（骨架 `Workbench`，见 `skeleton/workbench/explorer_host.rs`）负责：建立 / 重建工作区、
//! 订阅文件事件、经 #17 的 `OpenDocuments` 打开文件（`OpenSource::Explorer`）、
//! 活动编辑器变化时定位当前文件、发布全局上下文键。
//!
//! # 路径与节点
//!
//! 节点 ID 永远是**工作区路径**（紧凑文件夹链的 ID 是链首目录的路径，标签是 `a/b/c`），
//! 选中、定位、过滤与装饰都按路径处理，不依赖行下标。
//!
//! # 已知限制
//!
//! - 过滤作用在**已加载**的节点上（#18 的树组件不为过滤去加载未展开的目录，与 VS Code 的树一致）。
//! - `explorer.sortOrder = modified` 需要修改时间，#15 的契约不提供，按默认顺序排序。
//! - #15 的快照只有“目录存在与否”，目录读取失败只会表现为 `NotFound`：显示“该文件夹已不存在或无法读取”。
//! - `explorer.expandSingleFolderWorkspaces` 针对多根工作区，Rove 只有单根且不显示根行，不适用。
//! - 没有右键菜单（本 issue 没有可放进去的已实现项；文件操作菜单属于 R62）。
pub mod decorations;
pub mod model;
pub mod registry;
pub mod view;

#[cfg(test)]
mod tests;
