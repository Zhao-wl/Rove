//! Syntax snapshots, language data and shared symbol models (#9/#12/#20/#26).
use rove_base::*;
use rove_buffer::TextSource;
use std::sync::Arc;
/// Absolute decoded UTF-8 byte range and ordered outer-to-inner TextMate scope stack.
#[derive(Clone, Debug)]
pub struct Token {
    pub range: ByteRange,
    pub scopes: Arc<[Arc<str>]>,
}
#[derive(Clone, Debug)]
pub enum FoldKind {
    Syntax,
    Region,
    Indentation,
}
/// Half-open logical line range; first line remains visible when folded.
#[derive(Clone, Debug)]
pub struct Fold {
    pub lines: LineRange,
    pub kind: FoldKind,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Namespace,
    Class,
    Struct,
    Interface,
    Enum,
    Method,
    Function,
    Property,
    Field,
    Variable,
    Parameter,
    TypeParameter,
    Event,
    Constructor,
    Constant,
    Module,
    Object,
    Array,
    Key,
    Other(Arc<str>),
}
/// Shared by outline, breadcrumbs, sticky scroll, navigation and context basket.
/// selection lies within range; all positions belong to the snapshot's version.
#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: Arc<str>,
    pub kind: SymbolKind,
    pub range: TextRange,
    pub selection: TextRange,
    pub children: Arc<[Symbol]>,
}
/// Immutable Send + Sync result. O(log lines + returned tokens) line lookup, no parsing on read.
/// An old version can still be painted; consumers must not interpret its ranges as current edits.
/// OutOfBounds is distinct from a valid line with no tokens. Implementations own source snapshots.
pub trait SyntaxSnapshot: Send + Sync {
    fn version(&self) -> Version;
    fn tokens(&self, line: Line) -> Result<Arc<[Token]>>;
    fn folds(&self) -> Arc<[Fold]>;
    fn symbols(&self) -> Arc<[Symbol]>;
}
/// Incremental invalidation in NEW snapshot coordinates; deletion at EOF invalidates final line.
/// None means full invalidation (initial parse or lost baseline), Some(empty) means unchanged.
pub struct SyntaxUpdate {
    pub snapshot: Arc<dyn SyntaxSnapshot>,
    pub previous: Option<Version>,
    pub changed_lines: Option<Arc<[LineRange]>>,
}
/// Background-only parser; Send + Sync, accepts immutable text and cooperative cancellation.
/// Target incremental update <2ms on ordinary edits; never scan on UI thread. Error leaves old
/// snapshot usable. Unsupported languages return Unsupported rather than fabricated tokens.
pub trait SyntaxService: Send + Sync {
    fn analyze(
        &self,
        text: Arc<dyn TextSource>,
        language: Arc<str>,
        cancel: Cancellation,
    ) -> Task<SyntaxUpdate>;
}
/// Fixed one-line empty result; caller can choose revision explicitly for composition tests.
pub struct EmptySyntax(pub Version);
impl SyntaxSnapshot for EmptySyntax {
    fn version(&self) -> Version {
        self.0
    }
    fn tokens(&self, l: Line) -> Result<Arc<[Token]>> {
        if l.0 == 0 {
            Ok(Arc::from([]))
        } else {
            Err(Error::OutOfBounds)
        }
    }
    fn folds(&self) -> Arc<[Fold]> {
        Arc::from([])
    }
    fn symbols(&self) -> Arc<[Symbol]> {
        Arc::from([])
    }
}
pub struct EmptySyntaxService;
impl SyntaxService for EmptySyntaxService {
    fn analyze(&self, _: Arc<dyn TextSource>, _: Arc<str>, c: Cancellation) -> Task<SyntaxUpdate> {
        Box::pin(async move {
            c.check()?;
            Err(Error::Unsupported)
        })
    }
}
/// JavaScript regex source + flags preserved verbatim; #12 validates/compiles off the UI thread.
#[derive(Clone, Debug)]
pub struct JsRegex {
    pub source: Arc<str>,
    pub flags: Arc<str>,
}
#[derive(Clone, Debug)]
pub struct Pair {
    pub open: Arc<str>,
    pub close: Arc<str>,
}
#[derive(Clone, Debug)]
pub struct AutoClosingPair {
    pub pair: Pair,
    pub not_in: Arc<[Arc<str>]>,
}
#[derive(Clone, Debug)]
pub struct IndentationRules {
    pub increase: JsRegex,
    pub decrease: JsRegex,
    pub indent_next: Option<JsRegex>,
    pub unindented: Option<JsRegex>,
}
#[derive(Clone, Debug)]
pub enum IndentAction {
    None,
    Indent,
    Outdent,
    IndentOutdent,
}
#[derive(Clone, Debug)]
pub struct EnterRule {
    pub before: JsRegex,
    pub after: Option<JsRegex>,
    pub previous_line: Option<JsRegex>,
    pub indent: IndentAction,
    pub append_text: Option<Arc<str>>,
    pub remove_text: Option<u32>,
}
/// Lossless language-configuration data, not regex engine handles. Immutable, Send + Sync.
#[derive(Clone, Debug, Default)]
pub struct LanguageConfiguration {
    pub line_comment: Option<Arc<str>>,
    pub block_comment: Option<Pair>,
    pub brackets: Arc<[Pair]>,
    pub colorized_brackets: Option<Arc<[Pair]>>,
    pub auto_closing: Arc<[AutoClosingPair]>,
    pub auto_close_before: Option<Arc<str>>,
    pub surrounding: Arc<[Pair]>,
    pub folding_markers: Option<(JsRegex, JsRegex)>,
    pub folding_off_side: bool,
    pub word_pattern: Option<JsRegex>,
    pub indentation: Option<IndentationRules>,
    pub on_enter: Arc<[EnterRule]>,
}
/// Cached O(log languages) lookup, UI-safe/no I/O; None means unrecognized language.
pub trait LanguageConfigurations: Send + Sync {
    fn get(&self, language: &str) -> Option<Arc<LanguageConfiguration>>;
}
pub struct EmptyLanguages;
impl LanguageConfigurations for EmptyLanguages {
    fn get(&self, _: &str) -> Option<Arc<LanguageConfiguration>> {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn highlighter_keeps_old_snapshot() {
        let old: Arc<dyn SyntaxSnapshot> = Arc::new(EmptySyntax(Version(1)));
        assert!(old.tokens(Line(0)).unwrap().is_empty());
        assert_ne!(old.version(), Version(2));
    }
    #[test]
    fn outline_and_folding_share_snapshot() {
        let s = EmptySyntax(Version(0));
        assert!(s.symbols().is_empty());
        assert!(s.folds().is_empty());
    }
    #[test]
    fn editor_requests_language_configuration() {
        let p: &dyn LanguageConfigurations = &EmptyLanguages;
        assert!(p.get("csharp").is_none());
        assert!(LanguageConfiguration::default().on_enter.is_empty());
    }
}
