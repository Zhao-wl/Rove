//! 「重启以更新」：退出 → 静默安装 → 重新启动（issue #149 第 3 节）。
//!
//! 顺序（界面那一侧负责前两步，见 `rove-ui` 的 `update_host`）：
//!
//! 1. 走 #50 的热退出保存会话与未保存的修改；
//! 2. 退出窗口；
//! 3. **本模块**：拉起下载好的安装包，参数用清单里的 `installer.silent_args`
//!    （与 #76 对齐，不在代码里写死），装完再把 `rove.exe` 拉起来；
//! 4. 新进程启动时 #50 恢复会话。
//!
//! 全程**不弹 UAC**：安装器是用户级的（`PrivilegesRequired=lowest`，装到
//! `%LOCALAPPDATA%\Programs\Rove`）。
//!
//! 「装完再启动」靠一行 `cmd /C "<安装器> <参数> & start "" "<rove.exe>""`：
//! 安装器的 `RestartApplications=no`（#76）把「要不要重新启动」留给了更新端，
//! 而 Rove 自己马上就要退出，没法等安装器结束——所以让 `cmd` 等它。
//!
//! 便携版不走这条路：它只提示有新版本并打开发布页（[`is_portable`]）。

use crate::{paths::Pending, Result, UpdateError};
use std::path::{Path, PathBuf};

/// 安装器里装 Rove 的目录（#76 的 `DefaultDirName={localappdata}\Programs\Rove`）。
const INSTALL_DIR_TAIL: [&str; 2] = ["Programs", "Rove"];

/// 当前这份 Rove 是不是便携版（= 不是安装器装出来的）。
///
/// 判定只看一件事：`rove.exe` 在不在 `%LOCALAPPDATA%\Programs\Rove` 下面。
/// 便携版「解压即用」，放在哪里都行，所以**只要不在安装目录里就按便携版对待**——
/// 宁可少装一次（提示用户自己去下载），也不要把文件覆盖到用户自己摆放的目录里。
pub fn is_portable(exe: &Path, local_app_data: Option<&Path>) -> bool {
    let Some(local) = local_app_data else {
        return true;
    };
    let mut expected = local.to_path_buf();
    for part in INSTALL_DIR_TAIL {
        expected.push(part);
    }
    let Some(dir) = exe.parent() else {
        return true;
    };
    // 大小写不敏感地比（Windows 路径）。
    !dir.to_string_lossy()
        .to_lowercase()
        .starts_with(&expected.to_string_lossy().to_lowercase())
}

/// 要执行的那一行命令（纯函数，便于逐字检查）。
///
/// `cmd /C` 的参数解析规则和普通 argv 不一样，所以这里直接拼出完整的命令行，
/// 由调用方用 `raw_arg` 原样交给 `cmd.exe`。
pub fn command_line(installer: &Path, silent_args: &[String], exe: &Path) -> String {
    let quoted = |path: &Path| format!("\"{}\"", path.display());
    let mut line = String::from("/D /C \"");
    line.push_str(&quoted(installer));
    for arg in silent_args {
        line.push(' ');
        // 静默参数来自我们自己生成并签名的清单，仍然挡一手：引号会破坏命令行。
        line.push_str(&arg.replace('"', ""));
    }
    // `&` 而不是 `&&`：安装器返回非 0 时也要把 Rove 拉回来，不能把用户卡在没有编辑器的状态。
    line.push_str(" & start \"\" ");
    line.push_str(&quoted(exe));
    line.push('"');
    line
}

/// 拉起安装器并安排装完重启。调用方应当在**窗口已经关掉、会话已经存好**之后调用它，
/// 然后立刻退出进程（安装器要等 `rove.exe` 退出才能替换文件）。
pub fn launch(pending: &Pending, installer: &Path, exe: &Path) -> Result<()> {
    if !installer.is_file() {
        return Err(UpdateError::Install(format!(
            "{} 不在了",
            installer.display()
        )));
    }
    if pending.silent_args.is_empty() {
        return Err(UpdateError::Install("清单里没有静默安装参数".into()));
    }
    spawn(&command_line(installer, &pending.silent_args, exe))
}

#[cfg(windows)]
fn spawn(command_line: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW：别在用户眼前闪一个黑框。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new("cmd.exe")
        .raw_arg(command_line)
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map_err(|e| UpdateError::Install(format!("拉起安装器失败：{e}")))?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn(_command_line: &str) -> Result<()> {
    Err(UpdateError::Unavailable("只有 Windows 上有安装器".into()))
}

/// 重启之后的判定（issue #149 第 5 节：「重启后版本号没变」要通知并保留旧版本）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterRestart {
    /// 装上了：当前版本已经等于（或高于）待安装版本。
    Installed,
    /// 版本号没变：安装没成功，告诉用户，并把下载的文件删掉。
    Failed,
}

/// 启动时拿「待安装记录」和当前版本对一下。
pub fn after_restart(pending: &Pending, current: &crate::Version) -> AfterRestart {
    if pending.is_newer_than(current) {
        // 还是比当前新 → 要么还没重启（正常，继续提示），要么装失败了。
        // 区分这两种情况靠调用方：只有「这次启动是更新后的那一次」才判失败。
        AfterRestart::Failed
    } else {
        AfterRestart::Installed
    }
}

/// 默认的安装目录（诊断与日志用）。
pub fn install_dir(local_app_data: &Path) -> PathBuf {
    let mut dir = local_app_data.to_path_buf();
    for part in INSTALL_DIR_TAIL {
        dir.push(part);
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(args: &[&str]) -> Pending {
        Pending {
            version: "0.1.1".into(),
            file: "Rove-0.1.1-win-x64-setup.exe".into(),
            sha256: "0".repeat(64),
            size: 1,
            silent_args: args.iter().map(|s| s.to_string()).collect(),
            downloaded_at: String::new(),
            component_version: None,
        }
    }

    /// 静默参数逐字来自清单（#76 的那一组），而且装完会把 Rove 拉回来。
    #[test]
    fn the_command_line_runs_the_installer_then_restarts_rove() {
        let line = command_line(
            Path::new(r"C:\Local\Rove\updates\Rove-0.1.1-win-x64-setup.exe"),
            &[
                "/VERYSILENT".into(),
                "/SUPPRESSMSGBOXES".into(),
                "/NORESTART".into(),
                "/NOCANCEL".into(),
                "/SP-".into(),
            ],
            Path::new(r"C:\Local\Programs\Rove\rove.exe"),
        );
        assert_eq!(
            line,
            r#"/D /C ""C:\Local\Rove\updates\Rove-0.1.1-win-x64-setup.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /SP- & start "" "C:\Local\Programs\Rove\rove.exe"""#
        );
        // 路径带空格时仍然是一个整体。
        let spaced = command_line(
            Path::new(r"C:\Users\张 三\setup.exe"),
            &["/VERYSILENT".into()],
            Path::new(r"C:\Program Files\rove.exe"),
        );
        assert!(spaced.contains(r#""C:\Users\张 三\setup.exe""#));
        assert!(spaced.contains(r#"start "" "C:\Program Files\rove.exe""#));
    }

    /// 清单里混进引号也不会把命令行拆开。
    #[test]
    fn quotes_in_arguments_are_stripped() {
        let line = command_line(
            Path::new("s.exe"),
            &[r#"/X" & calc & ""#.into()],
            Path::new("rove.exe"),
        );
        assert!(!line.contains(r#"" & calc"#), "{line}");
        assert_eq!(line.matches("start \"\"").count(), 1);
    }

    #[test]
    fn launching_without_an_installer_or_arguments_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.exe");
        let error =
            launch(&pending(&["/VERYSILENT"]), &missing, Path::new("rove.exe")).unwrap_err();
        assert!(matches!(error, UpdateError::Install(_)), "{error:?}");

        let installer = dir.path().join("setup.exe");
        std::fs::write(&installer, b"x").unwrap();
        let error = launch(&pending(&[]), &installer, Path::new("rove.exe")).unwrap_err();
        assert_eq!(error, UpdateError::Install("清单里没有静默安装参数".into()));
    }

    /// 便携版不自己替换文件（#149「不包含」的最后一条）。
    #[test]
    fn a_portable_build_is_detected_by_its_location() {
        let local = Path::new(r"C:\Users\z\AppData\Local");
        assert!(!is_portable(
            Path::new(r"C:\Users\z\AppData\Local\Programs\Rove\rove.exe"),
            Some(local)
        ));
        // 大小写不影响判定。
        assert!(!is_portable(
            Path::new(r"c:\users\z\appdata\local\programs\rove\rove.exe"),
            Some(local)
        ));
        assert!(is_portable(
            Path::new(r"D:\Tools\Rove\rove.exe"),
            Some(local)
        ));
        assert!(is_portable(Path::new(r"D:\rove.exe"), None));
        assert_eq!(
            install_dir(local),
            Path::new(r"C:\Users\z\AppData\Local\Programs\Rove")
        );
    }

    /// 重启后版本号没变 = 安装失败（第 5 节）。
    #[test]
    fn a_version_that_did_not_change_counts_as_a_failed_install() {
        let pending = pending(&["/VERYSILENT"]);
        assert_eq!(
            after_restart(&pending, &crate::Version::parse("0.1.0").unwrap()),
            AfterRestart::Failed
        );
        assert_eq!(
            after_restart(&pending, &crate::Version::parse("0.1.1").unwrap()),
            AfterRestart::Installed
        );
        // 用户手动装了更新的版本也算装上了。
        assert_eq!(
            after_restart(&pending, &crate::Version::parse("0.2.0").unwrap()),
            AfterRestart::Installed
        );
    }
}
