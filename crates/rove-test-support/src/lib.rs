//! 测试环境辅助（issue #3）。
//!
//! 需要真实 Unity 项目的测试，项目路径通过环境变量指定（约定见 `AGENTS.md` §8）：
//!
//! | 环境变量 | 项目 |
//! |---|---|
//! | `ROVE_TEST_UNITY_MEDIUM` | 中型 Unity 项目根目录 |
//! | `ROVE_TEST_UNITY_LARGE` | 大型 Unity 项目根目录 |
//!
//! 规则：
//!
//! - **具体路径不写进仓库**；
//! - 变量未设置或路径不存在时**跳过而不是失败**，并在日志里打印「已跳过」与原因；
//! - CI 上不设置这两个变量，所以 `cargo test --workspace` 必须仍然全绿。
//!
//! 用法：在自己的 crate 里加 `[dev-dependencies] rove-test-support = { workspace = true }`，
//! 然后在测试开头写一行：
//!
//! ```ignore
//! let project = rove_test_support::require_unity_project!(UnityProject::Medium);
//! ```
//!
//! 宏在环境变量缺失时直接 `return`（测试视为通过，日志里写明跳过原因）。

use std::env;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

/// 中型 Unity 测试项目的环境变量名。
pub const UNITY_MEDIUM_ENV: &str = "ROVE_TEST_UNITY_MEDIUM";
/// 大型 Unity 测试项目的环境变量名。
pub const UNITY_LARGE_ENV: &str = "ROVE_TEST_UNITY_LARGE";

/// 两个 Unity 测试项目槽位。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnityProject {
    /// 中型项目（`ROVE_TEST_UNITY_MEDIUM`）。
    Medium,
    /// 大型项目（`ROVE_TEST_UNITY_LARGE`）。
    Large,
}

impl UnityProject {
    /// 全部槽位，便于遍历「两个项目都测一遍」。
    pub const ALL: [UnityProject; 2] = [UnityProject::Medium, UnityProject::Large];

    /// 该槽位对应的环境变量名。
    pub const fn env_var(self) -> &'static str {
        match self {
            UnityProject::Medium => UNITY_MEDIUM_ENV,
            UnityProject::Large => UNITY_LARGE_ENV,
        }
    }

    /// 人类可读的槽位名（用于日志）。
    pub const fn label(self) -> &'static str {
        match self {
            UnityProject::Medium => "中型",
            UnityProject::Large => "大型",
        }
    }

    /// 读取环境变量并确认目录存在。
    ///
    /// 未设置或目录不存在时打印「已跳过：…」并返回 [`None`]，调用方应当跳过当前测试；
    /// 需要项目**必须**存在的场景用 [`UnityProject::require`]。
    pub fn path(self) -> Option<PathBuf> {
        match resolve(env::var_os(self.env_var()), Path::is_dir) {
            Ok(path) => Some(path),
            Err(reason) => {
                eprintln!("{}", reason.describe(self));
                None
            }
        }
    }

    /// 与 [`UnityProject::path`] 相同，但缺失时 panic。
    ///
    /// 只在「跳过就等于没测」的场景用（例如手工运行的回归运行器），
    /// 普通的 `cargo test` 请用 [`UnityProject::path`] 或
    /// [`require_unity_project!`](crate::require_unity_project)。
    #[track_caller]
    pub fn require(self) -> PathBuf {
        match resolve(env::var_os(self.env_var()), Path::is_dir) {
            Ok(path) => path,
            Err(reason) => panic!(
                "{}（路径不要写进仓库，设置方法见 AGENTS.md §8）",
                reason.describe(self)
            ),
        }
    }
}

/// 环境变量缺失或无效的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// 环境变量未设置。
    Unset,
    /// 环境变量已设置，但指向的目录不存在。
    NotFound(PathBuf),
}

impl SkipReason {
    /// 「已跳过」日志文本。
    pub fn describe(&self, slot: UnityProject) -> String {
        match self {
            SkipReason::Unset => format!(
                "已跳过：未设置环境变量 {}（{} Unity 测试项目），相关测试不运行",
                slot.env_var(),
                slot.label()
            ),
            SkipReason::NotFound(path) => format!(
                "已跳过：环境变量 {} 指向的目录不存在（{}），相关测试不运行",
                slot.env_var(),
                path.display()
            ),
        }
    }
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::Unset => write!(f, "环境变量未设置"),
            SkipReason::NotFound(path) => write!(f, "目录不存在：{}", path.display()),
        }
    }
}

/// 纯函数形式的解析逻辑，便于单元测试（不读真实环境变量）。
fn resolve(value: Option<OsString>, is_dir: impl Fn(&Path) -> bool) -> Result<PathBuf, SkipReason> {
    let Some(value) = value else {
        return Err(SkipReason::Unset);
    };
    if value.is_empty() {
        return Err(SkipReason::Unset);
    }
    let path = PathBuf::from(value);
    if is_dir(&path) {
        Ok(path)
    } else {
        Err(SkipReason::NotFound(path))
    }
}

/// 取 Unity 测试项目根目录；环境变量缺失时**直接跳过当前测试**（`return`）。
///
/// ```ignore
/// #[test]
/// fn 引用数量正确() {
///     let project = rove_test_support::require_unity_project!(UnityProject::Medium);
///     // ...
/// }
/// ```
///
/// 只能用在返回 `()` 的测试里；需要返回值的场合请直接用 [`UnityProject::path`]。
#[macro_export]
macro_rules! require_unity_project {
    ($slot:expr) => {
        match $crate::UnityProject::path($slot) {
            ::core::option::Option::Some(path) => path,
            ::core::option::Option::None => return,
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_names_are_the_documented_ones() {
        assert_eq!(UnityProject::Medium.env_var(), "ROVE_TEST_UNITY_MEDIUM");
        assert_eq!(UnityProject::Large.env_var(), "ROVE_TEST_UNITY_LARGE");
        assert_eq!(
            UnityProject::ALL.map(|p| p.env_var()),
            ["ROVE_TEST_UNITY_MEDIUM", "ROVE_TEST_UNITY_LARGE"]
        );
    }

    #[test]
    fn unset_variable_is_a_skip_not_a_failure() {
        assert_eq!(resolve(None, |_| true), Err(SkipReason::Unset));
        assert_eq!(
            resolve(Some(OsString::new()), |_| true),
            Err(SkipReason::Unset)
        );
    }

    #[test]
    fn missing_directory_is_a_skip() {
        let missing = std::env::temp_dir().join("rove-test-support-does-not-exist");
        let reason = resolve(Some(missing.clone().into_os_string()), Path::is_dir).unwrap_err();
        assert_eq!(reason, SkipReason::NotFound(missing));
    }

    #[test]
    fn existing_directory_is_returned() {
        let dir = std::env::temp_dir();
        let path = resolve(Some(dir.clone().into_os_string()), Path::is_dir).unwrap();
        assert_eq!(path, dir);
    }

    #[test]
    fn skip_message_says_skipped_in_chinese_and_names_the_variable() {
        let message = SkipReason::Unset.describe(UnityProject::Medium);
        assert!(message.contains("已跳过"), "message: {message}");
        assert!(
            message.contains("ROVE_TEST_UNITY_MEDIUM"),
            "message: {message}"
        );

        let missing = SkipReason::NotFound(PathBuf::from(r"D:\nowhere\unity"));
        let message = missing.describe(UnityProject::Large);
        assert!(message.contains("已跳过"), "message: {message}");
        assert!(
            message.contains("ROVE_TEST_UNITY_LARGE"),
            "message: {message}"
        );
        assert!(message.contains("nowhere"), "message: {message}");
    }

    #[test]
    fn macro_returns_early_when_project_is_unavailable() {
        // 环境变量未设置（或指向不存在的目录）时，宏必须提前返回：函数体不继续执行。
        fn body(reached: &std::cell::Cell<bool>) {
            let _project = crate::require_unity_project!(UnityProject::Medium);
            reached.set(true);
        }

        // 本地开发机上如果设置了变量，这里无法验证跳过路径，直接跳过断言。
        let unset = env::var_os(UNITY_MEDIUM_ENV).is_none_or(|v| v.is_empty());
        if unset {
            let reached = std::cell::Cell::new(false);
            body(&reached);
            assert!(!reached.get(), "宏在项目缺失时必须提前返回");
        }
    }
}
