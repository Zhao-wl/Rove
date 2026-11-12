//! 语言级 `wordPattern` 经编辑核心生效（#46），与 R12（#12）的用例对齐：用例与期望值取自
//! `crates/rove-syntax/tests/language_configuration.rs` 的
//! `word_pattern_agrees_with_vscode_for_markdown_and_xml`（VS Code `getWordAtText` 的实测结果），
//! 这里把同一行放在多行文档的第二行，经 `word_at_with_pattern` 换算成文档坐标。

use rove_base::{ByteOffset, Line, Position, Utf8Column};
use rove_buffer::edit::word::{self, WordClassifier};
use rove_buffer::RopeText;
use rove_syntax::language_config::BuiltinLanguages;

fn at(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

#[test]
fn markdown_and_xml_word_patterns_take_effect() {
    // (language, line text, cursor byte column, expected word)
    let cases: &[(&str, &str, u64, Option<&str>)] = &[
        ("markdown", "hello **bold** world", 10, Some("bold")),
        (
            "markdown",
            "snake_case and kebab-case",
            3,
            Some("snake_case"),
        ),
        (
            "markdown",
            "\u{4E2D}\u{6587} words 2",
            3,
            Some("\u{4E2D}\u{6587}"),
        ),
        ("markdown", "e\u{301} vowel", 0, Some("e\u{301}")),
        ("markdown", "**", 1, None),
        (
            "markdown",
            "<https://example.com/path>",
            10,
            Some("example"),
        ),
        ("xml", "<b>hello</b> 3.14 foo-bar", 5, Some("hello")),
        ("xml", "<b>hello</b> 3.14 foo-bar", 14, Some("3.14")),
        ("xml", "<b>hello</b> 3.14 foo-bar", 19, Some("foo-bar")),
        ("xml", "a.b", 1, Some("a")),
        ("xml", "version=\"1.2.3\"", 12, Some("1.2")),
    ];
    let builtins = BuiltinLanguages::load().unwrap();
    for (language, line, column, expected) in cases {
        let configuration = builtins.compiled(language).unwrap();
        let document = format!("first line\r\n{line}\nlast");
        let text = RopeText::from_text(&document);
        let pattern = |content: &str, offset: ByteOffset| configuration.word_at(content, offset);
        let range = word::word_at_with_pattern(&text, at(1, *column), &pattern).unwrap();
        let actual = range.map(|range| {
            assert_eq!(range.start().line, Line(1));
            assert_eq!(range.end().line, Line(1));
            &line[range.start().column.0 as usize..range.end().column.0 as usize]
        });
        assert_eq!(actual, *expected, "{language} {line:?} @ {column}");
    }
}

#[test]
fn word_pattern_differs_from_separator_words_where_vscode_does() {
    // XML 的 wordPattern 把 `foo-bar`、`3.14` 当作一个词；按 `editor.wordSeparators` 时
    // `-` 与 `.` 是分隔符（移动与删除用后者，与 VS Code 一致）。
    let builtins = BuiltinLanguages::load().unwrap();
    let xml = builtins.compiled("xml").unwrap();
    let text = RopeText::from_text("<b>hello</b> 3.14 foo-bar");
    let pattern = |content: &str, offset: ByteOffset| xml.word_at(content, offset);
    let by_pattern = word::word_at_with_pattern(&text, at(0, 19), &pattern)
        .unwrap()
        .unwrap();
    assert_eq!(
        (by_pattern.start(), by_pattern.end()),
        (at(0, 18), at(0, 25))
    );
    let by_separators = word::word_at(&text, &WordClassifier::default(), at(0, 19))
        .unwrap()
        .unwrap();
    assert_eq!(
        (by_separators.start(), by_separators.end()),
        (at(0, 18), at(0, 21))
    );
    // 光标不在字符边界上时报错，而不是截断。
    let cjk = RopeText::from_text("中文");
    assert!(word::word_at_with_pattern(&cjk, at(0, 1), &pattern).is_err());
}
