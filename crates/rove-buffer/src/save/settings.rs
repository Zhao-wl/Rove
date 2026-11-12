//! 保存路径读取的 `files.*` 设置（#49）：登记（VS Code 默认值，已登记则跳过）与强类型读取。
//!
//! 只读，**绝不写回共用的设置文件**（`FEATURES.md` §13 规则 5）。`files.encoding` 由 #7 的
//! [`crate::settings`] 登记；只读规则（`files.readonly*`）由 #15 登记并判定，这里只消费判定结果
//! （文档元信息里的 [`ReadOnlyReason`](crate::ReadOnlyReason)）。

use std::sync::Arc;
use std::time::Duration;

use rove_base::{Result, Value};
use rove_config::{Scope, Setting, SettingRegistry, Settings};

use crate::contracts::Eol;

pub const EOL: &str = "files.eol";
pub const INSERT_FINAL_NEWLINE: &str = "files.insertFinalNewline";
pub const TRIM_FINAL_NEWLINES: &str = "files.trimFinalNewlines";
pub const TRIM_TRAILING_WHITESPACE: &str = "files.trimTrailingWhitespace";
pub const TRIM_TRAILING_WHITESPACE_IN_REGEX_AND_STRINGS: &str =
    "files.trimTrailingWhitespaceInRegexAndStrings";
pub const SAVE_CONFLICT_RESOLUTION: &str = "files.saveConflictResolution";
pub const AUTO_SAVE: &str = "files.autoSave";
pub const AUTO_SAVE_DELAY: &str = "files.autoSaveDelay";
pub const AUTO_SAVE_WORKSPACE_FILES_ONLY: &str = "files.autoSaveWorkspaceFilesOnly";
pub const AUTO_SAVE_WHEN_NO_ERRORS: &str = "files.autoSaveWhenNoErrors";

/// 本模块登记的键、VS Code 默认值与能否按语言覆盖（与 `settings-catalog.json` 一致）。
fn defaults() -> Vec<(&'static str, Value, bool)> {
    vec![
        (EOL, Value::String("auto".into()), true),
        (INSERT_FINAL_NEWLINE, Value::Bool(false), true),
        (TRIM_FINAL_NEWLINES, Value::Bool(false), true),
        (TRIM_TRAILING_WHITESPACE, Value::Bool(false), true),
        (
            TRIM_TRAILING_WHITESPACE_IN_REGEX_AND_STRINGS,
            Value::Bool(true),
            true,
        ),
        (
            SAVE_CONFLICT_RESOLUTION,
            Value::String("askUser".into()),
            false,
        ),
        (AUTO_SAVE, Value::String("off".into()), false),
        (AUTO_SAVE_DELAY, Value::Number(1000.0), false),
        (AUTO_SAVE_WORKSPACE_FILES_ONLY, Value::Bool(false), false),
        (AUTO_SAVE_WHEN_NO_ERRORS, Value::Bool(false), false),
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

/// `files.saveConflictResolution`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictResolution {
    AskUser,
    OverwriteFileOnDisk,
}

/// `files.autoSave`（`afterDelay` 带 `files.autoSaveDelay`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSave {
    Off,
    AfterDelay(Duration),
    OnFocusChange,
    OnWindowChange,
}

/// 触发自动保存的时机（由界面在对应时刻询问 [`SaveOptions::auto_save_due`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSaveTrigger {
    /// 最后一次编辑后经过了 `files.autoSaveDelay`。
    Delay,
    /// 编辑器失去焦点。
    FocusChange,
    /// 窗口失去焦点。
    WindowChange,
}

/// 保存路径的选项快照（按文档语言读取）。
#[derive(Clone, Debug, PartialEq)]
pub struct SaveOptions {
    /// `files.eol`：只用于新建文件；`None` 表示 `auto`（Windows 上是 CRLF）。
    pub eol: Option<Eol>,
    pub insert_final_newline: bool,
    pub trim_final_newlines: bool,
    pub trim_trailing_whitespace: bool,
    pub trim_trailing_whitespace_in_regex_and_strings: bool,
    pub conflict_resolution: ConflictResolution,
    pub auto_save: AutoSave,
    pub auto_save_workspace_files_only: bool,
    pub auto_save_when_no_errors: bool,
}

impl Default for SaveOptions {
    fn default() -> Self {
        Self {
            eol: None,
            insert_final_newline: false,
            trim_final_newlines: false,
            trim_trailing_whitespace: false,
            trim_trailing_whitespace_in_regex_and_strings: true,
            conflict_resolution: ConflictResolution::AskUser,
            auto_save: AutoSave::Off,
            auto_save_workspace_files_only: false,
            auto_save_when_no_errors: false,
        }
    }
}

impl SaveOptions {
    /// 经 #1 的设置接口读取；非法值回落到 VS Code 默认值。
    pub fn read(settings: &dyn Settings, language: &str) -> Self {
        let scope = Scope {
            language: (!language.is_empty()).then(|| Arc::<str>::from(language)),
        };
        let fallback = Self::default();
        let value = |key: &str| settings.read(key, &scope).ok();
        let boolean = |key: &str, default: bool| match value(key) {
            Some(Value::Bool(b)) => b,
            _ => default,
        };
        let string = |key: &str| match value(key) {
            Some(Value::String(s)) => Some(s),
            _ => None,
        };
        let delay = match value(AUTO_SAVE_DELAY) {
            Some(Value::Number(n)) if n.is_finite() && n >= 0.0 => n as u64,
            _ => 1000,
        };
        Self {
            eol: match string(EOL).as_deref() {
                Some("\n") => Some(Eol::Lf),
                Some("\r\n") => Some(Eol::CrLf),
                _ => None,
            },
            insert_final_newline: boolean(INSERT_FINAL_NEWLINE, fallback.insert_final_newline),
            trim_final_newlines: boolean(TRIM_FINAL_NEWLINES, fallback.trim_final_newlines),
            trim_trailing_whitespace: boolean(
                TRIM_TRAILING_WHITESPACE,
                fallback.trim_trailing_whitespace,
            ),
            trim_trailing_whitespace_in_regex_and_strings: boolean(
                TRIM_TRAILING_WHITESPACE_IN_REGEX_AND_STRINGS,
                fallback.trim_trailing_whitespace_in_regex_and_strings,
            ),
            conflict_resolution: match string(SAVE_CONFLICT_RESOLUTION).as_deref() {
                Some("overwriteFileOnDisk") => ConflictResolution::OverwriteFileOnDisk,
                _ => ConflictResolution::AskUser,
            },
            auto_save: match string(AUTO_SAVE).as_deref() {
                Some("afterDelay") => AutoSave::AfterDelay(Duration::from_millis(delay)),
                Some("onFocusChange") => AutoSave::OnFocusChange,
                Some("onWindowChange") => AutoSave::OnWindowChange,
                _ => AutoSave::Off,
            },
            auto_save_workspace_files_only: boolean(
                AUTO_SAVE_WORKSPACE_FILES_ONLY,
                fallback.auto_save_workspace_files_only,
            ),
            auto_save_when_no_errors: boolean(
                AUTO_SAVE_WHEN_NO_ERRORS,
                fallback.auto_save_when_no_errors,
            ),
        }
    }

    /// 新建文件的换行符：`files.eol`，`auto` 在 Windows 上是 CRLF、其他平台 LF（VS Code 相同）。
    pub fn new_file_eol(&self) -> Eol {
        self.eol
            .unwrap_or(if cfg!(windows) { Eol::CrLf } else { Eol::Lf })
    }

    /// 自动保存是否应在 `trigger` 时保存这个文档（VS Code `files.autoSave*` 的语义）：
    ///
    /// - `onFocusChange` 在编辑器或窗口失焦时都保存，`onWindowChange` 只在窗口失焦时保存；
    /// - 未命名文档从不自动保存；`autoSaveWorkspaceFilesOnly` 时工作区外的文件不自动保存；
    /// - `autoSaveWhenNoErrors` 时有错误级问题的文档不自动保存。没有问题提供方时 `has_errors`
    ///   传 `false`（视为“无问题”），R60 合并后由它提供。
    pub fn auto_save_due(
        &self,
        trigger: AutoSaveTrigger,
        untitled: bool,
        in_workspace: bool,
        has_errors: bool,
    ) -> bool {
        let mode_matches = matches!(
            (self.auto_save, trigger),
            (AutoSave::AfterDelay(_), AutoSaveTrigger::Delay)
                | (AutoSave::OnFocusChange, AutoSaveTrigger::FocusChange)
                | (AutoSave::OnFocusChange, AutoSaveTrigger::WindowChange)
                | (AutoSave::OnWindowChange, AutoSaveTrigger::WindowChange)
        );
        mode_matches
            && !untitled
            && (!self.auto_save_workspace_files_only || in_workspace)
            && (!self.auto_save_when_no_errors || !has_errors)
    }

    /// `afterDelay` 的延迟。
    pub fn auto_save_delay(&self) -> Option<Duration> {
        match self.auto_save {
            AutoSave::AfterDelay(delay) => Some(delay),
            _ => None,
        }
    }
}
