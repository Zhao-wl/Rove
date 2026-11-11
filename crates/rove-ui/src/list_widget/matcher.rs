//! Find/filter matching interface for lists, trees and quick input (#18).
//!
//! The contract is "score + highlight ranges". This module ships a simple contiguous and a simple
//! subsequence matcher; #24 ports VS Code's `fuzzyScorer` behind the same [`Matcher`] trait.
use std::{ops::Range, sync::Arc};

/// `workbench.list.defaultFindMatchType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchType {
    Fuzzy,
    Contiguous,
}

/// `workbench.list.defaultFindMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindMode {
    Highlight,
    Filter,
}

/// A successful match. `ranges` are sorted, disjoint, non-empty UTF-8 byte ranges into the label,
/// always on char boundaries. Higher `score` is better.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindMatch {
    pub score: i32,
    pub ranges: Vec<Range<usize>>,
}

/// Pure, allocation-light matching that runs on the UI thread within a frame.
/// An empty query matches every label with no highlight ranges.
pub trait Matcher: Send + Sync {
    fn score(&self, query: &str, label: &str) -> Option<FindMatch>;
}

fn eq_ignore_case(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

fn push_range(ranges: &mut Vec<Range<usize>>, r: Range<usize>) {
    if let Some(last) = ranges.last_mut() {
        if last.end == r.start {
            last.end = r.end;
            return;
        }
    }
    ranges.push(r);
}

/// Case-insensitive substring match (`contiguous`). Earlier matches score higher.
pub struct ContiguousMatcher;
impl Matcher for ContiguousMatcher {
    fn score(&self, query: &str, label: &str) -> Option<FindMatch> {
        let q: Vec<char> = query.chars().collect();
        if q.is_empty() {
            return Some(FindMatch {
                score: 0,
                ranges: vec![],
            });
        }
        for (start, _) in label.char_indices() {
            let mut it = label[start..].char_indices();
            let mut end = start;
            let mut ok = true;
            for qc in &q {
                match it.next() {
                    Some((o, c)) if eq_ignore_case(*qc, c) => end = start + o + c.len_utf8(),
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                return Some(FindMatch {
                    score: 1000 - start.min(999) as i32,
                    ranges: std::iter::once(start..end).collect(),
                });
            }
        }
        None
    }
}

/// Case-insensitive in-order subsequence match (`fuzzy`). Placeholder until #24 ports
/// VS Code's scorer; consecutive characters and matches at the start score higher.
pub struct SimpleFuzzyMatcher;
impl Matcher for SimpleFuzzyMatcher {
    fn score(&self, query: &str, label: &str) -> Option<FindMatch> {
        let mut ranges = Vec::new();
        let mut score = 0i32;
        let mut chars = label.char_indices();
        let mut previous_end = None;
        for qc in query.chars() {
            loop {
                let (o, c) = chars.next()?;
                if eq_ignore_case(qc, c) {
                    score += if previous_end == Some(o) { 5 } else { 1 };
                    if o == 0 {
                        score += 3;
                    }
                    previous_end = Some(o + c.len_utf8());
                    push_range(&mut ranges, o..o + c.len_utf8());
                    break;
                }
            }
        }
        Some(FindMatch { score, ranges })
    }
}

pub fn matcher_for(kind: MatchType) -> Arc<dyn Matcher> {
    match kind {
        MatchType::Fuzzy => Arc::new(SimpleFuzzyMatcher),
        MatchType::Contiguous => Arc::new(ContiguousMatcher),
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    #[test]
    fn contiguous_highlights_the_exact_substring() {
        let m = ContiguousMatcher;
        assert_eq!(m.score("lay", "Player.cs").unwrap().ranges, vec![1..4]);
        assert_eq!(m.score("PLAY", "player").unwrap().ranges, vec![0..4]);
        assert!(m.score("pyr", "Player.cs").is_none());
        // UTF-8 byte ranges stay on char boundaries.
        assert_eq!(
            m.score("配置", "游戏配置.json").unwrap().ranges,
            vec![6..12]
        );
        assert!(m.score("", "x").unwrap().ranges.is_empty());
    }

    #[test]
    fn fuzzy_highlights_each_matched_character() {
        let m = SimpleFuzzyMatcher;
        let found = m.score("pyr", "Player.cs").unwrap();
        assert_eq!(found.ranges, vec![0..1, 3..4, 5..6]);
        let joined = m.score("lay", "Player.cs").unwrap();
        assert_eq!(joined.ranges, vec![1..4]);
        assert!(m.score("zz", "Player.cs").is_none());
        assert!(m.score("pl", "Player").unwrap().score > m.score("pl", "aPxl").unwrap().score);
    }
}
