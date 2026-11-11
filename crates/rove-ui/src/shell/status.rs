//! 状态栏容器（#16，UX_SPEC §1.5）。
//!
//! 条目分两步：装配期在 `ContributionRegistry` 登记位置（`StatusLeft` / `StatusRight`）、
//! 优先级（`order`，与 VS Code 的 priority 相同：**数值越大越靠左**）、标题（无障碍名称）、
//! 默认点击命令；运行期由条目的负责方用 [`StatusBar::set`] 提供内容（文本支持 `$(icon)`、
//! 悬停提示、点击命令、颜色级别）。没有内容的条目不显示。
//! 具体条目（行列、编码、索引状态……）属于 #27 / #36，本模块只提供容器。
use crate::contributions::{Contribution, ContributionRegistry, Placement};
use rove_base::{Error, Result};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusKind {
    #[default]
    Standard,
    /// `statusBarItem.prominent*`
    Prominent,
    /// `statusBarItem.warning*`
    Warning,
    /// `statusBarItem.error*`
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Right,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusEntry {
    /// 显示文本，支持 `$(图标名)` 与 `$(图标名~spin)`。
    pub text: String,
    pub tooltip: Option<String>,
    /// None 时用登记的默认命令。
    pub command: Option<Arc<str>>,
    pub kind: StatusKind,
}

/// 运行期内容表；只由 UI 线程修改。
#[derive(Default)]
pub struct StatusBar {
    entries: BTreeMap<String, StatusEntry>,
    revision: u64,
}

pub fn alignment(placement: &Placement) -> Option<Alignment> {
    match placement {
        Placement::StatusLeft => Some(Alignment::Left),
        Placement::StatusRight => Some(Alignment::Right),
        _ => None,
    }
}

impl StatusBar {
    /// 设置条目内容；条目必须先在登记表中以状态栏位置登记，否则 NotFound。
    pub fn set(
        &mut self,
        contributions: &ContributionRegistry,
        id: &str,
        entry: StatusEntry,
    ) -> Result<()> {
        let c = contributions.get(id).ok_or(Error::NotFound)?;
        alignment(&c.placement).ok_or(Error::InvalidInput)?;
        if self.entries.get(id) != Some(&entry) {
            self.entries.insert(id.to_owned(), entry);
            self.revision += 1;
        }
        Ok(())
    }
    pub fn remove(&mut self, id: &str) -> bool {
        let removed = self.entries.remove(id).is_some();
        self.revision += removed as u64;
        removed
    }
    pub fn get(&self, id: &str) -> Option<&StatusEntry> {
        self.entries.get(id)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// 一侧要显示的条目，从左到右排列：priority 大的在左，相同时按 ID 排序。
    pub fn items<'a>(
        &'a self,
        contributions: &'a ContributionRegistry,
        side: Alignment,
    ) -> Vec<(&'a str, &'a Contribution, &'a StatusEntry)> {
        let mut items: Vec<_> = contributions
            .entries()
            .filter(|(_, c)| alignment(&c.placement) == Some(side))
            .filter_map(|(id, c)| self.entries.get(id).map(|e| (id, c, e)))
            .collect();
        items.sort_by(|a, b| b.1.order.cmp(&a.1.order).then(a.0.cmp(b.0)));
        items
    }
    /// 状态栏键盘焦点顺序：左侧从左到右，然后右侧从左到右（与 VS Code 相同）。
    pub fn focus_order<'a>(&'a self, contributions: &'a ContributionRegistry) -> Vec<&'a str> {
        self.items(contributions, Alignment::Left)
            .into_iter()
            .chain(self.items(contributions, Alignment::Right))
            .map(|(id, _, _)| id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::Message;

    fn register(r: &mut ContributionRegistry, id: &str, placement: Placement, order: i32) {
        r.register(
            id,
            Contribution {
                title: Message::key("x"),
                placement,
                order,
                icon: None,
                command: None,
                when: None,
                factory_key: id.into(),
            },
        )
        .unwrap();
    }

    #[test]
    fn orders_by_priority_and_requires_registration() {
        let mut r = ContributionRegistry::default();
        register(&mut r, "a", Placement::StatusLeft, 1);
        register(&mut r, "b", Placement::StatusLeft, 9);
        register(&mut r, "c", Placement::StatusRight, 0);
        register(&mut r, "view", Placement::Panel, 0);
        let mut bar = StatusBar::default();
        for id in ["a", "b", "c"] {
            bar.set(
                &r,
                id,
                StatusEntry {
                    text: id.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        assert_eq!(
            bar.set(&r, "missing", StatusEntry::default()),
            Err(Error::NotFound)
        );
        assert_eq!(
            bar.set(&r, "view", StatusEntry::default()),
            Err(Error::InvalidInput)
        );
        let left: Vec<_> = bar
            .items(&r, Alignment::Left)
            .into_iter()
            .map(|i| i.0)
            .collect();
        assert_eq!(left, ["b", "a"]);
        assert_eq!(bar.focus_order(&r), ["b", "a", "c"]);
    }
}
