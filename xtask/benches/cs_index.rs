//! `rove-lang-cs` 索引查询基准（issue #4）。
//!
//! 需要真实索引文件：`cargo xtask bench --cs-index out/medium.idx`，
//! 或用环境变量 `ROVE_PERF_CS_INDEX` 指定。没有索引时 `cargo xtask bench`
//! 会整组跳过（本地与 CI 都不强制要求索引）。
//!
//! 基准只使用 `crates/rove-lang-cs` 的公开查询接口，与 M0 的 `cs_index_bench`
//! 口径一致（同一批随机引用位置、可复现的 xorshift 采样）。

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::Utf8Column;
use rove_lang_cs::CsIndex;

/// 采样多少个引用位置作为查询输入（够覆盖热点与冷点）。
const SAMPLES: usize = 2000;

fn index_path() -> Option<PathBuf> {
    std::env::var_os("ROVE_PERF_CS_INDEX").map(PathBuf::from)
}

/// 采样「引用位置」：符号出现位置里的非定义项。
fn sample_positions(index: &CsIndex) -> Vec<(String, u32, Utf8Column)> {
    let mut samples = Vec::with_capacity(SAMPLES);
    let total = index.occurrence_count();
    if total == 0 {
        return samples;
    }
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    while samples.len() < SAMPLES {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let Some(occurrence) = index.occurrence((seed % total as u64) as usize) else {
            continue;
        };
        if occurrence.role & 2 != 0 {
            continue; // 只要引用，不要定义
        }
        let Some(path) = index.file_path(occurrence.file) else {
            continue;
        };
        samples.push((path.to_string(), occurrence.line, occurrence.col));
    }
    samples
}

fn bench_cs_index(c: &mut Criterion) {
    let Some(path) = index_path() else {
        eprintln!(
            "跳过 cs_index：未设置 ROVE_PERF_CS_INDEX（用 cargo xtask bench --cs-index <idx>）"
        );
        return;
    };
    let index = CsIndex::open(&path).expect("打开索引失败");
    println!(
        "索引：{} 个文件 / {} 个符号 / {} 个出现位置（{}）",
        index.files().len(),
        index.symbols().len(),
        index.occurrence_count(),
        path.display()
    );

    let samples = sample_positions(&index);
    if samples.is_empty() {
        eprintln!("跳过 cs_index：索引里没有可用的引用位置");
        return;
    }
    println!("采样 {} 个引用位置", samples.len());

    let mut group = c.benchmark_group("cs_index");
    group.bench_function("goto_definition", |b| {
        let mut next = 0usize;
        b.iter(|| {
            let (path, line, col) = &samples[next % samples.len()];
            next += 1;
            black_box(index.goto_definition(path, *line, *col))
        })
    });
    group.bench_function("find_references", |b| {
        let mut next = 0usize;
        b.iter(|| {
            let (path, line, col) = &samples[next % samples.len()];
            next += 1;
            black_box(index.find_references(path, *line, *col))
        })
    });
    group.bench_function("symbol_at", |b| {
        let mut next = 0usize;
        b.iter(|| {
            let (path, line, col) = &samples[next % samples.len()];
            next += 1;
            black_box(index.symbol_at(path, *line, *col))
        })
    });
    group.bench_function("open", |b| {
        b.iter(|| black_box(CsIndex::open(&path).expect("打开索引失败")))
    });
    group.finish();
}

criterion_group!(benches, bench_cs_index);
criterion_main!(benches);
