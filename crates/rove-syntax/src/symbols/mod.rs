//! 文档符号（#26）：在 tree-sitter 之上为 C#、Markdown、JSON/JSONC、YAML、XML 产出 #1 契约的
//! 符号树（[`Symbol`]），再建立一份只读索引 [`DocumentSymbols`]，供大纲、面包屑、转到文件中的符号、
//! 快速打开的 `@`（#24）、粘性滚动（R54）与上下文篮子（R65）共用。核心层，不依赖 gpui。
//!
//! | 入口 | 用途 |
//! |---|---|
//! | [`availability`] | 语言与大小级别是否提供符号（UI 线程可调用，O(1)）；超大文件不提供 |
//! | [`SymbolService::analyze`] | 后台：读取文本快照 → 解析 → 提取 → 与上一版比较，返回 [`SymbolUpdate`] |
//! | [`extract`] | 同步提取（后台线程或测试） |
//! | [`DocumentSymbols`] | 先序编号的符号（[`SymbolId`]）、父子关系、容器名、稳定键、按位置查符号路径 |
//!
//! 线程：`analyze` 返回的 `Task` 只在后台执行；结果是不可变的 `Arc`，UI 线程读取都是 O(log n)
//! 或 O(深度 · log 兄弟数)，不遍历整棵树。
//!
//! 种类：只用契约里的 [`SymbolKind`]。契约没有的 VS Code 种类用 `SymbolKind::Other(名称)`，名称取
//! VS Code `SymbolKind` 的小写驼峰形式（见 [`kinds`]），与 `outline.show*` / `breadcrumbs.show*`
//! 设置项的后缀一一对应。
//!
//! 增量：符号提取每次对新快照完整解析（只读阅读器阶段没有编辑，解析在后台完成）；
//! [`SymbolService::analyze`] 再把新树与上一版逐子树比较：内容相同的子树直接复用上一版的
//! `Arc<[Symbol]>`（不重建、不复制），并给出“哪些节点变了”（[`SymbolUpdate::changed`]）。
//! 消费方在新结果到达前继续显示旧结果，不闪烁。

mod csharp;
mod data;
mod markup;
mod source;
#[cfg(test)]
mod tests;

use crate::{Symbol, SymbolKind};
use rove_base::{
    ByteOffset, Cancellation, Error, Position, Result, Span, Task, TextRange, Version,
};
use rove_buffer::{SizeClass, TextSource};
use std::{collections::HashMap, sync::Arc};

/// `SymbolKind::Other` 使用的种类名（VS Code `SymbolKind` 的名称）。
pub mod kinds {
    pub const ENUM_MEMBER: &str = "enumMember";
    pub const OPERATOR: &str = "operator";
    pub const STRING: &str = "string";
    pub const NUMBER: &str = "number";
    pub const BOOLEAN: &str = "boolean";
    pub const NULL: &str = "null";
    /// YAML 多文档中的一个文档（`---`）；名称是从 1 开始的序号。
    pub const DOCUMENT: &str = "document";
}

/// 数据文件（JSON/YAML/XML）最多产出的符号数，超出的部分不进入大纲（[`DocumentSymbols::truncated`]）。
/// 与 VS Code `json.maxItemsComputed` 的默认值同一量级的保护，避免百万级键撑爆界面树。
pub const DATA_SYMBOL_LIMIT: usize = 50_000;

/// 提供符号的语言 ID（#12 语言配置的 ID）。
pub fn supports(language: &str) -> bool {
    matches!(
        language,
        "csharp" | "json" | "jsonc" | "markdown" | "xml" | "yaml"
    )
}

/// 当前文档能否提供符号，以及不能时的原因（UX_SPEC §3.1、§6.3）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// 语言没有符号提供方：「活动编辑器无法提供大纲信息。」
    Unsupported,
    /// 超大文件（`rove.largeFile.threshold`）不提供符号。
    TooLarge,
}

pub fn availability(language: &str, size: SizeClass) -> Availability {
    if !supports(language) {
        Availability::Unsupported
    } else if size == SizeClass::Huge {
        Availability::TooLarge
    } else {
        Availability::Available
    }
}

/// 同步提取一份文本的符号树（只在后台线程或测试中调用）。不支持的语言返回 `Unsupported`；
/// 有语法错误的文件返回能恢复出来的部分。
pub fn extract(language: &str, text: &str, cancel: &Cancellation) -> Result<Vec<Symbol>> {
    extract_with(language, text, cancel).map(|(symbols, _)| symbols)
}

fn extract_with(language: &str, text: &str, cancel: &Cancellation) -> Result<(Vec<Symbol>, bool)> {
    let limit = match language {
        "csharp" | "markdown" => usize::MAX,
        _ => DATA_SYMBOL_LIMIT,
    };
    let src = source::Source::new(text, cancel, limit);
    let symbols = match language {
        "csharp" => csharp::extract(&src)?,
        "json" | "jsonc" => data::json(&src)?,
        "yaml" => data::yaml(&src)?,
        "xml" => markup::xml(&src)?,
        "markdown" => markup::markdown(&src)?,
        _ => return Err(Error::Unsupported),
    };
    cancel.check()?;
    Ok((symbols, src.truncated()))
}

/// 先序编号：根的第一个符号是 0，父节点总是排在子节点之前。只在同一个 [`DocumentSymbols`] 内有效。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(pub u32);

/// 索引里的一个符号。
#[derive(Clone, Debug)]
pub struct SymbolNode {
    pub symbol: Symbol,
    pub parent: Option<SymbolId>,
    pub depth: u32,
    /// VS Code 的 `containerName`：父符号的名称，根为 `None`。
    pub container: Option<Arc<str>>,
    /// 跨版本稳定的键（父键 + 种类 + 名称 + 同名序号），用于保留展开状态与比较版本。
    pub key: Arc<str>,
    children: std::ops::Range<u32>,
}

/// 一个文档版本的全部符号：契约树（[`Self::roots`]）+ 先序扁平索引。不可变，`Send + Sync`。
#[derive(Debug)]
pub struct DocumentSymbols {
    version: Version,
    language: Arc<str>,
    roots: Arc<[Symbol]>,
    nodes: Vec<SymbolNode>,
    root_ids: Vec<SymbolId>,
    child_ids: Vec<SymbolId>,
    by_key: HashMap<Arc<str>, SymbolId>,
    truncated: bool,
}

impl DocumentSymbols {
    pub fn new(version: Version, language: Arc<str>, roots: Arc<[Symbol]>) -> Self {
        let mut this = Self {
            version,
            language,
            roots: roots.clone(),
            nodes: Vec::new(),
            root_ids: Vec::new(),
            child_ids: Vec::new(),
            by_key: HashMap::new(),
            truncated: false,
        };
        this.root_ids = this.index(&roots, None, "");
        this
    }

    /// 返回这一层的 ID；子节点的 ID 写入 `child_ids` 的连续区间。
    fn index(
        &mut self,
        list: &[Symbol],
        parent: Option<SymbolId>,
        parent_key: &str,
    ) -> Vec<SymbolId> {
        let mut ids = Vec::with_capacity(list.len());
        let mut seen: HashMap<(&str, &str), u32> = HashMap::new();
        for symbol in list {
            let id = SymbolId(self.nodes.len() as u32);
            let tag = kind_name(&symbol.kind);
            let n = seen.entry((symbol.name.as_ref(), tag)).or_default();
            let key: Arc<str> = if *n == 0 {
                format!("{parent_key}/{tag}:{}", symbol.name).into()
            } else {
                format!("{parent_key}/{tag}:{}#{n}", symbol.name).into()
            };
            *n += 1;
            let depth = parent.map_or(0, |p| self.nodes[p.0 as usize].depth + 1);
            let container = parent.map(|p| self.nodes[p.0 as usize].symbol.name.clone());
            self.nodes.push(SymbolNode {
                symbol: symbol.clone(),
                parent,
                depth,
                container,
                key: key.clone(),
                children: 0..0,
            });
            self.by_key.insert(key.clone(), id);
            let children = self.index(&symbol.children, Some(id), &key);
            let start = self.child_ids.len() as u32;
            self.child_ids.extend(children);
            self.nodes[id.0 as usize].children = start..self.child_ids.len() as u32;
            ids.push(id);
        }
        ids
    }

    pub fn version(&self) -> Version {
        self.version
    }
    pub fn language(&self) -> &Arc<str> {
        &self.language
    }
    /// 契约形式的符号树（与 `SyntaxSnapshot::symbols` 同一类型）。
    pub fn roots(&self) -> &Arc<[Symbol]> {
        &self.roots
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    /// 数据文件达到 [`DATA_SYMBOL_LIMIT`]，后面的符号没有提取。
    pub fn truncated(&self) -> bool {
        self.truncated
    }
    pub fn node(&self, id: SymbolId) -> Option<&SymbolNode> {
        self.nodes.get(id.0 as usize)
    }
    /// `None` 取根这一层。按文档位置排序。
    pub fn children(&self, parent: Option<SymbolId>) -> &[SymbolId] {
        match parent {
            None => &self.root_ids,
            Some(p) => match self.nodes.get(p.0 as usize) {
                Some(n) => &self.child_ids[n.children.start as usize..n.children.end as usize],
                None => &[],
            },
        }
    }
    /// 先序遍历（文档顺序）。
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (SymbolId, &SymbolNode)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (SymbolId(i as u32), n))
    }
    pub fn find_key(&self, key: &str) -> Option<SymbolId> {
        self.by_key.get(key).copied()
    }
    /// 从根到 `id` 的路径（含 `id`）。
    pub fn ancestors(&self, id: SymbolId) -> Vec<SymbolId> {
        let mut path = Vec::new();
        let mut at = Some(id);
        while let Some(i) = at {
            path.push(i);
            at = self.node(i).and_then(|n| n.parent);
        }
        path.reverse();
        path
    }

    /// 包含 `position` 的符号路径（由外到内）：每层在按位置排序的兄弟中二分查找，
    /// O(深度 · log 兄弟数)，不遍历整棵树（面包屑与大纲“跟随光标”的预算 < 1 ms）。
    /// 区间按闭区间判定：光标停在 `}` 之后仍算在该成员内。
    pub fn path_at(&self, position: Position) -> Vec<SymbolId> {
        self.descend(Vec::new(), position)
    }

    /// 与 [`Self::path_at`] 结果相同，但先复用上一次的路径 `hint`：保留其中仍包含 `position` 的前缀，
    /// 只从那里往下查找。光标在同一个成员内移动时只做常数次比较。
    pub fn path_from(&self, hint: &[SymbolId], position: Position) -> Vec<SymbolId> {
        let keep = hint
            .iter()
            .take_while(|id| {
                self.node(**id)
                    .is_some_and(|n| contains(&n.symbol.range, position))
            })
            .count();
        self.descend(hint[..keep].to_vec(), position)
    }

    fn descend(&self, mut path: Vec<SymbolId>, position: Position) -> Vec<SymbolId> {
        loop {
            let siblings = self.children(path.last().copied());
            let after = siblings
                .partition_point(|id| self.nodes[id.0 as usize].symbol.range.start() <= position);
            // 兄弟区间可能相接：取起点不晚于光标的最后一个，再往前找包含光标的（嵌套良好时只看一个）。
            let found = siblings[..after]
                .iter()
                .rev()
                .take(2)
                .find(|id| contains(&self.nodes[id.0 as usize].symbol.range, position));
            match found {
                Some(id) => path.push(*id),
                None => return path,
            }
        }
    }
}

fn contains(range: &TextRange, position: Position) -> bool {
    range.start() <= position && position <= range.end()
}

/// 种类的稳定名称：契约枚举的小写驼峰形式，`Other` 原样。与 `outline.show*` 的后缀对应。
pub fn kind_name(kind: &SymbolKind) -> &str {
    match kind {
        SymbolKind::Namespace => "namespace",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Interface => "interface",
        SymbolKind::Enum => "enum",
        SymbolKind::Method => "method",
        SymbolKind::Function => "function",
        SymbolKind::Property => "property",
        SymbolKind::Field => "field",
        SymbolKind::Variable => "variable",
        SymbolKind::Parameter => "parameter",
        SymbolKind::TypeParameter => "typeParameter",
        SymbolKind::Event => "event",
        SymbolKind::Constructor => "constructor",
        SymbolKind::Constant => "constant",
        SymbolKind::Module => "module",
        SymbolKind::Object => "object",
        SymbolKind::Array => "array",
        SymbolKind::Key => "key",
        SymbolKind::Other(name) => name,
    }
}

// ------------------------------------------------------------------ 后台服务与增量

/// 一次分析的结果。
#[derive(Clone, Debug)]
pub struct SymbolUpdate {
    pub symbols: Arc<DocumentSymbols>,
    /// 比较所用的上一版；`None` 表示没有基线（首次分析）。
    pub previous: Option<Version>,
    /// 新版本中内容变化或新增的节点（自身的名称、种类、区间或子节点列表变化）。
    /// `None` 表示没有基线、全部视为新节点；`Some(空)` 表示与上一版完全相同。
    pub changed: Option<Arc<[SymbolId]>>,
    /// 上一版中不再存在的节点键。
    pub removed: Arc<[Arc<str>]>,
}

/// 后台符号服务。无状态，`Send + Sync`；每个文档的上一版结果由调用方保存并传回。
#[derive(Clone, Copy, Debug, Default)]
pub struct SymbolService;

impl SymbolService {
    /// 读取文本快照并提取符号。返回的 `Task` 只能在后台执行器上运行；按协作方式检查取消。
    /// 不支持的语言与超大文件返回 `Unsupported`（原因用 [`availability`] 区分）；
    /// `previous` 版本比 `text` 新时返回 `Stale`。
    pub fn analyze(
        &self,
        text: Arc<dyn TextSource>,
        language: Arc<str>,
        size: SizeClass,
        previous: Option<Arc<DocumentSymbols>>,
        cancel: Cancellation,
    ) -> Task<SymbolUpdate> {
        Box::pin(async move {
            cancel.check()?;
            if availability(&language, size) != Availability::Available {
                return Err(Error::Unsupported);
            }
            let version = text.version();
            let previous = previous.filter(|p| p.language == language);
            if let Some(p) = &previous {
                if p.version > version {
                    return Err(Error::Stale);
                }
            }
            let mut source = String::with_capacity(text.byte_len() as usize);
            text.visit_chunks(
                Span::new(ByteOffset(0), ByteOffset(text.byte_len()))?,
                &mut |s| {
                    if cancel.check().is_ok() {
                        source.push_str(s);
                    }
                },
            )?;
            cancel.check()?;
            let (symbols, truncated) = extract_with(&language, &source, &cancel)?;
            Ok(update(
                version,
                language,
                symbols,
                truncated,
                previous.as_deref(),
            ))
        })
    }
}

/// 由新提取的符号与上一版构建结果：复用相同子树并列出变化的节点。
pub fn update(
    version: Version,
    language: Arc<str>,
    symbols: Vec<Symbol>,
    truncated: bool,
    previous: Option<&DocumentSymbols>,
) -> SymbolUpdate {
    let roots: Arc<[Symbol]> = match previous {
        Some(old) => share(symbols, &old.roots),
        None => symbols.into(),
    };
    let mut next = DocumentSymbols::new(version, language, roots);
    next.truncated = truncated;
    let (changed, removed) = match previous {
        None => (None, Arc::from([])),
        Some(old) => {
            let changed: Vec<SymbolId> = next
                .iter()
                .filter(|(id, node)| match old.find_key(&node.key) {
                    None => true,
                    Some(o) => !same_node(&next, *id, old, o),
                })
                .map(|(id, _)| id)
                .collect();
            let removed: Vec<Arc<str>> = old
                .iter()
                .filter(|(_, n)| next.find_key(&n.key).is_none())
                .map(|(_, n)| n.key.clone())
                .collect();
            (Some(changed.into()), removed.into())
        }
    };
    SymbolUpdate {
        previous: previous.map(|p| p.version),
        symbols: Arc::new(next),
        changed,
        removed,
    }
}

fn same_node(a: &DocumentSymbols, ai: SymbolId, b: &DocumentSymbols, bi: SymbolId) -> bool {
    let (x, y) = (&a.nodes[ai.0 as usize], &b.nodes[bi.0 as usize]);
    same_fields(&x.symbol, &y.symbol)
        && a.children(Some(ai))
            .iter()
            .map(|c| &a.nodes[c.0 as usize].key)
            .eq(b
                .children(Some(bi))
                .iter()
                .map(|c| &b.nodes[c.0 as usize].key))
}

fn same_fields(a: &Symbol, b: &Symbol) -> bool {
    a.name == b.name && a.kind == b.kind && a.range == b.range && a.selection == b.selection
}

fn same_tree(a: &[Symbol], b: &[Symbol]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| same_fields(x, y) && same_tree(&x.children, &y.children))
}

/// 新列表与旧列表内容相同时直接返回旧的 `Arc`；否则逐个符号对齐（同名同种类）再递归复用子树。
fn share(new: Vec<Symbol>, old: &Arc<[Symbol]>) -> Arc<[Symbol]> {
    if same_tree(&new, old) {
        return old.clone();
    }
    let key = |s: &Symbol| (s.name.clone(), kind_name(&s.kind).to_owned());
    let mut by_name: HashMap<(Arc<str>, String), Vec<&Symbol>> = HashMap::new();
    for s in old.iter().rev() {
        by_name.entry(key(s)).or_default().push(s);
    }
    new.into_iter()
        .map(|mut s| {
            if let Some(o) = by_name.get_mut(&key(&s)).and_then(|v| v.pop()) {
                let children: Vec<Symbol> = s.children.iter().cloned().collect();
                s.children = share(children, &o.children);
            }
            s
        })
        .collect()
}
