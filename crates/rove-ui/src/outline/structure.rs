//! Outline adapter for the lazy structured-document provider.
use crate::{
    list_widget::tree::{ChildCount, Children, TreeItem, TreeSource},
    lists::ItemId,
    outline::model::{Labeler, SymbolFilter},
};
use rove_base::{Cancellation, DocumentId, Position, Resource, Result, Version};
use rove_commands::{Command, CommandRegistry};
use rove_docview::{
    structure::{ParsedStructure, SyntaxProblem},
    StructureNode, StructureNodeId,
};
use rove_i18n::StringTable;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) const SETTING_KEYS: &[&str] = &[
    "problems.visibility",
    "problems.decorations.enabled",
    "problems.autoReveal",
    "problems.sortOrder",
    "problems.defaultViewMode",
    "problems.showCurrentInStatus",
    "outline.problems.enabled",
    "outline.problems.badges",
    "outline.problems.colors",
    "editor.formatOnSave",
    "editor.formatOnSaveMode",
    "editor.formatOnPaste",
    "json.format.enable",
    "json.format.keepLines",
    "json.maxItemsComputed",
];

pub const FORMAT: &str = "editor.action.formatDocument";
pub const FORMAT_NONE: &str = "editor.action.formatDocument.none";
pub const FORMAT_SELECTION: &str = "editor.action.formatSelection";
pub const MINIFY: &str = "rove.json.minify";
pub const COPY_PATH: &str = "rove.structure.copyPath";
pub const COPY_VALUE: &str = "rove.structure.copyValue";
pub const COLLAPSE_SIBLINGS: &str = "rove.structure.collapseSiblings";
pub const NEXT: &str = "editor.action.marker.next";
pub const PREVIOUS: &str = "editor.action.marker.prev";
pub const NEXT_FILES: &str = "editor.action.marker.nextInFiles";
pub const PREVIOUS_FILES: &str = "editor.action.marker.prevInFiles";
pub const CLOSE_MARKER: &str = "closeMarkersNavigation";
pub const COPY_MESSAGE: &str = "problems.action.copyMessage";
pub const WRAP_MESSAGE: &str = "problems.action.showMultilineMessage";
pub const TRUNCATE_MESSAGE: &str = "problems.action.showSinglelineMessage";
pub const FOCUS_PANEL: &str = "workbench.action.problems.focus";
pub const SHOW_PANEL: &str = "workbench.action.showErrorsWarnings";
pub const FOCUS_VIEW: &str = "workbench.panel.markers.view.focus";
pub const RESET_CONTAINER: &str = "workbench.panel.markers.resetViewContainerLocation";
pub const RESET_VIEW: &str = "workbench.panel.markers.view.resetViewLocation";
pub const REMOVE_VIEW: &str = "workbench.panel.markers.view.removeView";
pub const TOGGLE_VIEW: &str = "workbench.panel.markers.view.toggleVisibility";

const REGISTERED_ONLY: &[(&str, &str)] = &[
    ("editor.action.formatDocument.multiple", "structure.format"),
    (
        "editor.action.formatSelection.multiple",
        "structure.formatSelection",
    ),
    ("problems.action.showQuickFixes", "structure.quickFixes"),
    (
        "problems.action.copyRelatedInformationMessage",
        "structure.copyRelatedProblem",
    ),
];

const COMMANDS: &[(&str, &str, &str)] = &[
    (FORMAT, "structure.format", "格式化文档"),
    (
        FORMAT_NONE,
        "structure.noFormatter",
        "当前文档没有可用的格式化工具。",
    ),
    (
        FORMAT_SELECTION,
        "structure.formatSelection",
        "格式化选中内容",
    ),
    (
        COLLAPSE_SIBLINGS,
        "structure.collapseSiblings",
        "折叠同级节点",
    ),
    (NEXT, "structure.next", "转到下一个问题"),
    (PREVIOUS, "structure.previous", "转到上一个问题"),
    (CLOSE_MARKER, "structure.closeMarker", "关闭问题速览"),
    (
        NEXT_FILES,
        "structure.nextAcrossFiles",
        "转到工作区的下一个问题",
    ),
    (
        PREVIOUS_FILES,
        "structure.previousAcrossFiles",
        "转到工作区的上一个问题",
    ),
    (
        "problems.action.clearFilterText",
        "structure.clearProblemsFilter",
        "清除问题筛选",
    ),
    ("problems.action.copy", "structure.copyProblem", "复制问题"),
    (
        "problems.action.focusFilter",
        "structure.focusProblemsFilter",
        "聚焦问题筛选",
    ),
    (
        "problems.action.focusProblemsFromFilter",
        "structure.focusProblems",
        "聚焦问题列表",
    ),
    ("problems.action.open", "structure.openProblem", "打开问题"),
    (
        "problems.action.openToSide",
        "structure.openProblemBeside",
        "在侧边打开问题",
    ),
    (COPY_MESSAGE, "structure.copyProblemMessage", "复制问题消息"),
    (
        WRAP_MESSAGE,
        "structure.wrapProblemMessage",
        "多行显示问题消息",
    ),
    (
        TRUNCATE_MESSAGE,
        "structure.truncateProblemMessage",
        "单行显示问题消息",
    ),
    (FOCUS_PANEL, "structure.focusProblemPanel", "聚焦问题面板"),
    (SHOW_PANEL, "structure.showProblemPanel", "显示错误和警告"),
    (FOCUS_VIEW, "structure.focusProblemView", "聚焦问题视图"),
    (
        RESET_CONTAINER,
        "structure.resetProblemPanel",
        "重置问题面板位置",
    ),
    (RESET_VIEW, "structure.resetProblemView", "重置问题视图位置"),
    (REMOVE_VIEW, "structure.hideProblemView", "隐藏问题视图"),
    (TOGGLE_VIEW, "structure.toggleProblemView", "切换问题视图"),
];
const STRINGS: &[(&str, &str)] = &[
    ("structure.formatting", "正在格式化，按 Esc 取消。"),
    ("structure.markerTitle", "语法问题"),
    ("structure.problemsFilter", "筛选问题"),
    ("structure.quickFixes", "显示快速修复"),
    ("structure.copyRelatedProblem", "复制相关问题消息"),
    (
        "structure.settingsProblem",
        "无法读取设置。请检查文件语法。",
    ),
    (
        "structure.settingsWarning",
        "设置值类型不匹配。请检查该项的类型。",
    ),
    ("structure.formatInvalid", "格式化失败。请先修正语法错误。"),
    (
        "structure.unexpectedSyntax",
        "无法识别这段语法。请检查标点和分隔符。",
    ),
    ("structure.missingSyntax", "语法不完整。请补齐缺失的符号。"),
    (
        "structure.jsonComment",
        "JSON 不允许注释。请删除注释或切换到 JSONC。",
    ),
    ("structure.jsonRoot", "JSON 需要一个根值。请检查文档内容。"),
    (
        "structure.mismatchedXmlTag",
        "XML 的结束标签与起始标签不一致。",
    ),
    (
        "structure.duplicateXmlAttribute",
        "同一 XML 元素出现了重名属性。",
    ),
];

pub fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    for (id, key, _) in COMMANDS {
        registry.register(
            *id,
            Command {
                title_key: (*key).into(),
                category_key: None,
            },
        )?;
    }
    for (id, key) in REGISTERED_ONLY {
        registry.register(
            *id,
            Command {
                title_key: (*key).into(),
                category_key: None,
            },
        )?;
    }
    Ok(())
}

pub fn register_strings(table: &mut StringTable) -> Result<()> {
    for (key, label) in STRINGS
        .iter()
        .copied()
        .chain(COMMANDS.iter().map(|(_, k, s)| (*k, *s)))
    {
        table.register(key, Arc::from(label))?;
    }
    Ok(())
}

pub fn commands() -> impl Iterator<Item = &'static str> {
    COMMANDS
        .iter()
        .map(|(id, _, _)| *id)
        .chain([MINIFY, COPY_PATH, COPY_VALUE])
}

pub struct StructureTree {
    roots: Arc<[TreeItem]>,
}

impl StructureTree {
    pub fn new(roots: Arc<[TreeItem]>) -> Self {
        Self { roots }
    }
}

impl TreeSource for StructureTree {
    fn children(&self, parent: Option<&ItemId>) -> Children {
        if parent.is_some() {
            Children::Pending
        } else {
            Children::Ready(self.roots.clone())
        }
    }
}

pub fn item_id(node: StructureNodeId) -> ItemId {
    ItemId(format!("structured:{}", node.0).into())
}

pub fn node_id(item: &ItemId) -> Option<StructureNodeId> {
    item.0
        .strip_prefix("structured:")?
        .parse()
        .ok()
        .map(StructureNodeId)
}

pub fn items(nodes: &[StructureNode], filter: &SymbolFilter, labeler: &Labeler) -> Arc<[TreeItem]> {
    nodes
        .iter()
        .filter(|node| filter.shows(&node.symbol.kind))
        .map(|node| TreeItem {
            id: item_id(node.id),
            label: labeler(&node.symbol),
            detail: (!node.value_preview.is_empty()).then(|| node.value_preview.clone()),
            children: match node.children.as_deref() {
                None => ChildCount::Unknown,
                Some([]) => ChildCount::None,
                Some(children) => ChildCount::Known(children.len()),
            },
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct RowsStyle {
    pub symbols: BTreeMap<u64, rove_syntax::Symbol>,
    pub problems: Arc<[SyntaxProblem]>,
    pub badges: bool,
    pub colors: bool,
}

impl RowsStyle {
    pub fn count(&self, symbol: &rove_syntax::Symbol) -> usize {
        let start = self
            .problems
            .partition_point(|p| p.range.start() < symbol.range.start());
        let end = self
            .problems
            .partition_point(|p| p.range.start() < symbol.range.end());
        end.saturating_sub(start)
    }
}

pub struct Entry {
    pub snapshot: ParsedStructure,
    pub limit: usize,
    pub resource: Resource,
    pub problems: Vec<SyntaxProblem>,
}

pub struct Marker {
    pub document: DocumentId,
    pub version: Version,
    pub position: Position,
    pub kind: rove_docview::structure::ProblemKind,
    pub code: gpui::Entity<crate::code_view::CodeView>,
}

#[derive(Default)]
pub struct State {
    pub current: Option<(DocumentId, Version)>,
    pub failed: Option<(DocumentId, Version)>,
    pub pending: Option<(DocumentId, Version, Cancellation)>,
    pub replacement_parses: BTreeMap<DocumentId, (Version, Cancellation)>,
    pub documents: BTreeMap<DocumentId, Entry>,
    pub diagnostics: BTreeMap<DocumentId, (Version, Cancellation)>,
    pub nodes: BTreeMap<u64, StructureNode>,
    pub expansions: BTreeMap<u64, Cancellation>,
    pub expanded_paths: BTreeMap<DocumentId, std::collections::BTreeSet<Arc<str>>>,
    pub path: Option<(DocumentId, Version, Position, Arc<str>)>,
    pub path_request: Option<(DocumentId, Version, Position)>,
    pub path_cancel: Option<Cancellation>,
    pub edits: BTreeMap<DocumentId, rove_base::TextEdit>,
    pub context_menu: Option<StructureNodeId>,
    pub pending_marker: Option<(Resource, Position, rove_docview::structure::ProblemKind)>,
    pub marker: Option<Marker>,
    pub format: Option<Cancellation>,
    pub format_generation: u64,
    pub save_format_generation: u64,
    pub save_formats: BTreeMap<DocumentId, (u64, Cancellation)>,
}
