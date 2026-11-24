//! Context basket entries store source locations and user notes.
use super::tracking::Anchor;
use rove_base::WorkspacePath;

pub const MAX_LINES: u64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineRange {
    first: u64,
    last: u64,
}

impl LineRange {
    pub fn new(first: u64, last: u64) -> Option<Self> {
        (first > 0 && first <= last && last - first < MAX_LINES).then_some(Self { first, last })
    }

    pub fn first(self) -> u64 {
        self.first
    }

    pub fn last(self) -> u64 {
        self.last
    }

    pub fn lines(self) -> u64 {
        self.last - self.first + 1
    }

    fn touches(self, other: Self) -> bool {
        self.first <= other.last.saturating_add(1) && other.first <= self.last.saturating_add(1)
    }

    fn covering(self, other: Self) -> Option<Self> {
        Self::new(self.first.min(other.first), self.last.max(other.last))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Availability {
    #[default]
    Current,
    Stale,
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub(super) id: u64,
    pub path: WorkspacePath,
    pub lines: LineRange,
    pub symbol: Option<String>,
    pub note: String,
    pub availability: Availability,
    pub anchor: Option<Anchor>,
}

impl Entry {
    pub fn new(path: WorkspacePath, lines: LineRange, symbol: Option<String>) -> Self {
        Self {
            id: 0,
            path,
            lines,
            symbol,
            note: String::new(),
            availability: Availability::Current,
            anchor: None,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Basket {
    pub introduction: String,
    items: Vec<Entry>,
    next_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertError {
    RangeTooLarge,
    TooManyEntries,
}

impl Basket {
    pub fn items(&self) -> &[Entry] {
        &self.items
    }

    /// Merge connected, current ranges in one transaction. Unavailable locations need reselecting.
    pub fn insert(&mut self, incoming: Entry) -> Result<u64, InsertError> {
        let mut lines = incoming.lines;
        let mut covered = vec![false; self.items.len()];
        loop {
            let before = lines;
            for (index, item) in self.items.iter().enumerate() {
                if !covered[index]
                    && item.availability == incoming.availability
                    && same_path(&item.path, &incoming.path)
                    && lines.touches(item.lines)
                {
                    let joined = lines
                        .covering(item.lines)
                        .ok_or(InsertError::RangeTooLarge)?;
                    covered[index] = true;
                    lines = joined;
                }
            }
            if before == lines {
                break;
            }
        }

        let Some(first) = covered.iter().position(|included| *included) else {
            let next_id = self
                .next_id
                .checked_add(1)
                .ok_or(InsertError::TooManyEntries)?;
            self.next_id = next_id;
            self.items.push(Entry {
                id: next_id,
                ..incoming
            });
            return Ok(next_id);
        };

        let mut merged = self.items[first].clone();
        if merged.lines != lines {
            merged.anchor = None;
        }
        merged.lines = lines;
        if merged.symbol != incoming.symbol {
            merged.symbol = None;
        }
        for (other, included) in self.items[first + 1..].iter().zip(&covered[first + 1..]) {
            if *included {
                if merged.symbol != other.symbol {
                    merged.symbol = None;
                }
                append_note(&mut merged.note, &other.note);
            }
        }
        append_note(&mut merged.note, &incoming.note);
        let id = merged.id;
        self.items[first] = merged;
        let mut index = 0;
        self.items.retain(|_| {
            let keep = index == first || !covered[index];
            index += 1;
            keep
        });
        Ok(id)
    }

    pub(super) fn restore(introduction: String, items: Vec<Entry>) -> Option<Self> {
        let mut ids = std::collections::BTreeSet::new();
        for entry in &items {
            if entry.id == 0 || !ids.insert(entry.id) {
                return None;
            }
        }
        let next_id = ids.last().copied().unwrap_or_default();
        Some(Self {
            introduction,
            items,
            next_id,
        })
    }

    pub fn bind_anchor(&mut self, id: u64, rows: &[&str]) -> bool {
        let Some(entry) = self.items.iter_mut().find(|entry| entry.id == id) else {
            return false;
        };
        let Some(anchor) = Anchor::capture(rows, entry.lines) else {
            entry.availability = Availability::Stale;
            return false;
        };
        entry.anchor = Some(anchor);
        entry.availability = Availability::Current;
        true
    }

    pub fn settle_change(&mut self, path: &WorkspacePath, rows: &[&str]) {
        for entry in self
            .items
            .iter_mut()
            .filter(|entry| same_path(&entry.path, path))
        {
            if let Some((range, anchor)) = entry
                .anchor
                .and_then(|anchor| anchor.relocate(entry.lines, rows))
            {
                entry.lines = range;
                entry.anchor = Some(anchor);
                entry.availability = Availability::Current;
            } else {
                entry.availability = Availability::Stale;
            }
        }
    }

    pub fn edit_note(&mut self, id: u64, note: String) -> bool {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.note = note;
            true
        } else {
            false
        }
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let len = self.items.len();
        self.items.retain(|item| item.id != id);
        len != self.items.len()
    }

    pub fn reorder_before(&mut self, id: u64, before: Option<u64>) -> bool {
        if before == Some(id) {
            return false;
        }
        let Some(source) = self.items.iter().position(|item| item.id == id) else {
            return false;
        };
        let target = if let Some(before) = before {
            let Some(target) = self.items.iter().position(|item| item.id == before) else {
                return false;
            };
            target
        } else {
            self.items.len()
        };
        let index = target - usize::from(source < target);
        if source == index {
            return false;
        }
        let entry = self.items.remove(source);
        self.items.insert(index, entry);
        true
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.introduction.clear();
    }

    pub fn rename(&mut self, from: &WorkspacePath, to: &WorkspacePath) {
        let prefix: Vec<_> = from.as_str().split('/').collect();
        for item in &mut self.items {
            let segments: Vec<_> = item.path.as_str().split('/').collect();
            if segments.len() < prefix.len()
                || !segments
                    .iter()
                    .zip(&prefix)
                    .all(|(a, b)| same_component(a, b))
            {
                continue;
            }
            let tail = &segments[prefix.len()..];
            let replacement = if tail.is_empty() {
                to.as_str().to_owned()
            } else {
                format!("{}/{}", to.as_str(), tail.join("/"))
            };
            if let Ok(path) = WorkspacePath::new(&replacement) {
                item.path = path;
            }
        }
    }

    pub fn mark_removed(&mut self, path: &WorkspacePath) {
        for entry in &mut self.items {
            if same_path(&entry.path, path) {
                entry.availability = Availability::Missing;
            }
        }
    }

    pub fn mark_modified(&mut self, path: &WorkspacePath) {
        for entry in &mut self.items {
            if same_path(&entry.path, path) {
                entry.availability = Availability::Stale;
            }
        }
    }
}

fn append_note(notes: &mut String, next: &str) {
    if !next.is_empty() {
        if !notes.is_empty() {
            notes.push('\n');
        }
        notes.push_str(next);
    }
}

fn same_path(left: &WorkspacePath, right: &WorkspacePath) -> bool {
    same_component(left.as_str(), right.as_str())
}

fn same_component(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right) || left.to_lowercase() == right.to_lowercase()
    } else {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, first: u64, last: u64, note: &str) -> Entry {
        let mut entry = Entry::new(
            WorkspacePath::new(path).unwrap(),
            LineRange::new(first, last).unwrap(),
            None,
        );
        entry.note = note.into();
        entry
    }

    #[test]
    fn merges_overlaps_and_adjacency_without_dropping_notes_or_identity() {
        let mut basket = Basket::default();
        let first = basket
            .insert(entry("Some/File.cs", 1, 3, "before"))
            .unwrap();
        let distinct = basket
            .insert(entry("Some/File.cs", 8, 10, "second"))
            .unwrap();
        assert_ne!(first, distinct);
        assert_eq!(
            basket.insert(entry("Some/File.cs", 4, 8, "bridge")),
            Ok(first)
        );
        assert_eq!(basket.items().len(), 1);
        assert_eq!(basket.items()[0].lines, LineRange::new(1, 10).unwrap());
        assert_eq!(basket.items()[0].note, "before\nsecond\nbridge");
    }

    #[test]
    fn rejects_oversized_merge_without_mutating_existing_range() {
        let mut basket = Basket::default();
        basket
            .insert(entry("x.txt", 1, 10_000, "original"))
            .unwrap();
        let prior = basket.items().to_vec();
        assert_eq!(
            basket.insert(entry("x.txt", 10_001, 10_001, "new")),
            Err(InsertError::RangeTooLarge)
        );
        assert_eq!(basket.items(), prior);
    }

    #[test]
    fn supports_moving_items_and_tracks_directory_renames_at_component_boundaries() {
        let mut basket = Basket::default();
        let first = basket.insert(entry("A/B/C", 1, 1, "")).unwrap();
        let second = basket.insert(entry("A/Backup/C", 1, 1, "")).unwrap();
        basket.rename(
            &WorkspacePath::new("A/B").unwrap(),
            &WorkspacePath::new("Z").unwrap(),
        );
        assert_eq!(basket.items()[0].path.as_str(), "Z/C");
        assert!(basket.reorder_before(second, Some(first)));
        assert_eq!(basket.items()[0].path.as_str(), "A/Backup/C");
        let path = WorkspacePath::new("Z/C").unwrap();
        basket.mark_removed(&path);
        assert_eq!(basket.items()[1].availability, Availability::Missing);
        basket.mark_modified(&path);
        assert_eq!(basket.items()[1].availability, Availability::Stale);
    }
}
