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
    /// 取哪个聚合值：`mean`（默认）或 `p99`。导航类指标关心尾延迟，用 `p99`。
    #[serde(default)]
    pub aggregate: Option<String>,
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
        // 先校验声明本身，避免单位/聚合值写错时静默按错误口径判定。
        let aggregate = budget.aggregate.as_deref().unwrap_or("mean");
        if !matches!(aggregate, "mean" | "p99") {
            return Err(format!(
                "预算 `{}` 的 aggregate = `{aggregate}` 非法（只接受 mean / p99）",
                budget.id
            )
            .into());
        }
        // 时间类指标走 criterion 的结果（纳秒）；内存一类的非时间指标走 `metrics`
        //（#150：`cargo xtask startup` 写进来的空窗口内存，单位就是预算自己的 `unit`）。
        let is_time = time_unit(budget.value, &budget.unit).is_some();
        let measured = budget.bench.as_ref().and_then(|id| {
            if is_time {
                results.get(id).map(|r| {
                    if aggregate == "p99" {
                        r.p99_ns.unwrap_or(r.mean_ns)
                    } else {
                        r.mean_ns
                    }
                })
            } else {
                results.metric(id)
            }
        });
        // 非时间指标的展示值用预算自己的单位，时间指标统一换算成纳秒再格式化。
        let show = |value: f64| {
            if is_time {
                format_ns(value)
            } else {
                format!("{value:.1} {}", budget.unit)
            }
        };

        match (budget.status.as_str(), measured) {
            ("enforced", Some(measured_value)) => {
                let limit = time_unit(budget.value, &budget.unit).unwrap_or(budget.value);
                let status = if measured_value <= limit {
                    "OK"
                } else {
                    "超预算"
                };
                println!(
                    "  {:<7} {:<30} 目标 {:>8.2} {:<3} 实测 {:>10} [{}]  ({})  {}",
                    status,
                    budget.id,
                    budget.value,
                    budget.unit,
                    show(measured_value),
                    if is_time { aggregate } else { "median" },
                    budget.owner,
                    match status {
                        "OK" => String::new(),
                        _ => format!("超出 {:.1}%", (measured_value / limit - 1.0) * 100.0),
                    }
                );
                if measured_value > limit {
                    exceeded.push((budget, measured_value));
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
                    .map(|value| format!("实测 {}", show(value)))
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
    for (budget, measured) in &exceeded {
        let limit = time_unit(budget.value, &budget.unit).unwrap_or(budget.value);
        let shown = match time_unit(budget.value, &budget.unit) {
            Some(_) => format_ns(*measured),
            None => format!("{measured:.1} {}", budget.unit),
        };
        eprintln!(
            "  {}（{}）：目标 {} {}，实测 {}，超出 {:.1}%；负责 issue {}",
            budget.id,
            budget.metric,
            budget.value,
            budget.unit,
            shown,
            (measured / limit - 1.0) * 100.0,
            budget.owner
        );
        println!(
            "::error::性能预算超限 {}：目标 {} {}，实测 {}（超出 {:.1}%，负责 {}）",
            budget.id,
            budget.value,
            budget.unit,
            shown,
            (measured / limit - 1.0) * 100.0,
            budget.owner
        );
    }
    Ok(1)
}

/// 时间单位换算成纳秒；不认识的单位（包括 `MB` 这类非时间单位）返回 `None`，
/// 由调用方报错，绝不静默按原值处理。
fn time_unit(value: f64, unit: &str) -> Option<f64> {
    match unit {
        "ns" => Some(value),
        "us" | "µs" => Some(value * 1e3),
        "ms" => Some(value * 1e6),
        "s" => Some(value * 1e9),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::results::{BenchResult, Results};

    fn budgets() -> Budgets {
        Budgets::load_default().expect("读得到 perf/budgets.toml")
    }

    /// #150（v0.1 发布门槛 5）：**不允许带着 `pending` 条目发布**。
    /// 每条预算要么 `enforced`（有判定路径），要么 `recorded`（章程口径里就只记录不设门槛）。
    #[test]
    fn no_budget_is_still_pending() {
        let budgets = budgets();
        let pending: Vec<&str> = budgets
            .budget
            .iter()
            .filter(|b| b.status == "pending")
            .map(|b| b.id.as_str())
            .collect();
        assert!(
            pending.is_empty(),
            "perf/budgets.toml 还有 pending 条目：{pending:?}；v0.1 发布门槛要求每条要么 \
             enforced 且达标，要么按章程流程改成 recorded（docs/MILESTONE_v0.1.md §5）"
        );
    }

    /// `enforced` 的条目必须指得出测量来源，否则 `--check-budgets` 永远只会打印「未测量」。
    #[test]
    fn every_enforced_budget_names_where_the_number_comes_from() {
        for budget in &budgets().budget {
            if budget.status == "enforced" {
                assert!(
                    budget.bench.is_some(),
                    "预算 `{}` 是 enforced，却没有 bench / metric 来源",
                    budget.id
                );
            }
            assert!(
                matches!(budget.status.as_str(), "enforced" | "recorded"),
                "预算 `{}` 的 status = `{}` 非法",
                budget.id,
                budget.status
            );
            assert!(
                budget.reason.as_deref().is_some_and(|r| !r.is_empty()),
                "预算 `{}` 没写口径与来源",
                budget.id
            );
        }
    }

    /// id 不能重复：重复的话 `--check-budgets` 会把同一条判两遍，报告也会对不上。
    #[test]
    fn budget_ids_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for budget in &budgets().budget {
            assert!(
                seen.insert(budget.id.clone()),
                "预算 id `{}` 重复",
                budget.id
            );
        }
    }

    /// #150：内存一类的非时间指标走 `metrics`，和时间指标一样能判超限。
    #[test]
    fn a_memory_budget_is_judged_against_the_metrics_table() {
        let budget = Budget {
            id: "empty_window_memory".into(),
            metric: "空窗口常驻内存".into(),
            value: 60.0,
            unit: "MB".into(),
            owner: "#16".into(),
            status: "enforced".into(),
            reason: Some("测试".into()),
            bench: Some("startup/empty_window_working_set".into()),
            aggregate: None,
        };
        let budgets = Budgets {
            schema: SCHEMA,
            size: None,
            budget: vec![budget],
        };

        let mut under = Results::new();
        under
            .metrics
            .insert("startup/empty_window_working_set".into(), 52.5);
        assert_eq!(check(&budgets, &under).unwrap(), 0);

        let mut over = Results::new();
        over.metrics
            .insert("startup/empty_window_working_set".into(), 77.0);
        assert_eq!(check(&budgets, &over).unwrap(), 1, "超了必须非零退出");

        // 时间类的结果表里有同名条目也不会被误用（内存只认 metrics）。
        let mut wrong_table = Results::new();
        wrong_table.benchmarks.insert(
            "startup/empty_window_working_set".into(),
            BenchResult {
                mean_ns: 1.0,
                p99_ns: None,
                std_dev_ns: None,
            },
        );
        assert_eq!(check(&budgets, &wrong_table).unwrap(), 0, "未测量不算失败");
    }

    /// #219：「空窗口常驻内存」只能按**工作集**判定。
    ///
    /// 私有提交与私有工作集同时采集、各有一条 `recorded` 条目（报告里三个数都要写），
    /// 但都不设门槛——换成私有提交判定等于换了基线却不换 CHARTER §6 的 60 MB。
    #[test]
    fn only_the_working_set_judges_the_resident_memory_budget() {
        let budgets = budgets();
        let memory: Vec<&Budget> = budgets
            .budget
            .iter()
            .filter(|b| {
                b.bench
                    .as_deref()
                    .is_some_and(|bench| bench.starts_with("startup/empty_window"))
            })
            .collect();
        let enforced: Vec<&str> = memory
            .iter()
            .filter(|b| b.status == "enforced")
            .map(|b| b.id.as_str())
            .collect();
        assert_eq!(
            enforced,
            vec!["empty_window_memory"],
            "空窗口内存只有工作集那条判定（#219 的口径），其余只记录"
        );
        let judged = memory
            .iter()
            .find(|b| b.id == "empty_window_memory")
            .expect("有 empty_window_memory");
        assert_eq!(
            judged.bench.as_deref(),
            Some(crate::startup::ID_EMPTY_WINDOW_WORKING_SET)
        );
        assert_eq!(judged.value, 60.0, "CHARTER §6 的数值不因口径确认而改动");
        assert_eq!(judged.unit, "MB");

        for (id, bench) in [
            (
                "empty_window_private_commit",
                crate::startup::ID_EMPTY_WINDOW_PRIVATE,
            ),
            (
                "empty_window_private_working_set",
                crate::startup::ID_EMPTY_WINDOW_PRIVATE_WORKING_SET,
            ),
        ] {
            let budget = memory
                .iter()
                .find(|b| b.id == id)
                .unwrap_or_else(|| panic!("预算表里要有 `{id}`（#219：三个数都要进报告）"));
            assert_eq!(budget.status, "recorded", "`{id}` 只记录不设门槛");
            assert_eq!(budget.bench.as_deref(), Some(bench));
        }
    }

    /// 私有提交再高也不会让 `--check-budgets` 失败：它是 `recorded`。
    /// 同一次采样里工作集超了才失败。
    #[test]
    fn a_high_private_commit_is_reported_but_does_not_fail_the_check() {
        let budgets = Budgets {
            schema: SCHEMA,
            size: None,
            budget: budgets()
                .budget
                .into_iter()
                .filter(|b| {
                    b.bench
                        .as_deref()
                        .is_some_and(|bench| bench.starts_with("startup/empty_window"))
                })
                .collect(),
        };
        assert_eq!(budgets.budget.len(), 3, "工作集 + 私有工作集 + 私有提交");

        let mut measured = Results::new();
        measured
            .metrics
            .insert(crate::startup::ID_EMPTY_WINDOW_WORKING_SET.into(), 54.9);
        measured
            .metrics
            .insert(crate::startup::ID_EMPTY_WINDOW_PRIVATE.into(), 121.6);
        measured.metrics.insert(
            crate::startup::ID_EMPTY_WINDOW_PRIVATE_WORKING_SET.into(),
            48.0,
        );
        assert_eq!(
            check(&budgets, &measured).unwrap(),
            0,
            "#150 实测的那组数字（工作集 54.9 / 私有提交 121.6 MB）按本口径是达标的"
        );

        // 工作集超了才失败。
        measured
            .metrics
            .insert(crate::startup::ID_EMPTY_WINDOW_WORKING_SET.into(), 61.0);
        assert_eq!(check(&budgets, &measured).unwrap(), 1);
    }
}
