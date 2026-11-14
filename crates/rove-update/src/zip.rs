//! 只读 zip：解开 #43 的索引组件包（issue #149 第 4 节）。
//!
//! 为什么自己写：组件包是**我们自己打的**（`scripts/components/package-indexer.ps1`），
//! 只会用到「存储」与「deflate」两种压缩方式，而 deflate 已经有 `flate2` 了。为这点需求
//! 再拉一个 zip 库进 `rove.exe`（体积预算 30 MB）不划算，而且 zip 解压是典型的
//! 「拿外部数据喂解析器」，依赖越少、代码越短越好审。
//!
//! 安全上的三条硬规则（解的是从网上下载来的包）：
//!
//! 1. **路径穿越一律拒绝**：条目名里出现 `..`、以 `/` 或盘符开头、含 `\` 的，整包作废；
//! 2. **只认 0（存储）与 8（deflate）**，别的压缩方式作废；
//! 3. **解出来的大小有上限**，防止 zip 炸弹。

use crate::{Result, UpdateError};
use std::{
    fs,
    io::Read,
    path::{Component, Path},
};

/// 解压后允许的总字节数上限（组件包 trim 版约 32 MB，留足余量）。
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
const METHOD_STORE: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

fn bad(why: impl Into<String>) -> UpdateError {
    UpdateError::Integrity(format!("组件包无法解开（{}）", why.into()))
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    bytes
        .get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| bad("内容被截断"))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    bytes
        .get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| bad("内容被截断"))
}

/// 条目名是否安全（见模块文档规则 1）。
pub fn is_safe_entry(name: &str) -> bool {
    if name.is_empty() || name.starts_with('/') || name.contains('\\') || name.contains('\0') {
        return false;
    }
    // `C:` 这样的盘符、UNC 前缀。
    if name.len() >= 2 && name.as_bytes()[1] == b':' {
        return false;
    }
    Path::new(name)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
}

/// 把 `archive` 的内容解到 `target`（目录不存在就建出来）。返回解出来的文件数。
pub fn extract(archive: &[u8], target: &Path) -> Result<usize> {
    let entries = read_central_directory(archive)?;
    let mut total = 0u64;
    let mut files = 0usize;
    for entry in &entries {
        let data = read_entry(archive, entry)?;
        total += data.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(bad("解压后的内容超过上限"));
        }
        let path = target.join(&entry.name);
        if entry.is_dir {
            fs::create_dir_all(&path)
                .map_err(|e| UpdateError::Storage(format!("{} 建不出来：{e}", path.display())))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| UpdateError::Storage(format!("{} 建不出来：{e}", parent.display())))?;
        }
        fs::write(&path, &data)
            .map_err(|e| UpdateError::Storage(format!("{} 写不了：{e}", path.display())))?;
        files += 1;
    }
    Ok(files)
}

/// 只读出某一个条目（用来在解压前确认包里有 `component.json`）。
pub fn read_file(archive: &[u8], name: &str) -> Result<Vec<u8>> {
    let entries = read_central_directory(archive)?;
    let entry = entries
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| bad(format!("包里没有 {name}")))?;
    read_entry(archive, entry)
}

/// 包里全部条目的名字（测试与诊断用）。
pub fn entry_names(archive: &[u8]) -> Result<Vec<String>> {
    Ok(read_central_directory(archive)?
        .into_iter()
        .map(|e| e.name)
        .collect())
}

#[derive(Debug)]
struct Entry {
    name: String,
    method: u16,
    compressed_size: u64,
    uncompressed_size: u64,
    local_header: u64,
    is_dir: bool,
}

fn read_central_directory(archive: &[u8]) -> Result<Vec<Entry>> {
    // EOCD 在文件末尾，注释最长 64 KiB。
    let window = archive.len().min(66 * 1024);
    let start = archive.len() - window;
    let eocd = (start..archive.len().saturating_sub(21))
        .rev()
        .find(|at| u32_at(archive, *at).ok() == Some(EOCD_SIGNATURE))
        .ok_or_else(|| bad("不是 zip（找不到目录结尾）"))?;
    let count = u16_at(archive, eocd + 10)? as usize;
    let mut at = u32_at(archive, eocd + 16)? as usize;

    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if u32_at(archive, at)? != CENTRAL_SIGNATURE {
            return Err(bad("中央目录项的签名不对"));
        }
        let method = u16_at(archive, at + 10)?;
        let compressed_size = u32_at(archive, at + 20)? as u64;
        let uncompressed_size = u32_at(archive, at + 24)? as u64;
        let name_len = u16_at(archive, at + 28)? as usize;
        let extra_len = u16_at(archive, at + 30)? as usize;
        let comment_len = u16_at(archive, at + 32)? as usize;
        let local_header = u32_at(archive, at + 42)? as u64;
        let name = archive
            .get(at + 46..at + 46 + name_len)
            .ok_or_else(|| bad("条目名被截断"))?;
        let name = std::str::from_utf8(name).map_err(|_| bad("条目名不是 UTF-8"))?;
        let is_dir = name.ends_with('/');
        let clean = name.trim_end_matches('/');
        // 规则 1：路径穿越直接让整包作废。
        if !is_safe_entry(clean) {
            return Err(bad(format!("条目名不安全：{name}")));
        }
        // 规则 2：只认存储与 deflate。
        if !is_dir && method != METHOD_STORE && method != METHOD_DEFLATE {
            return Err(bad(format!("不支持的压缩方式 {method}")));
        }
        entries.push(Entry {
            name: clean.to_string(),
            method,
            compressed_size,
            uncompressed_size,
            local_header,
            is_dir,
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

fn read_entry(archive: &[u8], entry: &Entry) -> Result<Vec<u8>> {
    if entry.is_dir {
        return Ok(Vec::new());
    }
    let at = entry.local_header as usize;
    if u32_at(archive, at)? != LOCAL_SIGNATURE {
        return Err(bad("本地头的签名不对"));
    }
    let name_len = u16_at(archive, at + 26)? as usize;
    let extra_len = u16_at(archive, at + 28)? as usize;
    let data_at = at + 30 + name_len + extra_len;
    let data = archive
        .get(data_at..data_at + entry.compressed_size as usize)
        .ok_or_else(|| bad("条目数据被截断"))?;

    let out = match entry.method {
        METHOD_STORE => data.to_vec(),
        _ => {
            let mut out = Vec::with_capacity(entry.uncompressed_size as usize);
            flate2::read::DeflateDecoder::new(data)
                // 规则 3：解压也有上限，zip 炸弹撑不爆内存。
                .take(MAX_TOTAL_BYTES + 1)
                .read_to_end(&mut out)
                .map_err(|e| bad(format!("deflate 失败：{e}")))?;
            out
        }
    };
    if out.len() as u64 != entry.uncompressed_size {
        return Err(bad(format!(
            "{} 解出来是 {} 字节，目录里写的是 {}",
            entry.name,
            out.len(),
            entry.uncompressed_size
        )));
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// 现造一个 zip（只用「存储」，够测容器解析了）。
    pub(crate) fn build_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let offset = out.len() as u32;
            let crc = crc32(data);
            out.extend_from_slice(&LOCAL_SIGNATURE.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes()); // version
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&METHOD_STORE.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // time
            out.extend_from_slice(&0u16.to_le_bytes()); // date
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);

            central.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes()); // made by
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&0u16.to_le_bytes()); // flags
            central.extend_from_slice(&METHOD_STORE.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra
            central.extend_from_slice(&0u16.to_le_bytes()); // comment
            central.extend_from_slice(&0u16.to_le_bytes()); // disk
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_at = out.len() as u32;
        let central_len = central.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // disk
        out.extend_from_slice(&0u16.to_le_bytes()); // start disk
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&central_len.to_le_bytes());
        out.extend_from_slice(&central_at.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // comment length
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn a_component_package_extracts() {
        let archive = build_zip(&[
            ("component.json", br#"{"component":"indexer-cs"}"#),
            ("rove-indexer-cs.exe", b"MZ fake"),
            ("runtimes/win-x64/native/lib.dll", b"dll"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(extract(&archive, dir.path()).unwrap(), 3);
        assert_eq!(
            fs::read_to_string(dir.path().join("component.json")).unwrap(),
            r#"{"component":"indexer-cs"}"#
        );
        assert!(dir.path().join("runtimes/win-x64/native/lib.dll").is_file());
        assert_eq!(
            read_file(&archive, "rove-indexer-cs.exe").unwrap(),
            b"MZ fake"
        );
        assert!(entry_names(&archive)
            .unwrap()
            .contains(&"component.json".to_string()));
    }

    /// deflate 压出来的条目也能读（组件包真实用的是这个）。
    #[test]
    fn deflated_entries_are_inflated() {
        let payload = "x".repeat(4096);
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(payload.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut archive = build_zip(&[("a.txt", b"")]);
        // 把刚造好的包重做一遍，手动写成 deflate 条目。
        archive.clear();
        let name = "a.txt";
        archive.extend_from_slice(&LOCAL_SIGNATURE.to_le_bytes());
        archive.extend_from_slice(&20u16.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(&METHOD_DEFLATE.to_le_bytes());
        archive.extend_from_slice(&[0u8; 4]);
        archive.extend_from_slice(&crc32(payload.as_bytes()).to_le_bytes());
        archive.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        archive.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        archive.extend_from_slice(&(name.len() as u16).to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(&compressed);
        let central_at = archive.len() as u32;
        let mut central = Vec::new();
        central.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&METHOD_DEFLATE.to_le_bytes());
        central.extend_from_slice(&[0u8; 4]);
        central.extend_from_slice(&crc32(payload.as_bytes()).to_le_bytes());
        central.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        central.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 12]);
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
        let central_len = central.len() as u32;
        archive.extend_from_slice(&central);
        archive.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        archive.extend_from_slice(&[0u8; 4]);
        archive.extend_from_slice(&1u16.to_le_bytes());
        archive.extend_from_slice(&1u16.to_le_bytes());
        archive.extend_from_slice(&central_len.to_le_bytes());
        archive.extend_from_slice(&central_at.to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());

        assert_eq!(read_file(&archive, "a.txt").unwrap(), payload.as_bytes());
    }

    /// 规则 1：路径穿越的包整包作废，磁盘上什么都不留。
    #[test]
    fn path_traversal_is_refused() {
        for name in [
            "../evil.exe",
            "a/../../evil.exe",
            "/etc/passwd",
            "C:/Windows/evil.exe",
            r"a\b.exe",
        ] {
            assert!(!is_safe_entry(name.trim_end_matches('/')), "{name}");
            let archive = build_zip(&[(name, b"x")]);
            let dir = tempfile::tempdir().unwrap();
            let error = extract(&archive, dir.path()).unwrap_err();
            assert!(matches!(error, UpdateError::Integrity(_)), "{name}");
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0, "{name}");
        }
        assert!(is_safe_entry("a/b/c.dll"));
        assert!(is_safe_entry("component.json"));
    }

    #[test]
    fn garbage_is_not_a_zip() {
        for archive in [vec![], vec![0u8; 100], b"PK not really".to_vec()] {
            assert!(extract(&archive, Path::new("nope")).is_err());
        }
    }
}
