//! 转到文件中的符号（`workbench.action.gotoSymbol`）与快速打开 `@` 前缀（#24）共用的条目与排序。
//!
//! #24 接入时调用同一个 [`symbol_picks`]，传入自己的 `fuzzyScorer` 打分器（[`Matcher`]），
//! 两处的结果与顺序就一致。排序规则同 VS Code `gotoSymbolQuickAccess`：
//! - 没有查询：按文档位置；
//! - 有查询：只保留匹配的符号，按分数从高到低，同分按位置；
//! - 查询以 `:` 开头：按种类分组（组内按位置或分数），每组一个分隔行“种类 (数量)”。
use crate::{
    list_widget::matcher::Matcher,
    lists::{ItemId, ListModel, Row},
};
use rove_base::{Result, Version};
use rove_syntax::symbols::{kind_name, DocumentSymbols, SymbolId};
use std::{ops::Range, sync::Arc};

/// 一个候选。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolPick {
    pub id: SymbolId,
    /// 稳定键（[`ItemId`] 用它，接受时据此找回符号）。
    pub key: Arc<str>,
    pub label: Arc<str>,
    /// 容器名（显示在名称右侧）。
    pub container: Option<Arc<str>>,
    /// 名称中匹配的 UTF-8 字节区间。
    pub highlights: Vec<Range<usize>>,
    pub score: i32,
}

/// 结果：候选与（`:` 分组时的）组起点。
#[derive(Clone, Debug, Default)]
pub struct SymbolPicks {
    pub items: Vec<SymbolPick>,
    /// (组内第一个候选的下标, 种类名, 数量)。
    pub groups: Vec<(usize, &'static str, usize)>,
}

/// 计算候选。`label` 把符号转成显示名（YAML 多文档节点等由界面格式化）。
pub fn symbol_picks(
    symbols: &DocumentSymbols,
    query: &str,
    matcher: &dyn Matcher,
    label: &dyn Fn(SymbolId) -> Arc<str>,
) -> SymbolPicks {
    let (grouped, query) = match query.strip_prefix(':') {
        Some(rest) => (true, rest.trim()),
        None => (false, query.trim()),
    };
    let mut items: Vec<SymbolPick> = symbols
        .iter()
        .filter_map(|(id, node)| {
            let label = label(id);
            let m = matcher.score(query, &label)?;
            Some(SymbolPick {
                id,
                key: node.key.clone(),
                label,
                container: node.container.clone(),
                highlights: m.ranges,
                score: m.score,
            })
        })
        .collect();
    if !query.is_empty() {
        items.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    }
    let mut groups = Vec::new();
    if grouped {
        let kind = |p: &SymbolPick| kind_name_static(symbols, p.id);
        items.sort_by(|a, b| kind(a).cmp(kind(b)));
        let mut start = 0;
        while start < items.len() {
            let k = kind(&items[start]);
            let end = start + items[start..].iter().take_while(|p| kind(p) == k).count();
            groups.push((start, k, end - start));
            start = end;
        }
    }
    SymbolPicks { items, groups }
}

/// 种类名的静态形式（分组标签与字符串 key 用）。
fn kind_name_static(symbols: &DocumentSymbols, id: SymbolId) -> &'static str {
    let name = symbols
        .node(id)
        .map(|n| kind_name(&n.symbol.kind))
        .unwrap_or("");
    super::model::SHOW_SETTINGS
        .iter()
        .map(|(k, _)| *k)
        .chain(["parameter", rove_syntax::symbols::kinds::DOCUMENT])
        .find(|k| *k == name)
        .unwrap_or("other")
}

/// 快速输入的条目模型（`QuickPickConfig::filter = None`，由本模块过滤与排序）。
pub struct PickList {
    rows: Arc<[Row]>,
    revision: Version,
}

impl PickList {
    pub fn new(picks: &SymbolPicks, revision: Version) -> Self {
        let rows = picks
            .items
            .iter()
            .map(|p| Row {
                id: ItemId(p.key.clone()),
                label: p.label.clone(),
                detail: p.container.clone(),
                depth: 0,
                expandable: false,
                expanded: false,
                disabled: false,
            })
            .collect();
        Self { rows, revision }
    }
}

impl ListModel for PickList {
    fn revision(&self) -> Version {
        self.revision
    }
    fn count(&self) -> usize {
        self.rows.len()
    }
    fn rows(&self, visible: Range<usize>) -> Result<Arc<[Row]>> {
        let end = visible.end.min(self.rows.len());
        let start = visible.start.min(end);
        Ok(self.rows[start..end].into())
    }
}
