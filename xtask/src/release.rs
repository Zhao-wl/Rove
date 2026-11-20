//! `cargo xtask release`：本地发布流水线（issue #76）。
//!
//! 一条命令把一次发布需要的产物全部做出来，放在 `target/release-dist/<版本>/`：
//!
//! ```text
//! Rove-<版本>-win-x64.zip            便携版（解压即用）
//! Rove-<版本>-win-x64-setup.exe      安装包（需要 Inno Setup 6 的 ISCC.exe；没装就跳过）
//! rove-indexer-cs-<版本>-win-x64.zip         手动安装的 C# 索引组件
//! rove-indexer-cs-<版本>-win-x64-update.zip  更新器使用的 C# 索引组件
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

use serde::{Deserialize, Serialize};
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

#[derive(Serialize, Deserialize)]
struct Manifest {
    schema: u32,
    product: String,
    channel: String,
    version: String,
    released: String,
    commit: String,
    notes_url: String,
    installer: Option<Artifact>,
    portable: Artifact,
    components: Vec<Component>,
}

#[derive(Serialize, Deserialize)]
struct Artifact {
    file: String,
    url: String,
    size: u64,
    sha256: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    silent_args: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Component {
    id: String,
    version: String,
    file: String,
    url: String,
    size: u64,
    sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manual: Option<Artifact>,
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

    // 组件包在**构建之前**就解析并判定：正式发布漏传 `--component` 时当场失败，
    // 而不是等十几分钟构建完才发现清单里 `components` 是空的（issue #238）。
    let component_source = component_zip(args)?;
    match (
        &component_source,
        missing_component_policy(&version, &channel, util::has_flag(args, "--no-component")),
    ) {
        (Some(path), _) => println!("  C# 索引组件：{}", path.display()),
        (None, MissingComponent::Fail) => return Err(missing_component_error(&channel).into()),
        (None, MissingComponent::Warn) => {
            println!("::warning::没传 --component：latest.json 的 components 会是空数组，");
            println!(
                "  安装器里也不会出现「安装 C# 索引组件」勾选项；已装组件的用户拿不到组件更新。"
            );
        }
    }

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
    // 组件包先收进产物目录：发布附件就是这个目录里的文件（Actions 与 upload.ps1 都照它上传），
    // 组件留在 target\components 下就会出现「清单里有、附件里没有」的发布（issue #238）。
    // 安装器里那个「安装 C# 索引组件」勾选项是 rove.iss 按 /DComponentZip 条件编译出来的，
    // 所以这一步必须在建安装包**之前**完成（issue #76 §5）。
    let component = match &component_source {
        Some(path) => Some(stage_component(path, &out)?),
        None => {
            println!("  没传 --component：安装器里不会出现「安装 C# 索引组件」勾选项");
            None
        }
    };
    if let Some(path) = &component {
        println!("  随包携带 C# 索引组件：{}", path.display());
    }
    let update_component = component
        .as_deref()
        .map(|manual| stage_component_update(manual, &out))
        .transpose()?;
    let installer = match find_iscc() {
        Some(iscc) => Some(build_installer(
            &iscc,
            &version,
            &stage,
            &out,
            &info,
            component.as_deref(),
        )?),
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
    let mut artifacts: Vec<PathBuf> = vec![zip.clone()];
    artifacts.extend(installer.clone());
    artifacts.extend(component.clone());
    artifacts.extend(update_component.clone());

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
        update_component.as_deref(),
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

    // 自检：清单引用的每个文件都在产物目录里（大小与 SHA-256 对得上），
    // 目录里的每个发布附件也都在清单里（issue #238）。
    for line in verify_attachments(&out, &manifest_path)? {
        println!("  {line}");
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
    // 用户级安装（`winget install JRSoftware.InnoSetup --scope user` 的落点）。
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        let path = PathBuf::from(dir)
            .join("Programs")
            .join("Inno Setup 6")
            .join("ISCC.exe");
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// 解析 `--component <zip>`，并化成**绝对路径**。
///
/// rove.iss 的 `[Files]` 用 `{#ComponentZip}` 原样取文件，相对路径会被当成相对 .iss 所在目录，
/// 所以这里统一按当前工作目录补全（其余路径参数的习惯一样）。
fn component_zip(args: &[String]) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let Some(raw) = util::flag(args, "--component") else {
        return Ok(None);
    };
    let path = PathBuf::from(&raw);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    if !path.is_file() {
        return Err(format!("--component：找不到组件包 {}", path.display()).into());
    }
    Ok(Some(path))
}

/// 漏传 `--component` 时怎么办（issue #238）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissingComponent {
    /// 正式发布：直接失败，不产出一份组件列表为空的清单。
    Fail,
    /// 预发布 / 自定义频道 / 明确 `--no-component`：只警告，照常产出。
    Warn,
}

/// 正式发布（`stable` 频道 + 没有预发布后缀的版本号）漏了组件包就是个发布事故：
/// 清单里的 `components` 为空，已经装了索引组件的用户永远等不到这次更新（issue #238）。
/// 真的要发一版不带组件的，就显式写 `--no-component`。
fn missing_component_policy(version: &str, channel: &str, opted_out: bool) -> MissingComponent {
    if opted_out || channel != "stable" || version.contains('-') {
        MissingComponent::Warn
    } else {
        MissingComponent::Fail
    }
}

fn missing_component_error(channel: &str) -> String {
    format!(
        "正式发布（{channel} 频道）必须带上 C# 索引组件包，否则 latest.json 的 components 是空的，\
         已安装组件的用户拿不到这次更新（issue #238）。\n\
         先打组件包：powershell -NoProfile -File scripts\\components\\package-indexer.ps1\n\
         再传给本命令：cargo xtask release --component target\\components\\rove-indexer-cs-<版本>-win-x64.zip\n\
         确实要发一版不带组件的，显式加 --no-component。"
    )
}

/// 把组件包收进产物目录：**发布附件 = 产物目录里的文件**，组件留在 `target\components`
/// 下就会漏传（Actions 的 `gh release create` 只扫产物目录）。已经在目录里就原地用。
fn stage_component(src: &Path, out: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let name = src
        .file_name()
        .ok_or_else(|| format!("--component：{} 没有文件名", src.display()))?;
    let dest = out.join(name);
    if dest != *src {
        fs::copy(src, &dest).map_err(|e| format!("复制组件包到 {} 失败：{e}", dest.display()))?;
    }
    Ok(dest)
}

fn component_version(path: &Path) -> Result<&str, Box<dyn std::error::Error>> {
    let version = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("rove-indexer-cs-"))
        .and_then(|name| name.strip_suffix("-win-x64.zip"))
        .ok_or("组件包名应为 rove-indexer-cs-<版本>-win-x64.zip")?;
    if version.split('.').count() != 4
        || !version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(format!("组件包的版本号无效：{version}").into());
    }
    Ok(version)
}

/// 已发布的更新器要求根目录包含 component.json。手动包与 Inno 继续用版本目录。
fn stage_component_update(
    manual: &Path,
    out: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let version = component_version(manual)?;
    let dest = out.join(format!("rove-indexer-cs-{version}-win-x64-update.zip"));
    let script = util::repo_root().join("scripts/components/package-update-archive.ps1");
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .arg("-Source")
        .arg(manual)
        .arg("-Destination")
        .arg(&dest)
        .args(["-Version", version])
        .status()
        .map_err(|error| format!("生成自动更新组件包失败：{error}"))?;
    if !status.success() {
        return Err("生成自动更新组件包失败".into());
    }
    Ok(dest)
}

/// 发布附件与签名清单必须一一对应（issue #238 的验收标准）。
///
/// 两个方向都查，少了哪一边都是事故：
/// - 清单里引用的每个文件（安装包、便携版、每个组件）都在产物目录里，**大小与 SHA-256 一致**，
///   并且在 `SHA256SUMS.txt` 里有同样的校验值；
/// - 产物目录里的每个发布附件（`.zip` / `.exe`）都被清单引用——组件包只在附件里、
///   不在清单里，正是 tag 工作流漏传 `--component` 时的样子。
pub fn verify_attachments(
    dir: &Path,
    manifest_path: &Path,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let bytes = fs::read(manifest_path)
        .map_err(|e| format!("读取 {} 失败：{e}", manifest_path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{} 不是合法的更新清单：{e}", manifest_path.display()))?;

    let sums_path = dir.join("SHA256SUMS.txt");
    let sums = util::read_text(&sums_path)
        .map_err(|e| format!("读取 {} 失败：{e}", sums_path.display()))?;

    let mut referenced: Vec<(String, u64, String, String)> = Vec::new();
    if let Some(installer) = &manifest.installer {
        referenced.push((
            installer.file.clone(),
            installer.size,
            installer.sha256.clone(),
            "安装包".into(),
        ));
    }
    referenced.push((
        manifest.portable.file.clone(),
        manifest.portable.size,
        manifest.portable.sha256.clone(),
        "便携版".into(),
    ));
    for component in &manifest.components {
        referenced.push((
            component.file.clone(),
            component.size,
            component.sha256.clone(),
            format!("组件 {} {}", component.id, component.version),
        ));
        if let Some(manual) = &component.manual {
            referenced.push((
                manual.file.clone(),
                manual.size,
                manual.sha256.clone(),
                format!("手动组件 {} {}", component.id, component.version),
            ));
        }
    }

    let mut report = Vec::new();
    for (file, size, digest, what) in &referenced {
        let path = dir.join(file);
        if !path.is_file() {
            return Err(format!(
                "清单引用了 {file}（{what}），但发布附件里没有它：{} 不存在",
                path.display()
            )
            .into());
        }
        let actual_size = fs::metadata(&path)?.len();
        if actual_size != *size {
            return Err(
                format!("{file}（{what}）的大小是 {actual_size}，清单里写的是 {size}").into(),
            );
        }
        let actual = sha256_file(&path)?;
        if actual != *digest {
            return Err(format!("{file}（{what}）的 SHA-256 与清单不一致").into());
        }
        if !sums
            .lines()
            .any(|l| l.trim() == format!("{digest}  {file}"))
        {
            return Err(format!("SHA256SUMS.txt 里没有 {file}（{what}）的校验值").into());
        }
        report.push(format!("清单 ⟷ 附件一致：{file}（{what}，{size} 字节）"));
    }

    // 反向：目录里的发布附件不能有清单没提到的。
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_attachment = name.ends_with(".zip") || name.ends_with(".exe");
        if is_attachment && !referenced.iter().any(|(file, ..)| *file == name) {
            return Err(format!(
                "{name} 会被当成发布附件上传，但签名清单里没有它\
                 （组件包进了附件却没进清单，正是 issue #238 的 bug）"
            )
            .into());
        }
    }
    Ok(report)
}

/// ISCC 的 `/D` 参数（不含脚本路径本身）。单独拆出来是为了能在没装 Inno Setup 的机器上测。
fn iscc_defines(
    version: &str,
    stage: &Path,
    out: &Path,
    info: &BuildInfo,
    component: Option<&Path>,
) -> Vec<String> {
    let mut defines = vec![
        format!("/DRoveVersion={version}"),
        format!("/DRoveCommit={}", info.short_commit),
        format!("/DStageDir={}", stage.display()),
        format!("/DOutputDir={}", out.display()),
    ];
    // 只有带着组件包时才定义 ComponentZip：没定义时 rove.iss 把「安装 C# 索引组件」
    // 整段跳过，安装器里就不会出现一个点了没用的勾选项（issue #76 §5）。
    if let Some(zip) = component {
        defines.push(format!("/DComponentZip={}", zip.display()));
    }
    defines
}

fn build_installer(
    iscc: &Path,
    version: &str,
    stage: &Path,
    out: &Path,
    info: &BuildInfo,
    component: Option<&Path>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let script = util::repo_root().join("packaging/windows/rove.iss");
    let status = Command::new(iscc)
        .args(iscc_defines(version, stage, out, info, component))
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
    update_component: Option<&Path>,
    manual_component: Option<&Path>,
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
    let components = match update_component {
        Some(path) => {
            let version = component_version(manual_component.unwrap_or(path))?.to_owned();
            let package = artifact(path, false)?;
            vec![Component {
                id: "indexer-cs".into(),
                version,
                file: package.file,
                url: package.url,
                size: package.size,
                sha256: package.sha256,
                manual: manual_component
                    .map(|path| artifact(path, false))
                    .transpose()?,
            }]
        }
        None => Vec::new(),
    };
    Ok(Manifest {
        schema: 1,
        product: "rove".into(),
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
         - **C# 索引组件**（可选，单独下载）：将 `rove-indexer-cs-<版本>-win-x64.zip` 解压到 `%LOCALAPPDATA%\\Rove\\components\\indexer-cs\\`，\n\
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
         含索引缓存与设置备份）默认保留，卸载时会询问是否一并删除（默认「否」）。\n\
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

    /// 卸载默认保留用户数据，卸载前确认框选「是」才删（issue #76）。
    #[test]
    fn uninstaller_keeps_user_data_unless_the_user_confirms() {
        let iss = installer_script();
        assert!(
            iss.contains("RemoveUserData := False;") && iss.contains("MB_YESNO or MB_DEFBUTTON2"),
            "「同时删除用户数据」默认不删，确认框的默认按钮是「否」"
        );
        assert!(
            iss.contains("function InitializeUninstall(): Boolean;"),
            "卸载开始之前就问（进度页上的勾选框来不及勾）"
        );
        assert!(
            iss.contains(r"DelTree(ExpandConstant('{localappdata}\Rove')"),
            "确认后删除 %LOCALAPPDATA%\\Rove"
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

    /// `--component` 必须真的传到 ISCC：rove.iss 用 `#if ComponentZip != ""` 把
    /// 「安装 C# 索引组件」整段包了起来，少了这个 `/D` 勾选项就永远不会出现。
    #[test]
    fn component_zip_reaches_iscc_as_a_define() {
        let info = BuildInfo {
            commit: "0".repeat(40),
            short_commit: "0000000".into(),
            date: "2026-11-14".into(),
            dirty: false,
        };
        let stage = PathBuf::from(r"C:\stage");
        let out = PathBuf::from(r"C:\out");

        let without = iscc_defines("0.1.0", &stage, &out, &info, None);
        assert!(
            !without.iter().any(|a| a.starts_with("/DComponentZip=")),
            "没传 --component 时不能定义 ComponentZip，否则会生成一个装不上东西的勾选项"
        );

        let zip = PathBuf::from(r"C:\components\rove-indexer-cs-1.2.0.0-win-x64.zip");
        let with = iscc_defines("0.1.0", &stage, &out, &info, Some(&zip));
        assert!(
            with.contains(&format!("/DComponentZip={}", zip.display())),
            "带 --component 时必须把绝对路径传给 ISCC：{with:?}"
        );
        // rove.iss 里那一段确实是靠这个符号开关的。
        let iss = installer_script();
        assert!(iss.contains("#if ComponentZip != \"\""));
        assert!(
            iss.contains("Name: \"indexer\";"),
            "「安装 C# 索引组件」勾选项在 [Tasks] 里"
        );
    }

    /// 相对路径要补成绝对路径（ISPP 会把相对路径当成相对 .iss 所在目录），
    /// 路径不存在时要当场报错，而不是等到第 5 步算 SHA256 才炸。
    #[test]
    fn component_flag_requires_an_existing_file() {
        let missing = util::target_dir().join("no-such-component.zip");
        let err = component_zip(&[format!("--component={}", missing.display())])
            .expect_err("不存在的组件包必须报错");
        assert!(err.to_string().contains("找不到组件包"), "{err}");

        assert!(
            component_zip(&[]).unwrap().is_none(),
            "不传 --component 时是 None"
        );

        let file = util::target_dir().join("component-flag-test.zip");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"not a real zip").unwrap();
        let resolved = component_zip(&[format!("--component={}", file.display())])
            .unwrap()
            .unwrap();
        assert!(
            resolved.is_absolute(),
            "传给 ISCC 的必须是绝对路径：{}",
            resolved.display()
        );
        assert_eq!(resolved.file_name(), file.file_name());

        // 相对路径按当前工作目录补全。
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let here = std::env::current_dir().unwrap();
        if let Ok(rest) = file.strip_prefix(&here) {
            let resolved = component_zip(&[format!("--component={}", rest.display())])
                .unwrap()
                .unwrap();
            assert!(resolved.is_absolute());
            assert_eq!(resolved.file_name().unwrap().to_string_lossy(), name);
        }
        fs::remove_file(&file).ok();
    }

    // ------------------------------------------------- issue #238：组件进清单与附件

    fn fake_info() -> BuildInfo {
        BuildInfo {
            commit: "0".repeat(40),
            short_commit: "000000000000".into(),
            date: "2026-11-17".into(),
            dirty: false,
        }
    }

    /// 在 `dir` 里造一份假的产物（内容无所谓，校验的是大小与摘要的一致性）。
    fn fake_artifact(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    /// 按 `run()` 的做法写出 SHA256SUMS.txt 与 latest.json。
    fn write_release(
        out: &Path,
        zip: &Path,
        installer: Option<&Path>,
        component: Option<&Path>,
    ) -> PathBuf {
        write_release_manifest(out, zip, installer, component, None)
    }

    fn write_release_manifest(
        out: &Path,
        zip: &Path,
        installer: Option<&Path>,
        component: Option<&Path>,
        manual_component: Option<&Path>,
    ) -> PathBuf {
        let mut artifacts = vec![zip.to_path_buf()];
        artifacts.extend(installer.map(Path::to_path_buf));
        artifacts.extend(component.map(Path::to_path_buf));
        artifacts.extend(manual_component.map(Path::to_path_buf));
        let mut sums = String::new();
        for path in &artifacts {
            sums.push_str(&format!(
                "{}  {}\n",
                sha256_file(path).unwrap(),
                path.file_name().unwrap().to_string_lossy()
            ));
        }
        fs::write(out.join("SHA256SUMS.txt"), &sums).unwrap();
        let manifest = build_manifest(
            "0.1.0",
            "stable",
            &fake_info(),
            "https://github.com/Zhao-wl/Rove/releases/tag/v0.1.0",
            "https://github.com/Zhao-wl/Rove/releases/download/v0.1.0",
            zip,
            installer,
            component,
            manual_component,
        )
        .unwrap();
        let path = out.join("latest.json");
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
        )
        .unwrap();
        path
    }

    #[cfg(windows)]
    #[test]
    fn a_packaged_component_can_upgrade_older_clients_without_changing_manual_layout() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        let version = "1.4.0.0";
        let payload = out.join(version);
        fs::create_dir_all(payload.join("lang")).unwrap();
        let metadata = br#"{"component":"indexer-cs","version":"1.4.0.0"}"#;
        fs::write(payload.join("component.json"), metadata).unwrap();
        fs::write(payload.join("rove-indexer-cs.exe"), b"indexer").unwrap();
        fs::write(payload.join("lang/resources.dll"), b"resources").unwrap();
        let manual = out.join(format!("rove-indexer-cs-{version}-win-x64.zip"));
        compress(&payload, &manual).unwrap();
        let original = fs::read(&manual).unwrap();
        let update = stage_component_update(&manual, out).unwrap();
        assert_eq!(fs::read(&manual).unwrap(), original);

        let portable = fake_artifact(out, "Rove-0.1.0-win-x64.zip", b"portable");
        let manifest = write_release_manifest(out, &portable, None, Some(&update), Some(&manual));
        let bytes = fs::read(&manifest).unwrap();
        let client = rove_update::manifest::Manifest::parse(&bytes).unwrap();
        let component = client.indexer_component().unwrap();
        assert_eq!(component.version, version);
        assert_eq!(
            component.file,
            format!("rove-indexer-cs-{version}-win-x64-update.zip")
        );
        let update_bytes = fs::read(out.join(&component.file)).unwrap();
        assert_eq!(component.sha256, sha256(&update_bytes));
        let installed = rove_update::component::Installed {
            root: out.join("profile"),
            version: Some("1.3.0.0".into()),
            manifest_file: "component.json".into(),
        };
        let target =
            rove_update::component::install_archive(&update_bytes, &installed, version).unwrap();
        assert_eq!(target, installed.root.join(version));
        assert_eq!(fs::read(target.join("component.json")).unwrap(), metadata);
        assert_eq!(
            fs::read(target.join("rove-indexer-cs.exe")).unwrap(),
            b"indexer"
        );
        assert_eq!(
            fs::read(target.join("lang/resources.dll")).unwrap(),
            b"resources"
        );
        assert_eq!(verify_attachments(out, &manifest).unwrap().len(), 3);

        fs::write(&manual, b"modified manual archive").unwrap();
        let error = verify_attachments(out, &manifest).unwrap_err().to_string();
        assert!(error.contains("手动组件"), "{error}");
    }

    /// 一次带组件的发布：组件包进了附件、也进了清单，版本 / 摘要 / 大小都对得上。
    #[test]
    fn a_release_with_a_component_lists_it_in_both_the_attachments_and_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        let zip = fake_artifact(out, "Rove-0.1.0-win-x64.zip", b"portable");
        let installer = fake_artifact(out, "Rove-0.1.0-win-x64-setup.exe", b"installer");
        let component = fake_artifact(
            out,
            "rove-indexer-cs-1.3.0.0-win-x64.zip",
            b"indexer component package",
        );
        let manifest_path = write_release(out, &zip, Some(&installer), Some(&component));

        let report = verify_attachments(out, &manifest_path).expect("附件与清单应一致");
        assert_eq!(report.len(), 3, "安装包 + 便携版 + 组件：{report:?}");

        let text = util::read_text(&manifest_path).unwrap();
        let parsed: Manifest = serde_json::from_str(&text).unwrap();
        let listed = &parsed.components[0];
        assert_eq!(listed.id, "indexer-cs");
        assert_eq!(listed.version, "1.3.0.0", "版本号从包名解析");
        assert_eq!(listed.size, fs::metadata(&component).unwrap().len());
        assert_eq!(listed.sha256, sha256_file(&component).unwrap());
        assert!(listed.url.ends_with("rove-indexer-cs-1.3.0.0-win-x64.zip"));
    }

    /// issue #238 的现场：组件包是发布附件，却没进签名清单（tag 工作流漏传 `--component`）。
    #[test]
    fn a_component_attachment_missing_from_the_manifest_fails() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        let zip = fake_artifact(out, "Rove-0.1.0-win-x64.zip", b"portable");
        // 清单按「没传 --component」生成，组件包却躺在产物目录里等着被上传。
        let manifest_path = write_release(out, &zip, None, None);
        fake_artifact(out, "rove-indexer-cs-1.3.0.0-win-x64.zip", b"component");

        let err = verify_attachments(out, &manifest_path).expect_err("清单漏了组件必须失败");
        assert!(err.to_string().contains("rove-indexer-cs"), "{err}");
    }

    /// 反过来：清单里写了组件，附件里却没有（组件包留在 target\components 没收进产物目录）。
    #[test]
    fn a_manifest_component_without_an_attachment_fails() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        let elsewhere = dir.path().join("components");
        fs::create_dir_all(&elsewhere).unwrap();
        let zip = fake_artifact(out, "Rove-0.1.0-win-x64.zip", b"portable");
        let component = fake_artifact(&elsewhere, "rove-indexer-cs-1.3.0.0-win-x64.zip", b"c");
        let manifest_path = write_release(out, &zip, None, Some(&component));

        let err = verify_attachments(out, &manifest_path).expect_err("附件里没有组件必须失败");
        assert!(err.to_string().contains("发布附件里没有它"), "{err}");

        // `stage_component` 就是用来堵这个洞的：收进产物目录后再校验就通过了。
        let staged = stage_component(&component, out).unwrap();
        assert_eq!(staged, out.join("rove-indexer-cs-1.3.0.0-win-x64.zip"));
        let manifest_path = write_release(out, &zip, None, Some(&staged));
        verify_attachments(out, &manifest_path).expect("收进产物目录后应一致");
        // 已经在产物目录里的包不会被复制到自己身上。
        assert_eq!(stage_component(&staged, out).unwrap(), staged);
    }

    /// 清单与附件的大小 / 摘要对不上（比如重新打了包却没重算清单）也要被拦下来。
    #[test]
    fn a_tampered_attachment_fails_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        let zip = fake_artifact(out, "Rove-0.1.0-win-x64.zip", b"portable");
        let component = fake_artifact(out, "rove-indexer-cs-1.3.0.0-win-x64.zip", b"component");
        let manifest_path = write_release(out, &zip, None, Some(&component));
        fs::write(&component, b"component v2").unwrap();

        let err = verify_attachments(out, &manifest_path).expect_err("改了包就必须失败");
        assert!(err.to_string().contains("大小"), "{err}");
    }

    /// 正式发布漏传 `--component` 必须报错；预发布 / 显式 `--no-component` 只警告。
    #[test]
    fn a_formal_release_refuses_to_drop_the_component() {
        assert_eq!(
            missing_component_policy("0.1.0", "stable", false),
            MissingComponent::Fail,
            "正式版本号 + stable 频道漏了组件 = 发布事故（issue #238）"
        );
        assert_eq!(
            missing_component_policy("0.1.0", "stable", true),
            MissingComponent::Warn,
            "--no-component 是明确的选择，放行但要警告"
        );
        assert_eq!(
            missing_component_policy("0.2.0-rc1", "stable", false),
            MissingComponent::Warn,
            "预发布版本号只警告"
        );
        assert_eq!(
            missing_component_policy("0.1.0", "nightly", false),
            MissingComponent::Warn,
            "非 stable 频道只警告"
        );
        let message = missing_component_error("stable");
        assert!(message.contains("package-indexer.ps1"), "要给出下一步做法");
        assert!(message.contains("--no-component"));
    }

    /// 验收标准第二条的客户端一侧：用**本流水线产出的清单**喂 #149 的更新端，
    /// 已装组件的机器能更新，没装的机器不会被自动装上。
    #[test]
    fn installed_components_update_while_absent_ones_are_left_alone() {
        use rove_update::component::{plan, Installed, Plan};

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path();
        let zip = fake_artifact(out, "Rove-0.1.0-win-x64.zip", b"portable");
        let component = fake_artifact(out, "rove-indexer-cs-1.3.0.0-win-x64.zip", b"component");
        let manifest_path = write_release(out, &zip, None, Some(&component));
        verify_attachments(out, &manifest_path).unwrap();

        // 更新端读的是清单的**字节**（签名覆盖的也是这些字节）。
        let bytes = fs::read(&manifest_path).unwrap();
        let manifest = rove_update::Manifest::parse(&bytes).expect("#149 的解析器要认这份清单");
        let listed = manifest
            .indexer_component()
            .expect("清单里必须有 indexer-cs");
        assert_eq!(listed.version, "1.3.0.0");
        assert_eq!(listed.sha256, sha256_file(&component).unwrap());
        assert_eq!(listed.size, fs::metadata(&component).unwrap().len());

        let root = dir.path().join("components").join("indexer-cs");
        let installed = Installed {
            root: root.clone(),
            version: Some("1.2.0.0".into()),
            manifest_file: "component.json".into(),
        };
        assert!(
            matches!(plan(Some(listed), &installed), Plan::Update(c) if c.version == "1.3.0.0"),
            "已装 1.2.0.0 的机器应该能更新到清单里的 1.3.0.0"
        );

        let not_installed = Installed {
            root,
            version: None,
            manifest_file: "component.json".into(),
        };
        assert_eq!(
            plan(Some(listed), &not_installed),
            Plan::NotInstalled,
            "没装组件的机器不会被自动装上（#43 的组件是可选的）"
        );
    }

    #[test]
    fn silent_args_are_inno_setups_unattended_switches() {
        // #149 的静默升级直接照 latest.json 里的参数调用安装器。
        assert!(SILENT_ARGS.contains(&"/VERYSILENT"));
        assert!(SILENT_ARGS.contains(&"/SUPPRESSMSGBOXES"));
        assert!(SILENT_ARGS.contains(&"/NORESTART"));
    }
}
