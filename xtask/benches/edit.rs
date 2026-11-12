//! #46 编辑核心基准：10 万行文件中插入一个字符（不含高亮与绘制）。
//!
//! - `edit/insert_char_100k_lines`：`EditSession::type_text` 在文件中部输入一个字符——事务校验、
//!   rope 写时复制、新快照、`InputEdit` 与按行失效通知、选区映射、撤销记录，全部计入；
//! - `edit/buffer_apply_100k_lines`：只走 `EditBuffer::apply`（不含会话与撤销栈）；
//! - `edit/type_undo_100k_lines`：输入一个字符再撤销（撤销路径的成本）。
//!
//! 预算见 `perf/budgets.toml` 的 `edit_insert_char_100k_lines`（p99 < 100 µs）。只调用公开接口。

use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{ByteOffset, Span};
use rove_buffer::edit::{EditBuffer, EditOptions, EditSession, Selection, Selections, Transaction};
use rove_buffer::{Eol, LineEndings, TextSource};

const LINES: usize = 100_000;

/// 10 万行、接近真实 C# 的行长分布（约 4.4 MB）。
fn hundred_thousand_lines() -> String {
    let mut text = String::with_capacity(LINES * 48);
    for line in 0..LINES {
        writeln!(
            text,
            "        public int Value{line} {{ get; set; }} // 第 {line} 行"
        )
        .unwrap();
    }
    text
}

fn session() -> EditSession {
    let text = hundred_thousand_lines();
    let mut session = EditSession::new(
        EditBuffer::new(&text, LineEndings::Uniform(Eol::Lf)),
        EditOptions::default(),
    );
    session.set_read_only(Arc::from([]));
    let middle = session
        .snapshot()
        .line_range(rove_base::Line(LINES as u64 / 2))
        .unwrap();
    session.set_selections(Selections::single(Selection::cursor(ByteOffset(
        middle.start().0 + 8,
    ))));
    session
}

fn bench(c: &mut Criterion) {
    let mut group = c.benchmark_group("edit");
    group.measurement_time(Duration::from_secs(5));

    let mut typing = session();
    let now = Instant::now();
    group.bench_function("insert_char_100k_lines", |b| {
        b.iter(|| black_box(typing.type_text("x", now).unwrap()))
    });
    drop(typing);

    let mut buffer = EditBuffer::new(&hundred_thousand_lines(), LineEndings::Uniform(Eol::Lf));
    // 中间那一行的行首（ASCII，必然是字符边界）。
    let at = buffer
        .text()
        .line_range(rove_base::Line(LINES as u64 / 2))
        .unwrap()
        .start()
        .0;
    group.bench_function("buffer_apply_100k_lines", |b| {
        b.iter(|| {
            let insert =
                Transaction::single(Span::new(ByteOffset(at), ByteOffset(at)).unwrap(), "x");
            black_box(buffer.apply(&insert).unwrap())
        })
    });
    drop(buffer);

    let mut undo = session();
    group.bench_function("type_undo_100k_lines", |b| {
        b.iter(|| {
            undo.type_text("x", now).unwrap();
            undo.push_undo_stop();
            black_box(undo.undo().unwrap())
        })
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
