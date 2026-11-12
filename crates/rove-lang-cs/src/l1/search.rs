//! 工作区符号搜索用的名字表：全部不同符号名的扁平副本，ASCII 名字另存一份小写字节，
//! 先做子序列预筛再交给打分器。随索引增删同步维护（插入 O(1)、删除打墓碑，墓碑过半时压实），
//! 所以按键时不需要重建，也不用遍历散列表。

use super::{
    fuzzy::SymbolScorer,
    names::{FxMap, Interner, Name},
};
use rove_base::{Cancellation, Result};

/// 名字数超过它才分线程打分。
const PARALLEL_SCORE_THRESHOLD: usize = 32 * 1024;

#[derive(Clone, Copy)]
struct Entry {
    /// `Name::EMPTY` 表示墓碑。
    name: Name,
    /// 小写 ASCII 字节在 `lower` 中的区间；非 ASCII 名字 `len == u32::MAX`（不预筛）。
    start: u32,
    len: u32,
}

#[derive(Default)]
pub(crate) struct SearchNames {
    entries: Vec<Entry>,
    positions: FxMap<Name, u32>,
    lower: Vec<u8>,
    dead: usize,
}

impl SearchNames {
    pub fn insert(&mut self, name: Name, text: &str) {
        if self.positions.contains_key(&name) {
            return;
        }
        let entry = if text.is_ascii() {
            let start = self.lower.len() as u32;
            self.lower
                .extend(text.bytes().map(|b| b.to_ascii_lowercase()));
            Entry {
                name,
                start,
                len: text.len() as u32,
            }
        } else {
            Entry {
                name,
                start: 0,
                len: u32::MAX,
            }
        };
        self.positions.insert(name, self.entries.len() as u32);
        self.entries.push(entry);
    }

    pub fn remove(&mut self, name: Name) {
        let Some(pos) = self.positions.remove(&name) else {
            return;
        };
        self.entries[pos as usize].name = Name::EMPTY;
        self.dead += 1;
        if self.dead > 4096 && self.dead * 2 > self.entries.len() {
            self.compact();
        }
    }

    fn compact(&mut self) {
        let mut entries = Vec::with_capacity(self.entries.len() - self.dead);
        let mut lower = Vec::with_capacity(self.lower.len() / 2);
        self.positions.clear();
        for e in self.entries.drain(..).filter(|e| e.name != Name::EMPTY) {
            let mut e = e;
            if e.len != u32::MAX {
                let start = lower.len() as u32;
                lower.extend_from_slice(&self.lower[e.start as usize..(e.start + e.len) as usize]);
                e.start = start;
            }
            self.positions.insert(e.name, entries.len() as u32);
            entries.push(e);
        }
        self.entries = entries;
        self.lower = lower;
        self.dead = 0;
    }

    pub fn heap_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<Entry>()
            + self.positions.capacity() * 9
            + self.lower.capacity()
    }

    /// 返回 (分数, 名字长度, 名字)。
    pub fn score(
        &self,
        names: &Interner,
        scorer: &dyn SymbolScorer,
        query: &str,
        cancel: &Cancellation,
    ) -> Result<Vec<(i32, u32, Name)>> {
        let lowered: Option<Vec<u8>> = query
            .is_ascii()
            .then(|| query.bytes().map(|b| b.to_ascii_lowercase()).collect());
        let run = |entries: &[Entry]| -> Result<Vec<(i32, u32, Name)>> {
            let mut hits = Vec::new();
            for (i, e) in entries.iter().enumerate() {
                if i % 8192 == 0 {
                    cancel.check()?;
                }
                if e.name == Name::EMPTY {
                    continue;
                }
                if let (Some(q), true) = (&lowered, e.len != u32::MAX) {
                    let hay = &self.lower[e.start as usize..(e.start + e.len) as usize];
                    if !is_subsequence(q, hay) {
                        continue;
                    }
                }
                let text = names.get(e.name);
                if let Some(score) = scorer.score(query, text) {
                    hits.push((score, text.len() as u32, e.name));
                }
            }
            Ok(hits)
        };
        let n = self.entries.len();
        if n < PARALLEL_SCORE_THRESHOLD {
            return run(&self.entries);
        }
        let workers = std::thread::available_parallelism()
            .map_or(4, |c| c.get())
            .clamp(2, 8);
        let chunk = n.div_ceil(workers);
        std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .entries
                .chunks(chunk)
                .map(|part| scope.spawn(move || run(part)))
                .collect();
            let mut all = Vec::new();
            for h in handles {
                all.extend(h.join().expect("scoring thread")?);
            }
            Ok(all)
        })
    }
}

/// `needle` 是否是 `hay` 的子序列（两者都已小写）。
fn is_subsequence(needle: &[u8], hay: &[u8]) -> bool {
    if needle.len() > hay.len() {
        return false;
    }
    let mut rest = hay;
    for &b in needle {
        match memchr::memchr(b, rest) {
            Some(i) => rest = &rest[i + 1..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l1::fuzzy::SimpleScorer;

    #[test]
    fn insert_remove_and_compaction_keep_results_consistent() {
        let mut names = Interner::default();
        let mut table = SearchNames::default();
        let mut ids = Vec::new();
        for i in 0..10_000 {
            let text = format!("Name{i}");
            let id = names.intern(&text);
            table.insert(id, &text);
            ids.push(id);
        }
        let wide = names.intern("Ünicode");
        table.insert(wide, "Ünicode");
        // 删掉前 8,000 个，触发压实。
        for id in &ids[..8000] {
            table.remove(*id);
        }
        assert!(
            table.entries.len() < 8000,
            "过半墓碑时已压实：{}",
            table.entries.len()
        );
        assert_eq!(table.positions.len(), 2001);
        let c = Cancellation::default();
        let hits = table.score(&names, &SimpleScorer, "name9999", &c).unwrap();
        assert_eq!(hits.len(), 1);
        let removed = ids[1];
        assert!(table
            .score(&names, &SimpleScorer, "name1", &c)
            .unwrap()
            .iter()
            .all(|h| h.2 != removed));
        assert_eq!(
            table
                .score(&names, &SimpleScorer, "ünic", &c)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn subsequence_prefilter() {
        assert!(is_subsequence(b"pc", b"playercontroller"));
        assert!(!is_subsequence(b"cp", b"pc"));
        assert!(!is_subsequence(b"abc", b"ab"));
    }
}
