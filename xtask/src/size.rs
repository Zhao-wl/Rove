//! `cargo xtask check-size`：体积守护（issue #4）。
//!
//! 每个 PR 报告 `rove.exe` 的体积及相对基线（M0 实测 6.2 MB）的变化；
//! 超过 `perf/budgets.toml` 的 `[size].budget_bytes` 时以非零码退出。
//! 发布包的完整组成到 #75 才确定，届时由它把检查对象扩展到整个发布包。

use std::fs;
use std::path::PathBuf;

use crate::budgets::Budgets;
use crate::util;

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let budgets = Budgets::load_default()?;
    let size_budget = budgets
        .size
        .ok_or("perf/budgets.toml 缺少 [size] 段（体积守护需要 budget_bytes / baseline_bytes）")?;

    let exe = util::flag(args, "--binary")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            util::target_dir()
                .join("release")
                .join(if cfg!(windows) { "rove.exe" } else { "rove" })
        });
    let metadata = fs::metadata(&exe).map_err(|err| {
        format!(
            "找不到 {}：{err}\n先构建：cargo build --release -p rove-app",
            exe.display()
        )
    })?;
    let size = metadata.len();

    let baseline = match util::flag(args, "--baseline-bytes") {
        Some(raw) => raw.parse::<u64>()?,
        None => size_budget.baseline_bytes,
    };
    let budget = size_budget.budget_bytes;

    println!("体积守护（issue #4）");
    println!("  文件：{}", exe.display());
    println!(
        "  体积：{}（{} B）",
        util::human_bytes(size),
        util::thousands(size)
    );
    println!(
        "  基线：{}（M0 实测，{} B）→ 变化 {:+.2}%（{:+.0} B）",
        util::human_bytes(baseline),
        util::thousands(baseline),
        percent_delta(size, baseline),
        size as i64 - baseline as i64
    );
    println!(
        "  预算：{}（{} B，负责 {} / {}）",
        util::human_bytes(budget),
        util::thousands(budget),
        size_budget.owner,
        size_budget.status
    );
    if let Some(reason) = &size_budget.reason {
        println!("  说明：{reason}");
    }

    if size > budget {
        println!(
            "::error::rove.exe 体积 {} 超过预算 {}（超出 {:.2}%）",
            util::human_bytes(size),
            util::human_bytes(budget),
            (size as f64 / budget as f64 - 1.0) * 100.0
        );
        return Ok(1);
    }
    println!(
        "OK：体积在预算内（占预算 {:.1}%）。",
        size as f64 / budget as f64 * 100.0
    );
    Ok(0)
}

fn percent_delta(actual: u64, baseline: u64) -> f64 {
    if baseline == 0 {
        return 0.0;
    }
    (actual as f64 / baseline as f64 - 1.0) * 100.0
}
