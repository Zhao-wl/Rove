//! #149 第 4 节：C# 索引组件跟着主程序一起更新。
//!
//! 位置、版本与发现规则**只有一处**——#43 的 [`rove_lang_cs::lifecycle::component`]。
//! 这里把它翻译成 [`rove_update::component::Installed`] 再交给核心层：核心层既不解析组件
//! 清单、也不知道 #43 的查找顺序，所以 `rove-update` 不必多出一条到 `rove-lang-cs` 的依赖。
//!
//! 两个接入点（都在 `skeleton/workbench/update_host.rs` 里调用，都跑在后台执行器上）：
//!
//! 1. 主程序安装包下载完之后 → [`follow_update`]：**已安装组件才下载，没装不下**；
//! 2. 下次启动的首帧之后 → [`cleanup`]：删掉不再使用的旧版本目录，删不掉
//!    （进程还占着文件）就留到下一次启动再说。
//!
//! 组件这一步失败**不打扰用户**，只写进更新日志：主程序的更新才是主线，组件下次检查会再试。
//! 组件的安装位置永远是用户目录 `%LOCALAPPDATA%\Rove\components\indexer-cs\<版本>\`；
//! `rove.exe` 旁边的 `components`（随包放置）与 `ROVE_INDEXER_CS`（开发覆盖）不归更新管。

use rove_lang_cs::lifecycle::component as cs;
use rove_update::{component, Manifest, Updater};
use std::path::{Path, PathBuf};

/// 一次「跟随更新」的结果（给日志与测试看；界面上什么都不显示）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Followed {
    /// 没装组件，或者开发期用 `ROVE_INDEXER_CS` 指着别处 → 什么都不做。
    Skipped,
    /// 清单里没有组件，或者组件不比本地新。
    UpToDate,
    /// 装好了，返回新版本目录。
    Installed(PathBuf),
    /// 下载 / 校验 / 解压失败（旧版本不受影响）。
    Failed(String),
}

/// 把 #43 的位置与「现在装着哪个版本」翻译成核心层要的形状。
///
/// 返回 `None` 表示这台机器上不该由更新流程碰组件（没有用户级组件目录，
/// 或者 `ROVE_INDEXER_CS` 指着用户自己管理的那一份）。
pub fn installed(roots: &cs::Roots) -> Option<component::Installed> {
    if roots.env_override.is_some() {
        return None;
    }
    let root = roots.dirs.first()?.clone();
    Some(component::Installed {
        version: newest_installed(roots).map(cs::version_text),
        root,
        manifest_file: cs::MANIFEST_FILE.to_string(),
    })
}

/// 本机装着的最高组件版本。**兼容与否都算**：不兼容的那一份恰恰最需要跟着更新。
fn newest_installed(roots: &cs::Roots) -> Option<[u16; 4]> {
    roots.dirs.iter().flat_map(|dir| versions_in(dir)).max()
}

/// 一个组件根目录下的候选版本（判定条件与 #43 的发现规则一致：清单 + 可执行文件都在）。
fn versions_in(dir: &Path) -> Vec<[u16; 4]> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            path.join(cs::EXE_FILE).is_file().then_some(())?;
            let text = std::fs::read_to_string(path.join(cs::MANIFEST_FILE)).ok()?;
            Some(cs::Manifest::parse(&text)?.version)
        })
        .collect()
}

/// 主程序安装包下载完之后，组件跟着升级（#149 第 4 节）。
pub fn follow_update(updater: &Updater, manifest: &Manifest, roots: &cs::Roots) -> Followed {
    let log = updater.log();
    let Some(installed) = installed(roots) else {
        return Followed::Skipped;
    };
    let target = match component::plan(manifest.indexer_component(), &installed) {
        // 验收：未安装时不下载。
        component::Plan::NotInstalled => {
            log.line("component: 没有安装索引组件，不下载");
            return Followed::Skipped;
        }
        component::Plan::UpToDate => {
            log.line("component: 清单里没有更新的组件");
            return Followed::UpToDate;
        }
        component::Plan::Update(target) => target,
    };
    let (Some(transport), Some(paths)) = (updater.transport(), updater.paths()) else {
        return Followed::Skipped;
    };
    let staging = match paths.ensure_updates_dir() {
        Ok(dir) => dir,
        Err(error) => return failed(log, &error.reason()),
    };
    log.line(&format!(
        "component download start: {} ({} 字节)",
        target.file, target.size
    ));
    match component::install(transport.as_ref(), target, &staging, &installed) {
        Ok(dir) => {
            log.line(&format!("component installed: {}", target.version));
            Followed::Installed(dir)
        }
        Err(error) => failed(log, &error.reason()),
    }
}

fn failed(log: &rove_update::log::UpdateLog, reason: &str) -> Followed {
    log.line(&format!("component failed: {reason}（主程序更新不受影响）"));
    Followed::Failed(reason.to_string())
}

/// 启动时清理旧版本（#149 第 4 节「旧版本在下次启动、确认没有进程在用时再删」）。
///
/// `keep` 是上次更新随包装上的那一版（`pending.json` 里的 `component_version`）；
/// 没有就按 #43 的发现规则取「正在用的那一版」。两样都定不下来时**一个目录都不删**：
/// 宁可多留一份，也不能删掉用户正在用的组件。返回删掉的目录数。
pub fn cleanup(roots: &cs::Roots, keep: Option<&str>) -> usize {
    let Some(installed) = installed(roots) else {
        return 0;
    };
    let Some(keep) = keep
        .filter(|version| installed.version_dir(version).is_dir())
        .map(str::to_string)
        .or_else(|| in_use_version(roots, &installed.root))
    else {
        return 0;
    };
    component::cleanup_old(&installed, &keep)
}

/// #43 的发现规则选中的那一版，且必须落在用户目录里。
fn in_use_version(roots: &cs::Roots, root: &Path) -> Option<String> {
    let cs::Discovery::Found(found) = cs::discover(roots) else {
        return None;
    };
    let dir = found.program.parent()?;
    (dir.parent() == Some(root)).then_some(())?;
    Some(dir.file_name()?.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与本 Rove 兼容的组件清单（协议与索引格式都取当前常量）。
    fn install_version(root: &Path, version: &str) {
        let dir = root.join(version);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(cs::EXE_FILE), b"MZ").unwrap();
        std::fs::write(
            dir.join(cs::MANIFEST_FILE),
            format!(
                r#"{{"component":"indexer-cs","version":"{version}","protocol":{},"protocol_min":{},"index_format":{}}}"#,
                rove_lang_cs::lifecycle::resident::PROTOCOL,
                rove_lang_cs::lifecycle::resident::PROTOCOL_MIN,
                rove_lang_cs::lifecycle::validity::EXPECTED_FORMAT,
            ),
        )
        .unwrap();
    }

    fn roots(dir: &Path) -> cs::Roots {
        cs::Roots {
            env_override: None,
            dirs: vec![dir.to_path_buf()],
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    /// 没装组件 → 版本是 `None`，核心层据此判定「不下载」。
    #[test]
    fn an_empty_component_root_reads_as_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let installed = installed(&roots(dir.path())).expect("有用户级组件目录");
        assert_eq!(installed.version, None);
        assert_eq!(installed.root, dir.path());
        assert_eq!(installed.manifest_file, cs::MANIFEST_FILE);
    }

    /// 装了几个版本时取最高的那一个（#43 的发现规则也选它）。
    #[test]
    fn the_newest_installed_version_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        install_version(dir.path(), "1.2.0.0");
        install_version(dir.path(), "1.10.0.0");
        // 只有目录、没有可执行文件的不算候选。
        std::fs::create_dir_all(dir.path().join("1.99.0.0")).unwrap();
        let installed = installed(&roots(dir.path())).unwrap();
        assert_eq!(installed.version.as_deref(), Some("1.10.0.0"));
    }

    /// `ROVE_INDEXER_CS` 指着开发目录时，更新流程完全不碰组件。
    #[test]
    fn a_development_override_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        install_version(dir.path(), "1.2.0.0");
        let mut roots = roots(dir.path());
        roots.env_override = Some(dir.path().join("rove-indexer-cs.exe"));
        assert_eq!(installed(&roots), None);
        assert_eq!(cleanup(&roots, Some("1.2.0.0")), 0);
    }

    /// 验收：旧版本在下次启动后清理，正在用的那一版留着。
    #[test]
    fn old_versions_are_cleaned_up_and_the_one_in_use_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        install_version(dir.path(), "1.2.0.0");
        install_version(dir.path(), "1.3.0.0");
        std::fs::create_dir_all(dir.path().join("1.3.0.0.staging")).unwrap();
        // 没有提示时按 #43 的发现规则：选中的是最高的兼容版本。
        assert_eq!(cleanup(&roots(dir.path()), None), 2);
        assert_eq!(names(dir.path()), ["1.3.0.0"]);
    }

    /// 刚随更新装上的那一版（`pending.json` 记的）优先作为「要保留的那一版」。
    #[test]
    fn the_version_recorded_with_the_update_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        install_version(dir.path(), "1.2.0.0");
        install_version(dir.path(), "1.3.0.0");
        assert_eq!(cleanup(&roots(dir.path()), Some("1.2.0.0")), 1);
        assert_eq!(names(dir.path()), ["1.2.0.0"]);
    }

    /// 定不下来「正在用哪一版」时一个目录都不删（宁可多留一份）。
    #[test]
    fn nothing_is_deleted_when_the_version_in_use_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        // 只有不带可执行文件的目录：#43 发现不了它，提示的版本也不存在。
        std::fs::create_dir_all(dir.path().join("1.2.0.0")).unwrap();
        std::fs::create_dir_all(dir.path().join("1.3.0.0")).unwrap();
        assert_eq!(cleanup(&roots(dir.path()), Some("9.9.9.9")), 0);
        assert_eq!(names(dir.path()), ["1.2.0.0", "1.3.0.0"]);
    }

    /// `rove.exe` 旁边的随包组件不归更新管：它正在用时，用户目录里一个都不删。
    #[test]
    fn a_component_next_to_the_exe_is_not_cleaned_up() {
        let user = tempfile::tempdir().unwrap();
        let beside_exe = tempfile::tempdir().unwrap();
        install_version(beside_exe.path(), "1.3.0.0");
        std::fs::create_dir_all(user.path().join("1.2.0.0")).unwrap();
        let roots = cs::Roots {
            env_override: None,
            dirs: vec![user.path().to_path_buf(), beside_exe.path().to_path_buf()],
        };
        assert_eq!(cleanup(&roots, None), 0);
        assert_eq!(names(user.path()), ["1.2.0.0"]);
        assert_eq!(names(beside_exe.path()), ["1.3.0.0"]);
    }
}
