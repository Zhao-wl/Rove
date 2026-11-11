//! 代码视图的命令：VS Code 命令 ID、标题字符串登记，以及到 GPUI action 的临时桥接。

use rove_base::Result;
use rove_commands::{Command, CommandRegistry};
use rove_i18n::StringTable;
use std::sync::Arc;

/// (命令 ID, 标题字符串 key, 中文标题)。命令 ID 与 VS Code 一致。
pub const COMMANDS: &[(&str, &str, &str)] = &[
    (
        "scrollLineUp",
        "codeView.command.scrollLineUp",
        "向上滚动一行",
    ),
    (
        "scrollLineDown",
        "codeView.command.scrollLineDown",
        "向下滚动一行",
    ),
    (
        "scrollPageUp",
        "codeView.command.scrollPageUp",
        "向上滚动一页",
    ),
    (
        "scrollPageDown",
        "codeView.command.scrollPageDown",
        "向下滚动一页",
    ),
    ("cursorUp", "codeView.command.cursorUp", "光标上移"),
    (
        "cursorUpSelect",
        "codeView.command.cursorUpSelect",
        "向上扩展选择",
    ),
    ("cursorDown", "codeView.command.cursorDown", "光标下移"),
    (
        "cursorDownSelect",
        "codeView.command.cursorDownSelect",
        "向下扩展选择",
    ),
    ("cursorLeft", "codeView.command.cursorLeft", "光标左移"),
    (
        "cursorLeftSelect",
        "codeView.command.cursorLeftSelect",
        "向左扩展选择",
    ),
    ("cursorRight", "codeView.command.cursorRight", "光标右移"),
    (
        "cursorRightSelect",
        "codeView.command.cursorRightSelect",
        "向右扩展选择",
    ),
    ("cursorHome", "codeView.command.cursorHome", "光标移到行首"),
    (
        "cursorHomeSelect",
        "codeView.command.cursorHomeSelect",
        "选择到行首",
    ),
    ("cursorEnd", "codeView.command.cursorEnd", "光标移到行尾"),
    (
        "cursorEndSelect",
        "codeView.command.cursorEndSelect",
        "选择到行尾",
    ),
    (
        "cursorTop",
        "codeView.command.cursorTop",
        "光标移到文件开头",
    ),
    (
        "cursorTopSelect",
        "codeView.command.cursorTopSelect",
        "选择到文件开头",
    ),
    (
        "cursorBottom",
        "codeView.command.cursorBottom",
        "光标移到文件末尾",
    ),
    (
        "cursorBottomSelect",
        "codeView.command.cursorBottomSelect",
        "选择到文件末尾",
    ),
    (
        "cursorPageUp",
        "codeView.command.cursorPageUp",
        "光标上移一页",
    ),
    (
        "cursorPageUpSelect",
        "codeView.command.cursorPageUpSelect",
        "向上扩展选择一页",
    ),
    (
        "cursorPageDown",
        "codeView.command.cursorPageDown",
        "光标下移一页",
    ),
    (
        "cursorPageDownSelect",
        "codeView.command.cursorPageDownSelect",
        "向下扩展选择一页",
    ),
    (
        "editor.action.selectAll",
        "codeView.command.selectAll",
        "全选",
    ),
    (
        "expandLineSelection",
        "codeView.command.expandLineSelection",
        "展开行选择",
    ),
    (
        "cancelSelection",
        "codeView.command.cancelSelection",
        "取消选择",
    ),
    (
        "editor.action.clipboardCopyAction",
        "codeView.command.copy",
        "复制",
    ),
    (
        "editor.action.fontZoomIn",
        "codeView.command.fontZoomIn",
        "放大编辑器字体",
    ),
    (
        "editor.action.fontZoomOut",
        "codeView.command.fontZoomOut",
        "缩小编辑器字体",
    ),
    (
        "editor.action.fontZoomReset",
        "codeView.command.fontZoomReset",
        "重置编辑器字体大小",
    ),
];

/// 分类标题（命令面板里显示为“编辑器: …”）。
pub const CATEGORY_KEY: &str = "codeView.category";

/// 默认键位，取自 `docs/compat/KEYBINDINGS.md`（VS Code Windows 默认值）；GPUI 键名写法。
/// 字体缩放三个命令在 VS Code 中没有默认键位，这里也不分配。
pub const DEFAULT_KEYS: &[(&str, &str)] = &[
    ("ctrl-up", "scrollLineUp"),
    ("ctrl-down", "scrollLineDown"),
    ("alt-pageup", "scrollPageUp"),
    ("alt-pagedown", "scrollPageDown"),
    ("up", "cursorUp"),
    ("shift-up", "cursorUpSelect"),
    ("ctrl-shift-up", "cursorUpSelect"),
    ("down", "cursorDown"),
    ("shift-down", "cursorDownSelect"),
    ("ctrl-shift-down", "cursorDownSelect"),
    ("left", "cursorLeft"),
    ("shift-left", "cursorLeftSelect"),
    ("right", "cursorRight"),
    ("shift-right", "cursorRightSelect"),
    ("home", "cursorHome"),
    ("shift-home", "cursorHomeSelect"),
    ("end", "cursorEnd"),
    ("shift-end", "cursorEndSelect"),
    ("ctrl-home", "cursorTop"),
    ("ctrl-shift-home", "cursorTopSelect"),
    ("ctrl-end", "cursorBottom"),
    ("ctrl-shift-end", "cursorBottomSelect"),
    ("pageup", "cursorPageUp"),
    ("shift-pageup", "cursorPageUpSelect"),
    ("pagedown", "cursorPageDown"),
    ("shift-pagedown", "cursorPageDownSelect"),
    ("ctrl-a", "editor.action.selectAll"),
    ("ctrl-l", "expandLineSelection"),
    ("escape", "cancelSelection"),
    ("shift-escape", "cancelSelection"),
    ("ctrl-c", "editor.action.clipboardCopyAction"),
    ("ctrl-insert", "editor.action.clipboardCopyAction"),
];

/// 登记命令（已登记的 ID 跳过，便于 #14 先行登记全量 VS Code 命令）。
pub fn register(registry: &mut CommandRegistry) -> Result<()> {
    for (id, title, _) in COMMANDS {
        if registry.get(id).is_some() {
            continue;
        }
        registry.register(
            *id,
            Command {
                title_key: (*title).into(),
                category_key: Some(CATEGORY_KEY.into()),
            },
        )?;
    }
    Ok(())
}

/// 登记界面字符串（命令标题与分类）。
pub fn register_strings(table: &mut StringTable) -> Result<()> {
    if table.get(CATEGORY_KEY).is_none() {
        table.register(CATEGORY_KEY, Arc::from("编辑器"))?;
    }
    for (_, key, zh) in COMMANDS {
        if table.get(key).is_none() {
            table.register(*key, Arc::from(*zh))?;
        }
    }
    Ok(())
}

// SKELETON(commands): GPUI action 桥与上面的默认键位表是临时装配，#14 的 Keybindings/
// CommandExecutor 接管后改为按命令 ID 调用 `CodeView::execute`。
macro_rules! command_actions {
    ($($action:ident => $id:literal),* $(,)?) => {
        gpui::actions!(code_view, [$($action),*]);
        /// 命令 ID → GPUI action。
        pub fn action_for(id: &str) -> Option<Box<dyn gpui::Action>> {
            match id {
                $($id => Some(Box::new($action)),)*
                _ => None,
            }
        }
        fn binding_for(keys: &str, id: &str) -> Option<gpui::KeyBinding> {
            match id {
                $($id => Some(gpui::KeyBinding::new(keys, $action, Some(KEY_CONTEXT))),)*
                _ => None,
            }
        }
        /// 给代码视图的根元素挂上全部命令的 action 处理器。
        pub(super) fn on_actions(
            el: gpui::Div,
            cx: &mut gpui::Context<super::view::CodeView>,
        ) -> gpui::Div {
            use gpui::InteractiveElement;
            el$(.on_action(cx.listener(|v, _: &$action, window, cx| {
                v.execute($id, window, cx);
            })))*
        }
    };
}

command_actions! {
    ScrollLineUp => "scrollLineUp",
    ScrollLineDown => "scrollLineDown",
    ScrollPageUp => "scrollPageUp",
    ScrollPageDown => "scrollPageDown",
    CursorUp => "cursorUp",
    CursorUpSelect => "cursorUpSelect",
    CursorDown => "cursorDown",
    CursorDownSelect => "cursorDownSelect",
    CursorLeft => "cursorLeft",
    CursorLeftSelect => "cursorLeftSelect",
    CursorRight => "cursorRight",
    CursorRightSelect => "cursorRightSelect",
    CursorHome => "cursorHome",
    CursorHomeSelect => "cursorHomeSelect",
    CursorEnd => "cursorEnd",
    CursorEndSelect => "cursorEndSelect",
    CursorTop => "cursorTop",
    CursorTopSelect => "cursorTopSelect",
    CursorBottom => "cursorBottom",
    CursorBottomSelect => "cursorBottomSelect",
    CursorPageUp => "cursorPageUp",
    CursorPageUpSelect => "cursorPageUpSelect",
    CursorPageDown => "cursorPageDown",
    CursorPageDownSelect => "cursorPageDownSelect",
    SelectAll => "editor.action.selectAll",
    ExpandLineSelection => "expandLineSelection",
    CancelSelection => "cancelSelection",
    ClipboardCopy => "editor.action.clipboardCopyAction",
    FontZoomIn => "editor.action.fontZoomIn",
    FontZoomOut => "editor.action.fontZoomOut",
    FontZoomReset => "editor.action.fontZoomReset",
}

/// 键位上下文（对应 VS Code 的 `textInputFocus` / `editorTextFocus`）。
pub const KEY_CONTEXT: &str = "CodeView";

/// 把默认键位绑定到 GPUI（临时桥接，见上方 SKELETON 说明）。
pub fn default_key_bindings() -> Vec<gpui::KeyBinding> {
    DEFAULT_KEYS
        .iter()
        .filter_map(|(keys, id)| binding_for(keys, id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_action_title_and_string() {
        let mut r = CommandRegistry::default();
        register(&mut r).unwrap();
        register(&mut r).unwrap();
        let mut t = StringTable::default();
        register_strings(&mut t).unwrap();
        for (id, key, _) in COMMANDS {
            assert!(action_for(id).is_some(), "{id} 缺少 action");
            assert_eq!(&*r.get(id).unwrap().title_key, *key);
            assert!(t.get(key).is_some());
        }
        for (_, id) in DEFAULT_KEYS {
            assert!(COMMANDS.iter().any(|c| c.0 == *id), "{id} 未登记");
        }
        assert!(!DEFAULT_KEYS.iter().any(|(_, id)| id.contains("fontZoom")));
        assert_eq!(default_key_bindings().len(), DEFAULT_KEYS.len());
    }
}
