pub(crate) mod actions;
mod layout;
pub(crate) mod settings;
pub(crate) mod syntax;

pub(crate) use layout::{Axis, Direction, Layout};
use settings::{EditorKind, EditorSettings};

use crate::{documents::*, skeleton::events::EventBus};
use rove_base::*;
use rove_buffer::{Document, Documents, LineCount};
use rove_syntax::{highlight::DocumentHighlighter, SyntaxSnapshot, SyntaxUpdate};
use std::{collections::BTreeMap, rc::Rc, sync::Arc};

pub(crate) type TabId = u64;
const HISTORY_LIMIT: usize = 100;

#[derive(Clone)]
pub(crate) struct Opened {
    pub(crate) document: Arc<Document>,
    pub(crate) syntax: Option<Arc<dyn SyntaxSnapshot>>,
}

#[derive(Clone, Default)]
/// View state of a tab that is not currently loaded into its group's code view. While a tab is
/// shown, the code view owns scroll and selection and writes them back here as they change, so a
/// tab closed or moved through `OpenDocuments` keeps its latest state.
pub(crate) struct ViewState {
    /// Top edge in virtual lines (the code view's `ScrollPosition::top`).
    pub(crate) scroll: f64,
    /// Horizontal scroll in pixels.
    pub(crate) scroll_x: f64,
    pub(crate) cursor: Position,
    pub(crate) selection: Option<TextRange>,
    /// Collapsed fold ranges (#20); the code view writes them back through
    /// `CodeViewEvent::FoldsChanged` and restores them when the tab is shown again.
    pub(crate) folds: Arc<[LineRange]>,
}

impl ViewState {
    fn reveal(
        &mut self,
        position: Position,
        selection: Option<TextRange>,
        rows: f64,
        lines: LineCount,
    ) {
        self.cursor = position;
        self.selection = selection;
        let line = position.line.0 as f64;
        if line < self.scroll.ceil() || line >= (self.scroll + rows).floor() {
            let centered = (line - (rows / 2.0).floor()).max(0.0);
            self.scroll = if lines.complete {
                centered.min((lines.known as f64 - rows).max(0.0))
            } else {
                centered
            };
        }
    }
}

#[derive(Clone)]
pub(crate) enum Content {
    Loading,
    Ready(Opened),
    Failed(Error),
}

struct Request {
    id: OpenRequestId,
    cancel: Cancellation,
    slow: bool,
    from: Option<Location>,
    record: bool,
}

pub(crate) struct Tab {
    pub(crate) id: TabId,
    pub(crate) resource: Resource,
    pub(crate) kind: EditorKind,
    pub(crate) preview: bool,
    pub(crate) pinned: bool,
    pub(crate) dirty: bool,
    pub(crate) view: ViewState,
    pub(crate) secondary: Option<ViewState>,
    /// A completed request placed the cursor; the code view still has to reveal it.
    pub(crate) reveal: bool,
    pub(crate) content: Content,
    options: OpenOptions,
    request: Option<Request>,
    used: u64,
}

impl Tab {
    pub(crate) fn opened(&self) -> Option<&Opened> {
        match &self.content {
            Content::Ready(opened) => Some(opened),
            _ => None,
        }
    }

    pub(crate) fn show_progress(&self) -> bool {
        matches!(self.content, Content::Loading)
            && self.request.as_ref().is_some_and(|request| request.slow)
    }

    fn cancel(&mut self) -> Option<OpenRequestId> {
        let request = self.request.take()?;
        request.cancel.cancel();
        Some(request.id)
    }

    fn snapshot(&self, active: bool) -> EditorTabSnapshot {
        EditorTabSnapshot {
            id: EditorTabId(self.id),
            resource: self.resource.clone(),
            kind: match &self.kind {
                EditorKind::Text => EditorTabKind::Text,
                EditorKind::Placeholder { id, .. } => EditorTabKind::Custom(id.clone()),
            },
            preview: self.preview,
            pinned: self.pinned,
            dirty: self.dirty,
            active,
            state: match &self.content {
                Content::Loading => EditorTabState::Loading,
                Content::Ready(opened) => EditorTabState::Ready(opened.document.clone()),
                Content::Failed(error) => EditorTabState::Failed(error.clone()),
            },
        }
    }
}

impl Drop for Tab {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

pub(crate) struct Group {
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: Option<TabId>,
    pub(crate) locked: bool,
    pub(crate) rows: f64,
    saved: Vec<(Resource, EditorKind, ViewState)>,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: None,
            locked: false,
            rows: 1.0,
            saved: Vec::new(),
        }
    }
}

impl Group {
    pub(crate) fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub(crate) fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub(crate) fn current(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tab(id))
    }

    pub(crate) fn current_mut(&mut self) -> Option<&mut Tab> {
        self.active.and_then(|id| self.tab_mut(id))
    }

    fn take(&mut self, id: TabId) -> Option<Tab> {
        let index = self.tabs.iter().position(|tab| tab.id == id)?;
        let tab = self.tabs.remove(index);
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|t| t.id);
        }
        Some(tab)
    }

    fn remember(&mut self, tab: &Tab) {
        if !matches!(tab.content, Content::Ready(_)) {
            return;
        }
        self.saved
            .retain(|(r, k, _)| *r != tab.resource || *k != tab.kind);
        if self.saved.len() == HISTORY_LIMIT {
            self.saved.remove(0);
        }
        self.saved
            .push((tab.resource.clone(), tab.kind.clone(), tab.view.clone()));
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Location {
    group: EditorGroupId,
    resource: Resource,
    cursor: Position,
    selection: Option<TextRange>,
}

#[derive(Default)]
struct History {
    entries: Vec<Location>,
    index: usize,
}

impl History {
    fn push(&mut self, location: Location) {
        self.entries.truncate(self.index + 1);
        if self.entries.last() == Some(&location) {
            return;
        }
        if self.entries.len() == HISTORY_LIMIT {
            self.entries.remove(0);
        }
        self.entries.push(location);
        self.index = self.entries.len() - 1;
    }
}

pub(crate) struct OpenJob {
    pub(crate) id: OpenRequestId,
    pub(crate) task: Task<LoadResult>,
}

pub(crate) struct LoadResult {
    opened: Opened,
    position: Option<Position>,
    selection: Option<TextRange>,
}

pub(crate) struct EditorService {
    pub(crate) layout: Layout,
    pub(crate) groups: BTreeMap<EditorGroupId, Group>,
    pub(crate) focused: EditorGroupId,
    pub(crate) maximized: Option<EditorGroupId>,
    pub(crate) expanded: Option<EditorGroupId>,
    pub(crate) settings: EditorSettings,
    loader: Arc<dyn Documents>,
    highlights: syntax::Highlights,
    schedule: Rc<dyn Fn(OpenJob)>,
    schedule_syntax: Rc<dyn Fn(syntax::SyntaxJob)>,
    serial: u64,
    latest_navigation: Option<OpenRequestId>,
    history: History,
    last_location: Option<Location>,
    closed: Vec<(EditorGroupId, Resource, OpenOptions)>,
    open_ids: BTreeMap<DocumentId, usize>,
    events: EventBus<OpenEvent>,
    /// Ready events per tab, held until the tab is current in its group and the host has loaded
    /// its location into that group's code view (a tab left in the background waits).
    ready: Vec<(TabId, OpenEvent)>,
}

impl EditorService {
    pub(crate) fn new(
        loader: Arc<dyn Documents>,
        syntax: Arc<dyn DocumentHighlighter>,
        settings: EditorSettings,
        schedule: impl Fn(OpenJob) + 'static,
        schedule_syntax: impl Fn(syntax::SyntaxJob) + 'static,
    ) -> Self {
        let first = EditorGroupId(1);
        Self {
            layout: Layout::Group(first),
            groups: [(first, Group::default())].into(),
            focused: first,
            maximized: None,
            expanded: None,
            settings,
            loader,
            highlights: syntax::Highlights::new(syntax),
            schedule: Rc::new(schedule),
            schedule_syntax: Rc::new(schedule_syntax),
            serial: 1,
            latest_navigation: None,
            history: History::default(),
            last_location: None,
            closed: Vec::new(),
            open_ids: BTreeMap::new(),
            events: EventBus::empty(),
            ready: Vec::new(),
        }
    }

    fn next(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    /// 文档加载器（#41 的速览预览与片段读取共用同一份文档缓存）。
    pub(crate) fn loader(&self) -> Arc<dyn Documents> {
        self.loader.clone()
    }

    pub(crate) fn active(&self) -> &Group {
        &self.groups[&self.focused]
    }

    pub(crate) fn focus(&mut self, group: EditorGroupId) {
        if self.groups.contains_key(&group) {
            self.focused = group;
            if self.maximized.is_some() {
                self.maximized = Some(group);
            }
        }
    }

    pub(crate) fn activate(&mut self, group: EditorGroupId, tab: TabId, focus: bool) {
        let used = self.next();
        let Some(pane) = self.groups.get_mut(&group) else {
            return;
        };
        let Some(entry) = pane.tab_mut(tab) else {
            return;
        };
        entry.used = used;
        pane.active = Some(tab);
        if focus {
            self.focus(group);
        }
    }

    fn add_group(&mut self, anchor: EditorGroupId, direction: Direction) -> Option<EditorGroupId> {
        let rows = self.groups.get(&anchor)?.rows;
        let id = EditorGroupId(self.next());
        if !self.layout.split(anchor, id, direction) {
            return None;
        }
        let rows = if direction.axis() == Axis::Vertical {
            (rows / 2.0).floor().max(1.0)
        } else {
            rows
        };
        self.groups.insert(
            id,
            Group {
                rows,
                ..Default::default()
            },
        );
        self.maximized = None;
        self.expanded = None;
        Some(id)
    }

    pub(crate) fn split(&mut self, direction: Direction) -> Option<EditorGroupId> {
        let source = self.focused;
        let target = self.add_group(source, direction)?;
        let id = self.next();
        if let Some(tab) = self.groups[&source].current() {
            let copy = Tab {
                id,
                resource: tab.resource.clone(),
                kind: tab.kind.clone(),
                preview: false,
                pinned: tab.pinned,
                dirty: tab.dirty,
                view: tab.view.clone(),
                secondary: tab.secondary.clone(),
                content: tab.content.clone(),
                options: tab.options.clone(),
                request: None,
                used: id,
                reveal: false,
            };
            if matches!(copy.content, Content::Loading) {
                let mut options = copy.options.clone();
                options.target = OpenTarget::Group(target);
                options.preview_tab = false;
                options.preserve_focus = false;
                let _ = self.request_open(
                    copy.resource.clone(),
                    options,
                    Cancellation::default(),
                    false,
                );
            } else {
                self.count_up(&copy);
                let pane = self.groups.get_mut(&target).unwrap();
                pane.tabs.push(copy);
                pane.active = Some(id);
            }
        }
        self.focus(target);
        Some(target)
    }

    pub(crate) fn toggle_maximize(&mut self, group: EditorGroupId) {
        if self.groups.len() > 1 {
            self.expanded = None;
            let restore = self.maximized == Some(group);
            self.focus(group);
            self.maximized = if restore { None } else { Some(group) };
        }
    }

    pub(crate) fn toggle_expand(&mut self, group: EditorGroupId) {
        if self.groups.len() > 1 {
            self.maximized = None;
            self.expanded = (self.expanded != Some(group)).then_some(group);
        }
    }

    pub(crate) fn activate_index(&mut self, index: usize) {
        if let Some(id) = self.active().tabs.get(index).map(|tab| tab.id) {
            self.activate(self.focused, id, true);
        }
    }

    pub(crate) fn cycle_tab(&mut self, forward: bool) {
        let pane = self.active();
        let count = pane.tabs.len();
        if let Some(index) = pane.tabs.iter().position(|t| Some(t.id) == pane.active) {
            self.activate_index(if forward {
                (index + 1) % count
            } else {
                (index + count - 1) % count
            });
        }
    }

    pub(crate) fn pin(&mut self, group: EditorGroupId, tab: TabId, pinned: bool) {
        let Some(pane) = self.groups.get_mut(&group) else {
            return;
        };
        let Some(index) = pane.tabs.iter().position(|entry| entry.id == tab) else {
            return;
        };
        if pane.tabs[index].pinned == pinned {
            return;
        }
        let mut entry = pane.tabs.remove(index);
        entry.pinned = pinned;
        entry.preview = false;
        let boundary = pane.tabs.iter().take_while(|tab| tab.pinned).count();
        pane.tabs.insert(boundary, entry);
    }

    pub(crate) fn keep(&mut self, group: EditorGroupId, id: TabId) {
        if let Some(tab) = self
            .groups
            .get_mut(&group)
            .and_then(|pane| pane.tab_mut(id))
        {
            tab.preview = false;
        }
    }

    pub(crate) fn drop_tab(
        &mut self,
        origin: EditorGroupId,
        id: TabId,
        mut target: EditorGroupId,
        before: Option<TabId>,
        split: Option<Direction>,
    ) {
        if !self.groups.contains_key(&target)
            || self.groups.get(&origin).and_then(|g| g.tab(id)).is_none()
            || (origin == target && before == Some(id))
        {
            return;
        }
        if let Some(direction) = split {
            let Some(group) = self.add_group(target, direction) else {
                return;
            };
            target = group;
        }
        let mut tab = self.groups.get_mut(&origin).unwrap().take(id).unwrap();
        let pane = self.groups.get_mut(&target).unwrap();
        if origin != target {
            // A manual move keeps the tab and does not evict the destination's preview.
            tab.preview = false;
        }
        let boundary = pane.tabs.iter().take_while(|t| t.pinned).count();
        let index = before
            .and_then(|before| pane.tabs.iter().position(|t| t.id == before))
            .unwrap_or(if tab.pinned {
                boundary
            } else {
                pane.tabs.len()
            });
        let index = if tab.pinned {
            index.min(boundary)
        } else {
            index.max(boundary)
        };
        pane.tabs.insert(index, tab);
        self.activate(target, id, true);
        self.remove_if_empty(origin);
    }

    pub(crate) fn shift_tab(&mut self, right: bool) {
        let pane = self.active();
        let Some(index) = pane.tabs.iter().position(|tab| Some(tab.id) == pane.active) else {
            return;
        };
        let other = if right {
            index.checked_add(1)
        } else {
            index.checked_sub(1)
        };
        let Some(other) = other.filter(|i| *i < pane.tabs.len()) else {
            return;
        };
        if pane.tabs[index].pinned == pane.tabs[other].pinned {
            self.groups
                .get_mut(&self.focused)
                .unwrap()
                .tabs
                .swap(index, other);
        }
    }

    pub(crate) fn close_tab(&mut self, group: EditorGroupId, tab: TabId) -> CloseOutcome {
        match self.groups.get(&group).and_then(|pane| pane.tab(tab)) {
            Some(tab) if tab.dirty => return CloseOutcome::NeedsUserDecision,
            None => return CloseOutcome::NotOpen,
            _ => {}
        }
        self.remove_tab(group, tab, true);
        self.remove_if_empty(group);
        CloseOutcome::Closed
    }

    fn remove_tab(&mut self, group: EditorGroupId, id: TabId, remember: bool) {
        let Some(pane) = self.groups.get_mut(&group) else {
            return;
        };
        let Some(mut tab) = pane.take(id) else { return };
        if remember {
            pane.remember(&tab);
            if self.closed.len() == HISTORY_LIMIT {
                self.closed.remove(0);
            }
            self.closed
                .push((group, tab.resource.clone(), tab.options.clone()));
        }
        self.count_down(&tab);
        if let Some(request) = tab.cancel() {
            self.report_cancelled(group, request);
        }
    }

    fn report_cancelled(&self, group: EditorGroupId, request: OpenRequestId) {
        self.events.emit(OpenEvent::Failed {
            request,
            group,
            error: Error::Cancelled,
        });
    }

    fn count_up(&mut self, tab: &Tab) {
        if let Some(opened) = tab.opened() {
            *self.open_ids.entry(opened.document.id).or_default() += 1;
        }
    }

    fn count_down(&mut self, tab: &Tab) {
        if let Some(opened) = tab.opened() {
            Self::release_model(&mut self.open_ids, &mut self.highlights, opened.document.id);
        }
    }

    /// Drop one reference to a document; the last one also ends its highlighting.
    fn release_model(
        open_ids: &mut BTreeMap<DocumentId, usize>,
        highlights: &mut syntax::Highlights,
        id: DocumentId,
    ) {
        if let Some(count) = open_ids.get_mut(&id) {
            if *count > 1 {
                *count -= 1;
            } else {
                open_ids.remove(&id);
                highlights.release(id);
            }
        }
    }

    /// Highlight a ready document in the background, visible lines of `tab` first.
    fn start_highlight(&mut self, group: EditorGroupId, tab: TabId) {
        let Some(pane) = self.groups.get(&group) else {
            return;
        };
        let Some(entry) = pane.tab(tab) else {
            return;
        };
        let Some(opened) = entry.opened() else {
            return;
        };
        if entry.kind != EditorKind::Text
            || opened.syntax.is_some()
            || !syntax::highlighted(&opened.document)
        {
            return;
        }
        let visible = syntax::visible(entry.view.scroll, pane.rows);
        let document = opened.document.clone();
        if let Some(job) = self.highlights.start(&document, visible) {
            (self.schedule_syntax)(job);
        }
    }

    /// Apply a finished background analysis to every tab that shows the document at the
    /// analysed version. Returns whether any tab changed; a visible-first result is
    /// followed by the full pass.
    pub(crate) fn finish_syntax(
        &mut self,
        document: DocumentId,
        generation: u64,
        result: Result<SyntaxUpdate>,
    ) -> bool {
        let Some(partial) = self.highlights.finish(document, generation) else {
            return false;
        };
        // #47：分析期间文档又被编辑过：接着按累积的编辑增量分析（取代可见优先之后的全文补完）。
        let finished = result
            .as_ref()
            .map_or(Version(u64::MAX), |u| u.snapshot.version());
        let resumed = self.highlights.resume(document, finished);
        let partial = partial && resumed.is_none();
        if let Some(job) = resumed {
            (self.schedule_syntax)(job);
        }
        let Ok(update) = result else {
            return false;
        };
        let version = update.snapshot.version();
        self.highlights.note_baseline(document, version);
        let mut changed = false;
        let mut shown = None;
        for pane in self.groups.values_mut() {
            for tab in &mut pane.tabs {
                if let Content::Ready(opened) = &mut tab.content {
                    if opened.document.id == document
                        && self.highlights.same_content(
                            document,
                            opened.document.text.version(),
                            version,
                        )
                    {
                        opened.syntax = Some(update.snapshot.clone());
                        shown.get_or_insert_with(|| opened.document.clone());
                        changed = true;
                    }
                }
            }
        }
        if let Some(document) = shown.filter(|_| partial) {
            if let Some(job) = self.highlights.start(&document, None) {
                (self.schedule_syntax)(job);
            }
        }
        changed
    }

    /// #47：编辑后的新快照换进所有显示该文档的标签（文档身份不变，语法旧结果保留到新结果到达），
    /// 并按增量编辑在后台重新分析。返回是否有标签显示它。
    pub(crate) fn replace_document(
        &mut self,
        document: Arc<Document>,
        edit: Option<TextEdit>,
    ) -> bool {
        let mut found = false;
        for pane in self.groups.values_mut() {
            for tab in &mut pane.tabs {
                if let Content::Ready(opened) = &mut tab.content {
                    if opened.document.id == document.id {
                        opened.document = document.clone();
                        found = true;
                    }
                }
            }
        }
        if found {
            if let Some(job) = self.highlights.edited(&document, edit) {
                (self.schedule_syntax)(job);
            }
        }
        found
    }

    /// #47：同一内容换了快照（可编辑文档换上会话的快照）：换进标签，不重新分析，进行中与已有的
    /// 着色结果继续有效。
    pub(crate) fn rebase_document(&mut self, document: Arc<Document>) {
        let mut from = None;
        for pane in self.groups.values_mut() {
            for tab in &mut pane.tabs {
                if let Content::Ready(opened) = &mut tab.content {
                    if opened.document.id == document.id {
                        from.get_or_insert(opened.document.text.version());
                        opened.document = document.clone();
                    }
                }
            }
        }
        if let Some(from) = from {
            self.highlights
                .rebased(document.id, from, document.text.version());
        }
    }

    /// #47：标签的脏标记（保存控制器的判定）。返回是否有变化。
    pub(crate) fn set_dirty(&mut self, document: DocumentId, dirty: bool) -> bool {
        let mut changed = false;
        for pane in self.groups.values_mut() {
            for tab in &mut pane.tabs {
                if tab.opened().is_some_and(|o| o.document.id == document) && tab.dirty != dirty {
                    tab.dirty = dirty;
                    changed = true;
                }
            }
        }
        changed
    }

    /// #47：在焦点组里放一个已就绪的文档（`ctrl+n` 的未命名文档）并激活它。
    pub(crate) fn open_ready(&mut self, resource: Resource, document: Arc<Document>) -> TabId {
        let id = self.next();
        let group = self.focused;
        let tab = Tab {
            id,
            resource,
            kind: EditorKind::Text,
            preview: false,
            pinned: false,
            dirty: false,
            view: ViewState::default(),
            secondary: None,
            content: Content::Ready(Opened {
                document,
                syntax: None,
            }),
            request: None,
            options: OpenOptions::default(),
            used: id,
            reveal: false,
        };
        self.count_up(&tab);
        let pane = self.groups.get_mut(&group).unwrap();
        let active = pane
            .active
            .and_then(|a| pane.tabs.iter().position(|t| t.id == a));
        let index = self.settings.insert_at(active, pane.tabs.len());
        pane.tabs.insert(index.min(pane.tabs.len()), tab);
        self.activate(group, id, true);
        self.start_highlight(group, id);
        id
    }

    /// #47：关闭确认选了“保存”（已保存）或“不保存”：关掉显示该文档的全部标签。
    pub(crate) fn force_close_document(&mut self, document: DocumentId) {
        let tabs: Vec<(EditorGroupId, TabId)> = self
            .groups
            .iter()
            .flat_map(|(g, pane)| {
                pane.tabs
                    .iter()
                    .filter(|t| t.opened().is_some_and(|o| o.document.id == document))
                    .map(move |t| (*g, t.id))
            })
            .collect();
        for (group, tab) in tabs {
            self.remove_tab(group, tab, true);
            self.remove_if_empty(group);
        }
    }

    #[cfg(test)]
    pub(crate) fn highlighting(&self, document: DocumentId) -> bool {
        self.highlights.is_running(document)
    }

    /// Replace the syntax service (tests substitute a recording or failing one).
    #[cfg(test)]
    pub(crate) fn set_syntax(&mut self, syntax: Arc<dyn DocumentHighlighter>) {
        self.highlights = syntax::Highlights::new(syntax);
    }

    pub(crate) fn close_tabs(&mut self, group: Option<EditorGroupId>, preserve_pinned: bool) {
        let ids: Vec<_> = self
            .groups
            .iter()
            .filter(|(id, _)| group.is_none_or(|group| group == **id))
            .flat_map(|(&id, pane)| {
                pane.tabs
                    .iter()
                    .filter(|tab| !tab.dirty && (!preserve_pinned || !tab.pinned))
                    .map(move |tab| (id, tab.id))
            })
            .collect();
        for (group, id) in ids {
            self.close_tab(group, id);
        }
    }

    fn remove_if_empty(&mut self, group: EditorGroupId) {
        if self.settings.close_empty
            && self.groups.len() > 1
            && self
                .groups
                .get(&group)
                .is_some_and(|pane| pane.tabs.is_empty())
        {
            self.close_group(group);
        }
    }

    pub(crate) fn file_removed(&mut self, resource: &Resource) -> bool {
        if !self.settings.close_on_delete {
            return false;
        }
        let document = self.loader.find(resource).map(|doc| doc.id);
        let mut closing = Vec::new();
        for (&group, pane) in &self.groups {
            for tab in &pane.tabs {
                let same_document = document
                    .is_some_and(|id| tab.opened().is_some_and(|opened| opened.document.id == id));
                if !tab.dirty && (&tab.resource == resource || same_document) {
                    closing.push((group, tab.id));
                }
            }
        }
        let changed = !closing.is_empty();
        for (group, tab) in closing {
            self.close_tab(group, tab);
        }
        changed
    }

    pub(crate) fn close_group(&mut self, group: EditorGroupId) -> CloseOutcome {
        let Some(pane) = self.groups.get(&group) else {
            return CloseOutcome::NotOpen;
        };
        if pane.tabs.iter().any(|tab| tab.dirty) {
            return CloseOutcome::NeedsUserDecision;
        }
        let ids: Vec<_> = pane.tabs.iter().map(|tab| tab.id).collect();
        for id in ids {
            self.remove_tab(group, id, true);
        }
        if self.groups.len() > 1 {
            self.groups.remove(&group);
            self.layout.remove(group);
            if self.expanded == Some(group) || self.groups.len() == 1 {
                self.expanded = None;
            }
            if self.focused == group {
                self.focused = self.layout.order()[0];
            }
            if self.maximized == Some(group) || self.groups.len() == 1 {
                self.maximized = None;
            }
        } else {
            self.groups.insert(group, Group::default());
        }
        CloseOutcome::Closed
    }

    pub(crate) fn reopen(&mut self) {
        if let Some((group, resource, mut options)) = self.closed.pop() {
            options.target = if self.groups.contains_key(&group) {
                OpenTarget::Group(group)
            } else {
                OpenTarget::Active
            };
            options.location = None;
            options.selection = None;
            options.preview_tab = false;
            options.preserve_focus = false;
            let _ = self.request_open(resource, options, Cancellation::default(), false);
        }
    }

    pub(crate) fn retry(&mut self, group: EditorGroupId, id: TabId) {
        if let Some(tab) = self.groups.get(&group).and_then(|pane| pane.tab(id)) {
            let mut options = tab.options.clone();
            options.target = OpenTarget::Group(group);
            let _ = self.request_open(
                tab.resource.clone(),
                options,
                Cancellation::default(),
                false,
            );
        }
    }

    fn location(&self) -> Option<Location> {
        let tab = self.active().current()?;
        tab.opened()?;
        Some(Location {
            group: self.focused,
            resource: tab.resource.clone(),
            cursor: tab.view.cursor,
            selection: tab.view.selection.clone(),
        })
    }

    pub(crate) fn can_back(&self) -> bool {
        self.history.index > 0
    }

    pub(crate) fn can_forward(&self) -> bool {
        self.history.index + 1 < self.history.entries.len()
    }

    pub(crate) fn navigate(&mut self, forward: bool) {
        if forward && self.can_forward() {
            self.history.index += 1;
        } else if !forward && self.can_back() {
            self.history.index -= 1;
        } else {
            return;
        }
        self.open_location(self.history.entries[self.history.index].clone());
    }

    pub(crate) fn navigate_last(&mut self) {
        if let Some(location) = self.last_location.clone() {
            self.open_location(location);
        }
    }

    fn open_location(&mut self, location: Location) {
        let target = if self.groups.contains_key(&location.group) {
            OpenTarget::Group(location.group)
        } else {
            OpenTarget::Active
        };
        let _ = self.request_open(
            location.resource,
            OpenOptions {
                target,
                preview_tab: false,
                location: Some(location.cursor),
                selection: location.selection,
                ..Default::default()
            },
            Cancellation::default(),
            false,
        );
    }

    fn target(&mut self, target: OpenTarget) -> Result<EditorGroupId> {
        let current = self.focused;
        match target {
            OpenTarget::Group(group) => self
                .groups
                .contains_key(&group)
                .then_some(group)
                .ok_or(Error::NotFound),
            OpenTarget::Active if !self.active().locked => Ok(current),
            OpenTarget::Active | OpenTarget::Beside => {
                for direction in [
                    Direction::Right,
                    Direction::Down,
                    Direction::Left,
                    Direction::Up,
                ] {
                    if let Some(next) = self
                        .layout
                        .neighbor(current, direction)
                        .filter(|group| self.groups.get(group).is_some_and(|pane| !pane.locked))
                    {
                        return Ok(next);
                    }
                }
                self.add_group(current, Direction::Right)
                    .ok_or(Error::NotFound)
            }
        }
    }

    fn enforce_limit(&mut self, target: EditorGroupId, keep: TabId) {
        let Some(limit) = self.settings.tab_limit else {
            return;
        };
        loop {
            let tabs = self
                .groups
                .iter()
                .filter(|(id, _)| !self.settings.limit_per_group || **id == target)
                .flat_map(|(&id, group)| group.tabs.iter().map(move |tab| (id, tab)))
                .filter(|(_, tab)| !tab.dirty || !self.settings.limit_exclude_dirty);
            if tabs.clone().count() <= limit {
                break;
            }
            let Some((group, id)) = tabs
                .filter(|(_, tab)| tab.id != keep && !tab.pinned && !tab.dirty)
                .min_by_key(|(_, tab)| tab.used)
                .map(|(group, tab)| (group, tab.id))
            else {
                break;
            };
            self.close_tab(group, id);
        }
    }

    fn request_open(
        &mut self,
        resource: Resource,
        options: OpenOptions,
        cancel: Cancellation,
        history: bool,
    ) -> Result<OpenRequestId> {
        cancel.check()?;
        let from = self.location();
        let kind = self.settings.editors.resolve(&resource, &options)?;
        let text_editor = kind == EditorKind::Text;
        let known = self.loader.find(&resource);
        let matches = |tab: &Tab| {
            tab.kind == kind
                && (tab.resource == resource
                    || known
                        .as_ref()
                        .is_some_and(|doc| tab.opened().is_some_and(|o| o.document.id == doc.id)))
        };
        let reveal = self.settings.reveal_if_open && matches!(options.target, OpenTarget::Active);
        let group = if reveal {
            self.groups
                .iter()
                .find(|(_, group)| group.tabs.iter().any(&matches))
                .map(|(id, _)| *id)
        } else {
            None
        };
        let group = match group {
            Some(group) => group,
            None => self.target(options.target)?,
        };
        let preview = self.settings.preview(&options);
        let warm = self
            .groups
            .values()
            .flat_map(|group| &group.tabs)
            .filter_map(Tab::opened)
            .find(|opened| {
                known.as_ref().is_some_and(|doc| {
                    opened.document.id == doc.id
                        && opened.document.text.version() == doc.text.version()
                })
            })
            .cloned();
        let id = OpenRequestId(self.next());
        let mut replaced = None;
        let tab_id = if let Some(existing) = self.groups[&group]
            .tabs
            .iter()
            .find(|tab| matches(tab))
            .map(|tab| tab.id)
        {
            let tab = self
                .groups
                .get_mut(&group)
                .unwrap()
                .tab_mut(existing)
                .unwrap();
            replaced = tab.cancel();
            if !preview {
                tab.preview = false;
            }
            if !matches!(tab.content, Content::Ready(_)) {
                tab.content = Content::Loading;
            }
            self.supersede_ready(existing);
            existing
        } else {
            let replace = self.groups[&group]
                .tabs
                .iter()
                .position(|tab| preview && tab.preview && !tab.dirty);
            if let Some(index) = replace {
                let tab = self.groups[&group].tabs[index].id;
                self.remove_tab(group, tab, false);
            }
            let pane = self.groups.get_mut(&group).unwrap();
            let view = self
                .settings
                .restore_view
                .then(|| {
                    pane.saved
                        .iter()
                        .rev()
                        .find(|(r, k, _)| r == &resource && k == &kind)
                        .map(|(_, _, view)| view.clone())
                })
                .flatten()
                .unwrap_or_default();
            let boundary = pane.tabs.iter().take_while(|tab| tab.pinned).count();
            let active = pane.tabs.iter().position(|tab| Some(tab.id) == pane.active);
            let index = replace
                .unwrap_or_else(|| self.settings.insert_at(active, pane.tabs.len()))
                .clamp(boundary, pane.tabs.len());
            pane.tabs.insert(
                index,
                Tab {
                    id: id.0,
                    resource: resource.clone(),
                    kind,
                    preview,
                    pinned: false,
                    dirty: false,
                    view,
                    secondary: None,
                    content: Content::Loading,
                    request: None,
                    options: options.clone(),
                    used: id.0,
                    reveal: false,
                },
            );
            id.0
        };
        self.activate(group, tab_id, !options.preserve_focus);
        self.enforce_limit(group, tab_id);
        self.latest_navigation = Some(id);
        let cancel = cancel.child();
        let tab = self
            .groups
            .get_mut(&group)
            .unwrap()
            .tab_mut(tab_id)
            .unwrap();
        tab.options = options.clone();
        tab.request = Some(Request {
            id,
            cancel: cancel.clone(),
            slow: false,
            from,
            record: history,
        });
        if let Some(request) = replaced {
            self.report_cancelled(group, request);
        }
        let loader = self.loader.clone();
        let highlighter = self.highlights.service.clone();
        (self.schedule)(OpenJob {
            id,
            task: Box::pin(async move {
                cancel.check()?;
                let document = match &warm {
                    Some(opened) => opened.document.clone(),
                    None => loader.open(resource, cancel.clone()).await?,
                };
                let position = options.reveal_position();
                if let Some(position) = position {
                    document.text.offset(position)?;
                }
                if let Some(selection) = &options.selection {
                    document.text.offset(selection.start())?;
                    document.text.offset(selection.end())?;
                }
                let snapshot = if !text_editor || !syntax::highlighted(&document) {
                    None
                } else if let Some(snapshot) = warm.and_then(|opened| opened.syntax) {
                    Some(snapshot)
                } else if syntax::inline(&document) {
                    // An unsupported language opens as plain text rather than failing.
                    match highlighter
                        .analyze(
                            document.id,
                            document.text.clone(),
                            document.metadata.language.clone(),
                            None,
                            None,
                            cancel.clone(),
                        )
                        .await
                    {
                        Ok(update) => Some(update.snapshot),
                        Err(Error::Cancelled) => return Err(Error::Cancelled),
                        Err(_) => None,
                    }
                } else {
                    // Readable first; `finish` starts the background analysis.
                    None
                };
                cancel.check()?;
                Ok(LoadResult {
                    opened: Opened {
                        document,
                        syntax: snapshot,
                    },
                    position,
                    selection: options.selection,
                })
            }),
        });
        Ok(id)
    }

    fn pending(&self, id: OpenRequestId) -> Option<(EditorGroupId, TabId)> {
        self.groups.iter().find_map(|(&group, pane)| {
            pane.tabs
                .iter()
                .find(|tab| tab.request.as_ref().is_some_and(|request| request.id == id))
                .map(|tab| (group, tab.id))
        })
    }

    pub(crate) fn slow(&mut self, id: OpenRequestId) -> bool {
        let Some((group, tab)) = self.pending(id) else {
            return false;
        };
        self.groups
            .get_mut(&group)
            .unwrap()
            .tab_mut(tab)
            .unwrap()
            .request
            .as_mut()
            .unwrap()
            .slow = true;
        true
    }

    /// Folds a freshly loaded `alias` tab into another tab of the same group, kind and
    /// document. Returns the surviving tab, which takes over the alias's options and focus.
    fn merge_alias(
        &mut self,
        group: EditorGroupId,
        alias: TabId,
        document: DocumentId,
    ) -> Option<TabId> {
        let pane = &self.groups[&group];
        let loaded = pane.tab(alias)?;
        let existing = pane
            .tabs
            .iter()
            .find(|tab| {
                tab.id != alias
                    && tab.kind == loaded.kind
                    && tab.opened().is_some_and(|o| o.document.id == document)
            })?
            .id;
        let (preview, options) = (loaded.preview, loaded.options.clone());
        // The user may have moved on while the alias loaded: only an alias that is still the
        // group's active tab hands that role to the survivor, and focus only stays where it is.
        let was_active = pane.active == Some(alias);
        let focus = !options.preserve_focus && self.focused == group;
        self.remove_tab(group, alias, false);
        let tab = self.groups.get_mut(&group)?.tab_mut(existing)?;
        if !preview {
            tab.preview = false;
        }
        tab.options = options;
        if was_active {
            self.activate(group, existing, focus);
        }
        Some(existing)
    }

    /// Emits Ready for tabs that are current in their group and whose reveal the host has
    /// applied (text tabs clear `reveal` when the code view takes the location). A held request
    /// whose tab was closed in the meantime reports Cancelled instead.
    pub(crate) fn flush_ready(&mut self) {
        if self.ready.is_empty() {
            return;
        }
        let mut emit = Vec::new();
        for (tab, event) in std::mem::take(&mut self.ready) {
            let state = self.groups.values().find_map(|pane| {
                let entry = pane.tab(tab)?;
                Some(pane.active == Some(tab) && (entry.kind != EditorKind::Text || !entry.reveal))
            });
            match (state, event) {
                (Some(true), event) => emit.push(event),
                (Some(false), event) => self.ready.push((tab, event)),
                (None, OpenEvent::Ready { request, group, .. }) => emit.push(OpenEvent::Failed {
                    request,
                    group,
                    error: Error::Cancelled,
                }),
                (None, event) => emit.push(event),
            }
        }
        for event in emit {
            self.events.emit(event);
        }
    }

    /// A newer request on the same tab supersedes a Ready that is still held back.
    fn supersede_ready(&mut self, tab: TabId) {
        let (old, keep) = std::mem::take(&mut self.ready)
            .into_iter()
            .partition::<Vec<_>, _>(|(id, _)| *id == tab);
        self.ready = keep;
        for (_, event) in old {
            if let OpenEvent::Ready { request, group, .. } = event {
                self.report_cancelled(group, request);
            }
        }
    }

    pub(crate) fn finish(&mut self, id: OpenRequestId, result: Result<LoadResult>) -> bool {
        let Some((group, tab_id)) = self.pending(id) else {
            return false;
        };
        let pane = self.groups.get_mut(&group).unwrap();
        let rows = pane.rows;
        let request = pane.tab_mut(tab_id).unwrap().request.take().unwrap();
        let result = result.and_then(|loaded| {
            request.cancel.check()?;
            Ok(loaded)
        });
        // An alias (e.g. a symlink) is only resolved to its document after the loader
        // canonicalizes it, so deduplicate here and reuse the group's existing tab.
        let tab_id = match &result {
            Ok(loaded) => self
                .merge_alias(group, tab_id, loaded.opened.document.id)
                .unwrap_or(tab_id),
            Err(_) => tab_id,
        };
        let tab = self
            .groups
            .get_mut(&group)
            .unwrap()
            .tab_mut(tab_id)
            .unwrap();
        let event = match result {
            Ok(loaded) => {
                let document = loaded.opened.document.id;
                // Count the new document first, so reloading the same one keeps its baseline.
                *self.open_ids.entry(document).or_default() += 1;
                if let Some(old) = tab.opened() {
                    Self::release_model(&mut self.open_ids, &mut self.highlights, old.document.id);
                }
                if let Some(position) = loaded.position {
                    tab.view.reveal(
                        position,
                        loaded.selection,
                        rows,
                        loaded.opened.document.text.lines(),
                    );
                }
                tab.reveal |= loaded.position.is_some();
                tab.content = Content::Ready(loaded.opened);
                if self.latest_navigation == Some(id) {
                    let to = Location {
                        group,
                        resource: tab.resource.clone(),
                        cursor: tab.view.cursor,
                        selection: tab.view.selection.clone(),
                    };
                    if request.record {
                        if let Some(from) = request.from {
                            self.history.push(from);
                        }
                        self.history.push(to.clone());
                        self.last_location = Some(to);
                    }
                }
                OpenEvent::Ready {
                    request: id,
                    group,
                    document,
                }
            }
            Err(Error::Cancelled) => {
                if matches!(tab.content, Content::Loading) {
                    self.remove_tab(group, tab_id, false);
                    self.remove_if_empty(group);
                }
                OpenEvent::Failed {
                    request: id,
                    group,
                    error: Error::Cancelled,
                }
            }
            Err(error) => {
                if !matches!(tab.content, Content::Ready(_)) {
                    tab.content = Content::Failed(error.clone());
                }
                OpenEvent::Failed {
                    request: id,
                    group,
                    error,
                }
            }
        };
        if matches!(event, OpenEvent::Ready { .. }) {
            self.ready.push((tab_id, event));
            self.start_highlight(group, tab_id);
        } else {
            self.events.emit(event);
        }
        true
    }
}

impl OpenDocuments for EditorService {
    fn open(
        &mut self,
        resource: Resource,
        options: OpenOptions,
        cancel: Cancellation,
    ) -> Result<OpenRequestId> {
        self.request_open(resource, options, cancel, true)
    }

    fn find(&self, resource: &Resource) -> Option<Arc<Document>> {
        self.loader
            .find(resource)
            .filter(|doc| self.open_ids.contains_key(&doc.id))
    }

    fn close(
        &mut self,
        document: DocumentId,
        group: Option<EditorGroupId>,
    ) -> Result<CloseOutcome> {
        let mut matches = Vec::new();
        for (&id, pane) in &self.groups {
            if group.is_some_and(|group| group != id) {
                continue;
            }
            for tab in &pane.tabs {
                if tab
                    .opened()
                    .is_some_and(|opened| opened.document.id == document)
                {
                    if tab.dirty {
                        return Ok(CloseOutcome::NeedsUserDecision);
                    }
                    matches.push((id, tab.id));
                }
            }
        }
        if matches.is_empty() {
            return Ok(CloseOutcome::NotOpen);
        }
        for (group, id) in matches {
            self.close_tab(group, id);
        }
        Ok(CloseOutcome::Closed)
    }

    fn close_tab(&mut self, tab: EditorTabId) -> Result<CloseOutcome> {
        let Some(group) = self
            .groups
            .iter()
            .find_map(|(&group, pane)| pane.tab(tab.0).is_some().then_some(group))
        else {
            return Ok(CloseOutcome::NotOpen);
        };
        Ok(EditorService::close_tab(self, group, tab.0))
    }

    fn in_group(&self, group: EditorGroupId) -> Vec<EditorTabSnapshot> {
        self.groups
            .get(&group)
            .into_iter()
            .flat_map(|pane| {
                pane.tabs
                    .iter()
                    .map(move |tab| tab.snapshot(pane.active == Some(tab.id)))
            })
            .collect()
    }

    fn events(&self) -> &dyn Events<OpenEvent> {
        &self.events
    }
}
