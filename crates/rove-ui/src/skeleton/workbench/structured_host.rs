//! Structured-document parsing and editor commands in the Workbench.
use super::*;
use crate::{
    documents::OpenTarget,
    list_widget::{tree::TreeModel, widget::ListEvent},
    lists::ItemId,
    outline::{
        structure::{self as s, Entry, StructureTree},
        view::Status,
    },
};
use gpui::ClipboardItem;
use rove_base::{ByteOffset, ByteRange, Position};
use rove_buffer::{edit::Transaction, Eol, LineEndings, SizeClass, TextSource};
use rove_docview::{
    format::{format_selection, format_text, FormatMode, FormatOptions},
    structure::{ParsedStructure, ProblemKind, StructuredLanguage},
    Structure, StructureNode, StructureNodeId,
};

#[derive(Clone, Copy)]
enum ParseCause {
    Outline,
    Replacement,
}

impl Workbench {
    pub(super) fn retain_structured_documents(
        &mut self,
        open: &std::collections::HashSet<DocumentId>,
        cx: &mut Context<Self>,
    ) {
        self.structured
            .replacement_parses
            .retain(|id, (_, cancel)| {
                let remains_open = open.contains(id);
                if !remains_open {
                    cancel.cancel();
                }
                remains_open
            });
        let closed: Vec<_> = self
            .structured
            .documents
            .iter()
            .filter_map(|(id, entry)| (!open.contains(id)).then_some((*id, entry.resource.clone())))
            .collect();
        if !closed.is_empty() {
            for (id, resource) in closed {
                self.structured.documents.remove(&id);
                if let Some((_, cancel)) = self.structured.diagnostics.remove(&id) {
                    cancel.cancel();
                }
                self.structured.edits.remove(&id);
                self.structured.expanded_paths.remove(&id);
                self.publish_problem_decorations(&resource, 0, cx);
            }
            self.update_problem_count();
        }
    }

    /// Returns true when this document uses the structured outline.
    pub(super) fn sync_structured_outline(
        &mut self,
        document: &Arc<Document>,
        cursor: Position,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(language) = StructuredLanguage::from_id(&document.metadata.language) else {
            self.reset_structured_outline();
            return false;
        };
        if document.metadata.size == SizeClass::Huge {
            self.reset_structured_outline();
            return false;
        }
        let id = document.id;
        let version = document.text.version();
        let limit = self.structure_limit(language);
        if let Some(entry) = self
            .structured
            .documents
            .get(&id)
            .filter(|entry| entry.snapshot.version() == version && entry.limit == limit)
        {
            let snapshot = entry.snapshot.clone();
            let key = (id, version);
            if self.structured.current != Some(key) {
                let roots = snapshot.roots().unwrap_or_else(|_| Arc::from([]));
                self.install_structured_outline(key, &roots, cx);
            }
            self.update_structured_cursor_path(id, cursor, snapshot, cx);
        } else {
            if self
                .structured
                .current
                .is_some_and(|(shown, _)| shown != id)
            {
                self.reset_structured_outline();
            }
            if self.structured.current.is_none() {
                self.outline.clear(Status::Loading, cx);
            }
            if self.structured.failed != Some((id, version))
                && !self
                    .structured
                    .pending
                    .as_ref()
                    .is_some_and(|(pending, revision, _)| *pending == id && *revision == version)
                && !self
                    .structured
                    .replacement_parses
                    .get(&id)
                    .is_some_and(|(revision, _)| *revision == version)
            {
                self.request_structured_parse(document.clone(), language, ParseCause::Outline, cx);
            }
        }
        true
    }

    pub(super) fn reset_structured_outline(&mut self) {
        if let Some((_, _, cancel)) = self.structured.pending.take() {
            cancel.cancel();
        }
        self.clear_structured_tree();
    }

    fn clear_structured_tree(&mut self) {
        self.structured.current = None;
        self.structured.nodes.clear();
        *self.outline.structure_style.borrow_mut() = Default::default();
        self.structured.path = None;
        self.structured.path_request = None;
        if let Some(cancel) = self.structured.path_cancel.take() {
            cancel.cancel();
        }
        self.structured.context_menu = None;
        for (_, cancel) in std::mem::take(&mut self.structured.expansions) {
            cancel.cancel();
        }
    }

    pub(super) fn note_structure_edit(&mut self, id: DocumentId, edit: rove_base::TextEdit) {
        if self
            .structured
            .documents
            .get(&id)
            .is_some_and(|entry| entry.snapshot.version() == edit.version)
        {
            self.structured.edits.insert(id, edit);
        } else {
            self.structured.edits.remove(&id);
        }
    }

    fn structure_limit(&self, language: StructuredLanguage) -> usize {
        const DEFAULT: usize = 5_000;
        if !matches!(
            language,
            StructuredLanguage::Json | StructuredLanguage::Jsonc
        ) {
            return DEFAULT;
        }
        crate::code_view::options::json_item_limit(self.settings.as_ref())
    }

    pub(super) fn structured_limit_changed(&mut self) {
        if let Some((_, _, cancel)) = self.structured.pending.take() {
            cancel.cancel();
        }
        for (_, cancel) in std::mem::take(&mut self.structured.replacement_parses).into_values() {
            cancel.cancel();
        }
        self.structured.failed = None;
        self.clear_structured_tree();
    }

    pub(super) fn refresh_replaced_structure(&mut self, id: DocumentId, cx: &mut Context<Self>) {
        let Some(document) = self.open_document(id) else {
            return;
        };
        let Some(language) = StructuredLanguage::from_id(&document.metadata.language) else {
            return;
        };
        if document.metadata.size != SizeClass::Huge {
            self.request_structured_parse(document, language, ParseCause::Replacement, cx);
        }
    }

    fn request_structured_parse(
        &mut self,
        document: Arc<Document>,
        language: StructuredLanguage,
        cause: ParseCause,
        cx: &mut Context<Self>,
    ) {
        let id = document.id;
        let version = document.text.version();
        let outline_superseded = matches!(cause, ParseCause::Outline)
            || self
                .structured
                .pending
                .as_ref()
                .is_some_and(|(pending, _, _)| *pending == id);
        if let Some((_, _, old)) = outline_superseded
            .then(|| self.structured.pending.take())
            .flatten()
        {
            old.cancel();
        }
        if let Some((_, old)) = self.structured.replacement_parses.remove(&id) {
            old.cancel();
        }
        if let Some((_, cancel)) = self.structured.diagnostics.remove(&id) {
            cancel.cancel();
        }
        let limit = self.structure_limit(language);
        let cancel = Cancellation::default();
        match cause {
            ParseCause::Outline => self.structured.pending = Some((id, version, cancel.clone())),
            ParseCause::Replacement => {
                self.structured
                    .replacement_parses
                    .insert(id, (version, cancel.clone()));
            }
        }
        let completion = cancel.clone();
        let incremental = self.structured.edits.remove(&id).and_then(|edit| {
            self.structured
                .documents
                .get(&id)
                .filter(|entry| entry.snapshot.version() == edit.version && entry.limit == limit)
                .map(|entry| (entry.snapshot.clone(), edit))
        });
        let task = cx.background_executor().spawn(async move {
            let snapshot = if let Some((previous, edit)) = incremental {
                previous
                    .reparse(&edit.range, &edit.inserted, version, &cancel)?
                    .snapshot
            } else {
                let text = document_text(&*document.text, None, &cancel)?;
                ParsedStructure::parse(language, version, text.into(), limit, &cancel)?
            };
            Ok::<_, Error>(Entry {
                snapshot,
                limit,
                resource: document.resource.clone(),
                problems: Vec::new(),
            })
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |view, cx| {
                if completion.check().is_ok() && view.structure_limit(language) == limit {
                    view.finish_structured_parse(id, version, cause, result, cx);
                }
            });
        })
        .detach();
    }

    fn finish_structured_parse(
        &mut self,
        id: DocumentId,
        version: Version,
        cause: ParseCause,
        result: CoreResult<Entry>,
        cx: &mut Context<Self>,
    ) {
        match cause {
            ParseCause::Outline
                if self
                    .structured
                    .pending
                    .as_ref()
                    .is_some_and(|(doc, rev, _)| *doc == id && *rev == version) =>
            {
                self.structured.pending = None;
            }
            ParseCause::Replacement
                if self
                    .structured
                    .replacement_parses
                    .get(&id)
                    .is_some_and(|(rev, _)| *rev == version) =>
            {
                self.structured.replacement_parses.remove(&id);
            }
            _ => return,
        }
        if self
            .open_document(id)
            .is_none_or(|doc| doc.text.version() != version)
        {
            return;
        }
        match result {
            Ok(entry) => {
                let snapshot = entry.snapshot.clone();
                self.publish_problem_decorations(&entry.resource, 0, cx);
                self.structured.documents.insert(id, entry);
                self.update_problem_count();
                self.analyze_structured_diagnostics(id, version, snapshot, cx);
            }
            Err(error) => {
                if matches!(cause, ParseCause::Outline)
                    && !matches!(error, Error::Cancelled | Error::Stale)
                {
                    self.structured.failed = Some((id, version));
                }
            }
        }
        cx.notify();
    }

    fn analyze_structured_diagnostics(
        &mut self,
        id: DocumentId,
        version: Version,
        snapshot: ParsedStructure,
        cx: &mut Context<Self>,
    ) {
        let cancel = Cancellation::default();
        self.structured
            .diagnostics
            .insert(id, (version, cancel.clone()));
        let completion = cancel.clone();
        let task = cx
            .background_executor()
            .spawn(async move { snapshot.diagnostics(&cancel) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |view, cx| {
                if completion.check().is_err()
                    || !view
                        .structured
                        .diagnostics
                        .get(&id)
                        .is_some_and(|(v, _)| *v == version)
                {
                    return;
                }
                view.structured.diagnostics.remove(&id);
                let Ok(problems) = result else {
                    return;
                };
                if view
                    .open_document(id)
                    .is_none_or(|doc| doc.text.version() != version)
                {
                    return;
                }
                let Some(entry) = view
                    .structured
                    .documents
                    .get_mut(&id)
                    .filter(|entry| entry.snapshot.version() == version)
                else {
                    return;
                };
                if view.structured.current == Some((id, version)) {
                    view.outline.structure_style.borrow_mut().problems = problems.clone().into();
                }
                entry.problems = problems;
                let count = entry.problems.len();
                let resource = entry.resource.clone();
                view.publish_problem_decorations(&resource, count, cx);
                view.update_problem_count();
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn sync_structured_outline_style(&mut self) {
        let style = &self.outline.structure_style;
        let mut style = style.borrow_mut();
        let visible = self.structured_bool("problems.visibility", true)
            && self.structured_bool("outline.problems.enabled", true);
        style.badges = visible && self.structured_bool("outline.problems.badges", true);
        style.colors = visible && self.structured_bool("outline.problems.colors", true);
    }

    pub(super) fn sync_current_problem(&mut self) {
        let visible = self.structured_bool("problems.visibility", true);
        self.shell.problems_visible = visible;
        self.shell.current_problem = (visible
            && self.structured_bool("problems.showCurrentInStatus", false))
        .then(|| self.focused_text())
        .flatten()
        .and_then(|(_, doc, at)| {
            let entry = self.structured.documents.get(&doc.id)?;
            if entry.snapshot.version() != doc.text.version() {
                return None;
            }
            let start = entry
                .problems
                .partition_point(|p| p.range.start().line < at.line);
            entry.problems[start..]
                .iter()
                .take_while(|p| p.range.start().line == at.line)
                .find(|p| p.range.start() <= at && at <= p.range.end())
                .map(|p| Message::key(problem_key(p.kind)))
        });
    }

    /// Only polish slots whose shown revision or active theme actually changed.
    pub(super) fn sync_problem_marks(&mut self, cx: &mut Context<Self>) {
        use crate::code_view::{
            minimap::{Marker, MarkerKind},
            InlineDecoration, LayerId, LineDecorations, TextDecoration, TextStyle,
        };
        let color = self.theme.color("editorError.foreground").ok().flatten();
        let visible = self.structured_bool("problems.visibility", true);
        for slot in self
            .views
            .values_mut()
            .flat_map(|group| std::iter::once(&mut group.primary).chain(group.secondary.iter_mut()))
        {
            let active = slot.shown.and_then(|shown| {
                let entry = self.structured.documents.get(&shown.document)?;
                (visible && entry.snapshot.version() == shown.version && !entry.problems.is_empty())
                    .then_some((shown, entry))
            });
            let key = active.map(|(shown, _)| (shown.document, shown.version, color));
            if slot.structured_mark == key {
                continue;
            }
            slot.structured_mark = key;
            slot.code.update(cx, |code, cx| {
                let layer = LayerId("structure-problems".into());
                code.remove_decorations(&layer, cx);
                let Some((_, entry)) = active else {
                    code.set_minimap_markers("structure-problems", Vec::new(), cx);
                    return;
                };
                let markers = entry
                    .problems
                    .iter()
                    .map(|problem| Marker {
                        lines: problem.range.start().line.0
                            ..problem.range.end().line.0.saturating_add(1),
                        kind: MarkerKind::Error,
                    })
                    .collect();
                code.set_minimap_markers("structure-problems", markers, cx);
                let Some(color) = color else {
                    return;
                };
                let problems = entry.problems.clone();
                let _ = code.add_decorations(
                    "structure-problems",
                    45,
                    std::rc::Rc::new(move |line: u64, text: &str, out: &mut LineDecorations| {
                        let first = problems.partition_point(|p| p.range.start().line.0 < line);
                        for p in problems[first..]
                            .iter()
                            .take_while(|p| p.range.start().line.0 == line)
                        {
                            let mut start = (p.range.start().column.0 as usize).min(text.len());
                            let mut end = if p.range.end().line.0 == line {
                                (p.range.end().column.0 as usize).min(text.len())
                            } else {
                                text.len()
                            };
                            if end == start {
                                if let Some(c) =
                                    text.get(start..).and_then(|tail| tail.chars().next())
                                {
                                    end += c.len_utf8();
                                } else if let Some(c) =
                                    text.get(..start).and_then(|head| head.chars().next_back())
                                {
                                    start -= c.len_utf8();
                                }
                            }
                            if start < end {
                                out.inline.push(InlineDecoration {
                                    range: start as u32..end as u32,
                                    style: TextStyle {
                                        underline: Some(TextDecoration::Wavy(color)),
                                        ..Default::default()
                                    },
                                });
                            }
                        }
                    }),
                    cx,
                );
            });
        }
    }

    pub(super) fn refresh_problem_decorations(&self, cx: &mut Context<Self>) {
        for entry in self.structured.documents.values() {
            self.publish_problem_decorations(&entry.resource, entry.problems.len(), cx);
        }
    }

    fn publish_problem_decorations(
        &self,
        resource: &Resource,
        errors: usize,
        cx: &mut Context<Self>,
    ) {
        if let Resource::Workspace(path) = resource {
            let enabled = self.structured_bool("problems.visibility", true)
                && self.structured_bool("problems.decorations.enabled", true);
            self.explorer.update(cx, |explorer, cx| {
                explorer
                    .problems()
                    .set(path, if enabled { errors as u32 } else { 0 }, 0);
                cx.notify();
            });
        }
    }

    pub(super) fn update_problem_count(&mut self) {
        let errors = self
            .structured
            .documents
            .values()
            .map(|entry| entry.problems.len())
            .sum::<usize>();
        let (settings_errors, settings_warnings) = self.settings_error_counts();
        self.shell
            .set_problem_counts((errors + settings_errors) as u32, settings_warnings as u32);
    }

    fn install_structured_outline(
        &mut self,
        current: (DocumentId, Version),
        roots: &[StructureNode],
        cx: &mut Context<Self>,
    ) {
        self.clear_structured_tree();
        self.structured.current = Some(current);
        self.outline.shown = None;
        self.outline.status = Status::Ready;
        self.structured
            .nodes
            .extend(roots.iter().map(|node| (node.id.0, node.clone())));
        self.outline
            .structure_style
            .borrow_mut()
            .symbols
            .extend(roots.iter().map(|node| (node.id.0, node.symbol.clone())));
        if let Some(entry) = self.structured.documents.get(&current.0) {
            self.outline.structure_style.borrow_mut().problems = entry.problems.clone().into();
        }
        self.sync_structured_outline_style();
        let rows = s::items(roots, &self.outline.settings.filter, &self.outline.label);
        self.outline.list.update(cx, |list, cx| {
            list.widget
                .update_tree(|tree| *tree = TreeModel::new(Arc::new(StructureTree::new(rows))));
            list.widget.take_events();
            cx.notify();
        });
        self.restore_structured_expansions(current, roots, cx);
    }

    fn restore_structured_expansions(
        &mut self,
        (document, version): (DocumentId, Version),
        nodes: &[StructureNode],
        cx: &mut Context<Self>,
    ) {
        let Some(paths) = self.structured.expanded_paths.get(&document) else {
            return;
        };
        let expanded: Vec<_> = nodes
            .iter()
            .filter(|node| node.children.is_none() && paths.contains(&node.path))
            .map(|node| (node.id, s::item_id(node.id)))
            .collect();
        for (node, item) in expanded {
            let expanded = self.outline.list.update(cx, |list, cx| {
                let expanded = list.widget.expand_id(&item, true).unwrap_or(false);
                list.widget.take_events();
                cx.notify();
                expanded
            });
            if expanded {
                self.request_structured_children(document, version, node, item, cx);
            }
        }
    }

    fn update_structured_cursor_path(
        &mut self,
        id: DocumentId,
        cursor: Position,
        snapshot: ParsedStructure,
        cx: &mut Context<Self>,
    ) {
        let requested = (id, snapshot.version(), cursor);
        if self.structured.path_request == Some(requested) {
            return;
        }
        self.structured.path_request = Some(requested);
        let cancel = Cancellation::default();
        if let Some(old) = self.structured.path_cancel.replace(cancel.clone()) {
            old.cancel();
        }
        let task = cx
            .background_executor()
            .spawn(async move { snapshot.node_at(cursor, &cancel).ok().flatten() });
        cx.spawn(async move |weak, cx| {
            let node = task.await;
            let _ = weak.update(cx, |view, cx| {
                if view.structured.path_request != Some(requested) {
                    return;
                }
                view.structured.path = node
                    .as_ref()
                    .map(|node| (requested.0, requested.1, requested.2, node.path.clone()));
                if let Some(node) = node.filter(|_| view.outline.follow_cursor) {
                    let mut ancestors: Vec<_> = view
                        .structured
                        .nodes
                        .values()
                        .filter(|candidate| {
                            candidate.symbol.range.start() <= cursor
                                && cursor <= candidate.symbol.range.end()
                        })
                        .map(|candidate| (candidate.path.len(), s::item_id(candidate.id)))
                        .collect();
                    ancestors.sort_unstable_by_key(|(depth, _)| std::cmp::Reverse(*depth));
                    view.outline.list.update(cx, |list, cx| {
                        let item = s::item_id(node.id);
                        if let Some(item) = std::iter::once(item)
                            .chain(ancestors.into_iter().map(|(_, item)| item))
                            .find(|item| list.widget.focus_id(item).is_ok())
                        {
                            list.widget.select_ids(&[item]);
                        }
                        cx.notify();
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn structured_outline_event(
        &mut self,
        event: &ListEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((document, version)) = self.structured.current else {
            return false;
        };
        if self
            .focused_text()
            .is_none_or(|(_, active, _)| active.id != document || active.text.version() != version)
        {
            return true;
        }
        match event {
            ListEvent::Expanded { id, expanded } => {
                if let Some(node) = s::node_id(id) {
                    let paths = self.structured.expanded_paths.entry(document).or_default();
                    if let Some(row) = self.structured.nodes.get(&node.0) {
                        if *expanded {
                            paths.insert(row.path.clone());
                        } else {
                            paths.remove(&row.path);
                        }
                    }
                    if *expanded {
                        self.request_structured_children(document, version, node, id.clone(), cx);
                    } else if let Some(cancel) = self.structured.expansions.remove(&node.0) {
                        cancel.cancel();
                    }
                }
            }
            ListEvent::ContextMenu(ids) => {
                self.structured.context_menu = ids.first().and_then(s::node_id);
                cx.notify();
            }
            ListEvent::SelectionChanged(ids) if ids.len() == 1 => {
                if let Some(node) =
                    s::node_id(&ids[0]).and_then(|id| self.structured.nodes.get(&id.0))
                {
                    let position = node.symbol.selection.start();
                    if let Some(entry) = self.structured.documents.get(&document) {
                        self.open_structured_location(
                            entry.resource.clone(),
                            position,
                            OpenTarget::Active,
                            cx,
                        );
                    }
                }
            }
            ListEvent::Open {
                ids, side_by_side, ..
            } => {
                if let Some(node) = ids
                    .first()
                    .and_then(s::node_id)
                    .and_then(|id| self.structured.nodes.get(&id.0))
                {
                    let position = node.symbol.selection.start();
                    if let Some(entry) = self.structured.documents.get(&document) {
                        let target = if *side_by_side {
                            OpenTarget::Beside
                        } else {
                            OpenTarget::Active
                        };
                        self.open_structured_location(entry.resource.clone(), position, target, cx);
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn request_structured_children(
        &mut self,
        document: DocumentId,
        version: Version,
        node: StructureNodeId,
        item: ItemId,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.structured.documents.get(&document) else {
            return;
        };
        if self.structured.expansions.contains_key(&node.0) {
            return;
        }
        let cancel = Cancellation::default();
        self.structured.expansions.insert(node.0, cancel.clone());
        let task =
            cx.background_executor()
                .spawn(entry.snapshot.children(node, version, cancel.clone()));
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |view, cx| {
                if view.structured.current != Some((document, version)) || cancel.check().is_err() {
                    return;
                }
                view.structured.expansions.remove(&node.0);
                let Ok(children) = result else {
                    return;
                };
                view.structured
                    .nodes
                    .extend(children.iter().map(|node| (node.id.0, node.clone())));
                view.outline
                    .structure_style
                    .borrow_mut()
                    .symbols
                    .extend(children.iter().map(|node| (node.id.0, node.symbol.clone())));
                let rows = s::items(
                    &children,
                    &view.outline.settings.filter,
                    &view.outline.label,
                );
                view.outline.list.update(cx, |list, cx| {
                    list.widget.update_tree(|tree| {
                        let _ = tree.set_children(Some(&item), rows);
                    });
                    cx.notify();
                });
                view.restore_structured_expansions((document, version), &children, cx);
                view.structured.path_request = None;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn open_structured_location(
        &mut self,
        resource: Resource,
        at: Position,
        target: OpenTarget,
        cx: &mut Context<Self>,
    ) {
        let _ = self.open_documents().open(
            resource,
            OpenOptions {
                source: OpenSource::Outline,
                target,
                preserve_focus: true,
                selection: TextRange::new(at, at).ok(),
                ..Default::default()
            },
            Cancellation::default(),
        );
        cx.notify();
    }

    pub(super) fn structured_bool(&self, id: &'static str, fallback: bool) -> bool {
        Key::<bool>::new(id)
            .read(&*self.settings, &Scope::default())
            .unwrap_or(fallback)
    }

    pub(super) fn structured_context_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.structured.context_menu?;
        let mut menu = div()
            .id("structure-context-menu")
            .absolute()
            .right_3()
            .top_6()
            .min_w(px(130.0))
            .p_1()
            .flex()
            .flex_col()
            .border_1()
            .rounded_sm()
            .occlude()
            .bg(self.ui_color("menu.background"))
            .border_color(self.ui_color("menu.border"))
            .text_color(self.ui_color("menu.foreground"))
            .on_mouse_down_out(cx.listener(|view, _: &gpui::MouseDownEvent, _, cx| {
                view.structured.context_menu = None;
                cx.notify();
            }));
        let selected = self.ui_color("menu.selectionBackground");
        for (command, title) in [
            (s::COPY_PATH, "command.rove.structure.copyPath"),
            (s::COPY_VALUE, "command.rove.structure.copyValue"),
            (s::COLLAPSE_SIBLINGS, "structure.collapseSiblings"),
        ] {
            menu = menu.child(
                div()
                    .id(command)
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(move |item| item.bg(selected))
                    .child(self.label(title, &[]))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.structured_command(command, cx);
                        view.structured.context_menu = None;
                        cx.notify();
                    })),
            );
        }
        Some(menu.into_any_element())
    }

    pub(super) fn structured_command(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        match id {
            s::FORMAT | s::FORMAT_NONE | s::FORMAT_SELECTION | s::MINIFY => {
                self.format_structured(id, cx)
            }
            s::COPY_PATH | s::COPY_VALUE | s::COLLAPSE_SIBLINGS => {
                let shown = self.structured.current;
                if self.focused_text().is_none_or(|(_, document, _)| {
                    shown != Some((document.id, document.text.version()))
                }) {
                    return true;
                }
                let selected = self.outline.list.read(cx).widget.selected_ids();
                let target = self
                    .structured
                    .context_menu
                    .take()
                    .or_else(|| selected.first().and_then(s::node_id));
                if let Some(node) = target.and_then(|id| self.structured.nodes.get(&id.0)) {
                    match id {
                        s::COPY_PATH => {
                            cx.write_to_clipboard(ClipboardItem::new_string(node.path.to_string()))
                        }
                        s::COPY_VALUE => {
                            let value = self
                                .structured
                                .current
                                .and_then(|(id, _)| self.structured.documents.get(&id))
                                .and_then(|entry| entry.snapshot.value(node.id).ok());
                            if let Some(value) = value {
                                cx.write_to_clipboard(ClipboardItem::new_string(value.to_owned()));
                            }
                        }
                        _ => {
                            let item = s::item_id(node.id);
                            let mut collapsed = Vec::new();
                            self.outline.list.update(cx, |list, cx| {
                                list.widget.update_tree(|tree| {
                                    let Some(row) = tree.index_of(&item) else {
                                        return;
                                    };
                                    let parent = tree.parent_index(row);
                                    let span = tree.scope_range(parent);
                                    let siblings: Vec<_> = span
                                        .filter(|i| tree.parent_index(*i) == parent)
                                        .filter_map(|i| tree.id_at(i))
                                        .filter(|id| id != &item)
                                        .collect();
                                    for sibling in siblings {
                                        if tree.set_expanded(&sibling, false).unwrap_or(false) {
                                            collapsed.extend(s::node_id(&sibling));
                                        }
                                    }
                                });
                                cx.notify();
                            });
                            let (document, _) = shown.expect("verified outline version");
                            let paths = self.structured.expanded_paths.entry(document).or_default();
                            for id in collapsed {
                                if let Some(node) = self.structured.nodes.get(&id.0) {
                                    paths.remove(&node.path);
                                }
                                if let Some(cancel) = self.structured.expansions.remove(&id.0) {
                                    cancel.cancel();
                                }
                            }
                        }
                    }
                }
            }
            s::NEXT | s::PREVIOUS | s::NEXT_FILES | s::PREVIOUS_FILES => {
                self.navigate_structured_problem(id, cx)
            }
            s::CLOSE_MARKER => {
                self.close_structured_marker(cx);
            }
            _ => return false,
        }
        true
    }

    pub(super) fn format_before_save(
        &mut self,
        id: DocumentId,
        request: &rove_buffer::save::SaveRequest,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(document) = self.open_document(id) else {
            return false;
        };
        let Some(language) = StructuredLanguage::from_id(&document.metadata.language) else {
            return false;
        };
        if document.metadata.size == SizeClass::Huge
            || !Key::<bool>::new("editor.formatOnSave")
                .read(
                    &*self.settings,
                    &Scope {
                        language: Some(document.metadata.language.clone()),
                    },
                )
                .unwrap_or(false)
            || (!request.force && !self.editing.saves.borrow().is_dirty(id))
            || !self.can_format(language)
        {
            return false;
        }
        let scope = Scope {
            language: Some(document.metadata.language.clone()),
        };
        let mode = Key::<String>::new("editor.formatOnSaveMode")
            .read(&*self.settings, &scope)
            .unwrap_or_else(|_| "file".into());
        // Rove does not have a source-control range provider. Match the no-provider
        // behavior for modifications and modificationsIfAvailable.
        if mode == "modifications" {
            return false;
        }
        let request = request.clone();
        let source = document.text.clone();
        let version = source.version();
        let range =
            ByteRange::new(ByteOffset(0), ByteOffset(source.byte_len())).expect("valid text range");
        let options = self.structured_format_options(&document);
        let cancel = Cancellation::default();
        self.structured.save_format_generation += 1;
        let generation = self.structured.save_format_generation;
        if let Some((_, old)) = self
            .structured
            .save_formats
            .insert(id, (generation, cancel.clone()))
        {
            old.cancel();
        }
        if source.byte_len() >= 1_000_000 {
            self.structure_feedback("structure.formatting", cx);
        }
        let task = cx.background_executor().spawn(async move {
            let original = document_text(&*source, Some(range.clone()), &cancel)?;
            let formatted = format_text(
                language,
                &original,
                options.as_options(),
                FormatMode::Pretty,
                &cancel,
            )?;
            Ok::<_, Error>((original != formatted).then_some((range, formatted)))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |view, cx| {
                let pending = &mut view.structured.save_formats;
                if pending.get(&id).is_none_or(|(tag, _)| *tag != generation) {
                    return;
                }
                let (_, cancel) = pending.remove(&id).expect("verified format generation");
                if let Ok(Some((range, formatted))) = cancel.check().and(result) {
                    let mut saves = view.editing.saves.borrow_mut();
                    if let Some(file) = saves
                        .file_mut(id)
                        .filter(|file| file.snapshot().version() == version)
                    {
                        let transaction = Transaction::single(range, formatted).verbatim();
                        let _ = file.session_mut().edit(transaction, Instant::now());
                    }
                }
                view.start_save_without_format(id, request, cx);
            });
        })
        .detach();
        true
    }

    fn can_format(&self, language: StructuredLanguage) -> bool {
        match language {
            StructuredLanguage::Json | StructuredLanguage::Jsonc => {
                self.structured_bool("json.format.enable", true)
            }
            StructuredLanguage::Xml => true,
            StructuredLanguage::Yaml => false,
        }
    }

    fn structured_format_options(&self, document: &Document) -> OwnedFormatOptions {
        let scope = Scope {
            language: Some(document.metadata.language.clone()),
        };
        let indentation = if Key::<bool>::new("editor.insertSpaces")
            .read(&*self.settings, &scope)
            .unwrap_or(true)
        {
            " ".repeat(self.edit_options(&document.metadata.language).tab_size as usize)
        } else {
            "\t".to_owned()
        };
        let newline = match &document.metadata.line_endings {
            LineEndings::Uniform(eol) | LineEndings::Mixed { preferred: eol } => match eol {
                Eol::Lf => "\n",
                Eol::CrLf => "\r\n",
                Eol::Cr => "\r",
            },
        };
        OwnedFormatOptions {
            indentation,
            newline,
            keep_lines: self.structured_bool("json.format.keepLines", false),
        }
    }

    pub(super) fn has_structure_formatter(&self) -> bool {
        let Some((_, doc, _)) = self.focused_text() else {
            return false;
        };
        StructuredLanguage::from_id(&doc.metadata.language)
            .is_some_and(|language| self.can_format(language))
            && doc.metadata.size != SizeClass::Huge
    }

    fn format_structured(&mut self, command: &str, cx: &mut Context<Self>) {
        let Some((group, document, _)) = self.focused_text() else {
            return;
        };
        let Some(language) = StructuredLanguage::from_id(&document.metadata.language)
            .filter(|_| self.has_structure_formatter())
        else {
            self.structure_feedback("structure.noFormatter", cx);
            return;
        };
        let my_code = self.code_view().clone();
        let code = my_code.read(cx);
        if code.is_read_only() {
            my_code.update(cx, |code, cx| code.edit_blocked(cx));
            return;
        }
        let selection = code.selection();
        let source = document.text.clone();
        let full_range =
            ByteRange::new(ByteOffset(0), ByteOffset(source.byte_len())).expect("valid text range");
        let range = if command == s::FORMAT_SELECTION && selection.start() != selection.end() {
            let range = source
                .offset(selection.start())
                .and_then(|start| ByteRange::new(start, source.offset(selection.end())?));
            let Ok(range) = range else {
                return;
            };
            range
        } else {
            full_range
        };
        let mode = if command == s::MINIFY {
            FormatMode::Minify
        } else {
            FormatMode::Pretty
        };
        let options = self.structured_format_options(&document);
        if let Some(old) = self.structured.format.take() {
            old.cancel();
        }
        let cancel = Cancellation::default();
        self.structured.format = Some(cancel.clone());
        self.structured.format_generation += 1;
        let generation = self.structured.format_generation;
        let id = document.id;
        let version = source.version();
        if source.byte_len() >= 1_000_000 {
            self.structure_feedback("structure.formatting", cx);
        }
        let task_range = range.clone();
        let selection = command == s::FORMAT_SELECTION;
        let task = cx.background_executor().spawn(async move {
            let original = document_text(&*source, None, &cancel)?;
            if selection {
                let start =
                    usize::try_from(task_range.start().0).map_err(|_| Error::OutOfBounds)?;
                let end = usize::try_from(task_range.end().0).map_err(|_| Error::OutOfBounds)?;
                let formatted = format_selection(
                    language,
                    &original,
                    start..end,
                    options.as_options(),
                    &cancel,
                )?;
                return Ok::<_, Error>((original[start..end] != formatted).then_some(formatted));
            }
            let formatted = format_text(language, &original, options.as_options(), mode, &cancel)?;
            Ok::<_, Error>((original != formatted).then_some(formatted))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ =
                weak.update(cx, |view, cx| {
                    if view.structured.format_generation != generation
                        || view.structured.format.take().is_none()
                    {
                        return;
                    }
                    if !view.focused_text().is_some_and(|(current, doc, _)| {
                        current == group && doc.id == id && doc.text.version() == version
                    }) {
                        return;
                    }
                    match result {
                        Ok(Some(formatted)) => {
                            my_code.update(cx, |code, cx| {
                                code.run_edit(None, cx, |session, snapshot, now| {
                                    if snapshot.version() != version {
                                        return Err(Error::Stale);
                                    }
                                    Ok(Some(session.edit(
                                        Transaction::single(range, formatted).verbatim(),
                                        now,
                                    )?)
                                    .into())
                                });
                            });
                        }
                        Ok(None) | Err(Error::Cancelled | Error::Stale) => {}
                        Err(_) => view.structure_feedback("structure.formatInvalid", cx),
                    }
                });
        })
        .detach();
    }

    pub(super) fn cancel_structured_format(&mut self) -> bool {
        let explicit = self.structured.format.take();
        let mut cancelled = explicit.is_some();
        if let Some(cancel) = explicit {
            cancel.cancel();
        }
        if let Some((_, document, _)) = self.focused_text() {
            if let Some((_, cancel)) = self.structured.save_formats.get(&document.id) {
                cancelled |= cancel.check().is_ok();
                cancel.cancel();
            }
        }
        cancelled
    }

    fn structure_feedback(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let _ = shell::view::report(
            self,
            key,
            Reason::Receipt,
            Message::key(key),
            Arc::from([]),
            cx,
        );
    }

    pub(super) fn sync_structured_marker(&mut self, cx: &mut Context<Self>) {
        if !self.structured_bool("problems.visibility", true) {
            self.structured.pending_marker = None;
            self.close_structured_marker(cx);
            return;
        }
        let Some((group, document, _)) = self.focused_text() else {
            self.close_structured_marker(cx);
            return;
        };
        if self.structured.marker.as_ref().is_some_and(|marker| {
            marker.document != document.id || marker.version != document.text.version()
        }) {
            self.close_structured_marker(cx);
        }
        let Some((location, at, kind)) = self.structured.pending_marker.take() else {
            return;
        };
        if location != document.resource {
            self.structured.pending_marker = Some((location, at, kind));
            return;
        }
        let code = self.code_view().clone();
        self.close_structured_marker(cx);
        self.close_peek(group, false, cx);
        code.update(cx, |code, cx| {
            code.set_view_zone(at.line.0, 54.0, cx);
            crate::peek::reveal_zone(code, at.line.0, cx);
        });
        self.structured.marker = Some(s::Marker {
            document: document.id,
            version: document.text.version(),
            position: at,
            kind,
            code,
        });
        cx.notify();
    }

    pub(super) fn close_structured_marker(&mut self, cx: &mut Context<Self>) -> bool {
        let pending = self.structured.pending_marker.take().is_some();
        if let Some(marker) = self.structured.marker.take() {
            marker.code.update(cx, |code, cx| {
                code.set_view_zone(marker.position.line.0, 0.0, cx)
            });
            cx.notify();
            true
        } else {
            pending
        }
    }

    pub(super) fn structured_marker_overlay(
        &self,
        code: Option<Entity<CodeView>>,
        cx: &mut Context<Self>,
    ) -> Option<crate::peek::zone::ZoneOverlay> {
        let marker = self.structured.marker.as_ref()?;
        let code = code.filter(|code| code == &marker.code)?;
        let zone = div()
            .id("structure-marker-navigation")
            .size_full()
            .border_1()
            .flex()
            .flex_col()
            .bg(self.ui_color("editorMarkerNavigation.background"))
            .border_color(self.ui_color("editorMarkerNavigationError.background"))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .px_2()
                    .bg(self.ui_color("editorMarkerNavigationError.headerBackground"))
                    .child(self.label("structure.markerTitle", &[]))
                    .child(
                        div()
                            .id("structure-marker-close")
                            .cursor_pointer()
                            .child(crate::outline::view::codicon("close", px(14.0), None))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.close_structured_marker(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .child(self.label(problem_key(marker.kind), &[])),
            )
            .into_any_element();
        Some(crate::peek::zone::zone_overlay(
            code,
            marker.position.line.0,
            zone,
        ))
    }

    fn navigate_structured_problem(&mut self, command: &str, cx: &mut Context<Self>) {
        if !self.structured_bool("problems.visibility", true) {
            return;
        }
        let Some((_, doc, cursor)) = self.focused_text() else {
            return;
        };
        let cross_file = matches!(command, s::NEXT_FILES | s::PREVIOUS_FILES);
        let backward = matches!(command, s::PREVIOUS | s::PREVIOUS_FILES);
        let mut locations: Vec<_> = self
            .structured
            .documents
            .iter()
            .filter(|(id, _)| cross_file || **id == doc.id)
            .flat_map(|(id, e)| {
                e.problems
                    .iter()
                    .map(move |p| ((*id, p.range.start()), e.resource.clone(), p.kind))
            })
            .collect();
        locations.sort_by_key(|(key, _, _)| *key);
        if backward {
            locations.reverse();
        }
        let cursor = (doc.id, cursor);
        let at = locations
            .iter()
            .find(|(p, _, _)| if backward { *p < cursor } else { *p > cursor })
            .or_else(|| locations.first());
        if let Some(((_, position), resource, kind)) = at {
            self.structured.pending_marker = Some((resource.clone(), *position, *kind));
            self.open_structured_location(resource.clone(), *position, OpenTarget::Active, cx);
        }
    }
}

struct OwnedFormatOptions {
    indentation: String,
    newline: &'static str,
    keep_lines: bool,
}

impl OwnedFormatOptions {
    fn as_options(&self) -> FormatOptions<'_> {
        FormatOptions {
            indentation: &self.indentation,
            newline: self.newline,
            keep_lines: self.keep_lines,
        }
    }
}

fn document_text(
    source: &dyn TextSource,
    range: Option<ByteRange>,
    cancel: &Cancellation,
) -> CoreResult<String> {
    cancel.check()?;
    let range = range.unwrap_or(ByteRange::new(
        ByteOffset(0),
        ByteOffset(source.byte_len()),
    )?);
    let capacity =
        usize::try_from(range.end().0 - range.start().0).map_err(|_| Error::OutOfBounds)?;
    let mut text = String::with_capacity(capacity);
    source.visit_chunks(range, &mut |part| {
        if cancel.check().is_ok() {
            text.push_str(part);
        }
    })?;
    cancel.check()?;
    Ok(text)
}

pub(super) fn problem_key(kind: ProblemKind) -> &'static str {
    match kind {
        ProblemKind::UnexpectedSyntax => "structure.unexpectedSyntax",
        ProblemKind::MissingSyntax => "structure.missingSyntax",
        ProblemKind::JsonComment => "structure.jsonComment",
        ProblemKind::JsonRoot => "structure.jsonRoot",
        ProblemKind::MismatchedXmlTag => "structure.mismatchedXmlTag",
        ProblemKind::DuplicateXmlAttribute => "structure.duplicateXmlAttribute",
    }
}
