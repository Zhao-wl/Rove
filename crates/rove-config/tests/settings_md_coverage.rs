//! 对照 `docs/compat/SETTINGS.md`：支持的设置项全部登记、不支持的一个也不登记，
//! Rove 默认值差异、默认语言覆盖与文档一致。文档或 `rules.py` 变更后忘了重新生成
//! `data/settings-catalog.json` 时，这里会失败。

use rove_base::Value;
use rove_config::{
    catalog::{catalog, Priority, Source},
    jsonc, register_builtin, SettingRegistry,
};
use std::collections::{BTreeMap, BTreeSet};

fn settings_md() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/compat/SETTINGS.md");
    std::fs::read_to_string(path).expect("docs/compat/SETTINGS.md")
}

struct Doc {
    /// 第 3 节（隐藏设置）、第 4 节、第 5 节表格里的 `键 → 优先级`。
    supported: BTreeMap<String, Priority>,
    /// 第 3 节差异表：`键 → (VS Code 默认值, Rove 默认值)`。
    differences: BTreeMap<String, (String, String)>,
    /// 第 6 节“不支持”的键。
    unsupported: BTreeSet<String>,
    /// 第 2 节统计中“支持 P0”的条数。
    stated_p0: usize,
}

fn code_cells(line: &str) -> Vec<String> {
    line.split('|')
        .map(str::trim)
        .filter_map(|c| c.strip_prefix('`')?.strip_suffix('`'))
        .map(|c| c.replace("\\|", "|"))
        .collect()
}

fn parse_doc() -> Doc {
    let text = settings_md();
    let mut section = 0;
    let mut doc = Doc {
        supported: BTreeMap::new(),
        differences: BTreeMap::new(),
        unsupported: BTreeSet::new(),
        stated_p0: 0,
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            section = rest.split('.').next().unwrap().parse().unwrap_or(section);
            continue;
        }
        if section == 2 && line.starts_with("| 支持 P0 |") {
            doc.stated_p0 = line.split('|').nth(2).unwrap().trim().parse().unwrap();
        }
        let priority = match line.split('|').nth(1).map(str::trim) {
            Some("P0") => Some(Priority::P0),
            Some("P1") => Some(Priority::P1),
            Some("P2") => Some(Priority::P2),
            _ => None,
        };
        match (section, priority) {
            (3..=5, Some(p)) => {
                let key = code_cells(line).remove(0);
                assert!(
                    doc.supported.insert(key.clone(), p).is_none(),
                    "{key} listed twice"
                );
            }
            (3, None) if line.starts_with("| `") => {
                let cells = code_cells(line);
                doc.differences
                    .insert(cells[0].clone(), (cells[1].clone(), cells[2].clone()));
            }
            (6, _) if line.starts_with('`') => {
                doc.unsupported
                    .extend(line.split(", ").map(|k| k.trim_matches('`').to_string()));
            }
            _ => {}
        }
    }
    doc
}

fn raw_catalog() -> BTreeMap<String, Value> {
    let text = include_str!("../data/settings-catalog.json");
    match jsonc::parse_value(text).unwrap().unwrap() {
        Value::Object(map) => (*map).clone(),
        _ => unreachable!(),
    }
}

#[test]
fn every_supported_setting_in_settings_md_is_registered() {
    let doc = parse_doc();
    let mut registry = SettingRegistry::default();
    register_builtin(&mut registry).unwrap();
    let catalog = catalog();

    let mut p0 = 0;
    for (key, priority) in &doc.supported {
        if key.starts_with('[') {
            continue; // 默认语言覆盖，见下一个测试
        }
        let entry = catalog.get(key).unwrap_or_else(|| panic!("{key} 未登记"));
        assert_eq!(
            entry.priority, *priority,
            "{key} 的优先级与 SETTINGS.md 不一致"
        );
        assert!(registry.get(key).is_some(), "{key}");
        if *priority == Priority::P0 && entry.source == Source::VsCode {
            p0 += 1;
        }
    }
    let lang_blocks = doc
        .supported
        .iter()
        .filter(|(k, p)| k.starts_with('[') && **p == Priority::P0)
        .count();
    assert_eq!(p0 + lang_blocks, doc.stated_p0, "第 2 节的“支持 P0”条数");

    let doc_keys: BTreeSet<&str> = doc
        .supported
        .keys()
        .map(String::as_str)
        .filter(|k| !k.starts_with('['))
        .collect();
    let catalog_keys: BTreeSet<&str> = catalog.entries().map(|e| &*e.key).collect();
    assert_eq!(catalog_keys, doc_keys, "目录里多出或缺少设置项");
}

#[test]
fn unsupported_settings_are_never_registered() {
    let doc = parse_doc();
    assert!(doc.unsupported.len() > 1000, "解析 §6 失败");
    assert!(doc.unsupported.contains("search.useIgnoreFiles"));
    let registry = rove_config::builtin_registry();
    for key in &doc.unsupported {
        assert!(
            registry.get(key).is_none(),
            "{key} 在 SETTINGS.md 中是“不支持”，不能登记"
        );
    }
}

#[test]
fn rove_default_differences_match_section_3() {
    let doc = parse_doc();
    assert_eq!(doc.differences.len(), 4);
    for (key, (vscode, rove)) in &doc.differences {
        let setting = &catalog().get(key).unwrap().setting;
        let parse = |s: &str| jsonc::parse_value(s).unwrap().unwrap();
        assert_eq!(setting.default, parse(vscode), "{key}");
        assert_eq!(setting.rove_default.as_ref(), Some(&parse(rove)), "{key}");
    }
    let with_rove_default = catalog()
        .entries()
        .filter(|e| e.setting.rove_default.is_some())
        .count();
    assert_eq!(with_rove_default, 4, "§3 之外不允许有 Rove 默认值差异");
}

#[test]
fn default_language_overrides_match_section_5() {
    let doc = parse_doc();
    let Some(Value::Object(blocks)) = raw_catalog().remove("languageDefaults") else {
        panic!("languageDefaults");
    };
    let doc_blocks: BTreeSet<&str> = doc
        .supported
        .keys()
        .map(String::as_str)
        .filter(|k| k.starts_with('['))
        .collect();
    let data_blocks: BTreeSet<&str> = blocks.keys().map(String::as_str).collect();
    assert_eq!(doc_blocks.len(), 32);
    assert_eq!(data_blocks, doc_blocks);
    for block in blocks.values() {
        let Value::Object(block) = block else {
            panic!()
        };
        for key in block.keys() {
            let entry = catalog().get(key).unwrap_or_else(|| panic!("{key}"));
            assert!(entry.setting.language_overridable, "{key}");
        }
    }
}
