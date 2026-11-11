#[path = "../tests/support/mod.rs"]
mod support;
use futures_lite::future::block_on;
use rove_base::Cancellation;
use rove_syntax::{highlight::TreeSitterSyntax, SyntaxService};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn main() {
    std::fs::write("C:/Users/zhaowenlong/unity.cs", unity_source()).unwrap();
    let threads: usize = std::env::var("THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(16);
    let syntax = TreeSitterSyntax::default()
        .with_refresh_budget(Duration::from_secs(30))
        .with_parallelism(threads);
    let text: Arc<dyn rove_buffer::TextSource> = if std::env::args().any(|a| a == "rope") {
        Arc::new(rove_buffer::RopeText::new(unity_source()))
    } else {
        Arc::new(support::Text::new(unity_source(), 0))
    };
    for _ in 0..8 {
        let t = Instant::now();
        let u = block_on(syntax.analyze(text.clone(), "csharp".into(), Cancellation::default()))
            .unwrap();
        eprintln!("total {:?}", t.elapsed());
        drop(u);
    }
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
