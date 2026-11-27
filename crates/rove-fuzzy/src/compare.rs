//! 移植 VS Code `src/vs/base/common/comparers.ts` 中 `fuzzyScorer` 排序用到的部分：
//! `compareAnything`、`compareByPrefix`、`compareFileNames`。
//!
//! VS Code 的 `compareFileNames` 用 `Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })`。
//! 这里没有 ICU：数字串按数值比较，字母忽略大小写与重音，标点按 ICU 根排序的大致次序
//! （标点 < 数字 < 字母），其余字符按码点。只影响“分数与长度都相同”之后的最终决胜顺序。

use std::cmp::Ordering;

/// `compareAnything(one, other, lookFor)`：返回负数、0、正数。
pub fn compare_anything(one: &str, other: &str, look_for: &str) -> i32 {
    if one.is_ascii() && other.is_ascii() && look_for.is_ascii() {
        return compare_anything_ascii(one.as_bytes(), other.as_bytes(), look_for.as_bytes());
    }
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

// The lookup is already normalized by callers where needed. Fold only candidate bytes.
fn ascii_matches_lower(candidate: &[u8], look_for: &[u8]) -> bool {
    candidate
        .iter()
        .zip(look_for)
        .all(|(byte, query)| byte.to_ascii_lowercase() == *query)
}

fn compare_anything_ascii(one: &[u8], other: &[u8], look_for: &[u8]) -> i32 {
    let prefix = |candidate: &[u8]| {
        candidate
            .get(..look_for.len())
            .is_some_and(|head| ascii_matches_lower(head, look_for))
    };
    let a_prefix = prefix(one);
    let b_prefix = prefix(other);
    if a_prefix != b_prefix {
        return ord(b_prefix.cmp(&a_prefix));
    }
    // If both match, ASCII byte length is the same as folded UTF-16 length.
    if a_prefix && one.len() != other.len() {
        return ord(one.len().cmp(&other.len()));
    }
    let suffix = |candidate: &[u8]| {
        candidate
            .len()
            .checked_sub(look_for.len())
            .is_some_and(|start| ascii_matches_lower(&candidate[start..], look_for))
    };
    let by_suffix = suffix(other).cmp(&suffix(one));
    if by_suffix != Ordering::Equal {
        return ord(by_suffix);
    }
    let by_name = collate_ascii(one, other);
    if by_name != Ordering::Equal {
        return ord(by_name);
    }
    // Same fallback as compare_file_names on fully folded strings, including leading zero ties.
    ord(one
        .iter()
        .map(u8::to_ascii_lowercase)
        .cmp(other.iter().map(u8::to_ascii_lowercase)))
}

fn ascii_number(bytes: &[u8]) -> (&[u8], &[u8]) {
    let end = bytes
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(bytes.len());
    let (digits, rest) = bytes.split_at(end);
    let significant = digits
        .iter()
        .position(|byte| *byte != b'0')
        .unwrap_or(digits.len());
    (&digits[significant..], rest)
}

fn collate_ascii(mut one: &[u8], mut other: &[u8]) -> Ordering {
    loop {
        match (one.split_first(), other.split_first()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((&x, tail_a)), Some((&y, tail_b))) => {
                if x.is_ascii_digit() && y.is_ascii_digit() {
                    let (a, rest_a) = ascii_number(one);
                    let (b, rest_b) = ascii_number(other);
                    let digits = a.len().cmp(&b.len()).then_with(|| a.cmp(b));
                    if digits != Ordering::Equal {
                        return digits;
                    }
                    one = rest_a;
                    other = rest_b;
                } else {
                    if x != y {
                        // Keep Unicode's whitespace classification for ASCII controls such as VT.
                        let key = |byte: u8| char_key(char::from(byte.to_ascii_lowercase()));
                        let order = key(x).cmp(&key(y));
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    one = tail_a;
                    other = tail_b;
                }
            }
        }
    }
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
        assert_eq!(
            compare_anything("foo.ts", "foo2.ts", "foo"),
            -1,
            "同为前缀时短的在前"
        );
        assert_eq!(compare_anything("xa", "ab", "a"), 1);
    }

    // The previous compare_anything implementation, with the original
    // Unicode prefix / collation paths, stays independent of the ASCII branch.
    fn legacy_anything(one: &str, other: &str, look_for: &str) -> i32 {
        let a = one.to_lowercase();
        let b = other.to_lowercase();
        let prefix = compare_by_prefix(one, other, look_for);
        if prefix != 0 {
            return prefix;
        }
        let a_suffix = a.ends_with(look_for);
        let b_suffix = b.ends_with(look_for);
        if a_suffix != b_suffix {
            return if a_suffix { -1 } else { 1 };
        }
        let name = compare_file_names(&a, &b);
        if name != 0 {
            return name;
        }
        ord(a.cmp(&b))
    }

    #[test]
    fn compare_anything_ascii_exactly_matches_the_original_order() {
        let verify = |left: &str, right: &str, query: &str| {
            assert_eq!(
                compare_anything(left, right, query),
                legacy_anything(left, right, query),
                "left={left:?} right={right:?} query={query:?}"
            );
        };

        let bytes: Vec<String> = (0u8..=127)
            .map(|byte| char::from(byte).to_string())
            .collect();
        for a in &bytes {
            for b in &bytes {
                let left = format!("a{a}001Z");
                let right = format!("A{b}1z");
                verify(a, b, "");
                verify(a, b, a);
                verify(&left, &right, "z");
                verify(&left, &right, "Z");
                verify(&left, &right, "A");
            }
        }
        let long = [
            format!("file{}1.cs", "0".repeat(1024)),
            format!("file{}.cs", "9".repeat(1024)),
            format!("file1{}.cs", "0".repeat(1024)),
            format!("x{}a0007Y", "0".repeat(1024)),
            format!("X{}a7y", "0".repeat(1023)),
        ];
        let corpus: Vec<&str> = [
            "",
            "file0.cs",
            "file00.cs",
            "File001.cs",
            "file1.cs",
            "file2.cs",
            "file10.cs",
            "A0z1",
            "a00z2",
            "dir\\FiLe9.CS",
            "dir/File09.cs",
            "π1",
            "π9",
            "Écho2",
            "E\u{301}cho10",
            "İX",
            "IX",
            "ΟΣ",
            "οσ",
            "ος",
            "𐐀Name",
            "𐐨Name",
            "P😀2",
            "Pabc",
            "角色2",
            "角色10",
            "Ａ2",
            "Ａ10",
            "\x0b0",
            "\x1c0",
            "\0",
        ]
        .into_iter()
        .chain(long.iter().map(String::as_str))
        .collect();
        for a in &corpus {
            for b in &corpus {
                for query in [
                    "",
                    "file",
                    "FILE",
                    ".cs",
                    ".CS",
                    "dir",
                    "Dir",
                    "x",
                    "X",
                    "π",
                    "é",
                    "e\u{301}",
                    "i",
                    "i\u{307}",
                    "p",
                    "角色",
                    "not a match",
                ] {
                    verify(a, b, query);
                }
            }
        }

        assert_eq!(compare_anything("pz", "Apx", "p"), -1);
        assert_eq!(
            compare_anything("pz", "Apx", "P"),
            1,
            "prefix lookup remains case-sensitive"
        );
        assert_eq!(compare_anything("zB", "aa", "b"), -1);
        assert_eq!(
            compare_anything("zB", "aa", "B"),
            1,
            "suffix lookup remains case-sensitive"
        );
        assert_eq!(
            compare_anything("a\x0b2", "a\x1c2", "?"),
            -1,
            "keep the VT whitespace class"
        );
        assert_eq!(
            compare_anything("f002x3", "f2x10", "?"),
            -1,
            "equal numbers advance both runs"
        );
        assert_eq!(
            compare_anything("FILE02", "file2", "?"),
            -1,
            "folded codepoint tie"
        );
    }
}
