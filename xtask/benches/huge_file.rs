//! 超大文件基准（issue #8）：内存映射、分块行索引、增量索引、超长行取子串。
//!
//! - `huge/open_1gb_first_screen`：`open_file` 打开 `big1g.txt` 到首屏 50 行可读
//!   （同步索引头部 1 MiB；后台索引照常启动）。验收 < 100 ms；预算 `open_1gb_text`。
//! - `huge/index_1gb_full`：打开到全量索引完成（热缓存）。验收 < 1 s（M0 0.58–0.60 s）。
//! - `huge/goto_line_1gb`：索引完成后转到任意第 N 行（`line_range` + 取内容）。验收 < 1 ms。
//! - `huge/append_1mb_after_{64mb,256mb}`：文件末尾追加 1 MiB 后 `reload` 到索引完成，
//!   耗时应与追加量成正比、与文件总大小无关（写入后首次打开触发的防病毒扫描不计入）。
//! - `huge/long_line_slice_{1mb,100mb}`：单行文件中间取 4 KiB，耗时应与行长无关。
//!
//! 没有 `big1g.txt`（`cargo xtask gen-testdata`；CI 上不生成）时 1 GB 用例跳过。
//! 1 GB 用例结束后在 stderr 打印行索引堆内存（验收 < 60 MB，M0 `Vec<u64>` 为 114 MB）。
//! 其余数据在 `ROVE_PERF_TESTDATA` 下自动生成。基准只用公开接口。

use std::hint::black_box;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{ByteOffset, Line, Span};
use rove_buffer::{open_file, LoadOptions, MappedText, Reload, SizePolicy, TextSource};
use rove_config::{DefaultSettings, SettingRegistry};

const MIB: usize = 1024 * 1024;
const WAIT: Duration = Duration::from_secs(120);

fn testdata_dir() -> PathBuf {
    let directory = std::env::var_os("ROVE_PERF_TESTDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("rove-perf-testdata"));
    std::fs::create_dir_all(&directory).expect("创建基准数据目录");
    directory
}

fn settings() -> DefaultSettings {
    let mut registry = SettingRegistry::default();
    rove_buffer::settings::register(&mut registry).expect("登记编码设置键");
    DefaultSettings(registry)
}

/// 默认策略（64 MB 起为超大文件）；小文件用例把阈值压到 1 字节，同样走映射路径。
fn open_mapped(path: &Path, policy: SizePolicy) -> Arc<MappedText> {
    let settings = settings();
    let opened =
        open_file(path, &LoadOptions::new("plaintext", &settings), policy).expect("打开超大文件");
    opened.text.mapped().expect("应当走内存映射").clone()
}

fn mapped_policy() -> SizePolicy {
    SizePolicy {
        huge_bytes: 1,
        optimize: true,
    }
}

fn bench_big1g(c: &mut Criterion) {
    let path = testdata_dir().join("big1g.txt");
    if !path.is_file() {
        eprintln!("已跳过 huge/*_1gb：缺少 big1g.txt（先 cargo xtask gen-testdata；CI 上不生成）");
        return;
    }
    let mut group = c.benchmark_group("huge");
    group.sample_size(20);
    group.bench_function("open_1gb_first_screen", |b| {
        b.iter(|| {
            let text = open_mapped(&path, SizePolicy::default());
            let mut bytes = 0usize;
            for line in 0..50 {
                let range = text.line_range(Line(line)).expect("首屏行已索引");
                text.visit_chunks(range, &mut |chunk| bytes += chunk.len())
                    .expect("读首屏");
            }
            black_box(bytes)
        })
    });
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    group.bench_function("index_1gb_full", |b| {
        b.iter(|| {
            let text = open_mapped(&path, SizePolicy::default());
            let lines = text.wait_complete(WAIT);
            assert!(lines.complete, "全量索引超时");
            black_box(lines.known)
        })
    });

    let text = open_mapped(&path, SizePolicy::default());
    let lines = text.wait_complete(WAIT);
    eprintln!(
        "big1g.txt：{} 行，行索引堆内存 {:.1} MB（{} B；M0 Vec<u64> 基线 114 MB）",
        lines.known,
        text.index_heap_bytes() as f64 / 1e6,
        text.index_heap_bytes()
    );
    group.sample_size(100);
    group.measurement_time(Duration::from_secs(3));
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    group.bench_function("goto_line_1gb", |b| {
        b.iter(|| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let line = Line(seed % lines.known);
            let range = text.line_range(line).expect("索引已完成");
            let mut bytes = 0usize;
            text.visit_chunks(range, &mut |chunk| bytes += chunk.len())
                .expect("读目标行");
            black_box(bytes)
        })
    });
    group.finish();
}

/// 每行约 80 字节的日志文件，覆盖写到 `bytes` 大小（每次运行都重建：追加会让它变长）。
fn write_log(path: &Path, bytes: usize) {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).expect("创建日志文件"));
    let mut written = 0;
    let mut index = 0u64;
    while written < bytes {
        let line = format!("[{index:010}] Unity Editor log line with some payload text 中文\n");
        file.write_all(line.as_bytes()).expect("写日志文件");
        written += line.len();
        index += 1;
    }
    file.flush().expect("写日志文件");
}

fn bench_append(c: &mut Criterion) {
    let mut chunk = String::new();
    let mut index = 0u64;
    while chunk.len() < MIB {
        chunk.push_str(&format!(
            "[append {index:08}] appended log line 追加内容\r\n"
        ));
        index += 1;
    }
    let mut group = c.benchmark_group("huge");
    group.sample_size(10);
    for (name, size) in [
        ("append_1mb_after_64mb", 64 * MIB),
        ("append_1mb_after_256mb", 256 * MIB),
    ] {
        let path = testdata_dir().join(format!("{name}.log"));
        write_log(&path, size);
        let mut text = open_mapped(&path, mapped_policy());
        text.wait_complete(WAIT);
        group.bench_function(name, |b| {
            b.iter_custom(|iterations| {
                let mut total = Duration::ZERO;
                for _ in 0..iterations {
                    std::fs::OpenOptions::new()
                        .append(true)
                        .open(&path)
                        .and_then(|mut file| file.write_all(chunk.as_bytes()))
                        .expect("追加 1 MiB");
                    // 写入后第一次打开文件会触发系统实时防病毒扫描（本机实测约 300 ms，
                    // 与文件总大小相关，不属于本实现）；先打开一次再计时。
                    drop(std::fs::File::open(&path).expect("打开日志文件"));
                    let start = Instant::now();
                    let next = match text.reload().expect("重新加载") {
                        Reload::Appended(next) => next,
                        other => panic!("应当识别为追加：{other:?}"),
                    };
                    let lines = next.wait_complete(WAIT);
                    total += start.elapsed();
                    assert!(lines.complete);
                    text = next;
                }
                total
            })
        });
        drop(text);
        let _ = std::fs::remove_file(&path);
    }
    group.finish();
}

fn bench_long_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("huge");
    for (name, size) in [
        ("long_line_slice_1mb", MIB),
        ("long_line_slice_100mb", 100 * MIB),
    ] {
        let path = testdata_dir().join(format!("{name}.txt"));
        let valid = std::fs::metadata(&path).is_ok_and(|m| m.len() == size as u64);
        if !valid {
            let block = "abcdefgh中文😀".repeat(4096);
            let mut file = std::io::BufWriter::new(std::fs::File::create(&path).expect("创建"));
            let mut written = 0;
            while written + block.len() <= size {
                file.write_all(block.as_bytes()).expect("写");
                written += block.len();
            }
            file.write_all(&vec![b'z'; size - written]).expect("写");
            file.flush().expect("写");
        }
        let text = open_mapped(&path, mapped_policy());
        text.wait_complete(WAIT);
        let line = text.line_range(Line(0)).expect("单行");
        assert_eq!(line.end().0, size as u64);
        // 中间附近的字符边界。
        let mut start = size as u64 / 2;
        while text.position(ByteOffset(start)).is_err() {
            start += 1;
        }
        let mut end = start + 4096;
        while text.position(ByteOffset(end)).is_err() {
            end += 1;
        }
        let range = Span::new(ByteOffset(start), ByteOffset(end)).expect("区间");
        group.bench_function(name, |b| {
            b.iter(|| {
                let line_len = text.line_range(Line(0)).expect("整行长度").end().0;
                let mut bytes = 0usize;
                text.visit_chunks(range.clone(), &mut |chunk| bytes += chunk.len())
                    .expect("取子串");
                black_box((line_len, bytes))
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_big1g, bench_append, bench_long_line);
criterion_main!(benches);
