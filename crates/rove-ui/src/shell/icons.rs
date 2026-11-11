//! 产品图标（codicons）与 `$(图标名)` 语法（#16，UX_SPEC §1.5、FEATURES §7）。
//!
//! 字体与名称映射来自 VS Code 的 `@vscode/codicons`（CC-BY-4.0，见 THIRD_PARTY_NOTICES.md），
//! 随二进制编译，运行期不读文件、不联网。其他 issue 只引用图标名，不各自打包图标。
//!
//! 语法与 VS Code 的 `renderLabelWithIcons` 一致：`$(name)` 或 `$(name~spin)`；`\$(` 输出字面量 `$(`；
//! 未知图标名按原文输出（与 VS Code 相同，不吞掉文字）。
use std::{collections::BTreeMap, sync::OnceLock};

/// 字体族名，与 `codicon.ttf` 的 name 表一致。
pub const FONT_FAMILY: &str = "codicon";
/// 打包的字体文件（GPUI 启动时通过 `text_system().add_fonts` 注册）。
pub const FONT: &[u8] = include_bytes!("../../assets/codicons/codicon.ttf");
const MAPPING: &str = include_str!("../../assets/codicons/mapping.json");

/// 名称到码位的映射；首次访问时解析一次（约 600 项，微秒级），之后只读。
pub fn codepoint(name: &str) -> Option<char> {
    static TABLE: OnceLock<BTreeMap<String, char>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            let raw: BTreeMap<String, u32> =
                serde_json::from_str(MAPPING).expect("内置 codicons 映射必须是合法 JSON");
            raw.into_iter()
                .filter_map(|(k, v)| char::from_u32(v).map(|c| (k, c)))
                .collect()
        })
        .get(name)
        .copied()
}

/// 解析后的标签片段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelPart {
    Text(String),
    Icon {
        name: String,
        glyph: char,
        spin: bool,
    },
}

/// 把 `$(icon) 文本` 拆成文字与图标片段；相邻文字合并。
pub fn parse_label(label: &str) -> Vec<LabelPart> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut rest = label;
    while let Some(index) = rest.find("$(") {
        let escaped = rest[..index].ends_with('\\');
        let (before, after) = rest.split_at(index);
        if escaped {
            text.push_str(&before[..before.len() - 1]);
            text.push_str("$(");
            rest = &after[2..];
            continue;
        }
        text.push_str(before);
        let body = &after[2..];
        let Some(close) = body.find(')') else {
            text.push_str(after);
            rest = "";
            break;
        };
        let spec = &body[..close];
        let (name, modifier) = spec.split_once('~').unwrap_or((spec, ""));
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        match codepoint(name).filter(|_| valid) {
            Some(glyph) => {
                if !text.is_empty() {
                    parts.push(LabelPart::Text(std::mem::take(&mut text)));
                }
                parts.push(LabelPart::Icon {
                    name: name.to_owned(),
                    glyph,
                    spin: modifier == "spin",
                });
            }
            None => text.push_str(&after[..close + 3]),
        }
        rest = &body[close + 1..];
    }
    text.push_str(rest);
    if !text.is_empty() {
        parts.push(LabelPart::Text(text));
    }
    parts
}

/// 去掉图标后的纯文本（用于无障碍名称与测试断言），与 VS Code 的 `stripIcons` 相同。
pub fn strip_icons(label: &str) -> String {
    parse_label(label)
        .into_iter()
        .filter_map(|p| match p {
            LabelPart::Text(t) => Some(t),
            LabelPart::Icon { .. } => None,
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_covers_icons_used_by_the_workbench() {
        for name in [
            "files",
            "search",
            "settings-gear",
            "error",
            "warning",
            "info",
            "check",
            "sync",
            "circle-slash",
            "bell",
            "bell-dot",
            "bell-slash",
            "close",
            "chevron-down",
            "chevron-right",
            "layout-sidebar-left",
            "layout-sidebar-left-off",
            "layout-sidebar-right",
            "layout-sidebar-right-off",
            "layout-panel",
            "layout-panel-off",
            "chrome-minimize",
            "chrome-maximize",
            "chrome-restore",
            "chrome-close",
            "menu",
            "clear-all",
        ] {
            assert!(codepoint(name).is_some(), "缺少图标 {name}");
        }
        assert!(FONT.len() > 10_000);
    }

    #[test]
    fn parses_icons_spin_escape_and_unknown_names() {
        let parts = parse_label("$(sync~spin) C# 索引 37%");
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], LabelPart::Icon { name, spin: true, .. } if name == "sync"));
        assert_eq!(parts[1], LabelPart::Text(" C# 索引 37%".into()));
        assert_eq!(
            parse_label("\\$(check) 字面量"),
            vec![LabelPart::Text("$(check) 字面量".into())]
        );
        assert_eq!(
            parse_label("$(no-such-icon) x"),
            vec![LabelPart::Text("$(no-such-icon) x".into())]
        );
        assert_eq!(
            parse_label("未闭合 $(check"),
            vec![LabelPart::Text("未闭合 $(check".into())]
        );
        assert_eq!(strip_icons("$(error) 0 $(warning) 0"), "0  0");
    }
}
