//! 原子写入（#49）：同目录临时文件 → `ReplaceFileW`（目标不存在时重命名）。
//!
//! 保证：
//!
//! - **不留半截文件**：内容先完整写进同目录的临时文件并 `FlushFileBuffers`，再一次性替换目标；
//!   替换之前任何一步失败（包括进程被强杀），目标文件保持原样。
//! - **不留临时残留**：临时文件创建后立即设置“关闭即删除”的处置（`FileDispositionInfo`），
//!   进程在写入途中被强杀时由系统删除；替换前才撤销这个处置。其他失败路径显式删除临时文件。
//! - **保留属性与时间戳语义**：`ReplaceFileW` 保留原文件的属性（隐藏、系统……）、创建时间与 ACL，
//!   修改时间是这次写入的时间（与 VS Code 相同）。
//! - **失败分类**：被其他程序占用（Unity 正在导入：共享冲突、锁冲突、映射中、无法移除被替换文件）
//!   归为 [`WriteError::Busy`]，并按 [`WriteOptions::busy_retries`] 自动重试；只读属性归为
//!   [`WriteError::ReadOnly`]（[`WriteOptions::clear_read_only`] 为真时先清除再写，即通知里的“覆盖”）；
//!   其余拒绝访问归为 [`WriteError::AccessDenied`]。**不提供提权写入**。
//!
//! 写入的线性化点是替换成功的那一刻；之后发生的任何事情都不会把结果报告成失败（ARCHITECTURE §3.8）。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rove_base::Error;

use super::stamp::{content_hash, DiskStamp};

/// 一次写入的选项。
#[derive(Clone)]
pub struct WriteOptions {
    /// 目标被占用时自动重试的次数（`UX_SPEC` §6.4：自动重试一次，仍失败才通知）。
    pub busy_retries: u32,
    /// 两次尝试之间的等待。
    pub busy_retry_delay: Duration,
    /// 目标有只读属性时先清除（“覆盖”按钮）；为假时报 [`WriteError::ReadOnly`]。
    pub clear_read_only: bool,
    /// 测试用的故障注入点（见 [`FailPoint`]）。
    #[doc(hidden)]
    pub fail_point: Option<FailPoint>,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            busy_retries: 1,
            busy_retry_delay: Duration::from_millis(300),
            clear_read_only: false,
            fail_point: None,
        }
    }
}

/// 故障注入：临时文件已经完整写入并刷盘、还没替换目标时调用。返回 `Err` 模拟写入失败；
/// 测试也可以在这里阻塞，由父进程强杀（验证“强杀后原文件完整、无临时残留”）。
#[doc(hidden)]
#[derive(Clone)]
pub struct FailPoint(pub FailHook);

/// 故障注入回调：参数是临时文件路径。
#[doc(hidden)]
pub type FailHook = Arc<dyn Fn(&Path) -> std::io::Result<()> + Send + Sync>;

/// 写入失败的类别（界面按类别选通知文案与按钮）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// 被其他程序占用；`attempts` 是总共尝试的次数（含自动重试）。
    Busy { attempts: u32 },
    /// 目标文件有只读属性。
    ReadOnly,
    /// 没有写入权限（不是只读属性，也不是占用）。Rove 不提供“以管理员身份保存”。
    AccessDenied,
    /// 目标所在的目录不存在。
    DirectoryMissing,
    /// 其他 I/O 错误（磁盘已满等），保留类别与诊断文字。
    Io(Error),
}

/// 写入成功的结果。
#[derive(Clone, Debug)]
pub struct Written {
    /// 写入后的磁盘标记（长度、修改时间、内容哈希），用于冲突检测与自写事件去重。
    pub stamp: DiskStamp,
    /// 总共尝试的次数（1 表示一次成功）。
    pub attempts: u32,
}

/// 原子地把 `bytes` 写到 `path`。在后台线程调用（会刷盘、可能等待重试）。
pub fn write_atomic(
    path: &Path,
    bytes: &[u8],
    options: &WriteOptions,
) -> Result<Written, WriteError> {
    let parent = parent_dir(path);
    if !parent.is_dir() {
        return Err(WriteError::DirectoryMissing);
    }
    let existing = std::fs::metadata(path).ok();
    if let Some(metadata) = &existing {
        if metadata.permissions().readonly() {
            if !options.clear_read_only {
                return Err(WriteError::ReadOnly);
            }
            clear_read_only(path).map_err(|error| classify(path, error))?;
        }
    }
    let temp = TempFile::create(&parent, path).map_err(|error| classify(path, error))?;
    temp.write(bytes).map_err(|error| classify(path, error))?;
    if let Some(FailPoint(hook)) = &options.fail_point {
        hook(temp.path()).map_err(|error| WriteError::Io(error.into()))?;
    }
    let temp = temp.release().map_err(|error| classify(path, error))?;
    let mut attempts = 0;
    loop {
        attempts += 1;
        // 每次都重新判断：重试期间目标可能被删除或创建。
        let result = if path.exists() {
            replace(path, &temp)
        } else {
            rename(&temp, path)
        };
        match result.map_err(|error| classify(path, error)) {
            Ok(()) => break,
            Err(WriteError::Busy { .. }) if attempts <= options.busy_retries => {
                std::thread::sleep(options.busy_retry_delay);
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                return Err(match error {
                    WriteError::Busy { .. } => WriteError::Busy { attempts },
                    other => other,
                });
            }
        }
    }
    let metadata = std::fs::metadata(path).map_err(|error| WriteError::Io(error.into()));
    let stamp = match metadata {
        Ok(metadata) => DiskStamp::from_metadata(&metadata, content_hash(bytes)),
        // 写入已经成功（线性化点已过），读不到元数据也报告成功。
        Err(_) => DiskStamp {
            len: bytes.len() as u64,
            modified: None,
            hash: content_hash(bytes),
        },
    };
    Ok(Written { stamp, attempts })
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn clear_read_only(path: &Path) -> std::io::Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions)
}

/// 把 I/O 错误归类。Windows 错误码见模块文档。
fn classify(path: &Path, error: std::io::Error) -> WriteError {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::*;
        if let Some(code) = error.raw_os_error() {
            let code = code as u32;
            match code {
                ERROR_SHARING_VIOLATION
                | ERROR_LOCK_VIOLATION
                | ERROR_USER_MAPPED_FILE
                | ERROR_UNABLE_TO_REMOVE_REPLACED
                | ERROR_UNABLE_TO_MOVE_REPLACEMENT => return WriteError::Busy { attempts: 1 },
                ERROR_ACCESS_DENIED => {
                    // 拒绝访问既可能是权限，也可能是对方没有以 FILE_SHARE_DELETE 打开目标：
                    // 用一次普通的写打开探测。
                    return match OpenOptions::new().write(true).open(path) {
                        Err(probe)
                            if probe.raw_os_error().map(|c| c as u32)
                                == Some(ERROR_SHARING_VIOLATION) =>
                        {
                            WriteError::Busy { attempts: 1 }
                        }
                        _ if std::fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) => {
                            WriteError::ReadOnly
                        }
                        _ => WriteError::AccessDenied,
                    };
                }
                _ => {}
            }
        }
    }
    let _ = path;
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => WriteError::AccessDenied,
        std::io::ErrorKind::NotFound => WriteError::DirectoryMissing,
        _ => WriteError::Io(error.into()),
    }
}

/// 同目录的临时文件：`<文件名>.~rove<进程号>-<序号>.tmp`。
struct TempFile {
    file: File,
    path: PathBuf,
    /// 只在非 Windows 平台读取（Windows 由“关闭即删除”处理）。
    #[cfg_attr(windows, allow(dead_code))]
    released: bool,
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

impl TempFile {
    fn create(dir: &Path, target: &Path) -> std::io::Result<Self> {
        let name = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        loop {
            let path = dir.join(format!(
                ".{name}.~rove{}-{}.tmp",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
                use windows_sys::Win32::Storage::FileSystem::DELETE;
                options
                    .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
                    .share_mode(0);
            }
            match options.open(&path) {
                Ok(file) => {
                    let temp = TempFile {
                        file,
                        path,
                        released: false,
                    };
                    if let Err(error) = temp.set_delete_on_close(true) {
                        let path = temp.path.clone();
                        drop(temp);
                        let _ = std::fs::remove_file(path);
                        return Err(error);
                    }
                    return Ok(temp);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut file = &self.file;
        file.write_all(bytes)?;
        file.sync_all()
    }

    /// 撤销“关闭即删除”并关闭句柄，交出路径（之后由调用方负责替换或删除）。
    fn release(mut self) -> std::io::Result<PathBuf> {
        // 失败时 self 被丢弃，句柄关闭，系统按“关闭即删除”删掉它。
        self.set_delete_on_close(false)?;
        self.released = true;
        Ok(self.path.clone())
    }

    #[cfg(windows)]
    fn set_delete_on_close(&self, delete: bool) -> std::io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
        };
        let info = FILE_DISPOSITION_INFO { DeleteFile: delete };
        // SAFETY: 句柄在 self.file 的生命周期内有效；结构体大小与类别匹配。
        let ok = unsafe {
            SetFileInformationByHandle(
                self.file.as_raw_handle(),
                FileDispositionInfo,
                &info as *const FILE_DISPOSITION_INFO as *const core::ffi::c_void,
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(not(windows))]
    fn set_delete_on_close(&self, _: bool) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        // 没有被 release 的临时文件（写入失败、故障注入）：Windows 上句柄关闭即删除，
        // 其他平台显式删除。
        #[cfg(not(windows))]
        if !self.released {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn replace(target: &Path, replacement: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{ReplaceFileW, REPLACEFILE_IGNORE_MERGE_ERRORS};
    let target = wide(target);
    let replacement = wide(replacement);
    // SAFETY: 两个路径都是以 NUL 结尾的 UTF-16；不使用备份文件与保留参数。
    let ok = unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_IGNORE_MERGE_ERRORS,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace(target: &Path, replacement: &Path) -> std::io::Result<()> {
    std::fs::rename(replacement, target)
}

fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temps(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn writes_new_and_existing_files_without_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.cs");
        let written = write_atomic(&path, b"one", &WriteOptions::default()).unwrap();
        assert_eq!(written.attempts, 1);
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        write_atomic(&path, b"two", &WriteOptions::default()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert!(temps(dir.path()).is_empty());
        assert_eq!(written.stamp.hash, content_hash(b"one"));
    }

    #[test]
    fn failure_before_replace_keeps_original_and_removes_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.cs");
        std::fs::write(&path, b"original").unwrap();
        let options = WriteOptions {
            fail_point: Some(FailPoint(Arc::new(|_| {
                Err(std::io::Error::other("injected"))
            }))),
            ..WriteOptions::default()
        };
        assert!(matches!(
            write_atomic(&path, b"new content", &options),
            Err(WriteError::Io(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert!(temps(dir.path()).is_empty());
    }

    #[test]
    fn read_only_attribute_is_reported_and_cleared_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.meta");
        std::fs::write(&path, b"old").unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert_eq!(
            write_atomic(&path, b"new", &WriteOptions::default()).unwrap_err(),
            WriteError::ReadOnly
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        let options = WriteOptions {
            clear_read_only: true,
            ..WriteOptions::default()
        };
        write_atomic(&path, b"new", &options).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(!std::fs::metadata(&path).unwrap().permissions().readonly());
    }

    #[test]
    fn missing_directory_is_its_own_category() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("a.cs");
        assert_eq!(
            write_atomic(&path, b"x", &WriteOptions::default()).unwrap_err(),
            WriteError::DirectoryMissing
        );
    }

    #[cfg(windows)]
    #[test]
    fn hidden_attribute_survives_replace() {
        use std::os::windows::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hidden.txt");
        std::fs::write(&path, b"old").unwrap();
        let status = std::process::Command::new("attrib")
            .arg("+h")
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        write_atomic(&path, b"new", &WriteOptions::default()).unwrap();
        const HIDDEN: u32 = 2;
        assert_ne!(
            std::fs::metadata(&path).unwrap().file_attributes() & HIDDEN,
            0
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }
}
