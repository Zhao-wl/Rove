use std::time::{Instant, SystemTime, UNIX_EPOCH};
fn main() {
    let started = process_start();
    println!(
        "MAIN_EPOCH_MS {}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let strings = rove_ui::skeleton::appearance::strings().unwrap();
    let message = |key| rove_ui::skeleton::appearance::message(&strings, key, &[]);
    let (user_dir, workspace) = parse_args(std::env::args_os().skip(1));
    let Some(path) = workspace else {
        eprintln!("{}", message("app.usage"));
        std::process::exit(2)
    };
    // #104：应用唯一的设置服务（共享 VS Code 的 settings.json，热加载）。
    let settings = match rove_ui::app_settings::open(user_dir) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("{} ({e})", message("app.noUserDir"));
            std::process::exit(2)
        }
    };
    rove_ui::skeleton::launch(path.into(), settings, started);
}

/// 拆出 `--vscode-user-dir`（FEATURES §13 规则 1），剩下的第一个参数是工作区路径。
fn parse_args(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> (Option<std::path::PathBuf>, Option<std::ffi::OsString>) {
    let (user_dir, rest) = rove_config::paths::split_user_dir_arg(args);
    (user_dir, rest.into_iter().next())
}

/// Include loader time before main on the supported Windows platform.
fn process_start() -> Instant {
    let now = Instant::now();
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::FILETIME,
            System::Threading::{GetCurrentProcess, GetProcessTimes},
        };
        let mut created: FILETIME = unsafe { std::mem::zeroed() };
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        // SAFETY: current-process pseudo-handle is valid and all four FILETIMEs are writable.
        let ok = unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        };
        if ok != 0 {
            let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
            // FILETIME is 100ns since 1601, Unix epoch starts in 1970.
            if let Some(ticks) = ticks.checked_sub(116_444_736_000_000_000) {
                let start = UNIX_EPOCH + std::time::Duration::from_nanos(ticks * 100);
                if let Ok(age) = SystemTime::now().duration_since(start) {
                    if let Some(instant) = now.checked_sub(age) {
                        return instant;
                    }
                }
            }
        }
        eprintln!("PROCESS_CLOCK_UNAVAILABLE: falling back to main entry");
    }
    now
}

#[cfg(test)]
mod tests {
    use super::parse_args;
    use std::{ffi::OsString, path::PathBuf};

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn user_dir_argument_is_not_taken_for_the_workspace() {
        assert_eq!(
            parse_args(args(&["--vscode-user-dir", r"D:\Cursor\User", r"D:\Game"])),
            (
                Some(PathBuf::from(r"D:\Cursor\User")),
                Some(OsString::from(r"D:\Game"))
            )
        );
        assert_eq!(
            parse_args(args(&[r"D:\Game", r"--vscode-user-dir=E:\VSCodium"])),
            (
                Some(PathBuf::from(r"E:\VSCodium")),
                Some(OsString::from(r"D:\Game"))
            )
        );
        // 不传时由 rove-config 按环境变量与 %APPDATA% 解析。
        assert_eq!(
            parse_args(args(&["proj"])),
            (None, Some(OsString::from("proj")))
        );
        assert_eq!(
            parse_args(args(&["--vscode-user-dir", "X"])),
            (Some(PathBuf::from("X")), None)
        );
    }
}
