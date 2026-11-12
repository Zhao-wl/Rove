//! `scoreFuzzy2` 作为 #18 的 [`Matcher`]：`@` 符号与转到符号共用，排序与高亮与 VS Code 一致。
use crate::list_widget::matcher::{FindMatch, Matcher};
use rove_fuzzy::{prepare_query, score_fuzzy2, utf16_to_byte_ranges, PreparedQuery};
use std::sync::Mutex;

/// VS Code `gotoSymbolQuickAccess` 的打分：`scoreFuzzy2(label, preparedQuery)`。
#[derive(Default)]
pub struct FuzzyMatcher {
    /// 同一个查询对很多候选打分：缓存最近一次整理好的查询。
    prepared: Mutex<Option<(String, PreparedQuery)>>,
}

impl FuzzyMatcher {
    fn with_query<T>(&self, query: &str, f: impl FnOnce(&PreparedQuery) -> T) -> T {
        let mut guard = self.prepared.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_none_or(|(q, _)| q != query) {
            *guard = Some((query.to_owned(), prepare_query(query)));
        }
        f(&guard.as_ref().unwrap().1)
    }
}

impl Matcher for FuzzyMatcher {
    fn score(&self, query: &str, label: &str) -> Option<FindMatch> {
        if query.trim().is_empty() {
            return Some(FindMatch {
                score: 0,
                ranges: vec![],
            });
        }
        self.with_query(query, |q| {
            if q.normalized.is_empty() {
                return Some(FindMatch {
                    score: 0,
                    ranges: vec![],
                });
            }
            let (score, matches) = score_fuzzy2(label, q, 0, 0);
            Some(FindMatch {
                score: score?,
                ranges: utf16_to_byte_ranges(label, &matches),
            })
        })
    }
}
