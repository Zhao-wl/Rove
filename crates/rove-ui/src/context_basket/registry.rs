use rove_base::Result;
use rove_i18n::StringTable;
use std::sync::Arc;

pub const ADD: &str = "rove.contextBasket.add";
pub const FOCUS: &str = "rove.contextBasket.focus";
pub const CONTAINER: &str = "rove.view.contextBasket";
pub const SECTION: &str = "rove.contextBasket.items";
pub const COPY_MARKDOWN: &str = "rove.contextBasket.copyAsMarkdown";
pub const COPY_REFERENCES: &str = "rove.contextBasket.copyAsReferences";
pub const CLEAR: &str = "rove.contextBasket.clear";

pub const CONTEXT_LINES: rove_config::Key<f64> =
    rove_config::Key::new("rove.contextBasket.contextLines");
pub const CLEAR_AFTER_COPY: rove_config::Key<bool> =
    rove_config::Key::new("rove.contextBasket.clearAfterCopy");
pub const REFERENCE_TEMPLATE: rove_config::Key<String> =
    rove_config::Key::new("rove.contextBasket.referenceTemplate");

pub fn register_settings(registry: &mut rove_config::SettingRegistry) -> Result<()> {
    for key in [CONTEXT_LINES.id, CLEAR_AFTER_COPY.id, REFERENCE_TEMPLATE.id] {
        let entry = rove_config::catalog::catalog()
            .get(key)
            .ok_or(rove_base::Error::NotFound)?;
        registry.register(key, entry.setting.clone())?;
    }
    Ok(())
}

pub fn register_strings(table: &mut StringTable) -> Result<()> {
    for (key, text) in [
        ("contextBasket.title", "上下文篮子"),
        ("contextBasket.introduction", "说明（可选）"),
        ("contextBasket.note", "备注"),
        ("contextBasket.editNote", "编辑备注"),
        ("contextBasket.remove", "删除"),
        (
            "contextBasket.empty",
            "选中代码后，右键选择“添加到上下文篮子”，或者使用快捷键。",
        ),
        (
            "contextBasket.stats",
            "{lines} 行 · {chars} 字符 · 约 {tokens} tokens",
        ),
        ("contextBasket.stale", "可能已过期"),
        ("contextBasket.missing", "文件已删除"),
        ("contextBasket.reselect", "请在文件中重新选择要收集的范围。"),
        ("contextBasket.copyMarkdown", "复制为 Markdown"),
        ("contextBasket.copyReferences", "复制为引用"),
        ("contextBasket.clear", "清空"),
        ("contextBasket.loading", "正在恢复上下文篮子。"),
        ("contextBasket.noFile", "请先打开工作区里的文本文件。"),
        (
            "contextBasket.sourceUnavailable",
            "无法读取所选文件或代码范围。",
        ),
        ("contextBasket.addWholeFile", "将整个文件加入上下文篮子"),
        (
            "contextBasket.selectionRequired",
            "超大文件只能加入选中的代码。",
        ),
        ("contextBasket.tooLong", "每段最多 10,000 行。"),
        ("contextBasket.noExportableEntries", "没有可复制的代码段。"),
        ("contextBasket.cleared", "已清空上下文篮子。"),
        (
            "contextBasket.changedInOtherWindow",
            "上下文篮子已在另一个窗口更新，请重试。",
        ),
        (
            "contextBasket.copied",
            "已复制 {count} 段内容（约 {tokens} tokens）",
        ),
        (
            "contextBasket.copiedFallback",
            "引用模板无效，已使用默认格式复制 {count} 段内容（约 {tokens} tokens）",
        ),
        ("contextBasket.added", "已添加到上下文篮子（共 {count} 段）"),
        (
            "contextBasket.addedCurrentLine",
            "未找到所在符号，已加入当前行（共 {count} 段）",
        ),
    ] {
        table.register(key, Arc::from(text))?;
    }
    Ok(())
}

pub fn register_contributions(
    registry: &mut crate::contributions::ContributionRegistry,
) -> Result<()> {
    use crate::contributions::{Contribution, Placement};
    use rove_i18n::Message;

    registry.register(
        CONTAINER,
        Contribution {
            title: Message::key("contextBasket.title"),
            placement: Placement::ActivityBar,
            order: 30,
            icon: Some("inbox".into()),
            command: Some(FOCUS.into()),
            when: None,
            factory_key: crate::shell::catalog::CONTAINER_FACTORY.into(),
        },
    )?;
    registry.register(
        SECTION,
        Contribution {
            title: Message::key("contextBasket.title"),
            placement: Placement::Sidebar {
                container: CONTAINER.into(),
            },
            order: 0,
            icon: None,
            command: None,
            when: None,
            factory_key: SECTION.into(),
        },
    )
}
