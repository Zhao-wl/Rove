//! `cargo xtask bench`：跑 criterion 基准、汇总结果、与基线比较、校验预算。
//!
//! 设计取舍：
//! - 基准都在 `xtask/benches/` 下，只依赖 #1 的公开接口，不给接口开测量后门；
//! - 结果从 criterion 的 `target/criterion/**/new/estimates.json` 汇总，格式见 `results.rs`；
//! - 相对比较只报警不失败（托管机器的波动比 15% 大得多），绝对判定交给 `perf/budgets.toml`
//!   里 `status = "enforced"` 的条目。

use std::path::PathBuf;
use std::process::Command;

use crate::budgets::{self, Budgets};
use crate::results::Results;
use crate::util;

/// 相对基线退化超过这个比例就报警（issue #4）。
const REGRESSION_THRESHOLD: f64 = 0.15;

/// 低于这个量级（纳秒）的用例不参与相对比较：托管 runner 的噪声比信号大，
/// 当前这些用例又都是占位实现（`EmptyText` / `EmptyWorkspace`）。
const DEFAULT_COMPARE_FLOOR_NS: f64 = 100.0;

struct BenchTarget {
    /// `cargo bench --bench <name>`
    name: &'static str,
    /// 说明，打印在跳过原因里。
    requires: &'static str,
}

const TARGETS: [BenchTarget; 4] = [
    BenchTarget {
        name: "text_source",
        requires: "",
    },
    BenchTarget {
        name: "workspace",
        requires: "",
    },
    BenchTarget {
        name: "cs_index",
        requires: "--cs-index <idx>（或环境变量 ROVE_PERF_CS_INDEX）",
    },
    BenchTarget {
        name: "scroll_unique",
        requires: "uniq.txt（先 cargo xtask gen-testdata --only uniq）",
    },
];

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let cs_index = util::flag(args, "--cs-index")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("ROVE_PERF_CS_INDEX").map(PathBuf::from));
    let testdata = match util::flag(args, "--testdata") {
        Some(dir) => std::fs::canonicalize(&dir)
            .map_err(|err| format!("--testdata 指向的目录不可用（{dir}）：{err}"))?,
        None => util::default_testdata_dir(),
    };
    let filter = util::flag(args, "--filter");
    let save = util::flag(args, "--save").map(PathBuf::from);
    let baseline = util::flag(args, "--baseline").map(PathBuf::from);
    let check_budgets = util::has_flag(args, "--check-budgets");
    let quick = util::has_flag(args, "--quick");

    if let Some(index) = &cs_index {
        if !index.is_file() {
            return Err(format!("--cs-index 指向的文件不存在：{}", index.display()).into());
        }
    }
    // `cargo bench` 运行基准程序时的工作目录是 xtask/，不是仓库根目录，
    // 所以索引路径必须先变成绝对路径再传给基准。
    let cs_index = match cs_index {
        Some(index) => Some(
            std::fs::canonicalize(&index)
                .map_err(|err| format!("无法解析索引路径 {}：{err}", index.display()))?,
        ),
        None => None,
    };

    let uniq_ready = testdata.join("uniq.txt").is_file();
    let from = util::flag(args, "--from").map(PathBuf::from);

    let current = match &from {
        Some(path) => {
            println!("使用已保存的结果：{}（不重新跑基准）", path.display());
            Results::load(path)?
        }
        None => {
            // 先清掉上一次的结果：否则 criterion 目录里的旧用例会混进本次汇总。
            let criterion_dir = util::target_dir().join("criterion");
            if criterion_dir.is_dir() {
                std::fs::remove_dir_all(&criterion_dir)?;
            }

            let mut ran = 0usize;
            for target in &TARGETS {
                let ready = match target.name {
                    "cs_index" => cs_index.is_some(),
                    "scroll_unique" => uniq_ready,
                    _ => true,
                };
                if !ready {
                    println!("跳过 {}：缺少 {}", target.name, target.requires);
                    continue;
                }
                run_target(
                    target.name,
                    filter.as_deref(),
                    quick,
                    cs_index.as_deref(),
                    &testdata,
                )?;
                ran += 1;
            }
            if ran == 0 {
                return Err("没有可运行的基准组".into());
            }
            let mut results = Results::from_criterion(&util::target_dir())?;

            // 冷启动（首帧）用独立测量：启动 release 版 rove.exe，读它打印的
            // FIRSTFRAME_SINCE_PROCESS_MS。没有 release 构建时跳过（不失败）。
            if !util::has_flag(args, "--no-startup") {
                let options = crate::startup::Options::from_args(args)?;
                if options.exe.is_file() {
                    crate::startup::ensure_project_public(&options.project)?;
                    println!("\n>>> 冷启动测量（{}）", options.exe.display());
                    let samples = crate::startup::measure(&options);
                    if let Some(result) = crate::startup::summarize(&samples) {
                        println!(
                            "  首帧 median {}（n={}）",
                            format_ns(result.mean_ns),
                            samples.len()
                        );
                        results
                            .benchmarks
                            .insert("startup/first_frame".to_string(), result);
                    } else {
                        println!("跳过 startup：没有拿到首帧标记（没有桌面会话时会这样）");
                    }
                } else {
                    println!(
                        "\n跳过 startup：找不到 {}（先 cargo build --release -p rove-app）",
                        options.exe.display()
                    );
                }
            }
            results
        }
    };
    println!("\n汇总：{} 个用例", current.benchmarks.len(),);
    for (id, result) in &current.benchmarks {
        println!("  {:<36} {}", id, format_ns(result.mean_ns));
    }
    if current.benchmarks.is_empty() {
        return Err("结果里没有任何用例，检查基准是否真的运行了".into());
    }

    if let Some(path) = &save {
        current.save(path)?;
        println!("已写入结果：{}", path.display());
    }

    let mut code = 0u8;
    if let Some(path) = &baseline {
        let base = Results::load(path)?;
        let floor_ns = match util::flag(args, "--compare-floor-ns") {
            Some(raw) => raw.parse::<f64>()?,
            None => DEFAULT_COMPARE_FLOOR_NS,
        };
        code |= compare(&base, &current, floor_ns);
    }
    if check_budgets {
        let budgets = Budgets::load_default()?;
        code |= budgets::check(&budgets, &current)?;
    }
    Ok(code)
}

fn run_target(
    name: &str,
    filter: Option<&str>,
    quick: bool,
    cs_index: Option<&std::path::Path>,
    testdata: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n>>> cargo bench -p xtask --bench {name}");
    let mut cmd = Command::new(cargo_exe());
    cmd.args(["bench", "-p", "xtask", "--bench", name, "--", "--noplot"]);
    if quick {
        cmd.args([
            "--sample-size",
            "20",
            "--measurement-time",
            "1",
            "--warm-up-time",
            "0.3",
        ]);
    }
    if let Some(filter) = filter {
        cmd.arg(filter);
    }
    if let Some(index) = cs_index {
        cmd.env("ROVE_PERF_CS_INDEX", index);
    }
    cmd.env("ROVE_PERF_TESTDATA", testdata);
    if let Ok(note) = std::env::var("ROVE_PERF_MACHINE_NOTE") {
        cmd.env("ROVE_PERF_MACHINE_NOTE", note);
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("基准 {name} 运行失败（{status}）").into());
    }
    Ok(())
}

/// 与基线比较：只报警、不失败（issue #4 明确不做绝对判定）。
///
/// `floor_ns` 以下的用例不参与比较：托管 runner 上的噪声比信号大得多
/// （占位基准量到的是 0.x ns 的取值调用，逐次运行的差异能有 100%+）。
fn compare(baseline: &Results, current: &Results, floor_ns: f64) -> u8 {
    println!(
        "\n与基线的比较（退化超过 {:.0}% 报警；低于 {floor_ns} ns 的用例视为噪声跳过）；只报警不失败：",
        REGRESSION_THRESHOLD * 100.0
    );
    println!(
        "  {:<36} {:>12} {:>12} {:>9}  状态",
        "用例", "基线", "当前", "变化"
    );
    let mut regressions = Vec::new();
    let mut skipped = 0usize;
    for (id, cur) in &current.benchmarks {
        let Some(base) = baseline.get(id) else {
            println!("  {:<36} 基线里没有这个用例，跳过", id);
            continue;
        };
        if base.mean_ns < floor_ns && cur.mean_ns < floor_ns {
            skipped += 1;
            println!("  {:<36} 低于 {floor_ns} ns，噪声不可比，跳过", id);
            continue;
        }
        let delta = cur.mean_ns / base.mean_ns - 1.0;
        let status = if delta > REGRESSION_THRESHOLD {
            regressions.push((id.clone(), delta));
            "退化"
        } else if delta < -REGRESSION_THRESHOLD {
            "变快"
        } else {
            "持平"
        };
        println!(
            "  {:<36} {:>12} {:>12} {:>+8.1}%  {}",
            id,
            format_ns(base.mean_ns),
            format_ns(cur.mean_ns),
            delta * 100.0,
            status
        );
    }
    println!(
        "  机器：基线 {} / 当前 {}（两个数字只有在同一台机器、同一 profile 下才可比）",
        describe_machine(baseline),
        describe_machine(current)
    );
    if skipped > 0 {
        println!("  跳过 {skipped} 个低于 {floor_ns} ns 的用例（噪声不可比；可用 --compare-floor-ns 调整）");
    }
    if regressions.is_empty() {
        println!("OK：没有超过 {:.0}% 的退化。", REGRESSION_THRESHOLD * 100.0);
        return 0;
    }
    for (id, delta) in &regressions {
        println!(
            "::warning::基准 {} 相对基线退化 {:.1}%（只报警，不失败）",
            id,
            delta * 100.0
        );
    }
    // 报警不改变退出码：托管机器的波动比 15% 大。
    0
}

fn describe_machine(results: &Results) -> String {
    let note = results
        .machine
        .note
        .as_deref()
        .map(|n| format!("，{n}"))
        .unwrap_or_default();
    format!(
        "{}/{}/{} 核{note}",
        results.machine.os, results.machine.arch, results.machine.cpus
    )
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

fn cargo_exe() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}
