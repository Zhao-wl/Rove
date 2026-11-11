//! 加载规则（UX_SPEC §6.2）：操作在 100 ms 内完成时不显示任何加载状态，超过才显示细进度条。
//! #17 打开文件时用 [`ProgressDelay`] 判断，再用 `view::progress_bar` 绘制。
use std::time::{Duration, Instant};

/// 超过这个时长才显示进度条。
pub const SHOW_AFTER: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug)]
pub struct ProgressDelay {
    started: Instant,
}
impl ProgressDelay {
    pub fn start(now: Instant) -> Self {
        Self { started: now }
    }
    pub fn visible(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.started) >= SHOW_AFTER
    }
    /// 距离应当显示还剩多久（调用方据此安排一次重绘，而不是每帧轮询）。
    pub fn remaining(&self, now: Instant) -> Duration {
        SHOW_AFTER.saturating_sub(now.saturating_duration_since(self.started))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_for_the_first_100ms() {
        let t0 = Instant::now();
        let p = ProgressDelay::start(t0);
        assert!(!p.visible(t0 + Duration::from_millis(99)));
        assert!(p.visible(t0 + Duration::from_millis(100)));
        assert_eq!(
            p.remaining(t0 + Duration::from_millis(40)),
            Duration::from_millis(60)
        );
    }
}
