//! `project.json`：项目路径、最近打开时间与最近一次成功构建的记录。写入走临时文件 + 改名。
use serde_json::{json, Value};
use std::{
    fs, io,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectMeta {
    /// 项目根目录（只用于显示与排查，身份以目录名的哈希为准）。
    pub project: String,
    /// 最近一次打开的时间（Unix 毫秒），淘汰按它排序。
    pub last_opened_ms: u64,
    pub build: Option<BuildRecord>,
}

/// 最近一次成功构建的 L2 索引。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildRecord {
    pub generation: u64,
    /// 索引头部项目配置哈希的十六进制。
    pub config_hash: String,
    /// 构建时的项目输入指纹（`inputs::scan`）。
    pub fingerprint: u64,
    pub built_at_ms: u64,
    pub duration_ms: u64,
    pub files: u64,
    pub symbols: u64,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_u64(v: &Value) -> Option<u64> {
    u64::from_str_radix(v.as_str()?, 16).ok()
}

impl ProjectMeta {
    pub fn read(path: &Path) -> Option<Self> {
        let value: Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        let build = value
            .get("build")
            .filter(|b| b.is_object())
            .map(|b| BuildRecord {
                generation: hex_u64(&b["generation"]).unwrap_or(0),
                config_hash: b["configHash"].as_str().unwrap_or_default().to_owned(),
                fingerprint: hex_u64(&b["fingerprint"]).unwrap_or(0),
                built_at_ms: b["builtAt"].as_u64().unwrap_or(0),
                duration_ms: b["durationMs"].as_u64().unwrap_or(0),
                files: b["files"].as_u64().unwrap_or(0),
                symbols: b["symbols"].as_u64().unwrap_or(0),
            });
        Some(Self {
            project: value["project"].as_str().unwrap_or_default().to_owned(),
            last_opened_ms: value["lastOpened"].as_u64().unwrap_or(0),
            build,
        })
    }

    pub fn write(&self, path: &Path) -> io::Result<()> {
        let build = self.build.as_ref().map_or(Value::Null, |b| {
            json!({
                "generation": format!("{:016x}", b.generation),
                "configHash": b.config_hash,
                "fingerprint": format!("{:016x}", b.fingerprint),
                "builtAt": b.built_at_ms,
                "durationMs": b.duration_ms,
                "files": b.files,
                "symbols": b.symbols,
            })
        });
        let value = json!({
            "version": 1,
            "project": self.project,
            "lastOpened": self.last_opened_ms,
            "build": build,
        });
        let text = serde_json::to_vec_pretty(&value).map_err(io::Error::other)?;
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::write(&temp, text)?;
        fs::rename(&temp, path).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project.json");
        assert_eq!(ProjectMeta::read(&path), None);
        let meta = ProjectMeta {
            project: r"D:\Game".into(),
            last_opened_ms: 42,
            build: Some(BuildRecord {
                generation: 3,
                config_hash: hex(&[0xab; 32]),
                fingerprint: u64::MAX - 1,
                built_at_ms: 7,
                duration_ms: 8,
                files: 9,
                symbols: 10,
            }),
        };
        meta.write(&path).unwrap();
        assert_eq!(ProjectMeta::read(&path), Some(meta));
        fs::write(&path, "not json").unwrap();
        assert_eq!(ProjectMeta::read(&path), None);
    }
}
