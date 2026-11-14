//! `cargo xtask licenses`：Rust 依赖的许可汇总与守护（issue #76 第 6 节）。
//!
//! - 许可信息直接取自 `cargo metadata`（不联网解析网页，也不需要额外安装工具）；
//! - 允许的许可写在 `packaging/licenses/allowed.toml`；表达式按 SPDX 语义判定：
//!   `A OR B` 只要有一边被允许就行，`A AND B` 两边都要被允许。
//!   所以 `Apache-2.0 OR GPL-2.0-only` 可以用（走 Apache 那边），而纯 GPL 依赖会直接失败；
//! - 汇总表生成到 `packaging/licenses/rust-dependencies.md`（提交进仓库，由
//!   `THIRD_PARTY_NOTICES.md` 引用）。`--check` 发现它与当前依赖图不一致就失败，
//!   这样**新增一个未登记许可的依赖**（或忘了重新生成清单）都会在 CI 上被拦下。
//!
//! 用法：
//! - `cargo xtask licenses`：打印汇总；
//! - `cargo xtask licenses --write`：重新生成 `rust-dependencies.md`；
//! - `cargo xtask licenses --check`：CI 用，许可不被允许或清单过期时非零退出。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;

use crate::util;

pub const ALLOWED_PATH: &str = "packaging/licenses/allowed.toml";
pub const SUMMARY_PATH: &str = "packaging/licenses/rust-dependencies.md";
pub const SCHEMA: u32 = 1;

#[derive(Debug, Deserialize)]
struct Allowed {
    schema: u32,
    /// 允许的 SPDX 许可标识（不是表达式）。
    license: Vec<String>,
    #[serde(default)]
    exception: Vec<Exception>,
}

#[derive(Debug, Deserialize)]
struct Exception {
    /// crate 名。
    name: String,
    /// 该 crate 实际适用的许可（`cargo metadata` 里缺失或写法特殊时用）。
    license: String,
    reason: String,
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub version: String,
    pub license: String,
    pub source: String,
}

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let root = util::repo_root();
    let allowed_path = root.join(ALLOWED_PATH);
    let text = util::read_text(&allowed_path)
        .map_err(|e| format!("读取 {} 失败：{e}", allowed_path.display()))?;
    let allowed: Allowed = toml::from_str(&text)?;
    if allowed.schema != SCHEMA {
        return Err(format!("{} 的 schema 不是 {SCHEMA}", allowed_path.display()).into());
    }

    let deps = collect()?;
    println!("Rust 依赖许可汇总（issue #76）");
    println!(
        "  依赖数：{}（含 build / dev 依赖与全部 feature）",
        deps.len()
    );

    let mut rejected = Vec::new();
    for dep in &deps {
        let license = match allowed.exception.iter().find(|e| e.name == dep.name) {
            Some(exception) => {
                println!(
                    "  例外：{} 按 `{}` 登记（{}）",
                    dep.name, exception.license, exception.reason
                );
                exception.license.clone()
            }
            None => dep.license.clone(),
        };
        if !satisfiable(&license, &allowed.license) {
            rejected.push((dep.clone(), license));
        }
    }

    let mut by_license: BTreeMap<&str, usize> = BTreeMap::new();
    for dep in &deps {
        *by_license.entry(dep.license.as_str()).or_default() += 1;
    }
    for (license, count) in &by_license {
        println!("  {count:>4}  {license}");
    }

    if !rejected.is_empty() {
        for (dep, license) in &rejected {
            println!(
                "::error::依赖 {} {} 的许可 `{license}` 不在 {ALLOWED_PATH} 的允许列表里",
                dep.name, dep.version
            );
        }
        println!(
            "\n{} 个依赖的许可未登记。确认许可可接受后把它加进 {ALLOWED_PATH}（写清理由），不要绕过检查。",
            rejected.len()
        );
        return Ok(1);
    }

    let summary = render_summary(&deps);
    let summary_path = root.join(SUMMARY_PATH);
    if util::has_flag(args, "--write") {
        if let Some(parent) = summary_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&summary_path, &summary)?;
        println!("已写出 {}", summary_path.display());
        return Ok(0);
    }
    if util::has_flag(args, "--check") {
        let current = util::read_text(&summary_path).unwrap_or_default();
        if normalize(&current) != normalize(&summary) {
            println!("::error::{SUMMARY_PATH} 与当前依赖图不一致（多半是加了依赖没重新生成）");
            println!("  重新生成：cargo xtask licenses --write");
            return Ok(1);
        }
        println!("OK：全部依赖的许可都已登记，汇总表与依赖图一致。");
    }
    Ok(0)
}

/// 行尾与首尾空白不参与比较（仓库里是 LF，Windows 上读写可能带 CR）。
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").trim_end().to_string()
}

/// `cargo metadata` 里除工作区成员以外的全部包。
pub fn collect() -> Result<Vec<Dependency>, Box<dyn std::error::Error>> {
    let root = util::repo_root();
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(&root)
        .args(["metadata", "--format-version", "1", "--all-features"])
        .output()
        .map_err(|e| format!("调用 cargo metadata 失败：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .ok_or("cargo metadata 没有 workspace_members")?
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let mut out = Vec::new();
    for package in meta["packages"]
        .as_array()
        .ok_or("cargo metadata 没有 packages")?
    {
        let id = package["id"].as_str().unwrap_or_default();
        if members.contains(&id) {
            continue;
        }
        let name = package["name"].as_str().unwrap_or_default().to_string();
        let version = package["version"].as_str().unwrap_or_default().to_string();
        let license = package["license"]
            .as_str()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        // 本地路径依赖（vendor/gpui）没有 source。
        let source = match package["source"].as_str() {
            Some(_) => "crates.io".to_string(),
            None => {
                let manifest = PathBuf::from(package["manifest_path"].as_str().unwrap_or_default());
                manifest
                    .strip_prefix(&root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| "本地路径".into())
            }
        };
        out.push(Dependency {
            name,
            version,
            license,
            source,
        });
    }
    out.sort_by(|a, b| (a.name.as_str(), a.version.as_str()).cmp(&(&b.name, &b.version)));
    out.dedup_by(|a, b| a.name == b.name && a.version == b.version);
    Ok(out)
}

fn render_summary(deps: &[Dependency]) -> String {
    let mut out = String::new();
    out.push_str(
        "# Rust 依赖的许可汇总\n\n\
         由 `cargo xtask licenses --write` 从 `cargo metadata --all-features` 生成，**不要手改**。\n\
         CI 用 `cargo xtask licenses --check` 校验：许可不在 `packaging/licenses/allowed.toml` 的\n\
         允许列表里，或这份清单与依赖图不一致，都会失败。\n\n\
         清单覆盖全部 feature 的依赖图，因此包含只在构建、基准与测试时使用、**不进 `rove.exe`** 的包。\n\
         移植代码与随包资产的逐项声明在 [`THIRD_PARTY_NOTICES.md`](../../THIRD_PARTY_NOTICES.md)。\n\n",
    );
    let mut by_license: BTreeMap<&str, Vec<&Dependency>> = BTreeMap::new();
    for dep in deps {
        by_license
            .entry(dep.license.as_str())
            .or_default()
            .push(dep);
    }
    out.push_str(&format!(
        "依赖总数 {}，许可种类 {}。\n\n## 按许可汇总\n\n| 许可（SPDX） | 依赖数 |\n|---|---|\n",
        deps.len(),
        by_license.len()
    ));
    for (license, list) in &by_license {
        out.push_str(&format!("| `{license}` | {} |\n", list.len()));
    }
    out.push_str("\n## 逐个依赖\n\n| crate | 版本 | 许可（SPDX） | 来源 |\n|---|---|---|---|\n");
    for dep in deps {
        out.push_str(&format!(
            "| {} | {} | `{}` | {} |\n",
            dep.name, dep.version, dep.license, dep.source
        ));
    }
    out
}

// ---------------------------------------------------------------- SPDX 表达式

/// 这个 SPDX 表达式在允许列表下成立吗？`OR` 取其一，`AND` 两边都要满足。
pub fn satisfiable(expression: &str, allowed: &[String]) -> bool {
    // 老写法 `MIT/Apache-2.0` 等价于 OR。
    let normalized = expression.replace('/', " OR ");
    let tokens = tokenize(&normalized);
    let mut pos = 0;
    let value = parse_or(&tokens, &mut pos, allowed);
    pos == tokens.len() && value
}

fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        match ch {
            '(' | ')' => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                out.push(ch.to_string());
            }
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn parse_or(tokens: &[String], pos: &mut usize, allowed: &[String]) -> bool {
    let mut value = parse_and(tokens, pos, allowed);
    while tokens.get(*pos).map(|t| t.as_str()) == Some("OR") {
        *pos += 1;
        value |= parse_and(tokens, pos, allowed);
    }
    value
}

fn parse_and(tokens: &[String], pos: &mut usize, allowed: &[String]) -> bool {
    let mut value = parse_atom(tokens, pos, allowed);
    while tokens.get(*pos).map(|t| t.as_str()) == Some("AND") {
        *pos += 1;
        value &= parse_atom(tokens, pos, allowed);
    }
    value
}

fn parse_atom(tokens: &[String], pos: &mut usize, allowed: &[String]) -> bool {
    match tokens.get(*pos).map(|t| t.as_str()) {
        Some("(") => {
            *pos += 1;
            let value = parse_or(tokens, pos, allowed);
            if tokens.get(*pos).map(|t| t.as_str()) == Some(")") {
                *pos += 1;
            }
            value
        }
        Some(_) => {
            let mut identifier = tokens[*pos].clone();
            *pos += 1;
            // `Apache-2.0 WITH LLVM-exception` 当成一个标识。
            if tokens.get(*pos).map(|t| t.as_str()) == Some("WITH") {
                if let Some(exception) = tokens.get(*pos + 1) {
                    identifier = format!("{identifier} WITH {exception}");
                    *pos += 2;
                }
            }
            let trimmed = identifier.trim_end_matches('+').to_string();
            allowed.iter().any(|a| *a == identifier || *a == trimmed)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed() -> Vec<String> {
        [
            "MIT",
            "Apache-2.0",
            "Apache-2.0 WITH LLVM-exception",
            "Unicode-3.0",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn spdx_or_takes_the_permissive_branch() {
        let a = allowed();
        assert!(satisfiable("MIT", &a));
        assert!(satisfiable("MIT OR Apache-2.0", &a));
        assert!(
            satisfiable("Apache-2.0 OR GPL-2.0-only", &a),
            "OR 只要一边可用"
        );
        assert!(satisfiable("MIT/Apache-2.0", &a), "老写法的斜杠等价于 OR");
        assert!(satisfiable("Apache-2.0 WITH LLVM-exception OR MIT", &a));
        assert!(satisfiable("(MIT OR Apache-2.0) AND Unicode-3.0", &a));
    }

    #[test]
    fn spdx_rejects_unlisted_licenses() {
        let a = allowed();
        assert!(!satisfiable("GPL-3.0-only", &a));
        assert!(!satisfiable("GPL-2.0-only AND MIT", &a), "AND 两边都要可用");
        assert!(!satisfiable("(MIT OR Apache-2.0) AND NCSA", &a));
        assert!(!satisfiable("", &a), "空许可字段不能悄悄放过");
    }

    /// 允许列表本身要能解析，而且当前依赖图里的每个许可都已登记。
    #[test]
    fn every_dependency_license_is_registered() {
        let text = util::read_text(&util::repo_root().join(ALLOWED_PATH)).unwrap();
        let allowed: Allowed = toml::from_str(&text).unwrap();
        assert_eq!(allowed.schema, SCHEMA);
        for exception in &allowed.exception {
            assert!(
                !exception.reason.is_empty(),
                "{} 的例外没写理由",
                exception.name
            );
        }
        // 允许列表里不能混进 copyleft（它们只能作为 OR 的另一边出现）。
        for license in &allowed.license {
            assert!(
                !license.starts_with("GPL")
                    && !license.starts_with("LGPL")
                    && !license.starts_with("AGPL"),
                "{license} 不应出现在允许列表里"
            );
        }
    }
}
