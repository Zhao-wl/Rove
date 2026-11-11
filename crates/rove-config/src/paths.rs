//! 共用设置文件的位置（FEATURES §13 规则 1）。
//!
//! 优先级：启动参数 `--vscode-user-dir` > 环境变量 `ROVE_VSCODE_USER_DIR` >
//! `%APPDATA%\Code\User`。这个位置不能写在设置文件里，所以没有对应的 `rove.*` 设置项。
//! 备份目录固定为 `%LOCALAPPDATA%\Rove\settings-backup`。

use std::{ffi::OsString, path::PathBuf};

/// 启动参数：`--vscode-user-dir <目录>` 或 `--vscode-user-dir=<目录>`。
pub const USER_DIR_ARG: &str = "--vscode-user-dir";
/// 环境变量：VSCodium、Cursor 等发行版的用户目录。
pub const USER_DIR_ENV: &str = "ROVE_VSCODE_USER_DIR";
pub const SETTINGS_FILE: &str = "settings.json";
pub const KEYBINDINGS_FILE: &str = "keybindings.json";

/// 从参数列表中取出 `--vscode-user-dir`，返回其值与其余参数（保持原顺序）。
/// 同一参数出现多次时最后一次胜出；缺少值的 `--vscode-user-dir` 被丢弃。
pub fn split_user_dir_arg(
    args: impl IntoIterator<Item = OsString>,
) -> (Option<PathBuf>, Vec<OsString>) {
    let prefix = format!("{USER_DIR_ARG}=");
    let mut dir = None;
    let mut rest = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some(a) if a == USER_DIR_ARG => {
                if let Some(value) = args.next() {
                    dir = Some(PathBuf::from(value));
                }
            }
            Some(a) if a.starts_with(&prefix) => dir = Some(PathBuf::from(&a[prefix.len()..])),
            _ => rest.push(arg),
        }
    }
    (dir.filter(|d| !d.as_os_str().is_empty()), rest)
}

/// 解析用户目录。`env` 用来读取环境变量（测试可注入）。
pub fn resolve_user_dir(
    arg: Option<PathBuf>,
    env: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let non_empty = |v: OsString| (!v.is_empty()).then(|| PathBuf::from(v));
    arg.or_else(|| env(USER_DIR_ENV).and_then(non_empty))
        .or_else(|| {
            env("APPDATA")
                .and_then(non_empty)
                .map(|a| a.join("Code").join("User"))
        })
}

/// 写入前备份的目录：`%LOCALAPPDATA%\Rove\settings-backup`。
pub fn resolve_backup_dir(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    env("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(|l| PathBuf::from(l).join("Rove").join("settings-backup"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn argument_beats_environment_beats_appdata() {
        let vars = [
            (USER_DIR_ENV, r"D:\Cursor\User"),
            ("APPDATA", r"C:\AppData"),
        ];
        assert_eq!(
            resolve_user_dir(Some(r"E:\VSCodium\User".into()), env(&vars)),
            Some(PathBuf::from(r"E:\VSCodium\User"))
        );
        assert_eq!(
            resolve_user_dir(None, env(&vars)),
            Some(PathBuf::from(r"D:\Cursor\User"))
        );
        assert_eq!(
            resolve_user_dir(None, env(&[(USER_DIR_ENV, ""), ("APPDATA", r"C:\AppData")])),
            Some(PathBuf::from(r"C:\AppData").join("Code").join("User"))
        );
        assert_eq!(resolve_user_dir(None, env(&[])), None);
        assert_eq!(
            resolve_backup_dir(env(&[("LOCALAPPDATA", r"C:\Local")])),
            Some(
                PathBuf::from(r"C:\Local")
                    .join("Rove")
                    .join("settings-backup")
            )
        );
    }

    #[test]
    fn user_dir_argument_is_split_from_the_rest() {
        let args = [
            "rove",
            "--vscode-user-dir",
            "X",
            "proj",
            "--vscode-user-dir=Y",
        ]
        .map(OsString::from);
        let (dir, rest) = split_user_dir_arg(args);
        assert_eq!(dir, Some(PathBuf::from("Y")));
        assert_eq!(rest, ["rove", "proj"].map(OsString::from));
        let (dir, rest) = split_user_dir_arg(["rove", "--vscode-user-dir"].map(OsString::from));
        assert_eq!((dir, rest), (None, vec![OsString::from("rove")]));
    }
}
