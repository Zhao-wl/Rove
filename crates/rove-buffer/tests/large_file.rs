//! 超大文件的端到端测试（#8）：公开的 `open_file` → 分级与只读原因 → 索引进度 →
//! 转到行 → 追加内容的增量索引 → 截断/改写后重建。
//!
//! 阈值用很小的 `SizePolicy`，让几 MB 的文件就走超大文件路径（同步索引 1 MiB + 后台分块）。
//! 运行：`cargo test -p rove-buffer --test large_file`

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rove_base::*;
use rove_buffer::{
    disabled_features, open_file, session_toggle_allowed, settings, Encoding, FileText,
    LoadOptions, MappedText, ReadOnlyReason, Reload, RopeText, SizeClass, SizePolicy, TextSource,
    SYNC_INDEX_BYTES,
};
use rove_config::{DefaultSettings, SettingRegistry};

const MIB: u64 = 1024 * 1024;
const WAIT: Duration = Duration::from_secs(60);

fn default_settings() -> DefaultSettings {
    let mut registry = SettingRegistry::default();
    settings::register(&mut registry).unwrap();
    DefaultSettings(registry)
}

fn huge_policy() -> SizePolicy {
    SizePolicy {
        huge_bytes: 1,
        optimize: true,
    }
}

fn mapped(file: &FileText) -> Arc<MappedText> {
    file.mapped().expect("超大文件应当走内存映射").clone()
}

fn open_huge(path: &Path) -> Arc<MappedText> {
    let settings = default_settings();
    let opened = open_file(
        path,
        &LoadOptions::new("plaintext", &settings),
        huge_policy(),
    )
    .unwrap();
    let text = mapped(&opened.text);
    text.wait_complete(WAIT);
    text
}

fn collect(text: &dyn TextSource, range: ByteRange) -> String {
    let mut out = String::new();
    text.visit_chunks(range, &mut |chunk| out.push_str(chunk))
        .unwrap();
    out
}

fn line(text: &dyn TextSource, line: u64) -> String {
    collect(text, text.line_range(Line(line)).unwrap())
}

/// 把映射文本与同内容的 rope 快照逐行比较（行号、区间与内容都一致）。
fn assert_same_lines(mapped: &dyn TextSource, content: &str) {
    let rope = RopeText::from_text(content);
    let lines = mapped.lines();
    assert!(lines.complete);
    assert_eq!(lines.known, rope.lines().known);
    assert_eq!(mapped.byte_len(), content.len() as u64);
    for index in 0..lines.known {
        assert_eq!(
            mapped.line_range(Line(index)),
            rope.line_range(Line(index)),
            "第 {index} 行"
        );
    }
    assert_eq!(
        mapped.line_range(Line(lines.known)),
        Err(Error::OutOfBounds)
    );
}

fn numbered_lines(count: usize, eol: &str) -> String {
    let mut text = String::new();
    for index in 0..count {
        text.push_str(&format!(
            "line {index:08} 中文内容 abcdefghijklmnopqrstuvwxyz{eol}"
        ));
    }
    text
}

/// 入口：打开超大文件 → 只读原因 → 进度 → 转到行。
#[test]
fn open_huge_file_reports_read_only_progress_and_goes_to_line() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("huge.log");
    // 约 6 MB：头部 1 MiB 同步索引，其余在后台。
    let content = numbered_lines(100_000, "\n");
    std::fs::write(&path, &content).unwrap();

    let settings = default_settings();
    let policy = SizePolicy {
        huge_bytes: 4 * MIB,
        optimize: true,
    };
    let opened = open_file(&path, &LoadOptions::new("plaintext", &settings), policy).unwrap();
    assert_eq!(opened.metadata.size, SizeClass::Huge);
    assert_eq!(
        opened.metadata.read_only.as_ref(),
        [ReadOnlyReason::Session, ReadOnlyReason::HugeFile]
    );
    assert!(!session_toggle_allowed(&opened.metadata.read_only));
    assert_eq!(disabled_features(opened.metadata.size).len(), 10);
    assert_eq!(opened.metadata.encoding, Encoding::Utf8);
    assert!(!opened.detection.guessed);
    let text = mapped(&opened.text);
    assert!(text.is_memory_mapped());

    // 打开返回时头部 1 MiB 已经索引好，首屏可以直接读。
    let first = text.lines();
    assert!(first.indexed_bytes.0 >= SYNC_INDEX_BYTES);
    assert_eq!(first.total_bytes, content.len() as u64);
    assert!(first.known > 1000);
    assert_eq!(line(text.as_ref(), 0), content.lines().next().unwrap());

    // 进度单调增长直到完成；完成前最终行数未知（complete = false）。
    let done = text.wait_complete(WAIT);
    assert!(done.complete);
    assert!(done.known >= first.known);
    assert_eq!(done.indexed_bytes.0, content.len() as u64);
    assert_eq!(done.known, 100_001);

    // 转到行。
    let target = 87_654;
    let range = text.line_range(Line(target)).unwrap();
    assert_eq!(
        collect(text.as_ref(), range.clone()),
        content.lines().nth(target as usize).unwrap()
    );
    assert_eq!(
        text.position(range.start()).unwrap(),
        Position {
            line: Line(target),
            column: Utf8Column(0)
        }
    );

    // 文档模型拿到的是同一个文本源。
    let source = opened.text.source();
    assert_eq!(source.version(), text.version());
}

#[test]
fn lines_beyond_the_index_are_not_ready_rather_than_out_of_bounds() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("huge.log");
    // 足够大，保证打开时后台还没扫完（32 MiB 一块）。
    let content = numbered_lines(1_200_000, "\n");
    std::fs::write(&path, &content).unwrap();
    let settings = default_settings();
    let opened = open_file(
        &path,
        &LoadOptions::new("plaintext", &settings),
        huge_policy(),
    )
    .unwrap();
    let text = mapped(&opened.text);
    let lines = text.lines();
    if !lines.complete {
        assert_eq!(
            text.line_range(Line(1_100_000)),
            Err(Error::NotReady),
            "未索引的行必须报 NotReady"
        );
    }
    let done = text.wait_for_lines(1_100_001, WAIT);
    assert!(done.known >= 1_100_001);
    assert_eq!(
        line(text.as_ref(), 1_100_000),
        content.lines().nth(1_100_000).unwrap()
    );
    text.wait_complete(WAIT);
    assert_eq!(text.line_range(Line(5_000_000)), Err(Error::OutOfBounds));
}

#[test]
fn size_classes_through_open_file() {
    let directory = tempfile::tempdir().unwrap();
    let settings = default_settings();
    let options = LoadOptions::new("plaintext", &settings);
    let many_lines = directory.path().join("many-lines.txt");
    std::fs::write(&many_lines, "\n".repeat(300_000)).unwrap();
    let small = directory.path().join("small.txt");
    std::fs::write(&small, "hello\n").unwrap();

    let default = SizePolicy::default();
    let opened = open_file(&many_lines, &options, default).unwrap();
    assert_eq!(opened.metadata.size, SizeClass::Large);
    assert!(opened.text.mapped().is_none());
    assert_eq!(
        opened.metadata.read_only.as_ref(),
        [ReadOnlyReason::Session],
        "大文件仍然可以编辑"
    );
    let off = SizePolicy {
        optimize: false,
        ..default
    };
    assert_eq!(
        open_file(&many_lines, &options, off).unwrap().metadata.size,
        SizeClass::Normal
    );
    assert_eq!(
        open_file(&small, &options, default).unwrap().metadata.size,
        SizeClass::Normal
    );
    // largeFileOptimizations = false 不影响超大文件级。
    let huge_off = SizePolicy {
        huge_bytes: 100,
        optimize: false,
    };
    assert_eq!(
        open_file(&many_lines, &options, huge_off)
            .unwrap()
            .metadata
            .size,
        SizeClass::Huge
    );
}

/// 追加 1 MB：只索引新增部分，旧行号与区间不变（滚动位置保持）。
#[test]
fn appending_indexes_only_the_new_tail() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Editor.log");
    let mut content = numbered_lines(40_000, "\r\n");
    std::fs::write(&path, &content).unwrap();
    let before = open_huge(&path);
    let old_lines = before.lines().known;
    let old_ranges: Vec<_> = (0..old_lines - 1)
        .step_by(997)
        .map(|index| (index, before.line_range(Line(index)).unwrap()))
        .collect();

    let appended = numbered_lines(20_000, "\n");
    assert!(appended.len() as u64 >= MIB);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(appended.as_bytes())
        .unwrap();
    content.push_str(&appended);

    let after = match before.reload().unwrap() {
        Reload::Appended(text) => text,
        other => panic!("应当识别为追加，实际 {other:?}"),
    };
    after.wait_complete(WAIT);
    assert_ne!(after.version(), before.version());
    for (index, range) in old_ranges {
        assert_eq!(after.line_range(Line(index)).unwrap(), range);
    }
    assert_same_lines(after.as_ref(), &content);
    // 旧快照不受影响。
    assert_eq!(before.lines().known, old_lines);
    assert!(matches!(after.reload().unwrap(), Reload::Unchanged));
}

#[test]
fn appended_lf_completes_a_trailing_cr() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cr.log");
    std::fs::write(&path, "a\r\nb\r").unwrap();
    let before = open_huge(&path);
    assert_same_lines(before.as_ref(), "a\r\nb\r");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"\nc")
        .unwrap();
    let Reload::Appended(after) = before.reload().unwrap() else {
        panic!("应当识别为追加");
    };
    after.wait_complete(WAIT);
    assert_same_lines(after.as_ref(), "a\r\nb\r\nc");
}

/// 中间内容被改写：不能当作追加，必须整体重建，行号正确。
#[test]
fn rewriting_the_middle_rebuilds_the_index() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rewrite.log");
    let content = numbered_lines(20_000, "\n");
    std::fs::write(&path, &content).unwrap();
    let before = open_huge(&path);

    // 同样长度，中间的换行被改成空格、另一处空格改成换行（行号整体错位）。
    let mut bytes = content.clone().into_bytes();
    let middle = bytes.len() / 2;
    let newline = middle + bytes[middle..].iter().position(|&b| b == b'\n').unwrap();
    bytes[newline] = b' ';
    let space = middle + bytes[middle..].iter().position(|&b| b == b' ').unwrap();
    bytes[space] = b'\n';
    // 映射期间允许改写内容；旧快照读到的字节可能变化，但不能 panic。
    {
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
    }
    for index in [0u64, 9_999, 10_000, 19_999, 20_000] {
        let _ = before
            .line_range(Line(index))
            .map(|range| collect(before.as_ref(), range));
    }
    // 修改时间的精度可能不足以区分；头部也没变，所以显式把修改时间推后。
    let later = std::time::SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let after = match before.reload().unwrap() {
        Reload::Replaced(text) => text,
        other => panic!("应当整体重建，实际 {other:?}"),
    };
    after.wait_complete(WAIT);
    assert_same_lines(after.as_ref(), std::str::from_utf8(&bytes).unwrap());
}

/// 截断：Windows 上被映射的文件不能被截断（系统拒绝），旧快照照常可读；能截断的平台上
/// 重新加载必须整体重建。非 UTF-8 的超大文件内容是解码副本、不保留映射，截断总能发生。
#[test]
fn truncation_never_crashes_and_rebuilds_line_numbers() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("truncate.log");
    let content = numbered_lines(30_000, "\n");
    std::fs::write(&path, &content).unwrap();
    let before = open_huge(&path);
    let keep = content.len() / 3;
    let truncated = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_len(keep as u64));
    match truncated {
        Ok(()) => {
            let Reload::Replaced(after) = before.reload().unwrap() else {
                panic!("截断后应当整体重建");
            };
            after.wait_complete(WAIT);
            assert_same_lines(after.as_ref(), &content[..keep]);
        }
        Err(_) => {
            assert_same_lines(before.as_ref(), &content);
            assert!(matches!(before.reload().unwrap(), Reload::Unchanged));
        }
    }

    // UTF-16 LE（带 BOM）：解码副本，不持有映射。
    let utf16 = directory.path().join("utf16.log");
    let text = numbered_lines(5_000, "\r\n");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&utf16, &bytes).unwrap();
    let settings = default_settings();
    let opened = open_file(
        &utf16,
        &LoadOptions::new("plaintext", &settings),
        huge_policy(),
    )
    .unwrap();
    assert_eq!(opened.metadata.encoding, Encoding::Utf16Le);
    assert!(opened.metadata.bom);
    let before = mapped(&opened.text);
    assert!(!before.is_memory_mapped());
    before.wait_complete(WAIT);
    assert_same_lines(before.as_ref(), &text);
    let keep_chars = text.len() / 2;
    let keep_chars = (0..=keep_chars)
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap();
    let prefix = &text[..keep_chars];
    let prefix_bytes = 2 + prefix.encode_utf16().count() * 2;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&utf16)
        .unwrap()
        .set_len(prefix_bytes as u64)
        .unwrap();
    let Reload::Replaced(after) = before.reload().unwrap() else {
        panic!("截断后应当整体重建");
    };
    after.wait_complete(WAIT);
    assert_same_lines(after.as_ref(), prefix);
}

#[test]
fn edge_cases_match_the_rope_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, String)> = vec![
        (
            "no-final-newline",
            numbered_lines(3, "\n").trim_end().to_string(),
        ),
        ("only-empty-lines", "\n".repeat(70_000)),
        ("only-crlf", "\r\n".repeat(70_000)),
        ("only-cr", "\r".repeat(1000)),
        ("single", "x".repeat(10_000)),
    ];
    for (name, content) in cases {
        let path = directory.path().join(name);
        std::fs::write(&path, &content).unwrap();
        let text = open_huge(&path);
        assert_same_lines(text.as_ref(), &content);
    }
    // 空文件不会达到超大文件阈值（阈值至少 1 字节），走普通路径。
    let empty = directory.path().join("empty");
    std::fs::write(&empty, "").unwrap();
    let settings = default_settings();
    let opened = open_file(
        &empty,
        &LoadOptions::new("plaintext", &settings),
        huge_policy(),
    )
    .unwrap();
    assert_eq!(opened.metadata.size, SizeClass::Normal);
    assert_eq!(opened.text.source().lines().known, 1);
}

/// 单行 100 MB：整行字节长度 O(1)，按区间取子串只读请求的那一段。
#[test]
fn single_hundred_megabyte_line() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("one-line.txt");
    let length = 100 * MIB as usize;
    {
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let block = "abcdefgh中文😀".repeat(4096);
        let mut written = 0;
        while written + block.len() <= length {
            file.write_all(block.as_bytes()).unwrap();
            written += block.len();
        }
        file.write_all(&vec![b'z'; length - written]).unwrap();
    }
    let text = open_huge(&path);
    let lines = text.lines();
    assert_eq!(lines.known, 1);
    let range = text.line_range(Line(0)).unwrap();
    assert_eq!(range.end().0, length as u64);
    // 取末尾附近的一段。
    let tail = Span::new(ByteOffset(length as u64 - 8), ByteOffset(length as u64)).unwrap();
    assert_eq!(collect(text.as_ref(), tail), "zzzzzzzz".get(..8).unwrap());
    // 中间：先找一个字符边界。
    let mut start = length as u64 / 2;
    while text.position(ByteOffset(start)).is_err() {
        start += 1;
    }
    let piece = collect(
        text.as_ref(),
        Span::new(ByteOffset(start), ByteOffset(start + 16)).unwrap_or_else(|_| unreachable!()),
    );
    assert_eq!(piece.len(), 16);
}

#[test]
fn utf8_bom_is_skipped_and_binary_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let settings = default_settings();
    let bom = directory.path().join("bom.txt");
    std::fs::write(&bom, b"\xEF\xBB\xBFab\ncd").unwrap();
    let opened = open_file(
        &bom,
        &LoadOptions::new("plaintext", &settings),
        huge_policy(),
    )
    .unwrap();
    assert!(opened.metadata.bom);
    let text = mapped(&opened.text);
    text.wait_complete(WAIT);
    assert_eq!(text.byte_len(), 5);
    assert_same_lines(text.as_ref(), "ab\ncd");

    let binary = directory.path().join("binary.bin");
    std::fs::write(&binary, b"abc\0def").unwrap();
    let mut options = LoadOptions::new("plaintext", &settings);
    assert_eq!(
        open_file(&binary, &options, huge_policy()).err(),
        Some(Error::Unsupported)
    );
    options.force_open = true;
    let opened = open_file(&binary, &options, huge_policy()).unwrap();
    assert_eq!(opened.text.source().byte_len(), 7);
}

/// 后台按 32 MiB 分块并行扫描：CRLF 恰好跨在块边界上时只算一次。
#[test]
fn crlf_split_across_parallel_chunks_counts_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("boundary.log");
    let boundary = (SYNC_INDEX_BYTES + rove_buffer::INDEX_CHUNK_BYTES) as usize;
    let mut content = String::with_capacity(boundary + 64 * 1024 * 2);
    // 填到 CR 落在块的最后一个字节、LF 落在下一块的第一个字节。
    let line = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcd\r\n";
    while content.len() + line.len() < boundary - 1 {
        content.push_str(line);
    }
    while content.len() < boundary - 1 {
        content.push('x');
    }
    content.push_str("\r\n");
    while content.len() < boundary + (rove_buffer::INDEX_CHUNK_BYTES as usize) + 10 {
        content.push_str(line);
    }
    content.push('\r');
    assert_eq!(content.as_bytes()[boundary - 1], b'\r');
    assert_eq!(content.as_bytes()[boundary], b'\n');
    std::fs::write(&path, &content).unwrap();
    let text = open_huge(&path);
    let rope = RopeText::from_text(&content);
    assert_eq!(text.lines().known, rope.lines().known);
    let probe = rope.position(ByteOffset(boundary as u64 + 1)).unwrap().line;
    for index in [probe.0 - 2, probe.0 - 1, probe.0, probe.0 + 1] {
        assert_eq!(text.line_range(Line(index)), rope.line_range(Line(index)));
    }
    let last = rope.lines().known - 1;
    assert_eq!(text.line_range(Line(last)), rope.line_range(Line(last)));
    assert_eq!(
        text.line_range(Line(last - 1)),
        rope.line_range(Line(last - 1))
    );
}
