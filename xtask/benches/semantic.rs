//! #59：C# 语义着色的开销（纯计算，不开窗口）。
//!
//! - `semantic/styles_5000_lines_csharp`：5,000 行 C# 的全部语义 token → 样式区间
//!   （打开文件后首屏之后做一次；预算 < 100 ms，索引就绪时的「补齐」成本就是这一项）。
//! - `semantic/scroll_5000_lines_csharp`：连续滚动时语义装饰带来的每帧额外开销。
//!   口径取最坏情况：**整屏 40 行全部重新生成装饰**（真实情况下只有新进视口的一两行
//!   会重算，其余命中 #19 的按行缓存）。预算：< 100 µs。
//! - `semantic/classify_tokens`：符号种类 + 修饰符 → 标准 token 类型的映射本身。
//! - `semantic/theme_style_uncached`：主题侧的选择器匹配 + 回退表探测（缓存未命中）。
//!
//! 基准只调用公开接口：`rove_lang_cs::semantic`、`rove_theme::ResolvedTheme`、
//! `rove_ui::code_view::semantic`。
use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{Line, Utf8Column};
use rove_lang_cs::query::SemanticToken;
use rove_lang_cs::semantic::{syntax::FileFacts, CsSymbolKind, Modifiers, SymbolFacts};
use rove_theme::service::{load_theme, p0_registry};
use rove_theme::{Customizations, ThemeCatalog, DEFAULT_THEME};
use rove_ui::code_view::decorations::LineDecorations;
use rove_ui::code_view::semantic::{SemanticLayer, SemanticStyles, LANGUAGE};
use std::hint::black_box;
use std::sync::Arc;

/// Unity 风格的 5,000 行 C#（与 #57 的括号基准同一份生成器，便于横向比较）。
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

/// 真实分布的语义 token：每一行平均一个标识符，类型按 Unity 代码的比例分布。
fn tokens(text: &str) -> Vec<SemanticToken> {
    const KINDS: &[(&str, &[&str])] = &[
        ("field", &["declaration"]),
        ("field", &[]),
        ("method", &["declaration"]),
        ("method", &[]),
        ("parameter", &["declaration"]),
        ("variable", &["declaration"]),
        ("variable", &[]),
        ("class", &[]),
        ("property", &[]),
        ("variable", &["static", "readonly"]),
    ];
    text.lines()
        .enumerate()
        .filter_map(|(line, source)| {
            let start = source.len().checked_sub(source.trim_start().len())?;
            let len = source
                .trim()
                .split(|c: char| !c.is_alphanumeric())
                .next()?
                .len();
            if len == 0 {
                return None;
            }
            let (token_type, modifiers) = KINDS[line % KINDS.len()];
            Some(SemanticToken {
                line: Line(line as u64),
                column: Utf8Column(start as u64),
                length_bytes: len as u64,
                token_type: (*token_type).into(),
                modifiers: modifiers.iter().map(|m| Arc::<str>::from(*m)).collect(),
            })
        })
        .collect()
}

fn theme() -> Arc<rove_theme::ResolvedTheme> {
    let catalog = ThemeCatalog::builtin();
    let entry = catalog.get(DEFAULT_THEME).expect("内置主题");
    Arc::new(
        load_theme(entry, &Customizations::default(), Arc::new(p0_registry())).expect("加载主题"),
    )
}

fn bench(c: &mut Criterion) {
    let text = csharp();
    let tokens = tokens(&text);
    let theme = theme();
    let lines: Vec<&str> = text.lines().collect();
    let mut group = c.benchmark_group("semantic");

    // 单独测全文件 token 到样式区间的转换，真窗口另测首屏到补齐。
    group.bench_function("styles_5000_lines_csharp", |b| {
        b.iter(|| {
            let styles = SemanticStyles::build(black_box(&tokens), &|ty, mods| {
                theme.semantic_token_style(ty, mods, LANGUAGE)
            });
            black_box(styles.is_empty())
        })
    });

    // 索引里没有的声明细节（类型种类与修饰符）：整份文件的语法提取。
    group.bench_function("declaration_facts_5000_lines_csharp", |b| {
        b.iter(|| black_box(FileFacts::parse(black_box(&text)).len()))
    });

    // 滚动经过全文件，每帧重新生成当前 40 行的全部语义装饰。
    let mut layer = SemanticLayer::default();
    layer.update(SemanticStyles::build(&tokens, &|ty, mods| {
        theme.semantic_token_style(ty, mods, LANGUAGE)
    }));
    layer.update_inactive(vec![20..30, 100..140, 1000..1100, 3190..3210]);
    layer.unnecessary_opacity = Some(0xaa);
    group.bench_function("scroll_5000_lines_csharp", |b| {
        let mut first = 0;
        let last = lines.len() as u64 - 40;
        b.iter(|| {
            let mut total = 0usize;
            for line in first..first + 40 {
                let mut out = LineDecorations::default();
                layer.decorate(line, lines[line as usize], &mut out);
                total += out.inline.len() + usize::from(out.line.opacity.is_some());
            }
            first = if first == last { 0 } else { first + 1 };
            black_box(total)
        })
    });

    // 映射表本身：种类 + 修饰符 → 类型名 + 修饰符名。
    let facts: Vec<SymbolFacts> = [
        CsSymbolKind::Field,
        CsSymbolKind::Constant,
        CsSymbolKind::Method,
        CsSymbolKind::Class,
        CsSymbolKind::Parameter,
    ]
    .into_iter()
    .map(|kind| SymbolFacts {
        kind,
        modifiers: Modifiers {
            declaration: true,
            static_: true,
            ..Modifiers::default()
        },
    })
    .collect();
    group.bench_function("classify_tokens", |b| {
        b.iter(|| {
            for f in &facts {
                black_box(f.classify());
            }
        })
    });

    // 主题侧：选择器匹配 + 回退表探测（缓存未命中的最坏情况）。
    group.bench_function("theme_style_uncached", |b| {
        b.iter(|| {
            black_box(theme.semantic_token_style_uncached("variable", &["readonly"], LANGUAGE));
            black_box(theme.semantic_token_style_uncached("class", &["declaration"], LANGUAGE));
            black_box(theme.semantic_token_style_uncached("method", &["static"], LANGUAGE));
        })
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
