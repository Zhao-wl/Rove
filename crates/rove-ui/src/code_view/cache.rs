//! 按字节预算淘汰的 LRU。整形缓存与行文本缓存共用；O(log n) 读写，不在绘制中做全表扫描。

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

pub struct Lru<K, V> {
    map: HashMap<K, (V, u64, usize)>,
    order: BTreeMap<u64, K>,
    tick: u64,
    bytes: usize,
    capacity: usize,
    hits: u64,
    misses: u64,
}

/// 缓存统计（计入内存预算、供基准与测试断言）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub capacity: usize,
    pub hits: u64,
    pub misses: u64,
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: BTreeMap::new(),
            tick: 0,
            bytes: 0,
            capacity: capacity_bytes,
            hits: 0,
            misses: 0,
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.tick += 1;
        match self.map.get_mut(key) {
            Some(entry) => {
                self.hits += 1;
                self.order.remove(&entry.1);
                entry.1 = self.tick;
                self.order.insert(self.tick, key.clone());
                Some(&entry.0)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// 不更新最近使用次序的只读查询。
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.map.get(key).map(|e| &e.0)
    }

    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    /// 插入并按字节预算淘汰最久未用的条目。`cost` 是该条目的估计字节数。
    pub fn insert(&mut self, key: K, value: V, cost: usize) {
        self.tick += 1;
        if let Some(old) = self.map.insert(key.clone(), (value, self.tick, cost)) {
            self.order.remove(&old.1);
            self.bytes -= old.2;
        }
        self.order.insert(self.tick, key);
        self.bytes += cost;
        while self.bytes > self.capacity && self.order.len() > 1 {
            let (&tick, _) = self.order.iter().next().expect("non-empty");
            let k = self.order.remove(&tick).expect("present");
            if let Some(e) = self.map.remove(&k) {
                self.bytes -= e.2;
            }
        }
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.bytes = 0;
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let (value, tick, cost) = self.map.remove(key)?;
        self.order.remove(&tick);
        self.bytes -= cost;
        Some(value)
    }

    /// #47：按新键重排（编辑后行号平移）。`f` 返回 `None` 的条目被丢弃；保持原有的最近使用次序。
    pub fn rekey(&mut self, mut f: impl FnMut(&K) -> Option<K>) {
        let map = std::mem::take(&mut self.map);
        self.order.clear();
        self.bytes = 0;
        for (key, (value, tick, cost)) in map {
            if let Some(new) = f(&key) {
                self.order.insert(tick, new.clone());
                self.bytes += cost;
                if let Some(old) = self.map.insert(new, (value, tick, cost)) {
                    self.order.remove(&old.1);
                    self.bytes -= old.2;
                }
            }
        }
    }

    pub fn set_capacity(&mut self, capacity_bytes: usize) {
        self.capacity = capacity_bytes;
        while self.bytes > self.capacity && !self.order.is_empty() {
            let (&tick, _) = self.order.iter().next().expect("non-empty");
            let k = self.order.remove(&tick).expect("present");
            if let Some(e) = self.map.remove(&k) {
                self.bytes -= e.2;
            }
        }
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.map.len(),
            bytes: self.bytes,
            capacity: self.capacity,
            hits: self.hits,
            misses: self.misses,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used_by_bytes() {
        let mut c = Lru::new(30);
        c.insert(1, "a", 10);
        c.insert(2, "b", 10);
        c.insert(3, "c", 10);
        assert!(c.get(&1).is_some());
        c.insert(4, "d", 10);
        assert!(c.peek(&2).is_none());
        assert!(c.peek(&1).is_some());
        assert_eq!(c.stats().bytes, 30);
        c.insert(1, "a2", 5);
        assert_eq!(c.stats().bytes, 25);
        c.set_capacity(10);
        assert_eq!(c.stats().entries, 1);
        c.clear();
        assert_eq!(
            c.stats(),
            CacheStats {
                capacity: 10,
                hits: 1,
                misses: 0,
                ..Default::default()
            }
        );
    }
}
