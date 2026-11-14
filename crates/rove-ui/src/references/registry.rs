//! 侧边「引用」视图的登记项（#60）：命令（VS Code ID）、界面字符串、视图贡献点、上下文键名，
//! 以及 `references.preferredLocation` 的**一处判定**。
//!
//! - 设置项 `references.preferredLocation` 由 `rove_config::register_builtin` 按 compat/SETTINGS.md
//!   登记，这里只读（[`preferred_location`]）；
//! - 默认键位来自 #14 生成的 VS Code 默认键位表（`shift+alt+f12`、`f4`、`shift+f4`），
//!   when 条件逐条照抄，这里不重复登记、也不附加任何键位；
//! - 颜色键复用 #18 列表与 #16 侧边栏已登记的键，这里不新增颜色。

use crate::contributions::{Contribution, ContributionRegistry, Placement};
use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_config::{Key, Scope, Settings};
use rove_i18n::{Message, StringTable};
use std::sync::Arc;

/// 面板里的「引用」视图（`KEYBINDINGS.md` §7 附录「P0 · 布局」，无默认键位）。
/// 同名的命令用于显示该视图，视图贡献点也用这个 ID。
pub const VIEW: &str = "workbench.view.extension.references-view";

pub const FIND_REFERENCES: &str = "references-view.findReferences";
pub const FIND_IMPLEMENTATIONS: &str = "references-view.findImplementations";
pub const NEXT: &str = "references-view.next";
pub const PREV: &str = "references-view.prev";
pub const CLEAR: &str = "references-view.clear";
pub const CLEAR_HISTORY: &str = "references-view.clearHistory";
pub const COPY: &str = "references-view.copy";
pub const COPY_ALL: &str = "references-view.copyAll";
pub const COPY_PATH: &str = "references-view.copyPath";
pub const PICK_FROM_HISTORY: &str = "references-view.pickFromHistory";
pub const REFIND: &str = "references-view.refind";
pub const REFRESH: &str = "references-view.refresh";
pub const REMOVE_REFERENCE_ITEM: &str = "references-view.removeReferenceItem";
pub const TREE_FOCUS: &str = "references-view.tree.focus";
pub const TREE_RESET_VIEW_LOCATION: &str = "references-view.tree.resetViewLocation";

/// 调用层级与类型层级（P2）：**只登记命令，行为为空**，并且不出现在命令面板里
/// （[`action_for`] 不给它们 action，`keys::is_implemented` 据此判定）。
pub const HIERARCHY: &[(&str, &str, &str)] = &[
    (
        "references-view.removeCallItem",
        "references.command.removeCallItem",
        "从调用层级中移除",
    ),
    (
        "references-view.removeTypeItem",
        "references.command.removeTypeItem",
        "从类型层级中移除",
    ),
    (
        "references-view.showIncomingCalls",
        "references.command.showIncomingCalls",
        "显示传入调用",
    ),
    (
        "references-view.showOutgoingCalls",
        "references.command.showOutgoingCalls",
        "显示传出调用",
    ),
    (
        "references-view.showSubtypes",
        "references.command.showSubtypes",
        "显示子类型",
    ),
    (
        "references-view.showSupertypes",
        "references.command.showSupertypes",
        "显示超类型",
    ),
];

/// 本视图实现的命令：(命令 ID, 标题字符串 key, 中文标题)。标题取 VS Code 中文语言包的措辞。
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (VIEW, "references.command.showView", "显示引用"),
    (
        FIND_REFERENCES,
        "references.command.findReferences",
        "在引用视图中查找所有引用",
    ),
    (
        FIND_IMPLEMENTATIONS,
        "references.command.findImplementations",
        "在引用视图中查找所有实现",
    ),
    (NEXT, "references.command.next", "转到下一个引用"),
    (PREV, "references.command.prev", "转到上一个引用"),
    (CLEAR, "references.command.clear", "清除"),
    (CLEAR_HISTORY, "references.command.clearHistory", "清除历史"),
    (COPY, "references.command.copy", "复制"),
    (COPY_ALL, "references.command.copyAll", "全部复制"),
    (COPY_PATH, "references.command.copyPath", "复制路径"),
    (
        PICK_FROM_HISTORY,
        "references.command.pickFromHistory",
        "从历史记录中选取",
    ),
    (REFIND, "references.command.refind", "重新运行"),
    (REFRESH, "references.command.refresh", "刷新"),
    (
        REMOVE_REFERENCE_ITEM,
        "references.command.removeReferenceItem",
        "消除",
    ),
    (TREE_FOCUS, "references.command.treeFocus", "聚焦引用视图"),
    (
        TREE_RESET_VIEW_LOCATION,
        "references.command.treeResetViewLocation",
        "重置引用视图位置",
    ),
];

/// 命令分类（命令面板里显示为“编辑器: …”，与代码视图、速览共用）。
pub const CATEGORY_KEY: &str = "codeView.category";

/// 本模块发布的 VS Code 上下文键（`KEYBINDINGS.md` 导航一节的 when 子句逐条照抄）。
pub mod context {
    /// 引用视图里有结果。`f4` / `shift+f4` 的 when 的前半段。
    pub const HAS_RESULT: &str = "reference-list.hasResult";
    /// 当前结果可以用 `f4` / `shift+f4` 逐条导航（历史列表与空结果为假）。
    pub const CAN_NAVIGATE: &str = "references-view.canNavigate";
}

pub const STRINGS: &[(&str, &str)] = &[
    ("references.view.title", "引用"),
    ("references.title.references", "{symbol} — {count} 个引用"),
    (
        "references.title.implementations",
        "{symbol} — {count} 个实现",
    ),
    ("references.marker.approximate", "近似"),
    ("references.empty", "未找到“{symbol}”的引用"),
    (
        "references.empty.approximate",
        "未找到“{symbol}”的引用（语义索引构建中）",
    ),
    ("references.empty.implementations", "未找到“{symbol}”的实现"),
    (
        "references.empty.implementations.approximate",
        "未找到“{symbol}”的实现（语义索引构建中）",
    ),
    ("references.noResult", "还没有查找过引用"),
    ("references.loading", "正在查找…"),
    ("references.fileCount", "{count}"),
    ("references.lineNumber", "{line}"),
    ("references.history.title", "历史记录"),
    ("references.history.empty", "历史记录是空的"),
    ("references.copy.item", "{path}:{line}:{column}"),
];

pub fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, title, _) in COMMANDS.iter().chain(HIERARCHY) {
        match registry.register(
            *id,
            Command {
                title_key: (*title).into(),
                category_key: Some(CATEGORY_KEY.into()),
            },
        ) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn register_strings(table: &mut StringTable) -> Result<()> {
    let commands = COMMANDS.iter().chain(HIERARCHY).map(|(_, k, t)| (*k, *t));
    for (key, text) in STRINGS.iter().copied().chain(commands) {
        match table.register(key, Arc::from(text)) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 面板里的「引用」视图（`UX_SPEC` §1.4：与「问题」并列）。
pub fn register_contributions(r: &mut ContributionRegistry) -> Result<()> {
    r.register(
        VIEW,
        Contribution {
            title: Message::key("references.view.title"),
            placement: Placement::Panel,
            // 「问题」是 0、索引详情是 30；引用视图排在它们后面。
            order: 40,
            icon: Some("references".into()),
            command: Some(VIEW.into()),
            when: None,
            factory_key: VIEW.into(),
        },
    )
}

/// 本模块命令的 GPUI action；由工作台的引用视图宿主处理，键位服务按 when 解析后分发。
/// **层级相关的 6 条（[`HIERARCHY`]）不给 action**：它们只登记，不出现在命令面板里。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = references, no_json)]
pub struct ReferencesAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(ReferencesAction(cmd)) as Box<dyn Action>)
}

/// `references.preferredLocation`：`Shift+F12`（`editor.action.goToReferences`）把结果放在哪里。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferredLocation {
    /// 默认值：速览窗口（#41，决策 D3 的默认展示方式）。
    Peek,
    /// 侧边「引用」视图（D3 的备选形态）。
    View,
}

/// **唯一的一处判定**：`Shift+F12` 用速览还是侧边视图。#41 调用它，不要在两边各写一份
/// （issue #60 目标第 3 节）。无效值与缺省都回到 VS Code 的默认值 `peek`
/// （`FEATURES.md` §13 规则 5：没有用户设置时不改变默认值）。
pub fn preferred_location(settings: &dyn Settings) -> PreferredLocation {
    match Key::<String>::new("references.preferredLocation")
        .read(settings, &Scope::default())
        .as_deref()
    {
        Ok("view") => PreferredLocation::View,
        _ => PreferredLocation::Peek,
    }
}

/// [`preferred_location`] 的布尔形式：为真时 `editor.action.goToReferences` 在侧边视图中展示。
pub fn opens_in_view(settings: &dyn Settings) -> bool {
    preferred_location(settings) == PreferredLocation::View
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::{Cancellation, Events, NoEvents, Task, Value};
    use rove_config::{DefaultSettings, SettingRegistry, SettingsChange};

    struct Over(DefaultSettings, Option<&'static str>);
    impl Settings for Over {
        fn read(&self, key: &str, scope: &Scope) -> rove_base::Result<Value> {
            match (key, self.1) {
                ("references.preferredLocation", Some(v)) => Ok(Value::String(Arc::from(v))),
                _ => self.0.read(key, scope),
            }
        }
        fn write(&self, _: Arc<str>, _: Scope, _: Value, c: Cancellation) -> Task<()> {
            self.0.write("x".into(), Scope::default(), Value::Null, c)
        }
        fn changes(&self) -> &dyn Events<SettingsChange> {
            &NoEvents
        }
    }

    fn settings(value: Option<&'static str>) -> Over {
        let mut registry = SettingRegistry::default();
        rove_config::register_builtin(&mut registry).unwrap();
        Over(DefaultSettings(registry), value)
    }

    #[test]
    fn registers_every_command_string_and_the_panel_view() {
        let mut r = CommandRegistry::default();
        register_commands(&mut r).unwrap();
        // 重复登记不报错（共享热点：其他模块可能先登记了同一个 VS Code 命令）。
        register_commands(&mut r).unwrap();
        let mut t = StringTable::default();
        register_strings(&mut t).unwrap();
        for (id, key, _) in COMMANDS {
            assert!(r.get(id).is_some(), "{id}");
            assert!(t.get(key).is_some(), "{key}");
            assert!(action_for(id).is_some(), "{id}");
        }
        for (key, _) in STRINGS {
            assert!(t.get(key).is_some(), "{key}");
        }
        let mut c = ContributionRegistry::default();
        register_contributions(&mut c).unwrap();
        let view = c.get(VIEW).unwrap();
        assert!(matches!(view.placement, Placement::Panel));
        assert_eq!(view.command.as_deref(), Some(VIEW));
    }

    /// 层级相关的 6 条只登记：有命令、有标题，但没有 action（因此不出现在命令面板里）。
    #[test]
    fn the_six_hierarchy_commands_are_registered_but_not_implemented() {
        assert_eq!(HIERARCHY.len(), 6);
        let mut r = CommandRegistry::default();
        register_commands(&mut r).unwrap();
        let mut t = StringTable::default();
        register_strings(&mut t).unwrap();
        for (id, key, _) in HIERARCHY {
            assert!(r.get(id).is_some(), "{id}");
            assert!(t.get(key).is_some(), "{key}");
            assert!(action_for(id).is_none(), "{id} 不应出现在命令面板里");
        }
    }

    /// `KEYBINDINGS.md` 导航一节的三条：命令与 when 条件逐条照抄生成的默认键位表。
    #[test]
    fn the_navigation_keybindings_match_the_table_verbatim() {
        let defaults = rove_commands::keybindings::default_rules();
        let find = |id: &str| {
            defaults
                .iter()
                .find(|d| &*d.rule.command == id)
                .unwrap_or_else(|| panic!("{id} 应在生成的 VS Code 默认键位表里"))
        };
        let f = find(FIND_REFERENCES);
        assert_eq!(f.rule.key.as_deref(), Some("shift+alt+f12"));
        assert_eq!(f.rule.when.as_deref(), Some("editorHasReferenceProvider"));
        let n = find(NEXT);
        assert_eq!(n.rule.key.as_deref(), Some("f4"));
        assert_eq!(
            n.rule.when.as_deref(),
            Some("reference-list.hasResult && references-view.canNavigate")
        );
        let p = find(PREV);
        assert_eq!(p.rule.key.as_deref(), Some("shift+f4"));
        assert_eq!(p.rule.when.as_deref(), n.rule.when.as_deref());
        // 速览的 `f4` / `shift+f4` 条目一条不改：两条 when 同时存在、互不干扰。
        let peek_next = defaults
            .iter()
            .find(|d| &*d.rule.command == crate::peek::registry::NEXT)
            .unwrap();
        assert_eq!(
            peek_next.rule.when.as_deref(),
            Some("inReferenceSearchEditor || referenceSearchVisible")
        );
        // 视图本身没有默认键位。
        assert!(!defaults.iter().any(|d| &*d.rule.command == VIEW));
    }

    #[test]
    fn preferred_location_defaults_to_peek_and_honours_view() {
        assert_eq!(preferred_location(&settings(None)), PreferredLocation::Peek);
        assert!(!opens_in_view(&settings(None)));
        assert_eq!(
            preferred_location(&settings(Some("view"))),
            PreferredLocation::View
        );
        assert!(opens_in_view(&settings(Some("view"))));
        assert_eq!(
            preferred_location(&settings(Some("peek"))),
            PreferredLocation::Peek
        );
        // 无效值回到默认值。
        assert_eq!(
            preferred_location(&settings(Some("sidebar"))),
            PreferredLocation::Peek
        );
    }
}
