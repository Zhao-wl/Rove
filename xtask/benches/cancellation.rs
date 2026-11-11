use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::Cancellation;
use std::hint::black_box;

fn cancellation(c: &mut Criterion) {
    let mut group = c.benchmark_group("cancellation");
    let root = Cancellation::default();
    group.bench_function("check_root", |b| {
        b.iter(|| black_box(black_box(&root).check()));
    });
    let child = root.child();
    group.bench_function("check_child", |b| {
        b.iter(|| black_box(black_box(&child).check()));
    });
    let mut nested = root;
    for _ in 0..8 {
        nested = nested.child();
    }
    group.bench_function("check_depth_8", |b| {
        b.iter(|| black_box(black_box(&nested).check()));
    });
    group.finish();
}

criterion_group!(benches, cancellation);
criterion_main!(benches);
