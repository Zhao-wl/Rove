//! 编码识别、解码与换行符统计（#7）。
//!
//! 识别顺序：
//! 1. 明确的编码指定（“通过编码重新打开”，#27 会用到）；
//! 2. BOM（UTF-8 / UTF-16 LE / UTF-16 BE）——BOM 永远优先于猜测；
//! 3. `files.autoGuessEncoding` 开启时：合法 UTF-8 直接按 UTF-8，否则交给
//!    `chardetng` 推断，并只接受 `files.candidateGuessEncodings` 里的编码；
//! 4. 以上都不成立时遵循 `files.encoding`。
//!
//! 猜测一定会误判，遵循 issue #7 的取舍：**不确定就回落到 `files.encoding`**，
//! 不给出“看起来对”的编码。
//!
//! 解码后文本保留原始行终止符（混合换行不统一），BOM 单独记录、不进入文本。

use std::borrow::Cow;

use encoding_rs::{Encoding as RsEncoding, UTF_16BE, UTF_16LE, UTF_8};
use rove_base::{Error, Result};

use crate::contracts::{Encoding, Eol, LineEndings};
use crate::settings::{metadata_encoding, Preferences};

/// 短于此长度的输入不做自动识别：ASCII 兼容的多编码在短文本上无法区分，
/// 单个 `0xFF` 之类的字节在部分单字节编码里也算“解码成功”。
const MIN_GUESS_BYTES: usize = 8;

/// 解码失败的比例超过这个值就当作二进制 / 不支持的编码。
const MAX_DECODE_ERROR_RATIO: f64 = 0.01;

/// 一次打开得到的编码结论。`guessed` 表示编码来自自动识别而不是 BOM 或显式设置，
/// 状态栏悬停提示“自动识别”时使用（界面在 #27）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    pub encoding: Encoding,
    pub bom: bool,
    pub guessed: bool,
}

/// 解码结果；`text` 是解码后的 UTF-8（BOM 已剥离，行终止符原样保留）。
/// 合法 UTF-8 直接借用输入字节，只有真正需要转码时才分配。
pub(crate) struct DecodedText<'a> {
    pub text: Cow<'a, str>,
    pub detection: Detection,
    pub line_endings: LineEndings,
}

/// 把磁盘字节解码成文本快照的内容与元信息。
///
/// `explicit` 是用户明确指定的编码 id（优先于 BOM）；`force_open` 对应
/// “仍然打开”：跳过二进制/解码失败判定，按 lossy 显示。
pub(crate) fn decode<'a>(
    bytes: &'a [u8],
    preferences: &Preferences,
    explicit: Option<&str>,
    force_open: bool,
) -> Result<DecodedText<'a>> {
    let choice = choose_encoding(bytes, preferences, explicit)?;
    let (text, had_errors) = match choice.utf8 {
        Some(text) => (Cow::Borrowed(text), false),
        None => decode_encoding(choice.encoding, &bytes[choice.bom_len..]),
    };
    let scan = scan_text(&text, had_errors);
    if !force_open {
        if scan.nul {
            // NUL 几乎只出现在二进制文件里；UTF-16 的 NUL 字节在解码后已经变成字符。
            return Err(Error::Unsupported);
        }
        if scan.replacements > 0
            && scan.replacements as f64 > scan.chars as f64 * MAX_DECODE_ERROR_RATIO
        {
            return Err(Error::Unsupported);
        }
    }
    Ok(DecodedText {
        line_endings: detect_line_endings(&text),
        detection: Detection {
            encoding: metadata_encoding(choice.encoding),
            bom: choice.bom_len > 0,
            guessed: choice.guessed,
        },
        text,
    })
}

struct EncodingChoice<'a> {
    encoding: &'static RsEncoding,
    bom_len: usize,
    guessed: bool,
    /// 已确认合法、无需二次校验的 UTF-8（只在无 BOM 的自动识别路径出现）。
    utf8: Option<&'a str>,
}

fn choose_encoding<'a>(
    bytes: &'a [u8],
    preferences: &Preferences,
    explicit: Option<&str>,
) -> Result<EncodingChoice<'a>> {
    if let Some(id) = explicit {
        let encoding = crate::settings::encoding_from_id(id).ok_or(Error::Unsupported)?;
        return Ok(EncodingChoice {
            encoding,
            bom_len: matching_bom_len(bytes, encoding),
            guessed: false,
            utf8: None,
        });
    }
    if let Some((encoding, bom_len)) = sniff_bom(bytes) {
        return Ok(EncodingChoice {
            encoding,
            bom_len,
            guessed: false,
            utf8: None,
        });
    }
    if preferences.auto_guess {
        if let Ok(text) = std::str::from_utf8(bytes) {
            return Ok(EncodingChoice {
                encoding: UTF_8,
                bom_len: 0,
                guessed: false,
                utf8: Some(text),
            });
        }
        if let Some(encoding) = guess(bytes, preferences) {
            return Ok(EncodingChoice {
                encoding,
                bom_len: 0,
                guessed: true,
                utf8: None,
            });
        }
    }
    Ok(EncodingChoice {
        encoding: preferences.fallback,
        bom_len: 0,
        guessed: false,
        utf8: None,
    })
}

fn sniff_bom(bytes: &[u8]) -> Option<(&'static RsEncoding, usize)> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some((UTF_8, 3))
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        // UTF-32 LE 的 BOM 也以 FF FE 开头；剩下的 NUL 会让二进制检测拒绝它。
        Some((UTF_16LE, 2))
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some((UTF_16BE, 2))
    } else {
        None
    }
}

fn matching_bom_len(bytes: &[u8], encoding: &'static RsEncoding) -> usize {
    if encoding.name() == UTF_8.name() && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else if (encoding.name() == UTF_16LE.name() && bytes.starts_with(&[0xFF, 0xFE]))
        || (encoding.name() == UTF_16BE.name() && bytes.starts_with(&[0xFE, 0xFF]))
    {
        2
    } else {
        0
    }
}

/// 用 `chardetng` 推断，并按 `files.candidateGuessEncodings` 的语义筛选。
fn guess(bytes: &[u8], preferences: &Preferences) -> Option<&'static RsEncoding> {
    if bytes.len() < MIN_GUESS_BYTES {
        return None;
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let (detected, confident) = detector.guess_assess(None, false);
    if !confident {
        // 推断本身不可信时宁可回落到 files.encoding（issue #7 的风险提示）。
        return None;
    }
    // 无 BOM 的 UTF-16 在候选列表之外也要接受：它由 NUL 的奇偶分布识别，
    // 真正的二进制文件会随后被 NUL / 解码失败判定拦下。
    if is_utf16(detected) && !decode_has_nul(bytes, detected) {
        return Some(detected);
    }
    if preferences
        .candidates
        .iter()
        .any(|candidate| candidate.name() == detected.name())
    {
        return Some(detected);
    }
    preferences
        .candidates
        .iter()
        .copied()
        .find(|candidate| decodes_cleanly(bytes, candidate))
}

fn is_utf16(encoding: &'static RsEncoding) -> bool {
    encoding.name() == UTF_16LE.name() || encoding.name() == UTF_16BE.name()
}

fn decodes_cleanly(bytes: &[u8], encoding: &'static RsEncoding) -> bool {
    !encoding.decode_without_bom_handling(bytes).1
}

fn decode_has_nul(bytes: &[u8], encoding: &'static RsEncoding) -> bool {
    encoding.decode_without_bom_handling(bytes).0.contains('\0')
}

fn decode_encoding<'a>(encoding: &'static RsEncoding, bytes: &'a [u8]) -> (Cow<'a, str>, bool) {
    encoding.decode_without_bom_handling(bytes)
}

struct TextScan {
    chars: usize,
    replacements: usize,
    nul: bool,
}

fn scan_text(text: &str, had_errors: bool) -> TextScan {
    // NUL 用 SIMD 扫描；字符数只在需要算失败比例时才统计，正常文件全部跳过。
    let nul = memchr::memchr(0, text.as_bytes()).is_some();
    let mut scan = TextScan {
        chars: 0,
        replacements: 0,
        nul,
    };
    if had_errors {
        for ch in text.chars() {
            scan.chars += 1;
            if ch == '\u{FFFD}' {
                scan.replacements += 1;
            }
        }
    }
    scan
}

/// 统计 LF / CRLF / CR。`\r\n` 只算一次；三者都有时 `Mixed`，主换行符取多数，
/// 票数相同时按 CRLF → LF → CR 的顺序取（Windows 优先）。
pub(crate) fn detect_line_endings(text: &str) -> LineEndings {
    let bytes = text.as_bytes();
    // memchr/memmem 是 SIMD 扫描：10 MB 文件上也只要几毫秒。
    let crlf = memchr::memmem::find_iter(bytes, b"\r\n").count();
    let lf = memchr::memchr_iter(b'\n', bytes)
        .count()
        .saturating_sub(crlf);
    let cr = memchr::memchr_iter(b'\r', bytes)
        .count()
        .saturating_sub(crlf);
    let kinds = usize::from(crlf > 0) + usize::from(lf > 0) + usize::from(cr > 0);
    if kinds == 0 {
        // 没有行终止符（空文件或单行文件）：取平台中立的 LF，不凭空制造 CRLF。
        return LineEndings::Uniform(Eol::Lf);
    }
    let preferred = if crlf >= lf && crlf >= cr {
        Eol::CrLf
    } else if lf >= cr {
        Eol::Lf
    } else {
        Eol::Cr
    };
    if kinds > 1 {
        LineEndings::Mixed { preferred }
    } else {
        LineEndings::Uniform(preferred)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::TextSource;
    use crate::settings::{self, ROVE_DEFAULT_AUTO_GUESS};
    use rove_base::{ByteOffset, Span, Value};
    use rove_config::{DefaultSettings, Scope, SettingRegistry, Settings};
    use std::sync::Arc;

    fn preferences(auto_guess: bool, fallback: &str) -> Preferences {
        Preferences {
            auto_guess,
            candidates: vec![encoding_rs::GBK],
            fallback: settings::encoding_from_id(fallback).unwrap(),
        }
    }

    /// 用 `encoding_rs` 的编码器把文本编码成目标编码。注意 `encoding_rs` 按 Web 规范
    /// 把 UTF-16 的输出固定成 UTF-8，所以 UTF-16 由测试自己按码元写回。
    fn encode_with(encoding: &'static RsEncoding, text: &str) -> Vec<u8> {
        match encoding.name() {
            "UTF-16LE" => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            "UTF-16BE" => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            _ => {
                let mut encoder = encoding.new_encoder();
                let mut out = Vec::with_capacity(text.len() * 2);
                let (result, read, had_errors) =
                    encoder.encode_from_utf8_to_vec(text, &mut out, true);
                assert_eq!(result, encoding_rs::CoderResult::InputEmpty);
                assert_eq!(read, text.len());
                assert!(!had_errors);
                out
            }
        }
    }

    fn gbk(text: &str) -> Vec<u8> {
        encode_with(encoding_rs::GBK, text)
    }

    fn decode_default(bytes: &[u8]) -> Result<DecodedText<'_>> {
        decode(
            bytes,
            &preferences(ROVE_DEFAULT_AUTO_GUESS, "utf8"),
            None,
            false,
        )
    }

    #[test]
    fn recognises_utf8_with_and_without_bom() {
        for (bytes, bom) in [
            (b"// hello\r\n".to_vec(), false),
            ([&[0xEF, 0xBB, 0xBF][..], b"// hello\r\n"].concat(), true),
        ] {
            let decoded = decode_default(&bytes).unwrap();
            assert_eq!(decoded.text, "// hello\r\n");
            assert_eq!(decoded.detection.encoding, Encoding::Utf8);
            assert_eq!(decoded.detection.bom, bom);
            assert!(!decoded.detection.guessed);
        }
    }

    #[test]
    fn recognises_utf16_with_bom_in_both_byte_orders() {
        let text = "// 中文😀\r\n";
        for (encoding, bom) in [
            (encoding_rs::UTF_16LE, [0xFF, 0xFE]),
            (encoding_rs::UTF_16BE, [0xFE, 0xFF]),
        ] {
            let body = encode_with(encoding, text);
            let bytes = [&bom[..], &body].concat();
            let decoded = decode_default(&bytes).unwrap();
            assert_eq!(decoded.text, text);
            assert!(decoded.detection.bom);
            assert_eq!(
                decoded.detection.encoding,
                if encoding.name() == UTF_16LE.name() {
                    Encoding::Utf16Le
                } else {
                    Encoding::Utf16Be
                }
            );
            assert!(!decoded.detection.guessed);
        }
    }

    #[test]
    fn guesses_gbk_when_auto_guess_is_enabled() {
        let text = "// 这是一段 GBK 编码的中文注释，用来验证自动识别。\r\n";
        let bytes = gbk(text);
        assert!(std::str::from_utf8(&bytes).is_err());
        let decoded = decode_default(&bytes).unwrap();
        assert_eq!(decoded.text, text);
        assert_eq!(decoded.detection.encoding, Encoding::Gbk);
        assert!(decoded.detection.guessed);
    }

    #[test]
    fn explicit_auto_guess_off_falls_back_to_files_encoding() {
        // files.encoding = gbk 时不猜，按 gbk 解码，且不标记为“自动识别”。
        let text = "// 中文注释\r\n";
        let bytes = gbk(text);
        let decoded = decode(&bytes, &preferences(false, "gbk"), None, false).unwrap();
        assert_eq!(decoded.text, text);
        assert_eq!(decoded.detection.encoding, Encoding::Gbk);
        assert!(!decoded.detection.guessed);
        // files.encoding = utf8 时同样的字节无法解码成文本，明确拒绝。
        assert_eq!(
            decode(&bytes, &preferences(false, "utf8"), None, false).err(),
            Some(Error::Unsupported)
        );
    }

    #[test]
    fn explicit_encoding_wins_over_bom() {
        // 用 GBK 重新打开一个带 UTF-8 BOM 的文件：BOM 字节属于内容。
        let decoded = decode(
            b"// hello\r\n",
            &preferences(true, "utf8"),
            Some("gbk"),
            false,
        )
        .unwrap();
        assert_eq!(decoded.detection.encoding, Encoding::Gbk);
        assert!(!decoded.detection.bom);
        assert!(!decoded.detection.guessed);
    }

    #[test]
    fn binary_is_rejected_unless_forced_open() {
        for bytes in [
            b"abc\0def".to_vec(),
            vec![0xFF],
            // 无效 UTF-8 且无法用 GBK 干净解码的字节序列。
            vec![0x81, 0x20, 0x81, 0x20, 0xFF, 0xFE, 0x80, 0x81],
        ] {
            assert_eq!(decode_default(&bytes).err(), Some(Error::Unsupported));
            let forced = decode(
                &bytes,
                &preferences(ROVE_DEFAULT_AUTO_GUESS, "utf8"),
                None,
                true,
            )
            .unwrap();
            assert!(!forced.text.is_empty());
        }
    }

    fn assert_uniform(endings: LineEndings, expected: Eol) {
        match endings {
            LineEndings::Uniform(actual) => assert_eq!(actual, expected),
            other => panic!("expected a uniform line ending, got {other:?}"),
        }
    }

    #[test]
    fn empty_file_is_one_empty_utf8_line() {
        let decoded = decode_default(b"").unwrap();
        assert_eq!(decoded.text, "");
        assert_eq!(decoded.detection.encoding, Encoding::Utf8);
        assert!(!decoded.detection.bom);
        assert_uniform(decoded.line_endings, Eol::Lf);
    }

    #[test]
    fn detects_uniform_and_mixed_line_endings_without_normalizing() {
        assert_uniform(detect_line_endings("a\nb\n"), Eol::Lf);
        assert_uniform(detect_line_endings("a\r\nb\r\n"), Eol::CrLf);
        assert_uniform(detect_line_endings("a\rb\r"), Eol::Cr);
        assert_uniform(detect_line_endings("a\nb\nc"), Eol::Lf);
        // CRLF 2 票、LF 1 票、CR 1 票 → 混合，主换行符 CRLF。
        let mixed = detect_line_endings("a\r\nb\nc\rd\r\n");
        assert!(matches!(
            mixed,
            LineEndings::Mixed {
                preferred: Eol::CrLf
            }
        ));
        // 内容逐字节未被改写。
        let decoded = decode_default("a\r\nb\nc\rd\r\n".as_bytes()).unwrap();
        assert_eq!(decoded.text, "a\r\nb\nc\rd\r\n");
    }

    #[test]
    fn short_inputs_are_not_guessed() {
        // 单字节 0xFF 在 windows-1252 里是合法字符，但短输入不该猜。
        assert_eq!(decode_default(&[0xFF]).err(), Some(Error::Unsupported));
    }

    #[test]
    fn registered_defaults_are_read_from_the_contract() {
        let mut registry = SettingRegistry::default();
        settings::register(&mut registry).unwrap();
        let settings = DefaultSettings(registry);
        let preferences = Preferences::read(&settings, "csharp");
        assert!(preferences.auto_guess);
        assert_eq!(preferences.candidates, vec![encoding_rs::GBK]);
    }

    #[test]
    fn language_scope_is_passed_through_to_settings() {
        struct LanguageScoped;
        impl Settings for LanguageScoped {
            fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
                if key == settings::FILES_ENCODING && scope.language.as_deref() == Some("csharp") {
                    return Ok(Value::String("gbk".into()));
                }
                Err(Error::NotFound)
            }
            fn write(
                &self,
                _: Arc<str>,
                _: Scope,
                _: Value,
                _: rove_base::Cancellation,
            ) -> rove_base::Task<()> {
                Box::pin(async { Err(Error::Unsupported) })
            }
            fn changes(&self) -> &dyn rove_base::Events<rove_config::SettingsChange> {
                &rove_base::NoEvents
            }
        }
        let scoped = Preferences::read(&LanguageScoped, "csharp");
        assert_eq!(scoped.fallback, encoding_rs::GBK);
        let unscoped = Preferences::read(&LanguageScoped, "plaintext");
        assert_eq!(unscoped.fallback, UTF_8);
    }

    #[test]
    fn full_range_visit_reproduces_decoded_text() {
        let text = "第一行\r\nsecond\n第三行\r";
        let bytes = gbk(text);
        let decoded = decode_default(&bytes).unwrap();
        assert_eq!(decoded.text, text);
        let range = Span::new(ByteOffset(0), ByteOffset(decoded.text.len() as u64)).unwrap();
        let mut rebuilt = String::new();
        // 通过 TextSource 读取，而不是直接用 decoded.text。
        let source = crate::RopeText::from_text(&decoded.text);
        source
            .visit_chunks(range, &mut |chunk| rebuilt.push_str(chunk))
            .unwrap();
        assert_eq!(rebuilt, text);
    }
}
