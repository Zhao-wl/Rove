//! 发布包的内容清单（issue #76）。
//!
//! `packaging/package-contents.toml` 是「发布包里有什么」的唯一来源：
//! `cargo xtask release` 按它组装暂存目录，`cargo xtask check-size` 按它算发布包体积并分项报告。
//! 嵌入 `rove.exe` 的资产（主题、语言配置、codicons、Markdown 预览资源）在同一张表里登记，
//! 这样体积报告能看出是谁长的。

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::util;

pub const DEFAULT_PATH: &str = "packaging/package-contents.toml";
pub const SCHEMA: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct Contents {
    pub schema: u32,
    pub stage_name: String,
    #[serde(default)]
    pub file: Vec<FileEntry>,
    #[serde(default)]
    pub dir: Vec<DirEntry>,
    #[serde(default)]
    pub embedded: Vec<EmbeddedEntry>,
}

#[derive(Debug, Deserialize)]
pub struct FileEntry {
    /// 仓库根目录下的相对路径；`target/` 开头的会换成实际的 CARGO_TARGET_DIR。
    pub source: String,
    /// 在包里的文件名。
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Deserialize)]
pub struct DirEntry {
    pub source: String,
    /// 在包里的子目录名；`.` 表示直接放在包的根目录下。
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddedEntry {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// 包里的一个文件（已解析到绝对路径）。
#[derive(Debug, Clone)]
pub struct PackedFile {
    pub source: PathBuf,
    /// 相对包根目录的路径（用 `/` 分隔，报告里好看）。
    pub relative: String,
    pub kind: String,
    pub size: u64,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EmbeddedGroup {
    pub name: String,
    pub size: u64,
    pub files: usize,
    pub owner: Option<String>,
    pub note: Option<String>,
    /// 目录不存在时为 true（例如 Seti 文件图标主题还没合并）。
    pub missing: bool,
}

impl Contents {
    pub fn load_default() -> Result<Self, Box<dyn std::error::Error>> {
        Self::load(&util::repo_root().join(DEFAULT_PATH))
    }

    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let text =
            util::read_text(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
        let contents: Contents =
            toml::from_str(&text).map_err(|e| format!("解析 {} 失败：{e}", path.display()))?;
        if contents.schema != SCHEMA {
            return Err(format!(
                "{} 的 schema = {}，本版 xtask 只认识 {SCHEMA}",
                path.display(),
                contents.schema
            )
            .into());
        }
        Ok(contents)
    }

    /// 包的顶层目录名（zip 解开后的目录）。
    pub fn stage_name(&self, version: &str) -> String {
        self.stage_name.replace("{version}", version)
    }

    /// 解析清单里的每个条目；`exe` 覆盖 `kind = "binary"` 的来源路径（给 `--binary` 用）。
    pub fn resolve(
        &self,
        exe: Option<&Path>,
    ) -> Result<Vec<PackedFile>, Box<dyn std::error::Error>> {
        let root = util::repo_root();
        let mut out = Vec::new();
        for entry in &self.file {
            let source = match (&entry.kind[..], exe) {
                ("binary", Some(path)) => path.to_path_buf(),
                _ => resolve_source(&root, &entry.source),
            };
            match fs::metadata(&source) {
                Ok(meta) => out.push(PackedFile {
                    relative: entry.name.clone(),
                    kind: entry.kind.clone(),
                    size: meta.len(),
                    note: entry.note.clone(),
                    source,
                }),
                Err(err) if entry.optional => {
                    let _ = err;
                }
                Err(err) => {
                    let hint = if entry.kind == "binary" {
                        "\n先构建：cargo build --release -p rove-app"
                    } else {
                        ""
                    };
                    return Err(format!("找不到 {}：{err}{hint}", source.display()).into());
                }
            }
        }
        for entry in &self.dir {
            let source = resolve_source(&root, &entry.source);
            if !source.is_dir() {
                if entry.optional {
                    continue;
                }
                return Err(format!("找不到目录 {}", source.display()).into());
            }
            let prefix = if entry.name == "." {
                String::new()
            } else {
                format!("{}/", entry.name)
            };
            for (path, relative) in walk(&source)? {
                let size = fs::metadata(&path)?.len();
                out.push(PackedFile {
                    source: path,
                    relative: format!("{prefix}{relative}"),
                    kind: entry.kind.clone(),
                    size,
                    note: None,
                });
            }
        }
        out.sort_by(|a, b| {
            b.size
                .cmp(&a.size)
                .then_with(|| a.relative.cmp(&b.relative))
        });
        Ok(out)
    }

    /// 嵌入资产的分项体积（源目录的未压缩体积）。
    pub fn embedded(&self) -> Result<Vec<EmbeddedGroup>, Box<dyn std::error::Error>> {
        let root = util::repo_root();
        let mut out = Vec::new();
        for entry in &self.embedded {
            let dir = resolve_source(&root, &entry.path);
            if !dir.exists() {
                out.push(EmbeddedGroup {
                    name: entry.name.clone(),
                    size: 0,
                    files: 0,
                    owner: entry.owner.clone(),
                    note: entry.note.clone(),
                    missing: true,
                });
                continue;
            }
            let (mut size, mut files) = (0u64, 0usize);
            for (path, relative) in walk(&dir)? {
                let first = relative.split('/').next().unwrap_or_default().to_string();
                if entry.exclude.contains(&first) {
                    continue;
                }
                size += fs::metadata(&path)?.len();
                files += 1;
            }
            out.push(EmbeddedGroup {
                name: entry.name.clone(),
                size,
                files,
                owner: entry.owner.clone(),
                note: entry.note.clone(),
                missing: false,
            });
        }
        out.sort_by_key(|group| std::cmp::Reverse(group.size));
        Ok(out)
    }
}

fn resolve_source(root: &Path, source: &str) -> PathBuf {
    match source.strip_prefix("target/") {
        Some(rest) => util::target_dir().join(rest),
        None => root.join(source),
    }
}

/// 目录下的全部文件：（绝对路径，相对该目录的 `/` 分隔路径）。
fn walk(dir: &Path) -> std::io::Result<Vec<(PathBuf, String)>> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((current, prefix)) = stack.pop() {
        let mut entries: Vec<_> = fs::read_dir(&current)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, relative));
            } else {
                out.push((path, relative));
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

/// 把清单里的文件复制到 `stage`（已存在的目录会被清空）。
pub fn stage_files(files: &[PackedFile], stage: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if stage.exists() {
        fs::remove_dir_all(stage)?;
    }
    fs::create_dir_all(stage)?;
    for file in files {
        let target = stage.join(file.relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&file.source, &target)
            .map_err(|e| format!("复制 {} 失败：{e}", file.source.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_lists_the_binary_and_the_license_files() {
        let contents = Contents::load_default().unwrap();
        assert_eq!(contents.stage_name("0.1.0"), "Rove-0.1.0-win-x64");
        assert!(
            contents.file.iter().any(|f| f.kind == "binary"),
            "清单里必须有 rove.exe"
        );
        for required in ["THIRD_PARTY_NOTICES.md", "LICENSE.txt"] {
            assert!(
                contents.file.iter().any(|f| f.name == required),
                "发布包必须随带 {required}（issue #76 第 6 节）"
            );
        }
        // 嵌入资产分项：至少覆盖 Markdown 预览资源与 codicons（最大的两块）。
        let groups = contents.embedded().unwrap();
        assert!(groups.iter().any(|g| g.name.contains("Markdown")));
        assert!(groups.iter().any(|g| g.name.contains("codicons")));
        for group in &groups {
            assert!(!group.missing, "{} 的目录不存在", group.name);
            assert!(group.size > 0, "{} 体积为 0", group.name);
        }
    }

    #[test]
    fn every_non_binary_entry_exists_in_the_repository() {
        let contents = Contents::load_default().unwrap();
        let root = util::repo_root();
        for entry in &contents.file {
            if entry.kind == "binary" || entry.optional {
                continue;
            }
            let path = resolve_source(&root, &entry.source);
            assert!(path.is_file(), "{} 不存在", path.display());
        }
    }
}
