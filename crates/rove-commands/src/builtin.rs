//! Rove-only commands (`docs/compat/KEYBINDINGS.md` §5) and the strings owned by the command layer.
//!
//! Registration only: each command is implemented by the issue that owns the feature (the context
//! basket, long lines, JSON tools, structure view, C# index), which calls
//! `CommandCenter::implement` from its own module. Their default keys live in the generated
//! default table (source `rove`).
use crate::{center::CommandCenter, Command};
use rove_base::{Registry, Result};
use std::sync::Arc;

/// `(command ID, title string key, Chinese title)`.
pub const ROVE_COMMANDS: &[(&str, &str, &str)] = &[
    (
        "rove.contextBasket.add",
        "command.rove.contextBasket.add",
        "加入上下文篮子",
    ),
    (
        "rove.contextBasket.focus",
        "command.rove.contextBasket.focus",
        "打开上下文篮子",
    ),
    (
        "rove.contextBasket.copyAsMarkdown",
        "command.rove.contextBasket.copyAsMarkdown",
        "上下文篮子：复制为 Markdown",
    ),
    (
        "rove.contextBasket.copyAsReferences",
        "command.rove.contextBasket.copyAsReferences",
        "上下文篮子：复制为引用",
    ),
    (
        "rove.contextBasket.clear",
        "command.rove.contextBasket.clear",
        "清空上下文篮子",
    ),
    (
        "rove.editor.followTail",
        "command.rove.editor.followTail",
        "跟随文件末尾",
    ),
    (
        "rove.editor.viewFormatted",
        "command.rove.editor.viewFormatted",
        "格式化后查看",
    ),
    (
        "rove.editor.openLongLine",
        "command.rove.editor.openLongLine",
        "查看完整的超长行",
    ),
    ("rove.json.minify", "command.rove.json.minify", "压缩 JSON"),
    (
        "rove.structure.copyPath",
        "command.rove.structure.copyPath",
        "复制结构路径",
    ),
    (
        "rove.structure.copyValue",
        "command.rove.structure.copyValue",
        "复制节点的值",
    ),
    (
        "rove.index.rebuild",
        "command.rove.index.rebuild",
        "重建 C# 语义索引",
    ),
    (
        "rove.index.showDetails",
        "command.rove.index.showDetails",
        "显示 C# 索引详情",
    ),
    (
        "rove.index.openLog",
        "command.rove.index.openLog",
        "打开索引日志",
    ),
];

/// Category string key shared by the Rove-only commands.
pub const ROVE_CATEGORY: &str = "command.category.rove";

/// Strings owned by the command layer: titles above plus chord feedback shown by the UI.
pub const STRINGS: &[(&str, &str)] = &[
    (ROVE_CATEGORY, "Rove"),
    (
        "keybindings.chordPending",
        "已按下 ({keys})，正在等待组合键的下一个键…",
    ),
    ("keybindings.chordAborted", "组合键 ({keys}) 不是命令。"),
];

/// Register every Rove-only command (not implemented: features attach handlers later).
pub fn register_rove_commands(center: &mut CommandCenter) -> Result<()> {
    for (id, title, _) in ROVE_COMMANDS {
        center.register(
            id,
            Command {
                title_key: (*title).into(),
                category_key: Some(ROVE_CATEGORY.into()),
            },
        )?;
    }
    Ok(())
}

/// Register the command layer's Chinese strings into a string table (`rove_i18n::StringTable`
/// is a `Registry<Arc<str>>`; this crate does not depend on rove-i18n).
pub fn register_strings(table: &mut Registry<Arc<str>>) -> Result<()> {
    for (key, text) in STRINGS {
        table.register(*key, Arc::from(*text))?;
    }
    for (_, key, text) in ROVE_COMMANDS {
        table.register(*key, Arc::from(*text))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{format_keybinding, parse_keybinding};
    use std::collections::{BTreeMap, BTreeSet};

    fn doc(path: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::fs::read_to_string(root.join(path)).unwrap()
    }

    /// Rows of the §5 table in KEYBINDINGS.md: (command, key).
    fn section5() -> Vec<(String, String)> {
        let md = doc("docs/compat/KEYBINDINGS.md");
        let start = md.find("## 5. Rove 专有命令").unwrap();
        let end = md[start..].find("## 6.").unwrap() + start;
        md[start..end]
            .lines()
            .filter(|l| l.starts_with("| P"))
            .map(|l| {
                let cells: Vec<&str> = l.split('|').map(str::trim).collect();
                (
                    cells[2].trim_matches('`').to_owned(),
                    cells[3].trim_matches('`').to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn all_section5_commands_are_registered_with_strings() {
        let mut c = CommandCenter::default();
        register_rove_commands(&mut c).unwrap();
        let mut strings = Registry::<Arc<str>>::default();
        register_strings(&mut strings).unwrap();
        let rows = section5();
        assert_eq!(rows.len(), ROVE_COMMANDS.len());
        for (cmd, _) in &rows {
            let registered = c
                .registry()
                .get(cmd)
                .unwrap_or_else(|| panic!("{cmd} not registered"));
            assert!(strings.get(&registered.title_key).is_some());
            assert!(
                !c.is_implemented(cmd),
                "registration does not imply implementation"
            );
        }
        // Their default keys are in the generated table.
        let defaults: BTreeMap<String, Vec<String>> = crate::keybindings::default_rules()
            .into_iter()
            .filter(|d| d.source.as_ref() == "rove")
            .fold(BTreeMap::new(), |mut m, d| {
                m.entry(d.rule.command)
                    .or_default()
                    .push(d.rule.key.unwrap());
                m
            });
        for (cmd, key) in rows.iter().filter(|(_, k)| k != "—") {
            assert_eq!(defaults.get(cmd), Some(&vec![key.clone()]), "{cmd}");
        }
    }

    /// Automatic conflict check: no Rove default key collides with any VS Code default key —
    /// neither the same sequence, nor a prefix relation in either direction (which would turn a
    /// VS Code single key into a chord or shadow a VS Code chord) — and `ctrl+alt+a` is avoided.
    #[test]
    fn rove_keys_do_not_conflict_with_vscode_defaults() {
        let text = doc("docs/compat/vscode/default-keybindings.json");
        let (vscode, _) = crate::keybindings::parse_user_keybindings(&text).unwrap();
        assert_eq!(vscode.len(), 1269, "VS Code 1.145.2 export");
        let vs: BTreeSet<Vec<String>> = vscode
            .iter()
            .map(|r| parse_keybinding(r.key.as_deref().unwrap()).expect("every VS Code key parses"))
            .map(|k| k.iter().map(crate::keys::format_stroke).collect())
            .collect();
        assert_eq!(vs.len(), 445, "distinct key sequences in the export");
        // Rules that can never be active (`"when": "false"`, e.g. VS Code's own `ctrl+k`
        // placeholder) cannot conflict.
        let vs: BTreeSet<Vec<String>> = vscode
            .iter()
            .filter(|r| r.when.as_deref().map(str::trim) != Some("false"))
            .map(|r| parse_keybinding(r.key.as_deref().unwrap()).unwrap())
            .map(|k| k.iter().map(crate::keys::format_stroke).collect())
            .collect();
        let forbidden = parse_keybinding("ctrl+alt+a").unwrap();
        for (cmd, key) in section5().into_iter().filter(|(_, k)| k != "—") {
            let keys = parse_keybinding(&key).unwrap();
            let ours: Vec<String> = keys.iter().map(crate::keys::format_stroke).collect();
            assert!(
                !keys
                    .iter()
                    .any(|k| crate::keys::same_stroke(k, &forbidden[0])),
                "{cmd} uses ctrl+alt+a"
            );
            for theirs in &vs {
                let n = ours.len().min(theirs.len());
                let clash =
                    ours == *theirs || (ours.len() != theirs.len() && ours[..n] == theirs[..n]);
                assert!(
                    !clash,
                    "{cmd} `{}` conflicts with VS Code `{}`",
                    format_keybinding(&keys),
                    theirs.join(" ")
                );
            }
        }
    }
}
