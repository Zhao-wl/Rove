//! Generic virtualized list, tree and quick input (#18), implementing the #1 contracts in
//! `crate::lists` (`ListModel`, `ListController`, `QuickInput`).
//!
//! Layers:
//! - UI-free core (no GPUI types): [`widget::ListWidget`] (focus/selection/scroll/find and every
//!   `list.*` command), [`tree::TreeModel`] (lazy children), [`quick_input::QuickInputService`]
//!   (every `quickInput.*` command), [`history::InputHistory`], [`matcher`], [`options`].
//! - GPUI views: [`view::ListView`], [`quick_input_view::QuickInputView`],
//!   [`text_input::TextInput`]; keys reach them through #14's keybinding service (`crate::keys`),
//!   which dispatches [`keys::action_for`] actions; the views publish VS Code context keys.
//! - Registration: [`registry`] (commands, strings, colors, bindings), [`options`] (settings).
//!
//! Views supply data (a `ListModel` or `TreeSource`) and optionally a row renderer; the component
//! never learns the item type.
pub mod history;
pub mod keys;
pub mod matcher;
pub mod options;
pub mod quick_input;
pub mod quick_input_view;
pub mod registry;
pub mod text_input;
pub mod tree;
pub mod view;
pub mod widget;

use crate::lists::{ItemId, ListModel, Row};
use rove_base::{Error, Result, Version};
use std::{ops::Range, sync::Arc};

/// Immutable in-memory list snapshot; build a new one (with a higher revision) to change it.
pub struct StaticList {
    rows: Arc<[Row]>,
    revision: Version,
}

impl StaticList {
    pub fn new(rows: impl Into<Arc<[Row]>>, revision: Version) -> Self {
        Self {
            rows: rows.into(),
            revision,
        }
    }

    /// Convenience: one flat row per label, ids equal to labels.
    pub fn from_labels<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        Self::new(
            labels
                .into_iter()
                .map(|l| plain_row(l, l))
                .collect::<Vec<_>>(),
            Version(0),
        )
    }
}

/// A flat, non-expandable row.
pub fn plain_row(id: &str, label: &str) -> Row {
    Row {
        id: ItemId(id.into()),
        label: label.into(),
        detail: None,
        depth: 0,
        expandable: false,
        expanded: false,
        disabled: false,
    }
}

impl ListModel for StaticList {
    fn revision(&self) -> Version {
        self.revision
    }
    fn count(&self) -> usize {
        self.rows.len()
    }
    fn rows(&self, r: Range<usize>) -> Result<Arc<[Row]>> {
        if r.start > r.end {
            return Err(Error::InvalidInput);
        }
        if r.start > self.rows.len() {
            return Err(Error::OutOfBounds);
        }
        Ok(Arc::from(&self.rows[r.start..r.end.min(self.rows.len())]))
    }
}
