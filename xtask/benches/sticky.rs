//! #55：粘性滚动每帧额外开销的基准（公开接口，不开窗口）。
//!
//! - `sticky/scroll_5000_lines_outline`：5,000 行 C# 中逐行连续滚动，每帧按「粘性区下面的第一行」
//!   更新作用域链（大纲模型，含行区间缓存命中的帧）。预算：每帧 < 100 µs。
//! - `sticky/scroll_5000_lines_folding`：同一文件、同一滚动序列，折叠提供方模型（区间链）。
//! - `sticky/recompute_outline`：每次都强制重算（缓存全未命中）的最坏情况，用来看缓存省下了多少。
use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{Cancellation, Version};
use rove_syntax::symbols::{extract, DocumentSymbols};
use rove_ui::code_view::folding::{indentation_ranges, FoldRange};
use rove_ui::code_view::sticky::{StickyModel, StickyScroll, StickySources};
use std::{hint::black_box, sync::Arc};

/// Unity 风格的 5,000 行脚本：命名空间 → 25 个类 → 每类 8 个字段与 19 个方法。
fn source() -> String {
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
                s.push_str(&format!("            var step{k} = value * {k};\n"));
            }
            s.push_str("        }\n\n");
        }
        s.push_str("    }\n\n");
    }
    s.push_str("}\n");
    s
}

fn benchmark(c: &mut Criterion) {
    let text = source();
    let lines = text.lines().count() as u64;
    assert!((4_900..5_400).contains(&lines), "{lines}");
    let cancel = Cancellation::default();
    let symbols = Arc::new(DocumentSymbols::new(
        Version(1),
        "csharp".into(),
        extract("csharp", &text, &cancel).unwrap().into(),
    ));
    let indent: Vec<FoldRange> = indentation_ranges(&text, Some("csharp"), 4);
    let outline = StickySources {
        symbols: Some(&symbols),
        folds: &[],
        indent: &[],
    };
    let folding = StickySources {
        symbols: None,
        folds: &indent,
        indent: &indent,
    };
    let max = 5;

    let mut group = c.benchmark_group("sticky");
    let mut state = StickyScroll::default();
    let mut line = 0u64;
    group.bench_function("scroll_5000_lines_outline", |b| {
        b.iter(|| {
            line = (line + 1) % lines;
            black_box(state.update(line, max, StickyModel::Outline, outline))
        })
    });
    let mut state = StickyScroll::default();
    let mut line = 0u64;
    group.bench_function("scroll_5000_lines_folding", |b| {
        b.iter(|| {
            line = (line + 1) % lines;
            black_box(state.update(line, max, StickyModel::FoldingProvider, folding))
        })
    });
    let mut state = StickyScroll::default();
    let mut line = 0u64;
    group.bench_function("recompute_outline", |b| {
        b.iter(|| {
            line = (line + 1) % lines;
            // 丢掉行区间缓存：每一帧都重新二分查找符号路径。
            state.invalidate();
            black_box(state.update(line, max, StickyModel::Outline, outline))
        })
    });
    group.finish();
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
