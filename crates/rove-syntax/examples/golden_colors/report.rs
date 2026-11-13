//! 差异报告（#11）：一致率表、差异分布、逐条差异。
//!
//! 写到 `out/golden/`（已 gitignore）：`report.md` 给人看，`report.json` 给机器看。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use super::check::{summarize, Cause, Diff, Run};
use super::config::Config;

fn percent(rate: f64) -> String {
    format!("{:.2}%", rate * 100.0)
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\t' => out.push_str("\\t"),
            '|' => out.push_str("\\|"),
            '`' => out.push('\''),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{{{:04x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// 一致率表：行是语言，列是主题；受阈值约束的主题在前。
pub fn rate_table(config: &Config, runs: &[Run]) -> String {
    let summary = summarize(runs);
    let themes = config.themes();
    let mut out = String::new();
    let _ = write!(out, "| 语言 | 阈值 |");
    for theme in &themes {
        let mark = if config.is_enforced(theme) { "" } else { "*" };
        let _ = write!(out, " {theme}{mark} |");
    }
    out.push('\n');
    out.push_str("|---|---|");
    for _ in &themes {
        out.push_str("---|");
    }
    out.push('\n');
    for language in &config.languages {
        let threshold = language
            .threshold
            .map(percent)
            .unwrap_or_else(|| "只统计".into());
        let _ = write!(out, "| {} | {threshold} |", language.id);
        for theme in &themes {
            match summary.get(&(language.id.clone(), theme.clone())) {
                Some(cell) => {
                    let below = language
                        .threshold
                        .is_some_and(|t| config.is_enforced(theme) && cell.rate() < t);
                    let mark = if below { " ❌" } else { "" };
                    let _ = write!(out, " {}{mark} |", percent(cell.rate()));
                }
                None => out.push_str(" — |"),
            }
        }
        out.push('\n');
    }
    out.push_str("\n标 `*` 的主题只统计、不设阈值。\n");
    out
}

/// 差异成因表：每个（语言，主题）的差异字符按成因分类，并给出扣除「Rove 侧之外的成因」
/// 之后的一致率。成因由 VS Code 的 scope 路径判定，不是人工标注。
pub fn cause_table(config: &Config, runs: &[Run]) -> String {
    let summary = summarize(runs);
    let causes = [
        Cause::ForeignLanguage,
        Cause::MarkedInvalid,
        Cause::Placeholder,
        Cause::Rove,
    ];
    let mut out = String::new();
    out.push_str("| 语言 | 主题 | 差异字符 |");
    for cause in causes {
        let _ = write!(out, " {} |", cause.label());
    }
    out.push_str(" 扣除后一致率 |\n|---|---|---|---|---|---|---|---|\n");
    for language in &config.languages {
        for theme in &config.enforced_themes {
            let Some(cell) = summary.get(&(language.id.clone(), theme.clone())) else {
                continue;
            };
            let _ = write!(out, "| {} | {theme} | {} |", language.id, cell.diffs);
            for cause in causes {
                let _ = write!(out, " {} |", cell.causes.get(&cause).copied().unwrap_or(0));
            }
            let _ = writeln!(out, " {} |", percent(cell.adjusted_rate()));
        }
    }
    out
}

fn diff_rows(diffs: &[&Diff]) -> String {
    let mut out = String::new();
    out.push_str(
        "| 文件 | 行:列 | 片段 | 期望 | 实际 | VS Code 规则 | Rove 规则 | Rove scope | 类别 | 成因 |\n",
    );
    out.push_str("|---|---|---|---|---|---|---|---|---|---|\n");
    for d in diffs {
        let _ = writeln!(
            out,
            "| {} | {}:{} | `{}` | {} | {} | {} | {} | {} | {} | {} |",
            d.fixture,
            d.line,
            d.column,
            escape(&d.text),
            d.expected_hex(),
            d.actual_hex(),
            escape(&d.vscode_rule),
            escape(&d.rove_rule),
            if d.rove_scopes.is_empty() {
                "（无）".to_string()
            } else {
                escape(&d.rove_scopes)
            },
            d.kind.label(),
            d.cause.label(),
        );
    }
    out
}

/// 差异分布：按「VS Code 规则 → Rove 规则」归类，按涉及字符数排序。
pub fn distribution(runs: &[Run], theme: &str) -> String {
    let mut classes: BTreeMap<String, (usize, usize, String)> = BTreeMap::new();
    for run in runs.iter().filter(|r| r.theme == theme) {
        for diff in &run.diffs {
            let entry = classes.entry(diff.class()).or_insert((0, 0, String::new()));
            entry.0 += diff.chars;
            entry.1 += 1;
            if entry.2.is_empty() {
                entry.2 = format!(
                    "{} {}:{} {:?}",
                    diff.fixture, diff.line, diff.column, diff.text
                );
            }
        }
    }
    let mut rows: Vec<_> = classes.into_iter().collect();
    rows.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));
    let mut out = String::new();
    let _ = writeln!(
        out,
        "按「VS Code 命中的规则 → Rove 命中的规则」归类（主题 {theme}）：\n"
    );
    out.push_str("| 差异类别 | 字符数 | 段数 | 例子 |\n|---|---|---|---|\n");
    for (class, (chars, segments, sample)) in rows {
        let _ = writeln!(
            out,
            "| {} | {chars} | {segments} | {} |",
            escape(&class),
            escape(&sample)
        );
    }
    out
}

pub struct Written {
    pub markdown: PathBuf,
    pub json: PathBuf,
}

/// 写出完整报告。`top` 是摘要里列出的差异条数。
pub fn write(dir: &Path, config: &Config, runs: &[Run], top: usize) -> Result<Written, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("建不了 {}：{e}", dir.display()))?;
    let markdown = dir.join("report.md");
    let json = dir.join("report.json");
    std::fs::write(&markdown, render(config, runs, top))
        .map_err(|e| format!("写不了 {}：{e}", markdown.display()))?;
    std::fs::write(&json, render_json(config, runs))
        .map_err(|e| format!("写不了 {}：{e}", json.display()))?;
    Ok(Written { markdown, json })
}

pub fn render(config: &Config, runs: &[Run], top: usize) -> String {
    let mut out = String::new();
    out.push_str("# 着色黄金测试报告（#11）\n\n");
    out.push_str(
        "数据：VS Code `extensions/vscode-colorize-tests`（见 `tests/golden/README.md`）。\
         比对口径：逐字符比**最终前景色**，不比 scope。\n\n",
    );
    out.push_str("## 一致率\n\n");
    out.push_str(&rate_table(config, runs));

    let failures = super::check::failures(config, runs);
    out.push_str("\n## 判定\n\n");
    if failures.is_empty() {
        out.push_str("受阈值约束的（语言 × 主题）全部达标。\n");
    } else {
        for (language, theme, rate, threshold) in &failures {
            let _ = writeln!(
                out,
                "- {language} × {theme}：{} < {}",
                percent(*rate),
                percent(*threshold)
            );
        }
    }

    out.push_str("\n## 差异成因\n\n");
    out.push_str(&cause_table(config, runs));
    out.push_str(
        "\n「扣除后一致率」把前三类成因的字符从分母里去掉：注入语言未内置、VS Code 自己判为非法、\
         占位语言。每一类的判定规则与说明见 `docs/GOLDEN_COLORS.md`。\n",
    );

    out.push_str("\n## 每个 fixture 的一致率\n\n");
    out.push_str("| 语言 | fixture | 主题 | 字符数 | 一致 | 一致率 | 差异段数 |\n|---|---|---|---|---|---|---|\n");
    for run in runs {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} |",
            run.language,
            run.fixture,
            run.theme,
            run.chars,
            run.matched,
            percent(run.rate()),
            run.diffs.len()
        );
    }

    let mut all: Vec<&Diff> = runs.iter().flat_map(|r| r.diffs.iter()).collect();
    all.sort_by(|a, b| {
        b.chars
            .cmp(&a.chars)
            .then_with(|| a.fixture.cmp(&b.fixture))
    });
    let _ = write!(out, "\n## 差异分布\n\n");
    for theme in &config.enforced_themes {
        let _ = writeln!(out, "### {theme}\n");
        out.push_str(&distribution(runs, theme));
        out.push('\n');
    }

    let _ = writeln!(out, "## 最大的 {top} 处差异（全部主题）\n");
    out.push_str(&diff_rows(
        &all.iter().copied().take(top).collect::<Vec<_>>(),
    ));

    out.push_str("\n## 逐条差异\n\n");
    for run in runs {
        if run.diffs.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "### {} / {} / {}（{} 段）\n",
            run.language,
            run.fixture,
            run.theme,
            run.diffs.len()
        );
        out.push_str(&diff_rows(&run.diffs.iter().collect::<Vec<_>>()));
        out.push('\n');
    }
    out
}

pub fn render_json(config: &Config, runs: &[Run]) -> String {
    use serde_json::{json, Value};
    let summary: Vec<Value> = summarize(runs)
        .into_iter()
        .map(|((language, theme), cell)| {
            json!({
                "language": language,
                "theme": theme,
                "enforced": config.is_enforced(&theme),
                "chars": cell.chars,
                "matched": cell.matched,
                "rate": cell.rate(),
                "adjusted_rate": cell.adjusted_rate(),
                "causes": cell.causes.iter().map(|(cause, chars)| json!({
                    "cause": cause.label(),
                    "chars": chars,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let runs_json: Vec<Value> = runs
        .iter()
        .map(|run| {
            json!({
                "language": run.language,
                "fixture": run.fixture,
                "theme": run.theme,
                "chars": run.chars,
                "matched": run.matched,
                "rate": run.rate(),
                "diffs": run.diffs.iter().map(|d| json!({
                    "file": d.fixture,
                    "line": d.line,
                    "column": d.column,
                    "text": d.text,
                    "chars": d.chars,
                    "expected": d.expected_hex(),
                    "actual": d.actual_hex(),
                    "vscode_rule": d.vscode_rule,
                    "vscode_scopes": d.vscode_scopes,
                    "rove_rule": d.rove_rule,
                    "rove_scopes": d.rove_scopes,
                    "kind": d.kind.label(),
                    "cause": d.cause.label(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let failures: Vec<Value> = super::check::failures(config, runs)
        .into_iter()
        .map(|(language, theme, rate, threshold)| {
            json!({ "language": language, "theme": theme, "rate": rate, "threshold": threshold })
        })
        .collect();
    let document = json!({
        "thresholds": config.languages.iter().map(|l| json!({
            "language": l.id,
            "threshold": l.threshold,
            "fixtures": l.fixtures,
        })).collect::<Vec<_>>(),
        "enforced_themes": config.enforced_themes,
        "reported_themes": config.reported_themes,
        "summary": summary,
        "failures": failures,
        "runs": runs_json,
    });
    serde_json::to_string_pretty(&document).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}
