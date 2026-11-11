//! Context key service (#14): the `ContextKeys` implementation used by the workbench.
//!
//! Each UI owner sets the keys it is responsible for (focus, visibility, capabilities) when its
//! state changes; `when` clauses only read the cached values. Keys that nobody sets stay unknown
//! and evaluate false, which is how keys for features Rove does not build (terminal, debug, SCM...)
//! are handled — there is no list of "unsupported" keys to maintain.
use crate::{ContextChange, ContextKeys, ContextSnapshot};
use rove_base::{Error, Events, Result, Subscription, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};

/// Well-known context keys from `docs/compat/KEYBINDINGS.md` §6 that fall inside Rove's scope.
/// The owner column says which part of the product is expected to set the key.
pub mod keys {
    macro_rules! keys {
        ($($name:ident = $value:literal, $owner:literal;)*) => {
            $(#[doc = concat!("`", $value, "` — set by ", $owner)] pub const $name: &str = $value;)*
            /// Every key above, with the issue/area that sets it.
            pub const ALL: &[(&str, &str)] = &[$(($value, $owner)),*];
        };
    }
    keys! {
        EDITOR_TEXT_FOCUS = "editorTextFocus", "#19 code view";
        EDITOR_FOCUS = "editorFocus", "#19 code view";
        TEXT_INPUT_FOCUS = "textInputFocus", "#19 code view / #18 inputs";
        INPUT_FOCUS = "inputFocus", "#18 inputs";
        EDITOR_READONLY = "editorReadonly", "#17 editors";
        EDITOR_HAS_SELECTION = "editorHasSelection", "#45 editing";
        EDITOR_HAS_MULTIPLE_SELECTIONS = "editorHasMultipleSelections", "#45 editing";
        EDITOR_LANG_ID = "editorLangId", "#17 editors";
        RESOURCE_LANG_ID = "resourceLangId", "#17 editors";
        RESOURCE = "resource", "#17 editors";
        ACTIVE_EDITOR = "activeEditor", "#17 editors";
        ACTIVE_EDITOR_GROUP_EMPTY = "activeEditorGroupEmpty", "#17 editors";
        IN_QUICK_INPUT = "inQuickInput", "#18 quick input";
        QUICK_INPUT_TYPE = "quickInputType", "#18 quick input";
        IN_QUICK_OPEN = "inQuickOpen", "#24 quick open";
        IN_FILES_PICKER = "inFilesPicker", "#24 quick open";
        LIST_FOCUS = "listFocus", "#18 lists";
        TREE_ELEMENT_CAN_COLLAPSE = "treeElementCanCollapse", "#18 lists";
        TREE_ELEMENT_CAN_EXPAND = "treeElementCanExpand", "#18 lists";
        TREE_STICKY_SCROLL_FOCUSED = "treestickyScrollFocused", "#18 lists";
        FILES_EXPLORER_FOCUS = "filesExplorerFocus", "#23 explorer";
        EXPLORER_RESOURCE_IS_FOLDER = "explorerResourceIsFolder", "#23 explorer";
        FOLDERS_VIEW_VISIBLE = "foldersViewVisible", "#23 explorer";
        FOLDING_ENABLED = "foldingEnabled", "#20 code features";
        FIND_WIDGET_VISIBLE = "findWidgetVisible", "find widget";
        SEARCH_VIEWLET_VISIBLE = "searchViewletVisible", "search view";
        SEARCH_VIEWLET_FOCUS = "searchViewletFocus", "search view";
        IN_SEARCH_EDITOR = "inSearchEditor", "search view";
        BREADCRUMBS_VISIBLE = "breadcrumbsVisible", "breadcrumbs";
        BREADCRUMBS_ACTIVE = "breadcrumbsActive", "breadcrumbs";
        REFERENCE_SEARCH_VISIBLE = "referenceSearchVisible", "peek references";
        IN_REFERENCE_SEARCH_EDITOR = "inReferenceSearchEditor", "peek references";
        EDITOR_HOVER_FOCUSED = "editorHoverFocused", "hover";
        EDITOR_HAS_DEFINITION_PROVIDER = "editorHasDefinitionProvider", "#40 navigation";
        EDITOR_HAS_REFERENCE_PROVIDER = "editorHasReferenceProvider", "#40 navigation";
        CAN_NAVIGATE_BACK = "canNavigateBack", "#40 navigation";
        CAN_NAVIGATE_FORWARD = "canNavigateForward", "#40 navigation";
        NOTIFICATION_FOCUS = "notificationFocus", "#16 workbench";
        NOTIFICATION_TOASTS_VISIBLE = "notificationToastsVisible", "#16 workbench";
        FOCUSED_VIEW = "focusedView", "#16 workbench";
        STATUS_BAR_FOCUSED = "statusBarFocused", "#16 workbench";
        IN_KEYBINDINGS = "inKeybindings", "keybinding editor (P2)";
        IS_WINDOWS = "isWindows", "#14 platform defaults";
        IS_LINUX = "isLinux", "#14 platform defaults";
        IS_MAC = "isMac", "#14 platform defaults";
        IS_WEB = "isWeb", "#14 platform defaults";
    }
}

type Listener = Arc<dyn Fn(ContextChange) + Send + Sync>;
#[derive(Default)]
struct Listeners {
    next: u64,
    map: BTreeMap<u64, Listener>,
}

/// Broadcast hub for `ContextChange`; subscription drop unregisters.
#[derive(Default)]
struct Hub(Arc<Mutex<Listeners>>);
impl Events<ContextChange> for Hub {
    fn subscribe(&self, listener: Listener) -> Subscription {
        let mut l = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let id = l.next;
        l.next += 1;
        l.map.insert(id, listener);
        let weak = Arc::downgrade(&self.0);
        Subscription::new(move || {
            if let Some(hub) = weak.upgrade() {
                hub.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map
                    .remove(&id);
            }
        })
    }
}
impl Hub {
    fn publish(&self, keys: Vec<Arc<str>>) {
        if keys.is_empty() {
            return;
        }
        let listeners: Vec<Listener> = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .values()
            .cloned()
            .collect();
        let change = ContextChange { keys: keys.into() };
        for l in listeners {
            l(change.clone());
        }
    }
}

/// Thread-safe context key store. Reads are O(log keys); writes publish one `ContextChange`
/// after the lock is released, and only when a value actually changed.
#[derive(Default)]
pub struct ContextKeyService {
    values: RwLock<BTreeMap<Arc<str>, Value>>,
    events: Hub,
}
impl ContextKeyService {
    /// A service with the platform keys VS Code always defines (`isWindows`, `isWeb`...).
    pub fn with_platform_defaults() -> Self {
        let s = Self::default();
        let _ = s.set_many([
            (keys::IS_WINDOWS.into(), Some(Value::Bool(cfg!(windows)))),
            (
                keys::IS_LINUX.into(),
                Some(Value::Bool(cfg!(target_os = "linux"))),
            ),
            (
                keys::IS_MAC.into(),
                Some(Value::Bool(cfg!(target_os = "macos"))),
            ),
            (keys::IS_WEB.into(), Some(Value::Bool(false))),
        ]);
        s
    }
    /// Set several keys at once; listeners receive a single change listing the changed keys.
    pub fn set_many(
        &self,
        entries: impl IntoIterator<Item = (Arc<str>, Option<Value>)>,
    ) -> Result<()> {
        let mut changed = Vec::new();
        {
            let mut values = self.values.write().unwrap_or_else(|e| e.into_inner());
            for (key, value) in entries {
                if key.is_empty() {
                    return Err(Error::InvalidInput);
                }
                let changed_value = match value {
                    Some(v) if values.get(&key) == Some(&v) => false,
                    Some(v) => {
                        values.insert(key.clone(), v);
                        true
                    }
                    None => values.remove(&key).is_some(),
                };
                if !changed_value {
                    continue;
                }
                changed.push(key);
            }
        }
        self.events.publish(changed);
        Ok(())
    }
    /// Detached copy for background evaluation.
    pub fn snapshot(&self) -> ContextSnapshot {
        self.values
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}
impl ContextKeys for ContextKeyService {
    fn get(&self, key: &str) -> Option<Value> {
        self.values
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }
    fn set(&self, key: Arc<str>, value: Option<Value>) -> Result<()> {
        self.set_many([(key, value)])
    }
    fn changes(&self) -> &dyn Events<ContextChange> {
        &self.events
    }
}

/// A detached snapshot is read-only context: evaluation works, writes are unsupported.
impl ContextKeys for ContextSnapshot {
    fn get(&self, key: &str) -> Option<Value> {
        BTreeMap::get(self, key).cloned()
    }
    fn set(&self, _: Arc<str>, _: Option<Value>) -> Result<()> {
        Err(Error::Unsupported)
    }
    fn changes(&self) -> &dyn Events<ContextChange> {
        &rove_base::NoEvents
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn set_get_and_change_events() {
        let s = ContextKeyService::default();
        let seen = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let sink = seen.clone();
        let sub = s.changes().subscribe(Arc::new(move |c: ContextChange| {
            sink.lock()
                .unwrap()
                .push(c.keys.iter().map(|k| k.to_string()).collect())
        }));
        s.set(keys::EDITOR_TEXT_FOCUS.into(), Some(Value::Bool(true)))
            .unwrap();
        assert_eq!(s.get(keys::EDITOR_TEXT_FOCUS), Some(Value::Bool(true)));
        // Unchanged value: no event.
        s.set(keys::EDITOR_TEXT_FOCUS.into(), Some(Value::Bool(true)))
            .unwrap();
        s.set_many([
            (keys::LIST_FOCUS.into(), Some(Value::Bool(true))),
            (keys::EDITOR_TEXT_FOCUS.into(), None),
        ])
        .unwrap();
        // Removing a missing key: no event.
        s.set("never".into(), None).unwrap();
        assert_eq!(s.get(keys::EDITOR_TEXT_FOCUS), None);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                vec!["editorTextFocus".to_string()],
                vec!["listFocus".to_string(), "editorTextFocus".to_string()]
            ]
        );
        drop(sub);
        s.set(keys::LIST_FOCUS.into(), Some(Value::Bool(false)))
            .unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(s.set("".into(), None), Err(Error::InvalidInput));
    }

    #[test]
    fn listener_may_read_the_service() {
        let s = Arc::new(ContextKeyService::default());
        let hits = Arc::new(AtomicUsize::new(0));
        let (reader, h) = (s.clone(), hits.clone());
        let _sub = s.changes().subscribe(Arc::new(move |_| {
            // Published after unlocking: reading inside the callback must not deadlock.
            assert_eq!(reader.get("k"), Some(Value::Number(1.0)));
            h.fetch_add(1, Ordering::SeqCst);
        }));
        s.set("k".into(), Some(Value::Number(1.0))).unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn platform_defaults_and_snapshot() {
        let s = ContextKeyService::with_platform_defaults();
        assert_eq!(s.get(keys::IS_WEB), Some(Value::Bool(false)));
        assert_eq!(s.get(keys::IS_WINDOWS), Some(Value::Bool(cfg!(windows))));
        let snap = s.snapshot();
        assert_eq!(
            ContextKeys::get(&snap, keys::IS_WEB),
            Some(Value::Bool(false))
        );
        assert_eq!(
            ContextKeys::set(&snap, "x".into(), None),
            Err(Error::Unsupported)
        );
    }

    #[test]
    fn listed_keys_are_used_by_default_bindings() {
        let data = include_str!("../data/default-keybindings.json");
        for (key, _) in keys::ALL {
            assert!(
                data.contains(key) || matches!(*key, "isLinux" | "isMac" | "isWeb"),
                "{key} is not referenced by any default binding"
            );
        }
    }
}
