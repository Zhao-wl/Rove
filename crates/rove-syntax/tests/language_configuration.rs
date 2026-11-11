//! Integration tests for the M1 language configurations (#12).
//!
//! Fixtures ending in `.json` are found by walking `data/vscode-extensions/`
//! instead of naming files, so a configuration that is added or replaced is
//! parsed and compiled without editing the test (registration in the catalog is
//! still asserted, because language IDs must be mapped explicitly).
//!
//! Expected values are not produced by the Rust implementation:
//!
//! - Words: samples are hand-constructed; expected ranges come from running
//!   VS Code 1.145.2's own `src/vs/editor/common/core/wordHelper.ts`
//!   (`getWordAtText`, MIT) in Node.js 22 on the same inputs. VS Code has no unit
//!   test for that function, so its source is the reference.
//! - Folds: samples are hand-constructed around the same shape as VS Code's
//!   `src/vs/editor/contrib/folding/test/browser/foldingModel.test.ts`
//!   (nested markers, offSide blank lines); expected ranges come from running
//!   VS Code 1.145.2's `indentRangeProvider.ts` (`computeRanges`, MIT) in
//!   Node.js 22 on the same inputs.
//! - Regular expressions: samples are hand-constructed to include the
//!   JavaScript-vs-Rust differences that matter (`\d`, `\w` and `\b` are ASCII,
//!   `\s` includes U+FEFF, `.` excludes `\r`, `[]`/`[^]`); expected outcomes are
//!   `new RegExp(source, flags)` results from Node.js 22, the ECMAScript engine
//!   VS Code runs.
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

/// (text, cursor byte offset, expected range) for `word_at`.
type WordCase = (&'static str, u64, Option<(u64, u64)>);
/// (language, text, cursor byte offset, expected range, expected word).
type WordAtCase = (
    &'static str,
    &'static str,
    u64,
    Option<(u64, u64)>,
    &'static str,
);
/// (language, field, text, does the regular expression match anywhere?).
type RegexCase = (&'static str, &'static str, &'static str, bool);
/// (language, field, text, expected first match of `wordPattern`).
type WordPatternCase = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
);

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

/// Language ID that the catalog maps to `relative_path`, if any.
fn registered_language(relative_path: &str) -> Option<&'static str> {
    let config_path = format!("extensions/{relative_path}");
    language_config::builtin_languages()
        .find(|(_, path)| *path == config_path)
        .map(|(language, _)| language)
}

/// Parse and compile one configuration file; an unregistered file is compiled
/// under its directory name so the walk still covers it.
fn load_configuration(relative_path: &str) -> CompiledLanguageConfiguration {
    let language = registered_language(relative_path)
        .unwrap_or_else(|| relative_path.split('/').next().expect("directory"));
    let path = data_root().join(relative_path);
    let text = std::fs::read_to_string(&path).expect("read configuration");
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
    assert!(!files.is_empty(), "no configuration files found");
    let registered: BTreeSet<&str> = language_config::builtin_sources()
        .map(|source| source.relative_path)
        .collect();
    // Every catalog entry must exist on disk, and every file must compile.
    for source in &registered {
        assert!(
            files.iter().any(|file| file == source),
            "catalog entry {source} is missing on disk"
        );
    }
    let mut unregistered = Vec::new();
    for relative_path in &files {
        if registered_language(relative_path).is_none() {
            unregistered.push(relative_path.clone());
        }
        let compiled = load_configuration(relative_path);
        let regexes = language_config::javascript_regexes(compiled.data());
        assert!(
            !regexes.is_empty(),
            "{relative_path} declares no regular expressions"
        );
        for (field, js) in &regexes {
            assert!(!js.source.is_empty(), "{relative_path} {field} is empty");
        }
    }
    assert!(
        unregistered.is_empty(),
        "add these files to the built-in catalog: {unregistered:?}"
    );
    assert_eq!(files.len(), registered.len());
}

#[test]
fn every_language_regex_matches_javascript_samples() {
    // See the module docs for how the expected column was produced.
    const REGEX_SAMPLES: &[RegexCase] = &[
        ("csharp", "folding.markers.start", "#region", true),
        ("csharp", "folding.markers.start", "    #region Outer", true),
        ("csharp", "folding.markers.start", "#regionX", false),
        ("csharp", "folding.markers.start", "#region\u{E9}", true),
        ("csharp", "folding.markers.start", "/// #region", false),
        ("csharp", "folding.markers.start", "#endregion", false),
        ("csharp", "folding.markers.start", "<!-- #region -->", false),
        ("csharp", "folding.markers.start", "<!--#region-->", false),
        (
            "csharp",
            "folding.markers.start",
            "  <!-- region -->",
            false,
        ),
        (
            "csharp",
            "folding.markers.start",
            "<!-- #endregion -->",
            false,
        ),
        (
            "csharp",
            "folding.markers.start",
            "<!-- #regionX -->",
            false,
        ),
        ("csharp", "folding.markers.start", "// #region", false),
        ("csharp", "folding.markers.start", "//#region", false),
        ("csharp", "folding.markers.start", "   //   region", false),
        ("csharp", "folding.markers.start", "   //#region", false),
        ("csharp", "folding.markers.start", "// #endregion", false),
        ("csharp", "folding.markers.start", "# region", false),
        ("csharp", "folding.markers.start", "  #  region", false),
        ("csharp", "folding.markers.end", "#endregion", true),
        ("csharp", "folding.markers.end", "\t#endregion", true),
        ("csharp", "folding.markers.end", "#endregionx", false),
        ("csharp", "folding.markers.end", "#region", false),
        (
            "csharp",
            "folding.markers.end",
            "<!-- #endregion -->",
            false,
        ),
        ("csharp", "folding.markers.end", "<!--#endregion-->", false),
        ("csharp", "folding.markers.end", "// #endregion", false),
        ("csharp", "folding.markers.end", "//#endregion", false),
        ("csharp", "folding.markers.end", "// #region", false),
        ("csharp", "folding.markers.end", "# endregion", false),
        ("csharp", "folding.markers.end", "  #endregion", true),
        (
            "csharp",
            "onEnterRules[0].beforeText",
            "/// <summary>",
            true,
        ),
        ("csharp", "onEnterRules[0].beforeText", "    /// text", true),
        ("csharp", "onEnterRules[0].beforeText", "    // text", false),
        ("csharp", "onEnterRules[0].beforeText", "//", false),
        (
            "csharp",
            "onEnterRules[0].beforeText",
            "text /// here",
            false,
        ),
        ("csharp", "onEnterRules[0].beforeText", "// x", false),
        ("csharp", "onEnterRules[0].beforeText", "x // y", false),
        ("csharp", "onEnterRules[1].beforeText", "// x", true),
        ("csharp", "onEnterRules[1].beforeText", "   //x", true),
        ("csharp", "onEnterRules[1].beforeText", "x // y", true),
        ("csharp", "onEnterRules[1].beforeText", "//", false),
        ("csharp", "onEnterRules[1].beforeText", "    //", false),
        ("csharp", "onEnterRules[1].beforeText", "http://x", false),
        ("csharp", "onEnterRules[1].beforeText", "/// doc", true),
        ("csharp", "onEnterRules[1].afterText", "", false),
        ("csharp", "onEnterRules[1].afterText", "   ", false),
        ("csharp", "onEnterRules[1].afterText", "x", true),
        ("csharp", "onEnterRules[1].afterText", "  }", true),
        ("csharp", "onEnterRules[1].afterText", "\t", false),
        ("json", "folding.markers.start", "#region", false),
        ("json", "folding.markers.start", "    #region Outer", false),
        ("json", "folding.markers.start", "#regionX", false),
        ("json", "folding.markers.start", "#region\u{E9}", false),
        ("json", "folding.markers.start", "/// #region", false),
        ("json", "folding.markers.start", "#endregion", false),
        ("json", "folding.markers.start", "<!-- #region -->", false),
        ("json", "folding.markers.start", "<!--#region-->", false),
        ("json", "folding.markers.start", "  <!-- region -->", false),
        (
            "json",
            "folding.markers.start",
            "<!-- #endregion -->",
            false,
        ),
        ("json", "folding.markers.start", "<!-- #regionX -->", false),
        ("json", "folding.markers.start", "// #region", true),
        ("json", "folding.markers.start", "//#region", true),
        ("json", "folding.markers.start", "   //   region", true),
        ("json", "folding.markers.start", "   //#region", true),
        ("json", "folding.markers.start", "// #endregion", false),
        ("json", "folding.markers.start", "# region", false),
        ("json", "folding.markers.start", "  #  region", false),
        ("json", "folding.markers.end", "#endregion", false),
        ("json", "folding.markers.end", "\t#endregion", false),
        ("json", "folding.markers.end", "#endregionx", false),
        ("json", "folding.markers.end", "#region", false),
        ("json", "folding.markers.end", "<!-- #endregion -->", false),
        ("json", "folding.markers.end", "<!--#endregion-->", false),
        ("json", "folding.markers.end", "// #endregion", true),
        ("json", "folding.markers.end", "//#endregion", true),
        ("json", "folding.markers.end", "// #region", false),
        ("json", "folding.markers.end", "# endregion", false),
        ("json", "folding.markers.end", "  #endregion", false),
        ("json", "indentationRules.increaseIndentPattern", "{", true),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "  {",
            true,
        ),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "{\"a\": 1",
            false,
        ),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "{\"a\": \"b\"",
            false,
        ),
        ("json", "indentationRules.increaseIndentPattern", "}", false),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "{\"",
            false,
        ),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "{\"a\":",
            false,
        ),
        ("json", "indentationRules.increaseIndentPattern", "[{", true),
        ("json", "indentationRules.increaseIndentPattern", "x{", true),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "key:",
            false,
        ),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "key: value",
            false,
        ),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "  - item",
            false,
        ),
        ("json", "indentationRules.increaseIndentPattern", "-", false),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "key: {",
            true,
        ),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "key: {'a'",
            false,
        ),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "a: &amp;b",
            false,
        ),
        (
            "json",
            "indentationRules.increaseIndentPattern",
            "&amp;x",
            false,
        ),
        ("json", "indentationRules.decreaseIndentPattern", "}", true),
        (
            "json",
            "indentationRules.decreaseIndentPattern",
            "  ]",
            true,
        ),
        ("json", "indentationRules.decreaseIndentPattern", "},", true),
        (
            "json",
            "indentationRules.decreaseIndentPattern",
            "  ], ",
            true,
        ),
        (
            "json",
            "indentationRules.decreaseIndentPattern",
            "x}",
            false,
        ),
        (
            "json",
            "indentationRules.decreaseIndentPattern",
            "{}",
            false,
        ),
        ("json", "indentationRules.decreaseIndentPattern", "", false),
        (
            "json",
            "indentationRules.decreaseIndentPattern",
            "  }",
            true,
        ),
        ("json", "indentationRules.decreaseIndentPattern", " }", true),
        (
            "json",
            "indentationRules.decreaseIndentPattern",
            "\t}",
            true,
        ),
        (
            "json",
            "indentationRules.decreaseIndentPattern",
            "  } ",
            true,
        ),
        ("json", "onEnterRules[0].beforeText", "/// <summary>", true),
        ("json", "onEnterRules[0].beforeText", "    /// text", true),
        ("json", "onEnterRules[0].beforeText", "    // text", true),
        ("json", "onEnterRules[0].beforeText", "//", false),
        ("json", "onEnterRules[0].beforeText", "text /// here", false),
        ("json", "onEnterRules[0].beforeText", "// x", true),
        ("json", "onEnterRules[0].beforeText", "x // y", true),
        ("json", "onEnterRules[0].afterText", "", false),
        ("json", "onEnterRules[0].afterText", "   ", false),
        ("json", "onEnterRules[0].afterText", "x", true),
        ("json", "onEnterRules[0].afterText", "  }", true),
        ("json", "onEnterRules[0].afterText", "\t", false),
        ("markdown", "folding.markers.start", "#region", false),
        (
            "markdown",
            "folding.markers.start",
            "    #region Outer",
            false,
        ),
        ("markdown", "folding.markers.start", "#regionX", false),
        ("markdown", "folding.markers.start", "#region\u{E9}", false),
        ("markdown", "folding.markers.start", "/// #region", false),
        ("markdown", "folding.markers.start", "#endregion", false),
        (
            "markdown",
            "folding.markers.start",
            "<!-- #region -->",
            true,
        ),
        ("markdown", "folding.markers.start", "<!--#region-->", true),
        (
            "markdown",
            "folding.markers.start",
            "  <!-- region -->",
            true,
        ),
        (
            "markdown",
            "folding.markers.start",
            "<!-- #endregion -->",
            false,
        ),
        (
            "markdown",
            "folding.markers.start",
            "<!-- #regionX -->",
            false,
        ),
        ("markdown", "folding.markers.start", "// #region", false),
        ("markdown", "folding.markers.start", "//#region", false),
        ("markdown", "folding.markers.start", "   //   region", false),
        ("markdown", "folding.markers.start", "   //#region", false),
        ("markdown", "folding.markers.start", "// #endregion", false),
        ("markdown", "folding.markers.start", "# region", false),
        ("markdown", "folding.markers.start", "  #  region", false),
        ("markdown", "folding.markers.end", "#endregion", false),
        ("markdown", "folding.markers.end", "\t#endregion", false),
        ("markdown", "folding.markers.end", "#endregionx", false),
        ("markdown", "folding.markers.end", "#region", false),
        (
            "markdown",
            "folding.markers.end",
            "<!-- #endregion -->",
            true,
        ),
        ("markdown", "folding.markers.end", "<!--#endregion-->", true),
        ("markdown", "folding.markers.end", "// #endregion", false),
        ("markdown", "folding.markers.end", "//#endregion", false),
        ("markdown", "folding.markers.end", "// #region", false),
        ("markdown", "folding.markers.end", "# endregion", false),
        ("markdown", "folding.markers.end", "  #endregion", false),
        ("xml", "folding.markers.start", "#region", false),
        ("xml", "folding.markers.start", "    #region Outer", false),
        ("xml", "folding.markers.start", "#regionX", false),
        ("xml", "folding.markers.start", "#region\u{E9}", false),
        ("xml", "folding.markers.start", "/// #region", false),
        ("xml", "folding.markers.start", "#endregion", false),
        ("xml", "folding.markers.start", "<!-- #region -->", true),
        ("xml", "folding.markers.start", "<!--#region-->", true),
        ("xml", "folding.markers.start", "  <!-- region -->", false),
        ("xml", "folding.markers.start", "<!-- #endregion -->", false),
        ("xml", "folding.markers.start", "<!-- #regionX -->", false),
        ("xml", "folding.markers.start", "// #region", false),
        ("xml", "folding.markers.start", "//#region", false),
        ("xml", "folding.markers.start", "   //   region", false),
        ("xml", "folding.markers.start", "   //#region", false),
        ("xml", "folding.markers.start", "// #endregion", false),
        ("xml", "folding.markers.start", "# region", false),
        ("xml", "folding.markers.start", "  #  region", false),
        ("xml", "folding.markers.end", "#endregion", false),
        ("xml", "folding.markers.end", "\t#endregion", false),
        ("xml", "folding.markers.end", "#endregionx", false),
        ("xml", "folding.markers.end", "#region", false),
        ("xml", "folding.markers.end", "<!-- #endregion -->", true),
        ("xml", "folding.markers.end", "<!--#endregion-->", true),
        ("xml", "folding.markers.end", "// #endregion", false),
        ("xml", "folding.markers.end", "//#endregion", false),
        ("xml", "folding.markers.end", "// #region", false),
        ("xml", "folding.markers.end", "# endregion", false),
        ("xml", "folding.markers.end", "  #endregion", false),
        ("yaml", "folding.markers.start", "#region", true),
        ("yaml", "folding.markers.start", "    #region Outer", true),
        ("yaml", "folding.markers.start", "#regionX", false),
        ("yaml", "folding.markers.start", "#region\u{E9}", true),
        ("yaml", "folding.markers.start", "/// #region", false),
        ("yaml", "folding.markers.start", "#endregion", false),
        ("yaml", "folding.markers.start", "<!-- #region -->", false),
        ("yaml", "folding.markers.start", "<!--#region-->", false),
        ("yaml", "folding.markers.start", "  <!-- region -->", false),
        (
            "yaml",
            "folding.markers.start",
            "<!-- #endregion -->",
            false,
        ),
        ("yaml", "folding.markers.start", "<!-- #regionX -->", false),
        ("yaml", "folding.markers.start", "// #region", false),
        ("yaml", "folding.markers.start", "//#region", false),
        ("yaml", "folding.markers.start", "   //   region", false),
        ("yaml", "folding.markers.start", "   //#region", false),
        ("yaml", "folding.markers.start", "// #endregion", false),
        ("yaml", "folding.markers.start", "# region", true),
        ("yaml", "folding.markers.start", "  #  region", true),
        ("yaml", "folding.markers.end", "#endregion", true),
        ("yaml", "folding.markers.end", "\t#endregion", true),
        ("yaml", "folding.markers.end", "#endregionx", false),
        ("yaml", "folding.markers.end", "#region", false),
        ("yaml", "folding.markers.end", "<!-- #endregion -->", false),
        ("yaml", "folding.markers.end", "<!--#endregion-->", false),
        ("yaml", "folding.markers.end", "// #endregion", false),
        ("yaml", "folding.markers.end", "//#endregion", false),
        ("yaml", "folding.markers.end", "// #region", false),
        ("yaml", "folding.markers.end", "# endregion", true),
        ("yaml", "folding.markers.end", "  #endregion", true),
        ("yaml", "indentationRules.increaseIndentPattern", "{", false),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "  {",
            false,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "{\"a\": 1",
            false,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "{\"a\": \"b\"",
            false,
        ),
        ("yaml", "indentationRules.increaseIndentPattern", "}", false),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "{\"",
            false,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "{\"a\":",
            true,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "[{",
            false,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "x{",
            false,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "key:",
            true,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "key: value",
            false,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "  - item",
            false,
        ),
        ("yaml", "indentationRules.increaseIndentPattern", "-", true),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "key: {",
            true,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "key: {'a'",
            false,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "a: &amp;b",
            true,
        ),
        (
            "yaml",
            "indentationRules.increaseIndentPattern",
            "&amp;x",
            false,
        ),
        ("yaml", "indentationRules.decreaseIndentPattern", "}", false),
        (
            "yaml",
            "indentationRules.decreaseIndentPattern",
            "  ]",
            false,
        ),
        (
            "yaml",
            "indentationRules.decreaseIndentPattern",
            "},",
            false,
        ),
        (
            "yaml",
            "indentationRules.decreaseIndentPattern",
            "  ], ",
            false,
        ),
        (
            "yaml",
            "indentationRules.decreaseIndentPattern",
            "x}",
            false,
        ),
        (
            "yaml",
            "indentationRules.decreaseIndentPattern",
            "{}",
            false,
        ),
        ("yaml", "indentationRules.decreaseIndentPattern", "", false),
        (
            "yaml",
            "indentationRules.decreaseIndentPattern",
            "  }",
            true,
        ),
        ("yaml", "indentationRules.decreaseIndentPattern", " }", true),
        (
            "yaml",
            "indentationRules.decreaseIndentPattern",
            "\t}",
            true,
        ),
        (
            "yaml",
            "indentationRules.decreaseIndentPattern",
            "  } ",
            false,
        ),
    ];
    const WORD_PATTERN_SAMPLES: &[WordPatternCase] = &[
        (
            "markdown",
            "wordPattern.pattern",
            "hello **bold** world",
            Some("hello"),
        ),
        (
            "markdown",
            "wordPattern.pattern",
            "\u{4E2D}\u{6587} words",
            Some("\u{4E2D}\u{6587}"),
        ),
        (
            "markdown",
            "wordPattern.pattern",
            "e\u{301} vowel",
            Some("e\u{301}"),
        ),
        ("markdown", "wordPattern.pattern", "**", None),
        ("markdown", "wordPattern.pattern", "_x", Some("x")),
        ("markdown", "wordPattern.pattern", "3.14", Some("3")),
        ("markdown", "wordPattern.pattern", "foo-bar", Some("foo")),
        ("markdown", "wordPattern.pattern", "<b>hello</b>", Some("b")),
        ("markdown", "wordPattern.pattern", "   ", None),
        (
            "xml",
            "wordPattern.pattern",
            "hello **bold** world",
            Some("hello"),
        ),
        (
            "xml",
            "wordPattern.pattern",
            "\u{4E2D}\u{6587} words",
            Some("\u{4E2D}\u{6587}"),
        ),
        (
            "xml",
            "wordPattern.pattern",
            "e\u{301} vowel",
            Some("e\u{301}"),
        ),
        ("xml", "wordPattern.pattern", "**", None),
        ("xml", "wordPattern.pattern", "_x", Some("_x")),
        ("xml", "wordPattern.pattern", "3.14", Some("3.14")),
        ("xml", "wordPattern.pattern", "foo-bar", Some("foo-bar")),
        ("xml", "wordPattern.pattern", "<b>hello</b>", Some("b")),
        ("xml", "wordPattern.pattern", "   ", None),
    ];
    let builtins = BuiltinLanguages::load().unwrap();
    for (language, field, text, expected) in REGEX_SAMPLES {
        let configuration = builtins
            .compiled(language)
            .unwrap_or_else(|| panic!("{language} is served"));
        let compiled = language_config::javascript_regexes(configuration.data())
            .into_iter()
            .find(|(name, _)| name == field)
            .map(|(_, js)| js)
            .unwrap_or_else(|| panic!("{language} has no {field}"));
        let actual = language_config::compile_regex(
            compiled,
            &language_config::RegexOrigin::new(*language, "test", *field),
        )
        .unwrap_or_else(|error| panic!("{language} {field}: {error}"))
        .is_match(text)
        .unwrap();
        assert_eq!(actual, *expected, "{language} {field} on {text:?}");
    }
    for (language, field, text, expected) in WORD_PATTERN_SAMPLES {
        let configuration = builtins.compiled(language).expect("served");
        let found = configuration
            .word_pattern()
            .find(text)
            .unwrap()
            .map(|(start, end)| &text[start..end]);
        assert_eq!(found, *expected, "{language} {field} on {text:?}");
    }

    // Every regular expression in every shipped configuration has samples.
    let mut covered: BTreeSet<(String, String)> = BTreeSet::new();
    for (language, field, _, _) in REGEX_SAMPLES {
        covered.insert((language.to_string(), field.to_string()));
    }
    for (language, field, _, _) in WORD_PATTERN_SAMPLES {
        covered.insert((language.to_string(), field.to_string()));
    }
    // `json` and `jsonc` share one file, so a sample under either ID covers both.
    for source in language_config::builtin_sources() {
        let languages: Vec<&str> = language_config::builtin_languages()
            .filter(|(_, path)| *path == source.config_path)
            .map(|(language, _)| language)
            .collect();
        let configuration = builtins
            .compiled(languages.first().expect("shared configuration has an ID"))
            .expect("served");
        for (field, _) in language_config::javascript_regexes(configuration.data()) {
            assert!(
                languages
                    .iter()
                    .any(|language| covered.contains(&(language.to_string(), field.clone()))),
                "{languages:?} {field} has no JavaScript comparison sample"
            );
        }
    }
}

#[test]
fn word_pattern_agrees_with_vscode_for_markdown_and_xml() {
    // (language, text, cursor byte offset, expected word byte range, expected word)
    // from VS Code's getWordAtText. Non-ASCII offsets are converted from UTF-16
    // columns to the UTF-8 byte offsets the Rove contract uses.
    let cases: &[WordAtCase] = &[
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

    // Longer than the 1000-byte window: the word still resolves. Expected values
    // are hand-derived: "x" * 600 + " cursor " puts "cursor" at bytes 601..607.
    let mut long = "x".repeat(600);
    long.push_str(" cursor ");
    long.push_str(&"y".repeat(600));
    let range = markdown
        .word_at(&long, ByteOffset(603))
        .unwrap()
        .expect("word in a long line");
    assert_eq!(
        (range.start().0, range.end().0),
        (601, 607),
        "long lines use a 1000-byte window"
    );
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
    let cases: &[WordCase] = &[
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
fn compiled_markers_use_the_combined_matcher() {
    let builtins = BuiltinLanguages::load().unwrap();
    for language in ["csharp", "json", "jsonc", "markdown", "xml", "yaml"] {
        let markers = builtins
            .compiled(language)
            .unwrap()
            .folding_markers()
            .unwrap_or_else(|| panic!("{language} has folding markers"));
        // The combined matcher mirrors VS Code's `(start)|(?:end)`; the two-probe
        // fallback is covered by the unit tests.
        assert!(
            markers.is_combined(),
            "{language} uses the combined matcher"
        );
    }
}

#[test]
fn off_side_blank_lines_belong_to_the_next_block() {
    let builtins = BuiltinLanguages::load().unwrap();
    // Expected values from VS Code's computeRanges with offSide on/off.
    for (language, text, on, off) in [
        (
            "yaml",
            "top:\n  child: 1\n\nother: 2",
            (0u64, 2u64),
            (0u64, 3u64),
        ),
        (
            "markdown",
            "- item\n  - nested\n\nend",
            (0u64, 2u64),
            (0u64, 3u64),
        ),
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
    // The compiled view is shared too: json/jsonc compile their regexes once.
    assert!(std::ptr::eq(
        builtins.compiled("json").unwrap(),
        builtins.compiled("jsonc").unwrap()
    ));
    let handle = std::thread::spawn(move || {
        let languages: Arc<dyn LanguageConfigurations> = Arc::new(builtins);
        languages.get("yaml").is_some()
    });
    assert!(handle.join().unwrap());
}
