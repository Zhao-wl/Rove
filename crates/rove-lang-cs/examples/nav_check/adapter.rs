//! 适配层：用例 → 查询调用 → 结果归一化。
//!
//! **索引格式或查询接口变化时只改这个文件。** 用例文件（`cases.rs`）、定位（`locate.rs`）
//! 与统计（`report.rs`）只认识这里定义的 [`NavSource`] / [`DefAnswer`] / [`RefsAnswer`]。
//!
//! 现有两个查询来源：
//! - [`L2Index`]：完整语义索引（`CsIndex::goto_definition` / `find_references`）；
//! - [`L1Decls`]：L1 快速声明索引（#37）。#37 合并前返回“不支持”，合并后在这里接入。
//!
//! 迁移路径（#38 合并后）：把 [`L2Index`] 换成调用 #1 的 `query::CsQueries::navigate`
//! （`NavigationKind::Definition` / `References`），把 `QueryResult::precision` 填进
//! [`DefAnswer::precision`] / [`RefsAnswer::precision`]；用例里写了 `expect.precision`
//! 的会同时断言精确度标记。

use rove_lang_cs::index::Utf8Column;
use rove_lang_cs::CsIndex;
use std::path::Path;

/// 查询点：项目相对路径 + 0 起算的行号 + 0 起算的 UTF-8 字节列。
#[derive(Clone, Debug)]
pub struct QueryPos<'a> {
    pub file: &'a str,
    pub line0: u32,
    pub utf8_col0: u32,
}

/// 归一化后的源码位置：项目相对路径 + 0 起算的行号（判定只比到行）。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SrcLine {
    pub file: String,
    pub line0: u32,
}

impl std::fmt::Display for SrcLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file, self.line0 + 1)
    }
}

#[derive(Clone, Debug, Default)]
pub struct DefAnswer {
    /// 源码中的定义位置。
    pub defs: Vec<SrcLine>,
    /// 跳到外部符号元数据视图时，该符号的文档注释 ID。
    pub external: Option<String>,
    /// 诊断信息：查询点绑定到的符号（便于判断“绑定对了但没有落点”）。
    pub symbol: Option<String>,
    /// 精确度标记（exact / approximate / syntax）；来源不报告时为 None。
    pub precision: Option<&'static str>,
}

#[derive(Clone, Debug, Default)]
pub struct RefsAnswer {
    /// 引用位置，不含定义处。
    pub refs: Vec<SrcLine>,
    pub precision: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub enum SourceError {
    /// 该来源不提供这种查询（例如 L1 不做查找引用）。
    Unsupported(String),
}

pub trait NavSource {
    fn name(&self) -> &'static str;
    /// 写进报告的来源说明（索引格式、索引进程版本等）。
    fn describe(&self) -> String {
        self.name().to_owned()
    }
    fn definition(&self, at: &QueryPos<'_>) -> Result<DefAnswer, SourceError>;
    fn references(&self, at: &QueryPos<'_>) -> Result<RefsAnswer, SourceError>;
    /// 索引写出时刻，用于提示“源码比索引新”。没有就返回 None。
    fn built_at(&self) -> Option<std::time::SystemTime> {
        None
    }
}

/// 完整语义索引（`tools/rove-indexer-cs` 生成，格式见 `docs/CS_INDEX_FORMAT.md`）。
pub struct L2Index {
    idx: CsIndex,
    built_at: Option<std::time::SystemTime>,
}

impl L2Index {
    pub fn open(path: &Path) -> Result<Self, String> {
        let idx =
            CsIndex::open(path).map_err(|e| format!("打开索引 {} 失败：{e}", path.display()))?;
        let built_at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        Ok(L2Index { idx, built_at })
    }

    fn symbol_desc(&self, sym: u32) -> Option<String> {
        use rove_lang_cs::index::SymbolKey;
        let rec = self.idx.symbols().get(sym as usize)?;
        let display = self.idx.str(rec.display).unwrap_or("?");
        let key = match self.idx.symbol_key(sym).ok()? {
            SymbolKey::Global(k) => k.to_owned(),
            SymbolKey::Local { .. } => "<局部符号>".to_owned(),
        };
        Some(format!("{display} [{key}]"))
    }
}

/// 全局符号键 `程序集|文档注释 ID` 中的文档注释 ID。
fn doc_comment_id(key: &str) -> &str {
    key.split_once('|').map_or(key, |(_, id)| id)
}

impl NavSource for L2Index {
    fn name(&self) -> &'static str {
        "l2"
    }

    fn built_at(&self) -> Option<std::time::SystemTime> {
        self.built_at
    }

    fn describe(&self) -> String {
        let h = self.idx.header();
        let [a, b, c, d] = h.indexer_version;
        format!(
            "完整语义索引（格式 v{}，索引进程 {a}.{b}.{c}.{d}，{} 个文件）",
            h.format_version,
            self.idx.files().len()
        )
    }

    fn definition(&self, at: &QueryPos<'_>) -> Result<DefAnswer, SourceError> {
        let col = Utf8Column(u64::from(at.utf8_col0));
        let Some(sym) = self.idx.symbol_at(at.file, at.line0, col) else {
            let reason = if self.idx.file_id(at.file).is_none() {
                "索引中没有这个文件"
            } else {
                "该位置没有出现位置记录"
            };
            return Ok(DefAnswer {
                symbol: Some(format!("<{reason}>")),
                precision: Some("exact"),
                ..DefAnswer::default()
            });
        };
        let defs: Vec<SrcLine> = self
            .idx
            .goto_definition(at.file, at.line0, col)
            .into_iter()
            .map(|l| SrcLine {
                file: l.path.to_owned(),
                line0: l.line,
            })
            .collect();
        // 外部符号：只有索引里带元数据视图（#34）时才算有落点。v2 的元数据段为空。
        // TODO(#34)：元数据视图有读取接口后，改为按符号查找对应视图是否存在。
        let external = if defs.is_empty() && !self.idx.metadata_views().is_empty() {
            match self.idx.symbol_key(sym) {
                Ok(rove_lang_cs::index::SymbolKey::Global(k)) => Some(doc_comment_id(k).to_owned()),
                _ => None,
            }
        } else {
            None
        };
        Ok(DefAnswer {
            defs,
            external,
            symbol: self.symbol_desc(sym),
            precision: Some("exact"),
        })
    }

    fn references(&self, at: &QueryPos<'_>) -> Result<RefsAnswer, SourceError> {
        let col = Utf8Column(u64::from(at.utf8_col0));
        let refs = self
            .idx
            .find_references(at.file, at.line0, col)
            .into_iter()
            .filter(|l| !l.is_def)
            .map(|l| SrcLine {
                file: l.path.to_owned(),
                line0: l.line,
            })
            .collect();
        Ok(RefsAnswer {
            refs,
            precision: Some("exact"),
        })
    }
}

/// L1 快速声明索引（#37）。
///
/// TODO(#37)：L1 合并后在这里构建/加载声明索引，`definition` 返回其候选（精确度 approximate），
/// `references` 保持 Unsupported（L1 不做查找引用）。统计时 `type_ref` 分类的准确率即 #37 的验收数字。
pub struct L1Decls;

impl NavSource for L1Decls {
    fn name(&self) -> &'static str {
        "l1"
    }

    fn definition(&self, _: &QueryPos<'_>) -> Result<DefAnswer, SourceError> {
        Err(SourceError::Unsupported(
            "L1 声明索引（#37）尚未接入适配层".to_owned(),
        ))
    }

    fn references(&self, _: &QueryPos<'_>) -> Result<RefsAnswer, SourceError> {
        Err(SourceError::Unsupported("L1 不提供查找引用".to_owned()))
    }
}
