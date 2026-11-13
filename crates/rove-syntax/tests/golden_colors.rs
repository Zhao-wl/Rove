//! 着色黄金测试（#11）的集成测试：走完整条链路
//! fixture → #9 的 tree-sitter 高亮 → #10 的主题引擎 → 与 VS Code 的期望颜色逐字符比对。
//!
//! 比对器是 example（`cargo run -p rove-syntax --example golden_colors`），这里用 `#[path]`
//! 直接引入它的模块，保证 CI 跑的和本地排查用的是同一份代码。

#![allow(dead_code)]

#[path = "../examples/golden_colors/check.rs"]
mod check;
#[path = "../examples/golden_colors/config.rs"]
mod config;
#[path = "../examples/golden_colors/data.rs"]
mod data;
#[path = "../examples/golden_colors/report.rs"]
mod report;

use check::{Filter, Run};
use config::Config;
use std::sync::OnceLock;

/// 整个矩阵只跑一次，几个测试共用。
fn matrix() -> &'static (Config, Vec<Run>) {
    static MATRIX: OnceLock<(Config, Vec<Run>)> = OnceLock::new();
    MATRIX.get_or_init(|| {
        let root = config::repo_root();
        let config = Config::load(&root).expect("阈值文件可读");
        let runs = check::run_matrix(&root, &config, &Filter::default()).expect("比对矩阵可运行");
        (config, runs)
    })
}

/// 阈值判定：受阈值约束的（语言 × 主题）都必须达到 `tests/golden/thresholds.toml` 里的一致率。
/// 顺带把完整报告写到 `out/golden/`（已 gitignore），失败时可以直接翻差异明细。
#[test]
fn colors_match_vscode_within_thresholds() {
    let (config, runs) = matrix();
    let dir = config::repo_root().join("out/golden");
    match report::write(&dir, config, runs, 20) {
        Ok(written) => eprintln!("报告：{}", written.markdown.display()),
        // 只读的工作目录不该让判定失败。
        Err(e) => eprintln!("报告没有写成功（不影响判定）：{e}"),
    }
    let failures = check::failures(config, runs);
    assert!(
        failures.is_empty(),
        "低于阈值：{}\n一致率表：\n{}",
        failures
            .iter()
            .map(|(l, t, rate, threshold)| format!(
                "{l} × {t} = {:.2}% < {:.2}%",
                rate * 100.0,
                threshold * 100.0
            ))
            .collect::<Vec<_>>()
            .join("；"),
        report::rate_table(config, runs),
    );
}

/// 阈值文件确实在判定：把阈值提到 100% 之后，同一批结果必须被判为失败。
#[test]
fn thresholds_file_decides_failure() {
    let (config, runs) = matrix();
    let strict = Config::parse(
        "enforced_themes = [\"dark_modern\"]\n\
         reported_themes = []\n\
         [languages]\n\
         xml = { fixtures = [\"test-7115.xml\"], threshold = 1.0 }\n",
    )
    .expect("配置可解析");
    assert!(
        !check::failures(&strict, runs).is_empty(),
        "阈值 100% 时应当判为失败"
    );
    // 同一批结果在真实阈值下通过，说明判定读的是数据文件而不是写死的数字。
    assert!(check::failures(config, runs).is_empty());
}

/// 低于 issue #11 目标（98%）的阈值必须写明理由，否则配置就不合法。
#[test]
fn a_lowered_threshold_needs_a_reason() {
    let without = Config::parse(
        "enforced_themes = [\"dark_modern\"]
         reported_themes = []
         [languages]
         xml = { fixtures = [\"test.xml\"], threshold = 0.9 }
",
    );
    assert!(without.is_err(), "没写理由的低阈值应当被拒绝");
    let with = Config::parse(
        "enforced_themes = [\"dark_modern\"]
         reported_themes = []
         [languages]
         xml = { fixtures = [\"test.xml\"], threshold = 0.9, reason = \"上游语法限制\" }
",
    );
    assert!(with.is_ok());
    // 仓库里现有的低阈值也都写了理由。
    for language in &matrix().0.languages {
        if language.threshold.is_some_and(|t| t < config::TARGET) {
            assert!(
                language
                    .reason
                    .as_deref()
                    .is_some_and(|r| !r.trim().is_empty()),
                "{} 的阈值低于目标却没有写理由",
                language.id
            );
        }
    }
}

/// 加一种语言只需要加一行配置：占位语言（还没有语法）也能统计，只是不设阈值。
#[test]
fn placeholder_language_is_counted_without_a_threshold() {
    let (config, runs) = matrix();
    let placeholder = config
        .languages
        .iter()
        .find(|l| l.threshold.is_none())
        .expect("阈值文件里有一个占位语言");
    let counted: usize = runs
        .iter()
        .filter(|r| r.language == placeholder.id)
        .map(|r| r.chars)
        .sum();
    assert!(counted > 0, "占位语言 {} 没有统计到字符", placeholder.id);
    assert!(
        check::failures(config, runs)
            .iter()
            .all(|(language, ..)| language != &placeholder.id),
        "没有阈值的语言不应参与判定"
    );
    // 只加一行配置（语言 ID + fixture）就能接入，不需要改运行器。
    let added = Config::parse(
        "enforced_themes = [\"dark_modern\"]\n\
         reported_themes = []\n\
         [languages]\n\
         xml = { fixtures = [\"test-7115.xml\"] }\n",
    )
    .expect("配置可解析");
    let runs = check::run_matrix(&config::repo_root(), &added, &Filter::default())
        .expect("新加的语言可以直接跑");
    assert_eq!(runs.len(), 1);
    assert!(runs[0].chars > 0);
}

/// 比对器的最小闭环：给定 fixture 与主题，逐字符给出期望颜色与 Rove 实际算出的颜色。
#[test]
fn comparator_reports_every_character() {
    let root = config::repo_root();
    let fixture = data::load(&root, "xml", "test-7115.xml").expect("fixture 可读");
    let scopes = check::highlight(&fixture.text, "xml").expect("高亮可用");
    assert!(!scopes.is_empty(), "XML 应该产生 token");
    let theme = check::load("dark_modern").expect("主题可加载");
    let chars = check::characters(&fixture, &scopes, "dark_modern", &theme);
    // 期望结果覆盖除行终止符以外的每一个字符。
    let expected_chars = fixture
        .text
        .chars()
        .filter(|c| *c != '\n' && *c != '\r')
        .count();
    assert_eq!(chars.len(), expected_chars);
    let first = chars.first().expect("至少一个字符");
    assert_eq!(first.ch, '<');
    assert_eq!((first.line, first.column), (1, 1));
    assert_eq!(rove_theme::format_hex(first.expected), "#808080");
    let run = check::compare(&fixture, &scopes, "dark_modern", &theme, false);
    assert_eq!(run.chars, chars.len());
    assert_eq!(
        run.matched,
        chars.iter().filter(|c| c.matches()).count(),
        "一致率必须和逐字符结果一致"
    );
}

/// 占位语言没有语法时退回主题默认前景色，而不是报错。
#[test]
fn unsupported_language_falls_back_to_the_default_foreground() {
    let root = config::repo_root();
    let fixture = data::load(&root, "lua", "test.lua").expect("fixture 可读");
    let scopes = check::highlight(&fixture.text, "lua").expect("不支持的语言不应报错");
    assert!(scopes.is_empty(), "Lua 还没有语法，不应产生 token");
    let theme = check::load("dark_plus").expect("主题可加载");
    let run = check::compare(&fixture, &scopes, "dark_plus", &theme, true);
    assert!(run.chars > 0);
    assert!(run.rate() < 1.0, "没有语法时不可能全对");
}

/// fixture 与期望结果一一对应，且期望结果平铺整份文件（数据没有被加工过的自检）。
#[test]
fn fixtures_and_results_line_up() {
    let root = config::repo_root();
    let config = Config::load(&root).expect("阈值文件可读");
    for language in &config.languages {
        for name in &language.fixtures {
            let fixture = data::load(&root, &language.id, name).expect("fixture 与期望结果对齐");
            let covered: usize = fixture.tokens.iter().map(|t| t.end - t.start).sum();
            let newlines = fixture
                .text
                .bytes()
                .filter(|b| *b == b'\n' || *b == b'\r')
                .count();
            assert_eq!(
                covered + newlines,
                fixture.text.len(),
                "{name} 的期望结果没有平铺整份文件"
            );
            for theme in config.themes() {
                assert!(
                    fixture.tokens.iter().all(|t| t.colors.contains_key(&theme)),
                    "{name} 缺少主题 {theme} 的期望颜色"
                );
            }
        }
    }
}

/// `language_for_path` 认出 fixture 的语言，和阈值文件里登记的一致（占位语言除外）。
#[test]
fn fixture_extensions_map_to_the_configured_language() {
    let config = Config::load(&config::repo_root()).expect("阈值文件可读");
    for language in &config.languages {
        if language.threshold.is_none() {
            continue;
        }
        for name in &language.fixtures {
            let detected =
                rove_syntax::highlight::language_for_path(std::path::Path::new(name), None);
            assert_eq!(&*detected, language.id, "{name} 的语言识别结果不一致");
        }
    }
}
