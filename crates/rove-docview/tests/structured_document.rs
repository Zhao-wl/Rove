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
fn incremental_edit_reuses_expanded_branches_and_remaps_positions() {
    let text = r#"{"edit": 0, "retained": {"leaf": 42}}"#;
    let provider = parse(StructuredLanguage::Json, text);
    let cancel = Cancellation::default();
    let old = provider
        .node_for_path("$.retained.leaf", &cancel)
        .unwrap()
        .unwrap();
    let start = text.find('0').unwrap() as u64;
    let replace = ByteRange::new(ByteOffset(start), ByteOffset(start + 1)).unwrap();
    let updated = provider
        .reparse(&replace, "12345", Version(5), &cancel)
        .unwrap();
    let new = updated
        .snapshot
        .node_for_path("$.retained.leaf", &cancel)
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(&old.value_preview, &new.value_preview));
    assert_eq!(
        new.symbol.selection.start().column.0,
        old.symbol.selection.start().column.0 + 4
    );
    assert_eq!(updated.snapshot.value(new.id).unwrap(), "42");
    assert_eq!(provider.value(old.id).unwrap(), "42");
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
    assert_eq!(
        xml.node_at(item.symbol.selection.start(), &cancel)
            .unwrap()
            .unwrap()
            .path
            .as_ref(),
        "/root/item[3]",
    );
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
fn xml_space_keeps_references_and_unresolved_entities_conservative() {
    let cancel = Cancellation::default();
    let options = FormatOptions {
        indentation: "  ",
        ..Default::default()
    };
    let format = |source: &str| {
        format_text(
            StructuredLanguage::Xml,
            source,
            options,
            FormatMode::Pretty,
            &cancel,
        )
        .unwrap()
    };

    let preserve = r#"<root xml:space="pr&#101;serve"><block><a/><b/></block>  <end/></root>"#;
    assert_eq!(format(preserve), preserve);

    let reset = r#"<root xml:space="preserve"><block xml:space='d&#x65;fault'><a/><b/></block>  <end/></root>"#;
    let expected = "<root xml:space=\"preserve\"><block xml:space='d&#x65;fault'>\n    <a/>\n    <b/>\n  </block>  <end/></root>";
    assert_eq!(format(reset), expected);
    assert_eq!(format(expected), expected);

    let unresolved = r#"<root xml:space="&requested;"><block><a/><b/></block>  <end/></root>"#;
    let with_entity = format!("<!DOCTYPE root [<!ENTITY requested 'preserve'>]>{unresolved}");
    assert!(format(&with_entity).ends_with(unresolved));
}

#[test]
fn format_disjoint_selections_returns_contextual_replacements_in_order() {
    let text = r#"{"outer":{ "first":[1,2],"stay" : 1, "second":[3,4] }}"#;
    let first = text.find("\"first\"").unwrap();
    let second = text.find("\"second\"").unwrap();
    let ranges = [
        first..first + "\"first\":[1,2]".len(),
        second..second + "\"second\":[3,4]".len(),
    ];
    let options = FormatOptions {
        indentation: "  ",
        ..Default::default()
    };
    let replacements = rove_docview::format::format_selections(
        StructuredLanguage::Json,
        text,
        &ranges,
        options,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        replacements,
        [
            "\"first\": [\n      1,\n      2\n    ]",
            "\"second\": [\n      3,\n      4\n    ]",
        ]
    );
    let mut actual = text.to_owned();
    for (span, replacement) in ranges.into_iter().zip(replacements).rev() {
        actual.replace_range(span, &replacement);
    }
    assert!(actual.contains(r#","stay" : 1, "#));
}

#[test]
fn previews_count_arrays_and_truncate_long_strings_and_numbers() {
    let string = "数".repeat(80);
    let number = "7".repeat(80);
    let items = ["null"; 24].join(",");
    let text = format!(r#"{{"items":[{items}],"string":"{string}","number":{number}}}"#);
    let document = parse(StructuredLanguage::Json, &text);
    let roots = document.roots().unwrap();
    assert_eq!(roots[0].symbol.name.as_ref(), "items");
    assert_eq!(roots[0].value_preview.as_ref(), "[24]");
    assert_eq!(roots[1].value_preview.chars().count(), 65);
    assert_eq!(roots[2].value_preview.chars().count(), 65);
    assert!(roots[1].value_preview.ends_with('…'));
    assert!(roots[2].value_preview.ends_with('…'));
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

#[test]
fn jsonc_trailing_commas_preserve_locations_and_format_as_jsonc() {
    let source = "{\"items\":[2,3, /* items */],}";
    let cancel = Cancellation::default();
    let provider = parse(StructuredLanguage::Jsonc, source);
    assert!(provider.diagnostics(&cancel).unwrap().is_empty());
    let last = provider
        .node_for_path("$.items[1]", &cancel)
        .unwrap()
        .unwrap();
    assert_eq!(provider.value(last.id).unwrap(), "3");
    let pretty = format_text(
        StructuredLanguage::Jsonc,
        source,
        FormatOptions::default(),
        FormatMode::Pretty,
        &cancel,
    )
    .unwrap();
    assert!(pretty.contains("/* items */,") || pretty.contains(", /* items */"));
    assert_eq!(
        format_text(
            StructuredLanguage::Jsonc,
            &pretty,
            FormatOptions::default(),
            FormatMode::Pretty,
            &cancel
        )
        .unwrap(),
        pretty
    );
    assert!(!parse(StructuredLanguage::Jsonc, "[,]")
        .diagnostics(&cancel)
        .unwrap()
        .is_empty());
}

#[test]
fn format_selection_uses_document_context_without_touching_the_rest() {
    let text = r#"{"before":0, "chosen":[1,2,3], "after":4}"#;
    let start = text.find("\"chosen\"").unwrap();
    let end = text.find("],").unwrap() + 1;
    let result = rove_docview::format::format_selection(
        StructuredLanguage::Json,
        text,
        start..end,
        FormatOptions::default(),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(
        result,
        "\"chosen\": [\n        1,\n        2,\n        3\n    ]"
    );
}

#[test]
fn structure_snapshots_cover_empty_malformed_and_nested_documents() {
    use StructuredLanguage::*;
    let cancel = Cancellation::default();
    let cases = [
        (
            Json,
            "{\"outer\":{\"value\":3},\"other\":true}",
            vec![
                ("$.outer", "{1}"),
                ("$.outer.value", "3"),
                ("$.other", "true"),
            ],
        ),
        (
            Jsonc,
            "{/* note */\"outer\":{\"value\":3,},\"other\":true,}",
            vec![
                ("$.outer", "{1}"),
                ("$.outer.value", "3"),
                ("$.other", "true"),
            ],
        ),
        (
            Yaml,
            "outer:\n  value: 3\nother: true",
            vec![
                ("$.outer", "{1}"),
                ("$.outer.value", "3"),
                ("$.other", "true"),
            ],
        ),
        (
            Xml,
            "<outer id=\"x\"><value/><other/></outer>",
            vec![
                ("/outer", "id=\"x\" [2]"),
                ("/outer/@id", "\"x\""),
                ("/outer/value", "[0]"),
                ("/outer/other", "[0]"),
            ],
        ),
    ];
    for (language, text, expected) in cases {
        let document = parse(language, text);
        assert!(
            document.diagnostics(&cancel).unwrap().is_empty(),
            "{language:?}"
        );
        assert!(
            parse(language, "").roots().unwrap().is_empty(),
            "{language:?}"
        );
        let malformed = match language {
            Json | Jsonc => "{\"broken\": [",
            Yaml => "broken: [3,",
            Xml => "<broken><child></broken>",
        };
        assert!(
            !parse(language, malformed)
                .diagnostics(&cancel)
                .unwrap()
                .is_empty(),
            "{language:?}"
        );
        let mut nodes = document.roots().unwrap().to_vec();
        let mut actual = Vec::new();
        while let Some(node) = nodes.pop() {
            actual.push((node.path.to_string(), node.value_preview.to_string()));
            let mut task = document.children(node.id, Version(4), cancel.clone());
            let Poll::Ready(Ok(children)) =
                task.as_mut().poll(&mut Context::from_waker(Waker::noop()))
            else {
                panic!("children unavailable");
            };
            nodes.extend(children.iter().cloned());
        }
        actual.sort();
        let mut expected: Vec<_> = expected
            .into_iter()
            .map(|(p, v)| (p.to_owned(), v.to_owned()))
            .collect();
        expected.sort();
        assert_eq!(actual, expected, "{language:?}");
    }
}

proptest::proptest! {
    #[test]
    fn formatting_is_idempotent_for_mixed_eols_and_variable_document_shapes(
        values in proptest::collection::vec((-10000_i32..10000_i32, "[a-zA-Z0-9 ]{0,18}"), 1..8),
        spacing in proptest::sample::select(vec!["", " ", "   ", "\t"]),
        keep_lines in proptest::bool::ANY,
    ) {
        let options = FormatOptions { indentation: "  ", newline: "\r\n", keep_lines };
        let cancel = Cancellation::default();
        let items = values.iter().map(|(value, text)| format!("[{value},{spacing}\"{text}\"]")).collect::<Vec<_>>().join(",\n");
        let json = format!("{{\r\n\"items\": [{items}\r],\n\"enabled\": true\r\n}}");
        let items = values.iter().map(|(value, text)| format!("<item value=\"{value}\">{text}尾</item>")).collect::<Vec<_>>().join("\n");
        let xml = format!("<root>\r\n{items}\r<empty/>{spacing}\n<enabled/>\r\n</root>");
        for (language, source) in [(StructuredLanguage::Json, json), (StructuredLanguage::Xml, xml)] {
            let first = format_text(language, &source, options, FormatMode::Pretty, &cancel).unwrap();
            let second = format_text(language, &first, options, FormatMode::Pretty, &cancel).unwrap();
            proptest::prop_assert_eq!(&first, &second);
            let contains_in_order = |original: &str, formatted: &str| {
                let eols = |text: &str| {
                    let mut breaks = Vec::new();
                    let mut bytes = text.bytes().peekable();
                    while let Some(byte) = bytes.next() {
                        match byte {
                            b'\r' if bytes.peek() == Some(&b'\n') => { bytes.next(); breaks.push("\r\n"); }
                            b'\r' => breaks.push("\r"),
                            b'\n' => breaks.push("\n"),
                            _ => {}
                        }
                    }
                    breaks
                };
                let mut after = eols(formatted).into_iter();
                eols(original).into_iter().all(|before| after.any(|eol| eol == before))
            };
            proptest::prop_assert!(contains_in_order(&source, &first));
        }
    }
}
