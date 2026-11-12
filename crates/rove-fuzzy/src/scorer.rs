//! 移植 VS Code `src/vs/base/common/fuzzyScorer.ts`：`scoreFuzzy`、`scoreFuzzy2`、`scoreItemFuzzy`、
//! `compareItemsByFuzzyScore`、`prepareQuery`。分数、区间与排序规则逐条照搬，不做“更合理”的改动。
//!
//! 与 TypeScript 一致，字符串按 UTF-16 码元处理，区间是码元下标（需要字节区间时用
//! [`crate::utf16_to_byte_ranges`]）。Windows 路径分隔符为 `\`（`path.sep`）。

use crate::{
    compare::compare_anything,
    filters::{
        create_matches, fuzzy_score, is_upper, lower_units, matches_prefix, units, FuzzyScoreOptions,
        Match,
    },
};
use std::{
    borrow::Cow,
    cell::RefCell,
    cmp::Ordering,
    collections::HashMap,
    hash::{Hash, Hasher},
    ops::Deref,
};

/// `path.sep`。
#[cfg(windows)]
pub const SEP: char = '\\';
#[cfg(not(windows))]
pub const SEP: char = '/';

const NO_MATCH: i32 = 0;

pub const PATH_IDENTITY_SCORE: i32 = 1 << 18;
pub const LABEL_PREFIX_SCORE_THRESHOLD: i32 = 1 << 17;
pub const LABEL_SCORE_THRESHOLD: i32 = 1 << 16;

// ---------------------------------------------------------------------------------------------
// scoreFuzzy

/// `FuzzyScore`（`scoreFuzzy` 的结果）：分数与匹配位置（UTF-16 码元下标，升序）。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Score {
    pub score: i32,
    pub positions: Vec<usize>,
}

#[derive(Default)]
struct Matrix {
    scores: Vec<i32>,
    matches: Vec<u32>,
}

thread_local! {
    static MATRIX: RefCell<Matrix> = RefCell::new(Matrix::default());
}

/// `scoreFuzzy(target, query, queryLower, allowNonContiguousMatches)`。
pub fn score_fuzzy(
    target: &str,
    query: &str,
    query_lower: &str,
    allow_non_contiguous_matches: bool,
) -> Score {
    if target.is_empty() || query.is_empty() {
        return Score::default();
    }
    score_fuzzy_units(
        &units(target),
        &lower_units(target),
        &units(query),
        &units(query_lower),
        allow_non_contiguous_matches,
    )
}

fn consider_as_equal(a: u16, b: u16) -> bool {
    if a == b {
        return true;
    }
    // 路径分隔符：忽略平台差异
    if a == b'/' as u16 || a == b'\\' as u16 {
        return b == b'/' as u16 || b == b'\\' as u16;
    }
    false
}

fn score_fuzzy_units(
    target: &[u16],
    target_lower: &[u16],
    query: &[u16],
    query_lower: &[u16],
    allow_non_contiguous_matches: bool,
) -> Score {
    let target_length = target.len();
    let query_length = query.len();
    if target_length == 0 || query_length == 0 || target_length < query_length {
        return Score::default();
    }

    // 快速否决（不改变结果）：查询必须按顺序出现在目标中，否则矩阵里不会产生任何分数。
    {
        let mut qi = 0;
        for &t in target_lower {
            if consider_as_equal(query_lower[qi], t) {
                qi += 1;
                if qi == query_length {
                    break;
                }
            }
        }
        if qi < query_length {
            return Score::default();
        }
    }

    MATRIX.with(|m| {
        let mut m = m.borrow_mut();
        let cells = query_length * target_length;
        m.scores.clear();
        m.scores.resize(cells, 0);
        m.matches.clear();
        m.matches.resize(cells, 0);
        let Matrix { scores, matches } = &mut *m;

        for query_index in 0..query_length {
            let query_index_offset = query_index * target_length;
            let query_index_previous_offset = query_index_offset.wrapping_sub(target_length);
            let query_index_gt_null = query_index > 0;
            let query_char = query[query_index];
            let query_lower_char = query_lower[query_index];

            for target_index in 0..target_length {
                let target_index_gt_null = target_index > 0;
                let current_index = query_index_offset + target_index;
                let left_index = current_index.wrapping_sub(1);
                let diag_index = query_index_previous_offset
                    .wrapping_add(target_index)
                    .wrapping_sub(1);

                let left_score = if target_index_gt_null {
                    scores[left_index]
                } else {
                    0
                };
                let diag_score = if query_index_gt_null && target_index_gt_null {
                    scores[diag_index]
                } else {
                    0
                };
                let matches_sequence_length = if query_index_gt_null && target_index_gt_null {
                    matches[diag_index]
                } else {
                    0
                };

                // 不在第一个查询字符上时，只有上一个查询字符有分数才产生分数（保证按顺序匹配）。
                let score = if diag_score == 0 && query_index_gt_null {
                    0
                } else {
                    compute_char_score(
                        query_char,
                        query_lower_char,
                        target,
                        target_lower,
                        target_index,
                        matches_sequence_length,
                    )
                };

                let is_valid_score = score != 0 && diag_score + score >= left_score;
                if is_valid_score
                    && (allow_non_contiguous_matches
                        || query_index_gt_null
                        || target_lower[target_index..].starts_with(query_lower))
                {
                    matches[current_index] = matches_sequence_length + 1;
                    scores[current_index] = diag_score + score;
                } else {
                    matches[current_index] = NO_MATCH as u32;
                    scores[current_index] = left_score;
                }
            }
        }

        // 从矩阵右下角恢复位置
        let mut positions = Vec::new();
        let mut query_index = query_length as isize - 1;
        let mut target_index = target_length as isize - 1;
        while query_index >= 0 && target_index >= 0 {
            let current_index = query_index as usize * target_length + target_index as usize;
            if matches[current_index] == NO_MATCH as u32 {
                target_index -= 1;
            } else {
                positions.push(target_index as usize);
                query_index -= 1;
                target_index -= 1;
            }
        }
        positions.reverse();
        Score {
            score: scores[cells - 1],
            positions,
        }
    })
}

fn compute_char_score(
    query_char: u16,
    query_lower_char: u16,
    target: &[u16],
    target_lower: &[u16],
    target_index: usize,
    matches_sequence_length: u32,
) -> i32 {
    if !consider_as_equal(query_lower_char, target_lower[target_index]) {
        return 0;
    }
    let mut score = 1;

    // 连续匹配奖励：前 3 个满额（6），其余减半（3）
    if matches_sequence_length > 0 {
        let n = matches_sequence_length as i32;
        score += n.min(3) * 6 + (n - 3).max(0) * 3;
    }

    // 大小写相同奖励
    if query_char == target[target_index] {
        score += 1;
    }

    if target_index == 0 {
        // 词首奖励
        score += 8;
    } else {
        // 分隔符之后的奖励
        let separator_bonus = score_separator_at(target[target_index - 1]);
        if separator_bonus != 0 {
            score += separator_bonus;
        } else if is_upper(target[target_index]) && matches_sequence_length == 0 {
            // 单词内的大写（驼峰），只在不处于连续序列时给：NPE => NullPointerException 加分，HTTP => HTTP 不加
            score += 2;
        }
    }
    score
}

fn score_separator_at(code: u16) -> i32 {
    match code {
        0x2F | 0x5C => 5,                                      // / \
        0x5F | 0x2D | 0x2E | 0x20 | 0x27 | 0x22 | 0x3A => 4, // _ - . 空格 ' " :
        _ => 0,
    }
}

// ---------------------------------------------------------------------------------------------
// 查询规整

/// `IPreparedQueryPiece`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryPiece {
    pub original: String,
    pub original_lowercase: String,
    /// 把路径分隔符规整为平台分隔符。
    pub path_normalized: String,
    /// 另外去掉空白、通配符、引号、省略号与结尾的 `#`。
    pub normalized: String,
    pub normalized_lowercase: String,
    /// 查询被引号包住：必须是目标的子串，不做模糊匹配。
    pub expect_contiguous_match: bool,
    normalized_units: Vec<u16>,
    normalized_lower_units: Vec<u16>,
}

/// `IPreparedQuery`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedQuery {
    piece: QueryPiece,
    /// 按空格拆成的各段（只有多于一段时才有）。
    pub values: Option<Vec<QueryPiece>>,
    pub contains_path_separator: bool,
}

impl Deref for PreparedQuery {
    type Target = QueryPiece;
    fn deref(&self) -> &QueryPiece {
        &self.piece
    }
}

fn query_expects_exact_match(query: &str) -> bool {
    query.starts_with('"') && query.ends_with('"')
}

/// JS 正则 `\s`。
fn is_js_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn normalize_query(original: &str) -> (String, String, String) {
    #[cfg(windows)]
    let path_normalized = original.replace('/', "\\");
    #[cfg(not(windows))]
    let path_normalized = original.replace('\\', "/");

    let mut normalized: String = path_normalized
        .chars()
        .filter(|&c| c != '*' && c != '\u{2026}' && c != '"' && !is_js_whitespace(c))
        .collect();
    // `(?<=.)#$`：前面至少还有一个字符时去掉结尾的 `#`
    if normalized.ends_with('#') && normalized.chars().count() > 1 {
        normalized.pop();
    }
    let normalized_lowercase = normalized.to_lowercase();
    (path_normalized, normalized, normalized_lowercase)
}

fn make_piece(original: &str) -> QueryPiece {
    let (path_normalized, normalized, normalized_lowercase) = normalize_query(original);
    QueryPiece {
        original: original.to_owned(),
        original_lowercase: original.to_lowercase(),
        expect_contiguous_match: query_expects_exact_match(original),
        normalized_units: units(&normalized),
        normalized_lower_units: units(&normalized_lowercase),
        path_normalized,
        normalized,
        normalized_lowercase,
    }
}

/// `prepareQuery`。
pub fn prepare_query(original: &str) -> PreparedQuery {
    let piece = make_piece(original);
    let contains_path_separator = piece.path_normalized.contains(SEP);
    let mut values: Option<Vec<QueryPiece>> = None;
    let split: Vec<&str> = original.split(' ').collect();
    if split.len() > 1 {
        for part in split {
            let p = make_piece(part);
            if !p.normalized.is_empty() {
                values.get_or_insert_with(Vec::new).push(p);
            }
        }
    }
    PreparedQuery {
        piece,
        values,
        contains_path_separator,
    }
}

/// `pieceToQuery`：把一段（或若干段）重新规整成完整的查询。
pub fn piece_to_query(pieces: &[QueryPiece]) -> PreparedQuery {
    let joined: Vec<&str> = pieces.iter().map(|p| p.original.as_str()).collect();
    prepare_query(&joined.join(" "))
}

// ---------------------------------------------------------------------------------------------
// scoreFuzzy2（符号等场景）

/// `FuzzyScore2`：分数（`None` 表示没有匹配）与区间。
pub type Score2 = (Option<i32>, Vec<Match>);

/// `scoreFuzzy2(target, query, patternStart, wordStart)`；`query` 可以是整条查询或其中一段。
pub fn score_fuzzy2(
    target: &str,
    query: &PreparedQuery,
    pattern_start: usize,
    word_start: usize,
) -> Score2 {
    if let Some(values) = query.values.as_ref().filter(|v| v.len() > 1) {
        return do_score_fuzzy2_multiple(target, values, pattern_start, word_start);
    }
    do_score_fuzzy2_single(target, &query.piece, pattern_start, word_start)
}

/// 单个查询段的 `scoreFuzzy2`。
pub fn score_fuzzy2_piece(
    target: &str,
    piece: &QueryPiece,
    pattern_start: usize,
    word_start: usize,
) -> Score2 {
    do_score_fuzzy2_single(target, piece, pattern_start, word_start)
}

fn do_score_fuzzy2_multiple(
    target: &str,
    query: &[QueryPiece],
    pattern_start: usize,
    word_start: usize,
) -> Score2 {
    let mut total_score = 0;
    let mut total_matches = Vec::new();
    for piece in query {
        let (score, matches) = do_score_fuzzy2_single(target, piece, pattern_start, word_start);
        let Some(score) = score else {
            // 任何一段不匹配就整体没有分数：要求所有段都匹配
            return (None, Vec::new());
        };
        total_score += score;
        total_matches.extend(matches);
    }
    (Some(total_score), normalize_matches(total_matches))
}

fn do_score_fuzzy2_single(
    target: &str,
    query: &QueryPiece,
    pattern_start: usize,
    word_start: usize,
) -> Score2 {
    let word = units(target);
    let word_low = lower_units(target);
    match fuzzy_score(
        &query.normalized_units,
        &query.normalized_lower_units,
        pattern_start,
        &word,
        &word_low,
        word_start,
        FuzzyScoreOptions {
            first_match_can_be_weak: true,
            boost_full_match: true,
        },
    ) {
        Some(score) => (Some(score.score), create_matches(Some(&score))),
        None => (None, Vec::new()),
    }
}

// ---------------------------------------------------------------------------------------------
// 条目打分（label / description / path）

/// `IItemScore`。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ItemScore {
    pub score: i32,
    /// label 内的匹配区间。
    pub label_match: Option<Vec<Match>>,
    /// description 内的匹配区间。
    pub description_match: Option<Vec<Match>>,
}

const NO_ITEM_SCORE: ItemScore = ItemScore {
    score: 0,
    label_match: None,
    description_match: None,
};

/// `IItemAccessor`。
pub trait ItemAccessor<T: ?Sized> {
    fn item_label<'a>(&self, item: &'a T) -> Option<Cow<'a, str>>;
    fn item_description<'a>(&self, item: &'a T) -> Option<Cow<'a, str>>;
    /// 条目是文件时，用来打分的路径。
    fn item_path<'a>(&self, item: &'a T) -> Option<Cow<'a, str>>;
}

/// `FuzzyScorerCache`：键是“label + description + 是否允许不连续 + 查询”的哈希。
pub type FuzzyScorerCache = HashMap<u64, ItemScore>;

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

fn cache_key(
    label: &str,
    description: Option<&str>,
    allow_non_contiguous_matches: bool,
    query: &PreparedQuery,
) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    query.normalized.hash(&mut h);
    allow_non_contiguous_matches.hash(&mut h);
    description.hash(&mut h);
    label.hash(&mut h);
    match &query.values {
        Some(values) => {
            for v in values {
                v.expect_contiguous_match.hash(&mut h);
                v.normalized.hash(&mut h);
            }
        }
        None => {
            query.expect_contiguous_match.hash(&mut h);
            query.normalized.hash(&mut h);
        }
    }
    h.finish()
}

/// `scoreItemFuzzy`（带缓存的版本，经访问器取 label / description / path）。
pub fn score_item_fuzzy<T: ?Sized>(
    item: Option<&T>,
    query: &PreparedQuery,
    allow_non_contiguous_matches: bool,
    accessor: &dyn ItemAccessor<T>,
    cache: &mut FuzzyScorerCache,
) -> ItemScore {
    let Some(item) = item else {
        return NO_ITEM_SCORE;
    };
    if query.normalized.is_empty() {
        return NO_ITEM_SCORE;
    }
    let Some(label) = accessor.item_label(item).filter(|l| !l.is_empty()) else {
        return NO_ITEM_SCORE;
    };
    let description = accessor.item_description(item);
    let key = cache_key(
        &label,
        description.as_deref(),
        allow_non_contiguous_matches,
        query,
    );
    if let Some(cached) = cache.get(&key) {
        return cached.clone();
    }
    let path = accessor.item_path(item);
    let score = score_item(
        &label,
        description.as_deref(),
        path.as_deref(),
        query,
        allow_non_contiguous_matches,
    );
    cache.insert(key, score.clone());
    score
}

/// 不经访问器与缓存的条目打分（热路径用）。
pub fn score_item(
    label: &str,
    description: Option<&str>,
    path: Option<&str>,
    query: &PreparedQuery,
    allow_non_contiguous_matches: bool,
) -> ItemScore {
    if label.is_empty() || query.normalized.is_empty() {
        return NO_ITEM_SCORE;
    }
    let description = description.filter(|d| !d.is_empty());
    let path = path.filter(|p| !p.is_empty());
    do_score_item_fuzzy(label, description, path, query, allow_non_contiguous_matches)
}

fn equals_ignore_case(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

fn do_score_item_fuzzy(
    label: &str,
    description: Option<&str>,
    path: Option<&str>,
    query: &PreparedQuery,
    allow_non_contiguous_matches: bool,
) -> ItemScore {
    let prefer_label_matches = path.is_none() || !query.contains_path_separator;

    // 完整路径一致的最高
    if let Some(path) = path {
        if equals_ignore_case(&query.path_normalized, path) {
            return ItemScore {
                score: PATH_IDENTITY_SCORE,
                label_match: Some(vec![Match {
                    start: 0,
                    end: utf16_len(label),
                }]),
                description_match: description.map(|d| {
                    vec![Match {
                        start: 0,
                        end: utf16_len(d),
                    }]
                }),
            };
        }
    }

    if let Some(values) = query.values.as_ref().filter(|v| v.len() > 1) {
        return do_score_item_fuzzy_multiple(
            label,
            description,
            path,
            values,
            prefer_label_matches,
            allow_non_contiguous_matches,
        );
    }
    do_score_item_fuzzy_single(
        label,
        description,
        path,
        &query.piece,
        prefer_label_matches,
        allow_non_contiguous_matches,
    )
}

fn do_score_item_fuzzy_multiple(
    label: &str,
    description: Option<&str>,
    path: Option<&str>,
    query: &[QueryPiece],
    prefer_label_matches: bool,
    allow_non_contiguous_matches: bool,
) -> ItemScore {
    let mut total_score = 0;
    let mut total_label_matches = Vec::new();
    let mut total_description_matches = Vec::new();
    for piece in query {
        let s = do_score_item_fuzzy_single(
            label,
            description,
            path,
            piece,
            prefer_label_matches,
            allow_non_contiguous_matches,
        );
        if s.score == NO_MATCH {
            return NO_ITEM_SCORE;
        }
        total_score += s.score;
        if let Some(m) = s.label_match {
            total_label_matches.extend(m);
        }
        if let Some(m) = s.description_match {
            total_description_matches.extend(m);
        }
    }
    ItemScore {
        score: total_score,
        label_match: Some(normalize_matches(total_label_matches)),
        description_match: Some(normalize_matches(total_description_matches)),
    }
}

fn do_score_item_fuzzy_single(
    label: &str,
    description: Option<&str>,
    path: Option<&str>,
    query: &QueryPiece,
    prefer_label_matches: bool,
    allow_non_contiguous_matches: bool,
) -> ItemScore {
    let allow = allow_non_contiguous_matches && !query.expect_contiguous_match;

    // 要求优先 label 或没有 description 时先只对 label 打分
    if prefer_label_matches || description.is_none() {
        let label_units = units(label);
        let scored = score_fuzzy_units(
            &label_units,
            &lower_units(label),
            &query.normalized_units,
            &query.normalized_lower_units,
            allow,
        );
        if scored.score != 0 {
            // label 前缀匹配给高得多的基础分，让输入文件名的结果排在“label 中间包含”的前面
            let label_prefix_match = matches_prefix(&query.normalized, label);
            let base_score = if label_prefix_match.is_some() {
                // 再按“查询占 label 的比例”加分：window 查询里 window.ts 要比 windowActions.ts 高
                let prefix_length_boost = ((query.normalized_units.len() as f64
                    / label_units.len() as f64)
                    * 100.0)
                    .round() as i32;
                LABEL_PREFIX_SCORE_THRESHOLD + prefix_length_boost
            } else {
                LABEL_SCORE_THRESHOLD
            };
            return ItemScore {
                score: base_score + scored.score,
                label_match: Some(
                    label_prefix_match.unwrap_or_else(|| create_matches_from(&scored.positions)),
                ),
                description_match: None,
            };
        }
    }

    // 最后对 description + label 一起打分
    if let Some(description) = description {
        let description_prefix: Cow<str> = if path.is_some() {
            Cow::Owned(format!("{description}{SEP}")) // 视为文件路径
        } else {
            Cow::Borrowed(description)
        };
        let description_prefix_length = utf16_len(&description_prefix);
        let description_and_label = format!("{description_prefix}{label}");
        let scored = score_fuzzy_units(
            &units(&description_and_label),
            &lower_units(&description_and_label),
            &query.normalized_units,
            &query.normalized_lower_units,
            allow,
        );
        if scored.score != 0 {
            let mut label_match = Vec::new();
            let mut description_match = Vec::new();
            for h in create_matches_from(&scored.positions) {
                if h.start < description_prefix_length && h.end > description_prefix_length {
                    // 区间跨越 label 与 description：拆开
                    label_match.push(Match {
                        start: 0,
                        end: h.end - description_prefix_length,
                    });
                    description_match.push(Match {
                        start: h.start,
                        end: description_prefix_length,
                    });
                } else if h.start >= description_prefix_length {
                    label_match.push(Match {
                        start: h.start - description_prefix_length,
                        end: h.end - description_prefix_length,
                    });
                } else {
                    description_match.push(h);
                }
            }
            return ItemScore {
                score: scored.score,
                label_match: Some(label_match),
                description_match: Some(description_match),
            };
        }
    }
    NO_ITEM_SCORE
}

fn create_matches_from(offsets: &[usize]) -> Vec<Match> {
    let mut ret: Vec<Match> = Vec::new();
    for &pos in offsets {
        match ret.last_mut() {
            Some(last) if last.end == pos => last.end += 1,
            _ => ret.push(Match {
                start: pos,
                end: pos + 1,
            }),
        }
    }
    ret
}

/// 排序并合并重叠的区间。
fn normalize_matches(mut matches: Vec<Match>) -> Vec<Match> {
    matches.sort_by_key(|m| m.start);
    let mut normalized: Vec<Match> = Vec::new();
    for m in matches {
        match normalized.last_mut() {
            Some(current) if match_overlaps(current, &m) => {
                current.start = current.start.min(m.start);
                current.end = current.end.max(m.end);
            }
            _ => normalized.push(m),
        }
    }
    normalized
}

fn match_overlaps(a: &Match, b: &Match) -> bool {
    !(a.end < b.start || b.end < a.start)
}

// ---------------------------------------------------------------------------------------------
// 比较器

/// `compareItemsByFuzzyScore`：返回值与 JS 比较器同号（负数表示 `a` 排在前面）。
pub fn compare_items_by_fuzzy_score<T: ?Sized>(
    item_a: &T,
    item_b: &T,
    query: &PreparedQuery,
    allow_non_contiguous_matches: bool,
    accessor: &dyn ItemAccessor<T>,
    cache: &mut FuzzyScorerCache,
) -> Ordering {
    let score_a = score_item_fuzzy(
        Some(item_a),
        query,
        allow_non_contiguous_matches,
        accessor,
        cache,
    );
    let score_b = score_item_fuzzy(
        Some(item_b),
        query,
        allow_non_contiguous_matches,
        accessor,
        cache,
    );
    let label_a = accessor.item_label(item_a);
    let label_b = accessor.item_label(item_b);
    let description_a = accessor.item_description(item_a);
    let description_b = accessor.item_description(item_b);
    let path_a = accessor.item_path(item_a);
    let path_b = accessor.item_path(item_b);
    compare_scored(
        &Scored {
            score: &score_a,
            label: label_a.as_deref().unwrap_or(""),
            description: description_a.as_deref(),
            path: path_a.as_deref(),
        },
        &Scored {
            score: &score_b,
            label: label_b.as_deref().unwrap_or(""),
            description: description_b.as_deref(),
            path: path_b.as_deref(),
        },
        query,
    )
}

/// 比较器需要的条目信息。
pub struct Scored<'a> {
    pub score: &'a ItemScore,
    pub label: &'a str,
    pub description: Option<&'a str>,
    pub path: Option<&'a str>,
}

/// 已经打好分的两个条目的比较（`compareItemsByFuzzyScore` 的后半段）。
pub fn compare_scored(a: &Scored, b: &Scored, query: &PreparedQuery) -> Ordering {
    let score_a = a.score.score;
    let score_b = b.score.score;

    // 1.) 完整路径一致最高
    if (score_a == PATH_IDENTITY_SCORE || score_b == PATH_IDENTITY_SCORE) && score_a != score_b {
        return if score_a == PATH_IDENTITY_SCORE {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    // 2.) label 上的匹配高于 label + description 上的匹配
    if score_a > LABEL_SCORE_THRESHOLD || score_b > LABEL_SCORE_THRESHOLD {
        if score_a != score_b {
            return if score_a > score_b {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }

        // 更紧凑的匹配优先（前缀匹配除外：那里匹配更长的前缀反而优先）
        if score_a < LABEL_PREFIX_SCORE_THRESHOLD && score_b < LABEL_PREFIX_SCORE_THRESHOLD {
            let by_length =
                compare_by_match_length(a.score.label_match.as_deref(), b.score.label_match.as_deref());
            if by_length != Ordering::Equal {
                return by_length;
            }
        }

        // 更短的 label 优先
        let (la, lb) = (utf16_len(a.label), utf16_len(b.label));
        if la != lb {
            return la.cmp(&lb);
        }
    }

    // 3.) 按 label + description 的分数
    if score_a != score_b {
        return if score_a > score_b {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    // 4.) 分数相同：label 上有匹配的优先
    let a_has = a.score.label_match.as_ref().is_some_and(|m| !m.is_empty());
    let b_has = b.score.label_match.as_ref().is_some_and(|m| !m.is_empty());
    if a_has && !b_has {
        return Ordering::Less;
    } else if b_has && !a_has {
        return Ordering::Greater;
    }

    // 5.) 分数相同：更紧凑的匹配（label 与 description）优先
    let distance_a = label_and_description_match_distance(a);
    let distance_b = label_and_description_match_distance(b);
    if distance_a != 0 && distance_b != 0 && distance_a != distance_b {
        return if distance_b > distance_a {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    // 6.) 其余比较
    fallback_compare(a, b, query)
}

fn non_empty(m: &Option<Vec<Match>>) -> Option<&Vec<Match>> {
    m.as_ref().filter(|v| !v.is_empty())
}

fn label_and_description_match_distance(item: &Scored) -> i64 {
    let score = item.score;
    let mut match_start: i64 = -1;
    let mut match_end: i64 = -1;

    if let Some(d) = non_empty(&score.description_match) {
        match_start = d[0].start as i64;
    } else if let Some(l) = non_empty(&score.label_match) {
        match_start = l[0].start as i64;
    }

    if let Some(l) = non_empty(&score.label_match) {
        match_end = l[l.len() - 1].end as i64;
        if non_empty(&score.description_match).is_some() {
            if let Some(d) = item.description.filter(|d| !d.is_empty()) {
                match_end += utf16_len(d) as i64;
            }
        }
    } else if let Some(d) = non_empty(&score.description_match) {
        match_end = d[d.len() - 1].end as i64;
    }
    match_end - match_start
}

fn compare_by_match_length(a: Option<&[Match]>, b: Option<&[Match]>) -> Ordering {
    let a = a.filter(|m| !m.is_empty());
    let b = b.filter(|m| !m.is_empty());
    let (a, b) = match (a, b) {
        (None, None) => return Ordering::Equal,
        (Some(_), None) => return Ordering::Less,
        (None, Some(_)) => return Ordering::Greater,
        (Some(a), Some(b)) => (a, b),
    };
    let length_a = a[a.len() - 1].end - a[0].start;
    let length_b = b[b.len() - 1].end - b[0].start;
    // 更短的匹配优先
    match length_a.cmp(&length_b) {
        Ordering::Equal => Ordering::Equal,
        Ordering::Greater => Ordering::Greater,
        Ordering::Less => Ordering::Less,
    }
}

fn fallback_compare(a: &Scored, b: &Scored, query: &PreparedQuery) -> Ordering {
    // label + description 总长度短的优先
    let len_a = utf16_len(a.label) + a.description.map_or(0, utf16_len);
    let len_b = utf16_len(b.label) + b.description.map_or(0, utf16_len);
    if len_a != len_b {
        return len_a.cmp(&len_b);
    }

    // 路径短的优先
    if let (Some(pa), Some(pb)) = (a.path.filter(|p| !p.is_empty()), b.path.filter(|p| !p.is_empty())) {
        let (la, lb) = (utf16_len(pa), utf16_len(pb));
        if la != lb {
            return la.cmp(&lb);
        }
    }

    // 7.) 分数与长度都相同：按比较器
    if a.label != b.label {
        return compare_anything(a.label, b.label, &query.normalized).cmp(&0);
    }
    if let (Some(da), Some(db)) = (
        a.description.filter(|d| !d.is_empty()),
        b.description.filter(|d| !d.is_empty()),
    ) {
        if da != db {
            return compare_anything(da, db, &query.normalized).cmp(&0);
        }
    }
    if let (Some(pa), Some(pb)) = (a.path.filter(|p| !p.is_empty()), b.path.filter(|p| !p.is_empty())) {
        if pa != pb {
            return compare_anything(pa, pb, &query.normalized).cmp(&0);
        }
    }
    Ordering::Equal
}
