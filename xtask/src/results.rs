//! 基准结果的机器可读格式（供前后两次运行比较，也供 CI 的基线与对比任务使用）。
//!
//! 结果从 criterion 的 `target/criterion/<组>/<用例>/new/estimates.json` 汇总而来，
//! 单位统一为纳秒。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Results {
    pub schema: u32,
    /// 生成时刻（Unix 秒），不引入 chrono 依赖。
    pub unix_time: u64,
    pub machine: Machine,
    #[serde(default)]
    pub benchmarks: BTreeMap<String, BenchResult>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Machine {
    pub os: String,
    pub arch: String,
    pub cpus: usize,
    /// 手工填写的机器说明（参考设备 / 开发机 / CI runner），比较时一定要看。
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BenchResult {
    /// criterion 的 mean 点估计，单位纳秒。
    pub mean_ns: f64,
    #[serde(default)]
    pub std_dev_ns: Option<f64>,
}

impl Results {
    pub fn new() -> Self {
        Results {
            schema: SCHEMA,
            unix_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            machine: Machine {
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
                cpus: std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(0),
                note: std::env::var("ROVE_PERF_MACHINE_NOTE").ok(),
            },
            benchmarks: BTreeMap::new(),
        }
    }

    /// 汇总 `target/criterion` 下所有 `new/estimates.json`。
    pub fn from_criterion(target_dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut results = Results::new();
        let criterion = target_dir.join("criterion");
        if !criterion.is_dir() {
            return Err(format!(
                "找不到 {}：先跑一次基准（cargo xtask bench）",
                criterion.display()
            )
            .into());
        }
        let mut stack: Vec<PathBuf> = vec![criterion];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .file_name()
                    .is_some_and(|name| name == "estimates.json")
                    && dir.file_name().is_some_and(|name| name == "new")
                {
                    let Some(parent) = dir.parent() else { continue };
                    let group = parent
                        .parent()
                        .and_then(|p| p.strip_prefix(target_dir.join("criterion")).ok())
                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    let name = parent
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let id = if group.is_empty() {
                        name
                    } else {
                        format!("{group}/{name}")
                    };
                    if let Some(result) = read_estimates(&path)? {
                        results.benchmarks.insert(id, result);
                    }
                }
            }
        }
        results.benchmarks.retain(|id, _| !id.starts_with("report"));
        Ok(results)
    }

    pub fn get(&self, id: &str) -> Option<&BenchResult> {
        self.benchmarks.get(id)
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, format!("{json}\n"))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let text = crate::util::read_text(path)?;
        Ok(serde_json::from_str(&text)?)
    }
}

fn read_estimates(path: &Path) -> Result<Option<BenchResult>, Box<dyn std::error::Error>> {
    let text = fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let mean = value["mean"]["point_estimate"].as_f64();
    let std_dev = value["mean"]["standard_error"].as_f64();
    Ok(mean.map(|mean_ns| BenchResult {
        mean_ns,
        std_dev_ns: std_dev,
    }))
}
