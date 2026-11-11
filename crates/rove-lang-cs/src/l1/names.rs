//! 共享名称表：L1 常驻内存的大头是名字，同名声明（`Init`、`Update`……）只存一份字符串。

use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, Hasher},
    sync::Arc,
};

/// FxHash（rustc 用的乘法-旋转散列）：键都是短标识符，不需要抗碰撞攻击，比 SipHash 快得多。
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher(u64);

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().expect("8 bytes")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

pub(crate) type FxBuild = BuildHasherDefault<FxHasher>;
pub(crate) type FxMap<K, V> = HashMap<K, V, FxBuild>;

/// 文件内容哈希（增量更新判定用）。与 FxHasher 同一套运算，但跨版本稳定：
/// 算法写死在这里，不依赖标准库散列的实现细节，缓存文件里的值下次启动仍然可比。
pub(crate) fn content_hash(bytes: &[u8]) -> u64 {
    let mut h = FxHasher(bytes.len() as u64);
    h.write(bytes);
    h.finish()
}

/// 名称 ID。`Name::EMPTY` 是空串（全局命名空间、无容器）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Name(pub u32);

impl Name {
    pub const EMPTY: Name = Name(0);
}

/// 只增不减的驻留表。被替换文件留下的旧名字不回收：每次保存时只写被引用的条目，
/// 下次加载自然压实（见 `persist.rs`）。
pub(crate) struct Interner {
    strings: Vec<Arc<str>>,
    ids: FxMap<Arc<str>, Name>,
    bytes: usize,
}

impl Default for Interner {
    fn default() -> Self {
        let mut this = Self {
            strings: Vec::new(),
            ids: FxMap::default(),
            bytes: 0,
        };
        this.intern("");
        this
    }
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> Name {
        if let Some(&id) = self.ids.get(s) {
            return id;
        }
        let id = Name(self.strings.len() as u32);
        let arc: Arc<str> = Arc::from(s);
        self.bytes += s.len();
        self.strings.push(arc.clone());
        self.ids.insert(arc, id);
        id
    }

    pub fn lookup(&self, s: &str) -> Option<Name> {
        self.ids.get(s).copied()
    }

    pub fn get(&self, name: Name) -> &Arc<str> {
        &self.strings[name.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// 字符串正文 + Arc 头 + 两份指针 + 散列表槽位的估算值。
    pub fn heap_bytes(&self) -> usize {
        let entries = self.strings.len();
        self.bytes + entries * (16 + 16 + 16 + 8) + self.strings.capacity() * 16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_shares_equal_strings() {
        let mut names = Interner::default();
        assert_eq!(names.intern(""), Name::EMPTY);
        let a = names.intern("Init");
        let b = names.intern("Update");
        assert_eq!(names.intern("Init"), a);
        assert_ne!(a, b);
        assert_eq!(&**names.get(b), "Update");
        assert_eq!(names.lookup("Missing"), None);
    }

    #[test]
    fn content_hash_is_stable_and_length_sensitive() {
        assert_eq!(content_hash(b"class A {}"), content_hash(b"class A {}"));
        assert_ne!(content_hash(b"class A {}"), content_hash(b"class B {}"));
        assert_ne!(content_hash(b"\0"), content_hash(b"\0\0"));
        // 固定值：算法一旦改变，旧缓存里的哈希就对不上了（只会多重扫，不会出错），这里提醒改版本号。
        assert_eq!(content_hash(b"abc"), 0xfd79_00dc_46ce_1495);
    }
}
