//! #38 查询路由：决定一次 C# 导航查询该问 L2 / L1 / L0 中的哪一层，并把结果翻译回用户当前看到的坐标。
//!
//! 规则（`LANGUAGE_INTELLIGENCE` §3.2）：
//!
//! ```text
//! 请求（文件, 位置）
//!   ├─ 有 L2 快照，且文件在索引里？
//!   │     ├─ 文件与索引时一致 → 查 L2 → 精确
//!   │     ├─ 有编辑记录（缓冲区基线 = 索引版本）→ 位置映射：
//!   │     │      ├─ 映射成功且所在行未修改 → 查 L2 → 精确（结果再反向映射）
//!   │     │      └─ 否则 → 回退
//!   │     └─ 无法映射（磁盘内容与索引不同且没有编辑记录）→ 回退，并通知生命周期重建该文件
//!   └─ 没有 L2 快照（构建中 / 未安装 / 已禁用 / 失败）→ 回退，原因写在结果里
//! 回退：L0（当前文件语法树上的局部变量、参数、局部函数、lambda 参数，标为“语法”）
//!     → L1（类型名解析、同名成员候选，标为“近似”）→ 空结果（标为“近似”并带原因）
//! ```
//!
//! **精确度是逐条的**：[`CsRouter::navigate_detailed`] 给每个结果单独标记（引用列表里落在已修改行上
//! 的结果是近似，其余仍是精确）。冻结契约 [`CsQueries::navigate`] 只有一个批级精确度，按契约取所有
//! 结果中最弱的一个，不会把近似结果标成精确。
//!
//! 空结果的含义：精确度为 `Exact` 时表示“确实没有”；否则表示“不能确定”，`reason` 说明原因
//! （`IndexNotReady` 时界面从 [`CsRouter::semantic_status`] 取“构建中 xx%”）。
//!
//! 线程：全部方法同步执行，必须在后台线程调用（mmap 可能缺页，L0 回退会解析整个文件）。
pub mod buffers;
pub mod l0;
pub mod mapping;
mod metadata;
#[doc(hidden)]
pub mod testing;
#[cfg(test)]
mod tests;

pub use buffers::{hash_bytes, BufferState, BufferStates};
pub use mapping::{EditMap, LineEdit, Mapped};

use crate::index::{CsIndex, Occurrence, Role, REL_BASE_TYPE};
use crate::l1::L1Index;
use crate::lifecycle::{IndexLifecycle, IndexSnapshot, IndexState, SourceChange};
use crate::overlay::L2View;
use crate::query::{
    ApproximationReason, CsQueries, Hover, NavigationKind, Precision, QueryPoint, QueryResult,
    SemanticToken, Target, WorkspaceSymbol,
};
use l0::L0File;
use rove_base::{
    Cancellation, Error, Line, Position, Resource, Result, Span, TextRange, Utf8Column, Version,
    WorkspacePath,
};
use rove_syntax::Symbol;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

/// 外部符号（没有源码位置）的元数据视图资源：`rove-cs-metadata:<符号键>`，符号键为
/// `程序集|文档注释 ID`。#42 用 [`metadata_symbol_key`] 取回符号键并打开 #34 生成的视图。
pub const METADATA_SCHEME: &str = "rove-cs-metadata:";

/// 元数据视图资源里的符号键。
pub fn metadata_symbol_key(resource: &Resource) -> Option<&str> {
    match resource {
        Resource::Virtual(uri) => uri.strip_prefix(METADATA_SCHEME),
        _ => None,
    }
}

/// L1 回退结果的原因（与 #37 的约定一致）。
const L1_REASON: &str = "l1-declaration-index";
/// 查询点在 `#if` 不生效的区域里（L2 没有信息），回退结果的原因。
pub const INACTIVE_REASON: &str = "inactive-preprocessor-region";
/// 工作区符号最多返回多少个符号（partial 类型可能展开成多条）。
const WORKSPACE_SYMBOL_LIMIT: usize = 256;

/// 语义索引（L2）的可用状态，供界面显示“语义索引构建中 xx%”。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticStatus {
    Ready,
    Building { percent: Option<u8> },
    NotInstalled,
    Disabled,
    Failed,
    NoCSharp,
}

impl SemanticStatus {
    /// 没有可查询的 L2 快照时，结果里写的原因。
    pub fn reason(self) -> ApproximationReason {
        match self {
            Self::Ready | Self::Building { .. } => ApproximationReason::IndexNotReady,
            Self::NotInstalled => ApproximationReason::ComponentUnavailable,
            Self::Disabled => ApproximationReason::IndexDisabled,
            Self::Failed => ApproximationReason::Other("index-failed".into()),
            Self::NoCSharp => ApproximationReason::Other("no-csharp-project".into()),
        }
    }
}

/// 持有一份打开的 L2 索引（生命周期的快照，或测试/基准直接打开的索引）。
pub trait IndexHolder: Send + Sync {
    fn index(&self) -> &CsIndex;
    /// 查询用的视图：基础索引 + 覆盖层（#35）。没有覆盖层的持有者就是基础索引本身。
    fn view(&self) -> L2View<'_> {
        L2View::new(self.index(), None)
    }
}

impl IndexHolder for IndexSnapshot {
    fn index(&self) -> &CsIndex {
        IndexSnapshot::index(self)
    }
    fn view(&self) -> L2View<'_> {
        IndexSnapshot::view(self)
    }
}

impl IndexHolder for CsIndex {
    fn index(&self) -> &CsIndex {
        self
    }
}

/// 一次查询期间使用的 L2 快照；持有它就保证底层映射不被替换掉。
#[derive(Clone)]
pub struct L2Snapshot {
    pub holder: Arc<dyn IndexHolder>,
    pub generation: u64,
    /// false：索引已知与项目对不上（配置变化等），正在重建，所有 L2 结果按近似处理。
    pub trusted: bool,
}

/// 路由向 #36 取的数据：当前快照、状态、过期文件，以及“请重建这个文件”。
pub trait SemanticSource: Send + Sync {
    fn snapshot(&self) -> Option<L2Snapshot>;
    fn status(&self) -> SemanticStatus;
    /// 文件在磁盘上被修改过、索引还没更新。
    fn is_stale(&self, path: &str) -> bool;
    /// 有没有任何过期文件（每次查询调用一次，让无过期文件时不必逐文件查询）。
    fn has_stale(&self) -> bool {
        true
    }
    /// 文件的磁盘内容与索引不一致：请求增量重建（#35：转发给常驻索引进程，没有常驻进程时由 #36 标记过期）。
    fn request_update(&self, path: &str);
}

impl SemanticSource for IndexLifecycle {
    fn snapshot(&self) -> Option<L2Snapshot> {
        IndexLifecycle::snapshot(self).map(|s| L2Snapshot {
            generation: s.generation,
            trusted: s.trusted,
            holder: s,
        })
    }
    fn status(&self) -> SemanticStatus {
        match self.status().state {
            IndexState::Ready | IndexState::Updating { .. } => SemanticStatus::Ready,
            IndexState::Checking => SemanticStatus::Building { percent: None },
            IndexState::Building { progress, .. } => SemanticStatus::Building {
                percent: Some(progress.percent),
            },
            IndexState::NotInstalled => SemanticStatus::NotInstalled,
            IndexState::Disabled => SemanticStatus::Disabled,
            IndexState::Failed(_) => SemanticStatus::Failed,
            IndexState::NoCSharp => SemanticStatus::NoCSharp,
        }
    }
    fn is_stale(&self, path: &str) -> bool {
        IndexLifecycle::is_stale(self, path)
    }
    fn has_stale(&self) -> bool {
        self.status().info.stale_files > 0
    }
    fn request_update(&self, path: &str) {
        self.files_changed(&[SourceChange::Changed(path.to_owned())]);
    }
}

/// 手动控制的语义来源：一份固定的索引（或没有），状态、过期文件与重建请求都由调用方设置/读取。
/// 用于测试、基准与 `nav_check`。
pub struct ManualSemantic {
    state: Mutex<(Option<L2Snapshot>, SemanticStatus)>,
    stale: Mutex<HashSet<String>>,
    requested: Mutex<Vec<String>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ManualSemantic {
    /// 就绪的固定索引（世代 1）。
    pub fn ready(index: Arc<dyn IndexHolder>) -> Self {
        let snapshot = L2Snapshot {
            holder: index,
            generation: 1,
            trusted: true,
        };
        Self::with(Some(snapshot), SemanticStatus::Ready)
    }
    /// 没有索引，状态为 `status`。
    pub fn unavailable(status: SemanticStatus) -> Self {
        Self::with(None, status)
    }
    fn with(snapshot: Option<L2Snapshot>, status: SemanticStatus) -> Self {
        Self {
            state: Mutex::new((snapshot, status)),
            stale: Mutex::default(),
            requested: Mutex::default(),
        }
    }
    pub fn set(&self, snapshot: Option<L2Snapshot>, status: SemanticStatus) {
        *lock(&self.state) = (snapshot, status);
    }
    pub fn mark_stale(&self, path: &str) {
        lock(&self.stale).insert(path.to_owned());
    }
    /// 至今收到的重建请求（按顺序）。
    pub fn requested(&self) -> Vec<String> {
        lock(&self.requested).clone()
    }
}

impl SemanticSource for ManualSemantic {
    fn snapshot(&self) -> Option<L2Snapshot> {
        lock(&self.state).0.clone()
    }
    fn status(&self) -> SemanticStatus {
        lock(&self.state).1
    }
    fn is_stale(&self, path: &str) -> bool {
        lock(&self.stale).contains(path)
    }
    fn has_stale(&self) -> bool {
        !lock(&self.stale).is_empty()
    }
    fn request_update(&self, path: &str) {
        lock(&self.requested).push(path.to_owned());
    }
}

/// 结果在目标处扮演的角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetRole {
    /// 符号的定义（声明）处。
    Definition,
    /// 引用处（索引的 role 只区分定义与引用）。
    Reference,
    /// L1 给出的同名候选。
    Candidate,
    /// 外部符号的元数据视图（[`METADATA_SCHEME`]）。
    Metadata,
}

/// 带逐条精确度的导航结果。
#[derive(Clone, Debug)]
pub struct RoutedTarget {
    pub target: Target,
    pub precision: Precision,
    pub reason: Option<ApproximationReason>,
    pub role: TargetRole,
}

/// 精确度强弱：精确 > 近似 > 语法。
fn rank(p: Precision) -> u8 {
    match p {
        Precision::Exact => 0,
        Precision::Approximate => 1,
        Precision::Syntax => 2,
    }
}

/// 一个文件相对索引版本的状态。
#[derive(Clone)]
enum FileState {
    Clean,
    Mapped(Arc<EditMap>),
    /// 内容与索引不同且没有可用的编辑记录：位置无法映射。
    Diverged,
}

/// 查询点定位到 L2 的结果。
enum Located {
    Symbol {
        snap: L2Snapshot,
        occurrence: Occurrence,
        /// 查询侧的精确度上限（索引不可信时为近似）。
        cap: Cap,
        query_state: FileState,
    },
    Nothing {
        generation: u64,
        cap: Cap,
    },
    Fallback {
        reason: ApproximationReason,
        generation: u64,
    },
}

#[derive(Clone)]
struct Cap {
    precision: Precision,
    reason: Option<ApproximationReason>,
}

impl Cap {
    fn of(snap: &L2Snapshot) -> Self {
        if snap.trusted {
            Self {
                precision: Precision::Exact,
                reason: None,
            }
        } else {
            Self {
                precision: Precision::Approximate,
                reason: Some(ApproximationReason::IndexNotReady),
            }
        }
    }
}

/// 统一的 C# 查询入口（实现 #1 的 [`CsQueries`]）。
pub struct CsRouter {
    semantic: Arc<dyn SemanticSource>,
    buffers: Arc<BufferStates>,
    l1: Option<Arc<L1Index>>,
    root: Option<PathBuf>,
    /// #42：元数据视图的内容来源（视图内的查询点由它翻译）；没有时视图内的查询 `Unsupported`。
    views: Option<Arc<crate::metadata_view::MetadataViews>>,
    /// 最近一次 L0 解析（按路径与版本）。
    l0_cache: Mutex<Option<(String, Version, Arc<L0File>)>>,
    /// 已经请求过重建的（文件, 当前内容哈希），避免每次查询都请求。
    requested: Mutex<HashSet<(String, u64)>>,
}

impl CsRouter {
    pub fn new(semantic: Arc<dyn SemanticSource>, buffers: Arc<BufferStates>) -> Self {
        Self {
            semantic,
            buffers,
            l1: None,
            root: None,
            views: None,
            l0_cache: Mutex::default(),
            requested: Mutex::default(),
        }
    }

    /// L1 快速声明索引（#37）；没有时回退只到 L0。
    pub fn with_l1(mut self, l1: Arc<L1Index>) -> Self {
        self.l1 = Some(l1);
        self
    }

    /// 项目根目录：把 `Resource::File` 换成项目相对路径，并在文件未打开时读盘做 L0 回退。
    pub fn with_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// 元数据视图（#42）：视图文档内的位置 → 符号。路由重建时沿用同一个实例（`showDocs` 状态在里面）。
    pub fn with_metadata(mut self, views: Arc<crate::metadata_view::MetadataViews>) -> Self {
        self.views = Some(views);
        self
    }

    pub fn metadata_views(&self) -> Option<&Arc<crate::metadata_view::MetadataViews>> {
        self.views.as_ref()
    }

    pub fn buffers(&self) -> &Arc<BufferStates> {
        &self.buffers
    }

    /// L2 的状态（界面显示“语义索引构建中 xx%”）。
    pub fn semantic_status(&self) -> SemanticStatus {
        self.semantic.status()
    }

    /// 导航，逐条标记精确度。
    pub fn navigate_detailed(
        &self,
        kind: NavigationKind,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[RoutedTarget]>>> {
        cancel.check()?;
        if crate::metadata_view::view_key(&point.resource).is_some() {
            return self.metadata_navigate(kind, point, cancel);
        }
        let (rel, doc) = self.open_point(point)?;
        match self.locate(&rel, doc.as_ref(), point.position) {
            Located::Symbol {
                snap,
                occurrence,
                cap,
                ..
            } => self.l2_navigate(kind, &snap, occurrence, cap, point.version, cancel),
            Located::Nothing { generation, cap } => Ok(QueryResult {
                value: Arc::from([]),
                precision: cap.precision,
                reason: cap.reason,
                version: Some(point.version),
                index_generation: generation,
            }),
            Located::Fallback { reason, generation } => {
                self.fallback_navigate(kind, &rel, doc.as_ref(), point, reason, generation, cancel)
            }
        }
    }

    fn rel_path(&self, resource: &Resource) -> Option<String> {
        match resource {
            Resource::Workspace(p) => Some(p.as_str().to_owned()),
            Resource::File(abs) => {
                let rel = abs.strip_prefix(self.root.as_ref()?).ok()?;
                let rel = rel.to_str()?.replace('\\', "/");
                WorkspacePath::new(&rel).ok().map(|_| rel)
            }
            Resource::Virtual(_) => None,
        }
    }

    /// 查询点的项目相对路径与缓冲区状态；缓冲区版本与请求不一致时返回 `Stale`。
    fn open_point(&self, point: &QueryPoint) -> Result<(String, Option<BufferState>)> {
        let rel = self.rel_path(&point.resource).ok_or(Error::Unsupported)?;
        let doc = self.buffers.get(&rel);
        if doc.as_ref().is_some_and(|d| d.version != point.version) {
            return Err(Error::Stale);
        }
        Ok((rel, doc))
    }

    fn file_state(
        &self,
        index: L2View<'_>,
        file: u32,
        rel: &str,
        doc: Option<&BufferState>,
        has_stale: bool,
    ) -> FileState {
        // 文件在索引中的版本：覆盖层重绑过的文件取覆盖层里的哈希（#35 向 #38 暴露的“索引版本”）。
        let indexed = index.file_hash(file).unwrap_or(0);
        match doc {
            Some(d) if d.base_hash == indexed => {
                // 编辑记录仍然有效；磁盘已经变了（保存或外部修改后重新载入）就请求重建。
                if let Some(hash) = d.current_hash.filter(|&h| h != indexed) {
                    self.request_update(rel, hash);
                }
                if d.edits.is_empty() {
                    FileState::Clean
                } else {
                    FileState::Mapped(d.edits.clone())
                }
            }
            Some(d) if d.current_hash == Some(indexed) => {
                // 保存后索引已经重建：编辑记录作废。
                self.buffers.rebase(rel, d.version, indexed);
                FileState::Clean
            }
            Some(d) => {
                // 磁盘内容（保存或外部修改后）与索引不同：请求重建。未保存的编辑不请求——
                // 索引进程只读磁盘，重建也得不到缓冲区里的内容。
                if let Some(hash) = d.current_hash {
                    self.request_update(rel, hash);
                }
                FileState::Diverged
            }
            None if has_stale && self.semantic.is_stale(rel) => FileState::Diverged,
            None => FileState::Clean,
        }
    }

    fn request_update(&self, rel: &str, hash: u64) {
        if lock(&self.requested).insert((rel.to_owned(), hash)) {
            self.semantic.request_update(rel);
        }
    }

    fn locate(&self, rel: &str, doc: Option<&BufferState>, position: Position) -> Located {
        let Some(snap) = self.semantic.snapshot() else {
            return Located::Fallback {
                reason: self.semantic.status().reason(),
                generation: 0,
            };
        };
        let generation = snap.generation;
        let index = snap.holder.view();
        let Some(file) = index.file_id(rel) else {
            return Located::Fallback {
                reason: ApproximationReason::Other("file-not-indexed".into()),
                generation,
            };
        };
        let has_stale = doc.is_none() && self.semantic.has_stale();
        let state = self.file_state(index, file, rel, doc, has_stale);
        let old = match &state {
            FileState::Clean => position,
            FileState::Mapped(map) => match map.new_to_old_clean(position) {
                Mapped::At(p) => p,
                Mapped::Modified => {
                    return Located::Fallback {
                        reason: ApproximationReason::ModifiedRegion,
                        generation,
                    }
                }
            },
            FileState::Diverged => {
                return Located::Fallback {
                    reason: ApproximationReason::ModifiedRegion,
                    generation,
                }
            }
        };
        // `#if` 不生效的代码不参与编译，L2 在这里没有信息：“没有结果”不能当成精确的答案。
        let inactive = index.inactive_lines(file);
        let i = inactive.partition_point(|r| u64::from(r.end) <= old.line.0);
        if inactive
            .get(i)
            .is_some_and(|r| u64::from(r.start) <= old.line.0)
        {
            return Located::Fallback {
                reason: ApproximationReason::Other(INACTIVE_REASON.into()),
                generation,
            };
        }
        let cap = Cap::of(&snap);
        let found = u32::try_from(old.line.0)
            .ok()
            .and_then(|line| index.occurrence_at(file, line, old.column));
        match found {
            Some(occurrence) => Located::Symbol {
                snap,
                occurrence,
                cap,
                query_state: state,
            },
            None => Located::Nothing { generation, cap },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn l2_navigate(
        &self,
        kind: NavigationKind,
        snap: &L2Snapshot,
        at: Occurrence,
        cap: Cap,
        version: Version,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[RoutedTarget]>>> {
        let index = snap.holder.view();
        let symbol = at.symbol;
        let symbols: Vec<u32> = match kind {
            NavigationKind::Definition => vec![symbol],
            NavigationKind::TypeDefinition => {
                // 索引不记录符号的类型：只有类型本身能回答（= 定义）。
                if index.kind(symbol) != Some(KIND_NAMED_TYPE) {
                    return Err(Error::Unsupported);
                }
                vec![symbol]
            }
            NavigationKind::References => Vec::new(),
            NavigationKind::Implementations => {
                let is_type = index.kind(symbol) == Some(KIND_NAMED_TYPE);
                closure(symbol, cancel, |s| {
                    index
                        .implementations(s)
                        .into_iter()
                        .filter(|r| (r.kind == REL_BASE_TYPE) == is_type)
                        .map(|r| r.from)
                        .collect()
                })?
            }
            NavigationKind::DerivedTypes => closure(symbol, cancel, |s| {
                index
                    .implementations(s)
                    .into_iter()
                    .filter(|r| r.kind == REL_BASE_TYPE)
                    .map(|r| r.from)
                    .collect()
            })?,
            NavigationKind::BaseTypes => closure(symbol, cancel, |s| {
                index
                    .bases(s)
                    .into_iter()
                    .filter(|r| r.kind == REL_BASE_TYPE)
                    .map(|r| r.to)
                    .collect()
            })?,
            NavigationKind::Overrides => closure(symbol, cancel, |s| {
                index
                    .bases(s)
                    .into_iter()
                    .filter(|r| r.kind != REL_BASE_TYPE)
                    .map(|r| r.to)
                    .collect()
            })?,
        };
        let mut mapper = ResultMapper::new(self, index, cap.clone());
        let mut out = Vec::new();
        if matches!(kind, NavigationKind::References) {
            for (n, o) in index.occurrences(symbol).into_iter().enumerate() {
                if n % 1024 == 1023 {
                    cancel.check()?;
                }
                let role = if Role(o.role).is_def() {
                    TargetRole::Definition
                } else {
                    TargetRole::Reference
                };
                if let Some(t) = mapper.map(&o, role) {
                    out.push(t);
                }
            }
        } else {
            for s in symbols {
                cancel.check()?;
                let before = out.len();
                for o in index.definitions(s) {
                    if let Some(t) = mapper.map(&o, TargetRole::Definition) {
                        out.push(t);
                    }
                }
                if out.len() == before {
                    // 没有源码位置：外部符号指向元数据视图。
                    if let Some(key) = index.global_key(s) {
                        out.push(RoutedTarget {
                            target: Target {
                                resource: Resource::Virtual(
                                    format!("{METADATA_SCHEME}{key}").into(),
                                ),
                                range: empty_range(Position::default()),
                                selection: empty_range(Position::default()),
                            },
                            precision: cap.precision,
                            reason: cap.reason.clone(),
                            role: TargetRole::Metadata,
                        });
                    }
                }
            }
        }
        Ok(batch(out, cap, version, snap.generation))
    }

    #[allow(clippy::too_many_arguments)]
    fn fallback_navigate(
        &self,
        kind: NavigationKind,
        rel: &str,
        doc: Option<&BufferState>,
        point: &QueryPoint,
        reason: ApproximationReason,
        generation: u64,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[RoutedTarget]>>> {
        let empty = |precision| QueryResult {
            value: Arc::from([]),
            precision,
            reason: Some(reason.clone()),
            version: Some(point.version),
            index_generation: generation,
        };
        let Some(file) = self.l0_file(rel, doc, point.version) else {
            return Ok(empty(Precision::Approximate));
        };
        cancel.check()?;
        let offset = file.offset(point.position);
        // L0：局部变量与参数。
        if let Some(decl) = offset.and_then(|o| file.resolve(o)) {
            let resource = point.resource.clone();
            let syntax = |range: &std::ops::Range<usize>, role| {
                let r = file.range(range);
                RoutedTarget {
                    target: Target {
                        resource: resource.clone(),
                        range: r.clone(),
                        selection: r,
                    },
                    precision: Precision::Syntax,
                    reason: Some(reason.clone()),
                    role,
                }
            };
            let value: Vec<RoutedTarget> = match kind {
                NavigationKind::Definition => vec![syntax(&decl.name, TargetRole::Definition)],
                NavigationKind::References => file
                    .references(&decl)
                    .iter()
                    .map(|r| {
                        let role = if *r == decl.name {
                            TargetRole::Definition
                        } else {
                            TargetRole::Reference
                        };
                        syntax(r, role)
                    })
                    .collect(),
                _ => Vec::new(),
            };
            return Ok(QueryResult {
                value: value.into(),
                precision: Precision::Syntax,
                reason: Some(reason),
                version: Some(point.version),
                index_generation: generation,
            });
        }
        // L1：类型名解析，再退到同名成员候选。
        let (Some(l1), Some(offset)) = (&self.l1, offset) else {
            return Ok(empty(Precision::Approximate));
        };
        if !matches!(
            kind,
            NavigationKind::Definition | NavigationKind::TypeDefinition
        ) {
            return Ok(empty(Precision::Approximate));
        }
        let Some((ident, type_text)) = identifier_at(&file.text, offset) else {
            return Ok(empty(Precision::Approximate));
        };
        let path = WorkspacePath::new(rel)?;
        let mut candidates = match l1.resolve_type(&path, point.position, &type_text, cancel) {
            Ok(r) => r.value.to_vec(),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => Vec::new(),
        };
        if candidates.is_empty() && matches!(kind, NavigationKind::Definition) {
            candidates = l1.member_candidates(ident, cancel)?.value.to_vec();
        }
        let value: Vec<RoutedTarget> = candidates
            .iter()
            .flat_map(|s| s.locations.iter())
            .map(|target| RoutedTarget {
                target: target.clone(),
                precision: Precision::Approximate,
                reason: Some(reason.clone()),
                role: TargetRole::Candidate,
            })
            .collect();
        Ok(QueryResult {
            value: value.into(),
            precision: Precision::Approximate,
            reason: Some(reason),
            version: Some(point.version),
            index_generation: generation,
        })
    }

    /// 当前文本的语法树：打开的缓冲区按版本缓存，未打开的文件读盘（需要 `with_root`）。
    fn l0_file(
        &self,
        rel: &str,
        doc: Option<&BufferState>,
        version: Version,
    ) -> Option<Arc<L0File>> {
        if let Some((path, v, file)) = &*lock(&self.l0_cache) {
            if doc.is_some() && path == rel && *v == version {
                return Some(file.clone());
            }
        }
        let text = match doc {
            Some(d) => buffers::read_all(&*d.text).ok()?,
            None => {
                let bytes = std::fs::read(self.root.as_ref()?.join(rel)).ok()?;
                let text = String::from_utf8_lossy(&bytes);
                text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
            }
        };
        let file = Arc::new(L0File::parse(text)?);
        if doc.is_some() {
            *lock(&self.l0_cache) = Some((rel.to_owned(), version, file.clone()));
        }
        Some(file)
    }

    /// 悬停：签名 + 文档注释。
    pub fn hover_detailed(
        &self,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Option<Hover>>> {
        cancel.check()?;
        let (rel, doc) = self.open_point(point)?;
        let result = |value, precision, reason, generation| QueryResult {
            value,
            precision,
            reason,
            version: Some(point.version),
            index_generation: generation,
        };
        match self.locate(&rel, doc.as_ref(), point.position) {
            Located::Symbol {
                snap,
                occurrence: o,
                cap,
                query_state,
            } => {
                let view = snap.holder.view();
                let index = view.base;
                let text_of = |list: &[crate::index::SymbolText]| {
                    let i = list.partition_point(|t| t.symbol < o.symbol);
                    list.get(i)
                        .filter(|t| t.symbol == o.symbol)
                        .and_then(|t| index.str(t.text).ok())
                        .map(str::to_owned)
                };
                let display = view.display(o.symbol).unwrap_or_default().to_owned();
                let signature = text_of(index.signatures()).unwrap_or(display);
                let documentation = text_of(index.xml_docs()).unwrap_or_default();
                let start = position(o.line, o.col.0);
                let start = match &query_state {
                    FileState::Mapped(map) => map.old_to_new_nearest(start),
                    _ => start,
                };
                let range = Span::new(start, shifted(start, o.len))?;
                let hover = Hover {
                    signature: signature.into(),
                    documentation: documentation.into(),
                    range,
                };
                Ok(result(
                    Some(hover),
                    cap.precision,
                    cap.reason,
                    snap.generation,
                ))
            }
            Located::Nothing { generation, cap } => {
                Ok(result(None, cap.precision, cap.reason, generation))
            }
            Located::Fallback { reason, generation } => {
                let none = Ok(result(
                    None,
                    Precision::Approximate,
                    Some(reason.clone()),
                    generation,
                ));
                let Some(file) = self.l0_file(&rel, doc.as_ref(), point.version) else {
                    return none;
                };
                let Some(offset) = file.offset(point.position) else {
                    return none;
                };
                if let Some(decl) = file.resolve(offset) {
                    let ident = identifier_range(&file.text, offset).unwrap_or(offset..offset);
                    let hover = Hover {
                        signature: decl.signature.into(),
                        documentation: "".into(),
                        range: file.range(&ident),
                    };
                    return Ok(result(
                        Some(hover),
                        Precision::Syntax,
                        Some(reason),
                        generation,
                    ));
                }
                let (Some(l1), Some((_, type_text))) =
                    (&self.l1, identifier_at(&file.text, offset))
                else {
                    return none;
                };
                let path = WorkspacePath::new(&rel)?;
                let Ok(found) = l1.resolve_type(&path, point.position, &type_text, cancel) else {
                    return none;
                };
                let [symbol] = &*found.value else {
                    return none;
                };
                let ident = identifier_range(&file.text, offset).unwrap_or(offset..offset);
                let signature = if symbol.container.is_empty() {
                    symbol.name.to_string()
                } else {
                    format!("{}.{}", symbol.container, symbol.name)
                };
                let hover = Hover {
                    signature: signature.into(),
                    documentation: "".into(),
                    range: file.range(&ident),
                };
                Ok(result(
                    Some(hover),
                    Precision::Approximate,
                    Some(reason),
                    generation,
                ))
            }
        }
    }

    fn current_version_text(
        &self,
        file: &Resource,
        version: Version,
    ) -> Result<(String, Arc<L0File>)> {
        let rel = self.rel_path(file).ok_or(Error::Unsupported)?;
        let doc = self.buffers.get(&rel);
        if doc.as_ref().is_some_and(|d| d.version != version) {
            return Err(Error::Stale);
        }
        let parsed = self
            .l0_file(&rel, doc.as_ref(), version)
            .ok_or(Error::NotReady)?;
        Ok((rel, parsed))
    }
}

const KIND_NAMED_TYPE: u8 = 11;

/// 沿关系做传递闭包（不含起点），按发现顺序。
fn closure(start: u32, cancel: &Cancellation, next: impl Fn(u32) -> Vec<u32>) -> Result<Vec<u32>> {
    let mut seen = HashSet::from([start]);
    let mut out = Vec::new();
    let mut queue = vec![start];
    while let Some(s) = queue.pop() {
        cancel.check()?;
        for n in next(s) {
            if seen.insert(n) {
                out.push(n);
                queue.push(n);
            }
        }
    }
    Ok(out)
}

fn position(line: u32, col: u64) -> Position {
    Position {
        line: Line(u64::from(line)),
        column: Utf8Column(col),
    }
}

fn shifted(p: Position, len: u32) -> Position {
    Position {
        line: p.line,
        column: Utf8Column(p.column.0 + u64::from(len)),
    }
}

fn empty_range(p: Position) -> TextRange {
    Span::new(p, p).expect("empty range")
}

/// 逐条结果合成批级结果：批级精确度取最弱。
fn batch(
    out: Vec<RoutedTarget>,
    cap: Cap,
    version: Version,
    generation: u64,
) -> QueryResult<Arc<[RoutedTarget]>> {
    let weakest = out
        .iter()
        .max_by_key(|t| rank(t.precision))
        .map(|t| (t.precision, t.reason.clone()));
    let (precision, reason) = match weakest {
        Some((p, r)) if rank(p) > rank(cap.precision) => (p, r),
        _ => (cap.precision, cap.reason),
    };
    QueryResult {
        value: out.into(),
        precision,
        reason,
        version: Some(version),
        index_generation: generation,
    }
}

/// 把 L2 的位置翻译成当前坐标；每个文件的状态在一次查询内只算一次。
struct ResultMapper<'a> {
    router: &'a CsRouter,
    index: L2View<'a>,
    cap: Cap,
    has_stale: bool,
    files: HashMap<u32, Option<(Resource, FileState)>>,
    last: Option<(u32, Option<(Resource, FileState)>)>,
}

impl<'a> ResultMapper<'a> {
    fn new(router: &'a CsRouter, index: L2View<'a>, cap: Cap) -> Self {
        Self {
            router,
            index,
            cap,
            has_stale: router.semantic.has_stale(),
            files: HashMap::new(),
            last: None,
        }
    }

    fn file(&mut self, file: u32) -> Option<&(Resource, FileState)> {
        // 结果按文件连续排列：先看上一条的文件，命中时不查表。
        if self.last.as_ref().is_none_or(|(f, _)| *f != file) {
            let (router, index, has_stale) = (self.router, self.index, self.has_stale);
            let entry = self
                .files
                .entry(file)
                .or_insert_with(|| {
                    let path = index.file_path(file)?;
                    let resource = resource_for(path);
                    let doc = router.buffers.get(path);
                    let state = router.file_state(index, file, path, doc.as_ref(), has_stale);
                    Some((resource, state))
                })
                .clone();
            self.last = Some((file, entry));
        }
        self.last.as_ref().and_then(|(_, e)| e.as_ref())
    }

    fn map(&mut self, o: &Occurrence, role: TargetRole) -> Option<RoutedTarget> {
        let (precision_cap, reason_cap) = (self.cap.precision, self.cap.reason.clone());
        let (resource, state) = self.file(o.file)?;
        let old = position(o.line, o.col.0);
        let (start, precision, reason) = match state {
            FileState::Clean => (old, precision_cap, reason_cap),
            FileState::Mapped(map) => match map.old_to_new_clean(old) {
                Mapped::At(p) => (p, precision_cap, reason_cap),
                Mapped::Modified => (
                    map.old_to_new_nearest(old),
                    Precision::Approximate,
                    Some(ApproximationReason::ModifiedRegion),
                ),
            },
            FileState::Diverged => (
                old,
                Precision::Approximate,
                Some(ApproximationReason::ModifiedRegion),
            ),
        };
        let range = Span::new(start, shifted(start, o.len)).ok()?;
        Some(RoutedTarget {
            target: Target {
                resource: resource.clone(),
                range: range.clone(),
                selection: range,
            },
            precision,
            reason,
            role,
        })
    }
}

/// 索引里的路径 → 资源：项目相对路径；跨卷的绝对路径；生成代码的虚拟路径。
fn resource_for(path: &str) -> Resource {
    if let Ok(p) = WorkspacePath::new(path) {
        Resource::Workspace(p)
    } else if path.contains("://") {
        Resource::Virtual(path.into())
    } else {
        Resource::File(PathBuf::from(path))
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// 偏移处标识符的字节区间（不含 `@` 前缀）。
fn identifier_range(text: &str, offset: usize) -> Option<std::ops::Range<usize>> {
    let b = text.as_bytes();
    if offset >= b.len() || !is_ident_byte(b[offset]) {
        return None;
    }
    let mut start = offset;
    while start > 0 && is_ident_byte(b[start - 1]) {
        start -= 1;
    }
    let mut end = offset;
    while end < b.len() && is_ident_byte(b[end]) {
        end += 1;
    }
    Some(start..end)
}

/// 偏移处的标识符，以及引用处写出的类型文本：左侧的限定（`A.B.`、`global::`）+ 标识符 +
/// 右侧泛型实参个数（写成 `Name<,>`）。只看同一行。
fn identifier_at(text: &str, offset: usize) -> Option<(&str, String)> {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    let line = &text[line_start..line_end];
    let b = line.as_bytes();
    let range = identifier_range(line, offset - line_start)?;
    let ident = &text[line_start + range.start..line_start + range.end];
    let (start, end) = (range.start, range.end);
    let mut left = start;
    loop {
        if left > 0 && b[left - 1] == b'@' {
            left -= 1;
        }
        let sep = if left >= 2 && &b[left - 2..left] == b"::" {
            2
        } else if left >= 1 && b[left - 1] == b'.' {
            1
        } else {
            break;
        };
        let mut s = left - sep;
        while s > 0 && is_ident_byte(b[s - 1]) {
            s -= 1;
        }
        if s == left - sep {
            break;
        }
        left = s;
        if sep == 2 {
            break;
        }
    }
    let mut type_text = line[left..end].replace('@', "");
    let mut i = end;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    if i < b.len() && b[i] == b'<' {
        let (mut depth, mut commas, mut closed) = (0i32, 0usize, false);
        for &c in &b[i..] {
            match c {
                b'<' | b'(' | b'[' => depth += 1,
                b'>' | b')' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        closed = c == b'>';
                        break;
                    }
                }
                b',' if depth == 1 => commas += 1,
                b';' | b'{' | b'=' => break,
                _ => {}
            }
        }
        if closed {
            type_text.push('<');
            type_text.push_str(&",".repeat(commas));
            type_text.push('>');
        }
    }
    Some((ident.strip_prefix('@').unwrap_or(ident), type_text))
}

impl CsQueries for CsRouter {
    fn navigate(
        &self,
        kind: NavigationKind,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[Target]>>> {
        let detailed = self.navigate_detailed(kind, point, cancel)?;
        Ok(QueryResult {
            value: detailed.value.iter().map(|t| t.target.clone()).collect(),
            precision: detailed.precision,
            reason: detailed.reason,
            version: detailed.version,
            index_generation: detailed.index_generation,
        })
    }

    fn hover(
        &self,
        point: &QueryPoint,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Option<Hover>>> {
        self.hover_detailed(point, cancel)
    }

    /// 文件内符号：当前文本上的语法级大纲（#26 的提取），精确度为“语法”。
    fn file_symbols(
        &self,
        file: &Resource,
        version: Version,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[Symbol]>>> {
        cancel.check()?;
        let (_, parsed) = self.current_version_text(file, version)?;
        let symbols = rove_syntax::symbols::extract("csharp", &parsed.text, cancel)?;
        Ok(QueryResult {
            value: symbols.into(),
            precision: Precision::Syntax,
            reason: Some(ApproximationReason::Other("l0-syntax".into())),
            version: Some(version),
            index_generation: self.semantic.snapshot().map_or(0, |s| s.generation),
        })
    }

    /// 工作区符号：L1 声明索引的模糊搜索（精确度“语法”）。没有 L1 时 `NotReady`。
    fn workspace_symbols(
        &self,
        query: &str,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[WorkspaceSymbol]>>> {
        let l1 = self.l1.as_ref().ok_or(Error::NotReady)?;
        let found = l1.workspace_symbols(query, WORKSPACE_SYMBOL_LIMIT, cancel)?;
        Ok(QueryResult {
            value: found
                .value
                .iter()
                .flat_map(|s| s.to_workspace_symbols())
                .collect(),
            precision: found.precision,
            reason: found
                .reason
                .or(Some(ApproximationReason::Other(L1_REASON.into()))),
            version: None,
            index_generation: found.index_generation,
        })
    }

    /// 语义高亮属于 R58，路由不提供。
    fn semantic_tokens(
        &self,
        _: &Resource,
        _: Version,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[SemanticToken]>>> {
        cancel.check()?;
        Err(Error::Unsupported)
    }

    /// 不生效的预处理区域（L2 的行区间，整行），映射到当前坐标；文件改过时标为近似。
    fn inactive_regions(
        &self,
        file: &Resource,
        version: Version,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[TextRange]>>> {
        cancel.check()?;
        let rel = self.rel_path(file).ok_or(Error::Unsupported)?;
        let doc = self.buffers.get(&rel);
        if doc.as_ref().is_some_and(|d| d.version != version) {
            return Err(Error::Stale);
        }
        let snap = self.semantic.snapshot().ok_or(Error::NotReady)?;
        let index = snap.holder.view();
        let cap = Cap::of(&snap);
        let result = |value: Vec<TextRange>, precision, reason| QueryResult {
            value: value.into(),
            precision,
            reason,
            version: Some(version),
            index_generation: snap.generation,
        };
        let Some(id) = index.file_id(&rel) else {
            return Ok(result(
                Vec::new(),
                Precision::Approximate,
                Some(ApproximationReason::Other("file-not-indexed".into())),
            ));
        };
        let has_stale = doc.is_none() && self.semantic.has_stale();
        let state = self.file_state(index, id, &rel, doc.as_ref(), has_stale);
        let lines = index.inactive_lines(id);
        let line_start = |l: u32| position(l, 0);
        let (ranges, precision, reason) = match &state {
            FileState::Clean => (
                lines
                    .iter()
                    .filter_map(|r| Span::new(line_start(r.start), line_start(r.end)).ok())
                    .collect(),
                cap.precision,
                cap.reason,
            ),
            FileState::Mapped(map) => (
                lines
                    .iter()
                    .filter_map(|r| {
                        Span::new(
                            map.old_to_new_nearest(line_start(r.start)),
                            map.old_to_new_nearest(line_start(r.end)),
                        )
                        .ok()
                    })
                    .collect(),
                Precision::Approximate,
                Some(ApproximationReason::ModifiedRegion),
            ),
            FileState::Diverged => (
                lines
                    .iter()
                    .filter_map(|r| Span::new(line_start(r.start), line_start(r.end)).ok())
                    .collect(),
                Precision::Approximate,
                Some(ApproximationReason::ModifiedRegion),
            ),
        };
        Ok(result(ranges, precision, reason))
    }
}
