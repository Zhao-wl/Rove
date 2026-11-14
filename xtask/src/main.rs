//! Rove 的开发任务入口（issue #4）：`cargo xtask <命令>`。
//!
//! 子命令：
//! - `gen-testdata`：生成 M0 口径的测试数据（`small.txt` / `uniq.txt` / `big1g.txt`）；
//! - `bench`：跑 criterion 基准，输出机器可读结果，可选与基线比较、校验预算；
//! - `check-size`：报告 `rove.exe` 与发布包体积，超预算时非零退出；
//! - `check-deps`：依赖图里禁止出现 HTTP / TLS 栈（#74）；
//! - `release` / `release-keygen` / `release-sign` / `release-verify` / `check-version`：
//!   本地发布流水线与更新清单签名（#76）；
//! - `licenses`：依赖许可汇总与守护（#76）；
//! - `icon`：生成应用图标（#76）。
//!
//! 基准只使用 #1（`docs/ARCHITECTURE.md`）定义的公开接口，不绕开接口测量，
//! 也不在接口上开测量后门。

mod bench;
mod budgets;
mod deps;
mod gen_testdata;
mod icon;
mod licenses;
mod pack;
mod release;
mod results;
mod sign;
mod size;
mod startup;
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
        [--compare-floor-ns N] [--no-startup] [--exe PATH] [--project DIR]
      跑基准。--save 写出机器可读结果；--baseline 与旧结果比较，退化超过 15% 报警；
      --check-budgets 按 perf/budgets.toml 校验，超预算时以非零码退出；
      --from 直接读已保存的结果（不重新跑基准），常用于复核预算与对比；
      --compare-floor-ns（默认 100）以下的用例不参与相对比较（噪声不可比）；
      --cs-index 也可以用环境变量 ROVE_PERF_CS_INDEX 指定。

  check-size [--binary PATH] [--baseline-bytes N] [--component ZIP]
      报告 rove.exe 体积与基线（默认取 perf/budgets.toml 里的 M0 基线）的差值，
      以及发布包（packaging/package-contents.toml）的体积与嵌入资产分项；
      发布包超过预算时以非零码退出。--component 只报告索引组件包的体积，不计入预算。

  release [--version X] [--skip-build] [--out DIR] [--component ZIP]
          [--repo OWNER/NAME] [--base-url URL] [--notes-url URL]
          [--require-installer] [--unsigned] [--binary PATH]
      本地发布（#76）：构建 release、组装便携版 zip、调用 Inno Setup 出安装包、
      算 SHA256、生成并用 ed25519 私钥签名 latest.json、写出 Release 说明与上传脚本。
      **不打 tag、不创建 Release、不上传**：上传要人工跑生成出来的 upload.ps1。
      --component ZIP 同时做两件事：让安装器多出「安装 C# 索引组件」勾选项（离线解压），
      并把组件包写进 SHA256SUMS.txt 与 latest.json 的 components（体积不计入 30 MB 预算）。

  check-version --tag v0.1.0
      校验 tag 与 Cargo.toml 的版本一致（不一致时非零退出）。

  release-keygen [--force]
      生成更新清单的 ed25519 密钥对：私钥写到 %LOCALAPPDATA%\\Rove-release\\update-signing.key
     （仓库外，绝不打印），公钥写到 packaging/release/update-signing.pub（提交进仓库）。

  release-sign --file PATH [--out PATH]
  release-verify --file PATH [--sig PATH] [--pubkey PATH]
      对任意文件做 / 验分离式 ed25519 签名（发布流水线对 latest.json 用的就是它）。

  licenses [--check] [--write]
      汇总 Rust 依赖的许可并按 packaging/licenses/allowed.toml 判定；
      --check 在出现未登记许可或汇总表过期时非零退出（CI 用）。

  icon [--check] [--out PATH] [--preview PNG]
      生成多尺寸应用图标 packaging/windows/rove.ico（含 256×256）。

  startup [--exe PATH] [--project DIR] [--startup-tries N] [--startup-timeout SECS]
          [--save PATH] [--require]
      测量冷启动到首帧：启动 rove.exe，读它打印的 FIRSTFRAME_SINCE_PROCESS_MS。
      需要先 cargo build --release -p rove-app；拿不到标记时默认跳过并返回 0，
      加 --require 则视为失败。「可交互」时间留给 #16。

  check-deps [--package CRATE]
      依赖图里禁止出现 reqwest / hyper / rustls / ring / tokio 等 HTTP / TLS 栈（#74）。
      默认检查 workspace 全体成员的 normal,build 与 normal,build,dev 两个范围，
      出现被禁的包时以非零码退出。确实去不掉的项加进 xtask/src/deps.rs 的 ALLOWED
      并写明来源与理由，不要直接从 DENIED 里删包名。

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
        "startup" => startup::run(rest),
        "check-size" => size::run(rest),
        "check-deps" => deps::run(rest),
        "release" => release::run(rest),
        "check-version" => release::check_version(rest),
        "release-keygen" => sign::keygen(rest),
        "release-sign" => sign::sign_command(rest),
        "release-verify" => sign::verify_command(rest),
        "licenses" => licenses::run(rest),
        "icon" => icon::run(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(0)
        }
        other => Err(format!(
            "未知命令 `{other}`，可用：gen-testdata / bench / startup / check-size / check-deps / \
             release / check-version / release-keygen / release-sign / release-verify / licenses / \
             icon"
        )
        .into()),
    }
}
