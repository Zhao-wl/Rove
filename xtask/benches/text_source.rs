//! `rove-buffer` 文本源基准（issue #4）。
//!
//! `rove-buffer` 目前只有 #1 的契约与 `EmptyText` 占位实现（真实实现由 #7/#8 负责），
//! 所以这里量到的是「接口调用 + 空实现」的固定开销。等 #7/#8 合并后，把
//! [`text_source`] 换成真实实现（例如 `TextSource::open(path)`），基准与预算判定都不用改。
//!
//! 基准只使用 #1 的公开接口，不绕开接口测量。

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{ByteOffset, Line, Position, Utf8Column};
use rove_buffer::{EmptyText, TextSource};

/// 基准用的文本源。TODO(#7/#8)：换成真实的 `TextSource` 实现。
fn text_source() -> impl TextSource {
    EmptyText
}

fn bench_text_source(c: &mut Criterion) {
    let text = text_source();
    let mut group = c.benchmark_group("buffer");
    group.bench_function("byte_len", |b| b.iter(|| black_box(text.byte_len())));
    group.bench_function("lines", |b| b.iter(|| black_box(text.lines())));
    group.bench_function("line_range", |b| {
        b.iter(|| black_box(text.line_range(Line(0))))
    });
    group.bench_function("visit_chunks", |b| {
        b.iter(|| {
            let mut total = 0usize;
            let range = text.line_range(Line(0)).expect("第 0 行一定存在");
            text.visit_chunks(range, &mut |chunk| total += chunk.len())
                .expect("visit_chunks 不该失败");
            black_box(total)
        })
    });
    group.bench_function("position_lookup", |b| {
        b.iter(|| black_box(text.position(ByteOffset(0))))
    });
    group.bench_function("offset_lookup", |b| {
        b.iter(|| {
            black_box(text.offset(Position {
                line: Line(0),
                column: Utf8Column(0),
            }))
        })
    });
    group.finish();
}

criterion_group!(benches, bench_text_source);
criterion_main!(benches);
