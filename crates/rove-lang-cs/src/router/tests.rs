//! 路由的单元测试：构造的索引（真实读取端打开）+ 真实的 RopeText 缓冲区。
use super::testing::{
    find_word, sample_index, SampleSymbols, HERO_CS, HERO_PATH, KIND_METHOD, KIND_TYPE, PLAYER_CS,
    PLAYER_PATH,
};
use super::*;
use rove_base::{ByteOffset, ByteRange, TextEdit};
use rove_buffer::{RopeText, TextSource};

struct Fixture {
    _dir: tempfile::TempDir,
    semantic: Arc<ManualSemantic>,
    router: CsRouter,
    s: SampleSymbols,
}

fn fixture() -> Fixture {
    fixture_with(|_| {})
}

fn fixture_with(adjust: impl FnOnce(&mut testing::TestIndex)) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Assets")).unwrap();
    std::fs::write(dir.path().join(PLAYER_PATH), PLAYER_CS).unwrap();
    std::fs::write(dir.path().join(HERO_PATH), HERO_CS).unwrap();
    let (mut index, s) = sample_index(PLAYER_CS, HERO_CS);
    adjust(&mut index);
    let path = dir.path().join("sample.idx");
    index.write(&path).unwrap();
    let index: Arc<dyn IndexHolder> = Arc::new(CsIndex::open(&path).unwrap());
    let semantic = Arc::new(ManualSemantic::ready(index));
    let router =
        CsRouter::new(semantic.clone(), Arc::new(BufferStates::new())).with_root(dir.path());
    Fixture {
        _dir: dir,
        semantic,
        router,
        s,
    }
}

fn wp(path: &str) -> WorkspacePath {
    WorkspacePath::new(path).unwrap()
}

fn pos(line: u32, col: u32) -> Position {
    position(line, u64::from(col))
}

/// 未打开的文件：版本无关紧要。
fn point(path: &str, (line, col): (u32, u32)) -> QueryPoint {
    QueryPoint {
        resource: Resource::Workspace(wp(path)),
        position: pos(line, col),
        version: Version(0),
    }
}

fn point_in(path: &str, text: &dyn TextSource, (line, col): (u32, u32)) -> QueryPoint {
    QueryPoint {
        resource: Resource::Workspace(wp(path)),
        position: pos(line, col),
        version: text.version(),
    }
}

const EXACT: u8 = 0;
const APPROX: u8 = 1;
const SYNTAX: u8 = 2;

/// 结果的（路径, 行, 列, 精确度等级）。
fn starts(r: &QueryResult<Arc<[RoutedTarget]>>) -> Vec<(String, u64, u64, u8)> {
    r.value
        .iter()
        .map(|t| {
            let path = match &t.target.resource {
                Resource::Workspace(p) => p.as_str().to_owned(),
                other => format!("{other:?}"),
            };
            let s = t.target.selection.start();
            (path, s.line.0, s.column.0, rank(t.precision))
        })
        .collect()
}

fn word(text: &str, w: &str, nth: usize) -> (u32, u32) {
    find_word(text, w, nth)
}

/// 打开一个文件（磁盘内容 = 索引版本）。
fn open(f: &Fixture, path: &str, text: &str) -> Arc<RopeText> {
    let rope = Arc::new(RopeText::from_text(text));
    f.router
        .buffers()
        .opened(&wp(path), rope.clone(), hash_bytes(text.as_bytes()));
    rope
}

/// 在缓冲区上做一次编辑，返回编辑后的快照。
fn edit(
    f: &Fixture,
    path: &str,
    before: &Arc<RopeText>,
    start: usize,
    end: usize,
    ins: &str,
) -> Arc<RopeText> {
    let mut text = buffers::read_all(&**before).unwrap();
    text.replace_range(start..end, ins);
    let after = Arc::new(RopeText::new(text));
    let e = TextEdit {
        version: before.version(),
        range: ByteRange::new(ByteOffset(start as u64), ByteOffset(end as u64)).unwrap(),
        inserted: ins.into(),
    };
    f.router
        .buffers()
        .edited(&wp(path), &**before, &e, after.clone())
        .unwrap();
    after
}

fn nav(f: &Fixture, kind: NavigationKind, p: &QueryPoint) -> QueryResult<Arc<[RoutedTarget]>> {
    f.router
        .navigate_detailed(kind, p, &Cancellation::default())
        .unwrap()
}

#[test]
fn clean_file_goes_to_l2_and_is_exact() {
    let f = fixture();
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "Move", 0)),
    );
    let (l, c) = word(PLAYER_CS, "Move", 0);
    assert_eq!(
        starts(&r),
        [(PLAYER_PATH.into(), u64::from(l), u64::from(c), EXACT)]
    );
    assert_eq!(r.precision, Precision::Exact);
    assert!(r.reason.is_none());
    assert_eq!(r.index_generation, 1);
    // 契约接口：同样的结果，批级精确。
    let q: &dyn CsQueries = &f.router;
    let plain = q
        .navigate(
            NavigationKind::Definition,
            &point(HERO_PATH, word(HERO_CS, "Move", 0)),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(plain.value.len(), 1);
    assert_eq!(plain.precision, Precision::Exact);
}

/// 验收：在文件开头插入 10 行之后，对文件后部符号的跳转仍然精确（经位置映射走 L2）。
#[test]
fn inserting_ten_lines_at_top_keeps_jumps_exact() {
    let f = fixture();
    let v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    let header = "// added\n".repeat(10);
    let v1 = edit(&f, PLAYER_PATH, &v0, 0, 0, &header);
    let current = format!("{header}{PLAYER_CS}");
    // 查询点：当前缓冲区里 `Health -= step` 的 Health（旧第 10 行 → 新第 20 行）。
    let at = word(&current, "Health", 1);
    assert_eq!(at.0, word(PLAYER_CS, "Health", 1).0 + 10);
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*v1, at),
    );
    let def = word(&current, "Health", 0);
    assert_eq!(
        starts(&r),
        [(
            PLAYER_PATH.into(),
            u64::from(def.0),
            u64::from(def.1),
            EXACT
        )]
    );
    assert_eq!(r.precision, Precision::Exact);
    // 从干净的文件跳进这个脏缓冲区：目标也平移了 10 行。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "Move", 0)),
    );
    let def = word(&current, "Move", 0);
    assert_eq!(
        starts(&r),
        [(
            PLAYER_PATH.into(),
            u64::from(def.0),
            u64::from(def.1),
            EXACT
        )]
    );
    // 插入的行本身是修改区：退回 L0/L1，不会返回一个看起来像答案的 L2 结果。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*v1, (3, 3)),
    );
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Approximate);
    assert!(matches!(
        r.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    // 未保存的编辑不请求重建（索引进程只读磁盘）。
    assert!(f.semantic.requested().is_empty());
}

/// 验收：修改符号本身所在的行之后，该符号的结果标记为“近似”。
#[test]
fn editing_the_symbols_line_degrades_its_results() {
    let f = fixture();
    let v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    // `Health -= step;` → `Health -= step + 1;`
    let line = PLAYER_CS.find("step;").unwrap() + "step".len();
    let v1 = edit(&f, PLAYER_PATH, &v0, line, line, " + 1");
    let current = PLAYER_CS.replacen("step;", "step + 1;", 1);
    // 查询点在修改行上：不走 L2。Health 是字段，L0 不认识、没有 L1 → 空的近似结果。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*v1, word(&current, "Health", 1)),
    );
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Approximate);
    assert!(matches!(
        r.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    // 同一行上的局部变量由 L0 解析，标为“语法”。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*v1, word(&current, "step", 1)),
    );
    let def = word(&current, "step", 0);
    assert_eq!(
        starts(&r),
        [(
            PLAYER_PATH.into(),
            u64::from(def.0),
            u64::from(def.1),
            SYNTAX
        )]
    );
    assert_eq!(r.precision, Precision::Syntax);
    // 从干净文件查 Health 的引用：落在修改行上的那一条是近似，其余仍然精确。
    let r = nav(
        &f,
        NavigationKind::References,
        &point(HERO_PATH, word(HERO_CS, "Health", 0)),
    );
    let mut got = starts(&r);
    got.sort();
    let at = |text: &str, n| {
        let (l, c) = word(text, "Health", n);
        (u64::from(l), u64::from(c))
    };
    let mut want = vec![
        (
            HERO_PATH.to_owned(),
            at(HERO_CS, 0).0,
            at(HERO_CS, 0).1,
            EXACT,
        ),
        (
            PLAYER_PATH.to_owned(),
            at(&current, 0).0,
            at(&current, 0).1,
            EXACT,
        ),
        (
            PLAYER_PATH.to_owned(),
            at(&current, 1).0,
            at(&current, 1).1,
            APPROX,
        ),
    ];
    want.sort();
    assert_eq!(got, want);
    // 批级（契约接口）取最弱的一条。
    assert_eq!(r.precision, Precision::Approximate);
    assert!(matches!(
        r.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    let roles: Vec<_> = r.value.iter().map(|t| t.role).collect();
    assert_eq!(
        roles
            .iter()
            .filter(|&&r| r == TargetRole::Definition)
            .count(),
        1
    );
}

/// 验收：反向映射——引用结果落在脏缓冲区中时位置被平移；列表里混有干净文件与脏缓冲区。
#[test]
fn references_mix_clean_files_and_dirty_buffers() {
    let f = fixture();
    let v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    // 在 Health 字段声明之前插入两行，并在同一行内的 Health 之前插入文字（列平移）。
    let p = PLAYER_CS.find("        public int Health").unwrap();
    let v1 = edit(&f, PLAYER_PATH, &v0, p, p, "        // a\n        // b\n");
    let mid = format!(
        "{}{}{}",
        &PLAYER_CS[..p],
        "        // a\n        // b\n",
        &PLAYER_CS[p..]
    );
    let q = mid.find("Health -=").unwrap();
    let _v2 = edit(&f, PLAYER_PATH, &v1, q, q, "/*x*/ ");
    let current = format!("{}/*x*/ {}", &mid[..q], &mid[q..]);
    let r = nav(
        &f,
        NavigationKind::References,
        &point(HERO_PATH, word(HERO_CS, "Health", 0)),
    );
    let mut got = starts(&r);
    got.sort();
    let h = word(HERO_CS, "Health", 0);
    let d = word(&current, "Health", 0);
    let mut want = [
        (HERO_PATH.to_owned(), u64::from(h.0), u64::from(h.1), EXACT),
        (
            PLAYER_PATH.to_owned(),
            u64::from(d.0),
            u64::from(d.1),
            EXACT,
        ),
        // 同一行被修改过：位置尽力平移，但标为近似。
        (
            PLAYER_PATH.to_owned(),
            u64::from(word(&current, "Health", 1).0),
            0,
            APPROX,
        ),
    ];
    want.sort();
    let mut got_lines: Vec<_> = got.iter().map(|g| (g.0.clone(), g.1, g.3)).collect();
    got_lines.sort();
    let mut want_lines: Vec<_> = want.iter().map(|w| (w.0.clone(), w.1, w.3)).collect();
    want_lines.sort();
    assert_eq!(got_lines, want_lines);
    // 干净与未修改行上的结果：行列都精确。
    assert!(got.contains(&want[0]) || got.contains(&want[1]));
    assert!(got
        .iter()
        .any(|g| g.0 == PLAYER_PATH && g.1 == u64::from(d.0) && g.2 == u64::from(d.1)));
    assert_eq!(r.precision, Precision::Approximate);
}

/// 降级 1：L2 未就绪。局部变量走 L0（语法），其他返回空的近似结果并带“构建中”原因。
#[test]
fn l2_not_ready_falls_back_with_reason() {
    let f = fixture();
    f.semantic
        .set(None, SemanticStatus::Building { percent: Some(42) });
    assert_eq!(
        f.router.semantic_status(),
        SemanticStatus::Building { percent: Some(42) }
    );
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "Move", 0)),
    );
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Approximate);
    assert!(matches!(r.reason, Some(ApproximationReason::IndexNotReady)));
    assert_eq!(r.index_generation, 0);
    // lambda 参数：L0。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "x", 1)),
    );
    let def = word(HERO_CS, "x", 0);
    assert_eq!(
        starts(&r),
        [(HERO_PATH.into(), u64::from(def.0), u64::from(def.1), SYNTAX)]
    );
    assert!(matches!(r.reason, Some(ApproximationReason::IndexNotReady)));
}

/// 降级 2：组件未安装 / 已禁用。
#[test]
fn component_missing_or_disabled() {
    let f = fixture();
    f.semantic.set(None, SemanticStatus::NotInstalled);
    let at = point(PLAYER_PATH, word(PLAYER_CS, "speed", 1));
    let r = nav(&f, NavigationKind::References, &at);
    assert_eq!(r.value.len(), 2);
    assert!(r.value.iter().all(|t| t.precision == Precision::Syntax));
    assert!(matches!(
        r.reason,
        Some(ApproximationReason::ComponentUnavailable)
    ));
    f.semantic.set(None, SemanticStatus::Disabled);
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "Move", 0)),
    );
    assert!(matches!(r.reason, Some(ApproximationReason::IndexDisabled)));
}

/// 降级 3：文件已修改且无法映射（外部修改、没有编辑记录）→ 近似，并请求重建。
#[test]
fn diverged_file_cannot_be_mapped() {
    let f = fixture();
    // 结果落在过期文件里：位置照旧给出，但标为近似。
    f.semantic.mark_stale(PLAYER_PATH);
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "Move", 0)),
    );
    let def = word(PLAYER_CS, "Move", 0);
    assert_eq!(
        starts(&r),
        [(
            PLAYER_PATH.into(),
            u64::from(def.0),
            u64::from(def.1),
            APPROX
        )]
    );
    assert!(matches!(
        r.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    // 查询点所在的未打开文件被生命周期标记为过期。
    f.semantic.mark_stale(HERO_PATH);
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "Move", 0)),
    );
    assert!(r.value.is_empty());
    assert!(matches!(
        r.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    // 打开的文件：磁盘内容（重新载入后）与索引不同，且没有从索引版本开始的编辑记录。
    let other = "namespace Game { class Player { void Move() {} } }\n";
    let text = Arc::new(RopeText::from_text(other));
    f.router
        .buffers()
        .opened(&wp(PLAYER_PATH), text.clone(), hash_bytes(other.as_bytes()));
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*text, word(other, "Move", 0)),
    );
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Approximate);
    assert_eq!(f.semantic.requested(), [PLAYER_PATH]);
    // 同一内容不重复请求。
    nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*text, word(other, "Move", 0)),
    );
    assert_eq!(f.semantic.requested().len(), 1);
}

/// 降级 4：符号查不到 → 空结果；L2 就绪时是“确实没有”（精确）。
#[test]
fn nothing_found_is_empty_not_a_guess() {
    let f = fixture();
    // 空白处。
    let r = nav(&f, NavigationKind::Definition, &point(HERO_PATH, (0, 0)));
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Exact);
    // 未解析的名字（索引里没有出现位置）。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "Run", 0)),
    );
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Exact);
    // 不在索引里的文件：回退，近似。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point("Assets/New.cs", (0, 0)),
    );
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Approximate);
}

#[test]
fn relations_and_external_symbols() {
    let f = fixture();
    let at_player = point(PLAYER_PATH, word(PLAYER_CS, "Player", 0));
    let r = nav(&f, NavigationKind::DerivedTypes, &at_player);
    let hero = word(HERO_CS, "Hero", 0);
    assert_eq!(
        starts(&r),
        [(
            HERO_PATH.into(),
            u64::from(hero.0),
            u64::from(hero.1),
            EXACT
        )]
    );
    let r = nav(&f, NavigationKind::Implementations, &at_player);
    assert_eq!(r.value.len(), 1);
    let r = nav(&f, NavigationKind::BaseTypes, &point(HERO_PATH, hero));
    assert_eq!(starts(&r)[0].0, PLAYER_PATH);
    // 重写的外部方法：指向元数据视图。
    let r = nav(
        &f,
        NavigationKind::Overrides,
        &point(HERO_PATH, word(HERO_CS, "ToString", 0)),
    );
    assert_eq!(r.value.len(), 1);
    assert_eq!(r.value[0].role, TargetRole::Metadata);
    assert_eq!(
        metadata_symbol_key(&r.value[0].target.resource),
        Some("mscorlib|M:System.Object.ToString")
    );
    assert_eq!(r.value[0].precision, Precision::Exact);
    // 类型定义：类型本身可以，局部变量不支持。
    let r = nav(
        &f,
        NavigationKind::TypeDefinition,
        &point(HERO_PATH, word(HERO_CS, "Player", 0)),
    );
    assert_eq!(r.value.len(), 1);
    assert!(matches!(
        f.router.navigate_detailed(
            NavigationKind::TypeDefinition,
            &point(PLAYER_PATH, word(PLAYER_CS, "step", 1)),
            &Cancellation::default()
        ),
        Err(Error::Unsupported)
    ));
    let _ = f.s;
}

#[test]
fn hover_signature_and_docs() {
    let f = fixture();
    let c = Cancellation::default();
    let h = f
        .router
        .hover_detailed(&point(HERO_PATH, word(HERO_CS, "Player", 0)), &c)
        .unwrap();
    let hover = h.value.unwrap();
    assert_eq!(&*hover.documentation, "A player.");
    assert_eq!(&*hover.signature, "Player");
    assert_eq!(h.precision, Precision::Exact);
    // L2 不可用：局部参数由 L0 给出声明文本。
    f.semantic.set(None, SemanticStatus::NotInstalled);
    let h = f
        .router
        .hover_detailed(&point(PLAYER_PATH, word(PLAYER_CS, "speed", 1)), &c)
        .unwrap();
    assert_eq!(&*h.value.unwrap().signature, "int speed");
    assert_eq!(h.precision, Precision::Syntax);
}

/// 外部成员的元数据视图正文：`Run` 上方是 `///` 文档注释（#34 的生产索引形态）。
const RUNNER_VIEW: &str = "#region 程序集 Game.Core, Version=1.0.0.0
// Game.Core.dll
#endregion

namespace Game.Core
{
    public class Runner
    {
        /// <summary>Runs the thing.</summary>
        /// <returns>nothing</returns>
        public void Run(System.Func<int, int> f);
    }
}
";

const RUN_SIGNATURE: &str = "public void Run(System.Func<int, int> f);";

/// #142：生产索引的段 12 是空的，外部成员的文档只存在元数据视图正文里，悬停要回退过去取。
#[test]
fn hover_documentation_falls_back_to_metadata_view() {
    let f = fixture_with(|t| {
        // 段 12 清空：与索引进程写出的真实索引一致。
        t.docs.clear();
        let hero = t
            .files
            .iter()
            .position(|(path, _)| path == HERO_PATH)
            .unwrap() as u32;
        let runner = t.symbol("Game.Core|T:Game.Core.Runner", "Runner", KIND_TYPE, None);
        let run = t.symbol(
            "Game.Core|M:Game.Core.Runner.Run(System.Func{System.Int32,System.Int32})",
            "Run",
            KIND_METHOD,
            Some(runner),
        );
        let (line, col) = find_word(HERO_CS, "Run", 0);
        t.reference(run, hero, line, col);
        let (ty_line, ty_col) = find_word(RUNNER_VIEW, "Runner", 0);
        let (run_line, run_col) = find_word(RUNNER_VIEW, "Run", 0);
        t.views.push(testing::TestView {
            type_symbol: runner,
            text: RUNNER_VIEW.into(),
            members: vec![(runner, ty_line, ty_col, 6), (run, run_line, run_col, 3)],
        });
        t.signatures.push((run, RUN_SIGNATURE.into()));
    });
    let c = Cancellation::default();
    let hover = f
        .router
        .hover_detailed(&point(HERO_PATH, word(HERO_CS, "Run", 0)), &c)
        .unwrap()
        .value
        .unwrap();
    assert!(
        hover
            .documentation
            .contains("<summary>Runs the thing.</summary>"),
        "{:?}",
        hover.documentation
    );
    assert_eq!(&*hover.signature, RUN_SIGNATURE);
    // 源码成员不受影响：索引里没有文档时仍是空文档 + 显示名。
    let hover = f
        .router
        .hover_detailed(&point(HERO_PATH, word(HERO_CS, "Player", 0)), &c)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(&*hover.documentation, "");
    assert_eq!(&*hover.signature, "Player");
}

#[test]
fn stale_versions_untrusted_index_and_cancellation() {
    let f = fixture();
    let v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    let mut p = point_in(PLAYER_PATH, &*v0, word(PLAYER_CS, "Health", 1));
    p.version = Version(v0.version().0 + 1_000_000);
    assert!(matches!(
        f.router
            .navigate_detailed(NavigationKind::Definition, &p, &Cancellation::default()),
        Err(Error::Stale)
    ));
    // 不可信的索引（配置变化、正在重建）：L2 结果整体标为近似。
    let snap = f.semantic.snapshot().unwrap();
    f.semantic.set(
        Some(L2Snapshot {
            trusted: false,
            ..snap
        }),
        SemanticStatus::Building { percent: None },
    );
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "Move", 0)),
    );
    assert_eq!(r.value.len(), 1);
    assert_eq!(r.value[0].precision, Precision::Approximate);
    assert!(matches!(r.reason, Some(ApproximationReason::IndexNotReady)));
    let c = Cancellation::default();
    c.cancel();
    assert!(matches!(
        f.router
            .navigate_detailed(NavigationKind::Definition, &point(HERO_PATH, (0, 0)), &c),
        Err(Error::Cancelled)
    ));
}

#[test]
fn reload_after_external_edit_is_mapped() {
    let f = fixture();
    let _v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    let changed = format!("// header\n// header\n{PLAYER_CS}");
    let text = Arc::new(RopeText::from_text(&changed));
    f.router
        .buffers()
        .reloaded(
            &wp(PLAYER_PATH),
            text.clone(),
            hash_bytes(changed.as_bytes()),
        )
        .unwrap();
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*text, word(&changed, "Health", 1)),
    );
    let def = word(&changed, "Health", 0);
    assert_eq!(
        starts(&r),
        [(
            PLAYER_PATH.into(),
            u64::from(def.0),
            u64::from(def.1),
            EXACT
        )]
    );
    // 磁盘内容与索引不同：已请求重建；在此之前映射照常工作。
    assert_eq!(f.semantic.requested(), [PLAYER_PATH]);
}

#[test]
fn file_symbols_inactive_regions_and_unsupported_tokens() {
    let f = fixture();
    let c = Cancellation::default();
    let res = Resource::Workspace(wp(PLAYER_PATH));
    let symbols = f.router.file_symbols(&res, Version(0), &c).unwrap();
    assert_eq!(symbols.precision, Precision::Syntax);
    assert!(!symbols.value.is_empty());
    let regions = f.router.inactive_regions(&res, Version(0), &c).unwrap();
    assert!(regions.value.is_empty());
    assert_eq!(regions.precision, Precision::Exact);
    assert!(matches!(
        f.router.semantic_tokens(&res, Version(0), &c),
        Err(Error::Unsupported)
    ));
    // 没有 L1：工作区符号未就绪。
    assert!(matches!(
        f.router.workspace_symbols("Pla", &c),
        Err(Error::NotReady)
    ));
}

/// `#if` 不生效的区域：L2 没有信息，不返回“精确的空结果”，而是回退并说明原因。
#[test]
fn inactive_preprocessor_region_falls_back() {
    let line = word(PLAYER_CS, "step", 1).0;
    // 文件序号：Hero 0、Player 1（按路径排序）。
    let f = fixture_with(|t| t.inactive.push((1, line, line + 1)));
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(PLAYER_PATH, word(PLAYER_CS, "Health", 1)),
    );
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Approximate);
    assert!(matches!(&r.reason, Some(ApproximationReason::Other(s)) if &**s == INACTIVE_REASON));
    // 局部变量仍由 L0 解析。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(PLAYER_PATH, word(PLAYER_CS, "step", 1)),
    );
    assert_eq!(r.precision, Precision::Syntax);
    assert_eq!(r.value.len(), 1);
    // 区域外照常精确；不生效区域按行给出。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point(HERO_PATH, word(HERO_CS, "Move", 0)),
    );
    assert_eq!(r.precision, Precision::Exact);
    let regions = f
        .router
        .inactive_regions(
            &Resource::Workspace(wp(PLAYER_PATH)),
            Version(0),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(regions.value.len(), 1);
    assert_eq!(regions.value[0].start().line.0, u64::from(line));
}
