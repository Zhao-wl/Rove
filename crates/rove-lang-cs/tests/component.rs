//! #43 入口集成测试：索引组件的发现规则与版本握手。
//!
//! - 组件缺失 → “未安装”，不重复报失败；已安装但清单与本 Rove 不兼容（组件太旧 / 太新）→ “不兼容”，
//!   不重试，降级到 L1/L0（缓存索引仍有效时继续提供，不丢掉它）；
//! - 常驻进程的 `hello` 协议区间与本端没有交集 → 放弃该进程、提示更新，不硬跑；有交集（组件更新但仍兼容）→ 正常；
//! - 全程不发起网络请求：本文件没有任何网络依赖，组件发现只读本地目录。
//!
//! 运行：`cargo test -p rove-lang-cs --test component`。
use rove_base::Cancellation;
use rove_lang_cs::lifecycle::{
    component::{Roots, EXE_FILE, MANIFEST_FILE},
    layout::ProjectCache,
    locate_in,
    log::IndexLog,
    process::{BuildRequest, IndexerComponent},
    resident::{IncrementalIndexer, ResidentEvent, SessionRequest},
    testing::FixtureIndex,
    BuildFailure, Direction, IndexBuilder, IndexLifecycle, IndexState, IndexStatus,
    LifecycleConfig, ProcessBuilder, ProcessResident, Progress, SourceChange,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(20);

struct Project {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    root: PathBuf,
    cache_root: PathBuf,
}

fn project() -> Project {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let root = a.path().join("Game");
    fs::create_dir_all(root.join("Assets/Scripts")).unwrap();
    fs::write(root.join("Game.csproj"), "<Project/>").unwrap();
    fs::write(root.join("Assets/Scripts/A.cs"), "class A {}\n").unwrap();
    let cache_root = b.path().join("index");
    Project {
        _dirs: (a, b),
        root,
        cache_root,
    }
}

impl Project {
    fn config(&self) -> LifecycleConfig {
        LifecycleConfig::new(true, Ok(self.cache_root.clone()))
    }
    fn cache(&self) -> ProjectCache {
        ProjectCache::new(&self.cache_root, &self.root)
    }
}

fn settled(s: &IndexStatus) -> bool {
    !matches!(s.state, IndexState::Checking | IndexState::Building { .. })
}

fn manifest(version: &str, protocol: u32, min: u32, format: u32) -> String {
    format!(
        r#"{{"component":"indexer-cs","version":"{version}","protocol":{protocol},"protocol_min":{min},"index_format":{format}}}"#
    )
}

/// 在 `<root>\<dir>\` 里放一个假组件（空的 exe + 清单）。
fn install(root: &Path, dir: &str, manifest: &str) {
    let d = root.join(dir);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(EXE_FILE), b"").unwrap();
    fs::write(d.join(MANIFEST_FILE), manifest).unwrap();
}

fn lifecycle_with_roots(p: &Project, roots: Roots) -> IndexLifecycle {
    let builder = Arc::new(ProcessBuilder::with_locate(move || locate_in(&roots)));
    IndexLifecycle::start(p.root.clone(), p.config(), builder)
}

fn roots_in(dir: &Path) -> Roots {
    Roots {
        env_override: None,
        dirs: vec![dir.to_owned()],
    }
}

#[test]
fn nothing_installed_is_not_installed_and_stays_off_the_failure_path() {
    let p = project();
    let empty = tempfile::tempdir().unwrap();
    let lifecycle = lifecycle_with_roots(&p, roots_in(empty.path()));
    let status = lifecycle.wait_until(WAIT, settled);
    assert_eq!(status.state, IndexState::NotInstalled);
    assert!(status.info.component_issue.is_none());
    assert!(lifecycle.snapshot().is_none());
}

#[test]
fn incompatible_component_degrades_without_retrying() {
    for (name, text, direction) in [
        // 组件太新：索引格式比 Rove 读得懂的高
        (
            "9.0.0",
            manifest("9.0.0", 1, 1, 3),
            Direction::ComponentTooNew,
        ),
        // 组件太新：只会说更高的协议
        (
            "3.0.0",
            manifest("3.0.0", 4, 3, 2),
            Direction::ComponentTooNew,
        ),
        // 组件太旧：索引格式更低
        (
            "0.9.0",
            manifest("0.9.0", 1, 1, 1),
            Direction::ComponentTooOld,
        ),
    ] {
        let p = project();
        let components = tempfile::tempdir().unwrap();
        install(components.path(), name, &text);
        let lifecycle = lifecycle_with_roots(&p, roots_in(components.path()));
        let status = lifecycle.wait_until(WAIT, settled);
        let IndexState::Incompatible(why) = &status.state else {
            panic!("{name}: {status:?}");
        };
        assert_eq!(why.direction, direction, "{name}");
        assert_eq!(
            why.component_version.as_deref(),
            Some(format!("{name}.0").as_str())
        );
        // 降级到 L1/L0：没有 L2 快照，不崩溃、不卡住
        assert!(lifecycle.snapshot().is_none());
        let log = fs::read_to_string(p.cache().log_path()).unwrap();
        assert!(
            log.contains("component incompatible") || log.contains("incompatible"),
            "{log}"
        );
        assert!(!log.contains("build attempt 2"), "不兼容不应重试：{log}");
    }
}

#[test]
fn a_compatible_version_is_preferred_over_an_incompatible_newer_one() {
    let components = tempfile::tempdir().unwrap();
    install(components.path(), "1.2.0", &manifest("1.2.0", 1, 1, 2));
    install(components.path(), "2.0.0", &manifest("2.0.0", 1, 1, 3));
    let found = locate_in(&roots_in(components.path())).unwrap();
    assert_eq!(found.version, Some([1, 2, 0, 0]));
    assert!(found.program.ends_with(Path::new("1.2.0").join(EXE_FILE)));
}

/// 写缓存索引的假构建器；常驻进程换成 `hello` 里声明了指定协议区间的脚本。
struct CachedThenResident {
    resident: Arc<ProcessResident>,
}

impl IndexBuilder for CachedThenResident {
    fn build(
        &self,
        request: &BuildRequest,
        progress: &mut dyn FnMut(Progress),
        _log: &IndexLog,
        _cancel: &Cancellation,
    ) -> Result<(), BuildFailure> {
        progress(Progress {
            percent: 50,
            files: Some(1),
        });
        FixtureIndex::for_project(&request.project_root, &["Assets/Scripts/A.cs"], [9; 32])
            .write(&request.output)
            .map_err(|e| BuildFailure::InvalidOutput(e.to_string()))
    }
    fn incremental(&self) -> Option<Arc<dyn IncrementalIndexer>> {
        Some(self.resident.clone())
    }
}

/// 一个先打印 `hello` 再等一会儿的批处理文件，充当“索引进程”（忽略收到的参数与 stdin）。
fn hello_script(dir: &Path, protocol: u32, protocol_min: u32) -> IndexerComponent {
    let path = dir.join("fake-indexer.bat");
    fs::write(
        &path,
        format!(
            "@echo off\r\necho {{\"jsonrpc\":\"2.0\",\"method\":\"hello\",\"params\":{{\"protocol\":{protocol},\"protocol_min\":{protocol_min},\"indexer_version\":\"7.0.0.0\"}}}}\r\nping -n 4 127.0.0.1 >nul\r\n"
        ),
    )
    .unwrap();
    IndexerComponent {
        program: path,
        prefix: vec![],
        version: Some([7, 0, 0, 0]),
        protocol: None,
    }
}

fn session_request(p: &Project) -> SessionRequest {
    SessionRequest {
        project_root: p.root.clone(),
        base: p.root.join("base.rvcsidx"),
        journal: p.root.join("base.journal"),
        options: Default::default(),
    }
}

#[cfg(windows)]
#[test]
fn hello_with_disjoint_protocol_range_is_rejected_and_overlapping_range_is_accepted() {
    let p = project();
    let scripts = tempfile::tempdir().unwrap();
    let log = Arc::new(IndexLog::new(scripts.path().join("log.txt")));
    // 组件只会说 8..9，Rove 说 1..1 → 组件太新
    let too_new = hello_script(scripts.path(), 9, 8);
    let resident = ProcessResident::new(move || Ok(too_new.clone()));
    let (tx, rx) = mpsc::channel();
    let mut session = resident
        .start(
            &session_request(&p),
            Box::new(move |e| {
                let _ = tx.send(e);
            }),
            log.clone(),
        )
        .unwrap();
    let event = rx.recv_timeout(WAIT).unwrap();
    let ResidentEvent::Incompatible(why) = event else {
        panic!("{event:?}");
    };
    assert_eq!(why.direction, Direction::ComponentTooNew);
    assert_eq!(why.component_version.as_deref(), Some("7.0.0.0"));
    session.shutdown();
    let text = fs::read_to_string(log.path()).unwrap();
    assert!(text.contains("component too new"), "{text}");

    // 组件支持 1..9（更新，但仍然能说 Rove 的协议）→ 不会报不兼容
    let newer = hello_script(scripts.path(), 9, 1);
    let resident = ProcessResident::new(move || Ok(newer.clone()));
    let (tx, rx) = mpsc::channel();
    let mut session = resident
        .start(
            &session_request(&p),
            Box::new(move |e| {
                let _ = tx.send(e);
            }),
            log,
        )
        .unwrap();
    // 脚本几秒后退出：期间收到的第一个事件只能是退出，而不是不兼容
    let event = rx.recv_timeout(WAIT).unwrap();
    assert!(matches!(event, ResidentEvent::Exited { .. }), "{event:?}");
    session.shutdown();
}

#[cfg(windows)]
#[test]
fn incompatible_resident_keeps_the_valid_index_and_reports_the_component_issue() {
    let p = project();
    let scripts = tempfile::tempdir().unwrap();
    let component = hello_script(scripts.path(), 9, 8);
    let builder = Arc::new(CachedThenResident {
        resident: Arc::new(ProcessResident::new(move || Ok(component.clone()))),
    });
    let lifecycle = IndexLifecycle::start(p.root.clone(), p.config(), builder);
    let ready = lifecycle.wait_until(WAIT, |s| s.state == IndexState::Ready);
    assert_eq!(ready.state, IndexState::Ready);
    // 源文件变化 → 拉起常驻进程 → hello 协议对不上
    lifecycle.files_changed(&[SourceChange::Changed("Assets/Scripts/A.cs".into())]);
    let status = lifecycle.wait_until(WAIT, |s| s.info.component_issue.is_some());
    let why = status.info.component_issue.expect("component issue");
    assert_eq!(why.direction, Direction::ComponentTooNew);
    // 索引仍然可用（不是“不兼容”状态），不会卡在“更新中”
    assert!(lifecycle.snapshot().is_some());
    let status = lifecycle.wait_until(WAIT, |s| !matches!(s.state, IndexState::Updating { .. }));
    assert_eq!(status.state, IndexState::Ready);
}
