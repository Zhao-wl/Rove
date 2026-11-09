//! M0 基准：打开 L2 语义索引，测量定义跳转 / 查找引用的响应时间，
//! 并对比“按名称查引用”与“语义查引用”的结果数量（噪音）。
//! 用法: cargo run --release -p rove-lang-cs --example cs_index_bench -- <index.idx> [samples]

use rove_lang_cs::{CsIndex, Role};
use std::collections::HashMap;
use std::time::{Duration, Instant};

fn mem_mb() -> f64 {
    memory_stats::memory_stats().map_or(0.0, |m| m.physical_mem as f64 / 1048576.0)
}

fn stats(name: &str, mut v: Vec<Duration>) {
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
    let samples: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20000);

    let m0 = mem_mb();
    let t = Instant::now();
    let idx = CsIndex::open(path).expect("open index");
    let open = t.elapsed();
    println!(
        "index: {:.1} MB, files={}, symbols={}, occurrences={}",
        idx.byte_len() as f64 / 1048576.0,
        idx.files().len(),
        idx.symbols().len(),
        idx.occurrences().len()
    );
    println!(
        "open: {:.1} ms, resident +{:.1} MB",
        open.as_secs_f64() * 1e3,
        mem_mb() - m0
    );

    // 随机抽样引用位置（xorshift，可复现）
    let occs = idx.occurrences();
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut picks = Vec::with_capacity(samples);
    while picks.len() < samples {
        let o = &occs[(rnd() % occs.len() as u64) as usize];
        if !Role(o.role).is_def() {
            let path = idx.str(idx.files()[o.file as usize].path).to_string();
            picks.push((path, o.line, o.col + o.len as u32 / 2, o.symbol));
        }
    }

    // 冷查询（首次触达页面）与热查询分别统计
    let mut cold = Vec::new();
    let mut def_t = Vec::new();
    let mut miss = 0;
    let mut wrong = 0;
    for (i, (p, l, c, sym)) in picks.iter().enumerate() {
        let t = Instant::now();
        let defs = idx.goto_definition(p, *l, *c);
        let d = t.elapsed();
        if i < 200 {
            cold.push(d)
        } else {
            def_t.push(d)
        }
        if idx.symbol_at(p, *l, *c) != Some(*sym) {
            wrong += 1;
        }
        if defs.is_empty() {
            miss += 1; // 外部 DLL 符号没有源码定义
        }
    }
    stats("goto_definition (first 200)", cold);
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
    let mut heavy: Vec<(u32, u32)> = idx
        .symbols()
        .iter()
        .enumerate()
        .filter(|(_, s)| s.flags & 1 != 0)
        .map(|(i, s)| (s.ref_count, i as u32))
        .collect();
    heavy.sort_unstable_by(|a, b| b.cmp(a));
    let mut heavy_t = Vec::new();
    for &(_, sym) in heavy.iter().take(20) {
        let mut n = 0usize;
        let t = Instant::now();
        for o in idx.symbol_occurrences(sym) {
            let _ = idx.location(o);
            n += 1;
        }
        heavy_t.push(t.elapsed());
        assert!(n > 0);
    }
    stats("find_references (top-20 heavy)", heavy_t);
    println!(
        "  heaviest: {} refs ({})",
        heavy[0].0,
        idx.str(idx.symbols()[heavy[0].1 as usize].display)
    );

    // 同名噪音：按名称查引用 vs 语义查引用
    println!("\nname-based vs semantic references (source symbols):");
    let mut by_name: HashMap<&str, Vec<u32>> = HashMap::new();
    for (i, s) in idx.symbols().iter().enumerate() {
        if s.flags & 1 != 0 {
            by_name.entry(idx.str(s.name)).or_default().push(i as u32);
        }
    }
    for name in [
        "Update", "Init", "OnEnable", "Awake", "Show", "Get", "Value", "Count", "Dispose",
        "Refresh",
    ] {
        let Some(syms) = by_name.get(name) else {
            continue;
        };
        let total: u32 = syms
            .iter()
            .map(|&s| idx.symbols()[s as usize].ref_count)
            .sum();
        let max = syms
            .iter()
            .map(|&s| idx.symbols()[s as usize].ref_count)
            .max()
            .unwrap_or(0);
        let avg = total as f64 / syms.len() as f64;
        println!(
            "  {name:<10} distinct symbols={:<5} name-based hits={:<7} semantic per symbol: avg={:<7.1} max={}",
            syms.len(), total, avg, max
        );
    }
    println!("\nresident after queries: {:.1} MB", mem_mb());
}
