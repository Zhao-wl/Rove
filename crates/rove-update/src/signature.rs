//! 更新清单的 ed25519 分离签名（issue #149 第 2 节，格式由 #76 产出）。
//!
//! - 公钥**编进 `rove.exe`**：`include_str!` 直接读仓库里的
//!   [`packaging/release/update-signing.pub`](../../../packaging/release/update-signing.pub)，
//!   所以「仓库里的公钥」和「二进制里的公钥」永远是同一把，不可能忘记同步。
//!   私钥在仓库外（`%LOCALAPPDATA%\Rove-release\update-signing.key`），与本 crate 无关。
//! - 签名是**分离式**的：对 `latest.json` 的**字节**签名，`latest.json.sig` 里是一行
//!   `ed25519 <base64>`。验签读同样的两份原始字节，**绝不重新序列化 JSON**
//!   （字段顺序或空白的差异会让验签失败，`docs/RELEASE.md` §4 写明了这一点）。
//! - 验签失败一律当作「没有更新」，只写日志（#149 第 2 节）。

use crate::{Result, UpdateError};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use std::sync::OnceLock;

/// 编进二进制的公钥文件内容（与仓库里的那一份逐字节相同）。
pub const PUBLIC_KEY_FILE: &str = include_str!("../../../packaging/release/update-signing.pub");

/// 密钥与签名文件里的算法前缀（#76 的格式）。
const ALGORITHM: &str = "ed25519";

/// 解析 `ed25519 <base64>` 这样的一行（允许 `#` 注释行与空行）。
fn parse_line(text: &str, what: &str) -> Result<Vec<u8>> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (algorithm, rest) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| UpdateError::Signature(format!("{what}应为 `{ALGORITHM} <base64>`")))?;
        if algorithm != ALGORITHM {
            return Err(UpdateError::Signature(format!(
                "{what}的算法是 `{algorithm}`，只支持 `{ALGORITHM}`"
            )));
        }
        let encoded = rest.split_whitespace().next().unwrap_or_default();
        return base64_decode(encoded)
            .ok_or_else(|| UpdateError::Signature(format!("{what}的 base64 无法解码")));
    }
    Err(UpdateError::Signature(format!("{what}里没有内容行")))
}

/// 内嵌的验签公钥。公钥编译期就固定，解析失败只可能是仓库里的文件坏了。
pub fn verifying_key() -> Result<&'static VerifyingKey> {
    static KEY: OnceLock<std::result::Result<VerifyingKey, String>> = OnceLock::new();
    KEY.get_or_init(|| {
        let bytes = parse_line(PUBLIC_KEY_FILE, "公钥").map_err(|e| e.reason())?;
        let bytes: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| "公钥不是 32 字节".to_string())?;
        VerifyingKey::from_bytes(&bytes).map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(|e| UpdateError::Signature(e.clone()))
}

/// 公钥指纹（SHA-256 前 8 字节的十六进制），与 `cargo xtask release-keygen` 打印的一致。
/// 人工核对「发布用的私钥」和「二进制里的公钥」是不是一对时用得上。
pub fn fingerprint() -> Result<String> {
    use sha2::Digest;
    let key = verifying_key()?;
    let digest = sha2::Sha256::digest(key.to_bytes());
    Ok(digest.iter().take(8).map(|b| format!("{b:02x}")).collect())
}

/// 用内嵌公钥验证 `bytes` 的分离签名。`signature_file` 是 `latest.json.sig` 的全文。
pub fn verify(bytes: &[u8], signature_file: &str) -> Result<()> {
    verify_with(verifying_key()?, bytes, signature_file)
}

/// 用指定公钥验签。
///
/// 生产路径只会传[`verifying_key`]。另一个调用方是演练用的
/// [`crate::Updater::set_test_public_key`]——它**只在调试构建里存在**，
/// 所以发布的 `rove.exe` 里没有任何办法换掉验签用的公钥。
pub fn verify_with(key: &VerifyingKey, bytes: &[u8], signature_file: &str) -> Result<()> {
    let raw = parse_line(signature_file, "签名")?;
    let raw: [u8; 64] = raw
        .as_slice()
        .try_into()
        .map_err(|_| UpdateError::Signature("签名不是 64 字节".into()))?;
    key.verify(bytes, &Signature::from_bytes(&raw))
        .map_err(|_| UpdateError::Signature("签名与清单内容对不上".into()))
}

/// 把 32 字节原始公钥变成可用于 [`verify_with`] 的公钥。
pub fn key_from_bytes(bytes: [u8; 32]) -> Result<VerifyingKey> {
    VerifyingKey::from_bytes(&bytes).map_err(|e| UpdateError::Signature(format!("公钥不合法：{e}")))
}

/// 签名文件的一行（`ed25519 <base64>`）。发布侧由 `cargo xtask release-sign` 产出，
/// 这里提供同样的格式是为了让演练测试能造出一份真的签名文件。
pub fn signature_line(signature: &[u8; 64]) -> String {
    format!("{ALGORITHM} {}", base64_encode(signature))
}

/// 标准 base64 编码（带填充）。
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut buffer = [0u8; 3];
        buffer[..chunk.len()].copy_from_slice(chunk);
        let acc = u32::from_be_bytes([0, buffer[0], buffer[1], buffer[2]]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((acc >> (18 - 6 * i)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// 标准 base64 解码（带 `=` 填充）。只有密钥与签名两处用到，不值得为它加一个依赖。
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |b: u8| -> Option<u32> {
        Some(match b {
            b'A'..=b'Z' => u32::from(b - b'A'),
            b'a'..=b'z' => u32::from(b - b'a') + 26,
            b'0'..=b'9' => u32::from(b - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let padding = chunk.iter().filter(|b| **b == b'=').count();
        if padding > 2 || (padding > 0 && !chunk[4 - padding..].iter().all(|b| *b == b'=')) {
            return None;
        }
        let mut acc = 0u32;
        for b in &chunk[..4 - padding] {
            acc = (acc << 6) | value(*b)?;
        }
        acc <<= 6 * padding;
        let full = acc.to_be_bytes();
        out.extend_from_slice(&full[1..4 - padding]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仓库里的公钥能被二进制读出来，指纹与 `update-signing.pub` 的注释一致。
    #[test]
    fn the_embedded_public_key_is_the_one_in_the_repository() {
        let key = verifying_key().expect("内嵌公钥可解析");
        assert_eq!(key.to_bytes().len(), 32);
        let fingerprint = fingerprint().unwrap();
        assert_eq!(fingerprint.len(), 16);
        assert!(
            PUBLIC_KEY_FILE.contains(&fingerprint),
            "公钥文件的注释里应写着指纹 {fingerprint}"
        );
    }

    /// RFC 8032 §7.1 TEST 1：空消息。确认用的是标准 ed25519，而不是某种变体。
    #[test]
    fn rfc8032_test_vector_verifies() {
        let public = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let signature = hex(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );
        let key = VerifyingKey::from_bytes(&public.try_into().unwrap()).unwrap();
        let signature: [u8; 64] = signature.try_into().unwrap();
        assert!(key.verify(b"", &Signature::from_bytes(&signature)).is_ok());
    }

    /// 验收：签名错误 → 当作没有更新。改一个字节就验不过。
    #[test]
    fn a_wrong_signature_is_rejected() {
        let manifest = br#"{"schema":1}"#;
        // 用内嵌公钥（没有对应私钥）验一个凑出来的签名：必然失败，而且不会 panic。
        let bogus = format!("{ALGORITHM} {}", base64_encode(&[7u8; 64]));
        let error = verify(manifest, &bogus).unwrap_err();
        assert_eq!(error, UpdateError::Signature("签名与清单内容对不上".into()));
    }

    /// 签名文件本身格式不对的几种情况，报的也是「签名失败」而不是 panic。
    #[test]
    fn malformed_signature_files_are_rejected() {
        for text in [
            "",
            "# 只有注释\n",
            "ed25519",
            "rsa AAAA",
            "ed25519 这不是base64",
            "ed25519 AAAA", // 长度不是 64 字节
        ] {
            let error = verify(b"x", text).unwrap_err();
            assert!(matches!(error, UpdateError::Signature(_)), "{text:?}");
        }
    }

    #[test]
    fn base64_round_trips_and_rejects_garbage() {
        let cases: [&[u8]; 7] = [b"", b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"];
        for case in cases {
            assert_eq!(
                base64_decode(&base64_encode(case)).unwrap_or_default(),
                case
            );
        }
        assert_eq!(base64_decode("A"), None);
        assert_eq!(base64_decode("A==="), None);
        assert_eq!(base64_decode("A=BC"), None);
        assert_eq!(base64_decode("****"), None);
    }

    fn hex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
}
