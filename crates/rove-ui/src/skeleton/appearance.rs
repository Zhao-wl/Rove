//! Skeleton strings and contribution registrations. The theme comes from `rove_theme::ThemeService` (#10).
use crate::contributions::*;
use rove_base::*;
use rove_i18n::*;
use std::sync::Arc;
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
