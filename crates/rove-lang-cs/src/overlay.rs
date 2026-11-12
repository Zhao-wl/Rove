//! #35 覆盖层：读取常驻索引进程追加写入的覆盖层日志，与基础索引合并成一个查询视图。
//!
//! 格式见 `docs/CS_INDEX_FORMAT.md`「覆盖层（#35）」，写入端是 `tools/rove-indexer-cs/Overlay.cs`。
//!
//! 合并规则：
//! - **文件**：同一路径以最后写入的段为准，整份替换基础索引里这个文件的出现位置、不生效区间与内容哈希；
//!   “已清空”的文件（删除）不再出现在文件表里，它在基础索引中的出现位置也不再返回。
//! - **符号**：覆盖层按键（全局键文本，或“路径 + 声明偏移”的局部键）引用符号。键在基础索引里存在且不属于
//!   被替换的文件时沿用基础索引的符号序号，否则分配覆盖层专用序号（从基础符号数往后排）。符号记录（显示名、
//!   种类、容器）优先取覆盖层里最后一份源码版本。
//! - **关系**：按归属取最新：符号自己的基类型 / 重写关系归“声明它的文件”，成员实现接口成员的关系归“声明它所在
//!   类型的文件”。被覆盖层重算过的部分以最新段为准，其余沿用基础索引（声明已全部被替换掉的符号不再有关系）。
//!
//! 复杂度：合并在构建 [`Overlay`] 时一次完成（覆盖层规模有上限，见 lifecycle 的阈值），查询时基础索引仍是
//! 二分查找，覆盖层是哈希表与文件内二分，与覆盖层段数无关（`cargo xtask bench --filter cs_overlay/`）。
use crate::index::{CsIndex, InactiveLines, Occurrence, Relation, Role, SymbolKey, REL_IMPLEMENTS};
use rove_base::{ByteOffset, Utf8Column};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::Path,
};

pub const MAGIC: &[u8; 8] = b"RVCSOVL\0";
pub const VERSION: u32 = 1;
const HEADER_LEN: usize = 48;

/// 覆盖层里的符号键。
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OvKey {
    Global(String),
    Local { path: String, offset: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegSymbol {
    pub key: OvKey,
    pub name: String,
    pub display: String,
    pub kind: u8,
    /// bit 0：有源码位置；bit 1：来自外部 DLL。
    pub flags: u8,
    pub container: Option<OvKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegOccurrence {
    pub line: u32,
    pub col: u32,
    pub len: u32,
    /// 本段符号表的下标。
    pub symbol: u32,
    pub role: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SegFile {
    pub path: String,
    pub deleted: bool,
    pub hash: u64,
    pub occurrences: Vec<SegOccurrence>,
    pub inactive: Vec<InactiveLines>,
    /// 本文件声明的类型与成员（符号下标）及声明指纹（常驻进程自己用）。
    pub declarations: Vec<(u32, u64)>,
}

/// 关系归属：符号自己的（基类型、重写），或类型的成员实现接口成员的。
pub const OWNER_SELF: u8 = 1;
pub const OWNER_MEMBERS: u8 = 2;

/// 一个覆盖层段（一次重绑）。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Segment {
    pub sequence: u64,
    pub symbols: Vec<SegSymbol>,
    pub files: Vec<SegFile>,
    pub owners: Vec<(u32, u8)>,
    pub relations: Vec<(u32, u32, u8)>,
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(14695981039346656037, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(1099511628211)
    })
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.data.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }
    fn count(&mut self, min_item: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        // 防御损坏数据：元素数不可能超过剩余字节 / 最小元素长度。
        (n.checked_mul(min_item)? <= self.data.len() - self.at).then_some(n)
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
    fn key(&mut self) -> Option<Option<OvKey>> {
        Some(match self.u8()? {
            0 => None,
            1 => Some(OvKey::Global(self.str()?)),
            2 => Some(OvKey::Local {
                path: self.str()?,
                offset: self.u32()?,
            }),
            _ => return None,
        })
    }
}

impl Segment {
    /// 解出 `data` 开头的一个段，返回段与它的字节数。格式、校验和或基础索引标识不对时返回 `None`。
    pub fn decode(data: &[u8], identity: &[u8; 16]) -> Option<(Self, usize)> {
        if data.len() < HEADER_LEN + 8
            || &data[..8] != MAGIC
            || u32::from_le_bytes(data[8..12].try_into().unwrap()) != VERSION
        {
            return None;
        }
        let total = usize::try_from(u64::from_le_bytes(data[16..24].try_into().unwrap())).ok()?;
        if total % 8 != 0 || total < HEADER_LEN + 8 || total > data.len() {
            return None;
        }
        let bytes = &data[..total];
        let checksum = u64::from_le_bytes(bytes[total - 8..].try_into().unwrap());
        if fnv(&bytes[..total - 8]) != checksum || bytes[24..40] != identity[..] {
            return None;
        }
        let mut r = Reader {
            data: &bytes[..total - 8],
            at: 40,
        };
        let mut seg = Segment {
            sequence: r.u64()?,
            ..Default::default()
        };
        for _ in 0..r.count(12)? {
            let key = r.key()??;
            seg.symbols.push(SegSymbol {
                key,
                name: r.str()?,
                display: r.str()?,
                kind: r.u8()?,
                flags: r.u8()?,
                container: r.key()?,
            });
        }
        let n = seg.symbols.len() as u32;
        for _ in 0..r.count(21)? {
            let mut file = SegFile {
                path: r.str()?,
                deleted: r.u8()? != 0,
                hash: r.u64()?,
                ..Default::default()
            };
            for _ in 0..r.count(17)? {
                let o = SegOccurrence {
                    line: r.u32()?,
                    col: r.u32()?,
                    len: r.u32()?,
                    symbol: r.u32()?,
                    role: r.u8()?,
                };
                if o.symbol >= n || o.role == 0 || o.role > 7 {
                    return None;
                }
                file.occurrences.push(o);
            }
            for _ in 0..r.count(8)? {
                file.inactive.push(InactiveLines {
                    start: r.u32()?,
                    end: r.u32()?,
                });
            }
            for _ in 0..r.count(12)? {
                let (s, fp) = (r.u32()?, r.u64()?);
                if s >= n {
                    return None;
                }
                file.declarations.push((s, fp));
            }
            seg.files.push(file);
        }
        for _ in 0..r.count(5)? {
            let (s, part) = (r.u32()?, r.u8()?);
            if s >= n {
                return None;
            }
            seg.owners.push((s, part));
        }
        for _ in 0..r.count(9)? {
            let (from, to, kind) = (r.u32()?, r.u32()?, r.u8()?);
            if from >= n || to >= n {
                return None;
            }
            seg.relations.push((from, to, kind));
        }
        Some((seg, total))
    }

    /// 编码（测试、基准与假的常驻进程用；正式写入端在 C#）。
    pub fn encode(&self, identity: &[u8; 16]) -> Vec<u8> {
        let mut w = Vec::new();
        w.extend_from_slice(MAGIC);
        w.extend_from_slice(&VERSION.to_le_bytes());
        w.extend_from_slice(&0u32.to_le_bytes());
        w.extend_from_slice(&0u64.to_le_bytes());
        w.extend_from_slice(identity);
        w.extend_from_slice(&self.sequence.to_le_bytes());
        fn s(w: &mut Vec<u8>, text: &str) {
            w.extend_from_slice(&(text.len() as u32).to_le_bytes());
            w.extend_from_slice(text.as_bytes());
        }
        fn k(w: &mut Vec<u8>, key: Option<&OvKey>) {
            match key {
                None => w.push(0),
                Some(OvKey::Global(g)) => {
                    w.push(1);
                    s(w, g);
                }
                Some(OvKey::Local { path, offset }) => {
                    w.push(2);
                    s(w, path);
                    w.extend_from_slice(&offset.to_le_bytes());
                }
            }
        }
        let u32s = |w: &mut Vec<u8>, v: u32| w.extend_from_slice(&v.to_le_bytes());
        u32s(&mut w, self.symbols.len() as u32);
        for sym in &self.symbols {
            k(&mut w, Some(&sym.key));
            s(&mut w, &sym.name);
            s(&mut w, &sym.display);
            w.push(sym.kind);
            w.push(sym.flags);
            k(&mut w, sym.container.as_ref());
        }
        u32s(&mut w, self.files.len() as u32);
        for f in &self.files {
            s(&mut w, &f.path);
            w.push(u8::from(f.deleted));
            w.extend_from_slice(&f.hash.to_le_bytes());
            u32s(&mut w, f.occurrences.len() as u32);
            for o in &f.occurrences {
                for v in [o.line, o.col, o.len, o.symbol] {
                    u32s(&mut w, v);
                }
                w.push(o.role);
            }
            u32s(&mut w, f.inactive.len() as u32);
            for r in &f.inactive {
                u32s(&mut w, r.start);
                u32s(&mut w, r.end);
            }
            u32s(&mut w, f.declarations.len() as u32);
            for (sym, fp) in &f.declarations {
                u32s(&mut w, *sym);
                w.extend_from_slice(&fp.to_le_bytes());
            }
        }
        u32s(&mut w, self.owners.len() as u32);
        for (sym, part) in &self.owners {
            u32s(&mut w, *sym);
            w.push(*part);
        }
        u32s(&mut w, self.relations.len() as u32);
        for (from, to, kind) in &self.relations {
            u32s(&mut w, *from);
            u32s(&mut w, *to);
            w.push(*kind);
        }
        while (w.len() + 8) % 8 != 0 {
            w.push(0);
        }
        let total = (w.len() + 8) as u64;
        w[16..24].copy_from_slice(&total.to_le_bytes());
        let checksum = fnv(&w);
        w.extend_from_slice(&checksum.to_le_bytes());
        w
    }
}

/// 读出日志 `bytes` 中从头开始连续有效的段（序号递增）。写到一半的尾部、不配套的段在第一个无效处截止。
pub fn decode_journal(bytes: &[u8], identity: &[u8; 16]) -> (Vec<Segment>, usize) {
    let mut at = 0;
    let mut segments: Vec<Segment> = Vec::new();
    while at < bytes.len() {
        let Some((seg, len)) = Segment::decode(&bytes[at..], identity) else {
            break;
        };
        if segments.last().is_some_and(|l| l.sequence >= seg.sequence) {
            break;
        }
        segments.push(seg);
        at += len;
    }
    (segments, at)
}

/// 从 `offset` 起读日志里新追加的段（序号须大于 `last_sequence`），返回新段与新的有效末尾。
/// 生命周期每次更新只读增量，不重新解码整个日志。
pub fn read_journal_from(
    path: &Path,
    identity: &[u8; 16],
    offset: u64,
    last_sequence: u64,
) -> io::Result<(Vec<Segment>, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), offset)),
        Err(e) => return Err(e),
    };
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let (mut segments, valid) = decode_journal(&bytes, identity);
    if let Some(i) = segments.iter().position(|s| s.sequence <= last_sequence) {
        segments.truncate(i);
        return Ok((segments, offset));
    }
    Ok((segments, offset + valid as u64))
}

/// 读整个日志文件（不存在时为空）。常驻进程以共享方式写它，这里只读。
pub fn read_journal(path: &Path, identity: &[u8; 16]) -> io::Result<(Vec<Segment>, u64)> {
    match fs::read(path) {
        Ok(bytes) => {
            let (segments, valid) = decode_journal(&bytes, identity);
            Ok((segments, valid as u64))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((Vec::new(), 0)),
        Err(e) => Err(e),
    }
}

/// 覆盖层键在基础索引中的查找结果（同一基础索引内可复用）。
#[derive(Debug, Default)]
pub struct KeyCache(HashMap<OvKey, Option<u32>>);

/// 合并后的一个覆盖层文件。
#[derive(Debug)]
struct OvFile {
    path: String,
    /// 合并后的文件号（已清空的新文件为 `NO_FILE`）。
    merged: u32,
    deleted: bool,
    hash: u64,
    /// 文件内按 (行, 列) 排序；symbol 是合并后的序号，file 是合并后的文件号。
    occurrences: Vec<Occurrence>,
    inactive: Vec<InactiveLines>,
}

/// 覆盖层专用符号的记录（或替换基础记录的源码版本）。
#[derive(Clone, Debug)]
struct OvRecord {
    key: OvKey,
    display: String,
    kind: u8,
    container: Option<u32>,
}

/// 基础索引上的全部覆盖层，合并为一次构建好的查询结构。
#[derive(Default)]
pub struct Overlay {
    base_files: u32,
    base_symbols: u32,
    files: Vec<OvFile>,
    by_path: HashMap<String, usize>,
    /// 基础文件号 → 覆盖层文件下标（被替换的文件）。
    base_slot: Vec<u32>,
    /// 新文件（不在基础索引里）按路径排序：合并文件号 = 基础文件数 + 下标。
    new_files: Vec<usize>,
    /// 新文件在基础文件表里的插入点（结果按“路径序”交错排列基础文件与新文件）。
    new_order: Vec<u32>,
    /// 覆盖层专用符号：合并序号 = 基础符号数 + 下标。
    extra: Vec<OvRecord>,
    /// 替换基础记录的源码版本。
    records: FxMap<u32, OvRecord>,
    /// 符号 → 覆盖层里的出现位置（合并文件号, 文件内下标），定义在前、再按路径序与行列。
    by_symbol: FxMap<u32, Vec<(u32, u32)>>,
    self_owned: HashSet<u32, std::hash::BuildHasherDefault<Fx>>,
    members_owned: HashSet<u32, std::hash::BuildHasherDefault<Fx>>,
    /// 按归属取最新后的关系：from → (to, kind)。
    rel_from: FxMap<u32, Vec<(u32, u32)>>,
    rel_to: FxMap<u32, Vec<(u32, u32)>>,
    pub segments: usize,
    pub sequence: u64,
}

const NO_FILE: u32 = u32::MAX;
const UNRESOLVED: u32 = u32::MAX;

/// FxHash（rustc 的哈希）：合并时有几十万次小键查找，SipHash 的抗碰撞在这里用不上。
#[derive(Default, Clone, Copy)]
struct Fx(u64);

impl std::hash::Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut v = [0u8; 8];
            v[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(v));
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u32(&mut self, v: u32) {
        self.write_u64(u64::from(v));
    }
    fn write_u8(&mut self, v: u8) {
        self.write_u64(u64::from(v));
    }
    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<Fx>>;

/// 合并时的符号解析：段内下标 → 合并序号（按需），并记下每个符号的候选记录。
struct Resolver<'a, 'c> {
    maps: Vec<Vec<u32>>,
    ids: FxMap<&'a OvKey, u32>,
    best: FxMap<u32, (bool, usize, usize)>,
    cache: &'c mut KeyCache,
    base: &'a CsIndex,
    segments: &'a [Segment],
}

impl<'a> Resolver<'a, '_> {
    fn symbol(&mut self, ov: &mut Overlay, si: usize, k: u32) -> u32 {
        let id = self.maps[si][k as usize];
        if id != UNRESOLVED {
            return id;
        }
        let s = &self.segments[si].symbols[k as usize];
        let id = self.key(ov, &s.key);
        self.maps[si][k as usize] = id;
        let in_source = s.flags & 1 != 0;
        let candidate = (in_source, si, k as usize);
        let e = self.best.entry(id).or_insert(candidate);
        if (in_source, si) >= (e.0, e.1) {
            *e = candidate;
        }
        id
    }

    fn key(&mut self, ov: &mut Overlay, key: &'a OvKey) -> u32 {
        if let Some(&id) = self.ids.get(key) {
            return id;
        }
        let base_id = match key {
            OvKey::Local { path, .. } if ov.by_path.contains_key(path) => None,
            _ => match self.cache.0.get(key) {
                Some(&found) => found,
                None => {
                    let found = match key {
                        OvKey::Global(text) => self.base.find_symbol(SymbolKey::Global(text)),
                        OvKey::Local { path, offset } => self.base.file_id(path).and_then(|file| {
                            self.base.find_symbol(SymbolKey::Local {
                                file,
                                offset: ByteOffset(u64::from(*offset)),
                            })
                        }),
                    };
                    self.cache.0.insert(key.clone(), found);
                    found
                }
            },
        };
        let id = base_id.unwrap_or_else(|| {
            ov.extra.push(OvRecord {
                key: key.clone(),
                display: String::new(),
                kind: 0,
                container: None,
            });
            ov.base_symbols + ov.extra.len() as u32 - 1
        });
        self.ids.insert(key, id);
        id
    }
}

impl Overlay {
    /// 在基础索引上合并全部段（按顺序，后写的优先）。
    pub fn build(base: &CsIndex, segments: &[Segment]) -> Self {
        Self::build_cached(base, segments, &mut KeyCache::default())
    }

    /// 同 [`Overlay::build`]，键在基础索引里的查找结果记在 `cache` 里（只依赖基础索引，同一世代可以复用），
    /// 日志增长后重建时不必再对旧段的每个键二分。
    pub fn build_cached(base: &CsIndex, segments: &[Segment], cache: &mut KeyCache) -> Self {
        let mut ov = Overlay {
            base_slot: vec![NO_FILE; base.files().len()],
            base_files: base.files().len() as u32,
            base_symbols: base.symbols().len() as u32,
            segments: segments.len(),
            sequence: segments.last().map_or(0, |s| s.sequence),
            ..Default::default()
        };
        // 1. 每个路径的最新版本
        let mut latest: HashMap<&str, (usize, usize)> = HashMap::new();
        for (si, seg) in segments.iter().enumerate() {
            for (fi, f) in seg.files.iter().enumerate() {
                latest.insert(&f.path, (si, fi));
            }
        }
        let mut paths: Vec<&str> = latest.keys().copied().collect();
        paths.sort_unstable_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        for path in &paths {
            let (si, fi) = latest[path];
            let f = &segments[si].files[fi];
            let slot = ov.files.len();
            ov.files.push(OvFile {
                path: f.path.clone(),
                merged: NO_FILE,
                deleted: f.deleted,
                hash: f.hash,
                occurrences: Vec::new(),
                inactive: f.inactive.clone(),
            });
            ov.by_path.insert(f.path.clone(), slot);
            if let Some(id) = base.file_id(&f.path) {
                ov.base_slot[id as usize] = slot as u32;
                ov.files[slot].merged = id;
            } else if !f.deleted {
                ov.new_files.push(slot);
            }
        }
        for (rank, &slot) in ov.new_files.iter().enumerate() {
            let path = ov.files[slot].path.as_bytes();
            // 插入点：基础文件表里路径小于它的文件数（文件表按路径字节序排列，二分）。
            let (mut lo, mut hi) = (0, ov.base_files);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if base.file_path(mid).is_some_and(|p| p.as_bytes() < path) {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            let merged = ov.base_files + rank as u32;
            ov.files[slot].merged = merged;
            ov.new_order.push(lo);
        }

        // 2. 符号键 → 合并序号：按需解析，只解析最新文件版本、最新归属用到的符号（被后写段取代的旧版本不解析）
        let mut r = Resolver {
            maps: segments
                .iter()
                .map(|s| vec![UNRESOLVED; s.symbols.len()])
                .collect(),
            ids: FxMap::default(),
            best: FxMap::default(),
            cache,
            base,
            segments,
        };

        // 3. 出现位置
        for slot in 0..ov.files.len() {
            let (si, fi) = latest[ov.files[slot].path.as_str()];
            if ov.files[slot].deleted {
                continue;
            }
            let merged = ov.merged_file(slot);
            let mut occs: Vec<Occurrence> =
                Vec::with_capacity(segments[si].files[fi].occurrences.len());
            for o in &segments[si].files[fi].occurrences {
                occs.push(Occurrence {
                    file: merged,
                    line: o.line,
                    col: Utf8Column(u64::from(o.col)),
                    symbol: r.symbol(&mut ov, si, o.symbol),
                    len: o.len,
                    role: u16::from(o.role),
                });
            }
            occs.sort_unstable_by_key(|o| (o.line, o.col.0, o.role, o.len, o.symbol));
            for (i, o) in occs.iter().enumerate() {
                ov.by_symbol
                    .entry(o.symbol)
                    .or_default()
                    .push((merged, i as u32));
            }
            ov.files[slot].occurrences = occs;
        }
        let mut by_symbol = std::mem::take(&mut ov.by_symbol);
        for list in by_symbol.values_mut() {
            if list.len() > 1 {
                list.sort_unstable_by_key(|&(f, i)| {
                    let o = &ov.files[ov.slot(f).unwrap()].occurrences[i as usize];
                    (!Role(o.role).is_def(), ov.order(f), o.line, o.col.0)
                });
            }
        }
        ov.by_symbol = by_symbol;

        // 4. 关系：每个（符号, 部分）以最后一个声明它的段为准。归属只在“段里声明它的文件版本仍是该文件的
        //    最新版本”时有效：文件后来被改写、不再声明这个符号时，旧段的归属与关系一并作废（否则改名或
        //    删掉的成员会留下旧关系）。
        let mut owner_seg: FxMap<(u32, u8), usize> = FxMap::default();
        for (si, seg) in segments.iter().enumerate() {
            let mut declared = vec![false; seg.symbols.len()];
            for (fi, f) in seg.files.iter().enumerate() {
                if latest.get(f.path.as_str()) != Some(&(si, fi)) {
                    continue;
                }
                for o in &f.occurrences {
                    if u16::from(o.role) & Role::DEF != 0 {
                        declared[o.symbol as usize] = true;
                    }
                }
            }
            for &(s, part) in &seg.owners {
                if !declared[s as usize] {
                    continue;
                }
                let id = r.symbol(&mut ov, si, s);
                owner_seg.insert((id, part), si);
            }
        }
        for &(id, part) in owner_seg.keys() {
            if part == OWNER_SELF {
                ov.self_owned.insert(id);
            } else {
                ov.members_owned.insert(id);
            }
        }
        let mut seen = HashSet::new();
        for (si, seg) in segments.iter().enumerate() {
            for &(from, to, kind) in &seg.relations {
                let owner = if u32::from(kind) == REL_IMPLEMENTS {
                    let Some(container) = &seg.symbols[from as usize].container else {
                        continue;
                    };
                    (r.key(&mut ov, container), OWNER_MEMBERS)
                } else {
                    (r.symbol(&mut ov, si, from), OWNER_SELF)
                };
                if owner_seg.get(&owner) != Some(&si) {
                    continue;
                }
                let (f, t) = (r.symbol(&mut ov, si, from), r.symbol(&mut ov, si, to));
                if !seen.insert((f, t, kind)) {
                    continue;
                }
                ov.rel_from.entry(f).or_default().push((t, u32::from(kind)));
                ov.rel_to.entry(t).or_default().push((f, u32::from(kind)));
            }
        }

        // 5. 记录：用到的符号里，最后一份源码版本优先，其次最后一份
        let best: Vec<(u32, (bool, usize, usize))> =
            r.best.iter().map(|(&id, &b)| (id, b)).collect();
        for (id, (in_source, si, k)) in best {
            let s = &segments[si].symbols[k];
            let container = s.container.as_ref().map(|c| r.key(&mut ov, c));
            let record = OvRecord {
                key: s.key.clone(),
                display: s.display.clone(),
                kind: s.kind,
                container,
            };
            if id >= ov.base_symbols {
                ov.extra[(id - ov.base_symbols) as usize] = record;
            } else if in_source || base.symbols()[id as usize].flags & 1 == 0 {
                ov.records.insert(id, record);
            }
        }
        ov
    }

    /// 合并文件号的排序键：基础文件 (号, 1, 0)，新文件 (插入点, 0, 名次)——新文件排在插入点处的基础文件之前。
    fn order(&self, file: u32) -> (u32, u8, u32) {
        if file < self.base_files {
            (file, 1, 0)
        } else {
            let rank = file - self.base_files;
            (self.new_order[rank as usize], 0, rank)
        }
    }

    fn merged_file(&self, slot: usize) -> u32 {
        self.files[slot].merged
    }

    fn slot(&self, file: u32) -> Option<usize> {
        if file < self.base_files {
            self.base_slot
                .get(file as usize)
                .filter(|&&s| s != NO_FILE)
                .map(|&s| s as usize)
        } else {
            self.new_files
                .get((file - self.base_files) as usize)
                .copied()
        }
    }

    /// 有覆盖层数据的文件数（含已清空的）。
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// 基础索引里的这个文件已被覆盖层取代。
    pub fn shadows(&self, base_file: u32) -> bool {
        self.base_slot
            .get(base_file as usize)
            .is_some_and(|&s| s != NO_FILE)
    }

    /// 覆盖层里某个路径的最新内容哈希；`Some(None)` 表示已清空（删除）。
    pub fn file_hash(&self, path: &str) -> Option<Option<u64>> {
        let f = &self.files[*self.by_path.get(path)?];
        Some((!f.deleted).then_some(f.hash))
    }

    /// 覆盖层里的路径（含已清空的）。
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|f| f.path.as_str())
    }
}

/// “基础索引 + 覆盖层”的查询视图。没有覆盖层时等同于基础索引。符号与文件的序号是合并后的：
/// 基础序号不变，覆盖层新增的排在基础之后。
#[derive(Clone, Copy)]
pub struct L2View<'a> {
    pub base: &'a CsIndex,
    pub overlay: Option<&'a Overlay>,
}

impl<'a> L2View<'a> {
    pub fn new(base: &'a CsIndex, overlay: Option<&'a Overlay>) -> Self {
        Self { base, overlay }
    }

    fn file_slot(&self, file: u32) -> Option<&'a OvFile> {
        let ov = self.overlay?;
        ov.slot(file).map(|s| &ov.files[s])
    }

    /// 文件是否被覆盖层取代（基础索引里它的出现位置不再有效）。
    fn shadowed(&self, base_file: u32) -> bool {
        self.overlay.is_some_and(|o| o.shadows(base_file))
    }

    pub fn file_id(&self, rel: &str) -> Option<u32> {
        if let Some(ov) = self.overlay {
            if let Some(&slot) = ov.by_path.get(rel) {
                if ov.files[slot].deleted {
                    return None;
                }
                return Some(ov.merged_file(slot));
            }
        }
        self.base.file_id(rel)
    }

    pub fn file_path(&self, file: u32) -> Option<&'a str> {
        match self.file_slot(file) {
            Some(f) => (!f.deleted).then_some(f.path.as_str()),
            None => self.base.file_path(file),
        }
    }

    /// 合并后的文件号上限（基础文件数 + 新文件数）；其中已清空的文件 `file_path` 返回 `None`。
    pub fn file_count(&self) -> u32 {
        self.base.files().len() as u32 + self.overlay.map_or(0, |o| o.new_files.len() as u32)
    }

    /// 文件内的全部出现位置（按行列排序）。
    pub fn file_occurrences(&self, file: u32) -> Vec<Occurrence> {
        match self.file_slot(file) {
            Some(f) => f.occurrences.clone(),
            None => self
                .base
                .file_occurrences(file)
                .filter_map(|i| self.base.occurrence(i))
                .collect(),
        }
    }

    /// 符号键的文本形式（一致性检查用）：全局键原样，局部键写成 `路径@偏移`。
    pub fn key_text(&self, symbol: u32) -> Option<String> {
        if let Some(r) = self.record(symbol) {
            return Some(match &r.key {
                OvKey::Global(g) => g.clone(),
                OvKey::Local { path, offset } => format!("{path}@{offset}"),
            });
        }
        Some(match self.base.symbol_key(symbol).ok()? {
            SymbolKey::Global(g) => g.to_owned(),
            SymbolKey::Local { file, offset } => {
                format!("{}@{}", self.base.file_path(file)?, offset.0)
            }
        })
    }

    /// 文件在索引中的版本（源文件原始字节的 xxHash64）：#38 据此判断缓冲区与索引是否一致。
    pub fn file_hash(&self, file: u32) -> Option<u64> {
        match self.file_slot(file) {
            Some(f) => (!f.deleted).then_some(f.hash),
            None => self.base.files().get(file as usize).map(|f| f.hash),
        }
    }

    pub fn inactive_lines(&self, file: u32) -> &'a [InactiveLines] {
        match self.file_slot(file) {
            Some(f) => &f.inactive,
            None => self.base.inactive_lines(file),
        }
    }

    /// 查询点所在的出现位置（文件内二分查找）。
    pub fn occurrence_at(&self, file: u32, line: u32, col: Utf8Column) -> Option<Occurrence> {
        let Some(f) = self.file_slot(file) else {
            return self
                .base
                .occurrence_at(file, line, col)
                .and_then(|i| self.base.occurrence(i));
        };
        let n = f
            .occurrences
            .partition_point(|o| (o.line, o.col.0) <= (line, col.0));
        let o = f.occurrences.get(n.checked_sub(1)?)?;
        (o.line == line && col.0.checked_sub(o.col.0)? < u64::from(o.len)).then_some(*o)
    }

    fn record(&self, symbol: u32) -> Option<&'a OvRecord> {
        let ov = self.overlay?;
        if symbol >= ov.base_symbols {
            ov.extra.get((symbol - ov.base_symbols) as usize)
        } else {
            ov.records.get(&symbol)
        }
    }

    pub fn symbol_count(&self) -> u32 {
        self.base.symbols().len() as u32 + self.overlay.map_or(0, |o| o.extra.len() as u32)
    }

    pub fn kind(&self, symbol: u32) -> Option<u8> {
        match self.record(symbol) {
            Some(r) => Some(r.kind),
            None => self.base.symbols().get(symbol as usize).map(|s| s.kind),
        }
    }

    pub fn display(&self, symbol: u32) -> Option<&'a str> {
        match self.record(symbol) {
            Some(r) => Some(&r.display),
            None => {
                let s = self.base.symbols().get(symbol as usize)?;
                self.base.str(s.display).ok()
            }
        }
    }

    /// 全局符号键（局部符号返回 `None`）。
    pub fn global_key(&self, symbol: u32) -> Option<&'a str> {
        if let Some(r) = self.record(symbol) {
            return match &r.key {
                OvKey::Global(g) => Some(g),
                OvKey::Local { .. } => None,
            };
        }
        match self.base.symbol_key(symbol).ok()? {
            SymbolKey::Global(g) => Some(g),
            SymbolKey::Local { .. } => None,
        }
    }

    pub fn container(&self, symbol: u32) -> Option<u32> {
        match self.record(symbol) {
            Some(r) => r.container,
            None => self.base.container(symbol),
        }
    }

    /// 外部符号的声明签名（元数据视图，只在基础索引里有）。
    pub fn signature(&self, symbol: u32) -> Option<&'a str> {
        self.base.signature(symbol)
    }

    pub fn xml_doc(&self, symbol: u32) -> Option<String> {
        self.base.xml_doc(symbol)
    }

    /// 基础索引里的出现位置（定义在前），去掉被覆盖层取代的文件。
    fn base_occurrences(
        &self,
        symbol: u32,
        defs_only: bool,
    ) -> impl Iterator<Item = Occurrence> + 'a {
        let view = *self;
        let in_base = symbol < self.base.symbols().len() as u32;
        let definitions = if in_base {
            self.base.definition_count(symbol)
        } else {
            0
        };
        self.base
            .symbol_occurrences(if in_base { symbol } else { u32::MAX })
            .enumerate()
            .take_while(move |(i, o)| {
                !defs_only
                    || (Role(o.role).is_def()
                        && (definitions == u16::MAX || *i < usize::from(definitions)))
            })
            .map(|(_, o)| o)
            .filter(move |o| !view.shadowed(o.file))
    }

    fn overlay_occurrences(&self, symbol: u32) -> impl Iterator<Item = Occurrence> + 'a {
        let ov = self.overlay;
        ov.and_then(|ov| ov.by_symbol.get(&symbol))
            .into_iter()
            .flatten()
            .filter_map(move |&(f, i)| {
                let ov = ov?;
                Some(ov.files[ov.slot(f)?].occurrences[i as usize])
            })
    }

    /// 符号的定义位置（基础索引中未被取代的 + 覆盖层），按路径序。
    pub fn definitions(&self, symbol: u32) -> Vec<Occurrence> {
        let base: Vec<Occurrence> = self.base_occurrences(symbol, true).collect();
        let extra: Vec<Occurrence> = self
            .overlay_occurrences(symbol)
            .take_while(|o| Role(o.role).is_def())
            .collect();
        self.merge(base, extra)
    }

    /// 符号的全部出现位置：定义在前、引用在后，两组内按路径序与行列。
    pub fn occurrences(&self, symbol: u32) -> Vec<Occurrence> {
        if self.overlay.is_none() {
            return self.base.symbol_occurrences(symbol).collect();
        }
        let (base_defs, base_refs): (Vec<_>, Vec<_>) = self
            .base_occurrences(symbol, false)
            .partition(|o| Role(o.role).is_def());
        let (ov_defs, ov_refs): (Vec<_>, Vec<_>) = self
            .overlay_occurrences(symbol)
            .partition(|o| Role(o.role).is_def());
        let mut out = self.merge(base_defs, ov_defs);
        out.extend(self.merge(base_refs, ov_refs));
        out
    }

    fn order(&self, file: u32) -> (u32, u8, u32) {
        self.overlay.map_or((file, 1, 0), |o| o.order(file))
    }

    /// 合并两组各自按（路径序, 行, 列）排好的出现位置。
    fn merge(&self, a: Vec<Occurrence>, b: Vec<Occurrence>) -> Vec<Occurrence> {
        if b.is_empty() {
            return a;
        }
        let key = |o: &Occurrence| (self.order(o.file), o.line, o.col.0);
        let mut out = Vec::with_capacity(a.len() + b.len());
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            if key(&a[i]) <= key(&b[j]) {
                out.push(a[i]);
                i += 1;
            } else {
                out.push(b[j]);
                j += 1;
            }
        }
        out.extend_from_slice(&a[i..]);
        out.extend_from_slice(&b[j..]);
        out
    }

    /// 基础索引里还有未被取代的定义。
    fn alive_in_base(&self, symbol: u32) -> bool {
        self.base_occurrences(symbol, true).next().is_some()
    }

    /// 基础索引里 `from` 的这条关系是否仍然有效（没有被覆盖层重算、声明仍在）。
    fn base_relation_valid(&self, from: u32, kind: u32) -> bool {
        let Some(ov) = self.overlay else {
            return true;
        };
        if kind == REL_IMPLEMENTS {
            match self.base.container(from) {
                Some(t) => !ov.members_owned.contains(&t) && self.alive_in_base(t),
                None => self.alive_in_base(from),
            }
        } else {
            !ov.self_owned.contains(&from) && self.alive_in_base(from)
        }
    }

    /// 以 `symbol` 为 to 的关系（实现、重写、派生）。
    pub fn implementations(&self, symbol: u32) -> Vec<Relation> {
        let mut out: Vec<Relation> = Vec::new();
        if symbol < self.base.symbols().len() as u32 {
            out.extend(
                self.base
                    .implementations(symbol)
                    .filter(|r| self.base_relation_valid(r.from, r.kind))
                    .copied(),
            );
        }
        if let Some(list) = self.overlay.and_then(|o| o.rel_to.get(&symbol)) {
            out.extend(list.iter().map(|&(from, kind)| Relation {
                from,
                to: symbol,
                kind,
            }));
        }
        out
    }

    /// 以 `symbol` 为 from 的关系（基类型、实现的接口成员、重写的成员）。
    pub fn bases(&self, symbol: u32) -> Vec<Relation> {
        let mut out: Vec<Relation> = Vec::new();
        if symbol < self.base.symbols().len() as u32 {
            out.extend(
                self.base
                    .bases(symbol)
                    .filter(|r| self.base_relation_valid(r.from, r.kind))
                    .copied(),
            );
        }
        if let Some(list) = self.overlay.and_then(|o| o.rel_from.get(&symbol)) {
            out.extend(list.iter().map(|&(to, kind)| Relation {
                from: symbol,
                to,
                kind,
            }));
        }
        out
    }
}

#[cfg(test)]
#[path = "overlay_tests.rs"]
mod tests;
