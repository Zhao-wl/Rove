use super::*;
use crate::code_view::{ScrollPosition, Selection};
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
    cx.simulate_click(point(px(128.0), px(104.0)), Modifiers::default());
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
        assert!(matches!(
            events.lock().unwrap().as_slice(),
            [OpenEvent::Ready { request: got, group, document }] if *got == request && *group == v.editor.focused && *document == doc.id
        ));
    });
    assert_code_shows(cx, &view, &wanted, "a你好b");
    cx.simulate_keystrokes("ctrl-pageup");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-pagedown");
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        let tab = v.editor.active().current().unwrap();
        assert_eq!(tab.view.selection.as_ref(), Some(&wanted));
    });
    // Switching away and back restores the selection and keeps the target line on screen.
    assert_code_shows(cx, &view, &wanted, "a你好b");
}

/// The focused group's code view shows `selection` with its first line inside the viewport.
fn assert_code_shows(
    cx: &mut VisualTestContext,
    view: &Entity<Workbench>,
    selection: &TextRange,
    line_text: &str,
) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| {
        let s = c.selection();
        assert_eq!((s.start(), s.end()), (selection.start(), selection.end()));
        let line = selection.start().line.0;
        let viewport = c.viewport().expect("code view laid out");
        assert!(
            viewport.first_line <= line && line <= viewport.last_line,
            "line {line} outside {viewport:?}"
        );
        assert_eq!(c.painted_text(line).as_deref(), Some(line_text));
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
    cx.simulate_click(point(px(128.0), px(104.0)), Modifiers::default());
    cx.run_until_parked();
    let discarded = view.read_with(cx, |v, _| {
        Arc::downgrade(&v.editor.find(&resource("A.cs")).unwrap())
    });
    cx.simulate_click(point(px(128.0), px(128.0)), Modifiers::default());
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
    cx.simulate_click(point(px(128.0), px(152.0)), Modifiers::default());
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
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        let left_code = v.group_code(left, false).unwrap();
        let right_code = v.group_code(right, false).unwrap();
        assert_ne!(
            left_code.entity_id(),
            right_code.entity_id(),
            "each group owns its code view"
        );
        let (a, b) = (left_code.read(cx).text(), right_code.read(cx).text());
        assert!(
            Arc::ptr_eq(a.unwrap(), b.unwrap()),
            "both code views point at the one TextSource"
        );
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

#[gpui::test]
fn unseen_alias_of_an_open_file_reuses_its_tab(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join("sub")).unwrap();
    std::fs::write(folder.path().join("A.cs"), "public class A {}\n".repeat(50)).unwrap();
    let (view, cx) = window(cx, folder.path().into());
    // Same file, but a spelling the loader has never seen: only canonicalization
    // resolves it, which happens after the open request was accepted.
    let alias = Resource::File(folder.path().join("sub").join("..").join("A.cs"));
    for (target, options) in [
        (
            resource("A.cs"),
            OpenOptions {
                preview_tab: false,
                ..Default::default()
            },
        ),
        (
            alias.clone(),
            OpenOptions {
                source: OpenSource::Search,
                selection: Some(TextRange::new(pos(30, 7), pos(30, 12)).unwrap()),
                ..Default::default()
            },
        ),
    ] {
        cx.update(|_, app| {
            view.update(app, |v, cx| {
                v.open_documents()
                    .open(target, options, Cancellation::default())
                    .unwrap();
                cx.notify();
            })
        });
        cx.run_until_parked();
    }
    view.read_with(cx, |v, _| {
        let group = v.editor.active();
        assert_eq!(group.tabs.len(), 1, "the alias must not add a second tab");
        let tab = group.current().unwrap();
        assert_eq!(tab.resource, resource("A.cs"));
        assert_eq!(tab.view.cursor, pos(30, 7));
        assert_eq!(
            tab.view.selection,
            Some(TextRange::new(pos(30, 7), pos(30, 12)).unwrap())
        );
        assert!(v.editor.find(&alias).is_some());
    });
}

#[gpui::test]
fn switching_tabs_restores_each_tabs_scroll_and_selection_in_the_group_code_view(
    cx: &mut TestAppContext,
) {
    let folder = tempfile::tempdir().unwrap();
    for name in ["A.cs", "B.cs"] {
        std::fs::write(
            folder.path().join(name),
            "public class Value {}
"
            .repeat(400),
        )
        .unwrap();
    }
    let (view, cx) = window(cx, folder.path().into());
    for name in ["A.cs", "B.cs"] {
        cx.update(|_, app| {
            view.update(app, |v, cx| {
                v.open_documents()
                    .open(
                        resource(name),
                        OpenOptions {
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
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    let a_selection = Selection::new(pos(150, 7), pos(151, 5));
    let b_selection = Selection::caret(pos(20, 3));
    // B is active: give it its own state, then switch to A and give A a different one.
    code.update(cx, |c, cx| {
        c.set_scroll_position(ScrollPosition { top: 12.0, x: 0.0 }, false, cx);
        c.set_selection(b_selection, None, cx);
    });
    cx.simulate_keystrokes("ctrl-pageup");
    cx.run_until_parked();
    let same_view = view.read_with(cx, |v, _| v.code_view().clone());
    assert_eq!(
        same_view.entity_id(),
        code.entity_id(),
        "tabs of one group share its code view"
    );
    code.read_with(cx, |c, _| {
        assert_eq!(c.scroll_position().top, 0.0, "A starts at its own state");
    });
    code.update(cx, |c, cx| {
        c.set_scroll_position(ScrollPosition { top: 140.0, x: 0.0 }, false, cx);
        c.set_selection(a_selection, None, cx);
    });
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-pagedown");
    cx.run_until_parked();
    code.read_with(cx, |c, _| {
        assert_eq!(c.scroll_position().top, 12.0);
        assert_eq!(c.selection(), b_selection);
    });
    cx.simulate_keystrokes("ctrl-pageup");
    cx.run_until_parked();
    code.read_with(cx, |c, _| {
        assert_eq!(c.scroll_position().top, 140.0);
        assert_eq!(c.selection(), a_selection);
    });
    // Closing A and reopening it restores its state (workbench.editor.restoreViewState).
    cx.simulate_keystrokes("ctrl-w");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-shift-t");
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.editor.active().current().unwrap().resource,
            resource("A.cs")
        );
    });
    code.read_with(cx, |c, _| {
        assert_eq!(c.scroll_position().top, 140.0);
        assert_eq!(c.selection(), a_selection);
    });
}

fn redraw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        cx.refresh_windows();
    });
    cx.run_until_parked();
}

fn open_now(cx: &mut VisualTestContext, view: &Entity<Workbench>, name: &str) -> TabId {
    cx.update(|_, app| {
        view.update(app, |v, cx| {
            v.open_documents()
                .open(
                    resource(name),
                    OpenOptions {
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
    view.read_with(cx, |v, _| v.editor.active().active.unwrap())
}

/// #132：已经打开的文件再打开（带定位）不再走一次后台任务——`open` 返回时标签已经就绪、
/// 插入符已经在目标位置；读盘的那一次仍然是异步的。
#[gpui::test]
fn opening_an_already_open_document_locates_synchronously(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("A.cs"),
        "public class V { }\n".repeat(400),
    )
    .unwrap();
    std::fs::write(folder.path().join("B.cs"), "public class W { }\n").unwrap();
    let (view, cx) = window(cx, folder.path().into());
    open_now(cx, &view, "A.cs");
    let b = open_now(cx, &view, "B.cs");
    // 另一个标签是活动标签：再打开 A.cs 并定位到第 200 行。
    let at = pos(200, 13);
    view.update(cx, |v, cx| {
        assert_eq!(v.editor.active().active, Some(b));
        v.open_documents()
            .open(
                resource("A.cs"),
                OpenOptions {
                    preview_tab: false,
                    location: Some(at),
                    ..Default::default()
                },
                Cancellation::default(),
            )
            .unwrap();
        // 还没有 run_until_parked：没有后台任务，标签已经就绪并带着定位。
        let tab = v.editor.active().current().unwrap();
        assert!(matches!(tab.content, Content::Ready(_)), "同步就绪");
        assert_eq!(tab.view.cursor, at);
        assert!(tab.reveal, "等代码视图把目标行置入视口");
        assert!(!tab.show_progress());
        cx.notify();
    });
    cx.run_until_parked();
    redraw(cx);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.code_view().read(cx).selection().active, at);
        let tabs = v.editor.in_group(v.editor.focused);
        assert_eq!(tabs.len(), 2, "没有多出标签");
        assert!(tabs.iter().any(|t| t.active
            && matches!(&t.state, EditorTabState::Ready(_))
            && t.resource == resource("A.cs")));
    });
    // 没打开过的文件仍然走后台加载（读盘不能在 UI 线程上做）。
    std::fs::write(folder.path().join("C.cs"), "public class X { }\n").unwrap();
    view.update(cx, |v, cx| {
        v.open_documents()
            .open(
                resource("C.cs"),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        assert!(matches!(
            v.editor.active().current().unwrap().content,
            Content::Loading
        ));
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(matches!(
            v.editor.active().current().unwrap().content,
            Content::Ready(_)
        ));
    });
}

#[gpui::test]
fn alias_completing_after_the_user_moved_on_does_not_steal_focus(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join("sub")).unwrap();
    for name in ["A.cs", "B.cs"] {
        std::fs::write(folder.path().join(name), "public class V {}\n".repeat(40)).unwrap();
    }
    let (view, cx) = window(cx, folder.path().into());
    let a = open_now(cx, &view, "A.cs");
    let b = open_now(cx, &view, "B.cs");
    cx.simulate_keystrokes("ctrl-\\");
    cx.run_until_parked();
    let (left, right) = view.read_with(cx, |v, _| {
        let right = v.editor.focused;
        let left = *v.editor.groups.keys().find(|g| **g != right).unwrap();
        (left, right)
    });
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let alias = Resource::File(folder.path().join("sub").join("..").join("A.cs"));
    let (request, _subscription) = cx.update(|_, app| {
        view.update(app, |v, cx| {
            let subscription = v.editor.events().subscribe(Arc::new(move |event| {
                captured.lock().unwrap().push(event);
            }));
            let request = v
                .open_documents()
                .open(
                    alias,
                    OpenOptions {
                        target: OpenTarget::Group(left),
                        source: OpenSource::Search,
                        location: Some(pos(30, 2)),
                        ..Default::default()
                    },
                    Cancellation::default(),
                )
                .unwrap();
            // While the alias loads, the user goes back to B and into the other group.
            v.editor.activate(left, b, false);
            v.editor.focus(right);
            cx.notify();
            (request, subscription)
        })
    });
    cx.run_until_parked();
    redraw(cx);
    view.read_with(cx, |v, _| {
        let pane = &v.editor.groups[&left];
        assert_eq!(pane.tabs.len(), 2, "the alias folds into A's tab");
        assert_eq!(
            pane.active,
            Some(b),
            "the completed alias must not reactivate A"
        );
        assert_eq!(v.editor.focused, right, "nor pull focus back to its group");
        assert_eq!(pane.tab(a).unwrap().view.cursor, pos(30, 2));
    });
    assert!(
        events.lock().unwrap().is_empty(),
        "A is in the background, so its location is not in a code view yet"
    );
    cx.update(|_, app| {
        view.update(app, |v, cx| {
            v.switch_tab(left, a);
            cx.notify();
        })
    });
    redraw(cx);
    assert!(matches!(
        events.lock().unwrap().as_slice(),
        [OpenEvent::Ready { request: got, group, .. }] if *got == request && *group == left
    ));
    let code = view.read_with(cx, |v, _| v.group_code(left, false).unwrap().clone());
    code.read_with(cx, |c, _| {
        assert_eq!(c.selection(), Selection::caret(pos(30, 2)));
        let viewport = c.viewport().unwrap();
        assert!(viewport.first_line <= 30 && 30 <= viewport.last_line);
    });
}

#[gpui::test]
fn ready_for_a_background_tab_waits_until_its_code_view_shows_the_location(
    cx: &mut TestAppContext,
) {
    let folder = tempfile::tempdir().unwrap();
    for name in ["A.cs", "B.cs"] {
        std::fs::write(folder.path().join(name), "public class V {}\n".repeat(400)).unwrap();
    }
    let (view, cx) = window(cx, folder.path().into());
    let b = open_now(cx, &view, "B.cs");
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let (request, group, _subscription) = cx.update(|_, app| {
        view.update(app, |v, cx| {
            let group = v.editor.focused;
            let subscription = v.editor.events().subscribe(Arc::new(move |event| {
                captured.lock().unwrap().push(event);
            }));
            let request = v
                .open_documents()
                .open(
                    resource("A.cs"),
                    OpenOptions {
                        source: OpenSource::GoToLine,
                        location: Some(pos(300, 4)),
                        ..Default::default()
                    },
                    Cancellation::default(),
                )
                .unwrap();
            // The user switches back to B before A finishes loading.
            v.editor.activate(group, b, true);
            cx.notify();
            (request, group, subscription)
        })
    });
    cx.run_until_parked();
    redraw(cx);
    let a = view.read_with(cx, |v, _| {
        let pane = &v.editor.groups[&group];
        assert_eq!(pane.active, Some(b));
        let a = pane
            .tabs
            .iter()
            .find(|t| t.resource == resource("A.cs"))
            .unwrap();
        assert!(matches!(a.content, Content::Ready(_)));
        a.id
    });
    assert!(
        events.lock().unwrap().is_empty(),
        "Ready waits while A is not in the group's code view"
    );
    cx.simulate_keystrokes("ctrl-pageup");
    redraw(cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.editor.groups[&group].active, Some(a))
    });
    assert!(matches!(
        events.lock().unwrap().as_slice(),
        [OpenEvent::Ready { request: got, group: g, .. }] if *got == request && *g == group
    ));
    let code = view.read_with(cx, |v, _| v.code_view().clone());
    code.read_with(cx, |c, _| {
        assert_eq!(c.selection(), Selection::caret(pos(300, 4)));
        let viewport = c.viewport().unwrap();
        assert!(
            viewport.first_line <= 300 && 300 <= viewport.last_line,
            "{viewport:?}"
        );
    });
}
