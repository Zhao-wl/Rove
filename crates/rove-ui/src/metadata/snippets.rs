//! 速览预览里的外部符号片段（#41 `ExternalSnippets` 的提供方）：目标是 `rove-cs-metadata:<符号键>` 时，
//! 预览显示该符号所在类型的元数据视图，并定位到成员声明。

use crate::peek::external::{ExternalSnippet, ExternalSnippetProvider};
use rove_base::{Cancellation, Line, Position, TextRange, Utf8Column};
use rove_lang_cs::{
    metadata_view::{MetadataViews, VIEW_LANGUAGE},
    query::{Hover, Target},
    router::metadata_symbol_key,
};
use std::sync::Arc;

pub struct MetadataSnippets {
    views: Arc<MetadataViews>,
}

impl MetadataSnippets {
    pub fn new(views: Arc<MetadataViews>) -> Self {
        Self { views }
    }
}

impl ExternalSnippetProvider for MetadataSnippets {
    fn snippet(
        &self,
        target: &Target,
        _hover: Option<&Hover>,
        cancel: &Cancellation,
    ) -> Option<ExternalSnippet> {
        cancel.check().ok()?;
        let key = metadata_symbol_key(&target.resource)?;
        let location = self.views.locate(key).ok()?;
        let content = self.views.content(&location.type_key).ok()?;
        let start = Position {
            line: Line(u64::from(location.line)),
            column: Utf8Column(u64::from(location.col)),
        };
        let end = Position {
            line: start.line,
            column: Utf8Column(start.column.0 + u64::from(location.len)),
        };
        Some(ExternalSnippet {
            text: content.text.into(),
            language: VIEW_LANGUAGE.into(),
            focus: TextRange::new(start, end).ok(),
        })
    }
}
