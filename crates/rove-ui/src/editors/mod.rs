pub(crate) mod actions;
mod layout;
pub(crate) mod settings;
pub(crate) mod startup_settings;

pub(crate) use layout::{Axis, Direction, Layout};
use settings::{EditorKind, EditorSettings};

use crate::{documents::*, skeleton::events::EventBus};
use rove_base::*;
use rove_buffer::{Document, Documents, LineCount, SizeClass};
use rove_syntax::{SyntaxService, SyntaxSnapshot};
use std::{collections::BTreeMap, rc::Rc, sync::Arc};

pub(crate) type TabId = u64;
const HISTORY_LIMIT: usize = 100;

#[derive(Clone)]
pub(crate) struct Opened {
    pub(crate) document: Arc<Document>,
    pub(crate) syntax: Option<Arc<dyn SyntaxSnapshot>>,
}

#[derive(Clone, Default)]
pub(crate) struct ViewState {
    pub(crate) scroll: f64,
    pub(crate) cursor: Position,
    pub(crate) selection: Option<TextRange>,
    /// Owned by the tab; the code view will supply the ranges when folding is implemented.
    #[allow(dead_code, reason = "#19 owns folding and consumes these ranges")]
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
    syntax: Arc<dyn SyntaxService>,
    schedule: Rc<dyn Fn(OpenJob)>,
    serial: u64,
    latest_navigation: Option<OpenRequestId>,
    history: History,
    last_location: Option<Location>,
    closed: Vec<(EditorGroupId, Resource, OpenOptions)>,
    open_ids: BTreeMap<DocumentId, usize>,
    events: EventBus<OpenEvent>,
}

impl EditorService {
    pub(crate) fn new(
        loader: Arc<dyn Documents>,
        syntax: Arc<dyn SyntaxService>,
        settings: EditorSettings,
        schedule: impl Fn(OpenJob) + 'static,
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
            syntax,
            schedule: Rc::new(schedule),
            serial: 1,
            latest_navigation: None,
            history: History::default(),
            last_location: None,
            closed: Vec::new(),
            open_ids: BTreeMap::new(),
            events: EventBus::empty(),
        }
    }

    fn next(&mut self) -> u64 {
        self.serial += 1;
        self.serial
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
            Self::release_model(&mut self.open_ids, opened.document.id);
        }
    }

    fn release_model(open_ids: &mut BTreeMap<DocumentId, usize>, id: DocumentId) {
        if let Some(count) = open_ids.get_mut(&id) {
            if *count > 1 {
                *count -= 1;
            } else {
                open_ids.remove(&id);
            }
        }
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
        let syntax = self.syntax.clone();
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
                let snapshot = if !text_editor || document.metadata.size == SizeClass::Huge {
                    None
                } else if let Some(snapshot) = warm.and_then(|opened| opened.syntax) {
                    Some(snapshot)
                } else {
                    Some(
                        syntax
                            .analyze(
                                document.text.clone(),
                                document.metadata.language.clone(),
                                cancel.clone(),
                            )
                            .await?
                            .snapshot,
                    )
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

    pub(crate) fn finish(&mut self, id: OpenRequestId, result: Result<LoadResult>) -> bool {
        let Some((group, tab_id)) = self.pending(id) else {
            return false;
        };
        let pane = self.groups.get_mut(&group).unwrap();
        let rows = pane.rows;
        let tab = pane.tab_mut(tab_id).unwrap();
        let request = tab.request.take().unwrap();
        let result = result.and_then(|loaded| {
            request.cancel.check()?;
            Ok(loaded)
        });
        let event = match result {
            Ok(loaded) => {
                if let Some(old) = tab.opened() {
                    Self::release_model(&mut self.open_ids, old.document.id);
                }
                let document = loaded.opened.document.id;
                if let Some(position) = loaded.position {
                    tab.view.reveal(
                        position,
                        loaded.selection,
                        rows,
                        loaded.opened.document.text.lines(),
                    );
                }
                tab.content = Content::Ready(loaded.opened);
                *self.open_ids.entry(document).or_default() += 1;
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
        self.events.emit(event);
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
