//! 资源管理器（#23）在骨架窗口中的装配：视图创建、工作区与文件事件、经 #17 的 `OpenDocuments`
//! 打开文件（`OpenSource::Explorer`）、活动编辑器变化时定位当前文件、命令转发、焦点交接。
use super::*;
use crate::{
    documents::OpenTarget,
    explorer::{
        registry::{self, ExplorerAction},
        view::{ExplorerEvent, ExplorerView},
    },
};

impl Workbench {
    pub(super) fn new_explorer(
        theme: Arc<dyn Theme>,
        settings: &Arc<dyn Settings>,
        cx: &mut Context<Self>,
    ) -> Entity<ExplorerView> {
        let strings: Arc<dyn Strings> = Arc::new(appearance::strings().unwrap());
        let view = cx.new(|cx| ExplorerView::new(theme, strings, &**settings, cx));
        cx.subscribe(&view, |this, _, event: &ExplorerEvent, cx| {
            this.explorer_event(event, cx)
        })
        .detach();
        view
    }

    /// 工作区的根目录已可用（或重新扫描后换了新的）：资源管理器立即显示它，并订阅它的文件事件。
    /// 展开状态与选中项在重新扫描时按路径保留。
    pub(super) fn show_workspace(
        &mut self,
        workspace: &Arc<dyn Workspace>,
        cx: &mut Context<Self>,
    ) {
        let (send, receive) = async_channel::unbounded();
        self.explorer_events = Some(workspace.changes().subscribe(Arc::new(move |batch| {
            let _ = send.try_send(batch);
        })));
        cx.spawn(async move |this, cx| {
            while let Ok(first) = receive.recv().await {
                // 事件风暴（批量增删）合并成一次刷新。
                let mut events = first.events.to_vec();
                while let Ok(more) = receive.try_recv() {
                    events.extend(more.events.iter().cloned());
                }
                let alive = this
                    .update(cx, |view, cx| {
                        view.explorer
                            .update(cx, |explorer, cx| explorer.files_changed(&events, cx));
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
        // 先订阅再读快照：订阅之前发出的事件已经在快照里，之后的事件由订阅送到。
        self.explorer.update(cx, |explorer, cx| {
            explorer.set_workspace(Some(workspace.clone()), cx)
        });
    }

    pub(super) fn explorer_settings_changed(
        &mut self,
        keys: &BTreeSet<Arc<str>>,
        cx: &mut Context<Self>,
    ) {
        if ExplorerView::cares_about(keys) {
            let settings = self.settings.clone();
            self.explorer
                .update(cx, |explorer, cx| explorer.reload_settings(&*settings, cx));
        }
    }

    /// 资源管理器要求宿主做的事：打开文件，或执行空状态按钮的命令。
    fn explorer_event(&mut self, event: &ExplorerEvent, cx: &mut Context<Self>) {
        match event {
            ExplorerEvent::Open {
                path,
                preserve_focus,
                pinned,
                side,
            } => {
                let _ = crate::feedback::FeedbackService::dismiss(
                    &mut self.shell.feedback,
                    "command.placeholder",
                );
                // 一律经 #17 的打开文档服务；`.md` 的编辑器类型由它按来源解析（D6：从文件树进入预览）。
                let options = OpenOptions {
                    source: OpenSource::Explorer,
                    target: if *side {
                        OpenTarget::Beside
                    } else {
                        OpenTarget::Active
                    },
                    preview_tab: !*pinned,
                    preserve_focus: *preserve_focus || self.editor.settings.explorer_preserve_focus,
                    ..Default::default()
                };
                if let Err(error) = self.open_documents().open(
                    Resource::Workspace(path.clone()),
                    options,
                    Cancellation::default(),
                ) {
                    eprintln!("open {}: {error}", path.as_str());
                }
                cx.notify();
            }
            ExplorerEvent::Command(command) => self.command(command, cx),
        }
    }

    /// 活动标签对应的工作区文件。
    fn active_workspace_path(&self) -> Option<(TabId, WorkspacePath)> {
        let tab = self.editor.active().current()?;
        match &tab.resource {
            Resource::Workspace(path) => Some((tab.id, path.clone())),
            _ => None,
        }
    }

    /// 键位服务分发来的资源管理器命令（焦点不在树里时，例如编辑器里的“在资源管理器中显示活动文件”）。
    pub(super) fn explorer_command(
        &mut self,
        action: &ExplorerAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action.0 {
            registry::SHOW_ACTIVE_FILE => {
                let Some((_, path)) = self.active_workspace_path() else {
                    return;
                };
                shell::view::run_command(self, "workbench.view.explorer", window, cx);
                self.explorer
                    .update(cx, |explorer, cx| explorer.reveal(&path, true, cx));
                cx.notify();
            }
            id => self
                .explorer
                .update(cx, |explorer, cx| explorer.command(id, cx)),
        }
    }

    /// 每帧：活动编辑器换了就在树中定位（`explorer.autoReveal`）；显示资源管理器后把焦点交给树。
    pub(super) fn sync_explorer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active_workspace_path();
        let tab = active.as_ref().map(|(tab, _)| *tab);
        if tab != self.explorer_revealed {
            self.explorer_revealed = tab;
            if let Some((_, path)) = active {
                self.explorer
                    .update(cx, |explorer, cx| explorer.auto_reveal(&path, cx));
            }
        }
        let visible = self.shell.layout.sidebar_visible
            && self.shell.active_container() == crate::shell::catalog::EXPLORER;
        if visible && self.shell.sidebar_focus().is_focused(window) {
            let focus = self.explorer.focus_handle(cx);
            window.focus(&focus);
        }
    }
}
