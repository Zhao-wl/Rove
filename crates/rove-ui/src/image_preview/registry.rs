use crate::contributions::{Contribution, ContributionRegistry, Placement};
use rove_base::Result;
use rove_commands::{Command, CommandRegistry};
use rove_i18n::{Message, StringTable};
use std::sync::Arc;

pub(crate) const EDITOR_ID: &str = "vscode.imagePreview";
pub(crate) const LABEL: &str = "imagePreview.title";
pub(crate) const SIZE: &str = "rove.status.imagePreview.size";
pub(crate) const ZOOM: &str = "rove.status.imagePreview.zoom";
pub(crate) const TOGGLE_ZOOM: &str = "rove.imagePreview.toggleFit";
pub(crate) const ASSOCIATIONS: &[&str] = &["*.png", "*.jpg", "*.jpeg", "*.gif", "*.svg", "*.tga"];

pub(crate) fn register_strings(table: &mut StringTable) -> Result<()> {
    for (key, text) in [
        (LABEL, "图片预览"),
        ("imagePreview.invalid", "图片损坏或内容与扩展名不符"),
        ("imagePreview.unavailable", "无法读取图片文件"),
        ("imagePreview.rendering", "无法为图片分配显示资源"),
        ("imagePreview.tooLarge", "图片尺寸或解码数据超过预览上限"),
        ("imagePreview.blocked", "已阻止 {count} 个远程资源"),
        ("imagePreview.size", "图片尺寸"),
        ("imagePreview.fit", "适应窗口"),
        ("imagePreview.zoom", "切换图片缩放"),
    ] {
        table.register(key, Arc::from(text))?;
    }
    Ok(())
}

pub(crate) fn register_contributions(registry: &mut ContributionRegistry) -> Result<()> {
    for (id, key, order, command) in [
        (SIZE, "imagePreview.size", 100, None),
        (ZOOM, "imagePreview.zoom", 90, Some(TOGGLE_ZOOM.into())),
    ] {
        registry.register(
            id,
            Contribution {
                title: Message::key(key),
                placement: Placement::StatusRight,
                order,
                icon: None,
                command,
                when: None,
                factory_key: crate::shell::catalog::STATUS_FACTORY.into(),
            },
        )?;
    }
    Ok(())
}

pub(crate) fn register_command(registry: &mut CommandRegistry) -> Result<()> {
    registry.register(
        TOGGLE_ZOOM,
        Command {
            title_key: "imagePreview.zoom".into(),
            category_key: None,
        },
    )
}
