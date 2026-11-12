//! 转到行/列（`:` 前缀与 `workbench.action.gotoLine`）的文本解析。规则同 VS Code `gotoLineQuickAccess`。

/// 输入的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineState {
    /// 还没有有效的行号：提示当前位置与可输入范围。
    Prompt,
    /// 可以跳转（行号 1 起始，已按“负数从末尾数起”换算；列 1 起始，`None` 表示没写）。
    Go { line: u64, column: Option<u64> },
}

/// `rest` 是 `:` 之后的文本（也接受 `行,列`、`行#列`）；`line_count` 是文档总行数。
pub fn state(rest: &str, line_count: u64) -> LineState {
    let numbers: Vec<i64> = rest
        .split([',', ':', '#'])
        .filter_map(|part| parse_int(part.trim()))
        .collect();
    let Some(&first) = numbers.first() else {
        return LineState::Prompt;
    };
    // 负数表示从末尾数起
    let end_line = line_count as i64 + 1;
    let line = if first > 0 { first } else { end_line + first };
    if line < 1 || line > line_count as i64 {
        return LineState::Prompt;
    }
    LineState::Go {
        line: line as u64,
        column: numbers.get(1).copied().filter(|c| *c > 0).map(|c| c as u64),
    }
}

/// JS `parseInt(text, 10)`：取开头的（带符号）数字，没有则 `None`。
fn parse_int(text: &str) -> Option<i64> {
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let end = digits
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    digits[..end].parse::<i64>().ok().map(|v| sign * v)
}
