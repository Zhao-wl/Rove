//! #10 主题引擎：主题加载、TextMate scope 匹配（缓存命中 / 未命中）、颜色键查询。
use criterion::{criterion_group, criterion_main, Criterion};
use rove_theme::service::{load_theme, p0_registry};
use rove_theme::{Customizations, ThemeCatalog, DEFAULT_THEME};
use std::hint::black_box;
use std::sync::Arc;

/// C# 文件里常见的 scope 路径（由外到内），覆盖父 scope 选择器与深层 scope。
const PATHS: &[&[&str]] = &[
    &["source.cs", "storage.modifier.public.cs"],
    &["source.cs", "keyword.control.flow.return.cs"],
    &["source.cs", "storage.type.class.cs"],
    &["source.cs", "entity.name.type.class.cs"],
    &[
        "source.cs",
        "meta.class.body.cs",
        "meta.method.cs",
        "entity.name.function.cs",
    ],
    &[
        "source.cs",
        "meta.class.body.cs",
        "meta.method.cs",
        "variable.parameter.cs",
    ],
    &[
        "source.cs",
        "comment.line.double-slash.cs",
        "punctuation.definition.comment.cs",
    ],
    &["source.cs", "string.quoted.double.cs"],
    &[
        "source.cs",
        "string.quoted.double.cs",
        "constant.character.escape.cs",
    ],
    &["source.cs", "constant.numeric.decimal.cs"],
    &["source.cs", "punctuation.terminator.statement.cs"],
    &[
        "source.cs",
        "meta.preprocessor.cs",
        "keyword.preprocessor.region.cs",
    ],
    &["source.cs", "keyword.operator.assignment.cs"],
    &["source.cs", "variable.other.object.property.cs"],
    &["source.cs", "entity.name.type.namespace.cs"],
    &["source.cs", "meta.attribute.cs", "entity.name.type.cs"],
];

fn theme(c: &mut Criterion) {
    let catalog = ThemeCatalog::builtin();
    let entry = catalog.get(DEFAULT_THEME).unwrap().clone();
    let registry = Arc::new(p0_registry());
    let custom = Customizations::default();
    let mut group = c.benchmark_group("theme");

    group.bench_function("load_dark_2026", |b| {
        b.iter(|| black_box(load_theme(&entry, &custom, registry.clone()).unwrap()))
    });

    let resolved = load_theme(&entry, &custom, registry.clone()).unwrap();
    let paths: Vec<Vec<Arc<str>>> = PATHS
        .iter()
        .map(|p| p.iter().map(|s| Arc::from(*s)).collect())
        .collect();
    // 每次迭代解析全部 16 条路径；除以 16 得到单个 token 的开销。
    group.bench_function("token_style_uncached_16", |b| {
        b.iter(|| {
            for p in &paths {
                black_box(resolved.token_style_uncached(black_box(p)));
            }
        })
    });
    for p in &paths {
        resolved.token_style(p);
    }
    group.bench_function("token_style_cached_16", |b| {
        b.iter(|| {
            for p in &paths {
                black_box(resolved.token_style(black_box(p)));
            }
        })
    });
    group.bench_function("color_lookup", |b| {
        b.iter(|| black_box(resolved.color(black_box("editorLineNumber.activeForeground"))))
    });
    group.finish();
}

criterion_group!(benches, theme);
criterion_main!(benches);
