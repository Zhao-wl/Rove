//! Language configuration data (#12): the VS Code `language-configuration.json`
//! files for the M1 languages, their JavaScript regular expressions, and the
//! derived queries (brackets, word under the cursor, fold ranges).
//!
//! # Data
//!
//! The five files under `crates/rove-syntax/data/vscode-extensions/` are copied
//! verbatim from VS Code 1.145.2 (commit `c4453f8ea430795f941c1b2ff6c9321135a74a02`,
//! MIT; see `THIRD_PARTY_NOTICES.md` and the `README.md` next to the data). They
//! are compiled into the binary with `include_str!`: no runtime download, no reads
//! of a user's installed extensions, and no I/O on lookup. Six language IDs are
//! served from those five files, exactly as in VS Code (`jsonc` shares the `json`
//! extension's configuration).
//!
//! Upgrade procedure: replace the files as a unit, update [`VSCODE_VERSION`] and
//! [`VSCODE_COMMIT`], then run the crate tests. The test that walks the data
//! directory fails until every new file has a catalog entry.
//!
//! # Regular expressions
//!
//! Configuration regular expressions are ECMAScript syntax; see [`js_regex`] for
//! the translation into `fancy-regex` and for the errors that name the offending
//! configuration and field. Nothing silently degrades to "never matches".
//!
//! # Consumers
//!
//! - [`LanguageConfigurations`](crate::LanguageConfigurations) is the frozen
//!   contract: [`BuiltinLanguages::get`](crate::LanguageConfigurations::get)
//!   returns cached, immutable data. It is lossless: no compiled handles, no
//!   defaults baked in.
//! - [`BuiltinLanguages::compiled`] returns the same data plus the regular
//!   expressions compiled once, for consumers that match per keystroke or per
//!   document (#20 code view, #45 edit model, #52 comment/indent/auto-close,
//!   #26 outline for folding markers). [`BuiltinLanguages::shared`] is the
//!   process-wide instance, so those consumers do not each re-parse and recompile.
//! - [`BuiltinLanguages::load_partial`] keeps every language that loads and
//!   returns the errors: one bad field disables only its own language, as in VS
//!   Code. [`BuiltinLanguages::load`] is the all-or-nothing variant.
//! - [`CompiledLanguageConfiguration::fold_ranges`] turns `folding.markers` and
//!   `folding.offSide` into [`Fold`] ranges; `word_at` returns the word under a
//!   byte offset using `wordPattern` or VS Code's default pattern.
//!
//! Language-scoped settings (`"[csharp]"` overrides such as
//! `editor.maxTokenizationLineLength`) belong to #13: this module never parses
//! settings. Consumers read them through `rove_config::Key<T>::read` with
//! `Scope { language: Some(id) }` and keep them separate from language data.
use crate::{Fold, IndentAction, JsRegex, LanguageConfiguration, LanguageConfigurations, Pair};
use rove_base::{ByteOffset, ByteRange, Result as BaseResult};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, OnceLock},
};

pub mod folding;
pub mod js_regex;
pub mod parse;
pub mod word;

pub use folding::fold_ranges;
pub use js_regex::{compile_regex, CompiledRegex, RegexOrigin};
pub use parse::parse_language_configuration;
pub use word::{word_at, DEFAULT_WORD_PATTERN, DEFAULT_WORD_PATTERN_FLAGS};

/// VS Code release the shipped configurations were taken from.
pub const VSCODE_VERSION: &str = "1.145.2";
/// Commit of [`VSCODE_VERSION`]; the data files are byte-identical to this tree.
pub const VSCODE_COMMIT: &str = "c4453f8ea430795f941c1b2ff6c9321135a74a02";

/// VS Code's default `autoCloseBefore` for brackets, used when a configuration
/// omits the property (none of the shipped files set it).
pub const DEFAULT_AUTO_CLOSE_BEFORE: &str = "'\"`;:.,=}])> \n\t";
/// VS Code's narrower default `autoCloseBefore` for quotes; use with
/// [`auto_close_before_for`] when the caller closes a quote rather than a bracket.
pub const DEFAULT_AUTO_CLOSE_BEFORE_QUOTES: &str = ";:.,=}])> \n\t";

/// `autoCloseBefore` for brackets, falling back to VS Code's bracket default.
pub fn auto_close_before(configuration: &LanguageConfiguration) -> &str {
    auto_close_before_for(configuration, false)
}

/// `autoCloseBefore` for a language. `quotes` selects VS Code's quote default
/// rather than its bracket default when the configuration does not set the
/// property; a configured value is returned unchanged for both.
pub fn auto_close_before_for(configuration: &LanguageConfiguration, quotes: bool) -> &str {
    configuration
        .auto_close_before
        .as_deref()
        .unwrap_or(if quotes {
            DEFAULT_AUTO_CLOSE_BEFORE_QUOTES
        } else {
            DEFAULT_AUTO_CLOSE_BEFORE
        })
}

/// Bracket pairs used for colorization: `colorizedBracketPairs` when the
/// configuration defines it (even as an empty list), otherwise `brackets`.
pub fn colorized_brackets(configuration: &LanguageConfiguration) -> &[Pair] {
    configuration
        .colorized_brackets
        .as_deref()
        .unwrap_or(&configuration.brackets)
}

/// A configuration that could not be loaded. The message names the file, the
/// language ID and the exact field, e.g.
/// `language configuration "extensions/json/language-configuration.json"
/// (language "jsonc") field indentationRules.increaseIndentPattern: ...`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageConfigError {
    pub language: Arc<str>,
    pub config: Arc<str>,
    pub field: Arc<str>,
    pub reason: Arc<str>,
}
impl LanguageConfigError {
    pub fn new(
        language: impl Into<Arc<str>>,
        config: impl Into<Arc<str>>,
        field: impl Into<Arc<str>>,
        reason: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            language: language.into(),
            config: config.into(),
            field: field.into(),
            reason: reason.into(),
        }
    }
}
impl fmt::Display for LanguageConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "language configuration \"{}\" (language \"{}\") field {}: {}",
            self.config, self.language, self.field, self.reason
        )
    }
}
impl std::error::Error for LanguageConfigError {}

/// Loader result: either data or a [`LanguageConfigError`] naming the field.
pub type ConfigResult<T> = std::result::Result<T, LanguageConfigError>;

/// Catalog entry for one shipped VS Code configuration file.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinSource {
    /// Path of the file relative to `data/vscode-extensions/`.
    pub relative_path: &'static str,
    /// Path as it appears in the VS Code repository, used in error messages.
    pub config_path: &'static str,
    /// Verbatim file contents, compiled into the binary.
    pub text: &'static str,
}

const SOURCE_FILES: [BuiltinSource; 5] = [
    BuiltinSource {
        relative_path: "csharp/language-configuration.json",
        config_path: "extensions/csharp/language-configuration.json",
        text: include_str!("../../data/vscode-extensions/csharp/language-configuration.json"),
    },
    BuiltinSource {
        relative_path: "json/language-configuration.json",
        config_path: "extensions/json/language-configuration.json",
        text: include_str!("../../data/vscode-extensions/json/language-configuration.json"),
    },
    BuiltinSource {
        relative_path: "markdown-basics/language-configuration.json",
        config_path: "extensions/markdown-basics/language-configuration.json",
        text: include_str!(
            "../../data/vscode-extensions/markdown-basics/language-configuration.json"
        ),
    },
    BuiltinSource {
        relative_path: "xml/xml.language-configuration.json",
        config_path: "extensions/xml/xml.language-configuration.json",
        text: include_str!("../../data/vscode-extensions/xml/xml.language-configuration.json"),
    },
    BuiltinSource {
        relative_path: "yaml/language-configuration.json",
        config_path: "extensions/yaml/language-configuration.json",
        text: include_str!("../../data/vscode-extensions/yaml/language-configuration.json"),
    },
];

/// Language ID to catalog index. `jsonc` shares the `json` extension file, as in
/// VS Code; HLSL, ShaderLab and Lua are added together with their highlighters.
const LANGUAGE_IDS: [(&str, usize); 6] = [
    ("csharp", 0),
    ("json", 1),
    ("jsonc", 1),
    ("markdown", 2),
    ("xml", 3),
    ("yaml", 4),
];

/// Every shipped configuration file, for upgrade checks and tests.
pub fn builtin_sources() -> impl Iterator<Item = BuiltinSource> {
    SOURCE_FILES.into_iter()
}

/// Every served language ID with the VS Code path of its configuration file.
pub fn builtin_languages() -> impl Iterator<Item = (&'static str, &'static str)> {
    LANGUAGE_IDS
        .into_iter()
        .map(|(language, index)| (language, SOURCE_FILES[index].config_path))
}

/// All JavaScript regular expressions of a configuration, tagged with the field
/// path used in errors and tests.
pub fn javascript_regexes(data: &LanguageConfiguration) -> Vec<(String, &JsRegex)> {
    let mut found: Vec<(String, &JsRegex)> = Vec::new();
    if let Some((start, end)) = &data.folding_markers {
        found.push(("folding.markers.start".into(), start));
        found.push(("folding.markers.end".into(), end));
    }
    if let Some(pattern) = &data.word_pattern {
        found.push(("wordPattern.pattern".into(), pattern));
    }
    if let Some(rules) = &data.indentation {
        found.push((
            "indentationRules.increaseIndentPattern".into(),
            &rules.increase,
        ));
        found.push((
            "indentationRules.decreaseIndentPattern".into(),
            &rules.decrease,
        ));
        if let Some(pattern) = &rules.indent_next {
            found.push(("indentationRules.indentNextLinePattern".into(), pattern));
        }
        if let Some(pattern) = &rules.unindented {
            found.push(("indentationRules.unIndentedLinePattern".into(), pattern));
        }
    }
    for (index, rule) in data.on_enter.iter().enumerate() {
        found.push((format!("onEnterRules[{index}].beforeText"), &rule.before));
        if let Some(pattern) = &rule.after {
            found.push((format!("onEnterRules[{index}].afterText"), pattern));
        }
        if let Some(pattern) = &rule.previous_line {
            found.push((format!("onEnterRules[{index}].previousLineText"), pattern));
        }
    }
    found
}

/// One configuration with its regular expressions compiled once.
pub struct CompiledLanguageConfiguration {
    data: Arc<LanguageConfiguration>,
    word_pattern: CompiledRegex,
    folding: Option<CompiledFoldingMarkers>,
    indentation: Option<CompiledIndentationRules>,
    on_enter: Vec<CompiledEnterRule>,
}

/// Compiled `folding.markers`. VS Code scans each line once with
/// `(start)|(?:end)`; [`combined`](Self::is_combined) holds that matcher when both
/// sources share their flags and contain no numbered backreference (the added
/// group would renumber it).
pub struct CompiledFoldingMarkers {
    start: CompiledRegex,
    end: CompiledRegex,
    combined: Option<CompiledRegex>,
}
/// Which marker a line matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkerKind {
    Start,
    End,
}
impl CompiledFoldingMarkers {
    /// Assemble from already compiled handles plus the JavaScript sources.
    pub(crate) fn assemble(
        language: &str,
        config: &str,
        start: CompiledRegex,
        end: CompiledRegex,
        javascript: (&JsRegex, &JsRegex),
    ) -> ConfigResult<Self> {
        let combined = combine_markers(language, config, javascript.0, javascript.1)?;
        Ok(Self {
            start,
            end,
            combined,
        })
    }
    /// Compiled `folding.markers.start`.
    pub fn start(&self) -> &CompiledRegex {
        &self.start
    }
    /// Compiled `folding.markers.end`.
    pub fn end(&self) -> &CompiledRegex {
        &self.end
    }
    /// Whether one combined matcher is used instead of two probes per line.
    pub fn is_combined(&self) -> bool {
        self.combined.is_some()
    }
    /// Which marker `text` matches; start wins a tie, as in VS Code.
    pub(crate) fn kind(&self, text: &str) -> BaseResult<Option<MarkerKind>> {
        if let Some(combined) = &self.combined {
            return Ok(combined.captures_group1(text)?.map(|start| {
                if start {
                    MarkerKind::Start
                } else {
                    MarkerKind::End
                }
            }));
        }
        if self.start.find(text)?.is_some() {
            return Ok(Some(MarkerKind::Start));
        }
        if self.end.find(text)?.is_some() {
            return Ok(Some(MarkerKind::End));
        }
        Ok(None)
    }
}

/// Build VS Code's `(start)|(?:end)` matcher when that is safe.
fn combine_markers(
    language: &str,
    config: &str,
    start: &JsRegex,
    end: &JsRegex,
) -> ConfigResult<Option<CompiledRegex>> {
    if start.flags != end.flags
        || has_numbered_backreference(&start.source)
        || has_numbered_backreference(&end.source)
    {
        return Ok(None);
    }
    let javascript = JsRegex {
        source: format!("({})|(?:{})", start.source, end.source).into(),
        flags: start.flags.clone(),
    };
    compile_regex(
        &javascript,
        &RegexOrigin::new(language, config, "folding.markers"),
    )
    .map(Some)
}

/// `\1`-`\9` change meaning when the combined matcher adds a group; `\0` is
/// treated conservatively as well.
fn has_numbered_backreference(source: &str) -> bool {
    source
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'\\' && pair[1].is_ascii_digit())
}

/// Compiled `indentationRules`; fields mirror [`IndentationRules`](crate::IndentationRules).
pub struct CompiledIndentationRules {
    pub increase: CompiledRegex,
    pub decrease: CompiledRegex,
    pub indent_next: Option<CompiledRegex>,
    pub unindented: Option<CompiledRegex>,
}

/// Compiled `onEnterRules` entry; mirrors [`EnterRule`] plus its handles.
pub struct CompiledEnterRule {
    pub before: CompiledRegex,
    pub after: Option<CompiledRegex>,
    pub previous_line: Option<CompiledRegex>,
    pub indent: IndentAction,
    pub append_text: Option<Arc<str>>,
    pub remove_text: Option<u32>,
}

impl CompiledLanguageConfiguration {
    /// Parse `text` and compile every regular expression. `language` and
    /// `config` only appear in error messages.
    pub fn compile(
        language: &str,
        config: &str,
        data: Arc<LanguageConfiguration>,
    ) -> ConfigResult<Self> {
        let origin = |field: &str| RegexOrigin::new(language, config, field);
        // The single enumeration: every field listed by `javascript_regexes` is
        // compiled exactly once, and the typed views below take their handles
        // from this map instead of listing the sources again.
        let mut compiled: BTreeMap<String, CompiledRegex> = BTreeMap::new();
        for (field, javascript) in javascript_regexes(&data) {
            let handle = compile_regex(javascript, &origin(&field))?;
            compiled.insert(field, handle);
        }
        let take = |compiled: &mut BTreeMap<String, CompiledRegex>, field: &str| {
            compiled
                .remove(field)
                .expect("javascript_regexes enumerates every compiled field")
        };
        let word_pattern = match compiled.remove("wordPattern.pattern") {
            Some(pattern) => pattern,
            None => compile_regex(
                &JsRegex {
                    source: DEFAULT_WORD_PATTERN.into(),
                    flags: DEFAULT_WORD_PATTERN_FLAGS.into(),
                },
                &origin("wordPattern.pattern"),
            )?,
        };
        let folding = match &data.folding_markers {
            Some((start, end)) => Some(CompiledFoldingMarkers::assemble(
                language,
                config,
                take(&mut compiled, "folding.markers.start"),
                take(&mut compiled, "folding.markers.end"),
                (start, end),
            )?),
            None => None,
        };
        let indentation = data
            .indentation
            .as_ref()
            .map(|rules| CompiledIndentationRules {
                increase: take(&mut compiled, "indentationRules.increaseIndentPattern"),
                decrease: take(&mut compiled, "indentationRules.decreaseIndentPattern"),
                indent_next: rules
                    .indent_next
                    .as_ref()
                    .map(|_| take(&mut compiled, "indentationRules.indentNextLinePattern")),
                unindented: rules
                    .unindented
                    .as_ref()
                    .map(|_| take(&mut compiled, "indentationRules.unIndentedLinePattern")),
            });
        let mut on_enter = Vec::with_capacity(data.on_enter.len());
        for (index, rule) in data.on_enter.iter().enumerate() {
            on_enter.push(CompiledEnterRule {
                before: take(&mut compiled, &format!("onEnterRules[{index}].beforeText")),
                after: rule
                    .after
                    .as_ref()
                    .map(|_| take(&mut compiled, &format!("onEnterRules[{index}].afterText"))),
                previous_line: rule.previous_line.as_ref().map(|_| {
                    take(
                        &mut compiled,
                        &format!("onEnterRules[{index}].previousLineText"),
                    )
                }),
                indent: rule.indent.clone(),
                append_text: rule.append_text.clone(),
                remove_text: rule.remove_text,
            });
        }
        debug_assert!(
            compiled.is_empty(),
            "unused compiled regexes: {:?}",
            compiled.keys().collect::<Vec<_>>()
        );
        Ok(Self {
            data,
            word_pattern,
            folding,
            indentation,
            on_enter,
        })
    }
    /// Lossless configuration data; cheap to clone.
    pub fn data(&self) -> &Arc<LanguageConfiguration> {
        &self.data
    }
    /// `wordPattern` when defined, otherwise VS Code's default pattern.
    pub fn word_pattern(&self) -> &CompiledRegex {
        &self.word_pattern
    }
    /// Compiled `folding.markers`, if the configuration defines both markers.
    pub fn folding_markers(&self) -> Option<&CompiledFoldingMarkers> {
        self.folding.as_ref()
    }
    /// Compiled `indentationRules`, if the configuration defines them.
    pub fn indentation(&self) -> Option<&CompiledIndentationRules> {
        self.indentation.as_ref()
    }
    /// Compiled `onEnterRules` in configuration order.
    pub fn on_enter(&self) -> &[CompiledEnterRule] {
        &self.on_enter
    }
    /// See [`colorized_brackets`].
    pub fn colorized_brackets(&self) -> &[Pair] {
        colorized_brackets(&self.data)
    }
    /// Word range containing `offset`, biased to the word before the cursor when
    /// the cursor sits on its last boundary; see [`word_at`].
    pub fn word_at(&self, text: &str, offset: ByteOffset) -> BaseResult<Option<ByteRange>> {
        word_at(&self.word_pattern, text, offset)
    }
    /// Marker and indentation fold ranges from this configuration's rules.
    pub fn fold_ranges(&self, text: &str, tab_size: usize) -> BaseResult<Vec<Fold>> {
        fold_ranges(
            self.folding.as_ref(),
            self.data.folding_off_side,
            text,
            tab_size,
        )
    }
}

/// Built-in language configurations for the M1 languages; built once and shared.
pub struct BuiltinLanguages {
    languages: BTreeMap<Arc<str>, Arc<CompiledLanguageConfiguration>>,
}
impl BuiltinLanguages {
    /// Load and compile every shipped configuration, failing on the first error.
    pub fn load() -> ConfigResult<Self> {
        let (languages, errors) = Self::load_partial();
        match errors.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(languages),
        }
    }
    /// Load every shipped configuration, keeping the languages that load: one
    /// invalid field only disables its own language, as in VS Code. Errors name
    /// the file and field, and are reported once per language ID sharing a file.
    pub fn load_partial() -> (Self, Vec<LanguageConfigError>) {
        let mut languages: BTreeMap<Arc<str>, Arc<CompiledLanguageConfiguration>> = BTreeMap::new();
        let mut errors = Vec::new();
        for (index, source) in SOURCE_FILES.iter().enumerate() {
            let ids: Vec<&'static str> = LANGUAGE_IDS
                .iter()
                .filter(|(_, source_index)| *source_index == index)
                .map(|(language, _)| *language)
                .collect();
            let primary = *ids
                .first()
                .expect("every configuration file has a language id");
            let compiled = parse_language_configuration(primary, source.config_path, source.text)
                .and_then(|data| {
                    CompiledLanguageConfiguration::compile(
                        primary,
                        source.config_path,
                        Arc::new(data),
                    )
                });
            match compiled {
                Ok(compiled) => {
                    let compiled = Arc::new(compiled);
                    // `json` and `jsonc` share one file, so they share one compilation.
                    for language in ids {
                        languages.insert(Arc::from(language), compiled.clone());
                    }
                }
                Err(error) => {
                    for language in ids {
                        errors.push(LanguageConfigError {
                            language: Arc::from(language),
                            ..error.clone()
                        });
                    }
                }
            }
        }
        (Self { languages }, errors)
    }
    /// Process-wide instance compiled once for every consumer (#20/#26/#45/#52).
    /// The errors are returned rather than hidden, so startup can log them while
    /// the remaining languages stay usable.
    pub fn shared() -> std::result::Result<&'static Self, &'static [LanguageConfigError]> {
        static SHARED: OnceLock<(BuiltinLanguages, Vec<LanguageConfigError>)> = OnceLock::new();
        let (languages, errors) = SHARED.get_or_init(Self::load_partial);
        if errors.is_empty() {
            Ok(languages)
        } else {
            Err(errors)
        }
    }
    /// Served language IDs, sorted.
    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.languages.keys().map(|id| id.as_ref())
    }
    /// Configuration plus compiled regular expressions.
    pub fn compiled(&self, language: &str) -> Option<&CompiledLanguageConfiguration> {
        self.languages
            .get(language)
            .map(|compiled| compiled.as_ref())
    }
}
impl LanguageConfigurations for BuiltinLanguages {
    fn get(&self, language: &str) -> Option<Arc<LanguageConfiguration>> {
        self.languages
            .get(language)
            .map(|compiled| compiled.data.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_cover_the_m1_languages() {
        let ids: Vec<&str> = builtin_languages().map(|(id, _)| id).collect();
        assert_eq!(ids, ["csharp", "json", "jsonc", "markdown", "xml", "yaml"]);
        let sources: Vec<&str> = builtin_sources().map(|s| s.relative_path).collect();
        assert_eq!(sources.len(), 5);
        assert!(sources.contains(&"xml/xml.language-configuration.json"));
    }

    #[test]
    fn lookup_is_by_language_id_and_jsonc_shares_json() {
        let builtins = BuiltinLanguages::load().unwrap();
        let json = builtins.get("json").unwrap();
        let jsonc = builtins.get("jsonc").unwrap();
        assert!(Arc::ptr_eq(&json, &jsonc));
        assert_eq!(json.line_comment.as_deref(), Some("//"));
        assert!(builtins.get("lua").is_none());
        assert_eq!(builtins.languages().count(), 6);
    }

    #[test]
    fn error_message_names_configuration_and_field() {
        let error = LanguageConfigError::new(
            "jsonc",
            "extensions/json/language-configuration.json",
            "wordPattern.pattern",
            "boom",
        );
        let text = error.to_string();
        assert!(text.contains("extensions/json/language-configuration.json"));
        assert!(text.contains("jsonc"));
        assert!(text.contains("wordPattern.pattern"));
    }

    #[test]
    fn auto_close_before_falls_back_to_vscode_default() {
        let builtins = BuiltinLanguages::load().unwrap();
        let csharp = builtins.get("csharp").unwrap();
        assert!(csharp.auto_close_before.is_none());
        assert_eq!(auto_close_before(&csharp), DEFAULT_AUTO_CLOSE_BEFORE);
    }

    #[test]
    fn colorized_brackets_fall_back_to_brackets_only_when_absent() {
        let builtins = BuiltinLanguages::load().unwrap();
        let markdown = builtins.get("markdown").unwrap();
        assert!(markdown.colorized_brackets.is_some());
        assert!(colorized_brackets(&markdown).is_empty());
        let yaml = builtins.get("yaml").unwrap();
        assert!(yaml.colorized_brackets.is_none());
        assert_eq!(colorized_brackets(&yaml).len(), 3);
    }

    #[test]
    fn parsed_data_is_lossless_for_auto_closing() {
        let builtins = BuiltinLanguages::load().unwrap();
        let markdown = builtins.get("markdown").unwrap();
        let angles = markdown
            .auto_closing
            .iter()
            .find(|pair| pair.pair.open.as_ref() == "<")
            .expect("< pair");
        assert_eq!(angles.not_in.len(), 1);
        assert_eq!(angles.not_in[0].as_ref(), "string");
    }

    #[test]
    fn compile_fills_defaults_without_mutating_data() {
        let builtins = BuiltinLanguages::load().unwrap();
        let csharp = builtins.compiled("csharp").unwrap();
        assert!(csharp.data().word_pattern.is_none());
        assert_eq!(
            csharp.word_pattern().javascript().source.as_ref(),
            DEFAULT_WORD_PATTERN
        );
        assert!(csharp.fold_ranges("", 4).unwrap().is_empty());
    }

    #[test]
    fn pair_types_are_exported_for_downstream_use() {
        let pair = Pair {
            open: "(".into(),
            close: ")".into(),
        };
        let auto = crate::AutoClosingPair {
            pair: pair.clone(),
            not_in: Arc::from([Arc::from("comment")]),
        };
        assert_eq!(auto.pair.open.as_ref(), "(");
        assert_eq!(
            colorized_brackets(&LanguageConfiguration::default()).len(),
            0
        );
    }

    #[test]
    fn auto_close_before_exports_the_quote_default() {
        let configuration = LanguageConfiguration::default();
        assert_eq!(auto_close_before(&configuration), DEFAULT_AUTO_CLOSE_BEFORE);
        assert_eq!(
            auto_close_before_for(&configuration, true),
            DEFAULT_AUTO_CLOSE_BEFORE_QUOTES
        );
        assert_ne!(
            DEFAULT_AUTO_CLOSE_BEFORE_QUOTES, DEFAULT_AUTO_CLOSE_BEFORE,
            "quotes and brackets use different defaults"
        );
        let configured = LanguageConfiguration {
            auto_close_before: Some("xyz".into()),
            ..LanguageConfiguration::default()
        };
        assert_eq!(auto_close_before_for(&configured, true), "xyz");
        assert_eq!(auto_close_before_for(&configured, false), "xyz");
    }

    #[test]
    fn partial_load_reports_errors_without_hiding_good_languages() {
        let (languages, errors) = BuiltinLanguages::load_partial();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(languages.languages().count(), 6);
        assert!(languages.compiled("jsonc").is_some());
    }

    #[test]
    fn shared_instance_is_compiled_once() {
        let first = BuiltinLanguages::shared().expect("all shipped configurations load");
        let second = BuiltinLanguages::shared().expect("shared");
        assert!(std::ptr::eq(first, second));
        assert_eq!(first.languages().count(), 6);
    }
}
