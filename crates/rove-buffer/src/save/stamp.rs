//! 磁盘标记（#49）：判断“磁盘上的文件是不是我们最后一次读到 / 写入的那份”。
//!
//! 只比较长度与修改时间在慢盘、时间精度低的文件系统上会漏判，只比较内容哈希要读全文；
//! 所以标记同时记录三者：长度或修改时间变了才需要读内容，最终以内容哈希为准。

use std::path::Path;
use std::time::SystemTime;

/// 内容哈希（FNV-1a 64 位）。不用于安全目的，只判断内容是否相同。
pub fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 某一时刻磁盘上的文件：长度、修改时间与内容哈希。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskStamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub hash: u64,
}

impl DiskStamp {
    pub fn from_metadata(metadata: &std::fs::Metadata, hash: u64) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            hash,
        }
    }

    /// 从已经读到的字节建立（打开文件时用：读盘与取元数据之间文件可能又被改写，
    /// 以读到的字节为准，修改时间只是快速判断的提示）。
    pub fn from_bytes(path: &Path, bytes: &[u8]) -> Self {
        let modified = std::fs::metadata(path)
            .ok()
            .and_then(|metadata| metadata.modified().ok());
        Self {
            len: bytes.len() as u64,
            modified,
            hash: content_hash(bytes),
        }
    }

    /// 读当前磁盘内容并建立标记；文件不存在返回 `Ok(None)`。
    pub fn read(path: &Path) -> std::io::Result<Option<(Self, Vec<u8>)>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some((Self::from_bytes(path, &bytes), bytes))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// 长度与修改时间都没变（快速判断，不读内容）。
    pub fn same_metadata(&self, metadata: &std::fs::Metadata) -> bool {
        self.len == metadata.len()
            && self.modified.is_some()
            && self.modified == metadata.modified().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_distinguishes_content() {
        assert_ne!(content_hash(b"a"), content_hash(b"b"));
        assert_eq!(content_hash(b""), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn missing_file_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(DiskStamp::read(&dir.path().join("none")).unwrap().is_none());
    }
}
