//! 自动闭合、越过（overtype）、成对删除与自动包围。
//!
//! 逐段对应 `cursorTypeOperations.ts` 的 `_findAutoClosingPairOpen`、`_isAutoClosingOpenCharType`、
//! `_isAutoClosingOvertype`、`_isSurroundSelectionType`，以及 `cursorDeleteOperations.ts` 的
//! `_isAutoClosingPairDelete`。
//!
//! **关键细节**：只有“编辑器自己补上的”那个闭合符才参与 overtype 与成对删除。这一状态由
//! [`AutoClosed`] 记录，光标离开、文本被别的编辑改动后失效（见 [`AutoClosedTracker`]）。

use super::{is_quote, Change};
use rove_syntax::{AutoClosingPair, LanguageConfiguration, Pair};

/// `editor.autoClosingBrackets` / `autoClosingQuotes` / `autoClosingComments`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoClosing {
    Always,
    LanguageDefined,
    BeforeWhitespace,
    Never,
}

impl AutoClosing {
    pub fn parse(value: &str) -> Self {
        match value {
            "always" => AutoClosing::Always,
            "beforeWhitespace" => AutoClosing::BeforeWhitespace,
            "never" => AutoClosing::Never,
            _ => AutoClosing::LanguageDefined,
        }
    }
}

/// `editor.autoClosingOvertype` / `autoClosingDelete`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoClosingEdit {
    Always,
    Auto,
    Never,
}

impl AutoClosingEdit {
    pub fn parse(value: &str) -> Self {
        match value {
            "always" => AutoClosingEdit::Always,
            "never" => AutoClosingEdit::Never,
            _ => AutoClosingEdit::Auto,
        }
    }
}

/// `editor.autoSurround`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSurround {
    LanguageDefined,
    Quotes,
    Brackets,
    Never,
}

impl AutoSurround {
    pub fn parse(value: &str) -> Self {
        match value {
            "quotes" => AutoSurround::Quotes,
            "brackets" => AutoSurround::Brackets,
            "never" => AutoSurround::Never,
            _ => AutoSurround::LanguageDefined,
        }
    }
}

/// 自动闭合相关的设置快照。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoCloseOptions {
    pub brackets: AutoClosing,
    pub quotes: AutoClosing,
    pub comments: AutoClosing,
    pub overtype: AutoClosingEdit,
    pub delete: AutoClosingEdit,
    pub surround: AutoSurround,
}

impl Default for AutoCloseOptions {
    fn default() -> Self {
        Self {
            brackets: AutoClosing::LanguageDefined,
            quotes: AutoClosing::LanguageDefined,
            comments: AutoClosing::LanguageDefined,
            overtype: AutoClosingEdit::Auto,
            delete: AutoClosingEdit::Auto,
            surround: AutoSurround::LanguageDefined,
        }
    }
}

/// 一对编辑器自己补上的闭合符：闭合符所在的位置（行 + 字节列）与它本身。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoClosed {
    pub line: u64,
    pub column: usize,
    pub close: String,
}

/// “这对括号是编辑器补的”状态。任何不是紧接着这次自动闭合的编辑、或光标离开，都让它失效。
#[derive(Clone, Debug, Default)]
pub struct AutoClosedTracker(Vec<AutoClosed>);

impl AutoClosedTracker {
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn record(&mut self, closed: AutoClosed) {
        self.0.push(closed);
    }
    /// 光标（或任意编辑）到了别处：只保留仍然紧挨着插入点的那些。
    pub fn retain_at(&mut self, line: u64, column: usize) {
        self.0.retain(|c| c.line == line && c.column == column);
    }
    pub fn contains(&self, line: u64, column: usize) -> bool {
        self.0.iter().any(|c| c.line == line && c.column == column)
    }
    /// 自动闭合符所在位置随同一行上的插入 / 删除平移。
    pub fn shift(&mut self, line: u64, at: usize, delta: isize) {
        for c in &mut self.0 {
            if c.line == line && c.column >= at {
                c.column = (c.column as isize + delta).max(0) as usize;
            }
        }
    }
}

/// 按输入的字符判定该用哪种 `autoClosing*` 设置：注释对看的是 `autoClosingComments`。
fn setting_for(
    options: &AutoCloseOptions,
    config: &LanguageConfiguration,
    pair: &Pair,
) -> AutoClosing {
    if is_quote(&pair.open) {
        return options.quotes;
    }
    let is_comment = config
        .block_comment
        .as_ref()
        .is_some_and(|c| *c.open == *pair.open)
        || pair.open.starts_with("/*")
        || pair.open.starts_with("<!--");
    if is_comment {
        options.comments
    } else {
        options.brackets
    }
}

/// `autoCloseBefore`：闭合符只在这些字符之前自动插入（缺省值来自 #12）。
fn auto_close_before(config: &LanguageConfiguration, quotes: bool) -> String {
    rove_syntax::language_config::auto_close_before_for(config, quotes).to_string()
}

/// `_findAutoClosingPairOpen`：输入 `ch` 后，行首到光标的文本以哪个 `open` 结尾（取最长的）。
pub fn find_open_pair<'a>(
    config: &'a LanguageConfiguration,
    before: &str,
    ch: &str,
) -> Option<&'a AutoClosingPair> {
    let typed = format!("{before}{ch}");
    let mut best: Option<&AutoClosingPair> = None;
    for candidate in config.auto_closing.iter() {
        if !typed.ends_with(&*candidate.pair.open) {
            continue;
        }
        // 输入的这个字符必须是 `open` 的最后一段，否则只是碰巧的后缀。
        if !candidate.pair.open.ends_with(ch) {
            continue;
        }
        if best.is_none_or(|b| candidate.pair.open.len() > b.pair.open.len()) {
            best = Some(candidate);
        }
    }
    best
}

/// `_isBeforeClosingBrace`：光标后紧跟着的是某个 `close`（且不同时是某个 `open`）。
fn is_before_closing_brace(config: &LanguageConfiguration, after: &str) -> bool {
    let starts_open = config
        .auto_closing
        .iter()
        .any(|p| !p.pair.open.is_empty() && after.starts_with(&*p.pair.open));
    let starts_close = config
        .auto_closing
        .iter()
        .any(|p| !p.pair.close.is_empty() && after.starts_with(&*p.pair.close));
    !starts_open && starts_close
}

/// `notIn` 的判定输入：光标处是否在字符串 / 注释里（由语法着色给出）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenContext {
    pub in_string: bool,
    pub in_comment: bool,
    /// 正则字面量（本产品的语言里没有，保留以对齐 VS Code 的 `notIn` 取值）。
    pub in_regex: bool,
}

impl TokenContext {
    fn blocked_by(&self, not_in: &[std::sync::Arc<str>]) -> bool {
        not_in.iter().any(|n| match &**n {
            "string" => self.in_string,
            "comment" => self.in_comment,
            "regex" => self.in_regex,
            _ => false,
        })
    }
}

/// 输入 `ch` 时该不该自动补上闭合符；返回要补的那一对。
///
/// `before` / `after` 是光标所在行在光标前 / 后的文本（都**不含**刚输入的 `ch`）。
pub fn should_auto_close<'a>(
    options: &AutoCloseOptions,
    config: &'a LanguageConfiguration,
    before: &str,
    after: &str,
    ch: &str,
    tokens: TokenContext,
    word_separators: &str,
) -> Option<&'a Pair> {
    let pair = find_open_pair(config, before, ch)?;
    let setting = setting_for(options, config, &pair.pair);
    if setting == AutoClosing::Never {
        return None;
    }
    // 光标后的字符要合适：`autoCloseBefore`，或者紧跟着一个右括号。
    if !after.is_empty() {
        let next = after.chars().next()?;
        let before_close = is_before_closing_brace(config, after);
        let allowed = match setting {
            AutoClosing::Always => true,
            AutoClosing::BeforeWhitespace => next == ' ' || next == '\t',
            _ => auto_close_before(config, is_quote(&pair.pair.open)).contains(next),
        };
        if !before_close && !allowed {
            return None;
        }
    }
    // 单字符引号不在单词字符后面自动闭合（`always` 例外）。
    if pair.pair.open.chars().count() == 1
        && is_quote(&pair.pair.open)
        && setting != AutoClosing::Always
    {
        if let Some(prev) = before.chars().next_back() {
            let separator = prev.is_whitespace() || word_separators.contains(prev);
            if !separator {
                return None;
            }
        }
    }
    if tokens.blocked_by(&pair.not_in) {
        return None;
    }
    Some(&pair.pair)
}

/// `_isAutoClosingOvertype`：输入的右括号正好越过**自己补的**那一个。
#[allow(clippy::too_many_arguments)]
pub fn should_overtype(
    options: &AutoCloseOptions,
    config: &LanguageConfiguration,
    tracker: &AutoClosedTracker,
    line: u64,
    column: usize,
    before: &str,
    after: &str,
    ch: &str,
) -> bool {
    if options.overtype == AutoClosingEdit::Never {
        return false;
    }
    // 只有单字符的闭合符参与。
    if !config
        .auto_closing
        .iter()
        .any(|p| &*p.pair.close == ch && p.pair.close.chars().count() == 1)
    {
        return false;
    }
    if !after.starts_with(ch) {
        return false;
    }
    // 反斜杠后面的引号不越过。
    if is_quote(ch) && before.ends_with('\\') {
        return false;
    }
    if options.overtype == AutoClosingEdit::Auto && !tracker.contains(line, column) {
        return false;
    }
    true
}

/// `_isAutoClosingPairDelete`：退格时左右正好是一对**自动补上的**括号 / 引号。
#[allow(clippy::too_many_arguments)]
pub fn should_delete_pair(
    options: &AutoCloseOptions,
    config: &LanguageConfiguration,
    tracker: &AutoClosedTracker,
    line: u64,
    column: usize,
    before: &str,
    after: &str,
) -> bool {
    if options.delete == AutoClosingEdit::Never {
        return false;
    }
    if options.brackets == AutoClosing::Never && options.quotes == AutoClosing::Never {
        return false;
    }
    let (Some(open), Some(close)) = (before.chars().next_back(), after.chars().next()) else {
        return false;
    };
    let (open, close) = (open.to_string(), close.to_string());
    let found = config
        .auto_closing
        .iter()
        .any(|p| *p.pair.open == *open && *p.pair.close == *close);
    if !found {
        return false;
    }
    if is_quote(&open) {
        if options.quotes == AutoClosing::Never {
            return false;
        }
    } else if options.brackets == AutoClosing::Never {
        return false;
    }
    if options.delete == AutoClosingEdit::Auto && !tracker.contains(line, column) {
        return false;
    }
    true
}

/// `_shouldSurroundChar` + `_isSurroundSelectionType`：选区该不该被 `ch` 包围；返回包围用的那一对。
pub fn should_surround<'a>(
    options: &AutoCloseOptions,
    config: &'a LanguageConfiguration,
    ch: &str,
    selected: &str,
) -> Option<&'a Pair> {
    if options.surround == AutoSurround::Never {
        return None;
    }
    let quote = is_quote(ch);
    let allowed = match options.surround {
        AutoSurround::LanguageDefined => true,
        AutoSurround::Quotes => quote,
        AutoSurround::Brackets => !quote,
        AutoSurround::Never => false,
    };
    if !allowed {
        return None;
    }
    // 只有空白的选区不包围（VS Code：那更像是要替换掉它）。
    if selected.is_empty()
        || selected
            .chars()
            .all(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r')
    {
        return None;
    }
    config.surrounding.iter().find(|p| &*p.open == ch)
}

/// 自动包围产生的两处插入（选区本身不动，仍然被选中）。
pub fn surround_changes(pair: &Pair, start: (u64, usize), end: (u64, usize)) -> Vec<Change> {
    vec![
        Change::insert(start, pair.open.to_string()),
        Change::insert(end, pair.close.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_syntax::language_config::BuiltinLanguages;
    use rove_syntax::LanguageConfigurations;
    use std::sync::Arc;

    const SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

    fn data(language: &str) -> Arc<LanguageConfiguration> {
        BuiltinLanguages::shared().0.get(language).unwrap()
    }

    fn opts() -> AutoCloseOptions {
        AutoCloseOptions::default()
    }

    #[test]
    fn open_bracket_auto_closes_at_end_of_line() {
        let c = data("csharp");
        let p = should_auto_close(
            &opts(),
            &c,
            "if (x",
            "",
            "(",
            TokenContext::default(),
            SEPARATORS,
        );
        assert_eq!(p.map(|p| p.close.to_string()), Some(")".to_string()));
    }

    #[test]
    fn not_in_string_or_comment_blocks_auto_close() {
        let c = data("csharp");
        let in_string = TokenContext {
            in_string: true,
            ..TokenContext::default()
        };
        // C# 的 `"` 对带 notIn: ["string"]。
        assert!(should_auto_close(&opts(), &c, "x = ", "", "\"", in_string, SEPARATORS).is_none());
        assert!(should_auto_close(
            &opts(),
            &c,
            "x = ",
            "",
            "\"",
            TokenContext::default(),
            SEPARATORS
        )
        .is_some());
    }

    #[test]
    fn auto_close_before_limits_the_following_character() {
        let c = data("csharp");
        // 默认的 autoCloseBefore 里有 `)` 与空白，没有字母。
        assert!(should_auto_close(
            &opts(),
            &c,
            "f(",
            "abc",
            "(",
            TokenContext::default(),
            SEPARATORS
        )
        .is_none());
        assert!(should_auto_close(
            &opts(),
            &c,
            "f(",
            " abc",
            "(",
            TokenContext::default(),
            SEPARATORS
        )
        .is_some());
    }

    #[test]
    fn always_ignores_auto_close_before() {
        let c = data("csharp");
        let o = AutoCloseOptions {
            brackets: AutoClosing::Always,
            ..opts()
        };
        assert!(should_auto_close(
            &o,
            &c,
            "f(",
            "abc",
            "(",
            TokenContext::default(),
            SEPARATORS
        )
        .is_some());
    }

    #[test]
    fn never_disables_auto_close() {
        let c = data("csharp");
        let o = AutoCloseOptions {
            brackets: AutoClosing::Never,
            ..opts()
        };
        assert!(should_auto_close(
            &o,
            &c,
            "if (x",
            "",
            "(",
            TokenContext::default(),
            SEPARATORS
        )
        .is_none());
    }

    #[test]
    fn quotes_do_not_close_after_a_word_character() {
        let c = data("csharp");
        assert!(should_auto_close(
            &opts(),
            &c,
            "don",
            "",
            "'",
            TokenContext::default(),
            SEPARATORS
        )
        .is_none());
        assert!(should_auto_close(
            &opts(),
            &c,
            "x = ",
            "",
            "'",
            TokenContext::default(),
            SEPARATORS
        )
        .is_some());
    }

    #[test]
    fn only_the_editors_own_close_is_overtyped() {
        let c = data("csharp");
        let mut tracker = AutoClosedTracker::default();
        // 手工输入的 `)`：auto 下不越过。
        assert!(!should_overtype(
            &opts(),
            &c,
            &tracker,
            0,
            5,
            "f(x",
            ")",
            ")"
        ));
        tracker.record(AutoClosed {
            line: 0,
            column: 5,
            close: ")".into(),
        });
        assert!(should_overtype(
            &opts(),
            &c,
            &tracker,
            0,
            5,
            "f(x",
            ")",
            ")"
        ));
        // `always` 下手工输入的也越过。
        let o = AutoCloseOptions {
            overtype: AutoClosingEdit::Always,
            ..opts()
        };
        assert!(should_overtype(
            &o,
            &c,
            &AutoClosedTracker::default(),
            0,
            5,
            "f(x",
            ")",
            ")"
        ));
    }

    #[test]
    fn overtype_never_keeps_typing() {
        let c = data("csharp");
        let mut tracker = AutoClosedTracker::default();
        tracker.record(AutoClosed {
            line: 0,
            column: 5,
            close: ")".into(),
        });
        let o = AutoCloseOptions {
            overtype: AutoClosingEdit::Never,
            ..opts()
        };
        assert!(!should_overtype(&o, &c, &tracker, 0, 5, "f(x", ")", ")"));
    }

    #[test]
    fn quote_after_backslash_is_not_overtyped() {
        let c = data("csharp");
        let mut tracker = AutoClosedTracker::default();
        tracker.record(AutoClosed {
            line: 0,
            column: 3,
            close: "\"".into(),
        });
        assert!(!should_overtype(
            &opts(),
            &c,
            &tracker,
            0,
            3,
            "a\\",
            "\"",
            "\""
        ));
    }

    #[test]
    fn delete_removes_the_matching_auto_inserted_close() {
        let c = data("csharp");
        let mut tracker = AutoClosedTracker::default();
        assert!(!should_delete_pair(&opts(), &c, &tracker, 0, 1, "(", ")"));
        tracker.record(AutoClosed {
            line: 0,
            column: 1,
            close: ")".into(),
        });
        assert!(should_delete_pair(&opts(), &c, &tracker, 0, 1, "(", ")"));
        let o = AutoCloseOptions {
            delete: AutoClosingEdit::Never,
            ..opts()
        };
        assert!(!should_delete_pair(&o, &c, &tracker, 0, 1, "(", ")"));
    }

    #[test]
    fn surround_respects_the_three_settings() {
        let c = data("csharp");
        let quotes = AutoCloseOptions {
            surround: AutoSurround::Quotes,
            ..opts()
        };
        let brackets = AutoCloseOptions {
            surround: AutoSurround::Brackets,
            ..opts()
        };
        let never = AutoCloseOptions {
            surround: AutoSurround::Never,
            ..opts()
        };
        assert!(should_surround(&opts(), &c, "(", "abc").is_some());
        assert!(should_surround(&opts(), &c, "\"", "abc").is_some());
        assert!(should_surround(&quotes, &c, "(", "abc").is_none());
        assert!(should_surround(&quotes, &c, "\"", "abc").is_some());
        assert!(should_surround(&brackets, &c, "(", "abc").is_some());
        assert!(should_surround(&brackets, &c, "\"", "abc").is_none());
        assert!(should_surround(&never, &c, "(", "abc").is_none());
    }

    #[test]
    fn whitespace_only_selection_is_not_surrounded() {
        let c = data("csharp");
        assert!(should_surround(&opts(), &c, "(", "   ").is_none());
    }

    #[test]
    fn tracker_forgets_when_the_caret_moves() {
        let mut tracker = AutoClosedTracker::default();
        tracker.record(AutoClosed {
            line: 0,
            column: 5,
            close: ")".into(),
        });
        tracker.retain_at(0, 6);
        assert!(tracker.is_empty());
    }

    #[test]
    fn multi_character_open_is_recognized() {
        // XML 的 `<!--` → `-->` 是多字符对：只有输入最后一个 `-` 时才算命中。
        let x = data("xml");
        let p = find_open_pair(&x, "<!-", "-").map(|p| p.pair.close.to_string());
        assert_eq!(p.as_deref(), Some("-->"));
        assert!(find_open_pair(&x, "<!", "-").is_none());
    }

    #[test]
    fn multi_character_comment_pair_follows_auto_closing_comments() {
        let x = data("xml");
        let never = AutoCloseOptions {
            comments: AutoClosing::Never,
            ..opts()
        };
        assert!(should_auto_close(
            &opts(),
            &x,
            "<!-",
            "",
            "-",
            TokenContext::default(),
            SEPARATORS
        )
        .is_some());
        assert!(should_auto_close(
            &never,
            &x,
            "<!-",
            "",
            "-",
            TokenContext::default(),
            SEPARATORS
        )
        .is_none());
    }
}
