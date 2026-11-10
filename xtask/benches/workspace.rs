//! `rove-workspace` 扫描基准（issue #4）。
//!
//! `rove-workspace` 目前只有 #1 的契约与 `EmptyWorkspace` 占位实现（真实扫描由 #15 负责），
//! 所以这里量到的是「接口调用 + 空实现」的固定开销。等 #15 合并后，把 [`workspace`]
//! 换成真实的 `Workspace` 实现，基准与预算判定都不用改。
//!
//! 基准只使用 #1 的公开接口，不绕开接口测量。

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::WorkspacePath;
use rove_workspace::{EmptyWorkspace, Workspace};

/// 基准用的工作区。TODO(#15)：换成真实的 `Workspace` 实现（扫描一个 Unity 项目）。
fn workspace() -> impl Workspace {
    EmptyWorkspace
}

fn bench_workspace(c: &mut Criterion) {
    let workspace = workspace();
    let path = WorkspacePath::new("Assets/Scripts/Player.cs").expect("合法的相对路径");
    let mut group = c.benchmark_group("workspace");
    group.bench_function("revision", |b| b.iter(|| black_box(workspace.revision())));
    group.bench_function("files", |b| b.iter(|| black_box(workspace.files(0..16))));
    group.bench_function("children", |b| {
        b.iter(|| black_box(workspace.children(None, 0..16)))
    });
    group.bench_function("rules", |b| b.iter(|| black_box(workspace.rules(&path))));
    group.finish();
}

criterion_group!(benches, bench_workspace);
criterion_main!(benches);
