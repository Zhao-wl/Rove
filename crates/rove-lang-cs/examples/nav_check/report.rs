//! 统计与输出：控制台摘要、机器可读 JSON、Markdown 报告、与上一次结果对比。

use crate::cases::{Category, Project};
use crate::check::{CaseResult, Status};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tally {
    pub pass: usize,
    pub fail: usize,
    pub stale: usize,
    pub unsupported: usize,
}

impl Tally {
    pub fn add(&mut self, s: Status) {
        match s {
            Status::Pass => self.pass += 1,
            Status::Fail => self.fail += 1,
            Status::Stale => self.stale += 1,
            Status::Unsupported => self.unsupported += 1,
        }
    }

    pub fn total(&self) -> usize {
        self.pass + self.fail + self.stale + self.unsupported
    }

    /// 准确率 = 通过 / (通过 + 失败)；过期与不支持不计入分母。
    pub fn accuracy(&self) -> Option<f64> {
        let n = self.pass + self.fail;
        (n > 0).then(|| self.pass as f64 / n as f64)
    }

    pub fn accuracy_text(&self) -> String {
        self.accuracy()
            .map_or_else(|| "—".to_owned(), |a| format!("{:.1}%", a * 100.0))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CategoryRow {
    pub project: String,
    pub category: String,
    #[serde(flatten)]
    pub tally: Tally,
}

/// 机器可读的结果文件。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunReport {
    /// 结果文件格式版本。
    pub format: u32,
    /// 查询来源（l2 / l1）。
    pub source: String,
    /// 各项目实际使用的查询来源说明。
    pub sources: Vec<String>,
    /// 本次跳过的项目及原因。
    pub skipped: Vec<String>,
    pub summary: Vec<CategoryRow>,
    pub cases: Vec<CaseResult>,
}

pub fn build(
    source: &str,
    sources: Vec<String>,
    skipped: Vec<String>,
    cases: Vec<CaseResult>,
) -> RunReport {
    let mut map: BTreeMap<(String, Category), Tally> = BTreeMap::new();
    for c in &cases {
        let cat = Category::ALL
            .into_iter()
            .find(|k| k.key() == c.category)
            .expect("分类来自用例文件");
        map.entry((c.project.clone(), cat))
            .or_default()
            .add(c.status);
    }
    let summary = map
        .into_iter()
        .map(|((project, cat), tally)| CategoryRow {
            project,
            category: cat.key().to_owned(),
            tally,
        })
        .collect();
    RunReport {
        format: 1,
        source: source.to_owned(),
        sources,
        skipped,
        summary,
        cases,
    }
}

fn label_of(key: &str) -> &'static str {
    Category::ALL
        .into_iter()
        .find(|k| k.key() == key)
        .map_or("?", Category::label)
}

fn projects_in(r: &RunReport) -> Vec<String> {
    let mut v: Vec<String> = Project::ALL
        .iter()
        .map(|p| p.key().to_owned())
        .filter(|p| r.summary.iter().any(|row| &row.project == p))
        .collect();
    v.dedup();
    v
}

fn sum<'a>(rows: impl Iterator<Item = &'a CategoryRow>) -> Tally {
    rows.fold(Tally::default(), |mut t, r| {
        t.pass += r.tally.pass;
        t.fail += r.tally.fail;
        t.stale += r.tally.stale;
        t.unsupported += r.tally.unsupported;
        t
    })
}

/// 分类准确率表（Markdown，控制台也直接打印这一份）。
pub fn summary_table(r: &RunReport) -> String {
    let mut s = String::new();
    for p in projects_in(r) {
        let rows: Vec<&CategoryRow> = r.summary.iter().filter(|x| x.project == p).collect();
        let _ = writeln!(s, "\n### 项目 {p}\n");
        let _ = writeln!(s, "| 分类 | 通过 | 失败 | 过期 | 不支持 | 准确率 |");
        let _ = writeln!(s, "|---|---:|---:|---:|---:|---:|");
        for row in &rows {
            let t = row.tally;
            let _ = writeln!(
                s,
                "| {} (`{}`) | {} | {} | {} | {} | {} |",
                label_of(&row.category),
                row.category,
                t.pass,
                t.fail,
                t.stale,
                t.unsupported,
                t.accuracy_text()
            );
        }
        let t = sum(rows.iter().copied());
        let _ = writeln!(
            s,
            "| **合计** | {} | {} | {} | {} | {} |",
            t.pass,
            t.fail,
            t.stale,
            t.unsupported,
            t.accuracy_text()
        );
    }
    let t = sum(r.summary.iter());
    let tr = sum(r.summary.iter().filter(|x| x.category == "type_ref"));
    let _ = writeln!(
        s,
        "\n**全部**：{} 例，通过 {}、失败 {}、过期 {}、不支持 {}，准确率 {}。",
        t.total(),
        t.pass,
        t.fail,
        t.stale,
        t.unsupported,
        t.accuracy_text()
    );
    let _ = writeln!(
        s,
        "**类型引用（#37 验收口径）**：{} 例，通过 {}、失败 {}，准确率 {}。",
        tr.total(),
        tr.pass,
        tr.fail,
        tr.accuracy_text()
    );
    s
}

/// 非通过用例的逐条详情。
pub fn details(r: &RunReport, statuses: &[Status]) -> String {
    let mut s = String::new();
    for c in r.cases.iter().filter(|c| statuses.contains(&c.status)) {
        let _ = writeln!(
            s,
            "- **{}**（{} · {} · {}）`{}`",
            c.id,
            c.project,
            label_of(&c.category),
            c.status.label(),
            c.query.as_deref().unwrap_or("-")
        );
        for reason in &c.reasons {
            let _ = writeln!(s, "  - {reason}");
        }
    }
    s
}

pub fn markdown(r: &RunReport, title: &str) -> String {
    let mut s = format!("# {title}\n\n查询来源：`{}`\n\n", r.source);
    for line in &r.sources {
        let _ = writeln!(s, "- {line}");
    }
    if !r.skipped.is_empty() {
        let _ = writeln!(s, "\n跳过的项目：\n");
        for k in &r.skipped {
            let _ = writeln!(s, "- {k}");
        }
    }
    s.push_str("\n## 分类准确率\n");
    s.push_str(&summary_table(r));
    s.push_str("\n## 失败用例\n\n");
    s.push_str(&details(r, &[Status::Fail]));
    let stale = details(r, &[Status::Stale]);
    if !stale.is_empty() {
        s.push_str("\n## 过期用例（锚点失效，需要重新整理）\n\n");
        s.push_str(&stale);
    }
    s
}

/// 与上一次结果对比：返回（状态变化的说明，由通过变为失败的数量）。
pub fn compare(prev: &RunReport, now: &RunReport) -> (String, usize) {
    let before: BTreeMap<&str, Status> = prev
        .cases
        .iter()
        .map(|c| (c.id.as_str(), c.status))
        .collect();
    let mut s = String::new();
    let mut regressions = 0;
    for c in &now.cases {
        match before.get(c.id.as_str()) {
            Some(&old) if old != c.status => {
                if old == Status::Pass && c.status == Status::Fail {
                    regressions += 1;
                }
                let _ = writeln!(s, "- {}：{} → {}", c.id, old.label(), c.status.label());
            }
            None => {
                let _ = writeln!(s, "- {}：（新增）{}", c.id, c.status.label());
            }
            _ => {}
        }
    }
    if s.is_empty() {
        s.push_str("（与上一次结果相比没有状态变化）\n");
    }
    (s, regressions)
}
