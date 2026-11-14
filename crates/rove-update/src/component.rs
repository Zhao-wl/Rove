//! C# 索引组件（#43）的跟随更新（issue #149 第 4 节）。
//!
//! 三条约定，逐条对着 issue：
//!
//! 1. **只有已经安装了组件时才检查组件更新**；没装的不替用户下载
//!    （#43 的原则：组件是可选的，用户自己决定装不装）。
//! 2. 组件包解压到 `%LOCALAPPDATA%\Rove\components\indexer-cs\<新版本>\`，
//!    **校验通过才切换**：先解到临时目录，确认里面有组件清单，再整体改名到版本目录。
//!    多版本并存是 #43 的既有能力，所以「切换」就是把新版本目录放进去（它的版本号更高，
//!    #43 的 [发现规则] 自然会选它）。
//! 3. **旧版本在下次启动、确认没有进程在用时再删**：[`cleanup_old`] 在启动时调用，
//!    删不掉（文件被占用）就留着，下次再说。
//!
//! 版本握手仍然是 #43 的那一套，本模块**不解析清单内容、不判定兼容性**：
//! 位置与「当前装的是哪个版本」都由调用方（`rove-ui`，它本来就在用
//! `rove_lang_cs::lifecycle::component`）传进来。这样核心层不会多出一条
//! `rove-update → rove-lang-cs` 的依赖，组件的规则也只有一处。

use crate::{
    download, http::Transport, manifest::Component, version::Version, zip, Result, UpdateError,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// 解压时的临时目录后缀。
pub const STAGING_SUFFIX: &str = ".staging";

/// 组件在本机的位置与现状（由调用方按 #43 的规则填）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// 版本目录的父目录，例如 `%LOCALAPPDATA%\Rove\components\indexer-cs`。
    pub root: PathBuf,
    /// 当前装着的版本；**`None` 表示没装**，这时不下载任何东西。
    pub version: Option<String>,
    /// 版本目录里必须存在的清单文件名（#43 的 `component.json`）。
    pub manifest_file: String,
}

impl Installed {
    pub fn version_dir(&self, version: &str) -> PathBuf {
        self.root.join(version)
    }
}

/// 要不要更新组件，以及更新到哪个版本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan<'a> {
    /// 没装组件 → 什么都不做（#149 第 4 节）。
    NotInstalled,
    /// 装了，但清单里没有组件，或者清单里的版本不比本地新。
    UpToDate,
    /// 该更新了。
    Update(&'a Component),
}

/// 判断要不要更新组件。
pub fn plan<'a>(component: Option<&'a Component>, installed: &Installed) -> Plan<'a> {
    let Some(current) = installed.version.as_deref() else {
        // 没装：不替用户下载。
        return Plan::NotInstalled;
    };
    let Some(component) = component else {
        return Plan::UpToDate;
    };
    let (Ok(new), Ok(current)) = (Version::parse(&component.version), Version::parse(current))
    else {
        // 版本号读不出来就当没有更新（与主程序同一条规则）。
        return Plan::UpToDate;
    };
    if new.is_newer_than(&current) {
        Plan::Update(component)
    } else {
        Plan::UpToDate
    }
}

/// 下载 + 校验 + 解压 + 切换。返回新版本目录。
///
/// `staging_dir` 是下载 zip 的位置（`%LOCALAPPDATA%\Rove\updates`）。
pub fn install(
    transport: &dyn Transport,
    component: &Component,
    staging_dir: &Path,
    installed: &Installed,
) -> Result<PathBuf> {
    let archive = download::download_verified(
        transport,
        &component.url,
        staging_dir,
        &component.file,
        &download::Expect::new(component.size, component.sha256.clone()),
    )?;
    let bytes = fs::read(&archive)
        .map_err(|e| UpdateError::Storage(format!("{} 读不了：{e}", archive.display())))?;
    let target = install_archive(&bytes, installed, &component.version);
    // zip 已经解完（或失败），留着它没意义。
    let _ = fs::remove_file(&archive);
    target
}

/// 把已经校验过的 zip 解到版本目录（独立出来是为了能脱离网络测试）。
pub fn install_archive(archive: &[u8], installed: &Installed, version: &str) -> Result<PathBuf> {
    // 版本号来自清单，会变成目录名：先按路径安全规则过一遍。
    if !zip::is_safe_entry(version) || version.contains('/') {
        return Err(UpdateError::Integrity(format!(
            "组件版本号不安全：{version}"
        )));
    }
    // 包里必须有组件清单，否则解出来 #43 也发现不了它。
    zip::read_file(archive, &installed.manifest_file)?;

    let final_dir = installed.version_dir(version);
    if final_dir.exists() {
        // 这个版本已经在了（上次装过）。什么都不做，#43 会自己发现它。
        return Ok(final_dir);
    }
    let staging = installed.root.join(format!("{version}{STAGING_SUFFIX}"));
    let _ = fs::remove_dir_all(&staging);
    let result = (|| -> Result<()> {
        zip::extract(archive, &staging)?;
        // 解完再确认一次清单真的落了地。
        if !staging.join(&installed.manifest_file).is_file() {
            return Err(UpdateError::Integrity(format!(
                "解出来的组件里没有 {}",
                installed.manifest_file
            )));
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    // 切换：整体改名。改名之前，#43 的发现规则看不到这个版本。
    fs::rename(&staging, &final_dir).map_err(|e| {
        let _ = fs::remove_dir_all(&staging);
        UpdateError::Storage(format!("切换到 {} 失败：{e}", final_dir.display()))
    })?;
    Ok(final_dir)
}

/// 启动时清理旧版本：`keep` 以外的版本目录能删就删，删不掉（进程还在用）就留着。
/// 返回删掉的目录数。
pub fn cleanup_old(installed: &Installed, keep: &str) -> usize {
    let Ok(entries) = fs::read_dir(&installed.root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == keep {
            continue;
        }
        if !entry.path().is_dir() {
            continue;
        }
        // 没解完的临时目录一定可以删；旧版本目录能删才删。
        if fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::tests::build_zip;

    fn component(version: &str) -> Component {
        Component {
            id: "indexer-cs".into(),
            version: version.into(),
            file: format!("rove-indexer-cs-{version}-win-x64.zip"),
            url: format!("https://a/rove-indexer-cs-{version}-win-x64.zip"),
            size: 0,
            sha256: String::new(),
        }
    }

    fn installed(root: &Path, version: Option<&str>) -> Installed {
        Installed {
            root: root.to_path_buf(),
            version: version.map(str::to_string),
            manifest_file: "component.json".into(),
        }
    }

    fn package(version: &str) -> Vec<u8> {
        let manifest = format!(
            r#"{{"component":"indexer-cs","version":"{version}","protocol":1,"index_format":1}}"#
        );
        build_zip(&[
            ("component.json", manifest.as_bytes()),
            ("rove-indexer-cs.exe", b"MZ"),
        ])
    }

    /// 验收：未安装时不下载。
    #[test]
    fn nothing_happens_when_the_component_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let c = component("1.3.0.0");
        assert_eq!(
            plan(Some(&c), &installed(dir.path(), None)),
            Plan::NotInstalled
        );
        assert_eq!(plan(None, &installed(dir.path(), None)), Plan::NotInstalled);
    }

    /// 验收：已安装时跟着更新；不比本地新就什么都不做。
    #[test]
    fn an_installed_component_follows_the_release() {
        let dir = tempfile::tempdir().unwrap();
        let newer = component("1.3.0.0");
        assert_eq!(
            plan(Some(&newer), &installed(dir.path(), Some("1.2.0.0"))),
            Plan::Update(&newer)
        );
        let same = component("1.2.0.0");
        assert_eq!(
            plan(Some(&same), &installed(dir.path(), Some("1.2.0.0"))),
            Plan::UpToDate
        );
        let older = component("1.1.0.0");
        assert_eq!(
            plan(Some(&older), &installed(dir.path(), Some("1.2.0.0"))),
            Plan::UpToDate
        );
        // 清单里没有组件 → 不动。
        assert_eq!(
            plan(None, &installed(dir.path(), Some("1.2.0.0"))),
            Plan::UpToDate
        );
        // 版本号读不出来 → 当作没有更新。
        let broken = component("???");
        assert_eq!(
            plan(Some(&broken), &installed(dir.path(), Some("1.2.0.0"))),
            Plan::UpToDate
        );
    }

    #[test]
    fn a_package_is_staged_then_switched_in() {
        let dir = tempfile::tempdir().unwrap();
        let target = installed(dir.path(), Some("1.2.0.0"));
        let path = install_archive(&package("1.3.0.0"), &target, "1.3.0.0").unwrap();
        assert_eq!(path, dir.path().join("1.3.0.0"));
        assert!(path.join("component.json").is_file());
        assert!(path.join("rove-indexer-cs.exe").is_file());
        // 临时目录不留下。
        assert!(!dir.path().join("1.3.0.0.staging").exists());
    }

    /// 包里没有组件清单 → 不切换，磁盘上不留半个版本目录。
    #[test]
    fn a_package_without_a_manifest_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = installed(dir.path(), Some("1.2.0.0"));
        let archive = build_zip(&[("rove-indexer-cs.exe", b"MZ")]);
        let error = install_archive(&archive, &target, "1.3.0.0").unwrap_err();
        assert!(matches!(error, UpdateError::Integrity(_)), "{error:?}");
        assert!(!dir.path().join("1.3.0.0").exists());
        assert!(!dir.path().join("1.3.0.0.staging").exists());
    }

    /// 清单里的版本号变成目录名之前要过一遍路径安全检查。
    #[test]
    fn a_hostile_version_cannot_escape_the_component_root() {
        let dir = tempfile::tempdir().unwrap();
        let target = installed(dir.path(), Some("1.2.0.0"));
        for version in ["../evil", "..", "a/b", r"a\b"] {
            let error = install_archive(&package("1.3.0.0"), &target, version).unwrap_err();
            assert!(matches!(error, UpdateError::Integrity(_)), "{version}");
        }
    }

    /// 验收：旧版本在下次启动后清理。
    #[test]
    fn old_versions_are_cleaned_up_on_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["1.1.0.0", "1.2.0.0", "1.3.0.0", "1.3.0.0.staging"] {
            fs::create_dir_all(dir.path().join(name)).unwrap();
            fs::write(dir.path().join(name).join("component.json"), b"{}").unwrap();
        }
        let target = installed(dir.path(), Some("1.3.0.0"));
        assert_eq!(cleanup_old(&target, "1.3.0.0"), 3);
        let left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["1.3.0.0"]);
    }

    /// 已经装过同一个版本时不重复解压，也不报错。
    #[test]
    fn installing_the_same_version_twice_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let target = installed(dir.path(), Some("1.2.0.0"));
        let first = install_archive(&package("1.3.0.0"), &target, "1.3.0.0").unwrap();
        fs::write(first.join("marker"), b"x").unwrap();
        let second = install_archive(&package("1.3.0.0"), &target, "1.3.0.0").unwrap();
        assert_eq!(first, second);
        assert!(second.join("marker").is_file(), "没有重新解压");
    }
}
