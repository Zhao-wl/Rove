//! #36 入口集成测试：打开项目 → 检查缓存 → 预读 → 就绪 → 文件变更 → 标记过期，
//! 四类失效的自动重建、原子替换、索引进程失败的降级、设置与缓存淘汰。
//! 运行：`cargo test -p rove-lang-cs --test lifecycle`。
use rove_base::Cancellation;
use rove_lang_cs::lifecycle::{
    layout::ProjectCache,
    log::IndexLog,
    meta::ProjectMeta,
    process::{BuildRequest, IndexerComponent},
    testing::FixtureIndex,
    BuildFailure, CachePolicy, FailureKind, IndexBuilder, IndexLifecycle, IndexState, IndexStatus,
    LifecycleConfig, PrefetchState, ProcessBuilder, Progress, SourceChange,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(20);

/// 假索引进程：按磁盘内容写一份最小索引。
#[derive(Default)]
struct FakeIndexer {
    builds: AtomicUsize,
    delay_ms: AtomicUsize,
    failures: Mutex<Vec<BuildFailure>>,
}

fn sources(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.join("Assets")];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "cs") {
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
    }
    out
}

impl IndexBuilder for FakeIndexer {
    fn build(
        &self,
        request: &BuildRequest,
        progress: &mut dyn FnMut(Progress),
        log: &IndexLog,
        cancel: &Cancellation,
    ) -> Result<(), BuildFailure> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        log.line("fake indexer");
        if let Some(f) = self.failures.lock().unwrap().pop() {
            return Err(f);
        }
        progress(Progress {
            percent: 40,
            files: Some(2),
        });
        for _ in 0..self.delay_ms.load(Ordering::SeqCst) / 10 {
            if cancel.check().is_err() {
                return Err(BuildFailure::Cancelled);
            }
            thread::sleep(Duration::from_millis(10));
        }
        let files = sources(&request.project_root);
        let refs: Vec<&str> = files.iter().map(String::as_str).collect();
        FixtureIndex::for_project(&request.project_root, &refs, [9; 32])
            .write(&request.output)
            .map_err(|e| BuildFailure::InvalidOutput(e.to_string()))
    }
}

struct Project {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    root: PathBuf,
    cache_root: PathBuf,
}

fn project(files: usize) -> Project {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let root = a.path().join("Game");
    fs::create_dir_all(root.join("Assets/Scripts")).unwrap();
    fs::write(root.join("Game.csproj"), "<Project/>").unwrap();
    for i in 0..files {
        fs::write(
            root.join(format!("Assets/Scripts/C{i}.cs")),
            format!("class C{i} {{}}\n"),
        )
        .unwrap();
    }
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
    fn start(&self, builder: &Arc<FakeIndexer>) -> IndexLifecycle {
        IndexLifecycle::start(self.root.clone(), self.config(), builder.clone())
    }
}

fn ready(s: &IndexStatus) -> bool {
    s.state == IndexState::Ready
}
fn settled(s: &IndexStatus) -> bool {
    !matches!(s.state, IndexState::Checking | IndexState::Building { .. })
}

/// 第一次打开构建，第二次打开直接用缓存。
fn built(p: &Project, builder: &Arc<FakeIndexer>) {
    let first = p.start(builder);
    assert!(ready(&first.wait_until(WAIT, ready)));
    drop(first);
}

#[test]
fn open_check_prefetch_ready_then_mark_stale() {
    let p = project(3);
    let builder = Arc::new(FakeIndexer::default());
    let first = p.start(&builder);
    let status = first.wait_until(WAIT, ready);
    assert_eq!(status.state, IndexState::Ready);
    assert_eq!(builder.builds.load(Ordering::SeqCst), 1);
    assert_eq!(status.info.files, 3);
    assert_eq!(status.info.prefetch, PrefetchState::Skipped);
    assert!(status.info.built_at_ms.is_some());
    drop(first);

    // 第二次打开：缓存通过有效性检查，立即可用，不再构建；后台顺序预读完成。
    let second = p.start(&builder);
    let status = second.wait_until(WAIT, |s| {
        ready(s) && matches!(s.info.prefetch, PrefetchState::Done { .. })
    });
    assert!(ready(&status), "{status:?}");
    assert!(second.snapshot().unwrap().trusted);
    assert_eq!(builder.builds.load(Ordering::SeqCst), 1);
    assert_eq!(status.info.cache_dir.as_deref(), Some(p.cache().dir()));
    assert!(status.info.cache_bytes > 0);
    let log = fs::read_to_string(p.cache().log_path()).unwrap();
    assert!(
        log.contains("is valid") && log.contains("prefetched"),
        "{log}"
    );

    // 文件变化：标记过期（#35 合并前），不重建。
    fs::write(p.root.join("Assets/Scripts/C1.cs"), "class C1 { int x; }").unwrap();
    second.files_changed(&[SourceChange::Changed("Assets/Scripts/C1.cs".into())]);
    assert!(second.is_stale("Assets/Scripts/C1.cs"));
    assert!(!second.is_stale("Assets/Scripts/C0.cs"));
    assert_eq!(second.status().info.stale_files, 1);
    assert_eq!(builder.builds.load(Ordering::SeqCst), 1);
}

fn rewrite_cached(p: &Project, f: impl FnOnce(&mut FixtureIndex)) {
    let cache = p.cache();
    let (_, path) = cache.generations().into_iter().next().unwrap();
    let files = sources(&p.root);
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let mut fixture = FixtureIndex::for_project(&p.root, &refs, [9; 32]);
    f(&mut fixture);
    fs::write(path, fixture.bytes()).unwrap();
}

fn expect_rebuild(p: &Project, builder: &Arc<FakeIndexer>, reason: &str) {
    let before = builder.builds.load(Ordering::SeqCst);
    let generation = p.cache().generations()[0].0;
    let lifecycle = p.start(builder);
    let status = lifecycle.wait_until(WAIT, |s| ready(s) && s.info.generation > generation);
    assert!(ready(&status), "{status:?}");
    assert_eq!(builder.builds.load(Ordering::SeqCst), before + 1);
    let log = fs::read_to_string(p.cache().log_path()).unwrap();
    assert!(log.contains(reason), "missing `{reason}` in log:\n{log}");
    assert_eq!(p.cache().generations().len(), 1, "old generation cleaned");
}

#[test]
fn format_version_change_rebuilds() {
    let p = project(2);
    let builder = Arc::new(FakeIndexer::default());
    built(&p, &builder);
    rewrite_cached(&p, |f| f.format_version = 3);
    expect_rebuild(&p, &builder, "format version 3");
}

#[test]
fn indexer_version_change_rebuilds() {
    let p = project(2);
    let builder = Arc::new(FakeIndexer::default());
    built(&p, &builder);
    rewrite_cached(&p, |f| f.indexer_version = [1, 0, 1, 0]);
    expect_rebuild(&p, &builder, "indexer version");
}

#[test]
fn project_config_change_rebuilds() {
    let p = project(2);
    let builder = Arc::new(FakeIndexer::default());
    built(&p, &builder);
    // 头部配置哈希与构建记录不一致（索引被别处替换）。
    rewrite_cached(&p, |f| f.config_hash = [1; 32]);
    expect_rebuild(&p, &builder, "project configuration changed");
    // csproj 内容变化（输入指纹不一致）。
    fs::write(
        p.root.join("Game.csproj"),
        "<Project><!-- new --></Project>",
    )
    .unwrap();
    expect_rebuild(&p, &builder, "project configuration changed");
    // 离线新增源文件。
    fs::write(p.root.join("Assets/Scripts/New.cs"), "class New {}").unwrap();
    expect_rebuild(&p, &builder, "project configuration changed");
}

#[test]
fn sampled_file_hash_mismatch_rebuilds_and_small_drift_only_marks_stale() {
    let p = project(20);
    let builder = Arc::new(FakeIndexer::default());
    built(&p, &builder);
    // 20 个文件全部抽查，改 1 个：命中率 95% ≥ 90%，只标记过期。
    fs::write(p.root.join("Assets/Scripts/C3.cs"), "class C3 { }").unwrap();
    let lifecycle = p.start(&builder);
    let status = lifecycle.wait_until(WAIT, |s| ready(s) && s.info.stale_files == 1);
    assert_eq!(status.info.stale_files, 1);
    assert!(lifecycle.is_stale("Assets/Scripts/C3.cs"));
    assert_eq!(builder.builds.load(Ordering::SeqCst), 1);
    drop(lifecycle);
    // 改 5 个：命中率 75%，全量重建。
    for i in 4..8 {
        fs::write(p.root.join(format!("Assets/Scripts/C{i}.cs")), "// changed").unwrap();
    }
    expect_rebuild(&p, &builder, "sampled file hashes: 15/20 match");
}

#[test]
fn old_index_stays_queryable_and_is_replaced_atomically() {
    let p = project(4);
    let builder = Arc::new(FakeIndexer::default());
    built(&p, &builder);
    let lifecycle = p.start(&builder);
    lifecycle.wait_until(WAIT, ready);
    let old = lifecycle.snapshot().unwrap();
    let old_path = old.path.clone();
    builder.delay_ms.store(400, Ordering::SeqCst);
    fs::write(p.root.join("Assets/Scripts/Extra.cs"), "class Extra {}").unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let lifecycle = Arc::new(lifecycle);
    // 查询线程：重建期间与切换瞬间一直能读到完整的文件表。
    let reader = {
        let (lifecycle, stop) = (lifecycle.clone(), stop.clone());
        thread::spawn(move || {
            let mut reads = 0;
            while !stop.load(Ordering::SeqCst) {
                let snapshot = lifecycle.snapshot().expect("never without an index");
                let index = snapshot.index();
                let n = index.files().len();
                assert!(n == 4 || n == 5);
                for i in 0..n as u32 {
                    assert!(index.file_path(i).unwrap().starts_with("Assets/Scripts/"));
                }
                reads += 1;
            }
            reads
        })
    };
    lifecycle.rebuild();
    let building = lifecycle.wait_until(WAIT, |s| matches!(s.state, IndexState::Building { .. }));
    assert!(matches!(
        building.state,
        IndexState::Building {
            rebuilding: true,
            ..
        }
    ));
    let status = lifecycle.wait_until(WAIT, |s| ready(s) && s.info.files == 5);
    assert_eq!(status.info.files, 5);
    stop.store(true, Ordering::SeqCst);
    assert!(reader.join().unwrap() > 0);
    // 旧快照的持有者仍然读旧的映射（旧世代的文件名可能已被 POSIX 语义删除，数据在解除映射前有效）；
    // 放手后旧世代不再留在缓存目录里。
    assert_eq!(old.index().files().len(), 4);
    assert_eq!(old.index().file_path(0), Some("Assets/Scripts/C0.cs"));
    drop(old);
    assert!(!old_path.exists());
    assert_eq!(p.cache().generations().len(), 1);
    assert_ne!(lifecycle.snapshot().unwrap().path, old_path);
}

fn process_lifecycle(p: &Project, component: Option<IndexerComponent>) -> IndexLifecycle {
    let builder = Arc::new(ProcessBuilder::new(move || component.clone()));
    IndexLifecycle::start(p.root.clone(), p.config(), builder)
}

fn cmd(args: &[&str]) -> Option<IndexerComponent> {
    Some(IndexerComponent {
        program: PathBuf::from(std::env::var_os("ComSpec").unwrap_or("cmd.exe".into())),
        prefix: args.iter().map(Into::into).collect(),
    })
}

#[cfg(windows)]
#[test]
fn indexer_missing_failing_or_crashing_degrades_after_one_retry() {
    // 缺失：重试一次后显示“未安装语义组件”，不是失败通知。
    let p = project(1);
    let missing = process_lifecycle(&p, None);
    let status = missing.wait_until(WAIT, settled);
    assert_eq!(status.state, IndexState::NotInstalled);
    assert!(missing.snapshot().is_none());
    let log = fs::read_to_string(p.cache().log_path()).unwrap();
    assert!(
        log.contains("build attempt 2 failed: indexer component not found"),
        "{log}"
    );
    drop(missing);

    for (component, kind, text) in [
        (
            Some(IndexerComponent {
                program: PathBuf::from(r"Z:\no\such\rove-indexer-cs.exe"),
                prefix: vec![],
            }),
            FailureKind::SpawnFailed,
            "failed to start indexer",
        ),
        (
            cmd(&["/c", "exit", "3"]),
            FailureKind::ExitCode,
            "exited with code 3",
        ),
        (
            cmd(&["/c", "exit", "-1073741819"]),
            FailureKind::Crashed,
            "crashed (0xC0000005)",
        ),
    ] {
        let p = project(1);
        let lifecycle = process_lifecycle(&p, component);
        let status = lifecycle.wait_until(WAIT, settled);
        match &status.state {
            IndexState::Failed(f) => assert_eq!(f.kind, kind, "{f:?}"),
            other => panic!("{other:?}"),
        }
        assert!(lifecycle.snapshot().is_none(), "degraded to L1/L0");
        let log = fs::read_to_string(p.cache().log_path()).unwrap();
        assert_eq!(log.matches("build attempt").count(), 4, "{log}");
        assert!(log.contains(text), "{log}");
    }
}

#[cfg(windows)]
#[test]
fn real_process_output_is_logged_and_parsed_as_progress() {
    let p = project(2);
    let files = sources(&p.root);
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let fixture = p.root.parent().unwrap().join("fixture.rvcsidx");
    FixtureIndex::for_project(&p.root, &refs, [9; 32])
        .write(&fixture)
        .unwrap();
    let script = p.root.parent().unwrap().join("indexer.cmd");
    fs::write(
        &script,
        format!(
            "@echo files: 2, missing: 0 1>&2\r\n@echo   0.1s  [parse_syntax] 0.1s 1>&2\r\n@copy /y \"{}\" \"%2\" >nul\r\n",
            fixture.display()
        ),
    )
    .unwrap();
    let lifecycle = process_lifecycle(&p, cmd(&["/c", script.to_str().unwrap()]));
    let status = lifecycle.wait_until(WAIT, settled);
    assert_eq!(status.state, IndexState::Ready, "{status:?}");
    let log = fs::read_to_string(p.cache().log_path()).unwrap();
    assert!(
        log.contains("indexer: files: 2") && log.contains("exit: 0"),
        "{log}"
    );
}

#[test]
fn disabled_creates_nothing_and_custom_cache_dir_is_honoured() {
    let p = project(1);
    let builder = Arc::new(FakeIndexer::default());
    let disabled = IndexLifecycle::start(
        p.root.clone(),
        LifecycleConfig::new(false, Ok(p.cache_root.clone())),
        builder.clone(),
    );
    assert_eq!(
        disabled.wait_until(WAIT, settled).state,
        IndexState::Disabled
    );
    disabled.files_changed(&[SourceChange::Rescan]);
    thread::sleep(Duration::from_millis(50));
    assert!(!p.cache_root.exists());
    assert_eq!(builder.builds.load(Ordering::SeqCst), 0);

    // 自定义目录（不存在时创建）。
    let custom = p.cache_root.join("deeper/custom");
    let lifecycle = IndexLifecycle::start(
        p.root.clone(),
        LifecycleConfig::new(true, Ok(custom.clone())),
        builder.clone(),
    );
    assert!(ready(&lifecycle.wait_until(WAIT, ready)));
    assert!(ProjectCache::new(&custom, &p.root).meta_path().is_file());
    drop(lifecycle);

    // 不可用：相对路径、无法创建（同名文件挡住）。
    let blocker = p.cache_root.join("blocked");
    fs::write(&blocker, b"file").unwrap();
    for root in [Err("relative path".to_owned()), Ok(blocker.join("index"))] {
        let lifecycle = IndexLifecycle::start(
            p.root.clone(),
            LifecycleConfig::new(true, root),
            builder.clone(),
        );
        match lifecycle.wait_until(WAIT, settled).state {
            IndexState::Failed(f) => assert_eq!(f.kind, FailureKind::CacheDir),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn workspace_without_csharp_hides_until_a_source_appears() {
    let p = project(0);
    fs::remove_file(p.root.join("Game.csproj")).unwrap();
    let builder = Arc::new(FakeIndexer::default());
    let lifecycle = p.start(&builder);
    assert_eq!(
        lifecycle.wait_until(WAIT, settled).state,
        IndexState::NoCSharp
    );
    assert!(!p.cache_root.exists());
    fs::write(p.root.join("Assets/Scripts/A.cs"), "class A {}").unwrap();
    lifecycle.files_changed(&[SourceChange::Created("Assets/Scripts/A.cs".into())]);
    let status = lifecycle.wait_until(WAIT, ready);
    assert!(ready(&status));
    assert!(
        status.info.inferred_project,
        "no csproj: inferred structure"
    );
}

#[test]
fn eviction_keeps_recent_projects_and_the_current_one() {
    let p = project(1);
    let builder = Arc::new(FakeIndexer::default());
    for n in 0..4u64 {
        let dir = p.cache_root.join(format!("Old{n}-{n:016x}"));
        fs::create_dir_all(&dir).unwrap();
        ProjectMeta {
            project: format!("old {n}"),
            last_opened_ms: n + 1,
            build: None,
        }
        .write(&dir.join("project.json"))
        .unwrap();
    }
    let mut config = p.config();
    config.cache = CachePolicy {
        max_projects: 3,
        max_bytes: u64::MAX,
    };
    let lifecycle = IndexLifecycle::start(p.root.clone(), config, builder);
    lifecycle.wait_until(WAIT, ready);
    let mut left: Vec<_> = fs::read_dir(&p.cache_root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left.len(), 3);
    assert!(left.contains(&p.cache().key().to_owned()));
    assert!(
        left.contains(&format!("Old3-{:016x}", 3)) && left.contains(&format!("Old2-{:016x}", 2))
    );
}

#[test]
fn config_change_while_open_rebuilds_and_retry_recovers_from_failure() {
    let p = project(2);
    let builder = Arc::new(FakeIndexer::default());
    built(&p, &builder);
    let lifecycle = p.start(&builder);
    lifecycle.wait_until(WAIT, ready);
    *builder.failures.lock().unwrap() = vec![BuildFailure::ExitCode(1), BuildFailure::Crashed(5)];
    fs::write(p.root.join("Assets/Scripts/Game.asmdef"), r#"{"name":"G"}"#).unwrap();
    lifecycle.files_changed(&[SourceChange::Created("Assets/Scripts/Game.asmdef".into())]);
    let status = lifecycle.wait_until(WAIT, |s| matches!(s.state, IndexState::Failed(_)));
    assert!(matches!(status.state, IndexState::Failed(_)), "{status:?}");
    // 配置变了：已知对不上的旧索引不再提供（降级到 L1/L0）。
    assert!(lifecycle.snapshot().is_none());
    lifecycle.rebuild();
    assert!(ready(&lifecycle.wait_until(WAIT, ready)));
    assert!(lifecycle.snapshot().unwrap().trusted);
}
