//! 映射表逐行测试（#59 验收标准第 2 条）：左列是 C# 符号种类 + 修饰符，
//! 右列是 VS Code 标准语义 token 类型 + 修饰符名字。

use super::syntax::FileFacts;
use super::*;

fn facts(kind: CsSymbolKind, modifiers: Modifiers) -> (&'static str, Vec<&'static str>) {
    SymbolFacts { kind, modifiers }.classify()
}

#[test]
fn each_row_of_the_kind_table_maps_to_its_token_type() {
    // 表里每一行都在这里出现一次：改表必须同步改这张清单。
    const EXPECTED: &[(CsSymbolKind, &str)] = &[
        (CsSymbolKind::Namespace, "namespace"),
        (CsSymbolKind::Class, "class"),
        (CsSymbolKind::Record, "class"),
        (CsSymbolKind::Struct, "struct"),
        (CsSymbolKind::RecordStruct, "struct"),
        (CsSymbolKind::Interface, "interface"),
        (CsSymbolKind::Enum, "enum"),
        (CsSymbolKind::Delegate, "class"),
        (CsSymbolKind::Type, "type"),
        (CsSymbolKind::TypeParameter, "typeParameter"),
        (CsSymbolKind::Field, "field"),
        (CsSymbolKind::Constant, "variable"),
        (CsSymbolKind::EnumMember, "enumMember"),
        (CsSymbolKind::Property, "property"),
        (CsSymbolKind::Event, "event"),
        (CsSymbolKind::Method, "method"),
        (CsSymbolKind::LocalFunction, "function"),
        (CsSymbolKind::Local, "variable"),
        (CsSymbolKind::Parameter, "parameter"),
        (CsSymbolKind::RangeVariable, "variable"),
    ];
    assert_eq!(KIND_TABLE.len(), EXPECTED.len(), "表的行数变了");
    for (kind, token_type) in EXPECTED {
        assert_eq!(kind.token_type(), *token_type, "{kind:?}");
        assert_eq!(facts(*kind, Modifiers::default()).0, *token_type);
    }
    // 类型名一律取自标准集合，不发明新名字（`field` 是 VS Code C# 扩展登记的标准名）。
    const STANDARD: &[&str] = &[
        "namespace",
        "type",
        "class",
        "struct",
        "interface",
        "enum",
        "enumMember",
        "typeParameter",
        "parameter",
        "variable",
        "property",
        "field",
        "event",
        "function",
        "method",
        "macro",
        "keyword",
        "comment",
        "string",
        "number",
        "operator",
        "decorator",
    ];
    for (_, token_type) in KIND_TABLE {
        assert!(STANDARD.contains(token_type), "{token_type} 不在标准集合里");
    }
}

#[test]
fn modifiers_use_the_standard_names_in_a_fixed_order() {
    let m = Modifiers {
        declaration: true,
        static_: true,
        abstract_: true,
        deprecated: true,
        async_: true,
        readonly: true,
        default_library: true,
    };
    assert_eq!(
        m.names(),
        vec![
            "declaration",
            "static",
            "abstract",
            "deprecated",
            "async",
            "readonly",
            "defaultLibrary"
        ]
    );
    // 全部来自 LSP / VS Code 的标准修饰符集合。
    const STANDARD: &[&str] = &[
        "declaration",
        "definition",
        "readonly",
        "static",
        "deprecated",
        "abstract",
        "async",
        "modification",
        "documentation",
        "defaultLibrary",
    ];
    for name in m.names() {
        assert!(STANDARD.contains(&name), "{name} 不在标准集合里");
    }
    assert!(Modifiers::default().names().is_empty());
}

#[test]
fn const_fields_are_static_readonly_variables() {
    // `const` 字段：类型 variable + readonly + static，正好命中回退表的
    // `variable.readonly → variable.other.constant`。
    let (ty, mods) = facts(CsSymbolKind::Constant, Modifiers::default());
    assert_eq!(ty, "variable");
    assert_eq!(mods, vec!["static", "readonly"]);
    // 声明处再多一个 declaration。
    let (_, mods) = facts(
        CsSymbolKind::Constant,
        Modifiers {
            declaration: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(mods, vec!["declaration", "static", "readonly"]);
}

#[test]
fn readonly_static_async_and_abstract_members_carry_their_modifiers() {
    for (kind, modifiers, expected_type, expected_mods) in [
        (
            CsSymbolKind::Field,
            Modifiers {
                readonly: true,
                ..Modifiers::default()
            },
            "field",
            vec!["readonly"],
        ),
        (
            CsSymbolKind::Field,
            Modifiers {
                static_: true,
                ..Modifiers::default()
            },
            "field",
            vec!["static"],
        ),
        (
            CsSymbolKind::Method,
            Modifiers {
                static_: true,
                ..Modifiers::default()
            },
            "method",
            vec!["static"],
        ),
        (
            CsSymbolKind::Method,
            Modifiers {
                async_: true,
                ..Modifiers::default()
            },
            "method",
            vec!["async"],
        ),
        (
            CsSymbolKind::Method,
            Modifiers {
                abstract_: true,
                ..Modifiers::default()
            },
            "method",
            vec!["abstract"],
        ),
        (
            CsSymbolKind::Property,
            Modifiers {
                deprecated: true,
                ..Modifiers::default()
            },
            "property",
            vec!["deprecated"],
        ),
        (
            CsSymbolKind::Class,
            Modifiers {
                default_library: true,
                ..Modifiers::default()
            },
            "class",
            vec!["defaultLibrary"],
        ),
    ] {
        assert_eq!(
            facts(kind, modifiers),
            (expected_type, expected_mods),
            "{kind:?} {modifiers:?}"
        );
    }
}

#[test]
fn roslyn_symbol_kinds_map_to_the_coarse_rows() {
    for (roslyn, kind) in [
        (5u8, CsSymbolKind::Event),
        (6, CsSymbolKind::Field),
        (8, CsSymbolKind::Local),
        (9, CsSymbolKind::Method),
        (11, CsSymbolKind::Type),
        (12, CsSymbolKind::Namespace),
        (13, CsSymbolKind::Parameter),
        (15, CsSymbolKind::Property),
        (16, CsSymbolKind::RangeVariable),
        (17, CsSymbolKind::TypeParameter),
    ] {
        assert_eq!(CsSymbolKind::from_roslyn(roslyn), Some(kind), "{roslyn}");
    }
    // 别名、程序集、标签之类不产出语义 token。
    for roslyn in [0u8, 1, 2, 3, 4, 7, 10, 14, 18, 19, 20] {
        assert_eq!(CsSymbolKind::from_roslyn(roslyn), None, "{roslyn}");
    }
}

// ------------------------------------------------------------ 语法补的那半边

const SAMPLE: &str = r#"
using System;
namespace Game
{
    public interface IDamageable { void Hit(int amount); }
    public enum Team { Red, Blue }
    public struct Point { public int X; }
    public record Save(int Slot);
    public record struct Pair(int A, int B);
    public delegate void Handler(string message);

    public abstract class Player<T> : IDamageable
    {
        public const int MaxHealth = 100;
        private readonly string _name;
        private static int _count;
        [SerializeField] private int _health;
        public event Handler Died;
        public int Health { get; private set; }
        public Player(string name) { _name = name; }
        public abstract void Hit(int amount);
        public static async void Load(int slot) { }
        [Obsolete] public void Old() { }
        public void Run()
        {
            int local = 1;
            void Helper(int step) { }
            Helper(local);
        }
    }
}
"#;

fn at(facts: &FileFacts, needle: &str) -> super::syntax::DeclFacts {
    // 找到 `needle` 在样例里的位置（第一次出现），换算成行列。
    let offset = SAMPLE.find(needle).expect("样例里有这个名字");
    let line = SAMPLE[..offset].matches('\n').count() as u32;
    let col = offset - SAMPLE[..offset].rfind('\n').map_or(0, |i| i + 1);
    facts
        .at(line, col as u32)
        .unwrap_or_else(|| panic!("{needle} 应有声明细节"))
}

#[test]
fn syntax_supplies_type_kinds_and_modifiers_the_index_does_not_have() {
    let facts = FileFacts::parse(SAMPLE);
    let cases: &[(&str, CsSymbolKind, &[&str])] = &[
        ("IDamageable", CsSymbolKind::Interface, &["declaration"]),
        ("Team", CsSymbolKind::Enum, &["declaration"]),
        ("Point", CsSymbolKind::Struct, &["declaration"]),
        ("Save", CsSymbolKind::Record, &["declaration"]),
        ("Pair", CsSymbolKind::RecordStruct, &["declaration"]),
        ("Handler(string", CsSymbolKind::Delegate, &["declaration"]),
        (
            "Player<T>",
            CsSymbolKind::Class,
            &["declaration", "abstract"],
        ),
        ("T>", CsSymbolKind::TypeParameter, &["declaration"]),
        (
            "MaxHealth",
            CsSymbolKind::Constant,
            &["declaration", "static", "readonly"],
        ),
        ("_name", CsSymbolKind::Field, &["declaration", "readonly"]),
        ("_count", CsSymbolKind::Field, &["declaration", "static"]),
        ("_health", CsSymbolKind::Field, &["declaration"]),
        ("Died", CsSymbolKind::Event, &["declaration"]),
        ("Health { get", CsSymbolKind::Property, &["declaration"]),
        ("Hit(int amount); }", CsSymbolKind::Method, &["declaration"]),
        (
            "Hit(int amount);\n",
            CsSymbolKind::Method,
            &["declaration", "abstract"],
        ),
        (
            "Load",
            CsSymbolKind::Method,
            &["declaration", "static", "async"],
        ),
        ("Old", CsSymbolKind::Method, &["declaration", "deprecated"]),
        ("local", CsSymbolKind::Local, &["declaration"]),
        (
            "Helper(int step)",
            CsSymbolKind::LocalFunction,
            &["declaration"],
        ),
        ("step", CsSymbolKind::Parameter, &["declaration"]),
    ];
    for (needle, kind, modifiers) in cases {
        let f = at(&facts, needle);
        assert_eq!(f.kind, *kind, "{needle}");
        let classified = SymbolFacts {
            kind: f.kind,
            modifiers: f.modifiers,
        }
        .classify();
        assert_eq!(classified.1, *modifiers, "{needle}");
    }
    // `Player` 的构造函数按所在类型的种类着色（与 VS Code 一致）。
    assert_eq!(at(&facts, "Player(string name)").kind, CsSymbolKind::Class);
}

#[test]
fn an_unparsable_file_yields_no_facts_instead_of_panicking() {
    let facts = FileFacts::parse("}}} not c# at all {{{");
    // 可能解析出 ERROR 节点，但不应 panic；没有声明就是空表。
    let _ = facts.len();
}
