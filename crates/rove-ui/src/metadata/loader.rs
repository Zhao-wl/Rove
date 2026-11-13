//! 元数据视图的文档加载：类型键 → 只读文档内容。

use crate::skeleton::text::{VirtualContent, VirtualSource};
use rove_base::{Error, Resource, Result};
use rove_buffer::ReadOnlyReason;
use rove_lang_cs::metadata_view::{view_key, MetadataViews, ViewError, VIEW_LANGUAGE, VIEW_SCHEME};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// 一份已加载视图的标签信息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewInfo {
    /// 类型名，泛型带类型参数（`List<T>`）。
    pub title: String,
    /// 程序集显示名（含版本）。
    pub assembly: String,
    /// DLL 完整路径。
    pub dll_path: Option<String>,
}

pub struct MetadataLoader {
    views: Arc<MetadataViews>,
    infos: Mutex<HashMap<String, ViewInfo>>,
}

impl MetadataLoader {
    pub fn new(views: Arc<MetadataViews>) -> Self {
        Self {
            views,
            infos: Mutex::default(),
        }
    }

    pub fn views(&self) -> &Arc<MetadataViews> {
        &self.views
    }

    /// 标签标题与悬停提示用的信息（视图加载过才有）。
    pub fn info(&self, resource: &Resource) -> Option<ViewInfo> {
        let key = view_key(resource)?;
        self.infos
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }
}

impl VirtualSource for MetadataLoader {
    fn load(&self, name: &str) -> Option<Result<VirtualContent>> {
        let key = name.strip_prefix(VIEW_SCHEME)?;
        Some(match self.views.content(key) {
            Ok(content) => {
                self.infos.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    key.to_owned(),
                    ViewInfo {
                        title: content.title,
                        assembly: content.assembly,
                        dll_path: content.dll_path,
                    },
                );
                Ok(VirtualContent {
                    text: content.text,
                    language: VIEW_LANGUAGE.into(),
                    // UX_SPEC §3.4：只读来源之一；“在此会话中切换只读”对它无效（宿主不放行 Metadata 原因）。
                    read_only: vec![ReadOnlyReason::Metadata],
                })
            }
            Err(ViewError::NotReady) => Err(Error::NotReady),
            Err(ViewError::NotFound) => Err(Error::NotFound),
        })
    }
}
