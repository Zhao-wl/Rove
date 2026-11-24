use rove_base::{ByteOffset, ByteRange, Cancellation, Error, Line, Position, Utf8Column, Version};
use rove_docview::{
    format::{format_text, FormatMode, FormatOptions},
    structure::{ParsedStructure, ProblemKind, StructuredLanguage},
    Structure,
};
use std::{
    sync::Arc,
    task::{Context, Poll, Waker},
};

fn parse(language: StructuredLanguage, text: &str) -> ParsedStructure {
    ParsedStructure::parse(
        language,
        Version(4),
        text.into(),
        5_000,
        &Cancellation::default(),
    )
    .unwrap()
}

#[test]
fn nested_json_paths_previews_and_children_are_lazily_loaded() {
    let text = r#"{"items":[{}, {}, {}, {"name":"张三"}],"a.b":13}"#;
    let provider = parse(StructuredLanguage::Json, text);
    let roots = provider.roots().unwrap();
    assert_eq!(roots[0].symbol.name.as_ref(), "items");
    assert_eq!(roots[0].value_preview.as_ref(), "[4]");
    assert!(roots[0].children.is_none());
    assert!(roots[0].symbol.children.is_empty());

    let cancel = Cancellation::default();
    let mut task = provider.children(roots[0].id, Version(4), cancel.clone());
    let Poll::Ready(Ok(children)) = task.as_mut().poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("child list should be ready after background polling");
    };
    assert_eq!(children.len(), 4);
    assert!(children[3].children.is_none());
    assert!(roots[0].children.is_none());
    let node = provider
        .node_for_path("$.items[3].name", &cancel)
        .unwrap()
        .unwrap();
    assert_eq!(node.value_preview.as_ref(), "\"张三\"");
    let by_position = provider
        .node_at(node.symbol.selection.start(), &cancel)
        .unwrap()
        .unwrap();
    assert_eq!(node.id, by_position.id);
    assert_eq!(
        provider
            .node_for_path("$[\"a.b\"]", &cancel)
            .unwrap()
            .unwrap()
            .value_preview
            .as_ref(),
        "13",
    );
    let mut cached = provider.children(roots[0].id, Version(4), cancel);
    let Poll::Ready(Ok(second)) = cached
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("the expanded slice remains cached");
    };
    assert!(Arc::ptr_eq(&children, &second));
}

#[test]
fn reparse_preserves_the_previous_revision_and_reports_literal_changes() {
    let provider = parse(StructuredLanguage::Jsonc, "{ // path\r\n \"x\": 18\r}");
    let cancel = Cancellation::default();
    let start = provider
        .node_for_path("$.x", &cancel)
        .unwrap()
        .unwrap()
        .symbol
        .range
        .end()
        .column
        .0;
    let bytes = "{ // path\r\n \"x\": 18\r}".find("18").unwrap() as u64;
    let range = ByteRange::new(ByteOffset(bytes), ByteOffset(bytes + 2)).unwrap();
    let updated = provider.reparse(&range, "42", Version(5), &cancel).unwrap();
    assert_eq!(updated.changed[0].start().line, Line(1));
    assert_eq!(updated.changed[0].end().column, Utf8Column(start));
    assert_eq!(
        provider
            .node_for_path("$.x", &cancel)
            .unwrap()
            .unwrap()
            .value_preview
            .as_ref(),
        "18"
    );
    assert_eq!(
        updated
            .snapshot
            .node_for_path("$.x", &cancel)
            .unwrap()
            .unwrap()
            .value_preview
            .as_ref(),
        "42"
    );
    let mut task = updated
        .snapshot
        .children(provider.roots().unwrap()[0].id, Version(4), cancel);
    assert!(matches!(
        task.as_mut().poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(Err(Error::Stale))
    ));
}

#[test]
fn yaml_documents_and_xml_ordinals_have_distinct_paths() {
    let cancel = Cancellation::default();
    let yaml = parse(
        StructuredLanguage::Yaml,
        "---\nitems:\n  - name: first\n---\nitems:\n  - name: second\n",
    );
    assert_eq!(yaml.roots().unwrap().len(), 2);
    let last = yaml
        .node_for_path("$[1].items[0].name", &cancel)
        .unwrap()
        .unwrap();
    assert_eq!(last.value_preview.as_ref(), "second");

    let xml = parse(
        StructuredLanguage::Xml,
        "<root><item/><item id=\"two\"/><item name=\"三\"/></root>",
    );
    let item = xml
        .node_for_path("/root/item[3]", &cancel)
        .unwrap()
        .unwrap();
    assert_eq!(item.value_preview.as_ref(), "name=\"三\" [0]");
    let kind = xml
        .node_for_path("/root/item[3]/@name", &cancel)
        .unwrap()
        .unwrap();
    assert_eq!(
        xml.node_at(kind.symbol.selection.start(), &cancel)
            .unwrap()
            .unwrap()
            .id,
        kind.id
    );
    assert_eq!(
        xml.node_for_path("/root/item[0]", &cancel)
            .unwrap()
            .map(|n| n.id),
        None
    );
}

#[test]
fn diagnostics_use_editor_lines_and_utf8_columns() {
    let cancel = Cancellation::default();
    let json = parse(StructuredLanguage::Json, "{\r\n  \"中\": 1 // comments\n}");
    let problems = json.diagnostics(&cancel).unwrap();
    let problem = problems
        .iter()
        .find(|p| p.kind == ProblemKind::JsonComment)
        .unwrap();
    assert_eq!(
        problem.range.start(),
        Position {
            line: Line(1),
            column: Utf8Column(11)
        }
    );
    assert!(
        parse(StructuredLanguage::Jsonc, "{\n  \"中\": 1 // valid\n}")
            .diagnostics(&cancel)
            .unwrap()
            .is_empty()
    );
    assert!(!parse(StructuredLanguage::Yaml, "item: [1,")
        .diagnostics(&cancel)
        .unwrap()
        .is_empty());
    assert!(!parse(StructuredLanguage::Xml, "<root><nested></root>")
        .diagnostics(&cancel)
        .unwrap()
        .is_empty());
}

#[test]
fn json_formatting_preserves_tokens_and_mixed_eols() {
    let cancel = Cancellation::default();
    let options = FormatOptions {
        newline: "\r\n",
        ..Default::default()
    };
    let source = "{  \"a\" : 1,\n \"b\":[2, 3],\r \"c\": \"x\\n y\"\r\n}\n";
    let once = format_text(
        StructuredLanguage::Json,
        source,
        options,
        FormatMode::Pretty,
        &cancel,
    )
    .unwrap();
    let twice = format_text(
        StructuredLanguage::Json,
        &once,
        options,
        FormatMode::Pretty,
        &cancel,
    )
    .unwrap();
    assert_eq!(once, twice);
    assert!(once.contains("1,\n"));
    assert!(once.contains("],\r "));
    assert!(once.contains("\"x\\n y\"\r\n}"));
    assert_eq!(
        format_text(
            StructuredLanguage::Json,
            source,
            options,
            FormatMode::Minify,
            &cancel
        )
        .unwrap(),
        "{\"a\":1,\"b\":[2,3],\"c\":\"x\\n y\"}",
    );
    let one_line = r#"{"a": 1, "b": [2, 3]}"#;
    let keep = FormatOptions {
        keep_lines: true,
        ..options
    };
    assert_eq!(
        format_text(
            StructuredLanguage::Json,
            one_line,
            keep,
            FormatMode::Pretty,
            &cancel
        )
        .unwrap(),
        one_line
    );
}

#[test]
fn xml_formatting_respects_mixed_content_and_xml_space() {
    let cancel = Cancellation::default();
    let options = FormatOptions {
        indentation: "  ",
        ..Default::default()
    };
    let source = "<root><x/><p>Hello <b> world </b>!</p><block xml:space=\"preserve\"><x/><y/></block></root>";
    let first = format_text(
        StructuredLanguage::Xml,
        source,
        options,
        FormatMode::Pretty,
        &cancel,
    )
    .unwrap();
    assert!(first.starts_with("<root>\n  <x/>\n"));
    assert!(first.contains("<p>Hello <b> world </b>!</p>"));
    assert!(first.contains("<block xml:space=\"preserve\"><x/><y/></block>"));
    assert_eq!(
        first,
        format_text(
            StructuredLanguage::Xml,
            &first,
            options,
            FormatMode::Pretty,
            &cancel
        )
        .unwrap()
    );
}

#[test]
fn cancellation_does_not_publish_a_partial_snapshot() {
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        ParsedStructure::parse(
            StructuredLanguage::Json,
            Version(1),
            "{}".into(),
            5000,
            &cancel
        ),
        Err(Error::Cancelled)
    ));
}
