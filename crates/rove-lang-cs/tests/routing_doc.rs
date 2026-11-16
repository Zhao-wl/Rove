//! #213 口径看门测试：`docs/LANGUAGE_INTELLIGENCE.md` §3.2 的查询路由描述必须与 #38 的实现一致。
//!
//! #44（M2 集成验收）发现文档写的是「未保存修改也触发该文件的增量重建」，而实现只在**保存
//! （或外部改写）之后**才请求重建——索引进程只读磁盘，未保存时重建拿不到缓冲区里的内容。#213
//! 定的口径是**实现为准、文档改口**。这里两头都测：
//!
//! 1. 跑一遍真实的 `CsRouter`，把 §3.2 承诺的行为逐条断言出来；
//! 2. 读 §3.2 的正文，确认文档写的就是这些行为（旧措辞再回来就会红）。
//!
//! 运行：`cargo test -p rove-lang-cs --test routing_doc`。
use rove_base::{
    ByteOffset, ByteRange, Cancellation, Line, Position, Resource, TextEdit, Utf8Column, Version,
    WorkspacePath,
};
use rove_buffer::{RopeText, TextSource};
use rove_lang_cs::query::{ApproximationReason, CsQueries, NavigationKind, Precision, QueryPoint};
use rove_lang_cs::router::testing::{find_word, sample_index, HERO_CS, PLAYER_CS, PLAYER_PATH};
use rove_lang_cs::router::{
    hash_bytes, BufferStates, CsRouter, IndexHolder, ManualSemantic, SemanticSource,
};
use rove_lang_cs::CsIndex;
use std::sync::Arc;

struct Fixture {
    _dir: tempfile::TempDir,
    semantic: Arc<ManualSemantic>,
    buffers: Arc<BufferStates>,
    router: CsRouter,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Assets")).unwrap();
    std::fs::write(dir.path().join(PLAYER_PATH), PLAYER_CS).unwrap();
    let (index, _) = sample_index(PLAYER_CS, HERO_CS);
    let path = dir.path().join("sample.idx");
    index.write(&path).unwrap();
    let holder: Arc<dyn IndexHolder> = Arc::new(CsIndex::open(&path).unwrap());
    let semantic = Arc::new(ManualSemantic::ready(holder));
    let source: Arc<dyn SemanticSource> = semantic.clone();
    let buffers = Arc::new(BufferStates::new());
    let router = CsRouter::new(source, buffers.clone()).with_root(dir.path());
    Fixture {
        _dir: dir,
        semantic,
        buffers,
        router,
    }
}

fn point(text: &str, word: &str, nth: usize, version: Version) -> QueryPoint {
    let (line, col) = find_word(text, word, nth);
    QueryPoint {
        resource: Resource::Workspace(WorkspacePath::new(PLAYER_PATH).unwrap()),
        position: Position {
            line: Line(u64::from(line)),
            column: Utf8Column(u64::from(col)),
        },
        version,
    }
}

/// §3.2 承诺的行为：干净文件精确 → 改动所在行回退为近似且**不**请求重建 → 保存后才请求重建。
#[test]
fn rebuild_is_requested_only_after_save() {
    let f = fixture();
    let cancel = Cancellation::default();
    let player = WorkspacePath::new(PLAYER_PATH).unwrap();
    let v0: Arc<RopeText> = Arc::new(RopeText::from_text(PLAYER_CS));
    f.buffers
        .opened(&player, v0.clone(), hash_bytes(PLAYER_CS.as_bytes()));

    // 1. 文件与索引一致：精确。
    let clean = f
        .router
        .navigate(
            NavigationKind::Definition,
            &point(PLAYER_CS, "Health", 1, v0.version()),
            &cancel,
        )
        .unwrap();
    assert_eq!(clean.precision, Precision::Exact);
    assert!(
        f.semantic.requested().is_empty(),
        "没有改动就不该请求重建：{:?}",
        f.semantic.requested()
    );

    // 2. 在 `Health -= step;` 这一行上敲字（未保存）。
    let mut text = PLAYER_CS.to_owned();
    let offset = text.find("step;").unwrap() + "step".len();
    text.insert_str(offset, " + 1");
    let v1 = Arc::new(RopeText::new(text.clone()));
    f.buffers
        .edited(
            &player,
            &*v0,
            &TextEdit {
                version: v0.version(),
                range: ByteRange::new(ByteOffset(offset as u64), ByteOffset(offset as u64))
                    .unwrap(),
                inserted: " + 1".into(),
            },
            v1.clone(),
        )
        .unwrap();

    // 被改动的那一行回退为近似，原因是「已修改区域」（§3.2 的「结果标记为近似」+ `UX_SPEC` §2.2）。
    let on_modified = point(&text, "Health", 1, v1.version());
    let edited = f
        .router
        .navigate(NavigationKind::Definition, &on_modified, &cancel)
        .unwrap();
    assert_ne!(edited.precision, Precision::Exact);
    assert!(
        matches!(edited.reason, Some(ApproximationReason::ModifiedRegion)),
        "{:?}",
        edited.reason
    );
    // 未改动的行经位置映射后仍然精确（§3.2 的「映射成功且该符号所在行未被修改 → 查 L2」）。
    let untouched = f
        .router
        .navigate(
            NavigationKind::Definition,
            &point(&text, "Health", 0, v1.version()),
            &cancel,
        )
        .unwrap();
    assert_eq!(untouched.precision, Precision::Exact, "{untouched:?}");

    // **未保存的编辑不请求重建**：索引进程只读磁盘，重建也拿不到缓冲区里的内容。
    assert!(
        f.semantic.requested().is_empty(),
        "未保存的编辑不该请求重建（LANGUAGE_INTELLIGENCE §3.2，#213）：{:?}",
        f.semantic.requested()
    );

    // 3. 保存：磁盘内容与索引不同，这时才请求增量重建。
    f.buffers
        .saved(&player, hash_bytes(text.as_bytes()))
        .unwrap();
    f.router
        .navigate(NavigationKind::Definition, &on_modified, &cancel)
        .unwrap();
    assert_eq!(
        f.semantic.requested(),
        vec![PLAYER_PATH.to_owned()],
        "保存后必须请求一次（且只请求一次）增量重建"
    );
    // 同一份内容重复查询不会再发一次请求。
    f.router
        .navigate(NavigationKind::Definition, &on_modified, &cancel)
        .unwrap();
    assert_eq!(f.semantic.requested().len(), 1);
}

fn section_3_2() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/LANGUAGE_INTELLIGENCE.md"
    );
    let text = std::fs::read_to_string(path).expect("docs/LANGUAGE_INTELLIGENCE.md");
    let start = text
        .find("### 3.2 查询路由")
        .expect("LANGUAGE_INTELLIGENCE.md 里找不到 §3.2");
    let rest = &text[start..];
    let end = rest[1..]
        .find("\n### ")
        .map(|i| i + 1)
        .unwrap_or(rest.len());
    rest[..end].to_owned()
}

/// §3.2 的正文必须写的是上面那套行为：旧的「未保存也触发重建」措辞不能再回来。
#[test]
fn doc_section_3_2_matches_the_implementation() {
    let doc = section_3_2();

    // 重建的触发时机：只在保存（或外部改写）之后。
    assert!(
        doc.contains("只在文件保存"),
        "§3.2 必须写明增量重建只在保存后触发（#213）"
    );
    assert!(
        doc.contains("未保存的编辑不请求重建"),
        "§3.2 必须写明未保存的编辑不请求重建，并给出理由（索引进程只读磁盘）"
    );
    assert!(
        doc.contains("只读磁盘"),
        "§3.2 要写清为什么：索引进程只读磁盘（#213）"
    );

    // 旧措辞：流程图里不能再把「触发该文件的增量重建」挂在回退分支上。
    for line in doc.lines().filter(|l| l.contains("回退")) {
        assert!(
            !line.contains("重建"),
            "§3.2 的回退分支不该再写「触发增量重建」（#213）：{line}"
        );
    }

    // 位置映射的依据是编辑记录，不是文本 diff（#38 的 `EditMap` 按编辑事务累积）。
    assert!(
        doc.contains("按编辑记录平移行列"),
        "§3.2 的位置映射依据应当写「按编辑记录」"
    );
    assert!(
        !doc.contains("按文本 diff"),
        "§3.2 不该再写「按文本 diff 平移行列」：实现用的是编辑记录"
    );

    // 修改区判定是行级的（`router::mapping` 的 `line_modified_old/new`）。
    assert!(
        doc.contains("所在行未被修改") && doc.contains("行级"),
        "§3.2 要写明修改区判定是行级的"
    );

    // 近似结果必须标明（`UX_SPEC` §2.2），且精确度是逐条标记的。
    assert!(doc.contains("近似"), "§3.2 要写明回退结果标记为近似");
    assert!(
        doc.contains("逐条"),
        "§3.2 要写明精确度是逐条标记的（`navigate_detailed`）"
    );

    // L2 不可用的四种原因都要写到，不能只写「构建中」。
    for reason in ["构建中", "未安装", "已禁用", "失败"] {
        assert!(
            doc.contains(reason),
            "§3.2 要覆盖 L2 不可用的原因「{reason}」（#36 的状态机）"
        );
    }
}
