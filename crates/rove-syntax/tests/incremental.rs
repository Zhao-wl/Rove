//! #94: incremental updates, per-document baselines, visible-first results and settings.
mod support;

use futures_lite::future::block_on;
use rove_base::{ByteOffset, Cancellation, DocumentId, Error, Line, Span, TextEdit, Version};
use rove_buffer::{RopeText, TextSource};
use rove_syntax::{
    highlight::{DocumentHighlighter, DocumentSyntax, SyntaxDocuments, TreeSitterSyntax},
    SyntaxService, SyntaxSnapshot, SyntaxUpdate,
};
use std::{fmt::Write, sync::Arc, time::Duration};
use support::Text;

fn run(service: &dyn SyntaxService, language: &str, text: Arc<dyn TextSource>) -> SyntaxUpdate {
    block_on(service.analyze(text, language.into(), Cancellation::default())).unwrap()
}

fn edit(version: u64, start: usize, old_len: usize, inserted: &str) -> TextEdit {
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

/// Every token of every line, with the text it covers.
fn dump(snapshot: &dyn SyntaxSnapshot, text: &str, lines: u64) -> String {
    let mut out = String::new();
    for row in 0..lines {
        for token in snapshot.tokens(Line(row)).unwrap().iter() {
            let (start, end) = (token.range.start().0 as usize, token.range.end().0 as usize);
            writeln!(
                out,
                "{row} {start}..{end} {:?} {}",
                &text[start..end],
                token.scopes.join(" ")
            )
            .unwrap();
        }
    }
    out
}

fn line_count(text: &str) -> u64 {
    RopeText::from_text(text).lines().known
}

/// Apply `edit` to `document`, then check the result against a full parse of the new text.
/// Returns the reported changed lines.
fn apply(
    document: &DocumentSyntax,
    language: &str,
    current: &mut String,
    version: &mut u64,
    start: usize,
    old_len: usize,
    inserted: &str,
) -> Vec<(u64, u64)> {
    current.replace_range(start..start + old_len, inserted);
    let update = run(
        &document.with_edit(edit(*version, start, old_len, inserted)),
        language,
        Arc::new(Text::new(current.clone(), *version + 1)),
    );
    *version += 1;
    assert_eq!(update.snapshot.version(), Version(*version));
    let full = run(
        &TreeSitterSyntax::default().with_parallelism(0),
        language,
        Arc::new(Text::new(current.clone(), *version)),
    );
    let lines = Text::new(current.clone(), 0).lines().known;
    assert_eq!(
        dump(update.snapshot.as_ref(), current, lines),
        dump(full.snapshot.as_ref(), current, lines),
        "incremental result differs from a full parse after inserting {inserted:?}"
    );
    update
        .changed_lines
        .expect("an incremental update")
        .iter()
        .map(|r| (r.start().0, r.end().0))
        .collect()
}

fn unity_namespace(classes: usize) -> String {
    let mut source = String::from("using UnityEngine;\nnamespace Game {\n");
    for i in 0..classes {
        write!(
            source,
            concat!(
                "    public sealed class Mover{i} : MonoBehaviour\n    {{\n",
                "        #region Fields\n        [SerializeField] private float speed = 2.5f;\n",
                "        public bool IsActive {{ get; private set; }}\n        #endregion\n",
                "        private void Update()\n        {{\n",
                "            var step = speed * Time.deltaTime; // move {i}\n",
                "            Debug.Log($\"Speed {{speed:F2}} on {{gameObject.name}}\", this);\n",
                "        }}\n",
                "        #if UNITY_EDITOR\n        private void OnValidate() {{ speed = 0; }}\n        #endif\n",
                "    }}\n"
            ),
            i = i
        )
        .unwrap();
    }
    source.push_str("}\n");
    source
}

#[test]
fn partitioned_csharp_edits_match_a_full_parse() {
    // A private pool keeps this test independent of other analyses in the process.
    let syntax = TreeSitterSyntax::default().with_parallelism(4);
    let document = syntax.document();
    let mut current = unity_namespace(700);
    assert!(current.len() > 256 * 1024);
    run(&document, "csharp", Arc::new(Text::new(current.clone(), 1)));
    let mut version = 1;

    // Structural edit inside one method body.
    let at = current.find("speed * Time.deltaTime; // move 350").unwrap() + "speed".len();
    let changed = apply(
        &document,
        "csharp",
        &mut current,
        &mut version,
        at,
        0,
        " + (2 * 3)",
    );
    let row = current[..at].matches('\n').count() as u64;
    assert_eq!(changed, vec![(row, row + 1)]);

    // A new member with its own lines inside the same class.
    let at = current.find("        #if UNITY_EDITOR\n        private void OnValidate() { speed = 0; }\n        #endif\n    }\n    public sealed class Mover401").unwrap();
    apply(
        &document,
        "csharp",
        &mut current,
        &mut version,
        at,
        0,
        "        private int Count() { return 42; }\n",
    );

    // Removing a line, and an edit in the shared header falls back to a full rebuild.
    let line = "            var step = speed * Time.deltaTime; // move 10\n";
    let at = current.find(line).unwrap();
    let changed = apply(
        &document,
        "csharp",
        &mut current,
        &mut version,
        at,
        line.len(),
        "",
    );
    let row = current[..at].matches('\n').count() as u64;
    assert_eq!(changed, vec![(row, row + 1)]);
    let at = "using Unity".len();
    apply(
        &document,
        "csharp",
        &mut current,
        &mut version,
        at,
        6,
        "Engine2",
    );
}

#[test]
fn mixed_line_endings_highlight_like_a_full_parse() {
    // #92: CRLF, lone CR and LF in one file, read through the real RopeText.
    let source =
        "public class A {\r\n    int a = 1;\r    string b = \"x\";\n    void M() { }\r\n}\r";
    let text = Arc::new(RopeText::from_text(source));
    let result = run(&TreeSitterSyntax::default(), "csharp", text.clone());
    let lines = text.lines().known;
    assert_eq!(lines, 6);
    for row in 0..lines {
        let range = text.line_range(Line(row)).unwrap();
        for token in result.snapshot.tokens(Line(row)).unwrap().iter() {
            assert!(token.range.start() >= range.start() && token.range.end() <= range.end());
        }
    }
    let scopes = |row: u64| -> Vec<String> {
        result
            .snapshot
            .tokens(Line(row))
            .unwrap()
            .iter()
            .map(|t| t.scopes.join(" "))
            .collect()
    };
    assert!(scopes(1)
        .iter()
        .any(|s| s.contains("constant.numeric.decimal.cs")));
    assert!(scopes(2)
        .iter()
        .any(|s| s.contains("string.quoted.double.cs")));
    assert!(scopes(3)
        .iter()
        .any(|s| s.contains("entity.name.function.cs")));
    assert!(scopes(4)
        .iter()
        .any(|s| s.contains("punctuation.curlybrace.close.cs")));

    // Turning the lone CR into CRLF, and an LF into a lone CR, keeps tokens in place.
    let document = patient().document();
    let mut current = source.to_string();
    let first = Arc::new(RopeText::from_text(&current));
    run(&document, "csharp", first.clone());
    let mut version = first.version().0;
    for (needle, replace, with) in [("1;\r", 3, "1;\r\n"), ("\"x\";\n", 5, "\"x\";\r")] {
        let at = current.find(needle).unwrap();
        current.replace_range(at..at + replace, with);
        let next = Arc::new(RopeText::from_text(&current));
        let update = run(
            &document.with_edit(edit(version, at, replace, with)),
            "csharp",
            next.clone(),
        );
        assert_eq!(update.previous, Some(Version(version)));
        assert!(update.changed_lines.is_some_and(|lines| lines.len() <= 2));
        version = next.version().0;
        assert_eq!(update.snapshot.version(), next.version());
        let full = run(&TreeSitterSyntax::default(), "csharp", next.clone());
        let lines = next.lines().known;
        assert_eq!(
            dump(update.snapshot.as_ref(), &current, lines),
            dump(full.snapshot.as_ref(), &current, lines)
        );
    }
}

fn markdown(sections: usize) -> String {
    let mut source = String::new();
    for i in 0..sections {
        write!(
            source,
            "# Section {i}\n\nSome *emphasis* and `code` in paragraph {i}.\nSecond line **bold**.\n\n```cs\nclass C{i} {{ int x = {i}; }}\n```\n\n"
        )
        .unwrap();
    }
    source
}

#[test]
fn markdown_edits_rehighlight_only_the_affected_region() {
    let document = patient().document();
    let mut current = markdown(40);
    run(
        &document,
        "markdown",
        Arc::new(Text::new(current.clone(), 1)),
    );
    let mut version = 1;

    // Inline edit inside one paragraph: only that paragraph's lines are reported.
    let at = current.find("in paragraph 20.").unwrap();
    let row = current[..at].matches('\n').count() as u64;
    let changed = apply(
        &document,
        "markdown",
        &mut current,
        &mut version,
        at,
        2,
        "_in_",
    );
    assert!(!changed.is_empty());
    assert!(changed
        .iter()
        .all(|&(start, end)| start >= row && end <= row + 2));

    // Switching a fence language re-highlights that fence only.
    let at = current.find("```cs\nclass C30").unwrap() + 3;
    let row = current[..at].matches('\n').count() as u64;
    let changed = apply(
        &document,
        "markdown",
        &mut current,
        &mut version,
        at,
        2,
        "json",
    );
    assert!(changed
        .iter()
        .all(|&(start, end)| start >= row && end <= row + 3));

    // Removing the blank line between paragraphs joins them, so emphasis can span both.
    let mut current = String::from("*open\n\nclose* tail\n\nnext `a\n\nb` end\n");
    let document = patient().document();
    run(
        &document,
        "markdown",
        Arc::new(Text::new(current.clone(), 1)),
    );
    let mut version = 1;
    let at = current.find("\n\nclose").unwrap();
    apply(&document, "markdown", &mut current, &mut version, at, 1, "");
    let at = current.find("`a\n\nb").unwrap() + 2;
    apply(&document, "markdown", &mut current, &mut version, at, 1, "");
    assert!(dump(
        run(
            &TreeSitterSyntax::default(),
            "markdown",
            Arc::new(Text::new(current.clone(), 9))
        )
        .snapshot
        .as_ref(),
        &current,
        line_count(&current)
    )
    .contains("markup.italic.markdown"));
}

#[test]
fn markdown_refresh_past_the_budget_keeps_the_previous_result() {
    let document = TreeSitterSyntax::default()
        .with_refresh_budget(Duration::ZERO)
        .document();
    let source = markdown(4);
    let initial = run(
        &document,
        "markdown",
        Arc::new(Text::new(source.clone(), 1)),
    );
    let at = source.find("emphasis").unwrap();
    let mut next = source.clone();
    next.replace_range(at..at, "x");
    let stale = run(
        &document.with_edit(edit(1, at, 0, "x")),
        "markdown",
        Arc::new(Text::new(next, 2)),
    );
    assert!(Arc::ptr_eq(&initial.snapshot, &stale.snapshot));
    assert_eq!(stale.changed_lines.as_deref().map(<[_]>::len), Some(0));
}

#[test]
fn visible_first_results_are_completed_by_the_next_analysis() {
    let source = markdown(60);
    let text: Arc<dyn TextSource> = Arc::new(Text::new(source.clone(), 3));
    let document = patient().document();
    let visible = Span::new(Line(0), Line(12)).unwrap();
    let first = run(&document.with_visible(visible), "markdown", text.clone());
    assert!(first.changed_lines.is_none());
    assert!(document.is_partial());
    let late = source[..source.find("Section 50").unwrap()]
        .matches('\n')
        .count() as u64;
    assert!(!first.snapshot.tokens(Line(2)).unwrap().is_empty());
    assert!(first.snapshot.tokens(Line(late)).unwrap().is_empty());

    let complete = run(&document, "markdown", text.clone());
    assert!(!document.is_partial());
    assert!(complete.changed_lines.is_none());
    assert!(!complete.snapshot.tokens(Line(late)).unwrap().is_empty());
    let full = run(&TreeSitterSyntax::default(), "markdown", text.clone());
    let lines = text.lines().known;
    assert_eq!(
        dump(complete.snapshot.as_ref(), &source, lines),
        dump(full.snapshot.as_ref(), &source, lines)
    );
    // Asking again for the same text reuses the complete snapshot.
    let again = run(&document, "markdown", text);
    assert!(Arc::ptr_eq(&again.snapshot, &complete.snapshot));
}

fn analyze(
    documents: &SyntaxDocuments,
    id: u64,
    text: Arc<dyn TextSource>,
    edit: Option<TextEdit>,
) -> rove_base::Result<SyntaxUpdate> {
    block_on(documents.analyze(
        DocumentId(id),
        text,
        "csharp".into(),
        edit,
        None,
        Cancellation::default(),
    ))
}

#[test]
fn baselines_are_kept_per_document() {
    let documents = SyntaxDocuments::new(TreeSitterSyntax::default());
    // Two documents with the same version numbers.
    let a1: Arc<dyn TextSource> = Arc::new(Text::new("class A { int a = 1; }", 1));
    let b1: Arc<dyn TextSource> = Arc::new(Text::new("class B { string b; }", 1));
    let a = analyze(&documents, 1, a1.clone(), None).unwrap();
    let b = analyze(&documents, 2, b1.clone(), None).unwrap();
    assert_eq!(documents.len(), 2);
    // A → B → A: switching back reuses A's snapshot unchanged.
    let again = analyze(&documents, 1, a1.clone(), None).unwrap();
    assert!(Arc::ptr_eq(&again.snapshot, &a.snapshot));
    assert_eq!(again.changed_lines.as_deref().map(<[_]>::len), Some(0));
    let again = analyze(&documents, 2, b1, None).unwrap();
    assert!(Arc::ptr_eq(&again.snapshot, &b.snapshot));

    // An edit to A applies against A's baseline even though B shares its version.
    let a2: Arc<dyn TextSource> = Arc::new(Text::new("class A { int a = 2; }", 2));
    let at = "class A { int a = ".len();
    let updated = analyze(&documents, 1, a2.clone(), Some(edit(1, at, 1, "2"))).unwrap();
    assert_eq!(updated.previous, Some(Version(1)));
    assert!(updated.changed_lines.is_some());
    // An older version of A that finishes late is rejected.
    assert_eq!(analyze(&documents, 1, a1, None).err(), Some(Error::Stale));

    // Closing drops the baseline; a result that finishes afterwards is discarded.
    let late = documents.analyze(
        DocumentId(2),
        Arc::new(Text::new("class B { }", 2)),
        "csharp".into(),
        None,
        None,
        Cancellation::default(),
    );
    documents.close(DocumentId(2));
    assert_eq!(block_on(late).err(), Some(Error::Cancelled));
    assert_eq!(documents.len(), 1);
    // Reopening starts a new baseline: version 1 is accepted again.
    let reopened = analyze(&documents, 2, Arc::new(Text::new("class B { }", 1)), None).unwrap();
    assert_eq!(reopened.previous, None);
}

#[test]
fn csharp_line_limit_follows_settings_layers() {
    use rove_config::{register_builtin, SettingRegistry, SettingsOptions, SettingsService};

    let registry = || {
        let mut registry = SettingRegistry::default();
        rove_syntax::highlight::register_settings(&mut registry).unwrap();
        register_builtin(&mut registry).unwrap();
        registry
    };
    let long = |n: usize| format!("// {}\nclass A {{}}\n", "x".repeat(n));
    let highlighted = |syntax: &TreeSitterSyntax, language: &str, n: usize| {
        let text = if language == "csharp" {
            long(n)
        } else {
            format!("[{}]\n", "1,".repeat(n / 2))
        };
        !run(syntax, language, Arc::new(Text::new(text, 1)))
            .snapshot
            .tokens(Line(0))
            .unwrap()
            .is_empty()
    };

    // Registered defaults without a user file: C# keeps 2500.
    let defaults = TreeSitterSyntax::default()
        .with_settings(Arc::new(rove_config::DefaultSettings(registry())));
    assert!(highlighted(&defaults, "csharp", 2400));
    assert!(!highlighted(&defaults, "csharp", 2600));

    let dir = tempfile::tempdir().unwrap();
    let open = |json: &str| {
        std::fs::write(dir.path().join("settings.json"), json).unwrap();
        let mut options = SettingsOptions::new(dir.path(), dir.path().join("backup"));
        options.watch = None;
        Arc::new(SettingsService::open(options, registry()))
    };
    // A global user value does not beat VS Code's [csharp] default, but applies to JSON.
    let global = TreeSitterSyntax::default()
        .with_settings(open(r#"{ "editor.maxTokenizationLineLength": 1000 }"#));
    assert!(!highlighted(&global, "csharp", 2600));
    assert!(highlighted(&global, "csharp", 2400));
    assert!(!highlighted(&global, "json", 1200));
    // A user [csharp] value wins.
    let scoped = TreeSitterSyntax::default().with_settings(open(
        r#"{ "[csharp]": { "editor.maxTokenizationLineLength": 3000 } }"#,
    ));
    assert!(highlighted(&scoped, "csharp", 2600));
    assert!(!highlighted(&scoped, "csharp", 3100));
}

/// Incremental refreshes in debug builds on a busy machine can exceed the 50 ms budget;
/// these tests check results, not timing.
fn patient() -> TreeSitterSyntax {
    TreeSitterSyntax::default().with_refresh_budget(Duration::from_secs(30))
}
