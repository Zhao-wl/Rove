use super::*;
use std::path::Path;

fn wp(p: &str) -> WorkspacePath {
    WorkspacePath::new(p).unwrap()
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Fixture {
    dir: tempfile::TempDir,
    files: Vec<&'static str>,
}

impl Fixture {
    fn new(files: &[(&'static str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            write(dir.path(), rel, text);
        }
        Self {
            dir,
            files: files.iter().map(|(p, _)| *p).collect(),
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn paths(&self) -> Vec<WorkspacePath> {
        self.files.iter().map(|p| wp(p)).collect()
    }

    fn index(&self) -> L1Index {
        let index = L1Index::new(L1Options::new(self.root()).with_threads(2));
        index.build(self.paths(), &Cancellation::default()).unwrap();
        index
    }
}

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

/// 在 `file` 的源码里找 `needle` 第 `nth` 次出现的位置（UTF-8 字节列）。
fn find(root: &Path, file: &str, needle: &str, nth: usize) -> Position {
    let text = std::fs::read_to_string(root.join(file)).unwrap();
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let offset = text.match_indices(needle).nth(nth).expect("needle").0;
    let line = text[..offset].matches('\n').count();
    let col = offset - text[..offset].rfind('\n').map_or(0, |i| i + 1);
    pos(line as u64, col as u64)
}

fn resolve(
    index: &L1Index,
    f: &Fixture,
    file: &str,
    needle: &str,
    nth: usize,
    text: &str,
) -> Vec<String> {
    let at = find(f.root(), file, needle, nth);
    let r = index
        .resolve_type(&wp(file), at, text, &Cancellation::default())
        .unwrap();
    assert_eq!(r.precision, Precision::Approximate);
    r.value
        .iter()
        .map(|s| {
            if s.container.is_empty() {
                format!("{}`{}", s.name, s.arity)
            } else {
                format!("{}.{}`{}", s.container, s.name, s.arity)
            }
        })
        .collect()
}

fn path_of(t: &Target) -> &str {
    match &t.resource {
        Resource::Workspace(p) => p.as_str(),
        other => panic!("unexpected resource {other:?}"),
    }
}

#[test]
fn partial_class_in_three_files_is_one_symbol_with_three_locations() {
    let f = Fixture::new(&[
        (
            "A/Player.cs",
            "namespace Game { public partial class Player { void Move() {} } }",
        ),
        (
            "B/Player.Net.cs",
            "namespace Game {\n  partial class Player { int hp; }\n}",
        ),
        (
            "C/Player.UI.cs",
            "namespace Game;\n\npartial class Player\n{\n}\n",
        ),
        ("D/Other.cs", "namespace Other { partial class Player {} }"),
    ]);
    let index = f.index();
    let player = index.find_type("Game.Player").expect("aggregated type");
    assert_eq!(player.kind, DeclKind::Class);
    assert_eq!(player.container.as_ref(), "Game");
    let files: Vec<&str> = player.locations.iter().map(path_of).collect();
    assert_eq!(files, ["A/Player.cs", "B/Player.Net.cs", "C/Player.UI.cs"]);
    // 选区是名字本身。
    let ui = &player.locations[2];
    assert_eq!(ui.selection.start(), pos(2, 14));
    assert_eq!(ui.selection.end(), pos(2, 20));
    assert_eq!(player.to_workspace_symbols().count(), 3);

    let hits = index
        .workspace_symbols("Player", 10, &Cancellation::default())
        .unwrap();
    assert_eq!(hits.precision, Precision::Syntax);
    let players: Vec<_> = hits.value.iter().filter(|s| &*s.name == "Player").collect();
    assert_eq!(players.len(), 2, "Game.Player 与 Other.Player 是两个符号");
}

#[test]
fn nested_generic_record_and_file_scoped_namespace() {
    let f = Fixture::new(&[
        (
            "Coll.cs",
            "namespace Coll {\n  class List<T> {}\n  class List<T1, T2> {}\n  class List {}\n}\n",
        ),
        (
            "Outer.cs",
            "namespace N;\n\npublic class Outer<T>\n{\n    public class Inner { public struct Deep {} }\n    public record Point(int X, int Y);\n    public record struct Pair(int A);\n    public delegate void Handler<TArg>(TArg a);\n}\n",
        ),
    ]);
    let index = f.index();
    for (q, arity) in [("Coll.List", 0), ("Coll.List<>", 1), ("Coll.List<,>", 2)] {
        let s = index.find_type(q).unwrap_or_else(|| panic!("{q}"));
        assert_eq!(s.arity, arity, "{q}");
        assert_eq!(s.locations.len(), 1);
    }
    assert!(index.find_type("Coll.List<,,>").is_none());

    let inner = index.find_type("N.Outer<>.Inner").expect("nested");
    assert_eq!(inner.container.as_ref(), "N.Outer");
    let deep = index
        .find_type("N.Outer<T>.Inner.Deep")
        .expect("doubly nested");
    assert_eq!(deep.kind, DeclKind::Struct);
    assert_eq!(
        index.find_type("N.Outer<T>.Point").unwrap().kind,
        DeclKind::Record
    );
    assert_eq!(
        index.find_type("N.Outer<T>.Pair").unwrap().kind,
        DeclKind::Record
    );
    let handler = index.find_type("N.Outer<T>.Handler<X>").unwrap();
    assert_eq!((handler.kind, handler.arity), (DeclKind::Delegate, 1));
    // 文件级命名空间：顶层类型属于 N。
    assert_eq!(
        index.find_type("N.Outer<>").unwrap().container.as_ref(),
        "N"
    );
    assert!(index.find_type("Outer<>").is_none());

    // 类型引用按参数个数区分重名的泛型。
    write(
        f.root(),
        "Use.cs",
        "using Coll;\nclass U { List<int> a; List<int, string> b; List c; }\n",
    );
    index
        .apply_changes(
            &[L1Change::Upserted(wp("Use.cs"))],
            &Cancellation::default(),
        )
        .unwrap();
    let at = |needle: &str| find(f.root(), "Use.cs", needle, 0);
    let names = |text: &str, needle: &str| -> Vec<u8> {
        index
            .resolve_type(&wp("Use.cs"), at(needle), text, &Cancellation::default())
            .unwrap()
            .value
            .iter()
            .map(|s| s.arity)
            .collect()
    };
    assert_eq!(names("List<int>", "List<int>"), [1]);
    assert_eq!(names("List<int, string>", "List<int, string>"), [2]);
    assert_eq!(names("List", "List c"), [0]);
}

#[test]
fn type_resolution_follows_namespaces_usings_static_and_aliases() {
    let f = Fixture::new(&[
        (
            "Lib.cs",
            "namespace Lib.Core { public class Engine {} public static class Factory { public class Product {} } }\nnamespace Lib.Ui { public class Engine {} public class Button {} }\n",
        ),
        (
            "App.cs",
            "using Lib.Core;\nusing static Lib.Core.Factory;\nusing Btn = Lib.Ui.Button;\nusing UiNs = Lib.Ui;\n\nnamespace App.Feature\n{\n    class Local {}\n    class Host\n    {\n        class Nested {}\n        Engine e; Product p; Btn b; UiNs.Engine u; Local l; Nested n; Missing m; global::Lib.Ui.Button g;\n    }\n}\n",
        ),
        (
            "Globals.cs",
            "global using Lib.Widgets;\nnamespace Lib.Widgets { class Slider {} class Engine {} }\n",
        ),
        ("Other.cs", "namespace App { class Shared {} }\nnamespace App.Feature.Deep { class Shared {} }\n"),
        ("Z.cs", "namespace Z { class UsesGlobal { Slider s; } }\n"),
    ]);
    let index = f.index();
    let r = |needle: &str, text: &str| resolve(&index, &f, "App.cs", needle, 0, text);
    // Engine 同时可经本文件的 using Lib.Core 与全局 using Lib.Widgets 到达：同一层有歧义时全部返回。
    assert_eq!(
        r("Engine e", "Engine"),
        ["Lib.Core.Engine`0", "Lib.Widgets.Engine`0"]
    );
    assert_eq!(
        r("Product p", "Product"),
        ["Lib.Core.Factory.Product`0"],
        "using static"
    );
    assert_eq!(r("Btn b", "Btn"), ["Lib.Ui.Button`0"], "using 别名");
    assert_eq!(
        r("UiNs.Engine", "UiNs.Engine"),
        ["Lib.Ui.Engine`0"],
        "命名空间别名"
    );
    assert_eq!(r("Local l", "Local"), ["App.Feature.Local`0"]);
    assert_eq!(r("Nested n", "Nested"), ["App.Feature.Host.Nested`0"]);
    assert!(r("Missing m", "Missing").is_empty());
    assert_eq!(r("global::", "global::Lib.Ui.Button"), ["Lib.Ui.Button`0"]);
    // 外层命名空间里的类型：App.Feature 内引用 Shared → App.Shared（不是更深的 App.Feature.Deep.Shared）。
    assert_eq!(r("Local l", "Shared"), ["App.Shared`0"]);
    // 全局 using 对其他文件生效。
    assert_eq!(
        resolve(&index, &f, "Z.cs", "Slider s", 0, "Slider"),
        ["Lib.Widgets.Slider`0"]
    );
}

#[test]
fn members_are_extracted_and_always_approximate() {
    let f = Fixture::new(&[(
        "M.cs",
        "namespace M {\nclass C<T> {\n  public C() {}\n  ~C() {}\n  int a, b;\n  public event System.Action Fired, Done;\n  public event System.Action Custom { add {} remove {} }\n  public int P { get; set; }\n  public int this[int i] => i;\n  public static C<T> operator +(C<T> x, C<T> y) => x;\n  public static implicit operator int(C<T> c) => 0;\n  public void Init() {}\n  public void Init<TA, TB>(int x) { void Local() {} }\n#if UNITY_EDITOR\n  void EditorOnly() {}\n#else\n  void PlayerOnly() {}\n#endif\n}\nenum E { One, Two }\nclass D { void Init() {} }\n}\n",
    )]);
    let index = f.index();
    let c = Cancellation::default();
    let kinds = |name: &str| -> Vec<DeclKind> {
        let r = index.member_candidates(name, &c).unwrap();
        assert_eq!(r.precision, Precision::Approximate, "{name}");
        r.value.iter().map(|s| s.kind).collect()
    };
    assert_eq!(kinds("C"), [DeclKind::Constructor]);
    assert_eq!(kinds("~C"), [DeclKind::Destructor]);
    assert_eq!(kinds("a"), [DeclKind::Field]);
    assert_eq!(kinds("b"), [DeclKind::Field]);
    assert_eq!(kinds("Fired"), [DeclKind::Event]);
    assert_eq!(kinds("Done"), [DeclKind::Event]);
    assert_eq!(kinds("Custom"), [DeclKind::Event]);
    assert_eq!(kinds("P"), [DeclKind::Property]);
    assert_eq!(kinds("this"), [DeclKind::Indexer]);
    assert_eq!(kinds("operator +"), [DeclKind::Operator]);
    assert_eq!(kinds("implicit operator int"), [DeclKind::Operator]);
    assert_eq!(kinds("One"), [DeclKind::EnumMember]);
    assert_eq!(kinds("EditorOnly"), [DeclKind::Method], "#if 两侧都收集");
    assert_eq!(kinds("PlayerOnly"), [DeclKind::Method]);
    assert!(kinds("Local").is_empty(), "不进入方法体");
    // 三个 Init（含泛型重载、另一个类）：全部列出，全部近似。
    let inits = index.member_candidates("Init", &c).unwrap();
    assert_eq!(inits.value.len(), 3);
    assert!(inits.value.iter().all(|s| s.kind == DeclKind::Method));
    let containers: BTreeSet<&str> = inits.value.iter().map(|s| &*s.container).collect();
    assert_eq!(containers, BTreeSet::from(["M.C", "M.D"]));
    assert!(inits.value.iter().any(|s| s.arity == 2));
    // 类型名不当成员。
    assert!(kinds("E").is_empty());
    // 工作区符号搜索里也有成员（与类型一起），精确度为 Syntax，不会是 Exact。
    let ws = index.workspace_symbols("init", 50, &c).unwrap();
    assert_ne!(ws.precision, Precision::Exact);
    assert_eq!(ws.value.iter().filter(|s| &*s.name == "Init").count(), 3);
}

#[test]
fn workspace_symbol_search_ranks_and_limits() {
    let f = Fixture::new(&[(
        "S.cs",
        "class PlayerController {} class Player {} class ApplyCache {} class PlayerControllerEditor {} class Unrelated {}",
    )]);
    let index = f.index();
    let c = Cancellation::default();
    let names = |q: &str, limit: usize| -> Vec<String> {
        index
            .workspace_symbols(q, limit, &c)
            .unwrap()
            .value
            .iter()
            .map(|s| s.name.to_string())
            .collect()
    };
    assert_eq!(names("player", 10)[0], "Player");
    assert_eq!(
        names("pc", 10)[..2],
        ["PlayerController", "PlayerControllerEditor"]
    );
    assert_eq!(names("pc", 1), ["PlayerController"]);
    assert!(names("", 10).is_empty());
    assert!(names("zzz", 10).is_empty());
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(matches!(
        index.workspace_symbols("p", 10, &cancelled),
        Err(Error::Cancelled)
    ));
}

#[test]
fn persistence_skips_unchanged_files_and_incremental_updates_reparse_one() {
    let f = Fixture::new(&[
        ("A.cs", "namespace P { class A {} }"),
        ("B.cs", "namespace P { class B {} }"),
        ("C.cs", "namespace P { class C {} }"),
        ("notes.txt", "not C#"),
    ]);
    let cache = tempfile::tempdir().unwrap();
    let c = Cancellation::default();
    let open = || L1Index::new(L1Options::new(f.root()).with_cache_dir(cache.path()));

    let first = open();
    let report = first.build_and_save(f.paths(), &c).unwrap();
    assert_eq!((report.files, report.parsed), (3, 3));
    assert!(cache.path().join(CACHE_FILE_NAME).is_file());
    drop(first);

    // 关闭后重新打开：从缓存载入，不重扫未变化的文件。
    let second = open();
    assert!(!second.is_scanned(&wp("A.cs")));
    let report = second.build(f.paths(), &c).unwrap();
    assert_eq!(report.loaded_from_cache, 3);
    assert_eq!(report.parsed, 0, "{report:?}");
    assert_eq!(report.unchanged, 3);
    assert_eq!(second.stats().files_parsed_total, 0);
    assert!(second.is_scanned(&wp("A.cs")));
    assert!(second.find_type("P.B").is_some());

    // 单文件修改：只解析这一个。
    write(f.root(), "B.cs", "namespace P { class B2 {} }");
    let report = second
        .apply_changes(&[L1Change::Upserted(wp("B.cs"))], &c)
        .unwrap();
    assert_eq!(report.parsed, 1);
    assert!(second.find_type("P.B").is_none());
    assert!(second.find_type("P.B2").is_some());
    assert_eq!(second.stats().files_parsed_total, 1);

    // 修改时间变了、内容没变：只算哈希，不解析。
    let a = f.root().join("A.cs");
    let file = std::fs::File::options().write(true).open(&a).unwrap();
    file.set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    drop(file);
    let report = second
        .apply_changes(&[L1Change::Upserted(wp("A.cs"))], &c)
        .unwrap();
    assert_eq!((report.parsed, report.rehashed), (0, 1));

    // 删除与重命名。
    std::fs::rename(f.root().join("C.cs"), f.root().join("C2.cs")).unwrap();
    let report = second
        .apply_changes(
            &[L1Change::Renamed {
                from: wp("C.cs"),
                to: wp("C2.cs"),
            }],
            &c,
        )
        .unwrap();
    assert_eq!((report.removed, report.parsed), (1, 1));
    let cls = second.find_type("P.C").unwrap();
    assert_eq!(path_of(&cls.locations[0]), "C2.cs");
    assert!(!second.is_scanned(&wp("C.cs")));
    second
        .apply_changes(&[L1Change::Removed(wp("C2.cs"))], &c)
        .unwrap();
    assert!(second.find_type("P.C").is_none());
    second.save().unwrap();

    // 第三次打开：B 的新内容与删除都已持久化。
    let third = open();
    let report = third.build([wp("A.cs"), wp("B.cs")], &c).unwrap();
    assert_eq!(report.parsed, 0, "{report:?}");
    assert!(third.find_type("P.B2").is_some());
    assert!(third.find_type("P.C").is_none());

    // 列表里没有的文件在 build 时移除。
    let report = third.build([wp("A.cs")], &c).unwrap();
    assert_eq!(report.removed, 1);
    assert!(third.find_type("P.B2").is_none());
}

#[test]
fn corrupt_or_foreign_cache_falls_back_to_full_build() {
    let f = Fixture::new(&[("A.cs", "class A {}")]);
    let cache = tempfile::tempdir().unwrap();
    let c = Cancellation::default();
    let index = L1Index::new(L1Options::new(f.root()).with_cache_dir(cache.path()));
    index.build_and_save(f.paths(), &c).unwrap();
    let file = cache.path().join(CACHE_FILE_NAME);
    let mut bytes = std::fs::read(&file).unwrap();
    assert!(persist::decode(&bytes).is_some());
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    std::fs::write(&file, &bytes).unwrap();
    assert!(persist::decode(&bytes).is_none());
    assert!(persist::decode(&bytes[..10]).is_none());
    assert!(persist::decode(&[]).is_none());
    let again = L1Index::new(L1Options::new(f.root()).with_cache_dir(cache.path()));
    let report = again.build(f.paths(), &c).unwrap();
    assert_eq!((report.loaded_from_cache, report.parsed), (0, 1));
}

#[test]
fn progress_and_scanned_flags() {
    let f = Fixture::new(&[("A.cs", "class A {}"), ("Gone.cs", "class G {}")]);
    let index = L1Index::new(L1Options::new(f.root()));
    assert_eq!(index.progress().state, L1State::Empty);
    assert!(!index.is_scanned(&wp("A.cs")));
    std::fs::remove_file(f.root().join("Gone.cs")).unwrap();
    let g0 = index.generation();
    let report = index.build(f.paths(), &Cancellation::default()).unwrap();
    assert_eq!((report.parsed, report.failed), (1, 1));
    let p = index.progress();
    assert_eq!((p.state, p.done, p.total), (L1State::Ready, 2, 2));
    assert!(index.is_scanned(&wp("A.cs")));
    assert!(!index.is_scanned(&wp("Gone.cs")));
    assert!(index.generation() > g0);
    let stats = index.stats();
    assert_eq!((stats.files, stats.declarations, stats.symbols), (1, 1, 1));
    assert!(stats.approx_heap_bytes > 0);
    let r = index
        .workspace_symbols("A", 5, &Cancellation::default())
        .unwrap();
    assert_eq!(r.index_generation, index.generation());
    assert!(r.version.is_none());
    assert!(matches!(
        index.resolve_type(&wp("Gone.cs"), pos(0, 0), "A", &Cancellation::default()),
        Err(Error::NotReady)
    ));
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(matches!(
        index.build(f.paths(), &cancelled),
        Err(Error::Cancelled)
    ));
}

#[test]
fn bom_and_utf16_sources_use_decoded_utf8_columns() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Bom.cs"), b"\xEF\xBB\xBFclass Bom {}").unwrap();
    let utf16: Vec<u8> = std::iter::once(0xFEFFu16)
        .chain("class Wide { /* é */ int ü; }".encode_utf16())
        .flat_map(|u| u.to_le_bytes())
        .collect();
    std::fs::write(dir.path().join("Wide.cs"), utf16).unwrap();
    let index = L1Index::new(L1Options::new(dir.path()));
    index
        .build([wp("Bom.cs"), wp("Wide.cs")], &Cancellation::default())
        .unwrap();
    let bom = index.find_type("Bom").unwrap();
    assert_eq!(bom.locations[0].selection.start(), pos(0, 6));
    let field = index
        .member_candidates("ü", &Cancellation::default())
        .unwrap();
    // "class Wide { /* é */ int " 是 26 个字节（é 占 2 个）。
    assert_eq!(field.value[0].locations[0].selection.start(), pos(0, 26));
    assert_eq!(field.value[0].locations[0].selection.end(), pos(0, 28));
}

#[test]
fn class_header_split_from_body_by_preprocessor_is_recovered() {
    // tree-sitter 不做预处理：`{` 落在 `#if` / `#else` 两个分支里时整段是 ERROR。
    let src = "namespace N\n{\n    public abstract class P : S\n#if E\n    {\n        private class Q : A { }\n    }\n#else\n    {\n    }\n#endif\n\n    public abstract class P<T> : P\n    {\n    }\n}\n";
    let f = Fixture::new(&[("P.cs", src)]);
    let index = f.index();
    assert!(index.find_type("N.P").is_some());
    assert!(
        index.find_type("N.P.Q").is_some(),
        "#if 分支里的嵌套类型保留"
    );
    assert!(
        index.find_type("N.P<>").is_some(),
        "后面的兄弟声明仍在命名空间里"
    );
    assert_eq!(resolve(&index, &f, "P.cs", ": P\n", 0, "P"), ["N.P`0"]);
    // 没有语法错误的文件不走涂抹，`#else` 分支里的声明照常索引。
    let g = Fixture::new(&[(
        "Q.cs",
        "namespace M {\n#if E\n  class A {}\n#else\n  class B {}\n#endif\n}\n",
    )]);
    let index = g.index();
    assert!(index.find_type("M.A").is_some() && index.find_type("M.B").is_some());
}

#[test]
fn source_copies_resolve_to_the_nearest_one_but_partials_stay() {
    let f = Fixture::new(&[
        ("Assets/Game/Role.cs", "namespace G { enum Role { A } }"),
        (
            ".backup/run1/Assets/Game/Role.cs",
            "namespace G { enum Role { A } }",
        ),
        (
            "Assets/Game/Unit.cs",
            "namespace G { partial class Unit { Role r; } }",
        ),
        (
            "Assets/Game/Net/Unit.Net.cs",
            "namespace G { partial class Unit {} }",
        ),
        (
            "Assets/View/Hud.cs",
            "namespace G { class Hud { Unit u; } }",
        ),
    ]);
    let index = f.index();
    assert_eq!(index.find_type("G.Role").unwrap().locations.len(), 2);
    let at = find(f.root(), "Assets/Game/Unit.cs", "Role", 0);
    let r = index
        .resolve_type(
            &wp("Assets/Game/Unit.cs"),
            at,
            "Role",
            &Cancellation::default(),
        )
        .unwrap();
    let files: Vec<&str> = r.value[0].locations.iter().map(path_of).collect();
    assert_eq!(files, ["Assets/Game/Role.cs"]);
    let at = find(f.root(), "Assets/View/Hud.cs", "Unit", 0);
    let r = index
        .resolve_type(
            &wp("Assets/View/Hud.cs"),
            at,
            "Unit",
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        r.value[0].locations.len(),
        2,
        "文件名不同的 partial 部分都保留"
    );
}
