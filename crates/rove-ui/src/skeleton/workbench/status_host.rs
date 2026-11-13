//! #27 状态栏条目的窗口装配：每帧把活动编辑器的状态投影到 #16 的状态栏容器，并实现
//! 四个选择器（缩进、编码、换行符、语言模式）的点击行为。
//!
//! 文字与颜色的投影是纯数据，在 [`crate::status_items::model`]；这里只负责：
//!
//! - **同步**：[`Workbench::sync_status`] 每帧调用，比较上一帧的状态快照，只有变化时才写
//!   `StatusBar`（光标移动不会让整条状态栏重新布局：条目内容不变时 `StatusBar::set` 不涨
//!   revision）。没有活动文本编辑器时移除全部六个条目（与 VS Code 一致）。
//! - **选择器**：命令只记下意图（命令分发路径上没有 `Window`），浮层在下一帧的
//!   [`Workbench::sync_status`] 里用 #18 的快速输入打开。
//! - **编码重新打开**：走 #7 的指定编码重读（[`FileDocuments::reopen_with_encoding`]），
//!   文档身份不变，滚动位置由 #17 的快照替换保持。
//! - **语言模式**：会话内的覆盖，换上带新语言的快照后由 #17 重新触发 #9 的解析。
//! - **缩进**：`editor.action.indentUsing*` / `detectIndentation` 只改本文件的缩进视图参数；
//!   真正改写文件的「转换缩进」交给 #46 的编辑核心（由代码视图的命令执行）。

use super::super::text::FileDocuments;
use super::Workbench;
use crate::{
    code_view::view::presented_read_only,
    documents::EditorGroupId,
    editors::settings::EditorKind,
    list_widget::{
        quick_input::{QuickPickConfig, QuickPickEvent},
        StaticList,
    },
    lists::{ItemId, QuickInputOptions, Row},
    shell::feedback::Reason,
    status_items::{
        model::{self, Indentation, PickItem},
        registry as ids,
    },
};
use gpui::{Context, Window};
use rove_base::{DocumentId, Line, Position, Resource, Span, TextRange, Value, Version};
use rove_buffer::{
    Document, Encoding, Eol, LineEndings, ReadOnlyReason, ReducedFeature, SizeClass, TextSource,
};
use rove_config::{Key, Scope};
use rove_i18n::Message;
use std::{collections::BTreeMap, sync::Arc};

/// 检测缩进时最多读的行数（VS Code 读 10000 行；阅读器只需要前若干行就能判定）。
const DETECT_LINES: usize = 500;
/// 统计“已选择 N 个字符”时最多遍历的字节数：超过这个长度只显示行列，不算字符数。
const MAX_SELECTION_BYTES: u64 = 4 * 1024 * 1024;

const TAB_SIZE: &str = "editor.tabSize";
const INSERT_SPACES: &str = "editor.insertSpaces";
const DETECT_INDENTATION: &str = "editor.detectIndentation";
const FILES_ENCODING: &str = "files.encoding";
const CANDIDATE_ENCODINGS: &str = "files.candidateGuessEncodings";

/// 当前打开的选择器。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pick {
    Indent,
    /// 选了“使用空格 / 制表符缩进”之后的宽度选择（`true` 为空格）。
    IndentSize(bool),
    Encoding,
    /// 编码列表（目前只有“通过编码重新打开”进到这一层）。
    EncodingList,
    Eol,
    Language,
}

/// 一帧的状态快照；与上一帧相同时不重算条目。
#[derive(Clone, PartialEq)]
struct Snapshot {
    document: DocumentId,
    version: Version,
    cursor: Position,
    selection: Option<TextRange>,
    indent: Indentation,
    language: Arc<str>,
    encoding: Encoding,
    bom: bool,
    guessed: bool,
    eol: (Eol, bool),
    size: SizeClass,
    read_only: Vec<ReadOnlyReason>,
}

#[derive(Default)]
pub(super) struct StatusItems {
    /// 下一帧要打开的选择器（命令路径上没有 `Window`）。
    pending: Option<Pick>,
    /// 下一帧要在代码视图上执行的命令（缩进转换）。
    run: Option<&'static str>,
    /// 当前由本模块打开的浮层。按 ID 匹配「接受」事件：浮层被取消时这里留下的旧 ID 不会再命中
    /// （ID 单调递增），所以不需要额外的关闭通知。
    active: Option<(crate::lists::QuickInputId, Pick)>,
    revision: u64,
    /// 每个文档的缩进视图参数（`editor.action.indentUsing*`、`detectIndentation`）。
    indent: BTreeMap<DocumentId, Indentation>,
    /// 内容检测的缓存（按文本版本）。
    detected: BTreeMap<DocumentId, (Version, Indentation)>,
    /// 会话内的语言模式覆盖。
    language: BTreeMap<DocumentId, Arc<str>>,
    last: Option<Snapshot>,
}

impl StatusItems {
    /// 丢掉缓存的快照，强制下一次 [`Workbench::sync_status`] 重算全部条目（测试用）。
    #[cfg(test)]
    pub(super) fn invalidate(&mut self) {
        self.last = None;
    }
}

impl Workbench {
    // ------------------------------------------------------------- 每帧同步

    /// 每帧调用：打开待处理的选择器、执行待处理的命令，然后把活动编辑器的状态写进状态栏。
    pub(super) fn sync_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pick) = self.status.pending.take() {
            self.open_status_pick(pick, window, cx);
        }
        if let Some(command) = self.status.run.take() {
            let code = self.code_view().clone();
            code.update(cx, |code, cx| {
                code.execute(command, window, cx);
            });
        }
        let snapshot = self.status_snapshot();
        if self.status.last == snapshot {
            return;
        }
        self.status.last = snapshot.clone();
        let Some(snapshot) = snapshot else {
            let shell = &mut self.shell;
            for (id, ..) in ids::ITEMS {
                shell.status_bar.remove(id);
            }
            return;
        };
        self.write_status_entries(&snapshot);
    }

    /// 活动文本编辑器的状态；没有活动文本编辑器时 `None`（条目全部隐藏）。
    fn status_snapshot(&mut self) -> Option<Snapshot> {
        let (cursor, selection, document) = {
            let tab = self
                .editor
                .groups
                .get(&self.editor.focused)?
                .current()
                .filter(|tab| tab.kind == EditorKind::Text)?;
            let opened = tab.opened()?;
            (
                tab.view.cursor,
                tab.view.selection.clone(),
                opened.document.clone(),
            )
        };
        let read_only = presented_read_only(&self.read_only_of(&document));
        Some(Snapshot {
            document: document.id,
            version: document.text.version(),
            cursor,
            selection,
            indent: self.indentation_of(&document),
            language: self.language_of(&document),
            encoding: document.metadata.encoding.clone(),
            bom: document.metadata.bom,
            guessed: self.metadata.files.encoding_guessed(document.id),
            eol: eol_key(&document.metadata.line_endings),
            size: document.metadata.size,
            read_only,
        })
    }

    fn write_status_entries(&mut self, snapshot: &Snapshot) {
        let document = self.focused_document();
        let cursors = document
            .as_ref()
            .map(|document| cursor_summary(&*document.text, snapshot))
            .unwrap_or_default();
        let disabled: Vec<ReducedFeature> = if snapshot.size == SizeClass::Huge {
            rove_buffer::disabled_features(snapshot.size).to_vec()
        } else {
            Vec::new()
        };
        let strings = &self.strings;
        let entries = [
            (ids::CURSOR, Some(model::cursor_entry(strings, &cursors))),
            (
                ids::INDENT,
                Some(model::indent_entry(strings, snapshot.indent)),
            ),
            (
                ids::ENCODING,
                Some(model::encoding_entry(
                    strings,
                    &snapshot.encoding,
                    snapshot.bom,
                    snapshot.guessed,
                )),
            ),
            (
                ids::EOL,
                Some(model::eol_entry(strings, &eol_from_key(snapshot.eol))),
            ),
            (
                ids::LANGUAGE,
                Some(model::language_entry(strings, &snapshot.language)),
            ),
            (
                ids::READ_ONLY,
                model::read_only_entry(strings, snapshot.size, &snapshot.read_only, &disabled),
            ),
        ];
        let shell = &mut self.shell;
        for (id, entry) in entries {
            match entry {
                Some(entry) => {
                    let _ = shell.status_bar.set(&shell.contributions, id, entry);
                }
                None => {
                    shell.status_bar.remove(id);
                }
            }
        }
    }

    // ------------------------------------------------------------- 缩进与语言

    /// 本文件生效的缩进：会话内的覆盖 > `editor.detectIndentation` 的检测结果 > 设置值。
    /// 检测结果按文本版本缓存，切换标签时不会重复扫描文件头部。
    fn indentation_of(&mut self, document: &Document) -> Indentation {
        if let Some(indent) = self.status.indent.get(&document.id) {
            return *indent;
        }
        let configured = self.configured_indentation(&document.metadata.language);
        let scope = Scope {
            language: Some(document.metadata.language.clone()),
        };
        let detect = Key::<bool>::new(DETECT_INDENTATION)
            .read(&*self.settings, &scope)
            .unwrap_or(true);
        if !detect {
            return configured;
        }
        if let Some((version, indent)) = self.status.detected.get(&document.id) {
            if *version == document.text.version() {
                return *indent;
            }
        }
        self.cache_detection(document)
    }

    fn configured_indentation(&self, language: &str) -> Indentation {
        let scope = Scope {
            language: Some(Arc::from(language)),
        };
        let size = Key::<f64>::new(TAB_SIZE)
            .read(&*self.settings, &scope)
            .ok()
            .filter(|n| n.is_finite())
            .map_or(4, |n| n.clamp(1.0, 100.0) as u64);
        let insert_spaces = Key::<bool>::new(INSERT_SPACES)
            .read(&*self.settings, &scope)
            .unwrap_or(true);
        Indentation {
            insert_spaces,
            size,
            detected: false,
        }
    }

    /// 本文件生效的语言模式（会话内的覆盖优先）。
    fn language_of(&self, document: &Document) -> Arc<str> {
        self.status
            .language
            .get(&document.id)
            .cloned()
            .unwrap_or_else(|| document.metadata.language.clone())
    }

    /// 缓存一次内容检测的结果（`indentation_of` 是 `&self`，检测只在这里写回）。
    fn cache_detection(&mut self, document: &Document) -> Indentation {
        let configured = self.configured_indentation(&document.metadata.language);
        let indent = detect_indentation(&*document.text, configured);
        self.status
            .detected
            .insert(document.id, (document.text.version(), indent));
        indent
    }

    // ------------------------------------------------------------- 命令

    /// #27 的命令（状态栏点击、命令面板与键位都走这里）。只记下意图，浮层在下一帧打开。
    pub(super) fn status_command(&mut self, id: &str, args: &Value, cx: &mut Context<Self>) {
        // 带参数的 `changeEOL`（选择器选中后自己发出的那次）直接统一行尾，不再弹选择器。
        if id == ids::CHANGE_EOL && !matches!(args, Value::Null) {
            self.change_eol_command(args, cx);
            return;
        }
        if self.focused_document().is_none() {
            return;
        }
        self.status.pending = match id {
            ids::CHANGE_INDENTATION => Some(Pick::Indent),
            ids::INDENT_USING_SPACES => Some(Pick::IndentSize(true)),
            ids::INDENT_USING_TABS => Some(Pick::IndentSize(false)),
            ids::CHANGE_ENCODING => Some(Pick::Encoding),
            ids::CHANGE_EOL => Some(Pick::Eol),
            ids::CHANGE_LANGUAGE_MODE => Some(Pick::Language),
            ids::DETECT_INDENTATION => {
                self.detect_indentation_command(cx);
                None
            }
            _ => None,
        };
        cx.notify();
    }

    fn detect_indentation_command(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.focused_document() else {
            return;
        };
        let indent = self.cache_detection(&document);
        self.set_indentation(document.id, indent, cx);
    }

    fn focused_document(&self) -> Option<Arc<Document>> {
        let tab = self
            .editor
            .groups
            .get(&self.editor.focused)?
            .current()
            .filter(|tab| tab.kind == EditorKind::Text)?;
        tab.opened().map(|opened| opened.document.clone())
    }

    // ------------------------------------------------------------- 选择器

    fn open_status_pick(&mut self, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        let Some(document) = self.focused_document() else {
            return;
        };
        let editable = self.read_only_of(&document).is_empty()
            && document.metadata.size != SizeClass::Huge
            && matches!(
                document.resource,
                Resource::Workspace(_) | Resource::File(_)
            );
        let strings = &self.strings;
        let (items, placeholder) = match pick {
            Pick::Indent => (
                model::indent_picks(strings, editable),
                "status.indent.placeholder",
            ),
            Pick::IndentSize(_) => (
                model::indent_size_picks(strings),
                "status.indent.placeholder",
            ),
            Pick::Encoding => (
                model::encoding_actions(strings, false),
                "status.encoding.placeholder",
            ),
            Pick::EncodingList => (
                model::encoding_picks(
                    strings,
                    &self.configured_encoding(&document),
                    &self.candidate_encodings(),
                ),
                "status.encoding.reopenPlaceholder",
            ),
            Pick::Eol => (
                model::eol_picks(strings, editable),
                "status.eol.placeholder",
            ),
            Pick::Language => (
                model::language_picks(strings, &language_ids(), &self.language_of(&document)),
                "status.language.placeholder",
            ),
        };
        if items.is_empty() {
            return;
        }
        self.show_status_pick(pick, items, placeholder, window, cx);
    }

    fn configured_encoding(&self, document: &Document) -> String {
        let scope = Scope {
            language: Some(document.metadata.language.clone()),
        };
        Key::<String>::new(FILES_ENCODING)
            .read(&*self.settings, &scope)
            .unwrap_or_else(|_| "utf8".into())
    }

    fn candidate_encodings(&self) -> Vec<String> {
        match self.settings.read(CANDIDATE_ENCODINGS, &Scope::default()) {
            Ok(Value::Array(items)) => items
                .iter()
                .filter_map(|item| match item {
                    Value::String(id) => Some(id.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn show_status_pick(
        &mut self,
        pick: Pick,
        items: Vec<PickItem>,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.ensure_quick_view(cx);
        let mut rows: Vec<Row> = Vec::new();
        let mut separators: Vec<(usize, Option<Arc<str>>)> = Vec::new();
        for item in &items {
            if let Some(group) = &item.group {
                separators.push((rows.len(), Some(group.clone())));
            }
            rows.push(Row {
                id: ItemId(item.id.clone()),
                label: item.label.clone(),
                detail: item.detail.clone(),
                depth: 0,
                expandable: false,
                expanded: false,
                disabled: false,
            });
        }
        self.status.revision += 1;
        let model = StaticList::new(rows, Version(self.status.revision));
        let placeholder = Message::key(placeholder.to_owned());
        let result = view.update(cx, |q, cx| {
            q.show(
                QuickInputOptions {
                    placeholder,
                    initial: "".into(),
                    items: Arc::new(model),
                    multi_select: false,
                },
                QuickPickConfig {
                    separators,
                    ..Default::default()
                },
                window,
                cx,
            )
        });
        match result {
            Ok(id) => {
                // 快速打开（#24）的状态让位给本选择器：它们共用同一个浮层实体。
                self.quick.id = None;
                self.status.active = Some((id, pick));
            }
            Err(e) => eprintln!("statusBar pick: {e}"),
        }
        cx.notify();
    }

    /// #18 的快速输入发来的“接受”。返回是否由本模块处理（#24 的快速打开不再处理它）。
    pub(super) fn status_picked(&mut self, event: &QuickPickEvent, cx: &mut Context<Self>) -> bool {
        let QuickPickEvent::Accepted { id, items, .. } = event else {
            return false;
        };
        let Some((active, pick)) = self.status.active else {
            return false;
        };
        if active != *id {
            return false;
        }
        self.status.active = None;
        self.quick.focus_editor = true;
        if let Some(item) = items.first() {
            self.apply_status_pick(pick, item.0.clone(), cx);
        }
        cx.notify();
        true
    }

    fn apply_status_pick(&mut self, pick: Pick, item: Arc<str>, cx: &mut Context<Self>) {
        let Some(document) = self.focused_document() else {
            return;
        };
        match pick {
            Pick::Indent => match &*item {
                ids::INDENT_USING_SPACES => self.status.pending = Some(Pick::IndentSize(true)),
                ids::INDENT_USING_TABS => self.status.pending = Some(Pick::IndentSize(false)),
                ids::DETECT_INDENTATION => self.detect_indentation_command(cx),
                // 改写文件内容：交给 #46 的编辑核心（代码视图的命令），下一帧执行。
                "editor.action.indentationToSpaces" => {
                    self.status.run = Some("editor.action.indentationToSpaces")
                }
                "editor.action.indentationToTabs" => {
                    self.status.run = Some("editor.action.indentationToTabs")
                }
                _ => {}
            },
            Pick::IndentSize(insert_spaces) => {
                if let Ok(size) = item.parse::<u64>() {
                    self.set_indentation(
                        document.id,
                        Indentation {
                            insert_spaces,
                            size,
                            detected: false,
                        },
                        cx,
                    );
                }
            }
            Pick::Encoding => {
                if &*item == model::ENCODING_REOPEN {
                    self.status.pending = Some(Pick::EncodingList);
                }
            }
            Pick::EncodingList => self.reopen_with_encoding(&document, &item, cx),
            Pick::Eol => {
                self.change_eol_command(&Value::String(item), cx);
            }
            Pick::Language => self.set_language_mode(&document, item, cx),
        }
    }

    // ------------------------------------------------------------- 应用

    fn set_indentation(
        &mut self,
        document: DocumentId,
        indent: Indentation,
        cx: &mut Context<Self>,
    ) {
        self.status.indent.insert(document, indent);
        // 缩进宽度就是代码视图的制表位宽度（`editor.tabSize`）：只改显示这篇文档的视图。
        for code in self.code_views_of(document) {
            if code.read(cx).options().tab_size == indent.size as u32 {
                continue;
            }
            let mut options = code.read(cx).options().clone();
            options.tab_size = indent.size as u32;
            code.update(cx, |code, cx| code.set_options(options, cx));
        }
        cx.notify();
    }

    /// 「通过编码重新打开」：#7 按指定编码重读，文档身份不变，滚动位置与选区由 #17 保持。
    fn reopen_with_encoding(
        &mut self,
        document: &Arc<Document>,
        encoding: &str,
        cx: &mut Context<Self>,
    ) {
        let files: &FileDocuments = &self.metadata.files;
        match files.reopen_with_encoding(document, encoding) {
            Ok(reopened) => {
                self.metadata.files.replace(&reopened);
                let name = model::encoding_label(&reopened.metadata.encoding);
                self.editor.reload_document(reopened);
                self.sync_views(cx);
                let _ = crate::shell::view::report(
                    self,
                    "status.encoding.reopened",
                    Reason::Receipt,
                    Message {
                        key: "status.encoding.reopened".into(),
                        args: Arc::from([(Arc::from("name"), Arc::from(name.as_str()))]),
                    },
                    Arc::from([]),
                    cx,
                );
            }
            Err(error) => {
                eprintln!("reopenWithEncoding {encoding}: {error}");
                let _ = crate::shell::view::report(
                    self,
                    "status.encoding.reopenFailed",
                    Reason::Receipt,
                    Message {
                        key: "status.encoding.reopenFailed".into(),
                        args: Arc::from([(Arc::from("name"), Arc::from(encoding))]),
                    },
                    Arc::from([]),
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// 语言模式只对本文件生效（会话内）：换上带新语言的快照，#17 据此重新触发 #9 的解析。
    pub(super) fn set_language_mode(
        &mut self,
        document: &Arc<Document>,
        language: Arc<str>,
        cx: &mut Context<Self>,
    ) {
        if document.metadata.language == language {
            self.status.language.remove(&document.id);
        } else {
            self.status.language.insert(document.id, language.clone());
        }
        let mut metadata = document.metadata.clone();
        metadata.language = language.clone();
        let switched = Arc::new(Document {
            id: document.id,
            resource: document.resource.clone(),
            metadata,
            text: document.text.clone(),
        });
        self.metadata.files.replace(&switched);
        let id = switched.id;
        self.editor.reload_document(switched);
        // 文本版本没变，`sync_slot` 不会重新推送：直接把新语言与“等待新结果”的状态给视图。
        for code in self.code_views_of(id) {
            let language = language.clone();
            code.update(cx, |code, cx| {
                code.set_language(Some(language), cx);
                code.set_syntax(None, None, cx);
            });
        }
        // `set_language` 会按新语言重读设置，缩进覆盖要重新贴回去。
        if let Some(indent) = self.status.indent.get(&id).copied() {
            self.set_indentation(id, indent, cx);
        }
        self.status.last = None;
        cx.notify();
    }

    fn group_code_view(
        &self,
        group: EditorGroupId,
        secondary: bool,
    ) -> Option<gpui::Entity<crate::code_view::CodeView>> {
        let views = self.views.get(&group)?;
        if secondary {
            views.secondary.as_ref().map(|slot| slot.code.clone())
        } else {
            Some(views.primary.code.clone())
        }
    }

    /// 正在显示 `document` 的代码视图（主视图与组内并排视图）。
    fn code_views_of(&self, document: DocumentId) -> Vec<gpui::Entity<crate::code_view::CodeView>> {
        self.views
            .iter()
            .flat_map(|(group, views)| {
                std::iter::once((*group, false))
                    .chain(views.secondary.as_ref().map(|_| (*group, true)))
            })
            .filter(|(group, secondary)| {
                self.shown_document(*group, *secondary).map(|d| d.id) == Some(document)
            })
            .filter_map(|(group, secondary)| self.group_code_view(group, secondary))
            .collect()
    }
}

// --------------------------------------------------------------------- 纯函数

fn eol_key(endings: &LineEndings) -> (Eol, bool) {
    match endings {
        LineEndings::Uniform(eol) => (*eol, false),
        LineEndings::Mixed { preferred } => (*preferred, true),
    }
}

fn eol_from_key((eol, mixed): (Eol, bool)) -> LineEndings {
    if mixed {
        LineEndings::Mixed { preferred: eol }
    } else {
        LineEndings::Uniform(eol)
    }
}

/// 插入符位置（1 基，列按 UTF-16 计，与 VS Code 一致）与选中的字符数。
fn cursor_summary(text: &dyn TextSource, snapshot: &Snapshot) -> Vec<model::CursorSummary> {
    let column = text
        .utf8_to_utf16(snapshot.cursor.line, snapshot.cursor.column)
        .map(|c| c.0)
        .unwrap_or(snapshot.cursor.column.0);
    let selected = snapshot
        .selection
        .as_ref()
        .and_then(|range| selected_characters(text, range))
        .unwrap_or(0);
    vec![model::CursorSummary {
        line: snapshot.cursor.line.0 + 1,
        column: column + 1,
        selected,
    }]
}

fn selected_characters(text: &dyn TextSource, range: &TextRange) -> Option<u64> {
    let start = text.offset(range.start()).ok()?;
    let end = text.offset(range.end()).ok()?;
    if end.0.saturating_sub(start.0) > MAX_SELECTION_BYTES {
        return None;
    }
    let span = Span::new(start, end).ok()?;
    let mut characters = 0u64;
    text.visit_chunks(span, &mut |chunk| {
        characters += chunk.chars().count() as u64
    })
    .ok()?;
    Some(characters)
}

/// 读前 [`DETECT_LINES`] 行做缩进检测；行索引还没建好（超大文件）时保留设置值。
fn detect_indentation(text: &dyn TextSource, configured: Indentation) -> Indentation {
    let known = text.lines().known.min(DETECT_LINES as u64);
    let mut lines: Vec<String> = Vec::new();
    for index in 0..known {
        let Ok(range) = text.line_range(Line(index)) else {
            break;
        };
        let mut line = String::new();
        if text
            .visit_chunks(range, &mut |chunk| line.push_str(chunk))
            .is_err()
        {
            break;
        }
        lines.push(line);
    }
    model::guess_indentation(lines.iter().map(String::as_str), DETECT_LINES, configured)
}

/// 选择器里列出的语言：#12 已登记的语言配置 + `plaintext`（没有语言配置但是合法的模式）。
fn language_ids() -> Vec<Arc<str>> {
    let mut ids: Vec<Arc<str>> = rove_syntax::language_config::builtin_languages()
        .map(|(id, _)| Arc::from(id))
        .collect();
    if !ids.iter().any(|id| &**id == "plaintext") {
        ids.push(Arc::from("plaintext"));
    }
    ids
}
