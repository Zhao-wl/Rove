//! 比对器（#11）：fixture → #9 的 tree-sitter 高亮 → #10 的主题引擎 → 逐字符比颜色。
//!
//! 比的是**最终前景色**，不是 scope：只要颜色一致，scope 有出入也算通过（issue #11 的口径）。

use futures_lite::future::block_on;
use rove_base::{Cancellation, Error, Line};
use rove_buffer::{RopeText, TextSource};
use rove_syntax::{highlight::TreeSitterSyntax, SyntaxService, SyntaxUpdate, Token};
use rove_theme::{
    format_hex,
    resolved::ResolvedTheme,
    service::{load_theme, p0_registry, ThemeCatalog},
    Color, Customizations,
};
use std::collections::BTreeMap;
use std::sync::Arc;

use super::config::Config;
use super::data::Fixture;

/// 每个字节上 Rove 给出的 scope 路径（由外到内）；`None` 表示没有 token 覆盖。
pub struct RoveScopes(Vec<Option<Arc<[Arc<str>]>>>);

impl RoveScopes {
    pub fn at(&self, byte: usize) -> &[Arc<str>] {
        self.0
            .get(byte)
            .and_then(|s| s.as_deref())
            .unwrap_or_default()
    }

    /// 至少有一个字节被 token 覆盖。
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|s| s.is_none())
    }
}

/// 用 #9 的高亮服务给整份文件产生 scope。不支持的语言（占位语言）返回全空而不是报错。
pub fn highlight(text: &str, language: &str) -> Result<RoveScopes, String> {
    let source = Arc::new(RopeText::new(text.to_string()));
    let lines = source.lines().known;
    let service = TreeSitterSyntax::default();
    let update =
        block_on(service.analyze(source.clone(), language.into(), Cancellation::default()));
    let mut map: Vec<Option<Arc<[Arc<str>]>>> = vec![None; text.len()];
    let update: SyntaxUpdate = match update {
        Ok(update) => update,
        // 占位语言：还没有接入语法，所有字符退回主题默认前景色。
        Err(Error::Unsupported) => return Ok(RoveScopes(map)),
        Err(e) => return Err(format!("{language} 高亮失败：{e:?}")),
    };
    for row in 0..lines {
        let tokens = update
            .snapshot
            .tokens(Line(row))
            .map_err(|e| format!("{language} 第 {row} 行取不到 token：{e:?}"))?;
        for Token { range, scopes } in tokens.iter() {
            let (start, end) = (range.start().0 as usize, range.end().0 as usize);
            for slot in map.iter_mut().take(end.min(text.len())).skip(start) {
                *slot = Some(scopes.clone());
            }
        }
    }
    Ok(RoveScopes(map))
}

/// 加载一个主题。`key` 是 `colorize-results` 里的主题键，同时也是 `rove-theme` 的主题文件名。
pub fn load(key: &str) -> Result<ResolvedTheme, String> {
    let catalog = ThemeCatalog::builtin();
    let file = format!("{key}.json");
    let entry = catalog
        .entries()
        .iter()
        .find(|e| *e.path == file)
        .ok_or_else(|| format!("内置主题目录里没有 {file}"))?;
    load_theme(entry, &Customizations::default(), Arc::new(p0_registry()))
        .map_err(|e| format!("加载主题 {key} 失败：{e:?}"))
}

/// 一处差异（已经把相邻且同因的字符并成一段）。
#[derive(Debug, Clone)]
pub struct Diff {
    pub fixture: String,
    pub line: usize,
    pub column: usize,
    pub text: String,
    pub chars: usize,
    pub expected: Color,
    pub actual: Color,
    /// VS Code 命中的规则（`default` = 没有规则命中）。
    pub vscode_rule: String,
    pub vscode_scopes: String,
    /// Rove 这一侧决定了最终颜色的 scope 层级（`default` = 没有规则命中）。
    pub rove_rule: String,
    pub rove_scopes: String,
    pub kind: Kind,
    pub cause: Cause,
}

impl Diff {
    pub fn expected_hex(&self) -> String {
        format_hex(self.expected)
    }
    pub fn actual_hex(&self) -> String {
        format_hex(self.actual)
    }
    /// 差异的归类键：同一类差异在报告里合并统计。
    pub fn class(&self) -> String {
        format!("{} → {}", self.vscode_rule, self.rove_rule)
    }
}

/// 差异的成因，直接从 VS Code 的 scope 路径判定，不靠人工标注。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cause {
    /// VS Code 在这里注入了 Rove 没有内置的语言（HTML、TeX、TypeScript……）。
    /// Rove 的语言清单见 `PROJECT_CHARTER`：注入语言不在清单里时，这段文本只能保持无 token。
    ForeignLanguage,
    /// VS Code 自己把这段文本判成非法（`invalid.illegal.*`）。文档本身是坏的，
    /// TextMate 的逐行纠错与 tree-sitter 的错误恢复必然给出不同结果。
    MarkedInvalid,
    /// 还没有接入语法的占位语言。
    Placeholder,
    /// 其余差异：Rove 这一侧的 scope 还不够准，或者上游语法的节点划分与 TextMate 不同。
    Rove,
}

impl Cause {
    pub fn label(self) -> &'static str {
        match self {
            Cause::ForeignLanguage => "VS Code 注入了 Rove 未内置的语言",
            Cause::MarkedInvalid => "VS Code 判为非法文本",
            Cause::Placeholder => "占位语言（还没有语法）",
            Cause::Rove => "Rove 侧的差异",
        }
    }
}

/// Rove 不内置的语言在 scope 路径里的标志（后缀或 `meta.embedded.block.<语言>`）。
/// 根 scope（路径第一段，例如 Markdown 的 `text.html.markdown`）不参与判定。
const FOREIGN: &[&str] = &[
    ".html", ".tex", ".js", ".jsx", ".ts", ".tsx", ".css", ".scss", ".less", ".lua", ".php",
    ".python", ".shell", ".coffee",
];

fn cause(vscode_scopes: &str, placeholder: bool) -> Cause {
    if placeholder {
        return Cause::Placeholder;
    }
    let mut scopes = vscode_scopes.split_whitespace();
    let _root = scopes.next();
    for scope in scopes {
        if scope.contains("invalid.illegal") {
            return Cause::MarkedInvalid;
        }
        if FOREIGN.iter().any(|suffix| scope.ends_with(suffix)) {
            return Cause::ForeignLanguage;
        }
    }
    Cause::Rove
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// VS Code 着色、Rove 没有（退回默认前景色）。
    Missing,
    /// Rove 着色、VS Code 没有。
    Extra,
    /// 两边都着色，但颜色不同。
    Different,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Missing => "Rove 未着色",
            Kind::Extra => "Rove 多着色",
            Kind::Different => "颜色不同",
        }
    }
}

/// 一个 fixture 在一个主题下的结果。
#[derive(Debug, Clone)]
pub struct Run {
    pub language: String,
    pub fixture: String,
    pub theme: String,
    pub chars: usize,
    pub matched: usize,
    pub diffs: Vec<Diff>,
}

impl Run {
    pub fn rate(&self) -> f64 {
        if self.chars == 0 {
            1.0
        } else {
            self.matched as f64 / self.chars as f64
        }
    }
}

/// 决定最终前景色的 scope 层级：从外到内逐层用公开接口求值，记下最后一次改变前景色的那一层。
/// 对应 vscode-textmate 的逐层合并，报告里用它回答「Rove 命中的是哪条规则」。
fn deciding_scope(theme: &ResolvedTheme, scopes: &[Arc<str>]) -> String {
    let mut previous = theme.token_style(&[]).foreground;
    let mut rule = String::from("default");
    for depth in 1..=scopes.len() {
        let current = theme.token_style(&scopes[..depth]).foreground;
        if current != previous {
            rule = scopes[depth - 1].to_string();
            previous = current;
        }
    }
    rule
}

/// 逐字符比对一个 fixture 在一个主题下的颜色。
pub fn compare(
    fixture: &Fixture,
    scopes: &RoveScopes,
    theme_key: &str,
    theme: &ResolvedTheme,
    placeholder: bool,
) -> Run {
    let default = theme
        .token_style(&[])
        .foreground
        .unwrap_or(Color(0, 0, 0, 255));
    let mut run = Run {
        language: fixture.language.clone(),
        fixture: fixture.name.clone(),
        theme: theme_key.to_string(),
        chars: 0,
        matched: 0,
        diffs: Vec::new(),
    };
    // 相邻且「同因」（期望颜色、实际颜色、两边的规则都相同）的字符并成一段。
    let mut open: Option<Diff> = None;
    for token in &fixture.tokens {
        let Some(expected) = token.colors.get(theme_key) else {
            continue;
        };
        let vscode_rule = token
            .rules
            .get(theme_key)
            .cloned()
            .unwrap_or_else(|| "default".into());
        for (offset, ch) in token.text.char_indices() {
            let byte = token.start + offset;
            let rove_scopes = scopes.at(byte);
            let actual = theme.token_style(rove_scopes).foreground.unwrap_or(default);
            run.chars += 1;
            if actual == *expected {
                run.matched += 1;
                if let Some(diff) = open.take() {
                    run.diffs.push(diff);
                }
                continue;
            }
            let rove_rule = deciding_scope(theme, rove_scopes);
            let rove_scopes = rove_scopes
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let kind = match (vscode_rule.as_str(), rove_rule.as_str()) {
                ("default", _) => Kind::Extra,
                (_, "default") => Kind::Missing,
                _ => Kind::Different,
            };
            let same_cause = open.as_ref().is_some_and(|d| {
                d.expected == *expected
                    && d.actual == actual
                    && d.vscode_rule == vscode_rule
                    && d.rove_scopes == rove_scopes
            });
            if same_cause {
                let diff = open.as_mut().expect("same_cause implies an open diff");
                diff.text.push(ch);
                diff.chars += 1;
                continue;
            }
            if let Some(diff) = open.take() {
                run.diffs.push(diff);
            }
            let (line, column) = fixture.position(byte);
            open = Some(Diff {
                fixture: fixture.name.clone(),
                line,
                column,
                text: ch.to_string(),
                chars: 1,
                expected: *expected,
                actual,
                vscode_rule: vscode_rule.clone(),
                vscode_scopes: token.scopes.clone(),
                rove_rule,
                cause: cause(&token.scopes, placeholder),
                rove_scopes,
                kind,
            });
        }
    }
    if let Some(diff) = open.take() {
        run.diffs.push(diff);
    }
    run
}

/// 一个字符的比对结果，`--chars` 用它逐字符输出。
#[derive(Debug, Clone)]
pub struct CharColor {
    pub line: usize,
    pub column: usize,
    pub ch: char,
    pub expected: Color,
    pub actual: Color,
    pub vscode_scopes: String,
    pub rove_scopes: String,
}

impl CharColor {
    pub fn matches(&self) -> bool {
        self.expected == self.actual
    }
}

/// 逐字符给出期望颜色与 Rove 实际算出的颜色（比对器的「输出每个字符的颜色」入口）。
pub fn characters(
    fixture: &Fixture,
    scopes: &RoveScopes,
    theme_key: &str,
    theme: &ResolvedTheme,
) -> Vec<CharColor> {
    let default = theme
        .token_style(&[])
        .foreground
        .unwrap_or(Color(0, 0, 0, 255));
    let mut out = Vec::new();
    for token in &fixture.tokens {
        let Some(expected) = token.colors.get(theme_key) else {
            continue;
        };
        for (offset, ch) in token.text.char_indices() {
            let byte = token.start + offset;
            let rove_scopes = scopes.at(byte);
            let actual = theme.token_style(rove_scopes).foreground.unwrap_or(default);
            let (line, column) = fixture.position(byte);
            out.push(CharColor {
                line,
                column,
                ch,
                expected: *expected,
                actual,
                vscode_scopes: token.scopes.clone(),
                rove_scopes: rove_scopes
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
        }
    }
    out
}

/// 跑完整个矩阵：每个 fixture 高亮一次，再对每个主题比一遍。
pub fn run_matrix(
    root: &std::path::Path,
    config: &Config,
    filter: &Filter,
) -> Result<Vec<Run>, String> {
    let themes: BTreeMap<String, ResolvedTheme> = config
        .themes()
        .into_iter()
        .filter(|t| filter.theme.as_ref().is_none_or(|want| want == t))
        .map(|key| load(&key).map(|theme| (key, theme)))
        .collect::<Result<_, _>>()?;
    if themes.is_empty() {
        return Err("没有匹配的主题".into());
    }
    let mut runs = Vec::new();
    for language in &config.languages {
        if filter
            .language
            .as_ref()
            .is_some_and(|want| want != &language.id)
        {
            continue;
        }
        for name in &language.fixtures {
            if filter.fixture.as_ref().is_some_and(|want| want != name) {
                continue;
            }
            let fixture = super::data::load(root, &language.id, name)?;
            let scopes = highlight(&fixture.text, &language.id)?;
            for (key, theme) in &themes {
                runs.push(compare(
                    &fixture,
                    &scopes,
                    key,
                    theme,
                    language.threshold.is_none(),
                ));
            }
        }
    }
    if runs.is_empty() {
        return Err("没有匹配的 fixture".into());
    }
    Ok(runs)
}

#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub language: Option<String>,
    pub fixture: Option<String>,
    pub theme: Option<String>,
}

/// 一个（语言，主题）格子的汇总。
#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub chars: usize,
    pub matched: usize,
    pub diffs: usize,
    /// 按成因分类的差异字符数。
    pub causes: BTreeMap<Cause, usize>,
}

impl Cell {
    /// 扣除「Rove 侧之外的成因」之后的一致率：注入语言未内置、VS Code 判为非法、占位语言
    /// 这三类字符不计入分母，用来回答「剩下的差异里有多少是 Rove 能修的」。
    pub fn adjusted_rate(&self) -> f64 {
        let explained: usize = self
            .causes
            .iter()
            .filter(|(cause, _)| **cause != Cause::Rove)
            .map(|(_, chars)| *chars)
            .sum();
        let total = self.chars.saturating_sub(explained);
        if total == 0 {
            1.0
        } else {
            (self.matched as f64) / (total as f64)
        }
    }
}

impl Cell {
    pub fn rate(&self) -> f64 {
        if self.chars == 0 {
            1.0
        } else {
            self.matched as f64 / self.chars as f64
        }
    }
}

/// 按（语言，主题）汇总各 fixture 的结果。
pub fn summarize(runs: &[Run]) -> BTreeMap<(String, String), Cell> {
    let mut out: BTreeMap<(String, String), Cell> = BTreeMap::new();
    for run in runs {
        let cell = out
            .entry((run.language.clone(), run.theme.clone()))
            .or_default();
        cell.chars += run.chars;
        cell.matched += run.matched;
        cell.diffs += run.diffs.iter().map(|d| d.chars).sum::<usize>();
        for diff in &run.diffs {
            *cell.causes.entry(diff.cause).or_default() += diff.chars;
        }
    }
    out
}

/// 低于阈值的格子（语言、主题、实际一致率、阈值）。只有 `enforced_themes` 参与判定。
pub fn failures(config: &Config, runs: &[Run]) -> Vec<(String, String, f64, f64)> {
    let summary = summarize(runs);
    let mut out = Vec::new();
    for language in &config.languages {
        let Some(threshold) = language.threshold else {
            continue;
        };
        for theme in &config.enforced_themes {
            let Some(cell) = summary.get(&(language.id.clone(), theme.clone())) else {
                continue;
            };
            if cell.rate() < threshold {
                out.push((language.id.clone(), theme.clone(), cell.rate(), threshold));
            }
        }
    }
    out
}
