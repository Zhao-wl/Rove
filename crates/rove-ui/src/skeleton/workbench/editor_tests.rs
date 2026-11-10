use super::*;
use crate::documents::{EditorTabKind, EditorTabState, OpenEvent, OpenTarget};
use gpui::{Modifiers, TestAppContext, VisualTestContext};
use rove_base::{Position, TextRange, Utf8Column};
use std::sync::Mutex;

fn resource(name: &str) -> Resource {
    Resource::Workspace(WorkspacePath::new(name).unwrap())
}

fn pos(line: u64, column: u64) -> Position {
    Position {
        line: Line(line),
        column: Utf8Column(column),
    }
}

fn window(cx: &mut TestAppContext, root: PathBuf) -> (Entity<Workbench>, &mut VisualTestContext) {
    let (view, cx) = cx.add_window_view(|_, cx| Workbench::new(root, Instant::now(), cx));
    cx.update(|window, cx| {
        cx.bind_keys(editor_actions::bindings());
        window.focus(&view.read(cx).focus);
    });
    cx.run_until_parked();
    (view, cx)
}

#[gpui::test]
fn caller_reveals_utf8_selection_and_markdown_uses_source_for_locations(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("Readme.md"), "a你好b\n".repeat(300)).unwrap();
    let (view, cx) = window(cx, folder.path().into());
    cx.simulate_click(point(px(90.0), px(40.0)), Modifiers::default());
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(matches!(
            v.editor.active().current().unwrap().kind,
            EditorKind::Placeholder { ref id, .. } if id.as_ref() == crate::editors::settings::MARKDOWN
        ));
    });

    let wanted = TextRange::new(pos(185, 1), pos(185, 7)).unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let (request, _subscription) = cx.update(|_, app| {
        view.update(app, |v, cx| {
            let service = v.open_documents();
            let subscription = service.events().subscribe(Arc::new(move |event| {
                captured.lock().unwrap().push(event);
            }));
            let request = service
                .open(
                    Resource::File(folder.path().join("Readme.md")),
                    OpenOptions {
                        source: OpenSource::Search,
                        location: Some(pos(0, 0)),
                        selection: Some(wanted.clone()),
                        preview_tab: false,
                        ..Default::default()
                    },
                    Cancellation::default(),
                )
                .unwrap();
            cx.notify();
            (request, subscription)
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        let doc = v
            .editor
            .find(&resource("Readme.md"))
            .expect("the document is open");
        let tabs = v.editor.in_group(v.editor.focused);
        assert_eq!(tabs.len(), 2, "each presentation gets its own snapshot");
        assert!(matches!(&tabs[0].kind, EditorTabKind::Custom(_)));
        assert_eq!(tabs[1].kind, EditorTabKind::Text);
        assert_eq!(tab.kind, EditorKind::Text);
        assert_eq!(tab.view.selection.as_ref(), Some(&wanted));
        assert_eq!(tab.view.cursor, wanted.start());
        assert!(v
            .painted
            .iter()
            .any(|line| line.number == 185 && line.text == "a你好b"));
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [OpenEvent::Ready { request: got, group, document }] if *got == request && *group == v.editor.focused && *document == doc.id
        ));
    });
    cx.simulate_keystrokes("ctrl-pageup");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-pagedown");
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert_eq!(tab.view.selection.as_ref(), Some(&wanted));
        assert!(v.painted.iter().any(|line| line.number == 185));
    });
}

#[gpui::test]
fn open_preview_keep_split_pin_and_restore_use_the_real_window(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    for name in ["A.cs", "B.cs", "C.cs"] {
        std::fs::write(
            folder.path().join(name),
            "public class Value {}\n".repeat(300),
        )
        .unwrap();
    }
    let (view, cx) = window(cx, folder.path().into());
    let left = view.read_with(cx, |v, _| {
        assert!(v
            .command_registry()
            .get("workbench.action.splitEditor")
            .is_some());
        v.editor.focused
    });
    cx.simulate_click(point(px(90.0), px(40.0)), Modifiers::default());
    cx.run_until_parked();
    let discarded = view.read_with(cx, |v, _| {
        Arc::downgrade(&v.editor.find(&resource("A.cs")).unwrap())
    });
    cx.simulate_click(point(px(90.0), px(64.0)), Modifiers::default());
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.editor.active().tabs.len(), 1);
        assert!(v.editor.active().current().unwrap().preview);
        assert!(
            discarded.upgrade().is_none(),
            "replacing preview releases the only document owner"
        );
    });
    cx.simulate_keystrokes("ctrl-k enter");
    cx.simulate_click(point(px(90.0), px(88.0)), Modifiers::default());
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.editor.active().tabs.len(), 2);
        assert!(!v.editor.active().tabs[0].preview);
        assert!(v.editor.active().tabs[1].preview);
    });
    cx.simulate_keystrokes("ctrl-\\");
    cx.run_until_parked();
    let right = view.read_with(cx, |v, _| {
        assert_ne!(v.editor.focused, left);
        let right = v.editor.focused;
        let left_tabs = v.editor.in_group(left);
        let right_tabs = v.editor.in_group(right);
        let EditorTabState::Ready(a) = &left_tabs.last().unwrap().state else {
            panic!("left is not ready")
        };
        let EditorTabState::Ready(b) = &right_tabs.last().unwrap().state else {
            panic!("right is not ready")
        };
        let (a, b) = (&a.text, &b.text);
        assert!(Arc::ptr_eq(a, b), "both views own the same TextSource");
        right
    });
    cx.simulate_keystrokes("ctrl-k ctrl-m");
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(v.editor.maximized, Some(right)));
    cx.simulate_keystrokes("ctrl-k ctrl-m");
    cx.simulate_keystrokes("ctrl-1");
    cx.simulate_keystrokes("alt-1");
    cx.simulate_keystrokes("ctrl-k shift-enter");
    cx.simulate_keystrokes("ctrl-k ctrl-w");
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.editor.maximized, None);
        assert_eq!(v.editor.focused, left);
        assert_eq!(v.editor.active().tabs.len(), 1);
        let tab = v.editor.active().current().unwrap();
        assert_eq!(tab.resource, resource("B.cs"));
        assert!(tab.pinned);
        assert!(!tab.preview);
    });
}

#[gpui::test]
fn navigation_reopens_closed_tab_across_groups_and_missing_files_fail_in_place(
    cx: &mut TestAppContext,
) {
    let folder = tempfile::tempdir().unwrap();
    for name in ["A.cs", "B.cs", "C.cs"] {
        std::fs::write(
            folder.path().join(name),
            "public class Value {}\n".repeat(250),
        )
        .unwrap();
    }
    let (view, cx) = window(cx, folder.path().into());
    let open = |cx: &mut VisualTestContext, name, target, source, location| {
        cx.update(|_, app| {
            view.update(app, |v, cx| {
                v.open_documents()
                    .open(
                        resource(name),
                        OpenOptions {
                            source,
                            target,
                            location,
                            ..Default::default()
                        },
                        Cancellation::default(),
                    )
                    .unwrap();
                cx.notify();
            })
        });
        cx.run_until_parked();
    };
    open(
        cx,
        "A.cs",
        OpenTarget::Active,
        OpenSource::QuickOpen,
        Some(pos(71, 1)),
    );
    let left = view.read_with(cx, |v, _| v.editor.focused);
    open(
        cx,
        "B.cs",
        OpenTarget::Beside,
        OpenSource::CodeNavigation,
        Some(pos(80, 3)),
    );
    let right = view.read_with(cx, |v, _| {
        assert!(!v.editor.active().current().unwrap().preview);
        v.editor.focused
    });
    open(
        cx,
        "C.cs",
        OpenTarget::Group(right),
        OpenSource::Outline,
        Some(pos(110, 2)),
    );
    cx.update(|_, app| {
        view.update(app, |v, cx| {
            let b = v.editor.find(&resource("B.cs")).unwrap().id;
            assert_eq!(
                v.open_documents().close(b, Some(right)).unwrap(),
                crate::documents::CloseOutcome::Closed
            );
            cx.notify();
        })
    });
    cx.simulate_keystrokes("alt-left");
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert_eq!(v.editor.focused, right);
        assert_eq!(tab.resource, resource("B.cs"));
        assert_eq!(tab.view.cursor, pos(80, 3));
    });
    cx.simulate_keystrokes("alt-left");
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.editor.focused, left);
        assert_eq!(v.editor.active().current().unwrap().view.cursor, pos(71, 1));
    });
    cx.simulate_keystrokes("alt-right");
    cx.run_until_parked();
    view.read_with(cx, |v, _| assert_eq!(v.editor.focused, right));
    open(
        cx,
        "Missing.cs",
        OpenTarget::Active,
        OpenSource::Explorer,
        None,
    );
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert_eq!(tab.resource, resource("Missing.cs"));
        assert!(matches!(
            tab.content,
            Content::Failed(Error::Io {
                kind: std::io::ErrorKind::NotFound,
                ..
            })
        ));
    });
}

#[gpui::test]
fn superseded_opens_report_exactly_one_terminal_event(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    for name in ["A.cs", "B.cs"] {
        std::fs::write(folder.path().join(name), "public class Value {}\n").unwrap();
    }
    let (view, cx) = window(cx, folder.path().into());
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let parent = Cancellation::default();
    let (group, first, same_tab, latest, _subscription) = cx.update(|_, app| {
        view.update(app, |v, cx| {
            let group = v.editor.focused;
            let service = v.open_documents();
            let subscription = service.events().subscribe(Arc::new(move |event| {
                captured.lock().unwrap().push(event);
            }));
            let first = service
                .open(resource("A.cs"), OpenOptions::default(), parent.clone())
                .unwrap();
            let tab = service.in_group(group)[0].id;
            let same_tab = service
                .open(resource("A.cs"), OpenOptions::default(), parent.clone())
                .unwrap();
            assert_eq!(service.in_group(group)[0].id, tab);
            let latest = service
                .open(resource("B.cs"), OpenOptions::default(), parent.clone())
                .unwrap();
            assert_ne!(service.in_group(group)[0].id, tab);
            assert_eq!(parent.check(), Ok(()));
            cx.notify();
            (group, first, same_tab, latest, subscription)
        })
    });
    cx.run_until_parked();
    assert!(matches!(
        events.lock().unwrap().as_slice(),
        [
            OpenEvent::Failed { request: a, group: a_group, error: Error::Cancelled },
            OpenEvent::Failed { request: b, group: b_group, error: Error::Cancelled },
            OpenEvent::Ready { request: c, group: c_group, .. }
        ] if (*a, *b, *c) == (first, same_tab, latest) && [*a_group, *b_group, *c_group] == [group; 3]
    ));
    let beside = cx.update(|_, app| {
        view.update(app, |v, cx| {
            let request = v
                .open_documents()
                .open(
                    resource("B.cs"),
                    OpenOptions {
                        target: OpenTarget::Beside,
                        preserve_focus: true,
                        ..Default::default()
                    },
                    parent.clone(),
                )
                .unwrap();
            assert_eq!(v.editor.focused, group);
            assert_eq!(v.editor.groups.len(), 2);
            cx.notify();
            request
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(v.editor.focused, group);
        let events = events.lock().unwrap();
        let Some(OpenEvent::Ready {
            request,
            group: resolved,
            ..
        }) = events.last()
        else {
            panic!("beside request must report its resolved group")
        };
        assert_eq!(*request, beside);
        assert_ne!(*resolved, group);
        assert_eq!(v.editor.in_group(*resolved)[0].resource, resource("B.cs"));
    });
}
