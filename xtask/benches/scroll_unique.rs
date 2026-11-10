//! 「全唯一行滚动」基准（issue #4）：整形缓存全未命中的真实成本。
//!
//! M0 结论：可见行全部需要重新整形时，GPUI 与自研栈都守不住 120 fps
//! （帧间隔 p99 约 16.8 ms，见 `M0_UI_SPIKE_REPORT` §4.4 与 §7 约束 3）。
//! 所以这是一条**正式**基准用例，而不是只在乐观数据（`small.txt`）上测。
//!
//! 现状：`#7`（文本源实现）与 `#19`（代码视图 Element，负责取可见行 → 整形）都还没有
//! 真实实现，因此这一组先占位：校验 `uniq.txt` 存在并打印规格，注册一个明确标注的
//! `pending` 用例。#19 完成后，把占位体换成真实的「取可见行 → 整形」调用即可，
//! `perf/budgets.toml` 里 `frame_cpu_cache_miss` 那条预算也随之从 pending 转成 enforced。

use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion};

fn testdata_dir() -> PathBuf {
    std::env::var_os("ROVE_PERF_TESTDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/perf-testdata"))
}

fn bench_scroll_unique(c: &mut Criterion) {
    let path = testdata_dir().join("uniq.txt");
    let Ok(metadata) = std::fs::metadata(&path) else {
        eprintln!(
            "跳过 scroll_unique：找不到 {}（先 cargo xtask gen-testdata --only uniq）",
            path.display()
        );
        return;
    };
    println!(
        "uniq.txt：{} B，每行唯一，用于测量整形缓存全未命中的每帧成本\
         （M0 基线：GPUI 原型 4279 us/帧，帧间隔 p99 16.83 ms）",
        metadata.len()
    );

    let mut group = c.benchmark_group("scroll_unique");
    // 占位用例：等 #19 提供「取可见行 → 整形」的公开入口后替换。
    // 名字里带 pending，避免被误当成真实指标；预算里也没有引用它。
    group.bench_function("pending_19", |b| {
        b.iter(|| std::hint::black_box(metadata.len()))
    });
    group.finish();
}

criterion_group!(benches, bench_scroll_unique);
criterion_main!(benches);
