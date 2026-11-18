//! #57 第一部分的测试：括号对树、按语言的着色快照、注释 / 字符串的跳过、
//! 未配对括号、设置开关、增量更新的正确性与复用率、括号数上限。
//!
//! 「忽略区间」在这里是手工标注的：真实运行时它来自 #9 的 tree-sitter token（scope），
//! 测试里直接给出等价的结果，这样不依赖语法解析也能把 C# 的逐字字符串、XML 的 CDATA 覆盖到。

use super::parser::LineEdit;
use super::query::{BracketHit, MatchBrackets};
use super::*;
use rove_syntax::language_config::BuiltinLanguages;
use rove_syntax::LanguageConfiguration;
use std::sync::Arc;

// ------------------------------------------------------------------ 辅助

fn language(id: &str) -> Arc<LanguageConfiguration> {
    use rove_syntax::LanguageConfigurations;
    BuiltinLanguages::shared()
        .0
        .get(id)
        .unwrap_or_else(|| panic!("缺少语言配置 {id}"))
}

fn pairs(list: &[(&str, &str)]) -> Vec<(Arc<str>, Arc<str>)> {
    list.iter()
        .map(|(o, c)| (Arc::from(*o), Arc::from(*c)))
        .collect()
}

/// 造一个已经解析好的 [`BracketPairs`]。
fn build(kinds: Vec<BracketKind>, doc: &TextDocument) -> BracketPairs {
    let mut state = BracketPairs::default();
    state.set_kinds(kinds);
    state.update(doc);
    state
}

fn for_language(id: &str, doc: &TextDocument) -> BracketPairs {
    let config = language(id);
    build(kinds_for(Some(&config), None, None), doc)
}

/// 一行的着色快照：(列区间, 深度, 颜色键)。
fn snapshot(state: &BracketPairs, line: u64) -> Vec<(std::ops::Range<u32>, u32, &'static str)> {
    state.colored_keys(line)
}

/// 整篇文档的括号（按行），用来比较增量更新与整篇重算。
fn all_brackets(state: &BracketPairs, lines: u64) -> Vec<(u64, BracketHit)> {
    (0..lines)
        .flat_map(|l| {
            state
                .brackets_in_line(l)
                .into_iter()
                .map(move |hit| (l, hit))
        })
        .collect()
}

/// 固定种子的 xorshift，保证测试可复现。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

// ------------------------------------------------------------------ 四种语言的着色快照

#[test]
fn csharp_snapshot_has_depths_and_color_keys() {
    // C# 的 colorizedBracketPairs 比 brackets 多一对 `<>`（泛型），两者取并集建树。
    let doc = TextDocument::new("void M(List<int> a) { if (a[0] == 1) { } }");
    let state = for_language("csharp", &doc);
    let line = snapshot(&state, 0);
    let keys: Vec<(u32, u32, &str)> = line
        .iter()
        .map(|(r, level, key)| (r.start, *level, *key))
        .collect();
    assert_eq!(
        keys,
        vec![
            // M(
            (6, 1, "editorBracketHighlight.foreground1"),
            // List<
            (11, 2, "editorBracketHighlight.foreground2"),
            // >
            (15, 2, "editorBracketHighlight.foreground2"),
            // )
            (18, 1, "editorBracketHighlight.foreground1"),
            // {
            (20, 1, "editorBracketHighlight.foreground1"),
            // if (
            (25, 2, "editorBracketHighlight.foreground2"),
            // a[
            (27, 3, "editorBracketHighlight.foreground3"),
            // ]
            (29, 3, "editorBracketHighlight.foreground3"),
            // )
            (35, 2, "editorBracketHighlight.foreground2"),
            // {
            (37, 2, "editorBracketHighlight.foreground2"),
            // }
            (39, 2, "editorBracketHighlight.foreground2"),
            // }
            (41, 1, "editorBracketHighlight.foreground1"),
        ],
        "{line:?}"
    );
}

#[test]
fn csharp_depth_cycles_back_to_the_first_color_after_six() {
    let doc = TextDocument::new("(((((((x)))))))");
    let state = for_language("csharp", &doc);
    let keys: Vec<&str> = snapshot(&state, 0).into_iter().map(|(_, _, k)| k).collect();
    assert_eq!(keys[0], "editorBracketHighlight.foreground1");
    assert_eq!(keys[5], "editorBracketHighlight.foreground6");
    // 第 7 层回到第 1 个颜色。
    assert_eq!(keys[6], "editorBracketHighlight.foreground1");
}

#[test]
fn json_snapshot_falls_back_to_brackets() {
    // JSON 没有定义 colorizedBracketPairs，回落到 brackets（`{}` 与 `[]`）。
    assert!(language("json").colorized_brackets.is_none());
    let doc = TextDocument::new("{\"a\": [1, {\"b\": 2}]}");
    let state = for_language("json", &doc);
    let keys: Vec<(u32, u32, &str)> = snapshot(&state, 0)
        .iter()
        .map(|(r, level, key)| (r.start, *level, *key))
        .collect();
    assert_eq!(
        keys,
        vec![
            (0, 1, "editorBracketHighlight.foreground1"),
            (6, 2, "editorBracketHighlight.foreground2"),
            (10, 3, "editorBracketHighlight.foreground3"),
            (17, 3, "editorBracketHighlight.foreground3"),
            (18, 2, "editorBracketHighlight.foreground2"),
            (19, 1, "editorBracketHighlight.foreground1"),
        ]
    );
}

#[test]
fn xml_and_markdown_do_not_colorize_by_default() {
    // 这两份语言配置把 colorizedBracketPairs 定义成空表，所以一个括号都不着色
    // （与 VS Code 一致）；但 brackets 仍然用来做括号匹配。
    for id in ["xml", "markdown"] {
        assert!(
            language(id)
                .colorized_brackets
                .as_deref()
                .is_some_and(<[rove_syntax::Pair]>::is_empty),
            "{id} 的 colorizedBracketPairs 应当是空表"
        );
    }

    let doc = TextDocument::new("<root><item a=\"1\"/></root>");
    let state = for_language("xml", &doc);
    assert!(snapshot(&state, 0).is_empty(), "XML 默认不着色");
    assert!(
        !state.brackets_in_line(0).is_empty(),
        "树照样建起来了，活动括号对要用"
    );

    let doc = TextDocument::new("[link](http://x) and [a[b]c]");
    let state = for_language("markdown", &doc);
    assert!(snapshot(&state, 0).is_empty(), "Markdown 默认不着色");
    assert!(!state.brackets_in_line(0).is_empty());
}

#[test]
fn xml_snapshot_with_user_override() {
    // 用户用 editor.language.colorizedBracketPairs 把 `<>` 打开。
    let kinds = kinds_for(
        Some(&*language("xml")),
        None,
        Some(&pairs(&[("<", ">"), ("{", "}"), ("[", "]")])),
    );
    // `[` / `]` 不在 XML 的 brackets 里：只着色、不参与匹配，与 C# 的 `<>` 同理。
    assert!(kinds
        .iter()
        .any(|k| &*k.open == "[" && !k.matchable && k.colorized));
    let doc = TextDocument::new("<a b=\"{c:[1]}\">");
    let state = build(kinds, &doc);
    let keys: Vec<(u32, u32, &str)> = snapshot(&state, 0)
        .iter()
        .map(|(r, level, key)| (r.start, *level, *key))
        .collect();
    assert_eq!(
        keys,
        vec![
            (0, 1, "editorBracketHighlight.foreground1"),
            (6, 2, "editorBracketHighlight.foreground2"),
            (9, 3, "editorBracketHighlight.foreground3"),
            (11, 3, "editorBracketHighlight.foreground3"),
            (12, 2, "editorBracketHighlight.foreground2"),
            (14, 1, "editorBracketHighlight.foreground1"),
        ]
    );
}

#[test]
fn markdown_snapshot_with_user_override() {
    let kinds = kinds_for(
        Some(&*language("markdown")),
        None,
        Some(&pairs(&[("[", "]"), ("(", ")")])),
    );
    let doc = TextDocument::new("[a [b] c](u)");
    let state = build(kinds, &doc);
    let keys: Vec<(u32, u32, &str)> = snapshot(&state, 0)
        .iter()
        .map(|(r, level, key)| (r.start, *level, *key))
        .collect();
    assert_eq!(
        keys,
        vec![
            (0, 1, "editorBracketHighlight.foreground1"),
            (3, 2, "editorBracketHighlight.foreground2"),
            (5, 2, "editorBracketHighlight.foreground2"),
            (8, 1, "editorBracketHighlight.foreground1"),
            (9, 1, "editorBracketHighlight.foreground1"),
            (11, 1, "editorBracketHighlight.foreground1"),
        ]
    );
}

#[test]
fn yaml_falls_back_to_brackets() {
    // YAML 没有 colorizedBracketPairs，必须回落到 brackets 的三种括号。
    assert!(language("yaml").colorized_brackets.is_none());
    let kinds = kinds_for(Some(&*language("yaml")), None, None);
    assert_eq!(kinds.len(), 3);
    assert!(kinds.iter().all(|k| k.colorized && k.matchable));
    let doc = TextDocument::new("a: [1, {b: (2)}]");
    let state = build(kinds, &doc);
    let levels: Vec<u32> = snapshot(&state, 0).iter().map(|(_, l, _)| *l).collect();
    assert_eq!(levels, vec![1, 2, 3, 3, 2, 1]);
}

// ------------------------------------------------------------------ 注释与字符串

#[test]
fn brackets_in_comments_and_strings_are_ignored() {
    // 第 1 行：行注释里的括号；第 2 行：普通字符串里的括号。
    let doc = TextDocument::new("void M() { // ) } \n    var s = \"a { b\";\n}")
        .ignore(0, 11..18)
        .ignore(1, 12..19);
    let state = for_language("csharp", &doc);
    // 注释里的 `)` 与 `}` 不出现。
    let line0: Vec<u32> = state
        .brackets_in_line(0)
        .iter()
        .map(|h| h.range.start)
        .collect();
    assert_eq!(line0, vec![6, 7, 9]);
    assert!(state.brackets_in_line(1).is_empty(), "字符串里的 `{{` 不算");
    // 第 2 行的 `}` 与第 0 行的 `{` 正常配对，深度 1。
    let line2 = state.brackets_in_line(2);
    assert_eq!(line2.len(), 1);
    assert_eq!(line2[0].level, 1);
    assert!(!line2[0].unmatched);
}

#[test]
fn csharp_verbatim_string_is_ignored() {
    // 逐字字符串里可以有换行与未配对的括号；是否在字符串里完全由 #9 的 token 决定，
    // 这里按 tree-sitter 会给出的范围标注。
    let doc = TextDocument::new("var s = @\"a { b \"\" ) \";\nM();")
        .ignore(0, 8..22)
        .ignore(1, 0..0);
    let state = for_language("csharp", &doc);
    assert!(
        state.brackets_in_line(0).is_empty(),
        "逐字字符串里的 `{{` 与 `)` 都不参与配对"
    );
    let line1 = state.brackets_in_line(1);
    assert_eq!(line1.len(), 2);
    assert!(line1.iter().all(|h| !h.unmatched && h.level == 1));
}

#[test]
fn xml_cdata_is_ignored() {
    // CDATA 在 TextMate 语法里是 `string.unquoted.cdata.xml`，整段按字符串处理。
    let text = "<a><![CDATA[ <b> ]]></a>";
    let cdata = text.find("<![CDATA[").expect("有 CDATA") as u32;
    let end = text.find("]]>").expect("有结尾") as u32 + 3;
    let doc = TextDocument::new(text).ignore(0, cdata..end);
    let kinds = kinds_for(Some(&*language("xml")), None, Some(&pairs(&[("<", ">")])));
    let state = build(kinds, &doc);
    let keys: Vec<(u32, u32)> = snapshot(&state, 0)
        .iter()
        .map(|(r, level, _)| (r.start, *level))
        .collect();
    assert_eq!(
        keys,
        vec![(0, 1), (2, 1), (20, 1), (23, 1)],
        "只剩 `<a>` 与 `</a>`"
    );
}

// ------------------------------------------------------------------ 未配对的括号

#[test]
fn extra_closing_bracket_is_unexpected() {
    let doc = TextDocument::new("f(a))");
    let state = for_language("csharp", &doc);
    let hits = state.brackets_in_line(0);
    assert_eq!(hits.len(), 3);
    assert!(!hits[0].unmatched && !hits[1].unmatched);
    assert!(hits[2].unmatched, "多余的右括号");
    assert_eq!(
        color_key(&hits[2], false),
        "editorBracketHighlight.unexpectedBracket.foreground"
    );
}

#[test]
fn missing_closing_bracket_is_unexpected() {
    let doc = TextDocument::new("f(a");
    let state = for_language("csharp", &doc);
    let hits = state.brackets_in_line(0);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].unmatched, "缺右括号的左括号");
    assert_eq!(
        color_key(&hits[0], false),
        "editorBracketHighlight.unexpectedBracket.foreground"
    );
}

#[test]
fn a_closing_bracket_of_another_kind_does_not_match() {
    let doc = TextDocument::new("(a]b)");
    let state = for_language("csharp", &doc);
    let hits = state.brackets_in_line(0);
    assert_eq!(hits.len(), 3);
    assert!(!hits[0].unmatched, "`(` 配上了最后的 `)`");
    assert!(hits[1].unmatched, "`]` 没有对应的 `[`");
    assert!(!hits[2].unmatched);
}

// ------------------------------------------------------------------ 设置开关

#[test]
fn colorization_can_be_turned_off() {
    let doc = TextDocument::new("{[()]}");
    let mut state = for_language("csharp", &doc);
    state.colors = BracketColors {
        depth: [Some(rove_theme::Color(1, 2, 3, 255)); DEPTH_COLORS],
        unexpected: Some(rove_theme::Color(9, 9, 9, 255)),
    };
    let mut out = Vec::new();
    state.colorize(0, &mut out);
    assert_eq!(out.len(), 6);

    state.enabled = false;
    out.clear();
    state.colorize(0, &mut out);
    assert!(out.is_empty(), "enabled = false 时完全不着色");
    // 关掉着色之后树还在：活动括号对照常工作。
    state.update(&doc);
    assert!(state.has_tree());
    assert!(state.active_pair(0, 1, MatchBrackets::Near).is_some());
}

#[test]
fn fully_transparent_depth_colors_do_not_override_the_token_color() {
    // VS Code 的 foreground4–6 默认是 `#00000000`，含义是「不改前景色」。
    let doc = TextDocument::new("(((( ))))");
    let mut state = for_language("csharp", &doc);
    state.colors = BracketColors {
        depth: [
            Some(rove_theme::Color(1, 1, 1, 255)),
            Some(rove_theme::Color(2, 2, 2, 255)),
            Some(rove_theme::Color(3, 3, 3, 255)),
            Some(rove_theme::Color(0, 0, 0, 0)),
            None,
            None,
        ],
        unexpected: None,
    };
    let mut out = Vec::new();
    state.colorize(0, &mut out);
    assert_eq!(out.len(), 6, "第 4 层的两个括号不提交样式：{out:?}");
}

#[test]
fn independent_color_pool_per_bracket_type() {
    let doc = TextDocument::new("{[({[(x)]})]}");
    let state = for_language("csharp", &doc);
    let hits = state.brackets_in_line(0);
    let shared: Vec<u32> = hits.iter().map(|h| h.level).collect();
    assert_eq!(shared, vec![1, 2, 3, 4, 5, 6, 6, 5, 4, 3, 2, 1]);
    let per_kind: Vec<u32> = hits.iter().map(|h| h.level_of_kind).collect();
    assert_eq!(per_kind, vec![1, 1, 1, 2, 2, 2, 2, 2, 2, 1, 1, 1]);

    let shared_keys: Vec<&str> = hits.iter().map(|h| color_key(h, false)).collect();
    assert_eq!(shared_keys[5], "editorBracketHighlight.foreground6");
    let pool_keys: Vec<&str> = hits.iter().map(|h| color_key(h, true)).collect();
    assert_eq!(
        pool_keys[5], "editorBracketHighlight.foreground2",
        "每种括号各自从 1 开始数"
    );
}

// ------------------------------------------------------------------ 活动括号对

#[test]
fn active_pair_beats_a_nearest_scan_when_nesting_is_unbalanced() {
    // 就近扫描会被前面那个多余的 `)` 带偏，括号对树不会。
    let doc = TextDocument::new("a) f(b, (c))");
    let state = for_language("csharp", &doc);
    // 光标在最外层 `(` 右边。
    let m = state
        .active_pair(0, 5, MatchBrackets::Near)
        .expect("配上最后一个 `)`");
    assert_eq!((m.open.1, m.close.1), (4..5, 11..12));
    // 光标挨着多余的右括号：不高亮。
    assert!(state.active_pair(0, 2, MatchBrackets::Always).is_none());
}

#[test]
fn match_brackets_modes_keep_their_meaning() {
    let doc = TextDocument::new("f(a, (b))");
    let state = for_language("csharp", &doc);
    for mode in [MatchBrackets::Near, MatchBrackets::Always] {
        let m = state.active_pair(0, 2, mode).expect("紧邻");
        assert_eq!((m.open.1, m.close.1), (1..2, 8..9));
    }
    assert!(state.active_pair(0, 3, MatchBrackets::Near).is_none());
    let m = state
        .active_pair(0, 3, MatchBrackets::Always)
        .expect("包围");
    assert_eq!((m.open.1, m.close.1), (1..2, 8..9));
    assert!(state.active_pair(0, 2, MatchBrackets::Never).is_none());
}

#[test]
fn colorized_only_kinds_do_not_participate_in_matching() {
    // C# 的 `<>` 只着色、不参与匹配（#20 的 `brackets` 里没有它）。
    let doc = TextDocument::new("M<T>(x)");
    let state = for_language("csharp", &doc);
    assert!(
        state.brackets_in_line(0).iter().any(|h| h.range.start == 1),
        "`<` 进了树，会着色"
    );
    assert!(
        state.active_pair(0, 2, MatchBrackets::Near).is_none(),
        "`<` 不参与括号匹配"
    );
    let m = state.active_pair(0, 5, MatchBrackets::Near).expect("`(`");
    assert_eq!((m.open.1, m.close.1), (4..5, 6..7));
}

// ------------------------------------------------------------------ 增量更新

/// 整篇重算一棵树，用来和增量更新的结果比对。
fn reference(kinds: &[BracketKind], doc: &TextDocument) -> BracketPairs {
    build(kinds.to_vec(), doc)
}

#[test]
fn incremental_update_matches_a_full_reparse() {
    let kinds = kinds_for(Some(&*language("csharp")), None, None);
    let mut rng = Rng(0x5157_2024_1118);
    let fragments = ["{", "}", "(", ")", "[", "]", "a", "b ", "x(y)", "", "// c"];
    let mut lines: Vec<String> = (0..120)
        .map(|_| {
            (0..4)
                .map(|_| fragments[rng.below(fragments.len())])
                .collect::<String>()
        })
        .collect();
    let mut doc = TextDocument::new(&lines.join("\n"));
    let mut state = build(kinds.clone(), &doc);

    for step in 0..300 {
        let line = rng.below(lines.len());
        match rng.below(4) {
            // 改一行（最常见：单字符输入）。
            0 | 1 => {
                let text = format!("{}{}", lines[line], fragments[rng.below(fragments.len())]);
                lines[line] = text.clone();
                doc.set_line(line as u64, &text);
                state.note_lines(vec![LineEdit {
                    old: line as u64..line as u64 + 1,
                    new: line as u64..line as u64 + 1,
                }]);
            }
            // 插入一行。
            2 => {
                let text = fragments[rng.below(fragments.len())].to_string();
                lines.insert(line, text.clone());
                doc.insert_line(line as u64, &text);
                state.note_lines(vec![LineEdit {
                    old: line as u64..line as u64,
                    new: line as u64..line as u64 + 1,
                }]);
            }
            // 删一行。
            _ => {
                if lines.len() > 1 {
                    lines.remove(line);
                    doc.remove_line(line as u64);
                    state.note_lines(vec![LineEdit {
                        old: line as u64..line as u64 + 1,
                        new: line as u64..line as u64,
                    }]);
                }
            }
        }
        state.update(&doc);
        let expected = reference(&kinds, &doc);
        assert_eq!(
            all_brackets(&state, lines.len() as u64),
            all_brackets(&expected, lines.len() as u64),
            "第 {step} 次编辑后增量结果与整篇重算不一致"
        );
    }
}

#[test]
fn a_single_character_edit_reuses_most_of_the_tree() {
    let kinds = kinds_for(Some(&*language("csharp")), None, None);
    let mut text = String::new();
    for i in 0..2_000 {
        text.push_str(&format!("void M{i}() {{ if (a[{i}] > 0) {{ x(); }} }}\n"));
    }
    let mut doc = TextDocument::new(&text);
    let mut state = build(kinds, &doc);
    let full = state.stats();
    assert!(full.reused == 0 && full.constructed > 10_000, "{full:?}");

    doc.set_line(1_000, "void M1000() { if (a[1000] > 1) { x(); } }");
    state.note_lines(vec![LineEdit {
        old: 1_000..1_001,
        new: 1_000..1_001,
    }]);
    state.update(&doc);
    let incremental = state.stats();
    // 只重建改动那一行附近的 token，其余全部按子树复用。
    assert!(
        incremental.constructed < 100,
        "单字符编辑重建了太多节点：{incremental:?}"
    );
    assert!(incremental.reused > 0, "{incremental:?}");
}

#[test]
fn two_batches_before_an_update_fall_back_to_a_full_reparse() {
    let kinds = kinds_for(Some(&*language("csharp")), None, None);
    let mut doc = TextDocument::new("a(1)\nb(2)\nc(3)");
    let mut state = build(kinds, &doc);
    doc.set_line(0, "a((1))");
    state.note_lines(vec![LineEdit {
        old: 0..1,
        new: 0..1,
    }]);
    doc.set_line(2, "c((3))");
    state.note_lines(vec![LineEdit {
        old: 2..3,
        new: 2..3,
    }]);
    state.update(&doc);
    assert_eq!(state.stats().reused, 0, "退化成整篇重算");
    assert_eq!(state.brackets_in_line(0).len(), 4);
    assert_eq!(state.brackets_in_line(2).len(), 4);
}

// ------------------------------------------------------------------ 上限与退化保护

#[test]
fn too_many_brackets_turns_colorization_off() {
    let kinds = kinds_for(Some(&*language("json")), None, None);
    // 一行压缩 JSON：10,000 个括号。
    let mut text = String::from("[");
    for i in 0..5_000 {
        text.push_str(&format!("{{\"k\":{i}}},"));
    }
    text.push(']');
    let doc = TextDocument::new(&text);

    let mut state = BracketPairs::default();
    state.set_kinds(kinds.clone());
    state.limit = 1_000;
    state.update(&doc);
    assert!(state.over_limit(), "超过上限");
    assert!(!state.has_tree(), "整体关闭，而不是画错");
    let mut out = Vec::new();
    state.colorize(0, &mut out);
    assert!(out.is_empty());
    // 再次编辑不会反复尝试（上限是粘性的，重开文档才重新判定）。
    state.note_lines(vec![LineEdit {
        old: 0..1,
        new: 0..1,
    }]);
    assert!(!state.update(&doc));

    // 上限足够大时正常工作。
    let mut state = BracketPairs::default();
    state.set_kinds(kinds);
    state.limit = 100_000;
    state.update(&doc);
    assert!(!state.over_limit());
    assert_eq!(state.brackets_in_line(0).len(), 10_002);
}

#[test]
fn very_deep_nesting_does_not_recurse_without_bound() {
    let kinds = kinds_for(Some(&*language("json")), None, None);
    let depth = 5_000;
    let text = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
    let doc = TextDocument::new(&text);
    let state = build(kinds, &doc);
    let hits = state.brackets_in_line(0);
    // 超过 MAX_DEPTH 的括号当普通文本处理，不参与配对，所以配上对的只有前面一部分。
    assert!(!hits.is_empty() && hits.len() < depth * 2);
    let max = hits.iter().map(|h| h.level).max().unwrap_or(0);
    assert!(max <= super::ast::MAX_DEPTH, "深度被钳住：{max}");
}

/// Unity 风格的 5,000 行 C#（增量与实测用例共用）。
fn big_csharp() -> String {
    let mut text = String::from("using UnityEngine;\n\nnamespace Game.Generated\n{\n");
    for c in 0..25 {
        text.push_str(&format!(
            "    public class Component{c} : MonoBehaviour\n    {{\n"
        ));
        for f in 0..8 {
            text.push_str(&format!("        [SerializeField] private int field{f};\n"));
        }
        for m in 0..19 {
            text.push_str(&format!(
                "        public void Method{m}(int value)\n        {{\n"
            ));
            for k in 0..6 {
                text.push_str(&format!(
                    "            var step{k} = Mathf.Max(value * {k}, data[{k}]);\n"
                ));
            }
            text.push_str("        }\n\n");
        }
        text.push_str("    }\n\n");
    }
    text.push_str("}\n");
    text
}

/// 单行压缩 JSON（1 万个括号）。
fn minified_json() -> String {
    let mut json = String::from("[");
    for i in 0..5_000 {
        if i > 0 {
            json.push(',');
        }
        json.push_str(&format!("{{\"id\":{i},\"v\":[{i},{i}]}}"));
    }
    json.push(']');
    json
}

/// 括号对树的实测耗时。默认 `#[ignore]`，用 release 跑：
///
/// ```text
/// cargo test -p rove-ui --lib --release -j 1 bracket_pairs::tests::timing -- --ignored --nocapture
/// ```
///
/// 与 `xtask/benches/brackets.rs` 同口径（criterion 版本在那里），这一条是不跑 criterion
/// 也能拿到数字的轻量版本。
#[test]
#[ignore = "实测数据：手动用 release 跑"]
fn timing() {
    use std::time::Instant;

    fn stats(mut v: Vec<f64>) -> (f64, f64, f64) {
        v.sort_by(f64::total_cmp);
        (v[0], v[v.len() / 2], v[v.len() - 1])
    }

    let text = big_csharp();
    let line_count = text.split('\n').count();
    let kinds = kinds_for(Some(&*language("csharp")), None, None);
    let mut doc = TextDocument::new(&text);

    let mut full = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        let state = build(kinds.clone(), &doc);
        std::hint::black_box(state.brackets_in_line(10).len());
        full.push(start.elapsed().as_secs_f64() * 1e3);
    }
    let (lo, mid, hi) = stats(full);
    let mut state = build(kinds.clone(), &doc);
    let brackets: usize = (0..line_count as u64)
        .map(|l| state.brackets_in_line(l).len())
        .sum();
    println!(
        "[#57] {line_count} 行 C#（{brackets} 个括号）整篇解析：min {lo:.3} ms / p50 {mid:.3} ms / max {hi:.3} ms"
    );

    // 单字符编辑的增量更新。
    let lines: Vec<&str> = text.split('\n').collect();
    let target = lines
        .iter()
        .position(|l| l.contains("var step3"))
        .expect("目标行") as u64;
    let original = lines[target as usize].to_string();
    let edited = original.replace("step3", "stepX");
    let mut each = Vec::new();
    for i in 0..500 {
        doc.set_line(target, if i % 2 == 0 { &edited } else { &original });
        let start = Instant::now();
        state.note_lines(vec![LineEdit {
            old: target..target + 1,
            new: target..target + 1,
        }]);
        state.update(&doc);
        each.push(start.elapsed().as_secs_f64() * 1e3);
    }
    let (lo, mid, hi) = stats(each);
    let s = state.stats();
    println!(
        "[#57] 同一文件单字符编辑的增量更新：min {lo:.4} ms / p50 {mid:.4} ms / max {hi:.4} ms（复用 {} 棵子树、新建 {} 个节点）",
        s.reused, s.constructed
    );

    // 连续滚动：每帧为整屏 40 行重新生成样式区间（最坏情况）。
    state.colors.depth = [Some(rove_theme::Color(0xFF, 0xD7, 0, 0xFF)); DEPTH_COLORS];
    state.colors.unexpected = Some(rove_theme::Color(0xFF, 0x12, 0x12, 0xCC));
    let mut frames = Vec::new();
    let mut out = Vec::new();
    for frame in 0..500u64 {
        let top = frame % (line_count as u64 - 40);
        out.clear();
        let start = Instant::now();
        for line in top..top + 40 {
            state.colorize(line, &mut out);
        }
        frames.push(start.elapsed().as_secs_f64() * 1e6);
        std::hint::black_box(out.len());
    }
    let (lo, mid, hi) = stats(frames);
    println!(
        "[#57] 连续滚动每帧额外开销（整屏 40 行全部重算）：min {lo:.2} µs / p50 {mid:.2} µs / max {hi:.2} µs"
    );

    // 最坏情况：单行压缩 JSON。
    let json = minified_json();
    let json_kinds = kinds_for(Some(&*language("json")), None, None);
    let mut json_doc = TextDocument::new(&json);
    let mut parses = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        let state = build(json_kinds.clone(), &json_doc);
        std::hint::black_box(state.brackets_in_line(0).len());
        parses.push(start.elapsed().as_secs_f64() * 1e3);
    }
    let (lo, mid, hi) = stats(parses);
    let mut state = build(json_kinds.clone(), &json_doc);
    println!(
        "[#57] 单行压缩 JSON（{} 个括号）整篇解析：min {lo:.3} ms / p50 {mid:.3} ms / max {hi:.3} ms",
        state.brackets_in_line(0).len()
    );
    let json2 = format!("{json} ");
    let mut each = Vec::new();
    for i in 0..50 {
        json_doc.set_line(0, if i % 2 == 0 { &json2 } else { &json });
        let start = Instant::now();
        state.note_lines(vec![LineEdit {
            old: 0..1,
            new: 0..1,
        }]);
        state.update(&json_doc);
        each.push(start.elapsed().as_secs_f64() * 1e3);
    }
    let (lo, mid, hi) = stats(each);
    println!(
        "[#57] 单行压缩 JSON 的一次编辑（整行即整篇，没有可复用的子树）：min {lo:.3} ms / p50 {mid:.3} ms / max {hi:.3} ms"
    );
}

#[test]
fn an_empty_document_parses_to_nothing() {
    let kinds = kinds_for(Some(&*language("json")), None, None);
    let doc = TextDocument::new("");
    let state = build(kinds, &doc);
    assert!(state.has_tree(), "解析过了");
    assert!(state.brackets_in_line(0).is_empty());
    assert!(state.active_pair(0, 0, MatchBrackets::Always).is_none());
}
