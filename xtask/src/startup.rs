//! 冷启动测量（issue #4）。
//!
//! 启动 `rove.exe`，读它在首帧呈现时打印的 `FIRSTFRAME_SINCE_PROCESS_MS <ms>`
//! （#84 在 `rove-ui/src/skeleton/workbench.rs` 里加的），重复若干次取中位数，
//! 结果作为 `startup/first_frame` 写进基准结果，预算里用 `cold_start_first_frame` 记录。
//!
//! 「可交互」时间（首帧之后注入一个按键，记录它被处理并重新绘制完成的时刻）还没有埋点，
//! 留给 #16 的启动预算；见 `perf/budgets.toml` 的 `cold_start_interactive`。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::results::{BenchResult, Results};
use crate::util;

const MARKER: &str = "FIRSTFRAME_SINCE_PROCESS_MS";

pub struct Options {
    pub exe: PathBuf,
    pub project: PathBuf,
    pub tries: usize,
    pub timeout: Duration,
}

impl Options {
    /// 从参数里解析：`--exe`、`--project`、`--startup-tries`、`--startup-timeout`。
    pub fn from_args(args: &[String]) -> Result<Self, Box<dyn std::error::Error>> {
        let exe = util::flag(args, "--exe")
            .map(PathBuf::from)
            .unwrap_or_else(default_exe);
        let project = util::flag(args, "--project")
            .map(PathBuf::from)
            .unwrap_or_else(|| util::target_dir().join("perf-project"));
        let tries = match util::flag(args, "--startup-tries") {
            Some(raw) => raw.parse::<usize>()?,
            None => 5,
        };
        let timeout = match util::flag(args, "--startup-timeout") {
            Some(raw) => Duration::from_secs(raw.parse::<u64>()?),
            None => Duration::from_secs(30),
        };
        Ok(Options {
            exe,
            project,
            tries: tries.max(1),
            timeout,
        })
    }
}

pub fn default_exe() -> PathBuf {
    util::target_dir()
        .join("release")
        .join(if cfg!(windows) { "rove.exe" } else { "rove" })
}

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let options = Options::from_args(args)?;
    let require = util::has_flag(args, "--require");
    println!("冷启动测量（首帧已呈现）");
    println!("  exe：{}", options.exe.display());
    if !options.exe.is_file() {
        let message = format!(
            "找不到 {}：先构建 cargo build --release -p rove-app",
            options.exe.display()
        );
        if require {
            return Err(message.into());
        }
        println!("跳过：{message}");
        return Ok(0);
    }
    ensure_project(&options.project)?;
    println!("  项目：{}", options.project.display());
    let samples = measure(&options);
    if samples.is_empty() {
        let message = format!(
            "没有在 {:?} 内拿到 `{MARKER}`（没有桌面会话或窗口创建失败时会出现）",
            options.timeout
        );
        if require {
            return Err(message.into());
        }
        println!("跳过：{message}");
        return Ok(0);
    }

    let result = summarize(&samples).expect("样本非空");
    println!(
        "  首帧（进程启动 → 首帧已呈现）：median {}，min {}，max {}（n={}）",
        format_ms(result.mean_ns),
        format_ms(samples_min(&samples) * 1e6),
        format_ms(result.p99_ns.unwrap_or(result.mean_ns)),
        samples.len()
    );
    println!(
        "  注意：预算按「可交互」考核（首帧 + 第一个输入事件在一帧内响应），\
         「可交互」的埋点留给 #16；这里是首帧数据，单独记录。"
    );

    if let Some(path) = util::flag(args, "--save") {
        let path = PathBuf::from(path);
        let mut results = Results::new();
        results
            .benchmarks
            .insert("startup/first_frame".to_string(), result);
        results.save(&path)?;
        println!("已写入结果：{}", path.display());
    }
    Ok(0)
}

/// 测量一组首帧耗时（毫秒）；失败的尝试会被跳过并打印原因。
pub fn measure(options: &Options) -> Vec<f64> {
    let mut samples = Vec::with_capacity(options.tries);
    for index in 1..=options.tries {
        match measure_once(options) {
            Some(ms) => {
                println!("  第 {index} 次：{ms:.1} ms");
                samples.push(ms);
            }
            None => println!(
                "  第 {index} 次：{:?} 内没有拿到 `{MARKER}`，跳过",
                options.timeout
            ),
        }
    }
    samples
}

/// 把一组样本汇总成基准结果（`mean_ns` 存中位数，`p99_ns` 存最大值）。
pub fn summarize(samples: &[f64]) -> Option<BenchResult> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2];
    Some(BenchResult {
        mean_ns: median * 1e6,
        p99_ns: sorted.last().copied().map(|max| max * 1e6),
        std_dev_ns: None,
    })
}

fn samples_min(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(f64::INFINITY, f64::min)
}

fn measure_once(options: &Options) -> Option<f64> {
    let mut child = Command::new(&options.exe)
        .arg(&options.project)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix(MARKER) {
                if let Ok(value) = rest.trim().parse::<f64>() {
                    let _ = sender.send(value);
                }
                break;
            }
        }
    });
    let value = receiver.recv_timeout(options.timeout).ok();
    let _ = child.kill();
    let _ = child.wait();
    value
}

/// 没有指定项目时，造一个小目录（几个 C# 文件）让工作区扫描有东西可做。
pub fn ensure_project_public(project: &Path) -> std::io::Result<()> {
    ensure_project(project)
}

fn ensure_project(project: &Path) -> std::io::Result<()> {
    if project.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(project.join("Assets/Scripts"))?;
    for index in 0..3 {
        let path = project.join(format!("Assets/Scripts/Perf{index}.cs"));
        std::fs::write(
            path,
            format!(
                "namespace Perf\n{{\n    public class Perf{index}\n    {{\n        public int Value => {index};\n    }}\n}}\n"
            ),
        )?;
    }
    Ok(())
}

fn format_ms(ns: f64) -> String {
    format!("{:.1} ms", ns / 1e6)
}
