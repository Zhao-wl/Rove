//! #40 的登记项：命令（VS Code ID）、界面字符串、颜色键与上下文键名。
//!
//! - 设置项（`editor.gotoLocation.*`、`editor.hover.*`、`editor.definitionLinkOpensInPeek`、
//!   `editor.occurrencesHighlight*`）由 `rove_config::register_builtin` 按 compat/SETTINGS.md 登记，这里只读
//!   （[`super::settings::NavSettings`]）；
//! - 默认键位来自 #14 生成的 VS Code 默认键位表（`f12`、`ctrl+k f12`、`ctrl+f12`、`ctrl+k ctrl+i`、`f7` 等），
//!   不重复登记；
//! - `editor.wordHighlight*` 颜色由代码视图（#20）登记，这里只登记悬停卡片与链接的颜色。

use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use rove_theme::{Color, ColorRegistry};
use std::sync::Arc;

pub const REVEAL_DEFINITION: &str = "editor.action.revealDefinition";
pub const REVEAL_DEFINITION_ASIDE: &str = "editor.action.revealDefinitionAside";
pub const GOTO_DECLARATION: &str = "editor.action.goToDeclaration";
pub const GOTO_IMPLEMENTATION: &str = "editor.action.goToImplementation";
pub const NEXT_SYMBOL: &str = "editor.gotoNextSymbolFromResult";
pub const NEXT_SYMBOL_CANCEL: &str = "editor.gotoNextSymbolFromResult.cancel";
pub const SHOW_HOVER: &str = "editor.action.showHover";
pub const SCROLL_UP_HOVER: &str = "editor.action.scrollUpHover";
pub const SCROLL_DOWN_HOVER: &str = "editor.action.scrollDownHover";
pub const SCROLL_LEFT_HOVER: &str = "editor.action.scrollLeftHover";
pub const SCROLL_RIGHT_HOVER: &str = "editor.action.scrollRightHover";
pub const PAGE_UP_HOVER: &str = "editor.action.pageUpHover";
pub const PAGE_DOWN_HOVER: &str = "editor.action.pageDownHover";
pub const GOTO_TOP_HOVER: &str = "editor.action.goToTopHover";
pub const GOTO_BOTTOM_HOVER: &str = "editor.action.goToBottomHover";
pub const INCREASE_HOVER_VERBOSITY: &str = "editor.action.increaseHoverVerbosityLevel";
pub const DECREASE_HOVER_VERBOSITY: &str = "editor.action.decreaseHoverVerbosityLevel";
pub const WORD_HIGHLIGHT_NEXT: &str = "editor.action.wordHighlight.next";
pub const WORD_HIGHLIGHT_PREV: &str = "editor.action.wordHighlight.prev";
pub const WORD_HIGHLIGHT_TRIGGER: &str = "editor.action.wordHighlight.trigger";

/// 本模块发布的 VS Code 上下文键（KEYBINDINGS.md 的 when 子句用到）。
pub mod context {
    /// 当前编辑器的语言有定义提供方（C#）。`f12` 的 when。
    pub const HAS_DEFINITION: &str = "editorHasDefinitionProvider";
    pub const HAS_DECLARATION: &str = "editorHasDeclarationProvider";
    pub const HAS_IMPLEMENTATION: &str = "editorHasImplementationProvider";
    pub const HAS_REFERENCE: &str = "editorHasReferenceProvider";
    pub const HAS_HOVER: &str = "editorHasHoverProvider";
    /// 多候选的符号导航模式（`f12` 循环、`escape` 取消）。
    pub const HAS_SYMBOLS: &str = "hasSymbols";
    /// 悬停卡片可见 / 有焦点（卡片内的键盘导航）。
    pub const HOVER_VISIBLE: &str = "editorHoverVisible";
    pub const HOVER_FOCUSED: &str = "editorHoverFocused";

    /// C# 编辑器发布的“有提供方”键（D2：其他语言只有高亮和折叠，一个都不发布）。
    pub const PROVIDERS: &[&str] = &[
        HAS_DEFINITION,
        HAS_DECLARATION,
        HAS_IMPLEMENTATION,
        HAS_REFERENCE,
        HAS_HOVER,
    ];
}

/// (命令 ID, 标题字符串 key, 中文标题)。标题取 VS Code 中文语言包的措辞。
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (
        REVEAL_DEFINITION,
        "nav.command.revealDefinition",
        "转到定义",
    ),
    (
        REVEAL_DEFINITION_ASIDE,
        "nav.command.revealDefinitionAside",
        "打开侧边的定义",
    ),
    (GOTO_DECLARATION, "nav.command.goToDeclaration", "转到声明"),
    (
        GOTO_IMPLEMENTATION,
        "nav.command.goToImplementation",
        "转到实现",
    ),
    (NEXT_SYMBOL, "nav.command.nextSymbol", "转到下一个符号结果"),
    (
        NEXT_SYMBOL_CANCEL,
        "nav.command.nextSymbolCancel",
        "退出符号结果导航",
    ),
    (SHOW_HOVER, "nav.command.showHover", "显示或聚焦悬停"),
    (SCROLL_UP_HOVER, "nav.command.scrollUpHover", "向上滚动悬停"),
    (
        SCROLL_DOWN_HOVER,
        "nav.command.scrollDownHover",
        "向下滚动悬停",
    ),
    (
        SCROLL_LEFT_HOVER,
        "nav.command.scrollLeftHover",
        "向左滚动悬停",
    ),
    (
        SCROLL_RIGHT_HOVER,
        "nav.command.scrollRightHover",
        "向右滚动悬停",
    ),
    (PAGE_UP_HOVER, "nav.command.pageUpHover", "向上翻页悬停"),
    (PAGE_DOWN_HOVER, "nav.command.pageDownHover", "向下翻页悬停"),
    (GOTO_TOP_HOVER, "nav.command.goToTopHover", "转到悬停顶部"),
    (
        GOTO_BOTTOM_HOVER,
        "nav.command.goToBottomHover",
        "转到悬停底部",
    ),
    (
        INCREASE_HOVER_VERBOSITY,
        "nav.command.increaseHoverVerbosity",
        "提高悬停详细程度",
    ),
    (
        DECREASE_HOVER_VERBOSITY,
        "nav.command.decreaseHoverVerbosity",
        "降低悬停详细程度",
    ),
    (
        WORD_HIGHLIGHT_NEXT,
        "nav.command.wordHighlightNext",
        "转到下一个突出显示的符号",
    ),
    (
        WORD_HIGHLIGHT_PREV,
        "nav.command.wordHighlightPrev",
        "转到上一个突出显示的符号",
    ),
    (
        WORD_HIGHLIGHT_TRIGGER,
        "nav.command.wordHighlightTrigger",
        "触发符号高亮",
    ),
];

/// 命令分类（命令面板里显示为“编辑器: …”，与代码视图共用）。
pub const CATEGORY_KEY: &str = "codeView.category";

/// 其余界面字符串（UX_SPEC §2.2、§6.1–§6.3 的措辞：先说发生了什么，不加“错误：”前缀）。
pub const STRINGS: &[(&str, &str)] = &[
    ("nav.noDefinition", "未找到 `{name}` 的定义"),
    (
        "nav.noDefinition.approximate",
        "未找到 `{name}` 的定义（语义索引构建中）",
    ),
    ("nav.noDeclaration", "未找到 `{name}` 的声明"),
    (
        "nav.noDeclaration.approximate",
        "未找到 `{name}` 的声明（语义索引构建中）",
    ),
    ("nav.noImplementation", "未找到 `{name}` 的实现"),
    (
        "nav.noImplementation.approximate",
        "未找到 `{name}` 的实现（语义索引构建中）",
    ),
    ("nav.approximateJump", "近似结果：语义索引未就绪"),
    ("nav.noMetadata", "未找到 `{name}` 的元数据"),
    ("nav.symbolPosition", "第 {index} / 共 {total} 个符号"),
    ("nav.peek.approximateCandidates", "近似 · {count} 个候选"),
    ("nav.peek.approximate", "近似"),
    ("nav.hover.approximate", "近似结果"),
    ("nav.hover.loading", "正在加载…"),
    ("nav.hover.param", "参数 {name}：{text}"),
    ("nav.hover.returns", "返回：{text}"),
    ("nav.hover.exception", "异常 {name}：{text}"),
    (
        "nav.hover.longLine",
        "出于性能原因，未对长行进行词法分析。可以通过 `editor.maxTokenizationLineLength` 配置长行的长度。",
    ),
];

/// 悬停卡片与链接的颜色键（VS Code 颜色，默认值取 #10 的默认值表）。
pub const COLORS: &[&str] = &[
    "editorHoverWidget.background",
    "editorHoverWidget.foreground",
    "editorHoverWidget.border",
    "editorHoverWidget.statusBarBackground",
    "editorHoverWidget.highlightForeground",
    "editorLink.activeForeground",
    "descriptionForeground",
    "textCodeBlock.background",
    "widget.shadow",
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

/// 悬停卡片与链接的调色板；主题缺键时用默认值表。
#[derive(Clone, Debug, PartialEq)]
pub struct NavColors {
    pub hover_background: Option<Color>,
    pub hover_foreground: Option<Color>,
    pub hover_border: Option<Color>,
    pub hover_status_background: Option<Color>,
    pub hover_highlight: Option<Color>,
    pub link: Option<Color>,
    pub description: Option<Color>,
    pub code_background: Option<Color>,
    pub shadow: Option<Color>,
}

impl NavColors {
    pub fn resolve(theme: &dyn rove_theme::Theme, kind: rove_theme::ThemeKind) -> Self {
        let get = |key: &str| match theme.color(key) {
            Ok(c) => c,
            Err(_) => rove_theme::defaults::static_defaults(key).and_then(|d| d.for_kind(kind)),
        };
        Self {
            hover_background: get("editorHoverWidget.background"),
            hover_foreground: get("editorHoverWidget.foreground"),
            hover_border: get("editorHoverWidget.border"),
            hover_status_background: get("editorHoverWidget.statusBarBackground"),
            hover_highlight: get("editorHoverWidget.highlightForeground"),
            link: get("editorLink.activeForeground"),
            description: get("descriptionForeground"),
            code_background: get("textCodeBlock.background"),
            shadow: get("widget.shadow"),
        }
    }
}

/// 本模块命令的 GPUI action；由工作台的导航宿主处理，键位服务按 when 解析后分发。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = nav, no_json)]
pub struct NavAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(NavAction(cmd)) as Box<dyn Action>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_navigation_commands_strings_and_actions() {
        let mut r = CommandRegistry::default();
        register_commands(&mut r).unwrap();
        // 重复登记（其他消费方先登记了同一个 VS Code 命令）不报错。
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
        assert!(action_for("editor.action.goToReferences").is_none());
    }

    #[test]
    fn every_color_key_resolves_to_a_default() {
        let mut registry = ColorRegistry::default();
        register_colors(&mut registry).unwrap();
        for key in COLORS {
            assert!(registry.get(key).is_some(), "{key}");
        }
    }

    /// 默认键位表里的命令都已登记（否则键位服务会忽略这些键位）。
    #[test]
    fn default_keys_reach_registered_commands() {
        let mut r = CommandRegistry::default();
        register_commands(&mut r).unwrap();
        for id in [
            REVEAL_DEFINITION,
            REVEAL_DEFINITION_ASIDE,
            GOTO_IMPLEMENTATION,
            NEXT_SYMBOL,
            NEXT_SYMBOL_CANCEL,
            SHOW_HOVER,
            WORD_HIGHLIGHT_NEXT,
            WORD_HIGHLIGHT_PREV,
            PAGE_DOWN_HOVER,
        ] {
            assert!(r.get(id).is_some(), "{id}");
        }
    }
}
