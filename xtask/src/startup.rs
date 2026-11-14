//! 冷启动与空窗口内存的测量（issue #4、#150）。
//!
//! 三个数字，都由启动真实的 release 版 `rove.exe` 得到：
//!
//! | 结果 id | 口径 | 预算 |
//! |---|---|---|
//! | `startup/first_frame` | 进程启动 → 首帧已呈现（`FIRSTFRAME_SINCE_PROCESS_MS`，#84 埋的） | `cold_start_first_frame`（只记录） |
//! | `startup/interactive` | 进程启动 → 首帧之后的第一个输入事件被处理且包含它的那一帧画完（`INTERACTIVE_SINCE_PROCESS_MS`，#150 埋的） | `cold_start_interactive` |
//! | `startup/empty_window_working_set`（metric，MB） | 空窗口（不带任何路径参数）稳定后 `rove.exe` 进程树的工作集 | `empty_window_memory` |
//!
//! 埋点默认关闭：这里启动子进程时设置 `ROVE_STARTUP_PROBE=1`（见
//! `rove-ui/src/skeleton/workbench/startup_probe.rs`），普通使用的 `rove.exe` 不会多出一次按键。
//!
//! 测量用的是**独立的临时用户目录**（`--vscode-user-dir`），里面只写
//! `"rove.update.mode": "none"`：这样启动过程不受本机设置影响，也不会在测量期间联网。
//! 「按键到上屏」不含 Windows 键盘消息的排队时间（按键经 `Window::dispatch_keystroke` 注入），
//! 和 #47 的 `typing/key_to_frame` 同口径。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::procwatch::{self, Memory};
use crate::results::{BenchResult, Results};
use crate::util;

const FIRST_FRAME: &str = "FIRSTFRAME_SINCE_PROCESS_MS";
const INTERACTIVE: &str = "INTERACTIVE_SINCE_PROCESS_MS";
/// 首帧之后再等多久「可交互」标记：等不到就当这个 exe 没带埋点。
const PROBE_WAIT: Duration = Duration::from_secs(10);

/// 结果 id（`perf/budgets.toml` 的 `bench` 字段指向它们）。
pub const ID_FIRST_FRAME: &str = "startup/first_frame";
pub const ID_INTERACTIVE: &str = "startup/interactive";
pub const ID_EMPTY_WINDOW_WORKING_SET: &str = "startup/empty_window_working_set";
pub const ID_EMPTY_WINDOW_PRIVATE: &str = "startup/empty_window_private";

pub struct Options {
    pub exe: PathBuf,
    pub project: PathBuf,
    pub tries: usize,
    pub timeout: Duration,
    /// 空窗口内存：启动后等多久再采样（让后台扫描与首次绘制稳定下来）。
    pub settle: Duration,
    /// 跳过空窗口内存测量（只要启动耗时时用）。
    pub skip_memory: bool,
}

impl Options {
    /// 从参数里解析：`--exe`、`--project`、`--startup-tries`、`--startup-timeout`、
    /// `--startup-settle`、`--no-memory`。
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
        let settle = match util::flag(args, "--startup-settle") {
            Some(raw) => Duration::from_millis(raw.parse::<u64>()?),
            None => Duration::from_millis(2_000),
        };
        Ok(Options {
            exe,
            project,
            tries: tries.max(1),
            timeout,
            settle,
            skip_memory: util::has_flag(args, "--no-memory"),
        })
    }
}

pub fn default_exe() -> PathBuf {
    util::target_dir()
        .join("release")
        .join(if cfg!(windows) { "rove.exe" } else { "rove" })
}

/// 一次启动拿到的数字。
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub first_frame_ms: f64,
    /// 没有拿到「可交互」标记时为 `None`（例如没有桌面会话）。
    pub interactive_ms: Option<f64>,
    pub memory: Option<Memory>,
}

/// 一组启动的全部样本。
#[derive(Debug, Default)]
pub struct Samples {
    pub first_frame_ms: Vec<f64>,
    pub interactive_ms: Vec<f64>,
    pub empty_window: Vec<Memory>,
}

impl Samples {
    pub fn is_empty(&self) -> bool {
        self.first_frame_ms.is_empty() && self.empty_window.is_empty()
    }

    /// 写进基准结果：两条耗时进 `benchmarks`，内存进 `metrics`（单位 MB）。
    pub fn write_into(&self, results: &mut Results) {
        if let Some(result) = summarize(&self.first_frame_ms) {
            results
                .benchmarks
                .insert(ID_FIRST_FRAME.to_string(), result);
        }
        if let Some(result) = summarize(&self.interactive_ms) {
            results
                .benchmarks
                .insert(ID_INTERACTIVE.to_string(), result);
        }
        if let Some(memory) = median_memory(&self.empty_window) {
            results.metrics.insert(
                ID_EMPTY_WINDOW_WORKING_SET.to_string(),
                memory.working_set_mb(),
            );
            results
                .metrics
                .insert(ID_EMPTY_WINDOW_PRIVATE.to_string(), memory.private_mb());
        }
    }
}

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let options = Options::from_args(args)?;
    let require = util::has_flag(args, "--require");
    println!("冷启动测量（首帧、可交互、空窗口内存）");
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
            "没有在 {:?} 内拿到 `{FIRST_FRAME}`（没有桌面会话或窗口创建失败时会出现）",
            options.timeout
        );
        if require {
            return Err(message.into());
        }
        println!("跳过：{message}");
        return Ok(0);
    }

    if let Some(result) = summarize(&samples.first_frame_ms) {
        println!(
            "  首帧（进程启动 → 首帧已呈现）：median {}，min {}，max {}（n={}）",
            format_ms(result.mean_ns),
            format_ms(min(&samples.first_frame_ms) * 1e6),
            format_ms(result.p99_ns.unwrap_or(result.mean_ns)),
            samples.first_frame_ms.len()
        );
    }
    match summarize(&samples.interactive_ms) {
        Some(result) => println!(
            "  可交互（首帧 + 第一个输入事件处理完并画完那一帧）：median {}，max {}（n={}）",
            format_ms(result.mean_ns),
            format_ms(result.p99_ns.unwrap_or(result.mean_ns)),
            samples.interactive_ms.len()
        ),
        None => println!(
            "  可交互：没有拿到 `{INTERACTIVE}`（需要 release 版 rove.exe 带 #150 的埋点）"
        ),
    }
    match median_memory(&samples.empty_window) {
        Some(memory) => println!(
            "  空窗口常驻内存（稳定 {:?} 后）：工作集 {:.1} MB，私有提交 {:.1} MB（n={}）",
            options.settle,
            memory.working_set_mb(),
            memory.private_mb(),
            samples.empty_window.len()
        ),
        None if options.skip_memory => println!("  空窗口常驻内存：按 --no-memory 跳过"),
        None => println!("  空窗口常驻内存：没有采到（拿不到进程表时会这样）"),
    }
    println!("  注意：数字受机器负载影响，报告里要写明当时的负载；按键不经系统消息队列。");

    if let Some(path) = util::flag(args, "--save") {
        let path = PathBuf::from(path);
        let mut results = Results::new();
        samples.write_into(&mut results);
        results.save(&path)?;
        println!("已写入结果：{}", path.display());
    }
    Ok(0)
}

/// 跑一组启动：前 `tries` 次带项目测耗时，再 `memory_tries` 次空窗口测内存。
pub fn measure(options: &Options) -> Samples {
    let mut samples = Samples::default();
    let user = temp_user_dir();
    for index in 1..=options.tries {
        match measure_once(options, Some(&options.project), user.as_deref(), false) {
            Some(sample) => {
                match sample.interactive_ms {
                    Some(interactive) => println!(
                        "  第 {index} 次：首帧 {:.1} ms，可交互 {interactive:.1} ms",
                        sample.first_frame_ms
                    ),
                    None => println!("  第 {index} 次：首帧 {:.1} ms", sample.first_frame_ms),
                }
                samples.first_frame_ms.push(sample.first_frame_ms);
                if let Some(interactive) = sample.interactive_ms {
                    samples.interactive_ms.push(interactive);
                }
            }
            None => println!(
                "  第 {index} 次：{:?} 内没有拿到 `{FIRST_FRAME}`，跳过",
                options.timeout
            ),
        }
    }
    if !options.skip_memory {
        let memory_tries = options.tries.min(3);
        for index in 1..=memory_tries {
            match measure_once(options, None, user.as_deref(), true) {
                Some(Sample {
                    memory: Some(memory),
                    ..
                }) => {
                    println!(
                        "  空窗口第 {index} 次：工作集 {:.1} MB，私有提交 {:.1} MB",
                        memory.working_set_mb(),
                        memory.private_mb()
                    );
                    samples.empty_window.push(memory);
                }
                _ => println!("  空窗口第 {index} 次：没有采到内存，跳过"),
            }
        }
    }
    samples
}

/// 汇总一组样本（`mean_ns` 存中位数，`p99_ns` 存最大值）。
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

/// 一组内存样本的中位数（按工作集排序）。
pub fn median_memory(samples: &[Memory]) -> Option<Memory> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by_key(|memory| memory.working_set);
    Some(sorted[sorted.len() / 2])
}

fn min(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(f64::INFINITY, f64::min)
}

/// 启动一次 `rove.exe`：读两个标记，必要时在稳定之后采一次内存。
fn measure_once(
    options: &Options,
    project: Option<&Path>,
    user_dir: Option<&Path>,
    memory: bool,
) -> Option<Sample> {
    let mut command = Command::new(&options.exe);
    if let Some(user_dir) = user_dir {
        command.arg("--vscode-user-dir").arg(user_dir);
    }
    if let Some(project) = project {
        command.arg(project);
    }
    let mut child = command
        .env("ROVE_STARTUP_PROBE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let pid = child.id();
    let stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(value) = marker(&line, FIRST_FRAME) {
                let _ = sender.send((FIRST_FRAME, value));
            } else if let Some(value) = marker(&line, INTERACTIVE) {
                let _ = sender.send((INTERACTIVE, value));
                break;
            }
        }
    });

    let mut sample = Sample::default();
    let mut has_first_frame = false;
    // 首帧可能要等整个超时；「可交互」紧跟在首帧之后，等不到就说明这个 exe 没有 #150 的埋点，
    // 短等一会儿就放弃（否则每次都要白等一个完整的超时）。
    let mut left = options.timeout;
    while let Ok((name, value)) = receiver.recv_timeout(left) {
        if name == FIRST_FRAME {
            sample.first_frame_ms = value;
            has_first_frame = true;
            left = PROBE_WAIT.min(options.timeout);
        } else {
            sample.interactive_ms = Some(value);
            break;
        }
    }
    if has_first_frame && memory {
        std::thread::sleep(options.settle);
        let all = procwatch::snapshot();
        let tree = procwatch::tree(&all, pid);
        if !tree.is_empty() {
            sample.memory = Some(procwatch::total(&tree));
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    has_first_frame.then_some(sample)
}

/// `<标记> <数值>`（同一行后面还可以跟别的字段，例如 `KEY_TO_FRAME_MS`）。
fn marker(line: &str, name: &str) -> Option<f64> {
    line.strip_prefix(name)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// 测量用的临时用户目录：只写 `rove.update.mode = "none"`，避免测量期间联网。
fn temp_user_dir() -> Option<PathBuf> {
    let dir = util::target_dir().join("startup-user");
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(
        dir.join("settings.json"),
        "{\n  \"rove.update.mode\": \"none\"\n}\n",
    )
    .ok()?;
    Some(dir)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_read_from_the_line_even_with_extra_fields() {
        assert_eq!(
            marker("FIRSTFRAME_SINCE_PROCESS_MS 396.5", FIRST_FRAME),
            Some(396.5)
        );
        assert_eq!(
            marker(
                "INTERACTIVE_SINCE_PROCESS_MS 402.125 KEY_TO_FRAME_MS 5.5",
                INTERACTIVE
            ),
            Some(402.125)
        );
        assert_eq!(marker("something else", FIRST_FRAME), None);
        assert_eq!(marker("FIRSTFRAME_SINCE_PROCESS_MS", FIRST_FRAME), None);
    }

    #[test]
    fn the_median_is_taken_over_the_working_set() {
        let memory = |ws: u64| Memory {
            working_set: ws << 20,
            private: ws << 20,
        };
        let median = median_memory(&[memory(70), memory(40), memory(50)]).unwrap();
        assert_eq!(median.working_set_mb(), 50.0);
        assert!(median_memory(&[]).is_none());
    }
}
