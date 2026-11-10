//! Measures the public syntax service on an already-loaded TextSource.
#[path = "../../crates/rove-syntax/tests/support/mod.rs"]
mod support;

use criterion::{criterion_group, criterion_main, Criterion};
use futures_lite::future::block_on;
use rove_base::{ByteOffset, Cancellation, Line, Span, TextEdit, Version};
use rove_syntax::{highlight::TreeSitterSyntax, SyntaxService, SyntaxUpdate};
use std::{hint::black_box, sync::Arc, time::Duration};
use support::Text;

fn analyze(service: &dyn SyntaxService, text: Arc<Text>) -> SyntaxUpdate {
    block_on(service.analyze(text, "csharp".into(), Cancellation::default())).unwrap()
}

fn benchmark(c: &mut Criterion) {
    // Measure completed analysis even when this worker is temporarily descheduled.
    let syntax = TreeSitterSyntax::default().with_refresh_budget(Duration::from_secs(30));
    // Compile bundled queries once, as a running editor does. Each timed cold parse still
    // starts without a previous tree or tokens, and materializes highlighting for every line.
    analyze(&syntax, Arc::new(Text::new("public class Warmup {}", 0)));
    let mut group = c.benchmark_group("syntax");

    let mut source = String::from("class Component {\n");
    for index in 0..4_997 {
        source.push_str(&format!("    int Method{index}() {{ return 1; }}\n"));
    }
    source.push_str("}\n");
    assert_eq!(source.split('\n').count(), 5_000);
    let byte = source.find("Method2498() { return 1;").unwrap() + "Method2498() { return ".len();
    let first = Text::new(source.clone(), 0);
    source.replace_range(byte..byte + 1, "2");
    let second = Text::new(source, 1);
    group.bench_function("csharp_edit_5000_lines", |b| {
        let document = syntax.document();
        analyze(&document, Arc::new(first.clone()));
        let mut version = 0;
        b.iter(|| {
            let (text, inserted) = if version % 2 == 0 {
                (&second, "2")
            } else {
                (&first, "1")
            };
            let edit = TextEdit {
                version: Version(version),
                range: Span::new(ByteOffset(byte as u64), ByteOffset(byte as u64 + 1)).unwrap(),
                inserted: inserted.into(),
            };
            version += 1;
            let update = analyze(
                &document.with_edit(edit),
                Arc::new(text.at_version(Version(version))),
            );
            assert_eq!(update.snapshot.version(), Version(version));
            let lines = update.changed_lines.as_ref().unwrap();
            assert_eq!(
                lines.as_ref(),
                &[Span::new(Line(2499), Line(2500)).unwrap()]
            );
            black_box(update)
        })
    });

    let mut source = String::from("class Messages {\n");
    for index in 0.. {
        let line = format!(
            "    string Message{index}() {{ return \"The scene could not be initialized because a required component was missing from the selected prefab. Verify the prefab and try again.\"; }}\n"
        );
        if source.len() + line.len() + 2 > 1_000_000 {
            break;
        }
        source.push_str(&line);
    }
    source.push_str(&" ".repeat(1_000_000 - source.len() - 2));
    source.push_str("}\n");
    assert_eq!(source.len(), 1_000_000);
    let cold = Arc::new(Text::new(source, 0));
    group.bench_function("csharp_initial_1mb", |b| {
        b.iter(|| black_box(analyze(&syntax, cold.clone())))
    });
    group.finish();
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
