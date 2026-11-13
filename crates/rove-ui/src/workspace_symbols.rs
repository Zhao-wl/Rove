//! 转到工作区中的符号（#60）：`Ctrl+T`（`workbench.action.showAllSymbols`）与快速打开的 `#` 前缀。
//!
//! **两个入口是同一份数据、同一套排序、同一个实现**：`Ctrl+T` 只是把快速打开的输入预置成 `#`
//! （VS Code 里它们本来就是同一个 quick access provider），所以不存在“两个入口排序不一样”的可能。
//!
//! - 数据与排序来自 #1 契约的 [`CsQueries::workspace_symbols`]（#38 的路由把它接到 #37 的 L1 快速
//!   声明索引上，L1 已经复用 #24 移植的 VS Code 模糊匹配算法）。**这里不再写一套打分规则**：
//!   [`picks`] 只按查询算高亮区间，顺序一律保持查询返回的顺序。
//! - 近似标记按 `UX_SPEC` §2.2：结果不精确时在**列表顶部**加一行说明，不在每条结果上重复标注；
//!   精确结果不加任何标记。
//! - 列表项的渲染、键盘导航与浮层来自 #18 的快速输入组件；选中后经 #17 的打开文档服务打开并定位
//!   （`.md` 也打开源码，`UX_SPEC` §4.3）。
//!
//! 列表项目前没有 `symbolIcon.*` 种类图标：#1 契约的 `WorkspaceSymbol` 只有名字、容器与位置，
//! 不带符号种类，而契约改动要单独提 PR（AGENTS §5.2 第 2 条）。已开跟进 issue #178。
//!
//! 本模块不依赖 GPUI（只有命令的 action 是 GPUI 类型）。

use crate::{
    list_widget::{matcher::Matcher, quick_input::RowDecor},
    lists::{ItemId, Row},
    quick_access::Picks,
};
use gpui::Action;
use rove_base::{Error, Resource, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use rove_lang_cs::query::{Precision, WorkspaceSymbol};
use std::sync::Arc;

/// `Ctrl+T`（默认键位来自 #14 生成的 VS Code 默认键位表，这里不重复登记）。
pub const SHOW_ALL_SYMBOLS: &str = "workbench.action.showAllSymbols";

/// 快速打开里工作区符号的前缀（与 [`crate::quick_access::parse`] 的 `#` 一致）。
pub const PREFIX: &str = "#";

/// 一次按键最多取多少条候选（与 #38 路由的上限一致；再多列表也看不过来）。
pub const LIMIT: usize = 256;

/// (命令 ID, 标题字符串 key, 中文标题)。标题取 VS Code 中文语言包的措辞。
pub const COMMANDS: &[(&str, &str, &str)] = &[(
    SHOW_ALL_SYMBOLS,
    "workspaceSymbols.command.showAllSymbols",
    "转到工作区中的符号...",
)];

pub const STRINGS: &[(&str, &str)] = &[
    // UX_SPEC §2.2：近似结果永远要标明，但标注要克制——一行，且只在列表顶部。
    ("workspaceSymbols.approximate", "近似结果：语义索引构建中"),
    // UX_SPEC §6.3：先说发生了什么，不加“错误：”前缀。工作区里没有 C# 代码时走的也是这一条。
    ("workspaceSymbols.noResults", "未找到匹配的符号"),
    ("workspaceSymbols.detail", "{container} · {path}"),
    ("workspaceSymbols.detail.path", "{path}"),
];

pub fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, title, _) in COMMANDS {
        match registry.register(
            *id,
            Command {
                title_key: (*title).into(),
                category_key: None,
            },
        ) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn register_strings(table: &mut StringTable) -> Result<()> {
    let commands = COMMANDS.iter().map(|(_, k, t)| (*k, *t));
    for (key, text) in STRINGS.iter().copied().chain(commands) {
        match table.register(key, Arc::from(text)) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 本模块命令的 GPUI action；由窗口（宿主）处理，键位服务按 when 解析后分发。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = workspace_symbols, no_json)]
pub struct WorkspaceSymbolsAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(WorkspaceSymbolsAction(cmd)) as Box<dyn Action>)
}

/// 列表里要显示的文案（由宿主从字符串表取好后传进来，这里不碰 i18n）。
pub struct SymbolLabels {
    /// 近似时列表顶部的那一行。精确结果传 `None`，列表上不出现任何标记。
    pub approximate: Option<Arc<str>>,
}

/// 结果所在文件的工作区相对路径（外部资源给出它的显示文本）。
pub fn path_of(resource: &Resource) -> String {
    match resource {
        Resource::Workspace(p) => p.as_str().to_owned(),
        Resource::File(p) => p.to_string_lossy().replace('\\', "/"),
        Resource::Virtual(v) => v.to_string(),
    }
}

/// 查询结果 → 快速输入的候选列表。
///
/// **顺序一律是 `symbols` 的顺序**（#37 的打分与排序），`matcher` 只用来算符号名上的高亮区间。
/// `details` 与 `symbols` 一一对应，由调用方按已登记的模板格式化好（容器名 + 文件相对路径）。
pub fn picks(
    symbols: &[WorkspaceSymbol],
    details: &[Arc<str>],
    query: &str,
    matcher: &dyn Matcher,
    labels: &SymbolLabels,
) -> Picks {
    let mut picks = Picks::default();
    if symbols.is_empty() {
        return picks;
    }
    if let Some(text) = &labels.approximate {
        picks.separator(text.clone());
    }
    for (i, symbol) in symbols.iter().enumerate() {
        let highlights = matcher
            .score(query, &symbol.name)
            .map(|m| m.ranges)
            .unwrap_or_default();
        picks.push(
            Row {
                id: ItemId(i.to_string().into()),
                label: symbol.name.clone(),
                detail: details.get(i).cloned(),
                depth: 0,
                expandable: false,
                expanded: false,
                disabled: false,
            },
            RowDecor {
                label_highlights: highlights,
                detail_highlights: Vec::new(),
                trailing: None,
            },
        );
    }
    picks
}

/// 结果是否要标为近似（`UX_SPEC` §2.2）。L1 的声明索引是语法级结果，所以 `Syntax` 与
/// `Approximate` 都要标；只有 `Exact` 不加任何标记。
pub fn is_approximate(precision: Precision) -> bool {
    precision != Precision::Exact
}

/// 行 ID → 结果下标（[`picks`] 用下标做 ID）。
pub fn index_of(id: &str) -> Option<usize> {
    id.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick_access::matcher::FuzzyMatcher;
    use rove_base::{Line, Position, Span, Utf8Column, WorkspacePath};
    use rove_lang_cs::query::Target;

    fn symbol(name: &str, container: &str, path: &str) -> WorkspaceSymbol {
        let p = Position {
            line: Line(1),
            column: Utf8Column(4),
        };
        let span = Span::new(p, p).unwrap();
        WorkspaceSymbol {
            name: name.into(),
            container: container.into(),
            target: Target {
                resource: Resource::Workspace(WorkspacePath::new(path).unwrap()),
                range: span.clone(),
                selection: span,
            },
        }
    }

    fn details(symbols: &[WorkspaceSymbol]) -> Vec<Arc<str>> {
        symbols
            .iter()
            .map(|s| Arc::from(format!("{} · {}", s.container, path_of(&s.target.resource))))
            .collect()
    }

    #[test]
    fn registers_its_command_and_strings() {
        let mut r = CommandRegistry::default();
        register_commands(&mut r).unwrap();
        register_commands(&mut r).unwrap();
        let mut t = StringTable::default();
        register_strings(&mut t).unwrap();
        for (id, key, _) in COMMANDS {
            assert!(r.get(id).is_some());
            assert!(t.get(key).is_some(), "{key}");
            assert!(action_for(id).is_some());
        }
        for (key, _) in STRINGS {
            assert!(t.get(key).is_some(), "{key}");
        }
        assert!(action_for("workbench.action.quickOpen").is_none());
    }

    /// `ctrl+t` 的默认键位在 #14 生成的表里，指向这个命令。
    #[test]
    fn ctrl_t_reaches_the_registered_command() {
        let defaults = rove_commands::keybindings::default_rules();
        assert!(defaults
            .iter()
            .any(|d| &*d.rule.command == SHOW_ALL_SYMBOLS));
    }

    #[test]
    fn order_is_the_query_order_and_only_highlights_come_from_the_matcher() {
        let symbols = [
            symbol("PlayerController", "Game", "Assets/A.cs"),
            symbol("Player", "Game.Core", "Assets/B.cs"),
        ];
        let m = FuzzyMatcher::default();
        let p = picks(
            &symbols,
            &details(&symbols),
            "play",
            &m,
            &SymbolLabels { approximate: None },
        );
        assert_eq!(p.rows.len(), 2);
        // 顺序照抄查询结果：分数更高的 `Player` 不会被挪到前面。
        assert_eq!(&*p.rows[0].label, "PlayerController");
        assert_eq!(&*p.rows[1].label, "Player");
        assert_eq!(p.rows[0].detail.as_deref(), Some("Game · Assets/A.cs"));
        assert!(!p.decor[0].label_highlights.is_empty());
        assert!(p.separators.is_empty(), "精确结果不加任何标记");
        assert_eq!(index_of(&p.rows[1].id.0), Some(1));
    }

    #[test]
    fn approximate_results_get_one_restrained_line_at_the_top() {
        let symbols = [symbol("Player", "Game", "Assets/B.cs")];
        let m = FuzzyMatcher::default();
        let p = picks(
            &symbols,
            &details(&symbols),
            "pl",
            &m,
            &SymbolLabels {
                approximate: Some(Arc::from("近似结果：语义索引构建中")),
            },
        );
        assert_eq!(p.separators.len(), 1, "只有一行说明");
        assert_eq!(p.separators[0].0, 0, "在列表顶部");
        // 每条结果上没有重复标注。
        assert!(p.decor.iter().all(|d| d.trailing.is_none()));
        assert!(is_approximate(Precision::Syntax));
        assert!(is_approximate(Precision::Approximate));
        assert!(!is_approximate(Precision::Exact));
    }

    #[test]
    fn no_symbols_means_an_empty_list_without_the_marker_line() {
        let m = FuzzyMatcher::default();
        let p = picks(
            &[],
            &[],
            "x",
            &m,
            &SymbolLabels {
                approximate: Some(Arc::from("近似")),
            },
        );
        assert!(p.rows.is_empty());
        assert!(p.separators.is_empty());
    }
}
