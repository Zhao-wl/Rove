//! 大纲模型层单元测试：设置读取、种类过滤、树数据源、面包屑段、选择器排序、转到符号排序。
use super::{goto, model::*, registry};
use crate::{
    list_widget::{
        matcher::{matcher_for, MatchType},
        tree::{ChildCount, Children, TreeSource},
    },
    lists::ItemId,
};
use rove_base::{
    Cancellation, Line, Position, Resource, Utf8Column, Value, Version, WorkspacePath,
};
use rove_config::{Scope, Settings};
use rove_i18n::Strings;
use rove_syntax::symbols::{extract, DocumentSymbols, SymbolId};
use std::{collections::BTreeMap, sync::Arc};

const SOURCE: &str = "namespace Game\n{\n    public class Player\n    {\n        int hp;\n        public void Update() { }\n        public void Move(float speed) { }\n        public int Health { get; }\n    }\n    enum Mode { A, B }\n}\n";

fn symbols() -> Arc<DocumentSymbols> {
    let list = extract("csharp", SOURCE, &Cancellation::default()).unwrap();
    Arc::new(DocumentSymbols::new(
        Version(1),
        "csharp".into(),
        list.into(),
    ))
}

/// 默认值 + 用户覆盖的只读设置。
struct Overrides(rove_config::DefaultSettings, BTreeMap<&'static str, Value>);
impl Settings for Overrides {
    fn read(&self, key: &str, scope: &Scope) -> rove_base::Result<Value> {
        match self.1.get(key) {
            Some(v) => Ok(v.clone()),
            None => self.0.read(key, scope),
        }
    }
    fn write(&self, _: Arc<str>, _: Scope, _: Value, _: Cancellation) -> rove_base::Task<()> {
        Box::pin(async { Err(rove_base::Error::Unsupported) })
    }
    fn changes(&self) -> &dyn rove_base::Events<rove_config::SettingsChange> {
        &rove_base::NoEvents
    }
}

fn settings(values: &[(&'static str, Value)]) -> Overrides {
    Overrides(
        rove_config::DefaultSettings(crate::app_settings::registry().unwrap()),
        values.iter().cloned().collect(),
    )
}

fn label() -> Labeler {
    Arc::new(|s: &rove_syntax::Symbol| s.name.clone())
}

fn names(symbols: &DocumentSymbols, ids: &[SymbolId]) -> Vec<String> {
    ids.iter()
        .map(|id| symbols.node(*id).unwrap().symbol.name.to_string())
        .collect()
}

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

#[test]
fn settings_use_vscode_defaults_and_overrides() {
    let defaults = settings(&[]);
    let outline = OutlineSettings::read(&defaults);
    assert_eq!(outline.collapse, CollapseItems::AlwaysExpand);
    assert!(outline.icons);
    assert_eq!(outline.filter, SymbolFilter::default());
    let crumbs = BreadcrumbSettings::read(&defaults);
    assert!(crumbs.enabled && crumbs.icons);
    assert_eq!(crumbs.file_path, PathMode::On);
    assert_eq!(crumbs.symbol_path, PathMode::On);
    assert_eq!(crumbs.sort, SortOrder::Position);
    assert_eq!(&*crumbs.separator, ".");
    // 每个 `outline.show*` / `breadcrumbs.show*` 都是已登记的设置项。
    let registry = crate::app_settings::registry().unwrap();
    for (_, suffix) in SHOW_SETTINGS {
        for prefix in ["outline", "breadcrumbs"] {
            assert!(
                registry.get(&format!("{prefix}.show{suffix}")).is_some(),
                "{prefix}.show{suffix}"
            );
        }
    }
    let custom = settings(&[
        (
            "outline.collapseItems",
            Value::String("alwaysCollapse".into()),
        ),
        ("outline.icons", Value::Bool(false)),
        ("outline.showMethods", Value::Bool(false)),
        ("breadcrumbs.filePath", Value::String("last".into())),
        ("breadcrumbs.symbolPath", Value::String("off".into())),
        ("breadcrumbs.symbolSortOrder", Value::String("name".into())),
        (
            "breadcrumbs.symbolPathSeparator",
            Value::String("::".into()),
        ),
        ("breadcrumbs.showFields", Value::Bool(false)),
    ]);
    let outline = OutlineSettings::read(&custom);
    assert_eq!(outline.collapse, CollapseItems::AlwaysCollapse);
    assert!(!outline.icons);
    assert!(!outline.filter.shows(&rove_syntax::SymbolKind::Method));
    assert!(outline.filter.shows(&rove_syntax::SymbolKind::Field));
    let crumbs = BreadcrumbSettings::read(&custom);
    assert_eq!(crumbs.file_path, PathMode::Last);
    assert_eq!(crumbs.symbol_path, PathMode::Off);
    assert_eq!(crumbs.sort, SortOrder::Name);
    assert_eq!(&*crumbs.separator, "::");
    assert!(!crumbs.filter.shows(&rove_syntax::SymbolKind::Field));
}

#[test]
fn tree_source_hides_filtered_kinds_with_their_subtrees() {
    let s = symbols();
    let all = SymbolTree::new(s.clone(), SymbolFilter::default(), label());
    let Children::Ready(roots) = all.children(None) else {
        panic!()
    };
    assert_eq!(roots.len(), 1);
    assert_eq!(&*roots[0].label, "Game");
    assert_eq!(roots[0].children, ChildCount::Known(2));
    let Children::Ready(members) =
        all.children(Some(&ItemId("/namespace:Game/class:Player".into())))
    else {
        panic!()
    };
    let labels: Vec<_> = members.iter().map(|m| m.label.to_string()).collect();
    assert_eq!(labels, ["hp", "Update()", "Move(float)", "Health"]);
    assert!(members.iter().all(|m| m.children == ChildCount::None));
    // 全部展开时的节点：命名空间、类、枚举。
    assert_eq!(all.expandable().len(), 3);

    let no_methods = SymbolTree::new(s.clone(), SymbolFilter::default().hide("method"), label());
    assert_eq!(no_methods.visible_children(Some(SymbolId(1))).len(), 2);
    // 隐藏类：类及其成员都不显示。
    let no_classes = SymbolTree::new(s, SymbolFilter::default().hide("class"), label());
    let Children::Ready(game) = no_classes.children(Some(&ItemId("/namespace:Game".into()))) else {
        panic!()
    };
    assert_eq!(
        game.iter().map(|g| g.label.to_string()).collect::<Vec<_>>(),
        ["Mode"]
    );
}

#[test]
fn breadcrumb_segments_follow_settings() {
    let s = symbols();
    let resource = Resource::Workspace(WorkspacePath::new("Assets/Scripts/Player.cs").unwrap());
    let path = s.path_at(pos(6, 30));
    assert_eq!(names(&s, &path), ["Game", "Player", "Move(float)"]);
    let files = file_segments(&resource, PathMode::On);
    assert_eq!(
        files,
        [
            Segment::File {
                label: "Assets".into(),
                dir: "".into(),
                is_file: false
            },
            Segment::File {
                label: "Scripts".into(),
                dir: "Assets".into(),
                is_file: false
            },
            Segment::File {
                label: "Player.cs".into(),
                dir: "Assets/Scripts".into(),
                is_file: true
            },
        ]
    );
    assert_eq!(file_segments(&resource, PathMode::Last).len(), 1);
    assert!(file_segments(&resource, PathMode::Off).is_empty());
    let filter = SymbolFilter::default();
    assert_eq!(symbol_segments(&s, &filter, &path, PathMode::On).len(), 3);
    assert_eq!(
        symbol_segments(&s, &filter, &path, PathMode::Last),
        [Segment::Symbol(*path.last().unwrap())]
    );
    // 隐藏方法：路径在类处截断。
    assert_eq!(
        symbol_segments(&s, &filter.clone().hide("method"), &path, PathMode::On).len(),
        2
    );
    assert_eq!(symbol_path_text(&s, &path, "."), "Game.Player.Move(float)");
    assert_eq!(
        deepest_visible(&s, &filter.clone().hide("method"), &path),
        Some(path[1])
    );
}

#[test]
fn picker_lists_siblings_in_configured_order() {
    let s = symbols();
    let update = s.path_at(pos(5, 20))[2];
    let filter = SymbolFilter::default();
    let by_position = sibling_symbols(&s, &filter, update, SortOrder::Position);
    assert_eq!(
        names(&s, &by_position),
        ["hp", "Update()", "Move(float)", "Health"]
    );
    let by_name = sibling_symbols(&s, &filter, update, SortOrder::Name);
    assert_eq!(
        names(&s, &by_name),
        ["Health", "hp", "Move(float)", "Update()"]
    );
    let by_type = sibling_symbols(&s, &filter, update, SortOrder::Type);
    assert_eq!(
        names(&s, &by_type),
        ["hp", "Move(float)", "Update()", "Health"]
    );
}

#[test]
fn goto_symbol_order_matches_vscode_rules() {
    let s = symbols();
    let matcher = matcher_for(MatchType::Fuzzy);
    let label = |id: SymbolId| s.node(id).unwrap().symbol.name.clone();
    // 无查询：全部符号，按文档位置。
    let all = goto::symbol_picks(&s, "", matcher.as_ref(), &label);
    let labels: Vec<_> = all.items.iter().map(|p| p.label.to_string()).collect();
    assert_eq!(
        labels,
        [
            "Game",
            "Player",
            "hp",
            "Update()",
            "Move(float)",
            "Health",
            "Mode",
            "A",
            "B"
        ]
    );
    assert_eq!(all.items[3].container.as_deref(), Some("Player"));
    // 有查询：只保留匹配项，分数高者在前（前缀匹配优先）。
    let found = goto::symbol_picks(&s, "m", matcher.as_ref(), &label);
    let labels: Vec<_> = found.items.iter().map(|p| p.label.to_string()).collect();
    assert!(labels.contains(&"Move(float)".to_string()) && labels.contains(&"Mode".to_string()));
    assert!(!labels.contains(&"hp".to_string()));
    assert!(found.items.windows(2).all(|w| w[0].score >= w[1].score));
    // `:` 前缀：按种类分组。
    let grouped = goto::symbol_picks(&s, ":", matcher.as_ref(), &label);
    let kinds: Vec<_> = grouped.groups.iter().map(|(_, k, n)| (*k, *n)).collect();
    assert_eq!(
        kinds,
        [
            ("class", 1),
            ("enum", 1),
            ("enumMember", 2),
            ("field", 1),
            ("method", 2),
            ("namespace", 1),
            ("property", 1)
        ]
    );
    let list = goto::PickList::new(&found, Version(1));
    use crate::lists::ListModel;
    assert_eq!(list.count(), found.items.len());
}

#[test]
fn registrations_are_complete() {
    let mut commands = rove_commands::CommandRegistry::default();
    registry::register_commands(&mut commands).unwrap();
    // 重复登记（其他模块先登记了同一 VS Code 命令）不报错。
    registry::register_commands(&mut commands).unwrap();
    let mut strings = rove_i18n::chinese().unwrap();
    registry::register_strings(&mut strings).unwrap();
    for (id, title, _) in registry::COMMANDS {
        assert!(commands.get(id).is_some());
        assert!(strings.text(&rove_i18n::Message::key(*title)).is_ok());
        assert!(registry::action_for(id).is_some());
    }
    for (kind, _) in SHOW_SETTINGS {
        assert!(strings
            .text(&rove_i18n::Message::key(format!("symbolKind.{kind}")))
            .is_ok());
    }
    // 图标颜色键都是主题服务登记的 VS Code 键。
    let theme = rove_theme::ThemeService::new(crate::app_settings::defaults_only());
    for key in icon_color_keys().chain([
        "breadcrumb.background",
        "breadcrumb.foreground",
        "breadcrumb.focusForeground",
        "breadcrumb.activeSelectionForeground",
        "breadcrumbPicker.background",
    ]) {
        assert!(
            rove_theme::Theme::color(theme.as_ref(), key).is_ok(),
            "{key}"
        );
    }
    // 默认键位来自 VS Code 表：Ctrl+Shift+O 与面包屑键位都能解析到已登记的命令。
    let defaults = rove_commands::keybindings::default_rules();
    for id in [
        registry::GOTO_SYMBOL,
        registry::FOCUS,
        registry::FOCUS_AND_SELECT,
        registry::SELECT_EDITOR,
    ] {
        assert!(defaults.iter().any(|d| &*d.rule.command == id), "{id}");
    }
}
