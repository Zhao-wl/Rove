//! 热退出读取的 `files.*` 设置（#50）：登记（VS Code 默认值，已登记则跳过）与强类型读取。
//!
//! 只读，**绝不写回共用的设置文件**（`FEATURES.md` §13 规则 5）。本 issue 不新增 `rove.*`
//! 设置项：备份目录、去抖间隔与撤销栈上限都是固定值（见 [`super`] 的模块文档）。

use rove_base::{Result, Value};
use rove_config::{Scope, Setting, SettingRegistry, Settings};

pub const HOT_EXIT: &str = "files.hotExit";
pub const RESTORE_UNDO_STACK: &str = "files.restoreUndoStack";

/// `files.hotExit`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HotExitMode {
    /// 不备份；关闭有未保存修改的标签 / 窗口时按 #49 的模态框询问。
    Off,
    /// 退出应用时备份并跳过询问；还有别的窗口开着时关单个窗口仍然询问。
    #[default]
    OnExit,
    /// 关闭任意窗口时都备份并跳过询问。
    OnExitAndWindowClose,
}

impl HotExitMode {
    /// 未知取值回落到 VS Code 的默认值 `onExit`。
    pub fn from_setting(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "onExitAndWindowClose" => Self::OnExitAndWindowClose,
            _ => Self::OnExit,
        }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::OnExit => "onExit",
            Self::OnExitAndWindowClose => "onExitAndWindowClose",
        }
    }

    /// 关闭窗口时是否备份并跳过 #49 的“保存 / 不保存 / 取消”。`last_window` 为真表示关掉它
    /// 就等于退出应用（Rove 一个窗口一个文件夹，所以“最后一个窗口”= 退出应用）。
    ///
    /// 关闭**单个标签**永远不走热退出：VS Code 同样只在窗口 / 应用级别跳过询问。
    pub fn skips_close_confirmation(self, last_window: bool) -> bool {
        match self {
            Self::Off => false,
            Self::OnExit => last_window,
            Self::OnExitAndWindowClose => true,
        }
    }

    /// 是否持续备份。`off` 时一份都不写。
    pub fn backs_up(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// 热退出的设置快照。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotExitOptions {
    pub mode: HotExitMode,
    /// `files.restoreUndoStack`：恢复文档时一并恢复撤销栈。
    pub restore_undo_stack: bool,
}

impl Default for HotExitOptions {
    fn default() -> Self {
        Self {
            mode: HotExitMode::default(),
            restore_undo_stack: true,
        }
    }
}

impl HotExitOptions {
    /// 经 #1 的设置接口读取；非法值回落到 VS Code 默认值。两个键都不按语言覆盖。
    pub fn read(settings: &dyn Settings) -> Self {
        let scope = Scope::default();
        let fallback = Self::default();
        Self {
            mode: match settings.read(HOT_EXIT, &scope) {
                Ok(Value::String(s)) => HotExitMode::from_setting(&s),
                _ => fallback.mode,
            },
            restore_undo_stack: match settings.read(RESTORE_UNDO_STACK, &scope) {
                Ok(Value::Bool(b)) => b,
                _ => fallback.restore_undo_stack,
            },
        }
    }
}

/// 登记本模块读取的键；已被其他模块登记的跳过。
pub fn register(registry: &mut SettingRegistry) -> Result<()> {
    for (key, default) in [
        (HOT_EXIT, Value::String("onExit".into())),
        (RESTORE_UNDO_STACK, Value::Bool(true)),
    ] {
        if registry.get(key).is_some() {
            continue;
        }
        registry.register(
            key,
            Setting {
                default,
                rove_default: None,
                language_overridable: false,
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_config::DefaultSettings;

    #[test]
    fn defaults_match_vs_code_and_unknown_values_fall_back() {
        let mut registry = SettingRegistry::default();
        register(&mut registry).unwrap();
        register(&mut registry).unwrap();
        let settings = DefaultSettings(registry);
        let options = HotExitOptions::read(&settings);
        assert_eq!(options.mode, HotExitMode::OnExit);
        assert!(options.restore_undo_stack);

        assert_eq!(HotExitMode::from_setting("off"), HotExitMode::Off);
        assert_eq!(
            HotExitMode::from_setting("onExitAndWindowClose"),
            HotExitMode::OnExitAndWindowClose
        );
        assert_eq!(HotExitMode::from_setting("nonsense"), HotExitMode::OnExit);
        for mode in [
            HotExitMode::Off,
            HotExitMode::OnExit,
            HotExitMode::OnExitAndWindowClose,
        ] {
            assert_eq!(HotExitMode::from_setting(mode.as_setting()), mode);
        }
    }

    #[test]
    fn each_mode_decides_backups_and_close_confirmation() {
        // off：不备份，关标签与关窗口都询问。
        assert!(!HotExitMode::Off.backs_up());
        assert!(!HotExitMode::Off.skips_close_confirmation(true));
        assert!(!HotExitMode::Off.skips_close_confirmation(false));
        // onExit：备份；退出应用（最后一个窗口）不询问，还有别的窗口时询问。
        assert!(HotExitMode::OnExit.backs_up());
        assert!(HotExitMode::OnExit.skips_close_confirmation(true));
        assert!(!HotExitMode::OnExit.skips_close_confirmation(false));
        // onExitAndWindowClose：关任意窗口都不询问。
        assert!(HotExitMode::OnExitAndWindowClose.backs_up());
        assert!(HotExitMode::OnExitAndWindowClose.skips_close_confirmation(true));
        assert!(HotExitMode::OnExitAndWindowClose.skips_close_confirmation(false));
    }
}
