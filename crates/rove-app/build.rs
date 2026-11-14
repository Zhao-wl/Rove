//! Windows 资源：应用图标与版本信息（issue #76 第 4 节）。
//!
//! - **图标**：`packaging/windows/rove.ico`（由 `cargo xtask icon` 生成）以资源 ID 1 编进 `rove.exe`。
//!   gpui 的 `load_icon()` 就是按 ID 1 取图标，所以窗口、任务栏与 Alt+Tab 用的是同一张图。
//! - **版本信息**：`FileDescription` / `ProductName` / `CompanyName` / `FileVersion` /
//!   `LegalCopyright` 等，在资源管理器的「属性 → 详细信息」里可见。
//!
//! 版本号的**唯一来源**是根 `Cargo.toml` 的 `[workspace.package] version`（这里读
//! `CARGO_PKG_VERSION`，各 crate 都写 `version.workspace = true`）；提交号与构建日期来自
//! 发布流水线注入的 `ROVE_BUILD_COMMIT` / `ROVE_BUILD_DATE`（没有就现场问 git，再不行写 unknown）。
//! 「关于」对话框显示的是同一组值（`rove-ui/build.rs`）。
//!
//! 非 Windows 平台什么都不做（首发只有 Windows，`PROJECT_CHARTER` §3）。

fn main() {
    println!("cargo:rerun-if-env-changed=ROVE_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=ROVE_BUILD_DATE");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    windows::embed();
}

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};

    pub fn embed() {
        let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let repo = manifest
            .parent()
            .and_then(Path::parent)
            .expect("crates/rove-app 必须在仓库里");
        let icon = repo.join("packaging").join("windows").join("rove.ico");
        if !icon.is_file() {
            // 图标是生成物但已提交；真缺了就提醒怎么补，不要让整个构建莫名其妙地失败。
            println!(
                "cargo:warning=找不到 {}；先跑 cargo xtask icon。本次构建不带图标与版本信息。",
                icon.display()
            );
            return;
        }
        println!("cargo:rerun-if-changed={}", icon.display());

        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
        let commit = build_value("ROVE_BUILD_COMMIT", &["rev-parse", "HEAD"], repo);
        let date = build_value(
            "ROVE_BUILD_DATE",
            &["log", "-1", "--date=format:%Y-%m-%d", "--format=%cd"],
            repo,
        );
        let short: String = commit.chars().take(12).collect();

        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("rove.rc");
        std::fs::write(&out, resource_script(&icon, &version, &short, &date))
            .expect("写出 rove.rc");
        // rc.exe / llvm-rc 由 embed-resource 自己找；缺少 manifest 工具时不致命（manifest 由 gpui 提供）。
        embed_resource::compile(&out, embed_resource::NONE)
            .manifest_optional()
            .expect("编译 Windows 资源");
    }

    fn build_value(env: &str, args: &[&str], repo: &Path) -> String {
        if let Ok(value) = std::env::var(env) {
            if !value.is_empty() {
                return value;
            }
        }
        std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown".into())
    }

    /// 四段版本号：`0.1.0` → `0,1,0,0`（Windows 的版本资源固定四段）。
    fn quad(version: &str) -> String {
        let mut parts: Vec<String> = version
            .split(['.', '-', '+'])
            .take(4)
            .map(|p| p.parse::<u16>().unwrap_or(0).to_string())
            .collect();
        while parts.len() < 4 {
            parts.push("0".into());
        }
        parts.join(",")
    }

    fn resource_script(icon: &Path, version: &str, commit: &str, date: &str) -> String {
        // `#pragma code_page(65001)`：.rc 按 UTF-8 读，中文说明不会变成乱码。
        format!(
            r#"#pragma code_page(65001)
1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {quad}
PRODUCTVERSION {quad}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "080404b0"
        BEGIN
            VALUE "CompanyName", "Rove"
            VALUE "FileDescription", "Rove 代码阅读编辑器"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "rove"
            VALUE "LegalCopyright", "Copyright (c) 2026 Rove contributors. MIT License."
            VALUE "OriginalFilename", "rove.exe"
            VALUE "ProductName", "Rove"
            VALUE "ProductVersion", "{version} ({commit} {date})"
            VALUE "Comments", "https://github.com/Zhao-wl/Rove"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x804, 1200
    END
END
"#,
            icon = icon.display().to_string().replace('\\', "\\\\"),
            quad = quad(version),
        )
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn quad_pads_to_four_parts() {
            assert_eq!(super::quad("0.1.0"), "0,1,0,0");
            assert_eq!(super::quad("1.2.3.4"), "1,2,3,4");
            assert_eq!(super::quad("0.1.0-rc.1"), "0,1,0,0");
        }
    }
}

#[cfg(not(windows))]
mod windows {
    pub fn embed() {}
}
