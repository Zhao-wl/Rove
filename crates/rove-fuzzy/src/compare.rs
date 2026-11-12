//! 移植 VS Code `src/vs/base/common/comparers.ts` 中 `fuzzyScorer` 排序用到的部分：
//! `compareAnything`、`compareByPrefix`、`compareFileNames`。
//!
//! VS Code 的 `compareFileNames` 用 `Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })`。
//! 这里没有 ICU：数字串按数值比较，字母忽略大小写与重音，标点按 ICU 根排序的大致次序
//! （标点 < 数字 < 字母），其余字符按码点。只影响“分数与长度都相同”之后的最终决胜顺序。

use std::cmp::Ordering;

/// `compareAnything(one, other, lookFor)`：返回负数、0、正数。
pub fn compare_anything(one: &str, other: &str, look_for: &str) -> i32 {
    let a = one.to_lowercase();
    let b = other.to_lowercase();

    // 前缀匹配排在非前缀匹配之前
    let prefix = compare_by_prefix(one, other, look_for);
    if prefix != 0 {
        return prefix;
    }

    // 后缀匹配排在非后缀匹配之前
    let a_suffix = a.ends_with(look_for);
    let b_suffix = b.ends_with(look_for);
    if a_suffix != b_suffix {
        return if a_suffix { -1 } else { 1 };
    }

    // 按文件名规则
    let r = compare_file_names(&a, &b);
    if r != 0 {
        return r;
    }

    // `localeCompare` 的兜底
    ord(a.cmp(&b))
}

/// `compareByPrefix`。
pub fn compare_by_prefix(one: &str, other: &str, look_for: &str) -> i32 {
    let a = one.to_lowercase();
    let b = other.to_lowercase();
    let a_prefix = a.starts_with(look_for);
    let b_prefix = b.starts_with(look_for);
    if a_prefix != b_prefix {
        return if a_prefix { -1 } else { 1 };
    }
    // 同为前缀匹配：短的排前面，更精确的在上
    if a_prefix && b_prefix {
        let (la, lb) = (utf16_len(&a), utf16_len(&b));
        if la < lb {
            return -1;
        }
        if la > lb {
            return 1;
        }
    }
    0
}

/// `compareFileNames`（数字按数值、忽略大小写；相等但字符串不同时按码点决胜）。
pub fn compare_file_names(one: &str, other: &str) -> i32 {
    let result = collate(one, other);
    if result == 0 && one != other {
        return if one < other { -1 } else { 1 };
    }
    result
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

fn ord(o: Ordering) -> i32 {
    match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// ICU 根排序中 ASCII 标点的大致次序。
const PUNCTUATION_ORDER: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

fn char_key(c: char) -> (u8, u32) {
    if c.is_whitespace() {
        return (0, c as u32);
    }
    if let Some(i) = PUNCTUATION_ORDER.find(c) {
        return (1, i as u32);
    }
    if c.is_ascii_digit() {
        return (3, c as u32);
    }
    // 字母：忽略大小写（重音只做常见拉丁字母的折叠）
    let lower = c.to_lowercase().next().unwrap_or(c);
    let folded = fold_accent(lower);
    if folded.is_alphabetic() {
        return (4, folded as u32);
    }
    (2, c as u32)
}

fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        c => c,
    }
}

fn collate(a: &str, b: &str) -> i32 {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return 0,
            (None, Some(_)) => return -1,
            (Some(_), None) => return 1,
            (Some(x), Some(y)) => {
                if x.is_ascii_digit() && y.is_ascii_digit() {
                    let mut da = String::new();
                    while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                        da.push(c);
                        ai.next();
                    }
                    let mut db = String::new();
                    while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                        db.push(c);
                        bi.next();
                    }
                    let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                    let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                    if o != Ordering::Equal {
                        return ord(o);
                    }
                } else {
                    let o = char_key(x).cmp(&char_key(y));
                    if o != Ordering::Equal {
                        return ord(o);
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_compare_by_value_and_case_is_ignored() {
        assert_eq!(compare_file_names("file2", "file10"), -1);
        assert_eq!(compare_file_names("File", "file"), -1); // 相等时按码点决胜
        assert_eq!(compare_file_names("a", "b"), -1);
        assert_eq!(compare_file_names("a.b", "a1"), -1, "标点排在数字前");
        assert_eq!(compare_file_names("a1", "ab"), -1, "数字排在字母前");
    }

    #[test]
    fn prefix_and_suffix_preferences() {
        assert_eq!(compare_anything("foo.ts", "xfoo.ts", "foo"), -1);
        assert_eq!(compare_anything("foo.ts", "foo2.ts", "foo"), -1, "同为前缀时短的在前");
        assert_eq!(compare_anything("xa", "ab", "a"), 1);
    }
}
