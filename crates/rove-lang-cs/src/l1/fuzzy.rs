//! 工作区符号搜索的打分接口。
//!
//! 正式算法是 #24 移植的 VS Code `fuzzyScorer`；它合并前这里只提供一个最简实现，
//! #24 合并后由装配处把它的打分器包成 [`SymbolScorer`] 注入 [`super::L1Options::scorer`]，
//! 不在这里另写一套打分规则。

/// 纯函数、无分配、`Send + Sync`：每次按键会对全部不同的符号名各调用一次。
pub trait SymbolScorer: Send + Sync {
    /// 不匹配返回 None；分数越高越靠前。`query` 非空。
    ///
    /// 约定：只有 `query` 是 `candidate` 的大小写不敏感子序列时才可能匹配（VS Code 的模糊匹配同样如此）。
    /// L1 会先用这个条件预筛 ASCII 名字，不满足的候选不会交给打分器。
    fn score(&self, query: &str, candidate: &str) -> Option<i32>;
}

/// 占位实现：大小写不敏感的子序列匹配，完全相等 > 前缀 > 单词起点（驼峰、`_`）> 连续命中。
#[derive(Clone, Copy, Debug, Default)]
pub struct SimpleScorer;

impl SymbolScorer for SimpleScorer {
    fn score(&self, query: &str, candidate: &str) -> Option<i32> {
        if query.is_ascii() && candidate.is_ascii() {
            score_bytes(query.as_bytes(), candidate.as_bytes())
        } else {
            let q: Vec<char> = query.chars().collect();
            let c: Vec<char> = candidate.chars().collect();
            score_chars(&q, &c)
        }
    }
}

trait Unit: Copy {
    fn lower_eq(self, other: Self) -> bool;
    fn is_upper(self) -> bool;
    fn is_separator(self) -> bool;
}

impl Unit for u8 {
    #[inline]
    fn lower_eq(self, other: Self) -> bool {
        self.eq_ignore_ascii_case(&other)
    }
    #[inline]
    fn is_upper(self) -> bool {
        self.is_ascii_uppercase()
    }
    #[inline]
    fn is_separator(self) -> bool {
        matches!(self, b'_' | b'.' | b' ' | b'~')
    }
}

impl Unit for char {
    fn lower_eq(self, other: Self) -> bool {
        self == other || self.to_lowercase().eq(other.to_lowercase())
    }
    fn is_upper(self) -> bool {
        self.is_uppercase()
    }
    fn is_separator(self) -> bool {
        matches!(self, '_' | '.' | ' ' | '~')
    }
}

fn score_bytes(q: &[u8], c: &[u8]) -> Option<i32> {
    score_units(q, c)
}

fn score_chars(q: &[char], c: &[char]) -> Option<i32> {
    score_units(q, c)
}

fn score_units<T: Unit>(q: &[T], c: &[T]) -> Option<i32> {
    if q.is_empty() || q.len() > c.len() {
        return None;
    }
    // 快速排除：贪心子序列。
    let mut qi = 0;
    for &ch in c {
        if qi < q.len() && ch.lower_eq(q[qi]) {
            qi += 1;
        }
    }
    if qi < q.len() {
        return None;
    }
    if q.len() == c.len() && q.iter().zip(c).all(|(a, b)| a.lower_eq(*b)) {
        return Some(10_000 + exact_case_bonus(q, c));
    }
    if q.iter().zip(c).all(|(a, b)| a.lower_eq(*b)) {
        return Some(5_000 - (c.len() - q.len()) as i32 + exact_case_bonus(q, c));
    }
    // 子序列打分：优先命中单词起点，连续命中加分，越靠前越好。
    let mut score = 0i32;
    let mut qi = 0;
    let mut prev_hit = false;
    for (i, &ch) in c.iter().enumerate() {
        if qi == q.len() {
            break;
        }
        if ch.lower_eq(q[qi]) {
            let word_start =
                i == 0 || ch.is_upper() && !c[i - 1].is_upper() || c[i - 1].is_separator();
            score += 1;
            if word_start {
                score += 8;
            }
            if prev_hit {
                score += 4;
            }
            if i < 8 {
                score += 1;
            }
            prev_hit = true;
            qi += 1;
        } else {
            prev_hit = false;
        }
    }
    Some(score)
}

fn exact_case_bonus<T: Unit>(q: &[T], c: &[T]) -> i32 {
    // 大小写完全一致的再往前排一点（`init` 对 `init` 胜过 `Init`）。
    q.iter()
        .zip(c)
        .filter(|(a, b)| a.is_upper() == b.is_upper())
        .count() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_beats_prefix_beats_camel_beats_subsequence() {
        let s = SimpleScorer;
        let exact = s.score("player", "Player").unwrap();
        let prefix = s.score("player", "PlayerController").unwrap();
        let camel = s.score("pc", "PlayerController").unwrap();
        let loose = s.score("pc", "ApplyCache").unwrap();
        assert!(exact > prefix, "{exact} {prefix}");
        assert!(prefix > camel);
        assert!(camel > loose, "{camel} {loose}");
        assert_eq!(s.score("xyz", "PlayerController"), None);
        assert_eq!(s.score("toolong", "short"), None);
    }

    #[test]
    fn non_ascii_names_match_case_insensitively() {
        let s = SimpleScorer;
        assert!(s.score("ÄB", "äbc").is_some());
        assert!(s.score("名", "名字").is_some());
    }
}
