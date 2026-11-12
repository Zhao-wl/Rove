//! VS Code 模糊匹配算法的移植（#24）。核心层，不依赖 GPUI。
//!
//! 快速打开（`Ctrl+P`）的排序是用户的肌肉记忆，所以这里逐条移植 VS Code 的
//! `src/vs/base/common/fuzzyScorer.ts`（以及它用到的 `filters.ts`、`comparers.ts` 片段），
//! 不用 `nucleo` 之类的现成库近似。来源与许可见根目录 `THIRD_PARTY_NOTICES.md`。
//!
//! 与 TypeScript 一致，字符串按 UTF-16 码元处理：区间和位置都是码元下标。界面需要字节区间时
//! 用 [`utf16_to_byte_ranges`] 转换。
mod compare;
mod filters;
mod scorer;
mod words;

pub use compare::{compare_anything, compare_by_prefix, compare_file_names};
pub use filters::{create_matches, fuzzy_score, matches_prefix, FuzzyScore, FuzzyScoreOptions, Match};
pub use scorer::*;
pub use words::{matches_contiguous_sub_string, matches_words, word_filter};

use std::ops::Range;

/// 把 UTF-16 码元区间转换成 `s` 里的 UTF-8 字节区间（落在字符边界上；越界的部分被截断）。
pub fn utf16_to_byte_ranges(s: &str, matches: &[Match]) -> Vec<Range<usize>> {
    // 码元下标 → 字节偏移（只记录字符起点）
    let mut out = Vec::with_capacity(matches.len());
    let mut iter = s.char_indices().peekable();
    let mut unit = 0usize;
    let offset_at = |target: usize, iter: &mut std::iter::Peekable<std::str::CharIndices>, unit: &mut usize| -> usize {
        while *unit < target {
            match iter.next() {
                Some((_, c)) => *unit += c.len_utf16(),
                None => return s.len(),
            }
        }
        iter.peek().map_or(s.len(), |(b, _)| *b)
    };
    for m in matches {
        let start = offset_at(m.start, &mut iter, &mut unit);
        let end = offset_at(m.end, &mut iter, &mut unit);
        if end > start {
            out.push(start..end);
        }
    }
    out
}

#[cfg(test)]
mod tests;
