//! 最近打开的文件夹与文件（#28）。
//!
//! 列表存在 **Rove 自己的状态目录**（`%APPDATA%\Rove\recent.json`），绝不写进项目目录，
//! 也不写进共用的 `settings.json`。显示完整路径；同一路径只保留一条（Windows 路径大小写不敏感），
//! 最近打开的在最前面，超过 [`LIMIT`] 条后丢弃最旧的。

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 列表上限（与 VS Code 欢迎页的“最近”一致：只显示最近的一小段，其余走 `Ctrl+R`）。
pub const LIMIT: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentKind {
    Folder,
    File,
}

impl RecentKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::File => "file",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "file" => Self::File,
            _ => Self::Folder,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub kind: RecentKind,
}

impl RecentEntry {
    /// 列表里显示的完整路径（UX_SPEC §1.6）。
    pub fn display(&self) -> String {
        self.path.display().to_string()
    }
}

/// Windows 路径大小写不敏感：比较时统一小写并把分隔符归一。
fn key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecentList {
    entries: Vec<RecentEntry>,
}

impl RecentList {
    pub fn entries(&self) -> &[RecentEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 记一次打开：移到最前，去重，超出上限后丢弃最旧的。
    pub fn add(&mut self, path: impl Into<PathBuf>, kind: RecentKind) {
        let path = path.into();
        let k = key(&path);
        self.entries.retain(|e| key(&e.path) != k);
        self.entries.insert(0, RecentEntry { path, kind });
        self.entries.truncate(LIMIT);
    }

    /// 从列表中移除一项；返回是否移除了。
    pub fn remove(&mut self, path: &Path) -> bool {
        let k = key(path);
        let before = self.entries.len();
        self.entries.retain(|e| key(&e.path) != k);
        before != self.entries.len()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "version": 1,
            "entries": self
                .entries
                .iter()
                .map(|e| json!({ "path": e.path.to_string_lossy(), "kind": e.kind.as_str() }))
                .collect::<Vec<_>>(),
        })
    }

    pub fn from_json(value: &Value) -> Self {
        let mut list = Self::default();
        let items = value
            .get("entries")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for item in items {
            let Some(path) = item.get("path").and_then(Value::as_str) else {
                continue;
            };
            if path.is_empty() {
                continue;
            }
            let kind = RecentKind::parse(item.get("kind").and_then(Value::as_str).unwrap_or(""));
            let k = key(Path::new(path));
            if list.entries.iter().any(|e| key(&e.path) == k) {
                continue;
            }
            list.entries.push(RecentEntry {
                path: PathBuf::from(path),
                kind,
            });
        }
        list.entries.truncate(LIMIT);
        list
    }

    /// 读取状态文件；路径为 `None`（测试、取不到 `%APPDATA%`）或文件损坏时返回空列表。
    pub fn load(path: Option<&Path>) -> Self {
        path.and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .map(|v| Self::from_json(&v))
            .unwrap_or_default()
    }

    /// 写回状态文件（先写临时文件再替换）；路径为 `None` 时什么也不做。
    pub fn save(&self, path: Option<&Path>) {
        let Some(path) = path else { return };
        if let Err(e) = crate::shell::layout::write_json(path, &self.to_json()) {
            eprintln!("welcome: 无法保存最近打开列表：{e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_deduplicated_case_insensitively_and_capped() {
        let mut list = RecentList::default();
        list.add(r"D:\Game", RecentKind::Folder);
        list.add(r"D:\Game\Player.cs", RecentKind::File);
        // 同一个文件夹再打开一次：只留一条，并回到最前。
        list.add(r"d:\game", RecentKind::Folder);
        assert_eq!(list.entries().len(), 2);
        assert_eq!(list.entries()[0].path, PathBuf::from(r"d:\game"));
        assert_eq!(list.entries()[0].kind, RecentKind::Folder);
        for i in 0..LIMIT {
            list.add(format!(r"D:\p{i}"), RecentKind::Folder);
        }
        assert_eq!(list.entries().len(), LIMIT);
        assert_eq!(
            list.entries()[0].path,
            PathBuf::from(format!(r"D:\p{}", LIMIT - 1))
        );
    }

    #[test]
    fn remove_and_round_trip_through_json() {
        let mut list = RecentList::default();
        list.add(r"D:\Game", RecentKind::Folder);
        list.add(r"D:\Game\Player.cs", RecentKind::File);
        assert!(list.remove(Path::new(r"D:\GAME")));
        assert!(!list.remove(Path::new(r"D:\Nope")));
        let restored = RecentList::from_json(&list.to_json());
        assert_eq!(restored, list);
        assert_eq!(restored.entries()[0].kind, RecentKind::File);
        // 完整路径（不是文件名）。
        assert_eq!(restored.entries()[0].display(), r"D:\Game\Player.cs");
    }

    #[test]
    fn broken_or_missing_file_yields_empty_list() {
        assert!(RecentList::load(None).is_empty());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recent.json");
        assert!(RecentList::load(Some(&path)).is_empty());
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(RecentList::load(Some(&path)).is_empty());
    }

    #[test]
    fn saved_list_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("recent.json");
        let mut list = RecentList::default();
        list.add(r"D:\Game", RecentKind::Folder);
        list.save(Some(&path));
        assert_eq!(RecentList::load(Some(&path)), list);
    }
}
