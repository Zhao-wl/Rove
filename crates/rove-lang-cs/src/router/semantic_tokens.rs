//! 按文件产出语义 token（#59 的 1、5 两节），实现 #1 契约的 `CsQueries::semantic_tokens`。
//!
//! 一次查询做三件事：
//!
//! 1. 取出这个文件在 L2 索引里的**全部出现位置**（已经是 UTF-8 行列，#32 的 v2 格式）；
//! 2. 每个位置绑定到的符号给出种类（Roslyn `SymbolKind`），再由本文件的语法树（类型种类与
//!    修饰符）与 L1 声明索引（别处声明的类型是 class 还是 struct）补全，见
//!    [`crate::semantic::syntax`] 的模块文档；
//! 3. 文件被改过时按 #38 已有的「这一行改过没有」判定（[`EditMap`] 的 `old_to_new_clean`）
//!    把位置搬到当前坐标；**改过的行直接丢弃**，那些地方退回纯语法高亮。
//!
//! 精确度沿用导航查询的口径：索引不可信或文件被改过 → `Approximate`；界面据此决定还画不画
//! （`ModifiedRegion` 是逐行降级，其余原因一律不产出语义 token）。

use super::{
    buffers, map_position, position, Cap, CsRouter, FileState, L0File, L2View, Occurrence,
};
use crate::query::{ApproximationReason, Precision, QueryResult, SemanticToken};
use crate::semantic::syntax::{DeclFacts, FileFacts};
use crate::semantic::{CsSymbolKind, Modifiers, SymbolFacts};
use rove_base::{Cancellation, Error, Resource, Result, Version};
use std::collections::HashMap;
use std::sync::Arc;

/// 判定 `defaultLibrary` 的程序集（符号键的前半段）。只认 .NET 自身的程序集：
/// Unity 与第三方 DLL 不算（VS Code 也只把 BCL 标成 defaultLibrary）。
fn is_default_library(assembly: &str) -> bool {
    matches!(
        assembly,
        "mscorlib" | "netstandard" | "System" | "System.Private.CoreLib" | "System.Runtime"
    ) || assembly.starts_with("System.")
}

/// 符号键 `程序集|文档注释 ID` 的两半。
fn split_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('|')
}

/// 文档注释 ID（`T:NS.Type`、`M:NS.Type.M(…)`）里的完全限定名，去掉前缀与参数表。
fn qualified_name(doc_id: &str) -> Option<&str> {
    let rest = doc_id.split_once(':')?.1;
    Some(rest.split(['(', '`']).next().unwrap_or(rest))
}

impl CsRouter {
    /// L1 里这个完全限定类型名的声明种类。
    fn l1_type_kind(&self, qualified: &str) -> Option<CsSymbolKind> {
        use crate::l1::DeclKind;
        let kind = self.l1.as_ref()?.find_type(qualified)?.kind;
        Some(match kind {
            DeclKind::Class => CsSymbolKind::Class,
            DeclKind::Struct | DeclKind::RecordStruct => CsSymbolKind::Struct,
            DeclKind::Interface => CsSymbolKind::Interface,
            DeclKind::Enum => CsSymbolKind::Enum,
            DeclKind::Record => CsSymbolKind::Record,
            DeclKind::Delegate => CsSymbolKind::Delegate,
            _ => return None,
        })
    }

    /// 一个具名类型符号的种类：先看本文件的声明细节，再看 L1，都没有就是笼统的 `type`。
    fn type_kind_of(
        &self,
        index: L2View<'_>,
        symbol: u32,
        details: &HashMap<u32, DeclFacts>,
    ) -> CsSymbolKind {
        if let Some(d) = details.get(&symbol) {
            if d.kind.is_type() {
                return d.kind;
            }
        }
        index
            .global_key(symbol)
            .and_then(split_key)
            .and_then(|(_, doc)| qualified_name(doc))
            .and_then(|name| self.l1_type_kind(name))
            .unwrap_or(CsSymbolKind::Type)
    }

    /// 这个文件的语义 token（已按行列排序）。
    pub(super) fn semantic_tokens_of(
        &self,
        index: L2View<'_>,
        file: u32,
        state: &FileState,
        facts: &FileFacts,
        cancel: &Cancellation,
    ) -> Result<Vec<SemanticToken>> {
        let occurrences = index.file_occurrences(file);
        cancel.check()?;
        // 第一遍：声明处把语法树里的细节记到符号上，本文件里对同一符号的引用共用这份细节。
        let mut details: HashMap<u32, DeclFacts> = HashMap::new();
        for o in &occurrences {
            if !crate::index::Role(o.role).is_def() {
                continue;
            }
            let Some(mapped) = map_position(state, position(o.line, o.col.0), true) else {
                continue;
            };
            let (Ok(line), Ok(column)) =
                (u32::try_from(mapped.line.0), u32::try_from(mapped.column.0))
            else {
                continue;
            };
            if let Some(d) = facts.at(line, column) {
                details.entry(o.symbol).or_insert(d);
            }
        }
        cancel.check()?;
        let mut out = Vec::with_capacity(occurrences.len());
        for o in &occurrences {
            let Some(token) = self.token_for(index, o, &details) else {
                continue;
            };
            // 被改过的行丢弃（退回纯语法高亮）；其余按编辑记录搬到当前坐标。
            let at = position(o.line, o.col.0);
            let Some(mapped) = map_position(state, at, true) else {
                continue;
            };
            out.push(SemanticToken {
                line: mapped.line,
                column: mapped.column,
                length_bytes: o.len as u64,
                ..token
            });
        }
        out.sort_by_key(|t| (t.line.0, t.column.0));
        Ok(out)
    }

    fn token_for(
        &self,
        index: L2View<'_>,
        o: &Occurrence,
        details: &HashMap<u32, DeclFacts>,
    ) -> Option<SemanticToken> {
        let mut kind = CsSymbolKind::from_roslyn(index.kind(o.symbol)?)?;
        let mut modifiers = Modifiers::default();
        let key = index.global_key(o.symbol);
        let parts = key.and_then(split_key);
        if let Some((assembly, _)) = parts {
            modifiers.default_library = is_default_library(assembly);
        }
        match details.get(&o.symbol) {
            // 本文件里声明的符号：类型种类与修饰符都精确。
            Some(d) => {
                kind = d.kind;
                modifiers = Modifiers {
                    default_library: modifiers.default_library,
                    declaration: false,
                    ..d.modifiers
                };
            }
            None => {
                // 构造函数的名字就是类型名：按所在类型的种类着色。
                let is_ctor = parts.is_some_and(|(_, doc)| doc.contains("#ctor"));
                if kind == CsSymbolKind::Type || (kind == CsSymbolKind::Method && is_ctor) {
                    let target = if is_ctor && kind == CsSymbolKind::Method {
                        index.container(o.symbol)?
                    } else {
                        o.symbol
                    };
                    kind = self.type_kind_of(index, target, details);
                } else if kind == CsSymbolKind::Field {
                    // 枚举成员：容器是枚举类型。
                    if let Some(container) = index.container(o.symbol) {
                        if self.type_kind_of(index, container, details) == CsSymbolKind::Enum {
                            kind = CsSymbolKind::EnumMember;
                        }
                    }
                }
            }
        }
        modifiers.declaration = crate::index::Role(o.role).is_def();
        let (token_type, names) = SymbolFacts { kind, modifiers }.classify();
        Some(SemanticToken {
            line: rove_base::Line(o.line as u64),
            column: o.col,
            length_bytes: o.len as u64,
            token_type: token_type.into(),
            modifiers: names.into_iter().map(Arc::<str>::from).collect(),
        })
    }

    /// 本文件的声明细节（按路径与版本缓存一份，和 L0 解析共用同一次 tree-sitter 解析）。
    fn file_facts(
        &self,
        rel: &str,
        doc: Option<&super::BufferState>,
        version: Version,
    ) -> Option<Arc<FileFacts>> {
        if let Some((path, v, facts)) = &*super::lock(&self.facts_cache) {
            if path == rel && *v == version {
                return Some(facts.clone());
            }
        }
        let file: Arc<L0File> = self.l0_file_for_facts(rel, doc, version)?;
        let facts = Arc::new(FileFacts::from_tree(file.tree(), &file.text));
        *super::lock(&self.facts_cache) = Some((rel.to_owned(), version, facts.clone()));
        Some(facts)
    }

    /// The current index generation used by per-document semantic queries.
    pub fn semantic_generation(&self) -> Option<u64> {
        self.semantic.snapshot().map(|snapshot| snapshot.generation)
    }

    /// 语义 token 的入口（契约实现见 `CsQueries::semantic_tokens`）。
    pub(super) fn semantic_tokens_impl(
        &self,
        resource: &Resource,
        version: Version,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[SemanticToken]>>> {
        cancel.check()?;
        let rel = self.rel_path(resource).ok_or(Error::Unsupported)?;
        if !rel.to_ascii_lowercase().ends_with(".cs") {
            // D2：只有 C# 有语义高亮。
            return Err(Error::Unsupported);
        }
        let doc = self.buffers.get(&rel);
        if doc.as_ref().is_some_and(|d| d.version != version) {
            return Err(Error::Stale);
        }
        let snap = self.semantic.snapshot().ok_or(Error::NotReady)?;
        let index = snap.holder.view();
        let cap = Cap::of(&snap);
        let result = |value: Vec<SemanticToken>, precision, reason| QueryResult {
            value: value.into(),
            precision,
            reason,
            version: Some(version),
            index_generation: snap.generation,
        };
        if cap.precision != Precision::Exact {
            return Ok(result(Vec::new(), cap.precision, cap.reason));
        }
        let Some(id) = index.file_id(&rel) else {
            return Ok(result(
                Vec::new(),
                Precision::Approximate,
                Some(ApproximationReason::Other("file-not-indexed".into())),
            ));
        };
        let has_stale = doc.is_none() && self.semantic.has_stale();
        let state = self.file_state(index, id, &rel, doc.as_ref(), has_stale);
        if matches!(state, FileState::Diverged) {
            // 位置无法映射：整份文件退回纯语法高亮。
            return Ok(result(
                Vec::new(),
                Precision::Approximate,
                Some(ApproximationReason::ModifiedRegion),
            ));
        }
        let Some(facts) = self.file_facts(&rel, doc.as_ref(), version) else {
            return Ok(result(
                Vec::new(),
                Precision::Approximate,
                Some(ApproximationReason::Other("source-unavailable".into())),
            ));
        };
        let tokens = self.semantic_tokens_of(index, id, &state, &facts, cancel)?;
        let (precision, reason) = match &state {
            FileState::Clean => (cap.precision, cap.reason),
            _ => (
                Precision::Approximate,
                Some(ApproximationReason::ModifiedRegion),
            ),
        };
        Ok(result(tokens, precision, reason))
    }
}

/// 读文件文本的辅助（与 L0 回退同一条路径，复用那份解析缓存）。
impl CsRouter {
    fn l0_file_for_facts(
        &self,
        rel: &str,
        doc: Option<&super::BufferState>,
        version: Version,
    ) -> Option<Arc<L0File>> {
        if let Some((path, v, file)) = &*super::lock(&self.l0_cache) {
            if path == rel && *v == version {
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
        *super::lock(&self.l0_cache) = Some((rel.to_owned(), version, file.clone()));
        Some(file)
    }
}
