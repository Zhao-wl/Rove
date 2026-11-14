//! `cargo xtask check-deps`：依赖图里不允许出现 HTTP / TLS 栈（issue #74）。
//!
//! Rove 运行时不发起任何网络请求（`docs/PROJECT_CHARTER.md` §2.1，唯一例外是自动更新模块）。
//! `gpui` 对 `gpui_http_client` 的依赖不是 optional，默认会把 `zed-reqwest` → `hyper` →
//! `rustls` → `ring` / `tokio` 整条栈拉进来。#74 用 `[patch.crates-io]` 把 `gpui_http_client`
//! 换成了 `crates/rove-gpui-http-stub`，这条检查保证它不会悄悄回来。
//!
//! 判定对象是 **workspace 全体成员**的两个范围：
//!
//! 1. `normal,build`：产品真正链接进去的依赖；
//! 2. `normal,build,dev`：再加上测试与基准的依赖。
//!
//! 两个范围都用 `--target all`（否则平台条件依赖会被整棵剪掉），用 **默认 feature**：
//! `--all-features` 会打开 `macos-blade` / `screen-capture` 这类我们根本不启用的 gpui feature，
//! 把别的平台的依赖算进来，判定会变得没有意义。真正要防的是「不开任何 feature 就被拉进来」。
//!
//! 确实去不掉的包写进 `ALLOWED`，**必须带理由**；不允许直接从 `DENIED` 里删掉。

use std::collections::BTreeSet;
use std::process::Command;

use crate::util;

/// 禁止出现在依赖图里的包（包名 → 为什么禁）。
const DENIED: &[(&str, &str)] = &[
    ("reqwest", "HTTP 客户端"),
    ("zed-reqwest", "gpui_http_client 0.2.2 依赖的 reqwest 分支"),
    ("hyper", "HTTP/1 与 HTTP/2 实现"),
    ("hyper-rustls", "hyper 的 TLS 连接器"),
    ("hyper-util", "hyper 的客户端工具层"),
    ("h2", "HTTP/2 实现"),
    ("rustls", "TLS 实现"),
    ("rustls-native-certs", "TLS 根证书加载"),
    ("rustls-pemfile", "TLS 证书解析"),
    ("rustls-pki-types", "TLS 证书类型"),
    ("rustls-webpki", "TLS 证书校验"),
    ("tokio-rustls", "tokio 上的 TLS"),
    ("ring", "rustls 使用的加密实现"),
    ("aws-lc-rs", "rustls 的另一个加密后端"),
    ("native-tls", "系统 TLS 封装"),
    ("openssl", "TLS 实现"),
    ("openssl-sys", "TLS 实现"),
    (
        "tokio",
        "异步运行时；Rove 走 gpui / smol 的执行器，只有 HTTP 栈会拉它",
    ),
    ("tokio-util", "tokio 配套工具"),
    ("tokio-socks", "SOCKS 代理"),
    (
        "async-compression",
        "上游 gpui_http_client 为下载 / 解包 GitHub Release 引入（见 ALLOWED 的说明）",
    ),
    (
        "zed-async-tar",
        "gpui_http_client 为解包 GitHub Release 引入",
    ),
];

/// 显式白名单：确实去不掉的包（包名 → 来源与理由）。
///
/// 往这里加条目时必须写清楚「谁把它拉进来的」「为什么去不掉」，不要为了让 CI 变绿
/// 直接从 `DENIED` 里删包名。
const ALLOWED: &[(&str, &str)] = &[(
    "async-compression",
    "来源不是 HTTP 栈：gpui → gpui_util → async_zip → async-compression（只用 futures-io，\
     不带 tokio，也不发请求）。它属于 gpui 自己的依赖，#74 的范围只动 HTTP/TLS 这一条链，\
     整理 gpui 其余依赖另提 issue",
)];

/// 一个判定范围。
struct Scope {
    name: &'static str,
    edges: &'static str,
}

const SCOPES: &[Scope] = &[
    Scope {
        name: "产品依赖（normal,build）",
        edges: "normal,build",
    },
    Scope {
        name: "含测试与基准（normal,build,dev）",
        edges: "normal,build,dev",
    },
];

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let package = util::flag(args, "--package");
    let mut failed = false;

    println!("依赖检查（issue #74）：禁止 HTTP / TLS 栈出现在依赖图里");
    for (name, reason) in ALLOWED {
        println!("  白名单：{name}（{reason}）");
    }

    for scope in SCOPES {
        let output = cargo_tree(scope.edges, package.as_deref())?;
        let packages = parse_packages(&output);
        let hits = denied_hits(&packages);
        println!();
        println!(
            "  {}：去重后 {} 个包",
            scope.name,
            util::thousands(packages.len() as u64)
        );
        if hits.is_empty() {
            println!("    OK：没有被禁的包。");
            continue;
        }
        failed = true;
        for (name, reason) in &hits {
            // ::error:: 注解必须是单行。
            println!("::error::依赖图里出现了被禁的包 `{name}`（{reason}），范围：{}；见 issue #74 与 crates/rove-gpui-http-stub", scope.name);
            println!("    被禁的包：{name}（{reason}）");
        }
        println!(
            "    用 `cargo tree -i {} --target all -e {}` 看是谁引入的。",
            hits[0].0, scope.edges
        );
    }

    if failed {
        println!();
        println!(
            "失败：依赖图里出现了 HTTP / TLS 栈。确实去不掉的项请加进 xtask/src/deps.rs 的 ALLOWED 并写明理由。"
        );
        return Ok(1);
    }
    println!();
    println!("OK：依赖图里没有 HTTP / TLS 栈。");
    Ok(0)
}

fn cargo_tree(edges: &str, package: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut command = Command::new(cargo);
    command.current_dir(util::repo_root());
    command.arg("tree");
    match package {
        Some(package) => {
            command.arg("--package").arg(package);
        }
        None => {
            command.arg("--workspace");
        }
    }
    command
        .arg("--edges")
        .arg(edges)
        .arg("--target")
        .arg("all")
        .arg("--prefix")
        .arg("none")
        .arg("--format")
        .arg("{p}");
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "cargo tree 失败（{}）：{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// 从 `cargo tree --prefix none --format {p}` 的输出里取去重后的包名。
///
/// 每行形如 `anyhow v1.0.86`、`gpui v0.2.2 (D:\...\vendor\gpui)`，重复子树后面带 ` (*)`。
fn parse_packages(output: &str) -> BTreeSet<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| *name != "(*)")
        .map(str::to_string)
        .collect()
}

/// 依赖图里命中的被禁包（已扣掉白名单）。
fn denied_hits(packages: &BTreeSet<String>) -> Vec<(&'static str, &'static str)> {
    DENIED
        .iter()
        .filter(|(name, _)| packages.contains(*name))
        .filter(|(name, _)| !ALLOWED.iter().any(|(allowed, _)| allowed == name))
        .map(|(name, reason)| (*name, *reason))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
rove-app v0.0.1 (D:\\Rove\\crates\\rove-app)
gpui v0.2.2 (D:\\Rove\\vendor\\gpui)
gpui_http_client v0.2.2 (D:\\Rove\\crates\\rove-gpui-http-stub)
anyhow v1.0.86
http v1.3.1
url v2.5.4
anyhow v1.0.86 (*)
";

    #[test]
    fn package_names_are_parsed_and_deduplicated() {
        let packages = parse_packages(SAMPLE);
        assert!(packages.contains("gpui_http_client"));
        assert!(packages.contains("anyhow"));
        // 6 行不同的包：rove-app / gpui / gpui_http_client / anyhow / http / url
        assert_eq!(packages.len(), 6);
    }

    #[test]
    fn a_clean_tree_has_no_denied_packages() {
        assert!(denied_hits(&parse_packages(SAMPLE)).is_empty());
    }

    /// 验收：人为把 reqwest / rustls / ring 加回依赖图时检查必须失败。
    #[test]
    fn adding_an_http_stack_back_is_rejected() {
        let regressed = format!(
            "{SAMPLE}zed-reqwest v0.12.15-zed\nhyper v1.12.0\nrustls v0.23.46\nring v0.17.14\ntokio v1.54.1\n"
        );
        let hits = denied_hits(&parse_packages(&regressed));
        let names: Vec<&str> = hits.iter().map(|(name, _)| *name).collect();
        // 顺序跟着 DENIED 的书写顺序走。
        assert_eq!(names, ["zed-reqwest", "hyper", "rustls", "ring", "tokio"]);
    }

    /// 白名单为空时不会意外放过任何东西；DENIED 里的名字不能有重复。
    #[test]
    fn the_denied_list_is_well_formed() {
        let mut seen = BTreeSet::new();
        for (name, reason) in DENIED {
            assert!(seen.insert(*name), "DENIED 里 `{name}` 重复了");
            assert!(!reason.is_empty(), "`{name}` 没有写明为什么被禁");
        }
        for (name, reason) in ALLOWED {
            assert!(
                DENIED.iter().any(|(denied, _)| denied == name),
                "ALLOWED 里的 `{name}` 不在 DENIED 里，写了也没用"
            );
            assert!(!reason.is_empty(), "白名单项 `{name}` 必须写明来源与理由");
        }
    }
}
