//! 移植 VS Code `src/vs/base/common/filters.ts` 中 `fuzzyScorer.ts` 用到的部分：
//! `fuzzyScore`（符号搜索与 `scoreFuzzy2` 使用的矩阵打分）、`matchesPrefix`、`createMatches`。
//!
//! 与 TypeScript 一致，字符串按 UTF-16 码元处理：位置和区间都是码元下标。
//! 需要字节区间的调用方用 [`crate::utf16_to_byte_ranges`] 转换。

use std::cell::RefCell;

/// 目标字符串的区间（UTF-16 码元，左闭右开）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

const MAX_LEN: usize = 128;
const STRIDE: usize = MAX_LEN + 1;
const SENTINEL: i32 = i32::MIN;

/// `FuzzyScoreOptions`。
#[derive(Clone, Copy, Debug)]
pub struct FuzzyScoreOptions {
    pub first_match_can_be_weak: bool,
    pub boost_full_match: bool,
}

impl Default for FuzzyScoreOptions {
    fn default() -> Self {
        Self {
            first_match_can_be_weak: false,
            boost_full_match: true,
        }
    }
}

/// `FuzzyScore`：分数、匹配起点与（升序的）匹配位置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyScore {
    pub score: i32,
    pub word_start: usize,
    /// 已加上 `word_start` 的绝对位置，升序。
    pub positions: Vec<usize>,
}

struct Tables {
    min_word_match_pos: [usize; 2 * MAX_LEN + 1],
    max_word_match_pos: [usize; 2 * MAX_LEN + 1],
    diag: Vec<i32>,
    table: Vec<i32>,
    arrows: Vec<u8>,
}

impl Tables {
    fn new() -> Self {
        Self {
            min_word_match_pos: [0; 2 * MAX_LEN + 1],
            max_word_match_pos: [0; 2 * MAX_LEN + 1],
            diag: vec![0; STRIDE * STRIDE],
            table: vec![0; STRIDE * STRIDE],
            arrows: vec![0; STRIDE * STRIDE],
        }
    }
}

thread_local! {
    static TABLES: RefCell<Tables> = RefCell::new(Tables::new());
}

const ARROW_DIAG: u8 = 1;
const ARROW_LEFT: u8 = 2;
const ARROW_LEFT_LEFT: u8 = 3;

#[inline]
fn at(row: usize, column: usize) -> usize {
    row * STRIDE + column
}

pub(crate) fn lower_units(s: &str) -> Vec<u16> {
    s.to_lowercase().encode_utf16().collect()
}

pub(crate) fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[inline]
fn is_upper_unit(code: u16) -> bool {
    (b'A' as u16..=b'Z' as u16).contains(&code)
}

pub(crate) fn is_upper(code: u16) -> bool {
    is_upper_unit(code)
}

/// `isUpperCaseAtPos`：`word[pos] !== wordLow[pos]`（越界视为相等，与 JS 的 `undefined` 一致）。
fn is_upper_case_at(pos: usize, word: &[u16], word_low: &[u16]) -> bool {
    match (word.get(pos), word_low.get(pos)) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    }
}

/// `strings.isEmojiImprecise`。
fn is_emoji_imprecise(x: u32) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&x)
        || x == 8986
        || x == 8987
        || x == 9200
        || x == 9203
        || (9728..=10175).contains(&x)
        || x == 11088
        || x == 11093
        || (127744..=128591).contains(&x)
        || (128640..=128764).contains(&x)
        || (128992..=129008).contains(&x)
        || (129280..=129535).contains(&x)
        || (129648..=129782).contains(&x)
}

/// `isSeparatorAtPos`（`value.codePointAt(index)`：代理对按整个码点判断）。
fn is_separator_at(value: &[u16], index: isize) -> bool {
    if index < 0 || index as usize >= value.len() {
        return false;
    }
    let index = index as usize;
    let unit = value[index];
    let code = if (0xD800..0xDC00).contains(&unit) {
        match value.get(index + 1) {
            Some(&low) if (0xDC00..0xE000).contains(&low) => {
                0x10000 + (((unit as u32) - 0xD800) << 10) + (low as u32 - 0xDC00)
            }
            _ => unit as u32,
        }
    } else {
        unit as u32
    };
    match code {
        0x5F | 0x2D | 0x2E | 0x20 | 0x2F | 0x5C | 0x27 | 0x22 | 0x3A | 0x24 | 0x3C | 0x3E
        | 0x28 | 0x29 | 0x5B | 0x5D | 0x7B | 0x7D => true,
        c => is_emoji_imprecise(c),
    }
}

fn is_whitespace_at(value: &[u16], index: isize) -> bool {
    if index < 0 || index as usize >= value.len() {
        return false;
    }
    matches!(value[index as usize], 0x20 | 0x09)
}

/// `isPatternInWord`：`pattern` 的字符是否按顺序出现在 `word` 中；可顺便记录每个位置的最小下标。
fn is_pattern_in_word(
    pattern_low: &[u16],
    mut pattern_pos: usize,
    pattern_len: usize,
    word_low: &[u16],
    mut word_pos: usize,
    word_len: usize,
    min_word_match_pos: Option<&mut [usize]>,
) -> bool {
    let mut fill = min_word_match_pos;
    while pattern_pos < pattern_len && word_pos < word_len {
        if pattern_low[pattern_pos] == word_low[word_pos] {
            if let Some(arr) = fill.as_deref_mut() {
                arr[pattern_pos] = word_pos;
            }
            pattern_pos += 1;
        }
        word_pos += 1;
    }
    pattern_pos == pattern_len
}

#[allow(clippy::too_many_arguments)]
fn do_score(
    pattern: &[u16],
    pattern_low: &[u16],
    pattern_pos: usize,
    pattern_start: usize,
    word: &[u16],
    word_low: &[u16],
    word_pos: usize,
    word_len: usize,
    word_start: usize,
    new_match_start: bool,
    out_first_match_strong: &mut bool,
) -> i32 {
    if pattern_low[pattern_pos] != word_low[word_pos] {
        return SENTINEL;
    }
    let mut score = 1;
    let mut is_gap_location = false;
    if word_pos == pattern_pos - pattern_start {
        // 公共前缀：`foobar <-> foobaz`
        score = if pattern[pattern_pos] == word[word_pos] {
            7
        } else {
            5
        };
    } else if is_upper_case_at(word_pos, word, word_low)
        && (word_pos == 0 || !is_upper_case_at(word_pos - 1, word, word_low))
    {
        // 命中大写：`foo <-> forOthers`
        score = if pattern[pattern_pos] == word[word_pos] {
            7
        } else {
            5
        };
        is_gap_location = true;
    } else if is_separator_at(word_low, word_pos as isize)
        && (word_pos == 0 || !is_separator_at(word_low, word_pos as isize - 1))
    {
        // 命中分隔符：`. <-> foo.bar`
        score = 5;
    } else if is_separator_at(word_low, word_pos as isize - 1)
        || is_whitespace_at(word_low, word_pos as isize - 1)
    {
        // 分隔符之后：`foo <-> bar_foo`
        score = 5;
        is_gap_location = true;
    }

    if score > 1 && pattern_pos == pattern_start {
        *out_first_match_strong = true;
    }

    if !is_gap_location {
        is_gap_location = is_upper_case_at(word_pos, word, word_low)
            || is_separator_at(word_low, word_pos as isize - 1)
            || is_whitespace_at(word_low, word_pos as isize - 1);
    }

    if pattern_pos == pattern_start {
        if word_pos > word_start {
            // 第一个模式字符不在词首：为前面的空隙扣分
            score -= if is_gap_location { 3 } else { 5 };
        }
    } else if new_match_start {
        // 新匹配段的开头（前面有空隙）
        score += if is_gap_location { 2 } else { 0 };
    } else {
        // 连续匹配的一部分：给一点奖励（但不奖励本来就是首选空隙位置的）
        score += if is_gap_location { 0 } else { 1 };
    }

    if word_pos + 1 == word_len {
        // 空隙总要扣分，这对匹配到词尾的情形不公平：假装词尾后还有一个空隙
        score -= if is_gap_location { 3 } else { 5 };
    }
    score
}

/// `fuzzyScore(pattern, patternLow, patternStart, word, wordLow, wordStart, options)`。
#[allow(clippy::too_many_arguments)]
pub fn fuzzy_score(
    pattern: &[u16],
    pattern_low: &[u16],
    pattern_start: usize,
    word: &[u16],
    word_low: &[u16],
    word_start: usize,
    options: FuzzyScoreOptions,
) -> Option<FuzzyScore> {
    let pattern_len = pattern.len().min(MAX_LEN);
    let word_len = word.len().min(MAX_LEN);
    if pattern_start >= pattern_len
        || word_start >= word_len
        || (pattern_len - pattern_start) > (word_len - word_start)
    {
        return None;
    }
    TABLES.with(|tables| {
        let mut guard = tables.borrow_mut();
        let t = &mut *guard;

        // 先做一次廉价检查：模式字符是否按顺序出现在词里；顺便记下每个位置的最小下标。
        if !is_pattern_in_word(
            pattern_low,
            pattern_start,
            pattern_len,
            word_low,
            word_start,
            word_len,
            Some(&mut t.min_word_match_pos[..]),
        ) {
            return None;
        }

        // 每个模式位置的最大词位置
        {
            let mut pattern_pos = pattern_len as isize - 1;
            let mut word_pos = word_len as isize - 1;
            while pattern_pos >= pattern_start as isize && word_pos >= word_start as isize {
                if pattern_low[pattern_pos as usize] == word_low[word_pos as usize] {
                    t.max_word_match_pos[pattern_pos as usize] = word_pos as usize;
                    pattern_pos -= 1;
                }
                word_pos -= 1;
            }
        }

        let mut row = 1usize;
        let mut column = 1usize;
        let mut has_strong_first_match = false;

        let mut pattern_pos = pattern_start;
        while pattern_pos < pattern_len {
            let min_word_match_pos = t.min_word_match_pos[pattern_pos];
            let max_word_match_pos = t.max_word_match_pos[pattern_pos];
            let next_max_word_match_pos = if pattern_pos + 1 < pattern_len {
                t.max_word_match_pos[pattern_pos + 1]
            } else {
                word_len
            };

            column = min_word_match_pos - word_start + 1;
            let mut word_pos = min_word_match_pos;
            while word_pos < next_max_word_match_pos {
                let mut score = SENTINEL;
                let mut can_come_diag = false;
                if word_pos <= max_word_match_pos {
                    score = do_score(
                        pattern,
                        pattern_low,
                        pattern_pos,
                        pattern_start,
                        word,
                        word_low,
                        word_pos,
                        word_len,
                        word_start,
                        t.diag[at(row - 1, column - 1)] == 0,
                        &mut has_strong_first_match,
                    );
                }

                let mut diag_score = 0;
                if score != SENTINEL {
                    can_come_diag = true;
                    diag_score = score + t.table[at(row - 1, column - 1)];
                }

                let can_come_left = word_pos > min_word_match_pos;
                let left_score = if can_come_left {
                    t.table[at(row, column - 1)]
                        + if t.diag[at(row, column - 1)] > 0 {
                            -5
                        } else {
                            0
                        }
                } else {
                    0
                };

                let can_come_left_left =
                    word_pos > min_word_match_pos + 1 && t.diag[at(row, column - 1)] > 0;
                let left_left_score = if can_come_left_left {
                    t.table[at(row, column - 2)]
                        + if t.diag[at(row, column - 2)] > 0 {
                            -5
                        } else {
                            0
                        }
                } else {
                    0
                };

                let cell = at(row, column);
                if can_come_left_left
                    && (!can_come_left || left_left_score >= left_score)
                    && (!can_come_diag || left_left_score >= diag_score)
                {
                    // 总是优先跳过一条对角线：匹配更靠前
                    t.table[cell] = left_left_score;
                    t.arrows[cell] = ARROW_LEFT_LEFT;
                    t.diag[cell] = 0;
                } else if can_come_left && (!can_come_diag || left_score >= diag_score) {
                    t.table[cell] = left_score;
                    t.arrows[cell] = ARROW_LEFT;
                    t.diag[cell] = 0;
                } else if can_come_diag {
                    t.table[cell] = diag_score;
                    t.arrows[cell] = ARROW_DIAG;
                    t.diag[cell] = t.diag[at(row - 1, column - 1)] + 1;
                } else {
                    unreachable!("not possible");
                }
                column += 1;
                word_pos += 1;
            }
            row += 1;
            pattern_pos += 1;
        }

        if !has_strong_first_match && !options.first_match_can_be_weak {
            return None;
        }

        row -= 1;
        column -= 1;

        let mut result_score = t.table[at(row, column)];
        let mut positions = Vec::with_capacity(row);
        let mut backwards_diag_length = 0usize;
        let mut max_match_column = 0usize;

        while row >= 1 {
            // 找到向左上走的那一列
            let mut diag_column = column;
            loop {
                match t.arrows[at(row, diag_column)] {
                    ARROW_LEFT_LEFT => diag_column -= 2,
                    ARROW_LEFT => diag_column -= 1,
                    _ => break,
                }
                if diag_column < 1 {
                    break;
                }
            }

            // 如果保留“向后”的对角线能得到更好的匹配，就推翻“向前”的决定
            if backwards_diag_length > 1
                && pattern_low[pattern_start + row - 1] == word_low[word_start + column - 1]
                && !is_upper_case_at(diag_column + word_start - 1, word, word_low)
                && backwards_diag_length as i32 + 1 > t.diag[at(row, diag_column)]
            {
                diag_column = column;
            }

            if diag_column == column {
                backwards_diag_length += 1;
            } else {
                backwards_diag_length = 1;
            }
            if max_match_column == 0 {
                max_match_column = diag_column;
            }
            row -= 1;
            column = diag_column - 1;
            positions.push(column + word_start);
        }
        positions.reverse();

        if word_len - word_start == pattern_len && options.boost_full_match {
            // 词与模式完全吻合：整体加分，让它排在其他词前面
            result_score += 2;
        }
        // 词里每跳过一个字符扣 1 分
        let skipped = max_match_column as i32 - pattern_len as i32;
        result_score -= skipped;

        Some(FuzzyScore {
            score: result_score,
            word_start,
            positions,
        })
    })
}

/// `createMatches(score)`：把位置合并成区间。
pub fn create_matches(score: Option<&FuzzyScore>) -> Vec<Match> {
    let mut res: Vec<Match> = Vec::new();
    let Some(score) = score else { return res };
    for &pos in &score.positions {
        match res.last_mut() {
            Some(last) if last.end == pos => last.end = pos + 1,
            _ => res.push(Match {
                start: pos,
                end: pos + 1,
            }),
        }
    }
    res
}

/// `matchesPrefix`（忽略大小写）：`word` 是 `target` 的前缀时返回覆盖前缀的区间。
pub fn matches_prefix(word: &str, target: &str) -> Option<Vec<Match>> {
    let word_units = units(word);
    let target_units = units(target);
    if target_units.is_empty() || target_units.len() < word_units.len() {
        return None;
    }
    let head = String::from_utf16_lossy(&target_units[..word_units.len()]);
    if head.to_lowercase() != word.to_lowercase() {
        return None;
    }
    Some(if word_units.is_empty() {
        Vec::new()
    } else {
        vec![Match {
            start: 0,
            end: word_units.len(),
        }]
    })
}
