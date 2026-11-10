//! SKELETON(theme): Dark 2026 subset, replaced by #10. Source/license: THIRD_PARTY_NOTICES.md.
use crate::contributions::*;
use rove_base::*;
use rove_i18n::*;
use rove_theme::*;
use std::sync::Arc;
pub const KEYWORD: Color = Color(0xff, 0x7b, 0x72, 255);
pub struct DarkTheme {
    colors: ColorRegistry,
}
impl DarkTheme {
    pub fn new() -> Result<Self> {
        let mut colors = ColorRegistry::default();
        for (key, value) in [
            ("editor.background", 0x121314),
            ("editor.foreground", 0xbbbebf),
            ("sideBar.background", 0x191a1b),
            ("sideBar.foreground", 0xbfbfbf),
            ("statusBar.background", 0x191a1b),
            ("statusBar.foreground", 0x8c8c8c),
            ("editorLineNumber.foreground", 0x858889),
        ] {
            let c = Some(Color(
                (value >> 16) as u8,
                (value >> 8) as u8,
                value as u8,
                255,
            ));
            colors.register(
                key,
                ColorDefaults {
                    dark: c,
                    light: c,
                    high_contrast_dark: c,
                    high_contrast_light: c,
                },
            )?;
        }
        Ok(Self { colors })
    }
}
impl Theme for DarkTheme {
    fn color(&self, id: &str) -> Result<Option<Color>> {
        self.colors.get(id).map(|v| v.dark).ok_or(Error::NotFound)
    }
    fn token_style(&self, scopes: &[Arc<str>]) -> TokenStyle {
        // TextMate selectors match a scope and its dot-separated descendants.
        const RULES: [(&str, Color); 5] = [
            ("keyword", KEYWORD),
            ("storage", KEYWORD),
            ("comment", Color(0x8b, 0x94, 0x9e, 255)),
            ("string", Color(0xa5, 0xd6, 0xff, 255)),
            ("constant.numeric", Color(0x79, 0xc0, 0xff, 255)),
        ];
        let color = scopes.iter().rev().find_map(|s| {
            RULES.iter().find_map(|&(prefix, color)| {
                s.strip_prefix(prefix)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
                    .then_some(color)
            })
        });
        TokenStyle {
            foreground: color,
            ..Default::default()
        }
    }
    fn changes(&self) -> &dyn Events<Version> {
        &NoEvents
    }
}
pub fn strings() -> Result<StringTable> {
    let mut t = chinese()?;
    for (k, v) in [
        ("app.title", "Rove"),
        ("app.usage", "用法：rove <文件夹路径>"),
        ("app.failed", "无法启动 Rove，请查看日志。"),
        ("workbench.explorer", "资源管理器"),
        ("workbench.empty", "从左侧选择一个文件"),
        (
            "workbench.failed",
            "无法打开：{path}。请重试或选择其他文件。",
        ),
        ("workbench.scanFailed", "无法扫描文件夹，请检查路径和权限。"),
        ("workbench.status", "{name}  |  {lines} 行  |  {encoding}"),
        ("encoding.utf8", "UTF-8"),
        ("commands.quickOpen", "快速打开（尚未实现）"),
        ("commands.palette", "命令面板（尚未实现）"),
        ("tree.open", "▾ {name}"),
        ("tree.closed", "▸ {name}"),
        ("line.number", "{line}"),
        ("workbench.statusTitle", "文件状态"),
    ] {
        t.register(k, Arc::from(v))?;
    }
    Ok(t)
}
pub fn message(t: &dyn Strings, key: &str, args: &[(&str, String)]) -> String {
    let m = Message {
        key: key.into(),
        args: args
            .iter()
            .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
            .collect(),
    };
    tr!(t, &m).expect("registered skeleton string")
}
/// SKELETON(workbench): registered fixed placements; replaced by #16.
/// SKELETON(status): fixed filename/line-count/encoding status; replaced by #27.
pub fn contributions() -> Result<ContributionRegistry> {
    let mut r = ContributionRegistry::default();
    for (id, key, placement) in [
        (
            "workbench.view.explorer",
            "workbench.explorer",
            Placement::Sidebar {
                container: "workbench.view.explorer".into(),
            },
        ),
        (
            "rove.status.file",
            "workbench.statusTitle",
            Placement::StatusLeft,
        ),
    ] {
        r.register(
            id,
            Contribution {
                title: Message::key(key),
                placement,
                order: 0,
                icon: None,
                command: None,
                when: None,
                factory_key: id.into(),
            },
        )?;
    }
    Ok(r)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_style_matches_textmate_scope_prefixes() {
        let theme = DarkTheme::new().unwrap();
        let style = |scope: &str| theme.token_style(&[Arc::from(scope)]).foreground;
        assert_eq!(style("storage.type.class.cs"), Some(KEYWORD));
        assert_eq!(style("keyword.control.flow.return.cs"), Some(KEYWORD));
        assert_eq!(
            style("comment.line.double-slash.cs"),
            Some(Color(0x8b, 0x94, 0x9e, 255))
        );
        assert_eq!(
            style("string.quoted.double.cs"),
            Some(Color(0xa5, 0xd6, 0xff, 255))
        );
        assert_eq!(style("storagex.cs"), None);
    }
}
