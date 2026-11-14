//! Skeleton strings and contribution registrations. The theme comes from `rove_theme::ThemeService` (#10).
use crate::contributions::*;
use rove_base::*;
use rove_i18n::*;
use std::sync::Arc;
pub fn strings() -> Result<StringTable> {
    let mut t = chinese()?;
    for (k, v) in [
        ("app.title", "Rove"),
        (
            "app.noUserDir",
            "找不到 VS Code 用户目录，请用 --vscode-user-dir 指定。",
        ),
        ("app.failed", "无法启动 Rove，请查看日志。"),
        ("workbench.explorer", "资源管理器"),
        ("workbench.scanFailed", "无法扫描文件夹，请检查路径和权限。"),
        ("tree.open", "▾ {name}"),
        ("tree.closed", "▸ {name}"),
        ("editors.splitLeft", "←"),
        ("editors.splitUp", "↑"),
        ("editors.splitDown", "↓"),
        ("editors.splitRight", "→"),
        ("editors.lock", "◇"),
        ("editors.unlock", "◆"),
        ("editors.maximize", "□"),
        ("editors.restore", "▣"),
        ("editors.closeTab", "×"),
        ("editors.pinned", "◆"),
        ("editors.dirty", "●"),
        ("editors.errorBadge", "!"),
        ("editors.index", "{index}"),
        ("editors.markdown", "Markdown 预览"),
        ("editors.fileMissing", "无法打开编辑器：找不到文件。"),
        ("editors.fileDenied", "无法打开编辑器：没有读取权限。"),
        (
            "editors.fileUnsupported",
            "无法打开编辑器：文件格式或编码不受支持。",
        ),
        (
            "editors.invalidLocation",
            "无法定位：指定的行或列不在文件中。",
        ),
        (
            "editors.fileUnavailable",
            "无法打开编辑器：文件可能正被占用，或路径无效。",
        ),
        ("editors.retry", "重试"),
        ("editors.reveal", "在资源管理器中显示"),
    ] {
        t.register(k, Arc::from(v))?;
    }
    crate::shell::catalog::register_strings(&mut t)?;
    crate::code_view::commands::register_strings(&mut t)?;
    crate::editors::actions::strings(&mut t)?;
    // #14：列表/快速输入的命令标题（它们的命令登记在键位服务的命令表里）、Rove 专有命令与和弦提示。
    crate::list_widget::registry::register_strings(&mut t)?;
    rove_commands::builtin::register_strings(&mut t)?;
    // #36：C# 索引状态的文案。
    crate::index_status::register_strings(&mut t)?;
    crate::outline::registry::register_strings(&mut t)?;
    crate::find::registry::register_strings(&mut t)?;
    // #24：快速打开、命令面板与转到行的标题和提示。
    crate::quick_access::registry::register_strings(&mut t)?;
    // #49：保存失败通知、无法编码字符与关闭确认的模态框、外部修改提示条。
    crate::save::registry::register_strings(&mut t)?;
    // #50：热退出的状态栏提示。
    crate::hotexit::register_strings(&mut t)?;
    // #40：C# 导航交互（空状态、近似提示、悬停卡片、命令标题）。
    crate::nav::registry::register_strings(&mut t)?;
    // #25：搜索视图。
    crate::search::registry::register_strings(&mut t)?;
    // #41：速览窗口的命令标题、标题模板与提示。
    crate::peek::registry::register_strings(&mut t)?;
    // #42：元数据视图的标签标题与悬停提示。
    crate::metadata::registry::register_strings(&mut t)?;
    // #23：资源管理器的空状态、读取失败提示与命令标题。
    crate::explorer::registry::register_strings(&mut t)?;
    // #29：Markdown 预览。
    crate::markdown::registry::register_strings(&mut t)?;
    // #28：欢迎页、最近打开、水印与首选项入口。
    crate::welcome::registry::register_strings(&mut t)?;
    // #22：超大文件提示条、状态栏标记、行索引进度、转到行等待与超长行截断标记。
    crate::large_file::registry::register_strings(&mut t)?;
    // #27：状态栏右侧的文件状态条目与它们的选择器。
    crate::status_items::registry::register_strings(&mut t)?;
    // #60：引用视图的命令标题、标题模板与空状态。
    crate::references::registry::register_strings(&mut t)?;
    // #60：转到工作区中的符号的命令标题、占位符与近似说明。
    crate::workspace_symbols::register_strings(&mut t)?;
    // #76：「关于」对话框。
    crate::about::registry::register_strings(&mut t)?;
    // #149：自动更新的命令、状态、提示条与通知。
    crate::update::registry::register_strings(&mut t)?;
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
/// 工作台自带的视图容器、小节、面板与状态栏条目由 #16 的 `shell::catalog` 登记。
pub fn contributions() -> Result<ContributionRegistry> {
    let mut r = ContributionRegistry::default();
    crate::shell::catalog::register_contributions(&mut r)?;
    crate::outline::registry::register_contributions(&mut r)?;
    // #27：状态栏右侧的六个文件状态条目。
    crate::status_items::registry::register_contributions(&mut r)?;
    // #36：C# 索引状态条目与索引详情面板。
    crate::index_status::register_contributions(&mut r)?;
    // #25：搜索视图容器里的结果小节。
    crate::search::registry::register_contributions(&mut r)?;
    // #29：编辑器标题栏的“显示源码 / 显示预览”按钮。
    crate::markdown::registry::register_contributions(&mut r)?;
    // #22：文件大小标记与行索引进度（状态栏右侧）。
    crate::large_file::registry::register_contributions(&mut r)?;
    // #60：面板里的「引用」视图（与「问题」并列，UX_SPEC §1.4）。
    crate::references::registry::register_contributions(&mut r)?;
    Ok(r)
}
