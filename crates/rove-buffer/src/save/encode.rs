//! 文本 → 磁盘字节（#49）：按打开时识别出的编码与 BOM 原样写回。
//!
//! 文本快照里保存的是解码后的 UTF-8，行终止符按原样保留（混合换行不统一），BOM 单独记录。
//! 所以“原样写回”只需要：BOM（如果打开时有）+ 按原编码编码全文。合法输入解码再编码是
//! 逐字节可逆的（UTF-8 / UTF-16 / GBK 均如此），未改动的部分因此逐字节不变。
//!
//! - UTF-8 / UTF-16 LE / UTF-16 BE 能表示任何字符，不会出现无法编码的字符。`encoding_rs` 不提供
//!   UTF-16 编码器（它的 `output_encoding` 是 UTF-8），UTF-16 在这里自己编码。
//! - 其余编码（GBK 等）可能有无法编码的字符（例如 GBK 文件里的 emoji）：[`count_unencodable`]
//!   统计个数供保存前的模态框使用，[`Unencodable::Replace`] 把每个这样的字符替换为 `?`。

use encoding_rs::{EncoderResult, UTF_16BE, UTF_16LE, UTF_8};
use rove_base::{ByteOffset, Error, Result, Span};

use crate::contracts::{Encoding, TextSource};
use crate::settings::rs_encoding;

/// 遇到无法编码的字符时的处理。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unencodable {
    /// 拒绝写入：返回 [`EncodeError::Unencodable`]。
    Reject,
    /// 每个无法编码的字符替换为一个 `?`（“仍然保存（替换为 ?）”）。
    Replace,
}

/// 编码失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// 有 `count` 个字符无法用该编码表示；`first` 是第一个的位置（解码后文本的字节偏移）。
    Unencodable { count: usize, first: ByteOffset },
    /// 编码本身不受支持（元信息里的名字 `encoding_rs` 不认识），或读取文本失败。
    Other(Error),
}

impl From<Error> for EncodeError {
    fn from(error: Error) -> Self {
        EncodeError::Other(error)
    }
}

/// 该编码是否能表示所有 Unicode 字符（UTF-8 / UTF-16 / GB18030）。
pub fn is_unicode(encoding: &Encoding) -> bool {
    match encoding {
        Encoding::Utf8 | Encoding::Utf16Le | Encoding::Utf16Be => true,
        Encoding::Gbk => false,
        Encoding::Other(_) => {
            rs_encoding(encoding).is_some_and(|rs| rs == encoding_rs::GB18030 || rs == UTF_8)
        }
    }
}

/// 该编码的 BOM（只有 UTF-8 / UTF-16 有）。
pub fn bom_bytes(encoding: &Encoding) -> &'static [u8] {
    match encoding {
        Encoding::Utf8 => &[0xEF, 0xBB, 0xBF],
        Encoding::Utf16Le => &[0xFF, 0xFE],
        Encoding::Utf16Be => &[0xFE, 0xFF],
        _ => &[],
    }
}

/// 无法用 `encoding` 表示的字符个数（Unicode 编码恒为 0）。O(n)，在后台线程调用。
pub fn count_unencodable(text: &dyn TextSource, encoding: &Encoding) -> Result<usize> {
    if is_unicode(encoding) {
        return Ok(0);
    }
    let mut count = 0usize;
    encode_legacy(text, encoding, &mut |event| {
        if let Event::Unmappable(_) = event {
            count += 1;
        }
    })?;
    Ok(count)
}

/// 把整个文本编码成磁盘字节：`bom` 为真且编码有 BOM 时在开头写 BOM。
pub fn encode(
    text: &dyn TextSource,
    encoding: &Encoding,
    bom: bool,
    policy: Unencodable,
) -> std::result::Result<Vec<u8>, EncodeError> {
    let mut out = Vec::with_capacity(text.byte_len() as usize + 4);
    if bom {
        out.extend_from_slice(bom_bytes(encoding));
    }
    let whole = Span::new(ByteOffset(0), ByteOffset(text.byte_len()))?;
    match encoding {
        Encoding::Utf8 => {
            text.visit_chunks(whole, &mut |chunk| out.extend_from_slice(chunk.as_bytes()))?;
            return Ok(out);
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let little = *encoding == Encoding::Utf16Le;
            text.visit_chunks(whole, &mut |chunk| {
                for unit in chunk.encode_utf16() {
                    out.extend_from_slice(&if little {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    });
                }
            })?;
            return Ok(out);
        }
        _ => {}
    }
    let mut unmappable = 0usize;
    let mut first = None;
    encode_legacy(text, encoding, &mut |event| match event {
        Event::Bytes(bytes) => out.extend_from_slice(bytes),
        Event::Unmappable(at) => {
            unmappable += 1;
            first.get_or_insert(at);
            out.push(b'?');
        }
    })?;
    match (policy, first) {
        (Unencodable::Reject, Some(first)) => Err(EncodeError::Unencodable {
            count: unmappable,
            first,
        }),
        _ => Ok(out),
    }
}

enum Event<'a> {
    Bytes(&'a [u8]),
    /// 无法编码的字符在文本中的字节偏移。
    Unmappable(ByteOffset),
}

/// 非 Unicode 编码的流式编码：块与块之间由编码器保存状态（ISO-2022-JP 之类有状态编码也正确）。
fn encode_legacy(
    text: &dyn TextSource,
    encoding: &Encoding,
    sink: &mut dyn FnMut(Event<'_>),
) -> Result<()> {
    let rs = rs_encoding(encoding).ok_or(Error::Unsupported)?;
    let output = rs.output_encoding();
    if output == UTF_8 || output == UTF_16LE || output == UTF_16BE {
        // `replacement` / UTF-16 之类不能作为输出编码的标签：不假装写入成功。
        if rs != UTF_8 {
            return Err(Error::Unsupported);
        }
    }
    let mut encoder = output.new_encoder();
    let mut buffer = [0u8; 8192];
    let mut offset = 0u64;
    let whole = Span::new(ByteOffset(0), ByteOffset(text.byte_len()))?;
    let mut visit = |chunk: &str, last: bool| {
        let mut input = chunk;
        loop {
            let (result, read, written) =
                encoder.encode_from_utf8_without_replacement(input, &mut buffer, last);
            if written > 0 {
                sink(Event::Bytes(&buffer[..written]));
            }
            input = &input[read..];
            match result {
                EncoderResult::InputEmpty => {
                    offset += chunk.len() as u64;
                    break;
                }
                EncoderResult::OutputFull => {}
                EncoderResult::Unmappable(ch) => {
                    let consumed = chunk.len() - input.len();
                    let at = offset + consumed as u64 - ch.len_utf8() as u64;
                    sink(Event::Unmappable(ByteOffset(at)));
                }
            }
        }
    };
    text.visit_chunks(whole, &mut |chunk| visit(chunk, false))?;
    // 冲刷有状态编码器（例如 ISO-2022-JP 回到 ASCII 状态）。
    visit("", true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RopeText;

    fn text(s: &str) -> RopeText {
        RopeText::from_text(s)
    }

    #[test]
    fn utf8_with_and_without_bom() {
        let t = text("a\r\nb\n");
        assert_eq!(
            encode(&t, &Encoding::Utf8, false, Unencodable::Reject).unwrap(),
            b"a\r\nb\n"
        );
        assert_eq!(
            encode(&t, &Encoding::Utf8, true, Unencodable::Reject).unwrap(),
            b"\xEF\xBB\xBFa\r\nb\n"
        );
    }

    #[test]
    fn utf16_both_orders_including_surrogates() {
        let t = text("a😀");
        assert_eq!(
            encode(&t, &Encoding::Utf16Le, true, Unencodable::Reject).unwrap(),
            [0xFF, 0xFE, b'a', 0, 0x3D, 0xD8, 0x00, 0xDE]
        );
        assert_eq!(
            encode(&t, &Encoding::Utf16Be, false, Unencodable::Reject).unwrap(),
            [0, b'a', 0xD8, 0x3D, 0xDE, 0x00]
        );
    }

    #[test]
    fn gbk_counts_and_replaces_unencodable_characters() {
        let t = text("中文😀a😀");
        assert_eq!(count_unencodable(&t, &Encoding::Gbk).unwrap(), 2);
        assert_eq!(
            encode(&t, &Encoding::Gbk, false, Unencodable::Reject),
            Err(EncodeError::Unencodable {
                count: 2,
                first: ByteOffset(6)
            })
        );
        let replaced = encode(&t, &Encoding::Gbk, false, Unencodable::Replace).unwrap();
        let mut expected = encoding_rs::GBK.encode("中文").0.into_owned();
        expected.extend_from_slice(b"?a?");
        assert_eq!(replaced, expected);
        // BOM 对 GBK 没有意义，即使元信息误标也不写。
        assert_eq!(
            encode(&t, &Encoding::Gbk, true, Unencodable::Replace).unwrap(),
            expected
        );
    }

    #[test]
    fn unicode_encodings_never_have_unencodable_characters() {
        let t = text("😀");
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            assert_eq!(count_unencodable(&t, &encoding).unwrap(), 0);
        }
        assert!(is_unicode(&Encoding::Other("gb18030".into())));
        assert_eq!(
            count_unencodable(&t, &Encoding::Other("gb18030".into())).unwrap(),
            0
        );
    }

    #[test]
    fn unknown_encoding_is_unsupported_not_silently_utf8() {
        let t = text("a");
        assert_eq!(
            encode(
                &t,
                &Encoding::Other("no-such".into()),
                false,
                Unencodable::Reject
            ),
            Err(EncodeError::Other(Error::Unsupported))
        );
    }
}
