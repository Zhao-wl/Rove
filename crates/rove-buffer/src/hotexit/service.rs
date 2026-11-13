//! 一个窗口的热退出备份服务（#50）：去抖队列 + 备份目录 + 启动时的恢复决策。
//!
//! 与界面无关、不依赖 gpui、不读时钟（时间由调用方传入）。写盘本身可以放到后台线程：
//! [`BackupService::write`] 只用 `&self`，参数都是 `Send` 的。
//!
//! 一次启动的顺序（见 [`BackupService::open`]）：
//!
//! 1. 读会话状态：有没有干净的退出标记、这组备份已经连续失败几次；
//! 2. 失败次数已达上限 → 整组搬到 `backups-failed\`，本次按“没有备份”启动；
//! 3. 否则把失败次数 +1、退出标记清成“未正常退出”并立刻落盘——**这一步必须在恢复之前**，
//!    否则恢复过程中再崩一次就没人记得曾经试过；
//! 4. 宿主按 [`Restore`] 打开标签；成功后调用 [`BackupService::finish_restore`] 清零失败次数
//!    并删掉已消费的备份。

use std::path::{Path, PathBuf};
use std::time::Instant;

use rove_base::DocumentId;

use super::record::Backup;
use super::schedule::BackupSchedule;
use super::settings::{HotExitMode, HotExitOptions};
use super::store::{BackupStore, Session, MAX_RESTORE_ATTEMPTS};

/// 撤销栈最多恢复多少个撤销单元。
///
/// 取 200：VS Code 不限制撤销栈长度，但备份要在每次去抖时整份写出，无上限意味着一次长会话
/// 的备份可以涨到几十 MB、每 2 秒写一遍。200 个单元覆盖的是“连续输入 + 若干次成块编辑”的
/// 一整段工作（一个单元就是一次连续输入或一次粘贴 / 替换），超限时丢掉最旧的，不报错。
pub const UNDO_LIMIT: usize = 200;

/// 启动时该怎么处理这个工作区的备份。
#[derive(Debug)]
pub struct Restore {
    /// 要恢复的文档，按标签顺序排列（`active` 为真的是活动标签）。
    pub backups: Vec<Backup>,
    /// 上次是意外退出：恢复后在状态栏提示一次（`UX_SPEC` §6.4）。正常退出时静默恢复。
    pub crashed: bool,
    /// 这组备份连续两次恢复失败，已经搬到 `backups-failed\<目录>`：提示一次，不再尝试。
    pub quarantined: Option<PathBuf>,
}

impl Restore {
    pub fn is_empty(&self) -> bool {
        self.backups.is_empty()
    }
}

/// 窗口拥有的备份服务。
#[derive(Debug)]
pub struct BackupService {
    store: BackupStore,
    schedule: BackupSchedule,
    options: HotExitOptions,
    /// 已经写出备份的文档（关闭 / 保存时据此决定要不要删文件）。
    written: std::collections::BTreeMap<DocumentId, PathBuf>,
    /// 本次运行写出的备份次数与字节数（I/O 统计，PR 里报这两个数）。
    writes: u64,
    bytes: u64,
}

impl BackupService {
    /// 打开（或新建）本工作区的备份组，并给出启动时的恢复决策。
    ///
    /// `state_root` 是 `%LOCALAPPDATA%\Rove`（[`super::store::default_state_root`]）。
    /// 备份目录读不到、写不进时一律退化成“没有备份”，绝不让启动失败。
    pub fn open(state_root: &Path, workspace: &Path, options: HotExitOptions) -> (Self, Restore) {
        let store = BackupStore::new(state_root, workspace);
        // 工作区已经不存在的整组备份顺手清掉（每组只读一个几十字节的文件）。
        BackupStore::prune_missing_workspaces(state_root);
        let session = store.session();
        let mut restore = Restore {
            backups: Vec::new(),
            crashed: !session.clean_exit,
            quarantined: None,
        };
        if options.mode == HotExitMode::Off {
            // 关掉热退出：不恢复、不备份，顺手把遗留的整组清掉。
            let _ = store.remove_group();
            restore.crashed = false;
            return (Self::with_store(store, options), restore);
        }
        if store.has_backups() {
            if session.attempts >= MAX_RESTORE_ATTEMPTS {
                restore.quarantined = store.quarantine().ok();
                restore.crashed = false;
            } else {
                // 先把“又试了一次”落盘，再交给宿主恢复：恢复中途再崩也能数对次数。
                let _ = store.write_session(&Session {
                    workspace: workspace.to_owned(),
                    clean_exit: false,
                    attempts: session.attempts + 1,
                });
                restore.backups = store.backups();
            }
        } else {
            // 没有备份：本次运行默认处于“未正常退出”状态，不提示（没东西可恢复）。
            let _ = store.write_session(&Session::new(workspace));
            restore.crashed = false;
        }
        (Self::with_store(store, options), restore)
    }

    fn with_store(store: BackupStore, options: HotExitOptions) -> Self {
        Self {
            store,
            schedule: BackupSchedule::default(),
            options,
            written: Default::default(),
            writes: 0,
            bytes: 0,
        }
    }

    pub fn options(&self) -> HotExitOptions {
        self.options
    }

    /// 设置热加载：`files.hotExit` 改成 `off` 时立刻清掉本组备份。
    pub fn set_options(&mut self, options: HotExitOptions) {
        self.options = options;
        if options.mode == HotExitMode::Off {
            self.schedule.clear();
            self.written.clear();
            let _ = self.store.remove_group();
        }
    }

    pub fn store(&self) -> &BackupStore {
        &self.store
    }

    /// 恢复成功：失败次数清零，已消费的备份删掉（宿主之后的编辑会重新写出备份）。
    pub fn finish_restore(&self, consumed: &[PathBuf]) {
        for path in consumed {
            let _ = self.store.remove_backup(path);
        }
        let _ = self.store.write_session(&Session {
            workspace: self.store.workspace().to_owned(),
            clean_exit: false,
            attempts: 0,
        });
    }

    /// 撤销栈上限：`files.restoreUndoStack = false` 时为 0（不备份撤销栈）。
    pub fn undo_limit(&self) -> usize {
        if self.options.restore_undo_stack {
            UNDO_LIMIT
        } else {
            0
        }
    }

    // ----------------------------------------------------------------- 持续备份

    /// 一次编辑：排进去抖队列（`off` 时什么都不做）。
    pub fn note_edit(&mut self, document: DocumentId, now: Instant) {
        if self.options.mode.backs_up() {
            self.schedule.note_edit(document, now);
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.schedule.next_deadline()
    }

    /// 到期该备份的文档。
    pub fn due(&mut self, now: Instant) -> Vec<DocumentId> {
        self.schedule.due(now)
    }

    /// 写一份备份（可以在后台线程调用）。`off` 时不写。
    pub fn write(&self, backup: &Backup) -> std::io::Result<u64> {
        if !self.options.mode.backs_up() {
            return Ok(0);
        }
        self.store.write_backup(backup)
    }

    /// 记下某个文档的备份已经写出（UI 线程），用于保存 / 关闭时删除。
    pub fn note_written(&mut self, document: DocumentId, path: &Path, bytes: u64) {
        self.written.insert(document, path.to_owned());
        self.writes += 1;
        self.bytes += bytes;
    }

    /// 文档已保存或已关闭：出队并删掉它的备份。
    pub fn discard(&mut self, document: DocumentId) {
        self.schedule.forget(document);
        if let Some(path) = self.written.remove(&document) {
            let _ = self.store.remove_backup(&path);
        }
    }

    /// 本次运行写出的备份次数与总字节数。
    pub fn io_stats(&self) -> (u64, u64) {
        (self.writes, self.bytes)
    }

    // ----------------------------------------------------------------- 退出

    /// 关闭窗口时是否跳过 #49 的“保存 / 不保存 / 取消”。
    pub fn skips_close_confirmation(&self, last_window: bool) -> bool {
        self.options.mode.skips_close_confirmation(last_window)
    }

    /// 正常退出：写下干净的退出标记（下次启动静默恢复，不提示）。
    /// 还有脏文档时宿主必须先把它们的备份同步写完再调用这个方法。
    pub fn mark_clean_exit(&self) {
        if !self.options.mode.backs_up() {
            return;
        }
        if !self.store.has_backups() {
            // 没有未保存的修改：整组删掉，下次启动连目录都不用读。
            let _ = self.store.remove_group();
            return;
        }
        let _ = self.store.write_session(&Session {
            workspace: self.store.workspace().to_owned(),
            clean_exit: true,
            attempts: 0,
        });
    }
}
