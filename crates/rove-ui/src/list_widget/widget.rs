//! UI-free list/tree controller (#18): focus, selection, scrolling, find/filter, type navigation,
//! tree expansion and every `list.*` command. The GPUI view only translates input events and
//! paints what this reports, so all behavior is unit-testable without a window.
//!
//! Indexes are display indexes (after filtering/flattening) and are transient; identity across
//! refreshes is preserved through stable `ItemId`s (tree) or model indexes (flat filter).
use super::{
    matcher::{matcher_for, FindMode, MatchType, Matcher},
    options::{ListOptions, MultiSelectModifier, OpenMode, TypeNavigationMode},
    registry::context as ck,
    tree::{RowKind, TreeModel},
};
use crate::lists::{ItemId, ListController, ListModel, Navigation, Row};
use rove_base::{Cancellation, Error, Result, Value, Version};
use std::{collections::HashMap, ops::Range, sync::Arc};

/// Height of one row; `PerRow` is evaluated once per model revision (O(n)), `Fixed` is O(1).
#[derive(Clone)]
pub enum RowHeight {
    Fixed(f32),
    PerRow(Arc<dyn Fn(&Row) -> f32 + Send + Sync>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ListEvent {
    /// Keyboard `list.select`, `openMode` click, or selection-follows-focus.
    Open {
        ids: Arc<[ItemId]>,
        preserve_focus: bool,
        pinned: bool,
        side_by_side: bool,
    },
    FocusChanged(Option<ItemId>),
    SelectionChanged(Arc<[ItemId]>),
    Expanded {
        id: ItemId,
        expanded: bool,
    },
    ShowHover(ItemId),
    /// `shift+f10` / right click: the view registers menu content, the list only carries it.
    ContextMenu(Arc<[ItemId]>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub mode: FindMode,
    pub match_type: MatchType,
}

/// Value of the `listScrollAtBoundary` context key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollBoundary {
    None,
    Top,
    Bottom,
    Both,
}
impl ScrollBoundary {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Both => "both",
        }
    }
}

enum Backing {
    Flat {
        model: Arc<dyn ListModel>,
        /// display -> model index when filtering.
        filtered: Option<Vec<u32>>,
    },
    Tree(TreeModel),
}

enum Heights {
    Fixed(f32, usize),
    Prefix(Vec<f32>),
}
impl Heights {
    fn count(&self) -> usize {
        match self {
            Self::Fixed(_, n) => *n,
            Self::Prefix(p) => p.len() - 1,
        }
    }
    fn offset(&self, i: usize) -> f32 {
        match self {
            Self::Fixed(h, n) => *h * i.min(*n) as f32,
            Self::Prefix(p) => p[i.min(p.len() - 1)],
        }
    }
    fn height(&self, i: usize) -> f32 {
        match self {
            Self::Fixed(h, _) => *h,
            Self::Prefix(p) => p.get(i + 1).map_or(0.0, |e| e - p[i]),
        }
    }
    /// Index of the row containing `y`; `count()` when past the end.
    fn index_at(&self, y: f32) -> usize {
        if y < 0.0 {
            return 0;
        }
        match self {
            Self::Fixed(h, n) => ((y / h) as usize).min(*n),
            Self::Prefix(p) => p
                .partition_point(|&o| o <= y)
                .saturating_sub(1)
                .min(p.len() - 1),
        }
    }
}

struct Smooth {
    from: f32,
    to: f32,
    start: Option<f64>,
}

const SMOOTH_MS: f64 = 125.0;
const TYPE_NAV_TIMEOUT_MS: f64 = 800.0;
/// VS Code `list.scrollUp/Down` move the scroll position by 10 px.
const SCROLL_STEP: f32 = 10.0;

pub struct ListWidget {
    backing: Backing,
    revision: Version,
    row_height: RowHeight,
    heights: Heights,
    focus: Option<usize>,
    anchor: Option<usize>,
    selection: Vec<usize>,
    scroll_top: f32,
    scroll_left: f32,
    viewport: f32,
    viewport_width: f32,
    content_width: f32,
    smooth: Option<Smooth>,
    options: ListOptions,
    matcher: Arc<dyn Matcher>,
    custom_matcher: bool,
    /// `listSupportsMultiselect`.
    pub multi_select: bool,
    /// `listSupportsFind`.
    pub supports_find: bool,
    /// Selection follows keyboard focus (quick open / outline style lists).
    pub follow_focus: bool,
    find: FindState,
    filter_query: Option<(String, FindMode)>,
    separators: Vec<(usize, Option<Arc<str>>)>,
    display_separators: Vec<(usize, Option<Arc<str>>)>,
    loading_tail: bool,
    type_buffer: String,
    type_last: f64,
    type_triggered: bool,
    sticky_focus: Option<usize>,
    events: Vec<ListEvent>,
}

impl ListWidget {
    pub fn new(model: Arc<dyn ListModel>, row_height: RowHeight, options: ListOptions) -> Self {
        Self::with_backing(
            Backing::Flat {
                model,
                filtered: None,
            },
            row_height,
            options,
        )
    }

    pub fn tree(tree: TreeModel, row_height: RowHeight, options: ListOptions) -> Self {
        Self::with_backing(Backing::Tree(tree), row_height, options)
    }

    fn with_backing(backing: Backing, row_height: RowHeight, options: ListOptions) -> Self {
        let mut w = Self {
            backing,
            revision: Version(0),
            row_height,
            heights: Heights::Fixed(0.0, 0),
            focus: None,
            anchor: None,
            selection: vec![],
            scroll_top: 0.0,
            scroll_left: 0.0,
            viewport: 0.0,
            viewport_width: 0.0,
            content_width: 0.0,
            smooth: None,
            matcher: matcher_for(options.find_match_type),
            custom_matcher: false,
            find: FindState {
                open: false,
                query: String::new(),
                mode: options.find_mode,
                match_type: options.find_match_type,
            },
            options,
            multi_select: false,
            supports_find: true,
            follow_focus: false,
            filter_query: None,
            separators: vec![],
            display_separators: vec![],
            loading_tail: false,
            type_buffer: String::new(),
            type_last: f64::NEG_INFINITY,
            type_triggered: false,
            sticky_focus: None,
            events: vec![],
        };
        w.relayout();
        w
    }

    // ------------------------------------------------------------------ model access

    pub fn is_tree(&self) -> bool {
        matches!(self.backing, Backing::Tree(_))
    }

    pub fn tree_model(&self) -> Option<&TreeModel> {
        match &self.backing {
            Backing::Tree(t) => Some(t),
            Backing::Flat { .. } => None,
        }
    }

    /// Display row count (excludes the loading tail).
    pub fn len(&self) -> usize {
        match &self.backing {
            Backing::Flat { model, filtered } => filtered.as_ref().map_or(model.count(), Vec::len),
            Backing::Tree(t) => t.count(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn model_index(&self, i: usize) -> usize {
        match &self.backing {
            Backing::Flat {
                filtered: Some(f), ..
            } => f[i] as usize,
            _ => i,
        }
    }

    /// Display rows for `range` (clipped). O(log n + visible rows).
    pub fn display_rows(&self, range: Range<usize>) -> Arc<[Row]> {
        let end = range.end.min(self.len());
        if range.start >= end {
            return Arc::from([]);
        }
        match &self.backing {
            Backing::Tree(t) => t.rows(range.start..end).unwrap_or_else(|_| Arc::from([])),
            Backing::Flat {
                model,
                filtered: None,
            } => model
                .rows(range.start..end)
                .unwrap_or_else(|_| Arc::from([])),
            Backing::Flat {
                model,
                filtered: Some(f),
            } => {
                let mut out = Vec::with_capacity(end - range.start);
                let mut i = range.start;
                while i < end {
                    let start = f[i] as usize;
                    let mut j = i + 1;
                    while j < end && f[j] as usize == start + (j - i) {
                        j += 1;
                    }
                    if let Ok(rows) = model.rows(start..start + (j - i)) {
                        out.extend(rows.iter().cloned());
                    }
                    i = j;
                }
                out.into()
            }
        }
    }

    pub fn row(&self, i: usize) -> Option<Row> {
        self.display_rows(i..i + 1).first().cloned()
    }

    pub fn row_kind(&self, i: usize) -> RowKind {
        match &self.backing {
            Backing::Tree(t) => t.row_kind(i).unwrap_or(RowKind::Item),
            Backing::Flat { .. } => RowKind::Item,
        }
    }

    fn id_at(&self, i: usize) -> Option<ItemId> {
        match &self.backing {
            Backing::Tree(t) => t.id_at(i),
            _ => self.row(i).map(|r| r.id),
        }
    }

    fn label_at(&self, i: usize) -> Option<Arc<str>> {
        self.row(i).map(|r| r.label)
    }

    fn bump(&mut self) {
        self.revision.0 += 1;
    }

    /// Replaces a flat model (new revision from the data source). Focus and selection are kept by
    /// `ItemId`; the find filter is re-applied. Cost O(n) only when there are marks to restore.
    pub fn set_model(&mut self, model: Arc<dyn ListModel>) {
        let marks = self.capture_ids();
        if let Backing::Flat { model: m, filtered } = &mut self.backing {
            *m = model;
            *filtered = None;
        }
        self.reapply_filter();
        self.restore_ids(marks);
        self.bump();
    }

    /// Mutates the tree (e.g. to deliver pending children) keeping marks by `ItemId`.
    pub fn update_tree<R>(&mut self, f: impl FnOnce(&mut TreeModel) -> R) -> Option<R> {
        let marks = self.capture_ids();
        let Backing::Tree(t) = &mut self.backing else {
            return None;
        };
        let r = f(t);
        self.relayout();
        self.restore_ids(marks);
        self.bump();
        Some(r)
    }

    pub fn set_options(&mut self, options: ListOptions) {
        if !self.custom_matcher && options.find_match_type != self.options.find_match_type {
            self.matcher = matcher_for(options.find_match_type);
        }
        if !self.find.open {
            self.find.mode = options.find_mode;
            self.find.match_type = options.find_match_type;
        }
        self.options = options;
        self.clamp_scroll();
        self.bump();
    }

    pub fn options(&self) -> &ListOptions {
        &self.options
    }

    /// Installs the scorer (#24 replaces the simple matcher with VS Code's fuzzy scorer).
    pub fn set_matcher(&mut self, matcher: Arc<dyn Matcher>) {
        self.matcher = matcher;
        self.custom_matcher = true;
        self.reapply_filter_keeping_marks();
    }

    /// Group starts (model indexes) with optional labels, for `pickerGroup.*` separators.
    pub fn set_separators(&mut self, separators: Vec<(usize, Option<Arc<str>>)>) {
        self.separators = separators;
        self.separators.sort_by_key(|s| s.0);
        self.compute_display_separators();
        self.bump();
    }

    /// Group starts after filtering (display indexes).
    pub fn group_starts(&self) -> &[(usize, Option<Arc<str>>)] {
        &self.display_separators
    }

    /// Appends a non-focusable "loading" row at the bottom (e.g. quick open while scanning).
    pub fn set_loading_tail(&mut self, on: bool) {
        self.loading_tail = on;
        self.clamp_scroll();
        self.bump();
    }

    pub fn loading_tail(&self) -> bool {
        self.loading_tail
    }

    fn compute_display_separators(&mut self) {
        self.display_separators.clear();
        if self.separators.is_empty() {
            return;
        }
        let n = self.len();
        let mut d = 0;
        for (k, (start, label)) in self.separators.iter().enumerate() {
            let end = self.separators.get(k + 1).map_or(usize::MAX, |s| s.0);
            while d < n && self.model_index(d) < *start {
                d += 1;
            }
            if d < n && self.model_index(d) < end {
                self.display_separators.push((d, label.clone()));
            }
        }
    }

    fn relayout(&mut self) {
        let n = self.len();
        self.heights = match &self.row_height {
            RowHeight::Fixed(h) => Heights::Fixed(*h, n),
            RowHeight::PerRow(f) => {
                let mut p = Vec::with_capacity(n + 1);
                p.push(0.0);
                let mut acc = 0.0;
                let mut i = 0;
                while i < n {
                    let rows = self.display_rows(i..(i + 4096).min(n));
                    for r in rows.iter() {
                        acc += f(r);
                        p.push(acc);
                    }
                    i += rows.len().max(1);
                }
                Heights::Prefix(p)
            }
        };
        self.compute_display_separators();
        self.clamp_scroll();
    }

    // ------------------------------------------------------------------ geometry

    fn tail_height(&self) -> f32 {
        if !self.loading_tail {
            return 0.0;
        }
        match &self.heights {
            Heights::Fixed(h, _) => *h,
            Heights::Prefix(p) if p.len() > 1 => p[1],
            _ => 22.0,
        }
    }

    pub fn total_height(&self) -> f32 {
        self.heights.offset(self.heights.count()) + self.tail_height()
    }
    pub fn row_top(&self, i: usize) -> f32 {
        self.heights.offset(i)
    }
    pub fn row_height(&self, i: usize) -> f32 {
        self.heights.height(i)
    }
    pub fn scroll_top(&self) -> f32 {
        self.scroll_top
    }
    pub fn scroll_left(&self) -> f32 {
        self.scroll_left
    }
    pub fn viewport(&self) -> f32 {
        self.viewport
    }
    fn max_scroll(&self) -> f32 {
        (self.total_height() - self.viewport).max(0.0)
    }

    pub fn set_viewport(&mut self, height: f32, width: f32) {
        self.viewport = height.max(0.0);
        self.viewport_width = width.max(0.0);
        self.clamp_scroll();
    }

    /// Widest row measured by the view; enables horizontal scrolling when the setting is on.
    pub fn set_content_width(&mut self, width: f32) {
        self.content_width = width;
        self.scroll_left = self.scroll_left.clamp(0.0, self.max_scroll_left());
    }

    fn max_scroll_left(&self) -> f32 {
        if self.options.horizontal_scrolling {
            (self.content_width - self.viewport_width).max(0.0)
        } else {
            0.0
        }
    }

    fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        self.scroll_top = self.scroll_top.clamp(0.0, max);
        if let Some(s) = self.smooth.as_mut() {
            s.to = s.to.clamp(0.0, max);
        }
    }

    /// Row under content-space `y` (scroll offset already added); None past the last row.
    pub fn index_at(&self, y: f32) -> Option<usize> {
        let i = self.heights.index_at(y);
        (y >= 0.0 && i < self.len()).then_some(i)
    }

    /// Display rows intersecting the viewport (the only rows the view lays out and paints).
    pub fn visible_range(&self) -> Range<usize> {
        let n = self.len();
        let start = self.heights.index_at(self.scroll_top).min(n);
        let end = (self.heights.index_at(self.scroll_top + self.viewport) + 1).min(n);
        start..end.max(start)
    }

    pub fn scroll_boundary(&self) -> ScrollBoundary {
        let top = self.scroll_top <= 0.0;
        let bottom = self.total_height() - self.viewport - self.scroll_top < 1.0;
        match (top, bottom) {
            (true, true) => ScrollBoundary::Both,
            (true, false) => ScrollBoundary::Top,
            (false, true) => ScrollBoundary::Bottom,
            (false, false) => ScrollBoundary::None,
        }
    }

    /// Sets the scroll position immediately (cancels any smooth animation).
    pub fn set_scroll_top(&mut self, y: f32) {
        self.smooth = None;
        self.scroll_top = y.clamp(0.0, self.max_scroll());
    }

    /// Scrolls by `dy`, animated when `workbench.list.smoothScrolling` is on.
    pub fn scroll_by(&mut self, dy: f32) {
        let base = self.smooth.as_ref().map_or(self.scroll_top, |s| s.to);
        let to = (base + dy).clamp(0.0, self.max_scroll());
        if self.options.smooth_scrolling {
            self.smooth = Some(Smooth {
                from: self.scroll_top,
                to,
                start: None,
            });
        } else {
            self.set_scroll_top(to);
        }
    }

    pub fn is_animating(&self) -> bool {
        self.smooth.is_some()
    }

    /// Advances the smooth-scroll animation to `now_ms`; returns true while still animating.
    pub fn tick(&mut self, now_ms: f64) -> bool {
        let Some(s) = self.smooth.as_mut() else {
            return false;
        };
        let start = *s.start.get_or_insert(now_ms);
        let t = ((now_ms - start) / SMOOTH_MS).clamp(0.0, 1.0) as f32;
        // Ease-out cubic, like VS Code's smooth scrolling.
        let eased = 1.0 - (1.0 - t).powi(3);
        self.scroll_top = s.from + (s.to - s.from) * eased;
        if t >= 1.0 {
            self.scroll_top = s.to;
            self.smooth = None;
        }
        self.smooth.is_some()
    }

    /// Mouse wheel in pixels. Alt multiplies by `fastScrollSensitivity`; Shift scrolls
    /// horizontally when `horizontalScrolling` is enabled.
    pub fn wheel(&mut self, dx: f32, dy: f32, m: Modifiers) {
        let mut k = self.options.mouse_wheel_scroll_sensitivity;
        if m.alt {
            k *= self.options.fast_scroll_sensitivity;
        }
        let (dx, dy) = if m.shift && dx == 0.0 {
            (dy, 0.0)
        } else {
            (dx, dy)
        };
        if self.options.horizontal_scrolling && dx != 0.0 {
            self.scroll_left = (self.scroll_left + dx * k).clamp(0.0, self.max_scroll_left());
        }
        if dy != 0.0 {
            self.scroll_by(dy * k);
        }
    }

    /// Click on the scrollbar track at `y` (viewport coordinates). `scrollByPage` pages toward the
    /// click; otherwise jumps so the thumb centers on it.
    pub fn scrollbar_click(&mut self, y: f32) {
        let total = self.total_height();
        if total <= self.viewport || self.viewport <= 0.0 {
            return;
        }
        let thumb = (self.viewport * self.viewport / total).max(20.0);
        let thumb_top = self.scroll_top / self.max_scroll() * (self.viewport - thumb);
        if self.options.scroll_by_page {
            if y < thumb_top {
                self.scroll_by(-self.viewport);
            } else if y > thumb_top + thumb {
                self.scroll_by(self.viewport);
            }
        } else {
            let ratio = ((y - thumb / 2.0) / (self.viewport - thumb)).clamp(0.0, 1.0);
            self.set_scroll_top(ratio * self.max_scroll());
        }
    }

    /// Scrolls minimally so row `i` is fully visible (VS Code `reveal`).
    pub fn reveal(&mut self, i: usize) {
        if i >= self.len() {
            return;
        }
        let top = self.row_top(i) - self.sticky_height_for(i);
        let bottom = self.row_top(i) + self.row_height(i);
        if top < self.scroll_top {
            self.set_scroll_top(top.max(0.0));
        } else if bottom > self.scroll_top + self.viewport {
            self.set_scroll_top(bottom - self.viewport);
        }
    }

    // ------------------------------------------------------------------ focus & selection

    pub fn focus(&self) -> Option<usize> {
        self.focus
    }
    pub fn anchor(&self) -> Option<usize> {
        self.anchor
    }
    pub fn selection_indices(&self) -> &[usize] {
        &self.selection
    }
    pub fn is_selected(&self, i: usize) -> bool {
        self.selection.binary_search(&i).is_ok()
    }

    pub fn focused_id(&self) -> Option<ItemId> {
        self.focus.and_then(|i| self.id_at(i))
    }

    pub fn selected_ids(&self) -> Arc<[ItemId]> {
        self.selection
            .iter()
            .filter_map(|&i| self.id_at(i))
            .collect()
    }

    fn focusable(&self, i: usize) -> bool {
        self.row_kind(i) == RowKind::Item
    }

    /// Focuses display index `i` (or clears with None) without touching selection.
    pub fn set_focus(&mut self, i: Option<usize>) {
        self.focus = i.filter(|&i| i < self.len());
        self.sticky_focus = None;
    }

    pub fn set_selection(&mut self, mut indices: Vec<usize>) {
        let n = self.len();
        indices.retain(|&i| i < n && self.focusable(i));
        indices.sort_unstable();
        indices.dedup();
        self.selection = indices;
    }

    pub fn select_ids(&mut self, ids: &[ItemId]) {
        let marks = Marks {
            focus: None,
            anchor: None,
            selection: ids.to_vec(),
        };
        let keep_focus = self.focus;
        self.restore_ids(marks);
        self.focus = keep_focus;
    }

    pub fn focus_id(&mut self, id: &ItemId) -> Result<()> {
        let i = self.find_id(id).ok_or(Error::NotFound)?;
        self.set_focus(Some(i));
        self.reveal(i);
        Ok(())
    }

    fn find_id(&self, id: &ItemId) -> Option<usize> {
        match &self.backing {
            Backing::Tree(t) => t.index_of(id),
            _ => {
                let n = self.len();
                let mut i = 0;
                while i < n {
                    let rows = self.display_rows(i..(i + 4096).min(n));
                    if let Some(k) = rows.iter().position(|r| &r.id == id) {
                        return Some(i + k);
                    }
                    i += rows.len().max(1);
                }
                None
            }
        }
    }

    fn next_focusable(&self, from: isize, step: isize, wrap: bool) -> Option<usize> {
        let n = self.len() as isize;
        if n == 0 {
            return None;
        }
        let mut i = from;
        for _ in 0..n {
            if i < 0 || i >= n {
                if !wrap {
                    return None;
                }
                i = i.rem_euclid(n);
            }
            if self.focusable(i as usize) {
                return Some(i as usize);
            }
            i += step;
        }
        None
    }

    /// VS Code `focusNext(n, loop)`: no focus starts from the first row.
    pub fn focus_next(&mut self, n: usize, wrap: bool) {
        let from = self.focus.map_or(0, |f| f as isize + n as isize);
        let next = self
            .next_focusable(from, 1, wrap)
            .or_else(|| self.focus.filter(|_| !wrap));
        self.move_focus(next);
    }

    pub fn focus_previous(&mut self, n: usize, wrap: bool) {
        let from = self.focus.map_or(0, |f| f as isize - n as isize);
        let prev = self
            .next_focusable(from, -1, wrap)
            .or_else(|| self.focus.filter(|_| !wrap));
        self.move_focus(prev);
    }

    pub fn focus_first(&mut self) {
        let f = self.next_focusable(0, 1, false);
        self.move_focus(f);
    }

    pub fn focus_last(&mut self) {
        let f = self.next_focusable(self.len() as isize - 1, -1, false);
        self.move_focus(f);
    }

    fn move_focus(&mut self, i: Option<usize>) {
        if let Some(i) = i {
            self.set_focus(Some(i));
            self.reveal(i);
        }
    }

    /// VS Code `focusNextPage`.
    pub fn focus_page_down(&mut self) {
        for _ in 0..self.len().max(1) {
            let n = self.len();
            if n == 0 {
                return;
            }
            let mut last = self
                .heights
                .index_at(self.scroll_top + self.viewport)
                .min(n);
            last = last.saturating_sub(1);
            let current = self.focus;
            if current != Some(last) && current.is_none_or(|c| last > c) {
                let good = self
                    .next_focusable(last as isize, -1, false)
                    .unwrap_or(last);
                self.set_focus(Some(good));
                self.reveal(good);
                return;
            }
            let previous = self.scroll_top;
            let mut next = previous + self.viewport;
            if current.is_some_and(|c| last > c) {
                next -= self.row_height(last);
            }
            self.set_scroll_top(next);
            if self.scroll_top == previous {
                // Already at the bottom: keep focus on the last row.
                if let Some(c) = current {
                    self.reveal(c);
                }
                return;
            }
            self.focus = None;
        }
    }

    /// VS Code `focusPreviousPage`.
    pub fn focus_page_up(&mut self) {
        for _ in 0..self.len().max(1) {
            let n = self.len();
            if n == 0 {
                return;
            }
            let first = if self.scroll_top <= 0.0 {
                self.heights.index_at(0.0)
            } else {
                (self.heights.index_at(self.scroll_top - 1.0) + 1).min(n - 1)
            };
            let current = self.focus;
            if current != Some(first) && current.is_none_or(|c| c >= first) {
                let good = self
                    .next_focusable(first as isize, 1, false)
                    .unwrap_or(first);
                self.set_focus(Some(good));
                self.reveal(good);
                return;
            }
            let previous = self.scroll_top;
            self.set_scroll_top(previous - self.viewport);
            if self.scroll_top == previous {
                if let Some(c) = current {
                    self.reveal(c);
                }
                return;
            }
            self.focus = None;
        }
    }

    fn select_range_to_focus(&mut self) {
        let (Some(a), Some(f)) = (self.anchor.or(self.focus), self.focus) else {
            return;
        };
        self.set_selection((a.min(f)..=a.max(f)).collect());
    }

    fn expand_selection(&mut self, down: bool) {
        if !self.multi_select {
            return;
        }
        if self.anchor.is_none() {
            self.anchor = self.focus;
        }
        if down {
            self.focus_next(1, false);
        } else {
            self.focus_previous(1, false);
        }
        self.select_range_to_focus();
    }

    fn open(&mut self, ids: Arc<[ItemId]>, preserve_focus: bool, pinned: bool, side: bool) {
        if !ids.is_empty() {
            self.events.push(ListEvent::Open {
                ids,
                preserve_focus,
                pinned,
                side_by_side: side,
            });
        }
    }

    fn open_focused_selection(&mut self, preserve_focus: bool, pinned: bool, side: bool) {
        // Folders (expandable tree rows) toggle instead of opening.
        let ids: Arc<[ItemId]> = self
            .selection
            .iter()
            .filter(|&&i| !(self.is_tree() && self.is_expandable(i)))
            .filter_map(|&i| self.id_at(i))
            .collect();
        self.open(ids, preserve_focus, pinned, side);
    }

    fn follow(&mut self) {
        if self.follow_focus {
            if let Some(f) = self.focus {
                self.set_selection(vec![f]);
                self.anchor = Some(f);
                self.open_focused_selection(true, false, false);
            }
        }
    }

    // ------------------------------------------------------------------ tree

    fn tree_mut(&mut self) -> Option<&mut TreeModel> {
        match &mut self.backing {
            Backing::Tree(t) => Some(t),
            Backing::Flat { .. } => None,
        }
    }

    fn is_expandable(&self, i: usize) -> bool {
        self.tree_model().is_some_and(|t| t.is_expandable(i))
    }

    fn capture_ids(&self) -> Marks {
        Marks {
            focus: self.focus.and_then(|i| self.id_at(i)),
            anchor: self.anchor.and_then(|i| self.id_at(i)),
            selection: self
                .selection
                .iter()
                .filter_map(|&i| self.id_at(i))
                .collect(),
        }
    }

    fn restore_ids(&mut self, marks: Marks) {
        if marks.focus.is_none() && marks.anchor.is_none() && marks.selection.is_empty() {
            self.focus = None;
            self.anchor = None;
            self.selection.clear();
            return;
        }
        let lookup: Lookup = match &self.backing {
            Backing::Tree(t) => Box::new(move |id| t.index_of(id)),
            _ => {
                let n = self.len();
                let mut map = HashMap::with_capacity(marks.selection.len() + 2);
                let wanted: std::collections::HashSet<&str> = marks
                    .selection
                    .iter()
                    .chain(marks.focus.iter())
                    .chain(marks.anchor.iter())
                    .map(|id| &*id.0)
                    .collect();
                let mut i = 0;
                while i < n && map.len() < wanted.len() {
                    let rows = self.display_rows(i..(i + 4096).min(n));
                    for (k, r) in rows.iter().enumerate() {
                        if wanted.contains(&*r.id.0) {
                            map.insert(r.id.0.clone(), i + k);
                        }
                    }
                    i += rows.len().max(1);
                }
                Box::new(move |id| map.get(&*id.0).copied())
            }
        };
        let focus = marks.focus.as_ref().and_then(|id| {
            lookup(id).or_else(|| {
                // A tree focus hidden by collapse/filter restore moves to its visible ancestor.
                self.tree_model()
                    .and_then(|t| t.visible_index_or_ancestor(id))
            })
        });
        let anchor = marks.anchor.as_ref().and_then(&lookup);
        let selection: Vec<usize> = marks.selection.iter().filter_map(&lookup).collect();
        drop(lookup);
        self.focus = focus;
        self.anchor = anchor;
        self.set_selection(selection);
    }

    fn set_expanded_at(&mut self, i: usize, expanded: bool) -> bool {
        let marks = self.capture_ids();
        let id = self.id_at(i);
        let changed = self
            .tree_mut()
            .is_some_and(|t| t.set_expanded_at(i, expanded));
        if changed {
            self.relayout();
            self.restore_ids(marks);
            if let Some(id) = id {
                self.events.push(ListEvent::Expanded { id, expanded });
            }
            self.bump();
        }
        changed
    }

    /// Contract-level expand by id (tree only).
    pub fn expand_id(&mut self, id: &ItemId, expanded: bool) -> Result<bool> {
        let Backing::Tree(t) = &self.backing else {
            return Err(Error::Unsupported);
        };
        match t.index_of(id) {
            Some(i) => Ok(self.set_expanded_at(i, expanded)),
            None => {
                // Not displayed (inside a collapsed node) but known: change state anyway.
                let marks = self.capture_ids();
                let changed = self.tree_mut().unwrap().set_expanded(id, expanded)?;
                self.relayout();
                self.restore_ids(marks);
                Ok(changed)
            }
        }
    }

    fn expand_focused(&mut self) {
        let Some(f) = self.focus else { return };
        if !self.set_expanded_at(f, true) {
            if let Some(c) = self.tree_model().and_then(|t| t.first_child_index(f)) {
                self.set_focus(Some(c));
                self.reveal(c);
            }
        }
    }

    fn collapse_focused(&mut self) {
        let Some(f) = self.focus else { return };
        if !self.set_expanded_at(f, false) {
            if let Some(p) = self.tree_model().and_then(|t| t.parent_index(f)) {
                self.set_focus(Some(p));
                self.reveal(p);
            }
        }
    }

    fn collapse_all(&mut self) {
        let Some(top) = self.focus.map(|f| {
            self.tree_model()
                .and_then(|t| t.ancestors(f).first().copied())
                .unwrap_or(f)
        }) else {
            if let Some(t) = self.tree_mut() {
                t.collapse_all();
                self.relayout();
                self.bump();
            }
            return;
        };
        let top_id = self.id_at(top);
        let marks = self.capture_ids();
        if let Some(t) = self.tree_mut() {
            t.collapse_all();
        }
        self.relayout();
        self.restore_ids(marks);
        // Focus falls back to the outermost visible ancestor, as in VS Code.
        if self.focus.is_none() {
            self.focus = top_id.and_then(|id| self.find_id(&id));
        }
        if let Some(f) = self.focus {
            self.reveal(f);
        }
        self.bump();
    }

    fn toggle_expand_focused(&mut self) -> bool {
        let Some(f) = self.focus else { return false };
        if !self.is_expandable(f) {
            return false;
        }
        let expanded = self.tree_model().is_some_and(|t| t.can_collapse(f));
        self.set_expanded_at(f, !expanded)
    }

    /// VS Code tree `selectAll`: siblings of the focus; pressing again widens to the parent scope.
    fn select_all(&mut self) {
        if !self.multi_select {
            return;
        }
        let Some(t) = self.tree_model() else {
            self.set_selection((0..self.len()).collect());
            return;
        };
        let start = match self.focus {
            Some(f) if !self.is_selected(f) || self.selection.is_empty() => Some(f),
            _ => self.selection.first().copied().or(self.focus),
        };
        let mut scope = start.and_then(|s| t.parent_index(s));
        let collect = |scope: Option<usize>| -> Vec<usize> { t.scope_range(scope).collect() };
        let mut new = collect(scope);
        if new.len() == self.selection.len() && scope.is_some() {
            scope = scope.and_then(|s| t.parent_index(s));
            new = collect(scope);
        }
        let new: Vec<usize> = new.into_iter().filter(|&i| self.focusable(i)).collect();
        self.set_selection(new);
    }

    // ------------------------------------------------------------------ sticky scroll

    /// Ancestors of the first visible row pinned at the top (outermost first).
    pub fn sticky_rows(&self) -> Vec<usize> {
        let Some(t) = self.tree_model() else {
            return vec![];
        };
        if !self.options.sticky_scroll || self.scroll_top <= 0.0 {
            return vec![];
        }
        let mut first = self.heights.index_at(self.scroll_top);
        // Rows hidden under the sticky band itself also count (iterate until stable).
        let mut rows = vec![];
        for _ in 0..self.options.sticky_scroll_max_items + 1 {
            if first >= self.len() {
                break;
            }
            rows = t.ancestors(first);
            rows.truncate(self.options.sticky_scroll_max_items);
            let band: f32 = rows.iter().map(|&r| self.row_height(r)).sum();
            let next = self.heights.index_at(self.scroll_top + band);
            if next == first || next >= self.len() {
                break;
            }
            first = next;
        }
        rows
    }

    fn sticky_height_for(&self, _i: usize) -> f32 {
        // Revealing above the viewport ignores the sticky band; VS Code reveals under it, which
        // only shifts by whole rows. Keep reveal simple and stable.
        0.0
    }

    pub fn sticky_focus(&self) -> Option<usize> {
        self.sticky_focus
            .and_then(|k| self.sticky_rows().get(k).copied())
    }

    /// Moves keyboard focus into the sticky band (`treestickyScrollFocused`).
    pub fn focus_sticky(&mut self, k: usize) {
        if k < self.sticky_rows().len() {
            self.sticky_focus = Some(k);
        }
    }

    fn leave_sticky(&mut self, target: usize) {
        self.sticky_focus = None;
        self.set_focus(Some(target));
        self.reveal(target);
    }

    // ------------------------------------------------------------------ find / filter

    pub fn find_state(&self) -> &FindState {
        &self.find
    }

    pub fn open_find(&mut self) -> bool {
        if !self.supports_find {
            return false;
        }
        if !self.find.open {
            self.find = FindState {
                open: true,
                query: String::new(),
                mode: self.options.find_mode,
                match_type: self.options.find_match_type,
            };
            if !self.custom_matcher {
                self.matcher = matcher_for(self.find.match_type);
            }
        }
        self.bump();
        true
    }

    pub fn close_find(&mut self) {
        if !self.find.open {
            return;
        }
        self.find.open = false;
        self.find.query.clear();
        self.set_filter_query(None);
    }

    pub fn set_find_query(&mut self, query: &str) {
        if !self.find.open {
            self.open_find();
        }
        self.find.query = query.to_string();
        let q = (!query.is_empty()).then(|| (query.to_string(), self.find.mode));
        self.set_filter_query(q);
        self.focus_first_match();
    }

    pub fn toggle_find_mode(&mut self) {
        self.find.mode = match self.find.mode {
            FindMode::Highlight => FindMode::Filter,
            FindMode::Filter => FindMode::Highlight,
        };
        let q = self.find.query.clone();
        self.set_find_query(&q);
    }

    pub fn toggle_find_match_type(&mut self) {
        self.find.match_type = match self.find.match_type {
            MatchType::Fuzzy => MatchType::Contiguous,
            MatchType::Contiguous => MatchType::Fuzzy,
        };
        if !self.custom_matcher {
            self.matcher = matcher_for(self.find.match_type);
        }
        let q = self.find.query.clone();
        self.set_find_query(&q);
    }

    /// Generic filter used by quick input and the contract `filter()`: `Filter` hides
    /// non-matching rows, `Highlight` only highlights. `None` shows everything.
    pub fn set_filter_query(&mut self, query: Option<(String, FindMode)>) {
        self.filter_query = query;
        self.reapply_filter_keeping_marks();
    }

    fn reapply_filter_keeping_marks(&mut self) {
        if self.is_tree() {
            let marks = self.capture_ids();
            self.reapply_filter();
            self.restore_ids(marks);
        } else {
            // Flat: remap via model indexes, O(marks · log n).
            let to_model = |w: &Self, i: usize| w.model_index(i);
            let focus = self.focus.map(|i| to_model(self, i));
            let anchor = self.anchor.map(|i| to_model(self, i));
            let selection: Vec<usize> = self.selection.iter().map(|&i| to_model(self, i)).collect();
            self.reapply_filter();
            let back = |w: &Self, m: usize| match &w.backing {
                Backing::Flat {
                    filtered: Some(f), ..
                } => f.binary_search(&(m as u32)).ok(),
                _ => Some(m),
            };
            self.focus = focus.and_then(|m| back(self, m));
            self.anchor = anchor.and_then(|m| back(self, m));
            let sel = selection
                .into_iter()
                .filter_map(|m| back(self, m))
                .collect();
            self.set_selection(sel);
        }
        self.bump();
    }

    fn reapply_filter(&mut self) {
        let filter = match &self.filter_query {
            Some((q, FindMode::Filter)) if !q.is_empty() => Some(q.clone()),
            _ => None,
        };
        let matcher = self.matcher.clone();
        match &mut self.backing {
            Backing::Tree(t) => t.set_filter(filter.as_deref().map(|q| (&*matcher, q))),
            Backing::Flat { model, filtered } => {
                *filtered = filter.map(|q| {
                    let n = model.count();
                    let mut out = Vec::new();
                    let mut i = 0;
                    while i < n {
                        let rows = model
                            .rows(i..(i + 4096).min(n))
                            .unwrap_or_else(|_| Arc::from([]));
                        for (k, r) in rows.iter().enumerate() {
                            if matcher.score(&q, &r.label).is_some() {
                                out.push((i + k) as u32);
                            }
                        }
                        i += rows.len().max(1);
                    }
                    out
                });
            }
        }
        self.relayout();
    }

    /// Highlight ranges for row `i` under the active find/filter query.
    pub fn highlights(&self, i: usize) -> Vec<Range<usize>> {
        match &self.filter_query {
            Some((q, _)) if !q.is_empty() => self
                .label_at(i)
                .and_then(|l| self.matcher.score(q, &l))
                .map(|m| m.ranges)
                .unwrap_or_default(),
            _ => vec![],
        }
    }

    fn is_match(&self, i: usize) -> bool {
        match &self.filter_query {
            Some((q, _)) if !q.is_empty() => self
                .label_at(i)
                .is_some_and(|l| self.matcher.score(q, &l).is_some()),
            _ => true,
        }
    }

    /// False when a non-empty query matches nothing (`listFilterWidget.noMatchesOutline`).
    pub fn has_matches(&self) -> bool {
        match &self.filter_query {
            Some((q, mode)) if !q.is_empty() => match mode {
                FindMode::Filter => !self.is_empty(),
                FindMode::Highlight => (0..self.len()).any(|i| self.is_match(i)),
            },
            _ => true,
        }
    }

    fn focus_first_match(&mut self) {
        if self
            .focus
            .is_some_and(|f| self.is_match(f) && self.focusable(f))
        {
            return;
        }
        if let Some(i) = (0..self.len()).find(|&i| self.focusable(i) && self.is_match(i)) {
            self.set_focus(Some(i));
            self.reveal(i);
        }
    }

    /// Type navigation (find widget closed). Returns false when not handled (trigger mode).
    pub fn type_char(&mut self, text: &str, now_ms: f64) -> bool {
        if self.find.open || text.is_empty() {
            return false;
        }
        if now_ms - self.type_last > TYPE_NAV_TIMEOUT_MS {
            self.type_buffer.clear();
            if self.options.type_navigation_mode == TypeNavigationMode::Trigger
                && !self.type_triggered
            {
                return false;
            }
            self.type_triggered = false;
        }
        self.type_last = now_ms;
        self.type_buffer.push_str(text);
        let n = self.len();
        if n == 0 {
            return true;
        }
        let start = match self.focus {
            Some(f) if self.type_buffer.chars().count() == 1 => f + 1,
            Some(f) => f,
            None => 0,
        };
        let needle: Vec<char> = self
            .type_buffer
            .chars()
            .flat_map(char::to_lowercase)
            .collect();
        for k in 0..n {
            let i = (start + k) % n;
            let Some(label) = self.label_at(i) else {
                continue;
            };
            let mut l = label.chars().flat_map(char::to_lowercase);
            if self.focusable(i) && needle.iter().all(|c| l.next() == Some(*c)) {
                self.set_focus(Some(i));
                self.reveal(i);
                self.follow();
                break;
            }
        }
        true
    }

    // ------------------------------------------------------------------ mouse

    /// Row click. `on_twistie` = the expand arrow. Honors `multiSelectModifier`, `openMode`
    /// and `tree.expandMode`.
    pub fn click(&mut self, i: usize, m: Modifiers, click_count: u32, on_twistie: bool) {
        if i >= self.len() || !self.focusable(i) {
            return;
        }
        let before = self.snapshot();
        self.sticky_focus = None;
        let (toggle_mod, side_mod) = match self.options.multi_select_modifier {
            MultiSelectModifier::CtrlCmd => (m.ctrl || m.meta, m.alt),
            MultiSelectModifier::Alt => (m.alt, m.ctrl || m.meta),
        };
        if self.multi_select && m.shift {
            if self.anchor.is_none() {
                self.anchor = self.focus.or(Some(i));
            }
            self.set_focus(Some(i));
            self.select_range_to_focus();
        } else if self.multi_select && toggle_mod {
            self.set_focus(Some(i));
            self.anchor = Some(i);
            let mut s = self.selection.clone();
            match s.binary_search(&i) {
                Ok(k) => {
                    s.remove(k);
                }
                Err(k) => s.insert(k, i),
            }
            self.selection = s;
        } else {
            self.set_focus(Some(i));
            self.anchor = Some(i);
            self.set_selection(vec![i]);
            let expandable = self.is_expandable(i);
            let toggle = expandable
                && (on_twistie
                    || match self.options.expand_mode {
                        OpenMode::SingleClick => click_count == 1,
                        OpenMode::DoubleClick => click_count == 2,
                    });
            if toggle {
                self.toggle_expand_focused();
            } else if !expandable {
                let open = match self.options.open_mode {
                    OpenMode::SingleClick => true,
                    OpenMode::DoubleClick => click_count >= 2,
                };
                if open {
                    // Single click opens a preview keeping focus; double click pins.
                    let pinned = click_count >= 2;
                    self.open_focused_selection(!pinned, pinned, side_mod);
                }
            }
        }
        self.emit_changes(before);
        self.bump();
    }

    /// Click on a sticky row: reveal and focus that node (VS Code behavior).
    pub fn click_sticky(&mut self, k: usize) {
        if let Some(&target) = self.sticky_rows().get(k) {
            let before = self.snapshot();
            let top = self.row_top(target);
            self.set_scroll_top(top);
            self.leave_sticky(target);
            self.set_selection(vec![target]);
            self.anchor = Some(target);
            self.emit_changes(before);
        }
    }

    pub fn context_menu(&mut self) {
        let ids = if self.focus.is_some_and(|f| self.is_selected(f)) {
            self.selected_ids()
        } else {
            self.focused_id().into_iter().collect()
        };
        self.events.push(ListEvent::ContextMenu(ids));
    }

    // ------------------------------------------------------------------ commands

    pub fn take_events(&mut self) -> Vec<ListEvent> {
        std::mem::take(&mut self.events)
    }

    fn snapshot(&self) -> (Option<usize>, Vec<usize>, usize) {
        (self.focus, self.selection.clone(), self.len())
    }

    fn emit_changes(&mut self, before: (Option<usize>, Vec<usize>, usize)) {
        let (focus, selection, len) = before;
        let structure = len != self.len();
        if focus != self.focus || structure {
            let id = self.focused_id();
            self.events.push(ListEvent::FocusChanged(id));
        }
        if selection != self.selection {
            let ids = self.selected_ids();
            self.events.push(ListEvent::SelectionChanged(ids));
        }
    }

    /// Runs a `list.*` command by its VS Code ID. `Ok(false)` = known ID but nothing to do in the
    /// current state; unknown ID => NotFound.
    pub fn execute(&mut self, command: &str) -> Result<bool> {
        let before = self.snapshot();
        let sticky = self.sticky_focus();
        let handled = match command {
            "list.focusDown" | "list.focusAnyDown" => {
                self.focus_next(1, false);
                true
            }
            "list.focusUp" | "list.focusAnyUp" => {
                // Up on the first row under the sticky band enters the band (VS Code).
                let rows = self.sticky_rows();
                if command == "list.focusUp"
                    && !rows.is_empty()
                    && self.focus.is_some()
                    && self.focus == Some(self.first_row_under_sticky())
                {
                    self.sticky_focus = Some(rows.len() - 1);
                } else {
                    self.focus_previous(1, false);
                }
                true
            }
            "list.focusFirst" | "list.focusAnyFirst" => {
                self.focus_first();
                true
            }
            "list.focusLast" | "list.focusAnyLast" => {
                self.focus_last();
                true
            }
            "list.focusPageDown" => {
                self.focus_page_down();
                true
            }
            "list.focusPageUp" => {
                self.focus_page_up();
                true
            }
            "list.scrollDown" => {
                self.scroll_by(SCROLL_STEP);
                true
            }
            "list.scrollUp" => {
                self.scroll_by(-SCROLL_STEP);
                true
            }
            "list.select" => {
                let Some(f) = self.focus else {
                    return Ok(false);
                };
                if self.is_tree() && self.is_expandable(f) {
                    self.toggle_expand_focused();
                }
                self.set_selection(vec![f]);
                self.anchor = Some(f);
                self.open_focused_selection(false, true, false);
                true
            }
            "list.clear" => {
                if !self.selection.is_empty() {
                    self.selection.clear();
                } else if self.focus.is_some() {
                    self.focus = None;
                }
                self.anchor = None;
                true
            }
            "list.selectAll" => {
                self.select_all();
                self.multi_select
            }
            "list.toggleSelection" => {
                let Some(f) = self.focus else {
                    return Ok(false);
                };
                let mut s = self.selection.clone();
                match s.binary_search(&f) {
                    Ok(k) => {
                        s.remove(k);
                    }
                    Err(k) => {
                        if !self.multi_select {
                            s.clear();
                            s.push(f);
                        } else {
                            s.insert(k, f);
                        }
                    }
                }
                self.selection = s;
                self.anchor = Some(f);
                true
            }
            "list.expandSelectionDown" => {
                self.expand_selection(true);
                self.multi_select
            }
            "list.expandSelectionUp" => {
                self.expand_selection(false);
                self.multi_select
            }
            "list.showHover" => match self.focused_id() {
                Some(id) => {
                    self.events.push(ListEvent::ShowHover(id));
                    true
                }
                None => false,
            },
            "list.expand" => {
                self.expand_focused();
                self.is_tree()
            }
            "list.collapse" => {
                self.collapse_focused();
                self.is_tree()
            }
            "list.collapseAll" => {
                self.collapse_all();
                self.is_tree()
            }
            "list.toggleExpand" => {
                if !self.toggle_expand_focused() {
                    // Leaf (or flat list): select while keeping focus, like VS Code.
                    if let Some(f) = self.focus {
                        self.set_selection(vec![f]);
                        self.anchor = Some(f);
                        self.open_focused_selection(true, false, false);
                    }
                }
                true
            }
            "list.find" => self.open_find(),
            "list.closeFind" => {
                let open = self.find.open;
                self.close_find();
                open
            }
            "list.triggerTypeNavigation" => {
                self.type_triggered = true;
                self.type_last = f64::NEG_INFINITY;
                true
            }
            "list.stickyScroll.collapse" => match sticky {
                Some(s) => {
                    let id = self.id_at(s);
                    self.set_expanded_at(s, false);
                    if let Some(i) = id.and_then(|id| self.find_id(&id)) {
                        self.leave_sticky(i);
                    }
                    true
                }
                None => false,
            },
            "list.stickyScrollselect" => match sticky {
                Some(s) => {
                    self.leave_sticky(s);
                    self.set_selection(vec![s]);
                    self.anchor = Some(s);
                    self.open_focused_selection(false, true, false);
                    true
                }
                None => false,
            },
            "list.stickyScrolltoggleExpand" => match sticky {
                Some(s) => {
                    let id = self.id_at(s);
                    let expanded = self.tree_model().is_some_and(|t| t.can_collapse(s));
                    self.set_expanded_at(s, !expanded);
                    if let Some(i) = id.and_then(|id| self.find_id(&id)) {
                        self.leave_sticky(i);
                    }
                    true
                }
                None => false,
            },
            _ => return Err(Error::NotFound),
        };
        if matches!(
            command,
            "list.focusDown"
                | "list.focusUp"
                | "list.focusFirst"
                | "list.focusLast"
                | "list.focusPageDown"
                | "list.focusPageUp"
        ) && self.focus != before.0
        {
            self.follow();
        }
        self.emit_changes(before);
        self.bump();
        Ok(handled)
    }

    fn first_row_under_sticky(&self) -> usize {
        let band: f32 = self.sticky_rows().iter().map(|&r| self.row_height(r)).sum();
        self.heights.index_at(self.scroll_top + band)
    }

    /// Context keys for when-clauses (`listFocus` is added by the view while it holds focus).
    pub fn context_keys(&self) -> Vec<(&'static str, Value)> {
        let mut keys = vec![
            (
                ck::LIST_SUPPORTS_MULTISELECT,
                Value::Bool(self.multi_select),
            ),
            (
                ck::LIST_HAS_SELECTION_OR_FOCUS,
                Value::Bool(self.focus.is_some() || !self.selection.is_empty()),
            ),
            (
                ck::LIST_SCROLL_AT_BOUNDARY,
                Value::String(self.scroll_boundary().as_str().into()),
            ),
            (ck::LIST_SUPPORTS_FIND, Value::Bool(self.supports_find)),
            (ck::TREE_FIND_OPEN, Value::Bool(self.find.open)),
            (
                ck::TREE_STICKY_SCROLL_FOCUSED,
                Value::Bool(self.sticky_focus().is_some()),
            ),
        ];
        if let (Some(t), Some(f)) = (self.tree_model(), self.focus) {
            keys.push((
                ck::TREE_ELEMENT_CAN_COLLAPSE,
                Value::Bool(t.can_collapse(f)),
            ));
            keys.push((ck::TREE_ELEMENT_CAN_EXPAND, Value::Bool(t.can_expand(f))));
            keys.push((
                ck::TREE_ELEMENT_HAS_PARENT,
                Value::Bool(t.parent_index(f).is_some()),
            ));
            keys.push((
                ck::TREE_ELEMENT_HAS_CHILD,
                Value::Bool(t.first_child_index(f).is_some()),
            ));
        } else {
            for k in [
                ck::TREE_ELEMENT_CAN_COLLAPSE,
                ck::TREE_ELEMENT_CAN_EXPAND,
                ck::TREE_ELEMENT_HAS_PARENT,
                ck::TREE_ELEMENT_HAS_CHILD,
            ] {
                keys.push((k, Value::Bool(false)));
            }
        }
        keys
    }

    // ------------------------------------------------------------------ quick-input helpers

    /// Quick pick: first row of the next group (wraps). Without groups behaves like `next`.
    pub fn focus_next_separator(&mut self, fallback: bool) {
        if self.display_separators.is_empty() {
            if fallback {
                self.focus_next(1, true);
            }
            return;
        }
        let f = self.focus;
        let next = self
            .display_separators
            .iter()
            .map(|s| s.0)
            .find(|&s| f.is_none_or(|f| s > f))
            .unwrap_or(self.display_separators[0].0);
        self.move_focus(Some(next));
    }

    /// Quick pick: first row of the current group, or of the previous group when already there.
    pub fn focus_previous_separator(&mut self, fallback: bool) {
        if self.display_separators.is_empty() {
            if fallback {
                self.focus_previous(1, true);
            }
            return;
        }
        let f = self.focus.unwrap_or(0);
        let starts: Vec<usize> = self.display_separators.iter().map(|s| s.0).collect();
        let current = starts.iter().rposition(|&s| s <= f);
        let target = match current {
            Some(k) if starts[k] < f => starts[k],
            Some(0) | None => *starts.last().unwrap(),
            Some(k) => starts[k - 1],
        };
        self.move_focus(Some(target));
    }

    pub fn revision(&self) -> Version {
        self.revision
    }
}

type Lookup<'a> = Box<dyn Fn(&ItemId) -> Option<usize> + 'a>;

struct Marks {
    focus: Option<ItemId>,
    anchor: Option<ItemId>,
    selection: Vec<ItemId>,
}

impl ListModel for ListWidget {
    fn revision(&self) -> Version {
        self.revision
    }
    fn count(&self) -> usize {
        self.len()
    }
    fn rows(&self, r: Range<usize>) -> Result<Arc<[Row]>> {
        if r.start > r.end {
            return Err(Error::InvalidInput);
        }
        if r.start > self.len() {
            return Err(Error::OutOfBounds);
        }
        Ok(self.display_rows(r))
    }
}

impl ListController for ListWidget {
    fn navigate(&mut self, action: Navigation) -> Result<()> {
        let id = match action {
            Navigation::Next => "list.focusDown",
            Navigation::Previous => "list.focusUp",
            Navigation::First => "list.focusFirst",
            Navigation::Last => "list.focusLast",
            Navigation::PageUp => "list.focusPageUp",
            Navigation::PageDown => "list.focusPageDown",
            Navigation::Parent => {
                let p = self
                    .focus
                    .and_then(|f| self.tree_model().and_then(|t| t.parent_index(f)));
                if let Some(p) = p {
                    self.set_focus(Some(p));
                    self.reveal(p);
                }
                return Ok(());
            }
            Navigation::Expand => "list.expand",
            Navigation::Collapse => "list.collapse",
            Navigation::Accept => "list.select",
            Navigation::Cancel => "list.clear",
        };
        self.execute(id).map(|_| ())
    }
    fn filter(&mut self, text: Arc<str>, cancel: Cancellation) -> Result<()> {
        cancel.check()?;
        let q = (!text.is_empty()).then(|| (text.to_string(), FindMode::Filter));
        self.set_filter_query(q);
        cancel.check()
    }
    fn selection(&self) -> Arc<[ItemId]> {
        self.selected_ids()
    }
    fn expand(&mut self, id: &ItemId, expanded: bool) -> Result<()> {
        self.expand_id(id, expanded).map(|_| ())
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
pub(crate) mod tests;
