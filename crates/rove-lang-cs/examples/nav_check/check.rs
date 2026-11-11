//! 单个用例的判定：定位 → 通过适配层查询 → 与期望比较。
//!
//! 只通过 [`NavSource`] 查询，不感知索引格式。

use crate::adapter::{NavSource, QueryPos, SourceError, SrcLine};
use crate::cases::{Case, LineAnchor};
use crate::locate::{locate_line, locate_token, SourceText};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Fail,
    /// 锚点失效：代码已变，用例需要重新整理。计入统计，但不算失败。
    Stale,
    /// 查询来源不提供该查询（例如 L1 未接入）。
    Unsupported,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Pass => "通过",
            Status::Fail => "失败",
            Status::Stale => "过期",
            Status::Unsupported => "不支持",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseResult {
    pub id: String,
    pub project: String,
    pub category: String,
    pub status: Status,
    /// 失败或过期的原因，逐条列出。
    pub reasons: Vec<String>,
    /// 实际查询的位置（1 起算的 `路径:行:列`，列按字符计）。
    pub query: Option<String>,
    /// 查询点或期望位置是否经过锚点重定位。
    pub relocated: bool,
    pub expected_defs: Vec<String>,
    pub expected_external: Option<String>,
    pub actual_defs: Vec<String>,
    pub actual_external: Option<String>,
    pub actual_symbol: Option<String>,
    pub expected_refs: Option<u32>,
    pub actual_refs: Option<usize>,
    /// 源码文件比索引新（结果可能受影响）。
    pub source_newer_than_index: bool,
}

/// 按项目根目录缓存读过的源码文件。
pub struct Workspace {
    root: PathBuf,
    cache: HashMap<String, Option<SourceText>>,
}

impl Workspace {
    pub fn new(root: &Path) -> Self {
        Workspace {
            root: root.to_owned(),
            cache: HashMap::new(),
        }
    }

    fn text(&mut self, rel: &str) -> Option<&SourceText> {
        let root = &self.root;
        self.cache
            .entry(rel.to_owned())
            .or_insert_with(|| SourceText::load(&root.join(rel)).ok())
            .as_ref()
    }

    fn modified(&self, rel: &str) -> Option<std::time::SystemTime> {
        std::fs::metadata(self.root.join(rel))
            .and_then(|m| m.modified())
            .ok()
    }

    fn resolve(
        &mut self,
        a: &LineAnchor,
        window: u32,
        moved: &mut bool,
    ) -> Result<SrcLine, String> {
        let text = self
            .text(&a.file)
            .ok_or_else(|| format!("期望位置的文件不存在：{}", a.file))?;
        let hit = locate_line(text, a.line, &a.anchor, window)
            .map_err(|e| format!("期望位置 {}:{} {e}", a.file, a.line))?;
        *moved |= hit.moved;
        Ok(SrcLine {
            file: a.file.clone(),
            line0: hit.line0,
        })
    }
}

fn unsupported_msg(what: &str, e: SourceError) -> String {
    let SourceError::Unsupported(m) = e;
    format!("{what}：{m}")
}

pub fn run_case(
    project: &str,
    case: &Case,
    ws: &mut Workspace,
    source: &dyn NavSource,
    window: u32,
) -> CaseResult {
    let mut r = CaseResult {
        id: case.id.clone(),
        project: project.to_owned(),
        category: case.category.key().to_owned(),
        status: Status::Pass,
        reasons: Vec::new(),
        query: None,
        relocated: false,
        expected_defs: Vec::new(),
        expected_external: case.expect.external.clone(),
        actual_defs: Vec::new(),
        actual_external: None,
        actual_symbol: None,
        expected_refs: case.expect.refs,
        actual_refs: None,
        source_newer_than_index: false,
    };
    let stale = |mut r: CaseResult, why: String| {
        r.status = Status::Stale;
        r.reasons.push(why);
        r
    };

    // 1. 定位查询点与全部期望位置（任一锚点失效都标为过期）。
    let Some(text) = ws.text(&case.file) else {
        return stale(r, format!("文件不存在：{}", case.file));
    };
    let hit = match locate_token(text, case.line, case.col, &case.token, &case.anchor, window) {
        Ok(h) => h,
        Err(e) => return stale(r, format!("查询点：{e}")),
    };
    r.relocated = hit.moved;
    r.query = Some(format!(
        "{}:{}:{}",
        case.file,
        hit.line0 + 1,
        hit.char_col0 + 1
    ));
    let mut want_defs = BTreeSet::new();
    for a in &case.expect.defs {
        match ws.resolve(a, window, &mut r.relocated) {
            Ok(l) => {
                want_defs.insert(l);
            }
            Err(e) => return stale(r, e),
        }
    }
    let mut want_refs = Vec::new();
    for a in &case.expect.refs_include {
        match ws.resolve(a, window, &mut r.relocated) {
            Ok(l) => want_refs.push(l),
            Err(e) => return stale(r, e),
        }
    }
    r.expected_defs = want_defs.iter().map(ToString::to_string).collect();
    if let (Some(built), Some(m)) = (source.built_at(), ws.modified(&case.file)) {
        r.source_newer_than_index = m > built;
    }

    // 2. 查询并比较。
    let at = QueryPos {
        file: &case.file,
        line0: hit.line0,
        utf8_col0: hit.utf8_col0,
    };
    let mut unsupported = Vec::new();
    let mut answered = 0;
    match source.definition(&at) {
        Ok(ans) => {
            answered += 1;
            let got: BTreeSet<SrcLine> = ans.defs.iter().cloned().collect();
            r.actual_defs = got.iter().map(ToString::to_string).collect();
            r.actual_external = ans.external.clone();
            r.actual_symbol = ans.symbol.clone();
            if let Some(ext) = &case.expect.external {
                if ans.external.as_deref() != Some(ext.as_str()) {
                    r.reasons.push(match (&ans.external, got.is_empty()) {
                        (Some(a), _) => format!("定义：期望外部符号 {ext}，实际为外部符号 {a}"),
                        (None, false) => format!(
                            "定义：期望外部符号 {ext}，实际跳到源码 {}",
                            r.actual_defs.join(", ")
                        ),
                        (None, true) => format!(
                            "定义：期望外部符号 {ext}，实际没有落点（无元数据视图）；绑定到 {}",
                            ans.symbol.as_deref().unwrap_or("?")
                        ),
                    });
                }
            } else if got != want_defs {
                r.reasons.push(format!(
                    "定义：期望 [{}]，实际 [{}]；绑定到 {}",
                    r.expected_defs.join(", "),
                    r.actual_defs.join(", "),
                    ans.symbol.as_deref().unwrap_or("?")
                ));
            }
            check_precision(case, ans.precision, "定义", &mut r.reasons);
        }
        Err(e) => unsupported.push(unsupported_msg("定义", e)),
    }
    if case.expect.refs.is_some() || !want_refs.is_empty() {
        match source.references(&at) {
            Ok(ans) => {
                answered += 1;
                r.actual_refs = Some(ans.refs.len());
                if let Some(n) = case.expect.refs {
                    if ans.refs.len() != n as usize {
                        r.reasons
                            .push(format!("引用：期望 {n} 处，实际 {} 处", ans.refs.len()));
                    }
                }
                let got: BTreeSet<&SrcLine> = ans.refs.iter().collect();
                let missing: Vec<String> = want_refs
                    .iter()
                    .filter(|w| !got.contains(w))
                    .map(ToString::to_string)
                    .collect();
                if !missing.is_empty() {
                    r.reasons.push(format!(
                        "引用：缺少 [{}]（实际共 {} 处）",
                        missing.join(", "),
                        ans.refs.len()
                    ));
                }
                check_precision(case, ans.precision, "引用", &mut r.reasons);
            }
            Err(e) => unsupported.push(unsupported_msg("引用", e)),
        }
    }
    r.status = if !r.reasons.is_empty() {
        Status::Fail
    } else if answered == 0 {
        Status::Unsupported
    } else {
        Status::Pass
    };
    r.reasons
        .extend(unsupported.into_iter().map(|m| format!("（未检查）{m}")));
    r
}

fn check_precision(case: &Case, got: Option<&str>, what: &str, reasons: &mut Vec<String>) {
    if let (Some(want), Some(got)) = (&case.expect.precision, got) {
        if want != got {
            reasons.push(format!("{what}：期望精确度 {want}，实际 {got}"));
        }
    }
}
