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

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

/// 一个进程的内存（字节）。
#[derive(Debug, Clone, Copy, Default)]
pub struct Memory {
    pub working_set: u64,
    pub private: u64,
}

impl Memory {
    pub fn working_set_mb(&self) -> f64 {
        self.working_set as f64 / 1024.0 / 1024.0
    }
    pub fn private_mb(&self) -> f64 {
        self.private as f64 / 1024.0 / 1024.0
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
                },
            })
        })
        .collect()
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
