//! 编辑核心读取的 `editor.*` 设置项：登记（VS Code 默认值）与强类型读取。
//!
//! 只登记键与默认值，不读写设置文件（#13）。同一个键可能已经被别的模块登记（例如代码视图的
//! `editor.tabSize`、`editor.wordSeparators`），那时跳过而不是报 Duplicate，默认值都取 VS Code 的。
//! 读取时非法值回退到默认值，并按 VS Code 的范围钳制。

use std::time::Duration;

use rove_base::{Result, Value};
use rove_config::{Scope, Setting, SettingRegistry, Settings};

use super::clipboard::MultiCursorPaste;
use super::history::UNDO_GROUP_TIMEOUT;
use super::word::DEFAULT_WORD_SEPARATORS;

pub const WORD_SEPARATORS: &str = "editor.wordSeparators";
pub const EMPTY_SELECTION_CLIPBOARD: &str = "editor.emptySelectionClipboard";
pub const MULTI_CURSOR_MERGE_OVERLAPPING: &str = "editor.multiCursorMergeOverlapping";
pub const MULTI_CURSOR_LIMIT: &str = "editor.multiCursorLimit";
pub const MULTI_CURSOR_PASTE: &str = "editor.multiCursorPaste";
pub const TAB_SIZE: &str = "editor.tabSize";
pub const INDENT_SIZE: &str = "editor.indentSize";
pub const WORD_SEGMENTER_LOCALES: &str = "editor.wordSegmenterLocales";

/// 本模块登记的键与 VS Code 默认值（`docs/compat/SETTINGS.md`）。
fn defaults() -> [(&'static str, Value, bool); 8] {
    [
        (
            WORD_SEPARATORS,
            Value::String(DEFAULT_WORD_SEPARATORS.into()),
            true,
        ),
        (EMPTY_SELECTION_CLIPBOARD, Value::Bool(true), false),
        (MULTI_CURSOR_MERGE_OVERLAPPING, Value::Bool(true), false),
        (MULTI_CURSOR_LIMIT, Value::Number(10000.0), false),
        (MULTI_CURSOR_PASTE, Value::String("spread".into()), false),
        (TAB_SIZE, Value::Number(4.0), true),
        (INDENT_SIZE, Value::String("tabSize".into()), true),
        (
            WORD_SEGMENTER_LOCALES,
            Value::Array(std::sync::Arc::from(Vec::<Value>::new())),
            true,
        ),
    ]
}

/// 登记本模块读取的键；已被其他模块登记的跳过。
pub fn register(registry: &mut SettingRegistry) -> Result<()> {
    for (key, default, language_overridable) in defaults() {
        if registry.get(key).is_some() {
            continue;
        }
        registry.register(
            key,
            Setting {
                default,
                rove_default: None,
                language_overridable,
            },
        )?;
    }
    Ok(())
}

/// 编辑核心的选项快照。
#[derive(Clone, Debug, PartialEq)]
pub struct EditOptions {
    pub word_separators: String,
    /// `editor.wordSegmenterLocales`（只有启用 `intl-segmenter` feature 时生效）。
    pub word_segmenter_locales: Vec<String>,
    pub empty_selection_clipboard: bool,
    pub multi_cursor_merge_overlapping: bool,
    pub multi_cursor_limit: usize,
    pub multi_cursor_paste: MultiCursorPaste,
    pub tab_size: u64,
    pub indent_size: u64,
    /// 撤销分组的超时（不是 VS Code 设置项，见 [`UNDO_GROUP_TIMEOUT`]）。
    pub undo_group_timeout: Duration,
}

impl Default for EditOptions {
    fn default() -> Self {
        Self {
            word_separators: DEFAULT_WORD_SEPARATORS.into(),
            word_segmenter_locales: Vec::new(),
            empty_selection_clipboard: true,
            multi_cursor_merge_overlapping: true,
            multi_cursor_limit: 10000,
            multi_cursor_paste: MultiCursorPaste::Spread,
            tab_size: 4,
            indent_size: 4,
            undo_group_timeout: UNDO_GROUP_TIMEOUT,
        }
    }
}

impl EditOptions {
    /// 经 #1 的设置接口读取（`scope.language` 决定语言级覆盖）。
    pub fn read(settings: &dyn Settings, scope: &Scope) -> Self {
        let fallback = Self::default();
        let value = |key: &str| settings.read(key, scope).ok();
        let boolean = |key: &str, default: bool| match value(key) {
            Some(Value::Bool(b)) => b,
            _ => default,
        };
        let number = |key: &str| match value(key) {
            Some(Value::Number(n)) if n.is_finite() => Some(n),
            _ => None,
        };
        let tab_size = number(TAB_SIZE).map_or(fallback.tab_size, |n| n.clamp(1.0, 100.0) as u64);
        Self {
            word_separators: match value(WORD_SEPARATORS) {
                Some(Value::String(s)) => s.to_string(),
                _ => fallback.word_separators,
            },
            // VS Code 接受字符串或字符串数组。
            word_segmenter_locales: match value(WORD_SEGMENTER_LOCALES) {
                Some(Value::String(s)) if !s.is_empty() => vec![s.to_string()],
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(|item| match item {
                        Value::String(s) if !s.is_empty() => Some(s.to_string()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
            empty_selection_clipboard: boolean(
                EMPTY_SELECTION_CLIPBOARD,
                fallback.empty_selection_clipboard,
            ),
            multi_cursor_merge_overlapping: boolean(
                MULTI_CURSOR_MERGE_OVERLAPPING,
                fallback.multi_cursor_merge_overlapping,
            ),
            multi_cursor_limit: number(MULTI_CURSOR_LIMIT)
                .map_or(fallback.multi_cursor_limit, |n| {
                    n.clamp(1.0, 100_000.0) as usize
                }),
            multi_cursor_paste: match value(MULTI_CURSOR_PASTE) {
                Some(Value::String(s)) if &*s == "full" => MultiCursorPaste::Full,
                _ => MultiCursorPaste::Spread,
            },
            tab_size,
            indent_size: number(INDENT_SIZE).map_or(tab_size, |n| n.clamp(1.0, 100.0) as u64),
            undo_group_timeout: fallback.undo_group_timeout,
        }
    }
}
