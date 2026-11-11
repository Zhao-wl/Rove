//! 设置文件的磁盘操作：带重试的读取、备份（保留最近 N 份）、原子写入（临时文件 + 重命名）。

use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// 文件被其他进程占用（共享冲突、锁冲突、拒绝访问）时的重试策略。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// 总尝试次数（至少 1）。
    pub attempts: u32,
    /// 两次尝试之间的等待。
    pub delay: Duration,
}

impl Default for RetryPolicy {
    /// 约 1 秒：足够覆盖 VS Code / 杀毒软件短暂持有文件，又不会让用户等太久。
    fn default() -> Self {
        Self {
            attempts: 20,
            delay: Duration::from_millis(50),
        }
    }
}

/// Windows 上“文件正被使用”的几种错误（ERROR_ACCESS_DENIED、ERROR_SHARING_VIOLATION、
/// ERROR_LOCK_VIOLATION），其他平台只认 `PermissionDenied`。
pub(crate) fn is_busy(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
        || (cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | 33)))
}

fn retry<T>(policy: RetryPolicy, mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let attempts = policy.attempts.max(1);
    let mut attempt = 1;
    loop {
        match op() {
            Err(e) if is_busy(&e) && attempt < attempts => {
                attempt += 1;
                thread::sleep(policy.delay);
            }
            result => return result,
        }
    }
}

/// 读取文件；不存在时返回 `None`。被占用时按策略重试。
pub(crate) fn read_optional(path: &Path, policy: RetryPolicy) -> io::Result<Option<Vec<u8>>> {
    retry(policy, || match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    })
}

/// 先完整写入同目录的临时文件并落盘，再重命名覆盖目标；目标被占用时按策略重试。
/// 失败时删除临时文件，目标保持原样。
pub(crate) fn atomic_write(path: &Path, bytes: &[u8], policy: RetryPolicy) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        fs::create_dir_all(dir)?;
    }
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(
        ".rove-{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = dir.map_or_else(|| PathBuf::from(&tmp_name), |d| d.join(&tmp_name));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        retry(policy, || fs::rename(&tmp, path))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

const BACKUP_PREFIX: &str = "settings-";
const BACKUP_SUFFIX: &str = ".json";

/// 把 `bytes` 存为 `dir/settings-YYYYMMDD-HHMMSS-mmm-NN.json`（UTC），然后只保留最近 `keep` 份。
/// 返回新备份的路径。任何一步失败都返回错误，调用方据此放弃写入。
pub(crate) fn backup(
    dir: &Path,
    bytes: &[u8],
    keep: usize,
    now: SystemTime,
) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let stamp = timestamp(now);
    let mut created = None;
    for seq in 0..100 {
        let path = dir.join(format!("{BACKUP_PREFIX}{stamp}-{seq:02}{BACKUP_SUFFIX}"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let written = file.write_all(bytes).and_then(|_| file.sync_all());
                if let Err(e) = written {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(e);
                }
                created = Some(path);
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let created = created.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many backups in one millisecond",
        )
    })?;
    prune(dir, keep.max(1))?;
    Ok(created)
}

/// 备份目录中现有的备份文件，按时间从旧到新排序。
pub(crate) fn list_backups(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut names: Vec<String> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(BACKUP_PREFIX) && n.ends_with(BACKUP_SUFFIX))
        .collect();
    names.sort();
    Ok(names.into_iter().map(|n| dir.join(n)).collect())
}

fn prune(dir: &Path, keep: usize) -> io::Result<()> {
    let backups = list_backups(dir)?;
    let excess = backups.len().saturating_sub(keep);
    for old in &backups[..excess] {
        fs::remove_file(old)?;
    }
    Ok(())
}

/// UTC 时间戳 `YYYYMMDD-HHMMSS-mmm`，字典序即时间序。
fn timestamp(now: SystemTime) -> String {
    let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}-{:03}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        since.subsec_millis()
    )
}

/// 1970-01-01 起的天数 → 公历年月日（Howard Hinnant 的 civil_from_days）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_utc_and_sortable() {
        assert_eq!(timestamp(UNIX_EPOCH), "19700101-000000-000");
        let t = UNIX_EPOCH + Duration::from_millis(1_794_398_645_123);
        assert_eq!(timestamp(t), "20261111-120405-123");
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn backups_are_kept_up_to_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let base = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        for i in 0..5u64 {
            backup(
                dir.path(),
                format!("v{i}").as_bytes(),
                3,
                base + Duration::from_secs(i),
            )
            .unwrap();
        }
        // 同一毫秒内的两次备份也不会互相覆盖。
        backup(dir.path(), b"v5", 3, base + Duration::from_secs(4)).unwrap();
        let kept: Vec<String> = list_backups(dir.path())
            .unwrap()
            .iter()
            .map(|p| fs::read_to_string(p).unwrap())
            .collect();
        assert_eq!(kept, ["v3", "v4", "v5"]);
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        atomic_write(&path, b"{}", RetryPolicy::default()).unwrap();
        atomic_write(&path, b"{\"a\": 1}", RetryPolicy::default()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\"a\": 1}");
        let files: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(files.len(), 1);
        assert_eq!(
            read_optional(&dir.path().join("missing"), RetryPolicy::default()).unwrap(),
            None
        );
    }

    #[test]
    fn retry_gives_up_after_the_configured_attempts() {
        let mut calls = 0;
        let policy = RetryPolicy {
            attempts: 3,
            delay: Duration::ZERO,
        };
        let r: io::Result<()> = retry(policy, || {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert!(r.is_err());
        assert_eq!(calls, 3);
        let mut calls = 0;
        let r: io::Result<()> = retry(policy, || {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        });
        assert!(r.is_err());
        assert_eq!(calls, 1);
    }
}
