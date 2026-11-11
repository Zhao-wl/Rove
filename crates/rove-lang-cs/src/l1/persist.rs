//! `l1-decls.bin`：L1 结果的持久化格式（小端）。
//!
//! ```text
//! magic "ROVEL1\0\0" | u32 版本
//! u32 字符串数 | { u32 长度, UTF-8 } ...            只写被引用的名字，加载即压实
//! u32 文件数   | 文件 ...
//!   路径(u32 长度 + UTF-8) | u64 长度 | u64 修改时间(ns) | u64 内容哈希
//!   u32 作用域数 | { u32 ns, u32 parent, 4×u32 range } ...
//!   u32 using 数 | { u32 scope, u8 kind, u8 global, u32 alias, u32 target } ...
//!   u32 声明数   | { u32 name, u8 kind, u8 arity, u32 parent, u32 scope, 4×u32 range, 4×u32 selection } ...
//! u64 校验（前面全部字节的 content_hash）
//! ```
//!
//! 任何不一致（版本、截断、校验、越界下标）都整体作废，调用方随后全量构建。
//! 提取逻辑变化时必须改 [`VERSION`]，否则旧缓存会带着旧结果被当成“未变化”。

use super::{
    extract::{Decl, Parsed, Range32, Scope, Using, NONE},
    names::{content_hash, FxMap, Name},
    DeclKind, Stamp, State, UsingKind,
};
use rove_base::WorkspacePath;
use std::sync::Arc;

const MAGIC: &[u8; 8] = b"ROVEL1\0\0";
pub(crate) const VERSION: u32 = 1;

pub(crate) struct LoadedFile {
    pub path: WorkspacePath,
    pub stamp: Stamp,
    pub hash: u64,
    pub parsed: Parsed<Arc<str>>,
}

struct Writer {
    out: Vec<u8>,
    remap: FxMap<Name, u32>,
    strings: Vec<Name>,
}

impl Writer {
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.out.extend_from_slice(s.as_bytes());
    }
    fn range(&mut self, r: Range32) {
        self.u32(r.start_line);
        self.u32(r.start_col);
        self.u32(r.end_line);
        self.u32(r.end_col);
    }
    fn name(&mut self, n: Name) -> u32 {
        let next = self.strings.len() as u32;
        *self.remap.entry(n).or_insert_with(|| {
            self.strings.push(n);
            next
        })
    }
}

pub(crate) fn encode(state: &State) -> Vec<u8> {
    // 先写文件段（同时收集被引用的名字），再把字符串表拼在前面。
    let mut w = Writer {
        out: Vec::new(),
        remap: FxMap::default(),
        strings: Vec::new(),
    };
    let files: Vec<_> = state.files.iter().flatten().collect();
    w.u32(files.len() as u32);
    for f in files {
        w.str(f.path.as_str());
        w.u64(f.stamp.len);
        w.u64(f.stamp.mtime_ns);
        w.u64(f.hash);
        w.u32(f.scopes.len() as u32);
        for s in f.scopes.iter() {
            let ns = w.name(s.ns);
            w.u32(ns);
            w.u32(s.parent);
            w.range(s.range);
        }
        w.u32(f.usings.len() as u32);
        for u in f.usings.iter() {
            w.u32(u.scope);
            w.u8(u.kind as u8);
            w.u8(u.global as u8);
            let (a, t) = (w.name(u.alias), w.name(u.target));
            w.u32(a);
            w.u32(t);
        }
        w.u32(f.decls.len() as u32);
        for d in f.decls.iter() {
            let n = w.name(d.name);
            w.u32(n);
            w.u8(d.kind as u8);
            w.u8(d.arity);
            w.u32(d.parent);
            w.u32(d.scope);
            w.range(d.range);
            w.range(d.selection);
        }
    }
    let body = std::mem::take(&mut w.out);
    let mut out = Vec::with_capacity(body.len() + 64 * 1024);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(w.strings.len() as u32).to_le_bytes());
    for n in &w.strings {
        let s = state.names.get(*n);
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    out.extend_from_slice(&body);
    let check = content_hash(&out);
    out.extend_from_slice(&check.to_le_bytes());
    out
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8)
            .map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")))
    }
    fn str(&mut self) -> Option<&'a str> {
        let n = self.u32()? as usize;
        std::str::from_utf8(self.take(n)?).ok()
    }
    fn range(&mut self) -> Option<Range32> {
        Some(Range32 {
            start_line: self.u32()?,
            start_col: self.u32()?,
            end_line: self.u32()?,
            end_col: self.u32()?,
        })
    }
    /// 元素个数：每个元素至少占 `min` 字节，防止损坏的计数导致巨量预分配。
    fn count(&mut self, min: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        (n.checked_mul(min)? <= self.buf.len() - self.pos).then_some(n)
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Option<Vec<LoadedFile>> {
    let (data, check) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
    if content_hash(data) != u64::from_le_bytes(check.try_into().ok()?) {
        return None;
    }
    let mut r = Reader { buf: data, pos: 0 };
    if r.take(8)? != MAGIC || r.u32()? != VERSION {
        return None;
    }
    let n_strings = r.count(4)?;
    let mut strings: Vec<Arc<str>> = Vec::with_capacity(n_strings);
    for _ in 0..n_strings {
        strings.push(Arc::from(r.str()?));
    }
    let name = |id: u32| strings.get(id as usize).cloned();
    let n_files = r.count(28)?;
    let mut files = Vec::with_capacity(n_files);
    for _ in 0..n_files {
        let path = WorkspacePath::new(r.str()?).ok()?;
        let stamp = Stamp {
            len: r.u64()?,
            mtime_ns: r.u64()?,
        };
        let hash = r.u64()?;
        let n = r.count(24)?;
        let mut scopes = Vec::with_capacity(n);
        for _ in 0..n {
            let ns = name(r.u32()?)?;
            let parent = r.u32()?;
            if parent != NONE && parent as usize >= scopes.len() {
                return None;
            }
            scopes.push(Scope {
                ns,
                parent,
                range: r.range()?,
            });
        }
        if scopes.is_empty() {
            return None;
        }
        let n = r.count(14)?;
        let mut usings = Vec::with_capacity(n);
        for _ in 0..n {
            let scope = r.u32()?;
            let kind = match r.u8()? {
                0 => UsingKind::Namespace,
                1 => UsingKind::Static,
                2 => UsingKind::Alias,
                _ => return None,
            };
            let global = r.u8()? != 0;
            let (alias, target) = (name(r.u32()?)?, name(r.u32()?)?);
            if scope as usize >= scopes.len() {
                return None;
            }
            usings.push(Using {
                scope,
                kind,
                global,
                alias,
                target,
            });
        }
        let n = r.count(46)?;
        let mut decls = Vec::with_capacity(n);
        for i in 0..n {
            let name = name(r.u32()?)?;
            let kind = DeclKind::from_u8(r.u8()?)?;
            let arity = r.u8()?;
            let parent = r.u32()?;
            let scope = r.u32()?;
            if (parent != NONE && parent as usize >= i) || scope as usize >= scopes.len() {
                return None;
            }
            decls.push(Decl {
                name,
                kind,
                arity,
                parent,
                scope,
                range: r.range()?,
                selection: r.range()?,
            });
        }
        files.push(LoadedFile {
            path,
            stamp,
            hash,
            parsed: Parsed {
                decls,
                scopes,
                usings,
            },
        });
    }
    (r.pos == data.len()).then_some(files)
}
