//! #27 的纯数据单元测试：条目文字、悬停提示、颜色级别、缩进检测与各选择器的内容。
//!
//! 运行：`cargo test -p rove-ui status_items`。窗口一侧的入口测试在
//! `skeleton/workbench/status_tests.rs`。
use super::{model, registry};
use crate::{
    contributions::{ContributionRegistry, Placement},
    shell::{catalog::STATUS_FACTORY, status::StatusKind},
};
use rove_buffer::{Encoding, Eol, LineEndings, ReadOnlyReason, ReducedFeature, SizeClass};
use rove_i18n::StringTable;
use std::sync::Arc;

fn strings() -> StringTable {
    let mut table = rove_i18n::chinese().unwrap();
    // 只读原因的文案由 #20 登记，本模块复用它（不另造一份）。
    crate::code_view::commands::register_strings(&mut table).unwrap();
    registry::register_strings(&mut table).unwrap();
    table
}

fn cursor(line: u64, column: u64, selected: u64) -> model::CursorSummary {
    model::CursorSummary {
        line,
        column,
        selected,
    }
}

#[test]
fn six_items_register_on_the_right_in_vscode_order() {
    let mut r = ContributionRegistry::default();
    crate::shell::catalog::register_contributions(&mut r).unwrap();
    registry::register_contributions(&mut r).unwrap();
    // 重复登记被拒绝（登记表不被覆盖）。
    assert!(registry::register_contributions(&mut r).is_err());
    let mut right: Vec<_> = r
        .entries()
        .filter(|(_, c)| matches!(c.placement, Placement::StatusRight))
        .collect();
    // 数值越大越靠左；通知铃铛（i32::MIN）固定在最右。
    right.sort_by(|a, b| b.1.order.cmp(&a.1.order).then(a.0.cmp(b.0)));
    let ids: Vec<_> = right.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        ids,
        [
            registry::CURSOR,
            registry::INDENT,
            registry::ENCODING,
            registry::EOL,
            registry::LANGUAGE,
            registry::READ_ONLY,
            crate::shell::catalog::STATUS_NOTIFICATIONS,
        ]
    );
    let table = strings();
    for (id, title, _, command) in registry::ITEMS {
        let c = r.get(id).unwrap();
        assert_eq!(
            c.factory_key.as_ref(),
            STATUS_FACTORY,
            "{id} 用通用工厂绘制"
        );
        assert!(table.get(title).is_some(), "{id} 的标题来自字符串表");
        assert_eq!(c.command.as_deref(), command, "{id} 的默认点击命令");
    }
}

#[test]
fn every_registered_string_and_command_resolves() {
    let table = strings();
    for (key, _) in registry::STRINGS {
        assert!(table.get(key).is_some(), "{key}");
    }
    for (id, key, _) in registry::COMMANDS {
        assert!(table.get(key).is_some(), "{id}");
        assert!(registry::action_for(id).is_some(), "{id}");
        assert!(registry::is_status_command(id), "{id}");
    }
    // `changeEOL` 由 #49 登记，但无参形式由本模块弹选择器。
    assert!(registry::is_status_command(registry::CHANGE_EOL));
    assert!(!registry::is_status_command("workbench.action.quickOpen"));
}

#[test]
fn cursor_text_covers_caret_selection_and_multiple_selections() {
    let t = strings();
    assert_eq!(
        model::cursor_entry(&t, &[cursor(12, 34, 0)]).text,
        "行 12，列 34"
    );
    assert_eq!(
        model::cursor_entry(&t, &[cursor(12, 34, 5)]).text,
        "行 12，列 34（已选择 5）"
    );
    assert_eq!(
        model::cursor_entry(&t, &[cursor(1, 1, 0), cursor(2, 1, 0)]).text,
        "2 个选择"
    );
    assert_eq!(
        model::cursor_entry(&t, &[cursor(1, 1, 3), cursor(2, 1, 4)]).text,
        "2 个选择（已选择 7 个字符）"
    );
    let entry = model::cursor_entry(&t, &[cursor(1, 1, 0)]);
    assert_eq!(entry.command.as_deref(), Some(registry::GOTO_LINE));
    assert_eq!(entry.kind, StatusKind::Standard);
}

#[test]
fn indent_text_and_detection() {
    let t = strings();
    let spaces = model::Indentation {
        insert_spaces: true,
        size: 4,
        detected: false,
    };
    assert_eq!(model::indent_entry(&t, spaces).text, "空格: 4");
    let tabs = model::Indentation {
        insert_spaces: false,
        size: 8,
        detected: true,
    };
    let entry = model::indent_entry(&t, tabs);
    assert_eq!(entry.text, "制表符长度: 8");
    assert!(entry.tooltip.unwrap().contains("检测"));

    // 两个空格缩进的文件。
    let two = "class A {\n  void M() {\n    Call();\n  }\n}\n";
    let guessed = model::guess_indentation(two.lines(), 500, spaces);
    assert_eq!(
        guessed,
        model::Indentation {
            insert_spaces: true,
            size: 2,
            detected: true
        }
    );
    // 制表符缩进的文件保留设置里的制表符宽度。
    let tabbed = "class A {\n\tvoid M() {\n\t\tCall();\n\t}\n}\n";
    let guessed = model::guess_indentation(tabbed.lines(), 500, spaces);
    assert!(!guessed.insert_spaces && guessed.detected);
    // 完全没有缩进的文件保留设置值，不标记为“检测得出”。
    let flat = "a\nb\nc\n";
    assert_eq!(
        model::guess_indentation(flat.lines(), 500, spaces),
        model::Indentation {
            detected: false,
            ..spaces
        }
    );
}

#[test]
fn encoding_text_marks_bom_and_automatic_detection() {
    let t = strings();
    assert_eq!(
        model::encoding_entry(&t, &Encoding::Utf8, false, false).text,
        "UTF-8"
    );
    assert_eq!(
        model::encoding_entry(&t, &Encoding::Utf8, true, false).text,
        "UTF-8 with BOM"
    );
    let guessed = model::encoding_entry(&t, &Encoding::Gbk, false, true);
    assert_eq!(guessed.text, "GBK");
    assert!(guessed.tooltip.unwrap().contains("自动识别"));
    assert!(!model::encoding_entry(&t, &Encoding::Gbk, false, false)
        .tooltip
        .unwrap()
        .contains("自动识别"));
    assert_eq!(
        model::encoding_label(&Encoding::Other(Arc::from("windows-1252"))),
        "windows-1252"
    );
}

#[test]
fn eol_text_shows_mixed_and_explains_it() {
    let t = strings();
    assert_eq!(
        model::eol_entry(&t, &LineEndings::Uniform(Eol::Lf)).text,
        "LF"
    );
    assert_eq!(
        model::eol_entry(&t, &LineEndings::Uniform(Eol::CrLf)).text,
        "CRLF"
    );
    let mixed = model::eol_entry(&t, &LineEndings::Mixed { preferred: Eol::Lf });
    assert_eq!(mixed.text, "混合");
    assert!(mixed.tooltip.unwrap().contains("保持原样"));
}

#[test]
fn language_label_falls_back_to_the_id() {
    let t = strings();
    assert_eq!(model::language_entry(&t, "csharp").text, "C#");
    assert_eq!(model::language_entry(&t, "plaintext").text, "纯文本");
    assert_eq!(model::language_entry(&t, "rust").text, "rust");
}

#[test]
fn read_only_entry_lists_reasons_and_disabled_features() {
    let t = strings();
    assert!(model::read_only_entry(&t, SizeClass::Normal, &[], &[]).is_none());
    let rule = ReadOnlyReason::Rule(Arc::from(rove_workspace::REASON_READONLY_INCLUDE));
    let entry = model::read_only_entry(&t, SizeClass::Normal, &[rule], &[]).unwrap();
    assert_eq!(entry.text, "只读");
    assert_eq!(entry.kind, StatusKind::Warning);
    assert!(entry.command.is_none(), "纯提示，没有点击行为");
    assert!(entry.tooltip.unwrap().contains("files.readonlyInclude"));

    let huge = model::read_only_entry(
        &t,
        SizeClass::Huge,
        &[ReadOnlyReason::HugeFile],
        rove_buffer::disabled_features(SizeClass::Huge),
    )
    .unwrap();
    assert_eq!(huge.text, "超大文件 · 只读");
    let tooltip = huge.tooltip.unwrap();
    assert!(
        tooltip.contains("超大文件只能以只读方式打开。"),
        "{tooltip}"
    );
    assert!(tooltip.contains("语法高亮"), "{tooltip}");
    assert!(tooltip.contains("软换行"), "{tooltip}");
    assert_eq!(
        model::feature_key(ReducedFeature::Minimap),
        "status.feature.minimap"
    );
}

#[test]
fn pickers_hide_items_that_would_rewrite_a_read_only_file() {
    let t = strings();
    let view_only: Vec<String> = model::indent_picks(&t, false)
        .iter()
        .map(|p| p.id.to_string())
        .collect();
    assert_eq!(
        view_only,
        [
            registry::INDENT_USING_SPACES,
            registry::INDENT_USING_TABS,
            registry::DETECT_INDENTATION
        ]
    );
    assert_eq!(model::indent_picks(&t, true).len(), 5);
    assert!(
        model::eol_picks(&t, false).is_empty(),
        "只读文件不提供统一行尾"
    );
    assert_eq!(model::eol_picks(&t, true).len(), 2);
    assert_eq!(model::indent_size_picks(&t).len(), 8);
    // M1 只有“通过编码重新打开”。
    let actions = model::encoding_actions(&t, false);
    assert_eq!(actions.len(), 1);
    assert_eq!(&*actions[0].id, model::ENCODING_REOPEN);
    assert_eq!(model::encoding_actions(&t, true).len(), 2);
}

#[test]
fn encoding_picks_group_and_deduplicate() {
    let t = strings();
    let picks = model::encoding_picks(&t, "gbk", &["gbk".into(), "big5".into(), "nope".into()]);
    let ids: Vec<&str> = picks.iter().map(|p| &*p.id).collect();
    assert_eq!(ids[0], "gbk", "files.encoding 排在最前");
    assert_eq!(ids[1], "big5");
    assert!(!ids.contains(&"nope"), "无法识别的 ID 不出现：{ids:?}");
    assert_eq!(
        ids.iter().filter(|id| **id == "gbk").count(),
        1,
        "候选里重复的编码只出现一次"
    );
    assert_eq!(picks[0].label.as_ref(), "GBK");
    assert_eq!(picks[0].detail.as_deref(), Some("gbk"));
    assert_eq!(picks[0].group.as_deref(), Some("已配置"));
    assert_eq!(picks[1].group.as_deref(), Some("候选编码"));
    assert!(ids.contains(&"utf8"));
}

#[test]
fn language_picks_put_the_current_language_first() {
    let t = strings();
    let languages: Vec<Arc<str>> = ["csharp", "json", "markdown", "plaintext"]
        .iter()
        .map(|id| Arc::from(*id))
        .collect();
    let picks = model::language_picks(&t, &languages, "json");
    assert_eq!(&*picks[0].id, "json");
    let rest: Vec<&str> = picks[1..].iter().map(|p| &*p.label).collect();
    let mut sorted = rest.clone();
    sorted.sort();
    assert_eq!(rest, sorted, "其余按显示名排序");
}
