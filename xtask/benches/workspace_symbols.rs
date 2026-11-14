//! #60 转到工作区中的符号（`Ctrl+T` 与快速打开的 `#`）的每次按键：
//! 输入文本 → 经 #1 契约的 [`CsQueries::workspace_symbols`]（#38 的路由 + #37 的 L1 声明索引）
//! 取回结果 → 转成列表行与高亮（`rove_ui::workspace_symbols::picks`）。
//!
//! 验收：大型 Unity 项目（`ROVE_TEST_UNITY_LARGE`）**每次按键到列表刷新 < 30 ms（p99）**
//! （`LANGUAGE_INTELLIGENCE` §1）。与 #24 快速打开的 16 ms 预算分开报。
//!
//! - `workspace_symbols/keystroke_synthetic_1k`：合成 1 000 个 Unity 风格 `.cs`（CI 上也能跑）。
//! - `workspace_symbols/keystroke_large`：大型项目；未设置环境变量时跳过，
//!   `check-budgets` 显示「未测量」、不算失败。
//!
//! 只调用公开接口：`L1Index`、`CsRouter`（经 `CsQueries`）、`workspace_symbols::picks`。
use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{Cancellation, WorkspacePath};
use rove_lang_cs::l1::{L1Index, L1Options};
use rove_lang_cs::query::CsQueries;
use rove_lang_cs::router::{
    BufferStates, CsRouter, ManualSemantic, SemanticSource, SemanticStatus,
};
use rove_test_support::UnityProject;
use rove_ui::list_widget::matcher::Matcher;
use rove_ui::quick_access::matcher::FuzzyMatcher;
use rove_ui::workspace_symbols::{self, SymbolLabels};
use rove_workspace::{default_settings, EntryKind, Folder, Workspace, WorkspaceConfig};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::{hint::black_box, time::Instant};

/// 逐字键入的几组按键（与 #37 的 L1 基准同一组，便于对照）。
const KEYSTROKES: [&str; 8] = ["M", "Me", "Met", "Meth", "Method1", "B", "Beh", "pc"];

fn synthetic_fixture() -> (PathBuf, Vec<WorkspacePath>) {
    let root = std::env::temp_dir().join(format!("rove-ws-symbols-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut files = Vec::new();
    for i in 0..1000 {
        let rel = format!("Assets/Scripts/G{:02}/Behaviour{i:04}.cs", i % 40);
        let path = root.join(&rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = format!(
            "using UnityEngine;\n\nnamespace Game.Feature{g}\n{{\n    public partial class Behaviour{i:04} : MonoBehaviour\n    {{\n",
            g = i % 40
        );
        for m in 0..12 {
            text.push_str(&format!(
                "        private int field{m};\n        public int Value{m} {{ get; set; }}\n        public void Method{m}(int a, string b) {{ }}\n"
            ));
        }
        text.push_str("    }\n}\n");
        std::fs::write(&path, text).unwrap();
        files.push(WorkspacePath::new(&rel).unwrap());
    }
    (root, files)
}

fn workspace_files(root: &Path) -> (PathBuf, Vec<WorkspacePath>) {
    let config = WorkspaceConfig::from_settings(&default_settings())
        .expect("workspace defaults")
        .without_watching();
    let folder = Folder::scan_root(root, config).expect("scan workspace");
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        let page = folder.files(start..start + 4096).expect("files page");
        start += page.items.len();
        out.extend(
            page.items
                .iter()
                .filter(|e| matches!(e.kind, EntryKind::File))
                .map(|e| e.path.clone()),
        );
        if page.items.len() < 4096 {
            return (folder.root().to_path_buf(), out);
        }
    }
}

/// 正式装配里的查询入口：#38 的路由（L2 未安装，走 #37 的 L1）。
fn router(root: &Path, files: &[WorkspacePath]) -> Arc<CsRouter> {
    let l1 = Arc::new(L1Index::new(L1Options::new(root)));
    l1.build(files.iter().cloned(), &Cancellation::default())
        .expect("build l1");
    let semantic: Arc<dyn SemanticSource> =
        Arc::new(ManualSemantic::unavailable(SemanticStatus::NotInstalled));
    Arc::new(
        CsRouter::new(semantic, Arc::new(BufferStates::new()))
            .with_root(root)
            .with_l1(l1),
    )
}

/// 一次按键：查询 → 列表行与高亮（界面这一帧要做的全部工作）。
fn keystroke(router: &CsRouter, matcher: &dyn Matcher, query: &str) -> usize {
    let cancel = Cancellation::default();
    let found = match router.workspace_symbols(query, &cancel) {
        Ok(r) => r,
        Err(_) => return 0,
    };
    let details: Vec<Arc<str>> = found
        .value
        .iter()
        .map(|s| {
            Arc::from(format!(
                "{} · {}",
                s.container,
                workspace_symbols::path_of(&s.target.resource)
            ))
        })
        .collect();
    let labels = SymbolLabels {
        approximate: workspace_symbols::is_approximate(found.precision)
            .then(|| Arc::from("近似结果：语义索引构建中")),
    };
    let picks = workspace_symbols::picks(&found.value, &details, query, matcher, &labels);
    picks.rows.len()
}

fn bench_group(c: &mut Criterion, label: &str, root: &Path, files: &[WorkspacePath]) {
    let router = router(root, files);
    let matcher = FuzzyMatcher::default();
    // 逐字键入全部按键的 p50 / p99 / max（预算判定用 criterion 的分布，这里只是便于人读）。
    let mut samples: Vec<f64> = Vec::new();
    for _ in 0..20 {
        for q in KEYSTROKES {
            let t = Instant::now();
            black_box(keystroke(&router, &matcher, q));
            samples.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |p: f64| samples[((samples.len() as f64 - 1.0) * p) as usize];
    eprintln!(
        "workspace_symbols/{label}: p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms（{} 次按键）",
        at(0.50),
        at(0.99),
        samples[samples.len() - 1],
        samples.len()
    );

    let mut group = c.benchmark_group("workspace_symbols");
    if label == "large" {
        group.sample_size(20);
        group.measurement_time(Duration::from_secs(20));
    }
    let mut k = 0usize;
    group.bench_function(format!("keystroke_{label}"), |b| {
        b.iter(|| {
            k = (k + 1) % KEYSTROKES.len();
            black_box(keystroke(&router, &matcher, KEYSTROKES[k]))
        })
    });
    group.finish();
}

fn bench_workspace_symbols(c: &mut Criterion) {
    let (root, files) = synthetic_fixture();
    bench_group(c, "synthetic_1k", &root, &files);
    let _ = std::fs::remove_dir_all(&root);

    if let Some(project) = UnityProject::Large.path() {
        let (root, files) = workspace_files(&project);
        bench_group(c, "large", &root, &files);
    } else {
        eprintln!(
            "已跳过：workspace_symbols/keystroke_large 需要环境变量 {}（大型 Unity 项目）",
            UnityProject::Large.env_var()
        );
    }
}

criterion_group!(benches, bench_workspace_symbols);
criterion_main!(benches);
