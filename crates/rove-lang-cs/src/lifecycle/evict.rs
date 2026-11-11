//! 缓存淘汰（#36 目标 7）：只保留最近打开的 N 个项目，并限制总容量；当前项目永远保留。
use super::{
    layout::{dir_size, is_project_key, PROJECT_META},
    meta::ProjectMeta,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// 淘汰策略。
///
/// 默认 10 个项目、总量 5 GiB。依据：M0 实测索引中型项目 94 MB、大型项目 303 MB（#32 的 v2 目标
/// 大型项目 < 200 MB），L1 与日志相比可以忽略。同时打开或轮换的 Unity 项目通常是个位数，10 个
/// 足够覆盖“最近用过”的项目；5 GiB 能放下十几个大型项目，又不会让 `%LOCALAPPDATA%` 无声地涨到
/// 几十 GB。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CachePolicy {
    pub max_projects: usize,
    pub max_bytes: u64,
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            max_projects: 10,
            max_bytes: 5 << 30,
        }
    }
}

/// 一个被淘汰（或尝试淘汰）的项目目录。
#[derive(Debug)]
pub struct Evicted {
    pub dir: PathBuf,
    pub bytes: u64,
    pub result: io::Result<()>,
}

/// 按“最近打开”从新到旧保留，超过项目数或容量上限的目录被删除。只处理带 `project.json`、
/// 名字符合项目目录格式的子目录；`current_key` 对应的目录无论如何都保留。
pub fn evict(root: &Path, current_key: &str, policy: CachePolicy) -> Vec<Evicted> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut projects: Vec<(bool, u64, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let meta_path = e.path().join(PROJECT_META);
            if !is_project_key(&name) || !meta_path.is_file() {
                return None;
            }
            let opened = ProjectMeta::read(&meta_path).map_or(0, |m| m.last_opened_ms);
            Some((name == current_key, opened, e.path()))
        })
        .collect();
    // 当前项目排第一，其余按最近打开时间从新到旧。
    projects.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let (mut count, mut bytes) = (0usize, 0u64);
    let mut evicted = Vec::new();
    for (current, _, dir) in projects {
        let size = dir_size(&dir);
        if current || (count < policy.max_projects && bytes + size <= policy.max_bytes) {
            count += 1;
            bytes += size;
            continue;
        }
        let result = fs::remove_dir_all(&dir);
        evicted.push(Evicted {
            dir,
            bytes: size,
            result,
        });
    }
    evicted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(root: &Path, key: &str, opened: u64, bytes: usize) {
        let dir = root.join(key);
        fs::create_dir_all(dir.join("l2")).unwrap();
        ProjectMeta {
            project: key.into(),
            last_opened_ms: opened,
            build: None,
        }
        .write(&dir.join(PROJECT_META))
        .unwrap();
        fs::write(dir.join("l2/0000000000000001.rvcsidx"), vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn keeps_recent_projects_within_limits_and_never_the_current_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let key = |n: u8| format!("P{n}-{:016x}", n);
        for n in 1..=5 {
            project(root, &key(n), n as u64, 1000);
        }
        fs::create_dir(root.join("not-ours")).unwrap();
        // 项目数上限 3：保留当前（最旧的 P1）与最近的 P5、P4。
        let gone = evict(
            root,
            &key(1),
            CachePolicy {
                max_projects: 3,
                max_bytes: u64::MAX,
            },
        );
        let mut removed: Vec<_> = gone
            .iter()
            .map(|e| e.dir.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        removed.sort();
        assert_eq!(removed, [key(2), key(3)]);
        assert!(root.join(key(1)).exists() && root.join("not-ours").exists());
        // 容量上限小于当前项目本身：当前项目依然保留，其余全部淘汰。
        let gone = evict(
            root,
            &key(1),
            CachePolicy {
                max_projects: 10,
                max_bytes: 10,
            },
        );
        assert_eq!(gone.len(), 2);
        assert!(root.join(key(1)).exists());
        assert!(!root.join(key(5)).exists());
    }
}
