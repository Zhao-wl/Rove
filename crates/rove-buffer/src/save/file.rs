//! 可保存的文本文件（#49）：编辑会话 + 磁盘身份 + 编码 / BOM + 脏标记 + 外部修改状态。
//!
//! [`TextFile`] 不读时钟、不弹界面：时间由调用方传入，结果以 [`SaveOutcome`] / [`DiskChange`]
//! 返回，界面按 `UX_SPEC` §6.1 / §6.4 选择反馈级别（rove-ui 的 `save` 模块）。
//!
//! 保存分三步，方便界面把 I/O 放到后台执行器：
//!
//! 1. [`TextFile::begin_save`]（UI 线程，O(n) 编码）：执行保存时的文本变换（与最后一次改动合并成
//!    一个撤销单元）、检查无法编码的字符、编码成磁盘字节，产出可 `Send` 的 [`SaveJob`]；
//! 2. [`SaveJob::run`]（后台线程）：保存冲突检测（磁盘内容是否还是最后一次已知的那份）与原子写入；
//! 3. [`TextFile::complete_save`]（UI 线程）：成功时更新“保存点”、磁盘标记与编码。
//!
//! 第 1 步之后用户继续输入不影响正确性：保存点是第 1 步时的撤销标记，第 3 步之后文档按
//! “当前标记 ≠ 保存点”仍然显示为已修改。[`TextFile::save`] 是三步的同步组合（测试、批量保存）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rove_base::{Error, Line, Result};
use rove_config::Settings;

use super::encode::{self, EncodeError, Unencodable};
use super::settings::{ConflictResolution, SaveOptions};
use super::stamp::DiskStamp;
use super::transforms::{self, InStringOrRegex};
use super::write::{write_atomic_with, WriteError, WriteOptions, Written};
use crate::contracts::{Encoding, Eol, LineEndings, ReadOnlyReason, TextSource};
use crate::edit::{AppliedEdit, EditBuffer, EditOptions, EditSession};
use crate::open::{open_bytes, LoadOptions, LoadedText};
use crate::settings::{encoding_from_id, metadata_encoding, FILES_ENCODING};

/// 无法编码的字符怎么处理（保存前模态框的三个选项里的两个；“取消”就是不再保存）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnencodableChoice {
    /// 有无法编码的字符时不写入，返回 [`SaveOutcome::NeedsEncodingDecision`]（弹模态框）。
    #[default]
    Ask,
    /// “仍然保存（替换为 ?）”。
    Replace,
}

/// 一次保存的参数。
#[derive(Clone, Debug, Default)]
pub struct SaveRequest {
    pub unencodable: UnencodableChoice,
    /// 改用另一种编码保存：“改用 UTF-8 保存”是 `Some((Utf8, false))`，状态栏的“通过编码保存”
    /// 也走这里。保存成功后文档的编码随之改变。
    pub encoding: Option<(Encoding, bool)>,
    /// 保存冲突时覆盖磁盘上的版本（通知里的“覆盖”、`acceptLocalChanges`）。
    pub overwrite_conflict: bool,
    /// 目标有只读属性时先清除（通知里的“覆盖”）。
    pub clear_read_only: bool,
    /// 另存为 / 未命名文档首次保存的目标路径（界面的“另存为”对话框给出）。
    pub save_as: Option<PathBuf>,
    /// 文档未修改时也写入（另存为总是写入）。
    pub force: bool,
    /// 去除行尾空白时跳过的行（自动保存时光标所在的行，VS Code 相同）。
    pub keep_lines: Vec<Line>,
}

/// 保存的结果。界面据此选择反馈（`UX_SPEC` §6.4）。
#[derive(Clone, Debug, PartialEq)]
pub enum SaveOutcome {
    /// 已写入。`hash` 是磁盘内容哈希（#38 的 `BufferStates::saved` 用它），`attempts` 含自动重试。
    Saved {
        path: PathBuf,
        hash: u64,
        attempts: u32,
        encoding_changed: bool,
    },
    /// 没有未保存的修改，什么都没做。
    NotDirty,
    /// 未命名文档：需要先选择路径（打开“另存为”）。
    NeedsPath,
    /// 有 `count` 个字符无法用 `encoding` 保存：弹模态框（改用 UTF-8 / 仍然保存 / 取消）。
    NeedsEncodingDecision { count: usize, encoding: Encoding },
    /// 文档本身只读（只读规则、超大文件……），不能原地保存（可以另存为）。
    DocumentReadOnly,
    /// 磁盘上的文件在上次读取 / 保存之后被别人改过：通知（比较 / 覆盖 / 放弃本地修改）。
    Conflict,
    /// 文件有只读属性：通知（覆盖 / 另存为）。不提供提权写入。
    ReadOnly,
    /// 自动重试后仍被其他程序占用：通知（重试 / 另存为）。
    Busy { attempts: u32 },
    /// 没有写入权限。
    AccessDenied,
    /// 目标目录不存在。
    DirectoryMissing,
    /// 其他失败（磁盘已满、编码不受支持……）。
    Failed(Error),
}

/// [`TextFile::begin_save`] 产出的后台写入任务。
#[derive(Clone)]
pub struct SaveJob {
    path: PathBuf,
    bytes: Arc<[u8]>,
    /// 冲突检测的基准：`None` 表示不检测（新文件、另存为、选择了覆盖）。
    expected: Option<DiskStamp>,
    write: WriteOptions,
    marker: u64,
    encoding: Encoding,
    bom: bool,
}

/// 后台写入的结果，交回 [`TextFile::complete_save`]。
pub struct SaveResult {
    job: SaveJob,
    result: std::result::Result<Written, SaveFailure>,
}

#[derive(Clone, Debug)]
enum SaveFailure {
    Conflict,
    Write(WriteError),
}

impl From<WriteError> for SaveFailure {
    fn from(error: WriteError) -> Self {
        SaveFailure::Write(error)
    }
}

impl SaveJob {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 将写入的字节（测试与 #50 热退出备份用）。
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 冲突检测 + 原子写入。在后台线程调用。
    ///
    /// 冲突检测在**每次**替换目标之前重做（#139）：目标被占用时写入会等几百毫秒再重试，期间占用
    /// 文件的程序（Unity 导入、Git 检出）可能写入新内容并释放，只检测一次会把这份新内容悄悄覆盖掉。
    /// 检测与替换之间仍有很短的竞争窗口，见 [`write`](super::write) 的模块文档。
    pub fn run(self) -> SaveResult {
        let result = write_atomic_with(&self.path, &self.bytes, &self.write, &|| {
            self.check_conflict()
        });
        SaveResult { job: self, result }
    }

    fn check_conflict(&self) -> std::result::Result<(), SaveFailure> {
        let Some(expected) = &self.expected else {
            return Ok(());
        };
        let metadata = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            // 文件已被删除：保存视为新建写入。
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(SaveFailure::Write(WriteError::Io(error.into()))),
        };
        if expected.same_metadata(&metadata) {
            return Ok(());
        }
        match DiskStamp::read(&self.path) {
            Ok(Some((stamp, _))) if stamp.hash != expected.hash => Err(SaveFailure::Conflict),
            Ok(_) => Ok(()),
            Err(error) => Err(SaveFailure::Write(WriteError::Io(error.into()))),
        }
    }
}

/// 外部修改的处理结果（`FEATURES.md` §6、`UX_SPEC` §6.1）。
#[derive(Debug)]
pub enum DiskChange {
    /// 磁盘内容与文档最后一次已知的磁盘内容相同（自己的写入、或外部写入了同样的内容）。
    Unchanged,
    /// 本地没有修改：已静默重新加载。`edit` 是对文本的改动（交给语法与视图；选区已映射）；
    /// 编码或换行符重新识别后变化时对应标记为真（更新状态栏）。
    Reloaded {
        edit: Option<Box<AppliedEdit>>,
        encoding_changed: bool,
        line_endings_changed: bool,
    },
    /// 本地有修改：显示编辑器顶部提示条（重新加载 / 保留本地修改）。`first` 为假表示提示条
    /// 已经在显示，不要再弹（外部修改风暴不堆积提示）。
    NeedsDecision { first: bool },
    /// 之前提示的外部修改已经不存在（磁盘又变回了已知内容）：收起提示条。
    PromptResolved,
    /// 文件被删除：标签标记“已删除”，内容保留，下次保存视为新建写入。`first` 同上。
    Deleted { first: bool },
    /// 被删除的文件又出现了，内容与已知的相同。
    Restored,
}

/// 可保存的文本文件。
pub struct TextFile {
    path: Option<PathBuf>,
    untitled: Option<Arc<str>>,
    language: Arc<str>,
    session: EditSession,
    encoding: Encoding,
    bom: bool,
    saved_marker: u64,
    disk: Option<DiskStamp>,
    deleted: bool,
    external: Option<DiskStamp>,
}

impl TextFile {
    /// 从磁盘读取并打开（编码识别同 #7 的打开路径）。`read_only` 是 #15 判定的只读原因加上
    /// 会话只读等；超大文件不能编辑，返回 [`Error::Unsupported`]（由 #8 的只读路径打开）。
    pub fn load(
        path: &Path,
        language: &str,
        settings: &dyn Settings,
        read_only: Arc<[ReadOnlyReason]>,
        edit_options: EditOptions,
    ) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let stamp = DiskStamp::from_bytes(path, &bytes);
        let mut options = LoadOptions::new(language, settings);
        options.read_only = read_only;
        let loaded = open_bytes(&bytes, &options)?;
        Self::from_loaded(path, &loaded, stamp, edit_options)
    }

    /// 由已经加载好的文本建立（共享 rope）。`disk` 是加载时读到的字节的标记。
    pub fn from_loaded(
        path: &Path,
        loaded: &LoadedText,
        disk: DiskStamp,
        edit_options: EditOptions,
    ) -> Result<Self> {
        let buffer = EditBuffer::from_loaded(loaded)?;
        Ok(Self {
            path: Some(path.to_path_buf()),
            untitled: None,
            language: loaded.metadata.language.clone(),
            session: EditSession::new(buffer, edit_options),
            encoding: loaded.metadata.encoding.clone(),
            bom: loaded.metadata.bom,
            saved_marker: 0,
            disk: Some(disk),
            deleted: false,
            external: None,
        })
    }

    /// 新建只在内存中的未命名文档（`Untitled-1` 等）：编码取 `files.encoding`，换行符取
    /// `files.eol`（按 `language` 的语言级覆盖）。首次保存需要路径（[`SaveOutcome::NeedsPath`]）。
    pub fn untitled(
        name: &str,
        language: &str,
        settings: &dyn Settings,
        edit_options: EditOptions,
    ) -> Self {
        let scope = rove_config::Scope {
            language: (!language.is_empty()).then(|| Arc::<str>::from(language)),
        };
        let id = rove_config::Key::<String>::new(FILES_ENCODING)
            .read(settings, &scope)
            .unwrap_or_else(|_| "utf8".into());
        let (encoding, bom) = new_file_encoding(&id);
        let eol = SaveOptions::read(settings, language).new_file_eol();
        let buffer = EditBuffer::new("", LineEndings::Uniform(eol));
        Self {
            path: None,
            untitled: Some(Arc::from(name)),
            language: Arc::from(language),
            session: EditSession::new(buffer, edit_options),
            encoding,
            bom,
            saved_marker: 0,
            disk: None,
            deleted: false,
            external: None,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 未命名文档的名字（`Untitled-1`）；有路径的文档为 `None`。
    pub fn untitled_name(&self) -> Option<&str> {
        self.untitled.as_deref()
    }

    pub fn is_untitled(&self) -> bool {
        self.path.is_none()
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn encoding(&self) -> &Encoding {
        &self.encoding
    }

    pub fn bom(&self) -> bool {
        self.bom
    }

    pub fn line_endings(&self) -> &LineEndings {
        self.session.buffer().line_endings()
    }

    pub fn session(&self) -> &EditSession {
        &self.session
    }

    /// 编辑入口（R46 / R47 / R51 改文本都经过它）。
    pub fn session_mut(&mut self) -> &mut EditSession {
        &mut self.session
    }

    pub fn snapshot(&self) -> Arc<crate::RopeText> {
        self.session.snapshot()
    }

    /// 是否有未保存的修改：撤销标记与保存点不同。未命名文档有内容或被编辑过即为已修改。
    pub fn is_dirty(&self) -> bool {
        self.session.history_marker() != self.saved_marker
            || (self.is_untitled() && self.session.buffer().text().byte_len() > 0)
    }

    /// 磁盘上的文件已被删除（标签显示“已删除”）。
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    /// 有外部修改正等待用户决定（提示条在显示）。
    pub fn has_pending_external_change(&self) -> bool {
        self.external.is_some()
    }

    /// 最后一次已知的磁盘标记（读取或保存时）。
    pub fn disk_stamp(&self) -> Option<&DiskStamp> {
        self.disk.as_ref()
    }

    /// 关闭前是否需要“保存 / 不保存 / 取消”的确认。`files.hotExit` 生效时由 #50 跳过这个确认，
    /// 本 issue 默认总是确认。
    pub fn needs_close_confirmation(&self) -> bool {
        self.is_dirty()
    }

    /// `workbench.action.editor.changeEOL`：统一为 LF / CRLF，一次撤销（落盘走保存）。
    pub fn change_eol(&mut self, eol: Eol, now: Instant) -> Result<Option<AppliedEdit>> {
        self.session.change_eol(eol, now)
    }

    /// 第 1 步：变换、编码，产出写入任务。失败（或不需要写）时返回对应的 [`SaveOutcome`]。
    pub fn begin_save(
        &mut self,
        request: &SaveRequest,
        options: &SaveOptions,
        protect: Option<InStringOrRegex<'_>>,
        write: &WriteOptions,
        now: Instant,
    ) -> std::result::Result<SaveJob, SaveOutcome> {
        let target = match (&request.save_as, &self.path) {
            (Some(target), _) => target.clone(),
            (None, Some(path)) => path.clone(),
            (None, None) => return Err(SaveOutcome::NeedsPath),
        };
        let in_place = request.save_as.is_none();
        if in_place && self.blocks_in_place_save() {
            return Err(SaveOutcome::DocumentReadOnly);
        }
        if in_place && !request.force && !self.deleted && !self.is_dirty() {
            return Err(SaveOutcome::NotDirty);
        }
        if self.is_dirty() {
            self.apply_save_transforms(request, options, protect, now)
                .map_err(SaveOutcome::Failed)?;
        }
        let (encoding, bom) = request
            .encoding
            .clone()
            .unwrap_or_else(|| (self.encoding.clone(), self.bom));
        let policy = match request.unencodable {
            UnencodableChoice::Ask => Unencodable::Reject,
            UnencodableChoice::Replace => Unencodable::Replace,
        };
        let snapshot = self.session.snapshot();
        let bytes = match encode::encode(snapshot.as_ref(), &encoding, bom, policy) {
            Ok(bytes) => bytes,
            Err(EncodeError::Unencodable { count, .. }) => {
                return Err(SaveOutcome::NeedsEncodingDecision { count, encoding })
            }
            Err(EncodeError::Other(error)) => return Err(SaveOutcome::Failed(error)),
        };
        // 保存点之后的输入开始新的撤销单元。
        self.session.push_undo_stop();
        let check = in_place
            && !request.overwrite_conflict
            && options.conflict_resolution == ConflictResolution::AskUser;
        let mut write = write.clone();
        write.clear_read_only |= request.clear_read_only;
        Ok(SaveJob {
            path: target,
            bytes: Arc::from(bytes),
            expected: if check { self.disk.clone() } else { None },
            write,
            marker: self.session.history_marker(),
            encoding,
            bom,
        })
    }

    /// 第 3 步：应用后台写入的结果。
    pub fn complete_save(&mut self, result: SaveResult) -> SaveOutcome {
        let SaveResult { job, result } = result;
        match result {
            Ok(written) => {
                let encoding_changed = job.encoding != self.encoding || job.bom != self.bom;
                self.path = Some(job.path.clone());
                self.untitled = None;
                self.saved_marker = job.marker;
                self.disk = Some(written.stamp.clone());
                self.encoding = job.encoding;
                self.bom = job.bom;
                self.deleted = false;
                self.external = None;
                SaveOutcome::Saved {
                    path: job.path,
                    hash: written.stamp.hash,
                    attempts: written.attempts,
                    encoding_changed,
                }
            }
            Err(SaveFailure::Conflict) => SaveOutcome::Conflict,
            Err(SaveFailure::Write(error)) => match error {
                WriteError::Busy { attempts } => SaveOutcome::Busy { attempts },
                WriteError::ReadOnly => SaveOutcome::ReadOnly,
                WriteError::AccessDenied => SaveOutcome::AccessDenied,
                WriteError::DirectoryMissing => SaveOutcome::DirectoryMissing,
                WriteError::Io(error) => SaveOutcome::Failed(error),
            },
        }
    }

    /// 三步的同步组合。
    pub fn save(
        &mut self,
        request: &SaveRequest,
        options: &SaveOptions,
        protect: Option<InStringOrRegex<'_>>,
        write: &WriteOptions,
        now: Instant,
    ) -> SaveOutcome {
        match self.begin_save(request, options, protect, write, now) {
            Ok(job) => self.complete_save(job.run()),
            Err(outcome) => outcome,
        }
    }

    /// 文件监听报告 `path` 变化后调用（已按路径合并、已排除自己的写入）。`disk` 是当前磁盘内容，
    /// `None` 表示文件不存在。在 UI 线程调用；读盘由调用方在后台完成。
    pub fn disk_changed(
        &mut self,
        disk: Option<(DiskStamp, Vec<u8>)>,
        settings: &dyn Settings,
        now: Instant,
    ) -> Result<DiskChange> {
        if self.is_untitled() {
            return Ok(DiskChange::Unchanged);
        }
        let Some((stamp, bytes)) = disk else {
            let first = !self.deleted;
            self.deleted = true;
            self.external = None;
            return Ok(DiskChange::Deleted { first });
        };
        let known = self
            .disk
            .as_ref()
            .is_some_and(|disk| disk.hash == stamp.hash);
        if known {
            self.disk = Some(stamp);
            if std::mem::take(&mut self.deleted) {
                return Ok(DiskChange::Restored);
            }
            if self.external.take().is_some() {
                return Ok(DiskChange::PromptResolved);
            }
            return Ok(DiskChange::Unchanged);
        }
        self.deleted = false;
        if !self.is_dirty() {
            return self.reload(stamp, &bytes, settings, now);
        }
        let first = self.external.is_none();
        self.external = Some(stamp);
        Ok(DiskChange::NeedsDecision { first })
    }

    /// 提示条“重新加载（丢弃本地修改）”与 `workbench.action.files.revert` /
    /// `revertLocalChanges`：按磁盘内容重新加载，丢弃本地修改（可撤销）。
    pub fn revert(&mut self, settings: &dyn Settings, now: Instant) -> Result<DiskChange> {
        let path = self.path.clone().ok_or(Error::Unsupported)?;
        let (stamp, bytes) = DiskStamp::read(&path)?.ok_or(Error::NotFound)?;
        self.reload(stamp, &bytes, settings, now)
    }

    /// 提示条“保留本地修改”：以这次外部修改为新的基准，之后保存直接写入、不再报保存冲突。
    pub fn keep_local_changes(&mut self) {
        if let Some(stamp) = self.external.take() {
            self.disk = Some(stamp);
        }
    }

    fn reload(
        &mut self,
        stamp: DiskStamp,
        bytes: &[u8],
        settings: &dyn Settings,
        now: Instant,
    ) -> Result<DiskChange> {
        let mut options = LoadOptions::new(&self.language, settings);
        options.read_only = Arc::from(self.session.buffer().read_only());
        options.force_open = true;
        let loaded = open_bytes(bytes, &options)?;
        let line_endings_before = self.line_endings().clone();
        let mut text = String::with_capacity(loaded.text.byte_len() as usize);
        let whole = rove_base::Span::new(
            rove_base::ByteOffset(0),
            rove_base::ByteOffset(loaded.text.byte_len()),
        )?;
        loaded
            .text
            .visit_chunks(whole, &mut |chunk| text.push_str(chunk))?;
        let edit =
            self.session
                .replace_from_disk(&text, loaded.metadata.line_endings.clone(), now)?;
        let encoding_changed =
            loaded.metadata.encoding != self.encoding || loaded.metadata.bom != self.bom;
        self.encoding = loaded.metadata.encoding;
        self.bom = loaded.metadata.bom;
        self.session.push_undo_stop();
        self.saved_marker = self.session.history_marker();
        self.disk = Some(stamp);
        self.deleted = false;
        self.external = None;
        Ok(DiskChange::Reloaded {
            edit: edit.map(Box::new),
            encoding_changed,
            line_endings_changed: !same_line_endings(&line_endings_before, self.line_endings()),
        })
    }

    fn blocks_in_place_save(&self) -> bool {
        self.session
            .buffer()
            .read_only()
            .iter()
            .any(|reason| !matches!(reason, ReadOnlyReason::Session))
    }

    fn apply_save_transforms(
        &mut self,
        request: &SaveRequest,
        options: &SaveOptions,
        protect: Option<InStringOrRegex<'_>>,
        now: Instant,
    ) -> Result<()> {
        if !self.session.buffer().read_only().is_empty() {
            return Ok(());
        }
        let protect = if options.trim_trailing_whitespace_in_regex_and_strings {
            None
        } else {
            protect
        };
        if options.trim_trailing_whitespace {
            let snapshot = self.session.snapshot();
            if let Some(t) = transforms::trim_trailing_whitespace(
                snapshot.as_ref(),
                protect,
                &request.keep_lines,
            )? {
                self.session.edit_joined(t, now)?;
            }
        }
        if options.trim_final_newlines {
            let snapshot = self.session.snapshot();
            if let Some(t) = transforms::trim_final_newlines(snapshot.as_ref())? {
                self.session.edit_joined(t, now)?;
            }
        }
        if options.insert_final_newline {
            let snapshot = self.session.snapshot();
            let eol = self.session.buffer().eol();
            if let Some(t) = transforms::insert_final_newline(snapshot.as_ref(), eol)? {
                self.session.edit_joined(t, now)?;
            }
        }
        Ok(())
    }
}

/// `files.encoding` 的 id → 新文件的编码与 BOM（`utf8bom` 带 BOM；UTF-16 总是带 BOM，VS Code 相同）。
pub fn new_file_encoding(id: &str) -> (Encoding, bool) {
    let lower = id.trim().to_ascii_lowercase();
    let encoding = encoding_from_id(&lower)
        .map(metadata_encoding)
        .unwrap_or(Encoding::Utf8);
    let bom = lower == "utf8bom" || matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be);
    (encoding, bom)
}

fn same_line_endings(a: &LineEndings, b: &LineEndings) -> bool {
    match (a, b) {
        (LineEndings::Uniform(x), LineEndings::Uniform(y)) => x == y,
        (LineEndings::Mixed { .. }, LineEndings::Mixed { .. }) => true,
        _ => false,
    }
}
