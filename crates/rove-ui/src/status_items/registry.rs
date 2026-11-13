//! #27 的登记项：状态栏条目、命令（VS Code ID）、界面字符串与 GPUI action。
//!
//! - 条目经 #1 的登记机制加入（键名、位置 `StatusRight`、优先级、默认命令），绘制用 #16 的
//!   通用工厂 [`STATUS_FACTORY`]；运行期内容由 `StatusBar::set` 提供，没有内容的条目不显示；
//! - 设置项不在这里登记：`editor.tabSize` / `insertSpaces` / `detectIndentation`、`files.encoding`
//!   / `files.candidateGuessEncodings` 都已由代码视图、编辑核心、文本缓冲或 `compat/SETTINGS.md`
//!   的内置目录登记，本模块只读；
//! - 默认键位同样不在这里：`ctrl+k m`（切换语言模式）、`ctrl+g`（转到行）来自 #14 生成的 VS Code
//!   默认键位表；
//! - `workbench.action.gotoLine` 由 #24 登记、`workbench.action.editor.changeEOL` 由 #49 登记，
//!   本模块只引用它们的 ID，不重复登记（重复时跳过）。

use crate::{
    contributions::{Contribution, ContributionRegistry, Placement},
    shell::catalog::STATUS_FACTORY,
};
use gpui::Action;
use rove_base::{Error, Result};
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use std::sync::Arc;

// ------------------------------------------------------------------ 条目 ID 与顺序

/// 状态栏条目 ID（`StatusRight`）。`order` 数值越大越靠左，顺序与 VS Code 一致：
/// 行列 → 缩进 → 编码 → 换行符 → 语言 → 只读/超大文件 →（通知铃铛，`i32::MIN`）。
pub const CURSOR: &str = "rove.status.cursor";
pub const INDENT: &str = "rove.status.indentation";
pub const ENCODING: &str = "rove.status.encoding";
pub const EOL: &str = "rove.status.eol";
pub const LANGUAGE: &str = "rove.status.language";
/// 只读与超大文件是同一个条目的两种状态（#22 只提供数据，不另外登记条目）。
pub const READ_ONLY: &str = "rove.status.readOnly";

/// 右侧条目的登记顺序，供装配与测试使用。
pub const ITEMS: [(&str, &str, i32, Option<&str>); 6] = [
    (CURSOR, "status.cursor.name", 100, Some(GOTO_LINE)),
    (INDENT, "status.indent.name", 90, Some(CHANGE_INDENTATION)),
    (ENCODING, "status.encoding.name", 80, Some(CHANGE_ENCODING)),
    (EOL, "status.eol.name", 70, Some(CHANGE_EOL)),
    (
        LANGUAGE,
        "status.language.name",
        60,
        Some(CHANGE_LANGUAGE_MODE),
    ),
    (READ_ONLY, "status.readOnly.name", 50, None),
];

// ------------------------------------------------------------------ 命令 ID

/// #24 登记（转到行/列）；这里只作为光标位置条目的点击目标。
pub const GOTO_LINE: &str = "workbench.action.gotoLine";
/// #49 登记（更改行尾序列）；带参数 `"\n"` / `"\r\n"` 时直接统一，不带参数时打开选择器。
pub const CHANGE_EOL: &str = "workbench.action.editor.changeEOL";

pub const CHANGE_INDENTATION: &str = "changeEditorIndentation";
pub const INDENT_USING_SPACES: &str = "editor.action.indentUsingSpaces";
pub const INDENT_USING_TABS: &str = "editor.action.indentUsingTabs";
pub const DETECT_INDENTATION: &str = "editor.action.detectIndentation";
pub const CHANGE_ENCODING: &str = "workbench.action.editor.changeEncoding";
pub const CHANGE_LANGUAGE_MODE: &str = "workbench.action.editor.changeLanguageMode";

/// 本模块登记的命令：(ID, 标题字符串 key, 中文标题)。
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (
        CHANGE_INDENTATION,
        "status.command.changeIndentation",
        "更改缩进",
    ),
    (
        INDENT_USING_SPACES,
        "status.command.indentUsingSpaces",
        "使用空格缩进",
    ),
    (
        INDENT_USING_TABS,
        "status.command.indentUsingTabs",
        "使用制表符缩进",
    ),
    (
        DETECT_INDENTATION,
        "status.command.detectIndentation",
        "从内容中检测缩进",
    ),
    (
        CHANGE_ENCODING,
        "status.command.changeEncoding",
        "更改文件编码",
    ),
    (
        CHANGE_LANGUAGE_MODE,
        "status.command.changeLanguageMode",
        "更改语言模式",
    ),
];

pub const CATEGORY_KEY: &str = "status.category";

/// 本模块处理的命令（窗口装配按它分流；`CHANGE_EOL` 的无参形式也归这里）。
pub fn is_status_command(id: &str) -> bool {
    id == CHANGE_EOL || COMMANDS.iter().any(|(command, _, _)| *command == id)
}

// ------------------------------------------------------------------ 登记

pub fn register_contributions(r: &mut ContributionRegistry) -> Result<()> {
    for (id, title, order, command) in ITEMS {
        r.register(
            id,
            Contribution {
                title: rove_i18n::Message::key(title),
                placement: Placement::StatusRight,
                order,
                icon: None,
                command: command.map(Arc::from),
                when: None,
                factory_key: STATUS_FACTORY.into(),
            },
        )?;
    }
    Ok(())
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

/// 界面字符串（UX_SPEC §1.5、§3.4–§3.6；文字与 VS Code 中文版一致）。
pub const STRINGS: &[(&str, &str)] = &[
    (CATEGORY_KEY, "编辑器"),
    // 条目的无障碍名称（悬停提示缺省值）。
    ("status.cursor.name", "编辑器中的行与列"),
    ("status.indent.name", "编辑器缩进"),
    ("status.encoding.name", "编辑器编码"),
    ("status.eol.name", "编辑器行尾序列"),
    ("status.language.name", "编辑器语言模式"),
    ("status.readOnly.name", "文件状态"),
    // 光标位置。
    ("status.cursor.position", "行 {line}，列 {column}"),
    (
        "status.cursor.positionSelected",
        "行 {line}，列 {column}（已选择 {count}）",
    ),
    ("status.cursor.selections", "{count} 个选择"),
    (
        "status.cursor.selectionsCharacters",
        "{count} 个选择（已选择 {characters} 个字符）",
    ),
    ("status.cursor.tooltip", "转到行/列"),
    // 缩进。
    ("status.indent.spaces", "空格: {size}"),
    ("status.indent.tabs", "制表符长度: {size}"),
    ("status.indent.tooltip", "选择缩进"),
    ("status.indent.detected", "缩进由文件内容检测得出。"),
    ("status.indent.pick.useSpaces", "使用空格缩进"),
    ("status.indent.pick.useTabs", "使用制表符缩进"),
    ("status.indent.pick.detect", "从内容中检测缩进"),
    ("status.indent.pick.toSpaces", "将缩进转换为空格"),
    ("status.indent.pick.toTabs", "将缩进转换为制表符"),
    ("status.indent.pick.size", "{size}"),
    ("status.indent.placeholder", "选择此文件的缩进方式"),
    ("status.indent.group.view", "更改此文件的缩进视图"),
    ("status.indent.group.convert", "转换文件内容"),
    // 编码。
    ("status.encoding.tooltip", "选择编码"),
    (
        "status.encoding.tooltipGuessed",
        "选择编码（编码为自动识别）",
    ),
    ("status.encoding.bomSuffix", "{name} with BOM"),
    ("status.encoding.placeholder", "选择文件编码"),
    ("status.encoding.reopen", "通过编码重新打开"),
    ("status.encoding.save", "通过编码保存"),
    (
        "status.encoding.reopenPlaceholder",
        "选择要用来重新打开的编码",
    ),
    ("status.encoding.group.configured", "已配置"),
    ("status.encoding.group.guessed", "候选编码"),
    ("status.encoding.group.common", "常用编码"),
    ("status.encoding.reopened", "已用 {name} 重新打开"),
    (
        "status.encoding.reopenFailed",
        "无法用 {name} 重新打开该文件。",
    ),
    // 换行符。
    ("status.eol.mixed", "混合"),
    ("status.eol.tooltip", "选择行尾序列"),
    (
        "status.eol.tooltipMixed",
        "文件中同时存在 LF 与 CRLF，已保持原样。",
    ),
    ("status.eol.placeholder", "选择行尾序列"),
    ("status.eol.toLf", "统一为 LF"),
    ("status.eol.toCrlf", "统一为 CRLF"),
    // 语言模式。
    ("status.language.tooltip", "选择语言模式"),
    ("status.language.placeholder", "选择文件的语言模式"),
    ("status.language.csharp", "C#"),
    ("status.language.json", "JSON"),
    ("status.language.jsonc", "带注释的 JSON"),
    ("status.language.markdown", "Markdown"),
    ("status.language.xml", "XML"),
    ("status.language.yaml", "YAML"),
    ("status.language.plaintext", "纯文本"),
    // 只读与超大文件。
    ("status.readOnly.text", "只读"),
    ("status.readOnly.hugeText", "超大文件 · 只读"),
    ("status.readOnly.disabled", "已关闭的功能：{features}"),
    ("status.readOnly.separator", "、"),
    // 按文件大小关闭的功能名（清单与顺序来自 #8 的 `rove_buffer::disabled_features`）。
    ("status.feature.minimap", "缩略图"),
    ("status.feature.stickyScroll", "粘性滚动"),
    ("status.feature.bracketPairColorization", "括号对着色"),
    ("status.feature.occurrencesHighlight", "同名高亮"),
    ("status.feature.semanticHighlighting", "语义高亮"),
    ("status.feature.syntaxFolding", "基于语法的折叠"),
    ("status.feature.syntaxHighlighting", "语法高亮"),
    ("status.feature.folding", "折叠"),
    ("status.feature.wordWrap", "软换行"),
    ("status.feature.replace", "替换"),
];

pub fn register_strings(table: &mut StringTable) -> Result<()> {
    let commands = COMMANDS.iter().map(|(_, key, title)| (*key, *title));
    for (key, text) in STRINGS.iter().copied().chain(commands) {
        match table.register(key, Arc::from(text)) {
            Ok(()) | Err(Error::Duplicate(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 本模块命令的 GPUI action（键位服务按命令 ID 分发；窗口在 `status_host` 里处理）。
#[derive(Clone, PartialEq, Action)]
#[action(namespace = status, no_json)]
pub struct StatusAction(pub &'static str);

pub fn action_for(id: &str) -> Option<Box<dyn Action>> {
    COMMANDS
        .iter()
        .find(|(command, _, _)| *command == id)
        .map(|(command, _, _)| Box::new(StatusAction(command)) as Box<dyn Action>)
}
