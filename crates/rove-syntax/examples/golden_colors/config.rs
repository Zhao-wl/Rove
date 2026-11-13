//! 黄金测试的配置与阈值（#11）：`tests/golden/thresholds.toml`。
//!
//! 阈值是数据而不是代码：加一种语言只需要在 `[languages]` 下加一行，调阈值不用改运行器。

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// issue #11 给 C#、JSON、YAML、XML、Markdown 定的目标一致率。低于它的阈值要写明理由。
pub const TARGET: f64 = 0.98;

/// 数据文件相对仓库根目录的位置。
pub const THRESHOLDS: &str = "tests/golden/thresholds.toml";
pub const FIXTURES_DIR: &str = "tests/golden/colorize-fixtures";
pub const RESULTS_DIR: &str = "tests/golden/colorize-results";

#[derive(Debug, Deserialize)]
struct RawConfig {
    enforced_themes: Vec<String>,
    reported_themes: Vec<String>,
    languages: BTreeMap<String, RawLanguage>,
}

#[derive(Debug, Deserialize)]
struct RawLanguage {
    fixtures: Vec<String>,
    threshold: Option<f64>,
    reason: Option<String>,
}

/// 一种语言：Rove 语言 ID、它的 fixture 文件名，以及可选阈值（`None` 表示只统计）。
#[derive(Clone, Debug)]
pub struct Language {
    pub id: String,
    pub fixtures: Vec<String>,
    pub threshold: Option<f64>,
    /// 阈值低于 [`TARGET`] 时必须写明理由（issue #11：调低阈值要在 PR 里说明）。
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Config {
    /// 受阈值约束的主题（`colorize-results` 里的键，也是 `rove-theme` 的主题文件名）。
    pub enforced_themes: Vec<String>,
    /// 只统计、不判定的主题。
    pub reported_themes: Vec<String>,
    /// 按语言 ID 排序，保证报告顺序稳定。
    pub languages: Vec<Language>,
}

impl Config {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join(THRESHOLDS);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("读不到阈值文件 {}：{e}", path.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawConfig = toml::from_str(text).map_err(|e| format!("阈值文件格式错误：{e}"))?;
        if raw.enforced_themes.is_empty() {
            return Err("enforced_themes 不能为空".into());
        }
        let languages = raw
            .languages
            .into_iter()
            .map(|(id, lang)| {
                if lang.fixtures.is_empty() {
                    return Err(format!("语言 {id} 没有列出 fixture"));
                }
                if let Some(t) = lang.threshold {
                    if !(0.0..=1.0).contains(&t) {
                        return Err(format!("语言 {id} 的阈值 {t} 不在 0..=1 之间"));
                    }
                    if t < TARGET && lang.reason.as_deref().unwrap_or("").trim().is_empty() {
                        return Err(format!(
                            "语言 {id} 的阈值 {t} 低于 {TARGET}，必须用 reason 写明理由"
                        ));
                    }
                }
                Ok(Language {
                    id,
                    fixtures: lang.fixtures,
                    threshold: lang.threshold,
                    reason: lang.reason,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            enforced_themes: raw.enforced_themes,
            reported_themes: raw.reported_themes,
            languages,
        })
    }

    /// 全部参与统计的主题：先受阈值约束的，再只统计的。
    pub fn themes(&self) -> Vec<String> {
        self.enforced_themes
            .iter()
            .chain(self.reported_themes.iter())
            .cloned()
            .collect()
    }

    pub fn is_enforced(&self, theme: &str) -> bool {
        self.enforced_themes.iter().any(|t| t == theme)
    }
}

/// 仓库根目录（`crates/rove-syntax` 的上两级）。
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}
