//! 编辑核心模型的集成测试（#46）：只走公开接口。
//!
//! 入口链路：`open_bytes` → `EditBuffer::from_loaded` → `EditSession` 编辑 → 撤销，
//! 检查变更通知（按行失效）与 tree-sitter 的 `InputEdit`。

use std::sync::Arc;
use std::time::{Duration, Instant};

use rove_base::{ByteOffset, Error, Line, Position, Span, Utf8Column};
use rove_buffer::edit::clipboard::{self, ClipboardData, MultiCursorPaste};
use rove_buffer::edit::rewrite;
use rove_buffer::edit::selection::{self, column_from_visible, grapheme_left, grapheme_right};
use rove_buffer::edit::{
    settings as edit_settings, Bias, CursorPlacement, EditBuffer, EditKind, EditOptions,
    EditSession, InputEdit, LineChange, Replacement, Selection, Selections, Transaction,
};
use rove_buffer::{
    open_bytes, DocumentMetadata, Encoding, Eol, LineEndings, LoadOptions, ReadOnlyReason,
    SizeClass, TextSource,
};
use rove_config::{DefaultSettings, Scope, SettingRegistry};

fn span(a: u64, b: u64) -> rove_base::ByteRange {
    Span::new(ByteOffset(a), ByteOffset(b)).unwrap()
}

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

fn lines(a: u64, b: u64) -> rove_base::LineRange {
    Span::new(Line(a), Line(b)).unwrap()
}

fn lf_session(text: &str) -> EditSession {
    EditSession::new(
        EditBuffer::new(text, LineEndings::Uniform(Eol::Lf)),
        EditOptions::default(),
    )
}

fn cursor(session: &mut EditSession, at: u64) {
    session.set_selections(Selections::single(Selection::cursor(ByteOffset(at))));
}

fn cursors(session: &mut EditSession, at: &[u64]) {
    session.set_selections(
        Selections::new(
            at.iter()
                .map(|&a| Selection::cursor(ByteOffset(a)))
                .collect(),
        )
        .unwrap(),
    );
}

fn offsets(session: &EditSession) -> Vec<(u64, u64)> {
    session
        .selections()
        .all()
        .iter()
        .map(|s| (s.anchor.0, s.active.0))
        .collect()
}

fn default_settings() -> DefaultSettings {
    let mut registry = SettingRegistry::default();
    rove_buffer::settings::register(&mut registry).unwrap();
    edit_settings::register(&mut registry).unwrap();
    DefaultSettings(registry)
}

// ---------------------------------------------------------------- 入口：打开 → 编辑 → 撤销

#[test]
fn open_edit_undo_reports_line_changes_and_input_edits() {
    let settings = default_settings();
    let mut options = LoadOptions::new("csharp", &settings);
    options.read_only = Arc::from([]);
    let loaded = open_bytes(b"class A\r\n{\r\n}\r\n", &options).unwrap();
    let original = loaded.text.version();
    let mut session = EditSession::new(
        EditBuffer::from_loaded(&loaded).unwrap(),
        EditOptions::read(&settings, &Scope::default()),
    );
    assert_eq!(session.buffer().version(), original);

    // 在第 1 行（`{` 之后）输入一个换行和一行代码。
    cursor(&mut session, 10);
    let applied = session
        .type_text("\n    int x;", Instant::now())
        .unwrap()
        .unwrap();
    assert_eq!(session.contents(), "class A\r\n{\r\n    int x;\r\n}\r\n");
    assert_eq!(applied.before, original);
    assert_ne!(applied.after, original);
    assert_eq!(session.snapshot().version(), applied.after);
    assert_eq!(
        applied.input_edits,
        vec![InputEdit {
            start_byte: ByteOffset(10),
            old_end_byte: ByteOffset(10),
            new_end_byte: ByteOffset(22),
            start_position: pos(1, 1),
            old_end_position: pos(1, 1),
            new_end_position: pos(2, 10),
        }]
    );
    assert_eq!(applied.changes.line_delta, 1);
    assert_eq!(
        applied.changes.ranges,
        vec![LineChange {
            old: lines(1, 2),
            new: lines(1, 3),
        }]
    );
    assert_eq!(applied.text_edits.len(), 1);
    assert_eq!(&*applied.text_edits[0].inserted, "\r\n    int x;");
    assert_eq!(applied.text_edits[0].version, original);
    // 单个编辑合并后的 TextEdit 与逐个给出的相同（交给 R09 的 DocumentHighlighter）。
    let coalesced = applied.coalesced(&*session.snapshot()).unwrap().unwrap();
    assert_eq!(coalesced.range, span(10, 10));
    assert_eq!(&*coalesced.inserted, "\r\n    int x;");

    let undone = session.undo().unwrap();
    assert_eq!(undone.len(), 1);
    assert_eq!(session.contents(), "class A\r\n{\r\n}\r\n");
    assert_eq!(undone[0].changes.line_delta, -1);
    assert_eq!(
        undone[0].changes.ranges,
        vec![LineChange {
            old: lines(1, 3),
            new: lines(1, 2),
        }]
    );
    assert_eq!(undone[0].input_edits[0].start_byte, ByteOffset(10));
    assert_eq!(undone[0].input_edits[0].old_end_byte, ByteOffset(22));
    assert_eq!(undone[0].input_edits[0].new_end_byte, ByteOffset(10));
    assert_eq!(offsets(&session), vec![(10, 10)]);
}

#[test]
fn read_only_and_huge_documents_reject_edits() {
    let settings = default_settings();
    let options = LoadOptions::new("plaintext", &settings);
    let loaded = open_bytes(b"abc", &options).unwrap();
    // 默认是会话只读：编辑返回 Unsupported，文本不变。
    let mut session = EditSession::new(
        EditBuffer::from_loaded(&loaded).unwrap(),
        EditOptions::default(),
    );
    assert_eq!(
        session.type_text("x", Instant::now()).err(),
        Some(Error::Unsupported)
    );
    assert_eq!(session.contents(), "abc");
    session.set_read_only(Arc::from([]));
    session.type_text("x", Instant::now()).unwrap();
    assert_eq!(session.contents(), "xabc");

    let metadata = DocumentMetadata {
        language: "plaintext".into(),
        encoding: Encoding::Utf8,
        bom: false,
        line_endings: LineEndings::Uniform(Eol::Lf),
        read_only: Arc::from([ReadOnlyReason::HugeFile]),
        size: SizeClass::Huge,
    };
    assert_eq!(
        EditBuffer::from_source(&*loaded.text, &metadata).err(),
        Some(Error::Unsupported)
    );
    let normal = DocumentMetadata {
        size: SizeClass::Normal,
        read_only: Arc::from([]),
        ..metadata
    };
    let buffer = EditBuffer::from_source(&*loaded.text, &normal).unwrap();
    assert_eq!(buffer.contents(), "abc");
}

// ---------------------------------------------------------------- 事务

#[test]
fn overlapping_ranges_are_rejected_not_merged() {
    let overlap = Transaction::new(vec![
        Replacement::new(span(0, 3), "x"),
        Replacement::new(span(2, 4), "y"),
    ]);
    assert_eq!(overlap.err(), Some(Error::InvalidInput));
    // 首尾相接、同一位置的多个插入都合法。
    assert!(Transaction::new(vec![
        Replacement::new(span(0, 2), "a"),
        Replacement::new(span(2, 4), "b"),
        Replacement::insert(ByteOffset(4), "c"),
        Replacement::insert(ByteOffset(4), "d"),
    ])
    .is_ok());
}

#[test]
fn later_ranges_shift_by_earlier_changes() {
    let mut buffer = EditBuffer::new("0123456789", LineEndings::Uniform(Eol::Lf));
    // 故意乱序给出：事务按位置排序。
    let transaction = Transaction::new(vec![
        Replacement::new(span(8, 9), "EIGHT"),
        Replacement::insert(ByteOffset(2), "++"),
        Replacement::delete(span(4, 6)),
        Replacement::insert(ByteOffset(2), "--"),
    ])
    .unwrap();
    let applied = buffer.apply(&transaction).unwrap();
    assert_eq!(buffer.contents(), "01++--2367EIGHT9");
    assert_eq!(
        applied.inserted_ranges(),
        &[span(2, 4), span(4, 6), span(8, 8), span(10, 15)]
    );
    // InputEdit 按应用顺序（从后往前），每个都能依次应用。
    let starts: Vec<u64> = applied.input_edits.iter().map(|e| e.start_byte.0).collect();
    assert_eq!(starts, vec![8, 4, 2, 2]);
    let mut replay = "0123456789".to_string();
    for edit in &applied.text_edits {
        let start = edit.range.start().0 as usize;
        let end = edit.range.end().0 as usize;
        replay.replace_range(start..end, &edit.inserted);
    }
    assert_eq!(replay, buffer.contents());
    // 反向事务把文本逐字节还原。
    buffer.apply(applied.inverse()).unwrap();
    assert_eq!(buffer.contents(), "0123456789");
}

#[test]
fn invalid_offsets_leave_text_untouched() {
    let mut buffer = EditBuffer::new("a中\r\nb", LineEndings::Uniform(Eol::CrLf));
    let version = buffer.version();
    for range in [span(2, 2), span(0, 99), span(5, 5)] {
        assert_eq!(
            buffer.apply(&Transaction::single(range, "x")).err(),
            Some(Error::OutOfBounds)
        );
    }
    assert_eq!(buffer.version(), version);
    assert_eq!(buffer.contents(), "a中\r\nb");
}

#[test]
fn editing_never_leaves_the_cursor_inside_a_crlf() {
    // "\rX\n" 删掉 "X" 后 \r 与 \n 并成一个换行，光标不能停在中间，否则下一次输入被拒绝。
    for left in [true, false] {
        let mut session = EditSession::new(
            EditBuffer::new("\rX\n", LineEndings::Uniform(Eol::Lf)),
            EditOptions::default(),
        );
        cursor(&mut session, if left { 2 } else { 1 });
        if left {
            session.delete_left(Instant::now()).unwrap().unwrap();
        } else {
            session.delete_right(Instant::now()).unwrap().unwrap();
        }
        assert_eq!(session.contents(), "\r\n");
        assert_eq!(offsets(&session), vec![(2, 2)]);
        session.type_text("a", Instant::now()).unwrap().unwrap();
        assert_eq!(session.contents(), "\r\na");
        assert_eq!(offsets(&session), vec![(3, 3)]);
    }
}

#[test]
fn map_offset_after_skips_every_touching_range() {
    let mut buffer = EditBuffer::new("ab", LineEndings::Uniform(Eol::Lf));
    // 同一位置的两个插入：Bias::After 落在第二个插入之后，Bias::Before 落在第一个插入之前。
    let applied = buffer
        .apply(
            &Transaction::new(vec![
                Replacement::insert(ByteOffset(1), "X"),
                Replacement::insert(ByteOffset(1), "Y"),
            ])
            .unwrap(),
        )
        .unwrap();
    assert_eq!(buffer.contents(), "aXYb");
    assert_eq!(applied.inserted_ranges(), &[span(1, 2), span(2, 3)]);
    assert_eq!(
        applied.map_offset(ByteOffset(1), Bias::After),
        ByteOffset(3)
    );
    assert_eq!(
        applied.map_offset(ByteOffset(1), Bias::Before),
        ByteOffset(1)
    );
    assert_eq!(
        applied.map_offset(ByteOffset(2), Bias::After),
        ByteOffset(4)
    );

    // 替换区间的终点上又接一个插入：同样要映射到最后一段之后。
    let mut buffer = EditBuffer::new("ab", LineEndings::Uniform(Eol::Lf));
    let applied = buffer
        .apply(
            &Transaction::new(vec![
                Replacement::new(span(0, 1), "ZZ"),
                Replacement::insert(ByteOffset(1), "Y"),
            ])
            .unwrap(),
        )
        .unwrap();
    assert_eq!(buffer.contents(), "ZZYb");
    assert_eq!(
        applied.map_offset(ByteOffset(1), Bias::After),
        ByteOffset(3)
    );

    // 但首尾相接的两个**非空**替换之间的偏移仍停在接缝处，不会被后一段插入的文本越过。
    let mut buffer = EditBuffer::new("ab", LineEndings::Uniform(Eol::Lf));
    let applied = buffer
        .apply(
            &Transaction::new(vec![
                Replacement::new(span(0, 1), "XX"),
                Replacement::new(span(1, 2), "YY"),
            ])
            .unwrap(),
        )
        .unwrap();
    assert_eq!(buffer.contents(), "XXYY");
    assert_eq!(
        applied.map_offset(ByteOffset(1), Bias::After),
        ByteOffset(2)
    );
    assert_eq!(
        applied.map_offset(ByteOffset(1), Bias::Before),
        ByteOffset(0)
    );
}

#[test]
fn whole_document_rewrites_keep_the_cursor_at_the_seam() {
    // 连续空行换行尾：每个行尾一个替换，区间首尾相接，光标不能被带到下一行。
    let mut session = lf_session("a\n\n\nb");
    cursor(&mut session, 2);
    session.change_eol(Eol::CrLf, Instant::now()).unwrap();
    assert_eq!(session.contents(), "a\r\n\r\n\r\nb");
    assert_eq!(offsets(&session), vec![(3, 3)]);

    // 全部替换（#47）同理：光标停在两个匹配之间时只跟着前一个匹配走。
    let mut session = lf_session("abab");
    cursor(&mut session, 2);
    session
        .edit(
            Transaction::new(vec![
                Replacement::new(span(0, 2), "xyz"),
                Replacement::new(span(2, 4), "xyz"),
            ])
            .unwrap(),
            Instant::now(),
        )
        .unwrap();
    assert_eq!(session.contents(), "xyzxyz");
    assert_eq!(offsets(&session), vec![(3, 3)]);
}

#[test]
fn set_selections_never_parks_a_cursor_inside_a_crlf() {
    // 点击 / 转到定义（R51、R40）直接给出的偏移也可能落在 CRLF 中间，同样要规整。
    let mut session = EditSession::new(
        EditBuffer::new("a\r\nb", LineEndings::Uniform(Eol::CrLf)),
        EditOptions::default(),
    );
    cursor(&mut session, 2);
    assert_eq!(offsets(&session), vec![(3, 3)]);
    session.type_text("X", Instant::now()).unwrap().unwrap();
    assert_eq!(session.contents(), "a\r\nXb");
}

#[test]
fn line_changes_merge_ranges_on_the_same_line() {
    let mut buffer = EditBuffer::new("aaa\nbbb\nccc\nddd", LineEndings::Uniform(Eol::Lf));
    let applied = buffer
        .apply(
            &Transaction::new(vec![
                Replacement::insert(ByteOffset(0), "1"),
                Replacement::insert(ByteOffset(2), "2\n"),
                Replacement::delete(span(11, 12)),
            ])
            .unwrap(),
        )
        .unwrap();
    assert_eq!(buffer.contents(), "1aa2\na\nbbb\ncccddd");
    assert_eq!(applied.changes.line_delta, 0);
    assert_eq!(
        applied.changes.ranges,
        vec![
            LineChange {
                old: lines(0, 1),
                new: lines(0, 2),
            },
            LineChange {
                old: lines(2, 4),
                new: lines(3, 4),
            },
        ]
    );
}

#[test]
fn mixed_line_endings_keep_every_other_terminator() {
    // 多数派是 CRLF（2 个），另有 LF 与单独 CR 各一个。
    let original = "a\r\nb\nc\rd\r\ne";
    let mut session = EditSession::new(
        EditBuffer::new(
            original,
            LineEndings::Mixed {
                preferred: Eol::CrLf,
            },
        ),
        EditOptions::default(),
    );
    cursor(&mut session, 4); // "b" 之后
    session.type_text("\n", Instant::now()).unwrap();
    let expected = "a\r\nb\r\n\nc\rd\r\ne";
    assert_eq!(session.contents().as_bytes(), expected.as_bytes());
    // 粘贴带各种换行的文本：只有插入的部分被规范化。
    cursor(&mut session, 0);
    session
        .paste(&ClipboardData::plain("x\ny\rz\r\n"), Instant::now())
        .unwrap();
    assert_eq!(
        session.contents().as_bytes(),
        format!("x\r\ny\r\nz\r\n{expected}").as_bytes()
    );
    // 撤销后逐字节回到原文。
    session.undo().unwrap();
    session.undo().unwrap();
    assert_eq!(session.contents().as_bytes(), original.as_bytes());
}

// ---------------------------------------------------------------- 撤销分组

#[test]
fn consecutive_typing_is_one_undo_unit() {
    let mut s = lf_session("");
    let t = Instant::now();
    for ch in ["h", "e", "l", "l", "o"] {
        s.type_text(ch, t).unwrap();
    }
    assert_eq!(s.contents(), "hello");
    s.undo().unwrap();
    assert_eq!(s.contents(), "");
    s.redo().unwrap();
    assert_eq!(s.contents(), "hello");
    assert_eq!(offsets(&s), vec![(5, 5)]);
}

#[test]
fn cursor_jump_breaks_the_undo_unit() {
    let mut s = lf_session("");
    let t = Instant::now();
    s.type_text("a", t).unwrap();
    s.type_text("b", t).unwrap();
    cursor(&mut s, 0);
    s.type_text("c", t).unwrap();
    assert_eq!(s.contents(), "cab");
    s.undo().unwrap();
    assert_eq!(s.contents(), "ab");
    s.undo().unwrap();
    assert_eq!(s.contents(), "");
}

#[test]
fn timeout_breaks_the_undo_unit() {
    let mut s = lf_session("");
    let t = Instant::now();
    s.type_text("a", t).unwrap();
    s.type_text("b", t + Duration::from_millis(500)).unwrap();
    s.type_text(
        "c",
        t + Duration::from_millis(500) + s.options().undo_group_timeout + Duration::from_millis(1),
    )
    .unwrap();
    s.undo().unwrap();
    assert_eq!(s.contents(), "ab");
    s.undo().unwrap();
    assert_eq!(s.contents(), "");
}

#[test]
fn typing_and_deleting_are_separate_units() {
    let mut s = lf_session("");
    let t = Instant::now();
    s.type_text("a", t).unwrap();
    s.type_text("b", t).unwrap();
    s.type_text("c", t).unwrap();
    s.delete_left(t).unwrap();
    s.delete_left(t).unwrap();
    s.type_text("x", t).unwrap();
    assert_eq!(s.contents(), "ax");
    s.undo().unwrap();
    assert_eq!(s.contents(), "a");
    s.undo().unwrap(); // 两次连续退格是一个单元
    assert_eq!(s.contents(), "abc");
    s.undo().unwrap();
    assert_eq!(s.contents(), "");
    // 向左删与向右删之间也断开。
    let mut s = lf_session("abcd");
    cursor(&mut s, 2);
    s.delete_left(t).unwrap();
    s.delete_right(t).unwrap();
    assert_eq!(s.contents(), "ad");
    s.undo().unwrap();
    assert_eq!(s.contents(), "acd");
}

#[test]
fn spaces_follow_vs_code_word_grouping() {
    // `abc |d` 不断开，`abc  |d` 断开（VS Code shouldPushStackElementBetween）。
    let mut s = lf_session("");
    let t = Instant::now();
    for ch in ["a", "b", " ", "c", " ", " ", "d"] {
        s.type_text(ch, t).unwrap();
    }
    assert_eq!(s.contents(), "ab c  d");
    s.undo().unwrap();
    assert_eq!(s.contents(), "ab c  ");
    s.undo().unwrap();
    assert_eq!(s.contents(), "ab c");
    s.undo().unwrap();
    assert_eq!(s.contents(), "ab");
    assert_eq!(
        EditKind::typing(" ", EditKind::TypingOther),
        EditKind::TypingFirstSpace
    );
}

#[test]
fn multi_range_transaction_undoes_at_once_and_restores_selections_before() {
    let mut s = lf_session("foo foo foo");
    let t = Instant::now();
    s.set_selections(
        Selections::new(vec![
            Selection::new(ByteOffset(0), ByteOffset(3)),
            Selection::new(ByteOffset(4), ByteOffset(7)),
            Selection::new(ByteOffset(11), ByteOffset(8)),
        ])
        .unwrap(),
    );
    let before = offsets(&s);
    s.type_text("bar", t).unwrap();
    s.type_text("s", t).unwrap();
    assert_eq!(s.contents(), "bars bars bars");
    assert_eq!(offsets(&s), vec![(4, 4), (9, 9), (14, 14)]);
    s.undo().unwrap();
    assert_eq!(s.contents(), "foo foo foo");
    // 撤销恢复的是编辑**之前**的选区（包括反向选区）。
    assert_eq!(offsets(&s), before);
    s.redo().unwrap();
    assert_eq!(offsets(&s), vec![(4, 4), (9, 9), (14, 14)]);

    // “全部替换”这种一次事务：一次撤销。
    let mut s = lf_session("a-b-c-d");
    let transaction = Transaction::new(
        [1, 3, 5]
            .iter()
            .map(|&at| Replacement::new(span(at, at + 1), "+"))
            .collect(),
    )
    .unwrap();
    s.edit(transaction, t).unwrap();
    assert_eq!(s.contents(), "a+b+c+d");
    assert!(s.can_undo());
    s.undo().unwrap();
    assert_eq!(s.contents(), "a-b-c-d");
    assert!(!s.can_undo());
}

#[test]
fn cursor_undo_redo_moves_only_selections() {
    let mut s = lf_session("one two three");
    cursor(&mut s, 4);
    cursor(&mut s, 8);
    s.set_selections(Selections::single(Selection::new(
        ByteOffset(8),
        ByteOffset(13),
    )));
    let version = s.buffer().version();
    assert!(s.cursor_undo());
    assert_eq!(offsets(&s), vec![(8, 8)]);
    assert!(s.cursor_undo());
    assert_eq!(offsets(&s), vec![(4, 4)]);
    assert!(s.cursor_redo());
    assert_eq!(offsets(&s), vec![(8, 8)]);
    assert_eq!(s.buffer().version(), version);
    assert_eq!(s.contents(), "one two three");
    // 文本撤销栈不受影响；编辑后光标撤销栈清空。
    assert!(!s.can_undo());
    s.type_text("x", Instant::now()).unwrap();
    assert!(!s.cursor_undo());
}

// ---------------------------------------------------------------- 选区

#[test]
fn selections_merge_overlapping_and_keep_primary() {
    let mut set = Selections::new(vec![
        Selection::new(ByteOffset(10), ByteOffset(12)), // 主选区
        Selection::new(ByteOffset(0), ByteOffset(5)),
        Selection::new(ByteOffset(4), ByteOffset(8)), // 与上一个重叠
        Selection::new(ByteOffset(12), ByteOffset(14)), // 与主选区首尾相接：保留
        Selection::cursor(ByteOffset(14)),            // 空光标接触：合并
    ])
    .unwrap();
    set.normalize(true, 10000);
    let ranges: Vec<(u64, u64)> = set.all().iter().map(|s| (s.anchor.0, s.active.0)).collect();
    assert_eq!(ranges, vec![(10, 12), (0, 8), (12, 14)]);
    assert_eq!(set.primary().anchor, ByteOffset(10));
    let sorted: Vec<u64> = set.sorted().iter().map(|s| s.start().0).collect();
    assert_eq!(sorted, vec![0, 10, 12]);

    // 关掉合并时原样保留。
    let mut kept = Selections::new(vec![
        Selection::new(ByteOffset(0), ByteOffset(5)),
        Selection::new(ByteOffset(4), ByteOffset(8)),
    ])
    .unwrap();
    kept.normalize(false, 10000);
    assert_eq!(kept.len(), 2);

    // 反向选区合并后保持主选区的方向。
    let mut reversed = Selections::new(vec![
        Selection::new(ByteOffset(6), ByteOffset(2)),
        Selection::new(ByteOffset(5), ByteOffset(9)),
        Selection::cursor(ByteOffset(20)),
    ])
    .unwrap();
    reversed.normalize(true, 10000);
    assert_eq!(
        reversed.all()[0],
        Selection::new(ByteOffset(9), ByteOffset(2))
    );
}

#[test]
fn selections_are_truncated_to_the_limit() {
    let mut set = Selections::new(
        (0..10)
            .map(|i| Selection::cursor(ByteOffset(i * 2)))
            .collect(),
    )
    .unwrap();
    set.normalize(true, 3);
    let kept: Vec<u64> = set.all().iter().map(|s| s.active.0).collect();
    assert_eq!(kept, vec![0, 2, 4]);
    let mut s = EditSession::new(
        EditBuffer::new("abcdefghij", LineEndings::Uniform(Eol::Lf)),
        EditOptions {
            multi_cursor_limit: 2,
            ..EditOptions::default()
        },
    );
    cursors(&mut s, &[1, 3, 5]);
    assert_eq!(s.selections().len(), 2);
}

#[test]
fn desired_column_survives_short_lines() {
    let text = rove_buffer::RopeText::from_text("0123456789\nab\n\t\n中文中文中文\n0123456789");
    let start = Selection::cursor(ByteOffset(7));
    let down = selection::move_vertical(&text, &start, 1, 4, false).unwrap();
    assert_eq!(text.position(down.active).unwrap(), pos(1, 2)); // 短行：停在行尾
    assert_eq!(down.desired_column, Some(7));
    let down = selection::move_vertical(&text, &down, 1, 4, false).unwrap();
    assert_eq!(text.position(down.active).unwrap(), pos(2, 1)); // Tab 占 4 列
    let down = selection::move_vertical(&text, &down, 1, 4, false).unwrap();
    // 全角字符每个两列：可见列 7 离 6（3 个字符后）与 8（4 个字符后）一样近，取左边。
    assert_eq!(text.position(down.active).unwrap(), pos(3, 9));
    let down = selection::move_vertical(&text, &down, 1, 4, false).unwrap();
    assert_eq!(text.position(down.active).unwrap(), pos(4, 7)); // 回到原列
                                                                // 最后一行再向下：到行尾并清空期望列；向上越过第一行：到行首。
    let end = selection::move_vertical(&text, &down, 1, 4, true).unwrap();
    assert_eq!(text.position(end.active).unwrap(), pos(4, 10));
    assert_eq!(end.anchor, down.active);
    assert_eq!(end.desired_column, None);
    let top = selection::move_vertical(&text, &start, -3, 4, false).unwrap();
    assert_eq!(top.active, ByteOffset(0));
    assert_eq!(column_from_visible("\tx", 3, 4), Utf8Column(1));
}

#[test]
fn horizontal_moves_step_over_whole_characters() {
    // CJK（3 字节）、emoji（4 字节）、e + 组合重音（3 字节）、CRLF。
    let text = rove_buffer::RopeText::from_text("中😀e\u{301}\r\nx");
    let mut at = ByteOffset(0);
    let mut stops = vec![];
    for _ in 0..6 {
        at = grapheme_right(&text, at).unwrap();
        stops.push(at.0);
    }
    assert_eq!(stops, vec![3, 7, 10, 12, 13, 13]);
    let mut back = vec![];
    for _ in 0..6 {
        at = grapheme_left(&text, at).unwrap();
        back.push(at.0);
    }
    assert_eq!(back, vec![12, 10, 7, 3, 0, 0]);
    // 退格删除整个字素簇。
    let mut s = lf_session("ae\u{301}中");
    cursor(&mut s, 7);
    s.delete_left(Instant::now()).unwrap();
    assert_eq!(s.contents(), "ae\u{301}");
    s.delete_left(Instant::now()).unwrap();
    assert_eq!(s.contents(), "a");
}

// ---------------------------------------------------------------- 剪贴板

#[test]
fn empty_selection_clipboard_true_copies_and_pastes_whole_lines() {
    let mut s = lf_session("first\nsecond\nthird");
    cursor(&mut s, 8); // second 中间
    let data = s.copy().unwrap().unwrap();
    assert_eq!(data.text, "second\n");
    assert!(data.from_empty_selection);
    // 粘贴到另一行：整行插在当前行之前，光标留在原来的文字上。
    cursor(&mut s, 15); // third 的 "ir" 之间
    s.paste(&data, Instant::now()).unwrap();
    assert_eq!(s.contents(), "first\nsecond\nsecond\nthird");
    assert_eq!(s.primary_position().unwrap(), pos(3, 2));
    // 剪切整行。
    cursor(&mut s, 0);
    let cut = s.cut(Instant::now()).unwrap().unwrap();
    assert_eq!(cut.text, "first\n");
    assert_eq!(s.contents(), "second\nsecond\nthird");
    // 剪切最后一行：连带删除它前面的换行。
    cursor(&mut s, 16);
    s.cut(Instant::now()).unwrap();
    assert_eq!(s.contents(), "second\nsecond");
}

#[test]
fn empty_selection_clipboard_false_copies_nothing_without_selection() {
    let mut s = EditSession::new(
        EditBuffer::new("first\nsecond", LineEndings::Uniform(Eol::Lf)),
        EditOptions {
            empty_selection_clipboard: false,
            ..EditOptions::default()
        },
    );
    cursor(&mut s, 2);
    assert_eq!(s.copy().unwrap(), None);
    assert_eq!(s.cut(Instant::now()).unwrap(), None);
    assert_eq!(s.contents(), "first\nsecond");
    // 带着“整行”元数据的剪贴板内容按普通文本粘贴到光标处。
    let line = ClipboardData {
        text: "line\n".into(),
        multicursor: None,
        from_empty_selection: true,
    };
    s.paste(&line, Instant::now()).unwrap();
    assert_eq!(s.contents(), "filine\nrst\nsecond");
}

#[test]
fn copy_keeps_original_line_endings_inside_selections() {
    let text = rove_buffer::RopeText::from_text("a\r\nb\nc");
    let all = Selections::single(Selection::new(ByteOffset(0), ByteOffset(6)));
    let data = clipboard::copy(&text, &all, true, Eol::Lf)
        .unwrap()
        .unwrap();
    assert_eq!(data.text, "a\r\nb\nc");
}

#[test]
fn multi_cursor_copy_and_paste() {
    let mut s = lf_session("alpha beta gamma\n1\n2\n3");
    s.set_selections(
        Selections::new(vec![
            Selection::new(ByteOffset(11), ByteOffset(16)), // gamma（主选区，最后选中也一样）
            Selection::new(ByteOffset(0), ByteOffset(5)),   // alpha
            Selection::new(ByteOffset(6), ByteOffset(10)),  // beta
        ])
        .unwrap(),
    );
    let data = s.copy().unwrap().unwrap();
    // 按位置排序拼接，分段保存在元数据里。
    assert_eq!(data.text, "alpha\nbeta\ngamma");
    assert_eq!(
        data.multicursor.as_deref(),
        Some(&["alpha".to_string(), "beta".into(), "gamma".into()][..])
    );
    // 光标数与分段数相同：逐个分配（按位置）。
    cursors(&mut s, &[22, 18, 20]);
    s.paste(&data, Instant::now()).unwrap();
    assert_eq!(s.contents(), "alpha beta gamma\n1alpha\n2beta\n3gamma");

    // 外部纯文本：spread 按行分配，full 每个光标粘贴全文。
    for (mode, expected) in [
        (MultiCursorPaste::Spread, "xA\nyB\n"),
        (MultiCursorPaste::Full, "xA\nB\n\nyA\nB\n\n"),
    ] {
        let mut s = EditSession::new(
            EditBuffer::new("x\ny\n", LineEndings::Uniform(Eol::Lf)),
            EditOptions {
                multi_cursor_paste: mode,
                ..EditOptions::default()
            },
        );
        cursors(&mut s, &[1, 3]);
        s.paste(&ClipboardData::plain("A\nB\n"), Instant::now())
            .unwrap();
        assert_eq!(s.contents(), expected, "{mode:?}");
        s.undo().unwrap();
        assert_eq!(s.contents(), "x\ny\n");
    }
}

#[test]
fn multi_cursor_empty_selections_copy_each_line_once() {
    let text = rove_buffer::RopeText::from_text("aa\nbb\ncc");
    let set = Selections::new(vec![
        Selection::cursor(ByteOffset(1)),
        Selection::cursor(ByteOffset(2)),
        Selection::cursor(ByteOffset(7)),
    ])
    .unwrap();
    let data = clipboard::copy(&text, &set, true, Eol::Lf)
        .unwrap()
        .unwrap();
    assert_eq!(data.text, "aa\n\ncc\n");
    assert!(!data.from_empty_selection);
    let ranges = clipboard::cut_ranges(&text, &set, true).unwrap();
    assert_eq!(ranges, vec![span(0, 3), span(5, 8)]);
    let _ = CursorPlacement::Preserve;
}

// ---------------------------------------------------------------- 改写事务

#[test]
fn indentation_conversion_touches_only_leading_whitespace() {
    let original = "\tif (a)\t// x\n  \t  b;\n\n   \t\n    four\n        eight  \t";
    let mut s = lf_session(original);
    s.convert_indentation(true, Instant::now()).unwrap();
    assert_eq!(
        s.contents(),
        "    if (a)\t// x\n        b;\n\n       \n    four\n        eight  \t"
    );
    s.undo().unwrap();
    assert_eq!(s.contents(), original);

    let to_tabs =
        rewrite::indentation_transaction(&rove_buffer::RopeText::from_text(original), 4, false)
            .unwrap();
    let mut buffer = EditBuffer::new(original, LineEndings::Uniform(Eol::Lf));
    buffer.apply(&to_tabs).unwrap();
    // 空格转 Tab：前导空白里每 4 个连续空格换一个 Tab，余数保留，行内对齐不动。
    assert_eq!(
        buffer.contents(),
        "\tif (a)\t// x\n  \t  b;\n\n   \t\n\tfour\n\t\teight  \t"
    );
}

#[test]
fn eol_unification_is_one_transaction_and_one_undo() {
    let original = "a\r\nb\nc\r\r\n\nd";
    let mut s = EditSession::new(
        EditBuffer::new(
            original,
            LineEndings::Mixed {
                preferred: Eol::CrLf,
            },
        ),
        EditOptions::default(),
    );
    let applied = s.change_eol(Eol::Lf, Instant::now()).unwrap().unwrap();
    assert_eq!(s.contents(), "a\nb\nc\n\n\nd");
    assert_eq!(applied.changes.line_delta, 0);
    assert!(matches!(
        s.buffer().line_endings(),
        LineEndings::Uniform(Eol::Lf)
    ));
    // 之后新输入的换行用 LF。
    cursor(&mut s, 1);
    s.type_text("\r\n", Instant::now()).unwrap();
    assert_eq!(s.contents(), "a\n\nb\nc\n\n\nd");
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.contents().as_bytes(), original.as_bytes());
    assert!(matches!(
        s.buffer().line_endings(),
        LineEndings::Mixed {
            preferred: Eol::CrLf
        }
    ));
    let to_crlf =
        rewrite::eol_transaction(&rove_buffer::RopeText::from_text("x\ny\r\nz"), Eol::CrLf)
            .unwrap();
    assert_eq!(to_crlf.edits().len(), 1);
}

// ---------------------------------------------------------------- 设置

#[test]
fn options_are_read_through_the_settings_contract() {
    let settings = default_settings();
    let options = EditOptions::read(&settings, &Scope::default());
    assert_eq!(options, EditOptions::default());
    // 已被其他模块登记的键跳过，不报 Duplicate。
    let mut registry = SettingRegistry::default();
    edit_settings::register(&mut registry).unwrap();
    edit_settings::register(&mut registry).unwrap();
}
