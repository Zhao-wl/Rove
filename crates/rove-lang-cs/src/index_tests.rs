use super::*;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rove-cs-index-{}-{time}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        Self { dir }
    }

    fn open_bytes(&self, data: &[u8]) -> io::Result<CsIndex> {
        let path = self.dir.join("test.idx");
        fs::write(&path, data)?;
        CsIndex::open(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Fixture::new creates this unique directory before storing it.
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn set_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn seal_header(bytes: &mut [u8]) {
    let checksum = header_checksum(&bytes[..HEADER_LEN]);
    set_u64(bytes, 88, checksum);
}

fn fixture_bytes(strings: &[u8]) -> Vec<u8> {
    let paths = b"Assets/A.csAssets/A.cs.extraAssets/B.cs";
    let files = [
        FileRecord {
            path_start: 0,
            occ_start: 0,
            occ_count: 0,
            inactive_start: 0,
            hash: 0,
        },
        FileRecord {
            path_start: 11,
            occ_start: 0,
            occ_count: 0,
            inactive_start: 0,
            hash: 0,
        },
        FileRecord {
            path_start: 28,
            occ_start: 0,
            occ_count: 0,
            inactive_start: 0,
            hash: 0,
        },
    ];
    let string_offsets = [0_u32, 1, strings.len() as u32];
    let mut sections: [&[u8]; SECTION_COUNT] = [&[]; SECTION_COUNT];
    sections[Section::StringOffsets as usize] = bytemuck::cast_slice(&string_offsets);
    sections[Section::Files as usize] = bytemuck::cast_slice(&files);
    sections[Section::Paths as usize] = paths;
    sections[Section::Strings as usize] = strings;
    encode_fixture(sections)
}

fn encode_fixture(sections: [&[u8]; SECTION_COUNT]) -> Vec<u8> {
    let mut bytes = vec![0; HEADER_LEN];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..12].copy_from_slice(&VERSION.to_le_bytes());
    bytes[12..16].copy_from_slice(&(SECTION_COUNT as u32).to_le_bytes());
    for (i, section) in sections.into_iter().enumerate() {
        let offset = bytes.len().next_multiple_of(8);
        bytes.resize(offset, 0);
        set_u64(&mut bytes, TABLE_OFFSET + i * 16, offset as u64);
        set_u64(
            &mut bytes,
            TABLE_OFFSET + i * 16 + 8,
            (section.len() / SECTION_SIZES[i]) as u64,
        );
        bytes.extend_from_slice(section);
    }
    bytes.resize(bytes.len().next_multiple_of(8), 0);
    let len = bytes.len() as u64;
    set_u64(&mut bytes, 16, len);
    seal_header(&mut bytes);
    bytes
}

#[test]
fn sorted_paths_and_empty_metadata() {
    let fixture = Fixture::new();
    let index = fixture.open_bytes(&fixture_bytes(b"xabc")).unwrap();
    assert_eq!(index.file_id("Assets/A.cs"), Some(0));
    assert_eq!(index.file_id("Assets/A.cs.extra"), Some(1));
    assert_eq!(index.file_path(2), Some("Assets/B.cs"));
    assert_eq!(index.file_id("Assets/Missing.cs"), None);
    assert_eq!(index.file_id("Assets/A"), None);
    assert_eq!(index.file_id("Assets/A.cs.extr"), None);
    assert!(index.signatures().is_empty());
    assert!(index.xml_docs().is_empty());
    assert!(index.metadata_views().is_empty());
    assert_eq!(index.string_payload_reads(), 0);
}

#[test]
fn string_payload_is_checked_on_access() {
    let fixture = Fixture::new();
    let index = fixture.open_bytes(&fixture_bytes(b"x\xffabc")).unwrap();
    assert_eq!(index.string_payload_reads(), 0);
    assert_eq!(index.str(0).unwrap(), "x");
    assert!(index.str(1).is_err());
    assert!(index.str(u32::MAX).is_err());
    assert_eq!(index.string_payload_reads(), 2);
}

#[test]
fn randomized_header_and_section_table_bit_flips_are_rejected() {
    let fixture = Fixture::new();
    let original = fixture_bytes(b"xabc");
    let mut locations: Vec<_> = (0..HEADER_LEN * 8).collect();
    let mut rng = 0x956d_931c_a172_c16b_u64;
    for i in (1..locations.len()).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        locations.swap(i, (rng % (i + 1) as u64) as usize);
    }
    for bit in locations {
        let mut corrupted = original.clone();
        corrupted[bit / 8] ^= 1 << (bit % 8);
        assert!(
            fixture.open_bytes(&corrupted).is_err(),
            "accepted bit flip at {bit}"
        );
    }
}

#[test]
fn checks_structure_even_if_header_checksum_matches() {
    let fixture = Fixture::new();
    let original = fixture_bytes(b"xabc");
    for (at, value) in [
        (TABLE_OFFSET, 0),
        (TABLE_OFFSET, HEADER_LEN as u64 + 1),
        (TABLE_OFFSET + 16, HEADER_LEN as u64),
        (TABLE_OFFSET + 8, u64::MAX),
        (TABLE_OFFSET + 16, u64::MAX - 7),
    ] {
        let mut corrupted = original.clone();
        set_u64(&mut corrupted, at, value);
        seal_header(&mut corrupted);
        assert!(
            fixture.open_bytes(&corrupted).is_err(),
            "accepted {value} at {at}"
        );
    }
    let mut corrupt_offsets = original.clone();
    corrupt_offsets[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(&5_u32.to_le_bytes());
    assert!(fixture.open_bytes(&corrupt_offsets).is_err());
    let mut higher_version = original.clone();
    higher_version[8..12].copy_from_slice(&(VERSION + 1).to_le_bytes());
    let error = fixture.open_bytes(&higher_version).err().unwrap();
    assert!(error.to_string().contains("unsupported C# index version 3"));
}

fn indexer_dll() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/rove-indexer-cs/bin/Release/net9.0/rove-indexer-cs.dll")
}

/// Runs against the real .NET writer after the release build.
#[test]
#[ignore = "requires the Release .NET indexer"]
fn roslyn_writer_uses_utf8_coordinates_and_binary_local_keys() {
    const SOURCE: &str = "class Unicode {\n\
        static int Read(int 参数) { /* 中文😀𐐀 */ int 值 = 参数; return 值; }\n\
        #if ROVE_TEST_ACTIVE\n\
        static int Active() => 1;\n\
        #elif ROVE_TEST_OTHER\n\
        static int Inactive1() => 2;\n\
        #else\n\
        static int Inactive2() => 3;\n\
        #endif\n\
        }\n";
    let source_text = SOURCE.replace("中文😀𐐀", &format!("中文😀𐐀{}", "宽".repeat(90)));
    let fixture = Fixture::new();
    fs::create_dir(fixture.dir.join("Assets")).unwrap();
    let mut source = File::create(fixture.dir.join("Assets/Unicode.cs")).unwrap();
    source.write_all(b"\xef\xbb\xbf").unwrap();
    source.write_all(source_text.as_bytes()).unwrap();
    drop(source);
    fs::write(
        fixture.dir.join("Fixture.csproj"),
        r#"<Project><PropertyGroup><AssemblyName>Assembly-CSharp</AssemblyName><DefineConstants>ROVE_TEST_ACTIVE</DefineConstants></PropertyGroup><ItemGroup><Compile Include="Assets/Unicode.cs"/></ItemGroup></Project>"#,
    ).unwrap();

    let index_path = fixture.dir.join("test.idx");
    let dll = indexer_dll();
    let output = std::process::Command::new("dotnet")
        .arg(dll)
        .arg(&fixture.dir)
        .arg(&index_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let index = CsIndex::open(&index_path).unwrap();
    assert_eq!(index.header().indexer_version, [1, 2, 0, 0]);
    let rel_path = "Assets/Unicode.cs";
    let file = index.file_id(rel_path).unwrap();
    let line = source_text.lines().nth(1).unwrap();
    let def = line.find("值").unwrap();
    let reference = line.rfind("值").unwrap();
    let original_col = Utf16Column(line[..def].encode_utf16().count() as u64);
    assert!(def > u8::MAX as usize);
    assert_ne!(def as u64, original_col.0);
    let symbol = index
        .symbol_at(rel_path, 1, Utf8Column(reference as u64))
        .unwrap();
    let result = index.goto_definition(rel_path, 1, Utf8Column(reference as u64));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].col, Utf8Column(def as u64));
    for location in index.find_references(rel_path, 1, Utf8Column(reference as u64)) {
        let start = location.col.0 as usize;
        assert_eq!(&line[start..start + location.len as usize], "值");
    }
    let expected_offset = source_text.find("值").unwrap() as u64;
    assert_eq!(
        index.symbol_key(symbol).unwrap(),
        SymbolKey::Local {
            file,
            offset: ByteOffset(expected_offset)
        }
    );
    let absolute_path = fixture.dir.join("Assets/Unicode.cs");
    let path = absolute_path.to_string_lossy();
    let strings = index.bytes(Section::Strings);
    assert!(!strings
        .windows(path.len())
        .any(|part| part == path.as_bytes()));
    assert!(!strings
        .windows(rel_path.len())
        .any(|part| part == rel_path.as_bytes()));
    assert_eq!(
        index.inactive_lines(file),
        &[
            InactiveLines { start: 5, end: 6 },
            InactiveLines { start: 7, end: 8 }
        ]
    );
}

#[test]
fn rejects_every_truncated_fixture_prefix() {
    let fixture = Fixture::new();
    let original = fixture_bytes(b"xabc");
    for len in 0..original.len() {
        assert!(
            fixture.open_bytes(&original[..len]).is_err(),
            "accepted truncated index at {len} bytes"
        );
    }
}

#[test]
fn decodes_reference_ids_on_both_sides_of_the_width_boundary() {
    let short_width = reference_width(1 << 24);
    assert_eq!(short_width, 3);
    assert_eq!(
        read_reference_id(&[0xff, 0xff, 0xff], short_width),
        (1 << 24) - 1
    );

    let long_width = reference_width((1 << 24) + 1);
    assert_eq!(long_width, 4);
    assert_eq!(read_reference_id(&[0, 0, 0, 1], long_width), 1 << 24);
    assert_eq!(
        read_reference_id(&[0x78, 0x56, 0x34, 0x12], long_width),
        0x1234_5678
    );
}

#[test]
fn wide_occurrence_lookup_and_saturated_definition_count() {
    const DEFINITIONS: u32 = u16::MAX as u32 + 1;
    let fixture = Fixture::new();
    let files = [FileRecord {
        path_start: 0,
        occ_start: 0,
        occ_count: DEFINITIONS + 1,
        inactive_start: 0,
        hash: 0,
    }];
    let symbols = [SymbolRecord {
        key: 0,
        name: 0,
        display: 0,
        container: NONE,
        ref_start: 0,
        kind: 0,
        flags: 1,
        definition_count: u16::MAX,
    }];
    let mut occurrences: Vec<u64> = (0..DEFINITIONS)
        .map(|i| {
            (u64::from(i / 128) << 16) | (u64::from(i % 128) << 8) | (1 << 3) | u64::from(Role::DEF)
        })
        .collect();
    occurrences.push(0); // Wide occurrence #0.
    let wide = [WideOccurrence {
        file: 0,
        line: 4096,
        col: 300,
        symbol: 0,
        len: 32,
        role: u32::from(Role::REF),
    }];
    let references: Vec<_> = (0..=DEFINITIONS)
        .flat_map(|i| [i as u8, (i >> 8) as u8, (i >> 16) as u8])
        .collect();

    let mut sections: [&[u8]; SECTION_COUNT] = [&[]; SECTION_COUNT];
    sections[Section::StringOffsets as usize] = bytemuck::cast_slice(&[0_u32, 0]);
    sections[Section::Files as usize] = bytemuck::cast_slice(&files);
    sections[Section::Paths as usize] = b"Assets/A.cs";
    sections[Section::Symbols as usize] = bytemuck::cast_slice(&symbols);
    sections[Section::Occurrences as usize] = bytemuck::cast_slice(&occurrences);
    sections[Section::WideOccurrences as usize] = bytemuck::cast_slice(&wide);
    sections[Section::References as usize] = &references;
    let index = fixture.open_bytes(&encode_fixture(sections)).unwrap();
    let file = "Assets/A.cs";

    assert_eq!(index.symbol_at(file, 4096, Utf8Column(331)), Some(0));
    assert_eq!(index.symbol_at(file, 4096, Utf8Column(332)), None);
    assert_eq!(index.symbol_at(file, 4096, Utf8Column(1 << 32)), None);
    assert_eq!(index.reference_count(0), DEFINITIONS as usize + 1);
    let definitions = index.goto_definition(file, 4096, Utf8Column(300));
    assert_eq!(definitions.len(), DEFINITIONS as usize);
    assert!(definitions.iter().all(|location| location.is_def));
    assert_eq!(definitions.last().unwrap().col, Utf8Column(127));
    let references = index.find_references(file, 4096, Utf8Column(300));
    assert_eq!(references.len(), DEFINITIONS as usize + 1);
    assert_eq!(
        references.last(),
        Some(&Location {
            path: file,
            line: 4096,
            col: Utf8Column(300),
            len: 32,
            is_def: false
        })
    );
}

#[test]
#[ignore = "requires the Release .NET indexer and .NET SDK"]
fn generated_parameters_have_distinct_local_keys() {
    const GENERATED: &str = r#"
public static class GeneratedApi {
    public static int Left(/* 中文😀 */ int value) { return value; }
    public static int Right(int value) { return value; }
}
"#;
    const CALLER_A: &str =
        "class A { int Call() => GeneratedApi.Left(value: 1) + GeneratedApi.Right(value: 2); }";
    const CALLER_B: &str = "class B { int Call() => GeneratedApi.Left(value: 3); }";
    const CALLER_C: &str = "class C { int Call() => GeneratedApi.Left(value: 4); }";
    let fixture = Fixture::new();
    let dll = indexer_dll();
    let roslyn = dll.with_file_name("Microsoft.CodeAnalysis.dll");
    let generator = fixture.dir.join("generator");
    fs::create_dir(&generator).unwrap();
    let escape = |path: &Path| {
        path.to_string_lossy()
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    fs::write(generator.join("FixtureGenerator.csproj"), format!(r#"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup><TargetFramework>net9.0</TargetFramework></PropertyGroup>
  <ItemGroup><Reference Include="Microsoft.CodeAnalysis"><HintPath>{}</HintPath></Reference></ItemGroup>
</Project>
"#, escape(&roslyn))).unwrap();
    fs::write(
        generator.join("FixtureGenerator.cs"),
        format!(
            r####"
using Microsoft.CodeAnalysis;

[Generator]
public sealed class FixtureGenerator : ISourceGenerator
{{
    public void Initialize(GeneratorInitializationContext context) {{ }}
    public void Execute(GeneratorExecutionContext context) =>
        context.AddSource("Shared.g.cs", """
{GENERATED}
""");
}}
"####
        ),
    )
    .unwrap();
    let build = std::process::Command::new("dotnet")
        .args([
            "build",
            "-c",
            "Release",
            "--disable-build-servers",
            "--nologo",
        ])
        .current_dir(&generator)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let analyzer = generator.join("bin/Release/net9.0/FixtureGenerator.dll");

    let runtimes = std::process::Command::new("dotnet")
        .arg("--list-runtimes")
        .output()
        .unwrap();
    assert!(runtimes.status.success());
    let runtimes = String::from_utf8(runtimes.stdout).unwrap();
    let corelib = runtimes
        .lines()
        .find_map(|line| {
            let (version, folder) = line
                .strip_prefix("Microsoft.NETCore.App ")?
                .split_once(" [")?;
            if !version.starts_with("9.") {
                return None;
            }
            let path = Path::new(folder.strip_suffix(']')?)
                .join(version)
                .join("System.Private.CoreLib.dll");
            path.is_file().then_some(path)
        })
        .expect(".NET 9 runtime required by the indexer");
    for (project, source) in [("A", CALLER_A), ("B", CALLER_B)] {
        fs::write(fixture.dir.join(format!("{project}.cs")), source).unwrap();
        fs::write(
            fixture.dir.join(format!("{project}.csproj")),
            format!(
                r#"
<Project>
  <PropertyGroup><AssemblyName>{project}</AssemblyName></PropertyGroup>
  <ItemGroup>
    <Compile Include="{project}.cs" />
    <Reference Include="System.Private.CoreLib"><HintPath>{}</HintPath></Reference>
    <Analyzer Include="{}" />
  </ItemGroup>
</Project>
"#,
                escape(&corelib),
                escape(&analyzer)
            ),
        )
        .unwrap();
    }

    fs::write(fixture.dir.join("C.cs"), CALLER_C).unwrap();
    fs::write(
        fixture.dir.join("C.csproj"),
        format!(
            r#"
<Project>
  <ItemGroup>
    <Compile Include="C.cs" />
    <Reference Include="System.Private.CoreLib"><HintPath>{}</HintPath></Reference>
    <ProjectReference Include="A.csproj" />
  </ItemGroup>
</Project>
"#,
            escape(&corelib)
        ),
    )
    .unwrap();

    let path = fixture.dir.join("test.idx");
    let output = std::process::Command::new("dotnet")
        .arg(dll)
        .arg(&fixture.dir)
        .arg(&path)
        .arg("--generators")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let index = CsIndex::open(path).unwrap();
    assert_eq!(
        index.files().len(),
        5,
        "three handwritten and two generated files"
    );

    let left_offset = GENERATED.find("value").unwrap() as u64;
    let right_offset = (GENERATED.find("Right(int ").unwrap() + "Right(int ".len()) as u64;
    let cases = [
        (
            "A.cs",
            CALLER_A.find("value").unwrap(),
            "A.csproj",
            left_offset,
        ),
        (
            "A.cs",
            CALLER_A.rfind("value").unwrap(),
            "A.csproj",
            right_offset,
        ),
        (
            "B.cs",
            CALLER_B.find("value").unwrap(),
            "B.csproj",
            left_offset,
        ),
    ];
    let mut seen = std::collections::HashSet::new();
    for (caller, column, project, offset) in cases {
        let column = Utf8Column(column as u64);
        let symbol = index
            .symbol_at(caller, 0, column)
            .expect("named argument symbol");
        assert!(
            seen.insert(symbol),
            "different parameters shared a symbol ID"
        );
        let SymbolKey::Local {
            file,
            offset: declaration,
        } = index.symbol_key(symbol).unwrap()
        else {
            panic!("generated parameter did not get a local key")
        };
        assert_eq!(declaration, ByteOffset(offset));
        let generated_path = index.file_path(file).unwrap();
        assert!(generated_path.starts_with(&format!("rove-generated://{project}/")));
        assert!(generated_path.ends_with("Shared.g.cs"));
        let defs = index.goto_definition(caller, 0, column);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].path, generated_path);
        let references = index.find_references(caller, 0, column);
        let cross_project = project == "A.csproj" && offset == left_offset;
        assert_eq!(
            references.len(),
            if cross_project { 4 } else { 3 },
            "{references:?}"
        );
        if cross_project {
            let c = Utf8Column(CALLER_C.find("value").unwrap() as u64);
            assert_eq!(index.symbol_at("C.cs", 0, c), Some(symbol));
            assert!(references.iter().any(|r| r.path == "C.cs" && r.col == c));
        }
        assert_eq!(
            references
                .iter()
                .filter(|r| r.path == generated_path)
                .count(),
            2
        );
        assert!(references
            .iter()
            .any(|r| r.path == caller && r.col == column));
    }
}

fn net9_corelib() -> PathBuf {
    let runtimes = std::process::Command::new("dotnet")
        .arg("--list-runtimes")
        .output()
        .unwrap();
    assert!(runtimes.status.success());
    String::from_utf8(runtimes.stdout)
        .unwrap()
        .lines()
        .find_map(|line| {
            let (version, folder) = line
                .strip_prefix("Microsoft.NETCore.App ")?
                .split_once(" [")?;
            if !version.starts_with("9.") {
                return None;
            }
            let path = Path::new(folder.strip_suffix(']')?)
                .join(version)
                .join("System.Private.CoreLib.dll");
            path.is_file().then_some(path)
        })
        .expect(".NET 9 runtime required by the indexer")
}

/// #33：下游项目只引用上游的元数据镜像。经元数据看到的参数（命名实参、以实例形式调用的扩展方法）
/// 仍要回指上游源码里的声明；上游镜像输出失败（csproj 声明的语言版本低于代码实际使用的）时退回引用编译。
/// 同样的输入两次产出逐字节相同的索引。
#[test]
#[ignore = "requires the Release .NET indexer"]
fn cross_project_parameters_resolve_through_metadata_images() {
    const API: &str = "public static class Api {\n\
        public static int Twice(int value) => value * 2;\n\
        public static int Ext(this string text, int count) => count;\n\
        }\n";
    const OLD: &str = "public struct Old { public int X = 1; public Old() { } public static int Make(int seed) => seed; }\n";
    const USER: &str =
        "class User { int Run() => Api.Twice(value: 1) + \"x\".Ext(count: 2) + Old.Make(seed: 3); }\n";
    let fixture = Fixture::new();
    let corelib = net9_corelib();
    let corelib = corelib.to_string_lossy();
    let project = |name: &str, lang: &str, refs: &[&str]| {
        let refs: String = refs
            .iter()
            .map(|r| format!(r#"<ProjectReference Include="{r}.csproj" />"#))
            .collect();
        format!(
            r#"<Project><PropertyGroup><AssemblyName>{name}</AssemblyName><LangVersion>{lang}</LangVersion></PropertyGroup><ItemGroup><Compile Include="{name}.cs" /><Reference Include="System.Private.CoreLib"><HintPath>{corelib}</HintPath></Reference>{refs}</ItemGroup></Project>"#
        )
    };
    for (name, source, lang, refs) in [
        ("Api", API, "latest", &[][..]),
        ("Old", OLD, "9.0", &[][..]),
        ("User", USER, "latest", &["Api", "Old"][..]),
    ] {
        fs::write(fixture.dir.join(format!("{name}.cs")), source).unwrap();
        fs::write(
            fixture.dir.join(format!("{name}.csproj")),
            project(name, lang, refs),
        )
        .unwrap();
    }

    let run = |name: &str| {
        let path = fixture.dir.join(name);
        let output = std::process::Command::new("dotnet")
            .arg(indexer_dll())
            .arg(&fixture.dir)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        path
    };
    let first = run("first.idx");
    let second = run("second.idx");
    assert_eq!(
        fs::read(&first).unwrap(),
        fs::read(&second).unwrap(),
        "same input must produce the same index"
    );

    let index = CsIndex::open(&first).unwrap();
    for (name, file, line, source) in [
        ("value", "Api.cs", 1, API),
        ("count", "Api.cs", 2, API),
        ("seed", "Old.cs", 0, OLD),
    ] {
        let call = Utf8Column(USER.find(&format!("{name}:")).unwrap() as u64);
        let defs = index.goto_definition("User.cs", 0, call);
        assert_eq!(defs.len(), 1, "{name}: {defs:?}");
        let decl_line = source.lines().nth(line as usize).unwrap();
        assert_eq!(defs[0].path, file, "{name}");
        assert_eq!(defs[0].line, line, "{name}");
        assert_eq!(
            defs[0].col,
            Utf8Column(decl_line.find(&format!(" {name}")).unwrap() as u64 + 1),
            "{name}"
        );
        let references = index.find_references("User.cs", 0, call);
        assert!(
            references
                .iter()
                .any(|r| r.path == "User.cs" && r.col == call),
            "{name}: {references:?}"
        );
    }
    // 跨项目的成员引用与定义是同一个符号
    let twice = Utf8Column(USER.find("Twice").unwrap() as u64);
    let defs = index.goto_definition("User.cs", 0, twice);
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].path, "Api.cs");
}

/// 段 13（#34）：视图记录 + 成员记录，外部符号沿容器链找到最外层类型的视图。
fn metadata_fixture(views: &[u8]) -> Vec<u8> {
    let symbol = |key: u32, container: u32| SymbolRecord {
        key,
        name: key,
        display: key,
        container,
        ref_start: 0,
        kind: 0,
        flags: 0,
        definition_count: 0,
    };
    // 0 外部类型 Outer，1 其成员 Run，2 嵌套类型 Inner，3 Inner 的成员 Size，4 不在任何视图里的外部符号。
    let symbols = [
        symbol(0, NONE),
        symbol(1, 0),
        symbol(2, 0),
        symbol(3, 2),
        symbol(4, NONE),
    ];
    let parts = [
        "Outer",
        "Run",
        "Inner",
        "Size",
        "Other",
        VIEW_TEXT,
        "sig1",
        "<summary>doc</summary>",
    ];
    let strings = parts.concat().into_bytes();
    let offsets: Vec<u32> = std::iter::once(0)
        .chain(parts.iter().scan(0, |end, p| {
            *end += p.len() as u32;
            Some(*end)
        }))
        .collect();
    let signatures = [SymbolText { symbol: 1, text: 6 }];
    let docs = [SymbolText { symbol: 1, text: 7 }];
    let mut sections: [&[u8]; SECTION_COUNT] = [&[]; SECTION_COUNT];
    sections[Section::StringOffsets as usize] = bytemuck::cast_slice(&offsets);
    sections[Section::Symbols as usize] = bytemuck::cast_slice(&symbols);
    sections[Section::Signatures as usize] = bytemuck::cast_slice(&signatures);
    sections[Section::XmlDocs as usize] = bytemuck::cast_slice(&docs);
    sections[Section::MetadataViews as usize] = views;
    sections[Section::Strings as usize] = &strings;
    encode_fixture(sections)
}

/// 第 12 行是嵌套类型 Inner 的声明，上方两行是它的文档注释。
const VIEW_TEXT: &str = "#region\n\n\n\n\n\n\n\n\n\n    /// <summary>Inner</summary>\n    ///\n    public class Inner\n";

fn view_section(views: &[[u32; 4]], members: &[[u32; 4]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(views.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(members.len() as u32).to_le_bytes());
    for record in views.iter().chain(members) {
        bytes.extend_from_slice(bytemuck::cast_slice(record));
    }
    bytes
}

#[test]
fn metadata_views_resolve_external_definitions() {
    let fixture = Fixture::new();
    // 两个视图属于同一类型（同名程序集的两个版本），第二个只有类型与 Run。
    let views = view_section(
        &[[0, 5, 0, 4], [0, 5, 4, 2]],
        &[
            [0, 6, 13, 5],
            [1, 9, 16, 3],
            [2, 12, 17, 5],
            [3, 14, 19, 4],
            [0, 6, 13, 5],
            [1, 8, 16, 3],
        ],
    );
    let index = fixture.open_bytes(&metadata_fixture(&views)).unwrap();
    assert_eq!(index.metadata_view_records().len(), 2);
    assert_eq!(index.metadata_views_of(0), 0..2);
    assert_eq!(index.metadata_views_of(4), 2..2);
    assert_eq!(index.metadata_view_text(0).unwrap(), VIEW_TEXT);
    assert!(index.metadata_view_text(2).is_err());
    assert_eq!(index.metadata_view_members(1).len(), 2);
    assert!(index.metadata_view_members(9).is_empty());

    let run = index.metadata_definitions(1);
    assert_eq!(
        run,
        vec![
            MetadataLocation {
                view: 0,
                line: 9,
                col: Utf8Column(16),
                len: 3
            },
            MetadataLocation {
                view: 1,
                line: 8,
                col: Utf8Column(16),
                len: 3
            }
        ]
    );
    // 嵌套类型的成员：Size → Inner → Outer，只在第一个视图里有。
    assert_eq!(
        index.metadata_definitions(3),
        vec![MetadataLocation {
            view: 0,
            line: 14,
            col: Utf8Column(19),
            len: 4
        }]
    );
    assert_eq!(index.metadata_definitions(2)[0].line, 12);
    assert!(index.metadata_definitions(4).is_empty());
    assert!(index.metadata_definitions(99).is_empty());
    assert_eq!(index.signature(1), Some("sig1"));
    assert_eq!(index.xml_doc(1).as_deref(), Some("<summary>doc</summary>"));
    assert_eq!(index.signature(0), None);
    assert_eq!(index.xml_doc(4), None);
    // 段 12 没有的符号从视图里成员上方的 /// 行取。
    assert_eq!(
        index.xml_doc(2).as_deref(),
        Some("<summary>Inner</summary>\n")
    );
    assert_eq!(index.xml_doc(0), None);
}

#[test]
fn malformed_metadata_view_sections_are_rejected() {
    let fixture = Fixture::new();
    let good = view_section(&[[0, 5, 0, 1]], &[[0, 6, 13, 5]]);
    assert!(fixture.open_bytes(&metadata_fixture(&good)).is_ok());
    let unsorted = view_section(&[[1, 5, 0, 1], [0, 5, 0, 1]], &[[0, 6, 13, 5]]);
    let out_of_range = view_section(&[[0, 5, 0, 2]], &[[0, 6, 13, 5]]);
    let overflow = view_section(&[[0, 5, u32::MAX, 2]], &[[0, 6, 13, 5]]);
    let mut wrong_count = good.clone();
    wrong_count[4..8].copy_from_slice(&2_u32.to_le_bytes());
    for (name, bytes) in [
        ("unsorted", unsorted),
        ("out of range", out_of_range),
        ("overflow", overflow),
        ("wrong count", wrong_count),
        ("short", vec![1, 0, 0]),
    ] {
        assert!(
            fixture.open_bytes(&metadata_fixture(&bytes)).is_err(),
            "accepted {name} metadata view section"
        );
    }
}

/// 本机 .NET SDK 的 Microsoft.NETCore.App 参考程序集目录（带 XML 文档）。
fn netcore_ref_dir() -> PathBuf {
    let output = std::process::Command::new("dotnet")
        .arg("--list-sdks")
        .output()
        .unwrap();
    let list = String::from_utf8_lossy(&output.stdout);
    let sdk = list.lines().last().expect("a .NET SDK is installed");
    let root = Path::new(sdk[sdk.find('[').unwrap() + 1..sdk.rfind(']').unwrap()].trim())
        .parent()
        .unwrap()
        .to_path_buf();
    let mut versions: Vec<_> = fs::read_dir(root.join("packs/Microsoft.NETCore.App.Ref"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("ref/net9.0/System.Runtime.dll").exists())
        .collect();
    versions.sort();
    versions
        .pop()
        .expect("Microsoft.NETCore.App.Ref 9.x")
        .join("ref/net9.0")
}

/// #34：外部符号的元数据视图。两个项目分别引用同一个第三方库的 1.0 与 2.0（2.0 没有 .xml 文档），
/// B 还引用一个被独占锁住的 DLL；BCL 的 List<T> 由两个项目共同引用。
#[test]
#[ignore = "requires the Release .NET indexer and .NET SDK"]
fn roslyn_writer_generates_metadata_views() {
    const LIB: &str = r#"
namespace Rove.Fixture
{
    /// <summary>A widget.</summary>
    public class Widget<T> where T : class, new()
    {
        /// <summary>Makes it.</summary>
        public T Make(ref int count, params string[] names) => new T();
        /// <summary>Nested options.</summary>
        public class Options
        {
            /// <summary>Size.</summary>
            public int Size { get; set; }
        }
        protected internal event System.Action? Changed;
        public static Widget<T> operator +(Widget<T> a, Widget<T> b) => a;
        internal void Hidden() { }
        /// <summary>Renames it.</summary>
        public void Rename(string label) { }
        VERSION_2_MEMBER
    }
    /// <summary>Tools.</summary>
    public static class Tools
    {
        /// <summary>Scales it.</summary>
        public static int Scale(this Widget<object> w, int factor, string label) => factor;
    }
    /// <summary>A callback.</summary>
    public delegate void Callback(int code, string reason);
}
"#;
    const A: &str = "using System.Collections.Generic;\nusing Rove.Fixture;\nclass A { void M() { var l = new List<int>(); l.Add(1); var d = new Dictionary<string, int>(); var w = new Widget<object>(); int c = 0; w.Make(ref c); var o = new Widget<object>.Options(); o.Size = 1; w.Make(count: ref c); w.Scale(factor: 2, label: \"x\"); w.Rename(label: \"y\"); Callback cb = null; cb(code: 1, reason: \"r\"); l.Insert(index: 0, item: 3); } }\n";
    const B: &str = "using Rove.Fixture;\nusing System;\nclass B { void M() { var w = new Widget<object>(); w.V2Only(); var l = new System.Collections.Generic.List<int>(); l.Add(2); } }\n";

    let fixture = Fixture::new();
    let refs = netcore_ref_dir();
    let mut libs = Vec::new();
    for version in ["1.0.0", "2.0.0"] {
        let dir = fixture.dir.join(format!("lib{version}"));
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("Lib.csproj"),
            format!(
                r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net9.0</TargetFramework><AssemblyName>Rove.Fixture.Lib</AssemblyName><Version>{version}</Version><Nullable>enable</Nullable><GenerateDocumentationFile>true</GenerateDocumentationFile><NoWarn>CS0067</NoWarn></PropertyGroup></Project>"#
            ),
        )
        .unwrap();
        let member = if version == "2.0.0" {
            "/// <summary>Only in 2.0.</summary>\n        public void V2Only() { }"
        } else {
            ""
        };
        fs::write(dir.join("Lib.cs"), LIB.replace("VERSION_2_MEMBER", member)).unwrap();
        let out = dir.join("out");
        let build = std::process::Command::new("dotnet")
            .args(["build", "-c", "Release", "--nologo", "-o"])
            .arg(&out)
            .arg(dir.join("Lib.csproj"))
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stdout)
        );
        libs.push(out.join("Rove.Fixture.Lib.dll"));
    }
    // 2.0 缺文档；被锁的 DLL 是 1.0 的拷贝。
    fs::remove_file(libs[1].with_extension("xml")).unwrap();
    let locked = fixture.dir.join("Locked.dll");
    fs::copy(&libs[0], &locked).unwrap();

    fs::create_dir(fixture.dir.join("Assets")).unwrap();
    fs::write(fixture.dir.join("Assets/A.cs"), A).unwrap();
    fs::write(fixture.dir.join("Assets/B.cs"), B).unwrap();
    let reference = |path: &Path| {
        format!(
            r#"<Reference Include="{0}"><HintPath>{1}</HintPath></Reference>"#,
            path.file_stem().unwrap().to_string_lossy(),
            path.display()
        )
    };
    let bcl = ["System.Runtime.dll", "System.Collections.dll"]
        .map(|name| reference(&refs.join(name)))
        .join("");
    for (name, source, lib) in [
        ("A", "Assets/A.cs", reference(&libs[0])),
        (
            "B",
            "Assets/B.cs",
            reference(&libs[1]) + &reference(&locked),
        ),
    ] {
        fs::write(
            fixture.dir.join(format!("{name}.csproj")),
            format!(
                r#"<Project><PropertyGroup><AssemblyName>{name}</AssemblyName><LangVersion>latest</LangVersion></PropertyGroup><ItemGroup><Compile Include="{source}"/>{bcl}{lib}</ItemGroup></Project>"#
            ),
        )
        .unwrap();
    }

    let run = |name: &str| {
        let path = fixture.dir.join(name);
        let report = fixture.dir.join(format!("{name}.json"));
        let output = std::process::Command::new("dotnet")
            .arg(indexer_dll())
            .arg(&fixture.dir)
            .arg(&path)
            .arg("--report")
            .arg(&report)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "{stderr}");
        let report: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        (path, report, stderr)
    };
    let ((first, report, stderr), second) = {
        // 独占打开（不共享读），模拟被其他进程占用的 DLL。
        #[cfg(windows)]
        let _lock = {
            use std::os::windows::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&locked)
                .unwrap()
        };
        (run("first.idx"), run("second.idx").0)
    };
    // 同样的输入产出逐字节相同的索引。
    assert!(fs::read(&first).unwrap() == fs::read(&second).unwrap());
    #[cfg(windows)]
    assert!(stderr.contains("Locked.dll is unreadable"), "{stderr}");
    assert!(
        stderr.contains("no XML documentation for Rove.Fixture.Lib"),
        "{stderr}"
    );
    let metadata = &report["metadata"];
    assert_eq!(metadata["views_with_syntax_errors"], 0, "{metadata}");
    assert_eq!(
        metadata["external_references_without_target"], 0,
        "{metadata}"
    );

    let index = CsIndex::open(&first).unwrap();
    assert_eq!(index.header().indexer_version, [1, 2, 0, 0]);
    let at = |file: &str, source: &str, token: &str| {
        let col = source.lines().nth(2).unwrap().find(token).unwrap();
        index
            .symbol_at(file, 2, Utf8Column(col as u64))
            .unwrap_or_else(|| panic!("no symbol at {token}"))
    };
    let line_of = |location: &MetadataLocation| -> (String, String) {
        let text = index.metadata_view_text(location.view).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let line = lines[location.line as usize];
        let start = location.col.0 as usize;
        (
            line[start..start + location.len as usize].to_owned(),
            lines[location.line as usize - 1].trim().to_owned(),
        )
    };

    // List<T>.Add：两个项目引用同一个 System.Collections，只生成一份视图，带文档注释。
    let add = at("Assets/A.cs", A, "Add(1)");
    assert_eq!(at("Assets/B.cs", B, "Add(2)"), add);
    let defs = index.metadata_definitions(add);
    assert_eq!(defs.len(), 1, "{defs:?}");
    let (name, above) = line_of(&defs[0]);
    assert_eq!(name, "Add");
    assert!(above.starts_with("/// "), "{above}");
    assert!(index.xml_doc(add).unwrap().contains("<summary>"));
    assert_eq!(index.signature(add), Some("public void Add(T item);"));
    let text = index.metadata_view_text(defs[0].view).unwrap();
    let mut header = text.lines();
    assert!(header
        .next()
        .unwrap()
        .starts_with("#region 程序集 System.Collections, Version=9.0.0.0, Culture=neutral"));
    assert_eq!(
        header.next().unwrap(),
        // 索引进程写规范化后的完整路径（Windows 上统一为反斜杠）。
        format!(
            "// {}",
            refs.join("System.Collections.dll")
                .display()
                .to_string()
                .replace('/', std::path::MAIN_SEPARATOR_STR)
        )
    );
    assert_eq!(header.next().unwrap(), "#endregion");

    // 泛型参数与约束。
    let dictionary = at("Assets/A.cs", A, "Dictionary<string");
    let defs = index.metadata_definitions(dictionary);
    assert_eq!(defs.len(), 1);
    let text = index.metadata_view_text(defs[0].view).unwrap();
    let declaration = text.lines().nth(defs[0].line as usize).unwrap();
    assert!(
        declaration.starts_with("public class Dictionary<TKey, TValue> : ")
            && declaration.ends_with(" where TKey : notnull"),
        "{declaration}"
    );

    // 第三方库：1.0 与 2.0 各一份视图；2.0 没有文档；internal 成员不出现。
    let widget = at("Assets/A.cs", A, "Widget<object>()");
    assert_eq!(index.metadata_views_of(widget).len(), 2);
    let defs = index.metadata_definitions(widget);
    assert_eq!(defs.len(), 2);
    for location in &defs {
        let text = index.metadata_view_text(location.view).unwrap();
        assert!(
            text.starts_with("#region 程序集 Rove.Fixture.Lib, Version="),
            "{text}"
        );
        assert!(
            text.contains("public class Widget<T> where T : class, new()"),
            "{text}"
        );
        assert!(
            text.contains("protected internal event Action? Changed;"),
            "{text}"
        );
        assert!(
            text.contains("public static Widget<T> operator +(Widget<T> a, Widget<T> b);"),
            "{text}"
        );
        assert!(!text.contains("Hidden"), "{text}");
    }
    let make = at("Assets/A.cs", A, "Make(ref");
    assert_eq!(
        index.signature(make),
        Some("public T Make(ref int count, params string[] names);")
    );
    assert_eq!(
        index.xml_doc(make).as_deref(),
        Some("<summary>Makes it.</summary>")
    );
    let v2 = at("Assets/B.cs", B, "V2Only");
    let defs = index.metadata_definitions(v2);
    assert_eq!(defs.len(), 1);
    let (name, above) = line_of(&defs[0]);
    assert_eq!(name, "V2Only");
    assert!(!above.starts_with("///"), "2.0 has no XML file: {above}");
    assert_eq!(index.xml_doc(v2), None);

    // 嵌套类型：在外层类型的视图里有自己的位置，成员也能沿容器找到。
    let options = at("Assets/A.cs", A, "Options()");
    let defs = index.metadata_definitions(options);
    assert_eq!(defs.len(), 2);
    for location in &defs {
        assert_eq!(line_of(location).0, "Options");
        assert!(index.metadata_views_of(widget).contains(&location.view));
    }
    let size = at("Assets/A.cs", A, "Size = 1");
    let defs = index.metadata_definitions(size);
    assert_eq!(defs.len(), 2);
    let (name, above) = line_of(&defs[0]);
    assert_eq!(name, "Size");
    assert_eq!(above, "/// <summary>Size.</summary>");

    // 命名实参引用的外部方法参数（#126）：位置落在视图里所属成员签名行上的参数名。
    let line_at = |location: &MetadataLocation| -> String {
        let text = index.metadata_view_text(location.view).unwrap();
        text.lines().nth(location.line as usize).unwrap().to_owned()
    };
    let parameter = |token: &str,
                     expect_name: &str,
                     expect_line: &str,
                     views: usize|
     -> (u32, MetadataLocation) {
        let symbol = at("Assets/A.cs", A, token);
        let defs = index.metadata_definitions(symbol);
        assert_eq!(defs.len(), views, "{token}: {defs:?}");
        for location in &defs {
            assert_eq!(line_of(location).0, expect_name, "{token}");
            let line = line_at(location);
            assert!(line.trim().starts_with(expect_line), "{token}: {line}");
        }
        (symbol, defs[0])
    };
    // 普通方法的参数；方法本身与它的参数在同一行、不同列。Widget 有 1.0 与 2.0 两份视图。
    let (count, count_at) = parameter("count: ref c", "count", "public T Make(ref int count,", 2);
    let make_at = index.metadata_definitions(make);
    assert_eq!(count_at.view, make_at[0].view);
    assert_eq!(count_at.line, make_at[0].line);
    assert!(count_at.col > make_at[0].col);
    assert_ne!(count, make);
    // 扩展方法以实例形式调用：序号要算上 this 参数。
    parameter(
        "factor: 2",
        "factor",
        "public static int Scale(this Widget<object> w,",
        1,
    );
    // 同名同类型、但属于不同方法的参数不再共用一个后备键。
    let (scale_label, _) = parameter(
        "label: \"x\"",
        "label",
        "public static int Scale(this Widget<object> w,",
        1,
    );
    let (rename_label, _) = parameter(
        "label: \"y\"",
        "label",
        "public void Rename(string label);",
        2,
    );
    assert_ne!(scale_label, rename_label);
    // 委托的参数落在委托声明行上；泛型 BCL 类型的方法参数按原始定义取键。
    parameter(
        "code: 1",
        "code",
        "public delegate void Callback(int code,",
        1,
    );
    parameter("index: 0", "index", "public void Insert(int index,", 1);

    // 解锁后 B 的同一个编译里同时引用 2.0 与 1.0 的拷贝（同名不同版本）：Roslyn 在编译内只取其一，
    // 索引照常完成，视图没有语法错误。
    let (third, report, _) = run("third.idx");
    assert_eq!(report["metadata"]["views_with_syntax_errors"], 0);
    let index = CsIndex::open(&third).unwrap();
    let widget = index
        .symbol_at(
            "Assets/A.cs",
            2,
            Utf8Column(A.lines().nth(2).unwrap().find("Widget<object>()").unwrap() as u64),
        )
        .unwrap();
    assert!(!index.metadata_views_of(widget).is_empty());
    assert!(!index.metadata_definitions(widget).is_empty());
}
