//! 输入文本的模式判定。前缀只在文本开头生效（与 VS Code 的 quick access 前缀一致）。

/// 前缀字符。
pub const COMMANDS_PREFIX: char = '>';
pub const LINE_PREFIX: char = ':';
pub const SYMBOLS_PREFIX: char = '@';
pub const WORKSPACE_SYMBOLS_PREFIX: char = '#';

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode<'a> {
    /// 按文件名/路径找文件；可带 `:行[:列]`（或 `(行,列)`）后缀。
    Files {
        query: &'a str,
        target: Option<FileTarget>,
    },
    /// `>`：命令，参数是前缀之后的过滤文本。
    Commands(&'a str),
    /// `:`：转到当前编辑器的行/列。
    Line(&'a str),
    /// `@`：当前编辑器中的符号。
    Symbols(&'a str),
    /// `#`：工作区符号（#60 实现，与 `Ctrl+T` 同一份数据、同一套排序）。
    WorkspaceSymbols(&'a str),
}

/// 文件查询里的位置（1 起始，0 表示没写列）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileTarget {
    pub line: u32,
    pub column: u32,
}

pub fn parse(text: &str) -> Mode<'_> {
    let mut chars = text.chars();
    match chars.next() {
        Some(COMMANDS_PREFIX) => Mode::Commands(chars.as_str()),
        Some(LINE_PREFIX) => Mode::Line(chars.as_str()),
        Some(SYMBOLS_PREFIX) => Mode::Symbols(chars.as_str()),
        Some(WORKSPACE_SYMBOLS_PREFIX) => Mode::WorkspaceSymbols(chars.as_str()),
        _ => {
            let (query, target) = split_file_target(text);
            Mode::Files { query, target }
        }
    }
}

/// 从文件查询末尾取出 `:行[:列]`、`(行[,列])` 或 `#行[,列]`（VS Code `extractRangeFromFilter` 的常用写法）。
/// 没有文件名部分（例如只有 `:12`）时不拆：那是转到行的前缀，由 [`parse`] 先处理。
pub fn split_file_target(text: &str) -> (&str, Option<FileTarget>) {
    let trimmed = text.trim_end();
    // `name(12,5)` / `name(12)`
    if let Some(body) = trimmed.strip_suffix(')') {
        if let Some(open) = body.rfind('(') {
            if let Some(target) = parse_numbers(&body[open + 1..], ',') {
                return (&body[..open], Some(target));
            }
        }
    }
    // `name:12:5` / `name:12`
    let mut end = trimmed.len();
    let mut numbers: Vec<&str> = Vec::new();
    while numbers.len() < 2 {
        let head = &trimmed[..end];
        match head.rfind(':') {
            Some(colon)
                if colon > 0
                    && !head[colon + 1..].is_empty()
                    && head[colon + 1..].bytes().all(|b| b.is_ascii_digit()) =>
            {
                numbers.push(&head[colon + 1..]);
                end = colon;
            }
            _ => break,
        }
    }
    if numbers.is_empty() {
        return (text, None);
    }
    numbers.reverse();
    let line = numbers[0].parse().unwrap_or(0);
    let column = numbers.get(1).map_or(0, |c| c.parse().unwrap_or(0));
    (&trimmed[..end], Some(FileTarget { line, column }))
}

fn parse_numbers(text: &str, sep: char) -> Option<FileTarget> {
    let mut parts = text.split(sep).map(str::trim);
    let line: u32 = parts.next()?.parse().ok()?;
    let column = match parts.next() {
        Some(c) => c.parse().ok()?,
        None => 0,
    };
    parts
        .next()
        .is_none()
        .then_some(FileTarget { line, column })
}
