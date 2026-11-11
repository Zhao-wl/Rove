//! 编码识别、解码与换行符统计（#7）。
//!
//! 识别顺序：
//! 1. 明确的编码指定（“通过编码重新打开”，#27 会用到）；
//! 2. BOM（UTF-8 / UTF-16 LE / UTF-16 BE）——BOM 永远优先于猜测；
//! 3. `files.autoGuessEncoding` 开启时（见 [`guess_without_bom`]）：
//!    - 纯 ASCII 按 `files.encoding` 报告（内容不变）；含非 ASCII 的合法 UTF-8 按 UTF-8；
//!    - 否则 `chardetng` 有把握且结果（按编码族，见 [`same_family`]）在
//!      `files.candidateGuessEncodings` 里时，采用该候选；
//!    - 否则 `files.encoding` 能无错解码就用它；
//!    - 再否则按候选的列出顺序取第一个能无错解码的编码（VS Code 的“搜索空间”语义）；
//! 4. 以上都不成立时遵循 `files.encoding`（随后会被判成二进制/不支持的编码）。
//!
//! 无 BOM 的 UTF-16 不做自动推断（`chardetng` 不返回 UTF-16）：按 `files.encoding`
//! 处理，没有显式指定时会被 NUL/解码失败检测当成二进制。
//!
//! 解码后文本保留原始行终止符（混合换行不统一），BOM 单独记录、不进入文本。

use std::borrow::Cow;

use encoding_rs::{Encoding as RsEncoding, UTF_16BE, UTF_16LE, UTF_8};
use rove_base::{Error, Result};

use crate::contracts::{Encoding, Eol, LineEndings};
use crate::settings::{metadata_encoding, Preferences};

/// 短于此长度的输入不交给 `chardetng` 推断：ASCII 兼容的多编码在短文本上无法区分。
/// 短输入仍会按 `files.encoding` → 候选顺序尝试无错解码（issue #91）。
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
    let (text, had_errors) = match choice.decoded {
        Some(text) => (text, false),
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
    /// 选编码时已经无错解码过的文本（只在无 BOM 的自动识别路径出现），避免二次解码。
    decoded: Option<Cow<'a, str>>,
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
            decoded: None,
        });
    }
    if let Some((encoding, bom_len)) = sniff_bom(bytes) {
        return Ok(EncodingChoice {
            encoding,
            bom_len,
            guessed: false,
            decoded: None,
        });
    }
    if preferences.auto_guess {
        return Ok(guess_without_bom(bytes, preferences, detect));
    }
    Ok(EncodingChoice {
        encoding: preferences.fallback,
        bom_len: 0,
        guessed: false,
        decoded: None,
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
    match sniff_bom(bytes) {
        Some((bom_encoding, length)) if bom_encoding == encoding => length,
        _ => 0,
    }
}

/// 无 BOM、开启自动识别时的选编码逻辑。`detect` 给出 `chardetng` 有把握的推断结果
/// （生产代码传 [`detect`]），作为参数是为了让测试能注入“误判成同族编码”之类难以构造的结果。
/// 它是惰性的：只在字节不是合法 UTF-8 时才调用，UTF-8 / ASCII 文件不跑 chardetng。
///
/// 取舍（issue #91）：
/// - 推断结果不在候选里时**先试 `files.encoding`**，所以显式的 `windows1251` 不会被
///   候选 GBK 盖掉（PR #90 修掉的乱码问题）；
/// - `files.encoding` 也解不了时按候选顺序取第一个能无错解码的编码，GBK 文件即使被
///   推断成 Big5 / EUC-KR、过短或推断没把握，也不会被报成二进制；
/// - 返回值里 `guessed = true` 只表示编码来自候选列表，而不是 `files.encoding`。
fn guess_without_bom<'a>(
    bytes: &'a [u8],
    preferences: &Preferences,
    detect: impl FnOnce(&[u8]) -> Option<&'static RsEncoding>,
) -> EncodingChoice<'a> {
    let fallback = preferences.fallback;
    if let Ok(text) = std::str::from_utf8(bytes) {
        // 纯 ASCII 在任何 ASCII 兼容编码下内容都一样，按 files.encoding 报告，
        // 状态栏与保存（#27 / #48）才会和设置一致；VS Code 同样忽略 jschardet 的 ascii 结果。
        let encoding = if text.is_ascii() && fallback.is_ascii_compatible() {
            fallback
        } else {
            UTF_8
        };
        return EncodingChoice {
            encoding,
            bom_len: 0,
            guessed: false,
            decoded: Some(Cow::Borrowed(text)),
        };
    }
    let candidates = &preferences.candidates;
    if let Some(detected) = detect(bytes) {
        if let Some(candidate) = candidates
            .iter()
            .copied()
            .find(|candidate| same_family(candidate, detected))
        {
            return EncodingChoice {
                encoding: candidate,
                bom_len: 0,
                guessed: true,
                decoded: None,
            };
        }
    }
    if let Some(text) = decode_cleanly(fallback, bytes) {
        return EncodingChoice {
            encoding: fallback,
            bom_len: 0,
            guessed: false,
            decoded: Some(text),
        };
    }
    for &candidate in candidates {
        if candidate == fallback {
            continue;
        }
        if let Some(text) = decode_cleanly(candidate, bytes) {
            return EncodingChoice {
                encoding: candidate,
                bom_len: 0,
                guessed: true,
                decoded: Some(text),
            };
        }
    }
    EncodingChoice {
        encoding: fallback,
        bom_len: 0,
        guessed: false,
        decoded: None,
    }
}

/// `chardetng` 的推断；输入过短或推断没把握时返回 `None`。
///
/// 短输入上 ASCII 兼容的多字节编码无法区分，单个 `0xFF` 在部分单字节编码里也算
/// “解码成功”，所以短输入只走“按 files.encoding / 候选顺序无错解码”的路径。
fn detect(bytes: &[u8]) -> Option<&'static RsEncoding> {
    if bytes.len() < MIN_GUESS_BYTES {
        return None;
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    let (detected, confident) = detector.guess_assess(None, false);
    confident.then_some(detected)
}

/// 两个编码是否同族：同族时推断结果可以被候选列表里的另一个成员接受。
///
/// 目前只有 GBK / GB18030 一族：`encoding_rs` 的 GBK 解码器与 GB18030 解码器完全相同
/// （都接受四字节序列），按哪个解码结果都一样。命中时采用**候选列表里的那个**，
/// 让报告与保存（#48）使用的编码和用户的设置一致。
fn same_family(a: &'static RsEncoding, b: &'static RsEncoding) -> bool {
    let chinese = |encoding: &'static RsEncoding| {
        encoding == encoding_rs::GBK || encoding == encoding_rs::GB18030
    };
    a == b || (chinese(a) && chinese(b))
}

/// 无错解码时返回文本，遇到第一个错误即停止。
fn decode_cleanly<'a>(encoding: &'static RsEncoding, bytes: &'a [u8]) -> Option<Cow<'a, str>> {
    encoding.decode_without_bom_handling_and_without_replacement(bytes)
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
                // encoding_rs 会为数字字符引用预留 NCR_EXTRA(10) 字节，容量太小会误报 OutputFull。
                let mut out = Vec::with_capacity(text.len() * 2 + 32);
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
        // 用 GBK 重新打开一个**确实带 UTF-8 BOM** 的文件：BOM 字节属于内容，不会被剥掉。
        let bytes = [&[0xEF, 0xBB, 0xBF][..], b"abc"].concat();
        let decoded = decode(&bytes, &preferences(true, "utf8"), Some("gbk"), false).unwrap();
        assert_eq!(decoded.detection.encoding, Encoding::Gbk);
        assert!(!decoded.detection.bom);
        assert!(!decoded.detection.guessed);
        assert!(
            decoded.text.ends_with("bc"),
            "BOM 之后的内容仍然保留（BOM 的第三个字节会与 'a' 组成 GBK 双字节）"
        );
        assert_ne!(
            decoded.text, "abc",
            "显式 GBK 时 BOM 字节必须作为内容解码，而不是被剥离"
        );
        // BOM 字节作为内容重新编码后与原始字节完全一致，证明没有被丢弃。
        assert_eq!(encode_with(encoding_rs::GBK, &decoded.text), bytes);

        // 显式指定 UTF-8 时才能识别并剥离 BOM。
        let decoded = decode(&bytes, &preferences(true, "utf8"), Some("utf8"), false).unwrap();
        assert_eq!(decoded.text, "abc");
        assert!(decoded.detection.bom);
    }

    #[test]
    fn detected_encoding_outside_candidates_falls_back_to_files_encoding() {
        // windows-1251 的俄文：旧逻辑会退而求其次用候选 GBK“无错解码”，
        // 掩盖检测结果与 files.encoding，显示成乱码。现在必须回落到 files.encoding。
        let text = "Привет, мир! Это тестовый файл на русском языке.\r\n";
        let bytes = encode_with(encoding_rs::WINDOWS_1251, text);
        let decoded = decode(&bytes, &preferences(true, "windows1251"), None, true).unwrap();
        assert!(
            !decoded.detection.guessed,
            "检测结果不在候选列表里时不能标记为自动识别"
        );
        assert_eq!(
            decoded.detection.encoding,
            Encoding::Other(Arc::from("windows-1251"))
        );
        assert_eq!(decoded.text, text);
    }

    #[test]
    fn auto_guessed_encoding_must_be_a_registered_candidate() {
        // 同一份 windows-1251 字节，若用户把 windows1251 放进候选列表才允许猜。
        let text = "Привет, мир! Это тестовый файл на русском языке.\r\n";
        let bytes = encode_with(encoding_rs::WINDOWS_1251, text);
        let preferences = Preferences {
            auto_guess: true,
            candidates: vec![encoding_rs::WINDOWS_1251],
            fallback: UTF_8,
        };
        let decoded = decode(&bytes, &preferences, None, true).unwrap();
        assert!(decoded.detection.guessed);
        assert_eq!(decoded.text, text);
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
        // 单字节 0xFF 在 windows-1252 里是合法字符，但不在候选（GBK）里，且 GBK 解不了。
        assert_eq!(decode_default(&[0xFF]).err(), Some(Error::Unsupported));
    }

    #[test]
    fn short_gbk_file_opens_with_default_candidates() {
        // issue #91 第 3 条（手工构造）：6 字节的 `中文\r\n` 不经 chardetng，
        // files.encoding = utf8 解不了，按候选顺序用 GBK 打开，不报二进制。
        let text = "中文\r\n";
        let bytes = gbk(text);
        assert!(bytes.len() < MIN_GUESS_BYTES);
        let decoded = decode_default(&bytes).unwrap();
        assert_eq!(decoded.text, text);
        assert_eq!(decoded.detection.encoding, Encoding::Gbk);
        assert!(decoded.detection.guessed);
    }

    #[test]
    fn gbk_misdetected_as_sibling_cjk_encoding_still_opens_as_gbk() {
        // issue #91 第 1 条（手工构造，注入推断结果）：chardetng 有把握地给出 Big5 / EUC-KR
        // 时，结果不在候选里 → files.encoding(utf8) 解不了 → 按候选顺序用 GBK。
        let text = "// 这是一段 GBK 编码的中文注释，用来验证自动识别。\r\n";
        let bytes = gbk(text);
        for misdetected in [encoding_rs::BIG5, encoding_rs::EUC_KR] {
            let choice =
                guess_without_bom(&bytes, &preferences(true, "utf8"), |_| Some(misdetected));
            assert_eq!(choice.encoding, encoding_rs::GBK);
            assert!(choice.guessed);
            assert_eq!(choice.decoded.as_deref(), Some(text));
        }
    }

    #[test]
    fn utf8_fast_path_does_not_run_detection() {
        // 合法 UTF-8 / 纯 ASCII（长度超过 MIN_GUESS_BYTES）走快路径，不调用 chardetng；
        // 非 UTF-8 时才调用且只调用一次。
        let calls = std::cell::Cell::new(0);
        let counting = |bytes: &[u8]| {
            calls.set(calls.get() + 1);
            detect(bytes)
        };
        let ascii = b"public class Plain { public int Value = 1; }\r\n";
        let utf8 = "// 这是一段 UTF-8 编码的中文注释。\r\n".as_bytes();
        for bytes in [&ascii[..], utf8] {
            assert!(bytes.len() >= MIN_GUESS_BYTES);
            let choice = guess_without_bom(bytes, &preferences(true, "gbk"), counting);
            assert!(choice.decoded.is_some());
        }
        assert_eq!(calls.get(), 0, "UTF-8 快路径不应调用 detect");

        let gbk_bytes = gbk("// 这是一段 GBK 编码的中文注释，用来验证自动识别。\r\n");
        let choice = guess_without_bom(&gbk_bytes, &preferences(true, "utf8"), counting);
        assert_eq!(choice.encoding, encoding_rs::GBK);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn undecided_guess_tries_candidates_in_listed_order() {
        // VS Code 语义：候选是按列出顺序尝试的搜索空间。两个候选都能无错解码时取第一个。
        let bytes = gbk("中文注释：顺序测试");
        let ordered = |candidates| Preferences {
            auto_guess: true,
            candidates,
            fallback: UTF_8,
        };
        let first_gbk = guess_without_bom(
            &bytes,
            &ordered(vec![encoding_rs::GBK, encoding_rs::WINDOWS_1252]),
            |_| None,
        );
        assert_eq!(first_gbk.encoding, encoding_rs::GBK);
        let first_1252 = guess_without_bom(
            &bytes,
            &ordered(vec![encoding_rs::WINDOWS_1252, encoding_rs::GBK]),
            |_| None,
        );
        assert_eq!(first_1252.encoding, encoding_rs::WINDOWS_1252);
    }

    #[test]
    fn gb18030_and_gbk_are_one_family() {
        // issue #91 第 2 条：推断成 GB18030 时候选里的 gbk 也算命中，并报告候选本身；
        // 反过来候选写 gb18030、推断成 GBK 时报告 GB18030。
        let text = "// 中文注释\r\n";
        let bytes = gbk(text);
        let choice = guess_without_bom(&bytes, &preferences(true, "windows1251"), |_| {
            Some(encoding_rs::GB18030)
        });
        assert_eq!(choice.encoding, encoding_rs::GBK);
        assert!(choice.guessed);
        let gb18030 = Preferences {
            auto_guess: true,
            candidates: vec![encoding_rs::GB18030],
            fallback: UTF_8,
        };
        let choice = guess_without_bom(&bytes, &gb18030, |_| Some(encoding_rs::GBK));
        assert_eq!(choice.encoding, encoding_rs::GB18030);
        // 两者解码结果一致。
        assert_eq!(decode(&bytes, &gb18030, None, false).unwrap().text, text);
    }

    #[test]
    fn pure_ascii_reports_files_encoding() {
        // issue #91 第 4 条：files.encoding = gbk 时纯 ASCII 文件报告 GBK，内容不变、不算自动识别。
        let bytes = b"using UnityEngine;\r\n";
        let decoded = decode(bytes, &preferences(true, "gbk"), None, false).unwrap();
        assert_eq!(decoded.text, "using UnityEngine;\r\n");
        assert_eq!(decoded.detection.encoding, Encoding::Gbk);
        assert!(!decoded.detection.guessed);
        // 含非 ASCII 的合法 UTF-8 仍然按 UTF-8。
        let decoded = decode(
            "// 中文\n".as_bytes(),
            &preferences(true, "gbk"),
            None,
            false,
        )
        .unwrap();
        assert_eq!(decoded.detection.encoding, Encoding::Utf8);
        // files.encoding 不兼容 ASCII（UTF-16）时纯 ASCII 仍报 UTF-8，不把字节当 UTF-16 解。
        let decoded = decode(bytes, &preferences(true, "utf16le"), None, false).unwrap();
        assert_eq!(decoded.detection.encoding, Encoding::Utf8);
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
