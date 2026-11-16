//! #212 入口集成测试：用户定义的运算符与索引器在使用处和声明处都有出现位置。
//!
//! 在临时目录里搭一个最小的两程序集 C# 项目（不需要 Unity），用 Release 版索引进程做一次全量索引，
//! 然后按「光标在运算符记号 / `[` 上」的口径（与 `tests/nav-cases` 一致）查定义与引用。
//! 同时断言内建运算符（`1 + 2`）与数组下标（`arr[0]`）**不**产生出现位置，避免列表无谓膨胀。
//!
//! 需要先 `dotnet build tools/rove-indexer-cs -c Release`；没有构建产物时打印「已跳过」并通过。
//! 运行：`cargo test -p rove-lang-cs --test operators -- --nocapture`。

use rove_base::Utf8Column;
use rove_lang_cs::index::{CsIndex, Location};
use std::path::{Path, PathBuf};
use std::process::Command;

const V3_CS: &str = r#"namespace Game
{
    public struct V3
    {
        public float X;
        public V3(float x) { X = x; }
        public static V3 operator +(V3 a, V3 b) => new V3(a.X + b.X);
        public static V3 operator -(V3 a) => new V3(0f - a.X);
        public static bool operator ==(V3 a, V3 b) => a.X == b.X;
        public static bool operator !=(V3 a, V3 b) => a.X != b.X;
        public static explicit operator float(V3 a) => a.X;
        public bool Same(V3 other) => this == other;
    }
}
"#;

const BAG_CS: &str = r#"namespace Game
{
    public class Bag
    {
        private readonly int[] m_Items = new int[8];

        public int this[int i]
        {
            get { return m_Items[i]; }
            set { m_Items[i] = value; }
        }
    }
}
"#;

const USE_CS: &str = r#"namespace Game
{
    public class Use
    {
        public float Run(V3 a, V3 b)
        {
            V3 sum = a + b;
            V3 negated = -sum;
            bool differs = a != b;
            int builtin = 1 + 2;
            Bag bag = new Bag();
            bag[0] = builtin;
            int read = bag[1];
            int[] arr = new int[2];
            arr[0] = read;
            V3 acc = negated;
            acc += sum;
            return differs ? acc.X : arr[0];
        }
    }
}
"#;

/// 被测的索引进程：环境变量 `ROVE_INDEXER_CS`（`.exe` 或 `.dll`）优先，否则是 `dotnet build` 的产物。
fn indexer_command() -> Option<Command> {
    let from_env = std::env::var_os("ROVE_INDEXER_CS").filter(|v| !v.is_empty());
    let path = match from_env {
        Some(path) => PathBuf::from(path),
        None => {
            let dll = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/rove-indexer-cs/bin/Release/net9.0/rove-indexer-cs.dll");
            if !dll.is_file() {
                eprintln!(
                    "已跳过：没有找到 {}（先运行 dotnet build tools/rove-indexer-cs -c Release）",
                    dll.display()
                );
                return None;
            }
            dll
        }
    };
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
    {
        let mut c = Command::new("dotnet");
        c.arg(path);
        Some(c)
    } else {
        Some(Command::new(path))
    }
}

fn net9_corelib() -> PathBuf {
    let runtimes = Command::new("dotnet")
        .arg("--list-runtimes")
        .output()
        .unwrap();
    String::from_utf8(runtimes.stdout)
        .unwrap()
        .lines()
        .find_map(|line| {
            let (version, folder) = line
                .strip_prefix("Microsoft.NETCore.App ")?
                .split_once(" [")?;
            let path = Path::new(folder.strip_suffix(']')?)
                .join(version)
                .join("System.Private.CoreLib.dll");
            (version.starts_with("9.") && path.is_file()).then_some(path)
        })
        .expect(".NET 9 runtime")
}

fn write_project(root: &Path) {
    let corelib = net9_corelib();
    let corelib = corelib.to_string_lossy();
    std::fs::create_dir_all(root.join("Assets")).unwrap();
    std::fs::write(root.join("Assets/V3.cs"), V3_CS).unwrap();
    std::fs::write(root.join("Assets/Bag.cs"), BAG_CS).unwrap();
    std::fs::write(root.join("Assets/Use.cs"), USE_CS).unwrap();
    std::fs::write(
        root.join("Assembly-CSharp.csproj"),
        format!(
            r#"<Project><PropertyGroup><AssemblyName>Assembly-CSharp</AssemblyName></PropertyGroup><ItemGroup><Compile Include="Assets/V3.cs" /><Compile Include="Assets/Bag.cs" /><Compile Include="Assets/Use.cs" /><Reference Include="System.Private.CoreLib"><HintPath>{corelib}</HintPath></Reference></ItemGroup></Project>"#
        ),
    )
    .unwrap();
}

/// 源码里「包含 `line_contains` 的那一行」中 `token` 第一次出现的位置（行、列，0 起算）。
/// 全部是 ASCII，所以字节列号就是 UTF-8 列号。
fn at(source: &str, line_contains: &str, token: &str) -> (u32, Utf8Column) {
    let (i, line) = source
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(line_contains))
        .unwrap_or_else(|| panic!("源码里没有包含 {line_contains:?} 的行"));
    let col = line
        .find(token)
        .unwrap_or_else(|| panic!("{line:?} 里没有 {token:?}"));
    (i as u32, Utf8Column(col as u64))
}

/// 定义的（文件, 行）集合，按 nav-cases 的判定口径（文件 + 行）。
fn defs(index: &CsIndex, file: &str, at: (u32, Utf8Column)) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = index
        .goto_definition(file, at.0, at.1)
        .iter()
        .map(|l: &Location| (l.path.to_owned(), l.line))
        .collect();
    v.sort();
    v
}

/// 不含定义处的引用（文件, 行），与 nav-cases 的 `refs` 口径一致。
fn refs(index: &CsIndex, file: &str, at: (u32, Utf8Column)) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = index
        .find_references(file, at.0, at.1)
        .iter()
        .filter(|l| !l.is_def)
        .map(|l: &Location| (l.path.to_owned(), l.line))
        .collect();
    v.sort();
    v
}

#[test]
fn user_defined_operators_and_indexers_have_occurrences() {
    let Some(mut command) = indexer_command() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Game");
    write_project(&root);
    let idx = dir.path().join("game.idx");
    let output = command.arg(&root).arg(&idx).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let index = CsIndex::open(&idx).unwrap();

    let (v3, bag, use_) = ("Assets/V3.cs", "Assets/Bag.cs", "Assets/Use.cs");
    let plus_decl = at(V3_CS, "operator +(V3 a, V3 b)", "+(");
    let minus_decl = at(V3_CS, "operator -(V3 a)", "-(");
    let not_equal_decl = at(V3_CS, "operator !=", "!=");
    let indexer_decl = at(BAG_CS, "public int this[int i]", "this");
    let conversion_decl = at(V3_CS, "explicit operator float", "operator");

    // 1. 二元运算符：光标在 `+` 上能跳到 operator + 的声明，引用包含使用处与复合赋值。
    let plus_use = at(USE_CS, "V3 sum = a + b;", "+");
    assert_eq!(
        defs(&index, use_, plus_use),
        vec![(v3.to_owned(), plus_decl.0)]
    );
    assert_eq!(
        refs(&index, use_, plus_use),
        vec![
            (use_.to_owned(), plus_use.0),
            (use_.to_owned(), at(USE_CS, "acc += sum;", "+=").0),
        ]
    );

    // 2. 一元运算符与 `!=`。
    let minus_use = at(USE_CS, "V3 negated = -sum;", "-");
    assert_eq!(
        defs(&index, use_, minus_use),
        vec![(v3.to_owned(), minus_decl.0)]
    );
    let not_equal_use = at(USE_CS, "bool differs = a != b;", "!=");
    assert_eq!(
        defs(&index, use_, not_equal_use),
        vec![(v3.to_owned(), not_equal_decl.0)]
    );

    // 3. 复合赋值 `acc += sum;` 自己也跳到 operator +。
    let compound = at(USE_CS, "acc += sum;", "+=");
    assert_eq!(
        defs(&index, use_, compound),
        vec![(v3.to_owned(), plus_decl.0)]
    );

    // 4. 索引器：光标在 `[` 上，读写两处都跳到 `this[int i]`。
    let write = at(USE_CS, "bag[0] = builtin;", "[");
    let read = at(USE_CS, "int read = bag[1];", "[");
    assert_eq!(
        defs(&index, use_, write),
        vec![(bag.to_owned(), indexer_decl.0)]
    );
    assert_eq!(
        defs(&index, use_, read),
        vec![(bag.to_owned(), indexer_decl.0)]
    );
    assert_eq!(
        refs(&index, use_, read),
        vec![(use_.to_owned(), write.0), (use_.to_owned(), read.0)]
    );

    // 5. 声明处也是出现位置：从 `operator +` 记号本身出发能查到同一组引用。
    assert_eq!(
        defs(&index, v3, plus_decl),
        vec![(v3.to_owned(), plus_decl.0)]
    );
    assert_eq!(refs(&index, v3, plus_decl).len(), 2);
    // 转换运算符没有使用处记号，但声明本身要能被认出来（`operator` 关键字处）。
    assert_eq!(
        defs(&index, v3, conversion_decl),
        vec![(v3.to_owned(), conversion_decl.0)]
    );

    // 6. 内建运算符与数组下标不记：出现位置列表里这两个位置是空的。
    let builtin = at(USE_CS, "int builtin = 1 + 2;", "+");
    assert_eq!(index.symbol_at(use_, builtin.0, builtin.1), None);
    let array = at(USE_CS, "arr[0] = read;", "[");
    assert_eq!(index.symbol_at(use_, array.0, array.1), None);
    // `m_Items[i]` 是 int[] 的下标，Bag 自己的索引器引用里不能混进来。
    let inner = at(BAG_CS, "return m_Items[i];", "[");
    assert_eq!(index.symbol_at(bag, inner.0, inner.1), None);
}
