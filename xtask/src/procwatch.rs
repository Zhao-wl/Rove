//! 进程树的内存与网络连接采样（#150）。
//!
//! 两个用途：
//!
//! - `cargo xtask startup`：空窗口的常驻内存（预算 `empty_window_memory`）；
//! - `cargo xtask netwatch`：一次会话里 `rove.exe` 进程树的出站连接
//!   （R77 §4 的「`rove.update.mode = "none"` 时零出站连接」）。
//!
//! 取数方式和 `rove-ui` 的 `markdown_probe` 例子一致：`Get-CimInstance Win32_Process` 拿
//! 进程树与内存，`netstat -ano` 拿连接。**这不是 OS 级抓包**：它看不到已经关闭的瞬时连接，
//! 也看不到不经过 TCP/UDP 表的流量；需要更强的证据时用 `pktmon`（要管理员权限）。
//! 报告里必须照这个口径写。
//!
//! # 三个内存数各是什么（#219 确认的口径）
//!
//! | 字段 | 计数器 | 含义 |
//! |---|---|---|
//! | `working_set` | `Win32_Process.WorkingSetSize` | 进程当前**驻留在物理内存**里的页，含共享页（exe / DLL / 字体等映射文件） |
//! | `private_working_set` | `Win32_PerfRawData_PerfProc_Process.WorkingSetPrivate` | 工作集里**私有**的那部分（任务管理器「内存(专用工作集)」列） |
//! | `private` | `Win32_Process.PrivatePageCount` | 私有**提交量**（commit charge）：已向系统预订的私有地址空间，**不要求驻留物理内存** |
//!
//! 「常驻内存」判定用 `working_set`（`private` 里包含从未被触碰、或已被换出的提交页，
//! 不是常驻量）；`private` 与 `private_working_set` 同时记录，三个数都要写进报告。
//! 详见 `perf/README.md` 的采集口径第 3 条与 `docs/PROJECT_CHARTER.md` §6。

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

/// 一个进程的内存（字节）。
#[derive(Debug, Clone, Copy, Default)]
pub struct Memory {
    /// 工作集：驻留在物理内存里的页（含共享页）。**判定口径**。
    pub working_set: u64,
    /// 私有提交量（commit charge），不等于驻留量。
    pub private: u64,
    /// 私有工作集：私有提交里真正驻留的部分；拿不到性能计数器时为 0。
    pub private_working_set: u64,
}

impl Memory {
    pub fn working_set_mb(&self) -> f64 {
        self.working_set as f64 / 1024.0 / 1024.0
    }
    pub fn private_mb(&self) -> f64 {
        self.private as f64 / 1024.0 / 1024.0
    }
    pub fn private_working_set_mb(&self) -> f64 {
        self.private_working_set as f64 / 1024.0 / 1024.0
    }
    /// 已提交但**没有驻留**的私有内存（`private − private_working_set`）：
    /// 保留给图形栈与分配器、却从未落到物理内存上的那部分。
    /// 没拿到私有工作集（计数器不可用）时返回 `None`，绝不拿 0 当真值算差值。
    pub fn private_not_resident_mb(&self) -> Option<f64> {
        (self.private_working_set > 0)
            .then(|| self.private.saturating_sub(self.private_working_set) as f64 / 1024.0 / 1024.0)
    }
}

/// `Get-CimInstance Win32_Process` 的一行。
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
    pub memory: Memory,
}

/// 当前所有进程；拿不到（没有 PowerShell、权限不足）时返回空表。
pub fn snapshot() -> Vec<ProcessInfo> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,WorkingSetSize,PrivatePageCount | ConvertTo-Json -Compress",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let value: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let rows = match value {
        serde_json::Value::Array(rows) => rows,
        single => vec![single],
    };
    rows.iter()
        .filter_map(|row| {
            Some(ProcessInfo {
                pid: row["ProcessId"].as_u64()? as u32,
                parent: row["ParentProcessId"].as_u64().unwrap_or(0) as u32,
                name: row["Name"].as_str().unwrap_or_default().to_string(),
                memory: Memory {
                    working_set: row["WorkingSetSize"].as_u64().unwrap_or(0),
                    private: row["PrivatePageCount"].as_u64().unwrap_or(0),
                    private_working_set: 0,
                },
            })
        })
        .collect()
}

/// 每个进程的**私有工作集**（`Win32_PerfRawData_PerfProc_Process.WorkingSetPrivate`）。
///
/// 单独一次查询：`Win32_Process` 里没有这个计数器，而 `netwatch` 的轮询用不到它，
/// 不该让每次轮询都多付一次性能计数器的开销。计数器被禁用或拿不到时返回空表
///（调用方按「没测到」处理，不要当 0）。
pub fn private_working_sets() -> BTreeMap<u32, u64> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_PerfRawData_PerfProc_Process -ErrorAction SilentlyContinue | \
             Select-Object IDProcess,WorkingSetPrivate | ConvertTo-Json -Compress",
        ])
        .output();
    let Ok(output) = output else {
        return BTreeMap::new();
    };
    parse_private_working_sets(&String::from_utf8_lossy(&output.stdout))
}

/// `private_working_sets` 的解析部分（单独拆出来便于测试）。
pub fn parse_private_working_sets(text: &str) -> BTreeMap<u32, u64> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return BTreeMap::new();
    };
    let rows = match value {
        serde_json::Value::Array(rows) => rows,
        serde_json::Value::Null => Vec::new(),
        single => vec![single],
    };
    rows.iter()
        .filter_map(|row| {
            let pid = row["IDProcess"].as_u64()? as u32;
            // pid 0（Idle）在性能计数器里也占一行，留着没有意义。
            (pid != 0).then(|| (pid, row["WorkingSetPrivate"].as_u64().unwrap_or(0)))
        })
        .collect()
}

/// 给一棵进程树补上私有工作集（`private_working_sets` 的结果）。
pub fn apply_private_working_sets(
    tree: &mut BTreeMap<u32, ProcessInfo>,
    private_working_sets: &BTreeMap<u32, u64>,
) {
    for (pid, process) in tree.iter_mut() {
        process.memory.private_working_set =
            private_working_sets.get(pid).copied().unwrap_or_default();
    }
}

/// `root` 及其全部后代进程。
pub fn tree(all: &[ProcessInfo], root: u32) -> BTreeMap<u32, ProcessInfo> {
    let mut tree: BTreeMap<u32, ProcessInfo> = BTreeMap::new();
    if let Some(info) = all.iter().find(|p| p.pid == root) {
        tree.insert(root, info.clone());
    }
    loop {
        let before = tree.len();
        for process in all {
            if tree.contains_key(&process.parent) && !tree.contains_key(&process.pid) {
                tree.insert(process.pid, process.clone());
            }
        }
        if tree.len() == before {
            break;
        }
    }
    tree
}

/// 一棵进程树的内存合计。
pub fn total(tree: &BTreeMap<u32, ProcessInfo>) -> Memory {
    tree.values().fold(Memory::default(), |mut sum, process| {
        sum.working_set += process.memory.working_set;
        sum.private += process.memory.private;
        sum.private_working_set += process.memory.private_working_set;
        sum
    })
}

/// `netstat -ano` 中属于这些进程的 TCP / UDP 条目（原样保留，行内空白压成一个空格）。
pub fn connections(pids: &BTreeSet<u32>) -> Vec<String> {
    if pids.is_empty() {
        return Vec::new();
    }
    let Ok(output) = Command::new("netstat").arg("-ano").output() else {
        return Vec::new();
    };
    parse_netstat(&String::from_utf8_lossy(&output.stdout), pids)
}

/// 从 `netstat -ano` 的输出里挑出属于这些进程的行。
pub fn parse_netstat(text: &str, pids: &BTreeSet<u32>) -> Vec<String> {
    text.lines()
        .filter(|line| {
            line.split_whitespace()
                .last()
                .and_then(|pid| pid.parse::<u32>().ok())
                .is_some_and(|pid| pids.contains(&pid))
        })
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

/// 只保留**出站**连接：本机监听（LISTENING）与本地回环之间的连接不算出网。
pub fn outbound(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            // TCP  <本地>  <远端>  <状态>  <PID>
            let Some(remote) = fields.get(2) else {
                return false;
            };
            if line.contains("LISTENING") {
                return false;
            }
            let host = remote.rsplit_once(':').map(|(host, _)| host).unwrap_or("");
            let host = host.trim_start_matches('[').trim_end_matches(']');
            !(host.is_empty()
                || host == "*"
                || host == "0.0.0.0"
                || host == "::"
                || host == "::1"
                || host.starts_with("127."))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, ws: u64, private: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent,
            name: format!("p{pid}.exe"),
            memory: Memory {
                working_set: ws,
                private,
                private_working_set: 0,
            },
        }
    }

    #[test]
    fn the_tree_collects_every_descendant_and_sums_their_memory() {
        let all = vec![
            p(1, 0, 1, 1),
            p(10, 1, 2 << 20, 1 << 20),
            p(11, 10, 1 << 20, 1 << 20),
            p(12, 99, 8 << 20, 8 << 20),
        ];
        let tree = tree(&all, 10);
        assert_eq!(tree.keys().copied().collect::<Vec<_>>(), vec![10, 11]);
        let total = total(&tree);
        assert_eq!(total.working_set, 3 << 20);
        assert_eq!(total.private_mb(), 2.0);
    }

    /// #219：私有工作集来自另一个计数器类，按 pid 并进同一棵树后一起求和。
    #[test]
    fn the_private_working_set_is_joined_by_pid_and_summed_over_the_tree() {
        let all = vec![p(10, 1, 8 << 20, 6 << 20), p(11, 10, 4 << 20, 4 << 20)];
        let mut tree = tree(&all, 10);
        let counters = parse_private_working_sets(
            r#"[{"IDProcess":0,"WorkingSetPrivate":123},
                {"IDProcess":10,"WorkingSetPrivate":3145728},
                {"IDProcess":11,"WorkingSetPrivate":1048576},
                {"IDProcess":99,"WorkingSetPrivate":999}]"#,
        );
        assert!(!counters.contains_key(&0), "pid 0（Idle）不进表");
        apply_private_working_sets(&mut tree, &counters);
        let total = total(&tree);
        assert_eq!(total.private_mb(), 10.0);
        assert_eq!(total.private_working_set_mb(), 4.0);
        // 已提交但没有驻留的部分 = 私有提交 − 私有工作集。
        assert_eq!(total.private_not_resident_mb(), Some(6.0));
    }

    /// 计数器不可用（被禁用、权限不足、PowerShell 没输出）时按「没测到」处理：
    /// 私有工作集留 0，差值返回 `None`，绝不把 0 当成「全都没驻留」写进报告。
    #[test]
    fn a_missing_counter_class_leaves_the_private_working_set_unmeasured() {
        for text in ["", "不是 JSON", "null"] {
            assert!(parse_private_working_sets(text).is_empty(), "{text:?}");
        }
        let all = vec![p(10, 1, 8 << 20, 6 << 20)];
        let mut tree = tree(&all, 10);
        apply_private_working_sets(&mut tree, &BTreeMap::new());
        let total = total(&tree);
        assert_eq!(total.private_working_set, 0);
        assert_eq!(total.private_not_resident_mb(), None);
    }

    /// 只有一个进程时 `ConvertTo-Json` 给的是对象而不是数组。
    #[test]
    fn a_single_counter_row_is_accepted_as_an_object() {
        let counters = parse_private_working_sets(r#"{"IDProcess":10,"WorkingSetPrivate":2048}"#);
        assert_eq!(counters.get(&10).copied(), Some(2048));
    }

    #[test]
    fn netstat_lines_are_matched_by_the_trailing_pid() {
        let text = "\
  协议  本地地址          外部地址        状态           PID
  TCP    0.0.0.0:135      0.0.0.0:0      LISTENING       1234
  TCP    192.168.1.2:50010  140.82.112.4:443  ESTABLISHED  4321
  TCP    127.0.0.1:50011  127.0.0.1:9000  ESTABLISHED     4321
";
        let pids = BTreeSet::from([4321]);
        let lines = parse_netstat(text, &pids);
        assert_eq!(lines.len(), 2);
        let out = outbound(&lines);
        assert_eq!(out.len(), 1, "只有到 140.82.112.4:443 的那条算出网");
        assert!(out[0].contains("140.82.112.4:443"));
    }

    #[test]
    fn listening_sockets_and_loopback_are_not_outbound() {
        let pids = BTreeSet::from([7]);
        let lines = parse_netstat(
            "  TCP 0.0.0.0:7000 0.0.0.0:0 LISTENING 7\n  UDP 0.0.0.0:5353 *:* 7\n  TCP [::1]:7001 [::1]:7002 ESTABLISHED 7\n",
            &pids,
        );
        assert_eq!(lines.len(), 3);
        assert!(outbound(&lines).is_empty());
    }
}
