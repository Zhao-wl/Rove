//! #73：打开未知扩展名时，后台读取前缀并检测语言的开销。

use criterion::{criterion_group, criterion_main, Criterion};
use rove_buffer::RopeText;
use rove_syntax::detection::{detect_text, MAX_BYTES};
use std::hint::black_box;

fn detect(c: &mut Criterion) {
    let mut group = c.benchmark_group("language_detect");
    let jsonc = format!("/* config */ [{}0,]", "1,".repeat((MAX_BYTES - 18) / 2));
    assert!(jsonc.len() <= MAX_BYTES);
    assert_eq!(
        detect_text(&RopeText::from_text(&jsonc))
            .unwrap()
            .unwrap()
            .language,
        "jsonc"
    );
    let text = "a".repeat(MAX_BYTES);
    let yamlish = "status: ready\nactivity\n".repeat(400);
    for (id, sample) in [
        ("full_prefix_jsonc", jsonc.as_str()),
        ("full_prefix_plaintext", text.as_str()),
        ("full_prefix_sparse_lines", yamlish.as_str()),
        ("xml_declaration", "<?xml version=\"1.0\"?>\n<project/>"),
        ("lua_shebang", "#!/usr/bin/env lua\nprint(1)"),
    ] {
        let snapshot = RopeText::from_text(sample);
        group.bench_function(id, |b| {
            b.iter(|| black_box(detect_text(black_box(&snapshot)).unwrap()));
        });
    }
    group.finish();
}

criterion_group!(benches, detect);
criterion_main!(benches);
