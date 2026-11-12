//! 索引日志：缓存目录下的 `logs\indexer.log`，超过上限时轮转（`indexer.1.log` 最新的旧份，
//! 最多保留 [`KEEP`] 份），`rove.index.openLog` 打开的总是当前这一份。
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// 单个日志文件的大小上限。常驻模式（#35）下日志会持续增长，按大小轮转而不是按天。
pub const MAX_BYTES: u64 = 1 << 20;
pub const KEEP: usize = 3;

pub struct IndexLog {
    path: PathBuf,
    max_bytes: u64,
    file: Mutex<Option<File>>,
}

/// UTC 时间戳 `YYYY-MM-DD HH:MM:SS.mmm`（日志只在本机看，不依赖时区数据库）。
pub fn timestamp(time: SystemTime) -> String {
    let ms = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis()) as i64;
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // Howard Hinnant 的 civil_from_days。
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
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

impl IndexLog {
    pub fn new(path: PathBuf) -> Self {
        Self::with_limit(path, MAX_BYTES)
    }
    pub fn with_limit(path: PathBuf, max_bytes: u64) -> Self {
        Self {
            path,
            max_bytes,
            file: Mutex::new(None),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn rotated(&self, n: usize) -> PathBuf {
        let stem = self.path.file_stem().unwrap_or_default().to_string_lossy();
        self.path.with_file_name(format!("{stem}.{n}.log"))
    }
    /// 写一行（自动加时间戳）。写日志失败不影响索引本身，只打到 stderr。
    pub fn line(&self, text: &str) {
        let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let size = fs::metadata(&self.path).map_or(0, |m| m.len());
        if size >= self.max_bytes {
            *guard = None;
            let _ = fs::remove_file(self.rotated(KEEP));
            for n in (1..KEEP).rev() {
                let _ = fs::rename(self.rotated(n), self.rotated(n + 1));
            }
            let _ = fs::rename(&self.path, self.rotated(1));
        }
        if guard.is_none() {
            *guard = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .map_err(|e| eprintln!("rove-lang-cs: index log {}: {e}", self.path.display()))
                .ok();
        }
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "{} {text}", timestamp(SystemTime::now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timestamps_and_rotation() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01 00:00:00.000Z");
        assert_eq!(
            timestamp(UNIX_EPOCH + Duration::from_millis(1_762_819_200_123)),
            "2025-11-11 00:00:00.123Z"
        );
        let dir = tempfile::tempdir().unwrap();
        let log = IndexLog::with_limit(dir.path().join("indexer.log"), 64);
        for i in 0..40 {
            log.line(&format!("line {i}"));
        }
        assert!(fs::metadata(log.path()).unwrap().len() < 200);
        assert!(dir.path().join("indexer.1.log").exists());
        assert!(dir.path().join("indexer.3.log").exists());
        assert!(!dir.path().join("indexer.4.log").exists());
        assert!(fs::read_to_string(log.path()).unwrap().contains("line 39"));
    }
}
