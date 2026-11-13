//! 搜索引擎入口的集成测试（#25）：在临时目录里建文件，经公开入口 `rove_search::search` 断言结果。
#![allow(clippy::single_range_in_vec_init)] // 一行一处匹配本来就是只有一个区间的 Vec

use rove_base::{Cancellation, Error, Value};
use rove_config::{DefaultSettings, Setting, SettingRegistry, Settings};
use rove_search::{search, settings, FileMatches, SearchConfig, SearchQuery, SearchSummary};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

fn settings_with(overrides: &[(&str, Value)]) -> Arc<dyn Settings> {
    let base = settings::default_settings();
    let mut registry = SettingRegistry::default();
    for (id, setting) in base.0.entries() {
        let default = overrides
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| {
                setting
                    .rove_default
                    .clone()
                    .unwrap_or(setting.default.clone())
            });
        registry
            .register(
                id,
                Setting {
                    default,
                    rove_default: None,
                    language_overridable: setting.language_overridable,
                },
            )
            .unwrap();
    }
    Arc::new(DefaultSettings(registry))
}

fn config(overrides: &[(&str, Value)]) -> SearchConfig {
    SearchConfig::from_settings(settings_with(overrides))
}

fn write(root: &Path, rel: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn run(
    root: &Path,
    query: &SearchQuery,
    config: &SearchConfig,
) -> (Vec<FileMatches>, SearchSummary) {
    let mut files = vec![];
    let summary = search(root, query, config, &Cancellation::default(), |batch| {
        files.extend(batch)
    })
    .unwrap();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    (files, summary)
}

fn paths(files: &[FileMatches]) -> Vec<&str> {
    files.iter().map(|f| f.path.as_str()).collect()
}

fn object(pairs: &[(&str, bool)]) -> Value {
    Value::Object(Arc::new(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::Bool(*v)))
            .collect::<BTreeMap<_, _>>(),
    ))
}

#[test]
fn results_are_grouped_by_file_with_lines_previews_and_byte_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "Assets/Scripts/Player.cs",
        "class Player {\r\n    void Move() { Move(); }\r\n}\r\n",
    );
    write(root, "Assets/Scripts/Enemy.cs", "// 敌人 Move\n");
    write(root, "Assets/Readme.md", "nothing here\n");
    let (files, summary) = run(root, &SearchQuery::new("Move"), &config(&[]));
    assert_eq!(
        paths(&files),
        vec!["Assets/Scripts/Enemy.cs", "Assets/Scripts/Player.cs"]
    );
    let enemy = &files[0].lines[0];
    assert_eq!(enemy.line, 0);
    // “// 敌人 ” = 3 + 6 + 1 字节：列是 UTF-8 字节列。
    assert_eq!(enemy.ranges, vec![10..14]);
    let player = &files[1].lines[0];
    assert_eq!(player.line, 1);
    assert_eq!(&*player.preview, "void Move() { Move(); }");
    assert_eq!(player.preview_start, 4);
    assert_eq!(player.ranges, vec![9..13, 18..22]);
    assert_eq!(summary.matches, 3);
    assert_eq!(summary.files_with_matches, 2);
    assert_eq!(summary.files_searched, 3);
    assert!(!summary.limit_hit);
}

#[test]
fn regex_whole_word_and_case_options() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "a.cs",
        "GetComponent<Rigidbody>()\nmove Move MoveTo\n",
    );
    let cfg = config(&[]);
    let regex = SearchQuery {
        is_regex: true,
        ..SearchQuery::new(r"Get\w+<(\w+)>")
    };
    let (files, _) = run(root, &regex, &cfg);
    assert_eq!(files[0].lines[0].ranges, vec![0..23]);

    let word = SearchQuery {
        whole_word: true,
        ..SearchQuery::new("Move")
    };
    let (files, _) = run(root, &word, &cfg);
    // 默认不区分大小写：move、Move 两处；MoveTo 不是整个单词。
    assert_eq!(files[0].lines[0].ranges, vec![0..4, 5..9]);

    let sensitive = SearchQuery {
        whole_word: true,
        case_sensitive: true,
        ..SearchQuery::new("Move")
    };
    let (files, _) = run(root, &sensitive, &cfg);
    assert_eq!(files[0].lines[0].ranges, vec![5..9]);

    let invalid = SearchQuery {
        is_regex: true,
        ..SearchQuery::new("(")
    };
    assert_eq!(
        search(root, &invalid, &cfg, &Cancellation::default(), |_| {}).unwrap_err(),
        Error::InvalidInput
    );
}

#[test]
fn smart_case_setting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "a.cs", "player Player PLAYER\n");
    let smart = config(&[(settings::SMART_CASE, Value::Bool(true))]);
    let (files, _) = run(root, &SearchQuery::new("player"), &smart);
    assert_eq!(files[0].lines[0].ranges.len(), 3, "全小写：不区分大小写");
    let (files, _) = run(root, &SearchQuery::new("Player"), &smart);
    assert_eq!(files[0].lines[0].ranges, vec![7..13], "含大写：区分大小写");
    let (files, _) = run(root, &SearchQuery::new("Player"), &config(&[]));
    assert_eq!(files[0].lines[0].ranges.len(), 3, "smartCase 关闭");
}

#[test]
fn include_and_exclude_globs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "Assets/Scripts/A.cs", "needle\n");
    write(root, "Assets/Scripts/Editor/B.cs", "needle\n");
    write(root, "Assets/Shaders/C.shader", "needle\n");
    write(root, "Packages/Assets/D.cs", "needle\n");
    let cfg = config(&[]);
    let q = |includes: &str, excludes: &str| SearchQuery {
        includes: includes.into(),
        excludes: excludes.into(),
        ..SearchQuery::new("needle")
    };
    let (files, _) = run(root, &q("*.cs", ""), &cfg);
    assert_eq!(
        paths(&files),
        vec![
            "Assets/Scripts/A.cs",
            "Assets/Scripts/Editor/B.cs",
            "Packages/Assets/D.cs"
        ]
    );
    let (files, _) = run(root, &q("./Assets", "Editor, *.shader"), &cfg);
    assert_eq!(paths(&files), vec!["Assets/Scripts/A.cs"]);
    let (files, _) = run(root, &q("Scripts, Shaders", ""), &cfg);
    assert_eq!(
        paths(&files),
        vec![
            "Assets/Scripts/A.cs",
            "Assets/Scripts/Editor/B.cs",
            "Assets/Shaders/C.shader"
        ]
    );
}

#[test]
fn exclude_settings_merge_and_the_rove_default_flag() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "Assets/A.cs", "nothing\n");
    write(root, "Library/PackageCache/X.cs", "needle\n");
    write(root, "node_modules/x.js", "needle\n");
    write(root, "Build/out.txt", "needle\n");
    let cfg = config(&[(rove_workspace::FILES_EXCLUDE, object(&[("**/Build", true)]))]);
    // Library 由 rove.files.extraExclude 排除，node_modules 由 search.exclude，Build 由 files.exclude。
    let (files, summary) = run(root, &SearchQuery::new("needle"), &cfg);
    assert!(files.is_empty());
    assert!(summary.excluded_by_rove_defaults);
    // “包含排除的文件再搜索一次”：关闭“使用排除设置”。
    let all = SearchQuery {
        use_exclude_settings: false,
        ..SearchQuery::new("needle")
    };
    let (files, summary) = run(root, &all, &cfg);
    assert_eq!(
        paths(&files),
        vec![
            "Build/out.txt",
            "Library/PackageCache/X.cs",
            "node_modules/x.js"
        ]
    );
    assert!(!summary.excluded_by_rove_defaults);
    // 用户在 files.exclude 里显式写 false 取消 Rove 的附加排除：不再提示。
    let cfg = config(&[(
        rove_workspace::FILES_EXCLUDE,
        object(&[("**/Library", false)]),
    )]);
    let (files, summary) = run(root, &SearchQuery::new("needle"), &cfg);
    assert_eq!(
        paths(&files),
        vec!["Build/out.txt", "Library/PackageCache/X.cs"]
    );
    assert!(!summary.excluded_by_rove_defaults);
}

#[test]
fn gitignore_never_affects_results() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".git/info")).unwrap();
    write(root, ".gitignore", "Ignored/\n*.log\nsecret.cs\n");
    write(root, ".git/info/exclude", "Other/\n");
    write(root, ".ignore", "Third/\n");
    write(root, "Ignored/A.cs", "needle\n");
    write(root, "debug.log", "needle\n");
    write(root, "secret.cs", "needle\n");
    write(root, "Other/B.cs", "needle\n");
    write(root, "Third/C.cs", "needle\n");
    let (files, _) = run(root, &SearchQuery::new("needle"), &config(&[]));
    assert_eq!(
        paths(&files),
        vec![
            "Ignored/A.cs",
            "Other/B.cs",
            "Third/C.cs",
            "debug.log",
            "secret.cs"
        ]
    );
}

#[test]
fn max_results_truncates_exactly_and_reports_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for i in 0..20 {
        write(root, &format!("f{i:02}.txt"), "hit hit\nhit\n".repeat(5));
    }
    let cfg = config(&[(settings::MAX_RESULTS, Value::Number(37.0))]);
    let (files, summary) = run(root, &SearchQuery::new("hit"), &cfg);
    let total: usize = files.iter().map(FileMatches::match_count).sum();
    assert_eq!(total, 37);
    assert_eq!(summary.matches, 37);
    assert!(summary.limit_hit);
    // null 表示不限。
    let cfg = config(&[(settings::MAX_RESULTS, Value::Null)]);
    let (_, summary) = run(root, &SearchQuery::new("hit"), &cfg);
    assert_eq!(summary.matches, 20 * 15);
    assert!(!summary.limit_hit);
}

#[test]
fn gbk_files_are_decoded_through_the_buffer_encoding_detection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let (gbk, _, _) = encoding_rs::GBK.encode("// 玩家移动的速度\nfloat 速度 = 1;\n");
    write(root, "Assets/Gbk.cs", &gbk);
    let mut utf16_bytes = vec![0xFF, 0xFE];
    for unit in "x 速度\n".encode_utf16() {
        utf16_bytes.extend_from_slice(&unit.to_le_bytes());
    }
    write(root, "Assets/Utf16.txt", &utf16_bytes);
    write(
        root,
        "Assets/Bom.cs",
        b"\xEF\xBB\xBF\xE9\x80\x9F\xE5\xBA\xA6\n",
    );
    write(
        root,
        "Assets/binary.bin",
        b"\x00\x01\x02\xE9\x80\x9F\xE5\xBA\xA6",
    );
    let (files, summary) = run(root, &SearchQuery::new("速度"), &config(&[]));
    assert_eq!(
        paths(&files),
        vec!["Assets/Bom.cs", "Assets/Gbk.cs", "Assets/Utf16.txt"]
    );
    let gbk = &files[1];
    assert_eq!(gbk.lines.len(), 2);
    // 第 0 行“// 玩家移动的” = 3 + 5×3 字节；列按解码后的 UTF-8 计。
    assert_eq!(gbk.lines[0].ranges, vec![18..24]);
    assert_eq!(&*gbk.lines[1].preview, "float 速度 = 1;");
    assert_eq!(gbk.lines[1].ranges, vec![6..12]);
    // BOM 不占列。
    assert_eq!(files[0].lines[0].ranges, vec![0..6]);
    assert_eq!(files[2].lines[0].ranges, vec![2..8]);
    assert_eq!(summary.skipped, 1, "二进制文件被跳过");
}

#[test]
fn large_files_are_searched_in_chunks_with_correct_line_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut text = String::new();
    for i in 0..200_000 {
        if i % 50_000 == 7 {
            text.push_str("the NEEDLE is here\n");
        } else {
            text.push_str("lorem ipsum dolor sit amet\n");
        }
    }
    assert!(text.len() > 4 << 20);
    write(root, "big.txt", &text);
    let (files, _) = run(root, &SearchQuery::new("NEEDLE"), &config(&[]));
    let lines: Vec<u64> = files[0].lines.iter().map(|l| l.line).collect();
    assert_eq!(lines, vec![7, 50_007, 100_007, 150_007]);
}

#[test]
fn multiline_regex() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "a.cs", "void A()\n{\n}\nvoid B() {}\n");
    let q = SearchQuery {
        is_regex: true,
        ..SearchQuery::new(r"\(\)\n\{")
    };
    let (files, _) = run(root, &q, &config(&[]));
    assert_eq!(files[0].lines.len(), 1);
    assert_eq!(files[0].lines[0].line, 0);
    assert_eq!(files[0].lines[0].ranges, vec![6..8]);
}

/// 零宽命中保留为空区间（#145）：`^` 在 3 行文件里给出 3 条结果，与文件内查找的约定一致。
#[test]
fn zero_width_regex_matches_are_kept_as_empty_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "a.cs", "one\n    two\nthree\n");
    let q = SearchQuery {
        is_regex: true,
        ..SearchQuery::new("^")
    };
    let (files, summary) = run(root, &q, &config(&[]));
    assert_eq!(files.len(), 1);
    let lines: Vec<u64> = files[0].lines.iter().map(|l| l.line).collect();
    assert_eq!(lines, vec![0, 1, 2]);
    assert_eq!(summary.matches, 3);
    for line in &files[0].lines {
        assert_eq!(line.ranges, vec![0..0]);
        // 行首的零宽命中落在预览起点上，不能被当作“在预览之前”丢掉。
        assert_eq!(line.preview_ranges().collect::<Vec<_>>(), vec![0..0]);
    }
    // 行尾的零宽命中在行内容末尾，缩进照常不进预览。
    let q = SearchQuery {
        is_regex: true,
        ..SearchQuery::new("$")
    };
    let (files, _) = run(root, &q, &config(&[]));
    let ends: Vec<(u64, Vec<std::ops::Range<u32>>)> = files[0]
        .lines
        .iter()
        .map(|l| (l.line, l.ranges.clone()))
        .collect();
    assert_eq!(
        ends,
        vec![(0, vec![3..3]), (1, vec![7..7]), (2, vec![5..5])]
    );
    assert_eq!(&*files[0].lines[1].preview, "two");
    assert_eq!(
        files[0].lines[1].preview_ranges().collect::<Vec<_>>(),
        vec![3..3]
    );
}

/// 独立的 `\r` 也是换行（#145）：报告的行列与编辑器 `TextSource::position` 一致。
#[test]
fn lone_cr_is_a_line_break_like_the_editor() {
    use rove_base::{ByteOffset, Position};
    use rove_buffer::{RopeText, TextSource};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let content = "foo\rneedle\r\nbar\r  needle\nneedle\n";
    write(root, "cr.txt", content);
    let (files, summary) = run(root, &SearchQuery::new("needle"), &config(&[]));
    assert_eq!(summary.matches, 3);
    let got: Vec<(u64, u32, &str)> = files[0]
        .lines
        .iter()
        .map(|l| (l.line, l.ranges[0].start, &*l.preview))
        .collect();
    assert_eq!(
        got,
        vec![(1, 0, "needle"), (3, 2, "needle"), (4, 0, "needle")]
    );
    // 逐条与编辑器的行列换算对照。
    let text = RopeText::from_text(content);
    for ((line, column, _), (offset, _)) in got.iter().zip(content.match_indices("needle")) {
        assert_eq!(
            text.position(ByteOffset(offset as u64)).unwrap(),
            Position {
                line: rove_base::Line(*line),
                column: rove_base::Utf8Column(*column as u64),
            }
        );
    }
}

/// 大型 Unity 项目上的首批与全量耗时（热缓存）。未设置 `ROVE_TEST_UNITY_LARGE` 时跳过。
#[test]
fn unity_large_common_identifier_timing() {
    let project = rove_test_support::require_unity_project!(rove_test_support::UnityProject::Large);
    if cfg!(debug_assertions) {
        eprintln!("已跳过：大型项目计时只在 release 构建下有意义（cargo test --release -p rove-search unity）");
        return;
    }
    let q = SearchQuery::new("GetComponent");
    for threads in [0.0, 16.0] {
        let cfg = config(&[(settings::MAX_THREADS, Value::Number(threads))]);
        // 预热文件缓存。
        let _ = search(&project, &q, &cfg, &Cancellation::default(), |_| {});
        for _ in 0..2 {
            let start = Instant::now();
            let mut first = None;
            let summary = search(&project, &q, &cfg, &Cancellation::default(), |_| {
                first.get_or_insert_with(|| start.elapsed());
            })
            .unwrap();
            let total = start.elapsed();
            eprintln!(
                "大型项目 GetComponent（maxThreads={threads}）：首批 {:?}，全部 {:?}，文件 {}，命中文件 {}，匹配 {}，跳过 {}，上限 {}",
                first, total, summary.files_searched, summary.files_with_matches, summary.matches,
                summary.skipped, summary.limit_hit
            );
        }
    }
}
