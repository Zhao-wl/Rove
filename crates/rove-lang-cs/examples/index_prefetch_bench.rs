//! #36 预读基准：冷缓存下“打开索引 → 立即连续定义跳转”的延迟，对比三种预读策略
//! （不预读 / 全速预读 / 限速预读），并单独测预读整个索引文件的耗时。
//!
//! 冷缓存的做法：用无缓冲写（`FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH`）把索引复制到
//! `<scratch>` 下的新文件，新文件的数据从未进过系统文件缓存。每个策略各用一份新拷贝。查询位置从
//! 原索引（热）上预先抽好，抽样本身不触碰冷文件。
//!
//! 用法（scratch 放在要测的盘上，例如 HDD、SATA SSD、NVMe 各跑一次）：
//! cargo run --release -p rove-lang-cs --example index_prefetch_bench -- <index.rvcsidx> <scratch-dir> [queries=200] [throttle_mb_s=64]
use rove_base::Cancellation;
use rove_lang_cs::{
    lifecycle::prefetch::{prefetch, PrefetchMode},
    CsIndex, Role, Utf8Column,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
const CHUNK: usize = 4 << 20;

#[repr(align(4096))]
struct Aligned([u8; CHUNK]);

/// 无缓冲复制：目标文件的页从未进入系统缓存。
fn cold_copy(src: &Path, dst: &Path) -> std::io::Result<u64> {
    let _ = fs::remove_file(dst);
    let mut input = File::open(src)?;
    let len = input.metadata()?.len();
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH)
        .open(dst)?;
    let mut buf = Box::new(Aligned([0; CHUNK]));
    loop {
        let mut n = 0;
        while n < CHUNK {
            let r = input.read(&mut buf.0[n..])?;
            if r == 0 {
                break;
            }
            n += r;
        }
        if n == 0 {
            break;
        }
        let padded = n.div_ceil(4096) * 4096;
        buf.0[n..padded].fill(0);
        out.write_all(&buf.0[..padded])?;
        if n < CHUNK {
            break;
        }
    }
    drop(out);
    // 去掉补齐的尾巴（只改元数据，不读数据）。
    OpenOptions::new().write(true).open(dst)?.set_len(len)?;
    Ok(len)
}

fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn picks(index: &CsIndex, n: usize) -> Vec<(String, u32, Utf8Column)> {
    let count = index.occurrence_count();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let o = index
            .occurrence((next(&mut seed) % count as u64) as usize)
            .unwrap();
        if !Role(o.role).is_def() {
            out.push((index.file_path(o.file).unwrap().to_owned(), o.line, o.col));
        }
    }
    out
}

fn pct(sorted: &[Duration], p: usize) -> f64 {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)].as_secs_f64() * 1e3
}

fn run(
    label: &str,
    source: &Path,
    scratch: &Path,
    mode: PrefetchMode,
    picks: &[(String, u32, Utf8Column)],
) {
    let path: PathBuf = scratch.join(format!("cold-{label}.rvcsidx"));
    cold_copy(source, &path).expect("cold copy");
    let start = Instant::now();
    let index = CsIndex::open(&path).expect("open cold copy");
    let open = start.elapsed();
    let cancel = Cancellation::default();
    let prefetcher = {
        let (path, cancel) = (path.clone(), cancel.clone());
        std::thread::spawn(move || prefetch(&path, mode, &cancel).unwrap())
    };
    let mut times = Vec::with_capacity(picks.len());
    let mut found = 0usize;
    for (file, line, col) in picks {
        let t = Instant::now();
        found += index.goto_definition(file, *line, *col).len();
        times.push(t.elapsed());
    }
    let queries = start.elapsed();
    let stats = prefetcher.join().unwrap();
    times.sort();
    println!(
        "{label:<10} open={:>7.1} ms  {} queries in {:>7.1} ms  p50={:>7.3} ms  p99={:>8.3} ms  max={:>8.3} ms  prefetch={:>7.1} ms ({:.0} MB/s)  defs={found}",
        open.as_secs_f64() * 1e3,
        picks.len(),
        queries.as_secs_f64() * 1e3,
        pct(&times, 50),
        pct(&times, 99),
        times.last().unwrap().as_secs_f64() * 1e3,
        stats.elapsed.as_secs_f64() * 1e3,
        stats.bytes as f64 / 1048576.0 / stats.elapsed.as_secs_f64().max(1e-9),
    );
    drop(index);
    let _ = fs::remove_file(&path);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(index_path), Some(scratch)) = (args.get(1), args.get(2)) else {
        eprintln!(
            "usage: index_prefetch_bench <index.rvcsidx> <scratch-dir> [queries] [throttle_mb_s]"
        );
        std::process::exit(2);
    };
    let queries: usize = args.get(3).map_or(200, |s| s.parse().unwrap());
    let throttle: u64 = args.get(4).map_or(64, |s| s.parse().unwrap());
    let scratch = Path::new(scratch);
    fs::create_dir_all(scratch).unwrap();
    let warm = CsIndex::open(index_path).expect("open index");
    let picks = picks(&warm, queries);
    println!(
        "index {:.1} MB, {} files; scratch {}",
        warm.byte_len() as f64 / 1048576.0,
        warm.files().len(),
        scratch.display()
    );
    drop(warm);
    // 单独的“预读整个文件”耗时（没有并发查询）。
    let only = scratch.join("cold-prefetch-only.rvcsidx");
    cold_copy(Path::new(index_path), &only).unwrap();
    let stats = prefetch(&only, PrefetchMode::Full, &Cancellation::default()).unwrap();
    println!(
        "prefetch only: {:.1} MB in {:.1} ms ({:.0} MB/s)",
        stats.bytes as f64 / 1048576.0,
        stats.elapsed.as_secs_f64() * 1e3,
        stats.bytes as f64 / 1048576.0 / stats.elapsed.as_secs_f64().max(1e-9)
    );
    let _ = fs::remove_file(&only);
    let source = Path::new(index_path);
    run("off", source, scratch, PrefetchMode::Off, &picks);
    run("full", source, scratch, PrefetchMode::Full, &picks);
    run(
        &format!("{throttle}MB/s"),
        source,
        scratch,
        PrefetchMode::Throttled(throttle << 20),
        &picks,
    );
}
