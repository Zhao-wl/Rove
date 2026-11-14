//! `cargo xtask release`：本地发布流水线（issue #76）。
//!
//! 一条命令把一次发布需要的产物全部做出来，放在 `target/release-dist/<版本>/`：
//!
//! ```text
//! Rove-<版本>-win-x64.zip            便携版（解压即用）
//! Rove-<版本>-win-x64-setup.exe      安装包（需要 Inno Setup 6 的 ISCC.exe；没装就跳过）
//! SHA256SUMS.txt                     上面所有产物的 SHA-256
//! latest.json / latest.json.sig      #149 的更新清单与 ed25519 分离签名
//! release-notes.md                   Release 说明草稿（SmartScreen、WebView2 的说明已写好）
//! upload.ps1                         用 gh release create 上传的脚本（本命令不执行它）
//! ```
//!
//! **本命令不打 tag、不创建 Release、不上传任何东西**：上传要人工跑 `upload.ps1`（它会先让你确认）。
//! GitHub Actions 的同一条流水线在 `.github/workflows/release.yml`，两边产物一致。
//!
//! 版本号的唯一来源是根 `Cargo.toml` 的 `[workspace.package] version`；
//! 版本资源、安装包、`latest.json`、「关于」对话框都从它来（`--check-version` 校验 tag 与它一致）。

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::pack::{self, Contents, PackedFile};
use crate::sign;
use crate::util;

/// 默认的下载地址前缀（GitHub Release 的资产 URL 规则）。
const DEFAULT_REPO: &str = "Zhao-wl/Rove";
/// 安装器的静默升级参数（与 #149 对齐，写进 latest.json 让更新端照着调用）。
pub const SILENT_ARGS: &[&str] = &[
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/NOCANCEL",
    "/SP-",
];

#[derive(Serialize)]
struct Manifest {
    schema: u32,
    product: &'static str,
    channel: String,
    version: String,
    released: String,
    commit: String,
    notes_url: String,
    installer: Option<Artifact>,
    portable: Artifact,
    components: Vec<Component>,
}

#[derive(Serialize)]
struct Artifact {
    file: String,
    url: String,
    size: u64,
    sha256: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    silent_args: Vec<String>,
}

#[derive(Serialize)]
struct Component {
    id: String,
    version: String,
    file: String,
    url: String,
    size: u64,
    sha256: String,
}

pub fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = fs::read(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    Ok(sha256(&bytes))
}

/// 根 `Cargo.toml` 的 `[workspace.package] version`：版本号的唯一来源。
pub fn workspace_version() -> Result<String, Box<dyn std::error::Error>> {
    let path = util::repo_root().join("Cargo.toml");
    let text = util::read_text(&path)?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
            continue;
        }
        if in_package {
            if let Some(rest) = line.strip_prefix("version") {
                if let Some(value) = rest.split('=').nth(1) {
                    return Ok(value.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    Err("根 Cargo.toml 的 [workspace.package] 里没有 version".into())
}

/// tag（`v0.1.0`）与 `Cargo.toml` 的版本一致吗？CI 在打 tag 时用它把关。
pub fn check_version(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let tag = util::flag(args, "--tag")
        .or_else(|| args.iter().find(|a| !a.starts_with("--")).cloned())
        .ok_or("用法：cargo xtask check-version --tag v0.1.0")?;
    let version = workspace_version()?;
    let expected = format!("v{version}");
    println!("Cargo.toml 版本：{version}（期望的 tag：{expected}）");
    println!("实际的 tag：{tag}");
    if tag != expected {
        println!("::error::tag `{tag}` 与 Cargo.toml 的版本 `{version}` 不一致；先改 Cargo.toml 再打 tag");
        return Ok(1);
    }
    println!("OK：tag 与 Cargo.toml 的版本一致。");
    Ok(0)
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(util::repo_root())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 构建信息：提交号与构建日期注入到版本资源与「关于」对话框（#76 第 4 节）。
pub struct BuildInfo {
    pub commit: String,
    pub short_commit: String,
    pub date: String,
    pub dirty: bool,
}

pub fn build_info() -> BuildInfo {
    let commit = git_output(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let short_commit = commit.chars().take(12).collect::<String>();
    let date = git_output(&["log", "-1", "--date=format:%Y-%m-%d", "--format=%cd"])
        .unwrap_or_else(|| "unknown".into());
    let dirty = git_output(&["status", "--porcelain"]).is_some();
    BuildInfo {
        commit,
        short_commit,
        date,
        dirty,
    }
}

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let version = match util::flag(args, "--version") {
        Some(v) => v,
        None => workspace_version()?,
    };
    let repo = util::flag(args, "--repo").unwrap_or_else(|| DEFAULT_REPO.to_string());
    let channel = util::flag(args, "--channel").unwrap_or_else(|| "stable".to_string());
    let out_root = util::flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| util::target_dir().join("release-dist"));
    let out = out_root.join(&version);
    let info = build_info();

    println!("Rove 发布流水线（issue #76）");
    println!("  版本：{version}（来源：Cargo.toml 的 [workspace.package] version）");
    println!("  提交：{}（{}）", info.short_commit, info.date);
    if info.dirty {
        println!("::warning::工作区有未提交的改动，产物不可复现；正式发布前先提交干净");
    }
    println!("  输出：{}", out.display());
    fs::create_dir_all(&out)?;

    // ---------------------------------------------------------------- 1. 构建
    if util::has_flag(args, "--skip-build") {
        println!("\n[1/6] 跳过构建（--skip-build）");
    } else {
        println!("\n[1/6] cargo build --release -p rove-app");
        let status = Command::new(cargo())
            .current_dir(util::repo_root())
            .args(["build", "--release", "-p", "rove-app"])
            // 版本资源与「关于」对话框用同一组构建信息。
            .env("ROVE_BUILD_COMMIT", &info.commit)
            .env("ROVE_BUILD_DATE", &info.date)
            .env("ROVE_BUILD_CHANNEL", &channel)
            .status()?;
        if !status.success() {
            return Err("release 构建失败".into());
        }
    }

    // ---------------------------------------------------------------- 2. 暂存目录
    let contents = Contents::load_default()?;
    let exe = util::flag(args, "--binary").map(PathBuf::from);
    let files = contents.resolve(exe.as_deref())?;
    let stage_name = contents.stage_name(&version);
    let stage = out.join(&stage_name);
    println!("\n[2/6] 组装 {}", stage.display());
    pack::stage_files(&files, &stage)?;
    print_package_report(&files, &contents)?;

    // ---------------------------------------------------------------- 3. 便携版 zip
    println!("\n[3/6] 便携版 zip");
    let zip = out.join(format!("{stage_name}.zip"));
    if zip.exists() {
        fs::remove_file(&zip)?;
    }
    compress(&stage, &zip)?;
    println!(
        "  {}（{}）",
        zip.display(),
        util::human_bytes(fs::metadata(&zip)?.len())
    );

    // ---------------------------------------------------------------- 4. 安装包
    println!("\n[4/6] 安装包（Inno Setup）");
    let installer = match find_iscc() {
        Some(iscc) => Some(build_installer(&iscc, &version, &stage, &out, &info)?),
        None => {
            println!("::warning::没找到 Inno Setup 6 的 ISCC.exe，跳过安装包");
            println!("  装好后重跑本命令，或用 ROVE_ISCC 指定 ISCC.exe 的路径。");
            println!("  下载：https://jrsoftware.org/isdl.php（人工安装，本命令不下载任何东西）");
            if util::has_flag(args, "--require-installer") {
                return Err("--require-installer：没有 ISCC.exe 就不算完成".into());
            }
            None
        }
    };

    // ---------------------------------------------------------------- 5. 校验值与更新清单
    println!("\n[5/6] SHA256 与更新清单");
    let component = util::flag(args, "--component").map(PathBuf::from);
    let mut artifacts: Vec<PathBuf> = vec![zip.clone()];
    artifacts.extend(installer.clone());
    artifacts.extend(component.clone());

    let mut sums = String::new();
    for path in &artifacts {
        let digest = sha256_file(path)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        println!("  {digest}  {name}");
        sums.push_str(&format!("{digest}  {name}\n"));
    }
    let sums_path = out.join("SHA256SUMS.txt");
    fs::write(&sums_path, &sums)?;

    let base_url = util::flag(args, "--base-url")
        .unwrap_or_else(|| format!("https://github.com/{repo}/releases/download/v{version}"));
    let notes_url = util::flag(args, "--notes-url")
        .unwrap_or_else(|| format!("https://github.com/{repo}/releases/tag/v{version}"));
    let manifest = build_manifest(
        &version,
        &channel,
        &info,
        &notes_url,
        &base_url,
        &zip,
        installer.as_deref(),
        component.as_deref(),
    )?;
    let manifest_path = out.join("latest.json");
    // 签名覆盖的是这里写出的**字节**；#149 验签时不要重新序列化。
    let text = format!("{}\n", serde_json::to_string_pretty(&manifest)?);
    fs::write(&manifest_path, text.as_bytes())?;
    println!("  {}", manifest_path.display());

    if util::has_flag(args, "--unsigned") {
        println!("::warning::--unsigned：没有签名 latest.json，#149 的更新端会拒绝它");
    } else {
        let sig = sign::append_sig(&manifest_path);
        let key = sign::sign_file(&manifest_path, &sig)?;
        sign::verify_file(&manifest_path, &sig, &sign::public_key_path())
            .map_err(|e| format!("自检失败：签名无法用仓库里的公钥验证（{e}）"))?;
        println!(
            "  {}（公钥指纹 {}，已自检）",
            sig.display(),
            sign::fingerprint(&key)
        );
    }

    // ---------------------------------------------------------------- 6. 说明与上传脚本
    println!("\n[6/6] Release 说明草稿与上传脚本");
    let notes = out.join("release-notes.md");
    fs::write(&notes, release_notes(&version, &info, &sums))?;
    let upload = out.join("upload.ps1");
    fs::write(
        &upload,
        upload_script(
            &version,
            &repo,
            &out,
            &artifacts,
            &manifest_path,
            !util::has_flag(args, "--unsigned"),
        ),
    )?;
    println!("  {}", notes.display());
    println!("  {}", upload.display());
    println!("\n产物都在 {}。", out.display());
    println!("上传（人工执行，本命令不打 tag、不建 Release）：");
    println!("  powershell -NoProfile -File {}", upload.display());
    Ok(0)
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// 发布包的体积报告：文件分项 + 嵌入资产分项（issue #76 第 3 节）。
pub fn print_package_report(
    files: &[PackedFile],
    contents: &Contents,
) -> Result<u64, Box<dyn std::error::Error>> {
    let total: u64 = files.iter().map(|f| f.size).sum();
    println!("  发布包文件：");
    for file in files {
        println!(
            "    {:>12}  {} [{}]{}",
            util::human_bytes(file.size),
            file.relative,
            file.kind,
            file.note
                .as_ref()
                .map(|n| format!("（{n}）"))
                .unwrap_or_default()
        );
    }
    println!("    {:>12}  合计", util::human_bytes(total));
    println!("  嵌入 rove.exe 的资产（未压缩的源体积，按来源分项）：");
    for group in contents.embedded()? {
        if group.missing {
            println!("    {:>12}  {}（目录不存在）", "-", group.name);
            continue;
        }
        println!(
            "    {:>12}  {}（{} 个文件{}{}）",
            util::human_bytes(group.size),
            group.name,
            group.files,
            group.owner.map(|o| format!("，{o}")).unwrap_or_default(),
            group.note.map(|n| format!("，{n}")).unwrap_or_default()
        );
    }
    Ok(total)
}

fn compress(stage: &Path, zip: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Compress-Archive 在 Windows PowerShell 5.1 上就有；本机没有 pwsh（AGENTS.md §9）。
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "Compress-Archive -Path '{}' -DestinationPath '{}' -CompressionLevel Optimal",
            stage.display(),
            zip.display()
        ))
        .status()
        .map_err(|e| format!("调用 powershell 失败：{e}"))?;
    if !status.success() {
        return Err("Compress-Archive 失败".into());
    }
    Ok(())
}

/// 找 Inno Setup 6 的 ISCC.exe：`ROVE_ISCC` → PATH → 默认安装位置。
pub fn find_iscc() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ROVE_ISCC") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    if Command::new("ISCC.exe").arg("/?").output().is_ok() {
        return Some(PathBuf::from("ISCC.exe"));
    }
    for base in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(dir) = std::env::var_os(base) {
            let path = PathBuf::from(dir).join("Inno Setup 6").join("ISCC.exe");
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn build_installer(
    iscc: &Path,
    version: &str,
    stage: &Path,
    out: &Path,
    info: &BuildInfo,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let script = util::repo_root().join("packaging/windows/rove.iss");
    let status = Command::new(iscc)
        .arg(format!("/DRoveVersion={version}"))
        .arg(format!("/DRoveCommit={}", info.short_commit))
        .arg(format!("/DStageDir={}", stage.display()))
        .arg(format!("/DOutputDir={}", out.display()))
        .arg(&script)
        .status()
        .map_err(|e| format!("调用 {} 失败：{e}", iscc.display()))?;
    if !status.success() {
        return Err("ISCC 编译安装包失败".into());
    }
    let installer = out.join(format!("Rove-{version}-win-x64-setup.exe"));
    if !installer.is_file() {
        return Err(format!("ISCC 没有产出 {}", installer.display()).into());
    }
    println!(
        "  {}（{}）",
        installer.display(),
        util::human_bytes(fs::metadata(&installer)?.len())
    );
    Ok(installer)
}

#[allow(clippy::too_many_arguments)]
fn build_manifest(
    version: &str,
    channel: &str,
    info: &BuildInfo,
    notes_url: &str,
    base_url: &str,
    zip: &Path,
    installer: Option<&Path>,
    component: Option<&Path>,
) -> Result<Manifest, Box<dyn std::error::Error>> {
    let artifact = |path: &Path, silent: bool| -> Result<Artifact, Box<dyn std::error::Error>> {
        let file = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Ok(Artifact {
            url: format!("{base_url}/{file}"),
            size: fs::metadata(path)?.len(),
            sha256: sha256_file(path)?,
            silent_args: if silent {
                SILENT_ARGS.iter().map(|s| s.to_string()).collect()
            } else {
                Vec::new()
            },
            file,
        })
    };
    let components = match component {
        Some(path) => {
            let file = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            // 包名形如 rove-indexer-cs-<版本>-win-x64.zip（#43 的打包脚本产出）。
            let version = file
                .trim_start_matches("rove-indexer-cs-")
                .trim_end_matches("-win-x64.zip")
                .to_string();
            vec![Component {
                id: "indexer-cs".into(),
                version,
                url: format!("{base_url}/{file}"),
                size: fs::metadata(path)?.len(),
                sha256: sha256_file(path)?,
                file,
            }]
        }
        None => Vec::new(),
    };
    Ok(Manifest {
        schema: 1,
        product: "rove",
        channel: channel.to_string(),
        version: version.to_string(),
        released: info.date.clone(),
        commit: info.commit.clone(),
        notes_url: notes_url.to_string(),
        installer: match installer {
            Some(path) => Some(artifact(path, true)?),
            None => None,
        },
        portable: artifact(zip, false)?,
        components,
    })
}

fn release_notes(version: &str, info: &BuildInfo, sums: &str) -> String {
    format!(
        "# Rove {version}\n\
         \n\
         提交 `{}`，构建日期 {}。\n\
         \n\
         ## 安装\n\
         \n\
         - **安装包** `Rove-{version}-win-x64-setup.exe`：用户级安装，装到 `%LOCALAPPDATA%\\Programs\\Rove`，\n\
           **不需要管理员权限**，不写 `HKLM`。文件关联与资源管理器右键菜单是安装时的可选项，默认不勾选。\n\
         - **便携版** `Rove-{version}-win-x64.zip`：解压即用，不写注册表。\n\
         - **C# 索引组件**（可选，单独下载）：解压到 `%LOCALAPPDATA%\\Rove\\components\\indexer-cs\\`，\n\
           没有它时 C# 导航自动降级（见 README）。\n\
         \n\
         ## 首次运行会看到 SmartScreen 提示\n\
         \n\
         本版**没有做 Authenticode 代码签名**，Windows 会提示「Windows 已保护你的电脑 / 未知发布者」。\n\
         放行方法：点「更多信息」→「仍要运行」。介意的话先用上面的 SHA-256 核对文件再运行。\n\
         \n\
         ## 需要 WebView2 运行时的功能\n\
         \n\
         Markdown 预览用系统的 WebView2 运行时。Windows 11 自带；部分 Windows 10 没有，\n\
         这时 Markdown 以源码打开并提示一次。**安装器不捆绑、也不会联网下载 WebView2。**\n\
         \n\
         ## 卸载\n\
         \n\
         从「应用和功能」卸载。程序目录、快捷方式与注册表项都会清除；用户数据（`%LOCALAPPDATA%\\Rove`，\n\
         含索引缓存与设置备份）默认保留，卸载时可以勾选一并删除。\n\
         \n\
         ## SHA-256\n\
         \n\
         ```\n{sums}```\n",
        info.short_commit, info.date
    )
}

fn upload_script(
    version: &str,
    repo: &str,
    out: &Path,
    artifacts: &[PathBuf],
    manifest: &Path,
    signed: bool,
) -> String {
    let mut files: Vec<String> = artifacts
        .iter()
        .map(|p| format!("'{}'", p.display()))
        .collect();
    files.push(format!("'{}'", out.join("SHA256SUMS.txt").display()));
    files.push(format!("'{}'", manifest.display()));
    if signed {
        files.push(format!("'{}'", sign::append_sig(manifest).display()));
    }
    format!(
        "# 由 `cargo xtask release` 生成（issue #76）。上传是**人工步骤**：\r\n\
         # 本脚本会先打印要做什么并要求确认，确认后才打 tag、建草稿 Release 并上传产物。\r\n\
         # GitHub Actions 的额度耗尽时就用它代替 .github/workflows/release.yml。\r\n\
         $ErrorActionPreference = 'Stop'\r\n\
         $tag = 'v{version}'\r\n\
         $repo = '{repo}'\r\n\
         $files = @({files})\r\n\
         Write-Host \"将要：\"\r\n\
         Write-Host \"  1. git tag $tag 并推送到 origin\"\r\n\
         Write-Host \"  2. gh release create $tag --draft（草稿，不公开）\"\r\n\
         Write-Host \"  3. 上传 $($files.Count) 个产物\"\r\n\
         $answer = Read-Host '确认执行？输入 yes 继续'\r\n\
         if ($answer -ne 'yes') {{ Write-Host '已取消。'; exit 1 }}\r\n\
         git tag -a $tag -m \"Rove {version}\"\r\n\
         git push origin $tag\r\n\
         gh release create $tag --repo $repo --draft --title \"Rove {version}\" --notes-file '{notes}' @files\r\n\
         Write-Host '已创建草稿 Release；人工确认后再在网页上发布。'\r\n",
        version = version,
        repo = repo,
        files = files.join(", "),
        notes = out.join("release-notes.md").display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_version_is_the_single_source() {
        let version = workspace_version().unwrap();
        assert!(
            version.split('.').count() == 3,
            "版本号应为 x.y.z，实际 {version}"
        );
        // 各 crate 都写 version.workspace = true，不另写版本号。
        let app = util::read_text(&util::repo_root().join("crates/rove-app/Cargo.toml")).unwrap();
        assert!(
            app.contains("version.workspace = true"),
            "rove-app 必须跟随工作区版本"
        );
    }

    #[test]
    fn check_version_rejects_a_mismatched_tag() {
        let version = workspace_version().unwrap();
        assert_eq!(
            check_version(&[format!("--tag=v{version}")]).unwrap(),
            0,
            "tag 与 Cargo.toml 一致时应通过"
        );
        assert_eq!(
            check_version(&["--tag=v99.99.99".to_string()]).unwrap(),
            1,
            "tag 与 Cargo.toml 不一致时必须失败"
        );
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn installer_script() -> String {
        util::read_text(&util::repo_root().join("packaging/windows/rove.iss")).unwrap()
    }

    /// 安装器的硬约束（issue #76 的验收标准）：用户级、不写 HKLM、可选项默认不勾、
    /// 关联不抢默认程序、注册表项卸载时清除、不联网。
    /// 本机没装 Inno Setup 时 ISCC 跑不了，所以这些约束按脚本文本逐条检查。
    #[test]
    fn installer_script_is_user_level_and_reversible() {
        let iss = installer_script();
        assert!(
            iss.contains("PrivilegesRequired=lowest"),
            "必须是用户级安装"
        );
        assert!(
            iss.contains(r"DefaultDirName={localappdata}\Programs\{#RoveName}"),
            "装到 %LOCALAPPDATA%\\Programs\\Rove"
        );
        // 注释里说明「绝不写 HKLM」，所以只看非注释行。
        for line in iss.lines().filter(|l| !l.trim_start().starts_with(';')) {
            assert!(
                !line.to_uppercase().contains("HKLM"),
                "不得写 HKLM（用户级安装）：{line}"
            );
        }
        assert!(
            iss.contains("CloseApplications=yes"),
            "升级前要等 rove.exe 退出"
        );
        assert!(
            iss.contains("OutputBaseFilename=Rove-{#RoveVersion}-win-x64-setup"),
            "产物名要与 cargo xtask release 的期望一致"
        );
        assert!(iss.contains("SetupIconFile=rove.ico"));
        assert!(
            util::repo_root()
                .join("packaging/windows/rove.ico")
                .is_file(),
            "安装器图标必须已生成并提交（cargo xtask icon）"
        );
        // 安装器不联网：没有任何下载指令。
        for forbidden in [
            "DownloadTemporaryFile",
            "idpAddFile",
            "http://",
            "https://download",
        ] {
            assert!(
                !iss.contains(forbidden),
                "安装器不得联网，发现 `{forbidden}`"
            );
        }
    }

    #[test]
    fn optional_tasks_default_to_unchecked() {
        let iss = installer_script();
        let mut tasks = 0;
        for line in section(&iss, "[Tasks]") {
            if !line.starts_with("Name:") {
                continue;
            }
            tasks += 1;
            assert!(
                line.contains("Flags: unchecked"),
                "可选项必须默认不勾选：{line}"
            );
        }
        assert!(
            tasks >= 4,
            "至少有桌面快捷方式 / 文件关联 / 文件与目录右键菜单四项"
        );
    }

    /// 文件关联只加 `OpenWithProgids`（出现在「打开方式」里），不写扩展名的默认值，
    /// 所以不会抢默认程序；每一项都带 uninsdelete*，卸载时收回。
    #[test]
    fn file_associations_never_take_over_the_default_program() {
        let iss = installer_script();
        let mut associations = 0;
        for line in section(&iss, "[Registry]") {
            if !line.starts_with("Root:") {
                continue;
            }
            assert!(line.contains("Root: HKCU"), "只写 HKCU：{line}");
            assert!(
                line.contains("uninsdeletekey")
                    || line.contains("uninsdeletevalue")
                    // 同一个键下的附加值随 uninsdeletekey 一起删。
                    || line.contains(r#"ValueName: "Icon""#),
                "注册表项必须在卸载时清除：{line}"
            );
            if line.contains("OpenWithProgids") {
                associations += 1;
                assert!(line.contains("Tasks: fileassoc"), "{line}");
            }
            // 扩展名键的默认值（ValueName: ""）= 抢默认程序，绝不允许。
            let extension_default = line.contains(r#"Software\Classes\."#)
                && line.contains(r#"ValueName: """#)
                && !line.contains("OpenWithProgids");
            assert!(!extension_default, "不得写扩展名的默认值：{line}");
        }
        assert!(associations >= 6, "至少关联 .cs/.json/.yaml/.xml/.md/.log");
        // 右键菜单：文件用 %1，目录用 %V（把目录作为启动参数打开工作区）。
        assert!(iss.contains(r#"Software\Classes\*\shell\Rove\command"#));
        assert!(iss.contains(r#"Software\Classes\Directory\shell\Rove\command"#));
        assert!(iss.contains(r#"Software\Classes\Directory\Background\shell\Rove\command"#));
        assert!(
            iss.contains(r#"""%V"""#),
            "目录项把路径作为启动参数传给 rove.exe"
        );
    }

    /// 卸载默认保留用户数据，勾选后才删（issue #76）。
    #[test]
    fn uninstaller_keeps_user_data_unless_the_box_is_checked() {
        let iss = installer_script();
        assert!(
            iss.contains("RemoveUserDataCheckBox.Checked := False"),
            "「同时删除用户数据」默认不勾"
        );
        assert!(
            iss.contains(r"DelTree(ExpandConstant('{localappdata}\Rove')"),
            "勾选后删除 %LOCALAPPDATA%\\Rove"
        );
        assert!(
            iss.contains("if UninstallSilent() then"),
            "静默卸载不问，按默认保留"
        );
    }

    /// `.iss` 里取某一节的行（跳过注释与空行）。
    fn section<'a>(iss: &'a str, name: &str) -> Vec<&'a str> {
        let mut out = Vec::new();
        let mut inside = false;
        for line in iss.lines() {
            let line = line.trim();
            if line.starts_with('[') && line.ends_with(']') {
                inside = line == name;
                continue;
            }
            if inside && !line.is_empty() && !line.starts_with(';') && !line.starts_with('#') {
                out.push(line);
            }
        }
        out
    }

    #[test]
    fn silent_args_are_inno_setups_unattended_switches() {
        // #149 的静默升级直接照 latest.json 里的参数调用安装器。
        assert!(SILENT_ARGS.contains(&"/VERYSILENT"));
        assert!(SILENT_ARGS.contains(&"/SUPPRESSMSGBOXES"));
        assert!(SILENT_ARGS.contains(&"/NORESTART"));
    }
}
