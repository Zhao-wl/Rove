//! #37 L1 声明索引在真实项目上的数字：首次构建、再次打开、单文件增量、逐键符号搜索、常驻内存。
//!
//! 用法（release）：
//!   cargo run --release -p rove-lang-cs --example l1_bench -- <项目根目录> [--threads N] [--runs N]
//! 不传路径时依次读 ROVE_TEST_UNITY_LARGE / ROVE_TEST_UNITY_MEDIUM。项目文件不会被修改。
//! “热缓存”指系统文件缓存已经热（`--runs` 大于 1 时取后几次）；冷缓存数字需要先清系统缓存或重启。

use rove_base::{Cancellation, WorkspacePath};
use rove_lang_cs::l1::{L1Change, L1Index, L1Options};
use rove_workspace::{default_settings, EntryKind, Folder, Workspace, WorkspaceConfig};
use std::time::{Duration, Instant};

fn mem_mb() -> f64 {
    memory_stats::memory_stats().map_or(0.0, |m| m.physical_mem as f64 / 1048576.0)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn summary(name: &str, mut v: Vec<Duration>) {
    v.sort();
    let avg = v.iter().sum::<Duration>() / v.len().max(1) as u32;
    println!(
        "{name:<34} n={:<5} avg={:>8.2} ms  p50={:>8.2} ms  p99={:>8.2} ms  max={:>8.2} ms",
        v.len(),
        ms(avg),
        ms(v[v.len() / 2]),
        ms(v[(v.len() * 99 / 100).min(v.len() - 1)]),
        ms(*v.last().unwrap())
    );
}

fn files_of(ws: &dyn Workspace) -> Vec<WorkspacePath> {
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        let page = ws.files(start..start + 4096).expect("files page");
        start += page.items.len();
        out.extend(
            page.items
                .iter()
                .filter(|e| matches!(e.kind, EntryKind::File))
                .map(|e| e.path.clone()),
        );
        if page.items.len() < 4096 {
            return out;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<usize>().ok())
    };
    let threads = flag("--threads").unwrap_or(0);
    let runs = flag("--runs").unwrap_or(3).max(1);
    let root = args
        .iter()
        .find(|a| !a.starts_with("--") && a.parse::<usize>().is_err())
        .cloned()
        .or_else(|| std::env::var("ROVE_TEST_UNITY_LARGE").ok())
        .or_else(|| std::env::var("ROVE_TEST_UNITY_MEDIUM").ok())
        .expect("传入项目根目录，或设置 ROVE_TEST_UNITY_LARGE / ROVE_TEST_UNITY_MEDIUM");

    let config = WorkspaceConfig::from_settings(&default_settings())
        .expect("workspace defaults")
        .without_watching();
    let t = Instant::now();
    let folder = Folder::scan_root(&root, config).expect("scan workspace");
    let files = files_of(folder.as_ref());
    let cs = files
        .iter()
        .filter(|p| p.as_str().to_ascii_lowercase().ends_with(".cs"))
        .count();
    println!(
        "工作区扫描：{} 个文件（{cs} 个 .cs），{:.0} ms；线程 {}",
        files.len(),
        ms(t.elapsed()),
        if threads == 0 {
            "auto".into()
        } else {
            threads.to_string()
        }
    );

    let cancel = Cancellation::default();
    let cache = std::env::temp_dir().join(format!("rove-l1-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    let options = L1Options::new(folder.root())
        .with_cache_dir(&cache)
        .with_threads(threads);

    // 首次构建（无缓存）。第一次可能是冷系统缓存，后几次是热缓存。
    let mut keep: Option<L1Index> = None;
    for run in 0..runs {
        let before = mem_mb();
        let index = L1Index::new(L1Options {
            cache_dir: None,
            ..options.clone()
        });
        let t = Instant::now();
        let report = index.build(files.clone(), &cancel).expect("build");
        let elapsed = t.elapsed();
        let after = mem_mb();
        let stats = index.stats();
        println!(
            "首次构建 #{run}：{:>8.0} ms  解析 {} / 失败 {}  声明 {}  符号 {}  名字 {}  常驻增量 {:.1} MB（估算堆 {:.1} MB）",
            ms(elapsed),
            report.parsed,
            report.failed,
            stats.declarations,
            stats.symbols,
            stats.names,
            after - before,
            stats.approx_heap_bytes as f64 / 1048576.0
        );
        keep = Some(index);
    }
    drop(keep);
    let index = L1Index::new(options.clone());
    let t = Instant::now();
    index
        .build_and_save(files.clone(), &cancel)
        .expect("build+save");
    println!("首次构建 + 保存：{:.0} ms", ms(t.elapsed()));
    let size =
        std::fs::metadata(cache.join(rove_lang_cs::l1::CACHE_FILE_NAME)).map_or(0, |m| m.len());
    println!("缓存文件：{:.1} MB", size as f64 / 1048576.0);
    drop(index);

    let before = mem_mb();
    let index = L1Index::new(options.clone());
    let t = Instant::now();
    let report = index.build(files.clone(), &cancel).expect("reopen");
    println!(
        "再次打开（缓存 + 核对 {} 个文件的大小/修改时间）：{:.0} ms，解析 {}，常驻增量 {:.1} MB",
        report.files,
        ms(t.elapsed()),
        report.parsed,
        mem_mb() - before
    );

    // 单文件增量：移除再加回（走读盘 + 哈希 + 解析 + 并入的完整路径），不改项目文件。
    let mut cs_files: Vec<&WorkspacePath> = files
        .iter()
        .filter(|p| p.as_str().to_ascii_lowercase().ends_with(".cs"))
        .collect();
    cs_files.sort_by_key(|p| {
        std::cmp::Reverse(std::fs::metadata(folder.root().join(p.as_str())).map_or(0, |m| m.len()))
    });
    let mut inc = Vec::new();
    let step = (cs_files.len() / 200).max(1);
    for p in cs_files.iter().step_by(step).take(200) {
        index
            .apply_changes(&[L1Change::Removed((*p).clone())], &cancel)
            .unwrap();
        let t = Instant::now();
        index
            .apply_changes(&[L1Change::Upserted((*p).clone())], &cancel)
            .unwrap();
        inc.push(t.elapsed());
    }
    summary("单文件增量（均匀抽样 200 个）", inc);
    let biggest = cs_files[0];
    index
        .apply_changes(&[L1Change::Removed(biggest.clone())], &cancel)
        .unwrap();
    let t = Instant::now();
    index
        .apply_changes(&[L1Change::Upserted(biggest.clone())], &cancel)
        .unwrap();
    println!(
        "单文件增量（最大的文件，{:.0} KB）：{:.2} ms",
        std::fs::metadata(folder.root().join(biggest.as_str())).map_or(0, |m| m.len()) as f64
            / 1024.0,
        ms(t.elapsed())
    );

    // 同一个文件在空索引里构建：基本只剩读盘 + 解析，用来区分解析与并入的开销。
    let alone = L1Index::new(L1Options::new(folder.root()));
    let t = Instant::now();
    alone.build([biggest.clone()], &cancel).unwrap();
    println!(
        "  其中读盘 + 解析（空索引单独构建同一文件）：{:.2} ms，{} 条声明",
        ms(t.elapsed()),
        alone.stats().declarations
    );

    // 逐键符号搜索：模拟依次键入几个常见名字，每个前缀查一次（含打分与排序）。
    let mut keys = Vec::new();
    for word in [
        "MonoBehaviour",
        "PlayerController",
        "GetComponent",
        "Update",
        "UIManager",
        "pc",
        "Init",
        "Serialize",
        "Config",
    ] {
        for end in 1..=word.len() {
            let t = Instant::now();
            let r = index.workspace_symbols(&word[..end], 100, &cancel).unwrap();
            keys.push(t.elapsed());
            std::hint::black_box(r);
        }
    }
    summary("工作区符号搜索（每次按键）", keys);

    let mut res = Vec::new();
    for name in ["Init", "Update", "Awake", "Get", "Value", "OnEnable"] {
        let t = Instant::now();
        let r = index.member_candidates(name, &cancel).unwrap();
        res.push(t.elapsed());
        println!("  候选成员 {name:<10} {} 条（全部近似）", r.value.len());
    }
    summary("候选成员", res);
    let _ = std::fs::remove_dir_all(&cache);
}
