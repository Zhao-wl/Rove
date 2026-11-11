//! 工作台（#16）：标题栏与命令中心、菜单、活动栏、主侧边栏、面板、状态栏容器、布局持久化、
//! 五级反馈与加载规则。纯数据模型（反馈、布局、菜单、状态栏、图标语法、登记表）不依赖 GPUI，
//! GPUI 绘制与命令分发在 [`view`]。
pub mod catalog;
pub mod feedback;
pub mod icons;
pub mod layout;
pub mod menus;
pub mod progress;
pub mod status;
pub mod view;

pub use view::{Shell, ShellHost};
