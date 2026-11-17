//! 更新用到的目录与「待安装版本」的记录（issue #149 第 5 节）。
//!
//! ```text
//! %LOCALAPPDATA%\Rove\
//!     updates\
//!         pending.json                     待安装版本的记录（同时只保留一个）
//!         install-attempt.json             已经拉起过安装器的记录（见 Attempt）
//!         install-exit-code.txt            安装器的退出码（由 cmd 在重启 Rove 之前写）
//!         Rove-0.1.1-win-x64-setup.exe     下载好的安装包
//!         <下载中的临时文件>.part
//!     logs\update.log                      更新日志（见 crate::log）
//!     components\indexer-cs\<版本>\        C# 索引组件（#43 的位置，更新时沿用）
//! ```
//!
//! 目录约定与 #50（备份）、#36（索引缓存）、#43（组件）一致：状态一律放在
//! `%LOCALAPPDATA%\Rove` 下，**绝不写进项目目录**；取不到 `%LOCALAPPDATA%` 时
//! 返回 `None`，更新功能整体降级为不可用而不是另找地方写。

use crate::{manifest::Manifest, version::Version, Result, UpdateError};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

/// `%LOCALAPPDATA%` 下的状态根目录名。
pub const STATE_DIR: &str = "Rove";
/// 下载目录名。
pub const UPDATES_DIR: &str = "updates";
/// 日志目录名（与 #50 的崩溃日志同一个目录）。
pub const LOGS_DIR: &str = "logs";
/// 待安装记录的文件名。
pub const PENDING_FILE: &str = "pending.json";
/// 「已经拉起过安装器」的记录文件名（issue #235）。
pub const ATTEMPT_FILE: &str = "install-attempt.json";
/// 安装器退出码的落点：`cmd` 在把 Rove 拉回来**之前**往这里写一行（issue #235）。
pub const EXIT_CODE_FILE: &str = "install-exit-code.txt";
/// 下载中的临时文件后缀。
pub const PART_SUFFIX: &str = ".part";

/// 更新用到的全部位置。测试里直接构造（不碰真实的 `%LOCALAPPDATA%`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdatePaths {
    state_root: PathBuf,
}

impl UpdatePaths {
    pub fn new(state_root: PathBuf) -> Self {
        Self { state_root }
    }

    /// 从环境构造：`%LOCALAPPDATA%\Rove`。
    pub fn from_env(env: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let local = env("LOCALAPPDATA").filter(|v| !v.is_empty())?;
        Some(Self::new(PathBuf::from(local).join(STATE_DIR)))
    }

    pub fn current() -> Option<Self> {
        Self::from_env(|key| std::env::var_os(key))
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub fn updates_dir(&self) -> PathBuf {
        self.state_root.join(UPDATES_DIR)
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.state_root.join(LOGS_DIR)
    }

    pub fn pending_file(&self) -> PathBuf {
        self.updates_dir().join(PENDING_FILE)
    }

    /// 「这次启动之前拉起过安装器」的记录（issue #235）。
    pub fn attempt_file(&self) -> PathBuf {
        self.updates_dir().join(ATTEMPT_FILE)
    }

    /// 安装器退出码的落点（issue #235）。
    pub fn exit_code_file(&self) -> PathBuf {
        self.updates_dir().join(EXIT_CODE_FILE)
    }

    /// 下载目录；不存在就建出来。
    pub fn ensure_updates_dir(&self) -> Result<PathBuf> {
        let dir = self.updates_dir();
        fs::create_dir_all(&dir)
            .map_err(|e| UpdateError::Storage(format!("{} 建不出来：{e}", dir.display())))?;
        Ok(dir)
    }

    /// 读待安装记录。没有、读不出来或内容坏了都返回 `None`（当作没有待安装版本）。
    pub fn read_pending(&self) -> Option<Pending> {
        let text = fs::read_to_string(self.pending_file()).ok()?;
        let pending: Pending = serde_json::from_str(&text).ok()?;
        // 记录在、安装包不在（被清理软件删了）也当作没有。
        pending.installer_path(self).is_file().then_some(pending)
    }

    /// 写待安装记录。
    pub fn write_pending(&self, pending: &Pending) -> Result<()> {
        self.ensure_updates_dir()?;
        let text = serde_json::to_string_pretty(pending)
            .map_err(|e| UpdateError::Storage(format!("待安装记录无法序列化：{e}")))?;
        fs::write(self.pending_file(), text)
            .map_err(|e| UpdateError::Storage(format!("写待安装记录失败：{e}")))
    }

    /// 清掉待安装记录与下载目录里的全部文件（**同时只保留一个待安装版本**）。
    /// `keep` 里的文件名保留下来。
    pub fn clear_downloads(&self, keep: &[&str]) {
        let Ok(entries) = fs::read_dir(self.updates_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == PENDING_FILE && keep.contains(&PENDING_FILE) {
                continue;
            }
            if keep.contains(&name.as_ref()) {
                continue;
            }
            let path = entry.path();
            let _ = if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
        }
    }

    /// 丢弃待安装版本（校验失败、安装失败、已经装上了）。
    ///
    /// 下载目录整个清空，所以安装尝试的记录与退出码也一起没了：
    /// **一次安装请求到此结束**，不会在下次启动又提示装同一个包（issue #235）。
    pub fn discard_pending(&self) {
        let _ = fs::remove_file(self.pending_file());
        self.clear_downloads(&[]);
    }

    /// 读「这次启动之前拉起过安装器」的记录。没有、读不出来或内容坏了都返回 `None`。
    pub fn read_attempt(&self) -> Option<Attempt> {
        let text = fs::read_to_string(self.attempt_file()).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// 写安装尝试记录。**拉起安装器之前**写：只有写下来了，下次启动才分得清
    /// 「装失败了」与「用户只是点了稍后」（issue #235）。
    pub fn write_attempt(&self, attempt: &Attempt) -> Result<()> {
        self.ensure_updates_dir()?;
        // 上一轮留下的退出码不能混进这一次。
        let _ = fs::remove_file(self.exit_code_file());
        let text = serde_json::to_string_pretty(attempt)
            .map_err(|e| UpdateError::Storage(format!("安装尝试记录无法序列化：{e}")))?;
        fs::write(self.attempt_file(), text)
            .map_err(|e| UpdateError::Storage(format!("写安装尝试记录失败：{e}")))
    }

    /// 安装器留下的退出码（没有文件、是空的或者不是数字都返回 `None`）。
    pub fn read_exit_code(&self) -> Option<i32> {
        let text = fs::read_to_string(self.exit_code_file()).ok()?;
        text.trim().parse().ok()
    }
}

/// 「已经为这个版本拉起过安装器」的记录（issue #235）。
///
/// 没有它就分不清下面这两件事，旧版本会按原记录一遍遍提示装同一个包：
///
/// - 用户点了「稍后」然后普通重启 → 待安装记录仍然有效，继续提示；
/// - 点了「重启以更新」，安装器非零退出 → 这次安装失败了，要报告并结束这次安装请求。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Attempt {
    /// 这次要装成哪个版本。
    pub version: String,
    /// 拉起的安装包文件名（只给日志看）。
    pub file: String,
    /// 拉起安装器的时间（UTC 字符串）。
    #[serde(default)]
    pub started_at: String,
    /// 跟这次更新一起装的组件版本（记在这里，装完之后即使 `pending.json`
    /// 读不出来也还知道该留哪一版组件）。
    #[serde(default)]
    pub component_version: Option<String>,
}

impl Attempt {
    /// 从待安装记录生成。
    pub fn new(pending: &Pending, started_at: String) -> Self {
        Self {
            version: pending.version.clone(),
            file: pending.file.clone(),
            started_at,
            component_version: pending.component_version.clone(),
        }
    }

    /// 这次要装的版本是否仍然比当前版本新（= 重启之后版本号没变 = 没装上）。
    pub fn is_newer_than(&self, current: &Version) -> bool {
        Version::parse(&self.version)
            .map(|v| v.is_newer_than(current))
            .unwrap_or(false)
    }
}

/// 待安装版本的记录。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Pending {
    /// 这个安装包会把 Rove 装成哪个版本。
    pub version: String,
    /// 下载目录里的安装包文件名。
    pub file: String,
    pub sha256: String,
    pub size: u64,
    /// 调用安装器的静默参数（来自清单，与 #76 对齐）。
    pub silent_args: Vec<String>,
    /// 下载完成的时间（UTC 字符串，只给日志与「关于」对话框看）。
    #[serde(default)]
    pub downloaded_at: String,
    /// 跟随这次更新一起装的组件版本（没有就是 `None`）。
    #[serde(default)]
    pub component_version: Option<String>,
}

impl Pending {
    pub fn installer_path(&self, paths: &UpdatePaths) -> PathBuf {
        paths.updates_dir().join(&self.file)
    }

    pub fn version(&self) -> Result<Version> {
        Version::parse(&self.version)
    }

    /// 待安装的版本是否仍然比当前版本新。重启安装后版本号变了，这里就会是 `false`，
    /// 流程据此判断「装上了没有」。
    pub fn is_newer_than(&self, current: &Version) -> bool {
        self.version()
            .map(|v| v.is_newer_than(current))
            .unwrap_or(false)
    }

    /// 从清单生成记录（清单里必须有安装包）。
    pub fn from_manifest(manifest: &Manifest, downloaded_at: String) -> Option<Self> {
        let installer = manifest.installer.as_ref()?;
        Some(Self {
            version: manifest.version.clone(),
            file: installer.file.clone(),
            sha256: installer.sha256.clone(),
            size: installer.size,
            silent_args: installer.silent_args.clone(),
            downloaded_at,
            component_version: manifest.indexer_component().map(|c| c.version.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::sample;

    fn paths() -> (tempfile::TempDir, UpdatePaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = UpdatePaths::new(dir.path().join("Rove"));
        (dir, paths)
    }

    #[test]
    fn the_layout_follows_the_localappdata_convention() {
        let paths = UpdatePaths::from_env(|key| {
            (key == "LOCALAPPDATA").then(|| OsString::from(r"C:\Local"))
        })
        .unwrap();
        assert_eq!(paths.state_root(), Path::new(r"C:\Local\Rove"));
        assert_eq!(paths.updates_dir(), Path::new(r"C:\Local\Rove\updates"));
        assert_eq!(paths.logs_dir(), Path::new(r"C:\Local\Rove\logs"));
        assert_eq!(
            paths.pending_file(),
            Path::new(r"C:\Local\Rove\updates\pending.json")
        );
        // 没有 %LOCALAPPDATA% 就是「更新不可用」，不另找地方写。
        assert!(UpdatePaths::from_env(|_| None).is_none());
        assert!(UpdatePaths::from_env(|_| Some(OsString::new())).is_none());
    }

    #[test]
    fn a_pending_record_round_trips() {
        let (_dir, paths) = paths();
        let manifest = Manifest::parse(sample("0.1.1").as_bytes()).unwrap();
        let pending = Pending::from_manifest(&manifest, "2026-11-14 10:00:00Z".into()).unwrap();
        assert_eq!(pending.component_version.as_deref(), Some("1.2.0.0"));
        paths.write_pending(&pending).unwrap();
        // 安装包还没落地 → 当作没有待安装版本。
        assert_eq!(paths.read_pending(), None);
        fs::write(pending.installer_path(&paths), b"setup").unwrap();
        assert_eq!(paths.read_pending(), Some(pending.clone()));

        assert!(pending.is_newer_than(&Version::parse("0.1.0").unwrap()));
        // 装上之后当前版本就等于待安装版本，不再算「待安装」。
        assert!(!pending.is_newer_than(&Version::parse("0.1.1").unwrap()));
    }

    /// 同时只保留一个待安装版本：新下载前把旧的清掉。
    #[test]
    fn only_one_pending_version_is_kept() {
        let (_dir, paths) = paths();
        paths.ensure_updates_dir().unwrap();
        for name in ["Rove-0.1.1-setup.exe", "Rove-0.1.2-setup.exe", "x.part"] {
            fs::write(paths.updates_dir().join(name), b"x").unwrap();
        }
        paths.clear_downloads(&["Rove-0.1.2-setup.exe"]);
        let left: Vec<String> = fs::read_dir(paths.updates_dir())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["Rove-0.1.2-setup.exe"]);
    }

    #[test]
    fn discarding_removes_the_record_and_the_installer() {
        let (_dir, paths) = paths();
        let manifest = Manifest::parse(sample("0.1.1").as_bytes()).unwrap();
        let pending = Pending::from_manifest(&manifest, String::new()).unwrap();
        paths.write_pending(&pending).unwrap();
        fs::write(pending.installer_path(&paths), b"setup").unwrap();
        paths.discard_pending();
        assert!(!paths.pending_file().exists());
        assert!(!pending.installer_path(&paths).exists());
        assert_eq!(paths.read_pending(), None);
    }

    /// 安装尝试记录：写得下、读得回，丢弃待安装版本时跟着一起消失（issue #235）。
    #[test]
    fn an_install_attempt_is_recorded_and_discarded_with_the_pending_version() {
        let (_dir, paths) = paths();
        let manifest = Manifest::parse(sample("0.1.1").as_bytes()).unwrap();
        let pending = Pending::from_manifest(&manifest, String::new()).unwrap();
        paths.write_pending(&pending).unwrap();
        fs::write(pending.installer_path(&paths), b"setup").unwrap();

        // 上一轮留下的退出码不会混进这一次。
        fs::write(paths.exit_code_file(), b"7").unwrap();
        let attempt = Attempt::new(&pending, "2026-11-17 10:00:00Z".into());
        paths.write_attempt(&attempt).unwrap();
        assert_eq!(paths.read_exit_code(), None);
        assert_eq!(paths.read_attempt(), Some(attempt.clone()));
        assert_eq!(attempt.component_version.as_deref(), Some("1.2.0.0"));
        assert!(attempt.is_newer_than(&Version::parse("0.1.0").unwrap()));
        assert!(!attempt.is_newer_than(&Version::parse("0.1.1").unwrap()));

        fs::write(paths.exit_code_file(), b" 3\r\n").unwrap();
        assert_eq!(paths.read_exit_code(), Some(3));
        fs::write(paths.exit_code_file(), b"not a number").unwrap();
        assert_eq!(paths.read_exit_code(), None);

        // 一次安装请求到此结束：记录、退出码、安装包一起清掉。
        paths.discard_pending();
        assert_eq!(paths.read_attempt(), None);
        assert_eq!(paths.read_exit_code(), None);
        assert_eq!(paths.read_pending(), None);
        assert!(!pending.installer_path(&paths).exists());
    }

    /// 记录写坏了（手工编辑、断电）就当作没有，不让更新流程崩掉。
    #[test]
    fn a_corrupt_record_reads_as_nothing_pending() {
        let (_dir, paths) = paths();
        paths.ensure_updates_dir().unwrap();
        fs::write(paths.pending_file(), b"{ not json").unwrap();
        assert_eq!(paths.read_pending(), None);
        fs::write(paths.attempt_file(), b"{ not json").unwrap();
        assert_eq!(paths.read_attempt(), None);
    }
}
