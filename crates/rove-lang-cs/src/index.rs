use bytemuck::{Pod, Zeroable};
use memmap2::Mmap;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(not(target_endian = "little"))]
compile_error!("RVCSIDX requires a little-endian target");

const MAGIC: &[u8; 8] = b"RVCSIDX\0";
const VERSION: u32 = 2;
const TABLE_OFFSET: usize = 96;
const NONE: u32 = u32::MAX;
const LOCAL_BIT: u32 = 1 << 31;

#[repr(usize)]
#[derive(Clone, Copy)]
enum Section {
    StringOffsets,
    Files,
    Paths,
    Symbols,
    LocalKeys,
    Occurrences,
    WideOccurrences,
    References,
    RelationsFrom,
    RelationsTo,
    Inactive,
    Signatures,
    XmlDocs,
    MetadataViews,
    Strings,
}

const SECTION_COUNT: usize = Section::Strings as usize + 1;
const HEADER_LEN: usize = TABLE_OFFSET + SECTION_COUNT * 16;
const SECTION_SIZES: [usize; SECTION_COUNT] = [4, 24, 1, 24, 8, 8, 24, 1, 12, 12, 8, 8, 8, 1, 1];

pub use rove_base::{ByteOffset, Utf8Column};

#[cfg(test)]
use rove_base::Utf16Column;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKey<'a> {
    Global(&'a str),
    Local { file: u32, offset: ByteOffset },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexHeader {
    pub format_version: u32,
    pub indexer_version: [u16; 4],
    pub project_config_hash: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct FileRecord {
    path_start: u32,
    pub occ_start: u32,
    pub occ_count: u32,
    inactive_start: u32,
    pub hash: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct SymbolRecord {
    key: u32,
    pub name: u32,
    pub display: u32,
    pub container: u32,
    pub ref_start: u32,
    pub kind: u8,
    pub flags: u8,
    definition_count: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occurrence {
    pub file: u32,
    pub line: u32,
    pub col: Utf8Column,
    pub symbol: u32,
    pub len: u32,
    pub role: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WideOccurrence {
    file: u32,
    line: u32,
    col: u32,
    symbol: u32,
    len: u32,
    role: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Relation {
    pub from: u32,
    pub to: u32,
    pub kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq, Eq)]
pub struct InactiveLines {
    pub start: u32,
    pub end: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq, Eq)]
pub struct SymbolText {
    pub symbol: u32,
    pub text: u32,
}

/// 元数据视图记录（段 13，#34）：一个最外层外部类型的视图正文与其成员位置区间。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq, Eq)]
pub struct MetadataViewRecord {
    pub type_symbol: u32,
    pub text: u32,
    member_start: u32,
    member_count: u32,
}

/// 视图内成员（含类型自身与嵌套类型）的名字位置，相对于视图正文，列与长度为 UTF-8 字节。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq, Eq)]
pub struct MetadataMember {
    pub symbol: u32,
    pub line: u32,
    pub col: u32,
    pub len: u32,
}

/// 外部符号的定义位置：视图序号 + 视图内位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetadataLocation {
    pub view: u32,
    pub line: u32,
    pub col: Utf8Column,
    pub len: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Role(pub u16);

impl Role {
    pub const REF: u16 = 1;
    pub const DEF: u16 = 2;
    pub const CANDIDATE: u16 = 4;

    pub fn is_def(self) -> bool {
        self.0 & Self::DEF != 0
    }
}

pub const REL_BASE_TYPE: u32 = 1;
pub const REL_IMPLEMENTS: u32 = 2;
pub const REL_OVERRIDES: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location<'a> {
    pub path: &'a str,
    pub line: u32,
    pub col: Utf8Column,
    pub len: u32,
    pub is_def: bool,
}

pub struct CsIndex {
    mmap: Mmap,
    sections: [Range<usize>; SECTION_COUNT],
    header: IndexHeader,
    ref_width: usize,
    path_spans: Vec<(u32, u32)>,
    string_reads: AtomicUsize,
    /// 段 13 中视图记录与成员记录的字节区间（打开时校验）。
    view_records: Range<usize>,
    view_members: Range<usize>,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}

fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().unwrap())
}

fn reference_width(occurrence_count: usize) -> usize {
    if occurrence_count <= 1 << 24 {
        3
    } else {
        4
    }
}

fn read_reference_id(id: &[u8], width: usize) -> u32 {
    u32::from_le_bytes([id[0], id[1], id[2], if width == 4 { id[3] } else { 0 }])
}

fn header_checksum(header: &[u8]) -> u64 {
    header[..88]
        .iter()
        .chain(&header[TABLE_OFFSET..])
        .fold(14695981039346656037, |hash, &b| {
            (hash ^ u64::from(b)).wrapping_mul(1099511628211)
        })
}

impl CsIndex {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        // SAFETY: The writer publishes complete snapshots by atomic replacement. An open
        // snapshot must not be modified or truncated in place while it is being read.
        let mmap = unsafe { Mmap::map(&file)? };
        if mmap.len() < 12 || &mmap[..8] != MAGIC {
            return Err(invalid("not a Rove C# index"));
        }
        let version = read_u32(&mmap[8..12]);
        if version != VERSION {
            return Err(invalid(&format!(
                "unsupported C# index version {version} (expected {VERSION})"
            )));
        }
        let header = mmap
            .get(..HEADER_LEN)
            .ok_or_else(|| invalid("truncated index header"))?;
        if read_u32(&header[12..16]) as usize != SECTION_COUNT
            || read_u64(&header[16..24]) != mmap.len() as u64
            || header[64..88] != [0; 24]
            || read_u64(&header[88..96]) != header_checksum(header)
        {
            return Err(invalid("invalid C# index header"));
        }

        let mut sections = std::array::from_fn(|_| 0..0);
        let mut previous_end = HEADER_LEN;
        for (i, range) in sections.iter_mut().enumerate() {
            let at = TABLE_OFFSET + i * 16;
            let off = usize::try_from(read_u64(&header[at..at + 8]))
                .map_err(|_| invalid("section offset overflow"))?;
            let count = usize::try_from(read_u64(&header[at + 8..at + 16]))
                .map_err(|_| invalid("section count overflow"))?;
            let end = count
                .checked_mul(SECTION_SIZES[i])
                .and_then(|n| off.checked_add(n))
                .ok_or_else(|| invalid("section size overflow"))?;
            if off % 8 != 0 || off < previous_end || end > mmap.len() || count > u32::MAX as usize {
                return Err(invalid("misaligned, overlapping or out-of-bounds section"));
            }
            *range = off..end;
            previous_end = end;
        }
        if (previous_end
            .checked_add(7)
            .ok_or_else(|| invalid("file length overflow"))?
            & !7)
            != mmap.len()
        {
            return Err(invalid("unexpected data after final section"));
        }
        let occurrence_count = sections[Section::Occurrences as usize].len() / 8;
        let ref_width = reference_width(occurrence_count);
        if occurrence_count.checked_mul(ref_width)
            != Some(sections[Section::References as usize].len())
        {
            return Err(invalid("reference count does not match occurrence count"));
        }

        let index_header = IndexHeader {
            format_version: version,
            indexer_version: std::array::from_fn(|i| {
                let at = 24 + i * 2;
                u16::from_le_bytes(header[at..at + 2].try_into().unwrap())
            }),
            project_config_hash: header[32..64].try_into().unwrap(),
        };
        let mut index = Self {
            mmap,
            sections,
            header: index_header,
            ref_width,
            path_spans: Vec::new(),
            string_reads: AtomicUsize::new(0),
            view_records: 0..0,
            view_members: 0..0,
        };
        index.path_spans = index.validate_directories()?;
        (index.view_records, index.view_members) = index.validate_metadata_views()?;
        Ok(index)
    }

    fn bytes(&self, section: Section) -> &[u8] {
        &self.mmap[self.sections[section as usize].clone()]
    }

    fn section<T: Pod>(&self, section: Section) -> &[T] {
        bytemuck::cast_slice(self.bytes(section))
    }

    fn validate_directories(&self) -> io::Result<Vec<(u32, u32)>> {
        // Read directory offsets only. Neither string nor path payload pages are needed.
        let offsets: &[u32] = self.section(Section::StringOffsets);
        if offsets.first() != Some(&0)
            || offsets.last().copied().map(|n| n as usize)
                != Some(self.sections[Section::Strings as usize].len())
            || offsets.windows(2).any(|w| w[0] > w[1])
        {
            return Err(invalid("invalid string offsets"));
        }
        let mut path_spans = Vec::with_capacity(self.files().len());
        let mut occ_start = 0_u32;
        let mut prev_path = None;
        let mut inactive = 0;
        let path_len = self.sections[Section::Paths as usize].len();
        let inactive_count = self.sections[Section::Inactive as usize].len() / 8;
        for file in self.files() {
            let path = file.path_start as usize;
            if file.occ_start != occ_start
                || path >= path_len
                || prev_path.map_or(path != 0, |prev| prev >= path)
                || file.inactive_start < inactive
                || file.inactive_start as usize > inactive_count
                || (prev_path.is_none() && file.inactive_start != 0)
            {
                return Err(invalid("invalid file directory"));
            }
            if let Some(start) = prev_path {
                path_spans.push((start as u32, file.path_start));
            }
            prev_path = Some(path);
            inactive = file.inactive_start;
            occ_start = occ_start
                .checked_add(file.occ_count)
                .ok_or_else(|| invalid("occurrence range overflow"))?;
        }
        if occ_start as usize != self.occurrence_count()
            || (self.files().is_empty() && (path_len != 0 || inactive_count != 0))
        {
            return Err(invalid("invalid file directory totals"));
        }
        if let Some(start) = prev_path {
            path_spans.push((start as u32, path_len as u32));
        }
        Ok(path_spans)
    }

    /// 段 13：空，或 view_count、member_count 两个 u32 后接定长记录。视图按 type_symbol 排序，
    /// 成员区间不越界。符号与字符串 ID 在访问时检查。
    fn validate_metadata_views(&self) -> io::Result<(Range<usize>, Range<usize>)> {
        let range = self.sections[Section::MetadataViews as usize].clone();
        let bytes = &self.mmap[range.clone()];
        if bytes.is_empty() {
            return Ok((0..0, 0..0));
        }
        let bad = || invalid("invalid metadata view section");
        if bytes.len() < 8 {
            return Err(bad());
        }
        let views = read_u32(&bytes[..4]) as usize;
        let members = read_u32(&bytes[4..8]) as usize;
        let views_end = views
            .checked_mul(16)
            .and_then(|n| n.checked_add(8))
            .ok_or_else(bad)?;
        let members_end = members
            .checked_mul(16)
            .and_then(|n| n.checked_add(views_end))
            .ok_or_else(bad)?;
        if members_end != bytes.len() {
            return Err(bad());
        }
        let records: &[MetadataViewRecord] = bytemuck::cast_slice(&bytes[8..views_end]);
        let mut previous = 0;
        for r in records {
            let end = (r.member_start as usize).checked_add(r.member_count as usize);
            if r.type_symbol < previous || end.is_none_or(|end| end > members) {
                return Err(bad());
            }
            previous = r.type_symbol;
        }
        Ok((
            range.start + 8..range.start + views_end,
            range.start + views_end..range.end,
        ))
    }

    pub fn header(&self) -> &IndexHeader {
        &self.header
    }

    pub fn string_payload_reads(&self) -> usize {
        self.string_reads.load(Ordering::Relaxed)
    }

    pub fn str(&self, id: u32) -> io::Result<&str> {
        let offsets: &[u32] = self.section(Section::StringOffsets);
        let i = id as usize;
        let next = i
            .checked_add(1)
            .ok_or_else(|| invalid("invalid string ID"))?;
        let end = *offsets
            .get(next)
            .ok_or_else(|| invalid("invalid string ID"))? as usize;
        let start = offsets[i] as usize;
        self.string_reads.fetch_add(1, Ordering::Relaxed);
        std::str::from_utf8(&self.bytes(Section::Strings)[start..end])
            .map_err(|_| invalid("invalid UTF-8 in string payload"))
    }

    pub fn files(&self) -> &[FileRecord] {
        self.section(Section::Files)
    }

    pub fn symbols(&self) -> &[SymbolRecord] {
        self.section(Section::Symbols)
    }

    pub fn symbol_key(&self, symbol: u32) -> io::Result<SymbolKey<'_>> {
        let s = self
            .symbols()
            .get(symbol as usize)
            .ok_or_else(|| invalid("invalid symbol ID"))?;
        if s.key & LOCAL_BIT == 0 {
            return self.str(s.key).map(SymbolKey::Global);
        }
        let keys: &[[u32; 2]] = self.section(Section::LocalKeys);
        let &[file, offset] = keys
            .get((s.key & !LOCAL_BIT) as usize)
            .ok_or_else(|| invalid("invalid local key ID"))?;
        if file as usize >= self.files().len() {
            return Err(invalid("invalid local key file ID"));
        }
        Ok(SymbolKey::Local {
            file,
            offset: ByteOffset(u64::from(offset)),
        })
    }

    pub fn signatures(&self) -> &[SymbolText] {
        self.section(Section::Signatures)
    }

    pub fn xml_docs(&self) -> &[SymbolText] {
        self.section(Section::XmlDocs)
    }

    pub fn metadata_views(&self) -> &[u8] {
        self.bytes(Section::MetadataViews)
    }

    pub fn metadata_view_records(&self) -> &[MetadataViewRecord] {
        bytemuck::cast_slice(&self.mmap[self.view_records.clone()])
    }

    /// 视图正文（含 `#region` 文件头），按 C# 文件显示。
    pub fn metadata_view_text(&self, view: u32) -> io::Result<&str> {
        let record = self
            .metadata_view_records()
            .get(view as usize)
            .ok_or_else(|| invalid("invalid metadata view ID"))?;
        self.str(record.text)
    }

    /// 视图内有位置记录的成员，按 (symbol, line, col) 排序。
    pub fn metadata_view_members(&self, view: u32) -> &[MetadataMember] {
        let members: &[MetadataMember] =
            bytemuck::cast_slice(&self.mmap[self.view_members.clone()]);
        self.metadata_view_records()
            .get(view as usize)
            .and_then(|r| {
                members.get(r.member_start as usize..(r.member_start + r.member_count) as usize)
            })
            .unwrap_or_default()
    }

    /// 某个最外层外部类型的视图序号区间。同名程序集的不同版本各有一份视图，所以可能多于一个。
    pub fn metadata_views_of(&self, type_symbol: u32) -> Range<u32> {
        let records = self.metadata_view_records();
        let start = records.partition_point(|r| r.type_symbol < type_symbol);
        let end = start + records[start..].partition_point(|r| r.type_symbol == type_symbol);
        start as u32..end as u32
    }

    /// 外部符号的定义位置：沿容器找到最外层类型，在它的视图里找该符号的位置。
    /// 源码符号或没有视图的外部符号返回空。
    pub fn metadata_definitions(&self, symbol: u32) -> Vec<MetadataLocation> {
        let mut outer = symbol;
        // 嵌套层数有限；上限只防御损坏数据中的环。
        for _ in 0..64 {
            match self.container(outer) {
                Some(parent) if parent != outer => outer = parent,
                _ => break,
            }
        }
        let mut result = Vec::new();
        for view in self.metadata_views_of(outer) {
            let members = self.metadata_view_members(view);
            let start = members.partition_point(|m| m.symbol < symbol);
            result.extend(
                members[start..]
                    .iter()
                    .take_while(|m| m.symbol == symbol)
                    .map(|m| MetadataLocation {
                        view,
                        line: m.line,
                        col: Utf8Column(u64::from(m.col)),
                        len: m.len,
                    }),
            );
        }
        result
    }

    fn symbol_text(&self, section: &[SymbolText], symbol: u32) -> Option<&str> {
        let i = section.partition_point(|t| t.symbol < symbol);
        section
            .get(i)
            .filter(|t| t.symbol == symbol)
            .and_then(|t| self.str(t.text).ok())
    }

    /// 外部符号的声明签名（与视图中该成员所在行相同，不含缩进）。
    pub fn signature(&self, symbol: u32) -> Option<&str> {
        self.symbol_text(self.signatures(), symbol)
    }

    /// 外部符号的原始 XML 文档文本（`<summary>`、`<see cref>` 等标签原样保留）。
    ///
    /// #34 的索引进程把文档只存一份：元数据视图里成员上方的 `///` 行，段 12 留空。这里先查段 12，
    /// 没有时从第一个视图中取成员上方连续的 `///` 行，去掉前缀后按行连接。
    pub fn xml_doc(&self, symbol: u32) -> Option<String> {
        if let Some(doc) = self.symbol_text(self.xml_docs(), symbol) {
            return Some(doc.to_owned());
        }
        let location = *self.metadata_definitions(symbol).first()?;
        let text = self.metadata_view_text(location.view).ok()?;
        let lines: Vec<&str> = text.lines().take(location.line as usize).collect();
        let start = lines
            .iter()
            .rposition(|l| !l.trim_start().starts_with("///"))
            .map_or(0, |i| i + 1);
        let doc: Vec<&str> = lines[start..]
            .iter()
            .map(|l| {
                let l = l.trim_start();
                l.strip_prefix("/// ")
                    .or_else(|| l.strip_prefix("///"))
                    .unwrap_or(l)
            })
            .collect();
        (!doc.is_empty()).then(|| doc.join("\n"))
    }

    pub fn inactive_lines(&self, file: u32) -> &[InactiveLines] {
        let files = self.files();
        let ranges: &[InactiveLines] = self.section(Section::Inactive);
        let i = file as usize;
        let Some(record) = files.get(i) else {
            return &[];
        };
        let end = files
            .get(i + 1)
            .map_or(ranges.len(), |next| next.inactive_start as usize);
        &ranges[record.inactive_start as usize..end]
    }

    pub fn file_path(&self, file: u32) -> Option<&str> {
        let &(start, end) = self.path_spans.get(file as usize)?;
        std::str::from_utf8(&self.bytes(Section::Paths)[start as usize..end as usize]).ok()
    }

    pub fn file_id(&self, rel_path: &str) -> Option<u32> {
        let paths = self.bytes(Section::Paths);
        let needle = rel_path.as_bytes();
        self.path_spans
            .binary_search_by(|&(start, end)| paths[start as usize..end as usize].cmp(needle))
            .ok()
            .map(|i| i as u32)
    }

    pub fn file_occurrences(&self, file: u32) -> Range<usize> {
        self.files().get(file as usize).map_or(0..0, |f| {
            f.occ_start as usize..(f.occ_start + f.occ_count) as usize
        })
    }

    fn packed_occurrences(&self) -> &[u64] {
        self.section(Section::Occurrences)
    }

    pub fn occurrence_count(&self) -> usize {
        self.packed_occurrences().len()
    }

    fn wide_occurrence(&self, packed: u64) -> Option<&WideOccurrence> {
        let wide: &[WideOccurrence] = self.section(Section::WideOccurrences);
        wide.get(usize::try_from(packed >> 3).ok()?)
    }

    fn decode_occurrence(&self, packed: u64) -> Option<Occurrence> {
        let role = (packed & 7) as u16;
        if role != 0 {
            return Some(Occurrence {
                file: (packed >> 49) as u32,
                line: ((packed >> 16) & 0xfff) as u32,
                col: Utf8Column((packed >> 8) & 0xff),
                symbol: ((packed >> 28) & 0x1f_ffff) as u32,
                len: ((packed >> 3) & 0x1f) as u32,
                role,
            });
        }
        let wide = self.wide_occurrence(packed)?;
        Some(Occurrence {
            file: wide.file,
            line: wide.line,
            col: Utf8Column(u64::from(wide.col)),
            symbol: wide.symbol,
            len: wide.len,
            role: wide.role as u16,
        })
    }

    pub fn occurrence(&self, i: usize) -> Option<Occurrence> {
        self.decode_occurrence(*self.packed_occurrences().get(i)?)
    }

    pub fn occurrence_at(&self, file: u32, line: u32, col: Utf8Column) -> Option<usize> {
        self.find_occurrence(file, line, col)
            .map(|(index, _)| index)
    }

    fn find_occurrence(
        &self,
        file: u32,
        line: u32,
        col: Utf8Column,
    ) -> Option<(usize, Occurrence)> {
        let stored_column = u32::try_from(col.0).ok()?;
        let range = self.file_occurrences(file);
        let occurrences = self.packed_occurrences().get(range.clone())?;
        let n = occurrences.partition_point(|&packed| {
            let pos = if packed & 7 == 0 {
                let Some(wide) = self.wide_occurrence(packed) else {
                    return false;
                };
                (wide.line, wide.col)
            } else {
                (
                    ((packed >> 16) & 0xfff) as u32,
                    ((packed >> 8) & 0xff) as u32,
                )
            };
            pos <= (line, stored_column)
        });
        let index = n.checked_sub(1)?;
        let o = self.decode_occurrence(occurrences[index])?;
        (o.file == file && o.line == line && col.0.checked_sub(o.col.0)? < u64::from(o.len))
            .then_some((range.start + index, o))
    }

    fn symbol_refs(&self, symbol: u32) -> &[u8] {
        let symbols = self.symbols();
        let i = symbol as usize;
        let Some(s) = symbols.get(i) else { return &[] };
        let end = symbols
            .get(i + 1)
            .map_or(self.occurrence_count(), |next| next.ref_start as usize);
        let bytes = self.bytes(Section::References);
        (s.ref_start as usize)
            .checked_mul(self.ref_width)
            .zip(end.checked_mul(self.ref_width))
            .and_then(|(a, b)| bytes.get(a..b))
            .unwrap_or_default()
    }

    pub fn reference_count(&self, symbol: u32) -> usize {
        self.symbol_refs(symbol).len() / self.ref_width
    }

    fn decode_refs<'a>(
        &'a self,
        refs: &'a [u8],
        symbol: u32,
    ) -> impl Iterator<Item = Occurrence> + 'a {
        let occurrences = self.packed_occurrences();
        refs.chunks_exact(self.ref_width).filter_map(move |id| {
            let i = read_reference_id(id, self.ref_width);
            self.decode_occurrence(*occurrences.get(i as usize)?)
                .filter(|o| o.symbol == symbol)
        })
    }

    pub fn symbol_occurrences(&self, symbol: u32) -> impl Iterator<Item = Occurrence> + '_ {
        self.decode_refs(self.symbol_refs(symbol), symbol)
    }

    pub fn location(&self, o: &Occurrence) -> Option<Location<'_>> {
        Some(Self::location_on(self.file_path(o.file)?, o))
    }

    fn location_on<'a>(path: &'a str, o: &Occurrence) -> Location<'a> {
        Location {
            path,
            line: o.line,
            col: o.col,
            len: o.len,
            is_def: Role(o.role).is_def(),
        }
    }

    pub fn symbol_at(&self, rel_path: &str, line: u32, col: Utf8Column) -> Option<u32> {
        let f = self.file_id(rel_path)?;
        let (_, occurrence) = self.find_occurrence(f, line, col)?;
        let symbol = occurrence.symbol;
        self.symbols().get(symbol as usize)?;
        Some(symbol)
    }

    pub fn goto_definition(&self, rel_path: &str, line: u32, col: Utf8Column) -> Vec<Location<'_>> {
        let Some(sym) = self.symbol_at(rel_path, line, col) else {
            return Vec::new();
        };
        let definitions = self.symbols()[sym as usize].definition_count;
        if definitions == 0 {
            return Vec::new();
        }

        let refs = self.symbol_refs(sym);
        if definitions == u16::MAX {
            return self
                .decode_refs(refs, sym)
                .take_while(|o| Role(o.role).is_def())
                .filter_map(|o| self.location(&o))
                .collect();
        }

        let Some(prefix) = refs.get(..usize::from(definitions) * self.ref_width) else {
            return Vec::new();
        };
        let mut locations = Vec::with_capacity(usize::from(definitions));
        locations.extend(
            self.decode_refs(prefix, sym)
                .take_while(|o| Role(o.role).is_def())
                .filter_map(|o| self.location(&o)),
        );
        locations
    }

    pub fn find_references(&self, rel_path: &str, line: u32, col: Utf8Column) -> Vec<Location<'_>> {
        let Some(sym) = self.symbol_at(rel_path, line, col) else {
            return Vec::new();
        };
        // Positions are ordered by file within definitions and within references. Reuse
        // the borrowed UTF-8 path across adjacent results for the same file.
        let refs = self.symbol_refs(sym);
        let mut result = Vec::with_capacity(refs.len() / self.ref_width);
        let mut last = (NONE, None);
        for o in self.decode_refs(refs, sym) {
            if o.file != last.0 {
                last = (o.file, self.file_path(o.file));
            }
            if let Some(path) = last.1 {
                result.push(Self::location_on(path, &o));
            }
        }
        result
    }

    pub fn implementations(&self, symbol: u32) -> impl Iterator<Item = &Relation> + '_ {
        let rels: &[Relation] = self.section(Section::RelationsTo);
        let start = rels.partition_point(|r| r.to < symbol);
        rels[start..].iter().take_while(move |r| r.to == symbol)
    }

    pub fn bases(&self, symbol: u32) -> impl Iterator<Item = &Relation> + '_ {
        let rels: &[Relation] = self.section(Section::RelationsFrom);
        let start = rels.partition_point(|r| r.from < symbol);
        rels[start..].iter().take_while(move |r| r.from == symbol)
    }

    pub fn container(&self, symbol: u32) -> Option<u32> {
        let container = self.symbols().get(symbol as usize)?.container;
        self.symbols().get(container as usize).map(|_| container)
    }

    pub fn byte_len(&self) -> usize {
        self.mmap.len()
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
