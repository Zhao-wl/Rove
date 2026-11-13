//! Markdown 预览的登记项（#29）：命令（VS Code 1.145 的 ID 与默认键位）、界面字符串、颜色键、
//! 编辑器标题栏按钮（`Placement::EditorTitle`）与命令的 GPUI action。
//!
//! - 默认键位来自 #14 生成的 VS Code 默认键位表（`ctrl+shift+v` → `markdown.togglePreview`、
//!   `ctrl+k v` → `markdown.showPreviewToSide`，`when` 照抄 `KEYBINDINGS.md`），本模块不重复登记；
//! - 设置项 `markdown.*` 与 `rove.markdown.*` 已由 `rove_config::register_builtin` 按
//!   `compat/SETTINGS.md` 登记（含 `markdown.preview.doubleClickToSwitchToEditor` 的 Rove 默认值
//!   `true`，只在用户未设置时生效），本模块只读取；
//! - **不登记** `markdown.preview.toggleLock`、`markdown.showLockedPreviewToSide`、
//!   `markdown.showPreviewSecuritySelector`：Rove 的预览总是绑定到一个文件（相当于 VS Code 的锁定预览），
//!   远程资源始终拦截；未实现的命令不进命令面板（ARCHITECTURE §6）。
use crate::contributions::{Contribution, ContributionRegistry, Placement};
use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::{Message, StringTable};
use rove_theme::ColorRegistry;
use std::sync::Arc;

/// VS Code 的 Markdown 预览编辑器类型 ID（`workbench.editorAssociations` 的值）。
pub const EDITOR_ID: &str = "vscode.markdown.preview.editor";

pub const TOGGLE_PREVIEW: &str = "markdown.togglePreview";
pub const SHOW_PREVIEW: &str = "markdown.showPreview";
pub const SHOW_PREVIEW_TO_SIDE: &str = "markdown.showPreviewToSide";
pub const SHOW_SOURCE: &str = "markdown.showSource";
pub const REFRESH: &str = "markdown.preview.refresh";

/// 发布在工作台根上的 VS Code 上下文键（活动编辑器是预览时）。
pub const ACTIVE_CUSTOM_EDITOR_ID: &str = "activeCustomEditorId";

/// (命令 ID, 标题字符串 key, 中文标题)
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (TOGGLE_PREVIEW, "markdown.command.togglePreview", "切换预览"),
    (SHOW_PREVIEW, "markdown.command.showPreview", "打开预览"),
    (
        SHOW_PREVIEW_TO_SIDE,
        "markdown.command.showPreviewToSide",
        "打开侧边预览",
    ),
    (SHOW_SOURCE, "markdown.command.showSource", "显示源"),
    (REFRESH, "markdown.command.refresh", "刷新预览"),
];

/// 命令分类（命令面板里显示为“Markdown: …”）。
pub const CATEGORY_KEY: &str = "markdown.category";

/// 提示条与通知的去重 key。
pub const REMOTE_BAR: &str = "markdown.remoteBlocked";
pub const RUNTIME_MISSING: &str = "markdown.webviewMissing";
pub const CREATE_FAILED: &str = "markdown.webviewFailed";
pub const EXTERNAL_LINK: &str = "markdown.openExternalLink";
/// 反馈按钮发出的动作 ID（不是命令，只在本模块的反馈订阅里处理）。
pub const ACTION_DOWNLOAD_PAGE: &str = "markdown.action.openWebView2Download";
pub const ACTION_OPEN_LINK: &str = "markdown.action.openLink";
pub const ACTION_CANCEL: &str = "markdown.action.cancel";

/// 微软官方的 WebView2 运行时下载页（只在用户点击按钮时交给系统浏览器打开，Rove 自己不下载）。
pub const WEBVIEW2_DOWNLOAD_PAGE: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

/// 其余界面字符串。
pub const STRINGS: &[(&str, &str)] = &[
    ("markdown.category", "Markdown"),
    ("markdown.title.showSource", "显示源码"),
    ("markdown.title.showPreview", "显示预览"),
    ("markdown.title.showSource.icon", "$(go-to-file)"),
    ("markdown.title.showPreview.icon", "$(open-preview)"),
    ("markdown.loading", "正在加载预览…"),
    (
        "markdown.otherGroup",
        "预览正显示在另一个编辑器组中。单击此处在这里显示。",
    ),
    ("markdown.remoteBlocked", "已阻止 {count} 个远程资源。"),
    (
        "markdown.webviewMissing",
        "无法显示 Markdown 预览：此计算机没有安装 Microsoft Edge WebView2 运行时，Markdown 文件已改为以源码打开。安装运行时后重新启动 Rove 即可使用预览。",
    ),
    ("markdown.openDownloadPage", "打开下载页"),
    (
        "markdown.webviewFailed",
        "无法创建 Markdown 预览（{error}），Markdown 文件已改为以源码打开。",
    ),
    ("markdown.openExternal", "是否要打开外部网站？{url}"),
    ("markdown.open", "打开"),
    ("markdown.cancel", "取消"),
    ("markdown.alert.note", "备注"),
    ("markdown.alert.tip", "提示"),
    ("markdown.alert.important", "重要"),
    ("markdown.alert.warning", "警告"),
    ("markdown.alert.caution", "注意"),
];

/// 编辑器标题栏按钮的工厂 key（由编辑器组绘制）。
pub const TITLE_FACTORY: &str = "markdown.editorTitle";
pub const TITLE_SHOW_SOURCE: &str = "markdown.editorTitle.showSource";
pub const TITLE_SHOW_PREVIEW: &str = "markdown.editorTitle.showPreview";

/// 预览样式里用到、但不在 P0 颜色表里的 VS Code 颜色键（默认值取 #10 的 VS Code 默认值表）。
pub const COLORS: &[&str] = &[
    "textPreformat.foreground",
    "textPreformat.background",
    "textBlockQuote.background",
    "textBlockQuote.border",
    "textCodeBlock.background",
    "textLink.foreground",
    "textLink.activeForeground",
    "keybindingLabel.foreground",
    "keybindingLabel.background",
    "keybindingLabel.border",
    "keybindingLabel.bottomBorder",
    "diffEditor.insertedTextBackground",
    "diffEditor.removedTextBackground",
    "diffEditor.insertedTextBorder",
    "diffEditor.removedTextBorder",
    "diffEditorGutter.insertedLineBackground",
    "diffEditorGutter.removedLineBackground",
    "editorGutter.modifiedBackground",
    "testing.iconPassed",
    "toolbar.hoverBackground",
    "toolbar.activeBackground",
    "inputValidation.errorBackground",
    "editorError.foreground",
    "editorWidget.border",
    "widget.border",
    "contrastBorder",
    "editor.background",
    "editor.foreground",
    "editor.selectionBackground",
    "editorGroup.border",
    "focusBorder",
    "markdownAlert.note.foreground",
    "markdownAlert.tip.foreground",
    "markdownAlert.important.foreground",
    "markdownAlert.warning.foreground",
    "markdownAlert.caution.foreground",
];

/// 预览命令的 GPUI action：键位服务把命令派发成它，宿主执行同 ID 的命令。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = rove_markdown, no_json)]
pub struct MarkdownAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(MarkdownAction(cmd)) as Box<dyn Action>)
}

pub fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, title, _) in COMMANDS {
        match registry.register(
            *id,
            Command {
                title_key: (*title).into(),
                category_key: Some(CATEGORY_KEY.into()),
            },
        ) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn register_strings(table: &mut StringTable) -> Result<()> {
    for (key, text) in COMMANDS
        .iter()
        .map(|(_, key, text)| (*key, *text))
        .chain(STRINGS.iter().copied())
    {
        match table.register(key, Arc::from(text)) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 已存在的键跳过（主题或其他视图已登记）。
pub fn register_colors(registry: &mut ColorRegistry) -> Result<()> {
    let missing: Vec<&str> = COLORS
        .iter()
        .copied()
        .filter(|key| registry.get(key).is_none())
        .filter(|key| rove_theme::defaults::static_defaults(key).is_some())
        .collect();
    rove_theme::defaults::register_vscode_colors(registry, &missing)
}

/// 编辑器标题栏的“显示源码 / 显示预览”按钮（#1 的 `Placement::EditorTitle` 登记点）。
pub fn register_contributions(registry: &mut ContributionRegistry) -> Result<()> {
    registry.register(
        TITLE_SHOW_SOURCE,
        Contribution {
            title: Message::key("markdown.title.showSource"),
            placement: Placement::EditorTitle,
            order: 0,
            icon: Some("go-to-file".into()),
            command: Some(TOGGLE_PREVIEW.into()),
            when: Some(rove_commands::When(
                format!("{ACTIVE_CUSTOM_EDITOR_ID} == '{EDITOR_ID}'").into(),
            )),
            factory_key: TITLE_FACTORY.into(),
        },
    )?;
    registry.register(
        TITLE_SHOW_PREVIEW,
        Contribution {
            title: Message::key("markdown.title.showPreview"),
            placement: Placement::EditorTitle,
            order: 0,
            icon: Some("open-preview".into()),
            command: Some(TOGGLE_PREVIEW.into()),
            when: Some(rove_commands::When(
                "resourceLangId =~ /^(markdown|prompt|instructions|chatagent|skill)$/".into(),
            )),
            factory_key: TITLE_FACTORY.into(),
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_i18n::Strings;

    #[test]
    fn commands_strings_and_keys_are_registered() {
        let mut commands = CommandRegistry::default();
        register_commands(&mut commands).unwrap();
        let mut strings = StringTable::default();
        register_strings(&mut strings).unwrap();
        for (id, title, _) in COMMANDS {
            assert_eq!(commands.get(id).unwrap().title_key.as_ref(), *title);
            assert!(strings.text(&Message::key(*title)).is_ok());
            assert!(action_for(id).is_some());
        }
        for (key, _) in STRINGS {
            assert!(strings.get(key).is_some(), "{key}");
        }
        // 默认键位来自 VS Code 默认表，`when` 与 KEYBINDINGS.md 一致。
        let table = include_str!("../../../rove-commands/data/default-keybindings.json");
        assert!(table.contains(r#"{"key": "ctrl+shift+v", "command": "markdown.togglePreview", "when": "!terminalFocus && activeCustomEditorId == 'vscode.markdown.preview.editor' || editorFocus && !notebookEditorFocused && !terminalFocus && resourceLangId =~ /^(markdown|prompt|instructions|chatagent|skill)$/""#));
        assert!(table.contains(r#"{"key": "ctrl+k v", "command": "markdown.showPreviewToSide", "when": "editorFocus && !notebookEditorFocused && editorLangId =~ /^(markdown|prompt|instructions|chatagent|skill)$/""#));
        let doc = include_str!("../../../../docs/compat/KEYBINDINGS.md");
        assert!(doc.contains("| P0 | `ctrl+shift+v` | `markdown.togglePreview` | `!terminalFocus && activeCustomEditorId == 'vscode.markdown.preview.editor' \\|\\| editorFocus && !notebookEditorFocused && !terminalFocus && resourceLangId =~ /^(markdown\\|prompt\\|instructions\\|chatagent\\|skill)$/` |"));
    }

    #[test]
    fn colors_cover_the_alert_keys() {
        let mut registry = rove_theme::service::p0_registry();
        register_colors(&mut registry).unwrap();
        for key in COLORS {
            assert!(registry.get(key).is_some(), "{key}");
        }
    }
}
