//! v0.1 发布门槛的自检（#150）：`cargo xtask check-release-gate`。
//!
//! 目前判两件**机器能判**的事（其余门槛要人去做，见 `docs/V01_ACCEPTANCE_REPORT.md` §8）：
//!
//! 1. `perf/budgets.toml` 里**没有 `pending` 条目**，而且每条 `enforced` 都指得出测量来源
//!    （[MILESTONE_v0.1.md](../../docs/MILESTONE_v0.1.md) §5 第 5 条）；
//! 2. `docs/issues/COVERAGE.md` 中「v0.1」列为「是」的**每一行**都在
//!    `docs/V01_ACCEPTANCE_REPORT.md` 的核对表里有一行，且结论是「已完成」或「改判」
//!    （同上 §5 第 1 条）。
//!
//! 这两条同时也是 `cargo test -p xtask` 的用例：COVERAGE 改了而报告没跟上时，CI 直接会红。
//! **它判的是「有没有逐行给出结论」，不是「结论对不对」**——结论对不对要看报告里写的验证方式。

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::budgets::Budgets;
use crate::util;

pub const COVERAGE: &str = "docs/issues/COVERAGE.md";
pub const REPORT: &str = "docs/V01_ACCEPTANCE_REPORT.md";
/// 结论只允许这两种（issue #150 的原话）。
const CONCLUSIONS: [&str; 2] = ["已完成", "改判"];

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let root = util::repo_root();
    let coverage = util::flag(args, "--coverage")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(COVERAGE));
    let report = util::flag(args, "--report")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join(REPORT));

    println!("v0.1 发布门槛自检");
    let mut code = 0u8;

    // ---------------------------------------------------------------- 预算
    let budgets = Budgets::load_default()?;
    let pending: Vec<&str> = budgets
        .budget
        .iter()
        .filter(|b| b.status == "pending")
        .map(|b| b.id.as_str())
        .collect();
    if pending.is_empty() {
        println!(
            "  OK   perf/budgets.toml：{} 条，没有 pending",
            budgets.budget.len()
        );
    } else {
        code = 1;
        eprintln!("  失败 perf/budgets.toml 还有 pending 条目：{pending:?}");
        println!("::error::perf/budgets.toml 还有 pending 条目：{pending:?}");
    }

    // ---------------------------------------------------------------- 覆盖表
    let rows = v01_rows(&util::read_text(&coverage)?);
    let conclusions = report_conclusions(&util::read_text(&report)?);
    println!(
        "  ---- COVERAGE「v0.1 = 是」{} 行；报告里有结论的 {} 行",
        rows.len(),
        conclusions.len()
    );
    if rows.is_empty() {
        return Err(format!("{} 里没有「v0.1 = 是」的行，格式变了？", coverage.display()).into());
    }
    let mut missing = Vec::new();
    let mut wrong = Vec::new();
    for feature in &rows {
        match conclusions.get(feature) {
            None => missing.push(feature.clone()),
            Some(conclusion) if !CONCLUSIONS.contains(&conclusion.as_str()) => {
                wrong.push((feature.clone(), conclusion.clone()))
            }
            Some(_) => {}
        }
    }
    if missing.is_empty() && wrong.is_empty() {
        println!("  OK   {} 逐行都有「已完成 / 改判」的结论", REPORT);
    } else {
        code = 1;
        for feature in &missing {
            eprintln!("  失败 报告里没有这一行的结论：{feature}");
            println!("::error::{REPORT} 缺少 COVERAGE 行「{feature}」的结论");
        }
        for (feature, conclusion) in &wrong {
            eprintln!("  失败 「{feature}」的结论是「{conclusion}」，只允许 已完成 / 改判");
            println!("::error::{REPORT} 中「{feature}」的结论非法：{conclusion}");
        }
    }

    if code == 0 {
        println!("OK：机器能判的两条发布门槛都通过（其余见报告 §8 的人工清单）。");
    }
    Ok(code)
}

/// `COVERAGE.md` 里「v0.1」列为「是」的功能名（第 2 列）。
pub fn v01_rows(text: &str) -> Vec<String> {
    table_rows(text)
        .filter(|cells| cells.len() >= 5 && cells[3] == "是")
        .map(|cells| cells[1].to_string())
        .collect()
}

/// 报告里核对表的「功能 → 结论」（取表头含「功能」与「结论」的那张表）。
pub fn report_conclusions(text: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for cells in table_rows(text) {
        if cells.len() < 5 {
            continue;
        }
        // 只认与 COVERAGE 同形的 5 列表格：章节 | 功能 | 负责 issue | 验证方式 | 结论。
        if !cells[0].starts_with('§') {
            continue;
        }
        found.insert(cells[1].to_string(), cells[4].to_string());
    }
    found
}

/// Markdown 表格的数据行（去掉表头与分隔行）。
fn table_rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .filter(|line| line.starts_with('|'))
        .map(|line| {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect::<Vec<&str>>()
        })
        .filter(|cells| {
            !cells.is_empty()
                && !cells[0].starts_with("---")
                && cells[0] != "章节"
                && !cells.iter().all(|cell| cell.chars().all(|c| c == '-'))
        })
}

/// 测试用：仓库里的两份文件。
#[cfg(test)]
fn repo_file(name: &str) -> String {
    util::read_text(&util::repo_root().join(name)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 发布门槛 1：COVERAGE 的每一行在验收报告里都有「已完成 / 改判」的结论。
    #[test]
    fn the_report_answers_every_v01_coverage_row() {
        let rows = v01_rows(&repo_file(COVERAGE));
        assert!(rows.len() > 40, "COVERAGE 解析出来只有 {} 行", rows.len());
        let conclusions = report_conclusions(&repo_file(REPORT));
        let missing: Vec<&String> = rows
            .iter()
            .filter(|feature| !conclusions.contains_key(*feature))
            .collect();
        assert!(
            missing.is_empty(),
            "{REPORT} 里缺这些行的结论：{missing:?}（COVERAGE 改了就要同步改报告）"
        );
        for feature in &rows {
            let conclusion = &conclusions[feature];
            assert!(
                CONCLUSIONS.contains(&conclusion.as_str()),
                "「{feature}」的结论是「{conclusion}」，只允许 {CONCLUSIONS:?}"
            );
        }
    }

    /// 解析本身：表头、分隔行与非表格文本都不会混进来。
    #[test]
    fn only_data_rows_of_the_coverage_table_are_parsed() {
        let text = "\
# 标题

| 章节 | 功能 | 优先级 | v0.1 | 负责 issue |
|---|---|---|---|---|
| §2 | 活动栏 | P0 | 是 | R16 工作台（M1） |
| §3 | 缩略图 | P1 | 之后 | R55 缩略图（M4） |

正文。
";
        assert_eq!(v01_rows(text), vec!["活动栏".to_string()]);

        let report = "\
| 章节 | 功能 | 负责 issue | 验证方式 | 结论 |
|---|---|---|---|---|
| §2 | 活动栏 | R16 工作台（M1） | `cargo test -p rove-ui` | 已完成 |

| # | 场景 | 自动覆盖 | 结论 | 仍需人工 |
|---|---|---|---|---|
| 1 | 多光标 | 有 | 通过 | 录屏 |
";
        let conclusions = report_conclusions(report);
        assert_eq!(
            conclusions.get("活动栏").map(String::as_str),
            Some("已完成")
        );
        assert!(
            !conclusions.contains_key("多光标"),
            "场景表（第一列不是 §）不该被当成核对表"
        );
    }

    /// 发布门槛 5 的前半截（与 `budgets::tests::no_budget_is_still_pending` 是同一条线）。
    #[test]
    fn the_gate_fails_while_a_budget_is_pending() {
        let budgets = Budgets::load_default().unwrap();
        assert!(
            budgets.budget.iter().all(|b| b.status != "pending"),
            "perf/budgets.toml 还有 pending 条目"
        );
    }

    /// 报告里必须点名「可以发布 v0.1.0」或者列出阻塞项——两者至少有一个。
    #[test]
    fn the_report_states_either_a_go_or_the_blockers() {
        let report = repo_file(REPORT);
        assert!(
            report.contains("可以发布 v0.1.0") || report.contains("阻塞项"),
            "{REPORT} 既没有写「可以发布 v0.1.0」，也没有列出阻塞项"
        );
    }
}
