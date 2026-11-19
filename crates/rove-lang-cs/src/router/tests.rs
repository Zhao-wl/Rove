//! 路由的单元测试：构造的索引（真实读取端打开）+ 真实的 RopeText 缓冲区。
use super::testing::{
    find_word, sample_index, SampleSymbols, HERO_CS, HERO_PATH, KIND_FIELD, KIND_LOCAL,
    KIND_METHOD, KIND_PARAMETER, KIND_PROPERTY, KIND_TYPE, PLAYER_CS, PLAYER_PATH,
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
    fixture_files(&[], |index, _| adjust(index))
}

/// 示例项目 + 额外的文件（路径要排在 `Assets/Player.cs` 之后，文件号从 2 开始）。
fn fixture_files(
    extra: &[(&str, &str)],
    adjust: impl FnOnce(&mut testing::TestIndex, &SampleSymbols),
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Assets")).unwrap();
    std::fs::write(dir.path().join(PLAYER_PATH), PLAYER_CS).unwrap();
    std::fs::write(dir.path().join(HERO_PATH), HERO_CS).unwrap();
    let (mut index, s) = sample_index(PLAYER_CS, HERO_CS);
    for (path, text) in extra {
        std::fs::write(dir.path().join(path), text).unwrap();
        index.file(path, hash_bytes(text.as_bytes()));
    }
    adjust(&mut index, &s);
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

/// 保存：磁盘上写成 `text`，并按同一份字节登记给缓冲区。
fn save(f: &Fixture, path: &str, text: &str) {
    std::fs::write(f._dir.path().join(path), text).unwrap();
    f.router
        .buffers()
        .saved(&wp(path), hash_bytes(text.as_bytes()))
        .unwrap();
}

/// 索引换代：按 `player` 的当前内容重建示例索引，世代设为 `generation`。
fn reindex(f: &Fixture, player: &str, generation: u64) {
    let (index, _) = sample_index(player, HERO_CS);
    let path = f._dir.path().join(format!("sample-{generation}.idx"));
    index.write(&path).unwrap();
    let holder: Arc<dyn IndexHolder> = Arc::new(CsIndex::open(&path).unwrap());
    f.semantic.set(
        Some(L2Snapshot {
            holder,
            generation,
            trusted: true,
        }),
        SemanticStatus::Ready,
    );
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
    // 类型定义：类型本身指向自己的定义；`var step = speed * 2` 推不出类型 → 确实没有结果。
    let r = nav(
        &f,
        NavigationKind::TypeDefinition,
        &point(HERO_PATH, word(HERO_CS, "Player", 0)),
    );
    assert_eq!(r.value.len(), 1);
    let r = nav(
        &f,
        NavigationKind::TypeDefinition,
        &point(PLAYER_PATH, word(PLAYER_CS, "step", 1)),
    );
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Exact);
    let _ = f.s;
}

/// #60 转到类型定义的语义：光标在变量 / 字段 / 属性 / 参数 / 泛型实参上时跳到它**声明类型**的
/// 定义处。索引不记录声明类型，所以是“到声明处把写出来的类型名读回来，再用索引换成类型符号”。
const WRITES_PATH: &str = "Assets/Writes.cs";
const WRITES_CS: &str = "namespace Game
{
    public class Writes
    {
        private Player field;
        public Player Property { get; set; }

        void Use(Player parameter)
        {
            var local = new Player();
            Take(field, Property, parameter, local, count);
        }

        private int count;
    }
}
";

#[test]
fn type_definition_follows_the_declared_type() {
    let f = fixture_files(&[(WRITES_PATH, WRITES_CS)], |t, s| {
        let w = 2;
        // 写出来的类型名（字段、属性、参数、`new Player()`）都是 Player 的引用。
        for nth in 0..4 {
            let (line, col) = find_word(WRITES_CS, "Player", nth);
            t.reference(s.player, w, line, col);
        }
        let count = t.symbol("Game|F:Game.Writes.count", "count", KIND_FIELD, None);
        let members = [
            (
                t.symbol("Game|F:Game.Writes.field", "field", KIND_FIELD, None),
                "field",
            ),
            (
                t.symbol(
                    "Game|P:Game.Writes.Property",
                    "Property",
                    KIND_PROPERTY,
                    None,
                ),
                "Property",
            ),
            (t.local("parameter", KIND_PARAMETER, w, 0), "parameter"),
            (t.local("local", KIND_LOCAL, w, 1), "local"),
            (count, "count"),
        ];
        for (symbol, word) in members {
            // `count` 的声明写在使用之后：定义处是第二次出现。
            let (defs, uses) = if word == "count" { (1, 0) } else { (0, 1) };
            let (line, col) = find_word(WRITES_CS, word, defs);
            t.def(symbol, w, line, col);
            let (line, col) = find_word(WRITES_CS, word, uses);
            t.reference(symbol, w, line, col);
        }
    });
    let player = word(PLAYER_CS, "Player", 0);
    let expected = vec![(
        PLAYER_PATH.to_owned(),
        u64::from(player.0),
        u64::from(player.1),
        EXACT,
    )];
    // 光标在用处（不是声明处）上：字段、属性、参数、`var` 局部变量各一例。
    for word in ["field", "Property", "parameter", "local"] {
        let at = find_word(WRITES_CS, word, if word == "local" { 1 } else { 0 });
        let r = nav(&f, NavigationKind::TypeDefinition, &point(WRITES_PATH, at));
        assert_eq!(starts(&r), expected, "{word} 的声明类型");
    }
    // 光标在声明处的名字上，答案一样。
    for word in ["field", "Property", "parameter"] {
        let at = find_word(WRITES_CS, word, 0);
        let r = nav(&f, NavigationKind::TypeDefinition, &point(WRITES_PATH, at));
        assert_eq!(starts(&r), expected, "{word} 的声明");
    }
    // `int` 没有可跳的定义：空结果，而且是“确实没有”。
    let r = nav(
        &f,
        NavigationKind::TypeDefinition,
        &point(WRITES_PATH, find_word(WRITES_CS, "count", 0)),
    );
    assert!(r.value.is_empty());
    assert_eq!(r.precision, Precision::Exact);
}

/// 声明处的那一行被改过时位置映射不回去：不猜，给近似的空结果（声明在别处的符号不受影响）。
#[test]
fn type_definition_downgrades_when_the_declaration_changes_in_another_buffer() {
    const USE_PATH: &str = "Assets/Z.cs";
    const USE_CS: &str =
        "class Z { void Use(Writes writes) { Take(writes.field, writes.count); } }";
    let f = fixture_files(&[(WRITES_PATH, WRITES_CS), (USE_PATH, USE_CS)], |t, s| {
        let field = t.symbol("Game|F:Game.Writes.field", "field", KIND_FIELD, None);
        let (line, col) = find_word(WRITES_CS, "field", 0);
        t.def(field, 2, line, col);
        let (line, col) = find_word(USE_CS, "field", 0);
        t.reference(field, 3, line, col);
        t.relations
            .push((field, s.player, crate::index::REL_DECLARED_TYPE));
        let count = t.symbol("Game|F:Game.Writes.count", "count", KIND_FIELD, None);
        let (line, col) = find_word(WRITES_CS, "count", 1);
        t.def(count, 2, line, col);
        let (line, col) = find_word(USE_CS, "count", 0);
        t.reference(count, 3, line, col);
    });
    let at = point(USE_PATH, find_word(USE_CS, "field", 0));
    assert_eq!(
        nav(&f, NavigationKind::TypeDefinition, &at).precision,
        Precision::Exact
    );

    let count = point(USE_PATH, find_word(USE_CS, "count", 0));
    let indexed = nav(&f, NavigationKind::TypeDefinition, &count);
    assert_eq!(indexed.precision, Precision::Exact);
    assert!(indexed.value.is_empty());

    let original = open(&f, WRITES_PATH, WRITES_CS);
    let type_offset = WRITES_CS.find("Player field").unwrap();
    let changed_doc = edit(
        &f,
        WRITES_PATH,
        &original,
        type_offset,
        type_offset + "Player".len(),
        "Hero",
    );
    let changed = nav(&f, NavigationKind::TypeDefinition, &at);
    assert_eq!(changed.precision, Precision::Approximate);
    assert!(matches!(
        changed.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    assert!(changed.value.is_empty());

    let current = buffers::read_all(&*changed_doc).unwrap();
    let offset = current.find("int count").unwrap();
    edit(
        &f,
        WRITES_PATH,
        &changed_doc,
        offset,
        offset + "int".len(),
        "Player",
    );
    let changed = nav(&f, NavigationKind::TypeDefinition, &count);
    assert_eq!(changed.precision, Precision::Approximate);
    assert!(matches!(
        changed.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    assert!(changed.value.is_empty());
}

#[test]
fn type_definition_rejects_a_deleted_initializer_suffix() {
    const FILE: &str = "Assets/ZZ.cs";
    const SOURCE: &str = "class Z {
    void Run() {
        var value = Factory.Create()
            .ConvertToPlayer()
            ;
        Consume(value);
    }
}";
    let f = fixture_files(&[(FILE, SOURCE)], |t, s| {
        let decl = t.local("value", KIND_LOCAL, 2, SOURCE.find("value").unwrap() as u32);
        let (line, col) = find_word(SOURCE, "value", 0);
        t.def(decl, 2, line, col);
        let (line, col) = find_word(SOURCE, "value", 1);
        t.reference(decl, 2, line, col);
        t.relations
            .push((decl, s.player, crate::index::REL_DECLARED_TYPE));
    });
    let at = find_word(SOURCE, "value", 1);
    let indexed = nav(&f, NavigationKind::TypeDefinition, &point(FILE, at));
    assert_eq!(indexed.precision, Precision::Exact);
    assert_eq!(indexed.value.len(), 1);

    let old = open(&f, FILE, SOURCE);
    let removed = "            .ConvertToPlayer()\n";
    let start = SOURCE.find(removed).unwrap();
    let new = edit(&f, FILE, &old, start, start + removed.len(), "");
    let text = buffers::read_all(&*new).unwrap();
    let at = point_in(FILE, &*new, find_word(&text, "value", 1));
    let result = nav(&f, NavigationKind::TypeDefinition, &at);
    assert_eq!(result.precision, Precision::Approximate);
    assert!(matches!(
        result.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    assert!(result.value.is_empty());
}

#[test]
fn type_definition_maps_the_declaration_through_unsaved_edits() {
    let f = fixture_files(&[(WRITES_PATH, WRITES_CS)], |t, s| {
        for nth in 0..4 {
            let (line, col) = find_word(WRITES_CS, "Player", nth);
            t.reference(s.player, 2, line, col);
        }
        let field = t.symbol("Game|F:Game.Writes.field", "field", KIND_FIELD, None);
        let (line, col) = find_word(WRITES_CS, "field", 0);
        t.def(field, 2, line, col);
        let (line, col) = find_word(WRITES_CS, "field", 1);
        t.reference(field, 2, line, col);
    });
    let rope = open(&f, WRITES_PATH, WRITES_CS);
    // 在声明之前插入一行：声明整体下移，映射后仍然读得到类型名。
    let head = WRITES_CS.find("        private Player field;").unwrap();
    let rope = edit(&f, WRITES_PATH, &rope, head, head, "        // note\n");
    let text = buffers::read_all(&*rope).unwrap();
    let at = find_word(&text, "field", 1);
    let r = nav(
        &f,
        NavigationKind::TypeDefinition,
        &point_in(WRITES_PATH, &*rope, at),
    );
    let player = word(PLAYER_CS, "Player", 0);
    assert_eq!(
        starts(&r),
        [(
            PLAYER_PATH.to_owned(),
            u64::from(player.0),
            u64::from(player.1),
            EXACT
        )]
    );
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
fn file_symbols_and_inactive_regions() {
    let f = fixture();
    let c = Cancellation::default();
    let res = Resource::Workspace(wp(PLAYER_PATH));
    let symbols = f.router.file_symbols(&res, Version(0), &c).unwrap();
    assert_eq!(symbols.precision, Precision::Syntax);
    assert!(!symbols.value.is_empty());
    let regions = f.router.inactive_regions(&res, Version(0), &c).unwrap();
    assert!(regions.value.is_empty());
    assert_eq!(regions.precision, Precision::Exact);
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

/// #216：保存 → 索引换代 → **不查询**直接继续输入。基线必须在换代的那一版上推进，
/// 只有新敲的那几行降级，而不是整份文件。
#[test]
fn typing_right_after_a_reindex_only_degrades_the_new_edit() {
    let f = fixture();
    let v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    // ① 编辑 + 保存：磁盘与缓冲区都是 `saved`。
    let header = "// saved\n";
    let v1 = edit(&f, PLAYER_PATH, &v0, 0, 0, header);
    let saved = format!("{header}{PLAYER_CS}");
    save(&f, PLAYER_PATH, &saved);
    // ② 索引按磁盘内容换代。这之后一次查询都没有发生。
    reindex(&f, &saved, 2);
    // ③ 继续输入：改掉 `var step = speed * 2;` 这一行。
    let at = saved.find("* 2").unwrap();
    let v2 = edit(&f, PLAYER_PATH, &v1, at, at + 3, "* 3");
    let current = saved.replace("* 2", "* 3");
    // ④ 没被动过的行照旧精确（基线已经推进到换代的那一版）。
    let health = word(&current, "Health", 1);
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*v2, health),
    );
    let def = word(&current, "Health", 0);
    assert_eq!(
        starts(&r),
        [(
            PLAYER_PATH.into(),
            u64::from(def.0),
            u64::from(def.1),
            EXACT
        )],
        "换代后继续输入，未改动的行必须仍然精确"
    );
    assert_eq!(r.precision, Precision::Exact, "{:?}", r.reason);
    assert_eq!(r.index_generation, 2);
    // ⑤ 新敲的那一行降级（局部变量由 L0 兜底，结果是“语法”而不是 L2 的精确答案）。
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*v2, word(&current, "speed", 1)),
    );
    assert_ne!(r.precision, Precision::Exact, "{:?}", r.reason);
    // ⑥ 登记口径：基线推进到了索引那一版，编辑记录里只剩换代之后的那次编辑。
    let state = f.router.buffers().get(PLAYER_PATH).unwrap();
    assert_eq!(
        state.base_hash,
        hash_bytes(saved.as_bytes()),
        "基线必须是索引那一代的内容"
    );
    assert_eq!(state.current_hash, None, "③ 之后还没保存");
    assert_eq!(state.edits.hunks().len(), 1, "只剩换代之后的那一次编辑");
}

/// 索引落在**中间某一次**保存上（用户已经又保存过一次）：基线推进到那一次，
/// 它之后的编辑照常参与映射。
#[test]
fn reindex_catching_an_older_save_still_rebases() {
    let f = fixture();
    let v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    let first = format!("// one\n{PLAYER_CS}");
    let v1 = edit(&f, PLAYER_PATH, &v0, 0, 0, "// one\n");
    save(&f, PLAYER_PATH, &first);
    let second = format!("// two\n{first}");
    let v2 = edit(&f, PLAYER_PATH, &v1, 0, 0, "// two\n");
    save(&f, PLAYER_PATH, &second);
    // 索引这一代读到的是第一次保存的内容。
    reindex(&f, &first, 2);
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*v2, word(&second, "Health", 1)),
    );
    let def = word(&second, "Health", 0);
    assert_eq!(
        starts(&r),
        [(
            PLAYER_PATH.into(),
            u64::from(def.0),
            u64::from(def.1),
            EXACT
        )],
        "基线推进到中间那一版后，整体平移一行仍然精确"
    );
    let state = f.router.buffers().get(PLAYER_PATH).unwrap();
    assert_eq!(state.base_hash, hash_bytes(first.as_bytes()));
    assert_eq!(state.current_hash, Some(hash_bytes(second.as_bytes())));
    // 磁盘上还有更新的一版：请求重建。
    assert_eq!(f.semantic.requested(), [PLAYER_PATH]);
}

/// 索引换到的内容既不是基线也不是任何检查点（例如外部进程改了磁盘）：仍然判为无法映射。
#[test]
fn reindex_to_unknown_content_stays_diverged() {
    let f = fixture();
    let v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    let v1 = edit(&f, PLAYER_PATH, &v0, 0, 0, "// mine\n");
    let elsewhere = format!("// theirs\n{PLAYER_CS}");
    reindex(&f, &elsewhere, 2);
    let r = nav(
        &f,
        NavigationKind::Definition,
        &point_in(PLAYER_PATH, &*v1, word(&elsewhere, "Health", 1)),
    );
    assert_ne!(r.precision, Precision::Exact);
    assert_eq!(
        f.router.buffers().get(PLAYER_PATH).unwrap().base_hash,
        hash_bytes(PLAYER_CS.as_bytes()),
        "对不上任何检查点时不能推进基线"
    );
}

// ------------------------------------------------------------------ #59 语义 token

/// 一次语义 token 查询（未打开的文件用版本 0）。
fn semantic(f: &Fixture, path: &str, version: Version) -> QueryResult<Arc<[SemanticToken]>> {
    f.router
        .semantic_tokens(
            &Resource::Workspace(wp(path)),
            version,
            &Cancellation::default(),
        )
        .unwrap()
}

/// `(类型, 修饰符)`，便于逐条断言。
fn token_at(
    r: &QueryResult<Arc<[SemanticToken]>>,
    (line, col): (u32, u32),
) -> (String, Vec<String>) {
    let t = r
        .value
        .iter()
        .find(|t| t.line.0 == u64::from(line) && t.column.0 == u64::from(col))
        .unwrap_or_else(|| panic!("({line},{col}) 应有语义 token：{:?}", r.value));
    (
        t.token_type.to_string(),
        t.modifiers.iter().map(|m| m.to_string()).collect(),
    )
}

#[test]
fn semantic_tokens_classify_the_sample_project() {
    let f = fixture();
    let r = semantic(&f, PLAYER_PATH, Version(0));
    assert_eq!(r.precision, Precision::Exact);
    // 类型声明：索引只知道「具名类型」，class 由本文件的语法树补上。
    assert_eq!(
        token_at(&r, find_word(PLAYER_CS, "Player", 0)),
        ("class".to_owned(), vec!["declaration".to_owned()])
    );
    // 字段的声明处与引用处是同一个类型，引用处没有 declaration。
    assert_eq!(
        token_at(&r, find_word(PLAYER_CS, "Health", 0)),
        ("field".to_owned(), vec!["declaration".to_owned()])
    );
    assert_eq!(
        token_at(&r, find_word(PLAYER_CS, "Health", 1)),
        ("field".to_owned(), Vec::<String>::new())
    );
    assert_eq!(
        token_at(&r, find_word(PLAYER_CS, "Move", 0)),
        ("method".to_owned(), vec!["declaration".to_owned()])
    );
    assert_eq!(
        token_at(&r, find_word(PLAYER_CS, "speed", 0)),
        ("parameter".to_owned(), vec!["declaration".to_owned()])
    );
    assert_eq!(
        token_at(&r, find_word(PLAYER_CS, "step", 0)),
        ("variable".to_owned(), vec!["declaration".to_owned()])
    );
    // 另一个文件里引用 Player：声明不在本文件，没有 L1 时退回笼统的 `type`。
    let hero = semantic(&f, HERO_PATH, Version(0));
    let (ty, mods) = token_at(&hero, find_word(HERO_CS, "Player", 0));
    assert_eq!(ty, "type");
    assert!(mods.is_empty(), "{mods:?}");
}

#[test]
fn symbols_from_the_base_class_library_carry_default_library() {
    const SRC: &str = "class A { void M() { var s = string.Empty; } }\n";
    const EXTRA: &str = "Assets/zz.cs";
    let f = fixture_files(&[(EXTRA, SRC)], |index, _| {
        let sym = index.symbol("mscorlib|T:System.String", "String", KIND_TYPE, None);
        let (line, col) = find_word(SRC, "string", 0);
        index.reference(sym, 2, line, col);
    });
    let r = semantic(&f, EXTRA, Version(0));
    assert_eq!(
        token_at(&r, find_word(SRC, "string", 0)),
        ("type".to_owned(), vec!["defaultLibrary".to_owned()])
    );
}

#[test]
fn an_edited_region_falls_back_to_syntax_only() {
    let f = fixture();
    let v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    let before = semantic(&f, PLAYER_PATH, v0.version());
    assert_eq!(before.precision, Precision::Exact);
    assert_eq!(
        before
            .value
            .iter()
            .filter(|t| &*t.token_type == "field")
            .count(),
        2
    );
    // 改 `Health -= step;` 那一行。
    let at = PLAYER_CS.find("step;").unwrap() + "step".len();
    let v1 = edit(&f, PLAYER_PATH, &v0, at, at, " + 1");
    let after = semantic(&f, PLAYER_PATH, v1.version());
    assert_eq!(after.precision, Precision::Approximate);
    assert!(matches!(
        after.reason,
        Some(ApproximationReason::ModifiedRegion)
    ));
    // 改过的那一行上的 token 全部消失，其余保留（不是整份文件退化）。
    let modified_line = PLAYER_CS[..at].matches('\n').count() as u64;
    assert!(after.value.iter().all(|t| t.line.0 != modified_line));
    assert_eq!(
        token_at(&after, find_word(PLAYER_CS, "Health", 0)),
        ("field".to_owned(), vec!["declaration".to_owned()]),
        "没改过的行仍然有语义 token"
    );
}

#[test]
fn mapped_semantic_declarations_keep_modifiers_after_whole_line_insertion() {
    const FILE: &str = "Assets/ZZ.cs";
    const SOURCE: &str = "class Example { static readonly int Count = 1; }\n";
    let f = fixture_files(&[(FILE, SOURCE)], |index, _| {
        let field = index.symbol("Game|F:Example.Count", "Count", KIND_FIELD, None);
        let (line, column) = find_word(SOURCE, "Count", 0);
        index.def(field, 2, line, column);
    });
    let before = open(&f, FILE, SOURCE);
    let after = edit(&f, FILE, &before, 0, 0, "// header\n");
    let result = semantic(&f, FILE, after.version());
    assert_eq!(result.precision, Precision::Approximate);
    let (_, column) = find_word(SOURCE, "Count", 0);
    let (kind, modifiers) = token_at(&result, (1, column));
    assert_eq!(kind, "field");
    assert!(modifiers.iter().any(|modifier| modifier == "readonly"));
    assert!(modifiers.iter().any(|modifier| modifier == "static"));
}

#[test]
fn dirty_buffer_does_not_make_an_untrusted_index_drawable() {
    let f = fixture_with(|index| index.inactive.push((1, 2, 8)));
    let before = open(&f, PLAYER_PATH, PLAYER_CS);
    let after = edit(&f, PLAYER_PATH, &before, 0, 0, "// header\n");
    let mut snapshot = f.semantic.snapshot().unwrap();
    snapshot.trusted = false;
    f.semantic
        .set(Some(snapshot), SemanticStatus::Building { percent: None });

    let tokens = semantic(&f, PLAYER_PATH, after.version());
    assert!(tokens.value.is_empty());
    assert!(matches!(
        tokens.reason,
        Some(ApproximationReason::IndexNotReady)
    ));
    let inactive = f
        .router
        .inactive_regions(
            &Resource::Workspace(wp(PLAYER_PATH)),
            after.version(),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(inactive.value.is_empty());
    assert!(matches!(
        inactive.reason,
        Some(ApproximationReason::IndexNotReady)
    ));
}

#[test]
fn an_unindexed_file_and_a_missing_index_produce_no_tokens() {
    let f = fixture();
    let missing = semantic(&f, "Assets/Nope.cs", Version(0));
    assert!(missing.value.is_empty());
    assert_eq!(missing.precision, Precision::Approximate);
    // 索引还没好：整条查询不可用（界面据此不画语义装饰）。
    f.semantic
        .set(None, SemanticStatus::Building { percent: Some(10) });
    assert!(matches!(
        f.router.semantic_tokens(
            &Resource::Workspace(wp(PLAYER_PATH)),
            Version(0),
            &Cancellation::default(),
        ),
        Err(Error::NotReady)
    ));
}

#[test]
fn only_csharp_files_have_semantic_tokens() {
    let f = fixture_files(&[("Assets/data.json", "{}")], |_, _| {});
    assert!(matches!(
        f.router.semantic_tokens(
            &Resource::Workspace(wp("Assets/data.json")),
            Version(0),
            &Cancellation::default(),
        ),
        Err(Error::Unsupported)
    ));
}

#[test]
fn a_stale_version_is_rejected_instead_of_answered() {
    let f = fixture();
    let v0 = open(&f, PLAYER_PATH, PLAYER_CS);
    assert!(matches!(
        f.router.semantic_tokens(
            &Resource::Workspace(wp(PLAYER_PATH)),
            Version(v0.version().0 + 7),
            &Cancellation::default(),
        ),
        Err(Error::Stale)
    ));
}

#[test]
fn an_untrusted_index_does_not_produce_semantic_tokens() {
    let f = fixture();
    assert!(!semantic(&f, PLAYER_PATH, Version(0)).value.is_empty());
    let snapshot = f.semantic.snapshot().unwrap();
    f.semantic.set(
        Some(L2Snapshot {
            trusted: false,
            ..snapshot
        }),
        SemanticStatus::Building { percent: Some(90) },
    );
    let after = semantic(&f, PLAYER_PATH, Version(0));
    assert!(after.value.is_empty());
    assert_eq!(after.precision, Precision::Approximate);
    assert!(matches!(
        after.reason,
        Some(ApproximationReason::IndexNotReady)
    ));
}
