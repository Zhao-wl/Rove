//! #57：括号对树的增量更新与着色的每帧开销（纯计算，不开窗口）。
//!
//! - `brackets/parse_5000_lines_csharp`：5,000 行 C# 的整篇解析（打开文件时各做一次）。
//! - `brackets/edit_5000_lines_csharp`：同一份文件里做一次单字符编辑后的**增量更新**。
//!   预算：< 1 ms（与 #9 的 2 ms 重解析预算分开计）。
//! - `brackets/scroll_5000_lines_csharp`：连续滚动时括号着色带来的每帧额外开销。
//!   口径取最坏情况：**整屏 40 行全部重新生成样式区间**（真实情况下只有新进视口的那
//!   一两行会重算，其余命中 #19 的按行缓存）。预算：< 100 µs。
//! - `brackets/minified_json_*`：单行压缩 JSON 的最坏情况（括号极多）。
//!
//! 基准只调用 `rove_ui::code_view::bracket_pairs` 的公开接口。
use criterion::{criterion_group, criterion_main, Criterion};
use rove_syntax::language_config::BuiltinLanguages;
use rove_syntax::LanguageConfigurations;
use rove_ui::code_view::bracket_pairs::{
    kinds_for, parser::LineEdit, BracketKind, BracketPairs, TextDocument,
};
use rove_ui::code_view::decorations::InlineDecoration;
use std::hint::black_box;

fn kinds(language: &str) -> Vec<BracketKind> {
    let data = BuiltinLanguages::shared()
        .0
        .get(language)
        .expect("语言配置");
    kinds_for(Some(&*data), None, None)
}

/// Unity 风格的 5,000 行 C#：命名空间 → 25 个类 → 每类 8 个字段与 19 个方法。
fn csharp() -> String {
    let mut s = String::from("using UnityEngine;\n\nnamespace Game.Generated\n{\n");
    for c in 0..25 {
        s.push_str(&format!(
            "    public class Component{c} : MonoBehaviour\n    {{\n"
        ));
        for f in 0..8 {
            s.push_str(&format!("        [SerializeField] private int field{f};\n"));
        }
        for m in 0..19 {
            s.push_str(&format!(
                "        public void Method{m}(int value)\n        {{\n"
            ));
            for k in 0..6 {
                s.push_str(&format!(
                    "            var step{k} = Mathf.Max(value * {k}, data[{k}]);\n"
                ));
            }
            s.push_str("        }\n\n");
        }
        s.push_str("    }\n\n");
    }
    s.push_str("}\n");
    s
}

/// 单行压缩 JSON：5,000 个对象，一共 10,002 个括号。
fn minified_json() -> String {
    let mut s = String::from("[");
    for i in 0..5_000 {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{{\"id\":{i},\"v\":[{i},{i}]}}"));
    }
    s.push(']');
    s
}

fn built(kinds: Vec<BracketKind>, doc: &TextDocument) -> BracketPairs {
    let mut state = BracketPairs::default();
    state.set_kinds(kinds);
    state.limit = usize::MAX;
    state.colors.depth = [Some(rove_theme::Color(0xFF, 0xD7, 0, 0xFF)); 6];
    state.colors.unexpected = Some(rove_theme::Color(0xFF, 0x12, 0x12, 0xCC));
    state.update(doc);
    state
}

fn benchmark(c: &mut Criterion) {
    let source = csharp();
    let lines: Vec<&str> = source.split('\n').collect();
    assert!((4_900..5_400).contains(&lines.len()), "{} 行", lines.len());
    let csharp_kinds = kinds("csharp");
    let doc = TextDocument::new(&source);

    let mut group = c.benchmark_group("brackets");

    group.bench_function("parse_5000_lines_csharp", |b| {
        b.iter(|| {
            let state = built(csharp_kinds.clone(), &doc);
            black_box(state.brackets_in_line(10).len())
        })
    });

    // 单字符编辑：在某个方法体里把 `value * 3` 改成 `value * 4`，长度不变。
    let target = lines
        .iter()
        .position(|l| l.contains("var step3"))
        .expect("找得到目标行") as u64;
    let original = lines[target as usize].to_string();
    let edited = original.replace("step3", "stepX");
    group.bench_function("edit_5000_lines_csharp", |b| {
        let mut state = built(csharp_kinds.clone(), &doc);
        let mut doc = doc.clone();
        let mut flip = false;
        b.iter(|| {
            flip = !flip;
            doc.set_line(target, if flip { &edited } else { &original });
            state.note_lines(vec![LineEdit {
                old: target..target + 1,
                new: target..target + 1,
            }]);
            state.update(&doc);
            black_box(state.stats())
        })
    });

    // 连续滚动：每帧为整屏 40 行重新生成样式区间（最坏情况）。
    let state = built(csharp_kinds.clone(), &doc);
    group.bench_function("scroll_5000_lines_csharp", |b| {
        let mut top = 0u64;
        let mut out: Vec<InlineDecoration> = Vec::with_capacity(512);
        b.iter(|| {
            top = (top + 1) % (lines.len() as u64 - 40);
            out.clear();
            for line in top..top + 40 {
                state.colorize(line, &mut out);
            }
            black_box(out.len())
        })
    });

    // 最坏情况：单行压缩 JSON。
    let json_kinds = kinds("json");
    let json = minified_json();
    let json_doc = TextDocument::new(&json);
    group.bench_function("minified_json_parse", |b| {
        b.iter(|| {
            let state = built(json_kinds.clone(), &json_doc);
            black_box(state.brackets_in_line(0).len())
        })
    });
    group.bench_function("minified_json_edit_worst_case", |b| {
        let mut state = built(json_kinds.clone(), &json_doc);
        let mut doc = json_doc.clone();
        let mut flip = false;
        let edited = format!("{json} ");
        b.iter(|| {
            flip = !flip;
            doc.set_line(0, if flip { &edited } else { &json });
            state.note_lines(vec![LineEdit {
                old: 0..1,
                new: 0..1,
            }]);
            state.update(&doc);
            black_box(state.stats())
        })
    });

    group.finish();
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
