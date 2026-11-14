//! 输入时编辑器替你做的事（#53 / R52）：注释切换、自动缩进、`onEnterRules`、自动闭合与自动包围。
//!
//! 这一层是**纯算法**：只读行文本（[`LineSource`]）与 #12 的语言配置，产出抽象的 [`Change`]，
//! 由 [`super::edit`] 换算成 #46 的事务。没有 GPUI 类型，便于逐例对拍 VS Code 的单元测试。
//!
//! | 模块 | 对应的 VS Code 源文件 |
//! |---|---|
//! | [`indent`] | `editor/common/languages/autoIndent.ts`、`supports/indentRules.ts`、`commands/shiftCommand.ts` 的缩进换算 |
//! | [`on_enter`] | `editor/common/languages/supports/onEnter.ts`、`enterAction.ts`、`cursorTypeOperations.ts` 的 `_enter` |
//! | [`autoclose`] | `editor/common/cursor/cursorTypeOperations.ts` 的自动闭合 / 越过 / 包围，`cursorDeleteOperations.ts` 的 `_isAutoClosingPairDelete` |
//! | [`comments`] | `editor/contrib/comment/browser/lineCommentCommand.ts`、`blockCommentCommand.ts` |
//!
//! 行号从 0 开始（VS Code 从 1 开始），列是行内 UTF-8 字节偏移（VS Code 是 UTF-16 的 1 基列）。

pub mod autoclose;
pub mod comments;
pub mod indent;
pub mod on_enter;

/// 一处抽象改动：行 + 行内 UTF-8 字节列，半开区间 `[start, end)`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub start: (u64, usize),
    pub end: (u64, usize),
    pub text: String,
}

impl Change {
    pub fn replace(start: (u64, usize), end: (u64, usize), text: impl Into<String>) -> Self {
        Self {
            start,
            end,
            text: text.into(),
        }
    }
    pub fn insert(at: (u64, usize), text: impl Into<String>) -> Self {
        Self::replace(at, at, text)
    }
    pub fn delete(start: (u64, usize), end: (u64, usize)) -> Self {
        Self::replace(start, end, "")
    }
}

/// 一次自动行为的结果：改动加上（可选的）改动后光标位置。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoEdit {
    pub changes: Vec<Change>,
    /// 改动后的光标（行 + 字节列，按**改动后**的文本）。`None` 表示由事务映射决定。
    pub caret: Option<(u64, usize)>,
    /// 改动后的选区锚点；与 `caret` 一起构成一段选区（自动包围时用）。
    pub anchor: Option<(u64, usize)>,
}

/// 行首空白（空格与制表符）。VS Code `strings.getLeadingWhitespace`。
pub fn leading_whitespace(line: &str) -> &str {
    let n = line.len() - line.trim_start_matches([' ', '\t']).len();
    &line[..n]
}

/// 第一个非空白字符的字节下标；整行空白时为 `None`。VS Code `strings.firstNonWhitespaceIndex`。
pub fn first_non_whitespace(line: &str) -> Option<usize> {
    line.find(|c: char| c != ' ' && c != '\t')
}

/// 是否是引号（VS Code `isQuote`：`'`、`"`、`` ` ``）。
pub fn is_quote(s: &str) -> bool {
    matches!(s, "'" | "\"" | "`")
}

#[cfg(test)]
pub(crate) mod testing {
    use super::super::columns::LineSource;

    /// 测试用的行数组（与 VS Code 单元测试一样直接给文本行）。
    pub struct Lines(pub Vec<String>);

    pub fn lines(rows: &[&str]) -> Lines {
        Lines(rows.iter().map(|s| (*s).to_string()).collect())
    }

    impl LineSource for Lines {
        fn line_count(&self) -> u64 {
            self.0.len().max(1) as u64
        }
        fn line_text(&self, line: u64) -> Option<String> {
            self.0.get(line as usize).cloned()
        }
    }
}
