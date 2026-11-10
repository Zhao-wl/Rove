//! C# v2 基准：打开 L2 语义索引，测量定义跳转 / 查找引用的响应时间，
//! 并对比“按名称查引用”与“语义查引用”的结果数量（噪音）。
//! 用法: cargo run --release -p rove-lang-cs --example cs_index_bench -- <index.idx> [samples] [--check-truncation]

use rove_lang_cs::{CsIndex, Role};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

fn mem_mb() -> f64 {
    memory_stats::memory_stats().map_or(0.0, |m| m.physical_mem as f64 / 1048576.0)
}

fn stats(name: &str, mut v: Vec<Duration>) {
    if v.is_empty() {
        return;
    }
    v.sort();
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    let avg = v.iter().sum::<Duration>() / v.len() as u32;
    println!(
        "{name:<28} n={:<6} avg={:>8.1}µs  p50={:>8.1}µs  p99={:>8.1}µs  max={:>9.1}µs",
        v.len(),
        us(avg),
        us(v[v.len() / 2]),
        us(v[v.len() * 99 / 100]),
        us(*v.last().unwrap())
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: cs_index_bench <index.idx> [samples]");
    let samples: usize = args
        .iter()
        .skip(2)
        .find(|arg| !arg.starts_with("--"))
        .map(|s| s.parse().expect("sample count"))
        .unwrap_or(20000);
    assert!(samples > 0 && samples <= u32::MAX as usize);
    let check_truncation = args.iter().any(|s| s == "--check-truncation");

    let m0 = mem_mb();
    let t = Instant::now();
    let idx = CsIndex::open(path).expect("open index");
    let open = t.elapsed();
    println!(
        "index: {:.1} MB, files={}, symbols={}, occurrences={}",
        idx.byte_len() as f64 / 1048576.0,
        idx.files().len(),
        idx.symbols().len(),
        idx.occurrence_count()
    );
    println!(
        "open: {:.3} ms, resident +{:.1} MB, string payload reads: {}",
        open.as_secs_f64() * 1e3,
        mem_mb() - m0,
        idx.string_payload_reads()
    );
    assert_eq!(idx.string_payload_reads(), 0);

    // 随机抽样引用位置（xorshift，可复现）
    let count = idx.occurrence_count();
    if count == 0 || !(0..count).any(|i| idx.occurrence(i).is_some_and(|o| !Role(o.role).is_def()))
    {
        println!("no references to sample");
        return;
    }
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut picks = Vec::with_capacity(samples);
    while picks.len() < samples {
        let i = (next_random(&mut seed) % count as u64) as usize;
        let o = idx.occurrence(i).expect("occurrence");
        if !Role(o.role).is_def() {
            let path = idx.file_path(o.file).expect("occurrence path").to_owned();
            picks.push((path, o.line, o.col, o.symbol));
        }
    }

    let mut def_t = Vec::with_capacity(samples);
    let mut miss = 0;
    let mut wrong = 0;
    for (p, l, c, sym) in &picks {
        let t = Instant::now();
        let defs = idx.goto_definition(p, *l, *c);
        let d = t.elapsed();
        def_t.push(d);
        if idx.symbol_at(p, *l, *c) != Some(*sym) {
            wrong += 1;
        }
        if defs.is_empty() {
            miss += 1; // 外部 DLL 符号没有源码定义
        }
    }
    stats(
        "goto_definition (first 200)",
        def_t.iter().take(200).copied().collect(),
    );
    // The v1 example reported the tail separately from the first 200 queries.
    stats(
        "goto_definition (after first 200)",
        def_t.iter().skip(200).copied().collect(),
    );
    stats("goto_definition", def_t);
    println!("  no source definition (external/metadata symbol): {:.1}%  position lookup mismatches: {wrong}", miss as f64 * 100.0 / samples as f64);

    let mut ref_t = Vec::new();
    let mut ref_counts = Vec::new();
    for (p, l, c, _) in &picks {
        let t = Instant::now();
        let refs = idx.find_references(p, *l, *c);
        ref_t.push(t.elapsed());
        ref_counts.push(refs.len());
    }
    stats("find_references", ref_t);
    ref_counts.sort();
    println!(
        "  result size: p50={} p99={} max={}",
        ref_counts[ref_counts.len() / 2],
        ref_counts[ref_counts.len() * 99 / 100],
        ref_counts.last().unwrap()
    );

    // 最重的查找引用：引用最多的 20 个源码符号
    let mut heavy: Vec<(usize, u32)> = idx
        .symbols()
        .iter()
        .enumerate()
        .filter(|(_, s)| s.flags & 1 != 0)
        .map(|(i, _)| (idx.reference_count(i as u32), i as u32))
        .collect();
    heavy.sort_unstable_by(|a, b| b.cmp(a));
    let mut heavy_t = Vec::new();
    for &(_, sym) in heavy.iter().take(20) {
        let t = Instant::now();
        let n = idx
            .symbol_occurrences(sym)
            .filter(|o| idx.location(o).is_some())
            .count();
        heavy_t.push(t.elapsed());
        assert!(n > 0);
    }
    stats("find_references (top-20 heavy)", heavy_t);
    if let Some(&(count, symbol)) = heavy.first() {
        println!(
            "  heaviest: {count} refs ({})",
            idx.str(idx.symbols()[symbol as usize].display)
                .expect("display")
        );
    }

    // 同名噪音：按名称查引用 vs 语义查引用
    println!("\nname-based vs semantic references (source symbols):");
    let mut by_name: HashMap<&str, Vec<u32>> = HashMap::new();
    for (i, s) in idx.symbols().iter().enumerate() {
        if s.flags & 1 != 0 {
            by_name
                .entry(idx.str(s.name).expect("name"))
                .or_default()
                .push(i as u32);
        }
    }
    for name in [
        "Update", "Init", "OnEnable", "Awake", "Show", "Get", "Value", "Count", "Dispose",
        "Refresh",
    ] {
        let Some(syms) = by_name.get(name) else {
            continue;
        };
        let total: usize = syms.iter().map(|&s| idx.reference_count(s)).sum();
        let max = syms
            .iter()
            .map(|&s| idx.reference_count(s))
            .max()
            .unwrap_or(0);
        let avg = total as f64 / syms.len() as f64;
        println!(
            "  {name:<10} distinct symbols={:<5} name-based hits={:<7} semantic per symbol: avg={:<7.1} max={}",
            syms.len(), total, avg, max
        );
    }
    println!("\nresident after queries: {:.1} MB", mem_mb());
    assert_eq!(
        wrong, 0,
        "position lookups disagreed with sampled occurrences"
    );
    if check_truncation {
        verify_truncations(Path::new(path)).expect("truncation check");
        println!("1000 random truncations rejected");
    }
}

fn next_random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn verify_truncations(path: &Path) -> io::Result<()> {
    let scratch = path.with_extension(format!("truncation-{}.tmp", std::process::id()));
    let mut copy = File::create_new(&scratch)?;
    let result = (|| {
        let size = io::copy(&mut File::open(path)?, &mut copy)?;
        let mut seed = 0x8aef_4d3b_129f_620b;
        let mut lengths: Vec<_> = (0..1000).map(|_| next_random(&mut seed) % size).collect();
        // Work from longest to shortest so every sample keeps its original prefix.
        lengths.sort_unstable_by(|a, b| b.cmp(a));
        for len in lengths {
            copy.set_len(len)?;
            if CsIndex::open(&scratch).is_ok() {
                return Err(io::Error::other(format!(
                    "accepted truncated index at {len} bytes"
                )));
            }
        }
        Ok(())
    })();
    drop(copy);
    let cleanup = fs::remove_file(scratch);
    result.and(cleanup)
}
