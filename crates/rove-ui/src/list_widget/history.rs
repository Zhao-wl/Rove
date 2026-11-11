//! Input history for `history.showPrevious` / `history.showNext` (VS Code HistoryNavigator
//! semantics). Shared by quick input, the find widget (#21) and the search inputs (#25).
use super::registry::context as ck;
use rove_base::Value;

#[derive(Clone, Debug, Default)]
pub struct InputHistory {
    entries: Vec<String>,
    /// Index into `entries` while navigating; `None` = editing a fresh value.
    cursor: Option<usize>,
    /// The value being typed before navigation started (restored when walking past the end).
    draft: String,
    limit: usize,
}

impl InputHistory {
    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            ..Default::default()
        }
    }

    /// Records an accepted value (moves duplicates to the end, trims to the limit).
    pub fn add(&mut self, value: &str) {
        if value.is_empty() {
            return;
        }
        self.entries.retain(|e| e != value);
        self.entries.push(value.to_string());
        if self.entries.len() > self.limit {
            self.entries.remove(0);
        }
        self.cursor = None;
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn can_go_back(&self) -> bool {
        match self.cursor {
            None => !self.entries.is_empty(),
            Some(c) => c > 0,
        }
    }

    pub fn can_go_forward(&self) -> bool {
        self.cursor.is_some()
    }

    /// Returns the value to show, or None when there is nothing older.
    pub fn show_previous(&mut self, current: &str) -> Option<String> {
        let next = match self.cursor {
            None if !self.entries.is_empty() => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(c) if c > 0 => c - 1,
            _ => return None,
        };
        self.cursor = Some(next);
        Some(self.entries[next].clone())
    }

    /// Returns the value to show; walking past the newest entry restores the draft.
    pub fn show_next(&mut self) -> Option<String> {
        let c = self.cursor?;
        if c + 1 < self.entries.len() {
            self.cursor = Some(c + 1);
            Some(self.entries[c + 1].clone())
        } else {
            self.cursor = None;
            Some(std::mem::take(&mut self.draft))
        }
    }

    /// Typing resets navigation.
    pub fn reset(&mut self) {
        self.cursor = None;
    }

    pub fn context_keys(&self, focused: bool) -> Vec<(&'static str, Value)> {
        vec![
            (ck::HISTORY_NAVIGATION_WIDGET_FOCUS, Value::Bool(focused)),
            (
                ck::HISTORY_NAVIGATION_BACKWARDS_ENABLED,
                Value::Bool(self.can_go_back()),
            ),
            (
                ck::HISTORY_NAVIGATION_FORWARDS_ENABLED,
                Value::Bool(self.can_go_forward()),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_back_and_restores_the_draft() {
        let mut h = InputHistory::new(3);
        assert!(!h.can_go_back());
        for v in ["a", "b", "c", "b"] {
            h.add(v);
        }
        assert_eq!(h.entries(), ["a", "c", "b"]);
        assert_eq!(h.show_previous("dra").as_deref(), Some("b"));
        assert!(h.can_go_forward());
        assert_eq!(h.show_previous("ignored").as_deref(), Some("c"));
        assert_eq!(h.show_previous("").as_deref(), Some("a"));
        assert!(!h.can_go_back());
        assert_eq!(h.show_previous(""), None);
        assert_eq!(h.show_next().as_deref(), Some("c"));
        assert_eq!(h.show_next().as_deref(), Some("b"));
        assert_eq!(h.show_next().as_deref(), Some("dra"));
        assert!(!h.can_go_forward());
        h.add("d");
        assert_eq!(h.entries(), ["c", "b", "d"]);
    }
}
