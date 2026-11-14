//! #29：Markdown 预览的打包资源在构建时用 DEFLATE 压缩后嵌入 `rove.exe`（Mermaid 等合计约 4.3 MB，
//! 压缩后约 1.3 MB），请求时在内存里解压（`markdown::page::asset`）。woff2 字体本身已压缩，原样嵌入。
use std::{fs, io::Write, path::Path};

fn main() {
    let source = Path::new("assets/markdown");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("markdown");
    println!("cargo:rerun-if-changed=assets/markdown");
    compress_dir(source, source, &out);
    build_info();
}

/// #76：「关于」对话框要显示的构建信息。
///
/// 版本号来自 `CARGO_PKG_VERSION`（工作区唯一来源）；提交号与构建日期优先用发布流水线注入的
/// `ROVE_BUILD_COMMIT` / `ROVE_BUILD_DATE`（`cargo xtask release` 会设），本地构建时现场问 git。
/// gpui 的版本从锁定的依赖声明里读，不在代码里再写一遍。
///
/// 这里**故意不**对 `.git/HEAD` 声明 `rerun-if-changed`：那会让每次提交都重编 rove-ui。
/// 代价是开发构建里的提交号可能停在上一次构建脚本运行时的值；发布流水线显式设环境变量，
/// 环境变量一变构建脚本就重跑，所以**发布产物里的提交号总是准确的**。
fn build_info() {
    for key in ["ROVE_BUILD_COMMIT", "ROVE_BUILD_DATE", "ROVE_BUILD_CHANNEL"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let git = |args: &[&str]| -> Option<String> {
        std::process::Command::new("git")
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let value = |key: &str, fallback: Option<String>| {
        std::env::var(key)
            .ok()
            .filter(|v| !v.is_empty())
            .or(fallback)
            .unwrap_or_else(|| "unknown".into())
    };
    let commit = value("ROVE_BUILD_COMMIT", git(&["rev-parse", "HEAD"]));
    let date = value(
        "ROVE_BUILD_DATE",
        git(&["log", "-1", "--date=format:%Y-%m-%d", "--format=%cd"]),
    );
    let channel = value("ROVE_BUILD_CHANNEL", Some("dev".into()));
    println!("cargo:rustc-env=ROVE_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=ROVE_BUILD_DATE={date}");
    println!("cargo:rustc-env=ROVE_BUILD_CHANNEL={channel}");
    println!("cargo:rustc-env=ROVE_GPUI_VERSION={}", gpui_version());
}

/// 根 `Cargo.toml` 里 `gpui = { version = "=0.2.2", … }` 的版本号。
fn gpui_version() -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|repo| repo.join("Cargo.toml"));
    let Some(manifest) = manifest else {
        return "unknown".into();
    };
    println!("cargo:rerun-if-changed={}", manifest.display());
    let Ok(text) = fs::read_to_string(&manifest) else {
        return "unknown".into();
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("gpui = ") {
            if let Some(start) = rest.find("version = \"") {
                let rest = &rest[start + "version = \"".len()..];
                if let Some(end) = rest.find('"') {
                    return rest[..end].trim_start_matches('=').to_string();
                }
            }
        }
    }
    "unknown".into()
}

fn compress_dir(root: &Path, dir: &Path, out: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "LICENSES") {
                continue;
            }
            compress_dir(root, &path, out);
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let relative = path.strip_prefix(root).unwrap();
        let target = out.join(relative).with_extension(format!(
            "{}.deflate",
            path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
        ));
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let bytes = fs::read(&path).unwrap();
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(&bytes).unwrap();
        fs::write(&target, encoder.finish().unwrap()).unwrap();
    }
}
