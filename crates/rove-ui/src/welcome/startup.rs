//! 启动参数 `rove.exe <路径>`（#28，立项文档 §7）。
//!
//! - 参数是文件就打开文件，是文件夹就作为工作区打开；支持多个参数与相对路径。
//! - **长路径**（> 260 字符）在 Windows 上转成 `\\?\` 前缀的绝对路径再访问，
//!   否则 Win32 会在没有开启长路径支持的机器上直接失败。
//! - 路径不存在或没有权限时不弹模态框：窗口照常打开，内容区显示错误占位页（UX_SPEC §6.4），
//!   所以这里只做分类，判定结果交给宿主。
//! - 一个窗口一个文件夹（FEATURES §6，不做多根工作区）：第一个文件夹是本窗口的工作区，
//!   其余文件夹各开一个窗口。
//! - 不提供 `code` 那样的 shell 命令行工具。

use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

/// 一个启动参数的分类。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Folder(PathBuf),
    File(PathBuf),
    /// 不存在、没有权限或路径无效：窗口照常打开，内容区显示错误占位页。
    Missing(PathBuf),
}

/// 一次启动要做的事。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Startup {
    /// 本窗口的工作区（第一个文件夹参数）。
    pub folder: Option<PathBuf>,
    /// 其余文件夹：每个开一个窗口。
    pub extra_folders: Vec<PathBuf>,
    /// 要在本窗口打开的文件（含不存在的路径，后者会落到错误占位页）。
    pub files: Vec<Target>,
}

impl Startup {
    /// 没有任何参数：空窗口（显示欢迎页，资源管理器显示空状态）。
    pub fn is_empty_window(&self) -> bool {
        self.folder.is_none() && self.files.is_empty() && self.extra_folders.is_empty()
    }
}

/// Windows 的长路径前缀。
const VERBATIM: &str = r"\\?\";
/// Win32 的 `MAX_PATH`。
const MAX_PATH: usize = 260;

/// 绝对化并（必要时）加上 `\\?\` 前缀。`\\?\` 不接受 `.` / `..` 与 `/`，所以先把它们规整掉。
pub fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(path),
            Err(_) => path.to_path_buf(),
        }
    };
    if !cfg!(windows) {
        return absolute;
    }
    let text = absolute.to_string_lossy();
    if text.starts_with(VERBATIM) || text.starts_with(r"\\") {
        return absolute;
    }
    if text.len() < MAX_PATH {
        return absolute;
    }
    let cleaned = clean(&absolute);
    // 只有普通的盘符路径能安全地加前缀（UNC 用 `\\?\UNC\`，这里不处理）。
    match cleaned.components().next() {
        Some(Component::Prefix(_)) => {
            PathBuf::from(format!("{VERBATIM}{}", cleaned.to_string_lossy()))
        }
        _ => cleaned,
    }
}

/// 去掉 `.` 与 `..`（纯文本处理，不访问磁盘，长路径上也不会失败）。
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 单个路径的分类。`exists` 由调用方提供，测试里可以不碰磁盘。
pub fn classify_with(path: &Path, exists: &dyn Fn(&Path) -> Option<bool>) -> Target {
    let path = normalize(path);
    match exists(&path) {
        Some(true) => Target::Folder(path),
        Some(false) => Target::File(path),
        None => Target::Missing(path),
    }
}

/// 按磁盘状态分类：目录 → `Folder`，其他存在的路径 → `File`，取不到元数据 → `Missing`。
pub fn classify(path: &Path) -> Target {
    classify_with(path, &|p| std::fs::metadata(p).ok().map(|m| m.is_dir()))
}

/// 把启动参数（已去掉 `--vscode-user-dir`）变成一次启动计划。
pub fn plan_with(
    args: impl IntoIterator<Item = OsString>,
    exists: &dyn Fn(&Path) -> Option<bool>,
) -> Startup {
    let mut startup = Startup::default();
    for arg in args {
        if arg.is_empty() {
            continue;
        }
        match classify_with(Path::new(&arg), exists) {
            Target::Folder(path) => match startup.folder {
                None => startup.folder = Some(path),
                Some(_) => startup.extra_folders.push(path),
            },
            other => startup.files.push(other),
        }
    }
    startup
}

pub fn plan(args: impl IntoIterator<Item = OsString>) -> Startup {
    plan_with(args, &|p| std::fs::metadata(p).ok().map(|m| m.is_dir()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    fn fake(
        folders: &'static [&'static str],
        files: &'static [&'static str],
    ) -> impl Fn(&Path) -> Option<bool> {
        move |p: &Path| {
            let text = p.to_string_lossy().to_lowercase();
            if folders.iter().any(|f| text.ends_with(&f.to_lowercase())) {
                Some(true)
            } else if files.iter().any(|f| text.ends_with(&f.to_lowercase())) {
                Some(false)
            } else {
                None
            }
        }
    }

    #[test]
    fn no_arguments_is_an_empty_window() {
        assert!(plan_with(args(&[]), &fake(&[], &[])).is_empty_window());
    }

    #[test]
    fn folder_file_and_missing_paths() {
        let exists = fake(&["Game"], &["Player.cs"]);
        let plan = plan_with(
            args(&[r"D:\Game", r"D:\Game\Player.cs", r"D:\Gone\Nope.cs"]),
            &exists,
        );
        assert_eq!(plan.folder, Some(PathBuf::from(r"D:\Game")));
        assert!(plan.extra_folders.is_empty());
        assert_eq!(plan.files.len(), 2);
        assert!(matches!(plan.files[0], Target::File(_)));
        assert!(matches!(plan.files[1], Target::Missing(_)));
        assert!(!plan.is_empty_window());
    }

    #[test]
    fn one_folder_per_window_extra_folders_get_their_own() {
        let exists = fake(&["Game", "Other"], &[]);
        let plan = plan_with(args(&[r"D:\Game", r"D:\Other"]), &exists);
        assert_eq!(plan.folder, Some(PathBuf::from(r"D:\Game")));
        assert_eq!(plan.extra_folders, vec![PathBuf::from(r"D:\Other")]);
    }

    #[test]
    fn case_insensitive_paths_classify_the_same_way() {
        let exists = fake(&["Game"], &[]);
        assert!(matches!(
            classify_with(Path::new(r"D:\GAME"), &exists),
            Target::Folder(_)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn long_paths_get_the_verbatim_prefix() {
        let long = format!(r"D:\{}\Player.cs", "segment".repeat(40));
        assert!(long.len() > MAX_PATH);
        let normalized = normalize(Path::new(&long));
        assert!(
            normalized.to_string_lossy().starts_with(VERBATIM),
            "{}",
            normalized.display()
        );
        // 短路径不加前缀。
        assert!(!normalize(Path::new(r"D:\Game"))
            .to_string_lossy()
            .starts_with(VERBATIM));
    }

    #[cfg(windows)]
    #[test]
    fn relative_and_dotted_segments_are_resolved() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(normalize(Path::new("Cargo.toml")), cwd.join("Cargo.toml"));
        assert_eq!(clean(Path::new(r"D:\a\.\b\..\c")), PathBuf::from(r"D:\a\c"));
    }
}
