//! 路径与小工具。所有路径都从仓库根目录推出来，不依赖当前工作目录。

use std::path::{Path, PathBuf};

/// 仓库根目录（`xtask/` 的父目录）。
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask 必须位于仓库根目录下")
        .to_path_buf()
}

/// 测试数据的默认目录：`target/perf-testdata`（target 已被 .gitignore 忽略）。
pub fn default_testdata_dir() -> PathBuf {
    repo_root().join("target").join("perf-testdata")
}

/// cargo 的 target 目录，criterion 的结果写在这里。
pub fn target_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => repo_root().join("target"),
    }
}

/// 读取 UTF-8 文本，容忍 Windows 编辑器可能写进去的 BOM
/// （PowerShell 5.1 的 `Set-Content -Encoding UTF8` 就会加 BOM）。
pub fn read_text(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// 取 `--name value` 或 `--name=value` 的值。
pub fn flag(args: &[String], name: &str) -> Option<String> {
    let with_eq = format!("{name}=");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == name {
            return iter.next().cloned();
        }
        if let Some(value) = arg.strip_prefix(&with_eq) {
            return Some(value.to_string());
        }
    }
    None
}

/// 是否存在开关型参数。
pub fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

/// 人类可读的字节数（B / KiB / MiB / GiB）。
pub fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KIB * KIB * KIB {
        format!("{:.2} GiB", value / (KIB * KIB * KIB))
    } else if value >= KIB * KIB {
        format!("{:.2} MiB", value / (KIB * KIB))
    } else if value >= KIB {
        format!("{:.2} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// 带千分位的整数，便于对比行数。
pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
