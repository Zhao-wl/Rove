//! Commands, status entries and in-memory formatted previews for R66.
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use gpui::Action;
use rove_base::{Result, Value};
use rove_commands::{Command, CommandRegistry, When};
use rove_i18n::{Message, StringTable};

use crate::{
    contributions::{Contribution, ContributionRegistry, Placement},
    keys::RunCommand,
    shell::catalog::STATUS_FACTORY,
    skeleton::text::{VirtualContent, VirtualSource},
};

pub(crate) const FOLLOW: &str = "rove.editor.followTail";
pub(crate) const FORMATTED: &str = "rove.editor.viewFormatted";
pub(crate) const RESUME: &str = "rove.editor.resumeTail";
pub(crate) const CANCEL_FORMAT: &str = "rove.editor.cancelFormattedPreview";
pub(crate) const FOLLOW_STATUS: &str = "rove.status.followTail";
pub(crate) const FORMAT_STATUS: &str = "rove.status.formattedPreview";
pub(crate) const FORMAT_LABEL: &str = "command.rove.editor.viewFormatted";
pub(crate) const READONLY_REASON: &str = "followTail.formattedReadonly";
pub(crate) const TITLE_ID: &str = "rove.editor.title.followTail";
pub(crate) const TITLE_FACTORY: &str = "followTail.editorTitle";

const STRINGS: &[(&str, &str)] = &[
    ("followTail.resume", "恢复跟随文件末尾"),
    ("followTail.active", "跟随末尾"),
    ("followTail.paused", "已暂停跟随"),
    ("followTail.tooltip", "点击恢复跟随并滚到文件末尾"),
    ("followTail.formatting", "正在格式化（点击取消）"),
    ("followTail.cancelFormat", "取消格式化预览"),
    (
        "followTail.formatTooLarge",
        "文件超过 {limit}，无法格式化后查看。",
    ),
    (
        "followTail.outputTooLarge",
        "格式化结果超过 {limit}，无法打开预览。",
    ),
    (
        "followTail.formatFailed",
        "无法格式化此文件，请检查文件语法。",
    ),
    ("followTail.formattedTitle", "{name}（格式化后）"),
    (READONLY_REASON, "这是格式化后的只读副本，原文件保留原样。"),
];

pub(crate) fn register_strings(table: &mut StringTable) -> Result<()> {
    for &(key, text) in STRINGS {
        table.register(key, text.into())?;
    }
    Ok(())
}

pub(crate) fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, title) in [
        (RESUME, "followTail.resume"),
        (CANCEL_FORMAT, "followTail.cancelFormat"),
    ] {
        registry.register(
            id,
            Command {
                title_key: title.into(),
                category_key: None,
            },
        )?;
    }
    Ok(())
}

pub(crate) fn register_contributions(registry: &mut ContributionRegistry) -> Result<()> {
    registry.register(
        TITLE_ID,
        Contribution {
            title: Message::key("command.rove.editor.followTail"),
            placement: Placement::EditorTitle,
            order: 10,
            icon: Some("arrow-down".into()),
            command: Some(FOLLOW.into()),
            when: Some(When("roveCanFollowTail".into())),
            factory_key: TITLE_FACTORY.into(),
        },
    )?;
    for (id, title, order, command, when) in [
        (
            FOLLOW_STATUS,
            "followTail.resume",
            93,
            RESUME,
            "roveFollowTailActive",
        ),
        (
            FORMAT_STATUS,
            "followTail.cancelFormat",
            92,
            CANCEL_FORMAT,
            "roveFormattedPreviewPending",
        ),
    ] {
        registry.register(
            id,
            Contribution {
                title: Message::key(title),
                placement: Placement::StatusRight,
                order,
                icon: None,
                command: Some(command.into()),
                when: Some(When(when.into())),
                factory_key: STATUS_FACTORY.into(),
            },
        )?;
    }
    Ok(())
}

pub(crate) fn action_for(id: &str) -> Option<Box<dyn Action>> {
    matches!(id, FOLLOW | FORMATTED | RESUME | CANCEL_FORMAT).then(|| {
        Box::new(RunCommand {
            id: id.to_string().into(),
            args: Value::Null,
        }) as Box<dyn Action>
    })
}

const FORMATTED_SCHEME: &str = "rove-formatted:";
static NEXT_PREVIEW: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub(crate) struct PreviewSource(Mutex<BTreeMap<Arc<str>, VirtualContent>>);

impl PreviewSource {
    pub(crate) fn prepare(&self, name: &str, content: VirtualContent) -> Arc<str> {
        let serial = NEXT_PREVIEW.fetch_add(1, Ordering::Relaxed);
        let resource: Arc<str> = format!("{FORMATTED_SCHEME}{serial}/{name}").into();
        self.0.lock().unwrap().insert(resource.clone(), content);
        resource
    }

    pub(crate) fn discard(&self, name: &str) {
        self.0.lock().unwrap().remove(name);
    }

    pub(crate) fn name(resource: &str) -> Option<&str> {
        let (_, name) = resource.strip_prefix(FORMATTED_SCHEME)?.split_once('/')?;
        Some(name)
    }
}

impl VirtualSource for PreviewSource {
    fn load(&self, name: &str) -> Option<Result<VirtualContent>> {
        self.0.lock().unwrap().remove(name).map(Ok)
    }
}
