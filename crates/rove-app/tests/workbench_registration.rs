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

#[test]
fn status_items_register_the_file_state_entries_on_the_right() {
    // 例：#27 的六个文件状态条目全部经登记机制加入，顺序与 VS Code 一致（数值越大越靠左），
    // 通知铃铛（i32::MIN）仍在最右。
    let mut views = default_contributions();
    rove_ui::status_items::registry::register_contributions(&mut views).unwrap();
    let mut bar = StatusBar::default();
    for (id, ..) in rove_ui::status_items::registry::ITEMS {
        bar.set(
            &views,
            id,
            StatusEntry {
                text: id.into(),
                kind: StatusKind::Standard,
                ..Default::default()
            },
        )
        .unwrap();
    }
    bar.set(
        &views,
        catalog::STATUS_NOTIFICATIONS,
        StatusEntry {
            text: "$(bell)".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let right: Vec<_> = bar
        .items(&views, Alignment::Right)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    assert_eq!(
        right,
        [
            "rove.status.cursor",
            "rove.status.indentation",
            "rove.status.encoding",
            "rove.status.eol",
            "rove.status.language",
            "rove.status.readOnly",
            catalog::STATUS_NOTIFICATIONS,
        ]
    );
    // 每个条目都用工作台的通用工厂绘制，点击命令来自登记表。
    for (id, _, _, command) in rove_ui::status_items::registry::ITEMS {
        let c = views.get(id).unwrap();
        assert_eq!(c.factory_key.as_ref(), catalog::STATUS_FACTORY, "{id}");
        assert_eq!(c.command.as_deref(), command, "{id}");
    }
}

#[test]
fn about_dialog_registers_its_command_and_a_gear_menu_item() {
    // 例：#76 的「关于 Rove」。命令 ID 沿用 VS Code 的 `workbench.action.showAboutDialog`，
    // 菜单项登记在齿轮菜单里，界面字符串与颜色键也全部走登记机制（不写死）。
    use rove_ui::about::registry as about;
    let mut commands = CommandRegistry::default();
    catalog::register_commands(&mut commands).unwrap();
    about::register_commands(&mut commands).unwrap();
    let command = commands.get(about::SHOW_ABOUT).expect("命令已登记");

    let mut strings = rove_i18n::StringTable::default();
    about::register_strings(&mut strings).unwrap();
    // 命令标题与对话框里的每一条文案都能查到。
    rove_i18n::Strings::text(&strings, &Message::key(command.title_key.clone())).unwrap();
    for (key, _) in about::STRINGS {
        // 带 {version} 占位符的几条要给参数才能渲染，这里只确认都登记过。
        assert!(strings.get(key).is_some(), "{key} 没有登记");
    }

    let mut menus = default_menus();
    about::register_menus(&mut menus).unwrap();
    let implemented = |id: &str| catalog::is_implemented(id) || id == about::SHOW_ABOUT;
    let items = visible_items(&menus, MenuId::Manage, &commands, &implemented);
    assert!(
        items
            .iter()
            .any(|(_, item)| item.command.as_ref() == about::SHOW_ABOUT),
        "齿轮菜单里要有「关于」"
    );

    // 颜色键只用已有的主题键，不新造：每个键都有默认值。
    let mut colors = rove_theme::ColorRegistry::default();
    about::register_colors(&mut colors).unwrap();
    for key in about::COLORS {
        assert!(colors.get(key).is_some(), "{key}");
    }
}

#[test]
fn auto_update_registers_its_commands_a_menu_item_and_a_setting() {
    // 例：#149 自动更新。命令、界面字符串与齿轮菜单项全部走登记机制；
    // `rove.update.mode` 走 #13 的内置设置目录；**不登记任何状态栏条目或视图**
    //（issue 原文：状态栏不加常驻条目，只用提示条与通知）。
    use rove_ui::update::registry as update;
    let mut commands = CommandRegistry::default();
    catalog::register_commands(&mut commands).unwrap();
    update::register_commands(&mut commands).unwrap();
    for (id, _, _) in update::COMMANDS {
        assert!(commands.get(id).is_some(), "{id} 没有登记");
    }

    let mut strings = rove_i18n::StringTable::default();
    update::register_strings(&mut strings).unwrap();
    for (key, _) in update::STRINGS {
        assert!(strings.get(key).is_some(), "{key} 没有登记");
    }
    // 「关于」对话框要显示的每一种更新状态都查得到文案。
    for state in [
        rove_update::UpdateState::Disabled,
        rove_update::UpdateState::Checking,
        rove_update::UpdateState::UpToDate {
            checked_at: "2026-11-14 10:00:00Z".into(),
        },
        rove_update::UpdateState::Downloading {
            version: "0.1.1".into(),
        },
        rove_update::UpdateState::ReadyToInstall {
            version: "0.1.1".into(),
        },
        rove_update::UpdateState::Failed {
            at: "2026-11-14 10:00:00Z".into(),
            reason: "网络不可用".into(),
        },
    ] {
        assert!(strings.get(state.string_key()).is_some(), "{state:?}");
    }

    // 齿轮菜单（帮助组）里有「检查更新」。
    let mut menus = default_menus();
    update::register_menus(&mut menus).unwrap();
    let implemented = |id: &str| catalog::is_implemented(id) || id == update::CHECK_FOR_UPDATES;
    let items = visible_items(&menus, MenuId::Manage, &commands, &implemented);
    assert!(items
        .iter()
        .any(|(_, item)| item.command.as_ref() == update::CHECK_FOR_UPDATES));

    // `rove.update.mode` 在内置设置目录里，默认 `default`，四个取值都认得。
    let settings = rove_config::catalog::builtin_registry();
    let setting = settings
        .get(rove_update::mode::SETTING_KEY)
        .expect("rove.update.mode 已登记");
    let default = setting.rove_default.as_ref().unwrap_or(&setting.default);
    let rove_base::Value::String(default) = default else {
        panic!("默认值应该是字符串，实际 {default:?}");
    };
    assert_eq!(default.as_ref(), "default");
    for mode in rove_update::UpdateMode::ALL {
        assert!(rove_update::UpdateMode::parse(mode.as_str()).is_some());
    }

    // 状态栏与视图：一条都没有。
    let views = rove_ui::shell::view::default_contributions();
    assert!(
        !views.entries().any(|(id, _)| id.starts_with("update.")),
        "更新不应该登记视图或状态栏条目"
    );
}
