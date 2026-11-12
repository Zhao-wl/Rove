//! #35 “基础索引 + 覆盖层”的合并查询基准：覆盖层段数递增（0 / 1 / 16 / 128 / 1024），定义跳转与查找引用
//! 的耗时应保持不变（合并在构建覆盖层时一次完成，查询仍是基础索引的二分 + 覆盖层的哈希表与文件内二分）。
//!
//! 合成规模：1,000 个文件 × 2,000 行，5 万个符号，每个符号 1 处定义 + 10 处引用（共 55 万个出现位置）。
//! 第 k 个覆盖层段重写第 k 个文件（出现位置整体下移一行），引用在被改写的文件与未改写的文件之间混合。
//! 另测 1,024 段的合并构建耗时：冷（`build_1024`）与复用键查找缓存的重建（`rebuild_1024_cached`，生命周期的实际路径）。

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::Utf8Column;
use rove_lang_cs::overlay::{
    KeyCache, L2View, OvKey, Overlay, SegFile, SegOccurrence, SegSymbol, Segment,
};
use rove_lang_cs::router::testing::{TestIndex, KIND_METHOD};
use rove_lang_cs::{CsIndex, Role};

const FILES: u32 = 1000;
const LINES: u32 = 2000;
const SYMBOLS_PER_FILE: u32 = 50;
const REFS: u32 = 10;
const COUNTS: [usize; 5] = [0, 1, 16, 128, 1024];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n)) as u32
    }
}

fn path(file: u32) -> String {
    format!("Assets/Scripts/F{file:04}.cs")
}

fn key(f: u32, k: u32) -> String {
    format!("Bench|M:S{f:04}_{k:03}")
}

struct Setup {
    dir: std::path::PathBuf,
    index: CsIndex,
    /// 每个文件的出现位置（行, 列, 符号, 是否定义）。
    per_file: Vec<Vec<(u32, u32, u32, bool)>>,
}

fn setup() -> Setup {
    let mut t = TestIndex::default();
    for f in 0..FILES {
        t.file(&path(f), u64::from(f));
    }
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    // 键按 Ordinal 序加入（与写入端一致，覆盖层按键二分找基础符号）
    for f in 0..FILES {
        for k in 0..SYMBOLS_PER_FILE {
            let s = t.symbol(&key(f, k), &format!("S{f}_{k:03}"), KIND_METHOD, None);
            t.def(s, f, k * (LINES / SYMBOLS_PER_FILE), 4);
            for _ in 0..REFS {
                t.reference(
                    s,
                    rng.below(FILES),
                    1 + rng.below(LINES - 2),
                    20 + 10 * rng.below(20),
                );
            }
        }
    }
    t.occurrences.sort_by_key(|o| (o.file, o.line, o.col));
    t.occurrences.dedup_by_key(|o| (o.file, o.line, o.col));
    let mut per_file = vec![Vec::new(); FILES as usize];
    for o in &t.occurrences {
        per_file[o.file as usize].push((o.line, o.col, o.symbol, o.definition));
    }
    let dir = std::env::temp_dir().join(format!("rove-cs-overlay-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bench.idx");
    t.write(&file).unwrap();
    Setup {
        index: CsIndex::open(&file).unwrap(),
        dir,
        per_file,
    }
}

/// 第 `file` 个文件的新版本：出现位置整体下移一行，符号用键引用。
fn segment(s: &Setup, sequence: u64, file: u32) -> Segment {
    let mut seg = Segment {
        sequence,
        ..Default::default()
    };
    let mut ids = std::collections::HashMap::new();
    let mut occurrences = Vec::new();
    for &(line, col, symbol, def) in &s.per_file[file as usize] {
        let next = seg.symbols.len() as u32;
        let id = *ids.entry(symbol).or_insert_with(|| {
            let (f, k) = (symbol / SYMBOLS_PER_FILE, symbol % SYMBOLS_PER_FILE);
            seg.symbols.push(SegSymbol {
                key: OvKey::Global(key(f, k)),
                name: format!("S{f}_{k:03}"),
                display: format!("S{f}_{k:03}"),
                kind: KIND_METHOD,
                flags: 1,
                container: None,
            });
            next
        });
        occurrences.push(SegOccurrence {
            line: line + 1,
            col,
            len: 8,
            symbol: id,
            role: if def { 2 } else { 1 },
        });
    }
    seg.files.push(SegFile {
        path: path(file),
        hash: 1_000_000 + u64::from(file),
        occurrences,
        ..Default::default()
    });
    seg
}

fn bench_overlay(c: &mut Criterion) {
    let s = setup();
    let all: Vec<Segment> = (0..*COUNTS.last().unwrap())
        .map(|k| segment(&s, k as u64 + 1, k as u32 % FILES))
        .collect();
    // 查询点：未改写文件（≥ 1024 号）里的引用，与改写过的文件（0–1023 号，坐标下移一行）里的引用。
    let mut rng = Rng(42);
    let points: Vec<(u32, u32, u32)> = (0..1000)
        .map(|_| {
            let f = rng.below(FILES);
            let list = &s.per_file[f as usize];
            let (line, col, _, _) = list[rng.below(list.len() as u32) as usize];
            (f, line, col)
        })
        .collect();
    let mut group = c.benchmark_group("cs_overlay");
    for &count in &COUNTS {
        let overlay = (count > 0).then(|| Overlay::build(&s.index, &all[..count]));
        let view = L2View::new(&s.index, overlay.as_ref());
        let shift = |f: u32| u32::from((f as usize) < count);
        let resolved: Vec<_> = points
            .iter()
            .map(|&(f, l, c)| (f, l + shift(f), Utf8Column(u64::from(c))))
            .collect();
        assert!(resolved
            .iter()
            .all(|&(f, l, c)| view.occurrence_at(f, l, c).is_some()));
        let mut n = 0;
        group.bench_function(format!("definition_{count:04}"), |b| {
            b.iter(|| {
                n = (n + 1) % resolved.len();
                let (f, l, c) = resolved[n];
                let o = view.occurrence_at(f, l, c).unwrap();
                black_box(view.definitions(o.symbol))
            })
        });
        group.bench_function(format!("references_{count:04}"), |b| {
            b.iter(|| {
                n = (n + 1) % resolved.len();
                let (f, l, c) = resolved[n];
                let o = view.occurrence_at(f, l, c).unwrap();
                let occs = view.occurrences(o.symbol);
                black_box(occs.iter().filter(|o| Role(o.role).is_def()).count())
            })
        });
    }
    group.sample_size(10);
    group.bench_function("build_1024", |b| {
        b.iter(|| black_box(Overlay::build(&s.index, &all)))
    });
    // 生命周期的实际路径：同一世代内键的查找结果复用（日志增长后的重建）
    let mut cache = KeyCache::default();
    Overlay::build_cached(&s.index, &all, &mut cache);
    group.bench_function("rebuild_1024_cached", |b| {
        b.iter(|| black_box(Overlay::build_cached(&s.index, &all, &mut cache)))
    });
    group.finish();
    drop(s.index);
    let _ = std::fs::remove_dir_all(&s.dir);
}

criterion_group!(benches, bench_overlay);
criterion_main!(benches);
