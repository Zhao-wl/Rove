//! `rove-buffer` 文本源基准（issue #4 / #7）。
//!
//! #7 把 #1 的 `TextSource` 正式实现（`RopeText`）接进来后，这里量的是真实的行/坐标
//! 查询与打开文件（读盘 + 解码 + 建 rope 快照）的成本，不再是 `EmptyText` 的占位开销。
//!
//! 基准只使用公开接口（`TextSource`、`open_path`），不给实现开测量后门。

use std::fmt::Write as _;
use std::hint::black_box;
use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{ByteOffset, Line, Position, Utf8Column};
use rove_buffer::{open_path, LoadOptions, RopeText, TextSource};
use rove_config::{DefaultSettings, SettingRegistry};

const OPEN_10MB_BYTES: usize = 10 * 1024 * 1024;

/// 基准用的文本源：400 行、每行内容不同，接近真实源码的行长分布。
fn text_source() -> RopeText {
    let mut text = String::with_capacity(32 * 1024);
    for line in 0..400 {
        writeln!(
            text,
            "public class Row{line} {{ public int Value = {line}; }} // 第 {line} 行"
        )
        .unwrap();
    }
    RopeText::new(text)
}

/// 10 MB 的测试文件，只生成一次（`ROVE_PERF_TESTDATA` 或系统临时目录）。
///
/// 基准自己在 `target/perf-testdata` 下生成这份数据（不碰 `gen-testdata` 的
/// M0 对照文件）；每次运行都校验长度，并且先写临时文件再原子改名，
/// 避免被中断后留下的不完整文件被当成有效数据参与测量。
fn ten_megabyte_file() -> PathBuf {
    let directory = std::env::var_os("ROVE_PERF_TESTDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("rove-perf-testdata"));
    std::fs::create_dir_all(&directory).expect("创建基准数据目录");
    let path = directory.join("open10mb.txt");
    // 混合 CRLF/LF、含中文，覆盖解码与换行符统计；约 10 MiB。
    let line = "public class Row0000 { public int Value = 12345; } // 中文注释\r\n";
    let mut text = String::with_capacity(OPEN_10MB_BYTES + line.len());
    while text.len() < OPEN_10MB_BYTES {
        text.push_str(line);
    }
    let expected_length = text.len() as u64;
    let valid = std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() == expected_length);
    if !valid {
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, text).expect("写 10 MB 基准文件");
        std::fs::rename(&temporary, &path).expect("替换 10 MB 基准文件");
    }
    path
}

fn bench_text_source(c: &mut Criterion) {
    let text = text_source();
    let last_line = Line(text.lines().known - 1);
    let mut group = c.benchmark_group("buffer");
    group.bench_function("byte_len", |b| b.iter(|| black_box(text.byte_len())));
    group.bench_function("lines", |b| b.iter(|| black_box(text.lines())));
    group.bench_function("line_range", |b| {
        b.iter(|| black_box(text.line_range(last_line)))
    });
    group.bench_function("visit_chunks", |b| {
        b.iter(|| {
            let mut total = 0usize;
            let range = text.line_range(last_line).expect("最后一行一定存在");
            text.visit_chunks(range, &mut |chunk| total += chunk.len())
                .expect("visit_chunks 不该失败");
            black_box(total)
        })
    });
    group.bench_function("position_lookup", |b| {
        b.iter(|| {
            let middle = ByteOffset(text.byte_len() / 2);
            black_box(text.position(middle))
        })
    });
    group.bench_function("offset_lookup", |b| {
        b.iter(|| {
            black_box(text.offset(Position {
                line: last_line,
                column: Utf8Column(0),
            }))
        })
    });
    group.finish();
}

fn bench_open_10mb(c: &mut Criterion) {
    let path = ten_megabyte_file();
    let mut registry = SettingRegistry::default();
    rove_buffer::settings::register(&mut registry).expect("登记编码设置键");
    let settings = DefaultSettings(registry);
    let options = LoadOptions::new("plaintext", &settings);
    let mut group = c.benchmark_group("buffer");
    group.bench_function("open_10mb", |b| {
        b.iter(|| {
            let loaded = open_path(&path, &options).expect("打开 10 MB 基准文件");
            black_box(loaded.text.lines().known)
        })
    });
    group.finish();
}

criterion_group!(benches, bench_text_source, bench_open_10mb);
criterion_main!(benches);
