//! 项目输入：决定索引是否还对得上当前项目的那部分文件（#36 有效性检查的“项目配置”一项）。
//!
//! 索引头部的项目配置哈希由索引进程用 C# 计算（`docs/CS_INDEX_FORMAT.md`“项目配置哈希”），
//! 输入是 csproj 的解析结果，Rust 侧无法逐字节重算。所以生命周期层在**构建成功时**把头部哈希与
//! 这里的“输入指纹”一起记在 `project.json`；打开时要求两者都对得上：
//! - 头部哈希与记录一致：这份索引文件就是那次构建的产物（没有被别处替换）；
//! - 输入指纹与当前磁盘一致：根目录的 `*.csproj`、`Assets/` 与 `Packages/` 下 `*.asmdef` / `*.asmref`
//!   的**内容**，以及这两个目录下 `*.cs` 的**路径列表**（离线增删文件、改变程序集归属）都没变。
//!
//! 与索引进程的取舍不同之处：csproj 只改排版也会让指纹变化而触发重建。Unity 只在项目结构真正
//! 变化时重新生成 csproj，这个代价可以接受；换来的是打开项目时不必启动 .NET 进程。
use super::hash::xxh64;
use rove_base::Cancellation;
use std::{fs, io, path::Path};

/// 扫描到的项目输入。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectInputs {
    pub fingerprint: u64,
    /// 根目录的 `*.csproj` 数量。
    pub projects: usize,
    /// `*.asmdef` 与 `*.asmref` 数量。
    pub assembly_definitions: usize,
    /// `Assets/`、`Packages/` 下的 `*.cs` 数量。
    pub sources: usize,
}

impl ProjectInputs {
    /// 工作区里有没有 C# 代码；没有时状态栏条目整个不显示。
    pub fn has_csharp(&self) -> bool {
        self.projects > 0 || self.sources > 0
    }
    /// 没有 csproj、只能按 asmdef 推断项目结构（UX_SPEC §2.1 的悬停提示）。
    pub fn inferred(&self) -> bool {
        self.projects == 0 && (self.assembly_definitions > 0 || self.sources > 0)
    }
}

/// 文件路径是否属于项目配置（变化时全量重建，而不是标记单个文件过期）。
pub fn is_config_path(rel_path: &str) -> bool {
    let lower = rel_path.to_ascii_lowercase();
    (lower.ends_with(".csproj") && !lower.contains('/'))
        || lower.ends_with(".asmdef")
        || lower.ends_with(".asmref")
}

pub fn is_source_path(rel_path: &str) -> bool {
    rel_path.to_ascii_lowercase().ends_with(".cs")
}

fn extension_is(name: &str, ext: &str) -> bool {
    let name = name.as_bytes();
    let ext = ext.as_bytes();
    name.len() > ext.len() && name[name.len() - ext.len()..].eq_ignore_ascii_case(ext)
}

/// 扫描项目输入。只读 `*.csproj`、`*.asmdef`、`*.asmref` 的内容，源文件只取路径；
/// 不跟随符号链接与 reparse point。在后台线程调用。
pub fn scan(root: &Path, cancel: &Cancellation) -> io::Result<ProjectInputs> {
    let mut entries: Vec<(String, u64)> = Vec::new();
    let mut inputs = ProjectInputs::default();
    for entry in fs::read_dir(root)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|t| t.is_file()) && extension_is(&name, ".csproj") {
            inputs.projects += 1;
            let hash = fs::read(entry.path()).map_or(0, |b| xxh64(&b, 0));
            entries.push((format!("p:{name}"), hash));
        }
    }
    for top in ["Assets", "Packages"] {
        let mut stack = vec![(root.join(top), top.to_owned())];
        while let Some((dir, rel)) = stack.pop() {
            if cancel.check().is_err() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            let Ok(read) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                let child = format!("{rel}/{name}");
                if kind.is_dir() {
                    stack.push((entry.path(), child));
                } else if !kind.is_file() {
                    continue;
                } else if extension_is(&name, ".cs") {
                    inputs.sources += 1;
                    entries.push((format!("s:{child}"), 0));
                } else if extension_is(&name, ".asmdef") || extension_is(&name, ".asmref") {
                    inputs.assembly_definitions += 1;
                    let hash = fs::read(entry.path()).map_or(0, |b| xxh64(&b, 0));
                    entries.push((format!("a:{child}"), hash));
                }
            }
        }
    }
    entries.sort();
    let mut stream = Vec::with_capacity(entries.len() * 48);
    stream.extend_from_slice(b"rove-cs-inputs-v1\0");
    for (path, hash) in &entries {
        stream.extend_from_slice(path.as_bytes());
        stream.push(0);
        stream.extend_from_slice(&hash.to_le_bytes());
    }
    inputs.fingerprint = xxh64(&stream, 0);
    Ok(inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_tracks_config_content_and_source_paths_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let none = scan(root, &Cancellation::default()).unwrap();
        assert!(!none.has_csharp());
        fs::create_dir_all(root.join("Assets/Game")).unwrap();
        fs::write(root.join("Assets/Game/Game.asmdef"), r#"{"name":"Game"}"#).unwrap();
        fs::write(root.join("Assets/Game/A.cs"), "class A {}").unwrap();
        let inferred = scan(root, &Cancellation::default()).unwrap();
        assert!(inferred.has_csharp() && inferred.inferred());
        fs::write(root.join("Game.csproj"), "<Project/>").unwrap();
        let base = scan(root, &Cancellation::default()).unwrap();
        assert!(!base.inferred());
        assert_ne!(base.fingerprint, inferred.fingerprint);
        // 源文件内容不在指纹里（由抽样文件哈希负责）。
        fs::write(root.join("Assets/Game/A.cs"), "class A { int x; }").unwrap();
        assert_eq!(scan(root, &Cancellation::default()).unwrap(), base);
        // 离线新增文件、改程序集定义、改 csproj 都会改变指纹。
        fs::write(root.join("Assets/Game/B.cs"), "class B {}").unwrap();
        let added = scan(root, &Cancellation::default()).unwrap();
        assert_ne!(added.fingerprint, base.fingerprint);
        fs::write(root.join("Assets/Game/Game.asmdef"), r#"{"name":"Other"}"#).unwrap();
        assert_ne!(
            scan(root, &Cancellation::default()).unwrap().fingerprint,
            added.fingerprint
        );
        assert!(is_config_path("Game.csproj"));
        assert!(!is_config_path("Sub/Game.csproj"));
        assert!(is_config_path("Assets/X/Y.ASMDEF"));
        assert!(is_source_path("Assets/A.cs") && !is_source_path("Assets/A.csv"));
    }
}
