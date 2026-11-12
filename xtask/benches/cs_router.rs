//! #38 查询路由的开销基准（不需要真实索引：用 `router::testing::TestIndex` 生成一份合成索引）。
//!
//! 每种查询都测三个版本，路由开销 = 路由版本 − 直接查 L2：
//! - `*_direct`：直接调用 `CsIndex::goto_definition` / `find_references`；
//! - `*_clean`：经 `CsRouter`，文件都未打开（L2 就绪、文件未修改）；
//! - `*_mapped`：查询文件与结果文件都是打开的脏缓冲区（各 100 处编辑），查询位置与每个结果
//!   都要经过位置映射。
//!
//! 合成规模：100 个文件 × 2,000 行，1 万个符号，每个符号 1 处定义 + 10 处引用（共 11 万个出现位置）。
//! 真实索引上的端到端数字见 `cs_index/router_*`（需要 `--cs-index`）。

use std::hint::black_box;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{
    ByteOffset, ByteRange, Cancellation, Line, Position, Resource, TextEdit, Utf8Column,
    WorkspacePath,
};
use rove_buffer::{RopeText, TextSource};
use rove_lang_cs::query::{NavigationKind, QueryPoint};
use rove_lang_cs::router::testing::{TestIndex, KIND_METHOD};
use rove_lang_cs::router::{
    hash_bytes, BufferStates, CsRouter, IndexHolder, ManualSemantic, Mapped,
};
use rove_lang_cs::CsIndex;

const FILES: u32 = 100;
const LINES: u32 = 2000;
const SYMBOLS_PER_FILE: u32 = 100;
const REFS: u32 = 10;
const SAMPLES: usize = 1000;
/// 两个脏文件：0 号（查询点所在）与 1 号（结果所在）。
const DIRTY: [u32; 2] = [0, 1];

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
    format!("Assets/Scripts/F{file:03}.cs")
}

/// 文件正文：每行一样长的注释（路由在 L2 路径上不读正文，只要行数对）。
fn text(file: u32) -> String {
    (0..LINES)
        .map(|l| format!("// file {file} line {l:05} ................................\n"))
        .collect()
}

struct Setup {
    _dir: std::path::PathBuf,
    index: Arc<CsIndex>,
    /// 引用位置（文件, 行, 列），索引坐标。
    refs: Vec<(u32, u32, u32)>,
}

fn setup() -> Setup {
    let mut t = TestIndex::default();
    for f in 0..FILES {
        t.file(&path(f), hash_bytes(text(f).as_bytes()));
    }
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut refs = Vec::new();
    for f in 0..FILES {
        for k in 0..SYMBOLS_PER_FILE {
            let s = t.symbol(
                &format!("Bench|M:S{f}_{k}"),
                &format!("S{f}_{k:03}"),
                KIND_METHOD,
                None,
            );
            t.def(s, f, k * (LINES / SYMBOLS_PER_FILE), 4);
            for _ in 0..REFS {
                // 列错开，避免同一位置两个出现位置。
                let (rf, line, col) = (rng.below(FILES), rng.below(LINES), 20 + 10 * rng.below(20));
                t.reference(s, rf, line, col);
                refs.push((rf, line, col));
            }
        }
    }
    // 同一位置可能被随机撞上：只保留不重复的出现位置。
    t.occurrences.sort_by_key(|o| (o.file, o.line, o.col));
    t.occurrences.dedup_by_key(|o| (o.file, o.line, o.col));
    refs.sort();
    refs.dedup();
    let dir = std::env::temp_dir().join(format!("rove-cs-router-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bench.idx");
    t.write(&file).unwrap();
    let index = Arc::new(CsIndex::open(&file).unwrap());
    // 只采样仍然存在的出现位置（去重后）。
    refs.retain(|&(f, l, c)| {
        index
            .occurrence_at(f, l, Utf8Column(u64::from(c)))
            .is_some()
    });
    Setup {
        _dir: dir,
        index,
        refs,
    }
}

fn pos(line: u64, col: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(col),
    }
}

/// 打开一个文件并做 100 次编辑：每 20 行插入一行注释（整行插入，不触及已有行）。
fn open_dirty(buffers: &BufferStates, file: u32) -> Arc<RopeText> {
    let original = text(file);
    let wp = WorkspacePath::new(&path(file)).unwrap();
    let mut current: Arc<RopeText> = Arc::new(RopeText::from_text(&original));
    buffers.opened(&wp, current.clone(), hash_bytes(original.as_bytes()));
    let mut body = original;
    // 从后往前插入，前面的行号不受影响；插入点按编辑前的文本计算。
    for k in (0..100u64).rev() {
        let line = k * 20;
        let offset = current.offset(pos(line, 0)).unwrap();
        let at = offset.0 as usize;
        body.insert_str(at, "// inserted\n");
        let after = Arc::new(RopeText::from_text(&body));
        let edit = TextEdit {
            version: current.version(),
            range: ByteRange::new(offset, ByteOffset(offset.0)).unwrap(),
            inserted: "// inserted\n".into(),
        };
        buffers
            .edited(&wp, &*current, &edit, after.clone())
            .unwrap();
        current = after;
    }
    current
}

fn bench_router(c: &mut Criterion) {
    let s = setup();
    let cancel = Cancellation::default();
    let holder: Arc<dyn IndexHolder> = s.index.clone();
    let clean = CsRouter::new(
        Arc::new(ManualSemantic::ready(holder.clone())),
        Arc::new(BufferStates::new()),
    );
    let dirty_buffers = Arc::new(BufferStates::new());
    let versions: Vec<_> = DIRTY
        .iter()
        .map(|&f| open_dirty(&dirty_buffers, f).version())
        .collect();
    let dirty = CsRouter::new(
        Arc::new(ManualSemantic::ready(holder)),
        dirty_buffers.clone(),
    );
    let map = dirty_buffers.get(&path(0)).unwrap().edits;
    println!(
        "合成索引：{} 个文件 / {} 个符号 / {} 个出现位置；脏文件 {} 处修改块",
        s.index.files().len(),
        s.index.symbols().len(),
        s.index.occurrence_count(),
        map.hunks().len()
    );

    let clean_points: Vec<_> = s
        .refs
        .iter()
        .take(SAMPLES)
        .map(|&(f, l, col)| QueryPoint {
            resource: Resource::Workspace(WorkspacePath::new(&path(f)).unwrap()),
            position: pos(u64::from(l), u64::from(col)),
            version: rove_base::Version(0),
        })
        .collect();
    let direct_points: Vec<_> = s
        .refs
        .iter()
        .take(SAMPLES)
        .map(|&(f, l, col)| (path(f), l, Utf8Column(u64::from(col))))
        .collect();
    // 脏文件里的查询点：索引坐标映射到当前缓冲区坐标。
    let dirty_points: Vec<_> = s
        .refs
        .iter()
        .filter(|r| r.0 == DIRTY[0])
        .take(SAMPLES)
        .filter_map(
            |&(_, l, col)| match map.old_to_new_clean(pos(u64::from(l), u64::from(col))) {
                Mapped::At(p) => Some(QueryPoint {
                    resource: Resource::Workspace(WorkspacePath::new(&path(DIRTY[0])).unwrap()),
                    position: p,
                    version: versions[0],
                }),
                Mapped::Modified => None,
            },
        )
        .collect();
    println!(
        "采样：干净 {} 个，脏缓冲区 {} 个",
        clean_points.len(),
        dirty_points.len()
    );

    let mut group = c.benchmark_group("cs_router");
    let cycle = |n: &mut usize, len: usize| {
        *n += 1;
        *n % len
    };
    group.bench_function("definition_direct", |b| {
        let mut n = 0;
        b.iter(|| {
            let (p, l, c) = &direct_points[cycle(&mut n, direct_points.len())];
            black_box(s.index.goto_definition(p, *l, *c))
        })
    });
    group.bench_function("definition_clean", |b| {
        let mut n = 0;
        b.iter(|| {
            let p = &clean_points[cycle(&mut n, clean_points.len())];
            black_box(clean.navigate_detailed(NavigationKind::Definition, p, &cancel))
        })
    });
    group.bench_function("definition_mapped", |b| {
        let mut n = 0;
        b.iter(|| {
            let p = &dirty_points[cycle(&mut n, dirty_points.len())];
            black_box(dirty.navigate_detailed(NavigationKind::Definition, p, &cancel))
        })
    });
    group.bench_function("references_direct", |b| {
        let mut n = 0;
        b.iter(|| {
            let (p, l, c) = &direct_points[cycle(&mut n, direct_points.len())];
            black_box(s.index.find_references(p, *l, *c))
        })
    });
    group.bench_function("references_clean", |b| {
        let mut n = 0;
        b.iter(|| {
            let p = &clean_points[cycle(&mut n, clean_points.len())];
            black_box(clean.navigate_detailed(NavigationKind::References, p, &cancel))
        })
    });
    group.bench_function("references_mapped", |b| {
        let mut n = 0;
        b.iter(|| {
            let p = &dirty_points[cycle(&mut n, dirty_points.len())];
            black_box(dirty.navigate_detailed(NavigationKind::References, p, &cancel))
        })
    });
    group.finish();
    let _ = std::fs::remove_dir_all(&s._dir);
}

criterion_group!(benches, bench_router);
criterion_main!(benches);
