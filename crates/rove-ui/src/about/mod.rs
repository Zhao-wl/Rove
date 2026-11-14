//! 「关于」对话框（issue #76 第 4 节）：`workbench.action.showAboutDialog`。
//!
//! 显示版本号、提交号、构建日期、gpui 版本与 C# 索引组件状态，并提供「复制」按钮——
//! 排查用户问题时这是第一手信息。另有「第三方声明」按钮，打开随包分发的
//! `THIRD_PARTY_NOTICES.md`（和 `rove.exe` 放在同一个目录），用编辑器打开，不调用外部程序、不联网。
//!
//! 呈现方式是**浮层**而不是模态框：UX_SPEC §6.1 把模态框保留给「不处理就会丢数据」与安全确认，
//! 「关于」两样都不是（VS Code 用模态框，我们按自己的分级表走）。浮层由工作台绘制
//!（`skeleton/workbench/about_host.rs`），这里只有与界面无关的取值与排版。

pub mod info;
pub mod registry;

pub use info::{AboutInfo, Indexer};

use rove_i18n::{Message, Strings};
use std::sync::Arc;

/// 对话框里的一行：左边是标签，右边是值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub label: String,
    pub value: String,
}

fn text(strings: &dyn Strings, key: &str) -> String {
    rove_i18n::tr!(strings, &Message::key(key)).unwrap_or_else(|_| key.to_string())
}

fn text_with(strings: &dyn Strings, key: &str, args: &[(&str, &str)]) -> String {
    let message = Message {
        key: key.into(),
        args: args
            .iter()
            .map(|(k, v)| (Arc::from(*k), Arc::from(*v)))
            .collect(),
    };
    rove_i18n::tr!(strings, &message).unwrap_or_else(|_| key.to_string())
}

/// C# 索引组件状态的那一行文字。
pub fn indexer_text(strings: &dyn Strings, indexer: &Indexer) -> String {
    let unknown = text(strings, "about.unknown");
    match indexer {
        Indexer::Installed(Some(version)) => {
            text_with(strings, "about.indexer.installed", &[("version", version)])
        }
        Indexer::Installed(None) => text(strings, "about.indexer.installedUnknown"),
        Indexer::NotInstalled => text(strings, "about.indexer.notInstalled"),
        Indexer::Incompatible {
            version,
            component_too_old,
        } => {
            let key = if *component_too_old {
                "about.indexer.componentTooOld"
            } else {
                "about.indexer.componentTooNew"
            };
            let version = version.as_deref().unwrap_or(&unknown);
            text_with(strings, key, &[("version", version)])
        }
    }
}

/// 对话框正文的全部行（顺序即显示顺序）。
pub fn lines(strings: &dyn Strings, info: &AboutInfo) -> Vec<Line> {
    let unknown = text(strings, "about.unknown");
    let line = |key: &str, value: String| Line {
        label: text(strings, key),
        value,
    };
    let mut lines = vec![
        line("about.label.version", info.version.clone()),
        line("about.label.commit", info.short_commit()),
        line("about.label.date", info.build_date.clone()),
        line("about.label.channel", info.channel.clone()),
        line("about.label.gpui", info.gpui_version.clone()),
        line("about.label.indexer", indexer_text(strings, &info.indexer)),
    ];
    // #149：更新状态（没有更新服务时不显示这一行）。
    if let Some(state) = &info.update {
        let owned = state.args();
        let args: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        lines.push(Line {
            label: text(strings, "update.label"),
            value: text_with(strings, state.string_key(), &args),
        });
    }
    lines.push(line(
        "about.label.executable",
        info.executable
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or(unknown),
    ));
    lines
}

/// 「复制」按钮放进剪贴板的纯文本：每行 `标签: 值`，提交号用完整的 40 位。
pub fn clipboard_text(strings: &dyn Strings, info: &AboutInfo) -> String {
    let mut out = format!("{} {}\n", text(strings, "about.product"), info.version);
    for line in lines(strings, info) {
        // 提交号在对话框里只显示前 12 位，复制出来的要完整，方便直接 git show。
        let value = if line.label == text(strings, "about.label.commit") {
            info.commit.clone()
        } else {
            line.value
        };
        out.push_str(&format!("{}: {}\n", line.label, value));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::StringTable;

    fn strings() -> StringTable {
        let mut table = StringTable::default();
        registry::register_strings(&mut table).unwrap();
        table
    }

    fn info(indexer: Indexer) -> AboutInfo {
        AboutInfo {
            version: "0.1.0".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            build_date: "2026-11-14".into(),
            channel: "stable".into(),
            gpui_version: "0.2.2".into(),
            indexer,
            executable: Some(std::path::PathBuf::from(r"C:\Rove\rove.exe")),
            // #149：没有更新服务时不显示「更新」那一行。
            update: None,
        }
    }

    /// 验收：显示版本、提交号、构建日期、gpui 版本与索引组件状态。
    #[test]
    fn dialog_shows_version_commit_date_gpui_and_indexer() {
        let strings = strings();
        let lines = lines(&strings, &info(Indexer::Installed(Some("1.2.0.0".into()))));
        let values: Vec<&str> = lines.iter().map(|l| l.value.as_str()).collect();
        assert!(values.contains(&"0.1.0"), "{values:?}");
        assert!(
            values.contains(&"0123456789ab"),
            "提交号显示前 12 位：{values:?}"
        );
        assert!(values.contains(&"2026-11-14"), "{values:?}");
        assert!(values.contains(&"0.2.2"), "gpui 版本：{values:?}");
        assert!(
            values.iter().any(|v| v.contains("1.2.0.0")),
            "索引组件版本：{values:?}"
        );
        assert!(
            values.iter().any(|v| v.contains("rove.exe")),
            "可执行文件路径：{values:?}"
        );
        // 每一行的标签都来自登记过的字符串（不是写死的中文）。
        for line in &lines {
            assert!(!line.label.starts_with("about."), "{} 没有登记", line.label);
        }
    }

    #[test]
    fn indexer_line_covers_all_three_discovery_results() {
        let strings = strings();
        assert!(indexer_text(&strings, &Indexer::NotInstalled).contains("未安装"));
        assert!(indexer_text(&strings, &Indexer::Installed(None)).contains("开发覆盖"));
        let old = indexer_text(
            &strings,
            &Indexer::Incompatible {
                version: Some("1.0.0.0".into()),
                component_too_old: true,
            },
        );
        assert!(old.contains("1.0.0.0") && old.contains("更新组件"), "{old}");
        let new = indexer_text(
            &strings,
            &Indexer::Incompatible {
                version: None,
                component_too_old: false,
            },
        );
        assert!(new.contains("未知") && new.contains("更新 Rove"), "{new}");
    }

    /// 「复制」出来的文本是纯文本、带完整提交号，可以直接贴进 issue。
    #[test]
    fn copied_text_is_plain_and_has_the_full_commit() {
        let strings = strings();
        let info = info(Indexer::NotInstalled);
        let text = clipboard_text(&strings, &info);
        assert!(text.starts_with("Rove 0.1.0\n"), "{text}");
        assert!(
            text.contains(&info.commit),
            "复制的文本要带完整提交号：{text}"
        );
        assert!(text.contains("构建日期: 2026-11-14"), "{text}");
        assert!(text.contains("gpui: 0.2.2"), "{text}");
        assert_eq!(text.lines().count(), 8, "标题 + 七行：{text}");
    }
}
