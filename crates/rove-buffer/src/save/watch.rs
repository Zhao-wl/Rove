//! 文件事件的去重与合并（#49）。
//!
//! - [`SelfWrites`]：自己保存触发的事件必须丢弃，否则每次保存都会自触发一次“外部修改”。
//!   只按“路径 + 时间窗”去重在慢盘上会漏判（事件晚于时间窗到达），只按内容哈希去重在“保存后
//!   外部立刻改回同样内容”时会误判；所以两者结合：时间窗内、且磁盘内容哈希等于我们写入的哈希，
//!   才算自己的写入。时间窗外到达的事件交给 [`super::TextFile::disk_changed`]，它以“磁盘内容 ==
//!   文档最后一次已知的磁盘内容”为准，同样不会误报。
//! - [`ChangeCoalescer`]：外部修改风暴（脚本连续改写）按路径去抖：同一路径在安静期内的事件合并
//!   成一次，持续不断的事件最多等 `max_wait` 也处理一次；最终一定处理到最后一次事件之后的状态。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 路径的比较键：Windows 上不区分大小写、统一分隔符。
pub fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// 默认的自写时间窗。
pub const SELF_WRITE_WINDOW: Duration = Duration::from_secs(2);

/// 最近自己写入过的文件。
#[derive(Debug)]
pub struct SelfWrites {
    window: Duration,
    entries: BTreeMap<String, (u64, Instant)>,
}

impl Default for SelfWrites {
    fn default() -> Self {
        Self::new(SELF_WRITE_WINDOW)
    }
}

impl SelfWrites {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            entries: BTreeMap::new(),
        }
    }

    /// 记录一次写入（写入成功后立即调用）。
    pub fn record(&mut self, path: &Path, hash: u64, now: Instant) {
        self.entries.insert(path_key(path), (hash, now));
    }

    /// `path` 上的事件是否由自己的写入引起：时间窗内且磁盘内容就是我们写入的内容。
    /// `disk_hash` 为 `None` 表示文件已不存在（那一定不是我们的写入）。
    pub fn is_own(&mut self, path: &Path, disk_hash: Option<u64>, now: Instant) -> bool {
        self.prune(now);
        match (self.entries.get(&path_key(path)), disk_hash) {
            (Some((hash, _)), Some(disk)) => *hash == disk,
            _ => false,
        }
    }

    /// 是否有可能是自写事件（只看路径与时间窗，不读盘）：为假时一定不是。
    pub fn maybe_own(&mut self, path: &Path, now: Instant) -> bool {
        self.prune(now);
        self.entries.contains_key(&path_key(path))
    }

    fn prune(&mut self, now: Instant) {
        let window = self.window;
        self.entries
            .retain(|_, (_, at)| now.saturating_duration_since(*at) <= window);
    }
}

/// 默认安静期与最长等待。
pub const COALESCE_QUIET: Duration = Duration::from_millis(100);
pub const COALESCE_MAX_WAIT: Duration = Duration::from_millis(1000);

#[derive(Debug)]
struct Pending {
    path: PathBuf,
    first: Instant,
    last: Instant,
}

/// 按路径合并文件事件。
#[derive(Debug)]
pub struct ChangeCoalescer {
    quiet: Duration,
    max_wait: Duration,
    pending: BTreeMap<String, Pending>,
}

impl Default for ChangeCoalescer {
    fn default() -> Self {
        Self::new(COALESCE_QUIET, COALESCE_MAX_WAIT)
    }
}

impl ChangeCoalescer {
    pub fn new(quiet: Duration, max_wait: Duration) -> Self {
        Self {
            quiet,
            max_wait,
            pending: BTreeMap::new(),
        }
    }

    /// 记下一次事件。
    pub fn note(&mut self, path: &Path, now: Instant) {
        self.pending
            .entry(path_key(path))
            .and_modify(|pending| pending.last = now)
            .or_insert_with(|| Pending {
                path: path.to_path_buf(),
                first: now,
                last: now,
            });
    }

    /// 取出到期的路径（安静期已过或已等满 `max_wait`），每个路径只出现一次。
    pub fn due(&mut self, now: Instant) -> Vec<PathBuf> {
        let (quiet, max_wait) = (self.quiet, self.max_wait);
        let ready: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| {
                now.saturating_duration_since(p.last) >= quiet
                    || now.saturating_duration_since(p.first) >= max_wait
            })
            .map(|(key, _)| key.clone())
            .collect();
        ready
            .into_iter()
            .filter_map(|key| self.pending.remove(&key).map(|p| p.path))
            .collect()
    }

    /// 下一次可能有路径到期的时刻（界面据此安排定时器）。
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending
            .values()
            .map(|p| (p.last + self.quiet).min(p.first + self.max_wait))
            .min()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_write_needs_window_and_hash() {
        let mut writes = SelfWrites::new(Duration::from_secs(1));
        let t0 = Instant::now();
        let path = Path::new("C:/p/A.cs");
        writes.record(path, 7, t0);
        assert!(writes.is_own(Path::new("c:\\p\\a.cs"), Some(7), t0));
        // 时间窗内但内容不同：外部改写。
        assert!(!writes.is_own(path, Some(8), t0));
        // 文件被删除：不是我们的写入。
        assert!(!writes.is_own(path, None, t0));
        // 时间窗外：不再认领（交给内容比较）。
        assert!(!writes.is_own(path, Some(7), t0 + Duration::from_secs(2)));
        assert!(!writes.maybe_own(path, t0 + Duration::from_secs(2)));
    }

    #[test]
    fn storm_is_coalesced_per_path() {
        let mut c = ChangeCoalescer::new(Duration::from_millis(100), Duration::from_millis(1000));
        let t0 = Instant::now();
        let a = Path::new("a");
        for i in 0..100 {
            c.note(a, t0 + Duration::from_millis(i * 5));
            // 风暴期间不到期（安静期没过、也没等满）。
            assert!(c.due(t0 + Duration::from_millis(i * 5)).is_empty());
        }
        c.note(Path::new("b"), t0);
        assert_eq!(
            c.due(t0 + Duration::from_millis(700)),
            vec![PathBuf::from("a"), PathBuf::from("b")]
        );
        assert!(c.is_empty());
    }

    #[test]
    fn continuous_events_still_flush_after_max_wait() {
        let mut c = ChangeCoalescer::new(Duration::from_millis(100), Duration::from_millis(300));
        let t0 = Instant::now();
        let a = Path::new("a");
        let mut flushed = 0;
        for i in 0..100u64 {
            let now = t0 + Duration::from_millis(i * 10);
            c.note(a, now);
            flushed += c.due(now).len();
        }
        // 1 秒的连续事件，最长等待 300 ms：中途处理了几次，但远少于 100 次。
        assert!((2..=4).contains(&flushed), "{flushed}");
        assert!(c.next_deadline().is_some());
    }
}
