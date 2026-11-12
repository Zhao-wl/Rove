//! 移植 VS Code `filters.ts` 的单词过滤：`matchesWords`、`matchesContiguousSubString`，
//! 以及命令面板使用的 `WORD_FILTER = or(matchesPrefix, matchesWords, matchesContiguousSubString)`。
//!
//! 与原实现的差异：不做 `tryNormalizeToBase`（去重音）与“备用字符”（`getAlternateCodes`）处理，
//! Rove 的命令标题是中文，这两项只影响带重音的拉丁字母。区间是 UTF-16 码元下标。

use crate::filters::{matches_prefix, units, Match};
use std::collections::HashMap;

fn is_word_separator(code: u16) -> bool {
    matches!(code, 0x20 | 0x09 | 0x0A | 0x0D)
        || "()[]{}<>`'\"-/;:,.?!".encode_utf16().any(|c| c == code)
}

fn characters_match(a: u16, b: u16) -> bool {
    a == b || (is_word_separator(a) && is_word_separator(b))
}

fn next_word(word: &[u16], start: usize) -> usize {
    for i in start..word.len() {
        if is_word_separator(word[i]) || (i > 0 && is_word_separator(word[i - 1])) {
            return i;
        }
    }
    word.len()
}

fn join(head: Match, mut tail: Vec<Match>) -> Vec<Match> {
    if tail.is_empty() {
        return vec![head];
    }
    if head.end == tail[0].start {
        tail[0].start = head.start;
    } else {
        tail.insert(0, head);
    }
    tail
}

type Memo = HashMap<usize, Option<Vec<Match>>>;

/// `matchesWords(word, target, contiguous)`。
pub fn matches_words(word: &str, target: &str, contiguous: bool) -> Option<Vec<Match>> {
    if target.is_empty() {
        return None;
    }
    // `tryNormalizeToBase`：这里只做小写化（不去重音）
    let word = units(&word.to_lowercase());
    let target = units(&target.to_lowercase());
    let mut memo = Memo::new();
    let mut target_index = 0;
    while target_index < target.len() {
        if let Some(result) =
            matches_words_at(&word, &target, 0, target_index, contiguous, &mut memo)
        {
            return Some(result);
        }
        target_index = next_word(&target, target_index + 1);
    }
    None
}

fn matches_words_at(
    word: &[u16],
    target: &[u16],
    word_index: usize,
    target_index: usize,
    contiguous: bool,
    memo: &mut Memo,
) -> Option<Vec<Match>> {
    if word_index == word.len() {
        return Some(Vec::new());
    } else if target_index == target.len() {
        return None;
    }
    let key = word_index * (target.len() + 1) + target_index;
    if let Some(cached) = memo.get(&key) {
        return cached.clone();
    }
    let computed = compute(word, target, word_index, target_index, contiguous, memo);
    memo.insert(key, computed.clone());
    computed
}

fn compute(
    word: &[u16],
    target: &[u16],
    word_index: usize,
    target_index: usize,
    contiguous: bool,
    memo: &mut Memo,
) -> Option<Vec<Match>> {
    if !characters_match(word[word_index], target[target_index]) {
        return None;
    }
    let mut next_word_index = target_index + 1;
    let mut result = matches_words_at(
        word,
        target,
        word_index + 1,
        next_word_index,
        contiguous,
        memo,
    );
    if !contiguous {
        while result.is_none() {
            next_word_index = next_word(target, next_word_index);
            if next_word_index >= target.len() {
                break;
            }
            result = matches_words_at(
                word,
                target,
                word_index + 1,
                next_word_index,
                contiguous,
                memo,
            );
            next_word_index += 1;
        }
    }
    let result = result?;
    // 字符不完全相同时必然是单词分隔符：不计入匹配区间，但保留后面的结果
    if word[word_index] != target[target_index] {
        return Some(result);
    }
    Some(join(
        Match {
            start: target_index,
            end: target_index + 1,
        },
        result,
    ))
}

/// `matchesContiguousSubString`。
pub fn matches_contiguous_sub_string(word: &str, target: &str) -> Option<Vec<Match>> {
    let word_units = units(word);
    let target_units = units(target);
    if word_units.len() > target_units.len() {
        return None;
    }
    let target_low = units(&target.to_lowercase());
    let word_low = units(&word.to_lowercase());
    if word_low.is_empty() {
        return Some(vec![Match { start: 0, end: 0 }]);
    }
    let index = target_low
        .windows(word_low.len())
        .position(|w| w == &word_low[..])?;
    Some(vec![Match {
        start: index,
        end: index + word_units.len(),
    }])
}

/// 命令面板的过滤器：`or(matchesPrefix, matchesWords, matchesContiguousSubString)`。
pub fn word_filter(word: &str, target: &str) -> Option<Vec<Match>> {
    matches_prefix(word, target)
        .or_else(|| matches_words(word, target, false))
        .or_else(|| matches_contiguous_sub_string(word, target))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(m: Option<Vec<Match>>) -> Vec<(usize, usize)> {
        m.unwrap().iter().map(|m| (m.start, m.end)).collect()
    }

    #[test]
    fn words_match_at_word_starts() {
        assert_eq!(
            starts(word_filter("togg side", "Toggle Side Bar Visibility")),
            vec![(0, 4), (6, 11)]
        );
        assert!(word_filter("xyz", "Toggle Side Bar").is_none());
        assert_eq!(starts(word_filter("tog", "Toggle")), vec![(0, 3)]);
    }

    #[test]
    fn cjk_falls_back_to_contiguous_substring() {
        assert_eq!(starts(word_filter("侧边", "切换侧边栏")), vec![(2, 4)]);
        assert!(word_filter("边切", "切换侧边栏").is_none());
    }
}
