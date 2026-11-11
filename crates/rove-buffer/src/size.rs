//! 文件大小分级（#8，`UX_SPEC` §3.1）。
//!
//! | 级别 | 判定条件 | 可编辑 |
//! |---|---|---|
//! | 普通 | 其余情况 | 是 |
//! | 大文件 | ≥ 20 MB 或 ≥ 30 万行；`editor.largeFileOptimizations = false` 时不启用这一级 | 是 |
//! | 超大文件 | ≥ `rove.largeFile.threshold`（默认 64 MB） | 否，只读 |
//!
//! 分级结果写进冻结的 [`DocumentMetadata::size`](crate::DocumentMetadata)；每一级
//! 关掉哪些功能由 [`disabled_features`] 给出，状态栏悬停（#27）与超大文件界面（#22）
//! 逐项列出时使用。本模块只提供数据，不含界面字符串。

use rove_base::{Error, Result};
use rove_config::{Key, Scope, Settings};

use crate::contracts::{ReadOnlyReason, SizeClass};

/// `editor.largeFileOptimizations`：为 `false` 时不启用“大文件”这一级（VS Code 同名设置）。
pub const LARGE_FILE_OPTIMIZATIONS: &str = "editor.largeFileOptimizations";
/// `rove.largeFile.threshold`：超大文件（只读、内存映射）的字节阈值。
pub const LARGE_FILE_THRESHOLD: &str = "rove.largeFile.threshold";

/// 大文件的字节阈值：20 MB（与 VS Code `editor.largeFileOptimizations` 一致）。
pub const LARGE_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// 大文件的行数阈值：30 万行（与 VS Code 一致）。
pub const LARGE_FILE_LINES: u64 = 300_000;
/// `rove.largeFile.threshold` 的默认值：64 MB。
pub const DEFAULT_HUGE_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// 分级所需的两项设置。装配方从设置层读一次（[`SizePolicy::read`]），
/// 设置变化后重新读取；已经打开的文档不重新分级。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizePolicy {
    /// `rove.largeFile.threshold`（字节）；不小于 1。
    pub huge_bytes: u64,
    /// `editor.largeFileOptimizations`。
    pub optimize: bool,
}

impl Default for SizePolicy {
    fn default() -> Self {
        Self {
            huge_bytes: DEFAULT_HUGE_FILE_BYTES,
            optimize: true,
        }
    }
}

impl SizePolicy {
    /// 从设置层读取。两个键由编辑器设置（`rove-ui`）登记；没有登记时（例如单独使用
    /// 本 crate 的测试）按默认值处理，其他错误原样返回。
    pub fn read(settings: &dyn Settings) -> Result<Self> {
        let scope = Scope::default();
        let defaults = Self::default();
        let huge = match Key::<f64>::new(LARGE_FILE_THRESHOLD).read(settings, &scope) {
            Ok(value) if value.is_finite() => value.max(1.0) as u64,
            Ok(_) | Err(Error::NotFound) => defaults.huge_bytes,
            Err(error) => return Err(error),
        };
        let optimize = match Key::<bool>::new(LARGE_FILE_OPTIMIZATIONS).read(settings, &scope) {
            Ok(value) => value,
            Err(Error::NotFound) => defaults.optimize,
            Err(error) => return Err(error),
        };
        Ok(Self {
            huge_bytes: huge,
            optimize,
        })
    }

    /// 只看字节数就能确定的级别：超大文件在打开前判定（决定走内存映射还是读进内存）。
    pub fn is_huge(&self, bytes: u64) -> bool {
        bytes >= self.huge_bytes.max(1)
    }

    /// 分级。`bytes` 是磁盘上的文件大小；`lines` 是行数（超大文件不需要，传 `None`）。
    ///
    /// 超大文件优先于大文件，且不受 `editor.largeFileOptimizations` 影响。
    pub fn classify(&self, bytes: u64, lines: Option<u64>) -> SizeClass {
        if self.is_huge(bytes) {
            SizeClass::Huge
        } else if self.optimize
            && (bytes >= LARGE_FILE_BYTES || lines.is_some_and(|lines| lines >= LARGE_FILE_LINES))
        {
            SizeClass::Large
        } else {
            SizeClass::Normal
        }
    }
}

/// 因文件大小被关闭的功能（`UX_SPEC` §3.1）。顺序即状态栏悬停时的列出顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReducedFeature {
    /// 缩略图。
    Minimap,
    /// 粘性滚动。
    StickyScroll,
    /// 括号对着色。
    BracketPairColorization,
    /// 同名高亮。
    OccurrencesHighlight,
    /// 语义高亮。
    SemanticHighlighting,
    /// 基于语法的折叠。
    SyntaxFolding,
    /// 语法高亮（仅超大文件）。
    SyntaxHighlighting,
    /// 所有折叠，含基于缩进的折叠（仅超大文件）。
    Folding,
    /// 软换行（仅超大文件）。
    WordWrap,
    /// 替换（仅超大文件；超大文件只读）。
    Replace,
}

const LARGE_REDUCED: &[ReducedFeature] = &[
    ReducedFeature::Minimap,
    ReducedFeature::StickyScroll,
    ReducedFeature::BracketPairColorization,
    ReducedFeature::OccurrencesHighlight,
    ReducedFeature::SemanticHighlighting,
    ReducedFeature::SyntaxFolding,
];

const HUGE_REDUCED: &[ReducedFeature] = &[
    ReducedFeature::Minimap,
    ReducedFeature::StickyScroll,
    ReducedFeature::BracketPairColorization,
    ReducedFeature::OccurrencesHighlight,
    ReducedFeature::SemanticHighlighting,
    ReducedFeature::SyntaxFolding,
    ReducedFeature::SyntaxHighlighting,
    ReducedFeature::Folding,
    ReducedFeature::WordWrap,
    ReducedFeature::Replace,
];

/// 某一级关掉的功能清单。普通文件为空；超大文件是大文件清单再加四项。
pub fn disabled_features(size: SizeClass) -> &'static [ReducedFeature] {
    match size {
        SizeClass::Normal => &[],
        SizeClass::Large => LARGE_REDUCED,
        SizeClass::Huge => HUGE_REDUCED,
    }
}

/// “在此会话中切换只读”能否解除这些只读原因。
///
/// 超大文件只读（`UX_SPEC` §3.4 的只读来源 1）不能在会话里解除；只有
/// [`ReadOnlyReason::Session`] 一种原因时才能切换。
pub fn session_toggle_allowed(reasons: &[ReadOnlyReason]) -> bool {
    reasons
        .iter()
        .all(|reason| matches!(reason, ReadOnlyReason::Session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::Value;
    use rove_config::{DefaultSettings, SettingRegistry};

    const MB: u64 = 1024 * 1024;

    /// 真值表：(字节, 行数, optimize, 期望)。阈值都取默认值。
    #[test]
    fn classification_truth_table() {
        use SizeClass::*;
        let cases: &[(u64, Option<u64>, bool, SizeClass)] = &[
            (0, Some(1), true, Normal),
            (20 * MB - 1, Some(299_999), true, Normal),
            (20 * MB, Some(1), true, Large),
            (1024, Some(300_000), true, Large),
            (1024, Some(299_999), true, Normal),
            (64 * MB - 1, Some(1), true, Large),
            (64 * MB, None, true, Huge),
            (u64::MAX, None, true, Huge),
            // editor.largeFileOptimizations = false：大文件级不生效，超大文件级仍然生效。
            (20 * MB, Some(1), false, Normal),
            (1024, Some(300_000), false, Normal),
            (64 * MB - 1, Some(10_000_000), false, Normal),
            (64 * MB, None, false, Huge),
            // 行数未知时只按字节判定。
            (1024, None, true, Normal),
        ];
        for &(bytes, lines, optimize, expected) in cases {
            let policy = SizePolicy {
                optimize,
                ..SizePolicy::default()
            };
            assert_eq!(
                policy.classify(bytes, lines),
                expected,
                "bytes={bytes} lines={lines:?} optimize={optimize}"
            );
        }
    }

    #[test]
    fn huge_threshold_below_large_threshold_still_wins() {
        let policy = SizePolicy {
            huge_bytes: MB,
            optimize: true,
        };
        assert_eq!(policy.classify(MB, Some(1)), SizeClass::Huge);
        assert_eq!(policy.classify(MB - 1, Some(400_000)), SizeClass::Large);
        let zero = SizePolicy {
            huge_bytes: 0,
            optimize: true,
        };
        assert_eq!(zero.classify(0, Some(1)), SizeClass::Normal);
        assert_eq!(zero.classify(1, Some(1)), SizeClass::Huge);
    }

    #[test]
    fn huge_reduces_a_superset_of_large() {
        assert!(disabled_features(SizeClass::Normal).is_empty());
        let large = disabled_features(SizeClass::Large);
        let huge = disabled_features(SizeClass::Huge);
        assert_eq!(large.len(), 6);
        assert_eq!(huge.len(), 10);
        assert!(large.iter().all(|feature| huge.contains(feature)));
        for feature in [
            ReducedFeature::SyntaxHighlighting,
            ReducedFeature::Folding,
            ReducedFeature::WordWrap,
            ReducedFeature::Replace,
        ] {
            assert!(!large.contains(&feature));
            assert!(huge.contains(&feature));
        }
    }

    #[test]
    fn huge_file_read_only_cannot_be_toggled() {
        assert!(session_toggle_allowed(&[ReadOnlyReason::Session]));
        assert!(session_toggle_allowed(&[]));
        assert!(!session_toggle_allowed(&[
            ReadOnlyReason::Session,
            ReadOnlyReason::HugeFile
        ]));
        assert!(!session_toggle_allowed(&[ReadOnlyReason::Permissions]));
    }

    #[test]
    fn reads_policy_from_settings_with_defaults_when_unregistered() {
        let empty = DefaultSettings(SettingRegistry::default());
        assert_eq!(SizePolicy::read(&empty).unwrap(), SizePolicy::default());

        let mut registry = SettingRegistry::default();
        for (key, value) in [
            (LARGE_FILE_THRESHOLD, Value::Number(1000.0)),
            (LARGE_FILE_OPTIMIZATIONS, Value::Bool(false)),
        ] {
            registry
                .register(
                    key,
                    rove_config::Setting {
                        default: value,
                        rove_default: None,
                        language_overridable: false,
                    },
                )
                .unwrap();
        }
        let policy = SizePolicy::read(&DefaultSettings(registry)).unwrap();
        assert_eq!(
            policy,
            SizePolicy {
                huge_bytes: 1000,
                optimize: false
            }
        );
    }
}
