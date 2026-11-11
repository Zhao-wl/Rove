//! 大纲、面包屑与转到符号的登记项：命令（VS Code ID）、界面字符串、视图小节。
//! 设置项（`outline.*`、`breadcrumbs.*`）由 `rove_config::register_builtin` 按 compat/SETTINGS.md 登记；
//! 颜色键（`symbolIcon.*`、`breadcrumb*`）是 #10 主题服务登记的 P0 键；默认键位来自 #14 生成的
//! VS Code 默认键位表（`ctrl+shift+o`、`ctrl+shift+;`、`ctrl+shift+.` 等），这里都不重复登记。
use crate::contributions::{Contribution, ContributionRegistry, Placement};
use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::{Message, StringTable};
use std::sync::Arc;

/// 大纲小节的视图 ID 与工厂 key（与 VS Code 的视图 ID 相同）。
pub const OUTLINE_VIEW: &str = "outline";

pub const GOTO_SYMBOL: &str = "workbench.action.gotoSymbol";
pub const FOCUS: &str = "breadcrumbs.focus";
pub const FOCUS_AND_SELECT: &str = "breadcrumbs.focusAndSelect";
pub const FOCUS_NEXT: &str = "breadcrumbs.focusNext";
pub const FOCUS_PREVIOUS: &str = "breadcrumbs.focusPrevious";
pub const FOCUS_NEXT_WITH_PICKER: &str = "breadcrumbs.focusNextWithPicker";
pub const FOCUS_PREVIOUS_WITH_PICKER: &str = "breadcrumbs.focusPreviousWithPicker";
pub const REVEAL_FOCUSED: &str = "breadcrumbs.revealFocused";
pub const REVEAL_FOCUSED_ASIDE: &str = "breadcrumbs.revealFocusedFromTreeAside";
pub const SELECT_FOCUSED: &str = "breadcrumbs.selectFocused";
pub const SELECT_EDITOR: &str = "breadcrumbs.selectEditor";
pub const TOGGLE_TO_ON: &str = "breadcrumbs.toggleToOn";
pub const TOGGLE: &str = "breadcrumbs.toggle";
pub const COPY_PATH: &str = "breadcrumbs.copyPath";
pub const OUTLINE_FOCUS: &str = "outline.focus";
pub const OUTLINE_COLLAPSE: &str = "outline.collapse";
pub const OUTLINE_FOLLOW_CURSOR: &str = "outline.followCursor";

/// (命令 ID, 标题字符串 key, 中文标题)
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (
        GOTO_SYMBOL,
        "outline.command.gotoSymbol",
        "转到编辑器中的符号...",
    ),
    (FOCUS, "breadcrumbs.command.focus", "聚焦导航路径"),
    (
        FOCUS_AND_SELECT,
        "breadcrumbs.command.focusAndSelect",
        "聚焦并选择导航路径",
    ),
    (
        FOCUS_NEXT,
        "breadcrumbs.command.focusNext",
        "聚焦下一个导航路径段",
    ),
    (
        FOCUS_PREVIOUS,
        "breadcrumbs.command.focusPrevious",
        "聚焦上一个导航路径段",
    ),
    (
        FOCUS_NEXT_WITH_PICKER,
        "breadcrumbs.command.focusNextWithPicker",
        "聚焦并打开下一个导航路径段",
    ),
    (
        FOCUS_PREVIOUS_WITH_PICKER,
        "breadcrumbs.command.focusPreviousWithPicker",
        "聚焦并打开上一个导航路径段",
    ),
    (
        REVEAL_FOCUSED,
        "breadcrumbs.command.revealFocused",
        "显示聚焦的导航路径段",
    ),
    (
        REVEAL_FOCUSED_ASIDE,
        "breadcrumbs.command.revealFocusedFromTreeAside",
        "在侧边打开聚焦的导航路径项",
    ),
    (
        SELECT_FOCUSED,
        "breadcrumbs.command.selectFocused",
        "打开聚焦的导航路径段",
    ),
    (
        SELECT_EDITOR,
        "breadcrumbs.command.selectEditor",
        "离开导航路径",
    ),
    (
        TOGGLE_TO_ON,
        "breadcrumbs.command.toggleToOn",
        "启用导航路径",
    ),
    (TOGGLE, "breadcrumbs.command.toggle", "切换导航路径"),
    (COPY_PATH, "breadcrumbs.command.copyPath", "复制导航路径"),
    (OUTLINE_FOCUS, "outline.command.focus", "聚焦到大纲视图"),
    (OUTLINE_COLLAPSE, "outline.command.collapse", "全部折叠"),
    (
        OUTLINE_FOLLOW_CURSOR,
        "outline.command.followCursor",
        "跟随光标",
    ),
];

/// 界面字符串（UX_SPEC §6.3 的空状态等）。
pub const STRINGS: &[(&str, &str)] = &[
    ("view.outline", "大纲"),
    ("outline.empty", "活动编辑器无法提供大纲信息。"),
    ("outline.noEditor", "没有可提供大纲信息的已打开编辑器。"),
    ("outline.tooLarge", "超大文件不提供大纲信息。"),
    ("outline.loading", "正在加载文档符号…"),
    ("outline.noSymbols", "文档中没有符号。"),
    ("outline.truncated", "符号太多，只显示前 {count} 个。"),
    ("outline.document", "文档 {index}"),
    ("outline.followCursorOn", "跟随光标：开"),
    ("outline.followCursorOff", "跟随光标：关"),
    ("gotoSymbol.placeholder", "键入要转到的符号的名称。"),
    ("gotoSymbol.noSymbols", "活动文本编辑器中没有符号"),
    (
        "gotoSymbol.noEditor",
        "若要转到符号，请先打开具有符号信息的文本编辑器。",
    ),
    ("gotoSymbol.noResults", "没有匹配的编辑器符号"),
    ("gotoSymbol.group", "{kind} ({count})"),
    ("breadcrumbs.empty", "无符号"),
    ("symbolKind.file", "文件"),
    ("symbolKind.module", "模块"),
    ("symbolKind.namespace", "命名空间"),
    ("symbolKind.package", "包"),
    ("symbolKind.class", "类"),
    ("symbolKind.method", "方法"),
    ("symbolKind.property", "属性"),
    ("symbolKind.field", "字段"),
    ("symbolKind.constructor", "构造函数"),
    ("symbolKind.enum", "枚举"),
    ("symbolKind.interface", "接口"),
    ("symbolKind.function", "函数"),
    ("symbolKind.variable", "变量"),
    ("symbolKind.constant", "常量"),
    ("symbolKind.string", "字符串"),
    ("symbolKind.number", "数字"),
    ("symbolKind.boolean", "布尔值"),
    ("symbolKind.array", "数组"),
    ("symbolKind.object", "对象"),
    ("symbolKind.key", "键"),
    ("symbolKind.null", "null"),
    ("symbolKind.enumMember", "枚举成员"),
    ("symbolKind.struct", "结构"),
    ("symbolKind.event", "事件"),
    ("symbolKind.operator", "运算符"),
    ("symbolKind.typeParameter", "类型参数"),
    ("symbolKind.parameter", "参数"),
    ("symbolKind.document", "文档"),
    ("symbolKind.other", "其他"),
];

pub fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, title, _) in COMMANDS {
        match registry.register(
            *id,
            Command {
                title_key: (*title).into(),
                category_key: None,
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

/// 资源管理器中的“大纲”小节：顺序 20（打开的编辑器 0、文件夹 10，UX_SPEC §1.3）。
pub fn register_contributions(registry: &mut ContributionRegistry) -> Result<()> {
    registry.register(
        OUTLINE_VIEW,
        Contribution {
            title: Message::key("view.outline"),
            placement: Placement::Sidebar {
                container: crate::shell::catalog::EXPLORER.into(),
            },
            order: 20,
            icon: None,
            command: Some(OUTLINE_FOCUS.into()),
            when: None,
            factory_key: OUTLINE_VIEW.into(),
        },
    )
}

/// 本模块命令的 GPUI action；由窗口（宿主）处理，键位服务按 when 解析后分发。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = outline, no_json)]
pub struct OutlineAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(cmd, _, _)| *cmd == id)
        .map(|(cmd, _, _)| Box::new(OutlineAction(cmd)) as Box<dyn Action>)
}
