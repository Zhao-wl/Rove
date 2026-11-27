//! #62：替换命令经过真实工作台和搜索视图。所有写入发生在测试工作区副本。
use super::*;
use crate::search::registry as commands;
use gpui::{TestAppContext, VisualTestContext};
use rove_config::{SettingsOptions, SettingsService};

struct Fixture {
    folder: tempfile::TempDir,
    user: tempfile::TempDir,
}

impl Fixture {
    fn new(settings: &str) -> Self {
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("settings.json"), settings).unwrap();
        Self {
            folder: tempfile::tempdir().unwrap(),
            user,
        }
    }

    fn file(&self, rel: &str, contents: &[u8]) {
        let path = self.folder.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn read(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.folder.path().join(rel)).unwrap()
    }

    fn open<'a>(
        &self,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        self.open_at(self.folder.path(), cx)
    }

    fn open_at<'a>(
        &self,
        root: &std::path::Path,
        cx: &'a mut TestAppContext,
    ) -> (Entity<Workbench>, &'a mut VisualTestContext) {
        let mut options = SettingsOptions::new(self.user.path(), self.user.path().join("backup"));
        options.watch = None;
        let settings = Arc::new(SettingsService::open(
            options,
            crate::app_settings::registry().unwrap(),
        ));
        let (view, cx) = cx.add_window_view(|_, cx| {
            Workbench::with_settings(
                root.to_owned(),
                Instant::now(),
                LayoutStore::disabled(),
                settings,
                cx,
            )
            .unwrap()
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.focus(&view.read(cx).focus));
        (view, cx)
    }
}

#[track_caller]
fn wait(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    ready: impl Fn(&Workbench, &App) -> bool,
) {
    wait_for(view, cx, Duration::from_secs(20), ready);
}

#[track_caller]
fn wait_for(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    timeout: Duration,
    ready: impl Fn(&Workbench, &App) -> bool,
) {
    assert!(reaches(view, cx, timeout, ready), "等待全局替换条件超时");
}

fn reaches(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    timeout: Duration,
    ready: impl Fn(&Workbench, &App) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() <= deadline {
        cx.run_until_parked();
        if view.read_with(cx, |wb, cx| ready(wb, cx)) {
            return true;
        }
        cx.executor().advance_clock(Duration::from_millis(25));
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

fn prepare(view: &Entity<Workbench>, cx: &mut VisualTestContext, query: &str, replacement: &str) {
    prepare_for(view, cx, query, replacement, Duration::from_secs(20));
}

fn prepare_for(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    query: &str,
    replacement: &str,
    timeout: Duration,
) {
    start_search(view, cx, query, replacement);
    wait_for(view, cx, timeout, |wb, cx| {
        wb.search.read(cx).replacement_test_state().1
    });
}

fn start_search(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    query: &str,
    replacement: &str,
) {
    cx.simulate_keystrokes("ctrl-shift-h");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let search = view.read(cx).search.clone();
        assert!(search.read(cx).replacement_test_state().0);
        assert!(search
            .read(cx)
            .replace_input
            .focus_handle(cx)
            .is_focused(window));
        search.update(cx, |search, cx| {
            search
                .replace_input
                .update(cx, |input, cx| input.set_text(replacement, cx));
            search.set_query(query, cx);
        });
        window.refresh();
    });
}

fn command(view: &Entity<Workbench>, cx: &mut VisualTestContext, id: &'static str) {
    cx.update(|window, cx| {
        view.update(cx, |wb, cx| {
            wb.search_command(&commands::SearchAction(id), window, cx)
        });
    });
    cx.run_until_parked();
}

fn focus_file(view: &Entity<Workbench>, cx: &mut VisualTestContext, path: &WorkspacePath) {
    cx.update(|_, cx| {
        let search = view.read(cx).search.read(cx);
        let row = search.rows().file_row(path).unwrap();
        search.list.clone().update(cx, |list, cx| {
            list.widget.set_focus(Some(row));
            cx.notify();
        });
    });
}

fn open_file(
    fixture: &Fixture,
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    name: &str,
) -> DocumentId {
    open_file_at(fixture.folder.path(), view, cx, name)
}

fn open_file_at(
    root: &std::path::Path,
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    name: &str,
) -> DocumentId {
    let path = root.join(name);
    view.update(cx, |wb, cx| {
        wb.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new(name).unwrap()),
                OpenOptions {
                    preview_tab: false,
                    presentation: crate::documents::Presentation::Source,
                    ..OpenOptions::default()
                },
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    wait(view, cx, |wb, _| {
        wb.editing.saves.borrow().document_for(&path).is_some()
    });
    view.read_with(cx, |wb, _| {
        wb.editing.saves.borrow().document_for(&path).unwrap()
    })
}

fn change_setting(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    key: &str,
    value: rove_base::Value,
) {
    let scope = rove_config::Scope::default();
    view.update(cx, |wb, cx| {
        let task = wb.settings.write(
            key.into(),
            scope.clone(),
            value.clone(),
            Cancellation::default(),
        );
        cx.background_executor()
            .spawn(async move { task.await.unwrap() })
            .detach();
    });
    wait(view, cx, |wb, _| {
        wb.settings
            .read(key, &scope)
            .is_ok_and(|actual| actual == value)
    });
}

#[gpui::test]
fn replace_workspace_undo_restores_saved_closed_tabs_without_overwriting_external_edits(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new("{}");
    let original = b"\xef\xbb\xbfneedle\r\nkeep\n";
    fixture.file("restore.txt", original);
    fixture.file("external.txt", b"needle");
    let (view, cx) = fixture.open(cx);
    let restore = open_file(&fixture, &view, cx, "restore.txt");
    let external = open_file(&fixture, &view, cx, "external.txt");
    prepare(&view, cx, "needle", "REPLACED");
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().3
    });
    view.update(cx, |wb, cx| {
        for document in [restore, external] {
            wb.start_save(document, rove_buffer::save::SaveRequest::default(), cx);
        }
    });
    wait(&view, cx, |wb, _| {
        let saves = wb.editing.saves.borrow();
        [restore, external]
            .into_iter()
            .all(|id| saves.file(id).is_some_and(|file| !file.is_dirty()))
    });
    assert_eq!(
        fixture.read("restore.txt"),
        b"\xef\xbb\xbfREPLACED\r\nkeep\n"
    );
    assert_eq!(fixture.read("external.txt"), b"REPLACED");
    view.update(cx, |wb, cx| {
        for document in [restore, external] {
            let (group, tab) = wb
                .editor
                .groups
                .iter()
                .find_map(|(group, state)| {
                    state
                        .tabs
                        .iter()
                        .find(|tab| {
                            tab.opened()
                                .is_some_and(|open| open.document.id == document)
                        })
                        .map(|tab| (*group, tab.id))
                })
                .unwrap();
            wb.close_tab_checked(group, tab, cx);
        }
    });
    wait(&view, cx, |wb, _| {
        let saves = wb.editing.saves.borrow();
        [restore, external]
            .into_iter()
            .all(|id| saves.file(id).is_none())
    });
    fixture.file("external.txt", b"external edit");
    command(&view, cx, commands::UNDO_REPLACE);
    wait(&view, cx, |wb, cx| {
        wb.search
            .read(cx)
            .replacement_test_report()
            .is_some_and(|report| {
                report.undone && report.failed == [WorkspacePath::new("external.txt").unwrap()]
            })
    });
    assert_eq!(fixture.read("restore.txt"), original);
    assert_eq!(fixture.read("external.txt"), b"external edit");
}

#[gpui::test]
fn replace_workspace_rejects_changed_encoding_and_size_limit_after_preview(
    cx: &mut TestAppContext,
) {
    let fixture =
        Fixture::new(r#"{"files.encoding":"windows1252","files.autoGuessEncoding":false}"#);
    fixture.file("encoding.txt", &[0x80, b'A', b'B']);
    let mut large = vec![b' '; 256];
    large.push(b'A');
    fixture.file("size.txt", &large);
    let (view, cx) = fixture.open(cx);
    prepare(&view, cx, "A", "X");
    assert!(view.read_with(cx, |wb, cx| wb.search.read(cx).replacement_test_state().2));
    change_setting(
        &view,
        cx,
        "files.encoding",
        rove_base::Value::String("windows1251".into()),
    );
    change_setting(
        &view,
        cx,
        "rove.largeFile.threshold",
        rove_base::Value::Number(128.0),
    );
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search
            .read(cx)
            .replacement_test_report()
            .is_some_and(|report| {
                report.applied.is_empty()
                    && report.failed == [WorkspacePath::new("encoding.txt").unwrap()]
                    && report.skipped == [WorkspacePath::new("size.txt").unwrap()]
            })
    });
    assert_eq!(fixture.read("encoding.txt"), [0x80, b'A', b'B']);
    assert_eq!(fixture.read("size.txt"), large);
}

#[gpui::test]
fn replace_workspace_undo_rechecks_new_read_only_rule(cx: &mut TestAppContext) {
    let fixture = Fixture::new("{}");
    fixture.file("protected.txt", b"needle");
    let (view, cx) = fixture.open(cx);
    prepare(&view, cx, "needle", "REPLACED");
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().3
    });
    change_setting(
        &view,
        cx,
        rove_workspace::FILES_READONLY_INCLUDE,
        rove_base::Value::Object(Arc::new(BTreeMap::from([(
            "protected.txt".into(),
            rove_base::Value::Bool(true),
        )]))),
    );
    command(&view, cx, commands::UNDO_REPLACE);
    wait(&view, cx, |wb, cx| {
        wb.search
            .read(cx)
            .replacement_test_report()
            .is_some_and(|report| {
                !report.undone && report.failed == [WorkspacePath::new("protected.txt").unwrap()]
            })
    });
    assert_eq!(fixture.read("protected.txt"), b"REPLACED");
}

#[gpui::test]
fn replace_workspace_cancels_between_open_files_and_undoes_the_successful_prefix(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new("{}");
    fixture.file("a.txt", b"needle");
    fixture.file("b.txt", b"needle");
    let (view, cx) = fixture.open(cx);
    let first = open_file(&fixture, &view, cx, "a.txt");
    let second = open_file(&fixture, &view, cx, "b.txt");
    prepare(&view, cx, "needle", "REPLACED");
    let observation = view.update(cx, |_, cx| {
        cx.observe_self(move |wb, cx| {
            let first_changed = wb
                .editing
                .saves
                .borrow()
                .file(first)
                .is_some_and(|file| file.is_dirty());
            if first_changed {
                wb.search
                    .update(cx, |search, cx| search.cancel_replacement(cx));
            }
        })
    });
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search
            .read(cx)
            .replacement_test_report()
            .is_some_and(|report| report.cancelled && report.applied.len() == 1)
    });
    drop(observation);
    view.read_with(cx, |wb, _| {
        let saves = wb.editing.saves.borrow();
        assert_eq!(saves.file(first).unwrap().session().contents(), "REPLACED");
        assert_eq!(saves.file(second).unwrap().session().contents(), "needle");
    });
    command(&view, cx, commands::UNDO_REPLACE);
    wait(&view, cx, |wb, cx| {
        !wb.search.read(cx).replacement_test_state().3
    });
    view.read_with(cx, |wb, _| {
        let saves = wb.editing.saves.borrow();
        assert_eq!(saves.file(first).unwrap().session().contents(), "needle");
        assert_eq!(saves.file(second).unwrap().session().contents(), "needle");
    });
}

#[gpui::test]
fn replace_workspace_batch_undo_restores_bytes_and_keeps_open_buffer_dirty(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(r#"{"files.autoGuessEncoding": true}"#);
    let mut originals = BTreeMap::new();
    for i in 0..10 {
        let (path, data) = match i {
            0 => (
                "0.txt".to_string(),
                b"\xef\xbb\xbfneedle\r\nkeep\n".to_vec(),
            ),
            1 => {
                let (gbk, _, errors) =
                    encoding_rs::GBK.encode("全局替换的编码保留测试样本\r\nneedle\r\n保留原始中文");
                assert!(!errors);
                ("1.txt".to_string(), gbk.into_owned())
            }
            _ => (
                format!("{i}.txt"),
                format!("content\rneedle\nkeep {i}\r\n").into_bytes(),
            ),
        };
        fixture.file(&path, &data);
        originals.insert(path, data);
    }
    fixture.file("dirty.txt", b"needle\r\nkeep\n");
    fixture.file("omitted.txt", b"needle");
    let (view, cx) = fixture.open(cx);
    let path = fixture.folder.path().join("dirty.txt");
    view.update(cx, |wb, cx| {
        wb.open_documents()
            .open(
                Resource::Workspace(WorkspacePath::new("dirty.txt").unwrap()),
                OpenOptions::default(),
                Cancellation::default(),
            )
            .unwrap();
        cx.notify();
    });
    wait(&view, cx, |wb, _| {
        wb.editing.saves.borrow().document_for(&path).is_some()
    });
    let document = view.read_with(cx, |wb, _| {
        wb.editing.saves.borrow().document_for(&path).unwrap()
    });
    view.update(cx, |wb, cx| {
        {
            let mut saves = wb.editing.saves.borrow_mut();
            let file = saves.file_mut(document).unwrap();
            let offset = rove_base::ByteOffset(0);
            file.session_mut()
                .edit(
                    rove_buffer::edit::Transaction::single(
                        rove_base::ByteRange::new(offset, offset).unwrap(),
                        "unsaved\r\n",
                    ),
                    Instant::now(),
                )
                .unwrap();
        }
        wb.sync_document_from_session(document, cx);
    });
    prepare(&view, cx, "needle", "REPLACED");
    view.read_with(cx, |wb, cx| {
        let search = wb.search.read(cx);
        assert!(search.replacement_test_state().2);
        for file in originals.keys() {
            let path = WorkspacePath::new(file).unwrap();
            assert_eq!(
                search.replacement_test_comparison(&path),
                Some(("needle".to_owned(), "REPLACED".to_owned())),
                "{file}"
            );
        }
    });
    let omitted = WorkspacePath::new("omitted.txt").unwrap();
    focus_file(&view, cx, &omitted);
    command(&view, cx, commands::REMOVE);
    wait(&view, cx, |wb, cx| {
        let search = wb.search.read(cx);
        search.results().file(&omitted).is_none() && search.replacement_test_state().1
    });
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().3
    });
    for (path, before) in &originals {
        let index = before
            .windows(6)
            .position(|word| word == b"needle")
            .unwrap();
        let expected = [&before[..index], b"REPLACED", &before[index + 6..]].concat();
        assert_eq!(fixture.read(path), expected, "{path}");
    }
    assert_eq!(fixture.read("dirty.txt"), b"needle\r\nkeep\n");
    assert_eq!(fixture.read("omitted.txt"), b"needle");
    view.read_with(cx, |wb, _| {
        let saves = wb.editing.saves.borrow();
        let file = saves.file(document).unwrap();
        assert!(file.is_dirty());
        assert_eq!(file.session().contents(), "unsaved\r\nREPLACED\r\nkeep\n");
    });
    command(&view, cx, commands::UNDO_REPLACE);
    wait(&view, cx, |wb, cx| {
        !wb.search.read(cx).replacement_test_state().3
    });
    for (path, before) in &originals {
        assert_eq!(&fixture.read(path), before, "{path}");
    }
    view.read_with(cx, |wb, _| {
        let saves = wb.editing.saves.borrow();
        let file = saves.file(document).unwrap();
        assert!(file.is_dirty());
        assert_eq!(file.session().contents(), "unsaved\r\nneedle\r\nkeep\n");
    });
}

#[gpui::test]
fn replace_workspace_selection_file_folder_and_cancel_widget(cx: &mut TestAppContext) {
    let fixture = Fixture::new(r#"{"search.useReplacePreview": false}"#);
    fixture.file("folder/first.txt", b"needle needle needle");
    fixture.file("folder/second.txt", b"needle");
    fixture.file("other.txt", b"needle");
    let (view, cx) = fixture.open(cx);
    prepare(&view, cx, "needle", "NEW");
    assert!(!view.read_with(cx, |wb, cx| wb.search.read(cx).replacement_test_state().2));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(view.read_with(cx, |wb, cx| {
        let search = wb.search.read(cx);
        !search.replacement_test_state().0 && search.results().match_count() == 5
    }));
    command(&view, cx, commands::REPLACE_IN_FILES);
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().1
    });
    let set_focus = |view: &Entity<Workbench>, cx: &mut VisualTestContext, offset: usize| {
        cx.update(|_, cx| {
            let search = view.read(cx).search.clone();
            let search = search.read(cx);
            let path = WorkspacePath::new("folder/first.txt").unwrap();
            let index = search.rows().file_row(&path).unwrap() + offset;
            search.list.clone().update(cx, |list, cx| {
                list.widget.set_focus(Some(index));
                cx.notify();
            });
        });
    };
    set_focus(&view, cx, 3);
    command(&view, cx, commands::REMOVE);
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().1
    });
    set_focus(&view, cx, 1);
    command(&view, cx, commands::REPLACE_MATCH);
    wait(&view, cx, |_, _| {
        fixture.read("folder/first.txt") == b"NEW needle needle"
    });
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().1
    });
    set_focus(&view, cx, 0);
    command(&view, cx, commands::REPLACE_FILE);
    wait(&view, cx, |_, _| {
        fixture.read("folder/first.txt") == b"NEW NEW needle"
    });
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().1
    });
    cx.update(|_, cx| {
        let search = view.read(cx).search.clone();
        let search = search.read(cx);
        let path = WorkspacePath::new("folder/second.txt").unwrap();
        let index = search.rows().file_row(&path).unwrap();
        search.list.clone().update(cx, |list, cx| {
            list.widget.set_focus(Some(index));
            cx.notify();
        });
    });
    command(&view, cx, commands::REPLACE_FOLDER);
    wait(&view, cx, |_, _| {
        fixture.read("folder/second.txt") == b"NEW"
    });
    assert_eq!(fixture.read("other.txt"), b"needle");
}

fn outline_has(wb: &Workbench, cx: &App, label: &str) -> bool {
    let widget = &wb.outline.list.read(cx).widget;
    widget
        .display_rows(0..widget.len())
        .iter()
        .any(|row| row.label.as_ref() == label)
}

fn structure_is_current(wb: &Workbench, document: DocumentId) -> bool {
    use rove_buffer::TextSource;
    use rove_docview::Structure;

    let saves = wb.editing.saves.borrow();
    let version = saves.file(document).unwrap().snapshot().version();
    wb.structured.documents.get(&document).is_some_and(|entry| {
        entry.snapshot.version() == version && !wb.structured.diagnostics.contains_key(&document)
    })
}

fn has_structure_problem(wb: &Workbench, cx: &App, resource: &Resource) -> bool {
    wb.listed_problem_positions(cx)
        .iter()
        .any(|(path, _)| path == resource)
}

#[gpui::test]
fn replace_workspace_application_and_undo_refresh_structure_and_problems(cx: &mut TestAppContext) {
    let fixture = Fixture::new("{}");
    fixture.file("config.json", br#"{"oldName": true, "enabled": false}"#);
    let (view, cx) = fixture.open(cx);
    let document = open_file(&fixture, &view, cx, "config.json");
    let resource = Resource::Workspace(WorkspacePath::new("config.json").unwrap());
    wait(&view, cx, |wb, cx| {
        structure_is_current(wb, document)
            && outline_has(wb, cx, "oldName")
            && !has_structure_problem(wb, cx, &resource)
    });
    view.update(cx, |wb, cx| {
        wb.search
            .update(cx, |search, _| search.toggles.regex = true);
    });
    prepare(&view, cx, "oldName|false", "newName");
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().3
            && structure_is_current(wb, document)
            && outline_has(wb, cx, "newName")
            && has_structure_problem(wb, cx, &resource)
    });

    command(&view, cx, commands::UNDO_REPLACE);
    wait(&view, cx, |wb, cx| {
        !wb.search.read(cx).replacement_test_state().3
            && structure_is_current(wb, document)
            && outline_has(wb, cx, "oldName")
            && !has_structure_problem(wb, cx, &resource)
    });
}

#[gpui::test]
fn replace_workspace_refreshes_background_problems_without_switching_the_active_outline(
    cx: &mut TestAppContext,
) {
    use crate::explorer::decorations::DecorationProvider;

    let fixture = Fixture::new("{}");
    fixture.file("config.json", br#"{"enabled": invalid}"#);
    fixture.file("active.json", br#"{"untouched": 1}"#);
    let (view, cx) = fixture.open(cx);
    let document = open_file(&fixture, &view, cx, "config.json");
    let path = WorkspacePath::new("config.json").unwrap();
    let resource = Resource::Workspace(path.clone());
    let badge = |wb: &Workbench, cx: &App| {
        wb.explorer
            .read(cx)
            .problems()
            .decorate(&path, false)
            .is_some()
    };
    wait(&view, cx, |wb, cx| {
        structure_is_current(wb, document)
            && has_structure_problem(wb, cx, &resource)
            && badge(wb, cx)
    });
    let active = open_file(&fixture, &view, cx, "active.json");
    let active_outline = |wb: &Workbench, cx: &App| {
        wb.focused_text()
            .is_some_and(|(_, doc, _)| doc.id == active)
            && outline_has(wb, cx, "untouched")
    };
    wait(&view, cx, active_outline);
    prepare(&view, cx, "invalid", "false");
    let changed_limit = std::rc::Rc::new(std::cell::Cell::new(false));
    let mark = changed_limit.clone();
    let observation = view.update(cx, |_, cx| {
        cx.observe_self(move |wb, cx| {
            if !mark.get() && wb.structured.replacement_parses.contains_key(&document) {
                mark.set(true);
                let key: Arc<str> = "json.maxItemsComputed".into();
                futures_lite::future::block_on(wb.settings.write(
                    key.clone(),
                    rove_config::Scope::default(),
                    rove_base::Value::Number(10.0),
                    Cancellation::default(),
                ))
                .unwrap();
                // Deliver the setting at the exact boundary before the queued parser can run.
                wb.settings_changed(&BTreeSet::from([key]), cx);
            }
        })
    });
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().3
            && structure_is_current(wb, document)
            && !has_structure_problem(wb, cx, &resource)
            && !badge(wb, cx)
            && active_outline(wb, cx)
            && wb.structured.documents[&document].limit == 10
    });
    assert!(changed_limit.get(), "修改阈值前必须有后台解析在途");
    drop(observation);

    command(&view, cx, commands::UNDO_REPLACE);
    wait(&view, cx, |wb, cx| {
        !wb.search.read(cx).replacement_test_state().3
            && structure_is_current(wb, document)
            && has_structure_problem(wb, cx, &resource)
            && badge(wb, cx)
            && active_outline(wb, cx)
    });
}

#[gpui::test]
fn replace_workspace_application_and_undo_move_the_context_basket(cx: &mut TestAppContext) {
    use crate::context_basket::model::{Entry, LineRange};

    let fixture = Fixture::new("{}");
    fixture.file("tracked.txt", b"needle\ntracked line\n");
    let (view, cx) = fixture.open(cx);
    let document = open_file(&fixture, &view, cx, "tracked.txt");
    view.update(cx, |wb, cx| {
        wb.basket.use_test_state();
        wb.basket
            .contents
            .insert(Entry::new(
                WorkspacePath::new("tracked.txt").unwrap(),
                LineRange::new(2, 2).unwrap(),
                None,
            ))
            .unwrap();
        assert!(wb.basket_changed(cx));
        wb.search
            .update(cx, |search, _| search.toggles.regex = true);
    });
    prepare(&view, cx, "needle", r"NEW\nextra");
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().3
    });
    view.read_with(cx, |wb, _| {
        assert_eq!(
            wb.basket.contents.items()[0].lines,
            LineRange::new(3, 3).unwrap()
        );
        assert_eq!(
            wb.editing
                .saves
                .borrow()
                .file(document)
                .unwrap()
                .session()
                .contents(),
            "NEW\nextra\ntracked line\n",
        );
    });
    command(&view, cx, commands::UNDO_REPLACE);
    wait(&view, cx, |wb, cx| {
        !wb.search.read(cx).replacement_test_state().3
    });
    view.read_with(cx, |wb, _| {
        assert_eq!(
            wb.basket.contents.items()[0].lines,
            LineRange::new(2, 2).unwrap()
        );
        assert_eq!(
            wb.editing
                .saves
                .borrow()
                .file(document)
                .unwrap()
                .session()
                .contents(),
            "needle\ntracked line\n",
        );
    });
}

#[gpui::test]
fn replace_workspace_continues_partial_replacement_before_and_after_saving_an_open_file(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new("{}");
    fixture.file("opened.txt", b"needle needle needle needle");
    let (view, cx) = fixture.open(cx);
    let document = open_file(&fixture, &view, cx, "opened.txt");
    prepare(&view, cx, "needle", "NEW");
    let path = WorkspacePath::new("opened.txt").unwrap();
    let focus_match = |cx: &mut VisualTestContext, ordinal| {
        cx.update(|_, cx| {
            let search = view.read(cx).search.read(cx);
            let row = search.rows().file_row(&path).unwrap() + ordinal;
            search.list.clone().update(cx, |list, cx| {
                list.widget.set_focus(Some(row));
                cx.notify();
            });
        });
    };
    focus_match(cx, 4);
    command(&view, cx, commands::REMOVE);
    wait(&view, cx, |wb, cx| {
        let search = wb.search.read(cx);
        search.replacement_test_state().1 && search.results().file(&path).unwrap().count() == 3
    });
    for expected in ["NEW needle needle needle", "NEW NEW needle needle"] {
        focus_match(cx, 1);
        command(&view, cx, commands::REPLACE_MATCH);
        wait(&view, cx, |wb, cx| {
            wb.search.read(cx).replacement_test_state().1
                && wb
                    .editing
                    .saves
                    .borrow()
                    .file(document)
                    .unwrap()
                    .session()
                    .contents()
                    == expected
        });
    }
    assert_eq!(fixture.read("opened.txt"), b"needle needle needle needle");
    view.update(cx, |wb, cx| {
        wb.start_save(document, rove_buffer::save::SaveRequest::default(), cx);
    });
    wait(&view, cx, |wb, _| {
        !wb.editing.saves.borrow().is_dirty(document)
    });
    assert_eq!(fixture.read("opened.txt"), b"NEW NEW needle needle");
    focus_file(&view, cx, &path);
    command(&view, cx, commands::REPLACE_FILE);
    wait(&view, cx, |wb, _| {
        wb.editing
            .saves
            .borrow()
            .file(document)
            .unwrap()
            .session()
            .contents()
            == "NEW NEW NEW needle"
    });
}

#[cfg(windows)]
fn lock_in_child(first: &std::path::Path, second: &std::path::Path) -> std::process::Child {
    use std::{
        io::{BufRead, BufReader},
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = r#"
$ErrorActionPreference = 'Stop'
$first = [IO.File]::Open($env:ROVE_REPLACE_LOCK_FIRST, 'Open', 'Read', 'None')
try {
    $second = [IO.File]::Open($env:ROVE_REPLACE_LOCK_SECOND, 'Open', 'Read', 'None')
    try {
        [Console]::WriteLine('locked')
        [void][Console]::ReadLine()
    } finally {
        $second.Dispose()
    }
} finally {
    $first.Dispose()
}
"#;
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env("ROVE_REPLACE_LOCK_FIRST", first)
        .env("ROVE_REPLACE_LOCK_SECOND", second)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready.trim(), "locked");
    child
}

#[cfg(windows)]
#[gpui::test]
fn replace_workspace_skips_protected_and_keeps_locked_failure_in_results(cx: &mut TestAppContext) {
    let fixture = Fixture::new(r#"{"rove.largeFile.threshold": 128}"#);
    fixture.file("free.txt", b"needle");
    fixture.file("locked.txt", b"needle");
    fixture.file("locked-again.txt", b"needle");
    fixture.file("Library/PackageCache/package/read-only.txt", b"needle");
    let huge = [b"needle\n".as_slice(), &[b'a'; 200]].concat();
    fixture.file("huge.txt", &huge);
    let (view, cx) = fixture.open(cx);
    view.update(cx, |wb, cx| {
        wb.search
            .update(cx, |search, _| search.toggles.use_exclude_settings = false);
    });
    prepare(&view, cx, "needle", "NEW");
    let skipped = ["Library/PackageCache/package/read-only.txt", "huge.txt"]
        .map(|path| WorkspacePath::new(path).unwrap());
    for path in &skipped {
        focus_file(&view, cx, path);
        cx.update(|window, cx| {
            let search = view.read(cx).search.read(cx);
            assert!(!search.replacement_test_editable(window, cx));
            assert_eq!(
                search.replacement_test_comparison(path),
                Some(("needle".to_owned(), "NEW".to_owned())),
                "{path:?}"
            );
        });
    }
    let mut lock = lock_in_child(
        &fixture.folder.path().join("locked.txt"),
        &fixture.folder.path().join("locked-again.txt"),
    );
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search.read(cx).replacement_test_state().3
    });
    assert_eq!(fixture.read("free.txt"), b"NEW");
    drop(lock.stdin.take());
    assert!(lock.wait().unwrap().success());
    assert_eq!(fixture.read("locked.txt"), b"needle");
    assert_eq!(fixture.read("locked-again.txt"), b"needle");
    assert_eq!(
        fixture.read("Library/PackageCache/package/read-only.txt"),
        b"needle"
    );
    assert_eq!(fixture.read("huge.txt"), huge);
    view.read_with(cx, |wb, cx| {
        let search = wb.search.read(cx);
        let result = search.results();
        assert!(result
            .file(&WorkspacePath::new("locked.txt").unwrap())
            .is_some());
        assert!(result
            .file(&WorkspacePath::new("locked-again.txt").unwrap())
            .is_some());
        let report = search.replacement_test_report().unwrap();
        assert_eq!(report.skipped.len(), skipped.len());
        assert!(skipped.iter().all(|path| report.skipped.contains(path)));
        let notes = wb.shell.feedback.notifications();
        let mut busy = notes
            .iter()
            .filter(|notice| notice.key.as_ref() == "search.replace.busyFiles");
        let notice = busy.next().expect("应汇总报告被占用的文件");
        assert_eq!(notice.count, 1);
        assert!(notice.message.args.iter().any(|(key, value)| {
            key.as_ref() == "paths"
                && value.contains("locked.txt")
                && value.contains("locked-again.txt")
        }));
        assert!(busy.next().is_none());
        assert!(result
            .file(&WorkspacePath::new("Library/PackageCache/package/read-only.txt").unwrap())
            .is_some());
        assert!(result
            .file(&WorkspacePath::new("huge.txt").unwrap())
            .is_some());
        assert!(result
            .file(&WorkspacePath::new("free.txt").unwrap())
            .is_none());
    });
}

#[gpui::test]
fn replace_workspace_rechecks_read_only_rules_entered_after_preview(cx: &mut TestAppContext) {
    let fixture = Fixture::new("{}");
    fixture.file("newly-protected.txt", b"needle");
    fixture.file("still-writable.txt", b"needle");
    let (view, cx) = fixture.open(cx);
    prepare(&view, cx, "needle", "NEW");
    let path = WorkspacePath::new("newly-protected.txt").unwrap();
    let rule = rove_base::Value::Object(Arc::new(BTreeMap::from([(
        path.as_str().to_owned(),
        rove_base::Value::Bool(true),
    )])));
    let key = rove_workspace::FILES_READONLY_INCLUDE;
    let scope = rove_config::Scope::default();
    view.update(cx, |wb, cx| {
        let task = wb.settings.write(
            key.into(),
            scope.clone(),
            rule.clone(),
            Cancellation::default(),
        );
        cx.background_executor()
            .spawn(async move {
                task.await.unwrap();
            })
            .detach();
    });
    wait(&view, cx, |wb, _| {
        wb.settings
            .read(key, &scope)
            .is_ok_and(|value| value == rule)
    });
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search
            .read(cx)
            .replacement_test_report()
            .is_some_and(|report| {
                report.skipped.as_slice() == std::slice::from_ref(&path)
                    && report
                        .applied
                        .contains_key(&WorkspacePath::new("still-writable.txt").unwrap())
            })
    });
    assert_eq!(fixture.read("newly-protected.txt"), b"needle");
    assert_eq!(fixture.read("still-writable.txt"), b"NEW");
}

#[gpui::test]
fn replace_workspace_previews_multiline_captures_in_huge_read_only_file(cx: &mut TestAppContext) {
    let fixture = Fixture::new(r#"{"rove.largeFile.threshold": 128}"#);
    let huge = [b"needle\r\nsize\n".as_slice(), &[b'a'; 200]].concat();
    fixture.file("huge.txt", &huge);
    let (view, cx) = fixture.open(cx);
    view.update(cx, |wb, cx| {
        wb.search
            .update(cx, |search, _| search.toggles.regex = true);
    });
    prepare(&view, cx, r"(nee)(dle)\r?\n(size)", "$3 $2$1");
    let path = WorkspacePath::new("huge.txt").unwrap();
    view.read_with(cx, |wb, cx| {
        assert_eq!(
            wb.search.read(cx).replacement_test_comparison(&path),
            Some(("needle\r\nsize".to_owned(), "size dlenee".to_owned()))
        );
    });
    command(&view, cx, commands::REPLACE_ALL);
    wait(&view, cx, |wb, cx| {
        wb.search
            .read(cx)
            .replacement_test_report()
            .is_some_and(|report| {
                report.skipped.as_slice() == std::slice::from_ref(&path)
                    && report.applied.is_empty()
            })
    });
    assert_eq!(fixture.read("huge.txt"), huge);
}

#[gpui::test]
fn replace_workspace_refuses_all_when_search_results_are_truncated(cx: &mut TestAppContext) {
    let fixture = Fixture::new(r#"{"search.maxResults": 1}"#);
    fixture.file("a.txt", b"needle needle");
    let (view, cx) = fixture.open(cx);
    prepare(&view, cx, "needle", "NEW");
    assert!(view.read_with(cx, |wb, cx| matches!(
        wb.search.read(cx).state(),
        crate::search::view::State::Done(summary) if summary.limit_hit
    )));
    command(&view, cx, commands::REPLACE_ALL);
    assert_eq!(fixture.read("a.txt"), b"needle needle");
    assert!(!view.read_with(cx, |wb, cx| wb.search.read(cx).replacement_test_state().3));
}

fn scroll_replacement_results(view: &Entity<Workbench>, cx: &mut VisualTestContext) -> Duration {
    let scroll_top = |cx: &mut VisualTestContext| {
        view.read_with(cx, |wb, cx| {
            wb.search.read(cx).list.read(cx).widget.scroll_top()
        })
    };
    let initial = scroll_top(cx);
    let scroll_start = Instant::now();
    for _ in 0..5 {
        cx.update(|window, cx| {
            let list = view.read(cx).search.read(cx).list.clone();
            list.update(cx, |list, cx| {
                list.widget.scroll_by(320.0);
                cx.notify();
            });
            window.refresh();
            cx.refresh_windows();
        });
        cx.run_until_parked();
    }
    let scroll_elapsed = scroll_start.elapsed();
    assert!(scroll_top(cx) > initial, "五次滚动必须移动已经渲染的结果树");
    scroll_elapsed
}

fn prepare_full_unity_preview(
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
    label: &str,
    query: &str,
    replacement: &str,
) -> rove_search::SearchSummary {
    use crate::search::view::State;

    let timeout = Duration::from_secs(90);
    let search_start = Instant::now();
    start_search(view, cx, query, replacement);
    reaches(view, cx, timeout, |wb, cx| {
        matches!(
            wb.search.read(cx).state(),
            State::Done(_) | State::Failed | State::Cancelled | State::InvalidPattern
        )
    });
    let summary = view.read_with(cx, |wb, cx| {
        let search = wb.search.read(cx);
        let State::Done(summary) = search.state() else {
            let files = search.results().files();
            let matches = files.iter().map(|file| file.count()).sum::<usize>();
            panic!(
                "{label} 搜索未完成，耗时 {:?}，状态 {:?}，已收到 {} 个文件内 {matches} 处匹配",
                search_start.elapsed(),
                search.state(),
                files.len(),
            );
        };
        summary.clone()
    });
    eprintln!(
        "{label} 搜索完成：{:?}，{} 份文件，{} 个文件内 {} 处匹配，{} 跳过，截断={}",
        search_start.elapsed(),
        summary.files_searched,
        summary.files_with_matches,
        summary.matches,
        summary.skipped,
        summary.limit_hit,
    );
    let preview_start = Instant::now();
    let ready = reaches(view, cx, timeout, |wb, cx| {
        wb.search.read(cx).replacement_test_state().1
    });
    let preview_state = view.read_with(cx, |wb, cx| {
        wb.search.read(cx).replacement_test_preview_stage()
    });
    assert!(
        ready,
        "{label} 预览未就绪，搜索已完成，从确认搜索后等了 {:?}，预览状态 {preview_state}",
        preview_start.elapsed(),
    );
    eprintln!(
        "{label} 对照就绪：搜索完成后 {:?}，状态 {preview_state}",
        preview_start.elapsed(),
    );
    summary
}

#[gpui::test]
fn replace_workspace_large_unity_reads_the_original_workspace_for_preview(cx: &mut TestAppContext) {
    use rove_test_support::UnityProject;

    const READ_ONLY_PROBE: &str = "ROVE_TEST_REPLACE_UNITY_WORKSPACE";
    if std::env::var_os(READ_ONLY_PROBE).as_deref() != Some(std::ffi::OsStr::new("1")) {
        eprintln!("已跳过：在隔离用户目录的进程设置 {READ_ONLY_PROBE}=1 后运行原工作区只读预览");
        return;
    }
    let root = rove_test_support::require_unity_project!(UnityProject::Large);
    let fixture = Fixture::new(r#"{"search.maxResults":0,"files.readonlyInclude":{"**":true}}"#);
    let (view, cx) = fixture.open_at(&root, cx);
    let preview_start = Instant::now();
    let summary = prepare_full_unity_preview(
        &view,
        cx,
        "大型 Unity 原根只读",
        "GetComponent",
        "rove_read_only_preview",
    );
    let preview_elapsed = preview_start.elapsed();
    view.read_with(cx, |wb, cx| {
        let search = wb.search.read(cx);
        assert!(search.results().files().iter().any(|f| {
            search
                .replacement_test_comparison(&f.path)
                .is_some_and(|(old, new)| old == "GetComponent" && new == "rove_read_only_preview")
        }));
    });
    assert!(
        !summary.limit_hit,
        "原项目只读查询必须遍历全部符合范围的文件"
    );
    assert!(
        summary.files_searched > 1500 && summary.matches > 30,
        "{summary:?}"
    );
    let scroll_elapsed = scroll_replacement_results(&view, cx);
    eprintln!(
        "全局替换原大型 Unity 工作区只读预览：扫描 {} 文件，{} 个文件内 {} 处匹配，{} 跳过，截断={}；预览 {:?}，五次滚动 {:?}",
        summary.files_searched,
        summary.files_with_matches,
        summary.matches,
        summary.skipped,
        summary.limit_hit,
        preview_elapsed,
        scroll_elapsed,
    );
}

fn cancel_large_replacement(
    root: &std::path::Path,
    view: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> (Duration, WorkspacePath) {
    let first = view.read_with(cx, |wb, cx| {
        wb.search
            .read(cx)
            .replacement_test_writable_files()
            .into_iter()
            .next()
            .expect("预览必须产生至少一个可应用的文件")
    });
    let document = open_file_at(root, view, cx, first.as_str());
    command(view, cx, commands::REPLACE_IN_FILES);
    wait_for(view, cx, Duration::from_secs(90), |wb, cx| {
        wb.search.read(cx).replacement_test_state().1
    });
    let first_remains_writable = view.read_with(cx, |wb, cx| {
        wb.search.read(cx).replacement_test_writable_files().first() == Some(&first)
    });
    assert!(
        first_remains_writable,
        "打开文件后实际应用队列的首个可写文件必须一致"
    );
    let observation = view.update(cx, |_, cx| {
        cx.observe_self(move |wb, cx| {
            if wb
                .editing
                .saves
                .borrow()
                .file(document)
                .is_some_and(|file| file.is_dirty())
            {
                wb.search
                    .update(cx, |search, cx| search.cancel_replacement(cx));
            }
        })
    });
    let apply_start = Instant::now();
    command(view, cx, commands::REPLACE_ALL);
    wait(view, cx, |wb, cx| {
        wb.search
            .read(cx)
            .replacement_test_report()
            .is_some_and(|report| {
                report.cancelled && report.applied.len() == 1 && report.applied.contains_key(&first)
            })
    });
    let cancel_elapsed = apply_start.elapsed();
    drop(observation);
    command(view, cx, commands::UNDO_REPLACE);
    wait(view, cx, |wb, cx| {
        !wb.search.read(cx).replacement_test_state().3
    });
    assert!(!view.read_with(cx, |wb, _| wb.editing.saves.borrow().is_dirty(document)));
    (cancel_elapsed, first)
}

#[gpui::test]
fn replace_workspace_large_unity_writes_only_the_independent_full_copy(cx: &mut TestAppContext) {
    use rove_test_support::UnityProject;

    const TEST_COPY: &str = "ROVE_TEST_REPLACE_UNITY_COPY";
    let Some(copy) = std::env::var_os(TEST_COPY) else {
        eprintln!("已跳过：{TEST_COPY} 没有指定已验证的独立大型 Unity 副本");
        return;
    };
    let original = rove_test_support::require_unity_project!(UnityProject::Large);
    let original = original.canonicalize().unwrap();
    let copy = std::path::PathBuf::from(copy).canonicalize().unwrap();
    assert!(
        copy.is_dir() && !copy.starts_with(&original) && !original.starts_with(&copy),
        "写入测试副本必须独立于原大型 Unity 项目"
    );
    let fixture = Fixture::new(r#"{"search.maxResults":0}"#);
    let (view, cx) = fixture.open_at(&copy, cx);
    let query = "GetComponent";
    let replacement = "rove_full_copy_replacement";
    let preview_start = Instant::now();
    let summary = prepare_full_unity_preview(&view, cx, "大型 Unity 完整副本", query, replacement);
    let preview_elapsed = preview_start.elapsed();
    assert!(
        !summary.limit_hit && summary.files_searched > 1500,
        "{summary:?}"
    );
    let scroll_elapsed = scroll_replacement_results(&view, cx);
    let (cancel_elapsed, opened) = cancel_large_replacement(&copy, &view, cx);

    prepare_full_unity_preview(&view, cx, "大型 Unity 完整副本撤销后", query, replacement);
    let candidates = view.read_with(cx, |wb, cx| {
        wb.search.read(cx).replacement_test_writable_files()
    });
    let (path, file, original_bytes) = candidates
        .into_iter()
        .filter(|candidate| *candidate != opened && candidate.as_str().ends_with(".cs"))
        .find_map(|path| {
            let file = copy.join(path.as_str());
            let bytes = std::fs::read(&file).ok()?;
            let text = std::str::from_utf8(&bytes).ok()?;
            if !text.contains(query) || text.contains(replacement) {
                return None;
            }
            Some((path, file, bytes))
        })
        .expect("完整副本应有尚未打开的 UTF-8 C# 匹配文件");

    focus_file(&view, cx, &path);
    command(&view, cx, commands::REPLACE_FILE);
    wait(&view, cx, |wb, cx| {
        wb.search
            .read(cx)
            .replacement_test_report()
            .is_some_and(|report| report.applied.contains_key(&path))
    });
    let written = std::fs::read(&file).unwrap();
    assert_ne!(written, original_bytes);
    assert!(std::str::from_utf8(&written).unwrap().contains(replacement));
    command(&view, cx, commands::UNDO_REPLACE);
    wait(&view, cx, |wb, cx| {
        !wb.search.read(cx).replacement_test_state().3
    });
    let restored = std::fs::read(&file).unwrap();
    assert_eq!(restored, original_bytes);
    let content_hash = rove_buffer::save::content_hash;
    eprintln!(
        "完整 Unity 副本写盘与撤销：{}，FNV-1a {:016x} → {:016x} → {:016x}",
        path.as_str(),
        content_hash(&original_bytes),
        content_hash(&written),
        content_hash(&restored)
    );
    eprintln!(
        "全局替换完整 Unity 副本：扫描 {} 文件，{} 个文件内 {} 处匹配，{} 跳过，截断={}；预览 {:?}，五次滚动 {:?}，首个文件完成后取消 {:?}，独立磁盘文件逐字节撤销通过",
        summary.files_searched, summary.files_with_matches, summary.matches, summary.skipped,
        summary.limit_hit, preview_elapsed, scroll_elapsed, cancel_elapsed,
    );
}

#[gpui::test]
fn replace_workspace_large_unity_sample_previews_scrolls_and_cancels(cx: &mut TestAppContext) {
    use crate::search::view::State;
    use rove_test_support::UnityProject;
    use std::{
        cell::Cell,
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    const FIRST_PREVIEW_WAIT: Duration = Duration::from_secs(20);
    let original = rove_test_support::require_unity_project!(UnityProject::Large);
    let fixture = Fixture::new(r#"{"search.maxResults": 10000}"#);
    let fixture_started = Instant::now();
    let mut fixture_files = Vec::new();
    let mut folders = vec![original.join("Assets")];
    let mut file_count = 0;
    let mut total_bytes = 0;
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                folders.push(path);
                continue;
            }
            if !kind.is_file() || path.extension() != Some(std::ffi::OsStr::new("cs")) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.len() > 256 * 1024 {
                continue;
            }
            let relative = path.strip_prefix(&original).unwrap();
            let dest = fixture.folder.path().join(relative);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            let copied = std::fs::copy(&path, dest).unwrap();
            fixture_files.push((relative.to_owned(), copied));
            file_count += 1;
            total_bytes += copied;
            if file_count == 1500 {
                break;
            }
        }
        if file_count == 1500 {
            break;
        }
    }
    assert!(
        file_count >= 250,
        "大型 Unity 项目源码样本过少：{file_count}"
    );
    fixture_files.sort_unstable();
    let mut fixture_id = DefaultHasher::new();
    fixture_files.hash(&mut fixture_id);
    let fixture_id = fixture_id.finish();
    let fixture_elapsed = fixture_started.elapsed();
    let open_start = Instant::now();
    let (view, cx) = fixture.open(cx);
    let open_elapsed = open_start.elapsed();
    let preview_start = Instant::now();
    start_search(&view, cx, "namespace", "rove_sample_namespace");
    let wait_started = Instant::now();
    let first_done = Cell::new(None);
    let observed_ready = reaches(&view, cx, FIRST_PREVIEW_WAIT, |wb, cx| {
        let search = wb.search.read(cx);
        if first_done.get().is_none() && matches!(search.state(), State::Done(_)) {
            first_done.set(Some(wait_started.elapsed()));
        }
        search.replacement_test_state().1
    });
    let wait_elapsed = wait_started.elapsed();
    let preview_elapsed = preview_start.elapsed();
    let (state, done, files, matches, stage, published) = view.read_with(cx, |wb, cx| {
        let search = wb.search.read(cx);
        (
            format!("{:?}", search.state()),
            matches!(search.state(), State::Done(_)),
            search.results().files().len(),
            search.results().match_count(),
            search.replacement_test_preview_stage(),
            search.replacement_test_published_at(),
        )
    });
    let first_done = first_done.get();
    let published_since_dispatch = published.map(|at| at.saturating_duration_since(preview_start));
    let published_since_wait = published.map(|at| at.saturating_duration_since(wait_started));
    let within_deadline = published_since_wait.is_some_and(|elapsed| elapsed <= FIRST_PREVIEW_WAIT);
    eprintln!(
        "[#366 first preview] 夹具 ROVE_TEST_UNITY_LARGE/Assets 的 C# 副本 {file_count} 文件 / \
         {total_bytes} B，路径+字节数指纹 {fixture_id:016x}，复制与清单 {fixture_elapsed:?}，\
         工作台打开 {open_elapsed:?}；搜索状态 {state}，Done={done}，首次观察 Done={first_done:?}，\
         已有 {files} 个文件 / {matches} 处匹配，预览阶段 {stage}，观察 Ready={observed_ready}；\
         发布距派发 {published_since_dispatch:?}，发布距等待 {published_since_wait:?}，\
         等待实际 {wait_elapsed:?}，总计 {preview_elapsed:?}，截止 {FIRST_PREVIEW_WAIT:?}，\
         截止内发布={within_deadline}"
    );
    assert!(
        within_deadline,
        "首个全局替换预览未在 {FIRST_PREVIEW_WAIT:?} 内发布"
    );
    assert!(matches > 100, "真实样本匹配过少：{matches}");
    let scroll_elapsed = scroll_replacement_results(&view, cx);
    let (cancel_elapsed, _) = cancel_large_replacement(fixture.folder.path(), &view, cx);
    eprintln!(
        "全局替换真实 Unity 源码副本：{file_count} 文件 / {total_bytes} B / {matches} 处；预览 {preview_elapsed:?}，五次滚动 {scroll_elapsed:?}，取消 {cancel_elapsed:?}"
    );
}
