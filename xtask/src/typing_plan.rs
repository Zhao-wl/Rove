//! 按键到上屏基准的**采样计划与统计**（issue #215）。
//!
//! 这里只放纯逻辑，没有窗口、没有计时器：`xtask/benches/typing.rs` 用 `#[path]` 把本文件编进基准，
//! 单元测试跟着 `cargo test -p xtask` 跑。背景与口径见 `perf/README.md`「按键到上屏」一节。
//!
//! 要解决的问题：#47 的基准在**帧回调里**注入按键，也就是正好落在「刚收到垂直同步、还没开始绘制」
//! 这一个相位上。真实按键是 Windows 消息队列送来的，可能落在一帧里的**任何**相位，所以旧口径
//! 既测不到「等下一帧」的真实分布，也测不出「按键有没有被紧接着的那一帧吸收」。
//! 本模块负责把注入点**均匀铺开**在一个刷新间隔里，并统计吸收情况。

use std::collections::BTreeMap;
use std::time::Duration;

/// 注入相位的上限（占一个刷新间隔的比例）。
///
/// 留出尾部余量：注入本身要花几百微秒，相位太靠近下一次垂直同步时，按键会落到**下一个**间隔里，
/// 测到的就不再是「这一帧的相位」而是采样器自己的抖动。0.8 在 120 Hz（8.33 ms）上留约 1.7 ms。
pub const PHASE_CEILING: f64 = 0.8;

/// 低差异序列（黄金比例）给出的第 `i` 个相位比例，落在 `[0, 1)`。
///
/// 比随机数好的地方：确定可复现、任何前缀都接近均匀，不会像线性扫描那样和刷新率形成固定拍频。
pub fn phase_fraction(i: usize) -> f64 {
    const GOLDEN: f64 = 0.618_033_988_749_894_8;
    ((i as f64 + 1.0) * GOLDEN).fract()
}

/// 第 `i` 个样本的注入延迟：从帧回调（刚过垂直同步）算起，等多久再把按键送进去。
pub fn injection_delay(i: usize, interval: Duration, ceiling: f64) -> Duration {
    let ceiling = ceiling.clamp(0.0, 1.0);
    interval.mul_f64(phase_fraction(i) * ceiling)
}

/// 由相邻帧回调的间隔估计刷新间隔：取中位数，并丢掉明显异常的样本。
///
/// 丢弃规则：小于 1 ms（回调被挤在一起）或大于 100 ms（被别的进程抢走了 CPU）的间隔不参与。
pub fn estimate_interval(deltas: &[Duration]) -> Option<Duration> {
    let mut usable: Vec<Duration> = deltas
        .iter()
        .copied()
        .filter(|d| *d >= Duration::from_millis(1) && *d <= Duration::from_millis(100))
        .collect();
    if usable.is_empty() {
        return None;
    }
    usable.sort_unstable();
    Some(usable[usable.len() / 2])
}

/// 「按键被第几帧吸收」的统计。1 表示紧接着的那一帧就画出了新字符，也就是**没有多等一帧**。
#[derive(Debug, Default, Clone)]
pub struct Absorption {
    counts: BTreeMap<u32, u32>,
}

impl Absorption {
    pub fn record(&mut self, draws: u32) {
        *self.counts.entry(draws).or_default() += 1;
    }

    pub fn total(&self) -> u32 {
        self.counts.values().sum()
    }

    /// 被紧接着的那一帧吸收的样本数。
    pub fn next_frame(&self) -> u32 {
        self.counts.get(&1).copied().unwrap_or(0)
    }

    /// 最坏情况下等了几帧（没有样本时为 0）。
    pub fn worst(&self) -> u32 {
        self.counts.keys().next_back().copied().unwrap_or(0)
    }

    /// 全部样本都被紧接着的那一帧吸收。
    pub fn all_next_frame(&self) -> bool {
        self.total() > 0 && self.next_frame() == self.total()
    }

    /// 形如 `1 帧 ×400`，按帧数升序。
    pub fn histogram(&self) -> String {
        self.counts
            .iter()
            .map(|(frames, n)| format!("{frames} 帧 ×{n}"))
            .collect::<Vec<_>>()
            .join("，")
    }
}

/// 一组耗时样本的汇总（毫秒）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// 最近秩百分位：`p` 取 0..=1，空切片返回 `None`。
pub fn percentile_ms(sorted_ms: &[f64], p: f64) -> Option<f64> {
    if sorted_ms.is_empty() {
        return None;
    }
    let idx = ((sorted_ms.len() as f64 - 1.0) * p.clamp(0.0, 1.0)).round() as usize;
    sorted_ms.get(idx).copied()
}

pub fn summarize(samples: &[Duration]) -> Option<Summary> {
    if samples.is_empty() {
        return None;
    }
    let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1e3).collect();
    ms.sort_by(|a, b| a.partial_cmp(b).expect("耗时不会是 NaN"));
    Some(Summary {
        n: ms.len(),
        mean_ms: ms.iter().sum::<f64>() / ms.len() as f64,
        p50_ms: percentile_ms(&ms, 0.5)?,
        p99_ms: percentile_ms(&ms, 0.99)?,
        max_ms: ms[ms.len() - 1],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_cover_the_whole_frame_without_repeating() {
        let mut buckets = [0u32; 10];
        for i in 0..200 {
            let f = phase_fraction(i);
            assert!((0.0..1.0).contains(&f), "相位 {f} 越界");
            buckets[(f * 10.0) as usize] += 1;
        }
        // 黄金比例序列：200 个样本落进 10 个桶，每个桶都应该有，且不会偏到某一桶里。
        for (i, n) in buckets.iter().enumerate() {
            assert!(*n >= 15, "第 {i} 个相位桶只有 {n} 个样本，相位没有铺开");
            assert!(*n <= 25, "第 {i} 个相位桶有 {n} 个样本，相位分布不均");
        }
    }

    #[test]
    fn injection_delay_stays_under_the_ceiling() {
        let interval = Duration::from_micros(8_333);
        for i in 0..500 {
            let d = injection_delay(i, interval, PHASE_CEILING);
            assert!(
                d <= interval.mul_f64(PHASE_CEILING),
                "第 {i} 个相位 {d:?} 超过了上限"
            );
        }
        // 上限非法时夹紧，不会算出负数或者超过一整帧的延迟。
        assert_eq!(injection_delay(0, interval, -1.0), Duration::ZERO);
        assert!(injection_delay(0, interval, 5.0) <= interval);
    }

    #[test]
    fn interval_estimate_ignores_outliers() {
        let deltas = vec![
            Duration::from_micros(8_300),
            Duration::from_micros(8_340),
            Duration::from_micros(8_333),
            Duration::from_micros(120),    // 回调被挤在一起
            Duration::from_millis(2_000),  // 被别的进程抢走 CPU
            Duration::from_micros(16_700), // 掉了一帧，但仍是合法采样
        ];
        let interval = estimate_interval(&deltas).expect("有可用样本");
        assert!(
            interval >= Duration::from_micros(8_300) && interval <= Duration::from_micros(8_400),
            "估出来的刷新间隔是 {interval:?}"
        );
        assert_eq!(estimate_interval(&[]), None);
        assert_eq!(estimate_interval(&[Duration::from_micros(10)]), None);
    }

    #[test]
    fn absorption_counts_the_frame_that_painted_the_key() {
        let mut a = Absorption::default();
        for _ in 0..399 {
            a.record(1);
        }
        assert!(a.all_next_frame());
        a.record(2);
        assert_eq!(a.total(), 400);
        assert_eq!(a.next_frame(), 399);
        assert_eq!(a.worst(), 2);
        assert!(!a.all_next_frame(), "有样本多等了一帧就不算全部吸收");
        assert_eq!(a.histogram(), "1 帧 ×399，2 帧 ×1");
        assert!(!Absorption::default().all_next_frame(), "没有样本不算通过");
    }

    #[test]
    fn summary_reports_the_tail_not_just_the_mean() {
        // 最近秩：100 个样本的 p99 落在第 99 个（下标 98），所以要有两个长尾样本才轮得到它。
        let mut samples: Vec<Duration> = (0..98).map(|_| Duration::from_micros(500)).collect();
        samples.push(Duration::from_millis(40));
        samples.push(Duration::from_millis(40));
        let s = summarize(&samples).expect("有样本");
        assert_eq!(s.n, 100);
        assert!((s.p50_ms - 0.5).abs() < 1e-9, "p50 = {}", s.p50_ms);
        assert!((s.p99_ms - 40.0).abs() < 1e-9, "p99 = {}", s.p99_ms);
        assert!((s.max_ms - 40.0).abs() < 1e-9);
        assert!(s.mean_ms > 1.2 && s.mean_ms < 1.4, "mean = {}", s.mean_ms);
        // 只有一个长尾样本时 p99 够不到它，max 才看得见——分位数不是最大值。
        let mut one_tail: Vec<Duration> = (0..99).map(|_| Duration::from_micros(500)).collect();
        one_tail.push(Duration::from_millis(40));
        let s = summarize(&one_tail).expect("有样本");
        assert!((s.p99_ms - 0.5).abs() < 1e-9, "p99 = {}", s.p99_ms);
        assert!((s.max_ms - 40.0).abs() < 1e-9);
        assert_eq!(summarize(&[]), None);
    }
}
