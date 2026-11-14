//! 更新日志：`%LOCALAPPDATA%\Rove\logs\update.log`（issue #149 验收最后一条）。
//!
//! **每一次网络请求都有一行**，内容是方法、URL、状态码与字节数——出了问题可以直接
//! 看到 Rove 到底访问了什么。日志只写本机，**绝不上传**（与 #50 的崩溃日志同一条原则）。
//!
//! 与 `rove-lang-cs` 的索引日志同样按大小轮转，时间戳同样是 UTC
//! （日志只在本机看，不依赖时区数据库）。

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// 单个日志文件的大小上限。
pub const MAX_BYTES: u64 = 256 * 1024;
/// 轮转保留的旧份数。
pub const KEEP: usize = 2;
/// 日志文件名。
pub const FILE: &str = "update.log";

/// UTC 时间戳 `YYYY-MM-DD HH:MM:SSZ`。
pub fn timestamp(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant 的 civil_from_days（与 rove-lang-cs / rove-config 同一段算法）。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// 一份可共享的更新日志。路径取不到时所有写入都是空操作（不把日志写到项目目录里）。
#[derive(Debug, Default)]
pub struct UpdateLog {
    path: Option<PathBuf>,
    max_bytes: u64,
    lock: Mutex<()>,
}

impl UpdateLog {
    /// `logs_dir` 一般是 `%LOCALAPPDATA%\Rove\logs`。
    pub fn new(logs_dir: Option<PathBuf>) -> Self {
        Self {
            path: logs_dir.map(|d| d.join(FILE)),
            max_bytes: MAX_BYTES,
            lock: Mutex::new(()),
        }
    }

    /// 不落盘的日志（单元测试与「拿不到目录」时用）。
    pub fn disabled() -> Self {
        Self::new(None)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    #[cfg(test)]
    fn with_limit(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// 写一行。失败（目录只读、被杀毒软件锁住）时安静地放弃：
    /// 写不了日志不应该让更新流程挂掉。
    pub fn line(&self, text: &str) {
        let Some(path) = &self.path else { return };
        let _guard = self.lock.lock();
        let line = format!("{} {text}\n", timestamp(SystemTime::now()));
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::metadata(path).is_ok_and(|m| m.len() + line.len() as u64 > self.max_bytes) {
            self.rotate(path);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// 一次 HTTP 请求的记录。**请求的全部可变部分都在这一行里**：方法、URL、结果。
    pub fn request(&self, method: &str, url: &str, result: &str) {
        self.line(&format!("request {method} {url} -> {result}"));
    }

    fn rotate(&self, path: &Path) {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let rotated = |n: usize| path.with_file_name(format!("{stem}.{n}.log"));
        let _ = fs::remove_file(rotated(KEEP));
        for n in (1..KEEP).rev() {
            let _ = fs::rename(rotated(n), rotated(n + 1));
        }
        let _ = fs::rename(path, rotated(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_utc_and_sortable() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01 00:00:00Z");
        assert_eq!(
            timestamp(UNIX_EPOCH + std::time::Duration::from_secs(1_763_000_000)),
            "2025-11-13 02:13:20Z"
        );
    }

    /// 验收：日志里每次请求都有一行记录，而且写着访问了哪个地址。
    #[test]
    fn every_request_is_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let log = UpdateLog::new(Some(dir.path().to_path_buf()));
        log.request(
            "GET",
            "https://example.invalid/latest.json",
            "200 1234 字节",
        );
        log.request("GET", "https://example.invalid/latest.json.sig", "404");
        let text = fs::read_to_string(log.path().unwrap()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("request GET https://example.invalid/latest.json -> 200"));
        assert!(lines[1].ends_with("-> 404"));
        // 每行都以时间戳开头。
        assert!(lines.iter().all(|l| l.starts_with("20") && l.contains('Z')));
    }

    #[test]
    fn the_log_rotates_instead_of_growing_forever() {
        let dir = tempfile::tempdir().unwrap();
        let log = UpdateLog::new(Some(dir.path().to_path_buf())).with_limit(200);
        for i in 0..40 {
            log.line(&format!(
                "check {i} 这是一条足够长的日志，用来把文件撑过上限"
            ));
        }
        assert!(dir.path().join("update.1.log").is_file());
        assert!(fs::metadata(log.path().unwrap()).unwrap().len() <= 200);
        // 最多保留 KEEP 份旧日志。
        assert!(!dir.path().join(format!("update.{}.log", KEEP + 1)).exists());
    }

    /// 拿不到目录时写日志是空操作，不会 panic，也不会写到当前目录。
    #[test]
    fn a_log_without_a_directory_does_nothing() {
        let log = UpdateLog::disabled();
        log.request("GET", "https://example.invalid/x", "200");
        assert!(log.path().is_none());
    }
}
