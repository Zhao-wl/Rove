//! 移植自 VS Code fuzzyScorer.test.ts（MIT）。
//!
//! 逐条对应原测试，断言与期望值不变。只有三处按 Rust 实现改写：
//! - `scoreItem - cache key matches the hash of the scoring inputs`：验证 VS Code 内部 32 位哈希键，
//!   这里的缓存键是 64 位 `DefaultHasher`，实现不同，不移植（`#[ignore]` 空壳）。
//! - `cache keys separate items, queries and match modes`、`repeated scoring hits the cache and
//!   preserves the result`、`mutating a prepared query ...`：断言改成 `cache.len()` 与结果相等。
//!
//! 测试按 Windows 的语义写：`URI.file(p).fsPath` 是补前导 `/` 后把 `/` 换成 `\`；`basename` 与
//! `dirname` 按 `\` 取；`posix.normalize` / `win32.normalize` 只需要换分隔符。
use crate::{
    compare_items_by_fuzzy_score, piece_to_query, prepare_query, score_fuzzy as score_fuzzy_impl, score_fuzzy2,
    score_item_fuzzy, FuzzyScorerCache, ItemAccessor, ItemScore, Match, Score, Score2, SEP,
};
use std::borrow::Cow;

// ---------------------------------------------------------------------------------------------
// URI 与访问器

/// 只保留测试用到的 `URI`：`path`（带前导 `/` 的正斜杠路径）与 `fsPath`（Windows 形式）。
struct Uri {
    path: String,
    fs_path: String,
}

/// `URI.file(p)`（Windows）。
fn file(p: &str) -> Uri {
    let p = p.replace('\\', "/");
    let path = if p.starts_with('/') { p } else { format!("/{p}") };
    let fs_path = path.replace('/', "\\");
    Uri { path, fs_path }
}

/// `URI.from({ scheme: vscode-remote, path })`：fsPath 与本地一致。
fn remote(p: &str) -> Uri {
    file(p)
}

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

fn basename(p: &str) -> &str {
    p.rsplit(is_sep).next().unwrap_or(p)
}

fn dirname(p: &str) -> String {
    match p.rfind(is_sep) {
        None => ".".to_owned(),
        Some(0) => p[..1].to_owned(),
        Some(i) => p[..i].to_owned(),
    }
}

fn posix_normalize(p: &str) -> String {
    p.replace('\\', "/")
}

fn win32_normalize(p: &str) -> String {
    p.replace('/', "\\")
}

struct ResourceAccessor;

impl ItemAccessor<Uri> for ResourceAccessor {
    fn item_label<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        Some(Cow::Borrowed(basename(&r.fs_path)))
    }
    fn item_description<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        Some(Cow::Owned(dirname(&r.fs_path)))
    }
    fn item_path<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        Some(Cow::Borrowed(&r.fs_path))
    }
}

struct ResourceWithSlashAccessor;

impl ItemAccessor<Uri> for ResourceWithSlashAccessor {
    fn item_label<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        Some(Cow::Borrowed(basename(&r.fs_path)))
    }
    fn item_description<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        Some(Cow::Owned(posix_normalize(&dirname(&r.path))))
    }
    fn item_path<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        Some(Cow::Owned(posix_normalize(&r.path)))
    }
}

struct ResourceWithBackslashAccessor;

impl ItemAccessor<Uri> for ResourceWithBackslashAccessor {
    fn item_label<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        Some(Cow::Borrowed(basename(&r.fs_path)))
    }
    fn item_description<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        Some(Cow::Owned(win32_normalize(&dirname(&r.path))))
    }
    fn item_path<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        Some(Cow::Owned(win32_normalize(&r.path)))
    }
}

/// 三个方法都返回 `undefined`。
struct NullAccessor;

impl ItemAccessor<Uri> for NullAccessor {
    fn item_label<'a>(&self, _: &'a Uri) -> Option<Cow<'a, str>> {
        None
    }
    fn item_description<'a>(&self, _: &'a Uri) -> Option<Cow<'a, str>> {
        None
    }
    fn item_path<'a>(&self, _: &'a Uri) -> Option<Cow<'a, str>> {
        None
    }
}

/// `resourceAccessorWithDescription`：label 与 path 同 `ResourceAccessor`，description 固定。
struct AccessorWithDescription(Option<&'static str>);

fn resource_accessor_with_description(description: Option<&'static str>) -> AccessorWithDescription {
    AccessorWithDescription(description)
}

impl ItemAccessor<Uri> for AccessorWithDescription {
    fn item_label<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        ResourceAccessor.item_label(r)
    }
    fn item_description<'a>(&self, _: &'a Uri) -> Option<Cow<'a, str>> {
        self.0.map(Cow::Borrowed)
    }
    fn item_path<'a>(&self, r: &'a Uri) -> Option<Cow<'a, str>> {
        ResourceAccessor.item_path(r)
    }
}

// ---------------------------------------------------------------------------------------------
// 辅助函数

fn do_score(target: &str, query: &str, allow_non_contiguous_matches: Option<bool>) -> Score {
    let prepared = prepare_query(query);
    score_fuzzy_impl(
        target,
        &prepared.normalized,
        &prepared.normalized_lowercase,
        allow_non_contiguous_matches.unwrap_or(!prepared.expect_contiguous_match),
    )
}

fn do_score2(target: &str, query: &str, match_offset: usize) -> Score2 {
    let prepared = prepare_query(query);
    score_fuzzy2(target, &prepared, 0, match_offset)
}

fn score_item(
    item: Option<&Uri>,
    query: &str,
    allow_non_contiguous_matches: bool,
    accessor: &dyn ItemAccessor<Uri>,
) -> ItemScore {
    score_item_with_cache(item, query, allow_non_contiguous_matches, accessor, &mut FuzzyScorerCache::new())
}

fn score_item_with_cache(
    item: Option<&Uri>,
    query: &str,
    allow_non_contiguous_matches: bool,
    accessor: &dyn ItemAccessor<Uri>,
    cache: &mut FuzzyScorerCache,
) -> ItemScore {
    score_item_fuzzy(item, &prepare_query(query), allow_non_contiguous_matches, accessor, cache)
}

/// 用 `compareItemsByFuzzyScore`（每次比较新建缓存）排序，断言结果开头与 `expected` 逐个一致
/// （Rust 里没有对象恒等，用 fsPath 比较；各用例里的资源路径互不相同）。
fn assert_sorted(input: &[&Uri], query: &str, expected: &[&Uri]) {
    let prepared = prepare_query(query);
    let mut res: Vec<&Uri> = input.to_vec();
    res.sort_by(|a, b| {
        compare_items_by_fuzzy_score(
            *a,
            *b,
            &prepared,
            true,
            &ResourceAccessor,
            &mut FuzzyScorerCache::new(),
        )
    });
    let actual: Vec<&str> = res.iter().take(expected.len()).map(|r| r.fs_path.as_str()).collect();
    let wanted: Vec<&str> = expected.iter().map(|r| r.fs_path.as_str()).collect();
    assert_eq!(actual, wanted, "query {query:?}");
}

fn path_of(r: &Uri) -> String {
    ResourceAccessor.item_path(r).unwrap().into_owned()
}

fn label_of(r: &Uri) -> String {
    ResourceAccessor.item_label(r).unwrap().into_owned()
}

fn description_of(r: &Uri) -> String {
    ResourceAccessor.item_description(r).unwrap().into_owned()
}

fn len16(s: &str) -> usize {
    s.encode_utf16().count()
}

/// 断言区间数组存在且逐个 `start`/`end` 一致。
fn assert_matches(actual: &Option<Vec<Match>>, expected: &[(usize, usize)]) {
    let actual = actual.as_ref().expect("expected matches");
    assert_eq!(actual.len(), expected.len());
    for (m, (start, end)) in actual.iter().zip(expected) {
        assert_eq!(m.start, *start);
        assert_eq!(m.end, *end);
    }
}

/// JS 的 `assert.ok(score)`（数字为 0 或 undefined 都是假）。
fn truthy(score: Option<i32>) -> bool {
    score.is_some_and(|s| s != 0)
}

// ---------------------------------------------------------------------------------------------
// 测试

#[test]
fn score_fuzzy() {
    let target = "HelLo-World";

    let scores: Vec<Score> = vec![
        do_score(target, "HelLo-World", Some(true)), // direct case match
        do_score(target, "hello-world", Some(true)), // direct mix-case match
        do_score(target, "HW", Some(true)),          // direct case prefix (multiple)
        do_score(target, "hw", Some(true)),          // direct mix-case prefix (multiple)
        do_score(target, "H", Some(true)),           // direct case prefix
        do_score(target, "h", Some(true)),           // direct mix-case prefix
        do_score(target, "W", Some(true)),           // direct case word prefix
        do_score(target, "Ld", Some(true)),          // in-string case match (multiple)
        do_score(target, "ld", Some(true)), // in-string mix-case match (consecutive, avoids scattered hit)
        do_score(target, "w", Some(true)),  // direct mix-case word prefix
        do_score(target, "L", Some(true)),  // in-string case match
        do_score(target, "l", Some(true)),  // in-string mix-case match
        do_score(target, "4", Some(true)),  // no match
    ];

    // Assert scoring order（稳定排序，与 JS 的 Array.prototype.sort 一致）
    let mut sorted_scores = scores.clone();
    sorted_scores.sort_by(|a, b| b.score.cmp(&a.score));
    assert_eq!(scores, sorted_scores);

    // Assert scoring positions（原测试里这几条是注释掉的）
}

#[test]
fn score_non_fuzzy() {
    let target = "HelLo-World";

    assert!(do_score(target, "HelLo-World", Some(false)).score > 0);
    assert_eq!(do_score(target, "HelLo-World", Some(false)).positions.len(), "HelLo-World".len());

    assert!(do_score(target, "hello-world", Some(false)).score > 0);
    assert_eq!(do_score(target, "HW", Some(false)).score, 0);
    assert!(do_score(target, "h", Some(false)).score > 0);
    assert!(do_score(target, "ello", Some(false)).score > 0);
    assert!(do_score(target, "ld", Some(false)).score > 0);
    assert_eq!(do_score(target, "eo", Some(false)).score, 0);
}

#[test]
fn score_item_matches_are_proper() {
    let mut res = score_item(None, "something", true, &ResourceAccessor);
    assert_eq!(res.score, 0);

    let resource = file("/xyz/some/path/someFile123.txt");

    res = score_item(Some(&resource), "something", true, &NullAccessor);
    assert_eq!(res.score, 0);

    // Path Identity
    let identity_res = score_item(Some(&resource), &path_of(&resource), true, &ResourceAccessor);
    assert_ne!(identity_res.score, 0);
    assert_matches(&identity_res.description_match, &[(0, len16(&description_of(&resource)))]);
    assert_matches(&identity_res.label_match, &[(0, len16(&label_of(&resource)))]);

    // Basename Prefix
    let basename_prefix_res = score_item(Some(&resource), "som", true, &ResourceAccessor);
    assert_ne!(basename_prefix_res.score, 0);
    assert!(basename_prefix_res.description_match.is_none());
    assert_matches(&basename_prefix_res.label_match, &[(0, "som".len())]);

    // Basename Camelcase
    let basename_camelcase_res = score_item(Some(&resource), "sF", true, &ResourceAccessor);
    assert_ne!(basename_camelcase_res.score, 0);
    assert!(basename_camelcase_res.description_match.is_none());
    assert_matches(&basename_camelcase_res.label_match, &[(0, 1), (4, 5)]);

    // Basename Match
    let basename_res = score_item(Some(&resource), "of", true, &ResourceAccessor);
    assert_ne!(basename_res.score, 0);
    assert!(basename_res.description_match.is_none());
    assert_matches(&basename_res.label_match, &[(1, 2), (4, 5)]);

    // Path Match
    let path_res = score_item(Some(&resource), "xyz123", true, &ResourceAccessor);
    assert_ne!(path_res.score, 0);
    assert!(path_res.description_match.is_some());
    assert!(path_res.label_match.is_some());
    assert_matches(&path_res.label_match, &[(8, 11)]);
    assert_matches(&path_res.description_match, &[(1, 4)]);

    // Ellipsis Match
    let ellipsis_res =
        score_item(Some(&resource), "…me/path/someFile123.txt", true, &ResourceAccessor);
    assert_ne!(ellipsis_res.score, 0);
    assert!(path_res.description_match.is_some());
    assert!(path_res.label_match.is_some());
    assert_matches(&path_res.label_match, &[(8, 11)]);
    assert_matches(&path_res.description_match, &[(1, 4)]);

    // No Match
    let no_res = score_item(Some(&resource), "987", true, &ResourceAccessor);
    assert_eq!(no_res.score, 0);
    assert!(no_res.label_match.is_none());
    assert!(no_res.description_match.is_none());

    // No Exact Match
    let no_exact_res = score_item(Some(&resource), "\"sF\"", true, &ResourceAccessor);
    assert_eq!(no_exact_res.score, 0);
    assert!(no_exact_res.label_match.is_none());
    assert!(no_exact_res.description_match.is_none());
    assert_eq!(no_res.score, no_exact_res.score);

    // Verify Scores
    assert!(identity_res.score > basename_prefix_res.score);
    assert!(basename_prefix_res.score > basename_res.score);
    assert!(basename_res.score > path_res.score);
    assert!(path_res.score > no_res.score);
}

#[test]
fn score_item_multiple() {
    let resource = file("/xyz/some/path/someFile123.txt");

    let res1 = score_item(Some(&resource), "xyz some", true, &ResourceAccessor);
    assert_ne!(res1.score, 0);
    assert_matches(&res1.label_match, &[(0, 4)]);
    assert_matches(&res1.description_match, &[(1, 4)]);

    let res2 = score_item(Some(&resource), "some xyz", true, &ResourceAccessor);
    assert_ne!(res2.score, 0);
    assert_eq!(res1.score, res2.score);
    assert_matches(&res2.label_match, &[(0, 4)]);
    assert_matches(&res2.description_match, &[(1, 4)]);

    let res3 = score_item(Some(&resource), "some xyz file file123", true, &ResourceAccessor);
    assert_ne!(res3.score, 0);
    assert!(res3.score > res2.score);
    assert_matches(&res3.label_match, &[(0, 11)]);
    assert_matches(&res3.description_match, &[(1, 4)]);

    let res4 = score_item(Some(&resource), "path z y", true, &ResourceAccessor);
    assert_ne!(res4.score, 0);
    assert!(res4.score < res2.score);
    assert_matches(&res4.label_match, &[]);
    assert_matches(&res4.description_match, &[(2, 4), (10, 14)]);
}

#[test]
fn score_item_multiple_with_cache_yields_different_results() {
    let resource = file("/xyz/some/path/someFile123.txt");
    let mut cache = FuzzyScorerCache::new();
    let res1 =
        score_item_with_cache(Some(&resource), "xyz sm", true, &ResourceAccessor, &mut cache);
    assert_ne!(res1.score, 0);

    // from the cache's perspective this should be a totally different query
    let res2 =
        score_item_with_cache(Some(&resource), "xyz \"sm\"", true, &ResourceAccessor, &mut cache);
    assert_eq!(res2.score, 0);
}

// 原测试 `scoreItem - cache key matches the hash of the scoring inputs` 逐个校验 VS Code 内部
// `hash(...)`（32 位）算出的 `Object.keys(cache)` 键值。Rust 版缓存键是 64 位 `DefaultHasher`，
// 键的取值与实现细节不同，没有可比的期望值，所以不移植。键对各输入的区分能力由下面两个
// 行为测试（用 `cache.len()`）覆盖。
#[test]
#[ignore = "验证 VS Code 内部 32 位 hash 缓存键；Rust 版缓存键是 64 位 DefaultHasher，实现不同，不移植"]
fn score_item_cache_key_matches_the_hash_of_the_scoring_inputs() {}

// 改写：原测试用 `Object.keys(cache).length === 9`；这里用 `cache.len()`。
#[test]
fn score_item_cache_keys_separate_items_queries_and_match_modes() {
    let mut cache = FuzzyScorerCache::new();
    let acc = &ResourceAccessor;

    // same label, different description
    let r = file("/xyz/some/path/someFile123.txt");
    score_item_with_cache(Some(&r), "some", true, acc, &mut cache);
    let r = file("/xyz/other/path/someFile123.txt");
    score_item_with_cache(Some(&r), "some", true, acc, &mut cache);

    // same item, different queries (incl. multi piece and quoted/contiguous)
    let resource = file("/xyz/some/path/someFile123.txt");
    score_item_with_cache(Some(&resource), "somefile", true, acc, &mut cache);
    score_item_with_cache(Some(&resource), "xyz sm", true, acc, &mut cache);
    score_item_with_cache(Some(&resource), "xyz \"sm\"", true, acc, &mut cache);
    score_item_with_cache(Some(&resource), "sm xyz", true, acc, &mut cache);

    // same item and query, different match mode
    score_item_with_cache(Some(&resource), "somefile", false, acc, &mut cache);

    // an item without description
    let r = file("/someFile123.txt");
    let no_description = resource_accessor_with_description(None);
    score_item_with_cache(Some(&r), "some", true, &no_description, &mut cache);
    let empty_description = resource_accessor_with_description(Some(""));
    score_item_with_cache(Some(&r), "some", true, &empty_description, &mut cache);

    assert_eq!(cache.len(), 9, "each distinct scoring input needs its own cache entry");
}

// 改写：原测试用 `Object.keys(cache).length === 1` 与对象恒等；这里用 `cache.len()` 与结果相等。
#[test]
fn score_item_repeated_scoring_hits_the_cache_and_preserves_the_result() {
    let resource = file("/xyz/some/path/someFile123.txt");

    for query_value in ["somefile", "xyz sm", "xyz \"sm\"", "ünïcödé"] {
        let mut cache = FuzzyScorerCache::new();
        let cold =
            score_item_with_cache(Some(&resource), query_value, true, &ResourceAccessor, &mut cache);
        let warm =
            score_item_with_cache(Some(&resource), query_value, true, &ResourceAccessor, &mut cache);

        assert_eq!(cache.len(), 1);
        assert_eq!(warm, cold, "warm scoring should return the cached result");
    }
}

// 改写：`Object.keys(cache).length === 2` → `cache.len() == 2`。
#[test]
fn score_item_mutating_a_prepared_query_is_reflected_in_the_cache_key() {
    let resource = file("/xyz/some/path/someFile123.txt");
    let mut cache = FuzzyScorerCache::new();

    let mut query = prepare_query("xyz sm");
    let before = score_item_fuzzy(Some(&resource), &query, true, &ResourceAccessor, &mut cache);
    assert_ne!(before.score, 0);

    // callers own the prepared query object, so a mutated query must not reuse the previous entry
    query.values.as_mut().unwrap()[1].expect_contiguous_match = true;
    let after = score_item_fuzzy(Some(&resource), &query, true, &ResourceAccessor, &mut cache);

    assert_eq!(cache.len(), 2);
    assert_eq!(
        after,
        score_item_fuzzy(
            Some(&resource),
            &prepare_query("xyz \"sm\""),
            true,
            &ResourceAccessor,
            &mut FuzzyScorerCache::new()
        )
    );
}

#[test]
fn score_item_invalid_input() {
    // `null!` 的查询在 prepareQuery 里等同空串
    let mut res = score_item(None, "", true, &ResourceAccessor);
    assert_eq!(res.score, 0);

    res = score_item(None, "null", true, &ResourceAccessor);
    assert_eq!(res.score, 0);
}

#[test]
fn score_item_optimize_for_file_paths() {
    let resource = file("/xyz/others/spath/some/xsp/file123.txt");

    // xsp is more relevant to the end of the file path even though it matches
    // fuzzy also in the beginning. we verify the more relevant match at the
    // end gets returned.
    let path_res = score_item(Some(&resource), "xspfile123", true, &ResourceAccessor);
    assert_ne!(path_res.score, 0);
    assert!(path_res.description_match.is_some());
    assert!(path_res.label_match.is_some());
    assert_matches(&path_res.label_match, &[(0, 7)]);
    assert_matches(&path_res.description_match, &[(23, 26)]);
}

#[test]
fn score_item_avoid_match_scattering_bug_36119() {
    let resource = file("projects/ui/cula/ats/target.mk");

    let path_res = score_item(Some(&resource), "tcltarget.mk", true, &ResourceAccessor);
    assert_ne!(path_res.score, 0);
    assert!(path_res.description_match.is_some());
    assert!(path_res.label_match.is_some());
    assert_matches(&path_res.label_match, &[(0, 9)]);
}

#[test]
fn score_item_prefers_more_compact_matches() {
    let resource = file("/1a111d1/11a1d1/something.txt");

    // expect "ad" to be matched towards the end of the file because the
    // match is more compact
    let res = score_item(Some(&resource), "ad", true, &ResourceAccessor);
    assert_ne!(res.score, 0);
    assert!(res.description_match.is_some());
    assert!(res.label_match.as_ref().unwrap().is_empty());
    assert_matches(&res.description_match, &[(11, 12), (13, 14)]);
}

#[test]
fn score_item_proper_target_offset() {
    let resource = file("etem");

    let res = score_item(Some(&resource), "teem", true, &ResourceAccessor);
    assert_eq!(res.score, 0);
}

#[test]
fn score_item_proper_target_offset_2() {
    let resource = file("ede");

    let res = score_item(Some(&resource), "de", true, &ResourceAccessor);

    assert_matches(&res.label_match, &[(1, 3)]);
}

#[test]
fn score_item_proper_target_offset_3() {
    let resource = file("/src/vs/editor/browser/viewParts/lineNumbers/flipped-cursor-2x.svg");

    let res = score_item(Some(&resource), "debug", true, &ResourceAccessor);

    assert_matches(&res.description_match, &[(9, 10), (36, 37), (40, 41)]);
    assert_matches(&res.label_match, &[(9, 10), (20, 21)]);
}

#[test]
fn score_item_no_match_unless_query_contained_in_sequence() {
    let resource = file("abcde");

    let res = score_item(Some(&resource), "edcda", true, &ResourceAccessor);
    assert_eq!(res.score, 0);
}

#[test]
fn score_item_match_if_using_slash_or_backslash_local_remote_resource() {
    let local_resource = file("abcde/super/duper");
    let remote_resource = remote("abcde/super/duper");

    for resource in [&local_resource, &remote_resource] {
        let accessors: [&dyn ItemAccessor<Uri>; 3] =
            [&ResourceAccessor, &ResourceWithSlashAccessor, &ResourceWithBackslashAccessor];
        for query in ["abcde\\super\\duper", "abcde/super/duper"] {
            for accessor in accessors {
                let res = score_item(Some(resource), query, true, accessor);
                assert_ne!(res.score, 0);
            }
        }
    }
}

#[test]
fn score_item_ensure_upper_case_bonus_only_applies_on_non_consecutive_matches_bug_134723() {
    let resource_with_upper = file("ASDFasdfasdf");
    let resource_all_lower = file("asdfasdfasdf");

    assert!(
        score_item(Some(&resource_all_lower), "asdf", true, &ResourceAccessor).score
            > score_item(Some(&resource_with_upper), "asdf", true, &ResourceAccessor).score
    );
}

#[test]
fn compare_items_by_score_identity() {
    let resource_a = file("/some/path/fileA.txt");
    let resource_b = file("/some/path/other/fileB.txt");
    let resource_c = file("/unrelated/some/path/other/fileC.txt");

    // Full resource A path
    let query = path_of(&resource_a);

    assert_sorted(&[&resource_a, &resource_b, &resource_c], &query, &[&resource_a, &resource_b, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], &query, &[&resource_a, &resource_b, &resource_c]);

    // Full resource B path
    let query = path_of(&resource_b);

    assert_sorted(&[&resource_a, &resource_b, &resource_c], &query, &[&resource_b, &resource_a, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], &query, &[&resource_b, &resource_a, &resource_c]);
}

#[test]
fn compare_files_by_score_basename_prefix() {
    let resource_a = file("/some/path/fileA.txt");
    let resource_b = file("/some/path/other/fileB.txt");
    let resource_c = file("/unrelated/some/path/other/fileC.txt");

    // Full resource A basename
    let query = label_of(&resource_a);

    assert_sorted(&[&resource_a, &resource_b, &resource_c], &query, &[&resource_a, &resource_b, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], &query, &[&resource_a, &resource_b, &resource_c]);

    // Full resource B basename
    let query = label_of(&resource_b);

    assert_sorted(&[&resource_a, &resource_b, &resource_c], &query, &[&resource_b, &resource_a, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], &query, &[&resource_b, &resource_a, &resource_c]);
}

#[test]
fn compare_files_by_score_basename_camelcase() {
    let resource_a = file("/some/path/fileA.txt");
    let resource_b = file("/some/path/other/fileB.txt");
    let resource_c = file("/unrelated/some/path/other/fileC.txt");

    // resource A camelcase
    let query = "fA";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_a, &resource_b, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_a, &resource_b, &resource_c]);

    // resource B camelcase
    let query = "fB";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_b, &resource_a, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_b, &resource_a, &resource_c]);
}

#[test]
fn compare_files_by_score_basename_scores() {
    let resource_a = file("/some/path/fileA.txt");
    let resource_b = file("/some/path/other/fileB.txt");
    let resource_c = file("/unrelated/some/path/other/fileC.txt");

    // Resource A part of basename
    let query = "fileA";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_a, &resource_b, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_a, &resource_b, &resource_c]);

    // Resource B part of basename
    let query = "fileB";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_b, &resource_a, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_b, &resource_a, &resource_c]);
}

#[test]
fn compare_files_by_score_path_scores() {
    let resource_a = file("/some/path/fileA.txt");
    let resource_b = file("/some/path/other/fileB.txt");
    let resource_c = file("/unrelated/some/path/other/fileC.txt");

    // Resource A part of path
    let query = "pathfileA";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_a, &resource_b, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_a, &resource_b, &resource_c]);

    // Resource B part of path
    let query = "pathfileB";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_b, &resource_a, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_b, &resource_a, &resource_c]);
}

#[test]
fn compare_files_by_score_prefer_shorter_basenames() {
    let resource_a = file("/some/path/fileA.txt");
    let resource_b = file("/some/path/other/fileBLonger.txt");
    let resource_c = file("/unrelated/the/path/other/fileC.txt");

    // Resource A part of path
    let query = "somepath";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_a, &resource_b, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_a, &resource_b, &resource_c]);
}

#[test]
fn compare_files_by_score_prefer_shorter_basenames_match_on_basename() {
    let resource_a = file("/some/path/fileA.txt");
    let resource_b = file("/some/path/other/fileBLonger.txt");
    let resource_c = file("/unrelated/the/path/other/fileC.txt");

    // Resource A part of path
    let query = "file";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_a, &resource_c, &resource_b]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_a, &resource_c, &resource_b]);
}

#[test]
fn compare_files_by_score_prefer_shorter_paths() {
    let resource_a = file("/some/path/fileA.txt");
    let resource_b = file("/some/path/other/fileB.txt");
    let resource_c = file("/unrelated/some/path/other/fileC.txt");

    // Resource A part of path
    let query = "somepath";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_a, &resource_b, &resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_a, &resource_b, &resource_c]);
}

#[test]
fn compare_files_by_score_prefer_shorter_paths_bug_17443() {
    let resource_a = file("config/test/t1.js");
    let resource_b = file("config/test.js");
    let resource_c = file("config/test/t2.js");

    let query = "co/te";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_b, &resource_a, &resource_c]);
}

#[test]
fn compare_files_by_score_prefer_matches_in_label_over_description_if_scores_are_otherwise_equal() {
    let resource_a = file("parts/quick/arrow-left-dark.svg");
    let resource_b = file("parts/quickopen/quickopen.ts");

    let query = "partsquick";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
}

#[test]
fn compare_files_by_score_prefer_camel_case_matches() {
    let resource_a = file("config/test/NullPointerException.java");
    let resource_b = file("config/test/nopointerexception.java");

    for query in ["npe", "NPE"] {
        assert_sorted(&[&resource_a, &resource_b], query, &[&resource_a, &resource_b]);
        assert_sorted(&[&resource_b, &resource_a], query, &[&resource_a, &resource_b]);
    }
}

#[test]
fn compare_files_by_score_prefer_more_compact_camel_case_matches() {
    let resource_a = file("config/test/openthisAnythingHandler.js");
    let resource_b = file("config/test/openthisisnotsorelevantforthequeryAnyHand.js");

    let query = "AH";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);
}

#[test]
fn compare_files_by_score_prefer_more_compact_matches_label() {
    let resource_a = file("config/test/examasdaple.js");
    let resource_b = file("config/test/exampleasdaasd.ts");

    let query = "xp";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);
}

#[test]
fn compare_files_by_score_prefer_more_compact_matches_path() {
    let resource_a = file("config/test/examasdaple/file.js");
    let resource_b = file("config/test/exampleasdaasd/file.ts");

    let query = "xp";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);
}

#[test]
fn compare_files_by_score_prefer_more_compact_matches_label_and_path() {
    let resource_a = file("config/example/thisfile.ts");
    let resource_b = file("config/24234243244/example/file.js");

    let query = "exfile";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_34210() {
    let resource_a = file("node_modules1/bundle/lib/model/modules/ot1/index.js");
    let resource_b = file("node_modules1/bundle/lib/model/modules/un1/index.js");
    let resource_c = file("node_modules1/bundle/lib/model/modules/modu1/index.js");
    let resource_d = file("node_modules1/bundle/lib/model/modules/oddl1/index.js");

    // isWindows
    let query = "modu1\\index.js";

    assert_sorted(&[&resource_a, &resource_b, &resource_c, &resource_d], query, &[&resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a, &resource_d], query, &[&resource_c]);

    let query = "un1\\index.js";

    assert_sorted(&[&resource_a, &resource_b, &resource_c, &resource_d], query, &[&resource_b]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a, &resource_d], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_21019_1() {
    let resource_a =
        file("app/containers/Services/NetworkData/ServiceDetails/ServiceLoad/index.js");
    let resource_b =
        file("app/containers/Services/NetworkData/ServiceDetails/ServiceDistribution/index.js");
    let resource_c = file(
        "app/containers/Services/NetworkData/ServiceDetailTabs/ServiceTabs/StatVideo/index.js",
    );

    let query = "StatVideoindex";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_c]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_21019_2() {
    let resource_a = file("src/build-helper/store/redux.ts");
    let resource_b = file("src/repository/store/redux.ts");

    let query = "reproreduxts";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_26649() {
    let resource_a = file("photobook/src/components/AddPagesButton/index.js");
    let resource_b = file("photobook/src/components/ApprovalPageHeader/index.js");
    let resource_c = file("photobook/src/canvasComponents/BookPage/index.js");

    let query = "bookpageIndex";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_c]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_c]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_33247() {
    let resource_a = file("ui/src/utils/constants.js");
    let resource_b = file("ui/src/ui/Icons/index.js");

    let query = "ui\\icons"; // isWindows

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_33247_comment() {
    let resource_a = file("ui/src/components/IDInput/index.js");
    let resource_b = file("ui/src/ui/Input/index.js");

    let query = "ui\\input\\index"; // isWindows

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_36166() {
    let resource_a = file("django/contrib/sites/locale/ga/LC_MESSAGES/django.mo");
    let resource_b = file("django/core/signals.py");

    let query = "djancosig";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_32918() {
    let resource_a = file("adsys/protected/config.php");
    let resource_b =
        file("adsys/protected/framework/smarty/sysplugins/smarty_internal_config.php");
    let resource_c = file("duowanVideo/wap/protected/config.php");

    let query = "protectedconfig.php";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_a, &resource_c, &resource_b]);
    assert_sorted(&[&resource_c, &resource_b, &resource_a], query, &[&resource_a, &resource_c, &resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_14879() {
    let resource_a = file("pkg/search/gradient/testdata/constraint_attrMatchString.yml");
    let resource_b = file("cmd/gradient/main.go");

    let query = "gradientmain";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_14727_1() {
    let resource_a = file("alpha-beta-cappa.txt");
    let resource_b = file("abc.txt");

    let query = "abc";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_14727_2() {
    let resource_a = file("xerxes-yak-zubba/index.js");
    let resource_b = file("xyz/index.js");

    let query = "xyz";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_18381() {
    let resource_a = file("AssymblyInfo.cs");
    let resource_b = file("IAsynchronousTask.java");

    let query = "async";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_35572() {
    let resource_a =
        file("static/app/source/angluar/-admin/-organization/-settings/layout/layout.js");
    let resource_b =
        file("static/app/source/angular/-admin/-project/-settings/_settings/settings.js");

    let query = "partisettings";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_36810() {
    let resource_a = file("Trilby.TrilbyTV.Web.Portal/Views/Systems/Index.cshtml");
    let resource_b = file("Trilby.TrilbyTV.Web.Portal/Areas/Admins/Views/Tips/Index.cshtml");

    let query = "tipsindex.cshtml";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_prefer_shorter_hit_bug_20546() {
    let resource_a = file("editor/core/components/tests/list-view-spec.js");
    let resource_b = file("editor/core/components/list-view.js");

    let query = "listview";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_avoid_match_scattering_bug_12095() {
    let resource_a = file("src/vs/workbench/contrib/files/common/explorerViewModel.ts");
    let resource_b = file("src/vs/workbench/contrib/files/browser/views/explorerView.ts");
    let resource_c = file("src/vs/workbench/contrib/files/browser/views/explorerViewer.ts");

    let query = "filesexplorerview.ts";

    assert_sorted(&[&resource_a, &resource_b, &resource_c], query, &[&resource_b]);
    assert_sorted(&[&resource_a, &resource_c, &resource_b], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_prefer_case_match_bug_96122() {
    let resource_a = file("lists.php");
    let resource_b = file("lib/Lists.php");

    let query = "Lists.php";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b]);
}

#[test]
fn compare_files_by_score_prefer_shorter_match_bug_103052_foo_bar() {
    let resource_a = file("app/emails/foo.bar.js");
    let resource_b = file("app/emails/other-footer.other-bar.js");

    for query in ["foo bar", "foobar"] {
        assert_sorted(&[&resource_a, &resource_b], query, &[&resource_a, &resource_b]);
        assert_sorted(&[&resource_b, &resource_a], query, &[&resource_a, &resource_b]);
    }
}

#[test]
fn compare_files_by_score_prefer_shorter_match_bug_103052_payment_model() {
    let resource_a = file("app/components/payment/payment.model.js");
    let resource_b =
        file("app/components/online-payments-history/online-payments-history.model.js");

    for query in ["payment model", "paymentmodel"] {
        assert_sorted(&[&resource_a, &resource_b], query, &[&resource_a, &resource_b]);
        assert_sorted(&[&resource_b, &resource_a], query, &[&resource_a, &resource_b]);
    }
}

#[test]
fn compare_files_by_score_prefer_shorter_match_bug_103052_color() {
    let resource_a = file("app/constants/color.js");
    let resource_b = file("app/components/model/input/pick-avatar-color.js");

    for query in ["color js", "colorjs"] {
        assert_sorted(&[&resource_a, &resource_b], query, &[&resource_a, &resource_b]);
        assert_sorted(&[&resource_b, &resource_a], query, &[&resource_a, &resource_b]);
    }
}

#[test]
fn compare_files_by_score_prefer_strict_case_prefix() {
    let resource_a = file("app/constants/color.js");
    let resource_b = file("app/components/model/input/Color.js");

    let query = "Color";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);

    let query = "color";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_a, &resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_a, &resource_b]);
}

#[test]
fn compare_files_by_score_prefer_prefix_bug_103052() {
    let resource_a = file("test/smoke/src/main.ts");
    let resource_b = file("src/vs/editor/common/services/semantikTokensProviderStyling.ts");

    let query = "smoke main.ts";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_a, &resource_b]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_a, &resource_b]);
}

#[test]
fn compare_files_by_score_boost_better_prefix_match_if_multiple_queries_are_used() {
    let resource_a = file("src/vs/workbench/services/host/browser/browserHostService.ts");
    let resource_b = file("src/vs/workbench/browser/workbench.ts");

    for query in ["workbench.ts browser", "browser workbench.ts", "browser workbench", "workbench browser"] {
        assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
        assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);
    }
}

#[test]
fn compare_files_by_score_boost_shorter_prefix_match_if_multiple_queries_are_used() {
    let resource_a = file("src/vs/workbench/node/actions/windowActions.ts");
    let resource_b = file("src/vs/workbench/electron-node/window.ts");

    for query in ["window node", "window.ts node"] {
        assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
        assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);
    }
}

#[test]
fn compare_files_by_score_skip_preference_on_label_match_when_using_path_sep() {
    let resource_a = file("djangosite/ufrela/def.py");
    let resource_b = file("djangosite/urls/default.py");

    let query = "url/def";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);
}

#[test]
fn compare_files_by_score_boost_shorter_prefix_match_if_multiple_queries_are_used_99171() {
    let resource_a = file("mesh_editor_lifetime_job.h");
    let resource_b = file("lifetime_job.h");

    let query = "m life, life m";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);
}

#[test]
fn compare_files_by_score_boost_consecutive_matches_in_the_beginning_over_end() {
    let resource_a = file("src/vs/server/node/extensionHostStatusService.ts");
    let resource_b = file("src/vs/workbench/browser/parts/notifications/notificationsStatus.ts");

    let query = "notStatus";

    assert_sorted(&[&resource_a, &resource_b], query, &[&resource_b, &resource_a]);
    assert_sorted(&[&resource_b, &resource_a], query, &[&resource_b, &resource_a]);
}

#[test]
fn prepare_query_cases() {
    assert_eq!(prepare_query(" f*a ").normalized, "fa");
    assert_eq!(prepare_query(" f…a ").normalized, "fa");
    assert_eq!(prepare_query("main#").normalized, "main");
    assert_eq!(prepare_query("main#").original, "main#");
    assert_eq!(prepare_query("foo*").normalized, "foo");
    assert_eq!(prepare_query("foo*").original, "foo*");
    assert_eq!(prepare_query("model Tester.ts").original, "model Tester.ts");
    assert_eq!(prepare_query("model Tester.ts").original_lowercase, "model Tester.ts".to_lowercase());
    assert_eq!(prepare_query("model Tester.ts").normalized, "modelTester.ts");
    assert!(!prepare_query("model Tester.ts").expect_contiguous_match); // doesn't have quotes in it
    assert_eq!(prepare_query("Model Tester.ts").normalized_lowercase, "modeltester.ts");
    assert!(!prepare_query("ModelTester.ts").contains_path_separator);
    assert!(prepare_query(&format!("Model{SEP}Tester.ts")).contains_path_separator);
    assert!(prepare_query("\"hello\"").expect_contiguous_match);
    assert_eq!(prepare_query("\"hello\"").normalized, "hello");

    // with spaces
    let mut query = prepare_query("He*llo World");
    assert_eq!(query.original, "He*llo World");
    assert_eq!(query.normalized, "HelloWorld");
    assert_eq!(query.normalized_lowercase, "HelloWorld".to_lowercase());
    let values = query.values.as_ref().unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].original, "He*llo");
    assert_eq!(values[0].normalized, "Hello");
    assert_eq!(values[0].normalized_lowercase, "Hello".to_lowercase());
    assert_eq!(values[1].original, "World");
    assert_eq!(values[1].normalized, "World");
    assert_eq!(values[1].normalized_lowercase, "World".to_lowercase());

    let restored_query = piece_to_query(query.values.as_ref().unwrap());
    assert_eq!(restored_query.original, query.original);
    assert_eq!(restored_query.values.as_ref().map(Vec::len), query.values.as_ref().map(Vec::len));
    assert_eq!(restored_query.contains_path_separator, query.contains_path_separator);

    // with spaces that are empty
    query = prepare_query(" Hello   World  \t");
    assert_eq!(query.original, " Hello   World  \t");
    assert_eq!(query.original_lowercase, " Hello   World  \t".to_lowercase());
    assert_eq!(query.normalized, "HelloWorld");
    assert_eq!(query.normalized_lowercase, "HelloWorld".to_lowercase());
    let values = query.values.as_ref().unwrap();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].original, "Hello");
    assert_eq!(values[0].original_lowercase, "Hello".to_lowercase());
    assert_eq!(values[0].normalized, "Hello");
    assert_eq!(values[0].normalized_lowercase, "Hello".to_lowercase());
    assert_eq!(values[1].original, "World");
    assert_eq!(values[1].original_lowercase, "World".to_lowercase());
    assert_eq!(values[1].normalized, "World");
    assert_eq!(values[1].normalized_lowercase, "World".to_lowercase());

    // Path related（isWindows 分支）
    assert_eq!(prepare_query("C:\\some\\path").path_normalized, "C:\\some\\path");
    assert_eq!(prepare_query("C:\\some\\path").normalized, "C:\\some\\path");
    assert!(prepare_query("C:\\some\\path").contains_path_separator);
    assert_eq!(prepare_query("C:/some/path").path_normalized, "C:\\some\\path");
    assert_eq!(prepare_query("C:/some/path").normalized, "C:\\some\\path");
    assert!(prepare_query("C:/some/path").contains_path_separator);
}

#[test]
fn fuzzy_score2_matching() {
    let target = "HelLo-World";

    for offset in [0usize, 3] {
        let t = if offset == 0 { target.to_owned() } else { format!("123{target}") };
        let (score, matches) = do_score2(&t, "HelLo-World", offset);

        assert!(truthy(score));
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].start, offset);
        assert_eq!(matches[0].end, target.len() + offset);

        let (score, matches) = do_score2(&t, "HW", offset);

        assert!(truthy(score));
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].start, offset);
        assert_eq!(matches[0].end, 1 + offset);
        assert_eq!(matches[1].start, 6 + offset);
        assert_eq!(matches[1].end, 7 + offset);
    }
}

#[test]
fn fuzzy_score2_multiple_queries() {
    let target = "HelLo-World";

    let (first_single_score, first_single_matches) = do_score2(target, "HelLo", 0);
    let (second_single_score, second_single_matches) = do_score2(target, "World", 0);
    let first_and_second_single_matches: Vec<Match> =
        first_single_matches.iter().chain(second_single_matches.iter()).copied().collect();

    let assert_score = |multi_score: Option<i32>, multi_matches: &[Match]| {
        assert!(
            multi_score.unwrap_or(0) >= first_single_score.unwrap_or(0) + second_single_score.unwrap_or(0)
        );
        for (i, multi_match) in multi_matches.iter().enumerate() {
            let first_and_second_single_match = first_and_second_single_matches
                .get(i)
                .unwrap_or_else(|| panic!("no single match at index {i}"));
            assert_eq!(multi_match.start, first_and_second_single_match.start);
            assert_eq!(multi_match.end, first_and_second_single_match.end);
        }
    };

    let assert_no_score = |multi_score: Option<i32>, multi_matches: &[Match]| {
        assert_eq!(multi_score, None);
        assert_eq!(multi_matches.len(), 0);
    };

    let (multi_score, multi_matches) = do_score2(target, "HelLo World", 0);
    assert_score(multi_score, &multi_matches);

    let (multi_score, multi_matches) = do_score2(target, "World HelLo", 0);
    assert_score(multi_score, &multi_matches);

    let (multi_score, multi_matches) = do_score2(target, "World HelLo World", 0);
    assert_score(multi_score, &multi_matches);

    let (multi_score, multi_matches) = do_score2(target, "World HelLo Nothing", 0);
    assert_no_score(multi_score, &multi_matches);

    let (multi_score, multi_matches) = do_score2(target, "More Nothing", 0);
    assert_no_score(multi_score, &multi_matches);
}

#[test]
fn fuzzy_score2_95716() {
    let target = "# ❌ Wow";

    let score = do_score2(target, "❌", 0);
    // `assert.ok(score)` 对数组恒真；下面两条是实质断言
    assert!(score.0.is_some());
    assert!(!score.1.is_empty());
}

#[test]
fn using_quotes_should_expect_contiguous_matches_match() {
    // missing the "i" in the query
    assert_eq!(do_score("contiguous", "\"contguous\"", None).score, 0);

    let score = do_score("contiguous", "\"contiguous\"", None);
    assert!(score.score > 0);
}

#[test]
fn using_quotes_should_highlight_contiguous_indexes() {
    let score = do_score("2021-7-26.md", "\"26\"", None);
    assert_eq!(score.score, 14);

    // The indexes of the 2 and 6 of "26"
    assert_eq!(score.positions[0], 7);
    assert_eq!(score.positions[1], 8);
}

#[test]
fn workspace_symbol_search_with_special_characters() {
    // Simulates the scenario from the issue where rust-analyzer uses # and * as query modifiers
    // The original query (with special chars) should reach the language server
    // but normalized query (without special chars) should be used for fuzzy matching

    // Test #: User types "main#", language server returns "main" symbol
    let mut query = prepare_query("main#");
    assert_eq!(query.original, "main#"); // Sent to language server
    assert_eq!(query.normalized, "main"); // Used for fuzzy matching
    let (mut score, mut matches) = do_score2("main", "main#", 0);
    assert!(score.is_some_and(|s| s > 0), "Should match \"main\" symbol when query is \"main#\"");
    assert!(!matches.is_empty());

    // Test *: User types "foo*", language server returns "foo" symbol
    query = prepare_query("foo*");
    assert_eq!(query.original, "foo*"); // Sent to language server
    assert_eq!(query.normalized, "foo"); // Used for fuzzy matching
    (score, matches) = do_score2("foo", "foo*", 0);
    assert!(score.is_some_and(|s| s > 0), "Should match \"foo\" symbol when query is \"foo*\"");
    assert!(!matches.is_empty());

    // Test both: User types "MyClass#*", should match "MyClass"
    query = prepare_query("MyClass#*");
    assert_eq!(query.original, "MyClass#*");
    assert_eq!(query.normalized, "MyClass");
    (score, matches) = do_score2("MyClass", "MyClass#*", 0);
    assert!(
        score.is_some_and(|s| s > 0),
        "Should match \"MyClass\" symbol when query is \"MyClass#*\""
    );
    assert!(!matches.is_empty());

    // Test fuzzy matching still works: User types "MC#", should match "MyClass"
    query = prepare_query("MC#");
    assert_eq!(query.original, "MC#");
    assert_eq!(query.normalized, "MC");
    (score, matches) = do_score2("MyClass", "MC#", 0);
    assert!(
        score.is_some_and(|s| s > 0),
        "Should fuzzy match \"MyClass\" symbol when query is \"MC#\""
    );
    assert!(!matches.is_empty());

    // Make sure leading # or # in the middle are not removed.
    query = prepare_query("#SpecialFunction");
    assert_eq!(query.original, "#SpecialFunction");
    assert_eq!(query.normalized, "#SpecialFunction");
    (score, matches) = do_score2("#SpecialFunction", "#SpecialFunction", 0);
    assert!(
        score.is_some_and(|s| s > 0),
        "Should match \"#SpecialFunction\" symbol when query is \"#SpecialFunction\""
    );
    assert!(!matches.is_empty());

    // Make sure standalone # is not removed
    query = prepare_query("#");
    assert_eq!(query.original, "#");
    assert_eq!(query.normalized, "#", "Standalone # should not be removed");
    (score, matches) = do_score2("#", "#", 0);
    assert!(score.is_some_and(|s| s > 0), "Should match \"#\" symbol when query is \"#\"");
    assert!(!matches.is_empty());
}
