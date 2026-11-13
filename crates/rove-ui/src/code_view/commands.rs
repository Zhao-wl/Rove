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
    // #20：折叠（KEYBINDINGS.md 阅读一节）与“在此会话中切换只读”。
    ("editor.fold", "codeView.command.fold", "折叠"),
    ("editor.unfold", "codeView.command.unfold", "展开"),
    (
        "editor.toggleFold",
        "codeView.command.toggleFold",
        "切换折叠",
    ),
    (
        "editor.foldRecursively",
        "codeView.command.foldRecursively",
        "以递归方式折叠",
    ),
    (
        "editor.unfoldRecursively",
        "codeView.command.unfoldRecursively",
        "以递归方式展开",
    ),
    (
        "editor.toggleFoldRecursively",
        "codeView.command.toggleFoldRecursively",
        "以递归方式切换折叠",
    ),
    ("editor.foldAll", "codeView.command.foldAll", "全部折叠"),
    ("editor.unfoldAll", "codeView.command.unfoldAll", "全部展开"),
    (
        "editor.foldLevel1",
        "codeView.command.foldLevel1",
        "折叠级别 1",
    ),
    (
        "editor.foldLevel2",
        "codeView.command.foldLevel2",
        "折叠级别 2",
    ),
    (
        "editor.foldLevel3",
        "codeView.command.foldLevel3",
        "折叠级别 3",
    ),
    (
        "editor.foldLevel4",
        "codeView.command.foldLevel4",
        "折叠级别 4",
    ),
    (
        "editor.foldLevel5",
        "codeView.command.foldLevel5",
        "折叠级别 5",
    ),
    (
        "editor.foldLevel6",
        "codeView.command.foldLevel6",
        "折叠级别 6",
    ),
    (
        "editor.foldLevel7",
        "codeView.command.foldLevel7",
        "折叠级别 7",
    ),
    (
        "editor.foldAllBlockComments",
        "codeView.command.foldAllBlockComments",
        "折叠所有块注释",
    ),
    (
        "editor.foldAllMarkerRegions",
        "codeView.command.foldAllMarkerRegions",
        "折叠所有区域",
    ),
    (
        "editor.unfoldAllMarkerRegions",
        "codeView.command.unfoldAllMarkerRegions",
        "展开所有区域",
    ),
    (
        "editor.foldAllExcept",
        "codeView.command.foldAllExcept",
        "折叠除所选区域之外的所有区域",
    ),
    (
        "editor.unfoldAllExcept",
        "codeView.command.unfoldAllExcept",
        "展开除所选区域之外的所有区域",
    ),
    (
        "editor.createFoldingRangeFromSelection",
        "codeView.command.createFoldingRangeFromSelection",
        "从所选内容创建折叠范围",
    ),
    (
        "editor.removeManualFoldingRanges",
        "codeView.command.removeManualFoldingRanges",
        "删除手动折叠范围",
    ),
    (
        "workbench.action.files.toggleActiveEditorReadonlyInSession",
        "codeView.command.toggleReadonlyInSession",
        "在会话中切换只读模式",
    ),
    // #47：编辑基础（KEYBINDINGS「编辑基础」）、剪切粘贴、改写模式、缩进转换。`type` / `default:type`
    // 带参数（`{ "text": … }`），没有 GPUI action，经命令桥交给窗口再转给聚焦的代码视图。
    (
        "lineBreakInsert",
        "codeView.command.lineBreakInsert",
        "插入换行符",
    ),
    ("tab", "codeView.command.tab", "Tab 键"),
    ("outdent", "codeView.command.outdent", "减少缩进"),
    (
        "editor.action.indentLines",
        "codeView.command.indentLines",
        "行缩进",
    ),
    (
        "editor.action.outdentLines",
        "codeView.command.outdentLines",
        "行减少缩进",
    ),
    ("deleteLeft", "codeView.command.deleteLeft", "向左删除"),
    ("deleteRight", "codeView.command.deleteRight", "向右删除"),
    (
        "deleteWordLeft",
        "codeView.command.deleteWordLeft",
        "向左删除单词",
    ),
    (
        "deleteWordRight",
        "codeView.command.deleteWordRight",
        "向右删除单词",
    ),
    (
        "deleteWordStartLeft",
        "codeView.command.deleteWordStartLeft",
        "向左删除到单词开头",
    ),
    (
        "deleteWordStartRight",
        "codeView.command.deleteWordStartRight",
        "向右删除到单词开头",
    ),
    (
        "deleteWordEndLeft",
        "codeView.command.deleteWordEndLeft",
        "向左删除到单词结尾",
    ),
    (
        "deleteWordEndRight",
        "codeView.command.deleteWordEndRight",
        "向右删除到单词结尾",
    ),
    (
        "deleteWordPartLeft",
        "codeView.command.deleteWordPartLeft",
        "向左删除部分单词",
    ),
    (
        "deleteWordPartRight",
        "codeView.command.deleteWordPartRight",
        "向右删除部分单词",
    ),
    (
        "deleteAllLeft",
        "codeView.command.deleteAllLeft",
        "删除左侧所有内容",
    ),
    (
        "deleteAllRight",
        "codeView.command.deleteAllRight",
        "删除右侧所有内容",
    ),
    (
        "cursorWordLeft",
        "codeView.command.cursorWordLeft",
        "光标移到上一个单词",
    ),
    (
        "cursorWordLeftSelect",
        "codeView.command.cursorWordLeftSelect",
        "选择到上一个单词",
    ),
    (
        "cursorWordRight",
        "codeView.command.cursorWordRight",
        "光标移到下一个单词",
    ),
    (
        "cursorWordRightSelect",
        "codeView.command.cursorWordRightSelect",
        "选择到下一个单词",
    ),
    (
        "cursorWordStartLeft",
        "codeView.command.cursorWordStartLeft",
        "光标移到上一个单词开头",
    ),
    (
        "cursorWordStartLeftSelect",
        "codeView.command.cursorWordStartLeftSelect",
        "选择到上一个单词开头",
    ),
    (
        "cursorWordStartRight",
        "codeView.command.cursorWordStartRight",
        "光标移到下一个单词开头",
    ),
    (
        "cursorWordStartRightSelect",
        "codeView.command.cursorWordStartRightSelect",
        "选择到下一个单词开头",
    ),
    (
        "cursorWordEndLeft",
        "codeView.command.cursorWordEndLeft",
        "光标移到上一个单词结尾",
    ),
    (
        "cursorWordEndLeftSelect",
        "codeView.command.cursorWordEndLeftSelect",
        "选择到上一个单词结尾",
    ),
    (
        "cursorWordEndRight",
        "codeView.command.cursorWordEndRight",
        "光标移到下一个单词结尾",
    ),
    (
        "cursorWordEndRightSelect",
        "codeView.command.cursorWordEndRightSelect",
        "选择到下一个单词结尾",
    ),
    (
        "cursorWordPartLeft",
        "codeView.command.cursorWordPartLeft",
        "光标移到上一个单词部分",
    ),
    (
        "cursorWordPartLeftSelect",
        "codeView.command.cursorWordPartLeftSelect",
        "选择到上一个单词部分",
    ),
    (
        "cursorWordPartStartLeft",
        "codeView.command.cursorWordPartStartLeft",
        "光标移到上一个单词部分开头",
    ),
    (
        "cursorWordPartStartLeftSelect",
        "codeView.command.cursorWordPartStartLeftSelect",
        "选择到上一个单词部分开头",
    ),
    (
        "cursorWordPartRight",
        "codeView.command.cursorWordPartRight",
        "光标移到下一个单词部分",
    ),
    (
        "cursorWordPartRightSelect",
        "codeView.command.cursorWordPartRightSelect",
        "选择到下一个单词部分",
    ),
    (
        "cursorWordAccessibilityLeft",
        "codeView.command.cursorWordAccessibilityLeft",
        "光标移到上一个单词（无障碍）",
    ),
    (
        "cursorWordAccessibilityLeftSelect",
        "codeView.command.cursorWordAccessibilityLeftSelect",
        "选择到上一个单词（无障碍）",
    ),
    (
        "cursorWordAccessibilityRight",
        "codeView.command.cursorWordAccessibilityRight",
        "光标移到下一个单词（无障碍）",
    ),
    (
        "cursorWordAccessibilityRightSelect",
        "codeView.command.cursorWordAccessibilityRightSelect",
        "选择到下一个单词（无障碍）",
    ),
    (
        "cursorLineStart",
        "codeView.command.cursorLineStart",
        "光标移到行的开头",
    ),
    (
        "cursorLineStartSelect",
        "codeView.command.cursorLineStartSelect",
        "选择到行的开头",
    ),
    (
        "cursorLineEnd",
        "codeView.command.cursorLineEnd",
        "光标移到行的末尾",
    ),
    (
        "cursorLineEndSelect",
        "codeView.command.cursorLineEndSelect",
        "选择到行的末尾",
    ),
    ("undo", "codeView.command.undo", "撤消"),
    ("redo", "codeView.command.redo", "恢复"),
    ("cursorUndo", "codeView.command.cursorUndo", "光标撤消"),
    ("cursorRedo", "codeView.command.cursorRedo", "光标恢复"),
    (
        "editor.action.setSelectionAnchor",
        "codeView.command.setSelectionAnchor",
        "设置选择定位点",
    ),
    (
        "editor.action.selectFromAnchorToCursor",
        "codeView.command.selectFromAnchorToCursor",
        "选择从定位点到光标",
    ),
    (
        "editor.action.goToSelectionAnchor",
        "codeView.command.goToSelectionAnchor",
        "转到选择定位点",
    ),
    (
        "editor.action.cancelSelectionAnchor",
        "codeView.command.cancelSelectionAnchor",
        "取消选择定位点",
    ),
    (
        "editor.action.insertLineBefore",
        "codeView.command.insertLineBefore",
        "在上面插入行",
    ),
    (
        "editor.action.insertLineAfter",
        "codeView.command.insertLineAfter",
        "在下面插入行",
    ),
    (
        "editor.action.clipboardCutAction",
        "codeView.command.cut",
        "剪切",
    ),
    (
        "editor.action.clipboardPasteAction",
        "codeView.command.paste",
        "粘贴",
    ),
    (
        "editor.action.toggleOvertypeInsertMode",
        "codeView.command.toggleOvertypeInsertMode",
        "切换插入/改写模式",
    ),
    (
        "editor.action.indentationToSpaces",
        "codeView.command.indentationToSpaces",
        "将缩进转换为空格",
    ),
    (
        "editor.action.indentationToTabs",
        "codeView.command.indentationToTabs",
        "将缩进转换为制表符",
    ),
    ("type", "codeView.command.type", "键入"),
    ("default:type", "codeView.command.defaultType", "键入"),
];

/// #20 的界面字符串：只读提示（UX_SPEC §3.4、§6：先说发生了什么，再说可以怎么做）与按级别关闭的功能名。
pub const STRINGS: &[(&str, &str)] = &[
    (
        "codeView.readonly.cannotEdit",
        "无法在只读编辑器中编辑。{reason}",
    ),
    (
        "codeView.readonly.cannotEditToggle",
        "无法在只读编辑器中编辑。{reason}可以用“在会话中切换只读模式”临时取消只读。",
    ),
    (
        "codeView.readonly.reason.hugeFile",
        "超大文件只能以只读方式打开。",
    ),
    (
        "codeView.readonly.reason.metadata",
        "这是元数据视图，内容由程序集生成。",
    ),
    (
        "codeView.readonly.reason.permissions",
        "文件在磁盘上是只读的。",
    ),
    (
        "codeView.readonly.reason.readonlyInclude",
        "文件匹配 files.readonlyInclude 规则。",
    ),
    (
        "codeView.readonly.reason.extraReadonlyInclude",
        "Unity 包缓存目录，修改会被覆盖。",
    ),
    ("codeView.readonly.reason.session", "已在此会话中设为只读。"),
    ("codeView.readonly.lock", "只读"),
    ("codeView.feature.occurrencesHighlight", "同名高亮"),
    ("codeView.feature.syntaxFolding", "基于语法的折叠"),
    (
        "codeView.feature.backgroundHighlighting",
        "语法高亮改为后台完成",
    ),
    ("codeView.feature.syntaxHighlighting", "语法高亮"),
    ("codeView.feature.folding", "折叠"),
];

/// 分类标题（命令面板里显示为“编辑器: …”）。
pub const CATEGORY_KEY: &str = "codeView.category";

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
    for (key, zh) in STRINGS {
        if table.get(key).is_none() {
            table.register(*key, Arc::from(*zh))?;
        }
    }
    Ok(())
}

// 命令 ID → GPUI action。默认键位来自 #14 生成的 VS Code 默认键位表，由键位服务（`crate::keys`）
// 按 when 子句解析后分发到这里；代码视图在 key context 上发布 `editorTextFocus` 等上下文键。
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
    Fold => "editor.fold",
    Unfold => "editor.unfold",
    ToggleFold => "editor.toggleFold",
    FoldRecursively => "editor.foldRecursively",
    UnfoldRecursively => "editor.unfoldRecursively",
    ToggleFoldRecursively => "editor.toggleFoldRecursively",
    FoldAll => "editor.foldAll",
    UnfoldAll => "editor.unfoldAll",
    FoldLevel1 => "editor.foldLevel1",
    FoldLevel2 => "editor.foldLevel2",
    FoldLevel3 => "editor.foldLevel3",
    FoldLevel4 => "editor.foldLevel4",
    FoldLevel5 => "editor.foldLevel5",
    FoldLevel6 => "editor.foldLevel6",
    FoldLevel7 => "editor.foldLevel7",
    FoldAllBlockComments => "editor.foldAllBlockComments",
    FoldAllMarkerRegions => "editor.foldAllMarkerRegions",
    UnfoldAllMarkerRegions => "editor.unfoldAllMarkerRegions",
    FoldAllExcept => "editor.foldAllExcept",
    UnfoldAllExcept => "editor.unfoldAllExcept",
    CreateFoldingRangeFromSelection => "editor.createFoldingRangeFromSelection",
    RemoveManualFoldingRanges => "editor.removeManualFoldingRanges",
    ToggleReadonlyInSession => "workbench.action.files.toggleActiveEditorReadonlyInSession",
    EditLineBreakInsert => "lineBreakInsert",
    EditTab => "tab",
    EditOutdent => "outdent",
    EditIndentLines => "editor.action.indentLines",
    EditOutdentLines => "editor.action.outdentLines",
    EditDeleteLeft => "deleteLeft",
    EditDeleteRight => "deleteRight",
    EditDeleteWordLeft => "deleteWordLeft",
    EditDeleteWordRight => "deleteWordRight",
    EditDeleteWordStartLeft => "deleteWordStartLeft",
    EditDeleteWordStartRight => "deleteWordStartRight",
    EditDeleteWordEndLeft => "deleteWordEndLeft",
    EditDeleteWordEndRight => "deleteWordEndRight",
    EditDeleteWordPartLeft => "deleteWordPartLeft",
    EditDeleteWordPartRight => "deleteWordPartRight",
    EditDeleteAllLeft => "deleteAllLeft",
    EditDeleteAllRight => "deleteAllRight",
    EditCursorWordLeft => "cursorWordLeft",
    EditCursorWordLeftSelect => "cursorWordLeftSelect",
    EditCursorWordRight => "cursorWordRight",
    EditCursorWordRightSelect => "cursorWordRightSelect",
    EditCursorWordStartLeft => "cursorWordStartLeft",
    EditCursorWordStartLeftSelect => "cursorWordStartLeftSelect",
    EditCursorWordStartRight => "cursorWordStartRight",
    EditCursorWordStartRightSelect => "cursorWordStartRightSelect",
    EditCursorWordEndLeft => "cursorWordEndLeft",
    EditCursorWordEndLeftSelect => "cursorWordEndLeftSelect",
    EditCursorWordEndRight => "cursorWordEndRight",
    EditCursorWordEndRightSelect => "cursorWordEndRightSelect",
    EditCursorWordPartLeft => "cursorWordPartLeft",
    EditCursorWordPartLeftSelect => "cursorWordPartLeftSelect",
    EditCursorWordPartStartLeft => "cursorWordPartStartLeft",
    EditCursorWordPartStartLeftSelect => "cursorWordPartStartLeftSelect",
    EditCursorWordPartRight => "cursorWordPartRight",
    EditCursorWordPartRightSelect => "cursorWordPartRightSelect",
    EditCursorWordAccessibilityLeft => "cursorWordAccessibilityLeft",
    EditCursorWordAccessibilityLeftSelect => "cursorWordAccessibilityLeftSelect",
    EditCursorWordAccessibilityRight => "cursorWordAccessibilityRight",
    EditCursorWordAccessibilityRightSelect => "cursorWordAccessibilityRightSelect",
    EditCursorLineStart => "cursorLineStart",
    EditCursorLineStartSelect => "cursorLineStartSelect",
    EditCursorLineEnd => "cursorLineEnd",
    EditCursorLineEndSelect => "cursorLineEndSelect",
    EditUndo => "undo",
    EditRedo => "redo",
    EditCursorUndo => "cursorUndo",
    EditCursorRedo => "cursorRedo",
    EditSetSelectionAnchor => "editor.action.setSelectionAnchor",
    EditSelectFromAnchorToCursor => "editor.action.selectFromAnchorToCursor",
    EditGoToSelectionAnchor => "editor.action.goToSelectionAnchor",
    EditCancelSelectionAnchor => "editor.action.cancelSelectionAnchor",
    EditInsertLineBefore => "editor.action.insertLineBefore",
    EditInsertLineAfter => "editor.action.insertLineAfter",
    EditCut => "editor.action.clipboardCutAction",
    EditPaste => "editor.action.clipboardPasteAction",
    EditToggleOvertypeInsertMode => "editor.action.toggleOvertypeInsertMode",
    EditIndentationToSpaces => "editor.action.indentationToSpaces",
    EditIndentationToTabs => "editor.action.indentationToTabs",
}

/// 键位上下文名，以及聚焦时发布的 VS Code 上下文键（只读代码视图）。
pub const KEY_CONTEXT: &str = "CodeView";
pub const FOCUS_CONTEXT_KEYS: &[&str] = &["editorTextFocus", "editorFocus", "textInputFocus"];
/// 只读时发布（#20：只读来自超大文件、元数据视图、磁盘只读属性与只读规则，可在会话中临时取消）。
pub const READONLY_CONTEXT_KEY: &str = "editorReadonly";
/// 折叠可用时发布（`editor.folding` 开启且文档级别允许折叠）。
pub const FOLDING_CONTEXT_KEY: &str = "foldingEnabled";
/// 有单词高亮时发布（#40 的 `editor.action.wordHighlight.next/prev` 依赖它）。
pub const WORD_HIGHLIGHTS_CONTEXT_KEY: &str = "hasWordHighlights";
/// #47：设置了选择定位点（`editor.action.setSelectionAnchor`）时发布。
pub const SELECTION_ANCHOR_CONTEXT_KEY: &str = "selectionAnchorSet";
/// #47：带参数、没有 GPUI action 的命令（命令桥 → 窗口 → 聚焦的代码视图）。
pub const TYPE_COMMANDS: &[&str] = &["type", "default:type"];

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
            assert!(
                action_for(id).is_some() || TYPE_COMMANDS.contains(id),
                "{id} 缺少 action"
            );
            assert_eq!(&*r.get(id).unwrap().title_key, *key);
            assert!(t.get(key).is_some());
        }
    }
}
