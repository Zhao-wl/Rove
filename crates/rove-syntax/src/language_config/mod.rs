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
//!   #26 outline for folding markers).
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
use std::{collections::BTreeMap, fmt, sync::Arc};

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
/// omits the property (none of the shipped files set it). VS Code keeps a second,
/// narrower set for quotes (`';:.,=}])> \n\t'`); the contract exposes one string,
/// so consumers that need the quote variant should use that literal.
pub const DEFAULT_AUTO_CLOSE_BEFORE: &str = "'\"`;:.,=}])> \n\t";

/// `autoCloseBefore` for a language, falling back to VS Code's default.
pub fn auto_close_before(configuration: &LanguageConfiguration) -> &str {
    configuration
        .auto_close_before
        .as_deref()
        .unwrap_or(DEFAULT_AUTO_CLOSE_BEFORE)
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
    folding: Option<(CompiledRegex, CompiledRegex)>,
    indentation: Option<CompiledIndentationRules>,
    on_enter: Vec<CompiledEnterRule>,
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
        let compile = |field: &str, js: &JsRegex| {
            compile_regex(js, &RegexOrigin::new(language, config, field))
        };
        let word_pattern = match &data.word_pattern {
            Some(pattern) => compile("wordPattern.pattern", pattern)?,
            None => compile(
                "wordPattern.pattern",
                &JsRegex {
                    source: DEFAULT_WORD_PATTERN.into(),
                    flags: DEFAULT_WORD_PATTERN_FLAGS.into(),
                },
            )?,
        };
        let folding = match &data.folding_markers {
            Some((start, end)) => Some((
                compile("folding.markers.start", start)?,
                compile("folding.markers.end", end)?,
            )),
            None => None,
        };
        let indentation = match &data.indentation {
            Some(rules) => Some(CompiledIndentationRules {
                increase: compile("indentationRules.increaseIndentPattern", &rules.increase)?,
                decrease: compile("indentationRules.decreaseIndentPattern", &rules.decrease)?,
                indent_next: rules
                    .indent_next
                    .as_ref()
                    .map(|js| compile("indentationRules.indentNextLinePattern", js))
                    .transpose()?,
                unindented: rules
                    .unindented
                    .as_ref()
                    .map(|js| compile("indentationRules.unIndentedLinePattern", js))
                    .transpose()?,
            }),
            None => None,
        };
        let mut on_enter = Vec::with_capacity(data.on_enter.len());
        for (index, rule) in data.on_enter.iter().enumerate() {
            on_enter.push(CompiledEnterRule {
                before: compile(&format!("onEnterRules[{index}].beforeText"), &rule.before)?,
                after: rule
                    .after
                    .as_ref()
                    .map(|js| compile(&format!("onEnterRules[{index}].afterText"), js))
                    .transpose()?,
                previous_line: rule
                    .previous_line
                    .as_ref()
                    .map(|js| compile(&format!("onEnterRules[{index}].previousLineText"), js))
                    .transpose()?,
                indent: rule.indent.clone(),
                append_text: rule.append_text.clone(),
                remove_text: rule.remove_text,
            });
        }
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
    pub fn folding_markers(&self) -> Option<(&CompiledRegex, &CompiledRegex)> {
        self.folding.as_ref().map(|(start, end)| (start, end))
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
            self.folding.as_ref().map(|(start, end)| (start, end)),
            self.data.folding_off_side,
            text,
            tab_size,
        )
    }
}

/// Built-in language configurations for the M1 languages; built once and shared.
pub struct BuiltinLanguages {
    languages: BTreeMap<Arc<str>, CompiledLanguageConfiguration>,
}
impl BuiltinLanguages {
    /// Load and compile every shipped configuration. Fails on the first
    /// configuration whose JSONC or regular expressions are invalid, naming the
    /// file and field.
    pub fn load() -> ConfigResult<Self> {
        let mut languages = BTreeMap::new();
        let mut loaded: BTreeMap<usize, Arc<LanguageConfiguration>> = BTreeMap::new();
        for (language, index) in LANGUAGE_IDS {
            let source = SOURCE_FILES[index];
            let data = match loaded.get(&index) {
                Some(data) => data.clone(),
                None => {
                    let data = Arc::new(parse_language_configuration(
                        language,
                        source.config_path,
                        source.text,
                    )?);
                    loaded.insert(index, data.clone());
                    data
                }
            };
            let compiled =
                CompiledLanguageConfiguration::compile(language, source.config_path, data)?;
            languages.insert(Arc::from(language), compiled);
        }
        Ok(Self { languages })
    }
    /// Served language IDs, sorted.
    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.languages.keys().map(|id| id.as_ref())
    }
    /// Configuration plus compiled regular expressions.
    pub fn compiled(&self, language: &str) -> Option<&CompiledLanguageConfiguration> {
        self.languages.get(language)
    }
}
impl LanguageConfigurations for BuiltinLanguages {
    fn get(&self, language: &str) -> Option<Arc<LanguageConfiguration>> {
        self.languages.get(language).map(|c| c.data.clone())
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
}
