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
//! 「装完再启动」靠一行 `cmd /C "<安装器> <参数> & <记下退出码> & start "" "<rove.exe>""`：
//! 安装器的 `RestartApplications=no`（#76）把「要不要重新启动」留给了更新端，
//! 而 Rove 自己马上就要退出，没法等安装器结束——所以让 `cmd` 等它。
//!
//! **拉起安装器成功 ≠ 装成功**（issue #235）。Rove 这时已经在退出，拿不到安装器的退出码，
//! 所以这条命令行多做两件事，让下一次启动能报告结果并结束这次安装请求：
//!
//! 1. 拉起安装器**之前**，由 [`prepare`] 写下 [`crate::paths::Attempt`]（「为 x.y.z 拉起过安装器」）；
//! 2. 安装器退出之后、把 Rove 拉回来**之前**，`cmd` 把退出码写进
//!    `updates\install-exit-code.txt`（见 [`EXIT_CODE_SNIPPET`]）。
//!
//! 下次启动由 [`crate::Updater::after_start`] 对照「有没有尝试记录」与「版本号变没变」判定：
//! 没有尝试记录 = 用户只是点了稍后，待安装记录照旧有效；有尝试记录而版本号没变 = 这次装失败了，
//! 报告失败并把记录与下载一起清掉，用户可以明确重试。
//!
//! 便携版不走这条路：它只提示有新版本并打开发布页（[`is_portable`]）。

use crate::{
    log::timestamp,
    paths::{Attempt, Pending, UpdatePaths},
    Result, UpdateError,
};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

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

/// 把安装器的退出码写进文件的那一小段 `cmd` 语句。
///
/// `%^ERRORLEVEL%` 里的 `^` 是必须的：`cmd` 解析整条命令行时就会展开一次 `%ERRORLEVEL%`，
/// 那时安装器还没跑。加了 `^` 第一次展开认不出这个变量名，`call` 让 `cmd` 再解析一次，
/// 这时拿到的才是安装器真正的退出码。
pub const EXIT_CODE_SNIPPET: &str = "(call echo %^ERRORLEVEL%)";

/// 安装器那一段：跑安装器，再把退出码写下来（不含「把 Rove 拉回来」）。
///
/// 独立出来是为了能**用安装器替身真的跑一遍**而不拉起任何窗口（见本模块的测试）。
pub fn installer_step(installer: &Path, silent_args: &[String], exit_code_file: &Path) -> String {
    let quoted = |path: &Path| format!("\"{}\"", path.display());
    let mut line = quoted(installer);
    for arg in silent_args {
        line.push(' ');
        // 静默参数来自我们自己生成并签名的清单，仍然挡一手：引号会破坏命令行。
        line.push_str(&arg.replace('"', ""));
    }
    // `&` 而不是 `&&`：退出码非 0 时同样要记下来（issue #235）。
    line.push_str(" & ");
    line.push_str(EXIT_CODE_SNIPPET);
    line.push_str("> ");
    line.push_str(&quoted(exit_code_file));
    line
}

/// 要执行的那一行命令（纯函数，便于逐字检查）。
///
/// `cmd /C` 的参数解析规则和普通 argv 不一样，所以这里直接拼出完整的命令行，
/// 由调用方用 `raw_arg` 原样交给 `cmd.exe`。
pub fn command_line(
    installer: &Path,
    silent_args: &[String],
    exit_code_file: &Path,
    exe: &Path,
) -> String {
    let mut line = String::from("/D /C \"");
    line.push_str(&installer_step(installer, silent_args, exit_code_file));
    // `&` 而不是 `&&`：安装器返回非 0 时也要把 Rove 拉回来，不能把用户卡在没有编辑器的状态。
    line.push_str(" & start \"\" ");
    line.push_str(&format!("\"{}\"", exe.display()));
    line.push('"');
    line
}

/// 拉起安装器之前的准备：检查安装包与参数，写下安装尝试记录，返回要执行的命令行。
///
/// **写不下尝试记录就不装**：装失败了却没有记录，下次启动会按原记录再提示装同一个包，
/// 正是 issue #235 要消掉的那个循环。
pub fn prepare(paths: &UpdatePaths, pending: &Pending, exe: &Path) -> Result<String> {
    let installer = pending.installer_path(paths);
    if !installer.is_file() {
        return Err(UpdateError::Install(format!(
            "{} 不在了",
            installer.display()
        )));
    }
    if pending.silent_args.is_empty() {
        return Err(UpdateError::Install("清单里没有静默安装参数".into()));
    }
    paths.write_attempt(&Attempt::new(pending, timestamp(SystemTime::now())))?;
    Ok(command_line(
        &installer,
        &pending.silent_args,
        &paths.exit_code_file(),
        exe,
    ))
}

/// 拉起安装器并安排装完重启。调用方应当在**窗口已经关掉、会话已经存好**之后调用它，
/// 然后立刻退出进程（安装器要等 `rove.exe` 退出才能替换文件）。
pub fn launch(paths: &UpdatePaths, pending: &Pending, exe: &Path) -> Result<()> {
    spawn(&prepare(paths, pending, exe)?)
}

/// 安装失败的原因（给日志、通知与「关于」对话框看的短句）。
pub fn failure_reason(exit_code: Option<i32>) -> String {
    match exit_code {
        Some(code) => format!("安装器退出码 {code}"),
        None => "安装器没有留下退出码，重启后版本号也没有变".to_string(),
    }
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

    /// 静默参数逐字来自清单（#76 的那一组），退出码会被记下来，最后把 Rove 拉回来。
    #[test]
    fn the_command_line_runs_the_installer_records_the_exit_code_then_restarts_rove() {
        let line = command_line(
            Path::new(r"C:\Local\Rove\updates\Rove-0.1.1-win-x64-setup.exe"),
            &[
                "/VERYSILENT".into(),
                "/SUPPRESSMSGBOXES".into(),
                "/NORESTART".into(),
                "/NOCANCEL".into(),
                "/SP-".into(),
            ],
            Path::new(r"C:\Local\Rove\updates\install-exit-code.txt"),
            Path::new(r"C:\Local\Programs\Rove\rove.exe"),
        );
        assert_eq!(
            line,
            r#"/D /C ""C:\Local\Rove\updates\Rove-0.1.1-win-x64-setup.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /SP- & (call echo %^ERRORLEVEL%)> "C:\Local\Rove\updates\install-exit-code.txt" & start "" "C:\Local\Programs\Rove\rove.exe"""#
        );
        // 路径带空格时仍然是一个整体。
        let spaced = command_line(
            Path::new(r"C:\Users\张 三\setup.exe"),
            &["/VERYSILENT".into()],
            Path::new(r"C:\Users\张 三\install-exit-code.txt"),
            Path::new(r"C:\Program Files\rove.exe"),
        );
        assert!(spaced.contains(r#""C:\Users\张 三\setup.exe""#));
        assert!(spaced.contains(r#"> "C:\Users\张 三\install-exit-code.txt""#));
        assert!(spaced.contains(r#"start "" "C:\Program Files\rove.exe""#));
    }

    /// 清单里混进引号也不会把命令行拆开。
    #[test]
    fn quotes_in_arguments_are_stripped() {
        let line = command_line(
            Path::new("s.exe"),
            &[r#"/X" & calc & ""#.into()],
            Path::new("code.txt"),
            Path::new("rove.exe"),
        );
        assert!(!line.contains(r#"" & calc"#), "{line}");
        assert_eq!(line.matches("start \"\"").count(), 1);
    }

    #[test]
    fn launching_without_an_installer_or_arguments_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let paths = UpdatePaths::new(dir.path().join("Rove"));
        paths.ensure_updates_dir().unwrap();
        let error = prepare(&paths, &pending(&["/VERYSILENT"]), Path::new("rove.exe")).unwrap_err();
        assert!(matches!(error, UpdateError::Install(_)), "{error:?}");
        // 没装成的那一步不留尝试记录：下次启动照旧提示这个待安装版本。
        assert_eq!(paths.read_attempt(), None);

        let empty = pending(&[]);
        std::fs::write(empty.installer_path(&paths), b"x").unwrap();
        let error = prepare(&paths, &empty, Path::new("rove.exe")).unwrap_err();
        assert_eq!(error, UpdateError::Install("清单里没有静默安装参数".into()));
        assert_eq!(paths.read_attempt(), None);
    }

    /// 验收（#235）：**用安装器替身返回非零**，退出码真的被记了下来。
    ///
    /// 这里只跑命令行里「安装器 + 记退出码」那一段：`start "" "<rove.exe>"` 会真的
    /// 拉起一个进程，测试里不碰它（它由 [`command_line`] 的逐字断言覆盖）。
    #[cfg(windows)]
    #[test]
    fn a_stub_installer_that_fails_leaves_its_exit_code_behind() {
        use std::os::windows::process::CommandExt;

        let dir = tempfile::tempdir().unwrap();
        let paths = UpdatePaths::new(dir.path().join("Rove"));
        let updates = paths.ensure_updates_dir().unwrap();
        let mut pending = pending(&["/VERYSILENT"]);
        pending.file = "stub-setup.cmd".into();
        let installer = updates.join(&pending.file);
        std::fs::write(&installer, "@echo off\r\nexit /b 3\r\n").unwrap();

        // prepare 写下尝试记录并给出命令行。
        let line = prepare(&paths, &pending, Path::new(r"C:\nowhere\rove.exe")).unwrap();
        let attempt = paths.read_attempt().expect("拉起安装器之前要写下尝试记录");
        assert_eq!(attempt.version, "0.1.1");
        assert_eq!(attempt.file, "stub-setup.cmd");
        assert!(line.contains(r#"start "" "C:\nowhere\rove.exe""#));

        // 命令行里的安装器那一段照原样交给 cmd 跑一遍。
        let step = installer_step(&installer, &pending.silent_args, &paths.exit_code_file());
        assert!(line.contains(&step), "命令行里跑的不是同一段：{line}");
        let status = std::process::Command::new("cmd.exe")
            .raw_arg(format!("/D /C \"{step}\""))
            .creation_flags(0x0800_0000)
            .status()
            .unwrap();
        assert!(status.success(), "记退出码那一步自己不该失败");
        assert_eq!(paths.read_exit_code(), Some(3));
        assert_eq!(failure_reason(Some(3)), "安装器退出码 3");

        // 成功的安装器写下 0。
        std::fs::write(&installer, "@echo off\r\nexit /b 0\r\n").unwrap();
        std::process::Command::new("cmd.exe")
            .raw_arg(format!("/D /C \"{step}\""))
            .creation_flags(0x0800_0000)
            .status()
            .unwrap();
        assert_eq!(paths.read_exit_code(), Some(0));
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

    /// 失败原因是给人看的短句，没有退出码时也说得清楚。
    #[test]
    fn every_failed_install_has_a_reason() {
        assert_eq!(failure_reason(Some(1223)), "安装器退出码 1223");
        assert!(failure_reason(None).contains("版本号"));
    }
}
