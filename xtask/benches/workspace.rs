//! `rove-workspace` 扫描与查询基准（issue #4；#15 换成真实实现）。
//!
//! 查询用例（revision / files / children / rules）跑在一个小型合成工程上，量的是快照查询与
//! 规则求值；`full_scan_large` 跑「大型 Unity 项目」（`ROVE_TEST_UNITY_LARGE`，约 2 万个 `.cs`），
//! 量的是热缓存下的全量扫描（并行 walker + 排除规则 + 只读规则）。
//!
//! 环境变量未设置时 `full_scan_large` 跳过而不失败（AGENTS §8）；项目路径不进仓库。
//! 基准只使用 #1 的公开接口与 #15 的构造入口，不绕开接口测量。

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::WorkspacePath;
use rove_test_support::UnityProject;
use rove_workspace::{default_settings, Folder, Workspace, WorkspaceConfig};

fn config() -> WorkspaceConfig {
    WorkspaceConfig::from_settings(&default_settings()).expect("registered workspace defaults")
}

/// 小型合成工程：12 个目录 × 20 个文件，创建在系统临时目录里，不写仓库。
fn small_fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("rove-workspace-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for group in 0..12 {
        let dir = root.join("Assets").join(format!("Group{group:02}"));
        std::fs::create_dir_all(&dir).expect("fixture dir");
        for file in 0..20 {
            std::fs::write(
                dir.join(format!("File{file:02}.cs")),
                "public class Fixture {}\n",
            )
            .expect("fixture file");
        }
    }
    root
}

fn bench_workspace(c: &mut Criterion) {
    let fixture = small_fixture();
    let folder = Folder::scan_root(&fixture, config().without_watching()).expect("scan fixture");
    let path = WorkspacePath::new("Assets/Group00/File00.cs").expect("valid workspace path");

    let mut group = c.benchmark_group("workspace");
    group.bench_function("revision", |b| b.iter(|| black_box(folder.revision())));
    group.bench_function("files", |b| b.iter(|| black_box(folder.files(0..16))));
    group.bench_function("children", |b| {
        b.iter(|| black_box(folder.children(None, 0..16)))
    });
    group.bench_function("rules", |b| b.iter(|| black_box(folder.rules(&path))));

    // 全量扫描：大型 Unity 项目（热缓存）。没有环境变量时跳过，不算失败。
    if let Some(project) = UnityProject::Large.path() {
        group.bench_function("full_scan_large", |b| {
            b.iter(|| {
                let folder = Folder::scan_root(&project, config().without_watching())
                    .expect("scan large project");
                black_box(folder.files(0..0).expect("files page").complete)
            })
        });
    } else {
        eprintln!(
            "已跳过：workspace/full_scan_large 需要环境变量 {}（大型 Unity 项目）",
            UnityProject::Large.env_var()
        );
    }
    group.finish();

    let _ = std::fs::remove_dir_all(&fixture);
}

criterion_group!(benches, bench_workspace);
criterion_main!(benches);
