//! Integration tests for the M1 language configurations (#12).
//!
//! Fixtures ending in `.json` are read from `data/vscode-extensions/` by walking
//! the directory instead of naming files, so adding a configuration fails here
//! until it is registered in the built-in catalog and its regular expressions
//! compile. Expected values for words and folds were generated with a direct port
//! of VS Code 1.145.2 `src/vs/editor/common/core/wordHelper.ts` (`getWordAtText`)
//! and `src/vs/editor/contrib/folding/browser/indentRangeProvider.ts`
//! (`computeRanges`), both MIT; the fixtures below are byte-for-byte those inputs.
use rove_base::ByteOffset;
use rove_syntax::language_config::{
    self, BuiltinLanguages, CompiledLanguageConfiguration, DEFAULT_AUTO_CLOSE_BEFORE,
    DEFAULT_WORD_PATTERN,
};
use rove_syntax::{Fold, FoldKind, LanguageConfigurations};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

/// (language, text, cursor byte offset, expected range, expected word).
type WordCase = (
    &'static str,
    &'static str,
    u64,
    Option<(u64, u64)>,
    &'static str,
);
/// (text, cursor byte offset, expected range) for the default word pattern.
type DefaultWordCase = (&'static str, u64, Option<(u64, u64)>);

fn data_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data/vscode-extensions")
}

/// Every `*.json` file below `data/vscode-extensions/`, as `/`-separated paths.
fn configuration_files() -> Vec<String> {
    fn walk(base: &Path, dir: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("read data directory") {
            let entry = entry.expect("directory entry");
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, found);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                let relative = path.strip_prefix(base).expect("inside data root");
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut found = Vec::new();
    walk(&data_root(), &data_root(), &mut found);
    found.sort();
    found
}

/// Language ID for a data directory, mirroring the catalog's extension names.
fn language_for(relative_path: &str) -> &'static str {
    match relative_path.split('/').next().expect("directory") {
        "csharp" => "csharp",
        "json" => "json",
        "markdown-basics" => "markdown",
        "xml" => "xml",
        "yaml" => "yaml",
        other => panic!("unregistered configuration directory {other:?}"),
    }
}

fn load_configuration(relative_path: &str) -> CompiledLanguageConfiguration {
    let path = data_root().join(relative_path);
    let text = std::fs::read_to_string(&path).expect("read configuration");
    let language = language_for(relative_path);
    let config_path = format!("extensions/{relative_path}");
    let data = language_config::parse_language_configuration(language, &config_path, &text)
        .unwrap_or_else(|error| panic!("{relative_path}: {error}"));
    CompiledLanguageConfiguration::compile(language, &config_path, Arc::new(data))
        .unwrap_or_else(|error| panic!("{relative_path}: {error}"))
}

fn fold_ranges(folds: &[Fold], kind: FoldKind) -> Vec<(u64, u64)> {
    folds
        .iter()
        .filter(|fold| std::mem::discriminant(&fold.kind) == std::mem::discriminant(&kind))
        .map(|fold| (fold.lines.start().0, fold.lines.end().0))
        .collect()
}

#[test]
fn builtins_load_all_m1_languages() {
    let builtins = BuiltinLanguages::load().expect("built-in configurations load");
    let ids: Vec<&str> = builtins.languages().collect();
    assert_eq!(ids, ["csharp", "json", "jsonc", "markdown", "xml", "yaml"]);
    for id in &ids {
        let data = builtins.get(id).unwrap_or_else(|| panic!("{id} is served"));
        let compiled = builtins.compiled(id).expect("compiled view");
        assert!(Arc::ptr_eq(&data, compiled.data()));
        assert!(!data.brackets.is_empty(), "{id} has brackets");
    }
    assert!(builtins.get("lua").is_none());
    assert!(builtins.get("jsonl").is_none());
}

#[test]
fn field_level_assertions_match_the_vscode_files() {
    let builtins = BuiltinLanguages::load().unwrap();

    // csharp: comments, three brackets, quoted auto-closing pairs that are
    // suppressed inside strings and comments, surrounding/colorized extra `< >`,
    // #region markers, and the two on-enter rules (the second needs `afterText`).
    let csharp = builtins.get("csharp").unwrap();
    assert_eq!(csharp.line_comment.as_deref(), Some("//"));
    assert_eq!(
        csharp
            .block_comment
            .as_ref()
            .map(|pair| (pair.open.as_ref(), pair.close.as_ref())),
        Some(("/*", "*/"))
    );
    assert_eq!(
        csharp
            .brackets
            .iter()
            .map(|pair| (pair.open.as_ref(), pair.close.as_ref()))
            .collect::<Vec<_>>(),
        [("{", "}"), ("[", "]"), ("(", ")")]
    );
    assert_eq!(csharp.auto_closing.len(), 5);
    assert_eq!(csharp.surrounding.len(), 6);
    assert_eq!(csharp.colorized_brackets.as_ref().unwrap().len(), 4);
    assert_eq!(
        csharp
            .colorized_brackets
            .as_ref()
            .unwrap()
            .last()
            .map(|pair| (pair.open.as_ref(), pair.close.as_ref())),
        Some(("<", ">"))
    );
    assert_eq!(
        csharp
            .folding_markers
            .as_ref()
            .map(|(start, end)| (start.source.as_ref(), end.source.as_ref())),
        Some(("^\\s*#region\\b", "^\\s*#endregion\\b"))
    );
    assert!(!csharp.folding_off_side);
    assert!(csharp.word_pattern.is_none());
    assert!(csharp.indentation.is_none());
    assert_eq!(csharp.on_enter.len(), 2);
    assert_eq!(csharp.on_enter[0].append_text.as_deref(), Some("/// "));
    assert_eq!(csharp.on_enter[1].append_text.as_deref(), Some("// "));
    assert_eq!(
        csharp.on_enter[1]
            .after
            .as_ref()
            .map(|js| js.source.as_ref()),
        Some("^(?!\\s*$)")
    );

    // json/jsonc share one file: two brackets, cautious auto-closing (notIn for
    // strings, plus comments for quotes), indentation rules and one on-enter rule.
    for id in ["json", "jsonc"] {
        let json = builtins.get(id).unwrap();
        assert_eq!(json.line_comment.as_deref(), Some("//"));
        assert_eq!(json.brackets.len(), 2);
        assert_eq!(json.auto_closing.len(), 6);
        assert!(json.colorized_brackets.is_none());
        assert!(json.surrounding.is_empty());
        assert_eq!(json.on_enter.len(), 1);
        let rules = json.indentation.as_ref().unwrap();
        assert_eq!(
            rules.increase.source.as_ref(),
            "({+(?=((\\\\.|[^\"\\\\])*\"(\\\\.|[^\"\\\\])*\")*[^\"}]*)$)|(\\[+(?=((\\\\.|[^\"\\\\])*\"(\\\\.|[^\"\\\\])*\")*[^\"\\]]*)$)"
        );
        assert_eq!(rules.decrease.source.as_ref(), "^\\s*[}\\]],?\\s*$");
        assert_eq!(
            json.folding_markers
                .as_ref()
                .map(|(start, _)| start.source.as_ref()),
            Some("^\\s*//\\s*#?region\\b")
        );
        assert!(!json.folding_off_side);
        assert!(json.word_pattern.is_none());
    }

    // markdown: HTML block comments, off-side folding with `<!-- #region -->`
    // markers, and the Unicode `wordPattern` with the `ug` flags.
    let markdown = builtins.get("markdown").unwrap();
    assert!(markdown.line_comment.is_none());
    assert_eq!(
        markdown
            .block_comment
            .as_ref()
            .map(|pair| (pair.open.as_ref(), pair.close.as_ref())),
        Some(("<!--", "-->"))
    );
    assert_eq!(markdown.brackets.len(), 3);
    assert_eq!(markdown.colorized_brackets.as_ref().unwrap().len(), 0);
    assert_eq!(markdown.auto_closing.len(), 4);
    assert_eq!(markdown.surrounding.len(), 11);
    assert!(markdown.folding_off_side);
    assert_eq!(
        markdown
            .folding_markers
            .as_ref()
            .map(|(start, end)| (start.source.as_ref(), end.source.as_ref())),
        Some((
            "^\\s*<!--\\s*#?region\\b.*-->",
            "^\\s*<!--\\s*#?endregion\\b.*-->"
        ))
    );
    let word = markdown.word_pattern.as_ref().unwrap();
    assert!(word.source.contains("\\p{Alphabetic}"));
    assert_eq!(word.flags.as_ref(), "ug");
    assert!(markdown.on_enter.is_empty());

    // xml: four brackets including the comment pair, `wordPattern` without flags
    // (VS Code adds `g`), an empty explicit colorized-bracket list.
    let xml = builtins.get("xml").unwrap();
    assert!(xml.line_comment.is_none());
    assert_eq!(
        xml.brackets
            .iter()
            .map(|pair| pair.open.as_ref())
            .collect::<Vec<_>>(),
        ["<!--", "<", "{", "("]
    );
    assert_eq!(xml.auto_closing.len(), 6);
    assert_eq!(xml.colorized_brackets.as_ref().unwrap().len(), 0);
    assert!(!xml.folding_off_side);
    let word = xml.word_pattern.as_ref().unwrap();
    assert!(word.source.starts_with("(-?\\d*\\.\\d\\w*)"));
    assert_eq!(word.flags.as_ref(), "");
    assert!(xml.indentation.is_none());

    // yaml: `#` line comments only, off-side folding, indentation rules whose
    // increase pattern really contains the `&amp;` token from the upstream file.
    let yaml = builtins.get("yaml").unwrap();
    assert_eq!(yaml.line_comment.as_deref(), Some("#"));
    assert!(yaml.block_comment.is_none());
    assert_eq!(yaml.brackets.len(), 3);
    assert_eq!(yaml.auto_closing.len(), 5);
    assert_eq!(yaml.surrounding.len(), 5);
    assert!(yaml.folding_off_side);
    assert_eq!(
        yaml.folding_markers
            .as_ref()
            .map(|(start, _)| start.source.as_ref()),
        Some("^\\s*#\\s*region\\b")
    );
    let rules = yaml.indentation.as_ref().unwrap();
    assert_eq!(
        rules.increase.source.as_ref(),
        "^\\s*.*(:|-) ?(&amp;\\w+)?(\\{[^}\"']*|\\([^)\"']*)?$"
    );
    assert_eq!(rules.decrease.source.as_ref(), "^\\s+\\}$");
    assert!(yaml.word_pattern.is_none());
    assert!(yaml.on_enter.is_empty());

    // Every configuration that omits autoCloseBefore uses VS Code's default.
    assert!(csharp.auto_close_before.is_none());
    assert_eq!(
        language_config::auto_close_before(&csharp),
        DEFAULT_AUTO_CLOSE_BEFORE
    );
}

#[test]
fn every_regex_in_every_configuration_compiles() {
    let files = configuration_files();
    assert_eq!(files.len(), 5, "M1 ships five files: {files:?}");
    let registered: BTreeSet<String> = language_config::builtin_sources()
        .map(|source| source.relative_path.to_string())
        .collect();
    let mut regex_counts = BTreeSet::new();
    for relative_path in &files {
        assert!(
            registered.contains(relative_path),
            "{relative_path} is not in the built-in catalog"
        );
        let compiled = load_configuration(relative_path);
        let regexes = language_config::javascript_regexes(compiled.data());
        assert!(
            !regexes.is_empty(),
            "{relative_path} declares no regular expressions"
        );
        // Compilation already validated every field; record the fields so a
        // removed field is visible too.
        for (field, js) in &regexes {
            assert!(!js.source.is_empty(), "{relative_path} {field} is empty");
        }
        regex_counts.insert((relative_path.clone(), regexes.len()));
    }
    let counts: Vec<(&str, usize)> = regex_counts
        .iter()
        .map(|(path, count)| (path.as_str(), *count))
        .collect();
    assert_eq!(
        counts,
        [
            ("csharp/language-configuration.json", 5),
            ("json/language-configuration.json", 6),
            ("markdown-basics/language-configuration.json", 3),
            ("xml/xml.language-configuration.json", 3),
            ("yaml/language-configuration.json", 4),
        ]
    );
}

#[test]
fn word_pattern_agrees_with_vscode_for_markdown_and_xml() {
    // (language, text, cursor byte offset, expected word byte range, expected word)
    // from VS Code's getWordAtText; non-ASCII offsets are converted from UTF-16
    // columns to the UTF-8 byte offsets the Rove contract uses.
    let cases: &[WordCase] = &[
        (
            "markdown",
            "hello **bold** world",
            10,
            Some((8, 12)),
            "bold",
        ),
        (
            "markdown",
            "snake_case and kebab-case",
            3,
            Some((0, 10)),
            "snake_case",
        ),
        (
            "markdown",
            "\u{4E2D}\u{6587} words 2",
            3,
            Some((0, 6)),
            "\u{4E2D}\u{6587}",
        ),
        ("markdown", "e\u{301} vowel", 0, Some((0, 3)), "e\u{301}"),
        ("markdown", "hello world", 5, Some((0, 5)), "hello"),
        ("markdown", "hello world", 6, Some((6, 11)), "world"),
        ("markdown", "**", 1, None, ""),
        (
            "markdown",
            "<https://example.com/path>",
            10,
            Some((9, 16)),
            "example",
        ),
        (
            "markdown",
            "abcdefghijklmnopqrst",
            18,
            Some((0, 20)),
            "abcdefghijklmnopqrst",
        ),
        ("xml", "<b>hello</b> 3.14 foo-bar", 5, Some((3, 8)), "hello"),
        (
            "xml",
            "<b>hello</b> 3.14 foo-bar",
            14,
            Some((13, 17)),
            "3.14",
        ),
        (
            "xml",
            "<b>hello</b> 3.14 foo-bar",
            19,
            Some((18, 25)),
            "foo-bar",
        ),
        ("xml", "a.b", 1, Some((0, 1)), "a"),
        ("xml", "version=\"1.2.3\"", 12, Some((9, 12)), "1.2"),
    ];
    let builtins = BuiltinLanguages::load().unwrap();
    let markdown = builtins.compiled("markdown").unwrap();
    let xml = builtins.compiled("xml").unwrap();
    for (language, text, cursor, expected_range, expected_word) in cases {
        let configuration = match *language {
            "markdown" => markdown,
            "xml" => xml,
            other => panic!("unexpected language {other}"),
        };
        let range = configuration
            .word_at(text, ByteOffset(*cursor))
            .unwrap_or_else(|error| panic!("{language} {text:?} @ {cursor}: {error}"));
        let actual = range.map(|range| (range.start().0, range.end().0));
        assert_eq!(actual, *expected_range, "{language} {text:?} @ {cursor}");
        if let Some((start, end)) = actual {
            assert_eq!(
                &text[start as usize..end as usize],
                *expected_word,
                "{language} {text:?} @ {cursor}"
            );
        }
    }
}

#[test]
fn languages_without_word_pattern_use_the_vscode_default() {
    // Expected values from VS Code's getWordAtText with DEFAULT_WORD_REGEXP.
    let builtins = BuiltinLanguages::load().unwrap();
    for id in ["csharp", "json", "jsonc", "yaml"] {
        let configuration = builtins.compiled(id).unwrap();
        assert_eq!(
            configuration.word_pattern().javascript().source.as_ref(),
            DEFAULT_WORD_PATTERN,
            "{id} uses the default word pattern"
        );
    }
    let csharp = builtins.compiled("csharp").unwrap();
    let cases: &[DefaultWordCase] = &[
        ("var x = 1.5 + y;", 8, Some((8, 11))),
        ("System.Console.WriteLine", 8, Some((7, 14))),
        ("snake_case", 3, Some((0, 10))),
    ];
    for (text, cursor, expected) in cases {
        let actual = csharp
            .word_at(text, ByteOffset(*cursor))
            .unwrap()
            .map(|range| (range.start().0, range.end().0));
        assert_eq!(actual, *expected, "{text:?} @ {cursor}");
    }
}

#[test]
fn region_markers_nest_and_unpaired_markers_do_not_fold() {
    let builtins = BuiltinLanguages::load().unwrap();
    let csharp = builtins.compiled("csharp").unwrap();

    let nested = "#region Outer\nclass C\n{\n    #region Inner\n    void M() { }\n    #endregion\n}\n#endregion";
    let folds = csharp.fold_ranges(nested, 4).unwrap();
    assert_eq!(fold_ranges(&folds, FoldKind::Region), [(0, 8), (3, 6)]);
    assert_eq!(fold_ranges(&folds, FoldKind::Indentation), [(2, 6)]);

    // An unmatched `#endregion` is ignored ...
    let unmatched_end = "class C\n{\n}\n#endregion";
    let folds = csharp.fold_ranges(unmatched_end, 4).unwrap();
    assert!(fold_ranges(&folds, FoldKind::Region).is_empty());

    // ... and an unmatched `#region` only closes the indentation block above it.
    let unmatched_start = "class C\n{\n    #region Inner\n    void M() { }\n}";
    let folds = csharp.fold_ranges(unmatched_start, 4).unwrap();
    assert!(fold_ranges(&folds, FoldKind::Region).is_empty());
    assert_eq!(fold_ranges(&folds, FoldKind::Indentation), [(1, 4)]);
}

#[test]
fn off_side_blank_lines_belong_to_the_next_block() {
    let builtins = BuiltinLanguages::load().unwrap();
    // Expected values from VS Code's computeRanges with offSide on/off.
    for (language, text, on, off) in [
        ("yaml", "top:\n  child: 1\n\nother: 2", (0, 2), (0, 3)),
        ("markdown", "- item\n  - nested\n\nend", (0, 2), (0, 3)),
    ] {
        let configuration = builtins.compiled(language).unwrap();
        assert!(
            configuration.data().folding_off_side,
            "{language} sets folding.offSide"
        );
        let folds = configuration.fold_ranges(text, 4).unwrap();
        assert_eq!(
            fold_ranges(&folds, FoldKind::Indentation),
            [on],
            "{language} with offSide"
        );

        let mut without_off_side = (**configuration.data()).clone();
        without_off_side.folding_off_side = false;
        let without =
            CompiledLanguageConfiguration::compile(language, "test", Arc::new(without_off_side))
                .unwrap();
        let folds = without.fold_ranges(text, 4).unwrap();
        assert_eq!(
            fold_ranges(&folds, FoldKind::Indentation),
            [off],
            "{language} without offSide"
        );
    }
}

#[test]
fn auto_closing_not_in_is_parsed_for_every_configuration() {
    let builtins = BuiltinLanguages::load().unwrap();

    let csharp = builtins.get("csharp").unwrap();
    let quote = csharp
        .auto_closing
        .iter()
        .find(|pair| pair.pair.open.as_ref() == "\"")
        .expect("quote pair");
    assert_eq!(
        quote
            .not_in
            .iter()
            .map(|scope| scope.as_ref())
            .collect::<Vec<_>>(),
        ["string", "comment"]
    );

    let json = builtins.get("jsonc").unwrap();
    let backtick = json
        .auto_closing
        .iter()
        .find(|pair| pair.pair.open.as_ref() == "`")
        .expect("backtick pair");
    assert_eq!(backtick.not_in.len(), 2);
    let brace = json
        .auto_closing
        .iter()
        .find(|pair| pair.pair.open.as_ref() == "{")
        .expect("brace pair");
    assert_eq!(brace.not_in.len(), 1);
    assert_eq!(brace.not_in[0].as_ref(), "string");

    let xml = builtins.get("xml").unwrap();
    let comment = xml
        .auto_closing
        .iter()
        .find(|pair| pair.pair.open.as_ref() == "<!--")
        .expect("comment pair");
    assert_eq!(
        comment
            .not_in
            .iter()
            .map(|scope| scope.as_ref())
            .collect::<Vec<_>>(),
        ["comment", "string"]
    );

    let markdown = builtins.get("markdown").unwrap();
    let angle = markdown
        .auto_closing
        .iter()
        .find(|pair| pair.pair.open.as_ref() == "<")
        .expect("angle pair");
    assert_eq!(angle.not_in.as_ref(), [Arc::<str>::from("string")]);

    // yaml's pairs are plain tuples without `notIn`.
    let yaml = builtins.get("yaml").unwrap();
    assert!(yaml.auto_closing.iter().all(|pair| pair.not_in.is_empty()));
}

#[test]
fn unsupported_regex_feature_reports_configuration_and_field() {
    let path = "extensions/example/language-configuration.json";
    let cases: &[(&str, &str)] = &[
        (
            r#"{"wordPattern": {"pattern": "[\\D]", "flags": ""}}"#,
            "wordPattern.pattern",
        ),
        (
            r#"{"wordPattern": {"pattern": "x", "flags": "v"}}"#,
            "wordPattern.pattern",
        ),
        (
            r#"{"folding": {"markers": {"start": "(abc", "end": "x"}}}"#,
            "folding.markers.start",
        ),
        (
            r#"{"indentationRules": {"increaseIndentPattern": "[abc", "decreaseIndentPattern": "x"}}"#,
            "indentationRules.increaseIndentPattern",
        ),
    ];
    for (jsonc, expected_field) in cases {
        let data = language_config::parse_language_configuration("example", path, jsonc).unwrap();
        let error = CompiledLanguageConfiguration::compile("example", path, Arc::new(data))
            .err()
            .unwrap_or_else(|| panic!("{jsonc} must fail"));
        assert_eq!(error.language.as_ref(), "example");
        assert_eq!(error.config.as_ref(), path);
        assert_eq!(error.field.as_ref(), *expected_field, "{jsonc}");
        let message = error.to_string();
        assert!(message.contains(path), "{message}");
        assert!(message.contains(expected_field), "{message}");
        // Never silently degrade to "never matches".
        assert!(!error.reason.is_empty());
    }
}

#[test]
fn language_id_lookup_serves_brackets_words_and_folds() {
    let builtins = BuiltinLanguages::load().unwrap();
    // Downstream #20/#45/#52 consume the frozen trait object.
    let languages: &dyn LanguageConfigurations = &builtins;

    let csharp = languages.get("csharp").expect("csharp");
    assert_eq!(
        csharp
            .brackets
            .iter()
            .map(|pair| pair.open.as_ref())
            .collect::<Vec<_>>(),
        ["{", "[", "("]
    );
    assert_eq!(csharp.line_comment.as_deref(), Some("//"));

    let compiled = builtins.compiled("csharp").unwrap();
    let word = compiled
        .word_at("class Widget { }", ByteOffset(6))
        .unwrap()
        .expect("word at offset 6");
    assert_eq!(
        &"class Widget { }"[word.start().0 as usize..word.end().0 as usize],
        "Widget"
    );

    let folds = compiled
        .fold_ranges(
            "class Widget\n{\n    void Run()\n    {\n        var x = 1;\n    }\n}",
            4,
        )
        .unwrap();
    assert_eq!(fold_ranges(&folds, FoldKind::Indentation), [(1, 6), (3, 5)]);

    let markdown = languages.get("markdown").expect("markdown");
    assert!(markdown.word_pattern.is_some());
    assert!(markdown.folding_off_side);

    let yaml = builtins.compiled("yaml").unwrap();
    assert_eq!(
        yaml.word_at("key: value", ByteOffset(5))
            .unwrap()
            .map(|range| (range.start().0, range.end().0)),
        Some((5, 10))
    );

    assert!(languages.get("plaintext").is_none());
    assert!(languages.get("").is_none());
}

#[test]
fn builtins_are_send_sync_and_share_data() {
    fn assert_contract<T: LanguageConfigurations>() {}
    assert_contract::<BuiltinLanguages>();

    let builtins = BuiltinLanguages::load().unwrap();
    let first = builtins.get("json").unwrap();
    let second = builtins.get("jsonc").unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let handle = std::thread::spawn(move || {
        let languages: Arc<dyn LanguageConfigurations> = Arc::new(builtins);
        languages.get("yaml").is_some()
    });
    assert!(handle.join().unwrap());
}
