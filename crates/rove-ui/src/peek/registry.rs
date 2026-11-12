//! 速览窗口的登记项（#41）：命令（VS Code ID）、界面字符串、颜色键、上下文键名、设置读取与
//! 一条 Rove 附加默认键位。
//!
//! - 设置项 `editor.stablePeek`、`editor.peekWidgetDefaultFocus`、`editor.gotoLocation.*` 由
//!   `rove_config::register_builtin` 按 compat/SETTINGS.md 登记，这里只读（[`PeekSettings`]）；
//! - 默认键位来自 #14 生成的 VS Code 默认键位表（导航一节），不重复登记；
//! - 唯一的附加键位见 [`EXTRA_DEFAULTS`]。

use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_config::{Key, Scope, Settings};
use rove_i18n::StringTable;
use rove_theme::ColorRegistry;
use std::sync::Arc;

pub const GO_TO_REFERENCES: &str = "editor.action.goToReferences";
pub const PEEK_DEFINITION: &str = "editor.action.peekDefinition";
pub const PEEK_IMPLEMENTATION: &str = "editor.action.peekImplementation";
pub const REFERENCE_SEARCH_TRIGGER: &str = "editor.action.referenceSearch.trigger";
pub const CLOSE: &str = "closeReferenceSearch";
pub const NEXT: &str = "goToNextReference";
pub const PREVIOUS: &str = "goToPreviousReference";
pub const NEXT_FROM_EMBEDDED: &str = "goToNextReferenceFromEmbeddedEditor";
pub const PREVIOUS_FROM_EMBEDDED: &str = "goToPreviousReferenceFromEmbeddedEditor";
pub const REVEAL: &str = "revealReference";
pub const OPEN_TO_SIDE: &str = "openReferenceToSide";
pub const TOGGLE_FOCUS: &str = "togglePeekWidgetFocus";

/// 发布的 VS Code 上下文键（KEYBINDINGS.md 导航一节的 when 子句用到）。
pub mod context {
    /// 编辑器上打开着速览窗口（发布在宿主编辑器与窗口自身上）。
    pub const VISIBLE: &str = "referenceSearchVisible";
    /// 焦点在速览左侧的预览里。
    pub const IN_EDITOR: &str = "inReferenceSearchEditor";
    /// 焦点在嵌入的编辑器（速览预览）里。
    pub const EMBEDDED: &str = "isInEmbeddedEditor";
    /// 当前编辑器有 C# 导航数据（引用 / 定义 / 实现）。
    pub const HAS_REFERENCES: &str = "editorHasReferenceProvider";
    pub const HAS_DEFINITION: &str = "editorHasDefinitionProvider";
    pub const HAS_IMPLEMENTATION: &str = "editorHasImplementationProvider";
    /// `editor.stablePeek` 的值（发布到全局上下文键）。
    pub const STABLE_PEEK: &str = "config.editor.stablePeek";
}

/// (命令 ID, 标题字符串 key, 中文标题)。
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (GO_TO_REFERENCES, "peek.command.goToReferences", "转到引用"),
    (PEEK_DEFINITION, "peek.command.peekDefinition", "速览定义"),
    (
        PEEK_IMPLEMENTATION,
        "peek.command.peekImplementation",
        "速览实现",
    ),
    (
        REFERENCE_SEARCH_TRIGGER,
        "peek.command.referenceSearchTrigger",
        "速览引用",
    ),
    (CLOSE, "peek.command.close", "关闭速览窗口"),
    (NEXT, "peek.command.next", "转到下一个引用"),
    (PREVIOUS, "peek.command.previous", "转到上一个引用"),
    (
        NEXT_FROM_EMBEDDED,
        "peek.command.nextFromEmbedded",
        "转到下一个引用",
    ),
    (
        PREVIOUS_FROM_EMBEDDED,
        "peek.command.previousFromEmbedded",
        "转到上一个引用",
    ),
    (REVEAL, "peek.command.reveal", "打开引用"),
    (OPEN_TO_SIDE, "peek.command.openToSide", "在侧边打开引用"),
    (
        TOGGLE_FOCUS,
        "peek.command.toggleFocus",
        "在速览预览与结果列表之间切换焦点",
    ),
];

/// 命令分类（命令面板里显示为“编辑器: …”，与代码视图共用）。
pub const CATEGORY_KEY: &str = "codeView.category";

/// Rove 附加的默认键位（VS Code keybindings.json 语法，用户可覆盖或用 `-命令` 删除）。
///
/// VS Code 表里 `alt+f12` 的 when 带 `!inReferenceSearchEditor && !isInEmbeddedEditor`，在速览预览
/// 里不生效；UX_SPEC §2.3 要求“在预览中用 `Alt+F12` 嵌套速览”，所以只为预览补这一条，
/// 表里原有的条目一条不改。
pub const EXTRA_DEFAULTS: &[(&str, &str, &str)] = &[(
    "alt+f12",
    PEEK_DEFINITION,
    "editorHasDefinitionProvider && editorTextFocus && inReferenceSearchEditor",
)];

/// 其余界面字符串。
pub const STRINGS: &[(&str, &str)] = &[
    ("peek.title.references", "{symbol} — {count} 个引用"),
    ("peek.title.definitions", "{symbol} — {count} 个定义"),
    ("peek.title.implementations", "{symbol} — {count} 个实现"),
    ("peek.marker.approximate", "近似"),
    ("peek.marker.candidates", "近似 · {count} 个候选"),
    ("peek.empty.references", "未找到“{symbol}”的引用"),
    (
        "peek.empty.references.approximate",
        "未找到“{symbol}”的引用（语义索引构建中）",
    ),
    ("peek.empty.definitions", "未找到“{symbol}”的定义"),
    (
        "peek.empty.definitions.approximate",
        "未找到“{symbol}”的定义（语义索引构建中）",
    ),
    ("peek.empty.implementations", "未找到“{symbol}”的实现"),
    (
        "peek.empty.implementations.approximate",
        "未找到“{symbol}”的实现（语义索引构建中）",
    ),
    ("peek.unavailable.implementations", "未找到实现"),
    ("peek.loading", "正在加载…"),
    ("peek.external", "外部符号：{signature}"),
    ("peek.fileCount", "{count}"),
    ("peek.lineNumber", "{line}"),
    ("peek.close", "关闭"),
    (
        "peek.readOnly",
        "速览中的代码是只读的。双击或按 Enter 在编辑器中打开。",
    ),
    ("peek.symbolHere", "此处的符号"),
];

/// 速览窗口的颜色键（THEME_COLORS.md “Peek view colors”，16 个）。粘性滚动的两个键随 R54
/// 接入，这里只登记。
pub const COLORS: &[&str] = &[
    "peekView.border",
    "peekViewEditor.background",
    "peekViewEditor.matchHighlightBackground",
    "peekViewEditor.matchHighlightBorder",
    "peekViewEditorGutter.background",
    "peekViewEditorStickyScroll.background",
    "peekViewEditorStickyScrollGutter.background",
    "peekViewResult.background",
    "peekViewResult.fileForeground",
    "peekViewResult.lineForeground",
    "peekViewResult.matchHighlightBackground",
    "peekViewResult.selectionBackground",
    "peekViewResult.selectionForeground",
    "peekViewTitle.background",
    "peekViewTitleDescription.foreground",
    "peekViewTitleLabel.foreground",
];

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
    let commands = COMMANDS.iter().map(|(_, k, t)| (*k, *t));
    for (key, text) in STRINGS.iter().copied().chain(commands) {
        match table.register(key, Arc::from(text)) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 登记颜色键；已存在的键跳过（主题或其他视图已登记）。
pub fn register_colors(registry: &mut ColorRegistry) -> Result<()> {
    for key in COLORS {
        if registry.get(key).is_some() {
            continue;
        }
        if let Some(defaults) = rove_theme::defaults::static_defaults(key) {
            registry.register(*key, defaults)?;
        }
    }
    Ok(())
}

/// `editor.peekWidgetDefaultFocus`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultFocus {
    Tree,
    Editor,
}

/// 速览用到的设置当前值。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeekSettings {
    /// `editor.stablePeek`：为 true 时 `Esc` 与在预览里双击都不关闭窗口。
    pub stable: bool,
    pub default_focus: DefaultFocus,
}

impl PeekSettings {
    pub fn read(settings: &dyn Settings) -> Self {
        let scope = Scope::default();
        Self {
            stable: Key::<bool>::new("editor.stablePeek")
                .read(settings, &scope)
                .unwrap_or(false),
            default_focus: match Key::<String>::new("editor.peekWidgetDefaultFocus")
                .read(settings, &scope)
                .as_deref()
            {
                Ok("editor") => DefaultFocus::Editor,
                _ => DefaultFocus::Tree,
            },
        }
    }

    /// 能否关闭（四条 `closeReferenceSearch` 键位与预览双击共用的统一判定）。
    pub fn can_close_by_escape(&self) -> bool {
        !self.stable
    }
}

/// 本模块命令的 GPUI action；由速览窗口或宿主编辑器处理，键位服务按 when 解析后分发。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = peek, no_json)]
pub struct PeekAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(PeekAction(cmd)) as Box<dyn Action>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_commands_strings_and_sixteen_colors() {
        let mut r = CommandRegistry::default();
        register_commands(&mut r).unwrap();
        register_commands(&mut r).unwrap();
        let mut t = StringTable::default();
        register_strings(&mut t).unwrap();
        for (id, title, _) in COMMANDS {
            assert!(r.get(id).is_some());
            assert!(t.get(title).is_some(), "{title}");
            assert!(action_for(id).is_some());
        }
        for (key, _) in STRINGS {
            assert!(t.get(key).is_some());
        }
        assert_eq!(COLORS.len(), 16);
        let mut colors = ColorRegistry::default();
        register_colors(&mut colors).unwrap();
        register_colors(&mut colors).unwrap();
        for key in COLORS {
            assert!(colors.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn every_navigation_row_of_the_keybinding_table_targets_a_registered_command() {
        let defaults = rove_commands::keybindings::default_rules();
        for id in [
            GO_TO_REFERENCES,
            PEEK_DEFINITION,
            PEEK_IMPLEMENTATION,
            CLOSE,
            NEXT,
            PREVIOUS,
            REVEAL,
            OPEN_TO_SIDE,
            TOGGLE_FOCUS,
        ] {
            assert!(
                defaults.iter().any(|d| &*d.rule.command == id),
                "{id} 应在生成的 VS Code 默认键位表里"
            );
        }
    }
}
