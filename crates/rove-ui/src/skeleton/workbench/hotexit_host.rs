//! #50：热退出与崩溃恢复的窗口装配。核心逻辑在 `rove_buffer::hotexit`（不依赖 gpui）。
//!
//! - **启动**：`%LOCALAPPDATA%\Rove\backups\<工作区哈希>\` 里有本工作区的备份就恢复它们——
//!   按备份记下的标签顺序逐个打开、活动标签还原，内容取自备份并标记为脏（磁盘一个字节都不动）。
//!   上次没有干净的退出标记（= 意外退出）时，恢复完在状态栏提示一次“已恢复上次未保存的修改”
//!   （第 1 级反馈，3 秒；**不弹通知、不弹模态框**）。
//! - **持续备份**：代码视图每发出一次编辑就 `note_edit`，到期后在后台原子写出一份备份
//!   （安静期 500 ms、最长等待 2 s）。**不是只在退出时备份**——那样崩溃一条都救不回来。
//! - **清理**：保存、关闭、恢复消费完都删掉对应备份；工作区目录消失时整组清掉（核心层做）。
//! - **退出**：`files.hotExit` 允许时跳过 #49 的“保存 / 不保存 / 取消”（经
//!   `SaveController::set_hot_exit`），把全部脏文档同步备份完再写干净的退出标记。
//!
//! 恢复路径分两种：磁盘上还在的文件走 #17 正常的打开流程（文档身份归加载器，之后资源管理器
//! 再点同一个文件会复用这个标签，不会把未保存的修改冲掉），**已被删除的文件**走
//! `open_ready` 直接放一个就绪标签并标成“已删除”（与 #49 第 4 节一致）。

use super::*;
use rove_buffer::edit::EditOptions;
use rove_buffer::hotexit::{default_state_root, Backup, BackupService, HotExitOptions, Restore};
use rove_buffer::save::TextFile;
use rove_buffer::{DocumentMetadata, SizeClass, TextSource};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

/// 恢复出来的文档身份从这里开始，不与加载器的计数（从 1 开始）冲突，也不与 #47 的未命名
/// 文档（`1 << 62`）冲突。只有“磁盘上已经没有了”的文档才用它。
const DELETED_BASE: u64 = 1 << 61;

/// 窗口的热退出状态。
#[derive(Default)]
pub(super) struct HotExit {
    /// 没有 `%LOCALAPPDATA%` 或禁用时为 `None`：整个功能静默关闭，不影响启动。
    service: Option<BackupService>,
    /// 还没恢复到标签上的备份，按规范化后的路径索引。
    pending: BTreeMap<String, Backup>,
    /// 已经恢复到标签上的备份路径（全部恢复完后一次性清理）。
    consumed: Vec<PathBuf>,
    /// 上次是意外退出：恢复完提示一次。
    crashed: bool,
    /// 恢复完要激活的标签对应的路径。
    activate: Option<PathBuf>,
    /// 已经排了一个“到期写备份”的定时器。
    timer: bool,
    /// 正在走窗口关闭流程，且热退出接管了关闭确认。与 `SaveController::set_hot_exit` 的
    /// 闭包共享：关单个标签时为假（标签关闭永远询问）。
    closing: Rc<Cell<bool>>,
    /// 已经装过关闭确认的开关点。
    hooked: bool,
}

impl Workbench {
    /// 应用入口调用（测试入口 `Workbench::new` 不调用，所以单元测试不碰用户目录）。
    pub(crate) fn start_hot_exit(&mut self, cx: &mut Context<Self>) {
        let Some(state_root) = default_state_root(|key| std::env::var_os(key)) else {
            return;
        };
        self.start_hot_exit_in(&state_root, cx);
    }

    /// 指定状态根目录（测试传临时目录）。
    pub(crate) fn start_hot_exit_in(&mut self, state_root: &Path, cx: &mut Context<Self>) {
        let options = HotExitOptions::read(&*self.settings);
        let (service, restore) = BackupService::open(state_root, &self.root, options);
        self.hotexit.service = Some(service);
        self.hook_hot_exit();
        self.apply_restore(restore, cx);
    }

    /// 把热退出接到 #49 留下的开关点上：关窗口时（且设置允许）不再确认。
    fn hook_hot_exit(&mut self) {
        if self.hotexit.hooked {
            return;
        }
        self.hotexit.hooked = true;
        let closing = self.hotexit.closing.clone();
        self.editing
            .saves
            .borrow_mut()
            .set_hot_exit(move |_| closing.get());
    }

    /// 设置热加载：`files.hotExit` / `files.restoreUndoStack` 变了就换一套选项
    /// （改成 `off` 时核心层会立刻清掉本组备份）。
    pub(super) fn hot_exit_settings_changed(&mut self) {
        let options = HotExitOptions::read(&*self.settings);
        if let Some(service) = self.hotexit.service.as_mut() {
            if service.options() != options {
                service.set_options(options);
            }
        }
    }

    // ------------------------------------------------------------------------- 恢复

    fn apply_restore(&mut self, restore: Restore, cx: &mut Context<Self>) {
        if let Some(dir) = restore.quarantined {
            let _ = shell::view::report(
                self,
                crate::hotexit::KEY_QUARANTINED,
                Reason::Receipt,
                Message {
                    key: "hotexit.status.quarantined".into(),
                    args: Arc::from([("dir".into(), dir.display().to_string().into())]),
                },
                Arc::from([]),
                cx,
            );
        }
        if restore.backups.is_empty() {
            return;
        }
        self.hotexit.crashed = restore.crashed;
        self.hotexit.activate = restore
            .backups
            .iter()
            .find(|b| b.active)
            .map(|b| b.path.clone());
        // 备份已经按标签顺序排好；逐个打开，顺序自然就对了。
        for backup in restore.backups {
            let path = backup.path.clone();
            self.hotexit.consumed.push(path.clone());
            if path.exists() {
                self.hotexit.pending.insert(hot_exit_key(&path), backup);
                self.reopen_restored(&path, cx);
            } else {
                // 磁盘上的文件已经没了：直接放一个“已删除”状态的就绪标签，内容保留。
                self.open_deleted_restored(backup, cx);
            }
        }
        self.finish_restore_if_done(cx);
    }

    /// 经 #17 的打开文档服务重新打开（不预览、不抢焦点）。内容在 `load_text_file` 里换成备份。
    fn reopen_restored(&mut self, path: &Path, cx: &mut Context<Self>) {
        let resource = match path.strip_prefix(&self.root) {
            Ok(rel) => WorkspacePath::new(&rel.to_string_lossy().replace('\\', "/"))
                .map(Resource::Workspace)
                .unwrap_or_else(|_| Resource::File(path.to_path_buf())),
            Err(_) => Resource::File(path.to_path_buf()),
        };
        let options = OpenOptions {
            // 恢复出来的是有未保存修改的文档：固定钉住的源码标签，不能是预览标签。
            preview_tab: false,
            preserve_focus: true,
            presentation: crate::documents::Presentation::Source,
            ..Default::default()
        };
        if self
            .open_documents()
            .open(resource, options, Cancellation::default())
            .is_err()
        {
            // 打不开就当这份备份不存在（恢复失败绝不能让 Rove 打不开）。
            self.hotexit.pending.remove(&hot_exit_key(path));
        }
        cx.notify();
    }

    /// 磁盘上已被删除的文档：直接放一个就绪标签，标成“已删除”。
    fn open_deleted_restored(&mut self, backup: Backup, cx: &mut Context<Self>) {
        let path = backup.path.clone();
        let file = self.restored_file(backup);
        let id = DocumentId(DELETED_BASE + self.hotexit.consumed.len() as u64);
        let document = Arc::new(Document {
            id,
            resource: Resource::File(path.clone()),
            metadata: DocumentMetadata {
                language: Arc::from(file.language()),
                encoding: file.encoding().clone(),
                bom: file.bom(),
                line_endings: file.line_endings().clone(),
                read_only: Arc::from([]),
                size: SizeClass::Normal,
            },
            text: file.snapshot() as Arc<dyn TextSource>,
        });
        self.editing.saves.borrow_mut().attach(id, file);
        self.editing.deleted.insert(id);
        self.editor.open_ready(Resource::File(path), document);
        self.editor.set_dirty(id, true);
        self.sync_views(cx);
        self.sync_editing(cx);
    }

    /// 按备份建一个有未保存修改的文档。`files.restoreUndoStack = false` 时丢掉备份里的撤销栈
    /// （设置可能在两次会话之间被改掉，所以恢复时再判一次）。
    fn restored_file(&self, mut backup: Backup) -> TextFile {
        let keep_undo = self
            .hotexit
            .service
            .as_ref()
            .is_some_and(|service| service.options().restore_undo_stack);
        if !keep_undo {
            backup.undo = Default::default();
        }
        let options = EditOptions::read(
            &*self.settings,
            &Scope {
                language: (!backup.language.is_empty()).then(|| backup.language.clone()),
            },
        );
        TextFile::from_backup(backup, Arc::from([]), options)
    }

    /// `load_text_file` 的岔路：这个路径有待恢复的备份时，不读盘，直接用备份建文件。
    pub(super) fn take_restored_file(&mut self, path: &Path) -> Option<TextFile> {
        let backup = self.hotexit.pending.remove(&hot_exit_key(path))?;
        Some(self.restored_file(backup))
    }

    /// 全部备份都落到标签上了：激活原来的活动标签、清理已消费的备份、按需提示。
    pub(super) fn finish_restore_if_done(&mut self, cx: &mut Context<Self>) {
        if self.hotexit.consumed.is_empty() {
            return;
        }
        // 打不开的文件（权限、格式不受支持）不能把恢复流程永远卡住：标签已经是“打开失败”
        // 的那几份按“没有这份备份”处理。
        if !self.hotexit.pending.is_empty() {
            let failed: Vec<String> = self
                .editor
                .groups
                .values()
                .flat_map(|pane| &pane.tabs)
                .filter(|tab| matches!(tab.content, Content::Failed(_)))
                .filter_map(|tab| self.disk_path(&tab.resource))
                .map(|path| hot_exit_key(&path))
                .collect();
            for key in failed {
                self.hotexit.pending.remove(&key);
            }
        }
        if !self.hotexit.pending.is_empty() {
            return;
        }
        let consumed = std::mem::take(&mut self.hotexit.consumed);
        if let Some(path) = self.hotexit.activate.take() {
            self.activate_restored_tab(&path);
        }
        if let Some(service) = &self.hotexit.service {
            service.finish_restore(&consumed);
        }
        // 消费掉的备份文件已经删了，但恢复出来的文档仍然是脏的：立刻重新排进去抖队列，
        // 否则“恢复完马上再崩一次”会把刚救回来的修改弄丢。
        let dirty: Vec<DocumentId> = {
            let saves = self.editing.saves.borrow();
            saves.documents().filter(|d| saves.is_dirty(*d)).collect()
        };
        for document in dirty {
            self.note_hot_exit_edit(document, cx);
        }
        if std::mem::take(&mut self.hotexit.crashed) {
            // UX_SPEC §6.4：第 1 级反馈，3 秒后消失，不弹通知、不弹模态框。
            let _ = shell::view::report(
                self,
                crate::hotexit::KEY_RESTORED,
                Reason::Receipt,
                Message {
                    key: "hotexit.status.restored".into(),
                    args: Arc::from([]),
                },
                Arc::from([]),
                cx,
            );
        }
        cx.notify();
    }

    fn activate_restored_tab(&mut self, path: &Path) {
        let key = hot_exit_key(path);
        let found = self.editor.groups.iter().find_map(|(group, pane)| {
            pane.tabs
                .iter()
                .find(|tab| {
                    self.disk_path(&tab.resource)
                        .is_some_and(|p| hot_exit_key(&p) == key)
                })
                .map(|tab| (*group, tab.id))
        });
        if let Some((group, tab)) = found {
            self.editor.activate(group, tab, false);
        }
    }

    // ------------------------------------------------------------------------- 持续备份

    /// 代码视图里的一次编辑：排进去抖队列。
    pub(super) fn note_hot_exit_edit(&mut self, document: DocumentId, cx: &mut Context<Self>) {
        let Some(service) = self.hotexit.service.as_mut() else {
            return;
        };
        service.note_edit(document, Instant::now());
        self.schedule_hot_exit_backup(cx);
    }

    fn schedule_hot_exit_backup(&mut self, cx: &mut Context<Self>) {
        if self.hotexit.timer {
            return;
        }
        let Some(deadline) = self
            .hotexit
            .service
            .as_ref()
            .and_then(|s| s.next_deadline())
        else {
            return;
        };
        self.hotexit.timer = true;
        let timer = cx
            .background_executor()
            .timer(deadline.saturating_duration_since(Instant::now()));
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |view, cx| {
                view.hotexit.timer = false;
                view.write_due_backups(cx);
                view.schedule_hot_exit_backup(cx);
            });
        })
        .detach();
    }

    fn write_due_backups(&mut self, cx: &mut Context<Self>) {
        let due = match self.hotexit.service.as_mut() {
            Some(service) => service.due(Instant::now()),
            None => return,
        };
        for document in due {
            let Some(backup) = self.backup_of(document) else {
                continue;
            };
            // 写盘在后台执行器上（原子写入要刷盘）；UI 线程只做一次文本快照复制。
            let Some(store) = self
                .hotexit
                .service
                .as_ref()
                .filter(|service| service.options().mode.backs_up())
                .map(|service| service.store().clone())
            else {
                continue;
            };
            let path = backup.path.clone();
            let task = cx
                .background_executor()
                .spawn(async move { store.write_backup(&backup) });
            cx.spawn(async move |this, cx| {
                let written = task.await;
                let _ = this.update(cx, |view, _| {
                    if let (Some(service), Ok(bytes)) = (view.hotexit.service.as_mut(), written) {
                        service.note_written(document, &path, bytes);
                    }
                });
            })
            .detach();
        }
    }

    /// 某个文档此刻的备份（含标签顺序与是否活动标签）。
    fn backup_of(&self, document: DocumentId) -> Option<Backup> {
        let (index, active) = self.tab_position(document)?;
        let saves = self.editing.saves.borrow();
        let file = saves.file(document)?;
        if !file.is_dirty() {
            return None;
        }
        let limit = self.hotexit.service.as_ref()?.undo_limit();
        file.to_backup(limit, index, active)
    }

    /// 文档在窗口里的标签序号（跨编辑器组的全局序号）与是否活动标签。
    fn tab_position(&self, document: DocumentId) -> Option<(u32, bool)> {
        let mut index = 0u32;
        for (group, pane) in &self.editor.groups {
            for tab in &pane.tabs {
                if tab.opened().is_some_and(|o| o.document.id == document) {
                    let active = pane.active == Some(tab.id) && *group == self.editor.focused;
                    return Some((index, active));
                }
                index += 1;
            }
        }
        None
    }

    /// 文档已保存 / 已关闭：出队并删掉它的备份。
    pub(super) fn discard_hot_exit_backup(&mut self, document: DocumentId) {
        if let Some(service) = self.hotexit.service.as_mut() {
            service.discard(document);
        }
    }

    // ------------------------------------------------------------------------- 退出

    /// 窗口关闭流程开始：按 `files.hotExit` 决定要不要接管关闭确认。
    /// Rove 一个窗口一个文件夹，所以“最后一个窗口”= 退出应用。
    pub(super) fn begin_hot_exit_window_close(&mut self, last_window: bool) {
        let skips = self
            .hotexit
            .service
            .as_ref()
            .is_some_and(|s| s.skips_close_confirmation(last_window));
        self.hotexit.closing.set(skips);
    }

    /// 关闭单个标签：热退出不接管（VS Code 同样只在窗口 / 应用级别跳过询问）。
    pub(super) fn end_hot_exit_window_close(&mut self) {
        self.hotexit.closing.set(false);
    }

    /// 窗口真的要关了：把全部脏文档的备份同步写完，再写下干净的退出标记。
    pub(super) fn flush_hot_exit(&mut self) {
        let documents: Vec<DocumentId> = self.editing.saves.borrow().documents().collect();
        let backups: Vec<Backup> = documents
            .into_iter()
            .filter_map(|document| self.backup_of(document))
            .collect();
        let Some(service) = self.hotexit.service.as_mut() else {
            return;
        };
        for backup in &backups {
            let _ = service.write(backup);
        }
        service.mark_clean_exit();
    }

    /// 本次运行写出的备份次数与字节数（基准与排查用）。
    #[cfg(test)]
    pub(crate) fn hot_exit_io_stats(&self) -> (u64, u64) {
        self.hotexit
            .service
            .as_ref()
            .map(|s| s.io_stats())
            .unwrap_or_default()
    }

    /// 测试用：立刻把到期的备份写出来（不等定时器）。
    #[cfg(test)]
    pub(crate) fn flush_due_backups(&mut self, cx: &mut Context<Self>) {
        if let Some(service) = self.hotexit.service.as_mut() {
            // 去抖的安静期在测试里不值得等：直接把排队的文档都算作到期。
            let due = service.due(Instant::now() + rove_buffer::hotexit::MAX_WAIT);
            for document in due {
                if let Some(backup) = self.backup_of(document) {
                    if let Some(service) = self.hotexit.service.as_ref() {
                        if let Ok(bytes) = service.write(&backup) {
                            let path = backup.path.clone();
                            if let Some(service) = self.hotexit.service.as_mut() {
                                service.note_written(document, &path, bytes);
                            }
                        }
                    }
                }
            }
        }
        cx.notify();
    }
}

/// 路径的比较键（Windows 上不区分大小写、分隔符统一）。
fn hot_exit_key(path: &Path) -> String {
    rove_buffer::save::path_key(path)
}
