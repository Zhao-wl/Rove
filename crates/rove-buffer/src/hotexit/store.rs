//! 备份目录的布局与读写（#50）。
//!
//! ```text
//! %LOCALAPPDATA%\Rove\
//!   backups\
//!     <文件夹名>-<16 位哈希>\        每个工作区（= 每个窗口）一组
//!       session.rvbs                 工作区路径、上次是否正常退出、连续恢复失败次数
//!       <文件名>-<16 位哈希>.rvbak    每个脏文档一份（见 [`super::record`]）
//!   backups-failed\
//!     <同名>-<时间戳>\               连续两次恢复失败的整组备份搬到这里，不再尝试
//!   logs\                            崩溃日志（见 [`super::crash`]）
//! ```
//!
//! 目录名与 #36 的 `%LOCALAPPDATA%\Rove\index\<项目路径哈希>\` 是同一套约定：路径先规范化
//! （去掉 `\\?\`、统一反斜杠、去掉末尾分隔符、Windows 上不区分大小写），再取 FNV-1a-64，
//! 前面带上过滤后的文件夹名。两边的实现各自独立（核心层 crate 之间不互相依赖），
//! `layout_matches_index_convention` 用同样的输入钉住结果。
//!
//! 所有写入都走 #49 的原子写入（同目录临时文件 → `ReplaceFileW`），所以强杀进程时不会留下
//! 半截备份：要么是上一份完整的备份，要么是新的完整备份。
//!
//! **读不到就当没有**：备份目录可能被杀毒软件锁住、被清理工具删掉，或者 `%LOCALAPPDATA%`
//! 根本没设置。所有读取路径在失败时都退化成“这个工作区没有备份”，绝不让启动失败。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::codec::{DecodeError, DecodeResult, Reader, Writer};
use super::record::Backup;
use crate::save::{write_atomic, WriteOptions};

pub const BACKUPS_DIR: &str = "backups";
pub const FAILED_DIR: &str = "backups-failed";
pub const SESSION_FILE: &str = "session.rvbs";
pub const BACKUP_EXT: &str = ".rvbak";

const SESSION_MAGIC: &[u8] = b"ROVEBSE\x01";
const SESSION_VERSION: u32 = 1;

/// 连续恢复失败多少次之后把整组备份隔离到 `backups-failed\`（第二次仍失败就隔离）。
pub const MAX_RESTORE_ATTEMPTS: u32 = 2;

/// 工作区组的会话状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// 这组备份属于哪个工作区（目录已不存在时整组清理）。
    pub workspace: PathBuf,
    /// 上次是正常退出（有干净的退出标记）。读不到文件时为 `false`：没有标记就按意外退出处理。
    pub clean_exit: bool,
    /// 已经尝试恢复这组备份多少次（恢复成功后清零）。
    pub attempts: u32,
}

impl Session {
    pub fn new(workspace: &Path) -> Self {
        Self {
            workspace: workspace.to_owned(),
            clean_exit: false,
            attempts: 0,
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_magic(SESSION_MAGIC, SESSION_VERSION);
        w.path(&self.workspace);
        w.bool(self.clean_exit);
        w.u32(self.attempts);
        w.into_bytes()
    }

    fn decode(bytes: &[u8]) -> DecodeResult<Self> {
        let mut r = Reader::with_magic(bytes, SESSION_MAGIC, SESSION_VERSION)?;
        let session = Self {
            workspace: r.path()?,
            clean_exit: r.bool()?,
            attempts: r.u32()?,
        };
        if !r.at_end() {
            return Err(DecodeError);
        }
        Ok(session)
    }
}

/// `%LOCALAPPDATA%\Rove`；取不到环境变量时返回 `None`（不回退到当前目录，绝不把状态写进项目）。
pub fn default_state_root(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    env("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(|dir| PathBuf::from(dir).join("Rove"))
}

/// 路径规范化（与 #36 的 `project_key` 同一套规则）。
pub(super) fn normalize_path(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
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
    text
}

/// 工作区目录名：`<小写文件夹名>-<16 位哈希>`（与 #36 的 `project_key` 同一套约定）。
pub fn workspace_key(workspace: &Path) -> String {
    let name: String = workspace
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .map(|c| c.to_ascii_lowercase())
        .take(32)
        .collect();
    let hash = crate::save::content_hash(normalize_path(workspace).as_bytes());
    if name.is_empty() {
        format!("{hash:016x}")
    } else {
        format!("{name}-{hash:016x}")
    }
}

/// 一个工作区（= 一个窗口）的备份组。
#[derive(Clone)]
pub struct BackupStore {
    /// 状态根目录（`%LOCALAPPDATA%\Rove`）。
    state_root: PathBuf,
    workspace: PathBuf,
    dir: PathBuf,
    key: String,
    write: WriteOptions,
}

/// `WriteOptions` 里有不可打印的故障注入点，所以只打印目录与工作区。
impl std::fmt::Debug for BackupStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackupStore")
            .field("dir", &self.dir)
            .field("workspace", &self.workspace)
            .finish()
    }
}

impl BackupStore {
    /// `state_root` 一般是 [`default_state_root`] 的结果；测试传临时目录。
    pub fn new(state_root: &Path, workspace: &Path) -> Self {
        let key = workspace_key(workspace);
        Self {
            dir: state_root.join(BACKUPS_DIR).join(&key),
            state_root: state_root.to_owned(),
            workspace: workspace.to_owned(),
            key,
            write: WriteOptions::default(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn session_path(&self) -> PathBuf {
        self.dir.join(SESSION_FILE)
    }

    pub fn backup_path(&self, document_path: &Path) -> PathBuf {
        self.dir.join(Backup::file_name(document_path))
    }

    /// 读会话状态。文件不存在或损坏时按“意外退出、没有失败记录”处理。
    pub fn session(&self) -> Session {
        std::fs::read(self.session_path())
            .ok()
            .and_then(|bytes| Session::decode(&bytes).ok())
            .filter(|session| session.workspace == self.workspace)
            .unwrap_or_else(|| Session::new(&self.workspace))
    }

    pub fn write_session(&self, session: &Session) -> std::io::Result<()> {
        self.write_file(&self.session_path(), &session.encode())
    }

    /// 写一份文档备份（原子写入）。返回写出的字节数（I/O 统计用）。
    pub fn write_backup(&self, backup: &Backup) -> std::io::Result<u64> {
        let bytes = backup.encode();
        self.write_file(&self.backup_path(&backup.path), &bytes)?;
        Ok(bytes.len() as u64)
    }

    /// 删除一份文档备份（文档被保存或关闭、恢复消费完）。不存在时成功。
    pub fn remove_backup(&self, document_path: &Path) -> std::io::Result<()> {
        remove_file(&self.backup_path(document_path))
    }

    /// 读出本组全部备份，按标签顺序排列。读不出来的单份跳过（按“没有这份备份”处理）。
    pub fn backups(&self) -> Vec<Backup> {
        let mut found: Vec<Backup> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(BACKUP_EXT))
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .filter_map(|bytes| Backup::decode(&bytes).ok())
            .collect();
        found.sort_by_key(|backup| (backup.tab_index, backup.path.clone()));
        found
    }

    /// 本组是否还有备份文件（没有时不必保留目录）。
    pub fn has_backups(&self) -> bool {
        std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().ends_with(BACKUP_EXT))
    }

    /// 整组删除（`files.hotExit = off`、工作区消失、恢复完且没有新的脏文档）。
    pub fn remove_group(&self) -> std::io::Result<()> {
        match std::fs::remove_dir_all(&self.dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    /// 连续恢复失败：整组搬到 `backups-failed\<组名>-<编号>\`，下次启动按“没有备份”处理。
    /// 返回隔离后的目录。
    pub fn quarantine(&self) -> std::io::Result<PathBuf> {
        let base = self.state_root.join(FAILED_DIR);
        std::fs::create_dir_all(&base)?;
        // 编号只是为了不覆盖上一次隔离的内容，不参与任何判定。
        let mut target = base.join(&self.key);
        let mut n = 1;
        while target.exists() {
            target = base.join(format!("{}-{n}", self.key));
            n += 1;
        }
        std::fs::rename(&self.dir, &target)?;
        Ok(target)
    }

    /// 清理整个 `backups\` 下工作区已不存在的组（启动时做一次，很便宜：每组只读一个小文件）。
    /// 返回清掉的组数。只碰本模块生成的目录名，自定义内容不动。
    pub fn prune_missing_workspaces(state_root: &Path) -> usize {
        let root = state_root.join(BACKUPS_DIR);
        let mut removed = 0;
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let dir = entry.path();
            if !entry.file_type().is_ok_and(|t| t.is_dir())
                || !is_workspace_key(&entry.file_name().to_string_lossy())
            {
                continue;
            }
            let Some(session) = std::fs::read(dir.join(SESSION_FILE))
                .ok()
                .and_then(|bytes| Session::decode(&bytes).ok())
            else {
                continue;
            };
            if !session.workspace.is_dir() && std::fs::remove_dir_all(&dir).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(path, bytes, &self.write)
            .map(|_| ())
            .map_err(|error| match error {
                crate::save::WriteError::Io(error) => std::io::Error::other(error.to_string()),
                other => std::io::Error::other(format!("{other:?}")),
            })
    }
}

/// 目录名是不是本模块生成的工作区目录（与 #36 的 `is_project_key` 同一套判定）。
pub fn is_workspace_key(name: &str) -> bool {
    let hash = name.rsplit('-').next().unwrap_or(name);
    hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

fn remove_file(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_index_convention() {
        let key = workspace_key(Path::new(r"Z:\No\Such\Game"));
        assert_eq!(key, workspace_key(Path::new(r"z:/no/such/game/")));
        assert!(key.starts_with("game-"), "{key}");
        assert!(is_workspace_key(&key));
        assert!(!is_workspace_key("Unrelated"));
        assert_ne!(key, workspace_key(Path::new(r"Z:\No\Such\Other")));

        let env = |k: &str| (k == "LOCALAPPDATA").then(|| OsString::from(r"C:\Local"));
        let root = default_state_root(env).unwrap();
        assert_eq!(root, PathBuf::from(r"C:\Local\Rove"));
        let store = BackupStore::new(&root, Path::new(r"Z:\No\Such\Game"));
        assert_eq!(store.dir(), root.join("backups").join(&key));
        assert_eq!(store.session_path(), store.dir().join("session.rvbs"));
        assert!(default_state_root(|_| None).is_none());
        assert!(default_state_root(|_| Some(OsString::new())).is_none());
    }

    #[test]
    fn session_defaults_to_crash_when_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = BackupStore::new(dir.path(), &workspace);
        // 没有文件：按意外退出处理。
        assert!(!store.session().clean_exit);
        assert_eq!(store.session().attempts, 0);

        let mut session = Session::new(&workspace);
        session.clean_exit = true;
        session.attempts = 1;
        store.write_session(&session).unwrap();
        assert_eq!(store.session(), session);

        // 损坏的文件：同样按意外退出处理，不报错。
        std::fs::write(store.session_path(), b"garbage").unwrap();
        assert!(!store.session().clean_exit);
        // 记录的工作区对不上（目录被别的工作区复用）：丢弃。
        let other = Session {
            workspace: PathBuf::from(r"Z:\Elsewhere"),
            clean_exit: true,
            attempts: 7,
        };
        store
            .write_file(&store.session_path(), &other.encode())
            .unwrap();
        assert_eq!(store.session(), Session::new(&workspace));
    }

    #[test]
    fn prune_removes_only_groups_whose_workspace_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let alive = dir.path().join("alive");
        let gone = dir.path().join("gone");
        std::fs::create_dir_all(&alive).unwrap();
        std::fs::create_dir_all(&gone).unwrap();
        let alive_store = BackupStore::new(dir.path(), &alive);
        let gone_store = BackupStore::new(dir.path(), &gone);
        alive_store.write_session(&Session::new(&alive)).unwrap();
        gone_store.write_session(&Session::new(&gone)).unwrap();
        // 不是本模块生成的目录名：不碰。
        let foreign = dir.path().join(BACKUPS_DIR).join("README");
        std::fs::create_dir_all(&foreign).unwrap();

        std::fs::remove_dir_all(&gone).unwrap();
        assert_eq!(BackupStore::prune_missing_workspaces(dir.path()), 1);
        assert!(alive_store.dir().is_dir());
        assert!(!gone_store.dir().exists());
        assert!(foreign.is_dir());
        // 再跑一次不报错也不多删。
        assert_eq!(BackupStore::prune_missing_workspaces(dir.path()), 0);
    }

    #[test]
    fn quarantine_moves_the_group_aside_without_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let store = BackupStore::new(dir.path(), &workspace);
        store.write_session(&Session::new(&workspace)).unwrap();
        let first = store.quarantine().unwrap();
        assert!(first.join(SESSION_FILE).is_file());
        assert!(!store.dir().exists());
        store.write_session(&Session::new(&workspace)).unwrap();
        let second = store.quarantine().unwrap();
        assert_ne!(first, second);
        assert!(first.join(SESSION_FILE).is_file());
        assert!(second.join(SESSION_FILE).is_file());
    }
}
