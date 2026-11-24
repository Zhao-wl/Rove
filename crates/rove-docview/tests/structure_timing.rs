use rove_base::{Cancellation, Version};
use rove_docview::{
    structure::{ParsedStructure, StructuredLanguage},
    Structure,
};
use std::{sync::Arc, time::Instant};

/// Reproducible local timing for a compact JSON document, including the initial syntax parse
/// and the first layer. Node allocation below the root is verified without a timing assertion.
#[test]
#[ignore = "local timing; run in release mode with --nocapture"]
fn compact_json_first_layer() {
    const SIZE: usize = 50 * 1024 * 1024;
    let entry = r#"{"name":"abcdef","flags":[1,2,3],"nested":{"value":true}},"#;
    let mut text = String::with_capacity(SIZE + entry.len());
    text.push_str("{\"items\":[");
    while text.len() < SIZE {
        text.push_str(entry);
    }
    assert_eq!(text.pop(), Some(','));
    text.push_str("]}");
    let text: Arc<str> = text.into();
    let bytes = text.len();
    let start = Instant::now();
    let snapshot = ParsedStructure::parse(
        StructuredLanguage::Json,
        Version(1),
        text,
        5_000,
        &Cancellation::default(),
    )
    .unwrap();
    let roots = snapshot.roots().unwrap();
    println!(
        "structure/compact_json_first_layer: {bytes} bytes, {:?}",
        start.elapsed()
    );
    assert_eq!(roots.len(), 1);
    assert!(roots[0].children.is_none());
}
