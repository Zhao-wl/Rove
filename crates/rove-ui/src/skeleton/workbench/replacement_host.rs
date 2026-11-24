//! #62 在工作台分发编辑器缓冲区事务和关闭文件的后台原子写入。
use super::*;
use crate::{
    feedback::{Feedback, FeedbackService, Severity},
    search::{
        replace::{
            self, ApplyRequest, DiskUndo, Failure, FilePlan, OpenFile, OpenRecovery, Origin,
            PreviewRequest,
        },
        view::{ReplacementEvent, Report},
    },
};
use rove_buffer::{
    edit::Transaction,
    save::{WriteError, WriteOptions},
    TextSource,
};
use std::path::Path;

#[derive(Default)]
pub(super) struct ReplacementBatch {
    running: bool,
    undo: Vec<(WorkspacePath, Undo)>,
}

enum Undo {
    Open {
        document: DocumentId,
        marker: u64,
        version: Version,
    },
    Disk(DiskUndo),
}

impl Workbench {
    pub(super) fn replacement_event(&mut self, event: &ReplacementEvent, cx: &mut Context<Self>) {
        match event {
            ReplacementEvent::Preview(request) if request.root == self.root => {
                self.build_replace_preview(request.clone(), cx);
            }
            ReplacementEvent::Apply(request) if !self.replacement.running => {
                self.apply_replacements(request.clone(), cx);
            }
            ReplacementEvent::Undo
                if !self.replacement.running && !self.replacement.undo.is_empty() =>
            {
                self.undo_replacements(cx);
            }
            _ => {}
        }
    }

    fn build_replace_preview(&self, request: PreviewRequest, cx: &mut Context<Self>) {
        let saves = self.editing.saves.borrow();
        let mut opened = BTreeMap::new();
        for node in &request.files {
            let path = &node.path;
            let absolute = self.root.join(path.as_str());
            let Some(id) = saves.document_for(&absolute) else {
                continue;
            };
            let Some(file) = saves.file(id) else {
                continue;
            };
            opened.insert(
                path.clone(),
                OpenFile {
                    id,
                    text: file.snapshot(),
                    read_only: !file.session().buffer().read_only().is_empty(),
                    dirty: file.is_dirty(),
                    eol: file.session().buffer().eol(),
                    recovery: file
                        .is_dirty()
                        .then(|| {
                            let disk = file.disk_stamp()?.clone();
                            let marker = file.saved_marker();
                            let history = file.session().undo_snapshot(usize::MAX);
                            if !history.reaches(marker) {
                                return None;
                            }
                            let mut from = 0;
                            if marker != 0 {
                                from = history.undo.iter().position(|unit| unit.id == marker)? + 1;
                            }
                            let steps = history
                                .undo
                                .into_iter()
                                .skip(from)
                                .flat_map(|unit| unit.steps.into_iter().map(|step| step.forward))
                                .collect();
                            Some(OpenRecovery { disk, steps })
                        })
                        .flatten(),
                },
            );
        }
        drop(saves);
        let serial = request.serial;
        let settings = self.settings.clone();
        let task = cx
            .background_executor()
            .spawn(async move { replace::prepare_preview(&request, &opened, settings).ok() });
        let search = self.search.downgrade();
        cx.spawn(async move |_, cx| {
            let preview = task.await;
            let _ = search.update(cx, |view, cx| {
                view.receive_replace_preview(serial, preview, cx)
            });
        })
        .detach();
    }

    fn apply_replacements(&mut self, request: ApplyRequest, cx: &mut Context<Self>) {
        self.replacement.running = true;
        let settings = self.settings.clone();
        let root = self.root.clone();
        let rules = rove_workspace::WorkspaceConfig::from_settings(settings.as_ref());
        cx.spawn(async move |this, cx| {
            let mut report = Report {
                skipped: request.skipped,
                failed: request.stale,
                ..Default::default()
            };
            let mut undo = Vec::new();
            let mut busy = Vec::new();
            let mut read_only = Vec::new();
            for plan in request.plans {
                if request.cancel.check().is_err() {
                    report.cancelled = true;
                    break;
                }
                // 用户可能在预览之后更新只读设置。旧计划的可写判断不能授权这次事务。
                let Ok(rules) = &rules else {
                    report.failed.push(plan.path);
                    continue;
                };
                if !rules
                    .readonly
                    .evaluate(plan.path.as_str(), false)
                    .is_empty()
                {
                    report.skipped.push(plan.path);
                    continue;
                }
                let tx = match replace::transaction(plan.changes.clone()) {
                    Ok(tx) if tx.is_empty() => {
                        report.applied.insert(plan.path, plan.changes);
                        continue;
                    }
                    Ok(tx) => tx.verbatim(),
                    Err(_) => {
                        report.failed.push(plan.path);
                        continue;
                    }
                };
                let result = match plan.origin.clone() {
                    Origin::Open(document, version) => this
                        .update(cx, |wb, cx| {
                            wb.apply_open_replacement(&root, &plan, document, version, tx, cx)
                        })
                        .unwrap_or(Err(Failure::Changed)),
                    Origin::Disk {
                        path,
                        canonical,
                        stamp,
                    } => {
                        let may_write = this
                            .update(cx, |wb, _| wb.root == root && !wb.is_path_open(&path))
                            .unwrap_or(false);
                        if may_write {
                            let cancel = request.cancel.clone();
                            let settings = settings.clone();
                            cx.background_executor()
                                .spawn(async move {
                                    DiskUndo::apply(
                                        path,
                                        &stamp,
                                        canonical,
                                        &tx,
                                        settings.as_ref(),
                                        &WriteOptions::default(),
                                        &cancel,
                                    )
                                    .map(Undo::Disk)
                                })
                                .await
                        } else {
                            Err(Failure::Changed)
                        }
                    }
                };
                match result {
                    Ok(entry) => {
                        undo.push((plan.path.clone(), entry));
                        report.applied.insert(plan.path, plan.changes);
                    }
                    Err(Failure::Cancelled) => {
                        report.cancelled = true;
                        break;
                    }
                    Err(Failure::Skipped) => report.skipped.push(plan.path),
                    Err(Failure::Write(WriteError::ReadOnly)) => {
                        read_only.push(plan.path.clone());
                        report.failed.push(plan.path);
                    }
                    Err(Failure::Write(WriteError::Busy { .. })) => {
                        busy.push(plan.path.clone());
                        report.failed.push(plan.path);
                    }
                    Err(_) => report.failed.push(plan.path),
                }
            }
            let _ = this.update(cx, |wb, cx| {
                wb.replacement.running = false;
                if !undo.is_empty() {
                    wb.replacement.undo = undo;
                }
                wb.replace_receipt(&report);
                wb.replace_write_notice("search.replace.busyFiles", &busy);
                wb.replace_write_notice("search.replace.readOnlyFiles", &read_only);
                let can_undo = !wb.replacement.undo.is_empty();
                wb.search.update(cx, |view, cx| {
                    view.finish_replacement(request.serial, report, can_undo, cx)
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn is_path_open(&self, path: &Path) -> bool {
        self.editing.saves.borrow().document_for(path).is_some()
    }

    fn path_has_unpersisted_edits(&self, path: &Path) -> bool {
        let saves = self.editing.saves.borrow();
        saves.document_for(path).is_some_and(|id| {
            saves.file(id).is_some_and(|file| {
                file.is_dirty() || !file.session().buffer().read_only().is_empty()
            })
        })
    }

    fn apply_open_replacement(
        &mut self,
        root: &Path,
        plan: &FilePlan,
        document: DocumentId,
        version: Version,
        transaction: Transaction,
        cx: &mut Context<Self>,
    ) -> Result<Undo, Failure> {
        if self.root != root {
            return Err(Failure::Changed);
        }
        let (result, before, after, edit) = {
            let mut saves = self.editing.saves.borrow_mut();
            let file = saves.file_mut(document).ok_or(Failure::Changed)?;
            if file.path() != Some(root.join(plan.path.as_str()).as_path())
                || file.snapshot().version() != version
            {
                return Err(Failure::Changed);
            }
            let session = file.session_mut();
            if !session.buffer().read_only().is_empty() {
                return Err(Failure::Skipped);
            }
            let before = session.snapshot();
            session.push_undo_stop();
            let applied = session.edit(transaction, Instant::now())?;
            session.push_undo_stop();
            let after = session.snapshot();
            let spans = applied.input_edits.iter().map(|edit| {
                (
                    edit.start_byte.0 as i64,
                    edit.old_end_byte.0 as i64,
                    edit.new_end_byte.0 as i64,
                )
            });
            let combined = crate::code_view::edit::coalesce_spans(spans, applied.before);
            (
                Undo::Open {
                    document,
                    marker: session.history_marker(),
                    version: session.buffer().version(),
                },
                before,
                after,
                combined,
            )
        };
        self.sync_document_from_session(document, cx);
        if let Some(edit) = edit {
            let states = self.nav.router.buffers();
            let version = after.version();
            if states
                .edited_from_snapshot(
                    &plan.path,
                    before.as_ref(),
                    edit.before,
                    &edit.old,
                    &edit.new,
                    after,
                )
                .is_ok()
            {
                self.nav.buffers.note_edited(&plan.path, document, version);
            }
        }
        self.note_hot_exit_edit(document, cx);
        Ok(result)
    }

    fn undo_replacements(&mut self, cx: &mut Context<Self>) {
        self.replacement.running = true;
        let undo = std::mem::take(&mut self.replacement.undo);
        let cancel = Cancellation::default();
        self.search
            .update(cx, |view, cx| view.start_undo(cancel.clone(), cx));
        cx.spawn(async move |this, cx| {
            let mut retained = Vec::new();
            let mut report = Report::default();
            let mut succeeded = 0;
            for (path, entry) in undo {
                if cancel.check().is_err() {
                    report.cancelled = true;
                    retained.push((path, entry));
                    continue;
                }
                let result = match entry {
                    Undo::Open {
                        document,
                        marker,
                        version,
                    } => {
                        let ok = this
                            .update(cx, |wb, cx| {
                                let changed = wb
                                    .editing
                                    .saves
                                    .borrow_mut()
                                    .file_mut(document)
                                    .is_some_and(|file| {
                                        let session = file.session_mut();
                                        session.history_marker() == marker
                                            && session.buffer().version() == version
                                            && session.undo().is_ok_and(|edits| !edits.is_empty())
                                    });
                                if changed {
                                    wb.sync_document_from_session(document, cx);
                                    wb.note_hot_exit_edit(document, cx);
                                }
                                changed
                            })
                            .unwrap_or(false);
                        (!ok).then_some(Undo::Open {
                            document,
                            marker,
                            version,
                        })
                    }
                    Undo::Disk(mut disk) => {
                        if this
                            .update(cx, |wb, _| wb.path_has_unpersisted_edits(&disk.path))
                            .unwrap_or(true)
                        {
                            Some(Undo::Disk(disk))
                        } else {
                            let cancel = cancel.clone();
                            cx.background_executor()
                                .spawn(async move {
                                    disk.restore(&WriteOptions::default(), &cancel)
                                        .err()
                                        .map(|_| Undo::Disk(disk))
                                })
                                .await
                        }
                    }
                };
                if let Some(entry) = result {
                    report.failed.push(path.clone());
                    retained.push((path, entry));
                } else {
                    succeeded += 1;
                }
            }
            report.undone = succeeded != 0;
            let _ = this.update(cx, |wb, cx| {
                wb.replacement.running = false;
                wb.replacement.undo = retained;
                if report.undone {
                    wb.replace_feedback("search.replace.undone", Reason::Receipt, &[]);
                }
                let can_undo = !wb.replacement.undo.is_empty();
                wb.search.update(cx, |view, cx| {
                    if report.undone {
                        view.search_now(cx);
                    }
                    view.finish_replacement(u64::MAX, report, can_undo, cx);
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn replace_receipt(&mut self, report: &Report) {
        if !report.applied.is_empty() {
            let matches = report.applied.values().map(Vec::len).sum::<usize>();
            self.replace_feedback(
                "search.replace.applied",
                Reason::Receipt,
                &[
                    ("files", report.applied.len().to_string()),
                    ("matches", matches.to_string()),
                ],
            );
        } else if !report.skipped.is_empty() {
            self.replace_feedback(
                "search.replace.skipped",
                Reason::Receipt,
                &[("files", report.skipped.len().to_string())],
            );
        }
    }

    fn replace_write_notice(&mut self, key: &str, files: &[WorkspacePath]) {
        if !files.is_empty() {
            let paths = files
                .iter()
                .map(WorkspacePath::as_str)
                .collect::<Vec<_>>()
                .join("、");
            self.replace_feedback(key, Reason::Global(Severity::Error), &[("paths", paths)]);
        }
    }

    fn replace_feedback(&mut self, key: &str, reason: Reason, args: &[(&str, String)]) {
        let _ = self.shell.feedback.show(Feedback {
            key: Arc::from(key),
            level: crate::shell::feedback::level_for(&reason),
            message: Message {
                key: Arc::from(key),
                args: args
                    .iter()
                    .map(|(k, v)| (Arc::from(*k), Arc::from(v.as_str())))
                    .collect(),
            },
            actions: Arc::from([]),
        });
    }
}
