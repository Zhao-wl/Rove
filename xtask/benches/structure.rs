//! #61: first layer of a compact 50 MiB JSON document through the Structure contract.
use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{Cancellation, Version};
use rove_docview::{
    structure::{ParsedStructure, StructuredLanguage},
    Structure,
};
use std::{hint::black_box, sync::Arc};

fn benchmark(c: &mut Criterion) {
    const SIZE: usize = 50 * 1024 * 1024;
    let entry = r#"{"name":"abcdef","flags":[1,2,3],"nested":{"value":true}},"#;
    let mut source = String::with_capacity(SIZE + entry.len());
    source.push_str("{\"items\":[");
    while source.len() < SIZE {
        source.push_str(entry);
    }
    source.pop();
    source.push_str("]}");
    let source: Arc<str> = source.into();
    let cancel = Cancellation::default();
    let mut group = c.benchmark_group("structure");
    group.sample_size(10);
    group.bench_function("json_compact_50mb_first_layer", |b| {
        b.iter(|| {
            let snapshot = ParsedStructure::parse(
                StructuredLanguage::Json,
                Version(1),
                source.clone(),
                5_000,
                &cancel,
            )
            .unwrap();
            let provider: &dyn Structure = &snapshot;
            black_box(provider.roots().unwrap())
        });
    });
    group.finish();
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
