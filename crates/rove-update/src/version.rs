//! 版本号的解析与比较（issue #149 第 2 节）。
//!
//! 版本号的**唯一来源**是根 `Cargo.toml` 的 `[workspace.package] version`（#76 §3），
//! 形如 `0.1.0`；#43 的索引组件用四段（`1.2.0.0`）。两种都要能读，所以统一按
//! 「2 到 4 段十进制数字，可选 `-预发布` 后缀」解析，缺省的段补 0。
//!
//! 规则很保守：**解析不了就当没有更新**（#149 第 2 节）。宁可错过一次更新，
//! 也不要因为一个畸形的版本号去下载安装包。

use crate::{Result, UpdateError};
use std::{cmp::Ordering, fmt};

/// 最多四段的版本号，外加可选的预发布标签。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    parts: [u32; 4],
    /// `0.2.0-rc.1` 里的 `rc.1`。有预发布标签的版本**低于**同号的正式版本。
    pre: Option<String>,
}

impl Version {
    /// 解析 `1.2`、`1.2.3`、`1.2.3.4`、`1.2.3-rc.1`。
    ///
    /// 拒绝：空串、段数不足 2 或超过 4、非十进制数字、超过 `u32`、空段（`1..2`）、
    /// 前后缀字符（`v1.2.3` 要调用方自己去掉 `v`）。
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let bad = |why: &str| UpdateError::Version(format!("{text:?}：{why}"));
        if text.is_empty() {
            return Err(bad("空的版本号"));
        }
        let (numbers, pre) = match text.split_once('-') {
            Some((numbers, pre)) if !pre.is_empty() => (numbers, Some(pre.to_string())),
            Some(_) => return Err(bad("`-` 后面没有预发布标签")),
            None => (text, None),
        };
        let mut parts = [0u32; 4];
        let mut count = 0usize;
        for segment in numbers.split('.') {
            if count == 4 {
                return Err(bad("超过四段"));
            }
            if segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad("每一段都必须是十进制数字"));
            }
            parts[count] = segment.parse().map_err(|_| bad("数字太大"))?;
            count += 1;
        }
        if count < 2 {
            return Err(bad("至少要有两段"));
        }
        Ok(Self { parts, pre })
    }

    /// 当前运行的 Rove 版本（根 `Cargo.toml` 的 `version`，由调用方传进来，
    /// 这样核心层不必依赖任何构建期注入）。
    pub fn parts(&self) -> [u32; 4] {
        self.parts
    }

    pub fn pre_release(&self) -> Option<&str> {
        self.pre.as_deref()
    }

    /// 是否比 `current` 新。版本号不更新时返回 `false`（= 没有更新）。
    pub fn is_newer_than(&self, current: &Version) -> bool {
        self > current
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts.cmp(&other.parts).then_with(|| {
            // 预发布版本低于同号正式版本；两个预发布之间按字典序（够用：只有一个稳定通道）。
            match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d] = self.parts;
        if d == 0 {
            write!(f, "{a}.{b}.{c}")?;
        } else {
            write!(f, "{a}.{b}.{c}.{d}")?;
        }
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn two_to_four_segments_parse() {
        assert_eq!(v("0.1").parts(), [0, 1, 0, 0]);
        assert_eq!(v("0.1.0").parts(), [0, 1, 0, 0]);
        assert_eq!(v("1.2.3.4").parts(), [1, 2, 3, 4]);
        assert_eq!(v(" 0.1.1 ").parts(), [0, 1, 1, 0]);
        assert_eq!(v("0.2.0-rc.1").pre_release(), Some("rc.1"));
    }

    /// 验收：版本号非法 → 不更新（这里只负责报错，判定在 engine）。
    #[test]
    fn malformed_versions_are_rejected() {
        for text in [
            "",
            "   ",
            "1",
            "v1.2.3",
            "1.2.3.4.5",
            "1..2",
            "1.2.x",
            "0.1.0-",
            "1.2.-3",
            "99999999999.0.0",
        ] {
            let error = Version::parse(text).unwrap_err();
            assert!(
                matches!(error, UpdateError::Version(_)),
                "{text:?} 应该解析失败，实际 {error:?}"
            );
        }
    }

    #[test]
    fn newer_is_strictly_greater() {
        assert!(v("0.1.1").is_newer_than(&v("0.1.0")));
        assert!(v("0.2.0").is_newer_than(&v("0.1.99")));
        assert!(v("1.0.0").is_newer_than(&v("0.99.99")));
        assert!(v("1.2.3.4").is_newer_than(&v("1.2.3")));
        // 相同或更旧都不算更新（验收：版本号不比当前新 → 不更新）。
        assert!(!v("0.1.0").is_newer_than(&v("0.1.0")));
        assert!(!v("0.1.0").is_newer_than(&v("0.1.1")));
        assert!(!v("0.1.0").is_newer_than(&v("1.0.0")));
        // 预发布低于正式版。
        assert!(!v("0.2.0-rc.1").is_newer_than(&v("0.2.0")));
        assert!(v("0.2.0").is_newer_than(&v("0.2.0-rc.1")));
        assert!(v("0.2.0-rc.2").is_newer_than(&v("0.2.0-rc.1")));
    }

    #[test]
    fn display_round_trips() {
        for text in ["0.1.0", "1.2.3.4", "0.2.0-rc.1"] {
            assert_eq!(v(text).to_string(), text);
        }
        // 两段补成三段显示。
        assert_eq!(v("0.1").to_string(), "0.1.0");
    }
}
