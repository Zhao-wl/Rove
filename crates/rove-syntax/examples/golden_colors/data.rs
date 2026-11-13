//! 读取 VS Code 的着色测试数据（#11）。
//!
//! `colorize-fixtures/<name>` 是源文件，`colorize-results/<stem>_<ext>.json` 是它的期望结果：
//! 一个 token 数组，`c` 是 token 文本、`t` 是 TextMate scope 路径、`r` 是每个主题下
//! 「命中的规则: #颜色」。两个目录都是 VS Code 仓库里的原始文件，不加工。
//!
//! token 按文档顺序平铺整份文件，**只跳过行终止符**（`c` 里不含 `\r`/`\n`）。对齐就是
//! 顺着 fixture 文本走一遍：遇到行终止符先跳过，再要求剩下的文本以当前 token 的 `c` 开头。

use rove_theme::{parse_hex, Color};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct RawToken {
    c: String,
    t: String,
    r: BTreeMap<String, String>,
}

/// 期望结果里的一个 token，已经对齐到 fixture 的字节区间。
#[derive(Debug, Clone)]
pub struct Expected {
    pub start: usize,
    pub end: usize,
    pub text: String,
    /// VS Code 的 scope 路径（由外到内，空格分隔）。
    pub scopes: String,
    /// 主题 → 命中的规则（`default` 表示没有规则命中，用主题的默认前景色）。
    pub rules: BTreeMap<String, String>,
    /// 主题 → 期望的前景色。
    pub colors: BTreeMap<String, Color>,
}

#[derive(Debug, Clone)]
pub struct Fixture {
    pub name: String,
    pub language: String,
    pub text: String,
    pub tokens: Vec<Expected>,
    /// 每一行的起始字节偏移，用来把字节偏移换算成行列。
    line_starts: Vec<usize>,
}

impl Fixture {
    /// 字节偏移 → （1 起的行号，1 起的列号，按字符计）。
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let column = self.text[self.line_starts[line]..offset].chars().count() + 1;
        (line + 1, column)
    }
}

/// `test-7115.xml` → `test-7115_xml.json`，`tsconfig_off.json` → `tsconfig_off_json.json`。
pub fn result_name(fixture: &str) -> String {
    match fixture.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}_{ext}.json"),
        None => format!("{fixture}.json"),
    }
}

pub fn load(root: &Path, language: &str, name: &str) -> Result<Fixture, String> {
    let fixture_path = root.join(super::config::FIXTURES_DIR).join(name);
    let result_path = root
        .join(super::config::RESULTS_DIR)
        .join(result_name(name));
    let text = std::fs::read_to_string(&fixture_path)
        .map_err(|e| format!("读不到 {}：{e}", fixture_path.display()))?;
    let raw = std::fs::read_to_string(&result_path)
        .map_err(|e| format!("读不到 {}：{e}", result_path.display()))?;
    let raw: Vec<RawToken> =
        serde_json::from_str(&raw).map_err(|e| format!("{} 不是期望结果：{e}", name))?;
    let tokens = align(&text, &raw, name)?;
    let line_starts = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    Ok(Fixture {
        name: name.to_string(),
        language: language.to_string(),
        text,
        tokens,
        line_starts,
    })
}

/// 把期望 token 顺序对齐到 fixture 的字节区间；对不上就是数据有问题，直接报错。
fn align(text: &str, raw: &[RawToken], name: &str) -> Result<Vec<Expected>, String> {
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0usize;
    for (index, token) in raw.iter().enumerate() {
        while text[at..].starts_with(['\r', '\n']) {
            at += 1;
        }
        if !text[at..].starts_with(&token.c) {
            return Err(format!(
                "{name} 的第 {index} 个 token 对不上：期望 {:?}，fixture 在字节 {at} 处是 {:?}",
                token.c,
                &text[at..(at + 40).min(text.len())],
            ));
        }
        let (mut rules, mut colors) = (BTreeMap::new(), BTreeMap::new());
        for (theme, value) in &token.r {
            let (rule, color) = value
                .rsplit_once(": ")
                .ok_or_else(|| format!("{name} 的第 {index} 个 token 的 {theme} 值是 {value:?}"))?;
            let color = parse_hex(color)
                .ok_or_else(|| format!("{name} 的第 {index} 个 token 的颜色是 {color:?}"))?;
            rules.insert(theme.clone(), rule.to_string());
            colors.insert(theme.clone(), color);
        }
        out.push(Expected {
            start: at,
            end: at + token.c.len(),
            text: token.c.clone(),
            scopes: token.t.clone(),
            rules,
            colors,
        });
        at += token.c.len();
    }
    // 末尾只允许剩下行终止符：否则说明期望结果没有覆盖整份文件。
    if text[at..].chars().any(|c| c != '\r' && c != '\n') {
        return Err(format!(
            "{name} 的期望结果没有覆盖整份文件，剩余 {:?}",
            &text[at..(at + 40).min(text.len())]
        ));
    }
    Ok(out)
}
