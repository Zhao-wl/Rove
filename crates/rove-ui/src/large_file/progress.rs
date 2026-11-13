//! 行索引进度与滚动条的“字节 → 行”映射（#22，`UX_SPEC` §3.2）。
//!
//! 首屏立即可见、索引在后台补全，所以滚动条在索引建完之前必须知道“整份文件大概有多少行”，
//! 否则内容高度会随索引推进不断变长、滑块一路缩小。
//!
//! 做法按 issue 的“风险与提示”：**始终用同一个映射函数**——已索引的部分按真实行数，
//! 未索引的部分按已索引部分的平均行长线性外推。索引完成时外推项自然归零，估计值等于真实行数，
//! 不存在“完成时一次性切换”的跳变。顶端行号（滚动位置）本身不受估计值影响，
//! 所以估计值变化时视口里看到的内容不动。
//!
//! 本文件不依赖 GPUI。

use rove_buffer::LineCount;

/// 行索引进度的快照。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexProgress {
    /// 已知行数（至少 1）。
    pub known: u64,
    /// 已索引的字节数。
    pub indexed_bytes: u64,
    /// 文件总字节数。
    pub total_bytes: u64,
    pub complete: bool,
}

impl IndexProgress {
    pub fn of(count: &LineCount) -> Self {
        Self {
            known: count.known.max(1),
            indexed_bytes: count.indexed_bytes.0,
            total_bytes: count.total_bytes,
            complete: count.complete,
        }
    }

    /// 是否要在状态栏显示 `行索引 N%`（完成后条目消失）。
    pub fn is_indexing(&self) -> bool {
        !self.complete
    }

    /// 百分比（0–100，向下取整；只在真正建完时才是 100）。
    pub fn percent(&self) -> u8 {
        if self.complete {
            return 100;
        }
        if self.total_bytes == 0 {
            return 100;
        }
        let ratio = self.indexed_bytes as f64 / self.total_bytes as f64;
        ((ratio * 100.0).floor() as u8).min(99)
    }

    /// 滚动条使用的总行数估计。
    ///
    /// 已索引部分按真实行数，未索引部分按已索引部分的平均行长线性外推：
    /// `known + (total - indexed) / (indexed / known)`。索引完成时等于 `known`。
    /// 估计值永远不小于已知行数，所以滚动范围只会变大、不会把当前位置挤出去。
    pub fn estimated_lines(&self) -> u64 {
        if self.complete || self.total_bytes <= self.indexed_bytes {
            return self.known;
        }
        if self.indexed_bytes == 0 {
            // 还没有任何字节被索引：无法外推，先按已知行数（首屏一屏）。
            return self.known;
        }
        let remaining = (self.total_bytes - self.indexed_bytes) as f64;
        let per_line = self.indexed_bytes as f64 / self.known as f64;
        let extra = (remaining / per_line.max(1.0)).ceil();
        self.known.saturating_add(extra.min(u64::MAX as f64) as u64)
    }

    /// 估计值与真实行数的相对误差（测试与基准用；`actual` 为 0 时返回 0）。
    pub fn estimate_error(&self, actual: u64) -> f64 {
        if actual == 0 {
            return 0.0;
        }
        (self.estimated_lines() as f64 - actual as f64).abs() / actual as f64
    }
}

/// 状态栏 `行索引 N%` 条目的文案 key。
pub const STATUS_KEY: &str = "largeFile.status.indexing";
/// 悬停提示：说明这是正常的后台工作。
pub const STATUS_TOOLTIP_KEY: &str = "largeFile.tooltip.indexing";

#[cfg(test)]
mod tests {
    use super::*;
    use rove_base::ByteOffset;

    fn count(known: u64, indexed: u64, total: u64, complete: bool) -> LineCount {
        LineCount {
            known,
            indexed_bytes: ByteOffset(indexed),
            total_bytes: total,
            complete,
        }
    }

    #[test]
    fn percent_never_reaches_100_before_the_index_is_complete() {
        let p = IndexProgress::of(&count(1000, 45, 100, false));
        assert_eq!(p.percent(), 45);
        assert!(p.is_indexing());
        // 字节已读完但还没宣布完成：仍然显示 99%，不骗用户。
        let p = IndexProgress::of(&count(1000, 100, 100, false));
        assert_eq!(p.percent(), 99);
        let p = IndexProgress::of(&count(1000, 100, 100, true));
        assert_eq!(p.percent(), 100);
        assert!(!p.is_indexing());
    }

    /// 行长均匀的文件（`big1g.txt` 式）：任何进度下的外推值都接近真实行数。
    #[test]
    fn extrapolation_tracks_the_real_line_count_on_uniform_lines() {
        let total_bytes = 1_073_741_824u64; // 1 GiB
        let per_line = 75u64;
        let actual = total_bytes / per_line;
        for percent in [1u64, 5, 17, 50, 83, 99] {
            let indexed = total_bytes * percent / 100;
            let known = indexed / per_line;
            let p = IndexProgress::of(&count(known, indexed, total_bytes, false));
            assert!(
                p.estimate_error(actual) < 0.01,
                "{percent}%: 估计 {} 实际 {actual}",
                p.estimated_lines()
            );
        }
    }

    /// 估计值单调不减、且永远不小于已知行数：滚动范围只扩不缩，当前位置不会被挤掉。
    #[test]
    fn estimate_never_drops_below_the_known_line_count() {
        let total = 1_000_000u64;
        let mut last = 0u64;
        for step in 0..=100u64 {
            let indexed = total * step / 100;
            let known = (indexed / 50).max(1);
            let p = IndexProgress::of(&count(known, indexed, total, step == 100));
            let estimate = p.estimated_lines();
            assert!(estimate >= known, "步 {step}");
            // 行长均匀时估计值基本不动，允许 ±2% 的抖动，不允许崩塌。
            if last > 0 {
                assert!(
                    estimate as f64 >= last as f64 * 0.98,
                    "步 {step}: {last} → {estimate}"
                );
            }
            last = estimate;
        }
        // 完成时估计值就是真实行数，没有“一次性切换”。
        let done = IndexProgress::of(&count(20_000, total, total, true));
        assert_eq!(done.estimated_lines(), 20_000);
    }

    /// 单行压缩 JSON：一行覆盖整份文件，外推不能炸出天文数字。
    #[test]
    fn a_single_gigantic_line_extrapolates_to_about_one_line() {
        let total = 50 << 20;
        let p = IndexProgress::of(&count(1, total / 2, total, false));
        assert_eq!(p.estimated_lines(), 2);
        let p = IndexProgress::of(&count(1, total, total, true));
        assert_eq!(p.estimated_lines(), 1);
    }

    /// 还没有索引任何字节时不外推（避免除零）。
    #[test]
    fn no_extrapolation_before_the_first_byte_is_indexed() {
        let p = IndexProgress::of(&count(1, 0, 1 << 30, false));
        assert_eq!(p.estimated_lines(), 1);
        assert_eq!(p.percent(), 0);
    }
}
