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
        let _ = fs::remove_file(self.dir.join("test.idx"));
        let _ = fs::remove_file(self.dir.join("Fixture.csproj"));
        let _ = fs::remove_file(self.dir.join("Assets/Unicode.cs"));
        let _ = fs::remove_dir(self.dir.join("Assets"));
        let _ = fs::remove_dir(&self.dir);
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
    let fixture = Fixture::new();
    fs::create_dir(fixture.dir.join("Assets")).unwrap();
    let mut source = File::create(fixture.dir.join("Assets/Unicode.cs")).unwrap();
    source.write_all(b"\xef\xbb\xbf").unwrap();
    source.write_all(SOURCE.as_bytes()).unwrap();
    drop(source);
    fs::write(
        fixture.dir.join("Fixture.csproj"),
        r#"<Project><PropertyGroup><AssemblyName>Assembly-CSharp</AssemblyName><DefineConstants>ROVE_TEST_ACTIVE</DefineConstants></PropertyGroup><ItemGroup><Compile Include="Assets/Unicode.cs"/></ItemGroup></Project>"#,
    ).unwrap();

    let index_path = fixture.dir.join("test.idx");
    let dll = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/rove-indexer-cs/bin/Release/net9.0/rove-indexer-cs.dll");
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
    let rel_path = "Assets/Unicode.cs";
    let file = index.file_id(rel_path).unwrap();
    let line = SOURCE.lines().nth(1).unwrap();
    let def = line.find("值").unwrap();
    let reference = line.rfind("值").unwrap();
    assert_ne!(def, line[..def].encode_utf16().count());
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
    let expected_offset = SOURCE.find("值").unwrap() as u32;
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
