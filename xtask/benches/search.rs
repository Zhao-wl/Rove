//! 全局搜索基准（#25）：只经 `rove_search::search` 公开入口测量。
//!
//! - `search/full_synthetic`、`search/first_batch_synthetic`：系统临时目录里 2000 个合成文件（约 40 MB），
//!   CI 上也能跑；
//! - `search/first_batch_large`、`search/full_large`：大型 Unity 项目（`ROVE_TEST_UNITY_LARGE`）里搜常见标识符
//!   `GetComponent`，热缓存（criterion 的预热轮次即预热文件缓存）。环境变量未设置时跳过（AGENTS §8）。
//!
//! “首批”= 从调用到第一批结果交给回调的时间；“全部”= 调用返回（所有工作线程已结束）。

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::Cancellation;
use rove_search::{search, settings, SearchConfig, SearchQuery};
use rove_test_support::UnityProject;

fn config() -> SearchConfig {
    SearchConfig::from_settings(Arc::new(settings::default_settings()))
}

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("rove-search-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let line = "    public void Update() { transform.position += velocity * Time.deltaTime; }\n";
    for dir in 0..40 {
        let d = root.join("Assets").join(format!("Group{dir:02}"));
        std::fs::create_dir_all(&d).expect("fixture dir");
        for file in 0..50 {
            let mut text = line.repeat(260);
            if file % 7 == 0 {
                text.push_str("    var body = GetComponent<Rigidbody>();\n");
            }
            std::fs::write(d.join(format!("File{file:02}.cs")), text).expect("fixture file");
        }
    }
    root
}

/// 一次搜索：返回 (首批耗时, 全部耗时)。
fn once(root: &Path, query: &SearchQuery, config: &SearchConfig) -> (Duration, Duration) {
    let start = Instant::now();
    let mut first = None;
    let summary = search(root, query, config, &Cancellation::default(), |batch| {
        first.get_or_insert_with(|| start.elapsed());
        black_box(batch);
    })
    .expect("search");
    let total = start.elapsed();
    black_box(summary);
    (first.unwrap_or(total), total)
}

fn bench_pair(c: &mut Criterion, name: &str, root: &Path) {
    let query = SearchQuery::new("GetComponent");
    let config = config();
    let mut group = c.benchmark_group("search");
    group.sample_size(10);
    group.bench_function(format!("first_batch_{name}"), |b| {
        b.iter_custom(|iters| (0..iters).map(|_| once(root, &query, &config).0).sum())
    });
    group.bench_function(format!("full_{name}"), |b| {
        b.iter_custom(|iters| (0..iters).map(|_| once(root, &query, &config).1).sum())
    });
    group.finish();
}

fn bench_search(c: &mut Criterion) {
    let root = fixture();
    bench_pair(c, "synthetic", &root);
    let _ = std::fs::remove_dir_all(&root);

    match UnityProject::Large.path() {
        Some(project) => bench_pair(c, "large", &project),
        None => eprintln!(
            "已跳过：search/first_batch_large、search/full_large 需要环境变量 {}（大型 Unity 项目）",
            UnityProject::Large.env_var()
        ),
    }
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
