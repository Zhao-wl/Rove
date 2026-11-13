//! 保存与外部修改的界面控制器（#49）：不依赖 GPUI，可单独测试。
//!
//! 持有窗口里打开的可保存文档（[`TextFile`]），把核心层的 [`SaveOutcome`] / [`DiskChange`] 按
//! `UX_SPEC` §6.1 / §6.4 交给 #16 的五级反馈：
//!
//! | 情况 | 级别 | 按钮 |
//! |---|---|---|
//! | 被占用（已自动重试一次） | 4 通知 | 重试、另存为 |
//! | 只读属性 | 4 通知 | 覆盖（清除只读属性）、另存为；**没有**提权入口 |
//! | 保存冲突 | 4 通知 | 覆盖、放弃本地修改（“比较”在对比视图完成前隐藏）；`overwriteFileOnDisk` 时不提示 |
//! | 无法编码的字符 | 5 模态 | 改用 UTF-8 保存、仍然保存（替换为 ?）、取消 |
//! | 关闭有未保存修改的文档 | 5 模态 | 保存、不保存、取消（`files.hotExit` 的跳过由 #50 经 [`SaveController::set_hot_exit`] 接管） |
//! | 外部修改且本地有修改 | 3 提示条 | 重新加载（丢弃本地修改）、保留本地修改 |
//! | 外部修改且本地没有修改 | 无 | 静默重新加载 |
//!
//! 反馈键带文档 ID：同一文档同一原因只有一条，再次触发时反馈层更新次数而不是再弹一条。
//! 失败反馈里**那次请求的后续按钮**（“重试 / 覆盖 / 改用 UTF-8 / 仍然保存（替换为 ?）”）在
//! **最近一次未完成的保存请求**上叠加选项（#140）：另存为失败后它们仍然作用于另存为的目标，
//! 通知说的也是目标文件。关闭确认模态框的“保存”不在此列——它是一条新的“保存本文档”命令，
//! 永远是原地保存（未命名文档照旧请求选路径），和 `ctrl+s` 一致。
//! 宿主负责：命令分发、“另存为”对话框、把 [`Effect`] 反映到视图（文本、标签的脏标记 / “已删除”），
//! 以及在后台读盘（[`SaveController::due`] → 读盘 → [`SaveController::apply_disk`]）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rove_base::{DocumentId, Error, Result};
use rove_buffer::save::{
    ChangeCoalescer, DiskChange, DiskStamp, SaveJob, SaveOptions, SaveOutcome, SaveRequest,
    SaveResult, SelfWrites, TextFile, UnencodableChoice, WriteOptions,
};
use rove_buffer::{Encoding, Eol};
use rove_config::Settings;
use rove_i18n::Message;

use super::registry::*;
use crate::feedback::{FeedbackAction, FeedbackLevel, FeedbackService, Severity};
use crate::shell::feedback::{FeedbackHub, Reason};

/// 宿主需要做的事。
#[derive(Debug)]
pub enum Effect {
    /// 打开“另存为”对话框；选好路径后调用 [`SaveController::save_as`]。
    PickSaveAsPath(DocumentId),
    /// 已写入磁盘：清除标签的脏标记；#38 的 `BufferStates::saved(path, hash)` 在这里调用。
    Saved {
        document: DocumentId,
        path: PathBuf,
        hash: u64,
        encoding_changed: bool,
    },
    /// 文本被替换（静默重载、还原）：把新快照交给视图（选区已映射），编码 / 换行符变化时刷新状态栏。
    Reloaded {
        document: DocumentId,
        encoding_changed: bool,
        line_endings_changed: bool,
    },
    /// 文件被删除（标签标记“已删除”）或重新出现。
    Deleted { document: DocumentId, deleted: bool },
    /// 可以关闭这个文档了（关闭确认选了“保存”且已保存，或“不保存”）。
    Close(DocumentId),
}

/// 关闭前的检查结果。
#[derive(Debug, PartialEq, Eq)]
pub enum CloseCheck {
    /// 没有未保存的修改（或热退出接管）：直接关闭。
    CloseNow,
    /// 已弹出“保存 / 不保存 / 取消”模态框，等待选择（结果经 [`SaveController::on_action`] 到达）。
    Pending,
}

type HotExit = Box<dyn Fn(&TextFile) -> bool>;

/// 窗口拥有的保存控制器（只在 UI 线程使用）。
pub struct SaveController {
    files: BTreeMap<DocumentId, TextFile>,
    /// 每个文档最近一次**未完成**的保存请求（#140）：失败反馈里那次请求的后续按钮在它上面
    /// 叠加本次选项，这样另存为失败后的“重试 / 覆盖 / 改用 UTF-8 / 替换为 ?”仍然写向另存为的
    /// 目标路径。保存成功、取消、反馈被关掉（[`Self::on_dismissed`]）、还原、关闭（保存 /
    /// 不保存）、[`Self::attach`] 或 [`Self::detach`] 时清除；生命周期不超过它所属的那份
    /// [`TextFile`]。
    pending: BTreeMap<DocumentId, SaveRequest>,
    own: SelfWrites,
    coalescer: ChangeCoalescer,
    write: WriteOptions,
    hot_exit: Option<HotExit>,
}

impl Default for SaveController {
    fn default() -> Self {
        Self::new(WriteOptions::default())
    }
}

impl SaveController {
    pub fn new(write: WriteOptions) -> Self {
        Self {
            files: BTreeMap::new(),
            pending: BTreeMap::new(),
            own: SelfWrites::default(),
            coalescer: ChangeCoalescer::default(),
            write,
            hot_exit: None,
        }
    }

    /// 文件监听的合并参数（测试用更短的安静期）。
    pub fn with_coalescer(mut self, coalescer: ChangeCoalescer) -> Self {
        self.coalescer = coalescer;
        self
    }

    /// #50 的热退出开关点：返回真时关闭不再确认（默认没有，总是确认）。
    pub fn set_hot_exit(&mut self, hot_exit: impl Fn(&TextFile) -> bool + 'static) {
        self.hot_exit = Some(Box::new(hot_exit));
    }

    pub fn attach(&mut self, document: DocumentId, file: TextFile) {
        // 同一个 ID 换一份文档时，上一份遗留的未完成请求不能被新文档继承（#140）。
        self.pending.remove(&document);
        self.files.insert(document, file);
    }

    pub fn detach(&mut self, document: DocumentId) -> Option<TextFile> {
        self.pending.remove(&document);
        self.files.remove(&document)
    }

    pub fn file(&self, document: DocumentId) -> Option<&TextFile> {
        self.files.get(&document)
    }

    pub fn file_mut(&mut self, document: DocumentId) -> Option<&mut TextFile> {
        self.files.get_mut(&document)
    }

    pub fn documents(&self) -> impl Iterator<Item = DocumentId> + '_ {
        self.files.keys().copied()
    }

    pub fn document_for(&self, path: &Path) -> Option<DocumentId> {
        let key = rove_buffer::save::path_key(path);
        self.files.iter().find_map(|(id, file)| {
            file.path()
                .is_some_and(|p| rove_buffer::save::path_key(p) == key)
                .then_some(*id)
        })
    }

    pub fn is_dirty(&self, document: DocumentId) -> bool {
        self.files.get(&document).is_some_and(TextFile::is_dirty)
    }

    /// `workbench.action.files.save` / `saveWithoutFormatting`：同步保存（编码 + 写盘）。
    /// 需要后台写盘的宿主用 [`Self::begin_save`] / [`Self::complete_save`]。
    pub fn save(
        &mut self,
        document: DocumentId,
        request: &SaveRequest,
        options: &SaveOptions,
        feedback: &mut FeedbackHub,
        now: Instant,
    ) -> Vec<Effect> {
        match self.begin_save(document, request, options, feedback, now) {
            Ok(job) => self.complete_save(document, job.run(), feedback, now),
            Err(effects) => effects,
        }
    }

    /// 保存的第 1 步（UI 线程）：变换与编码。返回的任务在后台 [`SaveJob::run`]。
    pub fn begin_save(
        &mut self,
        document: DocumentId,
        request: &SaveRequest,
        options: &SaveOptions,
        feedback: &mut FeedbackHub,
        now: Instant,
    ) -> std::result::Result<SaveJob, Vec<Effect>> {
        let write = self.write.clone();
        // 记下这次请求：失败时的后续按钮要在它上面叠加选项（#140）。
        self.pending.insert(document, request.clone());
        let Some(file) = self.files.get_mut(&document) else {
            self.pending.remove(&document);
            return Err(Vec::new());
        };
        match file.begin_save(request, options, None, &write, now) {
            Ok(job) => Ok(job),
            Err(outcome) => Err(self.report(document, outcome, feedback, now)),
        }
    }

    /// 保存的第 3 步（UI 线程）：应用写入结果、登记自写、给出反馈。
    pub fn complete_save(
        &mut self,
        document: DocumentId,
        result: SaveResult,
        feedback: &mut FeedbackHub,
        now: Instant,
    ) -> Vec<Effect> {
        let Some(file) = self.files.get_mut(&document) else {
            return Vec::new();
        };
        let outcome = file.complete_save(result);
        self.report(document, outcome, feedback, now)
    }

    /// 另存为（对话框给出了路径；未命名文档的首次保存也走这里）。
    pub fn save_as(
        &mut self,
        document: DocumentId,
        path: PathBuf,
        options: &SaveOptions,
        feedback: &mut FeedbackHub,
        now: Instant,
    ) -> Vec<Effect> {
        let request = SaveRequest {
            save_as: Some(path),
            ..SaveRequest::default()
        };
        self.save(document, &request, options, feedback, now)
    }

    /// `saveAll` / `saveFiles` / `saveAllInGroup`：保存给定文档中有修改的那些（未命名文档请求另存为）。
    pub fn save_many(
        &mut self,
        documents: &[DocumentId],
        options: &SaveOptions,
        feedback: &mut FeedbackHub,
        now: Instant,
    ) -> Vec<Effect> {
        let mut effects = Vec::new();
        for &document in documents {
            if self.is_dirty(document) {
                effects.extend(self.save(
                    document,
                    &SaveRequest::default(),
                    options,
                    feedback,
                    now,
                ));
            }
        }
        effects
    }

    /// `workbench.action.files.revert` / `revertLocalChanges`：按磁盘内容重新加载（可撤销）。
    pub fn revert(
        &mut self,
        document: DocumentId,
        settings: &dyn Settings,
        feedback: &mut FeedbackHub,
        now: Instant,
    ) -> Result<Vec<Effect>> {
        let file = self.files.get_mut(&document).ok_or(Error::NotFound)?;
        let change = file.revert(settings, now)?;
        self.pending.remove(&document);
        Self::dismiss_document(feedback, document);
        Ok(Self::disk_effects(document, change))
    }

    /// `workbench.action.editor.changeEOL`：统一为 LF / CRLF（一次撤销；落盘走保存）。
    pub fn change_eol(&mut self, document: DocumentId, eol: Eol, now: Instant) -> Result<bool> {
        let file = self.files.get_mut(&document).ok_or(Error::NotFound)?;
        Ok(file.change_eol(eol, now)?.is_some())
    }

    /// 关闭前检查：有未保存的修改时弹“保存 / 不保存 / 取消”。
    pub fn request_close(
        &mut self,
        document: DocumentId,
        feedback: &mut FeedbackHub,
    ) -> CloseCheck {
        let Some(file) = self.files.get(&document) else {
            return CloseCheck::CloseNow;
        };
        if !file.needs_close_confirmation() || self.hot_exit.as_ref().is_some_and(|h| h(file)) {
            return CloseCheck::CloseNow;
        }
        let message = named("save.modal.close", file);
        let _ = feedback.report(
            key(KEY_CLOSE, document),
            Reason::DataLoss,
            message,
            actions(&[
                (ACTION_SAVE, "save.action.save"),
                (ACTION_DONT_SAVE, "save.action.dontSave"),
                (ACTION_CANCEL, "save.action.cancel"),
            ]),
        );
        CloseCheck::Pending
    }

    /// 反馈按钮（通知、模态、提示条）被点击：执行对应的后续动作。不是本模块的键时返回空。
    pub fn on_action(
        &mut self,
        feedback_key: &str,
        action: &str,
        options: &SaveOptions,
        settings: &dyn Settings,
        feedback: &mut FeedbackHub,
        now: Instant,
    ) -> Vec<Effect> {
        let Some((prefix, document)) = parse_key(feedback_key) else {
            return Vec::new();
        };
        if !self.files.contains_key(&document) {
            return Vec::new();
        }
        let save = |this: &mut Self, request: SaveRequest, feedback: &mut FeedbackHub| {
            this.save(document, &request, options, feedback, now)
        };
        // 那次请求的后续按钮都在最近一次未完成的请求上叠加本次选项：另存为的目标路径不能丢（#140）。
        let base = self.follow_up(document);
        match (prefix, action) {
            (_, ACTION_SAVE_AS) => vec![Effect::PickSaveAsPath(document)],
            (KEY_BUSY, ACTION_RETRY) | (KEY_FAILED, ACTION_RETRY) => save(self, base, feedback),
            (KEY_READ_ONLY, ACTION_OVERWRITE) => save(
                self,
                SaveRequest {
                    clear_read_only: true,
                    ..base
                },
                feedback,
            ),
            (KEY_CONFLICT, ACTION_OVERWRITE) => save(
                self,
                SaveRequest {
                    overwrite_conflict: true,
                    ..base
                },
                feedback,
            ),
            (KEY_CONFLICT, ACTION_DISCARD) | (KEY_EXTERNAL, ACTION_RELOAD) => self
                .revert(document, settings, feedback, now)
                .unwrap_or_default(),
            (KEY_EXTERNAL, ACTION_KEEP) => {
                if let Some(file) = self.files.get_mut(&document) {
                    file.keep_local_changes();
                }
                Vec::new()
            }
            (KEY_UNENCODABLE, ACTION_USE_UTF8) => save(
                self,
                SaveRequest {
                    encoding: Some((Encoding::Utf8, false)),
                    ..base
                },
                feedback,
            ),
            (KEY_UNENCODABLE, ACTION_REPLACE) => save(
                self,
                SaveRequest {
                    unencodable: UnencodableChoice::Replace,
                    ..base
                },
                feedback,
            ),
            // 关闭确认的“保存”是一条新的“保存本文档”命令，不是某次另存为的后续：模态框说的是
            // 当前文档，就得写回当前文档（未命名文档照旧请求选路径）。
            (KEY_CLOSE, ACTION_SAVE) => {
                self.pending.remove(&document);
                let mut effects = save(self, SaveRequest::default(), feedback);
                if effects.iter().any(|e| matches!(e, Effect::Saved { .. })) {
                    effects.push(Effect::Close(document));
                }
                effects
            }
            (KEY_CLOSE, ACTION_DONT_SAVE) => {
                self.pending.remove(&document);
                vec![Effect::Close(document)]
            }
            // 模态框的“取消”：放弃这次保存（文档保持原样、仍未保存），连同记住的请求一起忘掉。
            (KEY_UNENCODABLE, ACTION_CANCEL) => {
                self.pending.remove(&document);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// 保存的反馈被关掉了（用户点了通知 / 模态框自带的关闭按钮，宿主把
    /// `FeedbackEvent::Dismissed` 转给这里）：那次保存就此作罢，记住的请求一并忘掉，
    /// 免得它跨越到下一次保存（#140）。外部修改提示条与关闭确认不带保存请求，忽略。
    pub fn on_dismissed(&mut self, feedback_key: &str) {
        let Some((prefix, document)) = parse_key(feedback_key) else {
            return;
        };
        if matches!(
            prefix,
            KEY_BUSY | KEY_READ_ONLY | KEY_CONFLICT | KEY_FAILED | KEY_UNENCODABLE
        ) {
            self.pending.remove(&document);
        }
    }

    /// 后续按钮的基础请求：最近一次未完成的保存请求（没有时是默认的原地保存）。
    fn follow_up(&self, document: DocumentId) -> SaveRequest {
        self.pending.get(&document).cloned().unwrap_or_default()
    }

    /// 文件监听报告了 `path` 的变化（创建、修改、删除、重命名的两端）：只记下打开中的文档。
    pub fn note_change(&mut self, path: &Path, now: Instant) {
        if self.document_for(path).is_some() {
            self.coalescer.note(path, now);
        }
    }

    /// 下一次需要调用 [`Self::due`] 的时刻。
    pub fn next_deadline(&self) -> Option<Instant> {
        self.coalescer.next_deadline()
    }

    /// 安静期已过的路径（每个只出现一次）。宿主在后台读盘后调用 [`Self::apply_disk`]。
    pub fn due(&mut self, now: Instant) -> Vec<(DocumentId, PathBuf)> {
        self.coalescer
            .due(now)
            .into_iter()
            .filter_map(|path| self.document_for(&path).map(|id| (id, path)))
            .collect()
    }

    /// 读盘结果到达：去掉自己的写入，再交给文档；按结果显示 / 收起提示条。
    pub fn apply_disk(
        &mut self,
        document: DocumentId,
        disk: Option<(DiskStamp, Vec<u8>)>,
        settings: &dyn Settings,
        feedback: &mut FeedbackHub,
        now: Instant,
    ) -> Result<Vec<Effect>> {
        let file = self.files.get_mut(&document).ok_or(Error::NotFound)?;
        if let Some(path) = file.path() {
            if self
                .own
                .is_own(path, disk.as_ref().map(|(stamp, _)| stamp.hash), now)
            {
                return Ok(Vec::new());
            }
        }
        let change = file.disk_changed(disk, settings, now)?;
        match &change {
            DiskChange::NeedsDecision { first } => {
                if *first {
                    let message = named("save.bar.external", file);
                    let _ = feedback.show(crate::feedback::Feedback {
                        key: key(KEY_EXTERNAL, document).into(),
                        level: FeedbackLevel::EditorBar {
                            document,
                            dismissible: false,
                            suppress_key: None,
                        },
                        message,
                        actions: actions(&[
                            (ACTION_RELOAD, "save.action.reload"),
                            (ACTION_KEEP, "save.action.keep"),
                        ]),
                    });
                }
            }
            DiskChange::PromptResolved | DiskChange::Reloaded { .. } => {
                let _ = feedback.dismiss(&key(KEY_EXTERNAL, document));
            }
            _ => {}
        }
        Ok(Self::disk_effects(document, change))
    }

    fn disk_effects(document: DocumentId, change: DiskChange) -> Vec<Effect> {
        match change {
            DiskChange::Reloaded {
                encoding_changed,
                line_endings_changed,
                ..
            } => vec![Effect::Reloaded {
                document,
                encoding_changed,
                line_endings_changed,
            }],
            DiskChange::Deleted { first: true } => vec![Effect::Deleted {
                document,
                deleted: true,
            }],
            DiskChange::Restored => vec![Effect::Deleted {
                document,
                deleted: false,
            }],
            _ => Vec::new(),
        }
    }

    fn dismiss_document(feedback: &mut FeedbackHub, document: DocumentId) {
        for prefix in [
            KEY_EXTERNAL,
            KEY_CONFLICT,
            KEY_BUSY,
            KEY_READ_ONLY,
            KEY_FAILED,
        ] {
            let _ = feedback.dismiss(&key(prefix, document));
        }
    }

    /// 把保存结果映射为反馈与宿主动作。通知说的是**这次写入的目标**（另存为时是新路径）。
    fn report(
        &mut self,
        document: DocumentId,
        outcome: SaveOutcome,
        feedback: &mut FeedbackHub,
        now: Instant,
    ) -> Vec<Effect> {
        let request = self.follow_up(document);
        // 写成了、没得写、还没有路径：这次请求结束了，后续按钮不该再沿用它（#140）。
        if matches!(
            &outcome,
            SaveOutcome::Saved { .. } | SaveOutcome::NotDirty | SaveOutcome::NeedsPath
        ) {
            self.pending.remove(&document);
        }
        let Some(file) = self.files.get(&document) else {
            return Vec::new();
        };
        let name = target_name(file, &request);
        let notify =
            |feedback: &mut FeedbackHub, prefix: &str, text: &str, buttons: &[(&str, &str)]| {
                let _ = feedback.report(
                    key(prefix, document),
                    Reason::Global(Severity::Error),
                    message_for(text, &name),
                    actions(buttons),
                );
            };
        match outcome {
            SaveOutcome::Saved {
                path,
                hash,
                encoding_changed,
                ..
            } => {
                self.own.record(&path, hash, now);
                Self::dismiss_document(feedback, document);
                vec![Effect::Saved {
                    document,
                    path,
                    hash,
                    encoding_changed,
                }]
            }
            SaveOutcome::NotDirty => Vec::new(),
            SaveOutcome::NeedsPath => vec![Effect::PickSaveAsPath(document)],
            SaveOutcome::NeedsEncodingDecision { count, encoding } => {
                let message = Message {
                    key: "save.modal.unencodable".into(),
                    args: Arc::from([
                        (Arc::from("count"), Arc::from(count.to_string())),
                        (Arc::from("encoding"), Arc::from(encoding_label(&encoding))),
                    ]),
                };
                let _ = feedback.report(
                    key(KEY_UNENCODABLE, document),
                    Reason::DataLoss,
                    message,
                    actions(&[
                        (ACTION_USE_UTF8, "save.action.useUtf8"),
                        (ACTION_REPLACE, "save.action.replace"),
                        (ACTION_CANCEL, "save.action.cancel"),
                    ]),
                );
                Vec::new()
            }
            SaveOutcome::Busy { .. } => {
                notify(
                    feedback,
                    KEY_BUSY,
                    "save.notify.busy",
                    &[
                        (ACTION_RETRY, "save.action.retry"),
                        (ACTION_SAVE_AS, "save.action.saveAs"),
                    ],
                );
                Vec::new()
            }
            SaveOutcome::ReadOnly => {
                notify(
                    feedback,
                    KEY_READ_ONLY,
                    "save.notify.readOnly",
                    &[
                        (ACTION_OVERWRITE, "save.action.overwrite"),
                        (ACTION_SAVE_AS, "save.action.saveAs"),
                    ],
                );
                Vec::new()
            }
            SaveOutcome::Conflict => {
                // “比较”在对比视图（P2）完成前隐藏。
                notify(
                    feedback,
                    KEY_CONFLICT,
                    "save.notify.conflict",
                    &[
                        (ACTION_OVERWRITE, "save.action.overwrite"),
                        (ACTION_DISCARD, "save.action.discard"),
                    ],
                );
                Vec::new()
            }
            SaveOutcome::DocumentReadOnly => {
                notify(
                    feedback,
                    KEY_FAILED,
                    "save.notify.documentReadOnly",
                    &[(ACTION_SAVE_AS, "save.action.saveAs")],
                );
                Vec::new()
            }
            SaveOutcome::AccessDenied => {
                notify(
                    feedback,
                    KEY_FAILED,
                    "save.notify.accessDenied",
                    &[(ACTION_SAVE_AS, "save.action.saveAs")],
                );
                Vec::new()
            }
            SaveOutcome::DirectoryMissing => {
                notify(
                    feedback,
                    KEY_FAILED,
                    "save.notify.directoryMissing",
                    &[(ACTION_SAVE_AS, "save.action.saveAs")],
                );
                Vec::new()
            }
            SaveOutcome::Failed(error) => {
                eprintln!("rove-ui save: {error}");
                notify(
                    feedback,
                    KEY_FAILED,
                    "save.notify.failed",
                    &[
                        (ACTION_RETRY, "save.action.retry"),
                        (ACTION_SAVE_AS, "save.action.saveAs"),
                    ],
                );
                Vec::new()
            }
        }
    }
}

fn key(prefix: &str, document: DocumentId) -> String {
    format!("{prefix}{}", document.0)
}

fn parse_key(key: &str) -> Option<(&'static str, DocumentId)> {
    for prefix in [
        KEY_BUSY,
        KEY_READ_ONLY,
        KEY_CONFLICT,
        KEY_FAILED,
        KEY_UNENCODABLE,
        KEY_CLOSE,
        KEY_EXTERNAL,
    ] {
        if let Some(id) = key.strip_prefix(prefix) {
            return id.parse().ok().map(|id| (prefix, DocumentId(id)));
        }
    }
    None
}

fn actions(buttons: &[(&str, &str)]) -> Arc<[FeedbackAction]> {
    buttons
        .iter()
        .map(|(id, label)| FeedbackAction {
            id: Arc::from(*id),
            label: Message::key(*label),
        })
        .collect()
}

/// 文档在界面上的名字：文件名或 `Untitled-1`。
pub fn display_name(file: &TextFile) -> String {
    match (file.path(), file.untitled_name()) {
        (Some(path), _) => file_name(path),
        (None, Some(name)) => name.to_string(),
        (None, None) => String::new(),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// 保存反馈里的名字：另存为说的是目标路径，否则是文档当前的名字（#140）。
fn target_name(file: &TextFile, request: &SaveRequest) -> String {
    match &request.save_as {
        Some(path) => file_name(path),
        None => display_name(file),
    }
}

fn named(text_key: &str, file: &TextFile) -> Message {
    message_for(text_key, &display_name(file))
}

fn message_for(text_key: &str, name: &str) -> Message {
    Message {
        key: text_key.into(),
        args: Arc::from([(Arc::from("name"), Arc::from(name))]),
    }
}

/// 状态栏与模态框里的编码名（与 VS Code 的显示名一致）。
pub fn encoding_label(encoding: &Encoding) -> String {
    match encoding {
        Encoding::Utf8 => "UTF-8".into(),
        Encoding::Utf16Le => "UTF-16 LE".into(),
        Encoding::Utf16Be => "UTF-16 BE".into(),
        Encoding::Gbk => "GBK".into(),
        Encoding::Other(name) => name.to_string(),
    }
}
