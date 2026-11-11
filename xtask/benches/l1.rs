//! #37 L1 快速声明索引基准。
//!
//! 合成工程（CI 上也能跑）：1,000 个 Unity 风格的 `.cs`，创建在系统临时目录里，不写仓库。
//! - `l1/build_synthetic_1k`：无缓存全量构建（读盘 + tree-sitter 声明提取 + 并入）
//! - `l1/incremental_one_file`：单文件变化后的增量更新（读盘 + 哈希 + 解析 + 并入）
//! - `l1/workspace_symbols_keystroke`：一次按键的工作区符号搜索（打分 + 排序 + 取前 100）
//!
//! 大型 Unity 项目（`ROVE_TEST_UNITY_LARGE`，未设置时跳过）：`build_large`（热缓存首次构建）、
//! `incremental_large`、`workspace_symbols_large`。预算见 `perf/budgets.toml` 的 `l1_*`。
//! 文件列表来自 #15 的 `Folder`，与正式装配一致；只调用公开接口。

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use rove_base::{Cancellation, WorkspacePath};
use rove_lang_cs::l1::{L1Change, L1Index, L1Options};
use rove_test_support::UnityProject;
use rove_workspace::{default_settings, EntryKind, Folder, Workspace, WorkspaceConfig};

fn synthetic_fixture() -> (PathBuf, Vec<WorkspacePath>) {
    let root = std::env::temp_dir().join(format!("rove-l1-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut files = Vec::new();
    for i in 0..1000 {
        let rel = format!("Assets/Scripts/G{:02}/Behaviour{i:04}.cs", i % 40);
        let path = root.join(&rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = format!(
            "using UnityEngine;\nusing System.Collections.Generic;\n\nnamespace Game.Feature{g}\n{{\n    public partial class Behaviour{i:04} : MonoBehaviour\n    {{\n",
            g = i % 40
        );
        for m in 0..12 {
            text.push_str(&format!(
                "        [SerializeField] private int field{m};\n        public int Value{m} {{ get; set; }}\n        public void Method{m}(int a, string b)\n        {{\n            var list = new List<int>();\n            for (int k = 0; k < a; k++) {{ list.Add(k * field{m}); }}\n            Debug.Log(b + list.Count);\n        }}\n"
            ));
        }
        text.push_str("        private void Awake() {}\n        private void Update() {}\n        public class Nested { public enum State { Idle, Run } }\n    }\n}\n");
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

const KEYSTROKES: [&str; 8] = ["M", "Me", "Met", "Meth", "Method1", "B", "Beh", "pc"];

fn bench_group(c: &mut Criterion, label: &str, root: &Path, files: &[WorkspacePath]) {
    let cancel = Cancellation::default();
    let mut group = c.benchmark_group("l1");
    if label == "large" {
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(30));
    }
    group.bench_function(format!("build_{label}"), |b| {
        b.iter(|| {
            let index = L1Index::new(L1Options::new(root));
            black_box(index.build(files.iter().cloned(), &cancel).unwrap())
        })
    });

    let index = L1Index::new(L1Options::new(root));
    index.build(files.iter().cloned(), &cancel).unwrap();
    let probe = files
        .iter()
        .find(|p| p.as_str().to_ascii_lowercase().ends_with(".cs"))
        .unwrap()
        .clone();
    let name = if label == "large" {
        "incremental_large".to_owned()
    } else {
        "incremental_one_file".to_owned()
    };
    group.bench_function(name, |b| {
        b.iter(|| {
            // 先移除（不计入的部分很小），再加回：加回走“文件变了”的完整路径。
            index
                .apply_changes(&[L1Change::Removed(probe.clone())], &cancel)
                .unwrap();
            black_box(
                index
                    .apply_changes(&[L1Change::Upserted(probe.clone())], &cancel)
                    .unwrap(),
            )
        })
    });
    let name = if label == "large" {
        "workspace_symbols_large".to_owned()
    } else {
        "workspace_symbols_keystroke".to_owned()
    };
    let mut k = 0usize;
    group.bench_function(name, |b| {
        b.iter(|| {
            k = (k + 1) % KEYSTROKES.len();
            black_box(
                index
                    .workspace_symbols(KEYSTROKES[k], 100, &cancel)
                    .unwrap(),
            )
        })
    });
    group.finish();
}

fn bench_l1(c: &mut Criterion) {
    let (root, files) = synthetic_fixture();
    bench_group(c, "synthetic_1k", &root, &files);
    let _ = std::fs::remove_dir_all(&root);

    if let Some(project) = UnityProject::Large.path() {
        let (root, files) = workspace_files(&project);
        bench_group(c, "large", &root, &files);
    } else {
        eprintln!(
            "已跳过：l1/build_large 等需要环境变量 {}（大型 Unity 项目）",
            UnityProject::Large.env_var()
        );
    }
}

criterion_group!(benches, bench_l1);
criterion_main!(benches);
