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
    assert_eq!(index.header().indexer_version, [1, 0, 2, 0]);
    let rel_path = "Assets/Unicode.cs";
    let file = index.file_id(rel_path).unwrap();
    let line = source_text.lines().nth(1).unwrap();
    let def = line.find("值").unwrap();
    let reference = line.rfind("值").unwrap();
    let original_col = Utf16Column(line[..def].encode_utf16().count() as u32);
    assert!(def > u8::MAX as usize);
    assert_ne!(def as u32, original_col.0);
    let symbol = index
        .symbol_at(rel_path, 1, Utf8Column(reference as u32))
        .unwrap();
    let result = index.goto_definition(rel_path, 1, Utf8Column(reference as u32));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].col, Utf8Column(def as u32));
    for location in index.find_references(rel_path, 1, Utf8Column(reference as u32)) {
        let start = location.col.0 as usize;
        assert_eq!(&line[start..start + location.len as usize], "值");
    }
    let expected_offset = source_text.find("值").unwrap() as u32;
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

    let left_offset = GENERATED.find("value").unwrap() as u32;
    let right_offset = GENERATED.find("Right(int ").unwrap() as u32 + "Right(int ".len() as u32;
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
        let column = Utf8Column(column as u32);
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
            let c = Utf8Column(CALLER_C.find("value").unwrap() as u32);
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
