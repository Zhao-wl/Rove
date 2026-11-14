//! #47：可编辑代码视图的窗口装配，以及 #49 留下的保存装配。
//!
//! - 每个打开的、能编辑的文档（工作区 / 外部文件，非超大文件）在后台建一个 #49 的 `TextFile`，交给
//!   窗口唯一的 [`SaveController`]；显示它的代码视图拿到一个借出该文件会话的 [`EditHost`]。
//! - 代码视图发出 `Edited`：新快照换进打开文档服务的所有标签（文档身份不变）与 `FileDocuments` 缓存，
//!   同一文档的其他视图按事务增量更新，#94 按合成的增量编辑在后台重新着色，#38 的
//!   `BufferStates::edited` 记下编辑（C# 导航在未保存修改下正确映射），标签脏标记跟随保存点。
//! - 保存命令走 [`SaveController`]（变换与编码在 UI 线程，写盘在后台）；`Effect` 反映到标签（脏点、
//!   “已删除”）、视图（重载后的新快照，滚动位置不变）与 `BufferStates::saved`。
//! - 工作区文件事件 → `note_change` → 安静期后后台读盘 → `apply_disk`（静默重载 / 提示条 / 已删除）。
//! - 关闭有未保存修改的标签或窗口：#49 的“保存 / 不保存 / 取消”模态框。

use super::*;
use crate::code_view::edit::EditHost;
use crate::save::registry as save_ids;
use crate::save::{CloseCheck, Effect, SaveController};
use rove_buffer::edit::{EditOptions, EditSession};
use rove_buffer::save::{DiskStamp, SaveOptions, SaveRequest, TextFile};
use rove_buffer::{DocumentMetadata, SizeClass, TextSource};
use std::cell::{RefCell, RefMut};
use std::rc::Rc;

/// 窗口的编辑与保存状态。
#[derive(Default)]
pub(super) struct Editing {
    pub(super) saves: Rc<RefCell<SaveController>>,
    /// 正在后台建 `TextFile` 的文档。
    loading: BTreeSet<DocumentId>,
    /// 磁盘上已被删除的文档（标签显示“已删除”）。
    pub(super) deleted: BTreeSet<DocumentId>,
    untitled: u64,
    /// 已经排了一个“安静期结束后读盘”的定时器。
    disk_timer: bool,
    /// 窗口关闭按钮触发了逐个确认。
    closing_window: bool,
    /// 本窗口（全部确认完后由它关闭；#149 的「重启以更新」也用它收尾）。
    pub(super) window: Option<gpui::AnyWindowHandle>,
    /// 打开服务的具体加载器（编辑后的新快照要回填它的缓存）。
    pub(super) files: Option<Arc<FileDocuments>>,
}

impl Editing {
    pub(super) fn new(files: Arc<FileDocuments>) -> Self {
        Self {
            files: Some(files),
            ..Default::default()
        }
    }
}

/// 借出某个文档的编辑会话（同一文档的所有视图共用）。
struct FileSession {
    saves: Rc<RefCell<SaveController>>,
    document: DocumentId,
}

impl EditHost for FileSession {
    fn session(&self) -> Option<RefMut<'_, EditSession>> {
        let controller = self.saves.try_borrow_mut().ok()?;
        RefMut::filter_map(controller, |c| {
            c.file_mut(self.document).map(TextFile::session_mut)
        })
        .ok()
    }
}

/// 未命名文档的身份从高位开始，不与加载器的计数冲突。
const UNTITLED_BASE: u64 = 1 << 62;

impl Workbench {
    // ------------------------------------------------------------- 文档 ↔ 可保存文件

    pub(super) fn edit_options(&self, language: &str) -> EditOptions {
        let scope = Scope {
            language: (!language.is_empty()).then(|| Arc::from(language)),
        };
        EditOptions::read(&*self.settings, &scope)
    }

    fn save_options(&self, document: DocumentId) -> SaveOptions {
        let language = self
            .editing
            .saves
            .borrow()
            .file(document)
            .map(|f| f.language().to_string())
            .unwrap_or_default();
        SaveOptions::read(&*self.settings, &language)
    }

    /// 打开中的某个文档（任一标签上的）。
    pub(super) fn open_document(&self, document: DocumentId) -> Option<Arc<Document>> {
        self.editor
            .groups
            .values()
            .flat_map(|g| g.tabs.iter())
            .filter_map(Tab::opened)
            .find(|o| o.document.id == document)
            .map(|o| o.document.clone())
    }

    pub(super) fn disk_path(&self, resource: &Resource) -> Option<PathBuf> {
        match resource {
            Resource::Workspace(p) => Some(self.root.join(p.as_str())),
            Resource::File(p) => Some(p.clone()),
            Resource::Virtual(_) => None,
        }
    }

    /// 每帧：给新打开的文档建 `TextFile`，给代码视图装上 / 撤下编辑入口，关闭了的文档从控制器摘掉。
    pub(super) fn sync_editing(&mut self, cx: &mut Context<Self>) {
        let open: Vec<Arc<Document>> = {
            let mut seen = BTreeSet::new();
            self.editor
                .groups
                .values()
                .flat_map(|g| g.tabs.iter())
                .filter(|t| t.kind == EditorKind::Text)
                .filter_map(Tab::opened)
                .filter(|o| seen.insert(o.document.id))
                .map(|o| o.document.clone())
                .collect()
        };
        let ids: BTreeSet<DocumentId> = open.iter().map(|d| d.id).collect();
        let attached: Vec<DocumentId> = self.editing.saves.borrow().documents().collect();
        for id in attached {
            if !ids.contains(&id) {
                self.editing.saves.borrow_mut().detach(id);
                self.editing.deleted.remove(&id);
                // #50：标签关掉了，它的热退出备份也不再需要。
                self.discard_hot_exit_backup(id);
            }
        }
        // #50：打开失败的标签不能把启动恢复卡住。
        self.finish_restore_if_done(cx);
        for document in open {
            let attached = self.editing.saves.borrow().file(document.id).is_some();
            if attached || self.editing.loading.contains(&document.id) {
                continue;
            }
            if document.metadata.size == SizeClass::Huge {
                continue;
            }
            if let Some(path) = self.disk_path(&document.resource) {
                self.load_text_file(document, path, cx);
            }
        }
        // 编辑入口：显示的文档有可保存文件时装上，否则撤下。
        let saves = self.editing.saves.clone();
        for views in self.views.values_mut() {
            for slot in std::iter::once(&mut views.primary).chain(views.secondary.as_mut()) {
                let want = slot
                    .shown
                    .map(|s| s.document)
                    .filter(|d| saves.borrow().file(*d).is_some());
                if slot.edit_doc != want {
                    slot.edit_doc = want;
                    let host: Option<Rc<dyn EditHost>> = want.map(|document| {
                        Rc::new(FileSession {
                            saves: saves.clone(),
                            document,
                        }) as Rc<dyn EditHost>
                    });
                    slot.code
                        .update(cx, |code, cx| code.set_edit_host(host, cx));
                }
            }
        }
    }

    /// 后台读盘建 `TextFile`（编码识别同打开路径），回到窗口后挂到控制器，并把文件的快照换进文档。
    fn load_text_file(&mut self, document: Arc<Document>, path: PathBuf, cx: &mut Context<Self>) {
        // #50：这个路径有待恢复的热退出备份时不读盘，内容直接取自备份（标记为脏）。
        if let Some(file) = self.take_restored_file(&path) {
            self.attach_text_file(document, file, cx);
            self.finish_restore_if_done(cx);
            return;
        }
        self.editing.loading.insert(document.id);
        let settings = self.settings.clone();
        let language = document.metadata.language.to_string();
        let options = self.edit_options(&language);
        let task = cx.background_executor().spawn(async move {
            TextFile::load(&path, &language, &*settings, Arc::from([]), options)
        });
        let id = document.id;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                view.editing.loading.remove(&id);
                let Ok(file) = result else { return };
                let Some(current) = view.open_document(id) else {
                    return;
                };
                view.attach_text_file(current, file, cx);
            });
        })
        .detach();
    }

    fn attach_text_file(
        &mut self,
        document: Arc<Document>,
        mut file: TextFile,
        cx: &mut Context<Self>,
    ) {
        let reasons = self.read_only_of(&document);
        file.session_mut().set_read_only(reasons);
        let snapshot: Arc<dyn TextSource> = file.snapshot();
        let metadata = DocumentMetadata {
            line_endings: file.line_endings().clone(),
            encoding: file.encoding().clone(),
            bom: file.bom(),
            ..document.metadata.clone()
        };
        self.editing.saves.borrow_mut().attach(document.id, file);
        // 视图显示的必须是会话的快照（内容相同，版本不同）：换进文档，视图在同步时保持视图状态。
        let replaced = Arc::new(Document {
            id: document.id,
            resource: document.resource.clone(),
            metadata,
            text: snapshot,
        });
        self.editor.rebase_document(replaced.clone());
        if let Some(files) = &self.editing.files {
            files.replace(&replaced);
        }
        // #50：从备份恢复出来的文档带着未保存的修改，标签上的脏点要跟上。
        let dirty = self.editing.saves.borrow().is_dirty(document.id);
        self.editor.set_dirty(document.id, dirty);
        self.sync_views(cx);
        self.sync_editing(cx);
        cx.notify();
    }

    /// #27 的「通过编码重新打开」：#49 的编辑会话也要按新编码重载，否则会话仍持有旧编码的快照，
    /// 第一次编辑时 `run_edit` 会把旧快照换回视图，保存又按旧编码写回，覆盖掉原始字节。
    ///
    /// 返回换上会话快照的文档（视图显示的必须就是会话的快照）；没有编辑会话（超大文件、
    /// 虚拟文档、还在后台加载）时原样返回传入的文档。
    pub(super) fn reopen_edit_session(
        &mut self,
        document: Arc<Document>,
        encoding: &str,
    ) -> Arc<Document> {
        let saves = self.editing.saves.clone();
        let settings = self.settings.clone();
        let mut controller = saves.borrow_mut();
        let Some(file) = controller.file_mut(document.id) else {
            return document;
        };
        if let Err(error) = file.reopen_with_encoding(encoding, &*settings, Instant::now()) {
            // 会话重载不了：撤下它，`sync_editing` 下一帧按磁盘重新挂一个，别留着旧编码的会话。
            eprintln!("reopenWithEncoding session {encoding}: {error}");
            drop(controller);
            saves.borrow_mut().detach(document.id);
            return document;
        }
        let metadata = DocumentMetadata {
            line_endings: file.line_endings().clone(),
            encoding: file.encoding().clone(),
            bom: file.bom(),
            ..document.metadata.clone()
        };
        let text: Arc<dyn TextSource> = file.snapshot();
        Arc::new(Document {
            id: document.id,
            resource: document.resource.clone(),
            metadata,
            text,
        })
    }

    /// 只读状态变了（会话切换等）：会话的只读原因跟着变。
    pub(super) fn refresh_edit_read_only(&mut self) {
        let docs: Vec<DocumentId> = self.editing.saves.borrow().documents().collect();
        for id in docs {
            let Some(document) = self.open_document(id) else {
                continue;
            };
            let reasons = self.read_only_of(&document);
            if let Some(file) = self.editing.saves.borrow_mut().file_mut(id) {
                file.session_mut().set_read_only(reasons);
            }
        }
    }

    // ------------------------------------------------------------- 编辑事件

    /// 代码视图里的一次编辑：同步文档、其他视图、语法、`BufferStates` 与脏标记。
    pub(super) fn text_edited(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        notice: &crate::code_view::EditNotice,
        cx: &mut Context<Self>,
    ) {
        let Some(origin) = self.group_code(group, secondary).cloned() else {
            return;
        };
        let Some(document) = self.shown_document(group, secondary) else {
            return;
        };
        let edited = &notice.0;
        let line_endings = self
            .editing
            .saves
            .borrow()
            .file(document.id)
            .map(|f| f.line_endings().clone())
            .unwrap_or_else(|| document.metadata.line_endings.clone());
        let replaced = Arc::new(Document {
            id: document.id,
            resource: document.resource.clone(),
            metadata: DocumentMetadata {
                line_endings,
                ..document.metadata.clone()
            },
            text: edited.after.clone(),
        });
        self.editor
            .replace_document(replaced.clone(), edited.combined.clone());
        if let Some(files) = &self.editing.files {
            files.replace(&replaced);
        }
        let version = edited.after.version();
        let mut others = Vec::new();
        for views in self.views.values_mut() {
            for slot in std::iter::once(&mut views.primary).chain(views.secondary.as_mut()) {
                if let Some(shown) = slot.shown.as_mut().filter(|s| s.document == document.id) {
                    shown.version = version;
                    if slot.code != origin {
                        others.push(slot.code.clone());
                    }
                }
            }
        }
        for code in others {
            let (after, edits) = (edited.after.clone(), edited.edits.clone());
            code.update(cx, |code, cx| {
                if edits.is_empty() {
                    code.set_text(Some(after), true, cx);
                } else {
                    code.apply_edits(after, &edits, cx);
                }
            });
        }
        // #38：未保存修改下的位置映射（只登记过的 C# 文档会成功）。
        if let (Resource::Workspace(path), Some(edit)) = (&document.resource, &edited.combined) {
            let states = self.nav.router.buffers();
            if states
                .edited(path, &*edited.before, edit, edited.after.clone())
                .is_ok()
            {
                self.nav.buffers.note_edited(path, document.id, version);
            }
        }
        let dirty = self.editing.saves.borrow().is_dirty(document.id);
        self.editor.set_dirty(document.id, dirty);
        // #50：持续备份按编辑去抖（不是只在退出时备份）。
        self.note_hot_exit_edit(document.id, cx);
        cx.notify();
    }

    /// 会话的快照领先于文档（保存时的变换、重载、还原）：换进文档与所有显示它的视图，保持视图状态。
    fn sync_document_from_session(&mut self, document: DocumentId, cx: &mut Context<Self>) {
        let Some(current) = self.open_document(document) else {
            return;
        };
        let (snapshot, line_endings, selection) = {
            let saves = self.editing.saves.borrow();
            let Some(file) = saves.file(document) else {
                return;
            };
            let snapshot = file.snapshot();
            let primary = *file.session().selections().primary();
            let selection = snapshot
                .position(primary.anchor)
                .ok()
                .zip(snapshot.position(primary.active).ok())
                .map(|(a, b)| Selection::new(a, b));
            (snapshot, file.line_endings().clone(), selection)
        };
        if snapshot.version() == current.text.version() {
            return;
        }
        let text: Arc<dyn TextSource> = snapshot;
        let replaced = Arc::new(Document {
            id: current.id,
            resource: current.resource.clone(),
            metadata: DocumentMetadata {
                line_endings,
                ..current.metadata.clone()
            },
            text: text.clone(),
        });
        self.editor.replace_document(replaced.clone(), None);
        if let Some(files) = &self.editing.files {
            files.replace(&replaced);
        }
        let version = text.version();
        let mut codes = Vec::new();
        for views in self.views.values_mut() {
            for slot in std::iter::once(&mut views.primary).chain(views.secondary.as_mut()) {
                if let Some(shown) = slot.shown.as_mut().filter(|s| s.document == document) {
                    shown.version = version;
                    codes.push(slot.code.clone());
                }
            }
        }
        for code in codes {
            let text = text.clone();
            code.update(cx, |code, cx| {
                // 新快照：滚动位置不变，选区取会话映射后的位置。
                code.set_text(Some(text), true, cx);
                if let Some(s) = selection {
                    code.set_selection(s, None, cx);
                }
            });
        }
        let dirty = self.editing.saves.borrow().is_dirty(document);
        self.editor.set_dirty(document, dirty);
        cx.notify();
    }

    // ------------------------------------------------------------- 命令

    /// `type` / `default:type`（带 `{ "text": … }`）：交给聚焦的代码视图。
    pub(super) fn type_command(&mut self, args: &Value, cx: &mut Context<Self>) {
        let text = match args {
            Value::Object(map) => match map.get("text") {
                Some(Value::String(s)) => s.to_string(),
                _ => return,
            },
            Value::String(s) => s.to_string(),
            _ => return,
        };
        let code = self.code_view().clone();
        code.update(cx, |code, cx| {
            code.type_text(&text, None, cx);
        });
    }

    /// `workbench.action.editor.changeEOL`（参数 `"\n"` / `"\r\n"`）。
    pub(super) fn change_eol_command(&mut self, args: &Value, cx: &mut Context<Self>) {
        let eol = match args {
            Value::String(s) if &**s == "\r\n" => rove_buffer::Eol::CrLf,
            Value::String(s) if &**s == "\n" => rove_buffer::Eol::Lf,
            _ => return,
        };
        let Some(document) = self.active_document_id() else {
            return;
        };
        let changed = self
            .editing
            .saves
            .borrow_mut()
            .change_eol(document, eol, Instant::now())
            .unwrap_or(false);
        if changed {
            self.sync_document_from_session(document, cx);
        }
    }

    fn active_document_id(&self) -> Option<DocumentId> {
        self.editor
            .active()
            .current()
            .and_then(Tab::opened)
            .map(|o| o.document.id)
    }

    /// #49 的保存命令（`SaveAction`）。
    pub(super) fn save_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active_document_id();
        match id {
            save_ids::NEW_UNTITLED => self.new_untitled(window, cx),
            save_ids::SAVE | save_ids::SAVE_WITHOUT_FORMATTING => {
                if let Some(document) = active {
                    self.start_save(document, SaveRequest::default(), cx);
                }
            }
            save_ids::ACCEPT_LOCAL_CHANGES => {
                if let Some(document) = active {
                    let request = SaveRequest {
                        overwrite_conflict: true,
                        ..SaveRequest::default()
                    };
                    self.start_save(document, request, cx);
                }
            }
            save_ids::SAVE_AS => {
                if let Some(document) = active {
                    self.apply_save_effects(vec![Effect::PickSaveAsPath(document)], cx);
                }
            }
            save_ids::SAVE_ALL | save_ids::SAVE_ALL_SHORT | save_ids::SAVE_FILES => {
                let docs: Vec<DocumentId> = self.editing.saves.borrow().documents().collect();
                for document in docs {
                    if self.editing.saves.borrow().is_dirty(document) {
                        self.start_save(document, SaveRequest::default(), cx);
                    }
                }
            }
            save_ids::SAVE_ALL_IN_GROUP => {
                let docs: BTreeSet<DocumentId> = self
                    .editor
                    .active()
                    .tabs
                    .iter()
                    .filter_map(Tab::opened)
                    .map(|o| o.document.id)
                    .collect();
                for document in docs {
                    if self.editing.saves.borrow().is_dirty(document) {
                        self.start_save(document, SaveRequest::default(), cx);
                    }
                }
            }
            save_ids::REVERT | save_ids::REVERT_LOCAL_CHANGES => {
                if let Some(document) = active {
                    let effects = self
                        .editing
                        .saves
                        .borrow_mut()
                        .revert(
                            document,
                            &*self.settings,
                            &mut self.shell.feedback,
                            Instant::now(),
                        )
                        .unwrap_or_default();
                    self.apply_save_effects(effects, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// 保存：变换与编码在 UI 线程，冲突检测与原子写入在后台，结果回到窗口。
    pub(super) fn start_save(
        &mut self,
        document: DocumentId,
        request: SaveRequest,
        cx: &mut Context<Self>,
    ) {
        let options = self.save_options(document);
        let begun = self.editing.saves.borrow_mut().begin_save(
            document,
            &request,
            &options,
            &mut self.shell.feedback,
            Instant::now(),
        );
        // 保存时的变换（去行尾空白、补末尾换行）改了文本：先让视图跟上。
        self.sync_document_from_session(document, cx);
        match begun {
            Ok(job) => {
                let task = cx.background_executor().spawn(async move { job.run() });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    let _ = this.update(cx, |view, cx| {
                        let effects = view.editing.saves.borrow_mut().complete_save(
                            document,
                            result,
                            &mut view.shell.feedback,
                            Instant::now(),
                        );
                        view.apply_save_effects(effects, cx);
                    });
                })
                .detach();
            }
            Err(effects) => self.apply_save_effects(effects, cx),
        }
    }

    /// 把控制器的 [`Effect`] 反映到标签、视图与 #38。
    pub(super) fn apply_save_effects(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::Saved {
                    document,
                    path,
                    hash,
                    ..
                } => {
                    self.editing.deleted.remove(&document);
                    // #50：已经落盘了，备份没用了。
                    self.discard_hot_exit_backup(document);
                    self.saved_as(document, &path);
                    self.editor.set_dirty(document, false);
                    // 写盘期间缓冲区又被编辑过（会话的历史标记已经越过保存点）：磁盘上是保存时
                    // 的快照，缓冲区已经不是它了，这次保存不登记（理由见下）。
                    let clean = !self.editing.saves.borrow().is_dirty(document);
                    if let Some((Resource::Workspace(p), saved_version)) = self
                        .open_document(document)
                        .filter(|_| clean)
                        .map(|d| (d.resource.clone(), d.text.version()))
                    {
                        // #51：这两个「磁盘内容哈希」不是同一个算法。`Effect::Saved` 里的 `hash`
                        // 是 #49 自写去重用的 `DiskStamp`（FNV-1a）；#38 的 `BufferStates` 比的是
                        // **C# 索引文件表的 xxHash64**（`hash_bytes`）。直接把前者交过去，保存后的
                        // 文件就永远对不上索引版本：索引重建完成后仍判为 `Diverged`，整份文件的
                        // 查询一直停在近似，直到关掉重开。
                        //
                        // 这里在后台把刚写出的字节读回来算一次索引哈希（刚写完，页缓存是热的），
                        // 并用 FNV 哈希确认读到的仍然是我们写的那一份；不是就跳过，交给文件监听
                        // 走外部修改的路径。登记口径与 `apply_buffer_change` 的打开 / 重新载入一致。
                        //
                        // 登记的含义是「磁盘内容 == 当前缓冲区内容」，而保存是**异步**的：写盘与
                        // 这次读回期间，`text_edited` 照常接受编辑。所以除了校验磁盘内容，还要确认
                        // 缓冲区自这次保存写出的快照以来没有再变过，分两道：
                        //
                        // 1. 写盘期间就被改过（上面的 `clean`）→ 这次保存的快照已经不是缓冲区
                        //    内容，根本不读回、不登记；
                        // 2. 读回期间又被改 → 回到窗口时文档版本对不上，丢弃这次结果。
                        //
                        // 少了这两道守卫，就会把「只含保存快照」的哈希登记成当前内容的哈希：索引
                        // 随后对该文件重绑时 `file_state` 命中 `current_hash == indexed`，`rebase`
                        // 丢掉整份编辑记录并把文件判为 `Clean`，之后的跳转 / 引用 / 悬停全部按新
                        // 文本的位置去查旧内容的索引，还标成 `Exact`——是**静默的错误结果**。
                        // 对照 `nav_host.rs::apply_buffer_change` 里同一口径的 `still` 守卫。
                        let expected = hash;
                        let file = path.clone();
                        let task = cx.background_executor().spawn(async move {
                            let bytes = std::fs::read(&file).ok()?;
                            (rove_buffer::save::content_hash(&bytes) == expected)
                                .then(|| rove_lang_cs::router::hash_bytes(&bytes))
                        });
                        cx.spawn(async move |this, cx| {
                            let Some(index_hash) = task.await else { return };
                            let _ = this.update(cx, |view, _| {
                                let still = view.open_document(document).is_some_and(|d| {
                                    d.text.version() == saved_version
                                        && d.resource == Resource::Workspace(p.clone())
                                });
                                if still {
                                    let _ = view.nav.router.buffers().saved(&p, index_hash);
                                }
                            });
                        })
                        .detach();
                    }
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let _ = shell::view::report(
                        self,
                        "save.saved",
                        Reason::Receipt,
                        Message {
                            key: "save.status.saved".into(),
                            args: Arc::from([("name".into(), name.into())]),
                        },
                        Arc::from([]),
                        cx,
                    );
                }
                Effect::PickSaveAsPath(document) => self.pick_save_as(document, cx),
                Effect::Reloaded { document, .. } => {
                    self.sync_document_from_session(document, cx);
                }
                Effect::Deleted { document, deleted } => {
                    if deleted {
                        self.editing.deleted.insert(document);
                    } else {
                        self.editing.deleted.remove(&document);
                    }
                }
                Effect::Close(document) => {
                    self.editor.force_close_document(document);
                    self.editing.saves.borrow_mut().detach(document);
                    self.editing.deleted.remove(&document);
                    self.discard_hot_exit_backup(document);
                    self.sync_views(cx);
                    self.continue_window_close(cx);
                }
            }
        }
        cx.notify();
    }

    /// 另存为（或未命名文档首次保存）之后，标签与文档指向新路径。
    fn saved_as(&mut self, document: DocumentId, path: &std::path::Path) {
        let Some(current) = self.open_document(document) else {
            return;
        };
        let resource = match path.strip_prefix(&self.root) {
            Ok(rel) => WorkspacePath::new(&rel.to_string_lossy().replace('\\', "/"))
                .map(Resource::Workspace)
                .unwrap_or_else(|_| Resource::File(path.to_path_buf())),
            Err(_) => Resource::File(path.to_path_buf()),
        };
        if current.resource == resource {
            return;
        }
        let replaced = Arc::new(Document {
            resource: resource.clone(),
            ..(*current).clone()
        });
        for pane in self.editor.groups.values_mut() {
            for tab in &mut pane.tabs {
                if tab.opened().is_some_and(|o| o.document.id == document) {
                    tab.resource = resource.clone();
                }
            }
        }
        self.editor.replace_document(replaced, None);
    }

    /// “另存为”对话框（系统保存对话框，后台等待）。
    fn pick_save_as(&mut self, document: DocumentId, cx: &mut Context<Self>) {
        let directory = self
            .open_document(document)
            .and_then(|d| self.disk_path(&d.resource))
            .and_then(|p| p.parent().map(PathBuf::from))
            .unwrap_or_else(|| self.root.clone());
        let answer = cx.prompt_for_new_path(&directory, None);
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = answer.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                let request = SaveRequest {
                    save_as: Some(path),
                    ..SaveRequest::default()
                };
                view.start_save(document, request, cx);
            });
        })
        .detach();
    }

    /// 通知 / 模态框 / 提示条上的按钮（`save.*` 键）。
    pub(super) fn save_feedback_action(&mut self, key: &str, action: &str, cx: &mut Context<Self>) {
        let Some(document) = key
            .rsplit(':')
            .next()
            .and_then(|id| id.parse::<u64>().ok())
            .map(DocumentId)
        else {
            return;
        };
        if action == save_ids::ACTION_CANCEL {
            self.editing.closing_window = false;
        }
        let options = self.save_options(document);
        let effects = self.editing.saves.borrow_mut().on_action(
            key,
            action,
            &options,
            &*self.settings,
            &mut self.shell.feedback,
            Instant::now(),
        );
        // “保留本地修改”等不产生效果的动作也可能收起提示条。
        self.sync_document_from_session(document, cx);
        self.apply_save_effects(effects, cx);
    }

    /// `ctrl+n`：新建未命名的可编辑文档（`files.encoding` / `files.eol`），首次保存走另存为。
    fn new_untitled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing.untitled += 1;
        let n = self.editing.untitled;
        let name = self.label("save.untitled.name", &[("n", n.to_string())]);
        let language = "plaintext";
        let file = TextFile::untitled(
            &name,
            language,
            &*self.settings,
            self.edit_options(language),
        );
        let id = DocumentId(UNTITLED_BASE + n);
        let resource = Resource::Virtual(format!("untitled/{name}").into());
        let document = Arc::new(Document {
            id,
            resource: resource.clone(),
            metadata: DocumentMetadata {
                language: language.into(),
                encoding: file.encoding().clone(),
                bom: file.bom(),
                line_endings: file.line_endings().clone(),
                read_only: Arc::from([]),
                size: SizeClass::Normal,
            },
            text: file.snapshot(),
        });
        self.editing.saves.borrow_mut().attach(id, file);
        self.editor.open_ready(resource, document);
        self.sync_views(cx);
        self.sync_editing(cx);
        let group = self.editor.focused;
        self.focus_group_code(group, window, cx);
        cx.notify();
    }

    // ------------------------------------------------------------- 外部修改

    /// 工作区文件事件：打开中的文档记下变化，安静期后读盘。
    pub(super) fn note_disk_events(&mut self, events: &[FileEvent], cx: &mut Context<Self>) {
        let now = Instant::now();
        {
            let mut saves = self.editing.saves.borrow_mut();
            for event in events {
                let paths: Vec<&WorkspacePath> = match event {
                    FileEvent::Created(p) | FileEvent::Changed(p) | FileEvent::Removed(p) => {
                        vec![p]
                    }
                    FileEvent::Renamed { from, to } => vec![from, to],
                    FileEvent::RescanRequired => Vec::new(),
                };
                for p in paths {
                    saves.note_change(&self.root.join(p.as_str()), now);
                }
            }
        }
        self.schedule_disk_check(cx);
    }

    fn schedule_disk_check(&mut self, cx: &mut Context<Self>) {
        if self.editing.disk_timer {
            return;
        }
        let Some(deadline) = self.editing.saves.borrow().next_deadline() else {
            return;
        };
        self.editing.disk_timer = true;
        let wait = deadline.saturating_duration_since(Instant::now());
        let timer = cx.background_executor().timer(wait);
        cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |view, cx| {
                view.editing.disk_timer = false;
                view.read_due_files(cx);
                view.schedule_disk_check(cx);
            });
        })
        .detach();
    }

    fn read_due_files(&mut self, cx: &mut Context<Self>) {
        let due = self.editing.saves.borrow_mut().due(Instant::now());
        for (document, path) in due {
            let task = cx
                .background_executor()
                .spawn(async move { DiskStamp::read(&path) });
            cx.spawn(async move |this, cx| {
                let disk = task.await.ok().flatten();
                let _ = this.update(cx, |view, cx| {
                    let effects = view
                        .editing
                        .saves
                        .borrow_mut()
                        .apply_disk(
                            document,
                            disk,
                            &*view.settings,
                            &mut view.shell.feedback,
                            Instant::now(),
                        )
                        .unwrap_or_default();
                    view.apply_save_effects(effects, cx);
                });
            })
            .detach();
        }
    }

    // ------------------------------------------------------------- 关闭确认

    /// 关闭标签：有未保存的修改时弹“保存 / 不保存 / 取消”，否则直接关。
    pub(super) fn close_tab_checked(
        &mut self,
        group: EditorGroupId,
        tab: TabId,
        cx: &mut Context<Self>,
    ) {
        let document = self
            .editor
            .groups
            .get(&group)
            .and_then(|g| g.tab(tab))
            .filter(|t| t.dirty)
            .and_then(Tab::opened)
            .map(|o| o.document.id);
        // #50：关闭单个标签永远询问，热退出只在窗口 / 应用级别接管。
        self.end_hot_exit_window_close();
        match document {
            Some(document) => {
                let check = self
                    .editing
                    .saves
                    .borrow_mut()
                    .request_close(document, &mut self.shell.feedback);
                if check == CloseCheck::CloseNow {
                    self.editor.force_close_document(document);
                }
            }
            None => {
                self.editor.close_tab(group, tab);
            }
        }
        cx.notify();
    }

    /// `closeActiveEditor` 一类命令遇到脏标签：转成关闭确认。
    pub(super) fn close_active_checked(&mut self, cx: &mut Context<Self>) -> bool {
        let group = self.editor.focused;
        let Some(tab) = self
            .editor
            .active()
            .current()
            .filter(|t| t.dirty)
            .map(|t| t.id)
        else {
            return false;
        };
        self.close_tab_checked(group, tab, cx);
        true
    }

    /// 窗口关闭按钮：逐个确认未保存的文档；全部处理完才真正关闭。返回是否可以立即关闭。
    pub(super) fn window_should_close(&mut self, cx: &mut Context<Self>) -> bool {
        self.editing.closing_window = true;
        // #50：`files.hotExit` 允许时接管关闭确认（Rove 一个窗口一个文件夹，所以关掉本窗口
        // 就是退出应用）。
        self.begin_hot_exit_window_close(true);
        let next = self
            .editing
            .saves
            .borrow()
            .documents()
            .find(|d| self.editing.saves.borrow().is_dirty(*d));
        match next {
            None => {
                self.editing.closing_window = false;
                true
            }
            Some(document) => {
                let check = self
                    .editing
                    .saves
                    .borrow_mut()
                    .request_close(document, &mut self.shell.feedback);
                if check == CloseCheck::CloseNow {
                    // 热退出（#50）接管：不再确认。
                    self.editing.closing_window = false;
                    return true;
                }
                self.end_hot_exit_window_close();
                cx.notify();
                false
            }
        }
    }

    /// 窗口关闭按钮的完整流程：#49 的逐个确认 + #50 的备份与干净退出标记。
    /// 返回是否可以立即关闭。
    pub(super) fn request_window_close(&mut self, cx: &mut Context<Self>) -> bool {
        let closing = self.window_should_close(cx);
        if closing {
            // 窗口真的要关了：脏文档的备份同步写完，再写下干净的退出标记。
            self.flush_hot_exit();
        }
        closing
    }

    /// 一个文档处理完（保存或不保存）：窗口关闭流程中继续下一个，全部处理完就关窗口。
    fn continue_window_close(&mut self, cx: &mut Context<Self>) {
        if !self.editing.closing_window {
            return;
        }
        if self.window_should_close(cx) {
            self.editing.closing_window = false;
            self.flush_hot_exit();
            if let Some(window) = self.editing.window {
                cx.defer(move |cx| {
                    let _ = window.update(cx, |_, window, _| window.remove_window());
                });
            }
        }
    }

    /// 标签是否显示“已删除”。
    pub(super) fn tab_deleted(&self, tab: &Tab) -> bool {
        tab.opened()
            .is_some_and(|o| self.editing.deleted.contains(&o.document.id))
    }
}

/// 窗口关闭按钮接入关闭确认（替换 `Shell::attach_window` 登记的回调，先做它原来做的记录布局）。
pub(super) fn attach_close_confirmation(
    view: &mut Workbench,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    view.editing.window = Some(window.window_handle());
    let entity = cx.entity().downgrade();
    window.on_window_should_close(cx, move |window, cx| {
        let Some(entity) = entity.upgrade() else {
            return true;
        };
        entity.update(cx, |view, cx| {
            view.shell_mut().record_window(window);
            view.shell_mut().save_now();
            view.request_window_close(cx)
        })
    });
}
