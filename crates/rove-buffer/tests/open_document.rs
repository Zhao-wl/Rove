//! 打开文件的端到端测试（#7）：走公开的 `open_path` / `open_bytes` → `TextSource` →
//! 元信息链路，覆盖编码、BOM、换行符与二进制检测。

use std::collections::BTreeMap;
use std::sync::Arc;

use encoding_rs::Encoding as RsEncoding;
use rove_base::{ByteOffset, Cancellation, Error, Events, Line, Span, Task, Value};
use rove_buffer::settings;
use rove_buffer::{open_bytes, open_path, Encoding, Eol, LineEndings, LoadOptions, TextSource};
use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings};

fn default_settings() -> DefaultSettings {
    let mut registry = SettingRegistry::default();
    settings::register(&mut registry).unwrap();
    DefaultSettings(registry)
}

/// 显式用户设置的最小替身：#13 的设置层合并前，用它验证“遵从用户设置”的分支。
struct UserSettings(BTreeMap<&'static str, Value>);

impl UserSettings {
    fn new(values: &[(&'static str, Value)]) -> Self {
        Self(values.iter().cloned().collect())
    }
}

impl Settings for UserSettings {
    fn read(&self, key: &str, _: &Scope) -> rove_base::Result<Value> {
        self.0.get(key).cloned().ok_or(rove_base::Error::NotFound)
    }

    fn write(&self, _: Arc<str>, _: Scope, _: Value, _: Cancellation) -> Task<()> {
        Box::pin(async { Err(rove_base::Error::Unsupported) })
    }

    fn changes(&self) -> &dyn Events<rove_config::SettingsChange> {
        &rove_base::NoEvents
    }
}

fn full_text(text: &dyn TextSource) -> String {
    let mut out = String::new();
    let range = Span::new(ByteOffset(0), ByteOffset(text.byte_len())).unwrap();
    text.visit_chunks(range, &mut |chunk| out.push_str(chunk))
        .unwrap();
    out
}

fn encode(encoding: &Encoding, text: &str) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf16Le => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        Encoding::Utf16Be => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        Encoding::Gbk => {
            let mut encoder = encoding_rs::GBK.new_encoder();
            let mut out = Vec::with_capacity(text.len() * 2);
            let (result, read, had_errors) = encoder.encode_from_utf8_to_vec(text, &mut out, true);
            assert_eq!(result, encoding_rs::CoderResult::InputEmpty);
            assert_eq!(read, text.len());
            assert!(!had_errors);
            out
        }
        Encoding::Other(name) => {
            let encoding = RsEncoding::for_label(name.as_bytes()).unwrap();
            let mut encoder = encoding.new_encoder();
            let mut out = Vec::with_capacity(text.len() * 2);
            let (result, _, had_errors) = encoder.encode_from_utf8_to_vec(text, &mut out, true);
            assert_eq!(result, encoding_rs::CoderResult::InputEmpty);
            assert!(!had_errors);
            assert_eq!(out.len() % 2, 0);
            out
        }
    }
}

fn bom(encoding: &Encoding) -> &'static [u8] {
    match encoding {
        Encoding::Utf8 => &[0xEF, 0xBB, 0xBF],
        Encoding::Utf16Le => &[0xFF, 0xFE],
        Encoding::Utf16Be => &[0xFE, 0xFF],
        _ => &[],
    }
}

fn with_bom(encoding: &Encoding, text: &str) -> Vec<u8> {
    [bom(encoding), &encode(encoding, text)].concat()
}

/// “原样写回”：按元信息里的编码与 BOM 标记重新编码，用于往返比较。
fn round_trip(encoding: &Encoding, bom_present: bool, text: &str) -> Vec<u8> {
    let mut out = if bom_present {
        bom(encoding).to_vec()
    } else {
        Vec::new()
    };
    out.extend(encode(encoding, text));
    out
}

#[test]
fn opens_a_gbk_file_through_the_public_loader_and_reads_lines() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("Player.cs");
    let text = "// 中文注释：第一行\r\n// 第二行\n// 第三行\r";
    std::fs::write(&path, encode(&Encoding::Gbk, text)).unwrap();

    let settings = default_settings();
    let loaded = open_path(&path, &LoadOptions::new("csharp", &settings)).unwrap();

    assert_eq!(loaded.metadata.language.as_ref(), "csharp");
    assert_eq!(loaded.metadata.encoding, Encoding::Gbk);
    assert!(!loaded.metadata.bom);
    assert!(loaded.detection.guessed, "编码来自自动识别");
    assert!(
        matches!(
            loaded.metadata.line_endings,
            LineEndings::Mixed {
                preferred: Eol::CrLf
            }
        ),
        "混合换行符必须保持混合，主换行符是 CRLF"
    );
    assert_eq!(loaded.text.lines().known, 4);
    assert_eq!(
        full_text(loaded.text.as_ref()),
        text,
        "读出的文本逐字节保留原始换行符"
    );
    let line_zero = loaded.text.line_range(Line(0)).unwrap();
    let mut first = String::new();
    loaded
        .text
        .visit_chunks(line_zero, &mut |chunk| first.push_str(chunk))
        .unwrap();
    assert_eq!(first, "// 中文注释：第一行");
}

#[test]
fn every_encoding_round_trips_byte_for_byte() {
    let settings = default_settings();
    let options = LoadOptions::new("plaintext", &settings);

    let fixtures: Vec<(&str, Vec<u8>, Encoding, bool)> = vec![
        (
            "utf8",
            "public class A {}\r\n// 中文😀\n".as_bytes().to_vec(),
            Encoding::Utf8,
            false,
        ),
        (
            "utf8-bom",
            with_bom(&Encoding::Utf8, "// 带 BOM\r\n"),
            Encoding::Utf8,
            true,
        ),
        (
            "utf16le-bom",
            with_bom(&Encoding::Utf16Le, "// 中文😀\r\n第二行\n"),
            Encoding::Utf16Le,
            true,
        ),
        (
            "utf16be-bom",
            with_bom(&Encoding::Utf16Be, "// 中文😀\r\n第二行\n"),
            Encoding::Utf16Be,
            true,
        ),
        (
            "gbk",
            encode(
                &Encoding::Gbk,
                "// 这是一段 GBK 编码的中文注释，用来验证往返。\r\n",
            ),
            Encoding::Gbk,
            false,
        ),
        (
            "ascii",
            b"public class Plain {}\n".to_vec(),
            Encoding::Utf8,
            false,
        ),
        ("empty", Vec::new(), Encoding::Utf8, false),
        (
            "no-trailing-newline",
            b"first line\nsecond line".to_vec(),
            Encoding::Utf8,
            false,
        ),
        ("cr-only", b"a\rb\rc".to_vec(), Encoding::Utf8, false),
        ("mixed", b"a\r\nb\nc\rd\r\n".to_vec(), Encoding::Utf8, false),
        (
            "surrogate-pairs-and-combining",
            "😀🎉 e\u{301} 中文\r\n".as_bytes().to_vec(),
            Encoding::Utf8,
            false,
        ),
    ];

    for (name, bytes, expected_encoding, expected_bom) in fixtures {
        let loaded = open_bytes(&bytes, &options)
            .unwrap_or_else(|error| panic!("fixture {name} failed to open: {error:?}"));
        assert_eq!(
            loaded.metadata.encoding, expected_encoding,
            "fixture {name}"
        );
        assert_eq!(loaded.metadata.bom, expected_bom, "fixture {name}");
        let rebuilt = round_trip(
            &loaded.metadata.encoding,
            loaded.metadata.bom,
            &full_text(loaded.text.as_ref()),
        );
        assert_eq!(
            rebuilt, bytes,
            "fixture {name} must round-trip byte for byte"
        );
    }
}

#[test]
fn mixed_line_endings_are_not_normalized() {
    let settings = default_settings();
    let options = LoadOptions::new("plaintext", &settings);
    let bytes = b"a\r\nb\nc\rd\r\n";
    let loaded = open_bytes(bytes, &options).unwrap();
    assert!(matches!(
        loaded.metadata.line_endings,
        LineEndings::Mixed {
            preferred: Eol::CrLf
        }
    ));
    assert_eq!(full_text(loaded.text.as_ref()).as_bytes(), bytes.as_slice());
    assert_eq!(loaded.text.lines().known, 5);
}

#[test]
fn auto_guess_off_honours_files_encoding() {
    let bytes = encode(&Encoding::Gbk, "// 中文注释\r\n");

    // 用户显式关闭自动识别并指定 gbk：按 gbk 解码，且不标记为自动识别。
    let explicit = UserSettings::new(&[
        (settings::FILES_AUTO_GUESS_ENCODING, Value::Bool(false)),
        (settings::FILES_ENCODING, Value::String("gbk".into())),
    ]);
    let loaded = open_bytes(&bytes, &LoadOptions::new("csharp", &explicit)).unwrap();
    assert_eq!(loaded.metadata.encoding, Encoding::Gbk);
    assert!(!loaded.detection.guessed);

    // 关闭自动识别且 files.encoding 是 utf8：同一份字节不能被当作文本。
    let utf8_only = UserSettings::new(&[(settings::FILES_AUTO_GUESS_ENCODING, Value::Bool(false))]);
    assert_eq!(
        open_bytes(&bytes, &LoadOptions::new("csharp", &utf8_only)).err(),
        Some(Error::Unsupported)
    );
}

#[test]
fn explicitly_set_auto_guess_value_wins_over_rove_default() {
    // 用户显式设成 false 时，Rove 的“默认开启”不再生效。
    let settings = default_settings();
    let default_loaded = open_bytes(
        encode(&Encoding::Gbk, "// 这是默认设置下的 GBK 中文注释。\r\n").as_slice(),
        &LoadOptions::new("csharp", &settings),
    )
    .unwrap();
    assert_eq!(default_loaded.metadata.encoding, Encoding::Gbk);
    assert!(default_loaded.detection.guessed);

    let disabled = UserSettings::new(&[(settings::FILES_AUTO_GUESS_ENCODING, Value::Bool(false))]);
    assert_eq!(
        open_bytes(
            encode(&Encoding::Gbk, "// 这是默认设置下的 GBK 中文注释。\r\n").as_slice(),
            &LoadOptions::new("csharp", &disabled)
        )
        .err(),
        Some(Error::Unsupported)
    );
}

#[test]
fn binary_files_are_rejected_with_an_explicit_error_and_can_be_forced_open() {
    let settings = default_settings();
    let bytes = b"class A {}\0\0\0binary".to_vec();
    let mut options = LoadOptions::new("csharp", &settings);
    assert_eq!(open_bytes(&bytes, &options).err(), Some(Error::Unsupported));

    options.force_open = true;
    let forced = open_bytes(&bytes, &options).unwrap();
    assert_eq!(full_text(forced.text.as_ref()).as_bytes(), bytes.as_slice());
}

#[test]
fn large_single_line_file_has_one_line() {
    let settings = default_settings();
    let options = LoadOptions::new("plaintext", &settings);
    let text = "x".repeat(1024 * 1024);
    let loaded = open_bytes(text.as_bytes(), &options).unwrap();
    assert_eq!(loaded.text.lines().known, 1);
    assert_eq!(
        loaded.text.line_range(Line(0)).unwrap(),
        Span::new(ByteOffset(0), ByteOffset(text.len() as u64)).unwrap()
    );
    assert_eq!(
        loaded
            .text
            .position(ByteOffset(text.len() as u64))
            .unwrap()
            .line,
        Line(0)
    );
}
