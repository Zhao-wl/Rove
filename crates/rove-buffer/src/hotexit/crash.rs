//! 崩溃日志（#50）：panic 与致命错误写到 `%LOCALAPPDATA%\Rove\logs\`。
//!
//! **只保存在本地**。这里没有、也不会有任何网络代码：`FEATURES.md` §10 的遥测与崩溃上报是
//! “不做”，章程里运行时唯一允许联网的是自动更新模块（#149），不是这里。本模块只用
//! `std::fs` 与 `std::backtrace`。
//!
//! 日志内容限定为：时间戳、panic 消息与位置、调用栈、以及环境信息（Rove 版本、目标三元组、
//! 可执行文件路径、进程 ID）。**不写任何文件内容**；用户打开过哪些文件也不写进来——panic 消息
//! 本身由代码决定，调用栈里只有函数名与源码位置。
//!
//! 保留最近 [`KEEP_LOGS`] 份，多的按文件名（带时间戳，字典序即时间序）删掉最旧的。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOGS_DIR: &str = "logs";
/// 保留最近几份崩溃日志。
pub const KEEP_LOGS: usize = 10;
const PREFIX: &str = "crash-";
const EXT: &str = ".log";

/// 装上 panic 钩子：panic 时先写日志，再调用原来的钩子（保持控制台输出不变）。
/// 多次调用是安全的，但只应该在应用入口调用一次。
pub fn install_panic_hook(logs_dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = panic_message(info);
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        // 写日志失败（目录被锁、磁盘满）不能再 panic：忽略即可，原钩子照常打印。
        let _ = write_log(
            &logs_dir,
            "panic",
            &message,
            location.as_deref(),
            &std::backtrace::Backtrace::force_capture().to_string(),
        );
        previous(info);
    }));
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic without a string payload".to_owned()
    }
}

/// 致命错误（不是 panic，但这次运行走不下去了）：同样留一份本地日志。
pub fn log_fatal(logs_dir: &Path, reason: &str) -> std::io::Result<PathBuf> {
    write_log(
        logs_dir,
        "fatal",
        reason,
        None,
        &std::backtrace::Backtrace::force_capture().to_string(),
    )
}

fn write_log(
    logs_dir: &Path,
    kind: &str,
    message: &str,
    location: Option<&str>,
    backtrace: &str,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(logs_dir)?;
    let now = SystemTime::now();
    let path = unique_path(logs_dir, now);
    let mut file = std::fs::File::create(&path)?;
    writeln!(file, "kind: {kind}")?;
    writeln!(file, "time: {}", timestamp(now))?;
    writeln!(file, "rove: {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(file, "os: {}", std::env::consts::OS)?;
    writeln!(file, "arch: {}", std::env::consts::ARCH)?;
    writeln!(file, "pid: {}", std::process::id())?;
    writeln!(
        file,
        "exe: {}",
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    )?;
    if let Some(location) = location {
        writeln!(file, "location: {location}")?;
    }
    writeln!(file, "message: {}", message.replace('\n', " "))?;
    writeln!(file, "backtrace:")?;
    file.write_all(backtrace.as_bytes())?;
    file.flush()?;
    drop(file);
    prune(logs_dir, KEEP_LOGS);
    Ok(path)
}

fn unique_path(logs_dir: &Path, now: SystemTime) -> PathBuf {
    let stamp = file_stamp(now);
    let mut path = logs_dir.join(format!("{PREFIX}{stamp}{EXT}"));
    let mut n = 1;
    while path.exists() {
        path = logs_dir.join(format!("{PREFIX}{stamp}-{n}{EXT}"));
        n += 1;
    }
    path
}

/// 只保留最近 `keep` 份（文件名里的时间戳是定长的，字典序即时间序）。
pub fn prune(logs_dir: &Path, keep: usize) {
    let mut logs: Vec<PathBuf> = std::fs::read_dir(logs_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(EXT))
        })
        .collect();
    if logs.len() <= keep {
        return;
    }
    logs.sort();
    let extra = logs.len() - keep;
    for path in logs.into_iter().take(extra) {
        let _ = std::fs::remove_file(path);
    }
}

/// 文件名用的时间戳：`YYYYMMDD-HHMMSS`（UTC，定长，字典序即时间序）。
fn file_stamp(now: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = civil(now);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// 日志正文里的时间戳：`YYYY-MM-DD HH:MM:SS UTC`。
fn timestamp(now: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = civil(now);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

/// Unix 秒 → 公历（UTC）。Howard Hinnant 的 `civil_from_days`，不引入时间库。
fn civil(now: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn civil_matches_known_instants() {
        let at = |secs: u64| UNIX_EPOCH + Duration::from_secs(secs);
        assert_eq!(civil(UNIX_EPOCH), (1970, 1, 1, 0, 0, 0));
        assert_eq!(file_stamp(UNIX_EPOCH), "19700101-000000");
        // 2026-11-13 08:09:10 UTC
        assert_eq!(civil(at(1_794_557_350)), (2026, 11, 13, 8, 9, 10));
        assert_eq!(timestamp(at(1_794_557_350)), "2026-11-13 08:09:10 UTC");
        // 闰日。
        assert_eq!(civil(at(1_709_164_800)).0, 2024);
        assert_eq!(civil(at(1_709_164_800)).1, 2);
        assert_eq!(civil(at(1_709_164_800)).2, 29);
    }

    #[test]
    fn fatal_log_keeps_only_the_newest_and_has_no_user_data() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join(LOGS_DIR);
        let path = log_fatal(&logs, "索引进程无法启动").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("kind: fatal"));
        assert!(text.contains("索引进程无法启动"));
        assert!(text.contains("backtrace:"));
        assert!(text.contains("pid: "));
        // 轮转：写满上限 + 3 份后只剩上限数量，且留下的是最新的。
        for n in 0..KEEP_LOGS + 3 {
            std::fs::write(logs.join(format!("{PREFIX}2020010{n}-000000{EXT}")), b"x").unwrap();
        }
        prune(&logs, KEEP_LOGS);
        let mut left: Vec<String> = std::fs::read_dir(&logs)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left.len(), KEEP_LOGS);
        assert!(left.last().unwrap() > left.first().unwrap());
    }

    #[test]
    fn two_logs_in_the_same_second_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join(LOGS_DIR);
        let a = log_fatal(&logs, "一").unwrap();
        let b = log_fatal(&logs, "二").unwrap();
        assert_ne!(a, b);
        assert!(a.is_file() && b.is_file());
    }
}
