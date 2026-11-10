mod support;

use futures_lite::future::block_on;
use rove_base::{ByteOffset, Cancellation, Error, Line, Span, TextEdit, Version};
use rove_syntax::{
    highlight::{language_for_path, TreeSitterSyntax},
    SyntaxService, SyntaxSnapshot, SyntaxUpdate,
};
use std::{path::Path, sync::Arc, time::Duration};
use support::Text;

fn parse(service: &dyn SyntaxService, language: &str, input: &str, version: u64) -> SyntaxUpdate {
    block_on(service.analyze(
        Arc::new(Text::new(input, version)),
        language.into(),
        Cancellation::default(),
    ))
    .unwrap()
}

fn snapshot(value: &dyn SyntaxSnapshot, lines: u64) -> String {
    let mut output = String::new();
    for row in 0..lines {
        for token in value.tokens(Line(row)).unwrap().iter() {
            use std::fmt::Write;
            writeln!(
                output,
                "{row} {}..{} {}",
                token.range.start().0,
                token.range.end().0,
                token.scopes.join(" "),
            )
            .unwrap();
        }
    }
    output
}

#[test]
fn csharp_token_snapshot() {
    let result = parse(
        &TreeSitterSyntax::default(),
        "csharp",
        "public class Café {\r\n  int x = 42;\r\n}",
        1,
    );
    assert_eq!(
        snapshot(result.snapshot.as_ref(), 3),
        concat!(
            "0 0..6 storage.modifier.public.cs\n",
            "0 7..12 storage.type.class.cs\n",
            "0 13..18 entity.name.type.class.cs\n",
            "0 19..20 punctuation.curlybrace.open.cs\n",
            "1 24..27 keyword.type.int.cs\n",
            "1 28..29 entity.name.variable.field.cs\n",
            "1 30..31 keyword.operator.assignment.cs\n",
            "1 32..34 constant.numeric.decimal.cs\n",
            "1 34..35 punctuation.terminator.statement.cs\n",
            "2 37..38 punctuation.curlybrace.close.cs\n",
        )
    );
}

#[test]
fn json_token_snapshot() {
    let result = parse(&TreeSitterSyntax::default(), "json", "{\n\"é\": true\n}", 1);
    assert_eq!(snapshot(result.snapshot.as_ref(), 3), concat!(
        "0 0..1 punctuation.definition.dictionary.begin.json\n",
        "1 2..3 string.quoted.double.json string.json support.type.property-name.json punctuation.definition.string.begin.json punctuation.support.type.property-name.begin.json\n",
        "1 3..5 string.quoted.double.json string.json support.type.property-name.json\n",
        "1 5..6 string.quoted.double.json string.json support.type.property-name.json punctuation.definition.string.end.json punctuation.support.type.property-name.end.json\n",
        "1 6..7 punctuation.separator.dictionary.key-value.json\n",
        "1 8..12 constant.language.json\n",
        "2 13..14 punctuation.definition.dictionary.end.json\n",
    ));
}

#[test]
fn yaml_token_snapshot() {
    let result = parse(&TreeSitterSyntax::default(), "yaml", "é: true\n# note\n", 1);
    assert_eq!(snapshot(result.snapshot.as_ref(), 3), concat!(
        "0 0..2 string.unquoted.plain.out.yaml meta.map.key.yaml string.unquoted.plain.yaml entity.name.tag.yaml\n",
        "0 2..3 punctuation.separator.map.value.yaml\n",
        "0 4..8 constant.language.boolean.yaml\n",
        "1 9..15 comment.line.number-sign.yaml\n",
    ));
}

#[test]
fn xml_token_snapshot() {
    let result = parse(&TreeSitterSyntax::default(), "xml", "<a x='é'/>\n", 1);
    assert_eq!(snapshot(result.snapshot.as_ref(), 2), concat!(
        "0 0..1 meta.tag.no-content.xml punctuation.definition.tag.xml\n",
        "0 1..2 meta.tag.no-content.xml entity.name.tag.xml entity.name.tag.localname.xml\n",
        "0 2..3 meta.tag.no-content.xml\n",
        "0 3..4 meta.tag.no-content.xml entity.other.attribute-name.xml entity.other.attribute-name.localname.xml\n",
        "0 4..5 meta.tag.no-content.xml\n",
        "0 5..9 meta.tag.no-content.xml string.quoted.single.xml\n",
        "0 9..11 meta.tag.no-content.xml punctuation.definition.tag.xml\n",
    ));
}

#[test]
fn markdown_token_snapshot() {
    let result = parse(
        &TreeSitterSyntax::default(),
        "markdown",
        "# 标题\n\n**bold**\n",
        1,
    );
    assert_eq!(
        snapshot(result.snapshot.as_ref(), 4),
        concat!(
            "0 0..1 punctuation.definition.heading.markdown\n",
            "0 2..8 markup.heading.markdown heading.1.markdown entity.name.section.markdown\n",
            "2 10..12 markup.bold.markdown punctuation.definition.bold.markdown\n",
            "2 12..16 markup.bold.markdown\n",
            "2 16..18 markup.bold.markdown punctuation.definition.bold.markdown\n",
        )
    );
}

#[test]
fn markdown_injects_supported_languages_and_keeps_unknown_plain() {
    for (fence, body, scope) in [
        ("csharp", "public class A {}", "storage.modifier.public.cs"),
        ("json", "{\"x\": 7}", "constant.numeric.json"),
        ("yaml", "x: true", "constant.language.boolean.yaml"),
        ("xml", "<x/>", "entity.name.tag.localname.xml"),
        ("markdown", "# title", "entity.name.section.markdown"),
    ] {
        let mark = char::from(96).to_string().repeat(3);
        let source = format!("> {mark}{fence}\n> {body}\n> {mark}\n");
        let result = parse(&TreeSitterSyntax::default(), "markdown", &source, 0);
        let start = source.find(body).unwrap() as u64;
        let line = result.snapshot.tokens(Line(1)).unwrap();
        assert!(
            line.iter()
                .any(|t| t.range.start().0 >= start && t.scopes.iter().any(|s| &**s == scope)),
            "{fence}: {}",
            snapshot(result.snapshot.as_ref(), 4)
        );
    }

    let mark = char::from(96).to_string().repeat(3);
    let result = parse(
        &TreeSitterSyntax::default(),
        "markdown",
        &format!("{mark}unknown\npublic class A {{}}\n{mark}\n"),
        0,
    );
    assert!(result.snapshot.tokens(Line(1)).unwrap().is_empty());
}

#[test]
fn unity_files_open_and_analyze_on_a_background_worker() {
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<_> = [
        (
            "a.asmdef",
            "{\"name\":\"Assembly\"}",
            "json",
            "support.type.property-name.json",
        ),
        (
            "a.asmref",
            "{\"reference\":\"Assembly\"}",
            "json",
            "support.type.property-name.json",
        ),
        (
            "a.meta",
            "fileFormatVersion: 2",
            "yaml",
            "entity.name.tag.yaml",
        ),
        ("a.prefab", "GameObject: {}", "yaml", "entity.name.tag.yaml"),
        ("a.unity", "SceneRoots: []", "yaml", "entity.name.tag.yaml"),
        (
            "a.asset",
            "MonoBehaviour: {}",
            "yaml",
            "entity.name.tag.yaml",
        ),
    ]
    .into_iter()
    .map(|(name, source, language, scope)| {
        let path = dir.path().join(name);
        std::fs::write(&path, source).unwrap();
        (path, language, scope)
    })
    .collect();
    let parsed = std::thread::spawn(move || {
        let syntax = TreeSitterSyntax::default();
        files
            .into_iter()
            .map(|(path, language, scope)| {
                let id = language_for_path(&path, None);
                assert_eq!(&*id, language);
                let source = std::fs::read_to_string(path).unwrap();
                (parse(&syntax, &id, &source, 7).snapshot, scope)
            })
            .collect::<Vec<_>>()
    })
    .join()
    .unwrap();
    for (result, scope) in parsed {
        assert_eq!(result.version(), Version(7));
        assert!(result
            .tokens(Line(0))
            .unwrap()
            .iter()
            .any(|t| t.scopes.iter().any(|s| &**s == scope)));
        assert!(matches!(result.tokens(Line(1)), Err(Error::OutOfBounds)));
    }
}

fn replacement(version: u64, start: usize, old_len: usize, inserted: &str) -> TextEdit {
    TextEdit {
        version: Version(version),
        range: Span::new(
            ByteOffset(start as u64),
            ByteOffset((start + old_len) as u64),
        )
        .unwrap(),
        inserted: inserted.into(),
    }
}

#[test]
fn incremental_character_edit_reports_only_the_method_body_line() {
    let service = TreeSitterSyntax::default().document();
    let old = "class A {\n  int M() {\n    return 1;\n  }\n}\n";
    let initial = parse(&service, "csharp", old, 1);
    let new = old.replace('1', "2");
    let service = service.with_edit(replacement(1, old.find('1').unwrap(), 1, "2"));
    let update = parse(&service, "csharp", &new, 2);
    assert_eq!(update.previous, Some(Version(1)));
    assert_eq!(
        update.changed_lines.as_deref(),
        Some(&[Span::new(Line(2), Line(3)).unwrap()][..])
    );
    assert_eq!(initial.snapshot.version(), Version(1));
    let full = parse(&TreeSitterSyntax::default(), "csharp", &new, 2);
    assert_eq!(
        snapshot(update.snapshot.as_ref(), 6),
        snapshot(full.snapshot.as_ref(), 6)
    );
}

#[test]
fn incremental_matches_full_after_newline_insertion_and_multiline_comment() {
    let service = TreeSitterSyntax::default().document();
    let old = "class C {\n int a;\n int b;\n int c;\n // */\n}\n";
    parse(&service, "csharp", old, 1);
    let start = old.find("int a").unwrap();
    let new = format!("{}/*\n{}", &old[..start], &old[start..]);
    let patched = service.with_edit(replacement(1, start, 0, "/*\n"));
    let update = parse(&patched, "csharp", &new, 2);
    let full = parse(&TreeSitterSyntax::default(), "csharp", &new, 2);
    assert_eq!(
        snapshot(update.snapshot.as_ref(), 8),
        snapshot(full.snapshot.as_ref(), 8)
    );
    assert!(update
        .changed_lines
        .unwrap()
        .iter()
        .any(|r| r.start() <= Line(3) && r.end() > Line(3)));
}

#[test]
fn deadline_returns_previous_version_and_cancellation_remains_distinct() {
    let service = TreeSitterSyntax::default()
        .with_refresh_budget(Duration::ZERO)
        .document();
    let initial = parse(&service, "json", "true", 1);
    let changed = service.with_edit(replacement(1, 0, 4, "false"));
    let stale = parse(&changed, "json", "false", 2);
    assert_eq!(stale.snapshot.version(), Version(1));
    assert_eq!(stale.previous, Some(Version(1)));
    assert!(stale.changed_lines.unwrap().is_empty());
    assert!(Arc::ptr_eq(&initial.snapshot, &stale.snapshot));

    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(matches!(
        block_on(changed.analyze(Arc::new(Text::new("false", 2)), "json".into(), cancel)),
        Err(Error::Cancelled)
    ));
}

#[test]
fn long_csharp_lines_skip_highlighting_and_keep_following_lines() {
    let long = format!("// {}\npublic class A {{}}\n", "x".repeat(2500));
    let result = parse(&TreeSitterSyntax::default(), "csharp", &long, 1);
    assert!(result.snapshot.tokens(Line(0)).unwrap().is_empty());
    assert!(!result.snapshot.tokens(Line(1)).unwrap().is_empty());
    assert_eq!(
        &*language_for_path(Path::new("Example.JSONC"), None),
        "jsonc"
    );
}

#[test]
fn newline_insertion_preserves_unchanged_tokens_with_rebased_bytes() {
    let service = TreeSitterSyntax::default().document();
    let old = "class C {\n string a = \"中文\";\n int b = 7;\n}\n";
    let initial = parse(&service, "csharp", old, 1);
    let at = old.find("string").unwrap();
    let insertion = "// added 😀\r\n ";
    let new = format!("{}{insertion}{}", &old[..at], &old[at..]);
    let updated = parse(
        &service.with_edit(replacement(1, at, 0, insertion)),
        "csharp",
        &new,
        2,
    );
    assert_eq!(
        updated.changed_lines.as_deref(),
        Some(&[Span::new(Line(1), Line(3)).unwrap()][..])
    );
    let old_token = initial.snapshot.tokens(Line(2)).unwrap()[0].clone();
    let new_token = updated.snapshot.tokens(Line(3)).unwrap()[0].clone();
    assert_eq!(
        old_token.range.start().0 + insertion.len() as u64,
        new_token.range.start().0
    );
    assert!(Arc::ptr_eq(&old_token.scopes, &new_token.scopes));
    let full = parse(&TreeSitterSyntax::default(), "csharp", &new, 2);
    assert_eq!(
        snapshot(updated.snapshot.as_ref(), 6),
        snapshot(full.snapshot.as_ref(), 6)
    );
}

#[test]
fn settings_override_file_associations_and_highlighting_limits() {
    use rove_base::{Events, NoEvents, Result, Task, Value};
    use rove_config::{Scope, Settings};

    struct Overrides;
    impl Settings for Overrides {
        fn read(&self, key: &str, scope: &Scope) -> Result<Value> {
            match key {
                "files.associations" => Ok(Value::Object(Arc::new(
                    [
                        ("*.asset".into(), Value::String("json".into())),
                        ("**/config/*.asset".into(), Value::String("xml".into())),
                    ]
                    .into_iter()
                    .collect(),
                ))),
                "editor.maxTokenizationLineLength"
                    if scope.language.as_deref() == Some("csharp") =>
                {
                    Ok(Value::Number(10.0))
                }
                "rove.largeFile.threshold" => Ok(Value::Number(64.0)),
                _ => Err(Error::NotFound),
            }
        }

        fn write(&self, _: Arc<str>, _: Scope, _: Value, _: Cancellation) -> Task<()> {
            Box::pin(async { Err(Error::Unsupported) })
        }

        fn changes(&self) -> &dyn Events<rove_config::SettingsChange> {
            &NoEvents
        }
    }

    let settings = Arc::new(Overrides);
    assert_eq!(
        &*language_for_path(Path::new("src/a.asset"), Some(settings.as_ref())),
        "json"
    );
    assert_eq!(
        &*language_for_path(Path::new("src/config/a.asset"), Some(settings.as_ref())),
        "xml"
    );
    let syntax = TreeSitterSyntax::default().with_settings(settings);
    let result = parse(&syntax, "csharp", "public class A {}\n// note", 1);
    assert!(result.snapshot.tokens(Line(0)).unwrap().is_empty());
    assert!(!result.snapshot.tokens(Line(1)).unwrap().is_empty());
    let result = parse(&syntax, "json", &format!("true\n{}", " ".repeat(59)), 2);
    assert!(result.snapshot.tokens(Line(0)).unwrap().is_empty());
    assert!(result.snapshot.tokens(Line(1)).unwrap().is_empty());
}
