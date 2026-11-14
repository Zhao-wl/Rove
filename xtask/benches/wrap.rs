//! 软换行（#58）的换行点计算与视觉行映射基准。不开窗口，纯 CPU。
//!
//! - `wrap/recompute_300k`：**视口宽度变化的最坏情况**——30 万行全部重算换行点并重建映射。
//!   视图里这件事是分帧做的（每帧一个时间切片，期间按旧映射绘制），这里测的是总耗时，
//!   用它除以每帧预算就能算出要摊到多少帧。
//! - `wrap/line_100k_chars`：单行 10 万字符（按 `editor.stopRenderingLineAfter` 截断到 1 万）。
//! - `wrap/map_lookup_300k`：映射查询（视觉行 → 缓冲行 → 视觉行区间）。

use criterion::{criterion_group, criterion_main, Criterion};
use rove_ui::code_view::columns::DisplayLine;
use rove_ui::code_view::folding::FoldMap;
use rove_ui::code_view::wrap::{self, Metric, VisualMap, WordWrap, WrapOptions};
use std::hint::black_box;

const SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

fn options() -> WrapOptions {
    WrapOptions {
        mode: WordWrap::On,
        separators: SEPARATORS.into(),
        ..Default::default()
    }
}

/// 30 万行代码风格的文本（约 60–90 列，一部分会换行）。
fn lines(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| {
            format!(
                "        private static int Method{i}(int value, string name = \"item{i}\") {{ return value * {i}; }}"
            )
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let o = options();
    let text = lines(300_000);
    let mut group = c.benchmark_group("wrap");
    group.sample_size(10);
    group.bench_function("recompute_300k", |b| {
        b.iter(|| {
            let n = text.len() as u64;
            let mut map = VisualMap::new(FoldMap::default(), n);
            map.enable_wrap(n);
            for (i, line) in text.iter().enumerate() {
                let d = DisplayLine::build(line, 4, Some(10_000));
                let w = wrap::wrap_line(line, &d, Metric::Columns { limit: 80 }, &o);
                map.set_count(i as u64, w.count());
            }
            map.commit();
            black_box(map.visual_count())
        })
    });

    let long = "x".repeat(100_000);
    group.bench_function("line_100k_chars", |b| {
        let d = DisplayLine::build(&long, 4, Some(10_000));
        b.iter(|| {
            let w = wrap::wrap_line(&long, &d, Metric::Columns { limit: 120 }, &o);
            black_box(w.count())
        })
    });

    let mut map = VisualMap::new(FoldMap::new([(1000, 2000)], 300_000), 300_000);
    map.enable_wrap(300_000);
    for i in (0..300_000u64).step_by(3) {
        map.set_count(i, 2);
    }
    map.commit();
    let total = map.visual_count();
    group.bench_function("map_lookup_300k", |b| {
        b.iter(|| {
            let mut acc = 0u64;
            for v in (0..total).step_by(9973) {
                let (doc, seg) = map.to_doc(v);
                acc += seg as u64 + map.visual_range(doc).start;
            }
            black_box(acc)
        })
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
