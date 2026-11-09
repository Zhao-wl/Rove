use bytemuck::{Pod, Zeroable};
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;

const MAGIC: &[u8; 8] = b"RVCSIDX\0";
const VERSION: u32 = 1;
const SECTION_COUNT: usize = 9;
const HEADER_LEN: usize = 16 + SECTION_COUNT * 16;
const NONE: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct FileRecord {
    pub path: u32,
    pub occ_start: u32,
    pub occ_count: u32,
    _pad: u32,
    pub hash: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct SymbolRecord {
    pub key: u32,
    pub name: u32,
    pub display: u32,
    pub container: u32,
    pub ref_start: u32,
    pub ref_count: u32,
    pub kind: u8,
    pub flags: u8,
    _pad: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Occurrence {
    pub file: u32,
    pub line: u32,
    /// UTF-16 列号（Roslyn 坐标）
    pub col: u32,
    pub symbol: u32,
    pub len: u16,
    pub role: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Relation {
    pub from: u32,
    pub to: u32,
    pub kind: u32,
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
    pub col: u32,
    pub len: u16,
    pub is_def: bool,
}

pub struct CsIndex {
    mmap: Mmap,
    sections: [(usize, usize); SECTION_COUNT],
    path_to_file: HashMap<Box<str>, u32>,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

impl CsIndex {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        // SAFETY: 索引文件由索引进程原子替换写入，打开后不会被原地修改。
        let mmap = unsafe { Mmap::map(&file)? };
        if mmap.len() < HEADER_LEN || &mmap[..8] != MAGIC {
            return Err(invalid("not a Rove C# index"));
        }
        let version = u32::from_le_bytes(mmap[8..12].try_into().unwrap());
        if version != VERSION {
            return Err(invalid("unsupported index version"));
        }
        let mut sections = [(0usize, 0usize); SECTION_COUNT];
        for (i, s) in sections.iter_mut().enumerate() {
            let at = 16 + i * 16;
            let off = u64::from_le_bytes(mmap[at..at + 8].try_into().unwrap()) as usize;
            let count = u64::from_le_bytes(mmap[at + 8..at + 16].try_into().unwrap()) as usize;
            *s = (off, count);
        }
        let mut idx = CsIndex {
            mmap,
            sections,
            path_to_file: HashMap::new(),
        };
        idx.validate()?;
        let map = idx
            .files()
            .iter()
            .enumerate()
            .map(|(i, f)| (Box::from(idx.str(f.path)), i as u32))
            .collect();
        idx.path_to_file = map;
        Ok(idx)
    }

    fn validate(&self) -> io::Result<()> {
        let sizes = [4, 1, 24, 28, 20, 4, 12, 12, 1];
        for (i, &(off, count)) in self.sections.iter().enumerate() {
            let end = off.checked_add(
                count
                    .checked_mul(sizes[i])
                    .ok_or_else(|| invalid("overflow"))?,
            );
            if end.is_none_or(|e| e > self.mmap.len()) {
                return Err(invalid("section out of bounds"));
            }
        }
        Ok(())
    }

    fn section<T: Pod>(&self, i: usize) -> &[T] {
        let (off, count) = self.sections[i];
        bytemuck::cast_slice(&self.mmap[off..off + count * std::mem::size_of::<T>()])
    }

    fn str_offsets(&self) -> &[u32] {
        self.section(0)
    }
    pub fn str(&self, id: u32) -> &str {
        let offs = self.str_offsets();
        let (blob_off, _) = self.sections[1];
        let a = offs[id as usize] as usize;
        let b = offs[id as usize + 1] as usize;
        std::str::from_utf8(&self.mmap[blob_off + a..blob_off + b]).unwrap_or("")
    }
    pub fn files(&self) -> &[FileRecord] {
        self.section(2)
    }
    pub fn symbols(&self) -> &[SymbolRecord] {
        self.section(3)
    }
    pub fn occurrences(&self) -> &[Occurrence] {
        self.section(4)
    }
    fn ref_index(&self) -> &[u32] {
        self.section(5)
    }
    fn rels_by_from(&self) -> &[Relation] {
        self.section(6)
    }
    fn rels_by_to(&self) -> &[Relation] {
        self.section(7)
    }

    pub fn file_id(&self, rel_path: &str) -> Option<u32> {
        self.path_to_file.get(rel_path).copied()
    }

    pub fn file_occurrences(&self, file: u32) -> Range<usize> {
        let f = &self.files()[file as usize];
        f.occ_start as usize..(f.occ_start + f.occ_count) as usize
    }

    /// 找到覆盖 (line, col) 的出现位置。出现位置在文件内按 (line, col) 排序。
    pub fn occurrence_at(&self, file: u32, line: u32, col: u32) -> Option<usize> {
        let range = self.file_occurrences(file);
        let occs = &self.occurrences()[range.clone()];
        // 最后一个 (line, col) <= 目标的出现位置
        let n = occs.partition_point(|o| (o.line, o.col) <= (line, col));
        if n == 0 {
            return None;
        }
        let o = &occs[n - 1];
        (o.line == line && col < o.col + o.len as u32).then_some(range.start + n - 1)
    }

    /// 某符号的全部出现位置（定义在前）。
    pub fn symbol_occurrences(&self, symbol: u32) -> impl Iterator<Item = &Occurrence> + '_ {
        let s = &self.symbols()[symbol as usize];
        let ids = &self.ref_index()[s.ref_start as usize..(s.ref_start + s.ref_count) as usize];
        let occs = self.occurrences();
        ids.iter().map(move |&i| &occs[i as usize])
    }

    pub fn location(&self, o: &Occurrence) -> Location<'_> {
        Location {
            path: self.str(self.files()[o.file as usize].path),
            line: o.line,
            col: o.col,
            len: o.len,
            is_def: Role(o.role).is_def(),
        }
    }

    pub fn symbol_at(&self, rel_path: &str, line: u32, col: u32) -> Option<u32> {
        let f = self.file_id(rel_path)?;
        let i = self.occurrence_at(f, line, col)?;
        Some(self.occurrences()[i].symbol)
    }

    pub fn goto_definition(&self, rel_path: &str, line: u32, col: u32) -> Vec<Location<'_>> {
        let Some(sym) = self.symbol_at(rel_path, line, col) else {
            return Vec::new();
        };
        self.symbol_occurrences(sym)
            .take_while(|o| Role(o.role).is_def())
            .map(|o| self.location(o))
            .collect()
    }

    pub fn find_references(&self, rel_path: &str, line: u32, col: u32) -> Vec<Location<'_>> {
        let Some(sym) = self.symbol_at(rel_path, line, col) else {
            return Vec::new();
        };
        self.symbol_occurrences(sym)
            .map(|o| self.location(o))
            .collect()
    }

    /// 实现 / 重写 / 派生：关系表中 to == symbol 的所有 from。
    pub fn implementations(&self, symbol: u32) -> impl Iterator<Item = &Relation> + '_ {
        let rels = self.rels_by_to();
        let start = rels.partition_point(|r| r.to < symbol);
        rels[start..].iter().take_while(move |r| r.to == symbol)
    }

    /// 基类型 / 被实现的接口成员 / 被重写的成员。
    pub fn bases(&self, symbol: u32) -> impl Iterator<Item = &Relation> + '_ {
        let rels = self.rels_by_from();
        let start = rels.partition_point(|r| r.from < symbol);
        rels[start..].iter().take_while(move |r| r.from == symbol)
    }

    pub fn container(&self, symbol: u32) -> Option<u32> {
        let c = self.symbols()[symbol as usize].container;
        (c != NONE).then_some(c)
    }

    pub fn byte_len(&self) -> usize {
        self.mmap.len()
    }
}
