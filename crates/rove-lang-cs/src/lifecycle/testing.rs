//! 测试辅助：写一份只含文件表的最小 RVCSIDX v2 索引（没有符号与出现位置），供生命周期与界面的
//! 测试模拟索引进程的产物。不用于生产路径。
use super::{hash::xxh64, validity::EXPECTED_INDEXER_VERSION};
use std::{fs, io, path::Path};

/// 最小索引的内容。
#[derive(Clone, Debug)]
pub struct FixtureIndex {
    pub format_version: u32,
    pub indexer_version: [u16; 4],
    pub config_hash: [u8; 32],
    /// 项目相对路径与内容哈希（xxHash64）。写入前按 UTF-8 字节排序。
    pub files: Vec<(String, u64)>,
}

impl FixtureIndex {
    /// 按磁盘上的实际内容给出文件哈希。
    pub fn for_project(root: &Path, files: &[&str], config_hash: [u8; 32]) -> Self {
        Self {
            format_version: 2,
            indexer_version: EXPECTED_INDEXER_VERSION,
            config_hash,
            files: files
                .iter()
                .map(|f| {
                    let hash = fs::read(root.join(f)).map_or(0, |b| xxh64(&b, 0));
                    ((*f).to_owned(), hash)
                })
                .collect(),
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        const HEADER: usize = 96 + 15 * 16;
        let mut files = self.files.clone();
        files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut paths = Vec::new();
        let mut records = Vec::new();
        for (path, hash) in &files {
            records.extend_from_slice(&(paths.len() as u32).to_le_bytes());
            records.extend_from_slice(&[0u8; 12]);
            records.extend_from_slice(&hash.to_le_bytes());
            paths.extend_from_slice(path.as_bytes());
        }
        // 段内容与元素数：0 字符串偏移（1 项）、1 文件、2 路径正文，其余为空。
        let sections: [(Vec<u8>, usize); 15] = std::array::from_fn(|i| match i {
            0 => (0u32.to_le_bytes().to_vec(), 1),
            1 => (records.clone(), files.len()),
            2 => (paths.clone(), paths.len()),
            _ => (Vec::new(), 0),
        });
        let mut out = vec![0u8; HEADER];
        let mut table = Vec::new();
        for (data, count) in &sections {
            while !out.len().is_multiple_of(8) {
                out.push(0);
            }
            table.push((out.len() as u64, *count as u64));
            out.extend_from_slice(data);
        }
        while !out.len().is_multiple_of(8) {
            out.push(0);
        }
        let total = out.len() as u64;
        out[..8].copy_from_slice(b"RVCSIDX\0");
        out[8..12].copy_from_slice(&self.format_version.to_le_bytes());
        out[12..16].copy_from_slice(&15u32.to_le_bytes());
        out[16..24].copy_from_slice(&total.to_le_bytes());
        for (i, v) in self.indexer_version.iter().enumerate() {
            out[24 + i * 2..26 + i * 2].copy_from_slice(&v.to_le_bytes());
        }
        out[32..64].copy_from_slice(&self.config_hash);
        for (i, (offset, count)) in table.iter().enumerate() {
            let at = 96 + i * 16;
            out[at..at + 8].copy_from_slice(&offset.to_le_bytes());
            out[at + 8..at + 16].copy_from_slice(&count.to_le_bytes());
        }
        let checksum = out[..88]
            .iter()
            .chain(&out[96..HEADER])
            .fold(14695981039346656037u64, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(1099511628211)
            });
        out[88..96].copy_from_slice(&checksum.to_le_bytes());
        out
    }

    /// 像索引进程一样先写临时文件再改名。
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let temp = path.with_extension("fixture.tmp");
        fs::write(&temp, self.bytes())?;
        fs::rename(&temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CsIndex;

    #[test]
    fn fixture_opens_with_the_real_reader() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.rvcsidx");
        FixtureIndex {
            format_version: 2,
            indexer_version: EXPECTED_INDEXER_VERSION,
            config_hash: [7; 32],
            files: vec![("Assets/B.cs".into(), 2), ("Assets/A.cs".into(), 1)],
        }
        .write(&path)
        .unwrap();
        let index = CsIndex::open(&path).unwrap();
        assert_eq!(index.files().len(), 2);
        assert_eq!(index.file_path(0), Some("Assets/A.cs"));
        assert_eq!(index.files()[1].hash, 2);
        assert_eq!(index.header().project_config_hash, [7; 32]);
    }
}
