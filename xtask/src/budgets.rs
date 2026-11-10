//! `perf/budgets.toml`：PROJECT_CHARTER §6 的性能指标，机器可读。
//!
//! 规则（写入 `perf/README.md` 与 `AGENTS.md`）：
//! - 每条预算必须写明负责达标的 issue；
//! - 暂时达不到的标 `status = "pending"` 并写清原因，**不允许静默降低数值**；
//! - 要改数值只能走 `PROJECT_CHARTER` 的修订流程，不能直接改这个文件。

use std::path::Path;

use serde::Deserialize;

use crate::results::Results;
use crate::util;

pub const DEFAULT_PATH: &str = "perf/budgets.toml";
pub const SCHEMA: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct Budgets {
    pub schema: u32,
    #[serde(default)]
    pub size: Option<SizeBudget>,
    #[serde(default)]
    pub budget: Vec<Budget>,
}

#[derive(Debug, Deserialize)]
pub struct SizeBudget {
    /// 发布包体积上限（CHARTER §6：发布包 < 30 MB）。
    pub budget_bytes: u64,
    /// 体积守护的比较基线（M0 实测 6.2 MB）。
    pub baseline_bytes: u64,
    pub owner: String,
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Budget {
    pub id: String,
    pub metric: String,
    pub value: f64,
    pub unit: String,
    pub owner: String,
    /// `enforced`（自动判定）/ `pending`（暂时达不到）/ `recorded`（只记录不设门槛）。
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    /// criterion 的用例 id，例如 `cs_index/goto_definition`。
    #[serde(default)]
    pub bench: Option<String>,
}

impl Budgets {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let text = crate::util::read_text(path)
            .map_err(|err| format!("读不到 {}：{err}（先在本仓库根目录下运行）", path.display()))?;
        let budgets: Budgets = toml::from_str(&text)?;
        if budgets.schema != SCHEMA {
            return Err(format!(
                "perf/budgets.toml 的 schema = {}，本 xtask 只认识 {SCHEMA}",
                budgets.schema
            )
            .into());
        }
        Ok(budgets)
    }

    pub fn load_default() -> Result<Self, Box<dyn std::error::Error>> {
        Self::load(&util::repo_root().join(DEFAULT_PATH))
    }
}

/// 按预算逐条判定；超预算或状态非法时返回非零退出码。
///
/// `status = "pending"` 的条目只打印，不参与判定；
/// `status = "recorded"` 的条目有测量值就打印数值（不设门槛）。
pub fn check(budgets: &Budgets, results: &Results) -> Result<u8, Box<dyn std::error::Error>> {
    println!("预算校验（{} 条）：", budgets.budget.len());
    let mut exceeded: Vec<(&Budget, f64)> = Vec::new();
    let mut pending = 0usize;
    let mut unmeasured = 0usize;

    for budget in &budgets.budget {
        let limit_ns = to_ns(budget.value, &budget.unit);
        let measured = budget
            .bench
            .as_ref()
            .and_then(|id| results.get(id))
            .map(|r| r.mean_ns);

        match (budget.status.as_str(), measured) {
            ("enforced", Some(mean_ns)) => {
                let status = if mean_ns <= limit_ns {
                    "OK"
                } else {
                    "超预算"
                };
                println!(
                    "  {:<7} {:<30} 目标 {:>8.2} {:<3} 实测 {:>10}  ({})  {}",
                    status,
                    budget.id,
                    budget.value,
                    budget.unit,
                    format_ns(mean_ns),
                    budget.owner,
                    match status {
                        "OK" => String::new(),
                        _ => format!("超出 {:.1}%", (mean_ns / limit_ns - 1.0) * 100.0),
                    }
                );
                if mean_ns > limit_ns {
                    exceeded.push((budget, mean_ns));
                }
            }
            ("enforced", None) => {
                unmeasured += 1;
                println!(
                    "  {:<7} {:<30} 目标 {:>8.2} {:<3} 未测量（bench = {}，负责 {}）",
                    "跳过",
                    budget.id,
                    budget.value,
                    budget.unit,
                    budget.bench.as_deref().unwrap_or("<未指定>"),
                    budget.owner
                );
            }
            ("pending", _) => {
                pending += 1;
                println!(
                    "  {:<7} {:<30} 目标 {:>8.2} {:<3} 负责 {}  （{}）",
                    "pending",
                    budget.id,
                    budget.value,
                    budget.unit,
                    budget.owner,
                    budget.reason.as_deref().unwrap_or("未写原因")
                );
            }
            ("recorded", measured) => {
                let measured = measured
                    .map(|ns| format!("实测 {}", format_ns(ns)))
                    .unwrap_or_else(|| "未测量".to_string());
                println!(
                    "  {:<7} {:<30} 只记录不设门槛：{}  （负责 {}）",
                    "record", budget.id, measured, budget.owner
                );
            }
            (other, _) => {
                return Err(format!(
                    "预算 `{}` 的 status = `{other}` 非法（只接受 enforced / pending / recorded）",
                    budget.id
                )
                .into());
            }
        }
    }

    println!("  合计：pending {pending} 条，未测量 {unmeasured} 条。");

    if exceeded.is_empty() {
        println!("OK：没有超出预算的项。");
        return Ok(0);
    }

    eprintln!("超预算 {} 项：", exceeded.len());
    for (budget, mean_ns) in &exceeded {
        let limit_ns = to_ns(budget.value, &budget.unit);
        eprintln!(
            "  {}（{}）：目标 {} {}，实测 {}，超出 {:.1}%；负责 issue {}",
            budget.id,
            budget.metric,
            budget.value,
            budget.unit,
            format_ns(*mean_ns),
            (mean_ns / limit_ns - 1.0) * 100.0,
            budget.owner
        );
        println!(
            "::error::性能预算超限 {}：目标 {} {}，实测 {}（超出 {:.1}%，负责 {}）",
            budget.id,
            budget.value,
            budget.unit,
            format_ns(*mean_ns),
            (mean_ns / limit_ns - 1.0) * 100.0,
            budget.owner
        );
    }
    Ok(1)
}

fn to_ns(value: f64, unit: &str) -> f64 {
    match unit {
        "ns" => value,
        "us" | "µs" => value * 1e3,
        "ms" => value * 1e6,
        "s" => value * 1e9,
        _ => value,
    }
}

fn format_ns(ns: f64) -> String {
    if ns >= 1e9 {
        format!("{:.2} s", ns / 1e9)
    } else if ns >= 1e6 {
        format!("{:.2} ms", ns / 1e6)
    } else if ns >= 1e3 {
        format!("{:.2} us", ns / 1e3)
    } else {
        format!("{ns:.0} ns")
    }
}
