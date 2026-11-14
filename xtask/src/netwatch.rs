//! 一次会话的网络监视（#150，R77 §4）。
//!
//! 启动真实的 release 版 `rove.exe`（可带一个项目），在**独立的临时用户目录**里把
//! `rove.update.mode` 设成指定值，然后每隔一段时间用 `netstat -ano` 采样它整棵进程树
//!（含 C# 索引进程与 WebView2 进程）的连接，最后汇总出站连接。
//!
//! ```powershell
//! cargo xtask netwatch --mode none --seconds 60 --project <Unity 项目> --require-silent
//! cargo xtask netwatch --mode default --seconds 60
//! ```
//!
//! 判据（R77 §4）：
//!
//! - `--mode none`：整棵进程树**零出站连接**，`--require-silent` 下出现任何一条就以 1 退出；
//! - `--mode default`：只允许出现到 GitHub 的 HTTPS 连接，且只在检查 / 下载时出现。
//!   远端地址是 IP，这里**不做域名归属判定**（不解析 DNS、不发请求），只把采到的条目原样列出来，
//!   由人对照 `docs/UPDATE.md` §2 里登记的请求确认。
//!
//! **口径与限制**：采样看的是 TCP/UDP 连接表，不是 OS 级抓包。两次采样之间建立又关闭的连接
//! 可能漏掉；需要更强的证据时用 `pktmon`（要管理员权限）。这条限制必须写进验收报告。

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::procwatch;
use crate::startup;
use crate::util;

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let exe = util::flag(args, "--exe")
        .map(PathBuf::from)
        .unwrap_or_else(startup::default_exe);
    let mode = util::flag(args, "--mode").unwrap_or_else(|| "none".to_string());
    if !matches!(mode.as_str(), "none" | "manual" | "start" | "default") {
        return Err(format!(
            "--mode 只接受 none / manual / start / default（见 docs/UPDATE.md §3），收到 `{mode}`"
        )
        .into());
    }
    let seconds: u64 = match util::flag(args, "--seconds") {
        Some(raw) => raw.parse()?,
        None => 60,
    };
    let interval = Duration::from_millis(match util::flag(args, "--interval") {
        Some(raw) => raw.parse()?,
        None => 500,
    });
    let require_silent = util::has_flag(args, "--require-silent");
    let project = util::flag(args, "--project").map(PathBuf::from);

    println!("网络监视（rove.update.mode = {mode}，{seconds} s，每 {interval:?} 采一次）");
    println!("  exe：{}", exe.display());
    if !exe.is_file() {
        let message = format!(
            "找不到 {}：先构建 cargo build --release -p rove-app",
            exe.display()
        );
        if require_silent {
            return Err(message.into());
        }
        println!("跳过：{message}");
        return Ok(0);
    }

    let user = util::target_dir().join("netwatch-user");
    std::fs::create_dir_all(&user)?;
    std::fs::write(
        user.join("settings.json"),
        format!("{{\n  \"rove.update.mode\": \"{mode}\"\n}}\n"),
    )?;

    let mut command = Command::new(&exe);
    command.arg("--vscode-user-dir").arg(&user);
    if let Some(project) = &project {
        println!("  项目：{}", project.display());
        command.arg(project);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let root = child.id();

    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut outbound: BTreeSet<String> = BTreeSet::new();
    let mut tree_names: BTreeSet<String> = BTreeSet::new();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut samples = 0usize;
    while Instant::now() < deadline {
        let all = procwatch::snapshot();
        let tree = procwatch::tree(&all, root);
        if tree.is_empty() {
            // 进程已经退出（或拿不到进程表）：别再空转。
            break;
        }
        for process in tree.values() {
            tree_names.insert(process.name.clone());
        }
        let pids: BTreeSet<u32> = tree.keys().copied().collect();
        let lines = procwatch::connections(&pids);
        for line in procwatch::outbound(&lines) {
            outbound.insert(line);
        }
        for line in lines {
            seen.insert(line);
        }
        samples += 1;
        std::thread::sleep(interval);
    }
    let _ = child.kill();
    let _ = child.wait();

    println!("  采样 {samples} 次；进程树里出现过：{tree_names:?}");
    println!("  连接表里属于这棵进程树的条目：{} 条", seen.len());
    for line in &seen {
        println!("    {line}");
    }
    println!("  其中**出站**连接：{} 条", outbound.len());
    for line in &outbound {
        println!("    {line}");
    }
    println!(
        "  口径：netstat 连接表采样，不是抓包；两次采样之间建立又关闭的连接可能漏掉（见本命令的文档注释）"
    );

    if mode == "none" && !outbound.is_empty() {
        eprintln!(
            "rove.update.mode = none 下出现了 {} 条出站连接，违反 PROJECT_CHARTER §2.1",
            outbound.len()
        );
        if require_silent {
            return Ok(1);
        }
    }
    Ok(0)
}
