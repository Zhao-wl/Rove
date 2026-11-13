//! #42 元数据视图在骨架窗口中的装配。
//!
//! 元数据视图是一份普通的虚拟文档（见 `crate::metadata` 的模块文档）：这里只做四件事，其余全部沿用既有实现。
//!
//! 1. **打开**（[`Workbench::open_metadata`]，#40 的接入点）：外部符号 → 符号键 → 视图里的位置 →
//!    经 #17 的 `OpenDocuments` 打开 `rove-metadata:<类型键>`（`OpenSource::CodeNavigation`，带位置）。
//!    同一个类型的资源相同，所以标签复用、只滚动定位、不改变固定状态都是 #17 的既有语义；
//!    找不到元数据时按 UX_SPEC §4.1 末行在状态栏提示，不打开空标签。
//! 2. **刷新**：`rove.metadataView.showDocs` 变了、或索引被原子替换（快照代次变了）时，重新取内容并把新快照
//!    换进所有显示它的标签（[`crate::editors::EditorService::replace_document`]），滚动与光标按行号换算保持。
//! 3. **标签标题与悬停提示**：`Transform [元数据]`、DLL 完整路径。
//! 4. **速览预览里的外部符号片段**：向 #41 的 `ExternalSnippets` 注册提供方。

use super::*;
use crate::documents::OpenTarget;
use crate::metadata::{
    registry::SHOW_DOCS,
    snippets::MetadataSnippets,
    {MetadataLoader, ViewInfo},
};
use crate::skeleton::text::FileDocuments;
use rove_lang_cs::metadata_view::{
    original_line, to_display, view_key, view_resource, MetadataViews,
};
use rove_lang_cs::router::metadata_symbol_key;

/// 窗口内的元数据视图装配：文档加载器（同时是普通文件的加载器）与加载器里的视图来源。
pub(super) struct MetadataHost {
    /// 编辑器服务用的文档加载器：普通文件走磁盘，`rove-metadata:` 走 [`MetadataLoader`]。
    pub(super) files: Arc<FileDocuments>,
    pub(super) loader: Arc<MetadataLoader>,
    /// 上一次已同步给标签的索引快照代次。
    generation: Option<u64>,
}

impl MetadataHost {
    /// 装配时把别的虚拟文档来源串在元数据加载器前面（#22 的超长行临时标签）。
    /// `source` 必须最终把 `rove-metadata:` 转交给 `loader`。
    pub(super) fn with_loader(
        files: FileDocuments,
        loader: Arc<MetadataLoader>,
        source: Arc<dyn crate::skeleton::text::VirtualSource>,
    ) -> Self {
        Self {
            files: Arc::new(files.with_virtual_source(source)),
            loader,
            generation: None,
        }
    }
}

impl Workbench {
    /// `rove.metadataView.showDocs` 的当前值。
    pub(super) fn read_show_docs(settings: &dyn Settings) -> bool {
        !matches!(
            settings.read(SHOW_DOCS, &Scope::default()),
            Ok(Value::Bool(false))
        )
    }

    /// 元数据视图的内容来源（测试与装配用）。
    pub fn metadata_views(&self) -> &Arc<MetadataViews> {
        self.metadata.loader.views()
    }

    /// 装配完成后调用一次：向速览窗口注册外部符号的片段提供方（#41 的接缝）。
    pub(super) fn register_metadata_snippets(&self) {
        self.peek_external_snippets()
            .register(Arc::new(MetadataSnippets::new(
                self.metadata_views().clone(),
            )));
    }

    /// 只有外部符号的目标（#40 的接入点）：打开元数据视图并定位到成员；找不到元数据时状态栏提示。
    pub(super) fn open_metadata_targets(
        &mut self,
        origin_group: EditorGroupId,
        name: &str,
        targets: &[rove_lang_cs::query::Target],
        aside: bool,
        cx: &mut Context<Self>,
    ) {
        let views = self.metadata_views().clone();
        let found = targets
            .iter()
            .filter_map(|t| metadata_symbol_key(&t.resource))
            .find_map(|key| views.locate(key).ok());
        let Some(location) = found else {
            let message = Message {
                key: "nav.noMetadata".into(),
                args: Arc::from([("name".into(), name.into())]),
            };
            let _ = shell::view::report(
                self,
                "nav.status",
                Reason::Receipt,
                message,
                Arc::from([]),
                cx,
            );
            return;
        };
        let position = rove_base::Position {
            line: rove_base::Line(u64::from(location.line)),
            column: rove_base::Utf8Column(u64::from(location.col)),
        };
        let options = OpenOptions {
            target: if aside {
                OpenTarget::Beside
            } else {
                OpenTarget::Group(origin_group)
            },
            source: OpenSource::CodeNavigation,
            location: Some(position),
            ..Default::default()
        };
        if let Err(e) = self.open_documents().open(
            view_resource(&location.type_key),
            options,
            Cancellation::default(),
        ) {
            eprintln!("metadata view: {e}");
        }
        cx.notify();
    }

    /// 标签标题：元数据视图为 `Transform [元数据]`，其他资源为 `None`（由调用方取文件名）。
    pub(super) fn metadata_tab_title(&self, resource: &Resource) -> Option<String> {
        let info = self.metadata.loader.info(resource)?;
        Some(self.label("metadata.tabTitle", &[("name", info.title)]))
    }

    /// 标签悬停提示：DLL 完整路径（没有路径时是程序集名）。
    pub(super) fn metadata_tab_tooltip(&self, resource: &Resource) -> Option<String> {
        let ViewInfo {
            assembly, dll_path, ..
        } = self.metadata.loader.info(resource)?;
        Some(self.label(
            "metadata.tabTooltip",
            &[("path", dll_path.unwrap_or(assembly))],
        ))
    }

    /// 设置变了：`rove.metadataView.showDocs`。
    pub(super) fn metadata_settings_changed(
        &mut self,
        keys: &BTreeSet<Arc<str>>,
        cx: &mut Context<Self>,
    ) {
        if !keys.iter().any(|k| &**k == SHOW_DOCS) {
            return;
        }
        let show = Self::read_show_docs(&*self.settings);
        self.set_show_docs(show, cx);
    }

    /// 切换文档注释的显示：打开着的元数据视图立即重新渲染，不用重开标签。
    pub(super) fn set_show_docs(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.metadata_views().set_show_docs(show) {
            self.refresh_metadata(Some(!show), cx);
        }
    }

    /// 索引快照代次变了（原子替换后）：重新渲染打开着的元数据标签。每帧调用，只比较一个整数。
    pub(super) fn sync_metadata(&mut self, cx: &mut Context<Self>) {
        let generation = self.metadata_views().generation();
        if self.metadata.generation == generation {
            return;
        }
        let first = self.metadata.generation.is_none() && generation.is_some();
        self.metadata.generation = generation;
        // 第一次拿到快照：打开着的视图（索引就绪前不可能有）无需刷新。
        if generation.is_some() && !first {
            self.refresh_metadata(None, cx);
        }
    }

    /// 重新取每个打开着的元数据文档的内容，换进显示它的标签。`previous_show`：这次刷新由 `showDocs` 切换引起时，
    /// 切换前的值，用来把显示着它的代码视图的光标与滚动按“显示行 → 原始行 → 新显示行”换算。
    fn refresh_metadata(&mut self, previous_show: Option<bool>, cx: &mut Context<Self>) {
        let resources: BTreeSet<Resource> = self
            .editor
            .groups
            .values()
            .flat_map(|g| g.tabs.iter())
            .filter(|t| view_key(&t.resource).is_some())
            .map(|t| t.resource.clone())
            .collect();
        let show = self.metadata_views().show_docs();
        for resource in resources {
            let document = match self.metadata.files.reload_virtual(&resource) {
                Ok(Some(document)) => document,
                // 索引里没有了（或没就绪）：保留旧内容，不关标签。
                _ => continue,
            };
            // 显示着这份文档的代码视图：换之前记下光标行与滚动。
            let live: Vec<_> = self
                .views
                .keys()
                .flat_map(|g| [(*g, false), (*g, true)])
                .filter(|(g, secondary)| {
                    self.shown_document(*g, *secondary)
                        .is_some_and(|d| d.resource == resource)
                })
                .filter_map(|(g, secondary)| {
                    let code = self.group_code(g, secondary)?.clone();
                    let (caret, scroll) =
                        code.read_with(cx, |c, _| (c.selection().active, c.scroll_position()));
                    Some((code, caret, scroll))
                })
                .collect();
            self.editor.reload_document(document);
            self.sync_views(cx);
            let (Some(previous), Some(key)) = (previous_show, view_key(&resource)) else {
                continue;
            };
            let Some(original) = self.metadata_views().original_text(key) else {
                continue;
            };
            let map = |line: u64| -> u64 {
                let o = original_line(&original, line as u32, previous);
                u64::from(to_display(&original, o, show))
            };
            for (code, caret, scroll) in live {
                code.update(cx, |c, cx| {
                    let mut at = caret;
                    at.line.0 = map(at.line.0);
                    c.set_selection(Selection::caret(at), None, cx);
                    c.set_scroll_position(
                        ScrollPosition {
                            top: map(scroll.top.max(0.0) as u64) as f64,
                            x: scroll.x,
                        },
                        false,
                        cx,
                    );
                });
            }
        }
        cx.notify();
    }
}
