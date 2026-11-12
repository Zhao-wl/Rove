use super::*;
use rove_base::{Line, Utf8Column};

fn symbols(language: &str, text: &str) -> Vec<Symbol> {
    extract(language, text, &Cancellation::default()).unwrap()
}

/// `名称:种类` 的缩进大纲，便于整体比较顺序、嵌套与种类。
fn outline(list: &[Symbol]) -> Vec<String> {
    fn walk(list: &[Symbol], depth: usize, out: &mut Vec<String>) {
        for s in list {
            out.push(format!(
                "{}{}:{}",
                "  ".repeat(depth),
                s.name,
                kind_name(&s.kind)
            ));
            walk(&s.children, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(list, 0, &mut out);
    out
}

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

/// 每个符号的选择区间都在完整区间之内，兄弟按位置排序（契约要求）。
fn assert_well_formed(list: &[Symbol]) {
    let mut last = None;
    for s in list {
        assert!(
            s.range.start() <= s.selection.start() && s.selection.end() <= s.range.end(),
            "{s:?}"
        );
        if let Some(prev) = last {
            assert!(prev <= s.range.start(), "{s:?}");
        }
        last = Some(s.range.start());
        for c in s.children.iter() {
            assert!(
                s.range.start() <= c.range.start() && c.range.end() <= s.range.end(),
                "{c:?}"
            );
        }
        assert_well_formed(&s.children);
    }
}

const UNITY: &str = r#"using UnityEngine;

namespace Game.Player
{
    [RequireComponent(typeof(Rigidbody))]
    public class PlayerController<T> : MonoBehaviour where T : class
    {
        public const float MaxSpeed = 10f;
        [SerializeField] private float speed, jump;
        public event System.Action Died;
        public event System.Action Moved { add { } remove { } }
        public int Health { get; private set; }
        public int this[int index] => index;

        public PlayerController() { }
        ~PlayerController() { }

        void Update() { }
        public void Move(Vector3 direction, ref float delta, params string[] tags) { }
        T Get<U>(U value) where U : struct => null;
        public static PlayerController<T> operator +(PlayerController<T> a, PlayerController<T> b) => a;
        public static implicit operator int(PlayerController<T> p) => 0;
        void IDisposable.Dispose() { }

#if UNITY_EDITOR
        void OnValidate() { }
#endif

        public enum State { Idle, Running = 2 }
        private struct Cache { int value; }
        public interface IListener { void OnDied(); }
        public delegate void Handler(int code);
    }

    public record Score(int Value);
    public record struct Point(int X, int Y);
}
"#;

#[test]
fn csharp_unity_script_outline() {
    let list = symbols("csharp", UNITY);
    assert_well_formed(&list);
    assert_eq!(
        outline(&list),
        [
            "Game.Player:namespace",
            "  PlayerController<T>:class",
            "    MaxSpeed:constant",
            "    speed:field",
            "    jump:field",
            "    Died:event",
            "    Moved:event",
            "    Health:property",
            "    this[int]:property",
            "    PlayerController():constructor",
            "    ~PlayerController():method",
            "    Update():method",
            "    Move(Vector3, ref float, params string[]):method",
            "    Get<U>(U):method",
            "    operator +(PlayerController<T>, PlayerController<T>):operator",
            "    implicit operator int(PlayerController<T>):operator",
            "    IDisposable.Dispose():method",
            "    OnValidate():method",
            "    State:enum",
            "      Idle:enumMember",
            "      Running:enumMember",
            "    Cache:struct",
            "      value:field",
            "    IListener:interface",
            "      OnDied():method",
            "    Handler(int):function",
            "  Score:class",
            "  Point:struct",
        ]
    );
    // 选择区间是名称；完整区间从特性开始。
    let class = &list[0].children[0];
    assert_eq!(class.selection.start(), pos(5, 17));
    assert_eq!(class.range.start(), pos(4, 4));
}

#[test]
fn csharp_file_scoped_namespace_owns_following_declarations() {
    let list = symbols(
        "csharp",
        "namespace A.B;\n\npublic class C { void M() {} }\nenum E { X }\n",
    );
    assert_well_formed(&list);
    assert_eq!(
        outline(&list),
        [
            "A.B:namespace",
            "  C:class",
            "    M():method",
            "  E:enum",
            "    X:enumMember"
        ]
    );
    assert_eq!(list[0].range.end(), pos(4, 0));
}

#[test]
fn csharp_empty_and_broken_files() {
    assert!(symbols("csharp", "").is_empty());
    assert!(symbols("csharp", "// only a comment\n").is_empty());
    // 缺少右花括号与方法体：仍然恢复出类型与之前的成员，不 panic。
    let list = symbols(
        "csharp",
        "public class Broken {\n    int a;\n    void M( {\n    void N() {}\n",
    );
    assert_well_formed(&list);
    let names = outline(&list);
    assert_eq!(names, ["Broken:class", "  a:field", "  N():method"]);
}

#[test]
fn markdown_heading_levels_nest() {
    let text = "intro\n\n# Title\ntext\n## Install ##\n### Windows\n## Usage\n#### Deep\n# Second\nSetext\n------\n";
    let list = symbols("markdown", text);
    assert_well_formed(&list);
    assert_eq!(
        outline(&list),
        [
            "Title:string",
            "  Install:string",
            "    Windows:string",
            "  Usage:string",
            "    Deep:string",
            "Second:string",
            "  Setext:string",
        ]
    );
    // 小节区间到下一个同级标题之前。
    assert_eq!(list[0].range.start(), pos(2, 0));
    assert_eq!(list[0].range.end(), pos(8, 0));
    assert!(symbols("markdown", "").is_empty());
    assert!(symbols("markdown", "no headings here\n").is_empty());
    // 未闭合的代码块里的 `#` 不是标题。
    let fenced = symbols("markdown", "# A\n```\n# not a heading\n");
    assert_eq!(outline(&fenced), ["A:string"]);
}

#[test]
fn json_keys_with_value_kinds() {
    let text = r#"{
  "name": "com.rove.sample",
  "version": 3,
  "enabled": true,
  "parent": null,
  "dependencies": { "a": "1.0", "b": { "deep": [] } },
  "files": ["x", { "k": 1 }]
}"#;
    let list = symbols("json", text);
    assert_well_formed(&list);
    assert_eq!(
        outline(&list),
        [
            "name:string",
            "version:number",
            "enabled:boolean",
            "parent:null",
            "dependencies:object",
            "  a:string",
            "  b:object",
            "    deep:array",
            "files:array",
            "  0:string",
            "  1:object",
            "    k:number",
        ]
    );
    assert!(symbols("json", "").is_empty());
    assert!(symbols("json", "{}").is_empty());
    // JSONC 注释与语法错误：能恢复的键照常产出。
    let broken = symbols("jsonc", "{\n // c\n \"a\": 1,\n \"b\": {\n");
    assert_well_formed(&broken);
    assert_eq!(outline(&broken)[0], "a:number");
}

#[test]
fn yaml_keys_and_documents() {
    let single = symbols(
        "yaml",
        "name: rove\nlist:\n  - one\n  - key: 2\nempty:\nflag: true\nflow: {a: 1, b: [x]}\n",
    );
    assert_well_formed(&single);
    assert_eq!(
        outline(&single),
        [
            "name:string",
            "list:array",
            "  0:string",
            "  1:object",
            "    key:number",
            "empty:null",
            "flag:boolean",
            "flow:object",
            "  a:number",
            "  b:array",
            "    0:string",
        ]
    );
    // Unity 序列化文件：多文档各成一棵子树，带标签与锚点。
    let unity = "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!1 &100\nGameObject:\n  m_Name: Player\n--- !u!4 &200\nTransform:\n  m_Father: {fileID: 0}\n";
    let docs = symbols("yaml", unity);
    assert_well_formed(&docs);
    assert_eq!(
        outline(&docs),
        [
            "1:document",
            "  GameObject:object",
            "    m_Name:string",
            "2:document",
            "  Transform:object",
            "    m_Father:object",
            "      fileID:number",
        ]
    );
    assert!(symbols("yaml", "").is_empty());
    let broken = symbols("yaml", "a: 1\nb: [unclosed\nc: 3\n");
    assert_well_formed(&broken);
}

#[test]
fn xml_elements_nest() {
    let text = "<?xml version=\"1.0\"?>\n<Project Sdk=\"x\">\n  <PropertyGroup>\n    <TargetFramework>net9.0</TargetFramework>\n  </PropertyGroup>\n  <ItemGroup><Compile Include=\"a.cs\" /></ItemGroup>\n</Project>\n";
    let list = symbols("xml", text);
    assert_well_formed(&list);
    assert_eq!(
        outline(&list),
        [
            "Project:field",
            "  PropertyGroup:field",
            "    TargetFramework:field",
            "  ItemGroup:field",
            "    Compile:field",
        ]
    );
    assert!(symbols("xml", "").is_empty());
    let broken = symbols("xml", "<a>\n  <b>\n  <c/>\n");
    assert_well_formed(&broken);
    assert!(
        outline(&broken).iter().any(|n| n.trim_start() == "c:field"),
        "{:?}",
        outline(&broken)
    );
}

#[test]
fn unsupported_language_and_huge_files() {
    assert_eq!(
        extract("plaintext", "x", &Cancellation::default()).unwrap_err(),
        Error::Unsupported
    );
    assert_eq!(
        availability("plaintext", SizeClass::Normal),
        Availability::Unsupported
    );
    assert_eq!(
        availability("csharp", SizeClass::Huge),
        Availability::TooLarge
    );
    assert_eq!(
        availability("csharp", SizeClass::Large),
        Availability::Available
    );
}

#[test]
fn cancellation_is_observed() {
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        extract("csharp", UNITY, &cancel).unwrap_err(),
        Error::Cancelled
    );
}

#[test]
fn index_paths_containers_and_keys() {
    let list = symbols("csharp", UNITY);
    let doc = DocumentSymbols::new(Version(1), "csharp".into(), list.into());
    let name = |id: SymbolId| doc.node(id).unwrap().symbol.name.to_string();
    // `Update` 方法体内（第 17 行）。
    let path = doc.path_at(pos(17, 25));
    assert_eq!(
        path.iter().map(|id| name(*id)).collect::<Vec<_>>(),
        ["Game.Player", "PlayerController<T>", "Update()"]
    );
    let update = *path.last().unwrap();
    assert_eq!(
        doc.node(update).unwrap().container.as_deref(),
        Some("PlayerController<T>")
    );
    assert_eq!(doc.ancestors(update), path);
    // 复用上一次的路径：同一成员内移动结果不变；移到类体空白处退回到类。
    assert_eq!(doc.path_from(&path, pos(17, 20)), path);
    assert_eq!(doc.path_from(&path, pos(18, 0)).len(), 2);
    assert_eq!(doc.path_from(&path, pos(18, 0)), doc.path_at(pos(18, 0)));
    // 文件开头不在任何符号里。
    assert!(doc.path_at(pos(0, 0)).is_empty());
    // 同名字段在同一父节点下得到不同的稳定键。
    let fields = symbols("csharp", "class A { void M() {} void M() {} }");
    let doc2 = DocumentSymbols::new(Version(1), "csharp".into(), fields.into());
    let keys: Vec<_> = doc2.iter().map(|(_, n)| n.key.to_string()).collect();
    assert_eq!(
        keys,
        ["/class:A", "/class:A/method:M()", "/class:A/method:M()#1"]
    );
    // 先序编号：父在子前，children 按位置。
    assert_eq!(doc.children(None).len(), 1);
    assert!(doc.iter().all(|(id, n)| n.parent.is_none_or(|p| p < id)));
}

#[test]
fn incremental_update_shares_unchanged_subtrees_and_reports_changes() {
    let v1 = "class A { void M() {} }\nclass B { int x; }\n";
    let v2 = "class A { void M() {} }\nclass B { int x; int y; }\n";
    let cancel = Cancellation::default();
    let first = update(
        Version(1),
        "csharp".into(),
        extract("csharp", v1, &cancel).unwrap(),
        false,
        None,
    );
    assert!(first.changed.is_none());
    let second = update(
        Version(2),
        "csharp".into(),
        extract("csharp", v2, &cancel).unwrap(),
        false,
        Some(&first.symbols),
    );
    // A 的子树没有变化：直接复用上一版的同一块内存。
    assert!(Arc::ptr_eq(
        &first.symbols.roots()[0].children,
        &second.symbols.roots()[0].children
    ));
    let changed: Vec<_> = second
        .changed
        .as_ref()
        .unwrap()
        .iter()
        .map(|id| second.symbols.node(*id).unwrap().key.to_string())
        .collect();
    assert_eq!(changed, ["/class:B", "/class:B/field:y"]);
    assert!(second.removed.is_empty());
    // 完全相同的文本：没有变化，整棵树复用。
    let third = update(
        Version(3),
        "csharp".into(),
        extract("csharp", v2, &cancel).unwrap(),
        false,
        Some(&second.symbols),
    );
    assert!(third.changed.as_ref().unwrap().is_empty());
    assert!(Arc::ptr_eq(second.symbols.roots(), third.symbols.roots()));
}

#[test]
fn service_reads_text_source_and_rejects_huge_or_stale() {
    let text: Arc<dyn TextSource> = Arc::new(rove_buffer::RopeText::from_text("class A {}\n"));
    let service = SymbolService;
    let done = futures_lite::future::block_on(service.analyze(
        text.clone(),
        "csharp".into(),
        SizeClass::Normal,
        None,
        Cancellation::default(),
    ))
    .unwrap();
    assert_eq!(done.symbols.version(), text.version());
    assert_eq!(done.symbols.len(), 1);
    let huge = futures_lite::future::block_on(service.analyze(
        text.clone(),
        "csharp".into(),
        SizeClass::Huge,
        None,
        Cancellation::default(),
    ));
    assert_eq!(huge.unwrap_err(), Error::Unsupported);
    let newer = Arc::new(DocumentSymbols::new(
        Version(u64::MAX),
        "csharp".into(),
        Arc::from([]),
    ));
    let stale = futures_lite::future::block_on(service.analyze(
        text,
        "csharp".into(),
        SizeClass::Normal,
        Some(newer),
        Cancellation::default(),
    ));
    assert_eq!(stale.unwrap_err(), Error::Stale);
}

#[test]
fn data_files_are_capped() {
    let mut text = String::from("{");
    for i in 0..(DATA_SYMBOL_LIMIT + 10) {
        if i > 0 {
            text.push(',');
        }
        text.push_str(&format!("\"k{i}\":{i}"));
    }
    text.push('}');
    let cancel = Cancellation::default();
    let (list, truncated) = extract_with("json", &text, &cancel).unwrap();
    assert!(truncated);
    assert_eq!(list.len(), DATA_SYMBOL_LIMIT);
}
