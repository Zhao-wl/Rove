//! #24 快速打开的每次按键：输入文本 → 排好序的结果列表（打分、前缀剪枝、取前 512 条、转成行与高亮）。
//!
//! 验收：大型 Unity 项目（`ROVE_TEST_UNITY_LARGE`，约 2.1 万个 `.cs` 外加其他文件）每次按键到列表刷新 < 16 ms（p99）。
//! - `quickopen/extend_key_synthetic_25k`：合成 2.5 万个 Unity 风格路径，键入过程中的一次按键（可剪枝）；
//!   另外在 stderr 打印一组查询逐字键入时全部按键的 p50 / p99 / max。
//! - `quickopen/first_key_synthetic_25k`：最坏的一次按键——第一个字符、没有剪枝可用。
//! - 大型项目用例（`extend_key_large`、`first_key_large`）：候选来自 #15 的 `Folder`，与正式装配一致；
//!   未设置 `ROVE_TEST_UNITY_LARGE` 时跳过，`check-budgets` 显示「未测量」。
//!
//! 只调用公开接口：`FileIndex`、`Search`、`FileResults::picks`。
use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::WorkspacePath;
use rove_test_support::UnityProject;
use rove_ui::quick_access::files::{FileIndex, FileLabels, Search, SearchOptions};
use rove_workspace::{default_settings, Folder, WorkspaceConfig};
use std::{hint::black_box, time::Instant};

fn synthetic_index(count: usize) -> FileIndex {
    use rove_workspace::FileEvent;
    let mut x: u64 = 0x2545F4914F6CDD1D;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let words = [
        "Player",
        "Enemy",
        "Weapon",
        "Inventory",
        "UI",
        "Menu",
        "Level",
        "Camera",
        "Audio",
        "Network",
        "Spawner",
        "Health",
        "Score",
        "Input",
        "Manager",
        "Controller",
        "View",
        "Item",
        "Quest",
        "Dialog",
        "Skill",
        "Buff",
        "Map",
        "Tile",
        "Particle",
        "Shader",
    ];
    let dirs = [
        "Assets",
        "Scripts",
        "Gameplay",
        "Editor",
        "Tests",
        "Plugins",
        "Prefabs",
        "Core",
        "Common",
        "Systems",
        "Runtime",
        "Generated",
        "ThirdParty",
        "Art",
        "Resources",
    ];
    let mut events = Vec::with_capacity(count);
    for i in 0..count {
        let depth = 2 + next() as usize % 5;
        let mut path: Vec<String> = (0..depth)
            .map(|_| dirs[next() as usize % dirs.len()].to_string())
            .collect();
        let name = format!(
            "{}{}{}",
            words[next() as usize % words.len()],
            words[next() as usize % words.len()],
            i
        );
        path.push(format!("{name}.cs"));
        events.push(FileEvent::Created(
            WorkspacePath::new(&path.join("/")).unwrap(),
        ));
    }
    let mut index = FileIndex::default();
    index.apply(&events);
    index
}

fn large_index() -> Option<FileIndex> {
    let root = UnityProject::Large.path()?;
    let config = WorkspaceConfig::from_settings(&default_settings())
        .ok()?
        .without_watching();
    let folder = Folder::scan_root(root, config).ok()?;
    Some(FileIndex::from_workspace(&*folder))
}

fn labels() -> FileLabels {
    FileLabels {
        recent: "最近打开".into(),
        files: "文件结果".into(),
    }
}

/// 一次按键：搜索并转成行（界面刷新列表所需的全部计算）。
fn keystroke(search: &mut Search, index: &FileIndex, text: &str, labels: &FileLabels) {
    let results = search.run(index, text, &[], SearchOptions::default());
    black_box(results.picks(index, labels));
}

const TYPED: &[&str] = &[
    "PlayerHealth",
    "enemy spawner",
    "Scripts/UI/menu",
    "cam",
    "InventoryItem12",
    "q",
];

fn percentiles(index: &FileIndex, name: &str) {
    let labels = labels();
    let mut samples = Vec::new();
    for _ in 0..20 {
        for query in TYPED {
            let mut search = Search::default();
            let mut typed = String::new();
            for ch in query.chars() {
                typed.push(ch);
                let start = Instant::now();
                keystroke(&mut search, index, &typed, &labels);
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |p: f64| samples[((samples.len() as f64 * p) as usize).min(samples.len() - 1)];
    eprintln!(
        "quickopen/{name}: 候选 {} 个，{} 次按键：p50 {:.2} ms，p99 {:.2} ms，max {:.2} ms",
        index.len(),
        samples.len(),
        at(0.5),
        at(0.99),
        samples[samples.len() - 1],
    );
}

fn bench_set(c: &mut Criterion, index: &FileIndex, suffix: &str) {
    let labels = labels();
    let mut group = c.benchmark_group("quickopen");
    group.sample_size(20);
    // 键入过程中的一次按键：上一次查询已经有结果，可以剪枝。
    group.bench_function(format!("extend_key_{suffix}"), |b| {
        b.iter_batched(
            || {
                let mut search = Search::default();
                keystroke(&mut search, index, "PlayerHealt", &labels);
                search
            },
            |mut search| keystroke(&mut search, index, "PlayerHealth", &labels),
            criterion::BatchSize::SmallInput,
        )
    });
    group.bench_function(format!("first_key_{suffix}"), |b| {
        let mut search = Search::default();
        b.iter(|| {
            search.reset();
            keystroke(&mut search, index, "a", &labels)
        })
    });
    group.finish();
}

fn benchmark(c: &mut Criterion) {
    let synthetic = synthetic_index(25_000);
    percentiles(&synthetic, "synthetic_25k");
    bench_set(c, &synthetic, "synthetic_25k");
    match large_index() {
        Some(index) => {
            percentiles(&index, "large");
            bench_set(c, &index, "large");
        }
        None => eprintln!("quickopen/large：已跳过（未设置 ROVE_TEST_UNITY_LARGE 或路径不存在）"),
    }
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
