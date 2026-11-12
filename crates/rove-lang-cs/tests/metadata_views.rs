//! #34 外部符号元数据视图：在真实 Unity 项目上抽查跳转落点、文档注释与去重。
//!
//! 需要 `ROVE_TEST_UNITY_MEDIUM`（未设置时跳过）和 Release 版索引进程
//! （`dotnet build tools/rove-indexer-cs -c Release`，未构建时跳过）。建索引约 45 s、峰值约 1.6 GB。

use rove_lang_cs::index::{CsIndex, MetadataLocation, SymbolKey};
use rove_test_support::{require_unity_project, UnityProject};
use std::path::Path;

fn indexer_dll() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/rove-indexer-cs/bin/Release/net9.0/rove-indexer-cs.dll")
}

fn symbol(index: &CsIndex, key: &str) -> Option<u32> {
    (0..index.symbols().len() as u32)
        .find(|&s| matches!(index.symbol_key(s), Ok(SymbolKey::Global(k)) if k == key))
}

/// 视图里落点处的文本，以及上一行。
fn landing(index: &CsIndex, location: &MetadataLocation) -> (String, String) {
    let text = index.metadata_view_text(location.view).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let line = lines[location.line as usize];
    let start = location.col.0 as usize;
    (
        line[start..start + location.len as usize].to_owned(),
        lines[location.line as usize - 1].trim().to_owned(),
    )
}

#[test]
fn unity_external_symbols_land_in_metadata_views() {
    let project = require_unity_project!(UnityProject::Medium);
    let dll = indexer_dll();
    if !dll.exists() {
        eprintln!(
            "已跳过：没有 Release 版索引进程 {}（先 dotnet build tools/rove-indexer-cs -c Release）",
            dll.display()
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let idx = dir.path().join("medium.idx");
    let report = dir.path().join("medium.json");
    let output = std::process::Command::new("dotnet")
        .arg(&dll)
        .arg(&project)
        .arg(&idx)
        .arg("--report")
        .arg(&report)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    let metadata = &report["metadata"];
    eprintln!(
        "metadata: views={} text_bytes={} documented_ratio={} miss_ratio={} syntax_errors={}",
        metadata["views"],
        metadata["view_text_bytes"],
        metadata["documented_ratio"],
        metadata["external_miss_ratio"],
        metadata["views_with_syntax_errors"]
    );
    assert_eq!(metadata["views_with_syntax_errors"], 0, "{metadata}");
    assert!(
        metadata["external_miss_ratio"].as_f64().unwrap() < 0.01,
        "{metadata}"
    );

    let index = CsIndex::open(&idx).unwrap();

    // UnityEngine.Transform.position：落在 CoreModule 的 Transform 视图里，上方是文档注释。
    let position = symbol(
        &index,
        "UnityEngine.CoreModule|P:UnityEngine.Transform.position",
    )
    .expect("project references Transform.position");
    let defs = index.metadata_definitions(position);
    assert_eq!(defs.len(), 1, "{defs:?}");
    let (name, above) = landing(&index, &defs[0]);
    assert_eq!(name, "position");
    assert!(above.starts_with("///"), "{above}");
    assert!(index.xml_doc(position).unwrap().contains("<summary>"));
    let text = index.metadata_view_text(defs[0].view).unwrap();
    assert!(text.starts_with("#region 程序集 UnityEngine.CoreModule, Version="));
    assert!(text
        .lines()
        .nth(1)
        .unwrap()
        .ends_with("UnityEngine.CoreModule.dll"));

    // 去重：几十个项目都引用 UnityEngine.CoreModule，Transform 只有一份视图。
    let referencing = std::fs::read_dir(&project)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "csproj"))
        .filter(|e| {
            std::fs::read_to_string(e.path())
                .is_ok_and(|s| s.contains("UnityEngine.CoreModule.dll"))
        })
        .count();
    assert!(
        referencing >= 10,
        "only {referencing} projects reference CoreModule"
    );
    let transform = symbol(&index, "UnityEngine.CoreModule|T:UnityEngine.Transform").unwrap();
    assert_eq!(index.metadata_views_of(transform).len(), 1);

    // System.Collections.Generic.List<T>.Add：netstandard（运行时程序集）或 mscorlib（编辑器程序集）各一份。
    let mut found = 0;
    for assembly in ["netstandard", "mscorlib"] {
        let key = format!("{assembly}|M:System.Collections.Generic.List`1.Add(`0)");
        let Some(add) = symbol(&index, &key) else {
            continue;
        };
        let defs = index.metadata_definitions(add);
        assert_eq!(defs.len(), 1, "{key}: {defs:?}");
        assert_eq!(landing(&index, &defs[0]).0, "Add");
        assert_eq!(index.signature(add), Some("public void Add(T item);"));
        found += 1;
    }
    assert!(found > 0, "project references List<T>.Add");

    // 第三方 DLL（不是 Unity、BCL）：被引用的符号能跳到视图，且 DLL 旁有 .xml 时带文档注释。
    let third_party: Vec<u32> = (0..index.symbols().len() as u32)
        .filter(|&s| {
            matches!(index.symbol_key(s), Ok(SymbolKey::Global(k))
                if !k.starts_with('?') && k.contains("|M:")
                    && !["Unity", "System", "mscorlib", "netstandard", "Microsoft"]
                        .iter()
                        .any(|p| k.starts_with(p)))
                && index.symbols()[s as usize].flags & 1 == 0
        })
        .collect();
    assert!(!third_party.is_empty());
    for &s in &third_party {
        let defs = index.metadata_definitions(s);
        let key = index.symbol_key(s);
        assert!(!defs.is_empty(), "{key:?} has no metadata view");
    }
    let documented = third_party
        .iter()
        .find(|&&s| index.xml_doc(s).is_some())
        .expect("a third-party DLL with an XML file");
    eprintln!(
        "third-party sample: {:?} -> {:?}",
        index.symbol_key(*documented),
        index.xml_doc(*documented)
    );
}
