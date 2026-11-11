//! #94: highlighting on the real open path — background analysis of large files,
//! application by document and version, cancellation, and settings on the open path.
use super::*;
use crate::documents::OpenTarget;
use gpui::TestAppContext;
use rove_base::{LineRange, Task, TextEdit, Version};
use rove_buffer::{Documents, TextSource};
use rove_syntax::{highlight::DocumentHighlighter, EmptySyntax, SyntaxUpdate};
use std::sync::Mutex;

fn resource(name: &str) -> Resource {
    Resource::Workspace(rove_base::WorkspacePath::new(name).unwrap())
}

/// Analyses wait for `release`; every request and close is recorded.
struct Gated {
    calls: Mutex<Vec<(DocumentId, Option<LineRange>, Cancellation)>>,
    closed: Mutex<Vec<DocumentId>>,
    gate: async_channel::Receiver<()>,
    /// Report this version instead of the analysed one (a result for another version).
    version: Option<Version>,
}

impl DocumentHighlighter for Gated {
    fn analyze(
        &self,
        document: DocumentId,
        text: Arc<dyn TextSource>,
        _: Arc<str>,
        _: Option<TextEdit>,
        visible: Option<LineRange>,
        cancel: Cancellation,
    ) -> Task<SyntaxUpdate> {
        self.calls
            .lock()
            .unwrap()
            .push((document, visible, cancel.clone()));
        let gate = self.gate.clone();
        let version = self.version.unwrap_or(text.version());
        Box::pin(async move {
            let _ = gate.recv().await;
            cancel.check()?;
            Ok(SyntaxUpdate {
                snapshot: Arc::new(EmptySyntax(version)),
                previous: None,
                changed_lines: None,
            })
        })
    }

    fn close(&self, document: DocumentId) {
        self.closed.lock().unwrap().push(document);
    }
}

fn gated(version: Option<Version>) -> (Arc<Gated>, async_channel::Sender<()>) {
    let (send, gate) = async_channel::unbounded();
    (
        Arc::new(Gated {
            calls: Mutex::default(),
            closed: Mutex::default(),
            gate,
            version,
        }),
        send,
    )
}

fn large_csharp(folder: &std::path::Path, name: &str) {
    let member = "    int Method() { return 1; }\n";
    let body = member.repeat((syntax_limit() as usize) / member.len() + 1);
    std::fs::write(folder.join(name), format!("class Large {{\n{body}}}\n")).unwrap();
}

fn syntax_limit() -> u64 {
    crate::editors::syntax::INLINE_BYTES
}

fn open(
    view: &Entity<Workbench>,
    cx: &mut gpui::VisualTestContext,
    name: &str,
    target: OpenTarget,
) {
    cx.update(|_, app| {
        view.update(app, |v, cx| {
            v.open_documents()
                .open(
                    resource(name),
                    OpenOptions {
                        target,
                        preview_tab: false,
                        ..Default::default()
                    },
                    Cancellation::default(),
                )
                .unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
}

fn current(view: &Entity<Workbench>, cx: &mut gpui::VisualTestContext) -> crate::editors::Opened {
    view.read_with(cx, |v, _| {
        v.editor
            .active()
            .current()
            .and_then(Tab::opened)
            .cloned()
            .expect("an opened tab")
    })
}

#[gpui::test]
fn large_files_are_readable_first_and_highlighted_in_the_background(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    large_csharp(folder.path(), "Large.cs");
    std::fs::write(folder.path().join("Small.cs"), "class Small {}\n").unwrap();
    let (view, cx) =
        cx.add_window_view(|_, cx| Workbench::new(folder.path().into(), Instant::now(), cx));
    cx.run_until_parked();
    let (fake, release) = gated(None);
    view.update(cx, |v, _| v.editor.set_syntax(fake.clone()));

    open(&view, cx, "Large.cs", OpenTarget::Active);
    // The text is shown while the analysis is still waiting.
    let opened = current(&view, cx);
    assert!(opened.syntax.is_none());
    view.read_with(cx, |v, cx| {
        assert!(v.editor.highlighting(opened.document.id));
        assert!(v.code_view().read(cx).text().is_some());
    });
    {
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, opened.document.id);
        let visible = calls[0].1.clone().expect("visible lines first");
        assert_eq!(visible.start(), rove_base::Line(0));
    }

    // Visible-first result, then the full pass, both applied to the tab.
    release.try_send(()).unwrap();
    cx.run_until_parked();
    assert!(current(&view, cx).syntax.is_some());
    assert_eq!(fake.calls.lock().unwrap().len(), 2);
    assert!(fake.calls.lock().unwrap()[1].1.is_none());
    release.try_send(()).unwrap();
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(!v.editor.highlighting(opened.document.id))
    });

    // Small files are analysed before they are shown.
    let (fake, release) = gated(None);
    view.update(cx, |v, _| v.editor.set_syntax(fake.clone()));
    release.try_send(()).unwrap();
    open(&view, cx, "Small.cs", OpenTarget::Active);
    let small = current(&view, cx);
    assert!(small.syntax.is_some());
    assert_eq!(fake.calls.lock().unwrap()[0].1, None);
}

#[gpui::test]
fn results_for_another_version_or_a_closed_document_are_not_applied(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    large_csharp(folder.path(), "A.cs");
    large_csharp(folder.path(), "B.cs");
    let (view, cx) =
        cx.add_window_view(|_, cx| Workbench::new(folder.path().into(), Instant::now(), cx));
    cx.run_until_parked();

    // A result for a different version of the document is ignored.
    let (fake, release) = gated(Some(Version(u64::MAX)));
    view.update(cx, |v, _| v.editor.set_syntax(fake.clone()));
    open(&view, cx, "A.cs", OpenTarget::Active);
    release.try_send(()).unwrap();
    cx.run_until_parked();
    let a = current(&view, cx);
    assert!(a.syntax.is_none());

    // Closing the last tab of a document cancels its analysis and drops its baseline.
    let (fake, release) = gated(None);
    view.update(cx, |v, _| v.editor.set_syntax(fake.clone()));
    open(&view, cx, "B.cs", OpenTarget::Active);
    let b = current(&view, cx);
    let cancel = fake.calls.lock().unwrap()[0].2.clone();
    assert_eq!(cancel.check(), Ok(()));
    cx.update(|_, app| {
        view.update(app, |v, cx| {
            v.open_documents().close(b.document.id, None).unwrap();
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(cancel.check(), Err(Error::Cancelled));
    assert_eq!(fake.closed.lock().unwrap().as_slice(), &[b.document.id]);
    release.try_send(()).unwrap();
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert!(!v.editor.highlighting(b.document.id)));

    // Switching quickly: opening a file in a second group keeps A's tab, and the
    // background pass for A is not restarted when A is shown again.
    let calls = fake.calls.lock().unwrap().len();
    open(&view, cx, "B.cs", OpenTarget::Beside);
    open(&view, cx, "A.cs", OpenTarget::Active);
    assert_eq!(
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .skip(calls)
            .filter(|call| call.0 == a.document.id)
            .count(),
        1,
        "A is analysed once"
    );
}

#[test]
fn file_associations_apply_on_the_open_path() {
    use rove_base::{Events, NoEvents, Value};
    use rove_config::{Scope, SettingsChange};

    struct Associations;
    impl Settings for Associations {
        fn read(&self, key: &str, _: &Scope) -> CoreResult<Value> {
            match key {
                "files.associations" => Ok(Value::Object(Arc::new(
                    [("*.asset".into(), Value::String("json".into()))]
                        .into_iter()
                        .collect(),
                ))),
                _ => crate::app_settings::defaults_only().read(key, &Scope::default()),
            }
        }
        fn write(&self, _: Arc<str>, _: Scope, _: Value, _: Cancellation) -> Task<()> {
            Box::pin(async { Err(Error::Unsupported) })
        }
        fn changes(&self) -> &dyn Events<SettingsChange> {
            &NoEvents
        }
    }

    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("Data.asset"), "{\"a\": 1}\n").unwrap();
    std::fs::write(folder.path().join("Scene.unity"), "a: 1\n").unwrap();
    let documents = FileDocuments::new(folder.path().into(), Arc::new(Associations));
    let open = |name: &str| {
        futures_lite::future::block_on(documents.open(resource(name), Cancellation::default()))
            .unwrap()
    };
    assert_eq!(&*open("Data.asset").metadata.language, "json");
    assert_eq!(&*open("Scene.unity").metadata.language, "yaml");
}

#[test]
fn token_colors_use_the_most_specific_matching_selector() {
    use rove_base::{Events, NoEvents, Value};
    use rove_config::{Scope, SettingsChange};
    use rove_syntax::{highlight::TreeSitterSyntax, SyntaxService};

    struct Rules;
    impl Settings for Rules {
        fn read(&self, key: &str, scope: &Scope) -> CoreResult<Value> {
            if key != "editor.tokenColorCustomizations" {
                return crate::app_settings::defaults_only().read(key, scope);
            }
            // Least specific last: the result must not depend on rule order.
            let rule = |scope: &str, color: &str| {
                let settings = [("foreground".to_string(), Value::String(color.into()))];
                Value::Object(Arc::new(
                    [
                        ("scope".to_string(), Value::String(scope.into())),
                        (
                            "settings".to_string(),
                            Value::Object(Arc::new(settings.into_iter().collect())),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ))
            };
            let rules = [
                rule("keyword.control.conditional.if", "#0000ff"),
                rule("storage.type", "#445566"),
                rule("keyword.control", "#00ff00"),
                rule("storage", "#112233"),
                rule("keyword", "#ff0000"),
            ];
            Ok(Value::Object(Arc::new(
                [(
                    "textMateRules".to_string(),
                    Value::Array(rules.into_iter().collect()),
                )]
                .into_iter()
                .collect(),
            )))
        }
        fn write(&self, _: Arc<str>, _: Scope, _: Value, _: Cancellation) -> Task<()> {
            Box::pin(async { Err(Error::Unsupported) })
        }
        fn changes(&self) -> &dyn Events<SettingsChange> {
            &NoEvents
        }
    }

    let source = "class C { void M(bool a) { if (a) { } else { } } }";
    let text: Arc<dyn TextSource> = Arc::new(rove_buffer::RopeText::from_text(source));
    let update = futures_lite::future::block_on(TreeSitterSyntax::default().analyze(
        text,
        "csharp".into(),
        Cancellation::default(),
    ))
    .unwrap();
    let theme = rove_theme::ThemeService::new(Arc::new(Rules));
    let tokens = update.snapshot.tokens(rove_base::Line(0)).unwrap();
    let color = |word: &str| {
        let token = tokens
            .iter()
            .find(|t| &source[t.range.start().0 as usize..t.range.end().0 as usize] == word)
            .unwrap_or_else(|| panic!("token {word}"));
        theme.token_style(&token.scopes).foreground
    };
    assert_eq!(color("if"), Some(rove_theme::Color(0, 0, 0xff, 0xff)));
    assert_eq!(color("else"), Some(rove_theme::Color(0, 0xff, 0, 0xff)));
    assert_eq!(color("class"), Some(rove_theme::Color(0x44, 0x55, 0x66, 0xff)));
}
