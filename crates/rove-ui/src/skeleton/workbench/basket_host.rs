use super::{Context, FileEvent, Workbench};
use crate::context_basket::{
    model::{at_or_below, same_path, Availability, Basket, LineRange},
    persistence::BasketStore,
    source,
};
use gpui::{AppContext, Entity, Global, Subscription, WeakEntity};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
struct BasketSnapshot {
    contents: Basket,
    previews: BTreeMap<u64, BasketPreview>,
    revision: u64,
    collection_epoch: u64,
}

#[derive(Default)]
struct WorkspaceBasket {
    snapshot: Option<BasketSnapshot>,
    written: Arc<Mutex<u64>>,
}

#[derive(Default)]
struct WorkspaceBaskets(BTreeMap<PathBuf, WeakEntity<WorkspaceBasket>>);

impl Global for WorkspaceBaskets {}

#[derive(Default)]
struct BasketClipboardRequest(u64);

impl Global for BasketClipboardRequest {}

#[cfg(test)]
pub(super) struct BasketTestStateRoot(pub(super) PathBuf);

#[cfg(test)]
impl Global for BasketTestStateRoot {}

pub(super) struct WindowBasket {
    store: Option<BasketStore>,
    shared: Option<Entity<WorkspaceBasket>>,
    shared_observer: Option<Subscription>,
    pub(super) menu: Option<super::basket_menu::BasketMenu>,
    pub(super) contents: Basket,
    pub(super) loaded: bool,
    pub(super) panel: Option<super::basket_view::BasketPanel>,
    pub(super) focus_requested: bool,
    secondary_pane: bool,
    pub(super) reselect: Option<(u64, rove_base::Resource)>,
    pub(super) previews: BTreeMap<u64, BasketPreview>,
    pub(super) revision: u64,
    pub(super) pending_anchors: BTreeMap<u64, u64>,
    next_read: u64,
    collection_epoch: u64,
    written: Arc<Mutex<u64>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BasketPreview {
    pub(super) lines: Vec<String>,
    pub(super) characters: usize,
}

impl BasketPreview {
    fn from_text(text: &dyn rove_buffer::TextSource, range: LineRange) -> Option<Self> {
        let raw = source::read_excerpt(text, range)?;
        let canonical = raw.replace("\r\n", "\n").replace('\r', "\n");
        Some(Self {
            lines: canonical
                .lines()
                .take(2)
                .map(|line| line.chars().take(160).collect())
                .collect(),
            characters: canonical.chars().count(),
        })
    }
}

impl WindowBasket {
    pub(super) fn new(root: &Path) -> Self {
        let store = (!root.as_os_str().is_empty()).then(|| BasketStore::for_workspace(root));
        Self {
            loaded: store.is_none(),
            panel: None,
            focus_requested: false,
            secondary_pane: false,
            reselect: None,
            previews: BTreeMap::new(),
            store,
            shared: None,
            shared_observer: None,
            menu: None,
            contents: Basket::default(),
            revision: 0,
            pending_anchors: BTreeMap::new(),
            next_read: 0,
            collection_epoch: 0,
            written: Arc::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn use_test_state(&mut self) {
        self.store = None;
        self.shared = None;
        self.shared_observer = None;
        self.loaded = true;
    }

    fn set_preview(&mut self, id: u64, next: Option<BasketPreview>) -> bool {
        match next {
            Some(next) if self.previews.get(&id) != Some(&next) => {
                self.previews.insert(id, next);
                true
            }
            None => self.previews.remove(&id).is_some(),
            Some(_) => false,
        }
    }

    fn write_job(&self) -> Option<impl FnOnce() + Send + 'static> {
        let store = self.store.clone()?;
        let revision = self.revision;
        if !self.loaded || revision == 0 {
            return None;
        }
        let written = self.written.clone();
        let snapshot = self.contents.clone();
        Some(move || {
            let mut latest = written.lock().unwrap_or_else(|error| error.into_inner());
            if *latest >= revision {
                return;
            }
            match store.save(&snapshot) {
                Ok(()) => *latest = revision,
                Err(error) => eprintln!("context basket: {error}"),
            }
        })
    }
}

impl Workbench {
    pub(super) fn restore_basket(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        if let Some(base) = cx
            .try_global::<BasketTestStateRoot>()
            .filter(|_| self.basket.store.is_some())
        {
            self.basket.store = Some(BasketStore::with_base(Some(base.0.clone()), &self.root));
        }

        let Some(store) = self.basket.store.clone() else {
            return;
        };
        let Some(key) = store.state_path().map(Path::to_path_buf) else {
            self.basket.loaded = true;
            return;
        };
        let existing = cx
            .default_global::<WorkspaceBaskets>()
            .0
            .get(&key)
            .and_then(WeakEntity::upgrade);
        let needs_load = existing.is_none();
        let shared = existing.unwrap_or_else(|| {
            let shared = cx.new(|_| WorkspaceBasket::default());
            let windows = &mut cx.default_global::<WorkspaceBaskets>().0;
            windows.retain(|_, handle| handle.upgrade().is_some());
            windows.insert(key, shared.downgrade());
            shared
        });
        self.basket.written = shared.read(cx).written.clone();
        self.basket.shared = Some(shared.clone());
        self.basket.shared_observer = Some(cx.observe(&shared, |view, shared, cx| {
            view.sync_basket_snapshot(&shared, cx);
        }));
        self.sync_basket_snapshot(&shared, cx);
        if needs_load {
            let task = cx.background_executor().spawn(async move { store.load() });
            cx.spawn(async move |_, cx| {
                let contents = task.await;
                let _ = shared.update(cx, |state, cx| {
                    state.snapshot = Some(BasketSnapshot {
                        contents,
                        ..BasketSnapshot::default()
                    });
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn sync_basket_snapshot(&mut self, shared: &Entity<WorkspaceBasket>, cx: &mut Context<Self>) {
        let Some(snapshot) = shared.read(cx).snapshot.as_ref() else {
            return;
        };
        if self.basket.loaded && self.basket.revision == snapshot.revision {
            return;
        }
        let first_load = !self.basket.loaded && snapshot.revision == 0;
        if self.basket.collection_epoch != snapshot.collection_epoch {
            self.basket.pending_anchors.clear();
            self.basket.reselect = None;
        } else if !self.basket.pending_anchors.is_empty() {
            let previous: BTreeMap<_, _> = self
                .basket
                .contents
                .items()
                .iter()
                .map(|entry| (entry.id(), entry))
                .collect();
            let current: BTreeMap<_, _> = snapshot
                .contents
                .items()
                .iter()
                .map(|entry| (entry.id(), entry))
                .collect();
            self.basket.pending_anchors.retain(|id, _| {
                previous
                    .get(id)
                    .zip(current.get(id))
                    .is_some_and(|(old, new)| {
                        old.path == new.path
                            && old.lines == new.lines
                            && old.availability == new.availability
                            && old.anchor == new.anchor
                    })
            });
        }
        self.basket.contents = snapshot.contents.clone();
        self.basket.previews = snapshot.previews.clone();
        self.basket.revision = snapshot.revision;
        self.basket.collection_epoch = snapshot.collection_epoch;
        self.basket.loaded = true;
        if first_load {
            self.verify_basket_on_disk(&BTreeSet::new(), true, cx);
        }
        cx.notify();
    }

    pub(super) fn basket_changed(&mut self, cx: &mut Context<Self>) {
        self.basket.previews.retain(|id, _| {
            self.basket
                .contents
                .items()
                .iter()
                .any(|item| item.id() == *id && item.availability == Availability::Current)
        });
        if let Some(shared) = self.basket.shared.clone() {
            let accepted = shared.update(cx, |state, cx| {
                if state
                    .snapshot
                    .as_ref()
                    .is_none_or(|snapshot| snapshot.revision != self.basket.revision)
                {
                    return false;
                }
                state.snapshot = Some(BasketSnapshot {
                    contents: self.basket.contents.clone(),
                    previews: self.basket.previews.clone(),
                    revision: self.basket.revision + 1,
                    collection_epoch: self.basket.collection_epoch,
                });
                cx.notify();
                true
            });
            if !accepted {
                self.sync_basket_snapshot(&shared, cx);
                self.report_basket("contextBasket.changedInOtherWindow", Arc::from([]), cx);
                return;
            }
        }
        self.basket.revision += 1;
        if let Some(write) = self.basket.write_job() {
            let task = cx.background_executor().spawn(async move { write() });
            let shared = self.basket.shared.clone();
            cx.spawn(async move |_, _| {
                task.await;
                drop(shared);
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn flush_basket(&self) {
        if let Some(write) = self.basket.write_job() {
            write();
        }
    }

    pub(super) fn basket_workspace_files_changed(
        &mut self,
        events: &[FileEvent],
        cx: &mut Context<Self>,
    ) {
        if !self.basket.loaded || self.basket.contents.items().is_empty() {
            return;
        }
        let before = self.basket.contents.clone();
        let mut refresh = BTreeSet::new();
        let mut all = false;
        for event in events {
            match event {
                FileEvent::Renamed { from, to } => {
                    self.basket.contents.rename(from, to);
                    refresh.remove(from);
                    refresh.insert(to.clone());
                }
                FileEvent::Removed(path) => {
                    self.basket.contents.mark_removed(path);
                    refresh.remove(path);
                    for entry in self
                        .basket
                        .contents
                        .items()
                        .iter()
                        .filter(|entry| at_or_below(&entry.path, path))
                    {
                        self.basket.pending_anchors.remove(&entry.id());
                    }
                }
                FileEvent::Changed(path) | FileEvent::Created(path) => {
                    refresh.insert(path.clone());
                }
                FileEvent::RescanRequired => all = true,
            }
        }
        if self.basket.contents != before {
            self.basket_changed(cx);
        }
        self.verify_basket_on_disk(&refresh, all, cx);
    }

    fn verify_basket_on_disk(
        &mut self,
        refresh: &BTreeSet<rove_base::WorkspacePath>,
        all: bool,
        cx: &mut Context<Self>,
    ) {
        let dirty: Vec<_> = self
            .editor
            .groups
            .values()
            .flat_map(|group| group.tabs.iter())
            .filter_map(|tab| match (&tab.resource, tab.dirty) {
                (rove_base::Resource::Workspace(path), true) => Some(path),
                _ => None,
            })
            .collect();
        let mut paths = BTreeMap::<_, Vec<_>>::new();
        for entry in self.basket.contents.items() {
            if !dirty.iter().any(|path| same_path(path, &entry.path))
                && (all || refresh.iter().any(|path| same_path(path, &entry.path)))
            {
                paths.entry(entry.path.clone()).or_default().push((
                    entry.id(),
                    entry.lines,
                    entry.anchor,
                ));
            }
        }
        if paths.is_empty() {
            return;
        }
        self.basket.next_read += 1;
        let generation = self.basket.next_read;
        for entries in paths.values() {
            self.basket
                .pending_anchors
                .extend(entries.iter().map(|(id, _, _)| (*id, generation)));
        }
        let settings = self.settings.clone();
        let root = self.root.clone();
        let task = cx.background_executor().spawn(async move {
            let policy = rove_buffer::SizePolicy::read(&*settings).unwrap_or_default();
            let mut results = Vec::new();
            for (relative, entries) in paths {
                let opened = source::open_workspace_file(&root, &relative, &*settings, policy);
                let missing = matches!(
                    &opened,
                    Err(rove_base::Error::Io {
                        kind: std::io::ErrorKind::NotFound,
                        ..
                    })
                );
                let text = opened.ok().map(|(_, file)| file.text.source());
                for (id, range, anchor) in entries {
                    let relocated = anchor.and_then(|anchor| {
                        source::relocate_source(text.as_deref()?, range, anchor)
                    });
                    let preview = relocated
                        .and_then(|(range, _)| BasketPreview::from_text(text.as_deref()?, range));
                    results.push((id, range, relocated, preview, missing));
                }
            }
            results
        });
        cx.spawn(async move |this, cx| {
            let results = task.await;
            let _ = this.update(cx, |view, cx| {
                let mut changed = false;
                for (id, range, located, preview, missing) in results {
                    if view.basket.pending_anchors.get(&id) != Some(&generation) {
                        continue;
                    }
                    view.basket.pending_anchors.remove(&id);
                    changed |= view
                        .basket
                        .contents
                        .resolve_external(id, range, located, missing);
                    changed |= view.basket.set_preview(id, preview);
                }
                if changed {
                    view.basket_changed(cx);
                }
            });
        })
        .detach();
    }
}

impl Workbench {
    pub(super) fn sync_basket_editor_focus(&mut self, window: &super::Window, cx: &super::App) {
        use gpui::Focusable;
        let group = self.editor.focused;
        for secondary in [false, true] {
            if self
                .group_code(group, secondary)
                .is_some_and(|view| view.read(cx).focus_handle(cx).contains_focused(window, cx))
            {
                self.basket.secondary_pane = secondary;
                break;
            }
        }
    }

    pub(super) fn add_editor_selection_to_basket(&mut self, cx: &mut Context<Self>) {
        self.add_basket_editor_pane(self.editor.focused, self.basket.secondary_pane, cx);
    }

    pub(super) fn add_basket_editor_pane(
        &mut self,
        group: crate::documents::EditorGroupId,
        secondary: bool,
        cx: &mut Context<Self>,
    ) {
        use crate::context_basket::model::Entry;

        let Some(document) = self.shown_document(group, secondary) else {
            self.report_basket("contextBasket.noFile", Arc::from([]), cx);
            return;
        };
        let rove_base::Resource::Workspace(path) = &document.resource else {
            self.report_basket("contextBasket.noFile", Arc::from([]), cx);
            return;
        };
        if !self.basket.loaded {
            self.report_basket("contextBasket.loading", Arc::from([]), cx);
            return;
        }
        let Some(editor) = self.group_code(group, secondary) else {
            return;
        };
        let editor = editor.read(cx);
        let cursor = editor.selection().active;
        let Some(text) = editor.text().cloned() else {
            return;
        };
        let selections: Vec<_> = editor
            .selections()
            .into_iter()
            .filter(|selection| !selection.is_empty())
            .collect();
        if let Some((id, requested)) = &self.basket.reselect {
            if requested == &document.resource {
                let Some(selection) = selections.first() else {
                    self.report_basket("contextBasket.reselect", Arc::from([]), cx);
                    return;
                };
                let Some(range) = source::selection_lines(selection.start(), selection.end())
                else {
                    self.report_basket("contextBasket.tooLong", Arc::from([]), cx);
                    return;
                };
                let id = *id;
                self.basket.reselect = None;
                if self.basket.contents.replace_range(id, Some(range)) {
                    self.capture_basket_anchors(vec![(id, range)], text, cx);
                    self.basket_changed(cx);
                }
                return;
            }
        }
        let mut fallback = false;
        let entries = if selections.is_empty() {
            if document.metadata.size == rove_buffer::SizeClass::Huge {
                self.report_basket("contextBasket.selectionRequired", Arc::from([]), cx);
                return;
            }
            let symbol = self
                .outline
                .cached(document.id)
                .filter(|symbols| symbols.version() == text.version())
                .and_then(|symbols| {
                    let path = symbols.path_at(cursor);
                    let node = symbols.node(*path.last()?)?;
                    Some((
                        source::selection_lines(node.symbol.range.start(), node.symbol.range.end()),
                        crate::outline::model::symbol_path_text(symbols, &path, "."),
                    ))
                });
            let (range, name) = if let Some((range, name)) = symbol {
                (range, Some(name))
            } else {
                fallback = true;
                (LineRange::new(cursor.line.0 + 1, cursor.line.0 + 1), None)
            };
            vec![(range, name)]
        } else {
            selections
                .into_iter()
                .map(|selection| {
                    (
                        source::selection_lines(selection.start(), selection.end()),
                        None,
                    )
                })
                .collect()
        };
        let previous = self.basket.contents.clone();
        let mut ids = BTreeSet::new();
        for (range, symbol) in entries {
            let Some(range) = range else {
                self.basket.contents = previous;
                self.report_basket("contextBasket.tooLong", Arc::from([]), cx);
                return;
            };
            match self
                .basket
                .contents
                .insert(Entry::new(path.clone(), range, symbol))
            {
                Ok(id) => {
                    ids.insert(id);
                }
                Err(_) => {
                    self.basket.contents = previous;
                    self.report_basket("contextBasket.tooLong", Arc::from([]), cx);
                    return;
                }
            }
        }
        let ranges: Vec<_> = self
            .basket
            .contents
            .items()
            .iter()
            .filter(|entry| ids.contains(&entry.id()))
            .map(|entry| (entry.id(), entry.lines))
            .collect();
        self.capture_basket_anchors(ranges, text, cx);
        self.basket_changed(cx);
        let key = if fallback {
            "contextBasket.addedCurrentLine"
        } else {
            "contextBasket.added"
        };
        self.report_basket(
            key,
            Arc::from([(
                "count".into(),
                self.basket.contents.items().len().to_string().into(),
            )]),
            cx,
        );
    }

    fn capture_basket_anchors(
        &mut self,
        ranges: Vec<(u64, LineRange)>,
        text: Arc<dyn rove_buffer::TextSource>,
        cx: &mut Context<Self>,
    ) {
        if ranges.is_empty() {
            return;
        }
        self.basket.next_read += 1;
        let revision = self.basket.next_read;
        self.basket
            .pending_anchors
            .extend(ranges.iter().map(|(id, _)| (*id, revision)));
        let task = cx.background_executor().spawn(async move {
            ranges
                .into_iter()
                .map(|(id, range)| {
                    (
                        id,
                        range,
                        source::capture_source(&*text, range),
                        BasketPreview::from_text(&*text, range),
                    )
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let anchors = task.await;
            let _ = this.update(cx, |view, cx| {
                let mut changed = false;
                for (id, range, anchor, preview) in anchors {
                    if view.basket.pending_anchors.get(&id) != Some(&revision) {
                        continue;
                    }
                    view.basket.pending_anchors.remove(&id);
                    if let Some(anchor) = anchor {
                        changed |= view.basket.contents.install_anchor(id, range, anchor);
                        changed |= view.basket.set_preview(id, preview);
                    }
                }
                if changed {
                    view.basket_changed(cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn basket_text_edited(
        &mut self,
        path: &rove_base::WorkspacePath,
        edited: &crate::code_view::TextEdited,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<_> = self
            .basket
            .contents
            .items()
            .iter()
            .filter(|entry| {
                same_path(&entry.path, path) && entry.availability == Availability::Current
            })
            .map(|entry| (entry.id(), entry.lines))
            .collect();
        if ids.is_empty() {
            return;
        }
        let mut updated = Vec::new();
        for (id, old) in ids {
            let new = source::map_edited_range(&*edited.before, &*edited.after, &edited.edits, old);
            self.basket.pending_anchors.remove(&id);
            self.basket.contents.replace_range(id, new);
            if let Some(range) = new {
                updated.push((id, range));
            }
        }
        self.capture_basket_anchors(updated, edited.after.clone(), cx);
        self.basket_changed(cx);
    }

    pub(super) fn report_basket(
        &mut self,
        key: &'static str,
        args: Arc<[(Arc<str>, Arc<str>)]>,
        cx: &mut Context<Self>,
    ) {
        let _ = crate::shell::view::report(
            self,
            "contextBasket.status",
            crate::shell::feedback::Reason::Receipt,
            rove_i18n::Message {
                key: key.into(),
                args,
            },
            Arc::from([]),
            cx,
        );
    }
}

impl Workbench {
    pub(super) fn basket_command(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        use crate::context_basket::registry as reg;
        match id {
            reg::ADD => self.add_editor_selection_to_basket(cx),
            reg::FOCUS => {
                self.shell.layout.active_container = Some(reg::CONTAINER.into());
                self.shell.layout.sidebar_visible = true;
                self.shell.schedule_save(cx);
                self.basket.focus_requested = true;
                cx.notify();
            }
            reg::COPY_MARKDOWN => self.copy_basket(true, cx),
            reg::COPY_REFERENCES => self.copy_basket(false, cx),
            reg::CLEAR if self.basket.loaded => {
                self.clear_basket(cx);
                self.report_basket("contextBasket.cleared", Arc::from([]), cx);
            }
            reg::CLEAR => self.report_basket("contextBasket.loading", Arc::from([]), cx),
            _ => return false,
        }
        true
    }

    fn clear_basket(&mut self, cx: &mut Context<Self>) {
        self.basket.collection_epoch += 1;
        self.basket.contents.clear();
        self.basket.pending_anchors.clear();
        self.basket.previews.clear();
        self.basket.reselect = None;
        self.basket_changed(cx);
    }

    fn copy_basket(&mut self, markdown: bool, cx: &mut Context<Self>) {
        use crate::context_basket::{export, registry as reg};
        let latest = &mut cx.default_global::<BasketClipboardRequest>().0;
        *latest += 1;
        let request = *latest;
        if self.basket.contents.items().is_empty() {
            let key = if self.basket.loaded {
                "contextBasket.noExportableEntries"
            } else {
                "contextBasket.loading"
            };
            self.report_basket(key, Arc::from([]), cx);
            return;
        }
        let revision = self.basket.revision;
        let mut contents = self.basket.contents.clone();
        let mut cached: BTreeMap<_, _> = self
            .editor
            .groups
            .values()
            .flat_map(|group| group.tabs.iter())
            .filter_map(|tab| {
                let rove_base::Resource::Workspace(path) = &tab.resource else {
                    return None;
                };
                let document = &tab.opened()?.document;
                if !tab.dirty {
                    return None;
                }
                Some((
                    path.clone(),
                    Some((document.metadata.language.clone(), document.text.clone())),
                ))
            })
            .collect();
        let settings = self.settings.clone();
        let root = self.root.clone();
        let scope = rove_config::Scope::default();
        let clear = reg::CLEAR_AFTER_COPY
            .read(&*settings, &scope)
            .unwrap_or(false);
        let task = cx.background_executor().spawn(async move {
            let policy = rove_buffer::SizePolicy::read(&*settings).unwrap_or_default();
            let mut excerpts = BTreeMap::new();
            for entry in contents.items().to_vec() {
                if entry.availability != Availability::Current {
                    continue;
                }
                let file = cached.entry(entry.path.clone()).or_insert_with(|| {
                    let (language, file) =
                        source::open_workspace_file(&root, &entry.path, &*settings, policy).ok()?;
                    Some((language, file.text.source()))
                });
                let verified = file.as_ref().and_then(|(language, text)| {
                    let previous = entry.anchor?;
                    let same = source::capture_source(&**text, entry.lines)
                        .filter(|anchor| anchor == &previous)
                        .map(|anchor| (entry.lines, anchor));
                    let location =
                        same.or_else(|| source::relocate_source(&**text, entry.lines, previous))?;
                    if markdown {
                        excerpts.insert(
                            entry.id(),
                            export::Excerpt {
                                language: language.to_string(),
                                text: source::read_excerpt(&**text, location.0)?,
                            },
                        );
                    }
                    Some(location)
                });
                contents.resolve_external(entry.id(), entry.lines, verified, false);
            }
            if markdown {
                export::markdown(&contents, |entry| excerpts.remove(&entry.id()))
            } else {
                let template = reg::REFERENCE_TEMPLATE
                    .read(&*settings, &scope)
                    .unwrap_or_else(|_| export::DEFAULT_REFERENCE.to_owned());
                export::references(&contents, &template)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                view.finish_basket_copy(request, revision, clear, result, cx);
            });
        })
        .detach();
    }

    pub(super) fn finish_basket_copy(
        &mut self,
        request: u64,
        revision: u64,
        clear: bool,
        result: crate::context_basket::export::Export,
        cx: &mut Context<Self>,
    ) {
        if self.basket.revision != revision
            || cx
                .try_global::<BasketClipboardRequest>()
                .is_none_or(|latest| latest.0 != request)
        {
            return;
        }
        if result.entries == 0 {
            self.report_basket("contextBasket.noExportableEntries", Arc::from([]), cx);
            return;
        }
        let count = result.entries.to_string();
        let tokens = result.estimated_tokens().to_string();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(result.text));
        if clear {
            self.clear_basket(cx);
        }
        let key = if result.used_default_template {
            "contextBasket.copiedFallback"
        } else {
            "contextBasket.copied"
        };
        self.report_basket(
            key,
            Arc::from([
                ("count".into(), count.into()),
                ("tokens".into(), tokens.into()),
            ]),
            cx,
        );
    }
}

pub(super) struct BasketCapture {
    pub path: rove_base::WorkspacePath,
    pub range: crate::context_basket::capture::SourceRange,
    pub symbol: Option<String>,
}

impl Workbench {
    pub(super) fn add_basket_sources(
        &mut self,
        requests: Vec<BasketCapture>,
        expected_document: Option<(rove_base::DocumentId, rove_base::Version)>,
        cx: &mut Context<Self>,
    ) {
        use crate::context_basket::{
            capture::{allowed_range, RangeFailure},
            model::Entry,
        };
        if !self.basket.loaded {
            self.report_basket("contextBasket.loading", Arc::from([]), cx);
            return;
        }
        if requests.is_empty() {
            return;
        }
        let mut opened = BTreeMap::new();
        if let Some((document, version)) = expected_document {
            let Some(doc) = self
                .open_document(document)
                .filter(|doc| doc.text.version() == version)
            else {
                self.report_basket("contextBasket.reselect", Arc::from([]), cx);
                return;
            };
            if let rove_base::Resource::Workspace(path) = &doc.resource {
                opened.insert(path.clone(), (doc.text.clone(), doc.metadata.size));
            }
        }
        for request in &requests {
            if opened.contains_key(&request.path) {
                continue;
            }
            for tab in self.editor.groups.values().flat_map(|group| &group.tabs) {
                let rove_base::Resource::Workspace(path) = &tab.resource else {
                    continue;
                };
                if same_path(path, &request.path) && tab.kind == super::EditorKind::Text {
                    if let Some(opened_tab) = tab.opened() {
                        let doc = &opened_tab.document;
                        opened.insert(request.path.clone(), (doc.text.clone(), doc.metadata.size));
                        break;
                    }
                }
            }
        }
        let collection_epoch = self.basket.collection_epoch;
        let settings = self.settings.clone();
        let root = self.root.clone();
        let task = cx.background_executor().spawn(async move {
            let policy = rove_buffer::SizePolicy::read(&*settings).unwrap_or_default();
            let mut prepared = Vec::with_capacity(requests.len());
            for request in requests {
                let (text, size) = match opened.entry(request.path.clone()) {
                    std::collections::btree_map::Entry::Occupied(value) => value.into_mut(),
                    std::collections::btree_map::Entry::Vacant(value) => {
                        let (_, loaded) =
                            source::open_workspace_file(&root, &request.path, &*settings, policy)
                                .map_err(|_| "contextBasket.sourceUnavailable")?;
                        value.insert((loaded.text.source(), loaded.metadata.size))
                    }
                };
                let lines = allowed_range(
                    request.range,
                    text.lines().known,
                    *size == rove_buffer::SizeClass::Huge,
                )
                .map_err(|error| match error {
                    RangeFailure::TooLong => "contextBasket.tooLong",
                    RangeFailure::SelectionRequired => "contextBasket.selectionRequired",
                    RangeFailure::NoSuchLine => "contextBasket.sourceUnavailable",
                })?;
                prepared.push((
                    Entry::new(request.path, lines, request.symbol),
                    text.clone(),
                ));
            }
            Ok::<_, &'static str>(prepared)
        });
        cx.spawn(async move |this, cx| {
            let prepared = task.await;
            let _ = this.update(cx, |view, cx| {
                if view.basket.collection_epoch != collection_epoch {
                    return;
                }
                if expected_document.is_some_and(|(document, version)| {
                    view.open_document(document)
                        .is_none_or(|doc| doc.text.version() != version)
                }) {
                    view.report_basket("contextBasket.reselect", Arc::from([]), cx);
                    return;
                }
                let prepared = match prepared {
                    Ok(prepared) => prepared,
                    Err(key) => {
                        view.report_basket(key, Arc::from([]), cx);
                        return;
                    }
                };
                let mut next = view.basket.contents.clone();
                let mut sources = BTreeMap::new();
                for (entry, text) in prepared {
                    match next.insert(entry) {
                        Ok(id) => {
                            sources.insert(id, text);
                        }
                        Err(_) => {
                            view.report_basket("contextBasket.tooLong", Arc::from([]), cx);
                            return;
                        }
                    }
                }
                let ranges: Vec<_> = next
                    .items()
                    .iter()
                    .filter_map(|entry| {
                        Some((entry.id(), entry.lines, sources.remove(&entry.id())?))
                    })
                    .collect();
                view.basket.contents = next;
                for (id, range, text) in ranges {
                    view.capture_basket_anchors(vec![(id, range)], text, cx);
                }
                view.basket_changed(cx);
                view.report_basket(
                    "contextBasket.added",
                    Arc::from([(
                        "count".into(),
                        view.basket.contents.items().len().to_string().into(),
                    )]),
                    cx,
                );
            });
        })
        .detach();
    }
}
