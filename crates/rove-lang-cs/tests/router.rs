//! #38 入口集成测试：真实的索引生命周期（#36，假索引进程写出带符号的索引）+ L1（#37）+ 缓冲区，
//! 走“打开文件 → 查询（精确）→ 编辑 → 查询（近似）→ 保存并重建 → 查询（精确）”、
//! “保存 → 重建换代 → 一次查询都不做就继续输入 → 其余行仍然精确”（#216），以及组件未安装时
//! 的 L1/L0 回退。
//! 运行：`cargo test -p rove-lang-cs --test router`。
use rove_base::{
    ByteOffset, ByteRange, Cancellation, Line, Position, Resource, TextEdit, Utf8Column, Version,
    WorkspacePath,
};
use rove_buffer::{RopeText, TextSource};
use rove_lang_cs::l1::{L1Index, L1Options};
use rove_lang_cs::lifecycle::{
    log::IndexLog, process::BuildRequest, BuildFailure, IndexBuilder, IndexLifecycle, IndexState,
    LifecycleConfig, Progress,
};
use rove_lang_cs::query::{ApproximationReason, CsQueries, NavigationKind, Precision, QueryPoint};
use rove_lang_cs::router::testing::{
    find_word, sample_index, HERO_CS, HERO_PATH, PLAYER_CS, PLAYER_PATH,
};
use rove_lang_cs::router::{
    hash_bytes, BufferStates, CsRouter, SemanticSource, SemanticStatus, TargetRole,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(20);

/// 假索引进程：按磁盘上两个文件的当前内容写出示例项目的索引（位置靠搜索单词得到）。
#[derive(Default)]
struct SampleIndexer {
    builds: AtomicUsize,
    missing: AtomicBool,
}

impl IndexBuilder for SampleIndexer {
    fn build(
        &self,
        request: &BuildRequest,
        progress: &mut dyn FnMut(Progress),
        _: &IndexLog,
        _: &Cancellation,
    ) -> Result<(), BuildFailure> {
        if self.missing.load(Ordering::SeqCst) {
            return Err(BuildFailure::Missing);
        }
        self.builds.fetch_add(1, Ordering::SeqCst);
        progress(Progress {
            percent: 50,
            files: Some(2),
        });
        let read = |p: &str| fs::read_to_string(request.project_root.join(p)).unwrap();
        let (index, _) = sample_index(&read(PLAYER_PATH), &read(HERO_PATH));
        index
            .write(&request.output)
            .map_err(|e| BuildFailure::InvalidOutput(e.to_string()))
    }
}

struct Project {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    root: PathBuf,
    cache_root: PathBuf,
}

fn project() -> Project {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let root = a.path().join("Game");
    fs::create_dir_all(root.join("Assets")).unwrap();
    fs::write(root.join("Game.csproj"), "<Project/>").unwrap();
    fs::write(root.join(PLAYER_PATH), PLAYER_CS).unwrap();
    fs::write(root.join(HERO_PATH), HERO_CS).unwrap();
    Project {
        root,
        cache_root: b.path().join("index"),
        _dirs: (a, b),
    }
}

fn l1(root: &Path) -> Arc<L1Index> {
    let l1 = L1Index::new(L1Options::new(root));
    l1.build(
        [PLAYER_PATH, HERO_PATH].map(|p| WorkspacePath::new(p).unwrap()),
        &Cancellation::default(),
    )
    .unwrap();
    Arc::new(l1)
}

fn point(path: &str, version: Version, (line, col): (u32, u32)) -> QueryPoint {
    QueryPoint {
        resource: Resource::Workspace(WorkspacePath::new(path).unwrap()),
        position: Position {
            line: Line(u64::from(line)),
            column: Utf8Column(u64::from(col)),
        },
        version,
    }
}

fn start(r: &rove_lang_cs::router::RoutedTarget) -> (u64, u64) {
    let s = r.target.selection.start();
    (s.line.0, s.column.0)
}

fn at(text: &str, word: &str, nth: usize) -> (u64, u64) {
    let (l, c) = find_word(text, word, nth);
    (u64::from(l), u64::from(c))
}

/// 在缓冲区上做一次替换（`text` 跟着改），返回编辑后的快照。
fn splice(
    buffers: &BufferStates,
    path: &WorkspacePath,
    text: &mut String,
    before: &Arc<RopeText>,
    range: std::ops::Range<usize>,
    ins: &str,
) -> Arc<RopeText> {
    text.replace_range(range.clone(), ins);
    let after = Arc::new(RopeText::new(text.clone()));
    let edit = TextEdit {
        version: before.version(),
        range: ByteRange::new(ByteOffset(range.start as u64), ByteOffset(range.end as u64))
            .unwrap(),
        inserted: ins.into(),
    };
    buffers
        .edited(path, &**before, &edit, after.clone())
        .unwrap();
    after
}

#[test]
fn open_query_edit_query_save_rebuild_query() {
    let p = project();
    let builder = Arc::new(SampleIndexer::default());
    let lifecycle = Arc::new(IndexLifecycle::start(
        p.root.clone(),
        LifecycleConfig::new(true, Ok(p.cache_root.clone())),
        builder.clone(),
    ));
    let status = lifecycle.wait_until(WAIT, |s| s.state == IndexState::Ready);
    assert_eq!(status.state, IndexState::Ready, "{status:?}");
    let generation = status.info.generation;
    let buffers = Arc::new(BufferStates::new());
    let semantic: Arc<dyn SemanticSource> = lifecycle.clone();
    let router = CsRouter::new(semantic, buffers.clone())
        .with_l1(l1(&p.root))
        .with_root(&p.root);
    let cancel = Cancellation::default();
    let player = WorkspacePath::new(PLAYER_PATH).unwrap();

    // 1. 打开文件（读盘 → 哈希），查询：精确。
    let disk = fs::read(p.root.join(PLAYER_PATH)).unwrap();
    let v0: Arc<RopeText> = Arc::new(RopeText::from_text(PLAYER_CS));
    buffers.opened(&player, v0.clone(), hash_bytes(&disk));
    let r = router
        .navigate_detailed(
            NavigationKind::Definition,
            &point(PLAYER_PATH, v0.version(), find_word(PLAYER_CS, "Health", 1)),
            &cancel,
        )
        .unwrap();
    assert_eq!(r.value.len(), 1);
    assert_eq!(start(&r.value[0]), at(PLAYER_CS, "Health", 0));
    assert_eq!(r.precision, Precision::Exact);
    assert_eq!(r.index_generation, generation);

    // 2. 编辑：开头插入一行注释，并改写 `Health -= step;` 这一行。
    let mut text = PLAYER_CS.to_owned();
    let insert = |text: &mut String, before: &Arc<RopeText>, at: usize, ins: &str| {
        text.insert_str(at, ins);
        let after = Arc::new(RopeText::new(text.clone()));
        let edit = TextEdit {
            version: before.version(),
            range: ByteRange::new(ByteOffset(at as u64), ByteOffset(at as u64)).unwrap(),
            inserted: ins.into(),
        };
        buffers
            .edited(&player, &**before, &edit, after.clone())
            .unwrap();
        after
    };
    let mid = insert(&mut text, &v0, 0, "// edited\n");
    let s = text.find("step;").unwrap() + "step".len();
    let v1 = insert(&mut text, &mid, s, " + 1");

    // 3. 查询修改行上的 Health：近似（L1 给出同名成员候选）。
    let on_modified = point(PLAYER_PATH, v1.version(), find_word(&text, "Health", 1));
    let r = router
        .navigate_detailed(NavigationKind::Definition, &on_modified, &cancel)
        .unwrap();
    assert_eq!(r.precision, Precision::Approximate);
    assert!(matches!(
        r.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    assert!(!r.value.is_empty());
    assert!(r
        .value
        .iter()
        .all(|t| t.precision == Precision::Approximate && t.role == TargetRole::Candidate));
    // 未修改行上的查询仍然精确，结果平移了一行。
    let r = router
        .navigate(
            NavigationKind::Definition,
            &point(HERO_PATH, Version(0), find_word(HERO_CS, "Move", 0)),
            &cancel,
        )
        .unwrap();
    assert_eq!(r.precision, Precision::Exact);
    let s = r.value[0].selection.start();
    assert_eq!((s.line.0, s.column.0), at(&text, "Move", 0));
    // 未保存的编辑**不**请求重建：索引进程只读磁盘，重建也拿不到缓冲区里的内容
    // （`LANGUAGE_INTELLIGENCE` §3.2，口径见 #213）。
    assert!(
        !lifecycle.is_stale(PLAYER_PATH),
        "未保存的编辑不该把文件标记为过期"
    );

    // 4. 保存并重建。
    fs::write(p.root.join(PLAYER_PATH), &text).unwrap();
    buffers.saved(&player, hash_bytes(text.as_bytes())).unwrap();
    // 保存后第一次查询发现磁盘与索引不同：请求重建（#35 合并前由 #36 标记过期）。
    router
        .navigate_detailed(NavigationKind::Definition, &on_modified, &cancel)
        .unwrap();
    assert!(lifecycle.is_stale(PLAYER_PATH));
    lifecycle.rebuild();
    let status = lifecycle.wait_until(WAIT, |s| {
        s.state == IndexState::Ready && s.info.generation != generation
    });
    assert_ne!(status.info.generation, generation, "{status:?}");
    assert_eq!(builder.builds.load(Ordering::SeqCst), 2);

    // 5. 查询：重新精确。
    let r = router
        .navigate_detailed(NavigationKind::Definition, &on_modified, &cancel)
        .unwrap();
    assert_eq!(r.precision, Precision::Exact, "{:?}", r.reason);
    assert_eq!(r.value.len(), 1);
    assert_eq!(start(&r.value[0]), at(&text, "Health", 0));
    assert_eq!(r.index_generation, status.info.generation);
    let r = router
        .navigate_detailed(NavigationKind::References, &on_modified, &cancel)
        .unwrap();
    assert_eq!(r.value.len(), 3);
    assert!(r.value.iter().all(|t| t.precision == Precision::Exact));
}

/// #216：保存 → 索引重建完成（换代）→ **一次查询都没做**就继续输入 → 查询。
/// 真实的 `IndexLifecycle` 换代，走的是用户最常见的用法：保存后一边等索引一边接着写代码。
#[test]
fn typing_before_the_first_query_after_a_rebuild_keeps_other_lines_exact() {
    let p = project();
    let builder = Arc::new(SampleIndexer::default());
    let lifecycle = Arc::new(IndexLifecycle::start(
        p.root.clone(),
        LifecycleConfig::new(true, Ok(p.cache_root.clone())),
        builder.clone(),
    ));
    let status = lifecycle.wait_until(WAIT, |s| s.state == IndexState::Ready);
    assert_eq!(status.state, IndexState::Ready, "{status:?}");
    let generation = status.info.generation;
    let buffers = Arc::new(BufferStates::new());
    let semantic: Arc<dyn SemanticSource> = lifecycle.clone();
    let router = CsRouter::new(semantic, buffers.clone())
        .with_l1(l1(&p.root))
        .with_root(&p.root);
    let cancel = Cancellation::default();
    let player = WorkspacePath::new(PLAYER_PATH).unwrap();

    // 1. 打开 → 编辑（开头插入一行注释）→ 保存。
    let disk = fs::read(p.root.join(PLAYER_PATH)).unwrap();
    let v0: Arc<RopeText> = Arc::new(RopeText::from_text(PLAYER_CS));
    buffers.opened(&player, v0.clone(), hash_bytes(&disk));
    let mut text = PLAYER_CS.to_owned();
    let v1 = splice(&buffers, &player, &mut text, &v0, 0..0, "// edited\n");
    fs::write(p.root.join(PLAYER_PATH), &text).unwrap();
    buffers.saved(&player, hash_bytes(text.as_bytes())).unwrap();
    let saved = text.clone();

    // 2. 索引重建完成（换代）。这中间一次查询都没有发生。
    lifecycle.rebuild();
    let status = lifecycle.wait_until(WAIT, |s| {
        s.state == IndexState::Ready && s.info.generation != generation
    });
    assert_ne!(status.info.generation, generation, "{status:?}");

    // 3. 继续输入：改掉 `var step = speed * 2;` 这一行。
    let star = saved.find("* 2").unwrap();
    let v2 = splice(&buffers, &player, &mut text, &v1, star..star + 3, "* 3");

    // 4. 其余行仍然精确：基线已经推进到换代的那一版，只有新改的那一行降级。
    let r = router
        .navigate_detailed(
            NavigationKind::Definition,
            &point(PLAYER_PATH, v2.version(), find_word(&text, "Health", 1)),
            &cancel,
        )
        .unwrap();
    assert_eq!(r.precision, Precision::Exact, "{:?}", r.reason);
    assert_eq!(r.value.len(), 1);
    assert_eq!(start(&r.value[0]), at(&text, "Health", 0));
    assert_eq!(r.index_generation, status.info.generation);
    let r = router
        .navigate_detailed(
            NavigationKind::References,
            &point(PLAYER_PATH, v2.version(), find_word(&text, "Health", 1)),
            &cancel,
        )
        .unwrap();
    assert_eq!(r.value.len(), 3);
    assert!(r.value.iter().all(|t| t.precision == Precision::Exact));

    // 5. 新改的那一行按修改区处理（不是 L2 的精确答案）。
    let r = router
        .navigate_detailed(
            NavigationKind::Definition,
            &point(PLAYER_PATH, v2.version(), find_word(&text, "speed", 1)),
            &cancel,
        )
        .unwrap();
    assert_ne!(r.precision, Precision::Exact, "{:?}", r.reason);

    // 6. 登记口径：基线是索引那一代的内容，编辑记录里只剩换代之后的那一次编辑。
    let state = buffers.get(PLAYER_PATH).unwrap();
    assert_eq!(state.base_hash, hash_bytes(saved.as_bytes()));
    assert_eq!(state.current_hash, None, "第 3 步之后还没保存");
    assert_eq!(builder.builds.load(Ordering::SeqCst), 2);
}

#[test]
fn component_missing_falls_back_to_l1_and_l0() {
    let p = project();
    let builder = Arc::new(SampleIndexer::default());
    builder.missing.store(true, Ordering::SeqCst);
    let lifecycle = Arc::new(IndexLifecycle::start(
        p.root.clone(),
        LifecycleConfig::new(true, Ok(p.cache_root.clone())),
        builder,
    ));
    let status = lifecycle.wait_until(WAIT, |s| s.state == IndexState::NotInstalled);
    assert_eq!(status.state, IndexState::NotInstalled);
    let semantic: Arc<dyn SemanticSource> = lifecycle.clone();
    let router = CsRouter::new(semantic, Arc::new(BufferStates::new()))
        .with_l1(l1(&p.root))
        .with_root(&p.root);
    assert_eq!(router.semantic_status(), SemanticStatus::NotInstalled);
    let cancel = Cancellation::default();
    // 类型名：L1 解析，近似。
    let r = router
        .navigate_detailed(
            NavigationKind::Definition,
            &point(HERO_PATH, Version(0), find_word(HERO_CS, "Player", 0)),
            &cancel,
        )
        .unwrap();
    assert_eq!(r.precision, Precision::Approximate);
    assert!(matches!(
        r.reason,
        Some(ApproximationReason::ComponentUnavailable)
    ));
    assert_eq!(r.value.len(), 1);
    assert_eq!(start(&r.value[0]), at(PLAYER_CS, "Player", 0));
    // lambda 参数：L0，语法。
    let r = router
        .navigate_detailed(
            NavigationKind::References,
            &point(HERO_PATH, Version(0), find_word(HERO_CS, "x", 1)),
            &cancel,
        )
        .unwrap();
    assert_eq!(r.precision, Precision::Syntax);
    assert_eq!(r.value.len(), 2);
    // 工作区符号来自 L1。
    let symbols = router.workspace_symbols("Hero", &cancel).unwrap();
    let hero = symbols.value.iter().find(|s| &*s.name == "Hero").unwrap();
    assert_eq!(hero.kind, rove_syntax::SymbolKind::Class);

    let symbols = router.workspace_symbols("Move", &cancel).unwrap();
    let method = symbols.value.iter().find(|s| &*s.name == "Move").unwrap();
    assert_eq!(method.kind, rove_syntax::SymbolKind::Method);
}
