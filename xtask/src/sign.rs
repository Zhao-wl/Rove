//! 更新清单的 ed25519 签名（issue #76 第 8 节）。
//!
//! - **私钥只存在仓库外**：`%LOCALAPPDATA%\Rove-release\update-signing.key`（可用环境变量
//!   `ROVE_RELEASE_KEY` 指向别处）。本模块从不把私钥内容打印到任何输出，出错信息里也只有路径。
//! - **公钥提交进仓库**：`packaging/release/update-signing.pub`，供 #149 的自动更新编进 `rove.exe`。
//! - 签名是**分离式**的：对 `latest.json` 的字节原样签名，写出 `latest.json.sig`（一行 base64）。
//!   #149 验证时读同样的两个文件，不需要重新序列化 JSON（避免字段顺序或空白差异导致验签失败）。
//!
//! 子命令：
//! - `cargo xtask release-keygen [--force]`：生成密钥对（已存在时拒绝覆盖）；
//! - `cargo xtask release-sign --file <文件> [--out <文件.sig>]`；
//! - `cargo xtask release-verify --file <文件> [--sig <文件.sig>] [--pubkey <文件>]`。

use std::fs;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::util;

/// 仓库里的公钥文件（提交，供 #149 使用）。
pub const PUBLIC_KEY_PATH: &str = "packaging/release/update-signing.pub";
/// 私钥的默认位置（仓库外），以及覆盖它的环境变量。
pub const PRIVATE_KEY_ENV: &str = "ROVE_RELEASE_KEY";
pub const PRIVATE_KEY_DIR: &str = "Rove-release";
pub const PRIVATE_KEY_FILE: &str = "update-signing.key";
/// 公钥 / 私钥文件里的算法前缀。
const ALGORITHM: &str = "ed25519";

/// 私钥文件的位置：`ROVE_RELEASE_KEY` 优先，否则 `%LOCALAPPDATA%\Rove-release\update-signing.key`。
pub fn private_key_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os(PRIVATE_KEY_ENV) {
        return Ok(PathBuf::from(path));
    }
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or("找不到 %LOCALAPPDATA%；用 ROVE_RELEASE_KEY 指定私钥文件")?;
    Ok(PathBuf::from(local)
        .join(PRIVATE_KEY_DIR)
        .join(PRIVATE_KEY_FILE))
}

pub fn public_key_path() -> PathBuf {
    util::repo_root().join(PUBLIC_KEY_PATH)
}

/// 密钥文件的格式：`ed25519 <base64>`（可带 `#` 注释行）。
fn parse_key_file(text: &str, what: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (algorithm, encoded) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| format!("{what}格式应为 `{ALGORITHM} <base64>`"))?;
        if algorithm != ALGORITHM {
            return Err(format!("{what}的算法是 `{algorithm}`，只支持 `{ALGORITHM}`").into());
        }
        return base64_decode(encoded.split_whitespace().next().unwrap_or_default())
            .ok_or_else(|| format!("{what}的 base64 无法解码").into());
    }
    Err(format!("{what}里没有密钥行").into())
}

fn load_signing_key(path: &Path) -> Result<SigningKey, Box<dyn std::error::Error>> {
    let text = util::read_text(path).map_err(|e| {
        format!(
            "读取私钥 {} 失败：{e}\n先生成：cargo xtask release-keygen",
            path.display()
        )
    })?;
    // 注意：解析失败的信息里只提路径，绝不回显文件内容。
    let bytes = parse_key_file(&text, "私钥")
        .map_err(|e| format!("{} 不是合法的私钥文件：{e}", path.display()))?;
    let seed: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("{} 的私钥不是 32 字节", path.display()))?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn load_verifying_key(path: &Path) -> Result<VerifyingKey, Box<dyn std::error::Error>> {
    let text =
        util::read_text(path).map_err(|e| format!("读取公钥 {} 失败：{e}", path.display()))?;
    let bytes = parse_key_file(&text, "公钥")?;
    let bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("{} 的公钥不是 32 字节", path.display()))?;
    Ok(VerifyingKey::from_bytes(&bytes)?)
}

/// 公钥指纹：公钥字节的 SHA-256 前 8 字节，十六进制。人工核对两边用的是同一把钥匙时够了。
pub fn fingerprint(key: &VerifyingKey) -> String {
    let digest = crate::release::sha256(&key.to_bytes());
    digest[..16].to_string()
}

// ---------------------------------------------------------------- 子命令

pub fn keygen(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let private = private_key_path()?;
    let public = public_key_path();
    if private.exists() && !util::has_flag(args, "--force") {
        println!("私钥已存在：{}", private.display());
        println!("不会覆盖它（覆盖会让已发布的更新清单无法验签）。确需重新生成请加 --force。");
        let key = load_signing_key(&private)?.verifying_key();
        println!("当前公钥指纹：{}", fingerprint(&key));
        return Ok(1);
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| format!("无法获取系统随机数：{e}"))?;
    let signing = SigningKey::from_bytes(&seed);
    let verifying = signing.verifying_key();

    if let Some(parent) = private.parent() {
        fs::create_dir_all(parent)?;
    }
    // 私钥文件：一行 `ed25519 <base64>`，不打印内容。
    fs::write(
        &private,
        format!("{ALGORITHM} {}\n", base64_encode(&signing.to_bytes())),
    )?;
    if let Some(parent) = public.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &public,
        format!(
            "# Rove 更新清单的 ed25519 公钥（issue #76）。#149 的自动更新把它编进 rove.exe。\n\
             # 私钥在仓库外：%LOCALAPPDATA%\\{PRIVATE_KEY_DIR}\\{PRIVATE_KEY_FILE}（绝不提交）。\n\
             # 指纹（公钥的 SHA-256 前 8 字节）：{}\n\
             {ALGORITHM} {}\n",
            fingerprint(&verifying),
            base64_encode(&verifying.to_bytes())
        ),
    )?;
    println!("已生成 ed25519 密钥对：");
    println!(
        "  私钥：{}（仓库外，不要提交、不要打印）",
        private.display()
    );
    println!("  公钥：{}（提交进仓库）", public.display());
    println!("  指纹：{}", fingerprint(&verifying));
    println!(
        "人工事项：把私钥备份到密码管理器，并存入 GitHub Actions 的 secret（见 docs/RELEASE.md）。"
    );
    Ok(0)
}

pub fn sign_file(path: &Path, out: &Path) -> Result<VerifyingKey, Box<dyn std::error::Error>> {
    let key = load_signing_key(&private_key_path()?)?;
    let bytes = fs::read(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    let signature = key.sign(&bytes);
    fs::write(
        out,
        format!("{ALGORITHM} {}\n", base64_encode(&signature.to_bytes())),
    )?;
    Ok(key.verifying_key())
}

pub fn verify_file(
    path: &Path,
    sig: &Path,
    pubkey: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let key = load_verifying_key(pubkey)?;
    let bytes = fs::read(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    let text = util::read_text(sig).map_err(|e| format!("读取 {} 失败：{e}", sig.display()))?;
    let raw = parse_key_file(&text, "签名")?;
    let raw: [u8; 64] = raw
        .as_slice()
        .try_into()
        .map_err(|_| format!("{} 的签名不是 64 字节", sig.display()))?;
    key.verify(&bytes, &Signature::from_bytes(&raw))
        .map_err(|e| format!("签名校验失败：{e}"))?;
    Ok(())
}

pub fn sign_command(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let file = PathBuf::from(
        util::flag(args, "--file").ok_or("用法：cargo xtask release-sign --file <文件>")?,
    );
    let out = util::flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| append_sig(&file));
    let key = sign_file(&file, &out)?;
    println!("已签名 {} → {}", file.display(), out.display());
    println!("公钥指纹：{}", fingerprint(&key));
    Ok(0)
}

pub fn verify_command(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let file = PathBuf::from(
        util::flag(args, "--file").ok_or("用法：cargo xtask release-verify --file <文件>")?,
    );
    let sig = util::flag(args, "--sig")
        .map(PathBuf::from)
        .unwrap_or_else(|| append_sig(&file));
    let pubkey = util::flag(args, "--pubkey")
        .map(PathBuf::from)
        .unwrap_or_else(public_key_path);
    verify_file(&file, &sig, &pubkey)?;
    println!(
        "OK：{} 的签名可以用 {} 验证。",
        file.display(),
        pubkey.display()
    );
    Ok(0)
}

pub fn append_sig(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_os_string();
    name.push(".sig");
    PathBuf::from(name)
}

// ---------------------------------------------------------------- base64

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for ch in text.chars() {
        if ch == '=' {
            break;
        }
        if ch.is_whitespace() {
            continue;
        }
        let value = ALPHABET.iter().position(|c| *c as char == ch)? as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for bytes in [
            vec![],
            vec![0u8],
            vec![0u8, 1],
            vec![0u8, 1, 2],
            (0..=255u8).collect::<Vec<_>>(),
        ] {
            let text = base64_encode(&bytes);
            assert_eq!(base64_decode(&text).unwrap(), bytes, "{text}");
        }
        // 与标准实现对齐的几个定值。
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    /// RFC 8032 §7.1 TEST 1 的密钥对：私钥种子决定公钥，确认我们用的是标准 ed25519。
    #[test]
    fn rfc8032_test_vector_public_key() {
        let seed = hex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60").unwrap();
        let key = SigningKey::from_bytes(&seed.as_slice().try_into().unwrap());
        assert_eq!(
            hex_text(&key.verifying_key().to_bytes()),
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
    }

    /// 签名 → 验签 → 改一个字节就验不过；公钥文件的格式也走一遍。
    #[test]
    fn sign_and_verify_detached_signature() {
        let seed = [7u8; 32];
        let key = SigningKey::from_bytes(&seed);
        let message = br#"{"schema":1,"version":"0.1.0"}"#;
        let signature = key.sign(message);
        let text = format!("{ALGORITHM} {}\n", base64_encode(&signature.to_bytes()));
        let raw = parse_key_file(&text, "签名").unwrap();
        let signature = Signature::from_bytes(&raw.as_slice().try_into().unwrap());
        let verifying = key.verifying_key();
        verifying.verify(message, &signature).unwrap();
        let mut tampered = message.to_vec();
        tampered[2] = b'x';
        assert!(verifying.verify(&tampered, &signature).is_err());
        // 公钥文件的格式（带注释行）也能解析回同一把公钥。
        let file = format!(
            "# 注释\n{ALGORITHM} {}\n",
            base64_encode(&verifying.to_bytes())
        );
        assert_eq!(parse_key_file(&file, "公钥").unwrap(), verifying.to_bytes());
        // 换一把钥匙就验不过。
        let other = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        assert!(other.verify(message, &signature).is_err());
    }

    #[test]
    fn key_file_rejects_other_algorithms() {
        assert!(parse_key_file("rsa AAAA\n", "公钥").is_err());
        assert!(parse_key_file("# 只有注释\n", "公钥").is_err());
    }

    fn hex(text: &str) -> Option<Vec<u8>> {
        (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok())
            .collect()
    }

    fn hex_text(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
