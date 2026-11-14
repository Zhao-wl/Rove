//! `cargo xtask check-size`：体积守护（issue #4 建立，#76 扩展到整个发布包）。
//!
//! 每个 PR 报告两个数：
//! - `rove.exe` 的体积与相对基线（M0 实测 6.2 MB）的变化；
//! - **发布包**（`packaging/package-contents.toml` 里登记的全部文件）的体积，
//!   以及嵌入 `rove.exe` 的资产分项（主题、语言配置、codicons、Markdown 预览资源……）。
//!
//! 判定对象是**发布包**：超过 `perf/budgets.toml` 的 `[size].budget_bytes`
//!（CHARTER §6：不含 C# 索引组件的发布包 < 30 MB）时以非零码退出。
//! C# 索引组件是独立的可选包，不计入这个预算（`--component` 只报告、不判定）。

use std::fs;
use std::path::PathBuf;

use crate::budgets::{Budgets, SizeBudget};
use crate::pack::Contents;
use crate::release;
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

    println!("体积守护（issue #4 / #76）");
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

    // ---------------------------------------------------------------- 发布包
    println!("\n发布包（packaging/package-contents.toml，不含 C# 索引组件）");
    let contents = Contents::load_default()?;
    let files = contents.resolve(Some(&exe))?;
    let package = release::print_package_report(&files, &contents)?;

    // 可选：C# 索引组件包只报告、不计入预算（CHARTER §6）。
    if let Some(component) = util::flag(args, "--component") {
        let component = PathBuf::from(component);
        match fs::metadata(&component) {
            Ok(meta) => println!(
                "\nC# 索引组件包（可选，不计入 30 MB 预算）：{} = {}",
                component.display(),
                util::human_bytes(meta.len())
            ),
            Err(err) => println!(
                "\n::warning::读不到索引组件包 {}：{err}",
                component.display()
            ),
        }
    }

    // ::notice:: 会出现在 PR 的检查注解里，让体积变化在 PR 页面就能看到。
    println!(
        "\n::notice::rove.exe {}（{} B，相对 M0 基线 {:+.2}%）；发布包 {}（占预算 {:.1}%）",
        util::human_bytes(size),
        util::thousands(size),
        percent_delta(size, baseline),
        util::human_bytes(package),
        package as f64 / budget as f64 * 100.0
    );

    verdict(&size_budget, package, budget)
}

/// 判定发布包是否超预算。
fn verdict(
    size_budget: &SizeBudget,
    package: u64,
    budget: u64,
) -> Result<u8, Box<dyn std::error::Error>> {
    let over_budget = package > budget;
    match size_budget.status.as_str() {
        "enforced" => {
            if over_budget {
                println!(
                    "::error::发布包体积 {} 超过预算 {}（超出 {:.2}%）",
                    util::human_bytes(package),
                    util::human_bytes(budget),
                    (package as f64 / budget as f64 - 1.0) * 100.0
                );
                return Ok(1);
            }
            println!(
                "OK：发布包在预算内（占预算 {:.1}%）。",
                package as f64 / budget as f64 * 100.0
            );
            Ok(0)
        }
        "pending" | "recorded" => {
            if over_budget {
                println!(
                    "::warning::发布包体积 {} 超过预算 {}，但该预算状态是 `{}`：超限只报警、不失败",
                    util::human_bytes(package),
                    util::human_bytes(budget),
                    size_budget.status
                );
            } else {
                println!(
                    "OK：发布包在预算内（预算状态 `{}`，超限暂不失败）。",
                    size_budget.status
                );
            }
            Ok(0)
        }
        other => Err(format!(
            "perf/budgets.toml 的 [size].status = `{other}` 非法（只接受 enforced / pending / recorded）"
        )
        .into()),
    }
}

fn percent_delta(actual: u64, baseline: u64) -> f64 {
    if baseline == 0 {
        return 0.0;
    }
    (actual as f64 / baseline as f64 - 1.0) * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(status: &str) -> SizeBudget {
        SizeBudget {
            budget_bytes: 30_000_000,
            baseline_bytes: 6_200_000,
            owner: "#76".into(),
            status: status.into(),
            reason: None,
        }
    }

    /// 往资产目录里塞个大文件 → 发布包超过 30 MB → 检查以非零码退出（issue #76 验收标准）。
    #[test]
    fn package_over_budget_fails_when_enforced() {
        assert_eq!(
            verdict(&budget("enforced"), 29_999_999, 30_000_000).unwrap(),
            0
        );
        assert_eq!(
            verdict(&budget("enforced"), 30_000_001, 30_000_000).unwrap(),
            1
        );
        // 预算状态不是 enforced 时只报警。
        assert_eq!(
            verdict(&budget("recorded"), 40_000_000, 30_000_000).unwrap(),
            0
        );
        assert!(verdict(&budget("nonsense"), 1, 30_000_000).is_err());
    }
}
