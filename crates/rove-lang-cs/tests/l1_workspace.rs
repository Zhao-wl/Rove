//! #37 入口集成测试：#15 工作区扫描 → L1 构建 → 工作区符号搜索 → 类型跳转 → 文件事件增量更新。
//!
//! 装配方式与正式接线一致：文件列表、文件事件都来自 `rove_workspace::Folder`（L1 不遍历目录、
//! 不读 `.gitignore`），由这里的小适配函数转成 L1 的输入。真实 Unity 项目的用例在没有设置
//! `ROVE_TEST_UNITY_MEDIUM` / `ROVE_TEST_UNITY_LARGE` 时跳过。

use rove_base::{Cancellation, Line, Position, Resource, Utf8Column, WorkspacePath};
use rove_lang_cs::l1::{L1Change, L1Index, L1Options, L1State};
use rove_lang_cs::query::Precision;
use rove_test_support::{require_unity_project, UnityProject};
use rove_workspace::{default_settings, EntryKind, FileEvent, Folder, Workspace, WorkspaceConfig};
use std::time::{Duration, Instant};

fn config() -> WorkspaceConfig {
    WorkspaceConfig::from_settings(&default_settings()).expect("registered workspace defaults")
}

/// 装配处的适配：分页取出工作区的全部文件。
fn workspace_files(ws: &dyn Workspace) -> Vec<WorkspacePath> {
    let mut out = Vec::new();
    const PAGE: usize = 4096;
    loop {
        let page = ws.files(out.len()..out.len() + PAGE).expect("files page");
        let n = page.items.len();
        out.extend(
            page.items
                .iter()
                .filter(|e| matches!(e.kind, EntryKind::File))
                .map(|e| e.path.clone()),
        );
        if n < PAGE {
            return out;
        }
    }
}

/// 装配处的适配：#15 的文件事件 → L1 的变化。`RescanRequired` 返回 None（调用方改走 build）。
fn to_l1(events: &[FileEvent]) -> Option<Vec<L1Change>> {
    let mut out = Vec::new();
    for e in events {
        out.push(match e {
            FileEvent::Created(p) | FileEvent::Changed(p) => L1Change::Upserted(p.clone()),
            FileEvent::Removed(p) => L1Change::Removed(p.clone()),
            FileEvent::Renamed { from, to } => L1Change::Renamed {
                from: from.clone(),
                to: to.clone(),
            },
            FileEvent::RescanRequired => return None,
        });
    }
    Some(out)
}

fn position_of(text: &str, needle: &str) -> Position {
    let offset = text.find(needle).expect("needle");
    let line = text[..offset].matches('\n').count();
    let col = offset - text[..offset].rfind('\n').map_or(0, |i| i + 1);
    Position {
        line: Line(line as u64),
        column: Utf8Column(col as u64),
    }
}

#[test]
fn scan_workspace_then_search_symbols_then_jump_to_type() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write(
        "Assets/Scripts/Player.cs",
        "namespace Game.Actors\n{\n    public partial class PlayerController { }\n}\n",
    );
    write(
        "Assets/Scripts/Player.Move.cs",
        "namespace Game.Actors { partial class PlayerController { public void Move() {} } }\n",
    );
    let use_site =
        "using Game.Actors;\nnamespace Game.UI\n{\n    class Hud { PlayerController target; }\n}\n";
    write("Assets/Scripts/Hud.cs", use_site);
    write("Assets/readme.md", "# not code\n");
    // files.exclude 默认排除 .git 等；这个文件不应进入 L1（工作区不给出）。
    write(".git/Hidden.cs", "class Hidden {}\n");

    let folder = Folder::scan_root(root, config().without_watching()).expect("scan");
    let files = workspace_files(folder.as_ref());
    assert!(files.iter().all(|p| !p.as_str().starts_with(".git/")));

    let cache = tempfile::tempdir().unwrap();
    let index = L1Index::new(L1Options::new(folder.root()).with_cache_dir(cache.path()));
    let cancel = Cancellation::default();
    let report = index.build_and_save(files.clone(), &cancel).unwrap();
    assert_eq!((report.files, report.parsed), (3, 3), "{report:?}");
    assert_eq!(index.progress().state, L1State::Ready);
    assert!(index.find_type("Hidden").is_none());

    // 符号搜索：partial 类型是一个符号、两个位置。
    let hits = index.workspace_symbols("plc", 20, &cancel).unwrap();
    assert_ne!(hits.precision, Precision::Exact);
    let player = hits
        .value
        .iter()
        .find(|s| &*s.name == "PlayerController")
        .expect("found by fuzzy query");
    assert_eq!(&*player.container, "Game.Actors");
    assert_eq!(player.locations.len(), 2);

    // 类型跳转：Hud.cs 里的 PlayerController 经 using 解析到 Game.Actors.PlayerController。
    let hud = WorkspacePath::new("Assets/Scripts/Hud.cs").unwrap();
    let at = position_of(use_site, "PlayerController target");
    let resolved = index
        .resolve_type(&hud, at, "PlayerController", &cancel)
        .unwrap();
    assert_eq!(resolved.precision, Precision::Approximate);
    assert_eq!(resolved.value.len(), 1);
    let targets: Vec<String> = resolved.value[0]
        .locations
        .iter()
        .map(|t| match &t.resource {
            Resource::Workspace(p) => format!("{}:{}", p.as_str(), t.selection.start().line.0),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        targets,
        [
            "Assets/Scripts/Player.Move.cs:0",
            "Assets/Scripts/Player.cs:2"
        ]
    );

    // 文件事件 → 增量更新：只解析变化的那个文件。
    write(
        "Assets/Scripts/Player.Move.cs",
        "namespace Game.Actors { partial class PlayerController { } class Mover { } }\n",
    );
    let changes = to_l1(&[FileEvent::Changed(
        WorkspacePath::new("Assets/Scripts/Player.Move.cs").unwrap(),
    )])
    .unwrap();
    let report = index.apply_changes(&changes, &cancel).unwrap();
    assert_eq!(report.parsed, 1);
    assert!(index.find_type("Game.Actors.Mover").is_some());
    assert!(index
        .member_candidates("Move", &cancel)
        .unwrap()
        .value
        .is_empty());
    index.save().unwrap();

    // 重新打开项目：一个文件都不重扫。
    let reopened = L1Index::new(L1Options::new(folder.root()).with_cache_dir(cache.path()));
    let report = reopened.build(files, &cancel).unwrap();
    assert_eq!(report.parsed, 0, "{report:?}");
    assert_eq!(reopened.stats().files_parsed_total, 0);
    assert!(reopened.find_type("Game.Actors.Mover").is_some());
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / 1048576.0
}

/// 真实 Unity 项目：首次构建、再次打开、增量更新、符号搜索。数字打到 stderr
/// （`-- --nocapture` 可见）；正式性能数字用 release 跑 `examples/l1_bench.rs`。
fn unity_round_trip(project: std::path::PathBuf) {
    let folder = Folder::scan_root(&project, config().without_watching()).expect("scan");
    let files = workspace_files(folder.as_ref());
    let cache = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let options = L1Options::new(folder.root()).with_cache_dir(cache.path());

    let index = L1Index::new(options.clone());
    let t = Instant::now();
    let first = index.build_and_save(files.clone(), &cancel).unwrap();
    let stats = index.stats();
    eprintln!(
        "L1 首次构建：{} 个 .cs，解析 {}，{:?}；{} 条声明，{} 个符号，估算常驻 {:.1} MB",
        first.files,
        first.parsed,
        t.elapsed(),
        stats.declarations,
        stats.symbols,
        mb(stats.approx_heap_bytes)
    );
    assert!(first.files > 0 && first.parsed + first.failed == first.files);
    assert!(
        stats.declarations > first.files,
        "Unity 项目平均每个文件不止一条声明"
    );

    let reopened = L1Index::new(options);
    let t = Instant::now();
    let second = reopened.build(files.clone(), &cancel).unwrap();
    eprintln!("L1 再次打开（缓存）：{:?}，{second:?}", t.elapsed());
    assert_eq!(second.parsed, 0);
    assert_eq!(reopened.stats().declarations, stats.declarations);

    // 增量：不改用户的项目文件。先移除再加回一个最大的文件，加回走的就是“文件变了”的完整路径
    // （读盘 + 哈希 + 解析 + 并入），缓存里没有它，不会被哈希短路。
    let victim = files
        .iter()
        .filter(|p| p.as_str().to_ascii_lowercase().ends_with(".cs"))
        .max_by_key(|p| std::fs::metadata(folder.root().join(p.as_str())).map_or(0, |m| m.len()))
        .unwrap()
        .clone();
    reopened
        .apply_changes(&[L1Change::Removed(victim.clone())], &cancel)
        .unwrap();
    let t = Instant::now();
    let inc = reopened
        .apply_changes(&[L1Change::Upserted(victim.clone())], &cancel)
        .unwrap();
    eprintln!(
        "L1 单文件增量（最大的文件 {}）：{:?}",
        victim.as_str(),
        t.elapsed()
    );
    assert_eq!(inc.parsed, 1);
    assert_eq!(reopened.stats().declarations, stats.declarations);

    let mut worst = Duration::ZERO;
    for q in [
        "M", "Mo", "Mon", "Mono", "MonoB", "MonoBe", "pc", "Init", "Update",
    ] {
        let t = Instant::now();
        let r = reopened.workspace_symbols(q, 100, &cancel).unwrap();
        worst = worst.max(t.elapsed());
        assert_ne!(r.precision, Precision::Exact);
    }
    eprintln!("L1 符号搜索最慢一次（调试构建）：{worst:?}");
    let init = reopened.member_candidates("Init", &cancel).unwrap();
    assert_eq!(init.precision, Precision::Approximate);
}

#[test]
fn unity_medium_round_trip() {
    let project = require_unity_project!(UnityProject::Medium);
    unity_round_trip(project);
}

#[test]
fn unity_large_round_trip() {
    let project = require_unity_project!(UnityProject::Large);
    unity_round_trip(project);
}
