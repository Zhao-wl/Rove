//! Rove 的开发任务入口（issue #4）：`cargo xtask <命令>`。
//!
//! 子命令：
//! - `gen-testdata`：生成 M0 口径的测试数据（`small.txt` / `uniq.txt` / `big1g.txt`）；
//! - `bench`：跑 criterion 基准，输出机器可读结果，可选与基线比较、校验预算；
//! - `check-size`：报告 `rove.exe` 体积与基线差值，超预算时非零退出。
//!
//! 基准只使用 #1（`docs/ARCHITECTURE.md`）定义的公开接口，不绕开接口测量，
//! 也不在接口上开测量后门。

mod bench;
mod budgets;
mod gen_testdata;
mod results;
mod size;
mod util;

use std::process::ExitCode;

const USAGE: &str = "\
用法: cargo xtask <命令> [选项]

命令:
  gen-testdata [--out DIR] [--only small|uniq|big]
      生成测试数据。默认写到 target/perf-testdata（已在 .gitignore 里）。
      CI 上不要生成 big1g.txt。

  bench [--cs-index PATH] [--filter PATTERN] [--testdata DIR]
        [--save PATH] [--baseline PATH] [--check-budgets] [--quick] [--from PATH]
      跑基准。--save 写出机器可读结果；--baseline 与旧结果比较，退化超过 15% 报警；
      --check-budgets 按 perf/budgets.toml 校验，超预算时以非零码退出；
      --from 直接读已保存的结果（不重新跑基准），常用于复核预算与对比；
      --cs-index 也可以用环境变量 ROVE_PERF_CS_INDEX 指定。

  check-size [--binary PATH] [--baseline-bytes N]
      报告 rove.exe 体积与基线（默认取 perf/budgets.toml 里的 M0 基线）的差值，
      超过预算时以非零码退出。

  --help  显示本说明。";

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("xtask: {err}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        println!("{USAGE}");
        return Ok(0);
    };
    let rest = &args[1..];
    match command.as_str() {
        "gen-testdata" => gen_testdata::run(rest),
        "bench" => bench::run(rest),
        "check-size" => size::run(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(0)
        }
        other => Err(format!("未知命令 `{other}`，可用：gen-testdata / bench / check-size").into()),
    }
}
