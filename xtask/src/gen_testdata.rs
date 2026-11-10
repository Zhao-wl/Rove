//! 测试数据生成（issue #4）。
//!
//! 规格照搬 `M0_UI_SPIKE_REPORT` §2.5 与 `spikes/gen_testdata.py`，保证与 M0 的数据可比：
//!
//! | 文件 | 规格 | 用途 |
//! |---|---|---|
//! | `small.txt` | 46 KB / 402 行，7 种模式循环 | 整形缓存命中时的乐观值、字体回退 |
//! | `big1g.txt` | 1 GiB / 14,290,000 行，类 C# 代码，每 50 行一行中文 | 大文件打开与滚动、行索引 |
//! | `uniq.txt` | 33 MB / 400,000 行，每行都唯一 | 整形缓存全未命中的真实成本 |
//!
//! 生成物写到 `target/perf-testdata`（target 已 gitignore），**不要提交**。

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::util;

/// M0 记录值：(字节数, 行数)。行数以换行符计，末尾空行不算一行。
///
/// `small.txt` 的 M0 报告写「46 KB / 402 行」，按生成器实际是 46,492 B / 401 行
/// （差 1 行，0.25%），这里以生成器实际值为准，并允许 1% 误差。
const M0_SMALL: (u64, u64) = (46_492, 401);
const M0_BIG: (u64, u64) = (1_073_822_050, 14_290_000);
const M0_UNIQ: (u64, u64) = (33_057_614, 400_000);

const TOLERANCE: f64 = 0.01;

/// big1g.txt 的目标大小：1 GiB（按块循环写入，最后一整块会略微超出）。
const BIG_TARGET: u64 = 1024 * 1024 * 1024;

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let out = util::flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(util::default_testdata_dir);
    let only = util::flag(args, "--only");
    if let Some(only) = &only {
        if !matches!(only.as_str(), "small" | "uniq" | "big") {
            return Err(format!("--only 只接受 small / uniq / big，收到 `{only}`").into());
        }
    }
    fs::create_dir_all(&out)?;
    println!("输出目录：{}", out.display());

    let wanted = |name: &str| only.as_deref().is_none_or(|only| only == name);
    let mut failed = false;

    if wanted("small") {
        let (bytes, lines) = gen_small(&out)?;
        failed |= !report("small.txt", bytes, lines, M0_SMALL);
    }
    if wanted("uniq") {
        let (bytes, lines) = gen_unique(&out)?;
        failed |= !report("uniq.txt", bytes, lines, M0_UNIQ);
    }
    if wanted("big") {
        println!("生成 big1g.txt（1 GiB，机械硬盘上约 10–30 s）…");
        let (bytes, lines) = gen_big(&out)?;
        failed |= !report("big1g.txt", bytes, lines, M0_BIG);
    }

    if failed {
        eprintln!(
            "生成结果与 M0 的差值超过 1%，请检查生成逻辑是否与 spikes/gen_testdata.py 一致。"
        );
        return Ok(1);
    }
    println!("OK：生成结果与 M0 记录一致（误差 < 1%）。");
    Ok(0)
}

fn report(name: &str, bytes: u64, lines: u64, expected: (u64, u64)) -> bool {
    let (exp_bytes, exp_lines) = expected;
    let byte_delta = relative_delta(bytes, exp_bytes);
    let line_delta = relative_delta(lines, exp_lines);
    let ok = byte_delta <= TOLERANCE && line_delta <= TOLERANCE;
    println!(
        "  {name:<11} {:>15}  ({:>13} B)  {:>12} 行  | M0: {} B / {} 行  | 差值 {:.2}% / {:.2}%  {}",
        util::human_bytes(bytes),
        bytes,
        util::thousands(lines),
        util::thousands(exp_bytes),
        util::thousands(exp_lines),
        byte_delta * 100.0,
        line_delta * 100.0,
        if ok { "OK" } else { "超差" }
    );
    ok
}

fn relative_delta(actual: u64, expected: u64) -> f64 {
    if expected == 0 {
        return 0.0;
    }
    (actual as f64 - expected as f64).abs() / expected as f64
}

/// 7 种模式：纯 ASCII、中文、中英混排 + emoji（含 ZWJ 与肤色修饰）、日韩、tab 缩进、300 字符长行。
fn mixed_patterns() -> Vec<String> {
    vec![
        "// ASCII only: fn main() { let x = 42; }".to_string(),
        "// 中文注释：这是一个用于测试 CJK 字形渲染与字体回退的示例行。".to_string(),
        "// Mixed 混合 text with emoji 🚀🔥✅ and symbols ±≈→∞ ok".to_string(),
        "// 日本語テキスト・한국어 텍스트 mixed with English".to_string(),
        "let emoji_table = [\"😀\", \"🇨🇳\", \"👨‍👩‍👧‍👦\", \"🧑🏽‍💻\"]; // ZWJ + skin tone".to_string(),
        "\tpublic void Foo(int bar) { /* tab indented, 宽窄混排 */ }".to_string(),
        format!(
            "const LONG: &str = \"{}\"; // very long line",
            "x".repeat(300)
        ),
    ]
}

fn writer(path: &Path) -> std::io::Result<BufWriter<File>> {
    let file = File::create(path)?;
    Ok(BufWriter::with_capacity(1 << 20, file))
}

fn gen_small(out: &Path) -> std::io::Result<(u64, u64)> {
    let path = out.join("small.txt");
    let patterns = mixed_patterns();
    let mut w = writer(&path)?;
    let mut bytes = 0u64;
    let mut lines = 0u64;

    let header = "// Rove M0 UI spike - small mixed-script sample\n";
    w.write_all(header.as_bytes())?;
    bytes += header.len() as u64;
    lines += 1;

    for i in 0..400usize {
        let line = format!("{:>4} {}\n", i + 1, patterns[i % patterns.len()]);
        w.write_all(line.as_bytes())?;
        bytes += line.len() as u64;
        lines += 1;
    }
    w.flush()?;
    Ok((bytes, lines))
}

fn gen_unique(out: &Path) -> std::io::Result<(u64, u64)> {
    let path = out.join("uniq.txt");
    let mut w = writer(&path)?;
    let mut bytes = 0u64;
    let mut lines = 0u64;

    for i in 0..400_000u64 {
        let line = if i % 50 == 7 {
            format!(
                "    // 第 {i} 行：唯一内容，用于测量真实的文本整形开销 {}\n",
                i * 7919
            )
        } else {
            format!(
                "    public static int Method_{i:08}(int a{}, int b{}) {{ return a * {i} + b; }}\n",
                i % 97,
                i % 89
            )
        };
        w.write_all(line.as_bytes())?;
        bytes += line.len() as u64;
        lines += 1;
    }
    w.flush()?;
    Ok((bytes, lines))
}

fn gen_big(out: &Path) -> std::io::Result<(u64, u64)> {
    let path = out.join("big1g.txt");
    // 2000 行一块，逐块重复写入，直到 >= 1 GiB（与 M0 脚本一致）。
    let mut block = String::new();
    for i in 0..2000usize {
        if i % 50 == 7 {
            block.push_str(&format!(
                "    // 第 {i} 行：超大文件中的中文内容，用于验证回退字体在虚拟滚动下的表现。\n"
            ));
        } else {
            block.push_str(&format!(
                "    public static int Method_{i:06}(int a, int b) {{ return a * {i} + b; }}\n"
            ));
        }
    }
    let block_bytes = block.as_bytes();
    let mut w = writer(&path)?;
    let mut bytes = 0u64;
    let mut lines = 0u64;
    while bytes < BIG_TARGET {
        w.write_all(block_bytes)?;
        bytes += block_bytes.len() as u64;
        lines += 2000;
    }
    w.flush()?;
    Ok((bytes, lines))
}
