//! #26：大纲符号数据与面包屑的公开接口基准。
//!
//! - `outline/breadcrumb_cursor_move_5000_lines`：5,000 行 C# 中光标逐行移动，每次求面包屑的符号路径
//!   （`DocumentSymbols::path_from` 复用上一条路径 + 按 `breadcrumbs.*` 过滤出符号段）。预算 < 1 ms。
//! - `outline/csharp_symbols_5000_lines`：同一文件在后台完整提取符号并建立索引（不在 UI 线程）。
use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{Cancellation, Line, Position, Utf8Column, Version};
use rove_syntax::symbols::{extract, DocumentSymbols, SymbolId};
use rove_ui::outline::model::{symbol_segments, PathMode, SymbolFilter};
use std::{hint::black_box, sync::Arc};

/// Unity 风格的 5,000 行脚本：命名空间 → 25 个类 → 每类字段、属性与 38 个带方法体的方法。
fn source() -> String {
    let mut s = String::from("using UnityEngine;\n\nnamespace Game.Generated\n{\n");
    let mut class = 0;
    while s.lines().count() < 4_990 {
        s.push_str(&format!(
            "    public class Component{class} : MonoBehaviour\n    {{\n        [SerializeField] private float speed{class};\n        public int Health {{ get; private set; }}\n"
        ));
        for m in 0..38 {
            s.push_str(&format!(
                "        public void Method{m}(int value, Vector3 dir)\n        {{\n            var x = value * {m};\n            Debug.Log(x);\n        }}\n"
            ));
        }
        s.push_str("    }\n");
        class += 1;
    }
    s.push_str("}\n");
    s
}

fn benchmark(c: &mut Criterion) {
    let text = source();
    let lines = text.lines().count() as u64;
    assert!((4_990..5_300).contains(&lines), "{lines}");
    let cancel = Cancellation::default();
    let mut group = c.benchmark_group("outline");
    group.bench_function("csharp_symbols_5000_lines", |b| {
        b.iter(|| {
            let symbols = extract("csharp", &text, &cancel).unwrap();
            black_box(DocumentSymbols::new(
                Version(1),
                "csharp".into(),
                symbols.into(),
            ))
        })
    });
    let symbols = Arc::new(DocumentSymbols::new(
        Version(1),
        "csharp".into(),
        extract("csharp", &text, &cancel).unwrap().into(),
    ));
    let filter = SymbolFilter::default();
    let mut path: Vec<SymbolId> = Vec::new();
    let mut line = 0;
    group.bench_function("breadcrumb_cursor_move_5000_lines", |b| {
        b.iter(|| {
            line = (line + 1) % lines;
            let at = Position {
                line: Line(line),
                column: Utf8Column(12),
            };
            path = symbols.path_from(&path, at);
            black_box(symbol_segments(&symbols, &filter, &path, PathMode::On))
        })
    });
    group.finish();
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
