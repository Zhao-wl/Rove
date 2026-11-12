//! 命令面板（`>` 前缀、`workbench.action.showCommands`）。
//!
//! 规则同 VS Code `commandsQuickAccess`：过滤用 `WORD_FILTER`（前缀 / 单词 / 连续子串，**不是**
//! `fuzzyScorer`），最近使用的命令排在前面（按最近使用时间），其余按名称排序，分“最近使用”和“其他命令”
//! 两组；标签重复的命令在后面显示命令 ID；命令 ID 完全等于输入时也算匹配。
use super::Picks;
use crate::{list_widget::quick_input::RowDecor, lists::ItemId, lists::Row};
use rove_fuzzy::{utf16_to_byte_ranges, word_filter};
use std::sync::Arc;

/// 面板里的一条命令（标题已经按当前语言解析）。
#[derive(Clone, Debug)]
pub struct CommandEntry {
    pub id: Arc<str>,
    pub label: Arc<str>,
    /// 当前生效的键位（显示用，例如 `Ctrl+Shift+P`）。
    pub keybinding: Option<Arc<str>>,
}

/// 最近使用的命令，最近的在前。
#[derive(Clone, Debug, Default)]
pub struct CommandHistory {
    ids: Vec<Arc<str>>,
}

impl CommandHistory {
    pub const LIMIT: usize = 50;

    pub fn record(&mut self, id: &str) {
        self.ids.retain(|i| &**i != id);
        self.ids.insert(0, id.into());
        self.ids.truncate(Self::LIMIT);
    }

    /// 排名（0 最近）；没用过返回 `None`。
    pub fn rank(&self, id: &str) -> Option<usize> {
        self.ids.iter().position(|i| &**i == id)
    }

    pub fn ids(&self) -> &[Arc<str>] {
        &self.ids
    }
}

pub struct PaletteLabels {
    pub recent: Arc<str>,
    pub other: Arc<str>,
}

pub fn palette_picks(
    entries: &[CommandEntry],
    history: &CommandHistory,
    filter: &str,
    labels: &PaletteLabels,
) -> Picks {
    let filter = filter.trim();
    struct Hit<'a> {
        entry: &'a CommandEntry,
        highlights: Vec<std::ops::Range<usize>>,
        rank: Option<usize>,
    }
    let mut hits: Vec<Hit> = Vec::new();
    for entry in entries {
        let highlights = if filter.is_empty() {
            Some(Vec::new())
        } else if let Some(m) = word_filter(filter, &entry.label) {
            Some(utf16_to_byte_ranges(&entry.label, &m))
        } else if filter == &*entry.id {
            // 命令 ID 完全相同
            Some(Vec::new())
        } else {
            None
        };
        if let Some(highlights) = highlights {
            hits.push(Hit {
                entry,
                highlights,
                rank: history.rank(&entry.id),
            });
        }
    }
    // 最近使用的在前（更近的更靠前），其余按名称
    hits.sort_by(|a, b| match (a.rank, b.rank) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a
            .entry
            .label
            .to_lowercase()
            .cmp(&b.entry.label.to_lowercase())
            .then_with(|| a.entry.label.cmp(&b.entry.label))
            .then_with(|| a.entry.id.cmp(&b.entry.id)),
    });

    // 标签重复的命令显示命令 ID
    let mut counts = std::collections::HashMap::<&str, usize>::new();
    for h in &hits {
        *counts.entry(&h.entry.label).or_default() += 1;
    }

    let mut picks = Picks::default();
    let mut other_separator_pending = false;
    for (i, hit) in hits.iter().enumerate() {
        if i == 0 && hit.rank.is_some() {
            picks.separator(labels.recent.clone());
            other_separator_pending = true;
        }
        if other_separator_pending && hit.rank.is_none() {
            picks.separator(labels.other.clone());
            other_separator_pending = false;
        }
        let duplicate = counts.get(&*hit.entry.label).copied().unwrap_or(0) > 1;
        picks.push(
            Row {
                id: ItemId(hit.entry.id.clone()),
                label: hit.entry.label.clone(),
                detail: duplicate.then(|| hit.entry.id.clone()),
                depth: 0,
                expandable: false,
                expanded: false,
                disabled: false,
            },
            RowDecor {
                label_highlights: hit.highlights.clone(),
                detail_highlights: Vec::new(),
                trailing: hit.entry.keybinding.clone(),
            },
        );
    }
    picks
}
