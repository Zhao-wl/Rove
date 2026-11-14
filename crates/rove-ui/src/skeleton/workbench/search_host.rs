//! 全局搜索（#25）在骨架窗口中的装配：搜索视图的创建与绘制、命令转发、打开结果、全局上下文键。
//! 打开与定位一律经 #17 的 `OpenDocuments`（`OpenSource::Search`，带选区：`.md` 也打开源码）。
use super::*;
use crate::{
    documents::OpenTarget,
    search::{
        registry::{context as ck, SearchAction, SEARCH_VIEW},
        view::{OpenRequest, SearchView, SearchViewEvent},
    },
};
use rove_commands::ContextKeys;

impl Workbench {
    pub(super) fn new_search(
        root: PathBuf,
        settings: Arc<dyn Settings>,
        theme: Arc<dyn Theme>,
        cx: &mut Context<Self>,
    ) -> Entity<SearchView> {
        let strings: Arc<dyn Strings> = Arc::new(appearance::strings().unwrap());
        let view = cx.new(|cx| SearchView::new(root, settings, theme, strings, cx));
        cx.subscribe(&view, |this, _, event: &SearchViewEvent, cx| {
            this.search_event(event, cx)
        })
        .detach();
        view
    }

    /// 搜索视图的小节内容。
    pub(super) fn render_search(&mut self) -> AnyElement {
        self.search.clone().into_any_element()
    }

    /// 键位服务分发来的 `search.action.*` / `toggleSearch*`：焦点在编辑器里时（`f4`）也走这里。
    pub(super) fn search_command(
        &mut self,
        action: &SearchAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = action.0;
        self.search
            .update(cx, |search, cx| search.command(id, window, cx));
    }

    /// 每帧：发布 `searchViewletVisible` / `hasSearchResult`；`findInFiles` 或点击活动栏把焦点交给侧边栏后，
    /// 再交给查询框（并用编辑器的单行选中内容作为查询）。
    pub(super) fn sync_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let visible = self.shell.layout.sidebar_visible
            && self.shell.active_container() == crate::shell::catalog::SEARCH;
        let has_results = self.search.read(cx).has_results();
        if let Some(keys) = cx.try_global::<crate::keys::Keys>() {
            let _ = keys
                .context
                .set(ck::VIEWLET_VISIBLE.into(), Some(Value::Bool(visible)));
            let _ = keys
                .context
                .set(ck::HAS_RESULT.into(), Some(Value::Bool(has_results)));
        }
        if visible && self.shell.sidebar_focus().is_focused(window) {
            let seed = self.code_view().read(cx).selected_text();
            self.search.update(cx, |search, cx| {
                if let Some(text) = seed {
                    search.seed(&text, cx);
                }
                search.focus_query(window, cx);
            });
        }
    }

    pub(super) fn search_settings_changed(
        &mut self,
        keys: &BTreeSet<Arc<str>>,
        cx: &mut Context<Self>,
    ) {
        if keys
            .iter()
            .any(|k| k.starts_with("search.") || k.starts_with("workbench.list."))
        {
            self.search.update(cx, |s, cx| s.reload_settings(cx));
        }
    }

    pub(super) fn search_theme_changed(&mut self, cx: &mut Context<Self>) {
        let theme = self.theme.clone();
        self.search.update(cx, |s, cx| s.set_theme(theme, cx));
    }

    fn search_event(&mut self, event: &SearchViewEvent, cx: &mut Context<Self>) {
        match event {
            SearchViewEvent::Open(request) => self.open_search_result(request, cx),
            // #52：打开文件后把每一处匹配变成一个光标。
            SearchViewEvent::AddCursors(request) => self.add_cursors_at_results(request, cx),
            SearchViewEvent::OpenSettings => {
                // Rove 没有设置界面：打开共用的 VS Code settings.json（排除规则就在其中）。
                let Some(path) = crate::keys::user_keybindings_path(std::env::args_os().skip(1))
                    .map(|p| p.with_file_name(rove_config::paths::SETTINGS_FILE))
                else {
                    return;
                };
                let options = OpenOptions {
                    source: OpenSource::Search,
                    ..Default::default()
                };
                if let Err(e) = self.open_documents().open(
                    Resource::File(path),
                    options,
                    Cancellation::default(),
                ) {
                    eprintln!("search: {e}");
                }
                cx.notify();
            }
        }
    }

    /// #52：`addCursorsAtSearchResults`——打开目标文件，再把全部匹配写成多选区。
    pub(super) fn add_cursors_at_results(
        &mut self,
        request: &crate::search::view::AddCursorsRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(first) = request.selections.first().cloned() else {
            return;
        };
        self.open_search_result(
            &OpenRequest {
                path: request.path.clone(),
                selection: first,
                side: false,
                preserve_focus: false,
                pinned: true,
            },
            cx,
        );
        // 打开是异步的，而且显示出来时 #17 会按 `OpenOptions::selection` 放一个单选区；
        // 多光标留到那之后再写（见 `Workbench::sync_pending_cursors`）。
        self.pending_cursors = Some((
            Resource::Workspace(request.path.clone()),
            request.selections.clone(),
        ));
        cx.notify();
    }

    /// 打开并定位（`OpenOptions::selection`）已经应用到代码视图之后调用：把全部匹配写成多选区。
    /// 目标文件还不是活动标签时什么都不做，等它显示出来的那一帧。
    pub(super) fn apply_pending_cursors(&mut self, cx: &mut Context<Self>) {
        let Some((resource, ranges)) = self.pending_cursors.clone() else {
            return;
        };
        let shown = self
            .editor
            .active()
            .current()
            .map(|tab| tab.resource.clone());
        if shown.as_ref() != Some(&resource) {
            return;
        }
        let code = self.code_view().clone();
        if code.read(cx).text().is_none() {
            return;
        }
        self.pending_cursors = None;
        let selections: Vec<crate::code_view::Selection> = ranges
            .iter()
            .map(|r| crate::code_view::Selection::new(r.start(), r.end()))
            .collect();
        code.update(cx, |code, cx| {
            code.set_selections(
                selections,
                Some(crate::code_view::Reveal::CenterIfOutside),
                cx,
            );
        });
    }

    pub(super) fn open_search_result(&mut self, request: &OpenRequest, cx: &mut Context<Self>) {
        let options = OpenOptions {
            source: OpenSource::Search,
            target: if request.side {
                OpenTarget::Beside
            } else {
                OpenTarget::Active
            },
            preview_tab: !request.pinned,
            preserve_focus: request.preserve_focus,
            selection: Some(request.selection.clone()),
            ..Default::default()
        };
        if let Err(e) = self.open_documents().open(
            Resource::Workspace(request.path.clone()),
            options,
            Cancellation::default(),
        ) {
            eprintln!("search: {e}");
        }
        cx.notify();
    }
}

/// 视图工厂 key（与 `KNOWN_FACTORIES` 一起做装配校验）。
pub(super) const SEARCH_FACTORY: &str = SEARCH_VIEW;
