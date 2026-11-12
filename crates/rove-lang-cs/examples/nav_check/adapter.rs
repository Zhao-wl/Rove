//! 适配层：用例 → 查询调用 → 结果归一化。
//!
//! **索引格式或查询接口变化时只改这个文件。** 用例文件（`cases.rs`）、定位（`locate.rs`）
//! 与统计（`report.rs`）只认识这里定义的 [`NavSource`] / [`DefAnswer`] / [`RefsAnswer`]。
//!
//! 现有三个查询来源：
//! - [`L2Index`]：完整语义索引（`CsIndex::goto_definition` / `find_references`）；
//! - [`L1Decls`]：L1 快速声明索引（#37），只回答定义（类型名解析），不做查找引用；
//! - [`RouterSource`]：#38 查询路由（`CsRouter::navigate_detailed`，L2 就绪、文件未修改），
//!   `QueryResult::precision` 填进 [`DefAnswer::precision`] / [`RefsAnswer::precision`]；
//!   用例里写了 `expect.precision` 的会同时断言精确度标记。通过率应与 `l2` 一致。

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

    pub fn symbol_desc(&self, sym: u32) -> Option<String> {
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
        // 外部符号：在元数据视图（#34）里有定义位置才算有落点。
        let external = if defs.is_empty() && !self.idx.metadata_definitions(sym).is_empty() {
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
/// 打开时用 #15 的工作区扫描拿文件列表，构建一次 L1（不读写缓存）。`definition` 取光标处写出的
/// 类型名（含左侧的 `A.B.` / `global::` 限定与右侧泛型实参个数），交给 `L1Index::resolve_type`，
/// 返回全部候选的全部声明位置（partial 多处、歧义多个），精确度 approximate。
/// `references` 保持 Unsupported（L1 不做查找引用）。统计时 `type_ref` 分类的准确率即 #37 的验收数字；
/// 其余分类需要语义信息，L1 答不对是预期内的。
pub struct L1Decls {
    root: std::path::PathBuf,
    index: rove_lang_cs::l1::L1Index,
    desc: String,
}

impl L1Decls {
    pub fn open(root: &Path) -> Result<Self, String> {
        use rove_workspace::{default_settings, EntryKind, Folder, Workspace, WorkspaceConfig};
        let config = WorkspaceConfig::from_settings(&default_settings())
            .map_err(|e| format!("工作区默认设置：{e}"))?
            .without_watching();
        let folder = Folder::scan_root(root, config).map_err(|e| format!("扫描工作区失败：{e}"))?;
        let mut files = Vec::new();
        let mut start = 0;
        loop {
            let page = folder
                .files(start..start + 4096)
                .map_err(|e| format!("读取文件列表失败：{e}"))?;
            start += page.items.len();
            files.extend(
                page.items
                    .iter()
                    .filter(|e| matches!(e.kind, EntryKind::File))
                    .map(|e| e.path.clone()),
            );
            if page.items.len() < 4096 {
                break;
            }
        }
        let index = rove_lang_cs::l1::L1Index::new(rove_lang_cs::l1::L1Options::new(root));
        let report = index
            .build(files, &rove_base::Cancellation::default())
            .map_err(|e| format!("构建 L1 失败：{e}"))?;
        let desc = format!(
            "L1 快速声明索引（{} 个 .cs，{} 个符号，构建 {} ms）",
            report.files,
            index.stats().symbols,
            report.elapsed.as_millis()
        );
        Ok(L1Decls {
            root: root.to_owned(),
            index,
            desc,
        })
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// 光标处写出的类型名：标识符本身 + 左侧的限定（`A.B.`、`global::`）+ 右侧泛型实参个数
/// （写成 `Name<,>` 形式）。泛型实参必须在同一行闭合，否则按非泛型处理。
pub fn type_text_at(line: &str, col: usize) -> Option<String> {
    let b = line.as_bytes();
    if col >= b.len() || !is_ident_byte(b[col]) {
        return None;
    }
    let mut start = col;
    while start > 0 && is_ident_byte(b[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < b.len() && is_ident_byte(b[end]) {
        end += 1;
    }
    // 向左吞并限定符（`@` 逐字标识符前缀一并去掉）。
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
            break; // `global::` 只能在最左边。
        }
    }
    let mut text = line[left..end].replace('@', "");
    // 向右数泛型实参个数。
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
            text.push('<');
            text.push_str(&",".repeat(commas));
            text.push('>');
        }
    }
    Some(text)
}

impl NavSource for L1Decls {
    fn name(&self) -> &'static str {
        "l1"
    }

    fn describe(&self) -> String {
        self.desc.clone()
    }

    fn definition(&self, at: &QueryPos<'_>) -> Result<DefAnswer, SourceError> {
        use rove_base::{Cancellation, Line, Position, Resource, WorkspacePath};
        let none = |why: &str| DefAnswer {
            symbol: Some(format!("<{why}>")),
            precision: Some("approximate"),
            ..DefAnswer::default()
        };
        let Ok(bytes) = std::fs::read(self.root.join(at.file)) else {
            return Ok(none("读不到文件"));
        };
        let text = String::from_utf8_lossy(&bytes);
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let Some(line) = text.lines().nth(at.line0 as usize) else {
            return Ok(none("行号越界"));
        };
        let Some(written) = type_text_at(line, at.utf8_col0 as usize) else {
            return Ok(none("光标处不是标识符"));
        };
        let Ok(file) = WorkspacePath::new(at.file) else {
            return Ok(none("路径不合法"));
        };
        let position = Position {
            line: Line(u64::from(at.line0)),
            column: rove_base::Utf8Column(u64::from(at.utf8_col0)),
        };
        let found =
            match self
                .index
                .resolve_type(&file, position, &written, &Cancellation::default())
            {
                Ok(r) => r.value,
                Err(e) => return Ok(none(&format!("{written}：{e}"))),
            };
        let mut defs: Vec<SrcLine> = found
            .iter()
            .flat_map(|s| s.locations.iter())
            .filter_map(|t| match &t.resource {
                Resource::Workspace(p) => Some(SrcLine {
                    file: p.as_str().to_owned(),
                    line0: t.selection.start().line.0 as u32,
                }),
                _ => None,
            })
            .collect();
        defs.sort();
        defs.dedup();
        let names: Vec<String> = found
            .iter()
            .map(|s| {
                if s.container.is_empty() {
                    s.name.to_string()
                } else {
                    format!("{}.{}", s.container, s.name)
                }
            })
            .collect();
        Ok(DefAnswer {
            defs,
            external: None,
            symbol: Some(format!("{written} → [{}]", names.join(", "))),
            precision: Some("approximate"),
        })
    }

    fn references(&self, _: &QueryPos<'_>) -> Result<RefsAnswer, SourceError> {
        Err(SourceError::Unsupported("L1 不提供查找引用".to_owned()))
    }
}

/// #38 查询路由：L2 就绪、文件未打开（未修改）时经 `CsRouter` 查询，用来验证路由没有引入偏差
/// （`--source router` 的通过率应与 `--source l2` 完全一致）。精确度取路由给出的批级精确度。
pub struct RouterSource {
    index: std::sync::Arc<CsIndex>,
    router: rove_lang_cs::router::CsRouter,
    direct: L2Index,
}

impl RouterSource {
    pub fn open(path: &Path, root: &Path) -> Result<Self, String> {
        use rove_lang_cs::router::{BufferStates, CsRouter, IndexHolder, ManualSemantic};
        use std::sync::Arc;
        let index = Arc::new(
            CsIndex::open(path).map_err(|e| format!("打开索引 {} 失败：{e}", path.display()))?,
        );
        let holder: Arc<dyn IndexHolder> = index.clone();
        let router = CsRouter::new(
            Arc::new(ManualSemantic::ready(holder)),
            Arc::new(BufferStates::new()),
        )
        .with_root(root);
        Ok(RouterSource {
            index,
            router,
            direct: L2Index::open(path)?,
        })
    }

    fn query(
        &self,
        kind: rove_lang_cs::query::NavigationKind,
        at: &QueryPos<'_>,
    ) -> Result<
        rove_lang_cs::query::QueryResult<std::sync::Arc<[rove_lang_cs::router::RoutedTarget]>>,
        SourceError,
    > {
        use rove_base::{Cancellation, Line, Position, Resource, Version, WorkspacePath};
        use rove_lang_cs::query::QueryPoint;
        let path = WorkspacePath::new(at.file).map_err(|_| {
            SourceError::Unsupported(format!("路径不是工作区相对路径：{}", at.file))
        })?;
        let point = QueryPoint {
            resource: Resource::Workspace(path),
            position: Position {
                line: Line(u64::from(at.line0)),
                column: Utf8Column(u64::from(at.utf8_col0)),
            },
            version: Version(0),
        };
        self.router
            .navigate_detailed(kind, &point, &Cancellation::default())
            .map_err(|e| SourceError::Unsupported(format!("路由返回错误：{e}")))
    }
}

fn precision_name(p: rove_lang_cs::query::Precision) -> &'static str {
    match p {
        rove_lang_cs::query::Precision::Exact => "exact",
        rove_lang_cs::query::Precision::Approximate => "approximate",
        rove_lang_cs::query::Precision::Syntax => "syntax",
    }
}

fn src_line(t: &rove_lang_cs::router::RoutedTarget) -> Option<SrcLine> {
    match &t.target.resource {
        rove_base::Resource::Workspace(p) => Some(SrcLine {
            file: p.as_str().to_owned(),
            line0: t.target.selection.start().line.0 as u32,
        }),
        rove_base::Resource::File(p) => Some(SrcLine {
            file: p.to_string_lossy().replace('\\', "/"),
            line0: t.target.selection.start().line.0 as u32,
        }),
        rove_base::Resource::Virtual(v)
            if !v.starts_with(rove_lang_cs::router::METADATA_SCHEME) =>
        {
            Some(SrcLine {
                file: v.to_string(),
                line0: t.target.selection.start().line.0 as u32,
            })
        }
        rove_base::Resource::Virtual(_) => None,
    }
}

impl NavSource for RouterSource {
    fn name(&self) -> &'static str {
        "router"
    }

    fn built_at(&self) -> Option<std::time::SystemTime> {
        self.direct.built_at()
    }

    fn describe(&self) -> String {
        format!("#38 查询路由 → {}", self.direct.describe())
    }

    fn definition(&self, at: &QueryPos<'_>) -> Result<DefAnswer, SourceError> {
        use rove_lang_cs::router::{metadata_symbol_key, TargetRole};
        let r = self.query(rove_lang_cs::query::NavigationKind::Definition, at)?;
        let defs = r.value.iter().filter_map(src_line).collect();
        // 与 `L2Index` 同一口径：索引带元数据视图时才算外部符号有落点。
        let external = r
            .value
            .iter()
            .filter(|t| t.role == TargetRole::Metadata)
            .find_map(|t| metadata_symbol_key(&t.target.resource))
            .filter(|_| !self.index.metadata_views().is_empty())
            .map(|k| doc_comment_id(k).to_owned());
        let col = Utf8Column(u64::from(at.utf8_col0));
        let symbol = match self.index.symbol_at(at.file, at.line0, col) {
            Some(sym) => self.direct.symbol_desc(sym),
            None => Some("<该位置没有出现位置记录>".to_owned()),
        };
        Ok(DefAnswer {
            defs,
            external,
            symbol,
            precision: Some(precision_name(r.precision)),
        })
    }

    fn references(&self, at: &QueryPos<'_>) -> Result<RefsAnswer, SourceError> {
        use rove_lang_cs::router::TargetRole;
        let r = self.query(rove_lang_cs::query::NavigationKind::References, at)?;
        let refs = r
            .value
            .iter()
            .filter(|t| t.role != TargetRole::Definition)
            .filter_map(src_line)
            .collect();
        Ok(RefsAnswer {
            refs,
            precision: Some(precision_name(r.precision)),
        })
    }
}
