//! 移植 VS Code `src/vs/base/common/fuzzyScorer.ts`：`scoreFuzzy`、`scoreFuzzy2`、`scoreItemFuzzy`、
//! `compareItemsByFuzzyScore`、`prepareQuery`。分数、区间与排序规则逐条照搬，不做“更合理”的改动。
//!
//! 与 TypeScript 一致，字符串按 UTF-16 码元处理，区间是码元下标（需要字节区间时用
//! [`crate::utf16_to_byte_ranges`]）。Windows 路径分隔符为 `\`（`path.sep`）。

use crate::{
    compare::compare_anything,
    filters::{
        create_matches, fuzzy_score, is_upper, lower_units, matches_prefix, units,
        FuzzyScoreOptions, Match,
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
    /// 当前这一行里与查询字符相等的位置。
    hits: Vec<usize>,
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

/// 被打分的文本：按位置取原字符与小写字符（UTF-16 码元）。
/// `Wide` 是通用的（预先算好小写数组），`Ascii` 是纯 ASCII 的字节（小写现算，数据更紧凑）。
pub(crate) trait Text {
    /// 把所有与小写字符 `ql` 相等（`consider_as_equal`）的位置按升序追加到 `out`。
    fn collect_equal(&self, ql: u16, out: &mut Vec<usize>) {
        for i in 0..self.len() {
            if consider_as_equal(ql, self.low(i)) {
                out.push(i);
            }
        }
    }
    fn len(&self) -> usize;
    fn at(&self, i: usize) -> u16;
    fn low(&self, i: usize) -> u16;
}

struct Wide<'a> {
    orig: &'a [u16],
    low: &'a [u16],
}

impl Text for Wide<'_> {
    #[inline]
    fn len(&self) -> usize {
        self.orig.len()
    }
    #[inline]
    fn at(&self, i: usize) -> u16 {
        self.orig[i]
    }
    #[inline]
    fn low(&self, i: usize) -> u16 {
        self.low[i]
    }
}

struct Ascii<'a>(&'a [u8]);

impl Text for Ascii<'_> {
    fn collect_equal(&self, ql: u16, out: &mut Vec<usize>) {
        // ASCII 文本：用 SIMD 直接找到所有等价的字节
        if ql >= 128 {
            return;
        }
        let q = ql as u8;
        let (a, b) = if q == b'/' || q == b'\\' {
            (b'/', b'\\')
        } else {
            (q, q.to_ascii_uppercase())
        };
        out.extend(memchr::memchr2_iter(a, b, self.0));
    }
    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }
    #[inline]
    fn at(&self, i: usize) -> u16 {
        u16::from(self.0[i])
    }
    #[inline]
    fn low(&self, i: usize) -> u16 {
        u16::from(self.0[i].to_ascii_lowercase())
    }
}

fn starts_with_at<T: Text>(target: &T, at: usize, query_lower: &[u16]) -> bool {
    at + query_lower.len() <= target.len()
        && query_lower
            .iter()
            .enumerate()
            .all(|(k, &q)| target.low(at + k) == q)
}

/// 字符集合的 64 位掩码（只依赖字符本身，`/` 与 `\\` 视为同一个字符）：查询里出现的字符，
/// 目标里必须都出现过——这是“查询是目标的子序列”的必要条件，用来在打分前以几纳秒否决绝大多数候选。
fn char_mask(units: &[u16]) -> u64 {
    let mut mask = 0u64;
    for &u in units {
        let bit = match u {
            0x61..=0x7A => u - 0x61,
            0x30..=0x39 => 26 + (u - 0x30),
            0x2F | 0x5C => 36,
            0x2E => 37,
            0x5F => 38,
            0x2D => 39,
            0x20 => 40,
            _ => 63,
        };
        mask |= 1u64 << bit;
    }
    mask
}

fn score_fuzzy_units(
    target: &[u16],
    target_lower: &[u16],
    query: &[u16],
    query_lower: &[u16],
    allow_non_contiguous_matches: bool,
) -> Score {
    score_fuzzy_units_with(
        target,
        target_lower,
        query,
        query_lower,
        allow_non_contiguous_matches,
        true,
    )
}

/// `want_positions = false` 时只算分数（不恢复匹配位置、不分配）。
fn score_fuzzy_units_with(
    target: &[u16],
    target_lower: &[u16],
    query: &[u16],
    query_lower: &[u16],
    allow_non_contiguous_matches: bool,
    want_positions: bool,
) -> Score {
    let text = Wide {
        orig: target,
        low: target_lower,
    };
    if want_positions {
        let mut positions = Vec::new();
        let score = score_core(
            &text,
            query,
            query_lower,
            allow_non_contiguous_matches,
            Some(&mut positions),
        );
        Score { score, positions }
    } else {
        Score {
            score: score_core(
                &text,
                query,
                query_lower,
                allow_non_contiguous_matches,
                None,
            ),
            positions: Vec::new(),
        }
    }
}

/// 打分的核心：`out` 不为 `None` 时写入（升序的）匹配位置。
fn score_core<T: Text>(
    target: &T,
    query: &[u16],
    query_lower: &[u16],
    allow_non_contiguous_matches: bool,
    mut out: Option<&mut Vec<usize>>,
) -> i32 {
    if let Some(out) = out.as_deref_mut() {
        out.clear();
    }
    let target_length = target.len();
    let query_length = query.len();
    if target_length == 0 || query_length == 0 || target_length < query_length {
        return 0;
    }

    // 单个字符的查询：矩阵退化成“各匹配位置字符得分的最大值”（最后一个取得最大值的位置，因为矩阵里
    // 分数不低于左边就算有效），不必建矩阵。
    if query_length == 1 {
        let (mut best, mut best_at) = (0, 0);
        let mut found = Vec::new();
        target.collect_equal(query_lower[0], &mut found);
        for i in found {
            // 不允许不连续匹配时（引号包住的查询）要求精确相等（`startsWith`，不做路径分隔符等价）
            if !allow_non_contiguous_matches && !starts_with_at(target, i, query_lower) {
                continue;
            }
            let s = compute_char_score(query[0], query_lower[0], target, i, 0);
            if s != 0 && s >= best {
                best = s;
                best_at = i;
            }
        }
        if best != 0 {
            if let Some(out) = out {
                out.push(best_at);
            }
        }
        return best;
    }

    // 快速否决（不改变结果）：查询必须按顺序出现在目标中，否则矩阵里不会产生任何分数。
    {
        let mut qi = 0;
        for i in 0..target_length {
            if consider_as_equal(query_lower[qi], target.low(i)) {
                qi += 1;
                if qi == query_length {
                    break;
                }
            }
        }
        if qi < query_length {
            return 0;
        }
    }

    MATRIX.with(|m| {
        let mut m = m.borrow_mut();
        let cells = query_length * target_length;
        m.scores.clear();
        m.scores.resize(cells, 0);
        m.matches.clear();
        m.matches.resize(cells, 0);
        let Matrix {
            scores,
            matches,
            hits,
        } = &mut *m;

        for query_index in 0..query_length {
            let query_index_offset = query_index * target_length;
            let query_index_previous_offset = query_index_offset.wrapping_sub(target_length);
            let query_index_gt_null = query_index > 0;
            let query_char = query[query_index];
            let query_lower_char = query_lower[query_index];

            // 与查询字符不相等的格子没有分数：直接沿用左边的分数（`matches` 已经是 0）。
            // 所以只需要逐个计算相等的位置，其余的整段填充。
            hits.clear();
            target.collect_equal(query_lower_char, hits);
            let mut filled = 0usize;
            let mut carried = 0i32;
            for &target_index in hits.iter() {
                if target_index > filled {
                    scores[query_index_offset + filled..query_index_offset + target_index]
                        .fill(carried);
                }
                let target_index_gt_null = target_index > 0;
                let current_index = query_index_offset + target_index;
                let diag_index = query_index_previous_offset
                    .wrapping_add(target_index)
                    .wrapping_sub(1);

                let left_score = carried;
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
                        target_index,
                        matches_sequence_length,
                    )
                };

                let is_valid_score = score != 0 && diag_score + score >= left_score;
                if is_valid_score
                    && (allow_non_contiguous_matches
                        || query_index_gt_null
                        || starts_with_at(target, target_index, query_lower))
                {
                    matches[current_index] = matches_sequence_length + 1;
                    scores[current_index] = diag_score + score;
                } else {
                    matches[current_index] = NO_MATCH as u32;
                    scores[current_index] = left_score;
                }
                carried = scores[current_index];
                filled = target_index + 1;
            }
            if filled < target_length {
                scores[query_index_offset + filled..query_index_offset + target_length]
                    .fill(carried);
            }
        }

        let Some(positions) = out else {
            return scores[cells - 1];
        };
        // 从矩阵右下角恢复位置
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
        scores[cells - 1]
    })
}

#[inline]
fn compute_char_score<T: Text>(
    query_char: u16,
    query_lower_char: u16,
    target: &T,
    target_index: usize,
    matches_sequence_length: u32,
) -> i32 {
    if !consider_as_equal(query_lower_char, target.low(target_index)) {
        return 0;
    }
    let mut score = 1;

    // 连续匹配奖励：前 3 个满额（6），其余减半（3）
    if matches_sequence_length > 0 {
        let n = matches_sequence_length as i32;
        score += n.min(3) * 6 + (n - 3).max(0) * 3;
    }

    // 大小写相同奖励
    if query_char == target.at(target_index) {
        score += 1;
    }

    if target_index == 0 {
        // 词首奖励
        score += 8;
    } else {
        // 分隔符之后的奖励
        let separator_bonus = score_separator_at(target.at(target_index - 1));
        if separator_bonus != 0 {
            score += separator_bonus;
        } else if is_upper(target.at(target_index)) && matches_sequence_length == 0 {
            // 单词内的大写（驼峰），只在不处于连续序列时给：NPE => NullPointerException 加分，HTTP => HTTP 不加
            score += 2;
        }
    }
    score
}

fn score_separator_at(code: u16) -> i32 {
    match code {
        0x2F | 0x5C => 5,                                    // / \
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
    /// `char_mask(normalized_lower_units)`。
    mask: u64,
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

impl PreparedQuery {
    /// 整条查询（含各段与规整后的路径）都是纯 ASCII：可以用 [`score_ascii_summary`]。
    pub fn is_ascii(&self) -> bool {
        self.piece.path_normalized.is_ascii()
            && self.piece.normalized.is_ascii()
            && self.values.as_ref().is_none_or(|v| {
                v.iter()
                    .all(|p| p.normalized.is_ascii() && p.path_normalized.is_ascii())
            })
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
    let normalized_lower_units = units(&normalized_lowercase);
    QueryPiece {
        original: original.to_owned(),
        original_lowercase: original.to_lowercase(),
        expect_contiguous_match: query_expects_exact_match(original),
        normalized_units: units(&normalized),
        mask: char_mask(&normalized_lower_units),
        normalized_lower_units,
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

/// 预先算好 UTF-16 与小写形式的候选：热路径（每次按键对全部候选打分）不再逐次分配。
#[derive(Clone, Debug)]
pub struct Target {
    label: String,
    description: Option<String>,
    path: Option<String>,
    /// 一块连续内存：`label` 的 UTF-16 与小写形式、`description + SEP + label`（没有路径时为
    /// `description + label`）的 UTF-16 与小写形式，依次排列。按键时对全部候选打分，候选的数据放在一起，
    /// 避免每个候选四次缓存未命中。
    data: Box<[u16]>,
    /// 四段在 `data` 里的结束位置。
    ends: [u32; 4],
    /// 组合串里 description 前缀的长度（UTF-16 码元）。
    prefix_len: usize,
    label_mask: u64,
    combined_mask: u64,
    /// `path` 的字节数与是否纯 ASCII：完整路径一致的检查先比这两个，不碰堆上的字符串。
    path_len: u32,
    path_ascii: bool,
    /// UTF-16 长度（比较器要用，放在这里避免对候选逐个数字符）。
    description_len: u32,
    path_len16: u32,
}

impl Target {
    pub fn new(label: &str, description: Option<&str>, path: Option<&str>) -> Self {
        let description = description.filter(|d| !d.is_empty());
        let path = path.filter(|p| !p.is_empty());
        let (combined_units, combined_lower, prefix_len) = match description {
            Some(d) => {
                let prefix = if path.is_some() {
                    format!("{d}{SEP}")
                } else {
                    d.to_owned()
                };
                let combined = format!("{prefix}{label}");
                (units(&combined), lower_units(&combined), utf16_len(&prefix))
            }
            None => (Vec::new(), Vec::new(), 0),
        };
        let label_units = units(label);
        let label_lower = lower_units(label);
        let mut data: Vec<u16> = Vec::with_capacity(
            label_units.len() + label_lower.len() + combined_units.len() + combined_lower.len(),
        );
        let mut ends = [0u32; 4];
        for (i, part) in [&label_units, &label_lower, &combined_units, &combined_lower]
            .into_iter()
            .enumerate()
        {
            data.extend_from_slice(part);
            ends[i] = data.len() as u32;
        }
        Self {
            label: label.to_owned(),
            description: description.map(str::to_owned),
            path: path.map(str::to_owned),
            label_mask: char_mask(&label_lower),
            combined_mask: char_mask(&combined_lower),
            data: data.into_boxed_slice(),
            ends,
            prefix_len,
            path_len: path.map_or(0, |p| p.len() as u32),
            path_ascii: path.is_some_and(str::is_ascii),
            description_len: description.map_or(0, |d| utf16_len(d) as u32),
            path_len16: path.map_or(0, |p| utf16_len(p) as u32),
        }
    }
    /// 比较器用的条目信息（`summary` 是对这个候选打分的结果）。
    pub fn cmp_item<'a>(&'a self, summary: &Summary) -> CmpItem<'a> {
        CmpItem {
            score: summary.score,
            label_span: summary.label,
            description_span: summary.description,
            label: &self.label,
            label_len: self.ends[0] as usize,
            description: self.description.as_deref(),
            description_len: self.description_len as usize,
            path: self.path.as_deref(),
            path_len: self.path_len16 as usize,
        }
    }
    fn label_units(&self) -> &[u16] {
        &self.data[..self.ends[0] as usize]
    }
    fn label_lower(&self) -> &[u16] {
        &self.data[self.ends[0] as usize..self.ends[1] as usize]
    }
    fn combined_units(&self) -> &[u16] {
        &self.data[self.ends[1] as usize..self.ends[2] as usize]
    }
    fn combined_lower(&self) -> &[u16] {
        &self.data[self.ends[2] as usize..self.ends[3] as usize]
    }
    /// 完整路径是否与查询（规整后）一致（忽略大小写）。
    fn path_equals(&self, query_path: &str) -> bool {
        let Some(path) = self.path.as_deref() else {
            return false;
        };
        // 两边都是 ASCII 时长度不同必然不等：不用碰堆上的路径字符串
        if self.path_ascii && query_path.is_ascii() && self.path_len as usize != query_path.len() {
            return false;
        }
        equals_ignore_case(query_path, path)
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

/// 不经访问器与缓存的条目打分。
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
    score_target(
        &Target::new(label, description, path),
        query,
        allow_non_contiguous_matches,
    )
}

/// 只算分数，结果与 `score_target(..).score` 相同，但不生成匹配区间（不分配）。
/// 每次按键要对全部候选打分，大多数候选的分数低于当前第 N 名，用它先筛掉。
pub fn score_target_value(
    target: &Target,
    query: &PreparedQuery,
    allow_non_contiguous_matches: bool,
) -> i32 {
    if target.label_units().is_empty() || query.normalized.is_empty() {
        return 0;
    }
    let prefer_label_matches = target.path.is_none() || !query.contains_path_separator;
    if target.path_equals(&query.path_normalized) {
        return PATH_IDENTITY_SCORE;
    }
    if let Some(values) = query.values.as_ref().filter(|v| v.len() > 1) {
        let mut total = 0;
        for piece in values {
            let s = value_single(
                target,
                piece,
                prefer_label_matches,
                allow_non_contiguous_matches,
            );
            if s == NO_MATCH {
                return 0;
            }
            total += s;
        }
        return total;
    }
    value_single(
        target,
        &query.piece,
        prefer_label_matches,
        allow_non_contiguous_matches,
    )
}

fn value_single(
    target: &Target,
    query: &QueryPiece,
    prefer_label_matches: bool,
    allow_non_contiguous_matches: bool,
) -> i32 {
    let allow = allow_non_contiguous_matches && !query.expect_contiguous_match;
    if (prefer_label_matches || target.description.is_none())
        && query.mask & !target.label_mask == 0
    {
        let scored = score_fuzzy_units_with(
            target.label_units(),
            target.label_lower(),
            &query.normalized_units,
            &query.normalized_lower_units,
            allow,
            false,
        );
        if scored.score != 0 {
            let base_score = if label_has_prefix(target, query) {
                let boost = ((query.normalized_units.len() as f64
                    / target.label_units().len() as f64)
                    * 100.0)
                    .round() as i32;
                LABEL_PREFIX_SCORE_THRESHOLD + boost
            } else {
                LABEL_SCORE_THRESHOLD
            };
            return base_score + scored.score;
        }
    }
    if target.description.is_some() && query.mask & !target.combined_mask == 0 {
        return score_fuzzy_units_with(
            target.combined_units(),
            target.combined_lower(),
            &query.normalized_units,
            &query.normalized_lower_units,
            allow,
            false,
        )
        .score;
    }
    0
}

fn label_has_prefix(target: &Target, query: &QueryPiece) -> bool {
    if target.label_lower().len() != target.label_units().len() {
        return matches_prefix(&query.normalized, &target.label).is_some();
    }
    let n = query.normalized_units.len();
    target.label_lower().len() >= n && target.label_lower()[..n] == query.normalized_lower_units[..]
}

/// 打分结果的摘要：分数与匹配区间的首尾。比较器只用得到这些，所以不必为每个候选生成完整的区间列表。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub score: i32,
    /// label 内匹配的（起点, 终点）；没有匹配时为 `None`。
    pub label: Option<(usize, usize)>,
    /// description 内匹配的（起点, 终点）。
    pub description: Option<(usize, usize)>,
}

thread_local! {
    static POSITIONS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

fn merge_span(into: &mut Option<(usize, usize)>, span: Option<(usize, usize)>) {
    if let Some((s, e)) = span {
        *into = Some(match *into {
            Some((a, b)) => (a.min(s), b.max(e)),
            None => (s, e),
        });
    }
}

/// 对候选打分，只返回摘要：不分配，分数与 [`score_target`] 相同，区间首尾与它的 `label_match` /
/// `description_match` 的首尾相同。
pub fn score_target_summary(
    target: &Target,
    query: &PreparedQuery,
    allow_non_contiguous_matches: bool,
) -> Summary {
    if target.label_units().is_empty() || query.normalized.is_empty() {
        return Summary::default();
    }
    let prefer_label_matches = target.path.is_none() || !query.contains_path_separator;
    if target.path_equals(&query.path_normalized) {
        return Summary {
            score: PATH_IDENTITY_SCORE,
            label: Some((0, target.ends[0] as usize)),
            description: target
                .description
                .as_ref()
                .map(|_| (0, target.description_len as usize)),
        };
    }
    if let Some(values) = query.values.as_ref().filter(|v| v.len() > 1) {
        let mut total = Summary::default();
        for piece in values {
            let s = summary_single(
                target,
                piece,
                prefer_label_matches,
                allow_non_contiguous_matches,
            );
            if s.score == NO_MATCH {
                return Summary::default();
            }
            total.score += s.score;
            merge_span(&mut total.label, s.label);
            merge_span(&mut total.description, s.description);
        }
        return total;
    }
    summary_single(
        target,
        &query.piece,
        prefer_label_matches,
        allow_non_contiguous_matches,
    )
}

fn summary_single(
    target: &Target,
    query: &QueryPiece,
    prefer_label_matches: bool,
    allow_non_contiguous_matches: bool,
) -> Summary {
    let label = Wide {
        orig: target.label_units(),
        low: target.label_lower(),
    };
    let combined = Wide {
        orig: target.combined_units(),
        low: target.combined_lower(),
    };
    summary_generic(
        &label,
        target.description.is_some().then_some(&combined),
        target.prefix_len,
        (target.label_mask, target.combined_mask),
        query,
        prefer_label_matches,
        allow_non_contiguous_matches,
        || label_has_prefix(target, query),
    )
}

/// 对一个查询段打分并返回摘要（label 与 `description + 分隔符 + label` 两种文本）。
#[allow(clippy::too_many_arguments)]
fn summary_generic<T: Text>(
    label: &T,
    combined: Option<&T>,
    prefix_len: usize,
    (label_mask, combined_mask): (u64, u64),
    query: &QueryPiece,
    prefer_label_matches: bool,
    allow_non_contiguous_matches: bool,
    has_prefix: impl FnOnce() -> bool,
) -> Summary {
    let allow = allow_non_contiguous_matches && !query.expect_contiguous_match;
    POSITIONS.with(|positions| {
        let mut positions = positions.borrow_mut();
        if (prefer_label_matches || combined.is_none()) && query.mask & !label_mask == 0 {
            let score = score_core(
                label,
                &query.normalized_units,
                &query.normalized_lower_units,
                allow,
                Some(&mut positions),
            );
            if score != 0 {
                let (base, span) = if has_prefix() {
                    let n = query.normalized_units.len();
                    let boost = ((n as f64 / label.len() as f64) * 100.0).round() as i32;
                    (LABEL_PREFIX_SCORE_THRESHOLD + boost, (0, n))
                } else {
                    (
                        LABEL_SCORE_THRESHOLD,
                        (positions[0], positions[positions.len() - 1] + 1),
                    )
                };
                return Summary {
                    score: base + score,
                    label: Some(span),
                    description: None,
                };
            }
        }
        if let Some(combined) = combined.filter(|_| query.mask & !combined_mask == 0) {
            let score = score_core(
                combined,
                &query.normalized_units,
                &query.normalized_lower_units,
                allow,
                Some(&mut positions),
            );
            if score != 0 {
                let mut label_span: Option<(usize, usize)> = None;
                let mut description: Option<(usize, usize)> = None;
                for &p in positions.iter() {
                    if p < prefix_len {
                        merge_span(&mut description, Some((p, p + 1)));
                    } else {
                        merge_span(&mut label_span, Some((p - prefix_len, p - prefix_len + 1)));
                    }
                }
                return Summary {
                    score,
                    label: label_span,
                    description,
                };
            }
        }
        Summary::default()
    })
}

/// 纯 ASCII 候选的字符集合掩码：(label 的, 整条路径的)。`text` 是 `description + 分隔符 + label`
/// （没有 description 时就是 label），`label_start` 是 label 的起点。
pub fn ascii_masks(text: &[u8], label_start: usize) -> (u64, u64) {
    let lower = |bytes: &[u8]| {
        let units: Vec<u16> = bytes
            .iter()
            .map(|b| u16::from(b.to_ascii_lowercase()))
            .collect();
        char_mask(&units)
    };
    (lower(&text[label_start..]), lower(text))
}

/// 对纯 ASCII 的候选打分（查询也必须是纯 ASCII，见 [`PreparedQuery::is_ascii`]）：结果与对同一条路径用
/// [`score_target_summary`] 打分相同，但数据只有一段连续的字节。
pub fn score_ascii_summary(
    text: &[u8],
    label_start: usize,
    masks: (u64, u64),
    query: &PreparedQuery,
    allow_non_contiguous_matches: bool,
) -> Summary {
    let label_bytes = &text[label_start..];
    if label_bytes.is_empty() || query.normalized.is_empty() {
        return Summary::default();
    }
    let has_description = label_start > 0;
    let description_len = label_start.saturating_sub(1);
    // 路径（相对路径）总是有的：查询里有路径分隔符时不优先只对 label 打分
    let prefer_label_matches = !query.contains_path_separator;
    if text.eq_ignore_ascii_case(query.path_normalized.as_bytes()) {
        return Summary {
            score: PATH_IDENTITY_SCORE,
            label: Some((0, label_bytes.len())),
            description: has_description.then_some((0, description_len)),
        };
    }
    let label = Ascii(label_bytes);
    let combined = Ascii(text);
    let one = |piece: &QueryPiece| {
        summary_generic(
            &label,
            has_description.then_some(&combined),
            label_start,
            masks,
            piece,
            prefer_label_matches,
            allow_non_contiguous_matches,
            || {
                let n = piece.normalized_units.len();
                n <= label.len()
                    && piece
                        .normalized_lower_units
                        .iter()
                        .enumerate()
                        .all(|(k, &q)| label.low(k) == q)
            },
        )
    };
    if let Some(values) = query.values.as_ref().filter(|v| v.len() > 1) {
        let mut total = Summary::default();
        for piece in values {
            let s = one(piece);
            if s.score == NO_MATCH {
                return Summary::default();
            }
            total.score += s.score;
            merge_span(&mut total.label, s.label);
            merge_span(&mut total.description, s.description);
        }
        return total;
    }
    one(&query.piece)
}

/// 对预先准备好的候选打分（热路径用）。
pub fn score_target(
    target: &Target,
    query: &PreparedQuery,
    allow_non_contiguous_matches: bool,
) -> ItemScore {
    if target.label_units().is_empty() || query.normalized.is_empty() {
        return NO_ITEM_SCORE;
    }
    do_score_item_fuzzy(target, query, allow_non_contiguous_matches)
}

fn equals_ignore_case(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    // 热路径（对每个候选都比较一次路径）：ASCII 不分配
    if a.is_ascii() && b.is_ascii() {
        return a.eq_ignore_ascii_case(b);
    }
    a.to_lowercase() == b.to_lowercase()
}

fn do_score_item_fuzzy(
    target: &Target,
    query: &PreparedQuery,
    allow_non_contiguous_matches: bool,
) -> ItemScore {
    let (label, description, path) = (
        target.label.as_str(),
        target.description.as_deref(),
        target.path.as_deref(),
    );
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
            target,
            values,
            prefer_label_matches,
            allow_non_contiguous_matches,
        );
    }
    do_score_item_fuzzy_single(
        target,
        &query.piece,
        prefer_label_matches,
        allow_non_contiguous_matches,
    )
}

fn do_score_item_fuzzy_multiple(
    target: &Target,
    query: &[QueryPiece],
    prefer_label_matches: bool,
    allow_non_contiguous_matches: bool,
) -> ItemScore {
    let mut total_score = 0;
    let mut total_label_matches = Vec::new();
    let mut total_description_matches = Vec::new();
    for piece in query {
        let s = do_score_item_fuzzy_single(
            target,
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
    target: &Target,
    query: &QueryPiece,
    prefer_label_matches: bool,
    allow_non_contiguous_matches: bool,
) -> ItemScore {
    let allow = allow_non_contiguous_matches && !query.expect_contiguous_match;

    // 要求优先 label 或没有 description 时先只对 label 打分
    if (prefer_label_matches || target.description.is_none())
        && query.mask & !target.label_mask == 0
    {
        let label_units = target.label_units();
        let scored = score_fuzzy_units(
            label_units,
            target.label_lower(),
            &query.normalized_units,
            &query.normalized_lower_units,
            allow,
        );
        if scored.score != 0 {
            // label 前缀匹配给高得多的基础分，让输入文件名的结果排在“label 中间包含”的前面
            let label_prefix_match = label_prefix(target, query);
            let base_score = if label_prefix_match.is_some() {
                // 再按“查询占 label 的比例”加分：window 查询里 window.ts 要比 windowActions.ts 高
                let prefix_length_boost =
                    ((query.normalized_units.len() as f64 / label_units.len() as f64) * 100.0)
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
    if target.description.is_some() && query.mask & !target.combined_mask == 0 {
        // 前缀是 description（有路径时再加分隔符：视为文件路径）
        let description_prefix_length = target.prefix_len;
        let scored = score_fuzzy_units(
            target.combined_units(),
            target.combined_lower(),
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

/// `matchesPrefix(query.normalized, label)`：用预先算好的小写形式比较（长度不一致时回退）。
fn label_prefix(target: &Target, query: &QueryPiece) -> Option<Vec<Match>> {
    if target.label_lower().len() != target.label_units().len() {
        return matches_prefix(&query.normalized, &target.label);
    }
    let n = query.normalized_units.len();
    if target.label_lower().len() < n
        || target.label_lower()[..n] != query.normalized_lower_units[..]
    {
        return None;
    }
    Some(if n == 0 {
        Vec::new()
    } else {
        vec![Match { start: 0, end: n }]
    })
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

/// 比较器需要的条目信息：匹配区间只用到首尾，长度是 UTF-16 码元数。
#[derive(Clone, Copy, Debug)]
pub struct CmpItem<'a> {
    pub score: i32,
    pub label_span: Option<(usize, usize)>,
    pub description_span: Option<(usize, usize)>,
    pub label: &'a str,
    pub label_len: usize,
    pub description: Option<&'a str>,
    pub description_len: usize,
    pub path: Option<&'a str>,
    pub path_len: usize,
}

/// 比较器用到的整数部分（不含字符串）：热路径把它们存在候选旁边，比较时不再去取字符串。
#[derive(Clone, Copy, Debug)]
pub struct CmpKey {
    pub score: i32,
    pub label_span: Option<(usize, usize)>,
    pub description_span: Option<(usize, usize)>,
    pub label_len: usize,
    pub description_len: usize,
    pub path_len: usize,
    /// 有非空的路径。
    pub has_path: bool,
}

/// 比较器的最后几步才需要的字符串。
#[derive(Clone, Copy, Debug)]
pub struct CmpTexts<'a> {
    pub label: &'a str,
    pub description: Option<&'a str>,
    pub path: Option<&'a str>,
}

impl CmpItem<'_> {
    fn key(&self) -> CmpKey {
        CmpKey {
            score: self.score,
            label_span: self.label_span,
            description_span: self.description_span,
            label_len: self.label_len,
            description_len: self.description_len,
            path_len: self.path_len,
            has_path: self.path.is_some_and(|p| !p.is_empty()),
        }
    }

    fn texts(&self) -> CmpTexts<'_> {
        CmpTexts {
            label: self.label,
            description: self.description,
            path: self.path,
        }
    }
}

fn span_of(matches: &Option<Vec<Match>>) -> Option<(usize, usize)> {
    matches
        .as_ref()
        .filter(|v| !v.is_empty())
        .map(|v| (v[0].start, v[v.len() - 1].end))
}

fn cmp_item<'a>(item: &Scored<'a>) -> CmpItem<'a> {
    CmpItem {
        score: item.score.score,
        label_span: span_of(&item.score.label_match),
        description_span: span_of(&item.score.description_match),
        label: item.label,
        label_len: utf16_len(item.label),
        description: item.description,
        description_len: item.description.map_or(0, utf16_len),
        path: item.path,
        path_len: item.path.map_or(0, utf16_len),
    }
}

/// 已经打好分的两个条目的比较（`compareItemsByFuzzyScore` 的后半段）。
pub fn compare_scored(a: &Scored, b: &Scored, query: &PreparedQuery) -> Ordering {
    compare_items(&cmp_item(a), &cmp_item(b), query)
}

/// `compare_scored` 的实现，只依赖 [`CmpItem`]（O(1)，不扫描字符串）。
pub fn compare_items(a: &CmpItem, b: &CmpItem, query: &PreparedQuery) -> Ordering {
    compare_keys(&a.key(), &b.key(), query, || (a.texts(), b.texts()))
}

/// 比较器本体：整数部分决定绝大多数比较，只有走到最后几步才调用 `texts` 取字符串。
pub fn compare_keys<'a>(
    a: &CmpKey,
    b: &CmpKey,
    query: &PreparedQuery,
    texts: impl FnOnce() -> (CmpTexts<'a>, CmpTexts<'a>),
) -> Ordering {
    let score_a = a.score;
    let score_b = b.score;

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
            let by_length = compare_by_match_length(a.label_span, b.label_span);
            if by_length != Ordering::Equal {
                return by_length;
            }
        }

        // 更短的 label 优先
        if a.label_len != b.label_len {
            return a.label_len.cmp(&b.label_len);
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
    let a_has = a.label_span.is_some();
    let b_has = b.label_span.is_some();
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
    fallback_compare(a, b, query, texts)
}

fn label_and_description_match_distance(item: &CmpKey) -> i64 {
    let mut match_start: i64 = -1;
    let mut match_end: i64 = -1;

    // 有 description 匹配时起点是它的第一个匹配，否则是 label 的第一个匹配
    if let Some((start, _)) = item.description_span {
        match_start = start as i64;
    } else if let Some((start, _)) = item.label_span {
        match_start = start as i64;
    }

    // 有 label 匹配时终点是它的最后一个匹配（若同时有 description 匹配，再加上 description 的长度）
    if let Some((_, end)) = item.label_span {
        match_end = end as i64;
        if item.description_span.is_some() && item.description_len > 0 {
            match_end += item.description_len as i64;
        }
    } else if let Some((_, end)) = item.description_span {
        match_end = end as i64;
    }
    match_end - match_start
}

fn compare_by_match_length(a: Option<(usize, usize)>, b: Option<(usize, usize)>) -> Ordering {
    let (a, b) = match (a, b) {
        (None, None) => return Ordering::Equal,
        (Some(_), None) => return Ordering::Less,
        (None, Some(_)) => return Ordering::Greater,
        (Some(a), Some(b)) => (a, b),
    };
    // 更短的匹配优先
    (a.1 - a.0).cmp(&(b.1 - b.0))
}

fn fallback_compare<'a>(
    a: &CmpKey,
    b: &CmpKey,
    query: &PreparedQuery,
    texts: impl FnOnce() -> (CmpTexts<'a>, CmpTexts<'a>),
) -> Ordering {
    // label + description 总长度短的优先
    let len_a = a.label_len + a.description_len;
    let len_b = b.label_len + b.description_len;
    if len_a != len_b {
        return len_a.cmp(&len_b);
    }

    // 路径短的优先
    if a.has_path && b.has_path && a.path_len != b.path_len {
        return a.path_len.cmp(&b.path_len);
    }

    // 7.) 分数与长度都相同：按比较器
    let (ta, tb) = texts();
    if ta.label != tb.label {
        return compare_anything(ta.label, tb.label, &query.normalized).cmp(&0);
    }
    if let (Some(da), Some(db)) = (
        ta.description.filter(|d| !d.is_empty()),
        tb.description.filter(|d| !d.is_empty()),
    ) {
        if da != db {
            return compare_anything(da, db, &query.normalized).cmp(&0);
        }
    }
    if let (Some(pa), Some(pb)) = (
        ta.path.filter(|p| !p.is_empty()),
        tb.path.filter(|p| !p.is_empty()),
    ) {
        if pa != pb {
            return compare_anything(pa, pb, &query.normalized).cmp(&0);
        }
    }
    Ordering::Equal
}
