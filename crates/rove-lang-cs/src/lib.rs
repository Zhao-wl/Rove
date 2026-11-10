//! rove-lang-cs: C# 语言智能。
//!
//! 目前包含 L2 语义索引的读取与查询（索引由 `tools/rove-indexer-cs` 生成，格式见
//! `docs/CS_INDEX_FORMAT.md`）。索引文件整体内存映射，查询零拷贝：
//! - 定义跳转：文件内出现位置二分查找 → 符号 → 定义位置
//! - 查找引用：符号 → 预排序的出现位置区间

pub mod index;

pub use index::{CsIndex, Location, Occurrence, Role, SymbolRecord};

// Keep query contracts namespaced: storage exports above (notably Location) remain
// separate from navigation targets and coordinate units; avoid wildcard re-exports.
pub mod query;
