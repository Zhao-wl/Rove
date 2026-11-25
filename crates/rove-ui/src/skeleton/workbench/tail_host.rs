//! R66 commands and workbench state. Physical reloads are coalesced per document.
use super::*;
use crate::{
    documents::{OpenSource, OpenTarget},
    follow_tail::{self as reg, PreviewSource},
    large_file::{notice::format_size, TruncationAction},
    shell::status::StatusEntry,
    skeleton::text::VirtualContent,
};
use rove_docview::{
    format::{format_text_bounded, FormatMode, FormatOptions},
    structure::StructuredLanguage,
};

const MAX_FORMAT_INPUT: u64 = 16 * 1024 * 1024;

#[derive(Default)]
pub(super) struct FollowTail {
    pub(super) previews: Arc<PreviewSource>,
    formatting: Option<(DocumentId, Version, Cancellation)>,
    views: BTreeMap<TabId, FollowView>,
    documents: BTreeMap<DocumentId, FollowDocument>,
    status: Option<FollowMode>,
    polling: bool,
    pub(super) title_menu: Option<(EditorGroupId, TabId, gpui::Point<gpui::Pixels>)>,
    pub(super) counts: FollowCounts,
}

#[cfg(test)]
impl FollowTail {
    pub(super) fn has_ordinary_baseline(&self, id: DocumentId) -> bool {
        self.documents
            .get(&id)
            .is_some_and(|state| state.ordinary.is_some())
    }
}

fn format_language(id: &str) -> Option<StructuredLanguage> {
    StructuredLanguage::from_id(id).filter(|lang| *lang != StructuredLanguage::Yaml)
}

pub(super) fn preview_overlay() -> TruncationAction {
    TruncationAction {
        command: reg::FORMATTED,
        label_key: reg::FORMAT_LABEL,
        applies: |code| {
            code.language.as_deref().and_then(format_language).is_some()
                && code.text().is_some_and(|text| {
                    let lines = text.lines();
                    lines.complete && lines.known == 1
                })
        },
    }
}

impl Workbench {
    pub(super) fn formatted_tab_title(&self, resource: &Resource) -> Option<String> {
        let Resource::Virtual(name) = resource else {
            return None;
        };
        let name = PreviewSource::name(name)?;
        Some(self.label("followTail.formattedTitle", &[("name", name.to_owned())]))
    }

    fn source_for_format(&self, args: &Value) -> Option<Arc<Document>> {
        if let Value::Object(args) = args {
            if let Some(Value::String(id)) = args.get("documentId") {
                return self.open_document(DocumentId(id.parse().ok()?));
            }
        }
        self.active_large_file_document()
    }

    pub(super) fn follow_command_visible(&self, id: &str) -> bool {
        match id {
            reg::FORMATTED => self
                .active_large_file_document()
                .is_some_and(|doc| format_language(&doc.metadata.language).is_some()),
            reg::FOLLOW => self
                .active_large_file_document()
                .is_some_and(|doc| self.disk_path(&doc.resource).is_some()),
            reg::RESUME | reg::CANCEL_FORMAT => false,
            _ => true,
        }
    }

    fn format_preview(&mut self, args: &Value, cx: &mut Context<Self>) {
        let Some(document) = self.source_for_format(args) else {
            return;
        };
        let Some(language) = format_language(&document.metadata.language) else {
            return;
        };
        let limit = MAX_FORMAT_INPUT.min(self.editor.settings.size.huge_bytes);
        if document.text.byte_len() > limit {
            self.follow_message(
                Message {
                    key: "followTail.formatTooLarge".into(),
                    args: Arc::from([("limit".into(), format_size(limit).into())]),
                },
                cx,
            );
            return;
        }
        self.cancel_formatted_preview(cx);
        let cancel = Cancellation::default();
        let (id, version) = (document.id, document.text.version());
        let group = self.editor.focused;
        let filename = match &document.resource {
            Resource::Workspace(path) => path.as_str().rsplit('/').next(),
            Resource::File(path) => path.file_name().and_then(|name| name.to_str()),
            Resource::Virtual(name) => PreviewSource::name(name),
        }
        .map(str::to_owned)
        .unwrap_or_else(|| self.editor.settings.tab_label(&document.resource));
        let view_options = self.code_view().read(cx).options();
        let indentation = if view_options.insert_spaces {
            " ".repeat(view_options.tab_size as usize)
        } else {
            "\t".to_owned()
        };
        let newline = match document.metadata.line_endings {
            rove_buffer::LineEndings::Uniform(eol)
            | rove_buffer::LineEndings::Mixed { preferred: eol } => match eol {
                rove_buffer::Eol::Lf => "\n",
                rove_buffer::Eol::Cr => "\r",
                rove_buffer::Eol::CrLf => "\r\n",
            },
        };
        self.follow_tail.formatting = Some((id, version, cancel.clone()));
        self.publish_format_status(true);

        let completion = cancel.clone();
        let task = cx.background_executor().spawn(async move {
            let input =
                super::structured_host::document_text(document.text.as_ref(), None, &cancel)?;
            let text = format_text_bounded(
                language,
                &input,
                FormatOptions {
                    indentation: &indentation,
                    newline,
                    keep_lines: false,
                },
                FormatMode::Pretty,
                (MAX_FORMAT_INPUT * 4) as usize,
                &cancel,
            )?;
            Ok(VirtualContent {
                text,
                language: document.metadata.language.clone(),
                read_only: vec![ReadOnlyReason::Metadata],
            })
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |view, cx| {
                if completion.check().is_err() {
                    return;
                }
                view.follow_tail.formatting = None;
                view.publish_format_status(false);
                if !view
                    .open_document(id)
                    .is_some_and(|doc| doc.text.version() == version)
                {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(content) => {
                        let name = view.follow_tail.previews.prepare(&filename, content);
                        let preserve_focus = view.editor.focused != group
                            || view
                                .active_large_file_document()
                                .is_none_or(|doc| doc.id != id);
                        let options = OpenOptions {
                            target: OpenTarget::Group(group),
                            source: OpenSource::CodeNavigation,
                            preview_tab: false,
                            preserve_focus,
                            ..OpenOptions::default()
                        };
                        if view
                            .open_documents()
                            .open(
                                Resource::Virtual(name.clone()),
                                options,
                                Cancellation::default(),
                            )
                            .is_err()
                        {
                            view.follow_tail.previews.discard(&name);
                            view.follow_message(Message::key("followTail.formatFailed"), cx);
                        }
                    }
                    Err(Error::OutOfBounds) => {
                        view.follow_message(
                            Message {
                                key: "followTail.outputTooLarge".into(),
                                args: Arc::from([(
                                    "limit".into(),
                                    format_size(MAX_FORMAT_INPUT * 4).into(),
                                )]),
                            },
                            cx,
                        );
                    }
                    Err(_) => view.follow_message(Message::key("followTail.formatFailed"), cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn follow_message(&mut self, message: Message, cx: &mut Context<Self>) {
        let _ = shell::view::report(
            self,
            "followTail.feedback",
            Reason::Receipt,
            message,
            Arc::from([]),
            cx,
        );
    }

    fn publish_format_status(&mut self, visible: bool) {
        if visible {
            let text = self.label("followTail.formatting", &[]);
            let _ = self.shell.status_bar.set(
                &self.shell.contributions,
                reg::FORMAT_STATUS,
                StatusEntry {
                    text,
                    ..StatusEntry::default()
                },
            );
        } else {
            self.shell.status_bar.remove(reg::FORMAT_STATUS);
        }
    }

    pub(super) fn cancel_formatted_preview(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((_, _, cancel)) = self.follow_tail.formatting.take() else {
            return false;
        };
        cancel.cancel();
        self.publish_format_status(false);
        cx.notify();
        true
    }

    pub(super) fn follow_command(
        &mut self,
        invocation: &Invocation,
        cx: &mut Context<Self>,
    ) -> bool {
        match invocation.id.as_ref() {
            reg::FORMATTED => self.format_preview(&invocation.args, cx),
            reg::FOLLOW => self.toggle_follow_tail(cx),
            reg::RESUME => self.resume_follow_tail(cx),
            reg::CANCEL_FORMAT => {
                self.cancel_formatted_preview(cx);
            }
            _ => return false,
        }
        true
    }
}

use super::{
    tail_read::{self, Change as TailChange, OrdinaryBaseline, PathWatch, ReadJob, ReadResult},
    tail_state::{path_request, FollowMode, PathEvent, Refresh, ReloadQueue, ReloadRequest},
};
use rove_buffer::{Encoding, FileText};

struct FollowView {
    document: DocumentId,
    mode: FollowMode,
    last_end: Option<(Version, u64)>,
}

struct FollowDocument {
    path: PathBuf,
    workspace_path: Option<WorkspacePath>,
    queue: ReloadQueue,
    watch: PathWatch,
    ordinary: Option<OrdinaryBaseline>,
    storage_ready: bool,
    ready: Option<(Refresh, ReadResult)>,
}

#[derive(Default)]
pub(super) struct FollowCounts {
    pub(super) appends: u64,
    pub(super) reopens: u64,
}

impl Workbench {
    fn follow_tab(&self, group: EditorGroupId) -> Option<(&Tab, &Arc<Document>)> {
        let tab = self.editor.groups.get(&group)?.current()?;
        if tab.kind != EditorKind::Text {
            return None;
        }
        let document = &tab.opened()?.document;
        self.disk_path(&document.resource)?;
        Some((tab, document))
    }

    fn toggle_follow_tail(&mut self, cx: &mut Context<Self>) {
        let Some((tab, document)) = self
            .follow_tab(self.editor.focused)
            .map(|(tab, document)| (tab.id, document.clone()))
        else {
            return;
        };
        if self.follow_tail.views.remove(&tab).is_none() {
            let id = document.id;
            let path = self
                .disk_path(&document.resource)
                .expect("followable disk path");
            let workspace_path = self.follow_workspace_path(&document.resource);
            self.follow_tail.views.insert(
                tab,
                FollowView {
                    document: id,
                    mode: FollowMode::Following,
                    last_end: None,
                },
            );
            self.follow_tail.documents.entry(id).or_insert_with(|| {
                let mut queue = ReloadQueue::default();
                queue.request(ReloadRequest::CheckAppend);
                FollowDocument {
                    path,
                    workspace_path,
                    queue,
                    watch: PathWatch::default(),
                    ordinary: None,
                    storage_ready: false,
                    ready: None,
                }
            });
        }
        cx.notify();
    }

    fn resume_follow_tail(&mut self, cx: &mut Context<Self>) {
        let Some((tab, _)) = self.follow_tab(self.editor.focused) else {
            return;
        };
        let id = tab.id;
        if let Some(follow) = self.follow_tail.views.get_mut(&id) {
            follow.mode = FollowMode::Following;
            follow.last_end = None;
            cx.notify();
        }
    }

    pub(super) fn follow_title_action(
        &self,
        group: EditorGroupId,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (tab, _) = self.follow_tab(group)?;
        let entry = self.shell.contributions.get(reg::TITLE_ID)?;
        let title = self.strings.text(&entry.title).ok()?;
        let icon = format!("$({})", entry.icon.as_deref()?);
        let action = entry.command.clone()?;
        let tab = tab.id;
        let active = self.follow_tail.views.contains_key(&tab);
        let menu = self
            .follow_tail
            .title_menu
            .filter(|(g, t, _)| *g == group && *t == tab);
        Some(
            div()
                .id(SharedString::from(format!("{}-{}", reg::TITLE_ID, group.0)))
                .debug_selector(move || format!("editor-title-follow-{}", group.0))
                .h_full()
                .px_1()
                .flex()
                .items_center()
                .cursor_pointer()
                .text_color(self.ui_color(if active {
                    "tab.activeForeground"
                } else {
                    "descriptionForeground"
                }))
                .hover(|style| style.bg(self.ui_color("tab.hoverBackground")))
                .child(shell::view::icon_label(&icon, px(14.0 * self.shell.zoom())))
                .tooltip(crate::metadata::tooltip::tab_tooltip(
                    title.to_string(),
                    self.ui_color("editorHoverWidget.background"),
                    self.ui_color("editorHoverWidget.foreground"),
                    self.ui_color("editorHoverWidget.border"),
                ))
                .on_click(cx.listener(move |view, e: &gpui::ClickEvent, _, cx| {
                    let open = view.follow_tail.title_menu.take().is_none();
                    if open {
                        view.follow_tail.title_menu = Some((group, tab, e.position()));
                    }
                    cx.notify();
                }))
                .when_some(menu, |button, (_, _, at)| {
                    let row = div()
                        .id("editor-title-follow-menu-action")
                        .debug_selector(|| "editor-title-follow-menu-action".to_owned())
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .hover(|style| style.bg(self.ui_color("menu.selectionBackground")))
                        .child(shell::view::icon_label(
                            if active { "$(check)" } else { "" },
                            px(13.0),
                        ))
                        .child(title)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.follow_tail.title_menu = None;
                            if view
                                .follow_tab(group)
                                .is_some_and(|(current, _)| current.id == tab)
                            {
                                view.editor.focus(group);
                                view.focus_group_code(group, window, cx);
                                view.command(&action, cx);
                            }
                            cx.stop_propagation();
                        }));
                    let menu = div()
                        .id("editor-title-follow-menu")
                        .border_1()
                        .border_color(self.ui_color("menu.border"))
                        .bg(self.ui_color("menu.background"))
                        .text_color(self.ui_color("menu.foreground"))
                        .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                            view.follow_tail.title_menu = None;
                            cx.notify();
                        }))
                        .child(row);
                    button.child(
                        gpui::deferred(
                            gpui::anchored()
                                .anchor(gpui::Corner::TopRight)
                                .position(at + gpui::point(px(0.0), px(12.0)))
                                .snap_to_window_with_margin(px(4.0))
                                .child(menu),
                        )
                        .with_priority(2),
                    )
                })
                .into_any_element(),
        )
    }

    pub(super) fn follow_workspace_events(&mut self, events: &[FileEvent]) {
        for follow in self.follow_tail.documents.values_mut() {
            let Some(path) = follow.workspace_path.as_ref() else {
                continue;
            };
            for event in events {
                let path_event = match event {
                    FileEvent::Changed(path) => PathEvent::Changed(path.as_str()),
                    FileEvent::Created(path) => PathEvent::Created(path.as_str()),
                    FileEvent::Removed(path) => PathEvent::Removed(path.as_str()),
                    FileEvent::Renamed { from, to } => PathEvent::Renamed {
                        from: from.as_str(),
                        to: to.as_str(),
                    },
                    FileEvent::RescanRequired => PathEvent::Rescan,
                };
                follow
                    .queue
                    .request(path_request(path.as_str(), path_event));
            }
        }
        // The existing poll drains the requests. Until it reads a changed
        // snapshot, disk events alone do not invalidate the editor frame.
    }

    fn follow_workspace_path(&self, resource: &Resource) -> Option<WorkspacePath> {
        match resource {
            Resource::Workspace(path) => Some(path.clone()),
            Resource::File(path) => super::explorer_file_ops::workspace_path(&self.root, path),
            Resource::Virtual(_) => None,
        }
    }

    fn active_follow_documents(&self) -> impl Iterator<Item = &Arc<Document>> {
        self.editor.groups.keys().filter_map(|&group| {
            let (tab, document) = self.follow_tab(group)?;
            let follow = self.follow_tail.views.get(&tab.id)?;
            (follow.document == document.id).then_some(document)
        })
    }

    pub(super) fn followed_resources(&self) -> Vec<Resource> {
        self.active_follow_documents()
            .map(|document| document.resource.clone())
            .collect()
    }

    pub(super) fn rebind_followed_document(&mut self, document: &Document, path: &std::path::Path) {
        let workspace_path = self.follow_workspace_path(&document.resource);
        let Some(follow) = self.follow_tail.documents.get_mut(&document.id) else {
            return;
        };
        follow.path = path.to_owned();
        follow.workspace_path = workspace_path;
        follow.queue = ReloadQueue::default();
        follow.queue.request(ReloadRequest::ReopenPath);
        follow.watch = PathWatch::default();
        follow.ordinary = None;
        follow.ready = None;
    }

    fn follow_path_reserved(&self, id: DocumentId) -> bool {
        self.follow_tail
            .documents
            .get(&id)
            .is_some_and(|follow| self.editing.saves.borrow().path_is_reserved(&follow.path))
    }

    pub(super) fn follows_workspace_path(&self, path: &WorkspacePath) -> bool {
        self.active_follow_documents()
            .any(|document| self.follow_workspace_path(&document.resource).as_ref() == Some(path))
    }

    pub(super) fn follows_disk_path(&self, path: &std::path::Path) -> bool {
        self.active_follow_documents()
            .any(|document| self.disk_path(&document.resource).as_deref() == Some(path))
    }

    pub(super) fn follow_user_scroll(
        &mut self,
        group: EditorGroupId,
        secondary: bool,
        old_top: f64,
        new_top: f64,
        at_end: bool,
        cx: &mut Context<Self>,
    ) {
        if secondary {
            return;
        }
        let Some(tab) = self.follow_tab(group).map(|(tab, _)| tab.id) else {
            return;
        };
        if let Some(follow) = self.follow_tail.views.get_mut(&tab) {
            if follow.mode.user_scrolled(old_top, new_top, at_end) {
                if follow.mode.follows() {
                    follow.last_end = None;
                }
                cx.notify();
            }
        }
    }

    pub(super) fn sync_follow_tail(&mut self, cx: &mut Context<Self>) {
        if self.follow_tail.views.is_empty() && self.follow_tail.documents.is_empty() {
            self.publish_follow_status(None);
            return;
        }
        let live: BTreeSet<_> = self
            .editor
            .groups
            .keys()
            .copied()
            .filter_map(|group| self.follow_tab(group).map(|(tab, doc)| (tab.id, doc.id)))
            .collect();
        self.follow_tail
            .views
            .retain(|tab, follow| live.contains(&(*tab, follow.document)));
        let tracked: BTreeSet<_> = self
            .follow_tail
            .views
            .values()
            .map(|follow| follow.document)
            .collect();
        self.follow_tail
            .documents
            .retain(|doc, _| tracked.contains(doc));

        let ready: Vec<_> = self
            .follow_tail
            .documents
            .iter_mut()
            .filter_map(|(&id, doc)| doc.ready.take().map(|(token, result)| (id, token, result)))
            .collect();
        for (id, token, result) in ready {
            self.apply_follow_result(id, token, result, cx);
        }
        for id in tracked {
            self.start_follow_read(id, cx);
        }
        self.poll_follow_paths(cx);
        self.follow_views_to_end(true, cx);
        let mode = self
            .follow_tab(self.editor.focused)
            .and_then(|(tab, document)| {
                self.follow_tail
                    .documents
                    .get(&document.id)
                    .filter(|read| read.storage_ready)?;
                self.follow_tail.views.get(&tab.id)
            })
            .map(|follow| follow.mode);
        self.publish_follow_status(mode);
    }

    /// Windows may delay last-write events while the writer keeps its handle open.
    /// Use the same coalesced read queue for a periodic size/identity check.
    fn poll_follow_paths(&mut self, cx: &mut Context<Self>) {
        if self.follow_tail.polling || self.follow_tail.documents.is_empty() {
            return;
        }
        self.follow_tail.polling = true;
        let timer = cx.background_executor().timer(Duration::from_millis(100));
        cx.spawn(async move |weak, cx| {
            timer.await;
            let _ = weak.update(cx, |view, cx| {
                view.follow_tail.polling = false;
                let ids: Vec<_> = view.follow_tail.documents.keys().copied().collect();
                for id in ids {
                    if let Some(follow) = view.follow_tail.documents.get_mut(&id) {
                        follow.queue.request(ReloadRequest::PollPath);
                    }
                    view.start_follow_read(id, cx);
                }
                view.poll_follow_paths(cx);
            });
        })
        .detach();
    }

    fn start_follow_read(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        if self.follow_path_reserved(id) {
            return;
        }
        let Some(document) = self
            .active_follow_documents()
            .find(|document| document.id == id)
            .cloned()
        else {
            return;
        };
        let (saved, clean, can_append) = {
            let saves = self.editing.saves.borrow();
            saves.file(id).map_or((None, false, false), |file| {
                let clean = !file.is_dirty();
                (
                    file.disk_stamp().cloned(),
                    clean,
                    clean && *file.encoding() == Encoding::Utf8,
                )
            })
        };
        let Some(follow) = self.follow_tail.documents.get_mut(&id) else {
            return;
        };
        let Some(token) = follow.queue.start() else {
            return;
        };
        let policy = self.editor.settings.size;
        let job = ReadJob {
            document,
            path: follow.path.clone(),
            settings: self.settings.clone(),
            policy: rove_buffer::SizePolicy {
                huge_bytes: policy.huge_bytes,
                optimize: policy.optimize,
            },
            request: token.request,
            watch: follow.watch.clone(),
            mapped: self.metadata.files.mapped(id),
            ordinary: follow.ordinary.take(),
            saved,
            clean,
            can_append,
        };
        let task = cx
            .background_executor()
            .spawn(async move { tail_read::read(job) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |view, cx| {
                if matches!(
                    result.change,
                    Ok(TailChange::Unchanged | TailChange::Missing) | Err(_)
                ) {
                    view.apply_follow_result(id, token, result, cx);
                    return;
                }
                if let Some(follow) = view
                    .follow_tail
                    .documents
                    .get_mut(&id)
                    .filter(|follow| follow.queue.accepts(token))
                {
                    follow.ready = Some((token, result));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn apply_follow_result(
        &mut self,
        id: DocumentId,
        token: Refresh,
        result: ReadResult,
        cx: &mut Context<Self>,
    ) {
        let reserved = self.follow_path_reserved(id);
        let Some(follow) = self.follow_tail.documents.get_mut(&id) else {
            return;
        };
        if reserved && follow.queue.accepts(token) {
            follow.queue.complete(token);
            follow.queue.request(ReloadRequest::ReopenPath);
            return;
        }
        if !follow.queue.complete(token) {
            return;
        }
        let Some(current) = self
            .active_follow_documents()
            .find(|document| document.id == id)
            .cloned()
        else {
            return;
        };
        if current.text.version() != result.origin {
            self.request_follow_retry(id);
            return;
        }
        let follow = self.follow_tail.documents.get_mut(&id).unwrap();
        let watch_changed = result.watch.generation() != result.generation;
        follow.watch = result.watch;
        if watch_changed {
            self.request_follow_retry(id);
            return;
        }
        if result.storage_ready && !matches!(&result.change, Ok(TailChange::Missing) | Err(_)) {
            follow.watch.accepted = result.generation;
            follow.watch.has_baseline = true;
            if !follow.storage_ready {
                follow.storage_ready = true;
                cx.notify();
            }
        }
        follow.ordinary = result.baseline;
        match result.change {
            // Indexing can advance the same snapshot without any disk change
            // or a workbench render. Only move views whose known tail changed.
            Ok(TailChange::Unchanged) => self.follow_views_to_end(false, cx),
            Ok(TailChange::Missing) | Err(_) => {}
            Ok(TailChange::MappedAppend(mapped)) => {
                self.follow_tail.counts.appends += 1;
                self.publish_tail_text(
                    &current,
                    FileText::Mapped(mapped),
                    current.metadata.clone(),
                    true,
                    cx,
                );
            }
            Ok(TailChange::Reopened(opened)) => {
                if self.editing.saves.borrow().is_dirty(id) {
                    self.request_follow_retry(id);
                    return;
                }
                self.follow_tail.counts.reopens += 1;
                let mut metadata = opened.metadata;
                let mut reasons = metadata.read_only.to_vec();
                for reason in current.metadata.read_only.iter() {
                    if !matches!(reason, ReadOnlyReason::HugeFile) && !reasons.contains(reason) {
                        reasons.push(reason.clone());
                    }
                }
                metadata.read_only = reasons.into();
                self.publish_tail_text(&current, opened.text, metadata, false, cx);
            }
            Ok(TailChange::OrdinaryAppend {
                base,
                next,
                suffixed,
                endings,
            }) => {
                let applied = self
                    .editing
                    .saves
                    .borrow_mut()
                    .file_mut(id)
                    .is_some_and(|file| {
                        file.append_from_disk(
                            &base,
                            next,
                            &suffixed,
                            endings.line_endings(),
                            Instant::now(),
                        )
                        .is_ok()
                    });
                if applied {
                    self.follow_tail.counts.appends += 1;
                    self.sync_document_from_session(id, cx);
                } else {
                    self.request_follow_retry(id);
                }
            }
            Ok(TailChange::Disk(stamp, bytes)) => {
                let effects = self
                    .editing
                    .saves
                    .borrow_mut()
                    .apply_disk(
                        id,
                        Some((stamp, bytes)),
                        self.settings.as_ref(),
                        &mut self.shell.feedback,
                        Instant::now(),
                    )
                    .unwrap_or_default();
                if !effects.is_empty() {
                    self.follow_tail.counts.reopens += 1;
                    self.apply_save_effects(effects, cx);
                }
            }
        }
    }

    fn request_follow_retry(&mut self, id: DocumentId) {
        if let Some(doc) = self.follow_tail.documents.get_mut(&id) {
            doc.ordinary = None;
            doc.queue.request(ReloadRequest::ReopenPath);
        }
    }

    fn publish_tail_text(
        &mut self,
        current: &Document,
        text: FileText,
        metadata: rove_buffer::DocumentMetadata,
        appended: bool,
        cx: &mut Context<Self>,
    ) {
        let next = Arc::new(Document {
            id: current.id,
            resource: current.resource.clone(),
            metadata,
            text: text.source(),
        });
        if self.editor.replace_document(next.clone(), None) {
            if next.metadata.size == rove_buffer::SizeClass::Huge {
                self.editing.saves.borrow_mut().detach(next.id);
            }
            self.metadata.files.remember_mapped(next.id, text.mapped());
            self.metadata.files.replace(&next);
            if appended {
                for group in self.views.values_mut() {
                    for slot in std::iter::once(&mut group.primary).chain(group.secondary.as_mut())
                    {
                        let Some(shown) = slot.shown.as_mut().filter(|shown| {
                            shown.document == current.id && shown.version == current.text.version()
                        }) else {
                            continue;
                        };
                        shown.version = next.text.version();
                        let text = next.text.clone();
                        slot.code
                            .update(cx, |code, cx| code.set_appended_text(text, cx));
                    }
                }
            }
            self.sync_views(cx);
        }
    }

    fn follow_views_to_end(&mut self, frame_pending: bool, cx: &mut Context<Self>) {
        let targets: Vec<_> = self
            .editor
            .groups
            .iter()
            .filter_map(|(&group, pane)| {
                let tab = pane.current()?;
                let follow = self.follow_tail.views.get(&tab.id)?;
                if !follow.mode.follows() {
                    return None;
                }
                let document = &tab.opened()?.document;
                if !self.follow_tail.documents.get(&document.id)?.storage_ready {
                    return None;
                }
                let end = (
                    document.text.version(),
                    document.text.lines().known.saturating_sub(1),
                );
                if follow.last_end == Some(end) {
                    return None;
                }
                Some((tab.id, end, self.group_code(group, false)?.clone()))
            })
            .collect();
        for (tab, end, code) in targets {
            if let Some(follow) = self.follow_tail.views.get_mut(&tab) {
                follow.last_end = Some(end);
            }
            code.update(cx, |code, cx| {
                code.follow_end(end.1, frame_pending, cx);
            });
        }
    }

    fn publish_follow_status(&mut self, mode: Option<FollowMode>) {
        if self.follow_tail.status == mode {
            return;
        }
        self.follow_tail.status = mode;
        let Some(mode) = mode else {
            self.shell.status_bar.remove(reg::FOLLOW_STATUS);
            return;
        };
        let text = self.label(
            if mode.follows() {
                "followTail.active"
            } else {
                "followTail.paused"
            },
            &[],
        );
        let tooltip = self.label("followTail.tooltip", &[]);
        let _ = self.shell.status_bar.set(
            &self.shell.contributions,
            reg::FOLLOW_STATUS,
            StatusEntry {
                text,
                tooltip: Some(tooltip),
                ..StatusEntry::default()
            },
        );
    }
}
