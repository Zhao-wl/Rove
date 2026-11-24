use super::*;
use crate::code_view::links::{self, LinkTarget};
use crate::documents::{OpenTarget, Presentation};
use crate::feedback::{FeedbackAction, FeedbackEvent, FeedbackService};
use gpui::{anchored, deferred, ClipboardItem, Corner, Pixels, Point};
use rove_base::{ByteOffset, Position, Span, Utf16Column};
use rove_buffer::Documents;
use std::path::Path;

pub(super) const EXTERNAL_KEY: &str = "editor.link.external";
const ACTION_OPEN: &str = "editor.link.open";
const ACTION_COPY: &str = "editor.link.copy";
const ACTION_CANCEL: &str = "editor.link.cancel";

#[derive(Clone)]
pub(super) struct LinkOrigin {
    pub group: EditorGroupId,
    pub secondary: bool,
    pub document: Arc<Document>,
    pub position: Position,
}

pub(super) struct LinkMenu {
    pub origin: LinkOrigin,
    pub target: LinkTarget,
    pub point: Point<Pixels>,
}

impl Workbench {
    pub(super) fn link_origin(
        &self,
        group: EditorGroupId,
        secondary: bool,
        position: Position,
    ) -> Option<LinkOrigin> {
        Some(LinkOrigin {
            group,
            secondary,
            document: self.shown_document(group, secondary)?,
            position,
        })
    }

    fn link_origin_current(&self, origin: &LinkOrigin) -> bool {
        self.shown_document(origin.group, origin.secondary)
            .is_some_and(|doc| {
                doc.id == origin.document.id && doc.text.version() == origin.document.text.version()
            })
    }

    pub(super) fn open_editor_link(
        &mut self,
        origin: LinkOrigin,
        target: LinkTarget,
        aside: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.link_origin_current(&origin) {
            return;
        }
        self.nav.link_epoch += 1;
        let epoch = self.nav.link_epoch;
        if let LinkTarget::External { url, .. } = target {
            self.confirm_editor_link(url, cx);
            return;
        }
        let path_of_document = match &origin.document.resource {
            Resource::Workspace(path) => Some(self.root.join(path.as_str())),
            Resource::File(path) => Some(path.clone()),
            Resource::Virtual(_) => None,
        };
        let source = origin.document.clone();
        let root = self.root.clone();
        let documents = self.metadata.files.clone();
        let job = cx.background_executor().spawn(async move {
            let target = match target {
                LinkTarget::Reference(ref label) => {
                    document_reference(&source, label).unwrap_or(target)
                }
                other => other,
            };
            let LinkTarget::Path {
                ref path,
                position,
                ref fragment,
                document_relative,
                ..
            } = target
            else {
                return (target, None);
            };
            let resolved = if path.as_os_str().is_empty() {
                path_of_document.clone()
            } else if path.is_absolute() {
                Some(path.clone())
            } else {
                let parent = document_relative
                    .then(|| path_of_document.as_deref().and_then(Path::parent))
                    .flatten();
                Some(parent.unwrap_or(&root).join(path))
            };
            let Some(path) =
                resolved.filter(|p| is_local_absolute(p) && on_local_drive(p) && p.is_file())
            else {
                return (target, None);
            };
            let resource = path
                .strip_prefix(&root)
                .ok()
                .and_then(|p| WorkspacePath::new(&p.to_string_lossy().replace('\\', "/")).ok())
                .map_or_else(|| Resource::File(path.clone()), Resource::Workspace);
            let mut position = position;
            let fragment = fragment.as_deref().filter(|f| !f.is_empty());
            if fragment.is_some() || position.is_some_and(|p| p.column.0 != 0) {
                let cancel = Cancellation::default();
                let doc = if source.resource == resource || path_of_document.as_ref() == Some(&path)
                {
                    Some(source)
                } else {
                    documents.open(resource.clone(), cancel.clone()).await.ok()
                };
                if let Some(doc) = doc {
                    if let Some(p) = position.as_mut() {
                        if let Ok(column) = doc.text.utf16_to_utf8(p.line, Utf16Column(p.column.0))
                        {
                            p.column = column;
                        }
                    }
                    if let Some(fragment) = fragment {
                        position = heading(&doc, fragment, cancel).await.or(position);
                    }
                }
            }
            (target, Some((resource, position)))
        });
        cx.spawn(async move |this, cx| {
            let (target, opened) = job.await;
            let _ = this.update(cx, |view, cx| {
                if view.nav.link_epoch != epoch || !view.link_origin_current(&origin) {
                    return;
                }
                if let LinkTarget::External { url, .. } = target {
                    view.confirm_editor_link(url, cx);
                    return;
                }
                if let Some((resource, position)) = opened {
                    let options = OpenOptions {
                        target: if aside {
                            OpenTarget::Beside
                        } else {
                            OpenTarget::Group(origin.group)
                        },
                        source: OpenSource::CodeNavigation,
                        presentation: Presentation::Source,
                        location: position,
                        ..Default::default()
                    };
                    let _ = view
                        .open_documents()
                        .open(resource, options, Cancellation::default());
                } else {
                    let _ = shell::view::report(
                        view,
                        "editor.link.missing",
                        Reason::NoResult {
                            document: origin.document.id,
                            position: origin.position,
                        },
                        Message {
                            key: "editor.link.notFound".into(),
                            args: Arc::from([("path".into(), target.display().into())]),
                        },
                        Arc::from([]),
                        cx,
                    );
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_editor_link(&mut self, url: String, cx: &mut Context<Self>) {
        if self.nav.link_confirmation.is_some() {
            return;
        }
        let messages = [
            (ACTION_OPEN, "editor.link.openLabel"),
            (ACTION_CANCEL, "editor.link.cancelLabel"),
            (ACTION_COPY, "editor.link.copyLabel"),
        ];
        let actions: Arc<[FeedbackAction]> = messages
            .into_iter()
            .map(|(action, key)| FeedbackAction {
                id: action.into(),
                label: Message::key(key),
            })
            .collect::<Vec<_>>()
            .into();
        let message = Message {
            key: "editor.link.confirm".into(),
            args: Arc::from([("url".into(), url.as_str().into())]),
        };
        let (send, receive) = async_channel::bounded(1);
        let subscription =
            self.shell
                .feedback
                .events()
                .subscribe(Arc::new(move |event| match event {
                    FeedbackEvent::Action { key, action } if &*key == EXTERNAL_KEY => {
                        let _ = send.try_send(Some(action));
                    }
                    FeedbackEvent::Dismissed { key } if &*key == EXTERNAL_KEY => {
                        let _ = send.try_send(None);
                    }
                    _ => {}
                }));
        if shell::view::report(
            self,
            EXTERNAL_KEY,
            Reason::SecurityConfirmation,
            message,
            actions,
            cx,
        )
        .is_err()
        {
            return;
        }
        self.nav.link_confirmation = Some(cx.spawn(async move |this, cx| {
            let action = receive.recv().await.ok().flatten();
            drop(subscription);
            let _ = this.update(cx, |view, cx| {
                match action.as_deref() {
                    Some(ACTION_OPEN) => {
                        #[cfg(not(test))]
                        cx.open_url(&url);
                        #[cfg(test)]
                        view.nav.opened_link_urls.push(url);
                    }
                    Some(ACTION_COPY) => cx.write_to_clipboard(ClipboardItem::new_string(url)),
                    _ => {}
                }
                view.nav.link_confirmation = None;
                cx.notify();
            });
        }));
    }

    fn copy_editor_link(&mut self, origin: LinkOrigin, target: LinkTarget, cx: &mut Context<Self>) {
        if !self.link_origin_current(&origin) {
            return;
        }
        let LinkTarget::Reference(label) = target else {
            cx.write_to_clipboard(ClipboardItem::new_string(target.display().to_owned()));
            return;
        };
        let source = origin.document.clone();
        let job = cx
            .background_executor()
            .spawn(async move { document_reference(&source, &label) });
        cx.spawn(async move |this, cx| {
            if let Some(target) = job.await {
                let _ = this.update(cx, |view, cx| {
                    if view.link_origin_current(&origin) {
                        cx.write_to_clipboard(ClipboardItem::new_string(
                            target.display().to_owned(),
                        ));
                    }
                });
            }
        })
        .detach();
    }

    pub(super) fn editor_link_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.nav.link_menu.as_ref()?;
        let at = menu.point;
        let open = self
            .strings
            .text(&Message::key("editor.link.openLabel"))
            .ok()?;
        let copy = self
            .strings
            .text(&Message::key("editor.link.copyLabel"))
            .ok()?;
        let can_collect = matches!(menu.origin.document.resource, Resource::Workspace(_));
        let collect = self.label("command.rove.contextBasket.add", &[]);
        let style = &self.nav.colors;
        let card = div()
            .id("editor-link-menu")
            .flex()
            .flex_col()
            .p(px(4.0))
            .bg(crate::code_view::view::hsla(style.hover_background?))
            .text_color(crate::code_view::view::hsla(style.hover_foreground?))
            .border_1()
            .border_color(crate::code_view::view::hsla(style.hover_border?))
            .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                view.nav.link_menu = None;
                cx.notify();
            }))
            .child(
                div()
                    .id("editor-open-link")
                    .debug_selector(|| "editor-open-link".into())
                    .px(px(8.0))
                    .py(px(3.0))
                    .child(open)
                    .on_click(cx.listener(|view, _, _, cx| {
                        if let Some(menu) = view.nav.link_menu.take() {
                            view.open_editor_link(menu.origin, menu.target, false, cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("editor-copy-link")
                    .debug_selector(|| "editor-copy-link".into())
                    .px(px(8.0))
                    .py(px(3.0))
                    .child(copy)
                    .on_click(cx.listener(|view, _, _, cx| {
                        if let Some(menu) = view.nav.link_menu.take() {
                            view.copy_editor_link(menu.origin, menu.target, cx);
                        }
                        cx.notify();
                    })),
            )
            .when(can_collect, |card| {
                card.child(
                    div()
                        .id("editor-link-add-to-basket")
                        .px_2()
                        .py_1()
                        .child(collect)
                        .on_click(cx.listener(|view, _, _, cx| {
                            if let Some(menu) = view.nav.link_menu.take() {
                                if view.link_origin_current(&menu.origin) {
                                    view.add_basket_editor_pane(
                                        menu.origin.group,
                                        menu.origin.secondary,
                                        cx,
                                    );
                                }
                            }
                            cx.notify();
                        })),
                )
            });
        Some(
            deferred(
                anchored()
                    .anchor(Corner::TopLeft)
                    .position(at)
                    .snap_to_window_with_margin(px(4.0))
                    .child(card),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

fn is_local_absolute(path: &Path) -> bool {
    #[cfg(windows)]
    return path.is_absolute()
        && matches!(path.components().next(),
        Some(std::path::Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)));
    #[cfg(not(windows))]
    return path.is_absolute() && !path.as_os_str().to_string_lossy().starts_with("//");
}

#[cfg(windows)]
fn on_local_drive(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
                crate::markdown::webview::is_local_drive(drive)
            }
            _ => false,
        },
        _ => false,
    }
}

#[cfg(not(windows))]
fn on_local_drive(_: &Path) -> bool {
    true
}

fn document_reference(document: &Document, label: &str) -> Option<LinkTarget> {
    if document.metadata.size == rove_buffer::SizeClass::Huge {
        return None;
    }
    reference_destination(&*document.text, label)
}

// A reference destination can live outside the viewport; resolve it only on
// explicit activation, with Markdown's reference table on the background worker.
fn reference_destination(text: &dyn rove_buffer::TextSource, label: &str) -> Option<LinkTarget> {
    let mut source = String::new();
    text.visit_chunks(
        Span::new(ByteOffset(0), ByteOffset(text.byte_len())).ok()?,
        &mut |c| source.push_str(c),
    )
    .ok()?;
    let parser = pulldown_cmark::Parser::new(&source);
    links::target(&parser.reference_definitions().get(label)?.dest, true)
}

async fn heading(document: &Document, fragment: &str, cancel: Cancellation) -> Option<Position> {
    let outline = rove_syntax::symbols::SymbolService
        .analyze(
            document.text.clone(),
            document.metadata.language.clone(),
            document.metadata.size,
            None,
            cancel,
        )
        .await
        .ok()?;
    let mut counts = BTreeMap::new();
    let position = outline.symbols.iter().find_map(|(_, n)| {
        let base = crate::markdown::render::slugify(&n.symbol.name);
        let count = counts.entry(base.clone()).or_insert(0);
        let slug = if *count == 0 {
            base
        } else {
            format!("{base}-{count}")
        };
        *count += 1;
        (slug == fragment).then_some(n.symbol.range.start())
    });
    position
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::model::pos;
    use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext};

    fn settle(cx: &mut VisualTestContext) {
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.refresh();
                cx.refresh_windows();
            });
        }
        cx.run_until_parked();
    }

    fn fixture<'a>(
        files: &[(&str, &str)],
        opened: &str,
        cx: &'a mut TestAppContext,
    ) -> (
        tempfile::TempDir,
        Entity<Workbench>,
        &'a mut VisualTestContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        for &(path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let root = dir.path().to_path_buf();
        let (view, cx) = cx.add_window_view(|_, cx| Workbench::new(root, Instant::now(), cx));
        view.update(cx, |view, cx| {
            view.open_documents()
                .open(
                    Resource::Workspace(WorkspacePath::new(opened).unwrap()),
                    OpenOptions {
                        presentation: Presentation::Source,
                        ..Default::default()
                    },
                    Cancellation::default(),
                )
                .unwrap();
            cx.notify();
        });
        settle(cx);
        (dir, view, cx)
    }

    fn pointer_at(
        view: &Entity<Workbench>,
        at: Position,
        cx: &mut VisualTestContext,
    ) -> Point<Pixels> {
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        code.update(cx, |code, _| {
            assert!(code.link_at(at).is_some());
            let left = code.point_for_position(at).unwrap();
            let right = code
                .point_for_position(pos(at.line.0, at.column.0 + 1))
                .unwrap();
            point(
                left.x + (right.x - left.x) * 0.25,
                left.y + px(code.line_height() / 2.0),
            )
        })
    }

    #[gpui::test]
    fn editor_link_gesture_opens_markdown_source_at_cross_file_and_local_anchors(
        cx: &mut TestAppContext,
    ) {
        let (_dir, view, cx) = fixture(
            &[
                (
                    "Docs/Readme.md",
                    "[guide](Guide.md#getting-started) and <https://example.test/help>",
                ),
                (
                    "Docs/Guide.md",
                    "# Guide\n\n# Getting started\n\n[above](#guide)\n",
                ),
            ],
            "Docs/Readme.md",
            cx,
        );
        let at = pointer_at(&view, pos(0, 3), cx);
        cx.simulate_mouse_move(at, None, Modifiers::control());
        settle(cx);
        view.read_with(cx, |v, cx| {
            let code = v.code_view().read(cx);
            let expected = v
                .nav
                .colors_link()
                .map(crate::code_view::TextDecoration::Color);
            let decor = code.merged_decorations(0).unwrap();
            assert!(decor
                .runs
                .iter()
                .any(|r| r.style.underline == expected && expected.is_some()));
        });
        cx.simulate_click(at, Modifiers::control());
        settle(cx);
        view.read_with(cx, |v, cx| {
            let tab = v.editor.active().current().unwrap();
            assert_eq!(
                tab.resource,
                Resource::Workspace(WorkspacePath::new("Docs/Guide.md").unwrap())
            );
            assert!(matches!(tab.kind, EditorKind::Text));
            assert_eq!(v.code_view().read(cx).selection().active.line.0, 2);
        });
        let at = pointer_at(&view, pos(4, 3), cx);
        cx.simulate_click(at, Modifiers::control());
        settle(cx);
        assert_eq!(
            view.read_with(cx, |v, app| v
                .code_view()
                .read(app)
                .selection()
                .active
                .line
                .0),
            0
        );
    }

    #[gpui::test]
    fn editor_link_external_requires_a_new_confirmation_and_context_menu_copies(
        cx: &mut TestAppContext,
    ) {
        let url = "https://example.test/help";
        let (_dir, view, cx) = fixture(&[("Help.txt", url)], "Help.txt", cx);
        let at = pointer_at(&view, pos(0, 3), cx);
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
        settle(cx);
        assert!(view.read_with(cx, |v, _| v.nav.link_menu.is_some()));
        let copy = cx.debug_bounds("editor-copy-link").unwrap().center();
        cx.simulate_click(copy, Modifiers::none());
        settle(cx);
        assert_eq!(
            cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
                .as_deref(),
            Some(url)
        );
        assert!(view.read_with(cx, |v, _| v.nav.link_menu.is_none()));
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
        settle(cx);
        let menu_open = cx.debug_bounds("editor-open-link").unwrap().center();
        cx.simulate_click(menu_open, Modifiers::none());
        settle(cx);
        view.update(cx, |v, _| {
            v.shell.feedback.act(EXTERNAL_KEY, ACTION_CANCEL).unwrap()
        });
        settle(cx);
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
        settle(cx);
        cx.simulate_click(point(px(3.), px(3.)), Modifiers::none());
        settle(cx);
        assert!(view.read_with(cx, |v, _| v.nav.link_menu.is_none()));
        for action in [ACTION_CANCEL, ACTION_COPY, ACTION_OPEN, ACTION_CANCEL] {
            cx.simulate_click(at, Modifiers::control());
            settle(cx);
            view.update(cx, |v, _| {
                let modal = v.shell.feedback.modal().unwrap();
                assert_eq!(&*modal.key, EXTERNAL_KEY);
                assert!(v.strings.text(&modal.message).unwrap().ends_with(url));
                assert_eq!(modal.actions.len(), 3);
                v.shell.feedback.act(EXTERNAL_KEY, action).unwrap();
            });
            settle(cx);
            view.read_with(cx, |v, _| {
                assert!(v.shell.feedback.modal().is_none());
                assert!(v.nav.link_confirmation.is_none());
            });
        }
        assert_eq!(
            cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
                .as_deref(),
            Some(url)
        );
        assert_eq!(
            view.read_with(cx, |v, _| v.nav.opened_link_urls.clone()),
            [url]
        );
    }

    #[gpui::test]
    fn editor_link_detection_is_viewport_bounded_and_respects_settings(cx: &mut TestAppContext) {
        let text = "https://example.test/help\n".repeat(800);
        let (_dir, view, cx) = fixture(&[("Log.txt", &text)], "Log.txt", cx);
        let code = view.read_with(cx, |view, _| view.code_view().clone());
        let entries = code.read_with(cx, |code, _| {
            assert!(code.link_at(pos(0, 4)).is_some());
            assert!(code.link_at(pos(799, 4)).is_none());
            code.cached_link_lines()
        });
        assert!((1..100).contains(&entries), "{entries} detected lines");
        settle(cx);
        code.update(cx, |code, cx| {
            assert_eq!(code.cached_link_lines(), entries);
            code.set_document_level(rove_buffer::SizeClass::Large, cx);
            assert!(code.link_at(pos(0, 4)).is_some());
            code.set_document_level(rove_buffer::SizeClass::Huge, cx);
            assert!(code.link_at(pos(0, 4)).is_none());
            assert!(code
                .disabled_features()
                .contains(&crate::code_view::Feature::Links));
            code.set_document_level(rove_buffer::SizeClass::Normal, cx);
            let mut options = code.options().clone();
            options.links = false;
            code.set_options(options, cx);
            assert!(code.link_at(pos(0, 4)).is_none());
            assert_eq!(code.cached_link_lines(), 0);
        });
    }

    #[gpui::test]
    fn editor_link_gesture_follows_the_multicursor_modifier(cx: &mut TestAppContext) {
        let (_dir, view, cx) =
            fixture(&[("Help.txt", "https://example.test/help")], "Help.txt", cx);
        let code = view.read_with(cx, |v, _| v.code_view().clone());
        code.update(cx, |code, cx| {
            let mut options = code.options().clone();
            options.multi_cursor_modifier = crate::code_view::MultiCursorModifier::CtrlCmd;
            code.set_options(options, cx);
        });
        settle(cx);
        let at = pointer_at(&view, pos(0, 4), cx);
        cx.simulate_click(at, Modifiers::control());
        settle(cx);
        assert!(view.read_with(cx, |view, _| view.shell.feedback.modal().is_none()));
        cx.simulate_click(
            at,
            Modifiers {
                alt: true,
                ..Modifiers::none()
            },
        );
        settle(cx);
        view.update(cx, |view, _| {
            assert_eq!(&*view.shell.feedback.modal().unwrap().key, EXTERNAL_KEY);
            view.shell
                .feedback
                .act(EXTERNAL_KEY, ACTION_CANCEL)
                .unwrap();
        });
        settle(cx);
    }

    #[gpui::test]
    fn editor_link_command_opens_workspace_path_at_requested_column(cx: &mut TestAppContext) {
        let (_dir, view, cx) = fixture(
            &[
                ("Errors.txt", "Assets/Hero.txt:2:2"),
                ("Assets/Hero.txt", "first\n中ABC\n"),
            ],
            "Errors.txt",
            cx,
        );
        let at = pointer_at(&view, pos(0, 3), cx);
        cx.simulate_click(at, Modifiers::none());
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.nav_command(crate::nav::registry::OPEN_LINK, window, cx)
            });
        });
        settle(cx);
        view.read_with(cx, |view, cx| {
            assert_eq!(
                view.editor.active().current().unwrap().resource,
                Resource::Workspace(WorkspacePath::new("Assets/Hero.txt").unwrap())
            );
            assert_eq!(view.code_view().read(cx).selection().active, pos(1, 3));
        });
    }

    #[gpui::test]
    fn editor_link_missing_path_is_a_caret_hint(cx: &mut TestAppContext) {
        let (_dir, view, cx) = fixture(&[("Errors.txt", "Assets/Missing.cs:41")], "Errors.txt", cx);
        let at = pointer_at(&view, pos(0, 3), cx);
        cx.simulate_click(at, Modifiers::control());
        settle(cx);
        view.read_with(cx, |v, _| {
            assert_eq!(
                &*v.shell.feedback.cursor_hint().unwrap().message.key,
                "editor.link.notFound"
            );
            assert!(v.shell.feedback.notifications().is_empty());
            let tab = v.editor.active().current().unwrap();
            assert_eq!(
                tab.resource,
                Resource::Workspace(WorkspacePath::new("Errors.txt").unwrap())
            );
        });
    }

    #[test]
    fn reference_definitions_are_resolved_by_markdown_labels() {
        let source = rove_buffer::RopeText::from_text(
            "[Guide][manual]\n\n[manual]: ./guide.md#quick-start\n",
        );
        let target = reference_destination(&source, "MANUAL").unwrap();
        assert!(matches!(target, LinkTarget::Path { fragment: Some(s), .. } if s == "quick-start"));
    }

    #[cfg(windows)]
    #[test]
    fn local_paths_reject_unc_and_devices() {
        assert!(is_local_absolute(Path::new(r"C:\workspace\example.md")));
        assert!(!is_local_absolute(Path::new(r"\\host\share\example.md")));
        assert!(!is_local_absolute(Path::new(r"\\.\CON")));
        assert!(!is_local_absolute(Path::new(r"C:relative.md")));
    }
}
