//! 随机编辑序列的撤销 / 重做往返（#46 验收）：对随机文本（含 CJK、emoji、组合字符、CRLF、
//! 单独 CR）施加随机的多区间事务、输入与删除，全部撤销后与初始文本**逐字节相同**，
//! 再全部重做后与撤销前逐字节相同；每一步的 `TextEdit` 依次重放也得到同样的文本。

use std::time::{Duration, Instant};

use proptest::prelude::*;
use rove_base::{ByteOffset, Span};
use rove_buffer::edit::{
    EditBuffer, EditOptions, EditSession, Replacement, Selection, Selections, Transaction,
};
use rove_buffer::{Eol, LineEndings, TextSource};

/// 文本片段：覆盖多字节字符与各种换行符。
const PIECES: &[&str] = &[
    "a", "Z", " ", "\t", "中", "文", "😀", "👍🏽", "e\u{301}", "\r\n", "\n", "\r", "_", "{", "}",
];

fn text_strategy(max: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..max).prop_map(|v| v.concat())
}

/// 一次操作：多区间事务（区间用 0..1 的比例表示，执行时换算成合法位置）、输入、删除或撤销一步。
#[derive(Clone, Debug)]
enum Op {
    Transaction(Vec<(f64, f64, String)>),
    Type(f64, String),
    DeleteLeft(f64),
    DeleteRight(f64),
    Undo,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => prop::collection::vec((0.0..1.0f64, 0.0..0.2f64, text_strategy(4)), 1..5)
            .prop_map(Op::Transaction),
        3 => (0.0..1.0f64, text_strategy(3)).prop_map(|(at, t)| Op::Type(at, t)),
        1 => (0.0..1.0f64).prop_map(Op::DeleteLeft),
        1 => (0.0..1.0f64).prop_map(Op::DeleteRight),
        1 => Just(Op::Undo),
    ]
}

/// 把比例换算成不落在字符中间、也不落在 CRLF 中间的偏移。
fn boundary(text: &str, ratio: f64) -> u64 {
    let mut at = ((text.len() as f64) * ratio) as usize;
    at = at.min(text.len());
    while at > 0
        && (!text.is_char_boundary(at)
            || (text.as_bytes()[at - 1] == b'\r' && text.as_bytes().get(at) == Some(&b'\n')))
    {
        at -= 1;
    }
    at as u64
}

/// 任何一步之后都不允许有端点落在 `\r\n` 中间：那样的偏移会让下一次普通输入被判为越界。
fn selection_endpoints_are_legal(session: &EditSession, text: &str) -> bool {
    let bytes = text.as_bytes();
    session.selections().all().iter().all(|s| {
        [s.anchor.0, s.active.0].iter().all(|&at| {
            let at = at as usize;
            at <= bytes.len()
                && text.is_char_boundary(at)
                && !(at > 0 && bytes[at - 1] == b'\r' && bytes.get(at) == Some(&b'\n'))
        })
    })
}

fn full_text(text: &dyn TextSource) -> String {
    let mut out = String::new();
    text.visit_chunks(
        Span::new(ByteOffset(0), ByteOffset(text.byte_len())).unwrap(),
        &mut |c| out.push_str(c),
    )
    .unwrap();
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, .. ProptestConfig::default() })]

    #[test]
    fn undo_all_then_redo_all_round_trips(
        initial in text_strategy(40),
        eol in prop::sample::select(&[Eol::Lf, Eol::CrLf, Eol::Cr][..]),
        ops in prop::collection::vec(op_strategy(), 1..30),
    ) {
        let mut session = EditSession::new(
            EditBuffer::new(&initial, LineEndings::Mixed { preferred: eol }),
            EditOptions::default(),
        );
        let start = Instant::now();
        for (step, op) in ops.iter().enumerate() {
            // 时间每步前进，偶尔超过分组超时。
            let now = start + Duration::from_millis(step as u64 * 400);
            let text = session.contents();
            let before = session.snapshot();
            let result = match op {
                Op::Transaction(edits) => {
                    let mut ranges: Vec<(u64, u64, String)> = edits
                        .iter()
                        .map(|(at, len, t)| {
                            let a = boundary(&text, *at);
                            let b = boundary(&text, (*at + *len).min(1.0)).max(a);
                            (a, b, t.clone())
                        })
                        .collect();
                    ranges.sort_by_key(|r| (r.0, r.1));
                    // 去掉重叠：保留不与前一个重叠的区间（相接与同点插入保留）。
                    let mut kept: Vec<(u64, u64, String)> = Vec::new();
                    for r in ranges {
                        if kept.last().is_none_or(|p| r.0 >= p.1) {
                            kept.push(r);
                        }
                    }
                    let transaction = Transaction::new(
                        kept.into_iter()
                            .map(|(a, b, t)| {
                                Replacement::new(Span::new(ByteOffset(a), ByteOffset(b)).unwrap(), t)
                            })
                            .collect(),
                    )
                    .unwrap();
                    session.edit(transaction, now).map(|a| vec![a])
                }
                Op::Type(at, t) => {
                    let at = ByteOffset(boundary(&text, *at));
                    session.set_selections(Selections::single(Selection::cursor(at)));
                    session.type_text(t, now).map(|a| a.into_iter().collect())
                }
                Op::DeleteLeft(at) | Op::DeleteRight(at) => {
                    let at = ByteOffset(boundary(&text, *at));
                    session.set_selections(Selections::single(Selection::cursor(at)));
                    if matches!(op, Op::DeleteLeft(_)) {
                        session.delete_left(now)
                    } else {
                        session.delete_right(now)
                    }
                    .map(|a| a.into_iter().collect())
                }
                Op::Undo => session.undo(),
            };
            let applied = result.expect("合法编辑不应失败");
            // TextEdit 依次重放与缓冲一致，InputEdit 的字节区间与 TextEdit 对应。
            let mut replay = full_text(&*before);
            for edit in &applied {
                for (text_edit, input) in edit.text_edits.iter().zip(&edit.input_edits) {
                    prop_assert_eq!(text_edit.range.start(), input.start_byte);
                    prop_assert_eq!(text_edit.range.end(), input.old_end_byte);
                    prop_assert_eq!(
                        input.new_end_byte.0,
                        input.start_byte.0 + text_edit.inserted.len() as u64
                    );
                    let s = text_edit.range.start().0 as usize;
                    let e = text_edit.range.end().0 as usize;
                    replay.replace_range(s..e, &text_edit.inserted);
                }
            }
            let current = session.contents();
            prop_assert_eq!(replay.as_bytes(), current.as_bytes());
            prop_assert_eq!(full_text(&*session.snapshot()), session.contents());
            // 选区端点必须始终合法（#46）：字符边界上、且不在 `\r\n` 中间，否则下一次普通
            // 输入会被 `EditBuffer::apply` 判为越界。这里不真的再输入一次，以免干扰撤销分组。
            prop_assert!(selection_endpoints_are_legal(&session, &current));
        }

        let edited = session.contents();
        let mut undo_steps = 0;
        while session.can_undo() {
            session.undo().unwrap();
            undo_steps += 1;
            prop_assert!(undo_steps < 1000);
        }
        let undone = session.contents();
        prop_assert_eq!(undone.as_bytes(), initial.as_bytes());
        // 序列末尾的撤销操作也在重做栈里：只重做这里撤销掉的步数。
        for _ in 0..undo_steps {
            session.redo().unwrap();
        }
        let redone = session.contents();
        prop_assert_eq!(redone.as_bytes(), edited.as_bytes());
    }
}
