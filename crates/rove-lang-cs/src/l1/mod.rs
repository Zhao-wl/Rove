//! L1 快速声明索引（#37）：tree-sitter 扫描全工作区的 C# 声明，打开项目后几秒内可用。
//!
//! 能力边界照 `LANGUAGE_INTELLIGENCE` §2.1：只认名字、不认类型。
//!
//! | 查询 | 精确度 |
//! |---|---|
//! | [`L1Index::workspace_symbols`] 工作区符号模糊搜索 | `Syntax`（声明位置来自语法树，不涉及绑定） |
//! | [`L1Index::resolve_type`] 按命名空间 + using 解析类型引用 | `Approximate`（接近精确，但不是编译器绑定） |
//! | [`L1Index::member_candidates`] 按名称列同名成员 | **一律** `Approximate` |
//!
//! 什么时候用 L1、什么时候用 L2 由 #38 决定；L1 不实现 [`crate::query::CsQueries`]，只给出结果和精确度。
//!
//! 输入只来自调用方：文件列表与文件事件由装配处从 #15 的 `Workspace` 转发（[`L1Index::build`]、
//! [`L1Index::apply_changes`]），L1 不遍历目录、不读 `.gitignore`；缓存目录由 #36 或装配处给出
//! （[`L1Options::cache_dir`]），L1 只在其中读写 `l1-decls.bin`。
//!
//! 线程：全部方法是同步的，必须在后台线程调用（与 `CsQueries` 相同）。`build` 期间查询照常可用，
//! 看到的是已经并入的部分（[`L1Index::progress`] 报告进度）。

mod extract;
mod fuzzy;
mod names;
mod persist;
mod resolve;
mod search;
#[cfg(test)]
mod tests;

pub use fuzzy::{SimpleScorer, SymbolScorer};

use crate::query::{ApproximationReason, Precision, QueryResult, Target, WorkspaceSymbol};
use extract::{Decl, Parsed, Range32, Scope, Using, NONE};
use names::{content_hash, FxMap, Interner, Name};
use rove_base::{
    Cancellation, Error, Line, Position, Resource, Result, Span, TextRange, Utf8Column,
    WorkspacePath,
};
use std::{
    borrow::Cow,
    collections::{BTreeSet, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Mutex, RwLock,
    },
    time::{Duration, Instant, UNIX_EPOCH},
};

/// 缓存文件名，放在调用方给的目录下（与 L2 索引同一个项目目录，不同文件）。
pub const CACHE_FILE_NAME: &str = "l1-decls.bin";

/// 超过这个大小的 `.cs` 不解析（多半是生成的数据表），记为已扫描、没有声明。
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// 声明种类。类型 6 种，成员 9 种。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum DeclKind {
    Class = 0,
    Struct = 1,
    Interface = 2,
    Enum = 3,
    Record = 4,
    Delegate = 5,
    Method = 10,
    Property = 11,
    Field = 12,
    Event = 13,
    Constructor = 14,
    Destructor = 15,
    Indexer = 16,
    Operator = 17,
    EnumMember = 18,
}

impl DeclKind {
    pub fn is_type(self) -> bool {
        (self as u8) < 10
    }

    fn from_u8(v: u8) -> Option<Self> {
        use DeclKind::*;
        Some(match v {
            0 => Class,
            1 => Struct,
            2 => Interface,
            3 => Enum,
            4 => Record,
            5 => Delegate,
            10 => Method,
            11 => Property,
            12 => Field,
            13 => Event,
            14 => Constructor,
            15 => Destructor,
            16 => Indexer,
            17 => Operator,
            18 => EnumMember,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum UsingKind {
    Namespace = 0,
    Static = 1,
    Alias = 2,
}

/// 一个对外符号。`partial` 类型分布在多个文件时是**一个**符号、多个 `locations`
/// （按路径、行排序）；成员每条声明一个符号（重载各自独立）。
#[derive(Clone, Debug)]
pub struct L1Symbol {
    pub name: Arc<str>,
    /// 命名空间 + 外层类型，例如 `Game.Player`；全局命名空间的顶层类型为空串。
    pub container: Arc<str>,
    pub kind: DeclKind,
    pub arity: u8,
    pub locations: Arc<[Target]>,
}

impl L1Symbol {
    /// 转成 #1 契约的工作区符号：每个声明位置一条（partial 类型给出多条）。
    pub fn to_workspace_symbols(&self) -> impl Iterator<Item = WorkspaceSymbol> + '_ {
        self.locations.iter().map(|target| WorkspaceSymbol {
            name: self.name.clone(),
            container: self.container.clone(),
            target: target.clone(),
        })
    }
}

/// 文件变化（来自 #15 的 `FileEvent`，由装配处转换；非 `.cs` 文件自动忽略）。
/// `RescanRequired` 不在这里：收到它时调用方重新取文件列表调用 [`L1Index::build`]。
#[derive(Clone, Debug)]
pub enum L1Change {
    /// 新建或修改。
    Upserted(WorkspacePath),
    Removed(WorkspacePath),
    Renamed {
        from: WorkspacePath,
        to: WorkspacePath,
    },
}

#[derive(Clone)]
pub struct L1Options {
    /// 工作区根目录；`WorkspacePath` 相对于它。
    pub root: PathBuf,
    /// 缓存目录（#36 定义布局）；None 表示不持久化。
    pub cache_dir: Option<PathBuf>,
    /// 并行读文件 + 解析的线程数；0 表示 `min(可用核数, 16)`。热缓存时解析是瓶颈，线程越多越快；
    /// 冷缓存的 HDD 上瓶颈是寻道，调小（例如 2～4）可以减少随机读。
    pub threads: usize,
    /// 工作区符号搜索的打分器；#24 合并后由装配处换成 VS Code `fuzzyScorer` 的移植。
    pub scorer: Arc<dyn SymbolScorer>,
}

impl L1Options {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            cache_dir: None,
            threads: 0,
            scorer: Arc::new(SimpleScorer),
        }
    }

    pub fn with_cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(dir.into());
        self
    }

    pub fn with_threads(mut self, threads: usize) -> Self {
        self.threads = threads;
        self
    }

    pub fn with_scorer(mut self, scorer: Arc<dyn SymbolScorer>) -> Self {
        self.scorer = scorer;
        self
    }

    fn worker_count(&self) -> usize {
        if self.threads > 0 {
            return self.threads;
        }
        std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(16)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum L1State {
    /// 还没有任何数据。
    Empty,
    /// 正在构建或增量更新；已并入的部分可以查询。
    Building,
    Ready,
}

/// 当前构建进度（#36 状态栏、#38 路由用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct L1Progress {
    pub state: L1State,
    pub done: usize,
    pub total: usize,
}

/// 一次 [`L1Index::build`] / [`L1Index::apply_changes`] 的结果。
#[derive(Clone, Debug, Default)]
pub struct BuildReport {
    /// 输入中的 `.cs` 文件数。
    pub files: usize,
    /// 实际读取并用 tree-sitter 解析的文件数。
    pub parsed: usize,
    /// 大小与修改时间都没变、未读取内容的文件数。
    pub unchanged: usize,
    /// 修改时间变了但内容哈希相同、未重新解析的文件数。
    pub rehashed: usize,
    pub removed: usize,
    /// 读取失败（被删除、无权限）的文件数；它们不在索引里。
    pub failed: usize,
    /// 本次从缓存文件载入的文件数（只有首次构建会载入）。
    pub loaded_from_cache: usize,
    pub elapsed: Duration,
}

/// 内存与规模统计（常驻内存的估算，不含分配器开销）。
#[derive(Clone, Copy, Debug, Default)]
pub struct L1Stats {
    pub files: usize,
    pub declarations: usize,
    pub symbols: usize,
    pub names: usize,
    /// 自创建以来累计解析的文件数（持久化/增量测试用它断言“没有重扫”）。
    pub files_parsed_total: u64,
    pub approx_heap_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub len: u64,
    pub mtime_ns: u64,
}

pub(crate) struct FileEntry {
    pub path: WorkspacePath,
    pub stamp: Stamp,
    pub hash: u64,
    /// 本次会话中已核对过磁盘内容（从缓存载入、尚未核对的为 false）。
    pub verified: bool,
    pub decls: Box<[Decl<Name>]>,
    pub scopes: Box<[Scope<Name>]>,
    pub usings: Box<[Using<Name>]>,
    /// 每条声明对应的符号 ID。
    pub symbols: Box<[u32]>,
}

struct SymbolEntry {
    name: Name,
    kind: DeclKind,
    arity: u8,
    container: Name,
    /// 类型的完整键（`Ns.Outer`1.Inner`）；成员为 EMPTY。
    key: Name,
    decls: Vec<(u32, u32)>,
}

#[derive(Default)]
pub(crate) struct State {
    names: Interner,
    files: Vec<Option<FileEntry>>,
    file_ids: FxMap<WorkspacePath, u32>,
    free_files: Vec<u32>,
    symbols: Vec<SymbolEntry>,
    free_symbols: Vec<u32>,
    types: FxMap<Name, u32>,
    by_name: FxMap<Name, Vec<u32>>,
    /// `by_name` 键集合的扁平副本，随增删同步维护，符号搜索直接扫它。
    search: search::SearchNames,
    global_using_files: BTreeSet<u32>,
    decl_count: usize,
    generation: u64,
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        let mut s = String::with_capacity(prefix.len() + 1 + name.len());
        s.push_str(prefix);
        s.push('.');
        s.push_str(name);
        s
    }
}

fn type_segment(name: &str, arity: u8) -> Cow<'_, str> {
    if arity == 0 {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(format!("{name}`{arity}"))
    }
}

impl State {
    fn file(&self, id: u32) -> &FileEntry {
        self.files[id as usize].as_ref().expect("live file id")
    }

    fn live_symbol(&self, sid: u32) -> bool {
        !self.symbols[sid as usize].decls.is_empty()
    }

    fn alloc_symbol(&mut self, entry: SymbolEntry) -> u32 {
        let name = entry.name;
        let sid = if let Some(sid) = self.free_symbols.pop() {
            self.symbols[sid as usize] = entry;
            sid
        } else {
            self.symbols.push(entry);
            (self.symbols.len() - 1) as u32
        };
        let list = self.by_name.entry(name).or_default();
        if list.is_empty() {
            let text = self.names.get(name).clone();
            self.search.insert(name, &text);
        }
        list.push(sid);
        sid
    }

    /// 并入一个文件；同路径已存在时先移除旧内容（保持文件 ID 不变）。
    fn insert<S: AsRef<str>>(
        &mut self,
        path: WorkspacePath,
        stamp: Stamp,
        hash: u64,
        verified: bool,
        parsed: Parsed<S>,
    ) {
        let fid = match self.file_ids.get(&path) {
            Some(&fid) => {
                self.remove_contributions(fid);
                fid
            }
            None => {
                let fid = self.free_files.pop().unwrap_or_else(|| {
                    self.files.push(None);
                    (self.files.len() - 1) as u32
                });
                self.file_ids.insert(path.clone(), fid);
                fid
            }
        };
        let names = &mut self.names;
        let scopes: Box<[Scope<Name>]> = parsed
            .scopes
            .iter()
            .map(|s| Scope {
                ns: names.intern(s.ns.as_ref()),
                parent: s.parent,
                range: s.range,
            })
            .collect();
        let usings: Box<[Using<Name>]> = parsed
            .usings
            .iter()
            .map(|u| Using {
                scope: u.scope,
                kind: u.kind,
                global: u.global,
                alias: names.intern(u.alias.as_ref()),
                target: names.intern(u.target.as_ref()),
            })
            .collect();
        let decls: Box<[Decl<Name>]> = parsed
            .decls
            .iter()
            .map(|d| Decl {
                name: names.intern(d.name.as_ref()),
                kind: d.kind,
                arity: d.arity,
                parent: d.parent,
                scope: d.scope,
                range: d.range,
                selection: d.selection,
            })
            .collect();

        // 显示用容器（不带泛型标记）与类型键（带 `n）：父声明总在子声明之前（先序）。
        let mut display: Vec<Name> = Vec::with_capacity(decls.len());
        let mut keys: Vec<Name> = Vec::with_capacity(decls.len());
        let mut symbols = Vec::with_capacity(decls.len());
        for (i, d) in decls.iter().enumerate() {
            let (container, key_prefix) = match d.parent {
                NONE => {
                    let ns = scopes.get(d.scope as usize).map_or(Name::EMPTY, |s| s.ns);
                    (ns, ns)
                }
                p => (display[p as usize], keys[p as usize]),
            };
            // 只有类型会成为别人的容器，成员不需要自己的显示名与键（省下几十万个驻留字符串）。
            let (own_display, key) = if d.kind.is_type() {
                let name = self.names.get(d.name).clone();
                let c = self.names.get(container).clone();
                let prefix = self.names.get(key_prefix).clone();
                (
                    self.names.intern(&join(&c, &name)),
                    self.names
                        .intern(&join(&prefix, &type_segment(&name, d.arity))),
                )
            } else {
                (Name::EMPTY, Name::EMPTY)
            };
            display.push(own_display);
            keys.push(key);
            let sid = match (d.kind.is_type(), self.types.get(&key)) {
                (true, Some(&sid)) => {
                    self.symbols[sid as usize].decls.push((fid, i as u32));
                    sid
                }
                (is_type, _) => {
                    let sid = self.alloc_symbol(SymbolEntry {
                        name: d.name,
                        kind: d.kind,
                        arity: d.arity,
                        container,
                        key,
                        decls: vec![(fid, i as u32)],
                    });
                    if is_type {
                        self.types.insert(key, sid);
                    }
                    sid
                }
            };
            symbols.push(sid);
        }
        if usings.iter().any(|u| u.global) {
            self.global_using_files.insert(fid);
        }
        self.decl_count += decls.len();
        self.files[fid as usize] = Some(FileEntry {
            path,
            stamp,
            hash,
            verified,
            decls,
            scopes,
            usings,
            symbols: symbols.into_boxed_slice(),
        });
    }

    fn remove_contributions(&mut self, fid: u32) {
        let Some(entry) = self.files[fid as usize].take() else {
            return;
        };
        // 同名列表（`Init` 有上百条）按名字成批过滤一次，避免逐个符号 retain 的平方开销。
        let mut dead: FxMap<Name, Vec<u32>> = FxMap::default();
        for &sid in entry.symbols.iter() {
            let sym = &mut self.symbols[sid as usize];
            if sym.decls.is_empty() {
                continue; // 同一个 partial 类型在本文件里声明了多次，已经处理过
            }
            sym.decls.retain(|&(f, _)| f != fid);
            if sym.decls.is_empty() {
                let (name, key, is_type) = (sym.name, sym.key, sym.kind.is_type());
                if is_type && self.types.get(&key) == Some(&sid) {
                    self.types.remove(&key);
                }
                dead.entry(name).or_default().push(sid);
            }
        }
        for (name, mut sids) in dead {
            sids.sort_unstable();
            if let Some(list) = self.by_name.get_mut(&name) {
                list.retain(|s| sids.binary_search(s).is_err());
                if list.is_empty() {
                    self.by_name.remove(&name);
                    self.search.remove(name);
                }
            }
            self.free_symbols.extend(sids);
        }
        self.global_using_files.remove(&fid);
        self.decl_count -= entry.decls.len();
        // 槽位已空，但 file_ids 仍把路径映射到 fid（insert 会复用）；真正删除由 remove_file 完成。
    }

    fn remove_file(&mut self, path: &WorkspacePath) -> bool {
        let Some(fid) = self.file_ids.remove(path) else {
            return false;
        };
        self.remove_contributions(fid);
        self.free_files.push(fid);
        true
    }

    fn live_file(&self, path: &WorkspacePath) -> Option<(u32, &FileEntry)> {
        let fid = *self.file_ids.get(path)?;
        self.files[fid as usize].as_ref().map(|f| (fid, f))
    }

    fn target(&self, fid: u32, decl: u32) -> Target {
        let file = self.file(fid);
        let d = &file.decls[decl as usize];
        Target {
            resource: Resource::Workspace(file.path.clone()),
            range: text_range(d.range),
            selection: text_range(d.selection),
        }
    }

    fn symbol(&self, sid: u32) -> L1Symbol {
        let s = &self.symbols[sid as usize];
        let mut decls = s.decls.clone();
        decls.sort_by(|a, b| {
            let (fa, fb) = (self.file(a.0), self.file(b.0));
            fa.path.cmp(&fb.path).then(
                fa.decls[a.1 as usize]
                    .range
                    .start_line
                    .cmp(&fb.decls[b.1 as usize].range.start_line),
            )
        });
        L1Symbol {
            name: self.names.get(s.name).clone(),
            container: self.names.get(s.container).clone(),
            kind: s.kind,
            arity: s.arity,
            locations: decls.iter().map(|&(f, d)| self.target(f, d)).collect(),
        }
    }

    fn heap_bytes(&self) -> usize {
        use std::mem::size_of;
        let mut bytes = self.names.heap_bytes() + self.search.heap_bytes();
        for f in self.files.iter().flatten() {
            bytes += size_of::<FileEntry>()
                + f.path.as_str().len()
                + 16
                + f.decls.len() * (size_of::<Decl<Name>>() + 4)
                + f.scopes.len() * size_of::<Scope<Name>>()
                + f.usings.len() * size_of::<Using<Name>>();
        }
        bytes += self.files.capacity() * size_of::<Option<FileEntry>>();
        bytes += self.file_ids.capacity() * (size_of::<WorkspacePath>() + 4 + 8);
        for s in &self.symbols {
            bytes += size_of::<SymbolEntry>() + s.decls.capacity() * 8;
        }
        bytes += self.types.capacity() * (8 + 1);
        for list in self.by_name.values() {
            bytes += 8 + 24 + 1 + list.capacity() * 4;
        }
        bytes
    }
}

fn text_range(r: Range32) -> TextRange {
    let pos = |line: u32, col: u32| Position {
        line: Line(line as u64),
        column: Utf8Column(col as u64),
    };
    Span::new(pos(r.start_line, r.start_col), pos(r.end_line, r.end_col)).unwrap_or_else(|_| {
        Span::new(
            pos(r.start_line, r.start_col),
            pos(r.start_line, r.start_col),
        )
        .expect("empty span")
    })
}

fn is_cs(path: &WorkspacePath) -> bool {
    let p = path.as_str();
    p.len() > 3 && p[p.len() - 3..].eq_ignore_ascii_case(".cs")
}

/// 解码为 UTF-8：去掉 UTF-8 BOM（与 rove-buffer 一样，坐标不含 BOM）、按 BOM 解 UTF-16，
/// 其余非 UTF-8 内容（如 GBK 注释）有损替换——标识符是 ASCII，只有同一行里非 ASCII 字符之后的列会偏。
fn decode(bytes: &[u8]) -> Cow<'_, str> {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(rest);
    }
    let utf16 = |rest: &[u8], le: bool| {
        let (pairs, _odd) = rest.as_chunks::<2>();
        let units: Vec<u16> = pairs
            .iter()
            .map(|&c| {
                if le {
                    u16::from_le_bytes(c)
                } else {
                    u16::from_be_bytes(c)
                }
            })
            .collect();
        Cow::Owned(String::from_utf16_lossy(&units))
    };
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(rest, false);
    }
    String::from_utf8_lossy(bytes)
}

enum Outcome {
    Unchanged(WorkspacePath),
    Rehashed(WorkspacePath, Stamp),
    Parsed(WorkspacePath, Stamp, u64, Parsed<String>),
    Failed(WorkspacePath),
}

/// L1 声明索引。`Send + Sync`，可放进 `Arc` 在后台线程之间共享。
pub struct L1Index {
    options: L1Options,
    state: RwLock<State>,
    /// 同一时刻只有一个构建/增量更新。
    writer: Mutex<bool>,
    building: AtomicBool,
    total: AtomicUsize,
    done: AtomicUsize,
    parsed_total: AtomicU64,
}

const REASON: &str = "l1-declaration-index";

impl L1Index {
    pub fn new(options: L1Options) -> Self {
        Self {
            options,
            state: RwLock::new(State::default()),
            writer: Mutex::new(false),
            building: AtomicBool::new(false),
            total: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            parsed_total: AtomicU64::new(0),
        }
    }

    pub fn options(&self) -> &L1Options {
        &self.options
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(|e| e.into_inner())
    }

    /// 全量对齐：`files` 是工作区当前的文件列表（非 `.cs` 自动忽略）。首次调用会先载入缓存，
    /// 之后只读取大小/修改时间变化的文件，内容哈希相同的不重新解析；不在列表里的文件移除。
    /// 被取消时已并入的部分保留，返回 `Cancelled`。
    pub fn build(
        &self,
        files: impl IntoIterator<Item = WorkspacePath>,
        cancel: &Cancellation,
    ) -> Result<BuildReport> {
        let start = Instant::now();
        let mut guard = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let mut report = BuildReport::default();
        if !*guard {
            *guard = true;
            report.loaded_from_cache = self.load_cache();
        }
        let files: Vec<WorkspacePath> = {
            let mut seen = HashSet::new();
            files
                .into_iter()
                .filter(|p| is_cs(p) && seen.insert(p.clone()))
                .collect()
        };
        report.files = files.len();
        // 列表外的文件先移除，避免查询看到已删除文件。
        {
            let keep: HashSet<&WorkspacePath> = files.iter().collect();
            let mut state = self.write();
            let stale: Vec<WorkspacePath> = state
                .file_ids
                .keys()
                .filter(|p| !keep.contains(p))
                .cloned()
                .collect();
            for p in &stale {
                state.remove_file(p);
            }
            report.removed = stale.len();
            if !stale.is_empty() {
                state.generation += 1;
            }
        }
        let result = self.process(files, cancel, &mut report);
        report.elapsed = start.elapsed();
        result.map(|()| report)
    }

    /// 增量更新：处理文件事件。同一文件的修改只解析这一份；内容未变（哈希相同）不重新解析。
    pub fn apply_changes(
        &self,
        changes: &[L1Change],
        cancel: &Cancellation,
    ) -> Result<BuildReport> {
        let start = Instant::now();
        let _guard = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let mut report = BuildReport::default();
        let mut upserts = Vec::new();
        {
            let mut state = self.write();
            let push = |p: &WorkspacePath, upserts: &mut Vec<WorkspacePath>| {
                if is_cs(p) && !upserts.contains(p) {
                    upserts.push(p.clone());
                }
            };
            for change in changes {
                match change {
                    L1Change::Upserted(p) => push(p, &mut upserts),
                    L1Change::Removed(p) => {
                        upserts.retain(|u| u != p);
                        report.removed += state.remove_file(p) as usize;
                    }
                    L1Change::Renamed { from, to } => {
                        upserts.retain(|u| u != from);
                        report.removed += state.remove_file(from) as usize;
                        push(to, &mut upserts);
                    }
                }
            }
            if report.removed > 0 {
                state.generation += 1;
            }
        }
        report.files = upserts.len();
        let result = self.process(upserts, cancel, &mut report);
        report.elapsed = start.elapsed();
        result.map(|()| report)
    }

    fn process(
        &self,
        files: Vec<WorkspacePath>,
        cancel: &Cancellation,
        report: &mut BuildReport,
    ) -> Result<()> {
        if files.is_empty() {
            return cancel.check();
        }
        let known: FxMap<WorkspacePath, (Stamp, u64)> = {
            let state = self.read();
            files
                .iter()
                .filter_map(|p| {
                    state
                        .live_file(p)
                        .map(|(_, f)| (p.clone(), (f.stamp, f.hash)))
                })
                .collect()
        };
        self.total.store(files.len(), Ordering::Relaxed);
        self.done.store(0, Ordering::Relaxed);
        self.building.store(true, Ordering::Release);
        let workers = self.options.worker_count().min(files.len()).max(1);
        let next = AtomicUsize::new(0);
        let (tx, rx) = mpsc::sync_channel::<Outcome>(1024);
        let root = &self.options.root;
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let tx = tx.clone();
                let (files, known, next) = (&files, &known, &next);
                scope.spawn(move || {
                    let mut parser = extract::new_parser();
                    loop {
                        if cancel.check().is_err() {
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = files.get(i) else { break };
                        let outcome = scan_one(root, path, known.get(path).copied(), &mut parser);
                        if tx.send(outcome).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tx);
            // 主线程分批并入：每批持写锁一次，查询在批与批之间进行。
            let mut batch = Vec::with_capacity(256);
            loop {
                let first = rx.recv();
                let Ok(first) = first else { break };
                batch.push(first);
                while batch.len() < 256 {
                    match rx.try_recv() {
                        Ok(o) => batch.push(o),
                        Err(_) => break,
                    }
                }
                self.merge(&mut batch, report);
            }
        });
        self.building.store(false, Ordering::Release);
        cancel.check()
    }

    fn merge(&self, batch: &mut Vec<Outcome>, report: &mut BuildReport) {
        let n = batch.len();
        let mut state = self.write();
        let mut changed = false;
        for outcome in batch.drain(..) {
            match outcome {
                Outcome::Unchanged(path) => {
                    report.unchanged += 1;
                    if let Some(&fid) = state.file_ids.get(&path) {
                        if let Some(f) = state.files[fid as usize].as_mut() {
                            f.verified = true;
                        }
                    }
                }
                Outcome::Rehashed(path, stamp) => {
                    report.rehashed += 1;
                    if let Some(&fid) = state.file_ids.get(&path) {
                        if let Some(f) = state.files[fid as usize].as_mut() {
                            f.verified = true;
                            f.stamp = stamp;
                        }
                    }
                }
                Outcome::Parsed(path, stamp, hash, parsed) => {
                    report.parsed += 1;
                    self.parsed_total.fetch_add(1, Ordering::Relaxed);
                    state.insert(path, stamp, hash, true, parsed);
                    changed = true;
                }
                Outcome::Failed(path) => {
                    report.failed += 1;
                    changed |= state.remove_file(&path);
                }
            }
        }
        if changed {
            state.generation += 1;
        }
        drop(state);
        self.done.fetch_add(n, Ordering::Relaxed);
    }

    /// 从缓存目录载入上次的结果；返回载入的文件数。缓存缺失、版本不符或损坏时返回 0（随后全量构建）。
    fn load_cache(&self) -> usize {
        let Some(dir) = &self.options.cache_dir else {
            return 0;
        };
        let Ok(bytes) = std::fs::read(dir.join(CACHE_FILE_NAME)) else {
            return 0;
        };
        let Some(files) = persist::decode(&bytes) else {
            return 0;
        };
        let count = files.len();
        let mut state = self.write();
        for f in files {
            state.insert(f.path, f.stamp, f.hash, false, f.parsed);
        }
        state.generation += 1;
        count
    }

    /// 把当前结果写到缓存目录（临时文件 + 重命名，读者不会看到半截文件）。
    /// 没有配置缓存目录时什么都不做。`build` 结束时会自动保存；增量更新后由调用方择机保存
    /// （例如空闲或关闭项目时）。
    pub fn save(&self) -> Result<()> {
        let Some(dir) = &self.options.cache_dir else {
            return Ok(());
        };
        let bytes = persist::encode(&self.read());
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{CACHE_FILE_NAME}.tmp"));
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, dir.join(CACHE_FILE_NAME))?;
        Ok(())
    }

    /// `build` + `save`：常用的“打开项目”入口。
    pub fn build_and_save(
        &self,
        files: impl IntoIterator<Item = WorkspacePath>,
        cancel: &Cancellation,
    ) -> Result<BuildReport> {
        let report = self.build(files, cancel)?;
        self.save()?;
        Ok(report)
    }

    pub fn progress(&self) -> L1Progress {
        let building = self.building.load(Ordering::Acquire);
        let has_data = !self.read().file_ids.is_empty();
        L1Progress {
            state: match (building, has_data) {
                (true, _) => L1State::Building,
                (false, true) => L1State::Ready,
                (false, false) => L1State::Empty,
            },
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
        }
    }

    /// 文件是否已在本次会话中扫描（或核对）过；从缓存载入但还没核对的返回 false。
    pub fn is_scanned(&self, path: &WorkspacePath) -> bool {
        self.read().live_file(path).is_some_and(|(_, f)| f.verified)
    }

    /// 索引代次：每并入一批变化加一，查询结果的 `index_generation` 与之相同。
    pub fn generation(&self) -> u64 {
        self.read().generation
    }

    pub fn stats(&self) -> L1Stats {
        let state = self.read();
        L1Stats {
            files: state.file_ids.len(),
            declarations: state.decl_count,
            symbols: state.symbols.len() - state.free_symbols.len(),
            names: state.names.len(),
            files_parsed_total: self.parsed_total.load(Ordering::Relaxed),
            approx_heap_bytes: state.heap_bytes(),
        }
    }

    fn result<T>(&self, value: T, precision: Precision, generation: u64) -> QueryResult<T> {
        QueryResult {
            value,
            precision,
            reason: Some(ApproximationReason::Other(Arc::from(REASON))),
            version: None,
            index_generation: generation,
        }
    }

    /// 工作区符号模糊搜索：对每个不同的符号名打一次分，按分数、名字长度、名字排序，
    /// 返回至多 `limit` 个符号（同名的类型排在成员前）。空查询返回空结果。精确度为 `Syntax`。
    pub fn workspace_symbols(
        &self,
        query: &str,
        limit: usize,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[L1Symbol]>>> {
        cancel.check()?;
        let query = query.trim();
        let state = self.read();
        if query.is_empty() || limit == 0 {
            return Ok(self.result(Arc::from([]), Precision::Syntax, state.generation));
        }
        let mut hits = state
            .search
            .score(&state.names, &*self.options.scorer, query, cancel)?;
        let order = |a: &(i32, u32, Name), b: &(i32, u32, Name)| {
            b.0.cmp(&a.0)
                .then(a.1.cmp(&b.1))
                .then_with(|| state.names.get(a.2).cmp(state.names.get(b.2)))
        };
        if hits.len() > limit {
            hits.select_nth_unstable_by(limit - 1, order);
            hits.truncate(limit);
        }
        hits.sort_unstable_by(order);
        let mut out = Vec::with_capacity(limit.min(hits.len() * 2));
        'names: for (_, _, name) in hits {
            let mut sids: Vec<u32> = state.by_name[&name].clone();
            sids.sort_by(|&a, &b| {
                let (sa, sb) = (&state.symbols[a as usize], &state.symbols[b as usize]);
                sa.kind.cmp(&sb.kind).then_with(|| {
                    state
                        .names
                        .get(sa.container)
                        .cmp(state.names.get(sb.container))
                })
            });
            for sid in sids {
                if out.len() >= limit {
                    break 'names;
                }
                out.push(state.symbol(sid));
            }
        }
        Ok(self.result(out.into(), Precision::Syntax, state.generation))
    }

    /// 类型引用解析：`text` 是引用处写出来的类型（`Player`、`Game.Player`、`List<int>`、
    /// `Dictionary<,>`、`global::X.Y`），`position` 是它在 `file` 中的位置。按“外层类型的嵌套类型 →
    /// 由内到外的命名空间 + 该层的 using / using static / using 别名 → 全局 using”查找，泛型按参数个数区分。
    /// 同一层有多个匹配时全部返回（有歧义）。文件不在索引中返回 `NotReady`。精确度为 `Approximate`。
    pub fn resolve_type(
        &self,
        file: &WorkspacePath,
        position: Position,
        text: &str,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[L1Symbol]>>> {
        cancel.check()?;
        let state = self.read();
        let Some((fid, _)) = state.live_file(file) else {
            return Err(Error::NotReady);
        };
        let Some(reference) = resolve::TypeText::parse(text) else {
            return Err(Error::InvalidInput);
        };
        let line = u32::try_from(position.line.0).unwrap_or(u32::MAX);
        let col = u32::try_from(position.column.0).unwrap_or(u32::MAX);
        let sids = resolve::resolve(&state, fid, line, col, &reference);
        let value: Arc<[L1Symbol]> = sids.into_iter().map(|s| state.symbol(s)).collect();
        Ok(self.result(value, Precision::Approximate, state.generation))
    }

    /// 按完整名查类型：`Game.Player`、`Game.Outer.Inner`、`System.Collections.Generic.List<,>`。
    pub fn find_type(&self, qualified: &str) -> Option<L1Symbol> {
        let reference = resolve::TypeText::parse(qualified)?;
        let state = self.read();
        resolve::lookup_path(&state, "", &reference.segments).map(|s| state.symbol(s))
    }

    /// 同名成员候选（方法、属性、字段、事件、构造函数、析构函数、索引器、运算符、枚举成员）。
    /// L1 看不出接收者类型，结果**一律** `Approximate`，绝不标成精确。
    pub fn member_candidates(
        &self,
        name: &str,
        cancel: &Cancellation,
    ) -> Result<QueryResult<Arc<[L1Symbol]>>> {
        cancel.check()?;
        let state = self.read();
        let name = name.strip_prefix('@').unwrap_or(name);
        let value: Arc<[L1Symbol]> =
            match state.names.lookup(name).and_then(|n| state.by_name.get(&n)) {
                Some(sids) => sids
                    .iter()
                    .filter(|&&s| state.live_symbol(s) && !state.symbols[s as usize].kind.is_type())
                    .map(|&s| state.symbol(s))
                    .collect(),
                None => Arc::from([]),
            };
        Ok(self.result(value, Precision::Approximate, state.generation))
    }
}

fn stamp_of(meta: &std::fs::Metadata) -> Stamp {
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64);
    Stamp {
        len: meta.len(),
        mtime_ns,
    }
}

fn scan_one(
    root: &std::path::Path,
    path: &WorkspacePath,
    known: Option<(Stamp, u64)>,
    parser: &mut tree_sitter::Parser,
) -> Outcome {
    let disk = root.join(path.as_str());
    let Ok(meta) = std::fs::metadata(&disk) else {
        return Outcome::Failed(path.clone());
    };
    let stamp = stamp_of(&meta);
    if let Some((old, _)) = known {
        if old == stamp && stamp.mtime_ns != 0 {
            return Outcome::Unchanged(path.clone());
        }
    }
    if stamp.len > MAX_FILE_BYTES {
        return Outcome::Parsed(path.clone(), stamp, 0, Parsed::default_with_root());
    }
    let Ok(bytes) = std::fs::read(&disk) else {
        return Outcome::Failed(path.clone());
    };
    let hash = content_hash(&bytes);
    if let Some((_, old_hash)) = known {
        if old_hash == hash {
            return Outcome::Rehashed(path.clone(), stamp);
        }
    }
    let source = decode(&bytes);
    let parsed = extract::extract(parser, &source).unwrap_or_else(Parsed::default_with_root);
    Outcome::Parsed(path.clone(), stamp, hash, parsed)
}

impl Parsed<String> {
    fn default_with_root() -> Self {
        Parsed {
            decls: Vec::new(),
            scopes: vec![Scope {
                ns: String::new(),
                parent: NONE,
                range: Range32::default(),
            }],
            usings: Vec::new(),
        }
    }
}
