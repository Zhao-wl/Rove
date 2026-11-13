//! 文件装饰（#23，无 GPUI）：装饰接口与两类来源。
//!
//! - 只读标记：来自 #15 的只读规则（`files.readonlyInclude`、`rove.files.extraReadonlyInclude` 等）；
//! - 错误 / 警告计数：来自问题面板（R60 合并前为空），文件夹显示其下的总数。
//!
//! **不接 Git**：`gitDecoration.*` 颜色键静默忽略（FEATURES §10：不做版本控制）。
use rove_base::WorkspacePath;
use rove_workspace::{fold_key, Workspace};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// 徽标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    /// 只读（锁）。
    Lock,
    /// 错误或警告的数量。
    Count(u32),
}

/// 名称的着色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecorationColor {
    Error,
    Warning,
    Deemphasized,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decoration {
    pub badge: Option<Badge>,
    pub color: Option<DecorationColor>,
}

impl Decoration {
    /// 把 `other` 合并进来：已有的徽标与颜色优先（问题计数比只读更重要）。
    pub fn merged(self, other: Decoration) -> Decoration {
        Decoration {
            badge: self.badge.or(other.badge),
            color: self.color.or(other.color),
        }
    }
}

/// 装饰来源。在绘制线程上对可见行调用，必须很快返回。
pub trait DecorationProvider: Send + Sync {
    fn decorate(&self, path: &WorkspacePath, is_dir: bool) -> Option<Decoration>;
}

/// 取所有来源合并后的装饰。
pub fn decorate(
    providers: &[Arc<dyn DecorationProvider>],
    path: &WorkspacePath,
    is_dir: bool,
) -> Option<Decoration> {
    providers
        .iter()
        .filter_map(|p| p.decorate(path, is_dir))
        .reduce(Decoration::merged)
}

/// 只读规则（#15 的 `Workspace::rules`）：只读文件显示锁徽标，名称弱化。
pub struct ReadOnlyDecorations(pub Arc<dyn Workspace>);

impl DecorationProvider for ReadOnlyDecorations {
    fn decorate(&self, path: &WorkspacePath, is_dir: bool) -> Option<Decoration> {
        if is_dir {
            return None;
        }
        let rules = self.0.rules(path).ok()?;
        (!rules.read_only.is_empty()).then_some(Decoration {
            badge: Some(Badge::Lock),
            color: Some(DecorationColor::Deemphasized),
        })
    }
}

/// 问题计数：问题面板（R60）把每个文件的错误 / 警告数写进来，文件夹显示其下的总数。
#[derive(Default)]
pub struct ProblemDecorations {
    inner: RwLock<Problems>,
}

#[derive(Default)]
struct Problems {
    files: HashMap<String, (u32, u32)>,
    folders: HashMap<String, (u32, u32)>,
}

fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(move |(i, _)| &path[..i])
}

impl ProblemDecorations {
    /// 设置一个文件的错误与警告数；两者都是 0 等于清除。
    pub fn set(&self, path: &WorkspacePath, errors: u32, warnings: u32) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let key = fold_key(path.as_str());
        let old = if errors == 0 && warnings == 0 {
            inner.files.remove(&key)
        } else {
            inner.files.insert(key, (errors, warnings))
        }
        .unwrap_or((0, 0));
        for folder in ancestors(path.as_str()) {
            let entry = inner.folders.entry(fold_key(folder)).or_default();
            entry.0 = (entry.0 + errors).saturating_sub(old.0);
            entry.1 = (entry.1 + warnings).saturating_sub(old.1);
        }
    }

    pub fn clear(&self) {
        *self.inner.write().unwrap_or_else(|e| e.into_inner()) = Problems::default();
    }
}

impl DecorationProvider for ProblemDecorations {
    fn decorate(&self, path: &WorkspacePath, is_dir: bool) -> Option<Decoration> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let key = fold_key(path.as_str());
        let (errors, warnings) = if is_dir {
            inner.folders.get(&key)
        } else {
            inner.files.get(&key)
        }
        .copied()?;
        let (count, color) = if errors > 0 {
            (errors, DecorationColor::Error)
        } else if warnings > 0 {
            (warnings, DecorationColor::Warning)
        } else {
            return None;
        };
        Some(Decoration {
            // 文件夹只着色不显示数字（VS Code 的文件夹装饰是一个点）。
            badge: (!is_dir).then_some(Badge::Count(count)),
            color: Some(color),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> WorkspacePath {
        WorkspacePath::new(s).unwrap()
    }

    #[test]
    fn folders_total_the_problems_below_and_clear_with_the_file() {
        let store = ProblemDecorations::default();
        store.set(&p("A/B/x.cs"), 2, 0);
        store.set(&p("A/y.cs"), 0, 3);
        let folder = |s: &str| store.decorate(&p(s), true);
        assert_eq!(folder("A").unwrap().color, Some(DecorationColor::Error));
        assert_eq!(folder("A/B").unwrap().badge, None);
        assert_eq!(
            store.decorate(&p("A/B/x.cs"), false).unwrap().badge,
            Some(Badge::Count(2))
        );
        store.set(&p("A/B/x.cs"), 0, 0);
        assert_eq!(folder("A").unwrap().color, Some(DecorationColor::Warning));
        assert!(folder("A/B").is_none());
        assert!(store.decorate(&p("A/B/x.cs"), false).is_none());
    }

    #[test]
    fn errors_win_over_the_read_only_lock() {
        let error = Decoration {
            badge: Some(Badge::Count(1)),
            color: Some(DecorationColor::Error),
        };
        let lock = Decoration {
            badge: Some(Badge::Lock),
            color: Some(DecorationColor::Deemphasized),
        };
        assert_eq!(error.merged(lock), error);
    }
}
