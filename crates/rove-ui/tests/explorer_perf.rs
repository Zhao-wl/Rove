//! #23 性能：展开一个有 1 万个子项的目录 < 50 ms（验收标准）。
//!
//! 数据通路与资源管理器完全一致：真实的 #15 `Folder` → `ExplorerSource`（排序 + 紧凑文件夹探测）→
//! #18 的 `ListWidget` 树（展开、取可见行）。不需要窗口。
//!
//! - 合成目录：1 万个文件，以及 1 万个各含一个文件的子目录（紧凑文件夹探测的最坏情况）。
//! - 真实项目：设置 `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE` 时，找出子项最多的几个目录逐个展开
//!   （路径不写进仓库；未设置则跳过）。
//!
//! 数字打印到 stderr：`cargo test -p rove-ui --release --test explorer_perf -- --nocapture`。
//! 调试构建只检查功能（宽松上限），不判定 50 ms。
use rove_test_support::{require_unity_project, UnityProject};
use rove_ui::{
    explorer::model::{ExplorerSettings, ExplorerSource},
    list_widget::{
        options::ListOptions,
        tree::TreeModel,
        widget::{ListWidget, RowHeight},
    },
    lists::ItemId,
    skeleton::appearance,
};
use rove_workspace::{Folder, Workspace, WorkspaceConfig};
use std::{path::Path, sync::Arc, time::Instant};

const BUDGET_MS: f64 = 50.0;

fn widget(root: &Path) -> (Arc<Folder>, Arc<ExplorerSource>, ListWidget) {
    let folder = Folder::scan_root(root, WorkspaceConfig::default().without_watching()).unwrap();
    let source = Arc::new(ExplorerSource::new(
        folder.clone(),
        ExplorerSettings::default(),
        Arc::new(appearance::strings().unwrap()),
    ));
    let widget = ListWidget::tree(
        TreeModel::new(source.clone()),
        RowHeight::Fixed(24.0),
        ListOptions::default(),
    );
    (folder, source, widget)
}

/// 展开 `id` 到第一屏行都取到为止的耗时（毫秒），然后折叠回去。返回第一次（冷）的耗时。
fn expand_ms(widget: &mut ListWidget, id: &str, runs: usize) -> (f64, usize) {
    let id = ItemId(id.into());
    let mut samples = Vec::new();
    let mut rows = 0;
    for _ in 0..runs {
        let start = Instant::now();
        widget.expand_id(&id, true).unwrap();
        let visible = widget.display_rows(0..60);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        assert!(!visible.is_empty());
        rows = widget.len();
        widget.expand_id(&id, false).unwrap();
    }
    // 第一次展开才向工作区要子项（之后折叠不会丢弃已加载的子项），所以以第一次（冷）为准。
    (samples[0], rows)
}

fn check(name: &str, ms: f64) {
    eprintln!("explorer perf: {name}: {ms:.2} ms (预算 {BUDGET_MS} ms)");
    // 调试构建比发布慢一个数量级，只在发布构建判定预算。
    let limit = if cfg!(debug_assertions) {
        2000.0
    } else {
        BUDGET_MS
    };
    assert!(ms < limit, "{name}: {ms:.2} ms >= {limit} ms");
}

#[test]
fn expanding_a_directory_with_10000_files_is_fast() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Big")).unwrap();
    for i in 0..10_000 {
        std::fs::write(dir.path().join(format!("Big/File{i:05}.cs")), "").unwrap();
    }
    let (_folder, _source, mut widget) = widget(dir.path());
    let (ms, rows) = expand_ms(&mut widget, "Big", 7);
    assert_eq!(rows, 10_001);
    check("1 万个文件", ms);
}

#[test]
fn expanding_a_directory_with_10000_folders_is_fast() {
    // 每个子目录只含一个文件：每个子目录都要探测一次“是否是单子文件夹链”。
    let dir = tempfile::tempdir().unwrap();
    for i in 0..10_000 {
        let sub = dir.path().join(format!("Many/Dir{i:05}"));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("f.cs"), "").unwrap();
    }
    let (_folder, _source, mut widget) = widget(dir.path());
    let (ms, rows) = expand_ms(&mut widget, "Many", 5);
    assert_eq!(rows, 10_001);
    check("1 万个文件夹（含单链探测）", ms);
}

fn unity(project: UnityProject, label: &str) {
    let root = require_unity_project!(project);
    let (folder, source, mut widget) = widget(&root);
    // 子项最多的目录。
    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    let files = folder.files(0..usize::MAX).unwrap();
    eprintln!(
        "explorer perf: {label}项目：{} 个文件，扫描完成 = {}",
        files.items.len(),
        files.complete
    );
    for entry in files.items.iter() {
        if let Some((parent, _)) = entry.path.as_str().rsplit_once('/') {
            *counts.entry(parent.to_owned()).or_default() += 1;
        }
    }
    let mut biggest: Vec<_> = counts.into_iter().collect();
    biggest.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    for (dir, count) in biggest.into_iter().take(3) {
        // 逐级展开到该目录（祖先的展开不计入），再计时最后一级。
        let Some(plan) = rove_base::WorkspacePath::new(&dir)
            .ok()
            .and_then(|p| source.reveal_plan(&p))
        else {
            continue;
        };
        for id in &plan.expand {
            let _ = widget.expand_id(id, true);
        }
        let (ms, rows) = expand_ms(&mut widget, &plan.target.0, 5);
        eprintln!("explorer perf: {label}项目最大目录之一（{count} 个文件，{rows} 行）");
        check(&format!("{label}项目 {count} 个子项"), ms);
    }
}

#[test]
fn unity_medium_largest_directories_expand_within_budget() {
    unity(UnityProject::Medium, "中型");
}

#[test]
fn unity_large_largest_directories_expand_within_budget() {
    unity(UnityProject::Large, "大型");
}
