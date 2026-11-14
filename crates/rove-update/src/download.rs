//! 下载并校验（issue #149 第 2 节：「下载的安装包先核对 `size` 与 `sha256`，
//! 不一致时删除并发通知」）。
//!
//! 规矩：
//!
//! 1. 先写到 `<文件名>.part`，**校验通过之后**才改名成最终文件。中途断电只会留下一个
//!    `.part`，不会出现一个看起来完整、其实是半截的安装包。
//! 2. `size` 与 `sha256` **两样都要对**。只要有一样对不上，临时文件立刻删掉并报
//!    [`UpdateError::Integrity`]——调用方据此发通知、保留旧版本（第 5 节）。
//! 3. 下载前先看磁盘够不够：空间不足就放弃并通知，不去写一个注定失败的文件。

use crate::{http::Transport, Result, UpdateError};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

/// 校验用的期望值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expect {
    pub size: u64,
    /// 十六进制（大小写都接受，比较时统一成小写）。
    pub sha256: String,
}

impl Expect {
    pub fn new(size: u64, sha256: impl Into<String>) -> Self {
        Self {
            size,
            sha256: sha256.into(),
        }
    }
}

/// 字节流的 SHA-256，十六进制小写。
pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 已经在内存里的内容的校验（组件清单这种小东西）。
pub fn verify_bytes(bytes: &[u8], expect: &Expect) -> Result<()> {
    check(bytes.len() as u64, &sha256(bytes), expect)
}

fn check(size: u64, digest: &str, expect: &Expect) -> Result<()> {
    // 验收：体积不符 → 不更新。
    if size != expect.size {
        return Err(UpdateError::Integrity(format!(
            "体积是 {size} 字节，清单写的是 {} 字节",
            expect.size
        )));
    }
    // 验收：SHA256 不符 → 不更新。
    if !digest.eq_ignore_ascii_case(&expect.sha256) {
        return Err(UpdateError::Integrity(format!(
            "SHA-256 是 {digest}，清单写的是 {}",
            expect.sha256.to_ascii_lowercase()
        )));
    }
    Ok(())
}

/// 一边写文件一边算 SHA-256，同时盯着字节数。
struct Checked<W: Write> {
    inner: W,
    hasher: Sha256,
    written: u64,
}

impl<W: Write> Write for Checked<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buffer)?;
        self.hasher.update(&buffer[..n]);
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// 把 `url` 下载到 `dir/file`，校验通过才留下。返回最终文件路径。
///
/// 任何一步失败都会把临时文件删掉，**不留半截产物**。
pub fn download_verified(
    transport: &dyn Transport,
    url: &str,
    dir: &Path,
    file: &str,
    expect: &Expect,
) -> Result<PathBuf> {
    fs::create_dir_all(dir)
        .map_err(|e| UpdateError::Storage(format!("{} 建不出来：{e}", dir.display())))?;
    let target = dir.join(file);
    let part = dir.join(format!("{file}{}", crate::paths::PART_SUFFIX));
    let _ = fs::remove_file(&part);

    let result = (|| -> Result<String> {
        let handle = File::create(&part)
            .map_err(|e| UpdateError::Storage(format!("{} 写不了：{e}", part.display())))?;
        let mut sink = Checked {
            inner: io::BufWriter::new(handle),
            hasher: Sha256::new(),
            written: 0,
        };
        // 上限取清单声明体积多一点点：对不上的内容越早停越好。
        transport.get(url, &mut sink, expect.size.saturating_add(1))?;
        sink.flush()
            .map_err(|e| UpdateError::Storage(e.to_string()))?;
        let digest = hex(&sink.hasher.finalize());
        check(sink.written, &digest, expect)?;
        Ok(digest)
    })();

    match result {
        Ok(_) => {
            let _ = fs::remove_file(&target);
            fs::rename(&part, &target).map_err(|e| {
                let _ = fs::remove_file(&part);
                UpdateError::Storage(format!("改名成 {} 失败：{e}", target.display()))
            })?;
            Ok(target)
        }
        Err(error) => {
            // 校验失败 → 删除下载的文件（第 5 节）。
            let _ = fs::remove_file(&part);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::StubTransport;

    const BODY: &[u8] = b"a pretend installer";

    fn expect() -> Expect {
        // 固定下来的期望值；下面的测试靠改它来制造「体积不符 / SHA256 不符」。
        Expect::new(BODY.len() as u64, sha256(BODY))
    }

    #[test]
    fn a_good_download_lands_as_the_final_file() {
        let dir = tempfile::tempdir().unwrap();
        let stub = StubTransport::new().with("https://a/setup.exe", BODY.to_vec());
        let path = download_verified(
            &stub,
            "https://a/setup.exe",
            dir.path(),
            "setup.exe",
            &expect(),
        )
        .unwrap();
        assert_eq!(path, dir.path().join("setup.exe"));
        assert_eq!(fs::read(&path).unwrap(), BODY);
        // 临时文件不留下。
        assert!(!dir.path().join("setup.exe.part").exists());
    }

    /// 验收：SHA256 不符 → 「不更新」，而且下载的文件被删掉。
    #[test]
    fn a_wrong_hash_deletes_the_download() {
        let dir = tempfile::tempdir().unwrap();
        let stub = StubTransport::new().with("https://a/setup.exe", BODY.to_vec());
        let mut wrong = expect();
        wrong.sha256 = "0".repeat(64);
        let error = download_verified(
            &stub,
            "https://a/setup.exe",
            dir.path(),
            "setup.exe",
            &wrong,
        )
        .unwrap_err();
        assert!(matches!(error, UpdateError::Integrity(_)), "{error:?}");
        assert!(error.reason().contains("SHA-256"));
        assert!(!dir.path().join("setup.exe").exists());
        assert!(!dir.path().join("setup.exe.part").exists());
    }

    /// 验收：体积不符 → 「不更新」。
    #[test]
    fn a_wrong_size_deletes_the_download() {
        let dir = tempfile::tempdir().unwrap();
        let stub = StubTransport::new().with("https://a/setup.exe", BODY.to_vec());
        let mut wrong = expect();
        wrong.size = BODY.len() as u64 + 1;
        let error = download_verified(
            &stub,
            "https://a/setup.exe",
            dir.path(),
            "setup.exe",
            &wrong,
        )
        .unwrap_err();
        assert!(error.reason().contains("体积"), "{error:?}");
        assert!(!dir.path().join("setup.exe").exists());
    }

    /// 服务器多发了内容（比清单写的大）也要被拦住，而不是写一个超大文件到磁盘。
    #[test]
    fn a_longer_body_than_declared_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let stub = StubTransport::new().with("https://a/setup.exe", vec![7u8; 10_000]);
        let error = download_verified(
            &stub,
            "https://a/setup.exe",
            dir.path(),
            "setup.exe",
            &expect(),
        )
        .unwrap_err();
        assert!(matches!(error, UpdateError::Integrity(_)), "{error:?}");
        assert!(!dir.path().join("setup.exe").exists());
    }

    /// 旧的 `.part` 不会被当成已下载的内容。
    #[test]
    fn a_leftover_part_file_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("setup.exe.part"), b"stale").unwrap();
        let stub = StubTransport::new().with("https://a/setup.exe", BODY.to_vec());
        let path = download_verified(
            &stub,
            "https://a/setup.exe",
            dir.path(),
            "setup.exe",
            &expect(),
        )
        .unwrap();
        assert_eq!(fs::read(path).unwrap(), BODY);
    }

    #[test]
    fn hashes_compare_case_insensitively() {
        let digest = sha256(BODY);
        assert!(verify_bytes(
            BODY,
            &Expect::new(BODY.len() as u64, digest.to_ascii_uppercase())
        )
        .is_ok());
        // 已知向量：空串的 SHA-256。
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
