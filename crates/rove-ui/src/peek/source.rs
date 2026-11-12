//! 速览的数据入口（不依赖 GPUI）：查询接口、能力开关与一次查询的后台流水线。
//!
//! - [`PeekQueries`]：速览只需要“逐条精确度的导航”与“悬停签名”。正式实现是 #38 的
//!   [`CsRouter`]；冻结契约 [`CsQueries`] 经 [`BatchQueries`] 适配（逐条精确度取批级）。
//! - [`Availability`]：**随 #38 能力变化而变化的唯一降级开关**（issue #41 目标第 2 节）。
//!   查询之前问一次：不可用时不打开窗口，在光标处提示；界面代码里不散落任何判断。
//! - [`run`]：后台执行一次查询 → 分组 → 预读初始展开文件的片段 → 外部符号的悬停签名。

use super::model::{PeekKind, PeekResults};
use super::snippets::{self, FileSnippets};
use rove_base::{Cancellation, Error, Result};
use rove_buffer::Documents;
use rove_lang_cs::query::{
    ApproximationReason, CsQueries, Hover, NavigationKind, Precision, QueryPoint, QueryResult,
};
use rove_lang_cs::router::{CsRouter, RoutedTarget, SemanticStatus, TargetRole};
use std::sync::Arc;

/// 速览用到的查询（后台线程调用，Send + Sync）。
pub trait PeekQueries: Send + Sync {
    /// 导航，逐条标记精确度。
    fn navigate(
        &self,
        kind: NavigationKind,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[RoutedTarget]>>>;
    /// 悬停（外部符号的保底片段取它的签名）。
    fn hover(
        &self,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Option<Hover>>>;
    /// 某种查询当前能否给出有意义的结果；见 [`Availability`]。
    fn availability(&self, kind: PeekKind) -> Availability {
        let _ = kind;
        Availability::Available
    }
}

/// 一种速览当前是否可用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// 数据层没有这类关系（例如实现关系只在 L2 语义索引里）：不打开空窗口，光标处提示
    /// “未找到实现”（UX_SPEC §6.1 第 2 级）。
    Unavailable,
}

/// #38 的实现关系只来自 L2 语义索引（L1/L0 回退不给实现）。#38 以后在回退层也能给出关系时，
/// 只改这里。
pub fn router_availability(status: SemanticStatus, kind: PeekKind) -> Availability {
    match kind {
        PeekKind::Implementation if status != SemanticStatus::Ready => Availability::Unavailable,
        _ => Availability::Available,
    }
}

impl PeekQueries for CsRouter {
    fn navigate(
        &self,
        kind: NavigationKind,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[RoutedTarget]>>> {
        self.navigate_detailed(kind, point, cancel)
    }
    fn hover(
        &self,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Option<Hover>>> {
        self.hover_detailed(point, cancel)
    }
    fn availability(&self, kind: PeekKind) -> Availability {
        router_availability(self.semantic_status(), kind)
    }
}

/// 把冻结契约的批级精确度摊到每一条（测试替身与其他 `CsQueries` 实现用）。
pub struct BatchQueries(pub Arc<dyn CsQueries>);

impl PeekQueries for BatchQueries {
    fn navigate(
        &self,
        kind: NavigationKind,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[RoutedTarget]>>> {
        let r = self.0.navigate(kind, point, cancel)?;
        let value = r
            .value
            .iter()
            .map(|t| RoutedTarget {
                target: t.clone(),
                precision: r.precision,
                reason: r.reason.clone(),
                role: if rove_lang_cs::router::metadata_symbol_key(&t.resource).is_some() {
                    TargetRole::Metadata
                } else {
                    TargetRole::Reference
                },
            })
            .collect();
        Ok(QueryResult {
            value,
            precision: r.precision,
            reason: r.reason,
            version: r.version,
            index_generation: r.index_generation,
        })
    }
    fn hover(
        &self,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Option<Hover>>> {
        self.0.hover(point, cancel)
    }
}

/// 一次速览查询的后台产物。
pub struct Loaded {
    pub results: Arc<PeekResults>,
    /// 查询的批级精确度（空结果时区分“确实没有”与“不能确定”）。
    pub precision: Precision,
    pub reason: Option<ApproximationReason>,
    /// 初始展开的文件已经读好的片段（其余文件展开时再读）。
    pub snippets: Vec<(usize, Arc<FileSnippets>)>,
    /// 结果里有外部符号时，查询点的悬停签名（保底片段用）。
    pub hover: Option<Hover>,
}

/// 后台：查询 → 分组 → 预读初始展开文件的片段。在后台执行器上调用。
pub async fn run(
    queries: Arc<dyn PeekQueries>,
    documents: Arc<dyn Documents>,
    kind: PeekKind,
    point: QueryPoint,
    cancel: Cancellation,
) -> Result<Loaded> {
    let result = queries.navigate(kind.navigation(), &point, &cancel)?;
    cancel.check()?;
    let results = Arc::new(PeekResults::group(&result.value));
    let mut loaded = Vec::new();
    for file in results.initially_expanded() {
        cancel.check()?;
        let s = snippets::load(&results, file, &*documents, &cancel).await?;
        loaded.push((file, Arc::new(s)));
    }
    let hover = if results.files.iter().any(|f| f.is_external()) {
        queries.hover(&point, &cancel).ok().and_then(|h| h.value)
    } else {
        None
    };
    Ok(Loaded {
        precision: if results.is_empty() {
            result.precision
        } else {
            results.precision()
        },
        reason: result.reason,
        results,
        snippets: loaded,
        hover,
    })
}

/// 外部符号保底片段需要的签名：悬停签名，没有时用符号键。
pub fn external_signature(hover: Option<&Hover>, key: &str) -> Arc<str> {
    hover
        .map(|h| h.signature.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| key.into())
}

/// 查询失败的分类：只有取消（窗口已关闭）是静默的，其余关闭窗口并在光标处提示。
pub fn is_silent(error: &Error) -> bool {
    matches!(error, Error::Cancelled)
}

/// 失败时提示的口径：数据层未就绪（`NotReady` / `Stale`）按“不能确定”追加“（语义索引构建中）”，
/// 其余（不支持的位置、读不到文件）按“确实没有”。
pub fn error_precision(error: &Error) -> Precision {
    match error {
        Error::NotReady | Error::Stale => Precision::Approximate,
        _ => Precision::Exact,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implementations_depend_on_the_semantic_index_only() {
        for status in [
            SemanticStatus::Building { percent: Some(10) },
            SemanticStatus::NotInstalled,
            SemanticStatus::Disabled,
        ] {
            assert_eq!(
                router_availability(status, PeekKind::Implementation),
                Availability::Unavailable
            );
            assert_eq!(
                router_availability(status, PeekKind::References),
                Availability::Available,
                "引用与定义有 L1/L0 回退"
            );
        }
        assert_eq!(
            router_availability(SemanticStatus::Ready, PeekKind::Implementation),
            Availability::Available
        );
    }
}
