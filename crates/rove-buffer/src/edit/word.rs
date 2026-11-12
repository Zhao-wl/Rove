//! 单词边界（#46）：移植 VS Code 1.145.2 的 `cursorWordOperations.ts`
//! （`WordOperations` / `WordPartOperations`）与 `wordCharacterClassifier.ts`。
//!
//! 本模块只回答“给定位置与方向，下一个边界在哪”，命令本身由 R46 登记：
//!
//! | VS Code 命令 | 本模块 |
//! |---|---|
//! | `cursorWordLeft(Select)` | [`MoveWord::Left`]（`WordStartFast`） |
//! | `cursorWordStartLeft(Select)` / `cursorWordEndLeft(Select)` | [`MoveWord::StartLeft`] / [`MoveWord::EndLeft`] |
//! | `cursorWordRight(Select)` / `cursorWordEndRight(Select)` | [`MoveWord::Right`] / [`MoveWord::EndRight`]（都是 `WordEnd`） |
//! | `cursorWordStartRight(Select)` | [`MoveWord::StartRight`] |
//! | `cursorWordAccessibilityLeft/Right(Select)` | [`MoveWord::AccessibilityLeft`] / [`MoveWord::AccessibilityRight`]（固定用默认分隔符） |
//! | `cursorWordPartLeft/Right(Select)` | [`MoveWord::PartLeft`] / [`MoveWord::PartRight`] |
//! | `deleteWordLeft` / `deleteWordStartLeft` / `deleteWordEndLeft` | [`DeleteWord::Left`] / [`DeleteWord::StartLeft`] / [`DeleteWord::EndLeft`] |
//! | `deleteWordRight` / `deleteWordStartRight` / `deleteWordEndRight` | [`DeleteWord::Right`] / [`DeleteWord::StartRight`] / [`DeleteWord::EndRight`] |
//! | `deleteWordPartLeft` / `deleteWordPartRight` | [`DeleteWord::PartLeft`] / [`DeleteWord::PartRight`] |
//! | `deleteInsideWord` | [`delete_inside_word`] |
//!
//! 与 VS Code 的差异：VS Code 以 UTF-16 码元为下标，这里以 Unicode 字符为下标（代理对的两个
//! 码元在 VS Code 里都归为普通字符，结果等价）；`editor.wordSegmenterLocales`（`Intl.Segmenter`
//! 的词典分词）没有移植，等同于默认值 `[]`。
//!
//! 语言级 `wordPattern`（Markdown / XML 自定义了它）只影响“光标处的单词”（双击选词、同名高亮、
//! 选中下一个匹配），由 [`word_at_with_pattern`] 接入 R12 的 `word_at`；移动与删除与 VS Code
//! 一样只看 `editor.wordSeparators`。

use rove_base::*;

use super::lines::{line_count, LineChars};
use crate::contracts::TextSource;

/// VS Code `editor.wordSeparators` 的默认值。
pub const DEFAULT_WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

/// 字符分类（`WordCharacterClass`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordCharClass {
    Regular,
    Whitespace,
    Separator,
}

/// 由 `editor.wordSeparators` 构造的分类器：空格与 Tab 是空白，分隔符字符串里的字符是分隔符，
/// 其他都是普通字符。
#[derive(Clone, Debug)]
pub struct WordClassifier {
    ascii: [WordCharClass; 128],
    others: Vec<char>,
}

impl WordClassifier {
    pub fn new(separators: &str) -> Self {
        let mut ascii = [WordCharClass::Regular; 128];
        let mut others = Vec::new();
        for ch in separators.chars() {
            if ch.is_ascii() {
                ascii[ch as usize] = WordCharClass::Separator;
            } else {
                others.push(ch);
            }
        }
        others.sort_unstable();
        others.dedup();
        ascii[b' ' as usize] = WordCharClass::Whitespace;
        ascii[b'\t' as usize] = WordCharClass::Whitespace;
        Self { ascii, others }
    }

    pub fn class(&self, ch: char) -> WordCharClass {
        if ch.is_ascii() {
            self.ascii[ch as usize]
        } else if self.others.binary_search(&ch).is_ok() {
            WordCharClass::Separator
        } else {
            WordCharClass::Regular
        }
    }
}

impl Default for WordClassifier {
    fn default() -> Self {
        Self::new(DEFAULT_WORD_SEPARATORS)
    }
}

/// `WordNavigationType`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordNavigation {
    WordStart,
    WordStartFast,
    WordEnd,
    WordAccessibility,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WordType {
    None,
    Regular,
    Separator,
}

#[derive(Clone, Copy, Debug)]
struct FoundWord {
    start: usize,
    end: usize,
    kind: WordType,
    next: WordCharClass,
}

/// 行内位置：行号 + 字符下标（VS Code 的 `column - 1`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Pos {
    line: u64,
    ch: usize,
}

struct Doc<'a> {
    text: &'a dyn TextSource,
    count: u64,
}

impl<'a> Doc<'a> {
    fn new(text: &'a dyn TextSource) -> Result<Self> {
        Ok(Self {
            text,
            count: line_count(text)?,
        })
    }

    fn line(&self, line: u64) -> Result<LineChars> {
        LineChars::read(self.text, Line(line))
    }

    fn max(&self, line: u64) -> Result<usize> {
        Ok(self.line(line)?.len())
    }

    fn to_pos(&self, position: Position) -> Result<(Pos, LineChars)> {
        let chars = self.line(position.line.0)?;
        let ch = chars.index(position.column)?;
        Ok((
            Pos {
                line: position.line.0,
                ch,
            },
            chars,
        ))
    }

    fn to_position(&self, pos: Pos) -> Result<Position> {
        let chars = self.line(pos.line)?;
        Ok(Position {
            line: Line(pos.line),
            column: chars.column(pos.ch),
        })
    }

    fn range(&self, a: Pos, b: Pos) -> Result<TextRange> {
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        Span::new(self.to_position(start)?, self.to_position(end)?)
    }
}

fn create_word(kind: WordType, next: WordCharClass, start: usize, end: usize) -> FoundWord {
    FoundWord {
        start,
        end,
        kind,
        next,
    }
}

/// `_doFindPreviousWordOnLine`：`ch` 是光标的字符下标（VS Code 的 `position.column - 1`）。
fn previous_word(line: &[char], cls: &WordClassifier, ch: usize) -> Option<FoundWord> {
    let mut kind = WordType::None;
    for index in (0..ch.min(line.len())).rev() {
        let class = cls.class(line[index]);
        match class {
            WordCharClass::Regular => {
                if kind == WordType::Separator {
                    return Some(create_word(
                        kind,
                        class,
                        index + 1,
                        end_of_word(line, cls, kind, index + 1),
                    ));
                }
                kind = WordType::Regular;
            }
            WordCharClass::Separator => {
                if kind == WordType::Regular {
                    return Some(create_word(
                        kind,
                        class,
                        index + 1,
                        end_of_word(line, cls, kind, index + 1),
                    ));
                }
                kind = WordType::Separator;
            }
            WordCharClass::Whitespace => {
                if kind != WordType::None {
                    return Some(create_word(
                        kind,
                        class,
                        index + 1,
                        end_of_word(line, cls, kind, index + 1),
                    ));
                }
            }
        }
    }
    (kind != WordType::None).then(|| {
        create_word(
            kind,
            WordCharClass::Whitespace,
            0,
            end_of_word(line, cls, kind, 0),
        )
    })
}

/// `_findEndOfWord`。
fn end_of_word(line: &[char], cls: &WordClassifier, kind: WordType, start: usize) -> usize {
    for (index, &ch) in line.iter().enumerate().skip(start) {
        let class = cls.class(ch);
        if class == WordCharClass::Whitespace
            || (kind == WordType::Regular && class == WordCharClass::Separator)
            || (kind == WordType::Separator && class == WordCharClass::Regular)
        {
            return index;
        }
    }
    line.len()
}

/// `_doFindNextWordOnLine`：从字符下标 `ch` 开始向右找。
fn next_word(line: &[char], cls: &WordClassifier, ch: usize) -> Option<FoundWord> {
    let mut kind = WordType::None;
    for (index, &c) in line.iter().enumerate().skip(ch) {
        let class = cls.class(c);
        match class {
            WordCharClass::Regular => {
                if kind == WordType::Separator {
                    return Some(create_word(
                        kind,
                        class,
                        start_of_word(line, cls, kind, index - 1),
                        index,
                    ));
                }
                kind = WordType::Regular;
            }
            WordCharClass::Separator => {
                if kind == WordType::Regular {
                    return Some(create_word(
                        kind,
                        class,
                        start_of_word(line, cls, kind, index - 1),
                        index,
                    ));
                }
                kind = WordType::Separator;
            }
            WordCharClass::Whitespace => {
                if kind != WordType::None {
                    return Some(create_word(
                        kind,
                        class,
                        start_of_word(line, cls, kind, index - 1),
                        index,
                    ));
                }
            }
        }
    }
    (kind != WordType::None).then(|| {
        create_word(
            kind,
            WordCharClass::Whitespace,
            start_of_word(line, cls, kind, line.len() - 1),
            line.len(),
        )
    })
}

/// `_findStartOfWord`。
fn start_of_word(line: &[char], cls: &WordClassifier, kind: WordType, start: usize) -> usize {
    for index in (0..=start).rev() {
        let class = cls.class(line[index]);
        if class == WordCharClass::Whitespace
            || (kind == WordType::Regular && class == WordCharClass::Separator)
            || (kind == WordType::Separator && class == WordCharClass::Regular)
        {
            return index + 1;
        }
    }
    0
}

fn move_word_left_pos(
    doc: &Doc<'_>,
    cls: &WordClassifier,
    pos: Pos,
    nav: WordNavigation,
    has_multicursor: bool,
) -> Result<Pos> {
    let mut line = pos.line;
    let mut ch = pos.ch;
    if ch == 0 && line > 0 {
        line -= 1;
        ch = doc.max(line)?;
    }
    let chars = doc.line(line)?;
    let chars = &chars.chars;
    let mut previous = previous_word(chars, cls, ch);
    let start = |word: Option<FoundWord>| Pos {
        line,
        ch: word.map_or(0, |w| w.start),
    };
    match nav {
        WordNavigation::WordStart => Ok(start(previous)),
        WordNavigation::WordStartFast => {
            if let Some(word) = previous {
                if !has_multicursor
                    && word.kind == WordType::Separator
                    && word.end - word.start == 1
                    && word.next == WordCharClass::Regular
                {
                    // 跳过“单个分隔符 + 紧跟普通字符”构成的词。
                    previous = previous_word(chars, cls, word.start);
                }
            }
            Ok(start(previous))
        }
        WordNavigation::WordAccessibility => {
            while let Some(word) = previous.filter(|w| w.kind == WordType::Separator) {
                previous = previous_word(chars, cls, word.start);
            }
            Ok(start(previous))
        }
        WordNavigation::WordEnd => {
            if let Some(word) = previous {
                if ch <= word.end {
                    previous = previous_word(chars, cls, word.start);
                }
            }
            Ok(Pos {
                line,
                ch: previous.map_or(0, |w| w.end),
            })
        }
    }
}

fn move_word_right_pos(
    doc: &Doc<'_>,
    cls: &WordClassifier,
    pos: Pos,
    nav: WordNavigation,
) -> Result<Pos> {
    let mut line = pos.line;
    let mut ch = pos.ch;
    let mut moved_down = false;
    if ch == doc.max(line)? && line + 1 < doc.count {
        moved_down = true;
        line += 1;
        ch = 0;
    }
    let chars = doc.line(line)?;
    let len = chars.len();
    let chars = &chars.chars;
    let mut next = next_word(chars, cls, ch);
    let ch = match nav {
        WordNavigation::WordEnd => {
            if let Some(word) = next {
                if word.kind == WordType::Separator
                    && word.end - word.start == 1
                    && word.next == WordCharClass::Regular
                {
                    next = next_word(chars, cls, word.end);
                }
            }
            next.map_or(len, |w| w.end)
        }
        WordNavigation::WordAccessibility => {
            // 换到下一行时假装光标在第一个字符之前，免得漏掉行首的词。
            let current = if moved_down { -1 } else { ch as i64 };
            while let Some(word) =
                next.filter(|w| w.kind == WordType::Separator || w.start as i64 <= current)
            {
                next = next_word(chars, cls, word.end);
            }
            next.map_or(len, |w| w.start)
        }
        WordNavigation::WordStart | WordNavigation::WordStartFast => {
            if let Some(word) = next {
                if !moved_down && ch >= word.start {
                    next = next_word(chars, cls, word.end);
                }
            }
            next.map_or(len, |w| w.start)
        }
    };
    Ok(Pos { line, ch })
}

fn is_lower(ch: char) -> bool {
    ch.is_ascii_lowercase()
}
fn is_upper(ch: char) -> bool {
    ch.is_ascii_uppercase()
}
fn is_digit(ch: char) -> bool {
    ch.is_ascii_digit()
}

/// `_moveWordPartLeft`。
fn word_part_left(doc: &Doc<'_>, pos: Pos) -> Result<Pos> {
    if pos.ch == 0 {
        return Ok(if pos.line > 0 {
            Pos {
                line: pos.line - 1,
                ch: doc.max(pos.line - 1)?,
            }
        } else {
            pos
        });
    }
    let chars = doc.line(pos.line)?;
    let line = &chars.chars;
    let max_column = line.len() + 1;
    // VS Code 的 1 起列号：column 从 position.column - 1 递减到 2。
    let mut column = pos.ch;
    while column > 1 {
        let left = line[column - 2];
        let right = line[column - 1];
        if (left == '_' && right != '_')
            || (left == '-' && right != '-')
            || ((is_lower(left) || is_digit(left)) && is_upper(right))
        {
            return Ok(Pos {
                line: pos.line,
                ch: column - 1,
            });
        }
        if is_upper(left) && is_upper(right) && column + 1 < max_column {
            let right_right = line[column];
            if is_lower(right_right) || is_digit(right_right) {
                return Ok(Pos {
                    line: pos.line,
                    ch: column - 1,
                });
            }
        }
        column -= 1;
    }
    Ok(Pos {
        line: pos.line,
        ch: 0,
    })
}

/// `_moveWordPartRight`。
fn word_part_right(doc: &Doc<'_>, pos: Pos) -> Result<Pos> {
    let chars = doc.line(pos.line)?;
    let line = &chars.chars;
    let max_column = line.len() + 1;
    if pos.ch + 1 == max_column {
        return Ok(if pos.line + 1 < doc.count {
            Pos {
                line: pos.line + 1,
                ch: 0,
            }
        } else {
            pos
        });
    }
    let mut column = pos.ch + 2;
    while column < max_column {
        let left = line[column - 2];
        let right = line[column - 1];
        if (left != '_' && right == '_')
            || (left != '-' && right == '-')
            || ((is_lower(left) || is_digit(left)) && is_upper(right))
        {
            return Ok(Pos {
                line: pos.line,
                ch: column - 1,
            });
        }
        if is_upper(left) && is_upper(right) && column + 1 < max_column {
            let right_right = line[column];
            if is_lower(right_right) || is_digit(right_right) {
                return Ok(Pos {
                    line: pos.line,
                    ch: column - 1,
                });
            }
        }
        column += 1;
    }
    Ok(Pos {
        line: pos.line,
        ch: line.len(),
    })
}

/// 光标移动类命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveWord {
    Left,
    StartLeft,
    EndLeft,
    Right,
    StartRight,
    EndRight,
    AccessibilityLeft,
    AccessibilityRight,
    PartLeft,
    PartRight,
}

/// 计算一个光标按 `command` 移动后的位置。`has_multicursor` 只影响 `cursorWordLeft`
/// （多光标时不跳过单个分隔符，避免各光标停在不同的地方）。
pub fn move_word(
    text: &dyn TextSource,
    classifier: &WordClassifier,
    position: Position,
    command: MoveWord,
    has_multicursor: bool,
) -> Result<Position> {
    let doc = Doc::new(text)?;
    let (pos, _) = doc.to_pos(position)?;
    let accessibility = WordClassifier::default();
    let target = match command {
        MoveWord::Left => move_word_left_pos(
            &doc,
            classifier,
            pos,
            WordNavigation::WordStartFast,
            has_multicursor,
        )?,
        MoveWord::StartLeft => move_word_left_pos(
            &doc,
            classifier,
            pos,
            WordNavigation::WordStart,
            has_multicursor,
        )?,
        MoveWord::EndLeft => move_word_left_pos(
            &doc,
            classifier,
            pos,
            WordNavigation::WordEnd,
            has_multicursor,
        )?,
        MoveWord::Right | MoveWord::EndRight => {
            move_word_right_pos(&doc, classifier, pos, WordNavigation::WordEnd)?
        }
        MoveWord::StartRight => {
            move_word_right_pos(&doc, classifier, pos, WordNavigation::WordStart)?
        }
        MoveWord::AccessibilityLeft => move_word_left_pos(
            &doc,
            &accessibility,
            pos,
            WordNavigation::WordAccessibility,
            has_multicursor,
        )?,
        MoveWord::AccessibilityRight => {
            move_word_right_pos(&doc, &accessibility, pos, WordNavigation::WordAccessibility)?
        }
        MoveWord::PartLeft => {
            // `WordPartOperations.moveWordPartLeft`：三个候选里取最靠右的。
            let candidates = [
                move_word_left_pos(
                    &doc,
                    classifier,
                    pos,
                    WordNavigation::WordStart,
                    has_multicursor,
                )?,
                move_word_left_pos(
                    &doc,
                    classifier,
                    pos,
                    WordNavigation::WordEnd,
                    has_multicursor,
                )?,
                word_part_left(&doc, pos)?,
            ];
            candidates.into_iter().max().expect("三个候选")
        }
        MoveWord::PartRight => {
            let candidates = [
                move_word_right_pos(&doc, classifier, pos, WordNavigation::WordStart)?,
                move_word_right_pos(&doc, classifier, pos, WordNavigation::WordEnd)?,
                word_part_right(&doc, pos)?,
            ];
            candidates.into_iter().min().expect("三个候选")
        }
    };
    doc.to_position(target)
}

/// 删除类命令的上下文（`DeleteWordContext`）。
#[derive(Clone, Copy)]
pub struct DeleteWordContext<'a> {
    pub text: &'a dyn TextSource,
    pub classifier: &'a WordClassifier,
    /// 选区：`anchor == active` 表示没有选中内容。
    pub anchor: Position,
    pub active: Position,
    /// 光标正好夹在一对自动闭合的括号 / 引号中间且应一起删除（`editor.autoClosingDelete`）。
    /// 判定属于 R52（自动闭合），这里只执行结果：只对向左删除生效。
    pub auto_closing_pair: bool,
}

/// 删除类命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteWord {
    Left,
    StartLeft,
    EndLeft,
    Right,
    StartRight,
    EndRight,
    PartLeft,
    PartRight,
}

/// 计算删除区间。返回 `None` 表示不删除（文档开头向左删、文档末尾向右删）。
/// 有选中内容时总是删除选区。
pub fn delete_word(
    context: &DeleteWordContext<'_>,
    command: DeleteWord,
) -> Result<Option<TextRange>> {
    let doc = Doc::new(context.text)?;
    let (anchor, _) = doc.to_pos(context.anchor)?;
    let (active, _) = doc.to_pos(context.active)?;
    let range = match command {
        DeleteWord::Left => delete_left(
            &doc,
            context,
            anchor,
            active,
            true,
            WordNavigation::WordStart,
        )?,
        DeleteWord::StartLeft => delete_left(
            &doc,
            context,
            anchor,
            active,
            false,
            WordNavigation::WordStart,
        )?,
        DeleteWord::EndLeft => delete_left(
            &doc,
            context,
            anchor,
            active,
            false,
            WordNavigation::WordEnd,
        )?,
        DeleteWord::Right => {
            delete_right(&doc, context, anchor, active, true, WordNavigation::WordEnd)?
        }
        DeleteWord::StartRight => delete_right(
            &doc,
            context,
            anchor,
            active,
            false,
            WordNavigation::WordStart,
        )?,
        DeleteWord::EndRight => delete_right(
            &doc,
            context,
            anchor,
            active,
            false,
            WordNavigation::WordEnd,
        )?,
        DeleteWord::PartLeft => {
            // `WordPartOperations.deleteWordPartLeft`：按终点排序取最后一个。
            let mut candidates: Vec<(Pos, Pos)> = [
                delete_left(
                    &doc,
                    context,
                    anchor,
                    active,
                    true,
                    WordNavigation::WordStart,
                )?,
                delete_left(&doc, context, anchor, active, true, WordNavigation::WordEnd)?,
                Some(part_range(anchor, active, word_part_left(&doc, active)?)),
            ]
            .into_iter()
            .flatten()
            .collect();
            candidates.sort_by_key(|&(start, end)| (end, start));
            candidates.get(2).copied()
        }
        DeleteWord::PartRight => {
            let mut candidates: Vec<(Pos, Pos)> = [
                delete_right(
                    &doc,
                    context,
                    anchor,
                    active,
                    true,
                    WordNavigation::WordStart,
                )?,
                delete_right(&doc, context, anchor, active, true, WordNavigation::WordEnd)?,
                Some(part_range(anchor, active, word_part_right(&doc, active)?)),
            ]
            .into_iter()
            .flatten()
            .collect();
            candidates.sort();
            candidates.first().copied()
        }
    };
    range.map(|(a, b)| doc.range(a, b)).transpose()
}

fn ordered(a: Pos, b: Pos) -> (Pos, Pos) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// `_deleteWordPartLeft/Right`：有选区时删选区，否则从光标到目标位置。
fn part_range(anchor: Pos, active: Pos, target: Pos) -> (Pos, Pos) {
    if anchor != active {
        ordered(anchor, active)
    } else {
        ordered(active, target)
    }
}

fn is_space_or_tab(ch: char) -> bool {
    ch == ' ' || ch == '\t'
}

/// `deleteWordLeft`。
fn delete_left(
    doc: &Doc<'_>,
    context: &DeleteWordContext<'_>,
    anchor: Pos,
    position: Pos,
    whitespace_heuristics: bool,
    nav: WordNavigation,
) -> Result<Option<(Pos, Pos)>> {
    if anchor != position {
        return Ok(Some(ordered(anchor, position)));
    }
    let chars = doc.line(position.line)?;
    let line = &chars.chars;
    if context.auto_closing_pair && position.ch >= 1 && position.ch < line.len() {
        return Ok(Some((
            Pos {
                line: position.line,
                ch: position.ch - 1,
            },
            Pos {
                line: position.line,
                ch: position.ch + 1,
            },
        )));
    }
    if position.line == 0 && position.ch == 0 {
        return Ok(None);
    }
    if whitespace_heuristics {
        // `_deleteWordLeftWhitespace`：光标左侧是两个以上的空白时只删空白。
        let start_index = position.ch as i64 - 1;
        let mut last_non_whitespace: i64 = -1;
        let mut index = start_index;
        while index >= 0 {
            if !is_space_or_tab(line[index as usize]) {
                last_non_whitespace = index;
                break;
            }
            index -= 1;
        }
        if last_non_whitespace + 1 < start_index {
            return Ok(Some((
                Pos {
                    line: position.line,
                    ch: (last_non_whitespace + 1) as usize,
                },
                position,
            )));
        }
    }
    let mut previous = previous_word(line, context.classifier, position.ch);
    let mut target = Pos {
        line: position.line,
        ch: 0,
    };
    if nav == WordNavigation::WordStart {
        if let Some(word) = previous {
            target.ch = word.start;
        } else if position.ch == 0 {
            target.line -= 1;
            target.ch = doc.max(target.line)?;
        }
    } else {
        if let Some(word) = previous {
            if position.ch <= word.end {
                previous = previous_word(line, context.classifier, word.start);
            }
        }
        if let Some(word) = previous {
            target.ch = word.end;
        } else if position.ch == 0 {
            target.line -= 1;
            target.ch = doc.max(target.line)?;
        }
    }
    Ok(Some((target, position)))
}

/// `deleteWordRight`。
fn delete_right(
    doc: &Doc<'_>,
    context: &DeleteWordContext<'_>,
    anchor: Pos,
    position: Pos,
    whitespace_heuristics: bool,
    nav: WordNavigation,
) -> Result<Option<(Pos, Pos)>> {
    if anchor != position {
        return Ok(Some(ordered(anchor, position)));
    }
    let chars = doc.line(position.line)?;
    let line = &chars.chars;
    let max = line.len();
    let last_line = doc.count - 1;
    if position.line == last_line && position.ch == max {
        return Ok(None);
    }
    if whitespace_heuristics {
        // `_deleteWordRightWhitespace`：光标右侧是空白时只删空白。
        let first_non_whitespace = line[position.ch..]
            .iter()
            .position(|&ch| !is_space_or_tab(ch))
            .map_or(max, |at| position.ch + at);
        if position.ch < first_non_whitespace {
            return Ok(Some((
                position,
                Pos {
                    line: position.line,
                    ch: first_non_whitespace,
                },
            )));
        }
    }
    let classifier = context.classifier;
    let mut next = next_word(line, classifier, position.ch);
    let mut target = position;
    let fallback = |target: &mut Pos| -> Result<()> {
        if position.ch < max || position.line == last_line {
            target.ch = max;
        } else {
            target.line += 1;
            let following = doc.line(target.line)?;
            target.ch = match next_word(&following.chars, classifier, 0) {
                Some(word) => word.start,
                None => following.len(),
            };
        }
        Ok(())
    };
    if nav == WordNavigation::WordEnd {
        match next {
            Some(word) => target.ch = word.end,
            None => fallback(&mut target)?,
        }
    } else {
        if let Some(word) = next {
            if position.ch >= word.start {
                next = next_word(line, classifier, word.end);
            }
        }
        match next {
            Some(word) => target.ch = word.start,
            None => fallback(&mut target)?,
        }
    }
    Ok(Some(ordered(target, position)))
}

/// `deleteInsideWord`（`onlyWord` 为真时不连带删除相邻空白）。
pub fn delete_inside_word(
    text: &dyn TextSource,
    classifier: &WordClassifier,
    anchor: Position,
    active: Position,
    only_word: bool,
) -> Result<TextRange> {
    let doc = Doc::new(text)?;
    let (anchor, _) = doc.to_pos(anchor)?;
    let (position, chars) = doc.to_pos(active)?;
    if anchor != position {
        let (a, b) = ordered(anchor, position);
        return doc.range(a, b);
    }
    let line = &chars.chars;
    let len = line.len();
    let at = |ch: usize| Pos {
        line: position.line,
        ch,
    };
    // `_deleteInsideWordWhitespace`
    if len > 0 {
        let mut left = position.ch.saturating_sub(1);
        let mut right = position.ch.min(len - 1);
        if is_space_or_tab(line[left]) && is_space_or_tab(line[right]) {
            while left > 0 && is_space_or_tab(line[left - 1]) {
                left -= 1;
            }
            while right + 1 < len && is_space_or_tab(line[right + 1]) {
                right += 1;
            }
            return doc.range(at(left), at(right + 1));
        }
    }
    // `_deleteInsideWordDetermineDeleteRange`
    if len == 0 {
        return if position.line > 0 {
            let previous = position.line - 1;
            doc.range(
                Pos {
                    line: previous,
                    ch: doc.max(previous)?,
                },
                at(0),
            )
        } else if position.line + 1 < doc.count {
            doc.range(
                at(0),
                Pos {
                    line: position.line + 1,
                    ch: 0,
                },
            )
        } else {
            doc.range(at(0), at(0))
        };
    }
    let touches = |word: &FoundWord| word.start <= position.ch && position.ch <= word.end;
    let with_position =
        |start: usize, end: usize| doc.range(at(start.min(position.ch)), at(end.max(position.ch)));
    let delete_word_and_whitespace = |word: &FoundWord| {
        let mut start = word.start;
        let mut end = word.end;
        if only_word {
            return with_position(start, end);
        }
        let mut expanded_right = false;
        while end < len && is_space_or_tab(line[end]) {
            expanded_right = true;
            end += 1;
        }
        if !expanded_right {
            while start > 0 && is_space_or_tab(line[start - 1]) {
                start -= 1;
            }
        }
        with_position(start, end)
    };
    let previous = previous_word(line, classifier, position.ch);
    if let Some(word) = previous.filter(|w| touches(w)) {
        return delete_word_and_whitespace(&word);
    }
    let next = next_word(line, classifier, position.ch);
    if let Some(word) = next.filter(|w| touches(w)) {
        return delete_word_and_whitespace(&word);
    }
    match (previous, next) {
        (Some(p), Some(n)) => with_position(p.end, n.start),
        (Some(p), None) => with_position(p.start, p.end),
        (None, Some(n)) => with_position(n.start, n.end),
        (None, None) => with_position(0, len),
    }
}

/// `getWordAtPosition`（按分隔符）：光标接触到的普通单词，没有时 `None`。
pub fn word_at(
    text: &dyn TextSource,
    classifier: &WordClassifier,
    position: Position,
) -> Result<Option<TextRange>> {
    let doc = Doc::new(text)?;
    let (pos, chars) = doc.to_pos(position)?;
    let line = &chars.chars;
    let touches =
        |w: &FoundWord| w.kind == WordType::Regular && w.start <= pos.ch && pos.ch <= w.end;
    let found = previous_word(line, classifier, pos.ch)
        .filter(|w| touches(w))
        .or_else(|| next_word(line, classifier, pos.ch).filter(|w| touches(w)));
    found
        .map(|w| {
            doc.range(
                Pos {
                    line: pos.line,
                    ch: w.start,
                },
                Pos {
                    line: pos.line,
                    ch: w.end,
                },
            )
        })
        .transpose()
}

/// `WordOperations.word` 首次进入单词选择时的区间（双击选词）：接触到的普通单词或分隔符串，
/// 都不接触时取两词之间的空白。
pub fn select_word(
    text: &dyn TextSource,
    classifier: &WordClassifier,
    position: Position,
) -> Result<TextRange> {
    let doc = Doc::new(text)?;
    let (pos, chars) = doc.to_pos(position)?;
    let line = &chars.chars;
    let ch = pos.ch;
    let previous = previous_word(line, classifier, ch);
    let next = next_word(line, classifier, ch);
    let touching = |w: Option<FoundWord>, kind: WordType| {
        w.filter(|w| {
            w.kind == kind
                && w.start <= ch
                && if kind == WordType::Regular {
                    ch <= w.end
                } else {
                    ch < w.end
                }
        })
    };
    let (start, end) = if let Some(w) = touching(previous, WordType::Regular)
        .or_else(|| touching(previous, WordType::Separator))
        .or_else(|| touching(next, WordType::Regular))
        .or_else(|| touching(next, WordType::Separator))
    {
        (w.start, w.end)
    } else {
        (
            previous.map_or(0, |w| w.end),
            next.map_or(line.len(), |w| w.start),
        )
    };
    doc.range(
        Pos {
            line: pos.line,
            ch: start,
        },
        Pos {
            line: pos.line,
            ch: end,
        },
    )
}

/// 用语言级 `wordPattern` 取光标处的单词：`pattern` 收到整行内容与行内字节偏移，返回行内的
/// 字节区间（R12 的 `CompiledLanguageConfiguration::word_at` 正是这个签名）。
pub fn word_at_with_pattern(
    text: &dyn TextSource,
    position: Position,
    pattern: &dyn Fn(&str, ByteOffset) -> Result<Option<ByteRange>>,
) -> Result<Option<TextRange>> {
    let line = super::lines::line_text(text, position.line)?;
    if position.column.0 as usize > line.len() || !line.is_char_boundary(position.column.0 as usize)
    {
        return Err(Error::OutOfBounds);
    }
    let Some(range) = pattern(&line, ByteOffset(position.column.0))? else {
        return Ok(None);
    };
    let at = |offset: ByteOffset| Position {
        line: position.line,
        column: Utf8Column(offset.0),
    };
    Ok(Some(Span::new(at(range.start()), at(range.end()))?))
}
