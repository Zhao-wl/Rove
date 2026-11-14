//! 持续备份的去抖（#50）。
//!
//! **“只在退出时备份”是这个功能最常见的错误实现**：正常关闭能过，崩溃一条都过不了。所以
//! 每次编辑都登记一次“该备份了”，按安静期去抖写盘，并用最长等待封顶——连续输入时安静期永远
//! 等不到，没有封顶就又回到“只在退出时备份”。
//!
//! 参数与 #49 的 [`ChangeCoalescer`](crate::save::ChangeCoalescer) 同一套做法，但时间常数不同：
//!
//! | 参数 | 取值 | 理由 |
//! |---|---|---|
//! | 安静期 | 500 ms | 停手半秒就落盘；比击键间隔长得多，正常输入不会每个字符写一次 |
//! | 最长等待 | 2 s | 连续输入时约每 2 秒一次，最多丢 2 秒的输入；一分钟连打 29 次写入 |
//!
//! 不读时钟：时间由调用方传入，和 crate 里其他模型一致（便于测试与回放）。写盘本身在后台
//! 线程，UI 线程只做 O(1) 的登记与 O(脏文档数) 的到期扫描。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use rove_base::DocumentId;

/// 停止输入后多久写备份。
pub const QUIET_PERIOD: Duration = Duration::from_millis(500);
/// 连续输入时两次备份之间的最长间隔。
pub const MAX_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
struct Pending {
    /// 最后一次编辑的时刻 + 安静期。
    quiet_until: Instant,
    /// 第一次登记的时刻 + 最长等待。
    deadline: Instant,
}

/// 待备份文档的去抖队列。
#[derive(Clone, Debug)]
pub struct BackupSchedule {
    pending: BTreeMap<DocumentId, Pending>,
    quiet: Duration,
    max_wait: Duration,
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self::new(QUIET_PERIOD, MAX_WAIT)
    }
}

impl BackupSchedule {
    pub fn new(quiet: Duration, max_wait: Duration) -> Self {
        Self {
            pending: BTreeMap::new(),
            quiet,
            max_wait,
        }
    }

    /// 一次编辑：排进队列（已经在队列里的只延后安静期，最长等待不变）。
    pub fn note_edit(&mut self, document: DocumentId, now: Instant) {
        let quiet_until = now + self.quiet;
        self.pending
            .entry(document)
            .and_modify(|p| p.quiet_until = quiet_until)
            .or_insert(Pending {
                quiet_until,
                deadline: now + self.max_wait,
            });
    }

    /// 文档已保存 / 已关闭 / 刚写完备份：出队。
    pub fn forget(&mut self, document: DocumentId) {
        self.pending.remove(&document);
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// 下一次该醒来的时刻（定时器用）。
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending
            .values()
            .map(|p| p.quiet_until.min(p.deadline))
            .min()
    }

    /// 取出到期的文档（安静期已过，或已经等到最长等待）。
    pub fn due(&mut self, now: Instant) -> Vec<DocumentId> {
        let due: Vec<DocumentId> = self
            .pending
            .iter()
            .filter(|(_, p)| now >= p.quiet_until || now >= p.deadline)
            .map(|(id, _)| *id)
            .collect();
        for id in &due {
            self.pending.remove(id);
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: DocumentId = DocumentId(1);
    const B: DocumentId = DocumentId(2);

    #[test]
    fn a_pause_in_typing_writes_one_backup() {
        let mut s = BackupSchedule::default();
        let t0 = Instant::now();
        s.note_edit(A, t0);
        // 安静期内继续输入：不到期。
        s.note_edit(A, t0 + Duration::from_millis(100));
        assert!(s.due(t0 + Duration::from_millis(400)).is_empty());
        // 最后一次输入之后 500 ms：写一次。
        assert_eq!(s.due(t0 + Duration::from_millis(601)), vec![A]);
        assert!(s.is_empty());
        assert!(s.next_deadline().is_none());
    }

    #[test]
    fn continuous_typing_still_backs_up_every_max_wait() {
        let mut s = BackupSchedule::default();
        let t0 = Instant::now();
        let mut writes = 0;
        // 一分钟连续输入，每 50 ms 一次击键（比人手快），中间一次都不停。
        for tick in 0..1200u64 {
            let now = t0 + Duration::from_millis(tick * 50);
            s.note_edit(A, now);
            writes += s.due(now).len();
        }
        // 安静期永远等不到，全靠最长等待封顶：60 秒 / 约 2 秒一次 = 29 次（一次写入与下一次
        // 登记之间差一个击键间隔，所以实际周期略大于 2 秒）。
        assert_eq!(writes, 29, "连续输入 1 分钟的备份写入次数");
    }

    #[test]
    fn saved_or_closed_documents_leave_the_queue() {
        let mut s = BackupSchedule::default();
        let t0 = Instant::now();
        s.note_edit(A, t0);
        s.note_edit(B, t0);
        s.forget(A);
        assert_eq!(s.due(t0 + Duration::from_secs(1)), vec![B]);
        s.note_edit(A, t0);
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn next_deadline_is_the_earliest_of_quiet_and_max_wait() {
        let mut s = BackupSchedule::default();
        let t0 = Instant::now();
        s.note_edit(A, t0);
        assert_eq!(s.next_deadline(), Some(t0 + QUIET_PERIOD));
        // 持续输入把安静期推到最长等待之后：以最长等待为准。
        for tick in 1..40u64 {
            s.note_edit(A, t0 + Duration::from_millis(tick * 50));
        }
        assert_eq!(s.next_deadline(), Some(t0 + MAX_WAIT));
    }
}
