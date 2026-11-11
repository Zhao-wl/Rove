//! #16 验收：视图、状态栏条目、菜单项三类登记各有一个来自其他 crate 的示例，
//! 证明后续 issue（#23/#25/#26/#27/#36/R60/R65……）只用 rove-ui 的公开接口就能登记。
//! 运行：`cargo test -p rove-app --test workbench_registration`
use rove_commands::{Command, CommandRegistry};
use rove_i18n::Message;
use rove_ui::{
    contributions::{Contribution, Placement},
    shell::{
        catalog,
        menus::{visible_items, MenuId, MenuItem},
        status::{Alignment, StatusBar, StatusEntry, StatusKind},
        view::{default_contributions, default_menus},
    },
};
use std::sync::Arc;

fn contribution(placement: Placement, order: i32, factory: &str) -> Contribution {
    Contribution {
        title: Message::key("view.problems"),
        placement,
        order,
        icon: Some("list-tree".into()),
        command: None,
        when: None,
        factory_key: factory.into(),
    }
}

#[test]
fn outline_registers_a_section_in_the_explorer() {
    // 例：#26 大纲在资源管理器里登记一个小节（顺序 20，排在“文件夹” 10 之后）。
    let mut views = default_contributions();
    views
        .register(
            "outline",
            contribution(
                Placement::Sidebar {
                    container: catalog::EXPLORER.into(),
                },
                20,
                "outline",
            ),
        )
        .unwrap();
    let mut sections: Vec<_> = views
        .entries()
        .filter(|(_, c)| {
            matches!(&c.placement, Placement::Sidebar { container } if container.as_ref() == catalog::EXPLORER)
        })
        .collect();
    sections.sort_by_key(|(_, c)| c.order);
    let ids: Vec<_> = sections.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, [catalog::FILE_VIEW, "outline"]);
    // 重复登记被拒绝，不覆盖。
    assert!(views
        .register("outline", contribution(Placement::Panel, 0, "x"))
        .is_err());
}

#[test]
fn index_status_registers_a_status_bar_entry() {
    // 例：#36 C# 索引状态登记在状态栏左侧，内容运行期更新，支持 $(icon~spin) 与颜色级别。
    let mut views = default_contributions();
    views
        .register(
            "rove.status.csharpIndex",
            contribution(Placement::StatusLeft, 40, catalog::STATUS_FACTORY),
        )
        .unwrap();
    let mut bar = StatusBar::default();
    bar.set(
        &views,
        "rove.status.csharpIndex",
        StatusEntry {
            text: "$(sync~spin) C# 索引 37%".into(),
            tooltip: Some("已处理 370 / 1000 个文件".into()),
            command: Some("rove.index.showDetails".into()),
            kind: StatusKind::Standard,
        },
    )
    .unwrap();
    bar.set(
        &views,
        catalog::STATUS_PROBLEMS,
        StatusEntry {
            text: "$(error) 0  $(warning) 0".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let left: Vec<_> = bar
        .items(&views, Alignment::Left)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    // 问题计数 priority 50 > 索引 40：问题计数更靠左。
    assert_eq!(left, [catalog::STATUS_PROBLEMS, "rove.status.csharpIndex"]);
    // 未登记的条目不能设置内容。
    assert!(bar
        .set(&views, "rove.status.unknown", StatusEntry::default())
        .is_err());
}

#[test]
fn feature_registers_a_menu_item_that_shows_only_when_implemented() {
    // 例：#24 实现快速打开后，在“转到”菜单登记一项。
    let mut commands = CommandRegistry::default();
    catalog::register_commands(&mut commands).unwrap();
    commands
        .register(
            "workbench.action.quickOpen",
            Command {
                title_key: "commands.quickOpen".into(),
                category_key: None,
            },
        )
        .unwrap();
    let mut menus = default_menus();
    menus
        .register(
            "menubar.go.workbench.action.quickOpen",
            MenuItem {
                menu: MenuId::Go,
                group: Arc::from("1_files"),
                order: 10,
                command: "workbench.action.quickOpen".into(),
                title: None,
            },
        )
        .unwrap();
    let before = |id: &str| catalog::is_implemented(id);
    assert!(visible_items(&menus, MenuId::Go, &commands, &before).is_empty());
    let after = |id: &str| catalog::is_implemented(id) || id == "workbench.action.quickOpen";
    let items = visible_items(&menus, MenuId::Go, &commands, &after);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].1.command.as_ref(), "workbench.action.quickOpen");
}
