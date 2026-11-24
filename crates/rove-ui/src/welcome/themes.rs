//! 颜色主题选择器的模型（#28）：候选列表与“预览 / 还原 / 确认”状态机。
//!
//! **实时预览不写文件**：移动选中项只在内存中换主题（`ThemeService::preview`），
//! `Esc` 还原成打开选择器时的那一个，`Enter` 才通过 #13 的保留格式写入 `workbench.colorTheme`。

use rove_theme::{ThemeCatalog, ThemeKind};
use std::sync::Arc;

/// 列表里的一项。
#[derive(Clone, Debug)]
pub struct ThemeItem {
    /// 用于实时预览和还原的目录标识。
    pub id: Arc<str>,
    /// 显示名称。
    pub label: Arc<str>,
    pub kind: ThemeKind,
    pub from_extension: bool,
}

/// 选择器的一次会话。
#[derive(Clone, Debug)]
pub struct ThemePicker {
    pub items: Vec<ThemeItem>,
    /// 打开选择器时生效的主题 ID（`Esc` 还原到它）。
    pub original: Arc<str>,
    /// 当前预览中的主题 ID（没有预览时是 `original`）。
    previewed: Arc<str>,
    /// 是否已经确认（`Enter`）：确认后关闭浮层不再还原。
    accepted: bool,
}

impl ThemePicker {
    /// 内置主题按深浅排列，扩展贡献的主题放在独立分组。
    pub fn new(catalog: &ThemeCatalog, built_in: &ThemeCatalog, current: &str) -> Self {
        let mut items: Vec<ThemeItem> = catalog
            .entries()
            .iter()
            .map(|e| ThemeItem {
                id: e.id.clone(),
                label: e.label.clone(),
                kind: e.kind,
                from_extension: built_in.get(&e.id).is_none(),
            })
            .collect();
        items.sort_by_key(|i| (i.from_extension, group_of(i.kind)));
        let original: Arc<str> = match items.iter().find(|i| &*i.id == current) {
            Some(item) => item.id.clone(),
            None => Arc::from(current),
        };
        Self {
            items,
            previewed: original.clone(),
            original,
            accepted: false,
        }
    }

    /// 打开时选中的那一项（当前主题）。
    pub fn initial_index(&self) -> Option<usize> {
        self.items.iter().position(|i| i.id == self.original)
    }

    pub fn id_at(&self, index: usize) -> Option<&Arc<str>> {
        self.items.get(index).map(|i| &i.id)
    }

    pub fn previewed(&self) -> &Arc<str> {
        &self.previewed
    }

    pub fn accepted(&self) -> bool {
        self.accepted
    }

    /// 选中项移动到 `index`：返回需要预览的主题 ID（与当前预览相同时返回 `None`，不重复加载）。
    pub fn focus(&mut self, index: usize) -> Option<Arc<str>> {
        let id = self.items.get(index)?.id.clone();
        if id == self.previewed {
            return None;
        }
        self.previewed = id.clone();
        Some(id)
    }

    /// `Enter`：确认当前预览的主题；返回要写入 `workbench.colorTheme` 的值。
    /// 与打开时相同（用户只是确认了当前主题）时返回 `None`，不写文件。
    pub fn accept(&mut self, index: Option<usize>) -> Option<Arc<str>> {
        if let Some(index) = index {
            if let Some(id) = self.items.get(index).map(|i| i.id.clone()) {
                self.previewed = id;
            }
        }
        self.accepted = true;
        if self.previewed == self.original {
            return None;
        }
        self.items
            .iter()
            .find(|item| item.id == self.previewed)
            .map(|item| {
                if item.from_extension {
                    item.label.clone()
                } else {
                    item.id.clone()
                }
            })
    }

    /// `Esc` 或失去焦点关闭：返回要还原的主题 ID（没有预览过、或已确认时返回 `None`）。
    pub fn cancel(&mut self) -> Option<Arc<str>> {
        if self.accepted || self.previewed == self.original {
            return None;
        }
        self.previewed = self.original.clone();
        Some(self.original.clone())
    }
}

/// 深色（含高对比深色）排在浅色前面。
fn group_of(kind: ThemeKind) -> u8 {
    match kind {
        ThemeKind::Dark => 0,
        ThemeKind::HighContrastDark => 1,
        ThemeKind::Light => 2,
        ThemeKind::HighContrastLight => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker() -> ThemePicker {
        let built_in = ThemeCatalog::builtin();
        ThemePicker::new(&built_in, &built_in, "Dark 2026")
    }

    #[test]
    fn extension_selection_previews_by_id_and_writes_its_label() {
        let built_in = ThemeCatalog::builtin();
        let mut catalog = built_in.clone();
        let mut entry = built_in.entries()[0].clone();
        entry.id = "extension.theme-id".into();
        entry.label = "Extension Theme".into();
        catalog.add(entry).unwrap();
        let mut picker = ThemePicker::new(&catalog, &built_in, "Dark 2026");
        let index = picker.items.len() - 1;
        assert!(picker.items[index].from_extension);
        assert_eq!(picker.focus(index).as_deref(), Some("extension.theme-id"));
        assert_eq!(
            picker.accept(Some(index)).as_deref(),
            Some("Extension Theme")
        );
    }

    #[test]
    fn lists_builtin_themes_with_the_current_one_selected() {
        let p = picker();
        assert!(p.items.len() >= 10);
        assert_eq!(
            p.id_at(p.initial_index().unwrap()).unwrap().as_ref(),
            "Dark 2026"
        );
        // 深色在浅色前面。
        let first_light = p
            .items
            .iter()
            .position(|i| matches!(i.kind, ThemeKind::Light))
            .unwrap();
        let last_dark = p
            .items
            .iter()
            .rposition(|i| matches!(i.kind, ThemeKind::Dark))
            .unwrap();
        assert!(last_dark < first_light);
    }

    #[test]
    fn moving_the_selection_previews_and_escape_restores() {
        let mut p = picker();
        let light = p.items.iter().position(|i| &*i.id == "Light 2026").unwrap();
        assert_eq!(p.focus(light).unwrap().as_ref(), "Light 2026");
        // 同一项不重复预览。
        assert!(p.focus(light).is_none());
        assert_eq!(p.previewed().as_ref(), "Light 2026");
        assert_eq!(p.cancel().unwrap().as_ref(), "Dark 2026");
        assert_eq!(p.previewed().as_ref(), "Dark 2026");
        // 还原之后再取消没有动作。
        assert!(p.cancel().is_none());
    }

    #[test]
    fn enter_writes_once_and_closing_afterwards_does_not_restore() {
        let mut p = picker();
        let light = p.items.iter().position(|i| &*i.id == "Light 2026").unwrap();
        p.focus(light);
        assert_eq!(p.accept(Some(light)).unwrap().as_ref(), "Light 2026");
        assert!(p.accepted());
        assert!(p.cancel().is_none());
    }

    #[test]
    fn confirming_the_current_theme_writes_nothing() {
        let mut p = picker();
        let index = p.initial_index().unwrap();
        assert!(p.accept(Some(index)).is_none());
    }

    #[test]
    fn an_unknown_configured_theme_is_still_the_restore_target() {
        let built_in = ThemeCatalog::builtin();
        let mut p = ThemePicker::new(&built_in, &built_in, "Monokai");
        assert!(p.initial_index().is_none());
        let first = p.focus(0).unwrap();
        assert_ne!(first.as_ref(), "Monokai");
        assert_eq!(p.cancel().unwrap().as_ref(), "Monokai");
    }
}
