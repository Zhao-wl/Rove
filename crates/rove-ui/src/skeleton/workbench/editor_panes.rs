use super::*;
use crate::editors::settings::{EditorKind, Sizing, Tabs};
use gpui::{AnyElement, Div, MouseButton};

#[derive(Clone)]
struct TabDrag {
    group: EditorGroupId,
    tab: TabId,
    label: SharedString,
}

struct DragLabel(SharedString, Hsla, Hsla);
impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(self.1)
            .text_color(self.2)
            .child(self.0.clone())
    }
}

/// 标签拖到编辑器正文上的落点（`x`、`y` 是光标在正文里的相对位置，0..1）：
/// 离某条边不到三分之一就在那一侧拆分，否则移进这个组（与 VS Code 的手感一致：
/// 拆分区足够宽，不用贴着窗口边缘找）。
pub(super) fn tab_drop_zone(x: f32, y: f32) -> Option<Direction> {
    const SPLIT: f32 = 1.0 / 3.0;
    [
        (x, Direction::Left),
        (1.0 - x, Direction::Right),
        (y, Direction::Up),
        (1.0 - y, Direction::Down),
    ]
    .into_iter()
    .filter(|(d, _)| *d < SPLIT)
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .map(|(_, direction)| direction)
}

fn id(part: &str, group: EditorGroupId, tab: TabId) -> SharedString {
    format!("{part}-{}-{tab}", group.0).into()
}

/// #209：错误页的提示文案（登记在 `skeleton::appearance`，这里只选 key）。磁盘文件上的
/// `Unsupported` 就是 UX_SPEC §3.5 的「二进制文件或使用了不支持的文本编码」，虚拟资源
/// （元数据视图、欢迎页等）仍用通用的「格式或编码不受支持」。
pub(crate) fn error_page_key(error: &Error, resource: &Resource) -> &'static str {
    match error {
        Error::NotFound
        | Error::Io {
            kind: std::io::ErrorKind::NotFound,
            ..
        } => "editors.fileMissing",
        Error::Io {
            kind: std::io::ErrorKind::PermissionDenied,
            ..
        } => "editors.fileDenied",
        Error::Unsupported if can_open_anyway(error, resource) => "editors.fileBinary",
        Error::Unsupported => "editors.fileUnsupported",
        Error::OutOfBounds => "editors.invalidLocation",
        _ => "editors.fileUnavailable",
    }
}

/// #209：能不能给这个错误提供「仍然打开」。只有磁盘上的文件被判成二进制 / 不支持的编码时
/// 才有这条路径（强制打开要重新读盘）；虚拟资源没有字节可以 lossy 显示。
pub(crate) fn can_open_anyway(error: &Error, resource: &Resource) -> bool {
    matches!(error, Error::Unsupported) && !matches!(resource, Resource::Virtual(_))
}

impl Workbench {
    pub(super) fn editor_grid(&self, cx: &mut Context<Self>) -> Div {
        let content = if let Some(group) = self.editor.maximized {
            self.editor_pane(group, cx)
        } else {
            self.render_layout(&self.editor.layout, cx)
        };
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(content)
    }

    fn render_layout(&self, layout: &Layout, cx: &mut Context<Self>) -> Div {
        match layout {
            Layout::Group(group) => self.editor_pane(*group, cx),
            Layout::Split {
                axis,
                first,
                second,
            } => {
                let mut node = div()
                    .flex()
                    .size_full()
                    .gap(px(1.0))
                    .bg(self.ui_color("editorGroup.border"));
                if *axis == Axis::Vertical {
                    node = node.flex_col();
                }
                let shares = layout.shares(self.editor.expanded);
                node.children([(first, 0), (second, 1)].map(|(part, index)| {
                    let child = div();
                    let child = if let Some(shares) = shares {
                        let extent = relative(if index == 0 { shares.0 } else { shares.1 });
                        match axis {
                            Axis::Horizontal => child.w(extent),
                            Axis::Vertical => child.h(extent),
                        }
                    } else {
                        child.flex_1()
                    };
                    child
                        .min_h_0()
                        .min_w_0()
                        .overflow_hidden()
                        .child(self.render_layout(part, cx))
                }))
            }
        }
    }

    fn editor_pane(&self, group: EditorGroupId, cx: &mut Context<Self>) -> Div {
        let pane = &self.editor.groups[&group];
        let focused = self.editor.focused == group;
        let settings = &self.editor.settings;
        let controls = div()
            .flex()
            .flex_shrink_0()
            .h(px(settings.tab_height))
            // #29：`Placement::EditorTitle` 登记的按钮（显示源码 / 显示预览）。
            .children(self.markdown_title_actions(group, cx))
            .children(self.title_controls(group, cx));

        let mut tabs = div().flex_1().min_w_0().flex().flex_col();
        let tab_mode = self.shell.zen_tabs().unwrap_or(settings.tabs);
        match tab_mode {
            Tabs::None => {}
            Tabs::Single => {
                if let Some(tab) = pane.current() {
                    tabs = tabs.child(self.tab_strip(group, "single", std::iter::once(tab), cx));
                }
            }
            Tabs::Multiple if settings.pinned_row && pane.tabs.iter().any(|tab| tab.pinned) => {
                tabs = tabs
                    .child(self.tab_strip(
                        group,
                        "pins",
                        pane.tabs.iter().filter(|tab| tab.pinned),
                        cx,
                    ))
                    .child(self.tab_strip(
                        group,
                        "normal",
                        pane.tabs.iter().filter(|tab| !tab.pinned),
                        cx,
                    ));
            }
            Tabs::Multiple => {
                tabs = tabs.child(self.tab_strip(group, "tabs", pane.tabs.iter(), cx))
            }
        }
        let header = div()
            .flex()
            .items_start()
            .min_h(px(settings.tab_height))
            .flex_shrink_0()
            .bg(self.ui_color(if tab_mode == Tabs::None {
                "editorGroupHeader.noTabsBackground"
            } else {
                "editorGroupHeader.tabsBackground"
            }))
            .border_b_1()
            .border_color(self.ui_color("editorGroupHeader.tabsBorder"))
            .child(tabs)
            .child(controls);
        let progress = pane.current().filter(|tab| tab.show_progress()).map(|_| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(2.0))
                .bg(self.ui_color("progressBar.background"))
        });
        let mut body = div()
            .id(id("editor-body", group, 0))
            .debug_selector(move || format!("editor-body-{}", group.0))
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .bg(self.ui_color("editorPane.background"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, _, _, cx| {
                    // The code view takes keyboard focus itself; this only selects the group.
                    v.editor.focus(group);
                    v.focus_group = Some(group);
                    cx.notify();
                }),
            )
            // 拖动标签经过编辑器正文：按光标离哪条边近决定落点（见 `tab_drop_zone`），
            // 预览层画出新组会占的那一半（移进本组时是整块）。
            .on_drag_move(
                cx.listener(move |v, e: &gpui::DragMoveEvent<TabDrag>, _, cx| {
                    let inside = e.bounds.contains(&e.event.position);
                    let next = if inside {
                        let size = e.bounds.size;
                        let x =
                            f32::from(e.event.position.x - e.bounds.left()) / f32::from(size.width);
                        let y =
                            f32::from(e.event.position.y - e.bounds.top()) / f32::from(size.height);
                        Some((group, tab_drop_zone(x, y)))
                    } else if v.tab_drop.is_some_and(|(g, _)| g == group) {
                        None
                    } else {
                        return;
                    };
                    if v.tab_drop != next {
                        v.tab_drop = next;
                        cx.notify();
                    }
                }),
            )
            .on_drop(cx.listener(move |v, drag: &TabDrag, _, cx| {
                let direction = v
                    .tab_drop
                    .take()
                    .filter(|(g, _)| *g == group)
                    .and_then(|(_, d)| d);
                v.editor
                    .drop_tab(drag.group, drag.tab, group, None, direction);
                cx.stop_propagation();
                cx.notify();
            }));
        let mut viewports = div()
            .flex()
            .size_full()
            // #41：代码视图外框挂上速览（上下文键、命令、嵌入区域里的窗口）。
            .children(
                self.code_viewport(group, false)
                    .map(|d| self.peek_viewport(group, false, d, cx))
                    .map(|d| self.markdown_code_context(group, false, d)),
            );
        if pane.current().is_some_and(|tab| tab.secondary.is_some()) {
            let vertical = settings.secondary_vertical;
            viewports = viewports.gap(px(1.0)).bg(self.ui_color(if vertical {
                "sideBySideEditor.horizontalBorder"
            } else {
                "sideBySideEditor.verticalBorder"
            }));
            if vertical {
                viewports = viewports.flex_col();
            }
            viewports = viewports.children(
                self.code_viewport(group, true)
                    .map(|d| self.peek_viewport(group, true, d, cx))
                    .map(|d| self.markdown_code_context(group, true, d)),
            );
        }
        let text_ready = pane
            .current()
            .is_some_and(|tab| tab.kind == EditorKind::Text && tab.opened().is_some());
        if text_ready {
            body = body.child(viewports);
        }
        match pane.current() {
            // #28：水印的快捷键由 #14 的键位服务解析（含用户在共用 keybindings.json 中的覆盖）。
            None => body = body.child(self.watermark(cx)),
            Some(tab) => match &tab.content {
                Content::Loading => {}
                Content::Failed(error) => body = body.child(self.error_page(group, tab, error, cx)),
                Content::Ready(_) => {
                    if crate::editors::settings::is_welcome(&tab.kind) {
                        // #28：欢迎页。
                        body = body.child(self.render_welcome(group, tab.id, cx));
                    } else if crate::editors::settings::is_markdown_preview(&tab.kind) {
                        // #29：Markdown 预览（WebView2）。
                        body = body.child(self.markdown_viewport(group, tab.id, cx));
                    } else if crate::editors::settings::is_image_preview(&tab.kind) {
                        body = body.child(self.image_viewport(tab.id, cx));
                    } else if let EditorKind::Placeholder { label, .. } = &tab.kind {
                        body = body.child(self.centered(self.label(label, &[])));
                    }
                }
            },
        }
        body = body.children(progress);
        if let Some((_, direction)) = self
            .tab_drop
            .filter(|(g, _)| *g == group && cx.has_active_drag())
        {
            let hint = div()
                .id(id("drop-hint", group, 0))
                .debug_selector(move || format!("drop-hint-{}", group.0))
                .absolute()
                .bg(self.ui_color("editorGroup.dropBackground"));
            let hint = match direction {
                None => hint.inset_0(),
                Some(Direction::Left) => hint.left_0().top_0().bottom_0().w(relative(0.5)),
                Some(Direction::Right) => hint.right_0().top_0().bottom_0().w(relative(0.5)),
                Some(Direction::Up) => hint.top_0().left_0().right_0().h(relative(0.5)),
                Some(Direction::Down) => hint.bottom_0().left_0().right_0().h(relative(0.5)),
            };
            body = body.child(hint);
        }
        let mut result = div().flex().flex_col().size_full().min_w_0().min_h_0();
        if focused && pane.tabs.is_empty() && self.editor.groups.len() > 1 {
            result = result
                .border_1()
                .border_color(self.ui_color("editorGroup.focusedEmptyBorder"));
        }
        // #26: breadcrumbs between the tab strip and the editor body.
        let breadcrumbs = self.render_breadcrumbs(group, cx);
        result
            .when(self.shell.zen_tabs() != Some(Tabs::None), |pane| {
                pane.child(header)
            })
            .children(breadcrumbs)
            .child(body)
    }

    /// 标签栏右侧的编辑器组按钮：codicon 图标，悬停显示作用和快捷键（与 VS Code 一样，
    /// 常驻的只有「向右拆分」「向下拆分」和「最大化 / 还原」；锁定后才出现锁图标，点它解锁）。
    /// 向左、向上拆分和锁定仍可从命令面板或快捷键使用。
    fn title_controls(&self, group: EditorGroupId, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let pane = &self.editor.groups[&group];
        let maximized = self.editor.maximized.is_some();
        let mut buttons = vec![
            (
                "splitRight",
                "split-horizontal",
                "editors.tip.splitRight",
                "workbench.action.splitEditorRight",
            ),
            (
                "splitDown",
                "split-vertical",
                "editors.tip.splitDown",
                "workbench.action.splitEditorDown",
            ),
            if maximized {
                (
                    "max",
                    "screen-normal",
                    "editors.tip.restore",
                    "workbench.action.toggleMaximizeEditorGroup",
                )
            } else {
                (
                    "max",
                    "screen-full",
                    "editors.tip.maximize",
                    "workbench.action.toggleMaximizeEditorGroup",
                )
            },
        ];
        if pane.locked {
            buttons.insert(
                0,
                (
                    "lock",
                    "lock",
                    "editors.tip.unlock",
                    "workbench.action.toggleEditorGroupLock",
                ),
            );
        }
        let size = px(16.0 * self.shell.zoom());
        let fg = self.ui_color("tab.activeForeground");
        let hover = self.ui_color("tab.hoverBackground");
        buttons
            .into_iter()
            .map(|(key, glyph, tip, command)| {
                let mut text = self.label(tip, &[]);
                if let Some(keys) = crate::keys::key_label(command, cx) {
                    text = format!("{text} ({keys})");
                }
                div()
                    .id(id(key, group, 0))
                    .debug_selector(move || format!("editor-title-{key}-{}", group.0))
                    .h_full()
                    .px(px(5.0))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_color(fg)
                    .rounded(px(4.0))
                    .hover(move |style| style.bg(hover))
                    .child(crate::shell::view::icon_label(&format!("$({glyph})"), size))
                    .tooltip(crate::metadata::tooltip::tab_tooltip(
                        text,
                        self.ui_color("editorHoverWidget.background"),
                        self.ui_color("editorHoverWidget.foreground"),
                        self.ui_color("editorHoverWidget.border"),
                    ))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.focus_group_code(group, window, cx);
                        // Maximize needs to see the previous maximized group to decide whether to restore.
                        if command == "workbench.action.toggleMaximizeEditorGroup" {
                            view.editor.toggle_maximize(group);
                            cx.notify();
                        } else {
                            view.editor.focus(group);
                            view.command(command, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect()
    }

    fn tab_strip<'a>(
        &self,
        group: EditorGroupId,
        name: &'static str,
        tabs: impl Iterator<Item = &'a Tab>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = &self.editor.settings;
        let pane = &self.editor.groups[&group];
        let row = div()
            .id(id(name, group, 0))
            .flex()
            .items_center()
            .min_h(px(settings.tab_height))
            .on_drop(cx.listener(move |v, drag: &TabDrag, _, cx| {
                v.editor.drop_tab(drag.group, drag.tab, group, None, None);
                cx.stop_propagation();
                cx.notify();
            }));
        let row = if settings.wrap {
            row.flex_wrap()
        } else {
            row.overflow_x_scroll()
        };
        row.children(tabs.map(|tab| self.tab_header(group, pane, tab, cx)))
            .into_any_element()
    }

    fn tab_header(
        &self,
        group: EditorGroupId,
        pane: &crate::editors::Group,
        tab: &Tab,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = tab.id;
        let selected = pane.active == Some(tab_id);
        let focused = self.editor.focused == group;
        let settings = &self.editor.settings;
        let last_pin = tab.pinned
            && pane
                .tabs
                .iter()
                .take_while(|tab| tab.pinned)
                .last()
                .is_some_and(|tab| tab.id == tab_id);
        let bg = match (focused, selected) {
            (true, true) => "tab.activeBackground",
            (true, false) => "tab.inactiveBackground",
            (false, true) => "tab.unfocusedActiveBackground",
            (false, false) => "tab.unfocusedInactiveBackground",
        };
        let fg = if settings.colors && matches!(tab.content, Content::Failed(_)) {
            "errorForeground"
        } else if selected {
            "tab.activeForeground"
        } else {
            "tab.inactiveForeground"
        };
        // #42：元数据视图的标签标题是 `Transform [元数据]`，悬停显示 DLL 完整路径。
        // #28：欢迎页的标签标题是「欢迎使用」。
        let name = self
            .welcome_tab_title(&tab.resource)
            .or_else(|| self.metadata_tab_title(&tab.resource))
            .unwrap_or_else(|| settings.tab_label(&tab.resource));
        let tooltip = self.metadata_tab_tooltip(&tab.resource);
        let drag = TabDrag {
            group,
            tab: tab_id,
            label: name.clone().into(),
        };
        let drag_border = self.ui_color("tab.dragAndDropBorder");
        let drag_bg = self.ui_color("tab.hoverBackground");
        let drag_fg = self.ui_color("tab.activeForeground");
        let mut header = div()
            .id(id("editor-tab", group, tab_id))
            .debug_selector(move || format!("editor-tab-{}-{tab_id}", group.0))
            .flex()
            .items_center()
            .gap_1()
            .h(px(settings.tab_height))
            .px_2()
            .cursor_pointer()
            .bg(self.ui_color(bg))
            .border_r_1()
            .border_color(self.ui_color(if last_pin {
                "tab.lastPinnedBorder"
            } else {
                "tab.border"
            }))
            .text_color(self.ui_color(fg))
            .hover(|style| style.bg(self.ui_color("tab.hoverBackground")))
            .drag_over::<TabDrag>(move |style, _, _, _| {
                style.border_l_2().border_color(drag_border)
            })
            .on_drag(drag, move |drag, _, _, cx| {
                cx.new(|_| DragLabel(drag.label.clone(), drag_bg, drag_fg))
            })
            .on_drop(cx.listener(move |v, drag: &TabDrag, _, cx| {
                v.editor
                    .drop_tab(drag.group, drag.tab, group, Some(tab_id), None);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_click(cx.listener(move |v, event: &gpui::ClickEvent, window, cx| {
                v.focus_group_code(group, window, cx);
                v.focus_group = Some(group);
                let preview = v.editor.groups[&group]
                    .tab(tab_id)
                    .is_some_and(|tab| tab.preview);
                v.switch_tab(group, tab_id);
                if event.click_count() == 2 {
                    if preview {
                        v.editor.keep(group, tab_id);
                    } else {
                        match v.editor.settings.double_click.as_str() {
                            "maximize" => v.editor.toggle_maximize(group),
                            "expand" => v.editor.toggle_expand(group),
                            _ => {}
                        }
                    }
                }
                cx.notify();
            }));
        if let Some(text) = tooltip {
            header = header.tooltip(crate::metadata::tooltip::tab_tooltip(
                text,
                self.ui_color("editorHoverWidget.background"),
                self.ui_color("editorHoverWidget.foreground"),
                self.ui_color("editorHoverWidget.border"),
            ));
        }
        let top = if tab.dirty && settings.highlight_modified {
            Some(if selected {
                "tab.activeModifiedBorder"
            } else {
                "tab.inactiveModifiedBorder"
            })
        } else if selected {
            Some(if focused {
                "tab.activeBorderTop"
            } else {
                "tab.unfocusedActiveBorderTop"
            })
        } else {
            None
        };
        if let Some(key) = top {
            header = header.relative().child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(1.0))
                    .bg(self.ui_color(key)),
            );
        }
        if self.shell.zen_tabs().unwrap_or(settings.tabs) == Tabs::Multiple {
            header = match settings.sizing {
                Sizing::Fit => header.flex_shrink_0(),
                Sizing::Shrink => header.min_w(px(54.0)),
                Sizing::Fixed => header
                    .min_w(px(settings.min_width))
                    .w(px(settings.max_width))
                    .flex_shrink_0(),
            };
        }
        let button = div()
            .id(id("tab-action", group, tab_id))
            .flex_shrink_0()
            .px_1()
            .cursor_pointer()
            .hover(|style| style.bg(self.ui_color("tab.activeBackground")))
            .child(self.label(
                if tab.pinned {
                    "editors.pinned"
                } else if tab.dirty {
                    "editors.dirty"
                } else {
                    "editors.closeTab"
                },
                &[],
            ))
            .on_click(cx.listener(move |view, _, _, cx| {
                if view
                    .editor
                    .groups
                    .get(&group)
                    .and_then(|g| g.tab(tab_id))
                    .is_some_and(|tab| tab.pinned)
                {
                    view.editor.pin(group, tab_id, false);
                } else {
                    // #47：有未保存的修改时先确认。
                    view.close_tab_checked(group, tab_id, cx);
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let mut text = div().min_w_0().overflow_hidden().truncate().child(name);
        if tab.preview {
            text = text.italic();
        }
        let mut button = Some(button);
        if settings.actions_left {
            header = header.children(button.take());
        }
        if settings.show_index && self.shell.zen_tabs().unwrap_or(settings.tabs) == Tabs::Multiple {
            let index = pane.tabs.iter().position(|t| t.id == tab_id).unwrap() + 1;
            header = header.child(self.label("editors.index", &[("index", index.to_string())]));
        }
        let show_icons = rove_config::Key::<bool>::new("workbench.editor.showIcons")
            .read(self.settings.as_ref(), &rove_config::Scope::default())
            .unwrap_or(true);
        if show_icons {
            let path = match &tab.resource {
                Resource::Workspace(path) => Some(std::path::Path::new(path.as_str())),
                Resource::File(path) => Some(path.as_path()),
                Resource::Virtual(_) => None,
            };
            if let (Some((path, name)), Some(icons)) = (
                path.and_then(|path| path.file_name().map(|name| (path, name))),
                cx.try_global::<super::theme_host::AppIcons>(),
            ) {
                let language =
                    rove_syntax::highlight::language_for_path(path, Some(self.settings.as_ref()));
                let kind = self.theme_service.current().kind();
                header = header.children(icons.0.element(
                    rove_theme::file_icons::IconTarget::File {
                        name: &name.to_string_lossy(),
                        language: Some(&language),
                    },
                    kind,
                    16.0,
                    || div(),
                ));
            }
        }
        header = header.child(text);
        // #47：磁盘上已被删除的文档（#49 的 `save.label.deleted`），内容仍在编辑器里。
        if self.tab_deleted(tab) {
            header = header.child(
                div()
                    .id(id("tab-deleted", group, tab_id))
                    .flex_shrink_0()
                    .text_color(self.ui_color("tab.inactiveForeground"))
                    .child(self.label("save.label.deleted", &[])),
            );
        }
        // #20：只读文档的标签（单标签模式下即编辑器标题）显示锁形图标（UX_SPEC §3.4）。
        if self.tab_read_only(tab) {
            header = header.child(
                div()
                    .id(id("tab-lock", group, tab_id))
                    .flex_shrink_0()
                    .child(crate::shell::view::icon_label(
                        "$(lock-small)",
                        px(12.0 * self.shell.zoom()),
                    )),
            );
        }
        if settings.badges && matches!(tab.content, Content::Failed(_)) {
            header = header.child(self.label("editors.errorBadge", &[]));
        }
        header = header.children(button);
        header.into_any_element()
    }

    /// #28：打开了文件夹、还没有打开任何文件时的快捷键水印。
    fn watermark(&self, cx: &App) -> Div {
        let label = |key: &str| self.label(key, &[]);
        let keys = |command: &str| crate::keys::key_label(command, cx);
        let lines = crate::welcome::view::watermark_lines(&label, &keys);
        let mut body = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .text_color(self.ui_color("tab.inactiveForeground"));
        for line in lines {
            body = body.child(line);
        }
        body
    }

    pub(super) fn centered(&self, text: String) -> Div {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .text_color(self.ui_color("tab.inactiveForeground"))
            .child(text)
    }

    fn error_page(
        &self,
        group: EditorGroupId,
        tab: &Tab,
        error: &Error,
        cx: &mut Context<Self>,
    ) -> Div {
        let tab_id = tab.id;
        let key = error_page_key(error, &tab.resource);
        // #209：二进制 / 不支持的文本编码时多一个「仍然打开」（UX_SPEC §3.5）。
        let open_anyway = can_open_anyway(error, &tab.resource).then(|| {
            div()
                .id(id("open-anyway", group, tab_id))
                .cursor_pointer()
                .px_2()
                .py_1()
                .bg(self.ui_color("tab.hoverBackground"))
                .child(self.label("editors.openAnyway", &[]))
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.editor.open_anyway(group, tab_id);
                    cx.notify();
                }))
        });
        let path = match &tab.resource {
            Resource::File(path) => Some(path.clone()),
            Resource::Workspace(path) => Some(self.root.join(path.as_str())),
            Resource::Virtual(_) => None,
        };
        let path = path.map(|path| {
            if key == "editors.fileMissing" {
                path.parent().map_or(path.clone(), PathBuf::from)
            } else {
                path
            }
        });
        let reveal = path.map(|path| {
            div()
                .id(id("show-in-explorer", group, tab_id))
                .cursor_pointer()
                .px_2()
                .py_1()
                .bg(self.ui_color("tab.hoverBackground"))
                .child(self.label("editors.reveal", &[]))
                .on_click(move |_, _, cx| cx.reveal_path(&path))
        });
        div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(self.label(key, &[]))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id(id("retry", group, tab_id))
                            .cursor_pointer()
                            .px_2()
                            .py_1()
                            .bg(self.ui_color("tab.hoverBackground"))
                            .child(self.label("editors.retry", &[]))
                            .on_click(cx.listener(move |v, _, _, cx| {
                                v.editor.retry(group, tab_id);
                                cx.notify();
                            })),
                    )
                    .children(open_anyway)
                    .children(reveal),
            )
    }

    /// The group's code view (#19), which owns scrolling, selection and painting.
    fn code_viewport(&self, group: EditorGroupId, secondary: bool) -> Option<Div> {
        // #21：代码视图由查找条宿主包住（宿主把代码视图作为第一个子元素）。
        let views = self.views.get(&group)?;
        let code = if secondary {
            views.secondary.as_ref()?.find.clone()
        } else {
            views.primary.find.clone()
        };
        Some(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .font_family("Consolas")
                .child(code),
        )
    }
}
