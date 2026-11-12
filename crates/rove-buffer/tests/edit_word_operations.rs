//! VS Code 单词边界用例对拍（#46）：逐例翻译自 VS Code 1.145.2 的
//! `src/vs/editor/contrib/wordOperations/test/browser/wordOperations.test.ts` 与
//! `src/vs/editor/contrib/wordPartOperations/test/browser/wordPartOperations.test.ts`（MIT）。
//!
//! 用例文本、初始位置与期望值保持原样；`|` 标记的期望停靠点经 `deserialize` / `serialize`
//! 换算（VS Code 的列是 1 起的 UTF-16 列，这里是 0 起的 UTF-8 列，文本相同时等价）。
//!
//! 没有翻译的两例：`cursorWordLeft/Right - Recognize words`（`wordSegmenterLocales: 'ja'`
//! 依赖浏览器 `Intl.Segmenter` 的日文词典分词，本移植不支持该设置，等同默认值 `[]`；
//! 对应的 “Does not recognize words” 两例已翻译）。

use std::time::{Duration, Instant};

use rove_base::{Line, Position, Span, Utf8Column};
use rove_buffer::edit::word::{self, DeleteWordContext};
use rove_buffer::edit::{
    DeleteWord, EditBuffer, EditOptions, EditSession, MoveWord, Selection, Selections,
};
use rove_buffer::{Eol, LineEndings, TextSource};

fn session_with(text: &str, separators: Option<&str>) -> EditSession {
    let mut options = EditOptions::default();
    if let Some(separators) = separators {
        options.word_separators = separators.into();
    }
    EditSession::new(
        EditBuffer::new(text, LineEndings::Uniform(Eol::Lf)),
        options,
    )
}

fn session(lines: &[&str]) -> EditSession {
    session_with(&lines.join("\n"), None)
}

fn line_content(s: &EditSession, line: u64) -> String {
    let text = s.snapshot();
    let range = text.line_range(Line(line)).unwrap();
    let mut out = String::new();
    text.visit_chunks(range, &mut |c| out.push_str(c)).unwrap();
    out
}

/// VS Code 的 1 起 (行, 列) → Position；像 `setPosition` 一样钳到文档范围内。
fn vs(s: &EditSession, line: u64, column: u64) -> Position {
    let count = s.snapshot().lines().known;
    let line = (line - 1).min(count - 1);
    let content = line_content(s, line);
    let chars: Vec<usize> = content.char_indices().map(|(at, _)| at).collect();
    let index = (column - 1) as usize;
    let byte = chars.get(index).copied().unwrap_or(content.len());
    Position {
        line: Line(line),
        column: Utf8Column(byte as u64),
    }
}

fn offset(s: &EditSession, p: Position) -> rove_base::ByteOffset {
    s.snapshot().offset(p).unwrap()
}

fn set_position(s: &mut EditSession, p: Position) {
    let at = offset(s, p);
    s.set_selections(Selections::single(Selection::cursor(at)));
}

fn set_selection(s: &mut EditSession, anchor: Position, active: Position) {
    let selection = Selection::new(offset(s, anchor), offset(s, active));
    s.set_selections(Selections::single(selection));
}

fn position(s: &EditSession) -> Position {
    s.primary_position().unwrap()
}

/// (anchor, active)
fn selection(s: &EditSession) -> (Position, Position) {
    let text = s.snapshot();
    let primary = s.selections().primary();
    (
        text.position(primary.anchor).unwrap(),
        text.position(primary.active).unwrap(),
    )
}

fn now() -> Instant {
    Instant::now()
}

fn deserialize(text: &str) -> (String, Vec<Position>) {
    let mut result = String::new();
    let mut line = 0;
    let mut column = 0;
    let mut positions = Vec::new();
    for ch in text.chars() {
        if ch == '\n' {
            result.push(ch);
            line += 1;
            column = 0;
        } else if ch == '|' {
            positions.push(Position {
                line: Line(line),
                column: Utf8Column(column),
            });
        } else {
            result.push(ch);
            column += ch.len_utf8() as u64;
        }
    }
    (result, positions)
}

fn serialize(text: &str, mut positions: Vec<Position>) -> String {
    positions.sort();
    positions.reverse();
    let mut result = String::new();
    let mut line = 0;
    let mut column = 0;
    let mut take = |line: u64, column: u64, result: &mut String| {
        // 与 VS Code 一样每个字符前只输出一个停靠点；重复的停靠点会剩下并触发断言。
        if positions
            .last()
            .is_some_and(|p| p.line.0 == line && p.column.0 == column)
        {
            result.push('|');
            positions.pop();
        }
    };
    for ch in text.chars() {
        take(line, column, &mut result);
        result.push(ch);
        if ch == '\n' {
            line += 1;
            column = 0;
        } else {
            column += ch.len_utf8() as u64;
        }
    }
    take(line, column, &mut result);
    assert!(positions.is_empty(), "Unexpected left over positions!!!");
    result
}

/// `testRepeatedActionAndExtractPositions`。
fn repeated(
    text: &str,
    separators: Option<&str>,
    initial: (u64, u64),
    mut action: impl FnMut(&mut EditSession),
    record: impl Fn(&EditSession) -> Position,
    stop: impl Fn(&EditSession) -> bool,
) -> Vec<Position> {
    let mut s = session_with(text, separators);
    let start = vs(&s, initial.0, initial.1);
    set_position(&mut s, start);
    let mut stops = Vec::new();
    loop {
        action(&mut s);
        stops.push(record(&s));
        if stop(&s) {
            break;
        }
        assert!(
            stops.len() <= 1000,
            "Endless loop detected involving position {:?}!",
            position(&s)
        );
    }
    stops
}

fn at(line: u64, column: u64) -> impl Fn(&EditSession) -> bool {
    move |s| {
        let expected = vs(s, line, column);
        position(s) == expected
    }
}

fn moves(command: MoveWord, select: bool) -> impl FnMut(&mut EditSession) {
    move |s| s.move_word(command, select).unwrap()
}

fn deletes(command: DeleteWord) -> impl FnMut(&mut EditSession) {
    move |s| {
        s.delete_word(command, now()).unwrap();
    }
}

fn empty(s: &EditSession) -> bool {
    s.contents().is_empty()
}

/// 用光标前已删除的字符数记录位置（`new Position(1, text.length - value.length + 1)`）。
fn deleted_prefix(original: usize) -> impl Fn(&EditSession) -> Position {
    move |s| Position {
        line: Line(0),
        column: Utf8Column((original - s.contents().len()) as u64),
    }
}

fn check_moves(
    expected: &str,
    initial: (u64, u64),
    command: MoveWord,
    select: bool,
    stop: (u64, u64),
) {
    check_moves_with(expected, None, initial, command, select, stop)
}

fn check_moves_with(
    expected: &str,
    separators: Option<&str>,
    initial: (u64, u64),
    command: MoveWord,
    select: bool,
    stop: (u64, u64),
) {
    let (text, _) = deserialize(expected);
    let stops = repeated(
        &text,
        separators,
        initial,
        moves(command, select),
        position,
        at(stop.0, stop.1),
    );
    assert_eq!(serialize(&text, stops), expected);
}

fn check_deletes_left(expected: &str, separators: Option<&str>, command: DeleteWord) {
    let (text, _) = deserialize(expected);
    let stops = repeated(
        &text,
        separators,
        (1000, 10000),
        deletes(command),
        position,
        empty,
    );
    assert_eq!(serialize(&text, stops), expected);
}

fn check_deletes_right(expected: &str, separators: Option<&str>, command: DeleteWord) {
    let (text, _) = deserialize(expected);
    let stops = repeated(
        &text,
        separators,
        (1, 1),
        deletes(command),
        deleted_prefix(text.len()),
        empty,
    );
    assert_eq!(serialize(&text, stops), expected);
}

const LINES: &[&str] = &[
    "    \tMy First Line\t ",
    "\tMy Second Line",
    "    Third Line🐶",
    "",
    "1",
];

// ---------------------------------------------------------------- wordOperations.test.ts

#[test]
fn cursor_word_left_simple() {
    let expected = [
        "|    \t|My |First |Line\t ",
        "|\t|My |Second |Line",
        "|    |Third |Line🐶",
        "|",
        "|1",
    ]
    .join("\n");
    check_moves(&expected, (1000, 1000), MoveWord::Left, false, (1, 1));
}

#[test]
fn cursor_word_left_with_selection() {
    let mut s = session(LINES);
    let p = vs(&s, 5, 2);
    set_position(&mut s, p);
    s.move_word(MoveWord::Left, true).unwrap();
    assert_eq!(selection(&s), (vs(&s, 5, 2), vs(&s, 5, 1)));
}

#[test]
fn cursor_word_left_issue_832() {
    let expected = "|   |/* |Just |some   |more   |text |a|+= |3 |+|5-|3 |+ |7 |*/  ";
    check_moves(expected, (1000, 1000), MoveWord::Left, false, (1, 1));
}

#[test]
fn cursor_word_left_issue_48046_word_selection_doesnt_work_as_usual() {
    let expected = "|deep.|object.|property";
    check_moves(expected, (1, 21), MoveWord::Left, false, (1, 1));
}

#[test]
fn cursor_word_left_does_not_recognize_words() {
    let expected = "|/* |これはテストです |/*";
    check_moves(expected, (1000, 1000), MoveWord::Left, true, (1, 1));
}

#[test]
fn cursor_word_left_issue_169904_cursors_out_of_sync() {
    let text = [
        ".grid1 {",
        "  display: grid;",
        "  grid-template-columns:",
        "    [full-start] minmax(1em, 1fr)",
        "    [main-start] minmax(0, 40em) [main-end]",
        "    minmax(1em, 1fr) [full-end];",
        "}",
        ".grid2 {",
        "  display: grid;",
        "  grid-template-columns:",
        "    [full-start] minmax(1em, 1fr)",
        "    [main-start] minmax(0, 40em) [main-end] minmax(1em, 1fr) [full-end];",
        "}",
    ];
    let mut s = session(&text);
    let cursors = [(5, 44), (6, 32), (12, 44), (12, 72)];
    let items = cursors
        .iter()
        .map(|&(l, c)| Selection::cursor(offset(&s, vs(&s, l, c))))
        .collect();
    s.set_selections(Selections::new(items).unwrap());
    s.move_word(MoveWord::Left, false).unwrap();
    let snapshot = s.snapshot();
    let actual: Vec<Position> = s
        .selections()
        .all()
        .iter()
        .map(|sel| snapshot.position(sel.active).unwrap())
        .collect();
    let expected: Vec<Position> = [(5, 43), (6, 31), (12, 43), (12, 71)]
        .iter()
        .map(|&(l, c)| vs(&s, l, c))
        .collect();
    assert_eq!(actual, expected);
    assert!(s.selections().all().iter().all(|sel| sel.is_empty()));
}

#[test]
fn cursor_word_left_select_issue_74369_consistent_behaviour() {
    let expected = "|this.|is.|a.|test";
    check_moves(expected, (1, 15), MoveWord::Left, true, (1, 1));
}

#[test]
fn cursor_word_start_left() {
    // This is the behaviour observed in Visual Studio, please do not touch test
    let expected = "|   |/* |Just |some   |more   |text |a|+= |3 |+|5|-|3 |+ |7 |*/  ";
    check_moves(expected, (1000, 1000), MoveWord::StartLeft, false, (1, 1));
}

#[test]
fn cursor_word_start_left_issue_51119_vs_compatibility() {
    let expected = "|this|.|is|.|a|.|test";
    check_moves(expected, (1000, 1000), MoveWord::StartLeft, false, (1, 1));
}

#[test]
fn issue_51275_cursor_word_start_left_does_not_push_undo_redo_stack_element() {
    let mut s = session(&[""]);
    let t = now();
    for ch in "foo bar baz".chars() {
        s.type_text(&ch.to_string(), t).unwrap();
    }
    assert_eq!(s.contents(), "foo bar baz");
    s.move_word(MoveWord::StartLeft, false).unwrap();
    s.move_word(MoveWord::StartLeft, false).unwrap();
    s.type_text("q", t).unwrap();
    assert_eq!(s.contents(), "foo qbar baz");
    s.undo().unwrap();
    assert_eq!(s.contents(), "foo bar baz");
}

#[test]
fn cursor_word_end_left() {
    let expected = "|   /*| Just| some|   more|   text| a|+=| 3| +|5|-|3| +| 7| */|  ";
    check_moves(expected, (1000, 1000), MoveWord::EndLeft, false, (1, 1));
}

#[test]
fn cursor_word_right_simple() {
    let expected = [
        "    \tMy| First| Line|\t |",
        "\tMy| Second| Line|",
        "    Third| Line🐶|",
        "|",
        "1|",
    ]
    .join("\n");
    check_moves(&expected, (1, 1), MoveWord::Right, false, (5, 2));
}

#[test]
fn cursor_word_right_selection() {
    let mut s = session(LINES);
    let p = vs(&s, 1, 1);
    set_position(&mut s, p);
    s.move_word(MoveWord::Right, true).unwrap();
    assert_eq!(selection(&s), (vs(&s, 1, 1), vs(&s, 1, 8)));
}

#[test]
fn cursor_word_right_issue_832() {
    let expected = "   /*| Just| some|   more|   text| a|+=| 3| +5|-3| +| 7| */|  |";
    check_moves(expected, (1, 1), MoveWord::Right, false, (1, 50));
}

#[test]
fn cursor_word_right_issue_41199() {
    let expected = "console|.log|(err|)|";
    check_moves(expected, (1, 1), MoveWord::Right, false, (1, 17));
}

#[test]
fn cursor_word_right_does_not_recognize_words() {
    let expected = "/*| これはテストです|/*|";
    check_moves(expected, (1, 1), MoveWord::Right, false, (1, 14));
}

#[test]
fn move_word_end_right() {
    let expected = "   /*| Just| some|   more|   text| a|+=| 3| +5|-3| +| 7| */|  |";
    check_moves(expected, (1, 1), MoveWord::EndRight, false, (1, 50));
}

#[test]
fn move_word_start_right() {
    // This is the behaviour observed in Visual Studio, please do not touch test
    let expected = "   |/* |Just |some   |more   |text |a|+= |3 |+|5|-|3 |+ |7 |*/  |";
    check_moves(expected, (1, 1), MoveWord::StartRight, false, (1, 50));
}

#[test]
fn issue_51119_cursor_word_start_right_regression() {
    let expected = "this|.|is|.|a|.|test|";
    check_moves(expected, (1, 1), MoveWord::StartRight, false, (1, 15));
}

#[test]
fn issue_64810_cursor_word_start_right_skips_first_word_after_newline() {
    let expected = ["Hello |World|", "|Hei |mailman|"].join("\n");
    check_moves(&expected, (1, 1), MoveWord::StartRight, false, (2, 12));
}

#[test]
fn cursor_word_accessibility_left() {
    let expected = "|   /* |Just |some   |more   |text |a+= |3 +|5-|3 + |7 */  ";
    // VS Code 的测试辅助函数在这里把 Select 与非 Select 版本对调了，停靠点相同。
    check_moves(
        expected,
        (1000, 1000),
        MoveWord::AccessibilityLeft,
        true,
        (1, 1),
    );
}

#[test]
fn cursor_word_accessibility_right() {
    let expected = "   /* |Just |some   |more   |text |a+= |3 +|5-|3 + |7 */  |";
    check_moves(
        expected,
        (1, 1),
        MoveWord::AccessibilityRight,
        true,
        (1, 50),
    );
}

fn delete_case(
    command: DeleteWord,
    setup: impl FnOnce(&mut EditSession),
    line: u64,
    expected_line: &str,
    expected_position: (u64, u64),
) {
    let mut s = session(LINES);
    setup(&mut s);
    s.delete_word(command, now()).unwrap();
    assert_eq!(line_content(&s, line - 1), expected_line);
    assert_eq!(
        position(&s),
        vs(&s, expected_position.0, expected_position.1)
    );
}

fn select_3_7_to_3_9(s: &mut EditSession) {
    let (a, b) = (vs(s, 3, 7), vs(s, 3, 9));
    set_selection(s, a, b);
}

fn place(line: u64, column: u64) -> impl FnOnce(&mut EditSession) {
    move |s| {
        let p = vs(s, line, column);
        set_position(s, p);
    }
}

#[test]
fn delete_word_left_for_non_empty_selection() {
    delete_case(
        DeleteWord::Left,
        select_3_7_to_3_9,
        3,
        "    Thd Line🐶",
        (3, 7),
    );
}

#[test]
fn delete_word_left_for_cursor_at_beginning_of_document() {
    delete_case(
        DeleteWord::Left,
        place(1, 1),
        1,
        "    \tMy First Line\t ",
        (1, 1),
    );
}

#[test]
fn delete_word_left_for_cursor_at_end_of_whitespace() {
    delete_case(DeleteWord::Left, place(3, 11), 3, "    Line🐶", (3, 5));
}

#[test]
fn delete_word_left_for_cursor_just_behind_a_word() {
    delete_case(DeleteWord::Left, place(2, 11), 2, "\tMy  Line", (2, 5));
}

#[test]
fn delete_word_left_for_cursor_inside_of_a_word() {
    delete_case(
        DeleteWord::Left,
        place(1, 12),
        1,
        "    \tMy st Line\t ",
        (1, 9),
    );
}

#[test]
fn delete_word_right_for_non_empty_selection() {
    delete_case(
        DeleteWord::Right,
        select_3_7_to_3_9,
        3,
        "    Thd Line🐶",
        (3, 7),
    );
}

#[test]
fn delete_word_right_for_cursor_at_end_of_document() {
    delete_case(DeleteWord::Right, place(5, 3), 5, "1", (5, 2));
}

#[test]
fn delete_word_right_for_cursor_at_beggining_of_whitespace() {
    delete_case(DeleteWord::Right, place(3, 1), 3, "Third Line🐶", (3, 1));
}

#[test]
fn delete_word_right_for_cursor_just_before_a_word() {
    delete_case(DeleteWord::Right, place(2, 5), 2, "\tMy  Line", (2, 5));
}

#[test]
fn delete_word_right_for_cursor_inside_of_a_word() {
    delete_case(
        DeleteWord::Right,
        place(1, 11),
        1,
        "    \tMy Fi Line\t ",
        (1, 11),
    );
}

#[test]
fn delete_word_left_issue_832() {
    check_deletes_left(
        "|   |/* |Just |some |text |a|+= |3 |+|5 |*/|  ",
        None,
        DeleteWord::Left,
    );
}

#[test]
fn delete_word_start_left() {
    check_deletes_left(
        "|   |/* |Just |some |text |a|+= |3 |+|5 |*/  ",
        None,
        DeleteWord::StartLeft,
    );
}

#[test]
fn delete_word_end_left() {
    check_deletes_left(
        "|   /*| Just| some| text| a|+=| 3| +|5| */|  ",
        None,
        DeleteWord::EndLeft,
    );
}

#[test]
fn delete_word_left_issue_24947() {
    for command in [DeleteWord::Left, DeleteWord::StartLeft, DeleteWord::EndLeft] {
        let mut s = session(&["{", "}"]);
        let p = vs(&s, 2, 1);
        set_position(&mut s, p);
        s.delete_word(command, now()).unwrap();
        assert_eq!(line_content(&s, 0), "{}", "{command:?}");
    }
}

#[test]
fn delete_word_right_issue_832() {
    check_deletes_right(
        "   |/*| |Just| |some| |text| |a|+=| |3| |+|5|-|3| |*/|  |",
        None,
        DeleteWord::Right,
    );
}

#[test]
fn delete_word_right_issue_3882() {
    for command in [
        DeleteWord::Right,
        DeleteWord::StartRight,
        DeleteWord::EndRight,
    ] {
        let mut s = session(&["public void Add( int x,", "                 int y )"]);
        let p = vs(&s, 1, 24);
        set_position(&mut s, p);
        s.delete_word(command, now()).unwrap();
        assert_eq!(
            line_content(&s, 0),
            "public void Add( int x,int y )",
            "001 {command:?}"
        );
    }
}

#[test]
fn delete_word_start_right() {
    check_deletes_right(
        "   |/* |Just |some |text |a|+= |3 |+|5|-|3 |*/  |",
        None,
        DeleteWord::StartRight,
    );
}

#[test]
fn delete_word_end_right() {
    check_deletes_right(
        "   /*| Just| some| text| a|+=| 3| +|5|-|3| */|  |",
        None,
        DeleteWord::EndRight,
    );
}

#[test]
fn delete_word_right_issue_3882_1_ctrl_delete_at_end_of_line() {
    let mut s = session(&["A line with text.", "   And another one"]);
    let p = vs(&s, 1, 18);
    set_position(&mut s, p);
    s.delete_word(DeleteWord::Right, now()).unwrap();
    assert_eq!(
        line_content(&s, 0),
        "A line with text.And another one",
        "001"
    );
}

#[test]
fn delete_word_left_issue_3882_2_ctrl_delete_at_end_of_line() {
    let mut s = session(&["A line with text.", "   And another one"]);
    let p = vs(&s, 2, 1);
    set_position(&mut s, p);
    s.delete_word(DeleteWord::Left, now()).unwrap();
    assert_eq!(
        line_content(&s, 0),
        "A line with text.   And another one",
        "001"
    );
}

#[test]
fn delete_word_left_issue_91855_matching_pair_gets_deleted() {
    // 自动闭合对的判定属于 R52；这里按 VS Code 判定的结果（光标夹在自动闭合的 "" 之间、
    // `autoClosingDelete: 'always'`）传入 `auto_closing_pair`。
    let mut s = session(&["a \"\""]);
    let p = vs(&s, 1, 4);
    set_position(&mut s, p);
    let snapshot = s.snapshot();
    let context = DeleteWordContext {
        text: &*snapshot,
        classifier: s.classifier(),
        anchor: p,
        active: p,
        auto_closing_pair: true,
    };
    let range = word::delete_word(&context, DeleteWord::Left)
        .unwrap()
        .unwrap();
    let range = Span::new(
        snapshot.offset(range.start()).unwrap(),
        snapshot.offset(range.end()).unwrap(),
    )
    .unwrap();
    s.edit(rove_buffer::edit::Transaction::single(range, ""), now())
        .unwrap();
    assert_eq!(line_content(&s, 0), "a ");
}

fn inside(lines: &[&str], at: (u64, u64), only_word: bool, expected: &[&str]) {
    let mut s = session(lines);
    let p = vs(&s, at.0, at.1);
    set_position(&mut s, p);
    for value in expected {
        s.delete_inside_word(only_word, now()).unwrap();
        assert_eq!(s.contents(), *value);
    }
}

#[test]
fn delete_inside_word_empty_line() {
    inside(&["Line1", "", "Line2"], (2, 1), false, &["Line1\nLine2"]);
}

#[test]
fn delete_inside_word_in_whitespace_1() {
    inside(&["Just  some text."], (1, 6), false, &["Justsome text."]);
}

#[test]
fn delete_inside_word_in_whitespace_2() {
    inside(&["Just     some text."], (1, 6), false, &["Justsome text."]);
}

#[test]
fn delete_inside_word_in_whitespace_3() {
    inside(
        &["Just     \"some text."],
        (1, 6),
        false,
        &[
            "Just\"some text.",
            "\"some text.",
            "some text.",
            "text.",
            ".",
            "",
            "",
        ],
    );
}

#[test]
fn delete_inside_word_in_non_words() {
    inside(
        &["x=3+4+5+6"],
        (1, 7),
        false,
        &["x=3+45+6", "x=3++6", "x=36", "x=", "x", "", ""],
    );
}

#[test]
fn delete_inside_word_in_words_1() {
    inside(
        &["This is interesting"],
        (1, 7),
        false,
        &["This interesting", "This", "", ""],
    );
}

#[test]
fn delete_inside_word_in_words_2() {
    inside(
        &["This  is  interesting"],
        (1, 7),
        false,
        &["This  interesting", "This", "", ""],
    );
}

#[test]
fn delete_inside_word_only_word_does_not_delete_whitespace_before_last_word() {
    inside(&["hello world"], (1, 9), true, &["hello "]);
}

#[test]
fn delete_inside_word_only_word_deletes_just_the_word() {
    inside(
        &["This is interesting"],
        (1, 7),
        true,
        &["This  interesting"],
    );
}

// ---------------------------------------------------------------- wordPartOperations.test.ts

/// 去掉 `$-%~` 再加上 `·` 的分隔符（issue #158667）。
const KEBAB_SEPARATORS: &str = "!\"#&'()*+,./:;<=>?@[\\]^`{|}·";

#[test]
fn cursor_word_part_left_basic() {
    let expected = [
        "|start| |line|",
        "|this|Is|A|Camel|Case|Var|  |this_|is_|a_|snake_|case_|var| |THIS_|IS_|CAPS_|SNAKE| |this_|IS|Mixed|Use|",
        "|end| |line",
    ]
    .join("\n");
    check_moves(&expected, (1000, 1000), MoveWord::PartLeft, false, (1, 1));
}

#[test]
fn cursor_word_part_left_issue_53899_whitespace() {
    let expected = "|myvar| |=| |'|demonstration|     |of| |selection| |with| |space|'";
    check_moves(expected, (1000, 1000), MoveWord::PartLeft, false, (1, 1));
}

#[test]
fn cursor_word_part_left_issue_53899_underscores() {
    let expected = "|myvar| |=| |'|demonstration_____|of| |selection| |with| |space|'";
    check_moves(expected, (1000, 1000), MoveWord::PartLeft, false, (1, 1));
}

#[test]
fn cursor_word_part_right_basic() {
    let expected = [
        "start| |line|",
        "|this|Is|A|Camel|Case|Var|  |this|_is|_a|_snake|_case|_var| |THIS|_IS|_CAPS|_SNAKE| |this|_IS|Mixed|Use|",
        "|end| |line|",
    ]
    .join("\n");
    check_moves(&expected, (1, 1), MoveWord::PartRight, false, (3, 9));
}

#[test]
fn cursor_word_part_right_issue_53899_whitespace() {
    let expected = "myvar| |=| |'|demonstration|     |of| |selection| |with| |space|'|";
    check_moves(expected, (1, 1), MoveWord::PartRight, false, (1, 52));
}

#[test]
fn cursor_word_part_right_issue_53899_underscores() {
    let expected = "myvar| |=| |'|demonstration|_____of| |selection| |with| |space|'|";
    check_moves(expected, (1, 1), MoveWord::PartRight, false, (1, 52));
}

#[test]
fn cursor_word_part_right_issue_53899_second_case() {
    let expected = [
        ";| |--| |1|",
        "|;|        |--| |2|",
        "|;|    |#|3|",
        "|;|   |#|4|",
    ]
    .join("\n");
    check_moves(&expected, (1, 1), MoveWord::PartRight, false, (4, 7));
}

#[test]
fn issue_93239_cursor_word_part_right() {
    check_moves("foo|_bar|", (1, 1), MoveWord::PartRight, false, (1, 8));
}

#[test]
fn issue_93239_cursor_word_part_left() {
    check_moves("|foo_|bar", (1, 8), MoveWord::PartLeft, false, (1, 1));
}

#[test]
fn delete_word_part_left_basic() {
    check_deletes_left(
        "|   |/*| |Just| |some| |text| |a|+=| |3| |+|5|-|3| |*/|  |this|Is|A|Camel|Case|Var|  |this_|is_|a_|snake_|case_|var| |THIS_|IS_|CAPS_|SNAKE| |this_|IS|Mixed|Use",
        None,
        DeleteWord::PartLeft,
    );
}

#[test]
fn delete_word_part_right_basic() {
    check_deletes_right(
        "   |/*| |Just| |some| |text| |a|+=| |3| |+|5|-|3| |*/|  |this|Is|A|Camel|Case|Var|  |this|_is|_a|_snake|_case|_var| |THIS|_IS|_CAPS|_SNAKE| |this|_IS|Mixed|Use|",
        None,
        DeleteWord::PartRight,
    );
}

#[test]
fn issue_158667_cursor_word_part_left_stops_at_dash() {
    check_moves_with(
        "|this-|is-|a-|kebab-|case-|var| |THIS-|IS-|CAPS-|KEBAB| |this-|IS|Mixed|Use",
        Some(KEBAB_SEPARATORS),
        (1000, 1000),
        MoveWord::PartLeft,
        false,
        (1, 1),
    );
}

#[test]
fn issue_158667_cursor_word_part_right_stops_at_dash() {
    check_moves_with(
        "this|-is|-a|-kebab|-case|-var| |THIS|-IS|-CAPS|-KEBAB| |this|-IS|Mixed|Use|",
        Some(KEBAB_SEPARATORS),
        (1, 1),
        MoveWord::PartRight,
        false,
        (1, 60),
    );
}

#[test]
fn issue_158667_delete_word_part_left_stops_at_dash() {
    check_deletes_left(
        "|this-|is-|a-|kebab-|case-|var| |THIS-|IS-|CAPS-|KEBAB| |this-|IS|Mixed|Use",
        Some(KEBAB_SEPARATORS),
        DeleteWord::PartLeft,
    );
}

#[test]
fn issue_158667_delete_word_part_right_stops_at_dash() {
    check_deletes_right(
        "this|-is|-a|-kebab|-case|-var| |THIS|-IS|-CAPS|-KEBAB| |this|-IS|Mixed|Use|",
        Some(KEBAB_SEPARATORS),
        DeleteWord::PartRight,
    );
}

// ---------------------------------------------------------------- 本移植的补充

#[test]
fn word_at_and_select_word_follow_separators() {
    let s = session(&["deep.object.property  x"]);
    let text = s.snapshot();
    let classifier = s.classifier();
    let p = vs(&s, 1, 8);
    let word = word::word_at(&*text, classifier, p).unwrap().unwrap();
    assert_eq!((word.start(), word.end()), (vs(&s, 1, 6), vs(&s, 1, 12)));
    // 两个空白之间：取两词之间的空白。
    let gap = word::select_word(&*text, classifier, vs(&s, 1, 22)).unwrap();
    assert_eq!((gap.start(), gap.end()), (vs(&s, 1, 21), vs(&s, 1, 23)));
    // 紧挨在词前：取后面的词。
    let x = word::select_word(&*text, classifier, vs(&s, 1, 23)).unwrap();
    assert_eq!((x.start(), x.end()), (vs(&s, 1, 23), vs(&s, 1, 24)));
    // 紧挨在词后：取前面的词。
    let dot = word::select_word(&*text, classifier, vs(&s, 1, 5)).unwrap();
    assert_eq!((dot.start(), dot.end()), (vs(&s, 1, 1), vs(&s, 1, 5)));
}

#[test]
fn undo_group_timeout_is_respected_by_word_tests_clock() {
    // 输入间隔超过超时：分成两个撤销单元（这里只确认测试用的时钟不会意外合并）。
    let mut s = session(&[""]);
    let t = now();
    s.type_text("a", t).unwrap();
    s.type_text("b", t + Duration::from_secs(5)).unwrap();
    s.undo().unwrap();
    assert_eq!(s.contents(), "a");
}
