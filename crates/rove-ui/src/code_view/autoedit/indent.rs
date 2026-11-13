//! VS Code 缩进规则引擎的移植。
//!
//! 逐段对应 `src/vs/editor/common/languages/autoIndent.ts`：[`IndentRules`] 是
//! `IndentRulesSupport`，[`inherit_indent_for_line`] 是 `getInheritIndentForLine`，
//! [`good_indent_for_line`] 是 `getGoodIndentForLine`，[`enter_indent`] 是 `getIndentForEnter`，
//! [`indent_action_for_type`] 是 `getIndentActionForType`；缩进单位换算（[`IndentConverter`]）
//! 来自 `commands/shiftCommand.ts` 与 `model/textModel.ts` 的 `normalizeIndentation`。
//!
//! 与 VS Code 的差异只有两处，都是坐标：行号从 0 开始，列是 UTF-8 字节偏移。
//! 正则求值出错（`fancy-regex` 的回溯上限等）一律当作“不匹配”，不让一次按键失败。

use super::super::columns::LineSource;
use super::{first_non_whitespace, leading_whitespace};
use rove_syntax::language_config::{CompiledIndentationRules, CompiledLanguageConfiguration};

/// `editor.autoIndent`。顺序即 VS Code 的 `EditorAutoIndentStrategy`，比较用 `>=`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AutoIndent {
    None,
    Keep,
    Brackets,
    Advanced,
    Full,
}

impl AutoIndent {
    pub fn parse(value: &str) -> Self {
        match value {
            "none" => AutoIndent::None,
            "keep" => AutoIndent::Keep,
            "brackets" => AutoIndent::Brackets,
            "advanced" => AutoIndent::Advanced,
            _ => AutoIndent::Full,
        }
    }
}

/// `onEnterRules` / `indentationRules` 推导出的缩进动作（本地副本：契约里的
/// [`rove_syntax::IndentAction`] 没有 `Copy` / `PartialEq`，不能在本 PR 里改）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Indent,
    Outdent,
    IndentOutdent,
}

impl Action {
    pub fn of(action: &rove_syntax::IndentAction) -> Self {
        match action {
            rove_syntax::IndentAction::None => Action::None,
            rove_syntax::IndentAction::Indent => Action::Indent,
            rove_syntax::IndentAction::Outdent => Action::Outdent,
            rove_syntax::IndentAction::IndentOutdent => Action::IndentOutdent,
        }
    }
}

// ------------------------------------------------------------------ 缩进单位换算

/// `editor.insertSpaces` / `tabSize` / `indentSize` 的换算（`ShiftCommand` 与 `normalizeIndentation`）。
/// 取值以**文档当前的检测结果**为准（检测属于 #27，本模块只消费）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndentConverter {
    pub insert_spaces: bool,
    pub tab_size: usize,
    pub indent_size: usize,
}

impl Default for IndentConverter {
    fn default() -> Self {
        Self {
            insert_spaces: true,
            tab_size: 4,
            indent_size: 4,
        }
    }
}

/// VS Code `CursorColumns.nextIndentTabStop`。
fn next_indent_tab_stop(visible: usize, indent_size: usize) -> usize {
    visible + indent_size - visible % indent_size
}

/// VS Code `CursorColumns.prevIndentTabStop`。
fn prev_indent_tab_stop(visible: usize, indent_size: usize) -> usize {
    let back = match visible % indent_size {
        0 => indent_size,
        r => r,
    };
    visible.saturating_sub(back)
}

/// 空白串的可见宽度（制表符跳到下一个制表位）。
pub fn visible_width(text: &str, tab_size: usize) -> usize {
    let mut visible = 0;
    for ch in text.chars() {
        if ch == '\t' {
            visible = next_indent_tab_stop(visible, tab_size.max(1));
        } else {
            visible += 1;
        }
    }
    visible
}

impl IndentConverter {
    fn render(&self, visible: usize) -> String {
        if self.insert_spaces {
            " ".repeat(visible)
        } else {
            "\t".repeat(visible.div_ceil(self.tab_size.max(1)))
        }
    }

    /// `ShiftCommand.shiftIndent`：整段缩进加一级。
    pub fn shift_indent(&self, indentation: &str) -> String {
        let visible = visible_width(indentation, self.tab_size.max(1));
        self.render(next_indent_tab_stop(visible, self.indent_size.max(1)))
    }

    /// `ShiftCommand.unshiftIndent`：整段缩进减一级。
    pub fn unshift_indent(&self, indentation: &str) -> String {
        let visible = visible_width(indentation, self.tab_size.max(1));
        self.render(prev_indent_tab_stop(visible, self.indent_size.max(1)))
    }

    /// `normalizeIndentation`：把前导空白按当前缩进单位重写，其余部分原样保留。
    pub fn normalize(&self, text: &str) -> String {
        let split = first_non_whitespace(text).unwrap_or(text.len());
        let (white, rest) = text.split_at(split);
        let mut visible = 0;
        for ch in white.chars() {
            if ch == '\t' {
                visible = next_indent_tab_stop(visible, self.indent_size.max(1));
            } else {
                visible += 1;
            }
        }
        let mut out = String::new();
        if !self.insert_spaces {
            let size = self.indent_size.max(1);
            out.push_str(&"\t".repeat(visible / size));
            visible %= size;
        }
        out.push_str(&" ".repeat(visible));
        out.push_str(rest);
        out
    }

    /// 一级缩进的字面量（`tab` 命令与 `appendText` 的 `\t` 归一化都用它）。
    pub fn one_level(&self) -> String {
        if self.insert_spaces {
            " ".repeat(self.indent_size.max(1))
        } else {
            "\t".to_string()
        }
    }
}

// ------------------------------------------------------------------ indentationRules

/// `IndentRulesSupport`：四条正则的判定与 `getIndentMetadata` 的位掩码。
#[derive(Clone, Copy)]
pub struct IndentRules<'a>(pub &'a CompiledIndentationRules);

pub const INCREASE_MASK: u8 = 0b0001;
pub const DECREASE_MASK: u8 = 0b0010;
pub const INDENT_NEXTLINE_MASK: u8 = 0b0100;
pub const UNINDENT_MASK: u8 = 0b1000;

impl IndentRules<'_> {
    pub fn should_increase(&self, text: &str) -> bool {
        self.0.increase.is_match(text).unwrap_or(false)
    }
    pub fn should_decrease(&self, text: &str) -> bool {
        self.0.decrease.is_match(text).unwrap_or(false)
    }
    pub fn should_indent_next_line(&self, text: &str) -> bool {
        self.0
            .indent_next
            .as_ref()
            .is_some_and(|r| r.is_match(text).unwrap_or(false))
    }
    pub fn should_ignore(&self, text: &str) -> bool {
        self.0
            .unindented
            .as_ref()
            .is_some_and(|r| r.is_match(text).unwrap_or(false))
    }
    /// `getIndentMetadata`：被 `unIndentedLinePattern` 命中的行只返回 [`UNINDENT_MASK`]。
    pub fn indent_metadata(&self, text: &str) -> u8 {
        if self.should_ignore(text) {
            return UNINDENT_MASK;
        }
        let mut mask = 0;
        if self.should_increase(text) {
            mask |= INCREASE_MASK;
        }
        if self.should_decrease(text) {
            mask |= DECREASE_MASK;
        }
        if self.should_indent_next_line(text) {
            mask |= INDENT_NEXTLINE_MASK;
        }
        mask
    }
}

/// 语言的缩进规则句柄（没有 `indentationRules` 的语言为 `None`）。
pub fn rules_of(config: &CompiledLanguageConfiguration) -> Option<IndentRules<'_>> {
    config.indentation().map(IndentRules)
}

// ------------------------------------------------------------------ 虚拟模型

/// 把某一行替换成别的文本来看文档（VS Code `getIndentForEnter` 的 `virtualModel`）。
pub struct OverrideLine<'a> {
    pub inner: &'a dyn LineSource,
    pub line: u64,
    pub text: &'a str,
}

impl LineSource for OverrideLine<'_> {
    fn line_count(&self) -> u64 {
        self.inner.line_count()
    }
    fn line_text(&self, line: u64) -> Option<String> {
        if line == self.line {
            Some(self.text.to_string())
        } else {
            self.inner.line_text(line)
        }
    }
}

fn text_of(lines: &dyn LineSource, line: u64) -> String {
    lines.line_text(line).unwrap_or_default()
}

// ------------------------------------------------------------------ getInheritIndentForLine

/// `getInheritIndentForLine` 的结果。`line` 是缩进的来源行（0 基）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inherited {
    pub indentation: String,
    pub action: Option<Action>,
    pub line: Option<u64>,
}

/// `getPrecedingValidLine`：向上找第一条不该忽略、也不是空白行的行。
fn preceding_valid_line(lines: &dyn LineSource, line: u64, rules: IndentRules<'_>) -> Option<u64> {
    for last in (0..line).rev() {
        let text = text_of(lines, last);
        if rules.should_ignore(&text) || text.trim().is_empty() {
            continue;
        }
        return Some(last);
    }
    None
}

/// `getInheritIndentForLine`：`line` 这一行应当继承的缩进与动作。
pub fn inherit_indent_for_line(
    auto_indent: AutoIndent,
    lines: &dyn LineSource,
    rules: IndentRules<'_>,
    line: u64,
    honor_intentional_indent: bool,
) -> Option<Inherited> {
    if auto_indent < AutoIndent::Full {
        return None;
    }
    let empty = |indentation: String, l: Option<u64>| {
        Some(Inherited {
            indentation,
            action: None,
            line: l,
        })
    };
    if line == 0 {
        return empty(String::new(), None);
    }
    // 之前全是空行：不缩进。
    for prior in (0..line).rev() {
        if !text_of(lines, prior).is_empty() {
            break;
        }
        if prior == 0 {
            return empty(String::new(), None);
        }
    }
    let preceding = preceding_valid_line(lines, line, rules)?;
    let preceding_text = text_of(lines, preceding);
    if rules.should_increase(&preceding_text) || rules.should_indent_next_line(&preceding_text) {
        return Some(Inherited {
            indentation: leading_whitespace(&preceding_text).to_string(),
            action: Some(Action::Indent),
            line: Some(preceding),
        });
    }
    if rules.should_decrease(&preceding_text) {
        return empty(
            leading_whitespace(&preceding_text).to_string(),
            Some(preceding),
        );
    }
    if preceding == 0 {
        return empty(
            leading_whitespace(&preceding_text).to_string(),
            Some(preceding),
        );
    }
    let previous = preceding - 1;
    let metadata = rules.indent_metadata(&text_of(lines, previous));
    if metadata & (INCREASE_MASK | DECREASE_MASK) == 0 && metadata & INDENT_NEXTLINE_MASK != 0 {
        // 上一行是 `indentNextLinePattern` 的延续：回到那一串的第一行。
        let mut stop: i64 = -1;
        for i in (0..previous).rev() {
            if rules.should_indent_next_line(&text_of(lines, i)) {
                continue;
            }
            stop = i as i64;
            break;
        }
        let at = (stop + 1) as u64;
        return empty(
            leading_whitespace(&text_of(lines, at)).to_string(),
            Some(at),
        );
    }
    if honor_intentional_indent {
        return empty(
            leading_whitespace(&preceding_text).to_string(),
            Some(preceding),
        );
    }
    // 往上找一条“不是临时缩进”的行。
    for i in (0..=preceding).rev() {
        let text = text_of(lines, i);
        if rules.should_increase(&text) {
            return Some(Inherited {
                indentation: leading_whitespace(&text).to_string(),
                action: Some(Action::Indent),
                line: Some(i),
            });
        }
        if rules.should_indent_next_line(&text) {
            let mut stop: i64 = -1;
            for j in (0..i).rev() {
                if rules.should_indent_next_line(&text_of(lines, j)) {
                    continue;
                }
                stop = j as i64;
                break;
            }
            let at = (stop + 1) as u64;
            return empty(
                leading_whitespace(&text_of(lines, at)).to_string(),
                Some(at),
            );
        }
        if rules.should_decrease(&text) {
            return empty(leading_whitespace(&text).to_string(), Some(i));
        }
    }
    empty(leading_whitespace(&text_of(lines, 0)).to_string(), Some(0))
}

// ------------------------------------------------------------------ 粘贴后的重新缩进

/// 覆盖一段连续行的虚拟模型（`AutoIndentOnPaste` 逐行调整时用）。
struct PatchedLines<'a> {
    inner: &'a dyn LineSource,
    first: u64,
    rows: &'a [String],
}

impl LineSource for PatchedLines<'_> {
    fn line_count(&self) -> u64 {
        self.inner.line_count()
    }
    fn line_text(&self, line: u64) -> Option<String> {
        if line >= self.first && (line - self.first) < self.rows.len() as u64 {
            return Some(self.rows[(line - self.first) as usize].clone());
        }
        self.inner.line_text(line)
    }
}

/// `editor.autoIndentOnPaste`：把 `first..=last` 这几行按 [`good_indent_for_line`] 逐行重新缩进
/// （VS Code `contrib/indentation` 的 `AutoIndentOnPaste`：每一行都在“前面几行已经调整好”的
/// 模型上判定）。只改前导空白，缩进宽度不变的行不产生改动。
pub fn reindent_lines(
    auto_indent: AutoIndent,
    lines: &dyn LineSource,
    config: &CompiledLanguageConfiguration,
    first: u64,
    last: u64,
    converter: &IndentConverter,
) -> Vec<super::Change> {
    if auto_indent < AutoIndent::Full || last < first {
        return Vec::new();
    }
    let mut rows: Vec<String> = (first..=last).map(|l| text_of(lines, l)).collect();
    let mut changes = Vec::new();
    for line in first..=last {
        let index = (line - first) as usize;
        if rows[index].trim().is_empty() {
            continue;
        }
        let patched = PatchedLines {
            inner: lines,
            first,
            rows: &rows,
        };
        let Some(good) = good_indent_for_line(auto_indent, &patched, config, line, converter)
        else {
            continue;
        };
        let old = leading_whitespace(&rows[index]).to_string();
        let tab = converter.tab_size.max(1);
        if visible_width(&good, tab) == visible_width(&old, tab) {
            continue;
        }
        changes.push(super::Change::replace(
            (line, 0),
            (line, old.len()),
            good.clone(),
        ));
        rows[index] = format!("{good}{}", &rows[index][old.len()..]);
    }
    changes
}

// ------------------------------------------------------------------ getGoodIndentForLine

/// `getGoodIndentForLine`：`line` 这一行“应该有的”缩进（粘贴与 `reindent` 的判定基础）。
pub fn good_indent_for_line(
    auto_indent: AutoIndent,
    lines: &dyn LineSource,
    config: &CompiledLanguageConfiguration,
    line: u64,
    converter: &IndentConverter,
) -> Option<String> {
    if auto_indent < AutoIndent::Full {
        return None;
    }
    let rules = rules_of(config)?;
    let indent = inherit_indent_for_line(auto_indent, lines, rules, line, true)?;
    let line_content = text_of(lines, line);
    if let Some(inherit_line) = indent.line {
        // 来源行与本行之间全是空白行时才套用 onEnterRules。
        let mut apply = true;
        for between in inherit_line..line.saturating_sub(1) {
            if !text_of(lines, between).trim().is_empty() {
                apply = false;
                break;
            }
        }
        if apply {
            let inherit_text = text_of(lines, inherit_line);
            if let Some(result) =
                super::on_enter::on_enter(auto_indent, config, "", &inherit_text, "")
            {
                let mut indentation = leading_whitespace(&inherit_text).to_string();
                if let Some(remove) = result.remove_text {
                    let keep = indentation.len().saturating_sub(remove as usize);
                    indentation.truncate(keep);
                }
                indentation = match result.indent {
                    Action::Indent | Action::IndentOutdent => converter.shift_indent(&indentation),
                    Action::Outdent => converter.unshift_indent(&indentation),
                    Action::None => indentation,
                };
                if rules.should_decrease(&line_content) {
                    indentation = converter.unshift_indent(&indentation);
                }
                if let Some(append) = &result.append_text {
                    indentation.push_str(append);
                }
                return Some(leading_whitespace(&indentation).to_string());
            }
        }
    }
    Some(if rules.should_decrease(&line_content) {
        match indent.action {
            Some(Action::Indent) => indent.indentation,
            _ => converter.unshift_indent(&indent.indentation),
        }
    } else {
        match indent.action {
            Some(Action::Indent) => converter.shift_indent(&indent.indentation),
            _ => indent.indentation,
        }
    })
}

// ------------------------------------------------------------------ getIndentForEnter

/// `getIndentForEnter` 的结果：回车前那一行的缩进与回车后新行的缩进。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnterIndent {
    pub before_enter: String,
    pub after_enter: String,
}

/// `getIndentForEnter`：在 `start` 回车（选区到 `end`）时，回车前那一行与回车后新行的缩进。
pub fn enter_indent(
    auto_indent: AutoIndent,
    lines: &dyn LineSource,
    config: &CompiledLanguageConfiguration,
    start: (u64, usize),
    end: (u64, usize),
    converter: &IndentConverter,
) -> Option<EnterIndent> {
    if auto_indent < AutoIndent::Full {
        return None;
    }
    let rules = rules_of(config)?;
    let (line, column) = start;
    let text = text_of(lines, line);
    let before_enter_text = text.get(..column.min(text.len())).unwrap_or("").to_string();
    let end_text = text_of(lines, end.0);
    let after_enter_text = end_text
        .get(end.1.min(end_text.len())..)
        .unwrap_or("")
        .to_string();
    let before_enter_indent = leading_whitespace(&before_enter_text).to_string();
    let virtual_model = OverrideLine {
        inner: lines,
        line,
        text: &before_enter_text,
    };
    let Some(after) = inherit_indent_for_line(auto_indent, &virtual_model, rules, line + 1, true)
    else {
        return Some(EnterIndent {
            before_enter: before_enter_indent.clone(),
            after_enter: before_enter_indent,
        });
    };
    let mut after_enter_indent = after.indentation;
    if after.action == Some(Action::Indent) {
        after_enter_indent = converter.shift_indent(&after_enter_indent);
    }
    if rules.should_decrease(&after_enter_text) {
        after_enter_indent = converter.unshift_indent(&after_enter_indent);
    }
    Some(EnterIndent {
        before_enter: before_enter_indent,
        after_enter: after_enter_indent,
    })
}

// ------------------------------------------------------------------ getIndentActionForType

/// `getIndentActionForType`：输入 `ch` 后该行若命中 `decreaseIndentPattern`，返回新的行首缩进。
pub fn indent_action_for_type(
    auto_indent: AutoIndent,
    lines: &dyn LineSource,
    config: &CompiledLanguageConfiguration,
    start: (u64, usize),
    end: (u64, usize),
    ch: &str,
    converter: &IndentConverter,
) -> Option<String> {
    if auto_indent < AutoIndent::Full {
        return None;
    }
    let rules = rules_of(config)?;
    let (line, column) = start;
    let text = text_of(lines, line);
    let before = text.get(..column.min(text.len())).unwrap_or("");
    let after = if start == end {
        text.get(column.min(text.len())..).unwrap_or("")
    } else if end.0 == line {
        text.get(end.1.min(text.len())..).unwrap_or("")
    } else {
        ""
    };
    let without = format!("{before}{after}");
    let with = format!("{before}{ch}{after}");
    if rules.should_decrease(&without) || !rules.should_decrease(&with) {
        return None;
    }
    let inherited = inherit_indent_for_line(auto_indent, lines, rules, line, false)?;
    let mut indentation = inherited.indentation;
    if inherited.action != Some(Action::Indent) {
        indentation = converter.unshift_indent(&indentation);
    }
    Some(indentation)
}

#[cfg(test)]
mod tests {
    use super::super::testing::lines;
    use super::*;

    fn spaces(n: usize) -> IndentConverter {
        IndentConverter {
            insert_spaces: true,
            tab_size: n,
            indent_size: n,
        }
    }

    // --- `ShiftCommand` / `normalizeIndentation` 的换算（`shiftCommand.test.ts` 的口径）

    #[test]
    fn shift_and_unshift_follow_tab_stops() {
        let c = spaces(4);
        assert_eq!(c.shift_indent(""), "    ");
        assert_eq!(c.shift_indent("  "), "    ");
        assert_eq!(c.shift_indent("    "), "        ");
        assert_eq!(c.unshift_indent(""), "");
        assert_eq!(c.unshift_indent("  "), "");
        assert_eq!(c.unshift_indent("     "), "    ");
        assert_eq!(c.unshift_indent("        "), "    ");
    }

    #[test]
    fn shift_with_tabs_rounds_up() {
        let c = IndentConverter {
            insert_spaces: false,
            tab_size: 4,
            indent_size: 4,
        };
        assert_eq!(c.shift_indent("\t"), "\t\t");
        assert_eq!(c.unshift_indent("\t\t"), "\t");
        assert_eq!(c.unshift_indent(""), "");
    }

    #[test]
    fn normalize_indentation_rewrites_only_leading_whitespace() {
        let c = spaces(4);
        assert_eq!(c.normalize("\tfoo"), "    foo");
        assert_eq!(c.normalize("  \tfoo"), "    foo");
        assert_eq!(c.normalize("foo\t"), "foo\t");
        let t = IndentConverter {
            insert_spaces: false,
            tab_size: 4,
            indent_size: 4,
        };
        assert_eq!(t.normalize("      foo"), "\t  foo");
        assert_eq!(t.normalize("        foo"), "\t\tfoo");
    }

    #[test]
    fn visible_width_counts_tab_stops() {
        assert_eq!(visible_width("\t", 4), 4);
        assert_eq!(visible_width(" \t", 4), 4);
        assert_eq!(visible_width("     ", 4), 5);
    }

    // --- VS Code `javascriptIndentationRules.ts` 的四条正则，逐例验证推导过程
    //     （`autoIndent.ts` 的 `getInheritIndentForLine` / `getGoodIndentForLine` /
    //     `getIndentActionForType`，用例取自 `indentation.test.ts` 与 `cursor.test.ts` 的场景）。

    fn javascript_rules() -> rove_syntax::language_config::CompiledLanguageConfiguration {
        use rove_syntax::{IndentationRules, JsRegex, LanguageConfiguration};
        let js = |source: &str| JsRegex {
            source: source.into(),
            flags: "".into(),
        };
        let data = LanguageConfiguration {
            brackets: vec![
                rove_syntax::Pair {
                    open: "{".into(),
                    close: "}".into(),
                },
                rove_syntax::Pair {
                    open: "[".into(),
                    close: "]".into(),
                },
                rove_syntax::Pair {
                    open: "(".into(),
                    close: ")".into(),
                },
            ]
            .into(),
            indentation: Some(IndentationRules {
                increase: js(r"^((?!\/\/).)*(\{[^}\x22'`]*|\([^)\x22'`]*|\[[^\]\x22'`]*)$"),
                decrease: js(r"^((?!.*?\/\*).*\*\/)?\s*[\}\]].*$"),
                indent_next: Some(js(
                    r"^((.*=>\s*)|((.*[^\w]+|\s*)(if|while|for)\s*\(.*\)\s*))$",
                )),
                unindented: Some(js(
                    r"^(\t|[ ])*[ ]\*[^/]*\*\/\s*$|^(\t|[ ])*[ ]\*\/\s*$|^(\t|[ ])*[ ]\*([ ]([^\*]|\*(?!\/))*)?$",
                )),
            }),
            ..LanguageConfiguration::default()
        };
        rove_syntax::language_config::CompiledLanguageConfiguration::compile(
            "fixture",
            "fixture",
            std::sync::Arc::new(data),
        )
        .unwrap()
    }

    #[test]
    fn indent_metadata_matches_the_four_patterns() {
        let c = javascript_rules();
        let r = rules_of(&c).unwrap();
        assert_eq!(r.indent_metadata("function foo() {"), INCREASE_MASK);
        assert_eq!(r.indent_metadata("}"), DECREASE_MASK);
        assert_eq!(r.indent_metadata("if (true)"), INDENT_NEXTLINE_MASK);
        // `unIndentedLinePattern` 命中的行只返回 UNINDENT_MASK（VS Code 的 getIndentMetadata）。
        assert_eq!(r.indent_metadata("     * doc"), UNINDENT_MASK);
        assert_eq!(r.indent_metadata("let a = 1;"), 0);
    }

    #[test]
    fn inherit_indent_after_an_open_bracket() {
        let c = javascript_rules();
        let r = rules_of(&c).unwrap();
        let l = lines(&["function foo() {", "", ""]);
        let got = inherit_indent_for_line(AutoIndent::Full, &l, r, 1, true).unwrap();
        assert_eq!(got.action, Some(Action::Indent));
        assert_eq!(got.indentation, "");
        assert_eq!(
            good_indent_for_line(AutoIndent::Full, &l, &c, 1, &spaces(4)).unwrap(),
            "    "
        );
    }

    #[test]
    fn good_indent_outdents_a_closing_bracket() {
        let c = javascript_rules();
        let l = lines(&[
            "function foo() {",
            "    if (true) {",
            "        return 1;",
            "    }",
            "}",
        ]);
        assert_eq!(
            good_indent_for_line(AutoIndent::Full, &l, &c, 3, &spaces(4)).unwrap(),
            "    "
        );
        assert_eq!(
            good_indent_for_line(AutoIndent::Full, &l, &c, 4, &spaces(4)).unwrap(),
            ""
        );
    }

    #[test]
    fn indent_next_line_pattern_indents_only_one_line() {
        let c = javascript_rules();
        let l = lines(&["if (true)", "    foo();", "bar();"]);
        assert_eq!(
            good_indent_for_line(AutoIndent::Full, &l, &c, 1, &spaces(4)).unwrap(),
            "    "
        );
        // 第三行回到基准缩进：`indentNextLinePattern` 只影响紧挨着的那一行。
        assert_eq!(
            good_indent_for_line(AutoIndent::Full, &l, &c, 2, &spaces(4)).unwrap(),
            ""
        );
    }

    #[test]
    fn unindented_lines_are_skipped_when_looking_upwards() {
        let c = javascript_rules();
        let l = lines(&[
            "function foo() {",
            "    /**",
            "     * doc",
            "     */",
            "    bar();",
        ]);
        assert_eq!(
            good_indent_for_line(AutoIndent::Full, &l, &c, 4, &spaces(4)).unwrap(),
            "    "
        );
    }

    #[test]
    fn typing_a_closing_bracket_realigns_the_line() {
        let c = javascript_rules();
        let l = lines(&["function foo() {", "        "]);
        let got = indent_action_for_type(AutoIndent::Full, &l, &c, (1, 8), (1, 8), "}", &spaces(4));
        assert_eq!(got.as_deref(), Some(""));
        // 嵌套一层时回到 `if` 那一行的缩进。
        let l = lines(&["function foo() {", "    if (x) {", "            "]);
        let got =
            indent_action_for_type(AutoIndent::Full, &l, &c, (2, 12), (2, 12), "}", &spaces(4));
        assert_eq!(got.as_deref(), Some("    "));
    }

    #[test]
    fn typing_does_not_realign_when_the_line_already_decreases() {
        let c = javascript_rules();
        let l = lines(&["function foo() {", "        }"]);
        let got = indent_action_for_type(AutoIndent::Full, &l, &c, (1, 9), (1, 9), "}", &spaces(4));
        assert_eq!(got, None);
    }

    #[test]
    fn indentation_rules_are_off_below_full() {
        let c = javascript_rules();
        let r = rules_of(&c).unwrap();
        let l = lines(&["function foo() {", ""]);
        assert!(inherit_indent_for_line(AutoIndent::Advanced, &l, r, 1, true).is_none());
        assert!(good_indent_for_line(AutoIndent::Advanced, &l, &c, 1, &spaces(4)).is_none());
        assert!(indent_action_for_type(
            AutoIndent::Advanced,
            &l,
            &c,
            (1, 0),
            (1, 0),
            "}",
            &spaces(4)
        )
        .is_none());
    }

    #[test]
    fn reindent_lines_fixes_a_pasted_block() {
        let c = javascript_rules();
        // 粘到 `function foo() {` 下面的两行缩进不对：逐行按 good indent 调整。
        let l = lines(&["function foo() {", "if (x) {", "return 1;", "}", "}"]);
        let changes = reindent_lines(AutoIndent::Full, &l, &c, 1, 3, &spaces(4));
        let got: Vec<(u64, String)> = changes
            .iter()
            .map(|c| (c.start.0, c.text.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                (1, "    ".to_string()),
                (2, "        ".to_string()),
                (3, "    ".to_string()),
            ]
        );
    }

    #[test]
    fn reindent_lines_leaves_correct_indentation_alone() {
        let c = javascript_rules();
        let l = lines(&["function foo() {", "    return 1;", "}"]);
        assert!(reindent_lines(AutoIndent::Full, &l, &c, 1, 1, &spaces(4)).is_empty());
        // `full` 以下不重新缩进。
        let l = lines(&["function foo() {", "return 1;", "}"]);
        assert!(reindent_lines(AutoIndent::Advanced, &l, &c, 1, 1, &spaces(4)).is_empty());
    }

    #[test]
    fn first_non_blank_line_inherits_nothing() {
        let c = javascript_rules();
        let r = rules_of(&c).unwrap();
        let l = lines(&["", "", "foo();"]);
        let got = inherit_indent_for_line(AutoIndent::Full, &l, r, 2, true).unwrap();
        assert_eq!(got.indentation, "");
        assert_eq!(got.action, None);
    }

    #[test]
    fn override_line_sees_the_replacement() {
        let base = lines(&["a", "b", "c"]);
        let v = OverrideLine {
            inner: &base,
            line: 1,
            text: "B",
        };
        assert_eq!(v.line_text(0).unwrap(), "a");
        assert_eq!(v.line_text(1).unwrap(), "B");
        assert_eq!(v.line_count(), 3);
    }
}
