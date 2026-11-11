//! 打开项目时的有效性检查（#36 目标 2）。
//!
//! 四项检查，任一项不过就全量重建：
//! 1. magic 与格式版本（读文件前 12 字节，不经过 `CsIndex::open`，所以能区分“版本不对”与“文件坏了”）；
//! 2. 生成它的索引进程版本（头部 24..32）与 [`EXPECTED_INDEXER_VERSION`]；
//! 3. 项目配置：头部配置哈希与 `project.json` 的构建记录一致，且输入指纹与当前磁盘一致（见 `inputs`）；
//! 4. 抽样文件哈希：随机抽 [`SampleRule`] 规定数量的文件，比对 xxHash64，命中率低于阈值即失效。
//!
//! 前三项只读几百字节，打开时同步完成；第 4 项要读源文件，放在索引可用之后的后台检查里。
use super::{hash::xxh64, inputs::ProjectInputs, meta::hex, meta::BuildRecord};
use crate::index::CsIndex;
use rove_base::Cancellation;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const EXPECTED_FORMAT: u32 = 2;
/// 与 `tools/rove-indexer-cs/RoveIndexer.csproj` 的 `<Version>` 一致（测试对照该文件，改版本时
/// 会提醒同步这里）。#43 合并后改为与安装的组件握手得到的版本。
pub const EXPECTED_INDEXER_VERSION: [u16; 4] = [1, 0, 2, 0];
const GENERATED_PREFIX: &str = "rove-generated://";

/// 失效原因（写进日志；界面只显示“正在重建”）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalid {
    Unreadable(String),
    Format { found: Option<u32> },
    Corrupt(String),
    IndexerVersion { found: [u16; 4] },
    ProjectConfig,
    SampledFiles { matched: usize, sampled: usize },
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(e) => write!(f, "index unreadable: {e}"),
            Self::Format { found: Some(v) } => {
                write!(f, "format version {v}, expected {EXPECTED_FORMAT}")
            }
            Self::Format { found: None } => write!(f, "not a Rove C# index"),
            Self::Corrupt(e) => write!(f, "index corrupt: {e}"),
            Self::IndexerVersion { found } => write!(
                f,
                "indexer version {found:?}, expected {EXPECTED_INDEXER_VERSION:?}"
            ),
            Self::ProjectConfig => write!(f, "project configuration changed"),
            Self::SampledFiles { matched, sampled } => {
                write!(f, "sampled file hashes: {matched}/{sampled} match")
            }
        }
    }
}

/// 抽样规则：抽 `max(min_files, fraction × 文件数)` 个（不超过文件总数），命中率低于
/// `min_hit_rate` 判定失效；命中率达标时，不一致的文件只标记为过期。
///
/// 默认 200 个或 2%（取大者）、阈值 90%：中型项目（约 1 万个文件）抽 200 个，源文件平均十几 KB，
/// 冷缓存 HDD 上估计 1–2 秒（未单独实测），在后台进行，不阻塞打开；少量文件离线改动只标过期，大面积改动
/// （切分支、Unity 升级）才重建。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleRule {
    pub min_files: usize,
    pub fraction: f64,
    pub min_hit_rate: f64,
}

impl Default for SampleRule {
    fn default() -> Self {
        Self {
            min_files: 200,
            fraction: 0.02,
            min_hit_rate: 0.9,
        }
    }
}

impl SampleRule {
    pub fn count(&self, files: usize) -> usize {
        let by_fraction = (files as f64 * self.fraction).ceil() as usize;
        self.min_files.max(by_fraction).min(files)
    }
}

/// 读格式版本并打开索引，再核对索引进程版本与项目配置（检查 1–3）。
pub fn open_checked(
    path: &Path,
    record: Option<&BuildRecord>,
    inputs: &ProjectInputs,
) -> Result<CsIndex, Invalid> {
    let mut head = [0u8; 12];
    let mut file = fs::File::open(path).map_err(|e| Invalid::Unreadable(e.to_string()))?;
    if file.read_exact(&mut head).is_err() || &head[..8] != b"RVCSIDX\0" {
        return Err(Invalid::Format { found: None });
    }
    drop(file);
    let format = u32::from_le_bytes(head[8..12].try_into().unwrap());
    if format != EXPECTED_FORMAT {
        return Err(Invalid::Format {
            found: Some(format),
        });
    }
    let index = CsIndex::open(path).map_err(|e| Invalid::Corrupt(e.to_string()))?;
    let header = index.header();
    if header.indexer_version != EXPECTED_INDEXER_VERSION {
        return Err(Invalid::IndexerVersion {
            found: header.indexer_version,
        });
    }
    match record {
        Some(r)
            if r.config_hash == hex(&header.project_config_hash)
                && r.fingerprint == inputs.fingerprint => {}
        _ => return Err(Invalid::ProjectConfig),
    }
    Ok(index)
}

/// 抽样结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleOutcome {
    pub sampled: usize,
    pub matched: usize,
    /// 内容不一致或已删除的文件（项目相对路径），命中率达标时标记为过期。
    pub mismatched: Vec<String>,
}

impl SampleOutcome {
    pub fn verdict(&self, rule: &SampleRule) -> Result<(), Invalid> {
        if self.sampled == 0 || self.matched as f64 >= self.sampled as f64 * rule.min_hit_rate {
            Ok(())
        } else {
            Err(Invalid::SampledFiles {
                matched: self.matched,
                sampled: self.sampled,
            })
        }
    }
}

fn source_path(root: &Path, rel: &str) -> PathBuf {
    let path = Path::new(rel);
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(rel)
    }
}

/// 检查 4：随机抽样比对文件内容哈希。跳过 `rove-generated://` 虚拟文件。取消时返回已抽查的部分。
pub fn sample_files(
    index: &CsIndex,
    root: &Path,
    rule: &SampleRule,
    seed: u64,
    cancel: &Cancellation,
) -> SampleOutcome {
    let mut candidates: Vec<u32> = (0..index.files().len() as u32)
        .filter(|&i| {
            index
                .file_path(i)
                .is_some_and(|p| !p.starts_with(GENERATED_PREFIX))
        })
        .collect();
    let count = rule.count(candidates.len());
    let mut state = seed | 1;
    let mut outcome = SampleOutcome::default();
    for i in 0..count {
        if cancel.check().is_err() {
            break;
        }
        // xorshift64*，只需要“不可预测地分散”，不需要密码学强度。
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let pick = i
            + (state.wrapping_mul(0x2545_F491_4F6C_DD1D) % (candidates.len() - i) as u64) as usize;
        candidates.swap(i, pick);
        let file = candidates[i];
        let Some(rel) = index.file_path(file) else {
            continue;
        };
        outcome.sampled += 1;
        let expected = index.files()[file as usize].hash;
        match fs::read(source_path(root, rel)) {
            Ok(bytes) if xxh64(&bytes, 0) == expected => outcome.matched += 1,
            _ => outcome.mismatched.push(rel.to_owned()),
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_version_matches_the_indexer_project() {
        let csproj = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/rove-indexer-cs/RoveIndexer.csproj"
        ))
        .unwrap();
        let version = csproj
            .split("<Version>")
            .nth(1)
            .and_then(|s| s.split("</Version>").next())
            .unwrap();
        let mut parts: Vec<u16> = version.split('.').map(|p| p.parse().unwrap()).collect();
        parts.resize(4, 0);
        assert_eq!(
            parts, EXPECTED_INDEXER_VERSION,
            "更新 EXPECTED_INDEXER_VERSION"
        );
    }

    #[test]
    fn sample_size_rule() {
        let rule = SampleRule::default();
        assert_eq!(rule.count(50), 50);
        assert_eq!(rule.count(5_000), 200);
        assert_eq!(rule.count(50_000), 1_000);
        let outcome = SampleOutcome {
            sampled: 10,
            matched: 9,
            mismatched: vec!["a".into()],
        };
        assert!(outcome.verdict(&rule).is_ok());
        let outcome = SampleOutcome {
            sampled: 10,
            matched: 8,
            ..Default::default()
        };
        assert_eq!(
            outcome.verdict(&rule),
            Err(Invalid::SampledFiles {
                matched: 8,
                sampled: 10
            })
        );
    }
}
