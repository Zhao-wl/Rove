//! UI-free quick input (#18): implements the #1 [`QuickInput`] contract plus every
//! `quickInput.*` command, input history, checkboxes, separators and buttons. The GPUI
//! `QuickInputView` renders it; #24/#27/#28 drive it with their own items.
use super::{
    history::InputHistory,
    matcher::{FindMode, MatchType, Matcher},
    options::ListOptions,
    registry::context as ck,
    widget::{ListWidget, Modifiers, RowHeight},
};
use crate::lists::{
    ItemId, ListModel, QuickInput, QuickInputEvent, QuickInputId, QuickInputOptions,
};
use rove_base::{Error, Events, Result, Subscription, Value};
use rove_i18n::Message;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

type Listener<T> = Arc<dyn Fn(T) + Send + Sync>;
type Listeners<T> = Arc<Mutex<Vec<(u64, Listener<T>)>>>;

/// Event source with RAII subscriptions. Listeners run on the emitting thread after the lock is
/// released, as the #1 `Events` contract requires.
pub struct Broadcast<T> {
    listeners: Listeners<T>,
    next: AtomicU64,
}
impl<T> Default for Broadcast<T> {
    fn default() -> Self {
        Self {
            listeners: Arc::default(),
            next: AtomicU64::new(0),
        }
    }
}
impl<T: Clone> Broadcast<T> {
    pub fn emit(&self, value: T) {
        let listeners: Vec<_> = self
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
        for l in listeners {
            l(value.clone());
        }
    }
}
impl<T: Clone + 'static> Events<T> for Broadcast<T> {
    fn subscribe(&self, listener: Arc<dyn Fn(T) + Send + Sync>) -> Subscription {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((id, listener));
        let weak = Arc::downgrade(&self.listeners);
        Subscription::new(move || {
            if let Some(l) = weak.upgrade() {
                l.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .retain(|(k, _)| *k != id);
            }
        })
    }
}

/// `quickInputType` context key value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickInputType {
    QuickPick,
    QuickTree,
    QuickWidget,
}
impl QuickInputType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QuickPick => "quickPick",
            Self::QuickTree => "quickTree",
            Self::QuickWidget => "quickWidget",
        }
    }
}

#[derive(Clone, Debug)]
pub struct QuickInputButton {
    pub id: Arc<str>,
    pub tooltip: Message,
    /// Glyph/icon content data (codicon name or character); not prose.
    pub icon: Arc<str>,
}

/// Optional presentation beyond the frozen `QuickInputOptions`.
#[derive(Clone)]
pub struct QuickPickConfig {
    pub title: Option<Message>,
    pub kind: QuickInputType,
    /// Group starts (model indexes) with optional labels (`pickerGroup.*`).
    pub separators: Vec<(usize, Option<Arc<str>>)>,
    pub buttons: Vec<QuickInputButton>,
    /// Built-in filtering of item labels by the typed text. `None` = the caller filters on
    /// `Changed` and calls `update` (#24 does this with its own scorer and prefixes).
    pub filter: Option<MatchType>,
    /// Shows a non-selectable loading row at the bottom (e.g. workspace scan still running).
    pub loading_tail: Option<Message>,
    pub row_height: f32,
    /// Empty-result text; defaults to `quickInput.noResults`.
    pub empty: Option<Message>,
}
impl Default for QuickPickConfig {
    fn default() -> Self {
        Self {
            title: None,
            kind: QuickInputType::QuickPick,
            separators: vec![],
            buttons: vec![],
            filter: Some(MatchType::Fuzzy),
            loading_tail: None,
            row_height: 22.0,
            empty: None,
        }
    }
}

/// Richer events than the contract carries (modifiers, background accept, buttons).
#[derive(Clone, Debug, PartialEq)]
pub enum QuickPickEvent {
    Accepted {
        id: QuickInputId,
        items: Arc<[ItemId]>,
        text: Arc<str>,
        in_background: bool,
        modifiers: Modifiers,
    },
    Button {
        id: QuickInputId,
        button: Arc<str>,
    },
    CheckedChanged {
        id: QuickInputId,
        items: Arc<[ItemId]>,
    },
}

pub struct ActiveInput {
    pub id: QuickInputId,
    pub placeholder: Message,
    pub text: String,
    /// UTF-8 cursor offset in `text`.
    pub cursor: usize,
    pub input_focused: bool,
    pub list: ListWidget,
    pub multi_select: bool,
    pub checked: Vec<ItemId>,
    pub hover_visible: bool,
    pub config: QuickPickConfig,
}

pub struct QuickInputService {
    active: Option<ActiveInput>,
    next_id: u64,
    events: Broadcast<QuickInputEvent>,
    pick_events: Broadcast<QuickPickEvent>,
    history: InputHistory,
    options: ListOptions,
    matcher: Option<Arc<dyn Matcher>>,
    viewport: f32,
}

impl Default for QuickInputService {
    fn default() -> Self {
        Self::new(ListOptions::default())
    }
}

impl QuickInputService {
    pub fn new(options: ListOptions) -> Self {
        Self {
            active: None,
            next_id: 1,
            events: Broadcast::default(),
            pick_events: Broadcast::default(),
            history: InputHistory::new(50),
            options,
            matcher: None,
            viewport: 22.0 * 15.0,
        }
    }

    pub fn pick_events(&self) -> &dyn Events<QuickPickEvent> {
        &self.pick_events
    }

    pub fn active(&self) -> Option<&ActiveInput> {
        self.active.as_ref()
    }

    pub fn active_mut(&mut self) -> Option<&mut ActiveInput> {
        self.active.as_mut()
    }

    pub fn history(&self) -> &InputHistory {
        &self.history
    }

    /// Scorer used by built-in filtering (#24 installs VS Code's fuzzy scorer).
    pub fn set_matcher(&mut self, matcher: Arc<dyn Matcher>) {
        if let Some(a) = self.active.as_mut() {
            a.list.set_matcher(matcher.clone());
        }
        self.matcher = Some(matcher);
    }

    /// Height available for the result list (the view reports it every layout).
    pub fn set_viewport(&mut self, height: f32, width: f32) {
        self.viewport = height;
        if let Some(a) = self.active.as_mut() {
            a.list.set_viewport(height, width);
        }
    }

    pub fn show_with(
        &mut self,
        options: QuickInputOptions,
        config: QuickPickConfig,
    ) -> Result<QuickInputId> {
        if let Some(previous) = self.active.take() {
            self.events.emit(QuickInputEvent::Cancelled(previous.id));
        }
        let id = QuickInputId(self.next_id);
        self.next_id += 1;
        let mut list = ListWidget::new(
            options.items.clone(),
            RowHeight::Fixed(config.row_height),
            self.options.clone(),
        );
        list.supports_find = false;
        list.multi_select = options.multi_select;
        list.set_viewport(self.viewport, 0.0);
        if let Some(m) = &self.matcher {
            list.set_matcher(m.clone());
        } else if let Some(t) = config.filter {
            list.set_matcher(super::matcher::matcher_for(t));
        }
        list.set_separators(config.separators.clone());
        list.set_loading_tail(config.loading_tail.is_some());
        let mut active = ActiveInput {
            id,
            placeholder: options.placeholder,
            text: options.initial.to_string(),
            cursor: options.initial.len(),
            input_focused: true,
            list,
            multi_select: options.multi_select,
            checked: vec![],
            hover_visible: false,
            config,
        };
        Self::refilter(&mut active);
        self.active = Some(active);
        self.history.reset();
        Ok(id)
    }

    fn refilter(a: &mut ActiveInput) {
        if a.config.filter.is_some() {
            let q = (!a.text.is_empty()).then(|| (a.text.clone(), FindMode::Filter));
            a.list.set_filter_query(q);
        } else {
            a.list.set_filter_query(None);
        }
        a.list.set_focus(None);
        a.list.focus_first();
    }

    fn require(&mut self, id: QuickInputId) -> Result<&mut ActiveInput> {
        match self.active.as_mut() {
            Some(a) if a.id == id => Ok(a),
            _ => Err(Error::NotFound),
        }
    }

    /// The user edited the input: emits `Changed`, refilters and focuses the first match.
    pub fn set_text(&mut self, text: &str, cursor: usize) {
        let Some(a) = self.active.as_mut() else {
            return;
        };
        let changed = a.text != text;
        a.text = text.to_string();
        a.cursor = cursor.min(a.text.len());
        if changed {
            let id = a.id;
            Self::refilter(a);
            self.history.reset();
            self.events.emit(QuickInputEvent::Changed {
                id,
                text: text.into(),
            });
        }
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        if let Some(a) = self.active.as_mut() {
            a.cursor = cursor.min(a.text.len());
        }
    }

    pub fn set_input_focused(&mut self, focused: bool) {
        if let Some(a) = self.active.as_mut() {
            a.input_focused = focused;
        }
    }

    pub fn set_loading_tail(&mut self, id: QuickInputId, message: Option<Message>) -> Result<()> {
        let a = self.require(id)?;
        a.list.set_loading_tail(message.is_some());
        a.config.loading_tail = message;
        Ok(())
    }

    fn accept(&mut self, in_background: bool, modifiers: Modifiers) -> bool {
        let Some(a) = self.active.as_ref() else {
            return false;
        };
        let items: Arc<[ItemId]> = if a.multi_select {
            a.checked.iter().cloned().collect()
        } else {
            a.list.focused_id().into_iter().collect()
        };
        let id = a.id;
        let text: Arc<str> = a.text.as_str().into();
        self.history.add(&text);
        if !in_background {
            self.active = None;
        }
        self.events.emit(QuickInputEvent::Accepted {
            id,
            items: items.clone(),
            text: text.clone(),
        });
        self.pick_events.emit(QuickPickEvent::Accepted {
            id,
            items,
            text,
            in_background,
            modifiers,
        });
        true
    }

    fn hide(&mut self) -> bool {
        match self.active.take() {
            Some(a) => {
                self.events.emit(QuickInputEvent::Cancelled(a.id));
                true
            }
            None => false,
        }
    }

    pub fn toggle_checked(&mut self, index: Option<usize>) -> bool {
        let Some(a) = self.active.as_mut() else {
            return false;
        };
        if !a.multi_select {
            return false;
        }
        let Some(id) = index
            .or(a.list.focus())
            .and_then(|i| a.list.row(i))
            .map(|r| r.id)
        else {
            return false;
        };
        match a.checked.iter().position(|c| c == &id) {
            Some(k) => {
                a.checked.remove(k);
            }
            None => a.checked.push(id),
        }
        let (qid, items) = (a.id, a.checked.iter().cloned().collect());
        self.pick_events
            .emit(QuickPickEvent::CheckedChanged { id: qid, items });
        true
    }

    /// Mouse click on result row `index`: toggles the checkbox (multi) or accepts the row.
    pub fn click(&mut self, index: usize, modifiers: Modifiers) -> bool {
        let Some(a) = self.active.as_mut() else {
            return false;
        };
        if index >= a.list.len() {
            return false;
        }
        a.list.set_focus(Some(index));
        if a.multi_select {
            self.toggle_checked(Some(index))
        } else {
            self.accept(false, modifiers)
        }
    }

    pub fn trigger_button(&mut self, button: &str) -> Result<()> {
        let a = self.active.as_ref().ok_or(Error::NotFound)?;
        if !a.config.buttons.iter().any(|b| &*b.id == button) {
            return Err(Error::NotFound);
        }
        self.pick_events.emit(QuickPickEvent::Button {
            id: a.id,
            button: button.into(),
        });
        Ok(())
    }

    /// Runs a `quickInput.*` / `history.*` command. Unknown ID => NotFound.
    pub fn execute(&mut self, command: &str, modifiers: Modifiers) -> Result<bool> {
        if self.active.is_none() {
            return match command {
                c if c.starts_with("quickInput.") || c.starts_with("history.") => Ok(false),
                _ => Err(Error::NotFound),
            };
        }
        let handled = match command {
            "quickInput.accept" => self.accept(false, modifiers),
            "quickInput.acceptInBackground" => self.accept(true, modifiers),
            "quickInput.hide" => self.hide(),
            "quickInput.toggleCheckbox" => self.toggle_checked(None),
            "history.showPrevious" | "history.showNext" => {
                let a = self.active.as_ref().unwrap();
                let value = if command == "history.showPrevious" {
                    self.history.show_previous(&a.text)
                } else {
                    self.history.show_next()
                };
                match value {
                    Some(v) => {
                        let a = self.active.as_mut().unwrap();
                        a.text = v;
                        a.cursor = a.text.len();
                        let (id, text) = (a.id, a.text.as_str().into());
                        Self::refilter(a);
                        self.events.emit(QuickInputEvent::Changed { id, text });
                        true
                    }
                    None => false,
                }
            }
            _ => {
                let a = self.active.as_mut().unwrap();
                let l = &mut a.list;
                match command {
                    "quickInput.next" => l.focus_next(1, true),
                    "quickInput.previous" => l.focus_previous(1, true),
                    "quickInput.first" => l.focus_first(),
                    "quickInput.last" => l.focus_last(),
                    "quickInput.pageNext" => l.focus_page_down(),
                    "quickInput.pagePrevious" => l.focus_page_up(),
                    "quickInput.nextSeparator" => l.focus_next_separator(false),
                    "quickInput.previousSeparator" => l.focus_previous_separator(false),
                    "quickInput.nextSeparatorWithQuickAccessFallback" => {
                        l.focus_next_separator(true)
                    }
                    "quickInput.previousSeparatorWithQuickAccessFallback" => {
                        l.focus_previous_separator(true)
                    }
                    "quickInput.toggleHover" => a.hover_visible = !a.hover_visible,
                    _ => return Err(Error::NotFound),
                }
                true
            }
        };
        Ok(handled)
    }

    pub fn context_keys(&self) -> Vec<(&'static str, Value)> {
        let Some(a) = self.active.as_ref() else {
            return vec![(ck::IN_QUICK_INPUT, Value::Bool(false))];
        };
        let mut keys = vec![
            (ck::IN_QUICK_INPUT, Value::Bool(true)),
            (
                ck::QUICK_INPUT_TYPE,
                Value::String(a.config.kind.as_str().into()),
            ),
            (ck::INPUT_FOCUS, Value::Bool(a.input_focused)),
            (
                ck::CURSOR_AT_END_OF_QUICK_INPUT_BOX,
                Value::Bool(a.input_focused && a.cursor == a.text.len()),
            ),
        ];
        keys.extend(self.history.context_keys(a.input_focused));
        keys
    }
}

impl QuickInput for QuickInputService {
    fn show(&mut self, options: QuickInputOptions) -> Result<QuickInputId> {
        self.show_with(options, QuickPickConfig::default())
    }
    fn update(&mut self, id: QuickInputId, items: Arc<dyn ListModel>) -> Result<()> {
        let a = self.require(id)?;
        a.list.set_model(items);
        if a.list.focus().is_none() {
            a.list.focus_first();
        }
        Ok(())
    }
    fn close(&mut self, id: QuickInputId) -> Result<()> {
        self.require(id)?;
        self.hide();
        Ok(())
    }
    fn events(&self) -> &dyn Events<QuickInputEvent> {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list_widget::StaticList;

    fn options(labels: &[&str], multi: bool) -> QuickInputOptions {
        QuickInputOptions {
            placeholder: Message::key("common.loading"),
            initial: "".into(),
            items: Arc::new(StaticList::from_labels(labels.iter().copied())),
            multi_select: multi,
        }
    }

    fn record(q: &QuickInputService) -> (Arc<Mutex<Vec<QuickInputEvent>>>, Subscription) {
        let log = Arc::new(Mutex::new(vec![]));
        let l = log.clone();
        let sub = q
            .events()
            .subscribe(Arc::new(move |e| l.lock().unwrap().push(e)));
        (log, sub)
    }

    fn focused(q: &QuickInputService) -> Option<String> {
        let a = q.active()?;
        a.list.focused_id().map(|i| i.0.to_string())
    }

    #[test]
    fn typing_filters_and_enter_accepts_the_focused_item() {
        let mut q = QuickInputService::default();
        let (log, _s) = record(&q);
        let id = q
            .show(options(&["UTF-8", "GBK", "UTF-16 LE"], false))
            .unwrap();
        assert_eq!(focused(&q).as_deref(), Some("UTF-8"));
        q.set_text("16", 2);
        assert_eq!(q.active().unwrap().list.len(), 1);
        assert_eq!(focused(&q).as_deref(), Some("UTF-16 LE"));
        assert!(q
            .execute("quickInput.accept", Modifiers::default())
            .unwrap());
        assert!(q.active().is_none());
        let log = log.lock().unwrap();
        assert!(
            matches!(&log[0], QuickInputEvent::Changed { id: i, text } if *i == id && &**text == "16")
        );
        assert!(
            matches!(&log[1], QuickInputEvent::Accepted { items, .. } if items[0].0.as_ref() == "UTF-16 LE")
        );
    }

    #[test]
    fn navigation_wraps_pages_and_jumps_between_groups() {
        let labels: Vec<String> = (0..40).map(|i| format!("item{i}")).collect();
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let mut q = QuickInputService::default();
        q.set_viewport(22.0 * 10.0, 300.0);
        q.show_with(
            options(&refs, false),
            QuickPickConfig {
                separators: vec![(0, None), (10, Some("最近".into())), (25, None)],
                ..Default::default()
            },
        )
        .unwrap();
        let m = Modifiers::default();
        q.execute("quickInput.previous", m).unwrap();
        assert_eq!(focused(&q).as_deref(), Some("item39"), "previous wraps");
        q.execute("quickInput.next", m).unwrap();
        assert_eq!(focused(&q).as_deref(), Some("item0"), "next wraps");
        q.execute("quickInput.pageNext", m).unwrap();
        assert_eq!(focused(&q).as_deref(), Some("item9"));
        q.execute("quickInput.pageNext", m).unwrap();
        assert_eq!(focused(&q).as_deref(), Some("item19"));
        q.execute("quickInput.pagePrevious", m).unwrap();
        assert_eq!(focused(&q).as_deref(), Some("item10"));
        q.execute("quickInput.nextSeparator", m).unwrap();
        assert_eq!(focused(&q).as_deref(), Some("item25"));
        q.execute("quickInput.nextSeparator", m).unwrap();
        assert_eq!(
            focused(&q).as_deref(),
            Some("item0"),
            "wraps to first group"
        );
        q.execute("quickInput.last", m).unwrap();
        q.execute("quickInput.previousSeparator", m).unwrap();
        assert_eq!(
            focused(&q).as_deref(),
            Some("item25"),
            "start of current group"
        );
        q.execute("quickInput.previousSeparator", m).unwrap();
        assert_eq!(
            focused(&q).as_deref(),
            Some("item10"),
            "then previous group"
        );
        q.execute("quickInput.first", m).unwrap();
        assert_eq!(focused(&q).as_deref(), Some("item0"));
        // No groups: the quick-access fallback moves one item.
        q.show(options(&["a", "b"], false)).unwrap();
        q.execute("quickInput.nextSeparatorWithQuickAccessFallback", m)
            .unwrap();
        assert_eq!(focused(&q).as_deref(), Some("b"));
        q.execute("quickInput.previousSeparatorWithQuickAccessFallback", m)
            .unwrap();
        assert_eq!(focused(&q).as_deref(), Some("a"));
        q.execute("quickInput.nextSeparator", m).unwrap();
        assert_eq!(
            focused(&q).as_deref(),
            Some("a"),
            "no fallback without groups"
        );
    }

    #[test]
    fn background_accept_keeps_the_picker_and_hide_cancels() {
        let mut q = QuickInputService::default();
        let (log, _s) = record(&q);
        let picks = Arc::new(Mutex::new(vec![]));
        let p = picks.clone();
        let _s2 = q
            .pick_events()
            .subscribe(Arc::new(move |e| p.lock().unwrap().push(e)));
        let id = q.show(options(&["a", "b"], false)).unwrap();
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        q.execute("quickInput.acceptInBackground", ctrl).unwrap();
        assert!(q.active().is_some());
        assert!(matches!(
            &picks.lock().unwrap()[0],
            QuickPickEvent::Accepted { in_background: true, modifiers, .. } if modifiers.ctrl
        ));
        q.execute("quickInput.hide", Modifiers::default()).unwrap();
        assert!(q.active().is_none());
        let log = log.lock().unwrap();
        assert!(matches!(log.last(), Some(QuickInputEvent::Cancelled(i)) if *i == id));
        assert_eq!(
            log.iter()
                .filter(|e| matches!(e, QuickInputEvent::Accepted { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn checkboxes_hover_history_and_context_keys() {
        let mut q = QuickInputService::default();
        let m = Modifiers::default();
        q.show(options(&["a", "b", "c"], true)).unwrap();
        q.set_input_focused(false);
        q.execute("quickInput.toggleCheckbox", m).unwrap();
        q.execute("quickInput.next", m).unwrap();
        q.execute("quickInput.next", m).unwrap();
        q.execute("quickInput.toggleCheckbox", m).unwrap();
        assert_eq!(q.active().unwrap().checked.len(), 2);
        q.execute("quickInput.toggleHover", m).unwrap();
        assert!(q.active().unwrap().hover_visible);
        let keys = q.context_keys();
        assert!(keys.contains(&(ck::QUICK_INPUT_TYPE, Value::String("quickPick".into()))));
        assert!(keys.contains(&(ck::INPUT_FOCUS, Value::Bool(false))));
        q.set_text("x", 1);
        q.execute("quickInput.accept", m).unwrap();
        q.show(options(&["a"], false)).unwrap();
        assert!(q
            .context_keys()
            .contains(&(ck::CURSOR_AT_END_OF_QUICK_INPUT_BOX, Value::Bool(true))));
        assert!(q.execute("history.showPrevious", m).unwrap());
        assert_eq!(q.active().unwrap().text, "x");
        assert!(q.execute("history.showNext", m).unwrap());
        assert_eq!(q.active().unwrap().text, "");
        assert_eq!(q.execute("bogus", m), Err(Error::NotFound));
    }

    #[test]
    fn contract_update_close_and_replacement() {
        let mut q = QuickInputService::default();
        let (log, _s) = record(&q);
        let first = q.show(options(&["a"], false)).unwrap();
        let second = q.show(options(&["b"], false)).unwrap();
        assert!(matches!(log.lock().unwrap()[0], QuickInputEvent::Cancelled(i) if i == first));
        assert_eq!(
            q.update(first, Arc::new(StaticList::from_labels(["x"]))),
            Err(Error::NotFound)
        );
        q.update(second, Arc::new(StaticList::from_labels(["x", "y"])))
            .unwrap();
        assert_eq!(q.active().unwrap().list.len(), 2);
        q.close(second).unwrap();
        assert_eq!(q.close(second), Err(Error::NotFound));
    }
}
