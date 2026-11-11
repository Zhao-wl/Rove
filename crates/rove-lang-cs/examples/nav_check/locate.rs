//! 锚点定位：先按行列找，行文本对不上时在附近按锚点文本重新定位。
//!
//! 只依赖磁盘上的源码文本，不感知索引。

use std::path::Path;

/// 默认在原行号上下各搜索多少行。
pub const DEFAULT_WINDOW: u32 = 40;

/// 一个源码文件的行（已去 BOM、去行尾 `\r`）。
pub struct SourceText {
    lines: Vec<String>,
}

impl SourceText {
    pub fn from_text(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let lines = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
            .collect();
        SourceText { lines }
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Ok(Self::from_text(&String::from_utf8_lossy(&bytes)))
    }

    pub fn line(&self, line0: usize) -> Option<&str> {
        self.lines.get(line0).map(String::as_str)
    }
}

/// 行定位结果：0 起算的行号，以及是否偏离了用例里写的行号。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineHit {
    pub line0: u32,
    pub moved: bool,
}

/// 按“行号 + 锚点文本”找行。原行匹配直接用；否则在 ±window 行内由近到远找
/// 去首尾空白后与锚点完全相同的行；同一距离上下各有一处时视为有歧义。
pub fn locate_line(
    text: &SourceText,
    line1: u32,
    anchor: &str,
    window: u32,
) -> Result<LineHit, String> {
    let want = line1.saturating_sub(1) as i64;
    let matches = |l: i64| l >= 0 && text.line(l as usize).is_some_and(|s| s.trim() == anchor);
    if matches(want) {
        return Ok(LineHit {
            line0: want as u32,
            moved: false,
        });
    }
    for d in 1..=window as i64 {
        let (up, down) = (matches(want - d), matches(want + d));
        match (up, down) {
            (true, true) => {
                return Err(format!(
                    "锚点在第 {} 行与第 {} 行同时出现，无法确定",
                    want - d + 1,
                    want + d + 1
                ))
            }
            (true, false) => {
                return Ok(LineHit {
                    line0: (want - d) as u32,
                    moved: true,
                })
            }
            (false, true) => {
                return Ok(LineHit {
                    line0: (want + d) as u32,
                    moved: true,
                })
            }
            _ => {}
        }
    }
    Err(format!(
        "第 {line1} 行附近 ±{window} 行内找不到锚点文本 {anchor:?}"
    ))
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// token 在行内的全部出现位置（字符下标，0 起算）。以标识符字符开头/结尾的 token 要求词边界。
pub fn token_positions(line: &str, token: &str) -> Vec<usize> {
    let chars: Vec<char> = line.chars().collect();
    let tok: Vec<char> = token.chars().collect();
    if tok.is_empty() || tok.len() > chars.len() {
        return Vec::new();
    }
    let need_left = is_ident(tok[0]);
    let need_right = is_ident(*tok.last().unwrap());
    (0..=chars.len() - tok.len())
        .filter(|&i| chars[i..i + tok.len()] == tok[..])
        .filter(|&i| !need_left || i == 0 || !is_ident(chars[i - 1]))
        .filter(|&i| {
            let end = i + tok.len();
            !need_right || end == chars.len() || !is_ident(chars[end])
        })
        .collect()
}

/// 定位到的查询点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenHit {
    pub line0: u32,
    /// 0 起算的字符列。
    pub char_col0: u32,
    /// 0 起算的 UTF-8 字节列（查询接口用）。
    pub utf8_col0: u32,
    pub moved: bool,
}

/// 字符列 → UTF-8 字节列。
pub fn utf8_col(line: &str, char_col0: usize) -> u32 {
    line.chars()
        .take(char_col0)
        .map(char::len_utf8)
        .sum::<usize>() as u32
}

/// 按“行号 + 列号 + token + 锚点”找查询点。
///
/// - 原行与锚点一致：`col` 处必须正好是 `token`，否则是用例写错了。
/// - 行已移动：token 在行内只出现一次就用它；出现多次时优先与原列号相同的那一处，
///   都不是则失败（标为过期，需要人工重新整理）。
pub fn locate_token(
    text: &SourceText,
    line1: u32,
    col1: u32,
    token: &str,
    anchor: &str,
    window: u32,
) -> Result<TokenHit, String> {
    let hit = locate_line(text, line1, anchor, window)?;
    let line = text.line(hit.line0 as usize).unwrap_or_default();
    let positions = token_positions(line, token);
    let want = col1.saturating_sub(1) as usize;
    let col = if positions.contains(&want) {
        want
    } else if hit.moved && positions.len() == 1 {
        positions[0]
    } else if hit.moved {
        return Err(format!(
            "锚点行已移动到第 {} 行，但 token {token:?} 在行内出现 {} 次且原列号 {col1} 处不是它",
            hit.line0 + 1,
            positions.len()
        ));
    } else {
        return Err(format!(
            "第 {line1} 行第 {col1} 列不是 token {token:?}（用例的列号写错了）"
        ));
    };
    Ok(TokenHit {
        line0: hit.line0,
        char_col0: col as u32,
        utf8_col0: utf8_col(line, col),
        moved: hit.moved,
    })
}
