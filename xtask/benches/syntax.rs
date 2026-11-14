//! Measures the public syntax service on texts read through the real `RopeText`, so
//! buffer reads and per-line range lookups are part of every measurement (#94).
use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use futures_lite::future::block_on;
use rove_base::{ByteOffset, Cancellation, Line, Span, TextEdit};
use rove_buffer::{RopeText, TextSource};
use rove_syntax::{highlight::TreeSitterSyntax, SyntaxService, SyntaxUpdate};
use std::{cell::Cell, hint::black_box, sync::Arc, time::Duration};

fn analyze(service: &dyn SyntaxService, text: Arc<dyn TextSource>) -> SyntaxUpdate {
    block_on(service.analyze(text, "csharp".into(), Cancellation::default())).unwrap()
}

fn rope(text: &str) -> Arc<dyn TextSource> {
    Arc::new(RopeText::from_text(text))
}

/// 一屏可见行数（首屏口径，与 #115 的「先显示可见区域」一致）。
const VISIBLE_LINES: u64 = 50;

fn benchmark(c: &mut Criterion) {
    // Measure completed analysis even when this worker is temporarily descheduled.
    let syntax = TreeSitterSyntax::default().with_refresh_budget(Duration::from_secs(30));
    // Compile bundled queries once, as a running editor does. Each timed cold parse still
    // starts without a previous tree or tokens, and materializes highlighting for every line.
    analyze(&syntax, rope("public class Warmup {}"));
    let mut group = c.benchmark_group("syntax");

    let mut source = String::from("class Component {\n");
    for index in 0..4_997 {
        source.push_str(&format!("    int Method{index}() {{ return 1; }}\n"));
    }
    source.push_str("}\n");
    assert_eq!(source.split('\n').count(), 5_000);
    let byte = source.find("Method2498() { return 1;").unwrap() + "Method2498() { return ".len();
    let first = source.clone();
    source.replace_range(byte..byte + 1, "2");
    let second = source;
    group.bench_function("csharp_edit_5000_lines", |b| {
        let document = syntax.document();
        let initial = rope(&first);
        let previous = Cell::new(initial.version());
        analyze(&document, initial);
        let toggle = Cell::new(false);
        // Building the next RopeText snapshot belongs to the buffer, not to highlighting.
        b.iter_batched(
            || {
                let (text, inserted) = if toggle.replace(!toggle.get()) {
                    (&first, "1")
                } else {
                    (&second, "2")
                };
                (rope(text), inserted)
            },
            |(text, inserted)| {
                let edit = TextEdit {
                    version: previous.get(),
                    range: Span::new(ByteOffset(byte as u64), ByteOffset(byte as u64 + 1)).unwrap(),
                    inserted: inserted.into(),
                };
                let version = text.version();
                let update = analyze(&document.with_edit(edit), text);
                previous.set(version);
                assert_eq!(update.snapshot.version(), version);
                let lines = update.changed_lines.as_ref().unwrap();
                assert_eq!(
                    lines.as_ref(),
                    &[Span::new(Line(2499), Line(2500)).unwrap()]
                );
                black_box(update)
            },
            BatchSize::SmallInput,
        )
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
    let cold = rope(&source);
    group.bench_function("csharp_initial_1mb", |b| {
        b.iter(|| black_box(analyze(&syntax, cold.clone())))
    });
    let unity = rope(&unity_source());
    group.bench_function("csharp_initial_1mb_unity", |b| {
        b.iter(|| black_box(analyze(&syntax, unity.clone())))
    });
    // 首次打开的口径（#117）：没有上一份结果要释放，只量「可见区域出高亮」。
    // `iter_batched` 把上一轮结果的回收放在计时之外，`with_visible` 之后的全量补齐
    // 在后台进行，由 csharp_initial_1mb_unity 记录。
    let viewport = Span::new(Line(0), Line(VISIBLE_LINES)).unwrap();
    group.bench_function("csharp_visible_1mb_unity", |b| {
        b.iter_batched(
            || syntax.document().with_visible(viewport.clone()),
            |document| {
                let update = analyze(&document, unity.clone());
                assert!(!update.snapshot.tokens(Line(1)).unwrap().is_empty());
                assert!(update
                    .snapshot
                    .tokens(Line(VISIBLE_LINES + 100))
                    .unwrap()
                    .is_empty());
                black_box(update)
            },
            BatchSize::PerIteration,
        )
    });
    group.finish();
}

// Reproducible Unity-style code, with complete declarations and no project data.
fn unity_source() -> String {
    let mut source = String::from(
        "using UnityEngine;\nusing System.Collections.Generic;\nnamespace BenchFixtures {\n",
    );
    for index in 0.. {
        let class = format!(
            r#"    public sealed class MovementController{index} : MonoBehaviour
    {{
        #region Configuration
        [SerializeField] private float speed = 2.5f;
        [SerializeField] private Transform target;
        private readonly List<Vector3> waypoints = new List<Vector3>();
        public bool IsActive {{ get; private set; }}
        public int WaypointCount => waypoints.Count;
        #endregion

        private void Awake()
        {{
            IsActive = target != null;
            waypoints.Add(transform.position);
        }}

        private void Update()
        {{
            if (!IsActive)
                return;

            var step = speed * Time.deltaTime;
            transform.position = Vector3.MoveTowards(transform.position, target.position, step);
        }}

        #if UNITY_EDITOR
        private void OnValidate()
        {{
            if (speed < 0)
            {{
                Debug.LogWarning($"Negative speed on {{gameObject.name}}: {{speed:F2}}", this);
                speed = 0;
            }}
        }}
        #endif
    }}
"#
        );
        if source.len() + class.len() + 2 > 1_000_000 {
            break;
        }
        source.push_str(&class);
    }
    source.push_str(&" ".repeat(1_000_000 - source.len() - 2));
    source.push_str("}\n");
    assert_eq!(source.len(), 1_000_000);
    source
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
