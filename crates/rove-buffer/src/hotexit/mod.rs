//! 热退出与崩溃恢复（#50，文档 ID R49）：与界面无关，不依赖 gpui。
//!
//! 关窗口时有未保存的修改不追问，下次打开原样回来；上次**意外退出**时同样回来，并在状态栏
//! 提示一次“已恢复上次未保存的修改”（`UX_SPEC` §6.4）。
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`settings`] | `files.hotExit` 的三种取值与 `files.restoreUndoStack` 的登记与读取 |
//! | [`record`] | 一份备份的内容与编解码：路径、编码 + BOM、主换行符与“是否混合”、文档版本、缓冲内容、磁盘标记、撤销栈、标签顺序 |
//! | [`codec`] | 长度前缀的二进制编解码；读不出来一律按“没有备份” |
//! | [`store`] | `%LOCALAPPDATA%\Rove\backups\<工作区哈希>\` 的布局、会话状态（退出标记 + 恢复失败次数）、隔离与清理 |
//! | [`schedule`] | 按编辑去抖的持续备份（安静期 500 ms、最长等待 2 s） |
//! | [`service`] | [`BackupService`]：窗口拥有的服务，启动时给出 [`Restore`] 决策 |
//! | [`crash`] | panic 与致命错误的本地日志（`%LOCALAPPDATA%\Rove\logs\`，保留最近 10 份，**不上传**） |
//!
//! **本 issue 不新增 `rove.*` 设置项**（issue §2）。三个固定值写在这里，改动时一起看：
//!
//! | 固定值 | 取值 | 位置 |
//! |---|---|---|
//! | 去抖安静期 / 最长等待 | 500 ms / 2 s | [`schedule::QUIET_PERIOD`] / [`schedule::MAX_WAIT`] |
//! | 撤销栈上限 | 200 个撤销单元 | [`service::UNDO_LIMIT`] |
//! | 崩溃日志保留份数 | 10 | [`crash::KEEP_LOGS`] |
//!
//! **不发起任何网络请求**：本模块只用 `std::fs`、`std::backtrace` 与 crate 内部的原子写入。
//!
//! 不包含：保存本身、原子写入、编码回写、外部修改与保存冲突、关闭未保存文件的模态框
//! （都在 [`crate::save`]，#49）；本模块只决定什么时候跳过那个模态框。

pub mod codec;
pub mod crash;
pub mod record;
pub mod schedule;
pub mod service;
pub mod settings;
pub mod store;

pub use codec::DecodeError;
pub use record::Backup;
pub use schedule::{BackupSchedule, MAX_WAIT, QUIET_PERIOD};
pub use service::{BackupService, Restore, UNDO_LIMIT};
pub use settings::{HotExitMode, HotExitOptions, HOT_EXIT, RESTORE_UNDO_STACK};
pub use store::{default_state_root, workspace_key, BackupStore, Session};
