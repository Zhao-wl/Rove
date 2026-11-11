//! 缓存目录布局（#36 定义；#37 的 L1 快速声明索引共用）。
//!
//! ```text
//! <缓存根>\                      rove.index.cacheDir；为空时 %LOCALAPPDATA%\Rove\index
//!   <文件夹名>-<16 位哈希>\       每个项目一个目录（项目根路径规范化后的 FNV-1a-64）
//!     project.json              项目路径、最近打开时间（淘汰依据）、L2 构建记录
//!     l1\l1-decls.bin           L1 快速声明索引（#37；目录由本模块给出，文件名由 #37 固定）
//!     l2\<世代>.rvcsidx         L2 语义索引，世代号为 16 位十六进制，最大的有效世代即当前索引
//!     l2\*.tmp                  索引进程写入中的临时文件（索引进程自己先写临时文件再改名）
//!     logs\indexer.log          索引日志（超过上限时轮转为 indexer.1.log … indexer.3.log）
//! ```
//!
//! 世代文件让“原子替换”在 Windows 上成立：正在被内存映射的文件不能被覆盖，所以新索引写到
//! 新世代，切换只是在内存里换一个 `Arc`，旧世代在最后一个查询放手后删除。
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

pub const PROJECT_META: &str = "project.json";
/// #37 的 L1 索引目录名；`ProjectCache::l1_dir` 返回它的完整路径。
pub const L1_DIR: &str = "l1";
/// #37 固定的 L1 文件名（这里只用来计算缓存占用与文档说明，不读写它）。
pub const L1_FILE: &str = "l1-decls.bin";
pub const L2_DIR: &str = "l2";
pub const L2_EXT: &str = "rvcsidx";
pub const LOG_DIR: &str = "logs";
pub const LOG_FILE: &str = "indexer.log";

/// 默认缓存根目录：`%LOCALAPPDATA%\Rove\index`。
pub fn default_root(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    env("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(|dir| PathBuf::from(dir).join("Rove").join("index"))
}

/// 由 `rove.index.cacheDir` 的值得到缓存根目录。空值用默认目录；相对路径没有明确的基准，
/// 视为无效（返回 `Err`，界面显示“缓存目录不可用”）。
pub fn resolve_root(
    setting: &str,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, String> {
    let value = setting.trim();
    if value.is_empty() {
        return default_root(env).ok_or_else(|| "LOCALAPPDATA is not set".to_owned());
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "rove.index.cacheDir is not an absolute path: {value}"
        ))
    }
}

fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// 项目目录名：`<小写文件夹名>-<哈希>`。路径先规范化（去掉 `\\?\`、统一反斜杠、去掉末尾分隔符、
/// Windows 上不区分大小写），所以同一个文件夹换种写法打开得到同一个目录。
pub fn project_key(project_root: &Path) -> String {
    let canonical = fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_owned());
    let mut text = canonical.to_string_lossy().replace('/', "\\");
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        text = format!(r"\\{rest}");
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        text = rest.to_owned();
    }
    while text.len() > 3 && text.ends_with('\\') {
        text.pop();
    }
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    let name: String = project_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .map(|c| c.to_ascii_lowercase())
        .take(32)
        .collect();
    let hash = fnv1a(text.bytes());
    if name.is_empty() {
        format!("{hash:016x}")
    } else {
        format!("{name}-{hash:016x}")
    }
}

/// 目录名是不是本模块生成的项目目录（淘汰只碰这种目录，自定义缓存根里的其他东西不动）。
pub fn is_project_key(name: &str) -> bool {
    let hash = name.rsplit('-').next().unwrap_or(name);
    hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 一个项目的缓存目录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectCache {
    root: PathBuf,
    key: String,
    dir: PathBuf,
}

impl ProjectCache {
    pub fn new(root: &Path, project_root: &Path) -> Self {
        let key = project_key(project_root);
        Self {
            dir: root.join(&key),
            root: root.to_owned(),
            key,
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn meta_path(&self) -> PathBuf {
        self.dir.join(PROJECT_META)
    }
    /// 交给 #37 持久化接口的目录（文件名 `l1-decls.bin` 由 #37 固定）。
    pub fn l1_dir(&self) -> PathBuf {
        self.dir.join(L1_DIR)
    }
    pub fn l2_dir(&self) -> PathBuf {
        self.dir.join(L2_DIR)
    }
    pub fn l2_path(&self, generation: u64) -> PathBuf {
        self.l2_dir().join(format!("{generation:016x}.{L2_EXT}"))
    }
    pub fn log_dir(&self) -> PathBuf {
        self.dir.join(LOG_DIR)
    }
    pub fn log_path(&self) -> PathBuf {
        self.log_dir().join(LOG_FILE)
    }
    /// 创建项目目录与子目录；`rove.index.enabled = false` 时从不调用。
    pub fn ensure(&self) -> io::Result<()> {
        for dir in [self.l1_dir(), self.l2_dir(), self.log_dir()] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }
    /// 已有的 L2 世代，新的在前。
    pub fn generations(&self) -> Vec<(u64, PathBuf)> {
        let mut found: Vec<_> = fs::read_dir(self.l2_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension()? != L2_EXT {
                    return None;
                }
                let generation = u64::from_str_radix(path.file_stem()?.to_str()?, 16).ok()?;
                Some((generation, path))
            })
            .collect();
        found.sort_by_key(|g| std::cmp::Reverse(g.0));
        found
    }
    /// 下一个世代号（比现有最大的大 1）。
    pub fn next_generation(&self) -> u64 {
        self.generations().first().map_or(1, |g| g.0 + 1)
    }
    /// 项目目录的总字节数（详情里的“缓存大小”与淘汰的容量统计）。
    pub fn size(&self) -> u64 {
        dir_size(&self.dir)
    }
}

pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_stable_and_leaves_room_for_l1() {
        let env =
            |k: &str| (k == "LOCALAPPDATA").then(|| OsString::from(r"C:\Users\u\AppData\Local"));
        assert_eq!(
            resolve_root("", env).unwrap(),
            PathBuf::from(r"C:\Users\u\AppData\Local\Rove\index")
        );
        assert_eq!(
            resolve_root(r"  D:\cache  ", env).unwrap(),
            PathBuf::from(r"D:\cache")
        );
        assert!(resolve_root("relative", env).is_err());
        assert!(resolve_root("", |_| None).is_err());

        let a = project_key(Path::new(r"Z:\No\Such\Game"));
        assert_eq!(a, project_key(Path::new(r"z:/no/such/game/")));
        assert!(a.starts_with("game-"));
        assert!(is_project_key(&a));
        assert!(!is_project_key("Unrelated"));
        let cache = ProjectCache::new(Path::new(r"D:\cache"), Path::new(r"Z:\No\Such\Game"));
        assert_eq!(cache.l1_dir(), cache.dir().join("l1"));
        assert_eq!(
            cache.l2_path(0x2a),
            cache.dir().join("l2").join("000000000000002a.rvcsidx")
        );
        assert_eq!(
            cache.log_path(),
            cache.dir().join("logs").join("indexer.log")
        );
    }
}
