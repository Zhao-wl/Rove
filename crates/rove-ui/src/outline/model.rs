//! 大纲与面包屑的数据层（不依赖 gpui）：设置读取、按种类过滤、树数据源、面包屑段与选择器条目。
use crate::{
    list_widget::tree::{ChildCount, Children, TreeItem, TreeSource},
    lists::ItemId,
};
use rove_base::{Resource, Value};
use rove_config::{Scope, Settings};
use rove_syntax::{
    symbols::{kind_name, kinds, DocumentSymbols, SymbolId},
    Symbol, SymbolKind,
};
use std::{collections::BTreeSet, sync::Arc};

/// `outline.show*` / `breadcrumbs.show*` 的后缀与符号种类名（[`kind_name`]）的对应。
pub const SHOW_SETTINGS: [(&str, &str); 26] = [
    ("file", "Files"),
    ("module", "Modules"),
    ("namespace", "Namespaces"),
    ("package", "Packages"),
    ("class", "Classes"),
    ("method", "Methods"),
    ("property", "Properties"),
    ("field", "Fields"),
    ("constructor", "Constructors"),
    ("enum", "Enums"),
    ("interface", "Interfaces"),
    ("function", "Functions"),
    ("variable", "Variables"),
    ("constant", "Constants"),
    ("string", "Strings"),
    ("number", "Numbers"),
    ("boolean", "Booleans"),
    ("array", "Arrays"),
    ("object", "Objects"),
    ("key", "Keys"),
    ("null", "Null"),
    ("enumMember", "EnumMembers"),
    ("struct", "Structs"),
    ("event", "Events"),
    ("operator", "Operators"),
    ("typeParameter", "TypeParameters"),
];

fn read_bool(settings: &dyn Settings, key: &str, default: bool) -> bool {
    match settings.read(key, &Scope::default()) {
        Ok(Value::Bool(b)) => b,
        _ => default,
    }
}

fn read_string(settings: &dyn Settings, key: &str) -> Option<Arc<str>> {
    match settings.read(key, &Scope::default()) {
        Ok(Value::String(s)) => Some(s),
        _ => None,
    }
}

/// 被 `*.show*: false` 隐藏的种类。隐藏一个符号也隐藏它的子树（与 VS Code 的大纲过滤一致）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SymbolFilter {
    hidden: BTreeSet<&'static str>,
}

impl SymbolFilter {
    /// `prefix` 是 `outline` 或 `breadcrumbs`。
    pub fn read(settings: &dyn Settings, prefix: &str) -> Self {
        let hidden = SHOW_SETTINGS
            .iter()
            .filter(|(_, suffix)| !read_bool(settings, &format!("{prefix}.show{suffix}"), true))
            .map(|(kind, _)| *kind)
            .collect();
        Self { hidden }
    }

    pub fn hide(mut self, kind: &'static str) -> Self {
        self.hidden.insert(kind);
        self
    }

    pub fn shows(&self, kind: &SymbolKind) -> bool {
        !self.hidden.contains(kind_name(kind))
    }
}

/// `outline.collapseItems`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollapseItems {
    AlwaysExpand,
    AlwaysCollapse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineSettings {
    pub collapse: CollapseItems,
    pub icons: bool,
    pub filter: SymbolFilter,
}

impl OutlineSettings {
    pub fn read(settings: &dyn Settings) -> Self {
        Self {
            collapse: match read_string(settings, "outline.collapseItems").as_deref() {
                Some("alwaysCollapse") => CollapseItems::AlwaysCollapse,
                _ => CollapseItems::AlwaysExpand,
            },
            icons: read_bool(settings, "outline.icons", true),
            filter: SymbolFilter::read(settings, "outline"),
        }
    }
}

/// `breadcrumbs.filePath` / `breadcrumbs.symbolPath`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathMode {
    On,
    Off,
    Last,
}

/// `breadcrumbs.symbolSortOrder`：选择器里同级符号的顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Position,
    Name,
    Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreadcrumbSettings {
    pub enabled: bool,
    pub file_path: PathMode,
    pub symbol_path: PathMode,
    pub icons: bool,
    pub filter: SymbolFilter,
    pub sort: SortOrder,
    pub separator: Arc<str>,
}

impl BreadcrumbSettings {
    pub fn read(settings: &dyn Settings) -> Self {
        let mode = |key| match read_string(settings, key).as_deref() {
            Some("off") => PathMode::Off,
            Some("last") => PathMode::Last,
            _ => PathMode::On,
        };
        Self {
            enabled: read_bool(settings, "breadcrumbs.enabled", true),
            file_path: mode("breadcrumbs.filePath"),
            symbol_path: mode("breadcrumbs.symbolPath"),
            icons: read_bool(settings, "breadcrumbs.icons", true),
            filter: SymbolFilter::read(settings, "breadcrumbs"),
            sort: match read_string(settings, "breadcrumbs.symbolSortOrder").as_deref() {
                Some("name") => SortOrder::Name,
                Some("type") => SortOrder::Type,
                _ => SortOrder::Position,
            },
            separator: read_string(settings, "breadcrumbs.symbolPathSeparator")
                .unwrap_or_else(|| ".".into()),
        }
    }
}

/// codicon 名称与 `symbolIcon.*` 颜色键（均为已登记的 VS Code 键）。
pub fn icon(kind: &SymbolKind) -> (&'static str, &'static str) {
    match kind_name(kind) {
        "namespace" => ("json", "symbolIcon.namespaceForeground"),
        "module" => ("json", "symbolIcon.moduleForeground"),
        "package" => ("json", "symbolIcon.packageForeground"),
        "object" => ("json", "symbolIcon.objectForeground"),
        "class" => ("symbol-class", "symbolIcon.classForeground"),
        "struct" => ("symbol-structure", "symbolIcon.structForeground"),
        "interface" => ("symbol-interface", "symbolIcon.interfaceForeground"),
        "enum" => ("symbol-enum", "symbolIcon.enumeratorForeground"),
        kinds::ENUM_MEMBER => (
            "symbol-enum-member",
            "symbolIcon.enumeratorMemberForeground",
        ),
        "method" => ("symbol-method", "symbolIcon.methodForeground"),
        "function" => ("symbol-method", "symbolIcon.functionForeground"),
        "constructor" => ("symbol-method", "symbolIcon.constructorForeground"),
        "property" => ("symbol-property", "symbolIcon.propertyForeground"),
        "field" => ("symbol-field", "symbolIcon.fieldForeground"),
        "variable" => ("symbol-variable", "symbolIcon.variableForeground"),
        "parameter" | "typeParameter" => ("symbol-parameter", "symbolIcon.typeParameterForeground"),
        "event" => ("symbol-event", "symbolIcon.eventForeground"),
        "constant" => ("symbol-constant", "symbolIcon.constantForeground"),
        "array" => ("symbol-array", "symbolIcon.arrayForeground"),
        "key" => ("symbol-key", "symbolIcon.keyForeground"),
        kinds::STRING => ("symbol-key", "symbolIcon.stringForeground"),
        kinds::NUMBER => ("symbol-numeric", "symbolIcon.numberForeground"),
        kinds::BOOLEAN => ("symbol-boolean", "symbolIcon.booleanForeground"),
        kinds::NULL => ("symbol-boolean", "symbolIcon.nullForeground"),
        kinds::OPERATOR => ("symbol-operator", "symbolIcon.operatorForeground"),
        kinds::DOCUMENT => ("file", "symbolIcon.fileForeground"),
        _ => ("symbol-misc", "symbolIcon.textForeground"),
    }
}

/// 全部图标颜色键（装配测试校验它们都已登记）。
pub fn icon_color_keys() -> impl Iterator<Item = &'static str> {
    SHOW_SETTINGS
        .iter()
        .map(|(k, _)| *k)
        .chain([kinds::DOCUMENT, "parameter", "unknown"])
        .map(|k| icon(&kind_from_name(k)).1)
}

fn kind_from_name(name: &str) -> SymbolKind {
    match name {
        "namespace" => SymbolKind::Namespace,
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "interface" => SymbolKind::Interface,
        "enum" => SymbolKind::Enum,
        "method" => SymbolKind::Method,
        "function" => SymbolKind::Function,
        "property" => SymbolKind::Property,
        "field" => SymbolKind::Field,
        "variable" => SymbolKind::Variable,
        "parameter" => SymbolKind::Parameter,
        "typeParameter" => SymbolKind::TypeParameter,
        "event" => SymbolKind::Event,
        "constructor" => SymbolKind::Constructor,
        "constant" => SymbolKind::Constant,
        "module" => SymbolKind::Module,
        "object" => SymbolKind::Object,
        "array" => SymbolKind::Array,
        "key" => SymbolKind::Key,
        other => SymbolKind::Other(other.into()),
    }
}

/// 显示名称：YAML 多文档节点（名称是序号）由界面用登记的字符串格式化。
pub type Labeler = Arc<dyn Fn(&Symbol) -> Arc<str> + Send + Sync>;

/// 大纲树的数据源：节点 ID 是符号的稳定键，所以新版本到达后展开状态与选择得以保留。
pub struct SymbolTree {
    symbols: Arc<DocumentSymbols>,
    filter: SymbolFilter,
    label: Labeler,
}

impl SymbolTree {
    pub fn new(symbols: Arc<DocumentSymbols>, filter: SymbolFilter, label: Labeler) -> Self {
        Self {
            symbols,
            filter,
            label,
        }
    }

    pub fn visible_children(&self, parent: Option<SymbolId>) -> Vec<SymbolId> {
        visible_children(&self.symbols, &self.filter, parent)
    }

    /// 可见的子孙（含自身），用于全部展开。
    pub fn expandable(&self) -> Vec<ItemId> {
        let mut out = Vec::new();
        let mut stack = self.visible_children(None);
        while let Some(id) = stack.pop() {
            let children = self.visible_children(Some(id));
            if !children.is_empty() {
                out.push(item_id(&self.symbols, id));
                stack.extend(children);
            }
        }
        out
    }
}

pub fn visible_children(
    symbols: &DocumentSymbols,
    filter: &SymbolFilter,
    parent: Option<SymbolId>,
) -> Vec<SymbolId> {
    symbols
        .children(parent)
        .iter()
        .copied()
        .filter(|id| {
            symbols
                .node(*id)
                .is_some_and(|n| filter.shows(&n.symbol.kind))
        })
        .collect()
}

pub fn item_id(symbols: &DocumentSymbols, id: SymbolId) -> ItemId {
    ItemId(symbols.node(id).map(|n| n.key.clone()).unwrap_or_default())
}

impl TreeSource for SymbolTree {
    fn children(&self, parent: Option<&ItemId>) -> Children {
        let parent = match parent {
            None => None,
            Some(p) => match self.symbols.find_key(&p.0) {
                Some(id) => Some(id),
                None => return Children::Ready(Arc::from([])),
            },
        };
        Children::Ready(
            self.visible_children(parent)
                .into_iter()
                .map(|id| {
                    let node = self.symbols.node(id).expect("child id");
                    let n = self.visible_children(Some(id)).len();
                    TreeItem {
                        id: ItemId(node.key.clone()),
                        label: (self.label)(&node.symbol),
                        detail: None,
                        children: if n == 0 {
                            ChildCount::None
                        } else {
                            ChildCount::Known(n)
                        },
                    }
                })
                .collect(),
        )
    }
}

/// 光标路径中最深的可见符号（大纲“跟随光标”选中它；被隐藏的种类退到最近的可见祖先）。
pub fn deepest_visible(
    symbols: &DocumentSymbols,
    filter: &SymbolFilter,
    path: &[SymbolId],
) -> Option<SymbolId> {
    let mut last = None;
    for id in path {
        match symbols.node(*id) {
            Some(n) if filter.shows(&n.symbol.kind) => last = Some(*id),
            _ => break,
        }
    }
    last
}

// ------------------------------------------------------------------ 面包屑

/// 一个面包屑段。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    /// 文件路径的一段；`dir` 是这一段所在的目录（工作区相对路径，根为空串），选择器列出该目录的条目。
    File {
        label: Arc<str>,
        dir: Arc<str>,
        is_file: bool,
    },
    Symbol(SymbolId),
}

/// 文件路径段：工作区文件按 `/` 拆分（文件夹名 + 文件名）；外部文件只显示文件名。
pub fn file_segments(resource: &Resource, mode: PathMode) -> Vec<Segment> {
    // #42：元数据视图的文件路径段只有程序集名（`UnityEngine.CoreModule › UnityEngine › Transform › position`）；
    // 目录 `rove-metadata:` 不是合法的工作区路径，选择器因此没有可列的条目。
    if let Some(key) = rove_lang_cs::metadata_view::view_key(resource) {
        let segment = Segment::File {
            label: rove_lang_cs::metadata_view::assembly_of(key).into(),
            dir: rove_lang_cs::metadata_view::VIEW_SCHEME.into(),
            is_file: true,
        };
        return match mode {
            PathMode::Off => Vec::new(),
            PathMode::On | PathMode::Last => vec![segment],
        };
    }
    let path: String = match resource {
        Resource::Workspace(p) => p.as_str().to_string(),
        Resource::File(p) => p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        Resource::Virtual(p) => p.to_string(),
    };
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let mut out = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        out.push(Segment::File {
            label: (*part).into(),
            dir: parts[..i].join("/").into(),
            is_file: i + 1 == parts.len(),
        });
    }
    match mode {
        PathMode::On => out,
        PathMode::Off => Vec::new(),
        PathMode::Last => out.pop().into_iter().collect(),
    }
}

/// 符号路径段：光标所在路径中可见的符号（被隐藏的种类在路径里截断，与大纲一致）。
pub fn symbol_segments(
    symbols: &DocumentSymbols,
    filter: &SymbolFilter,
    path: &[SymbolId],
    mode: PathMode,
) -> Vec<Segment> {
    let visible: Vec<Segment> = path
        .iter()
        .take_while(|id| {
            symbols
                .node(**id)
                .is_some_and(|n| filter.shows(&n.symbol.kind))
        })
        .map(|id| Segment::Symbol(*id))
        .collect();
    match mode {
        PathMode::On => visible,
        PathMode::Off => Vec::new(),
        PathMode::Last => visible.into_iter().last().into_iter().collect(),
    }
}

/// 选择器：某个符号段的同级符号，按 `breadcrumbs.symbolSortOrder` 排序。
pub fn sibling_symbols(
    symbols: &DocumentSymbols,
    filter: &SymbolFilter,
    of: SymbolId,
    sort: SortOrder,
) -> Vec<SymbolId> {
    let parent = symbols.node(of).and_then(|n| n.parent);
    let mut list = visible_children(symbols, filter, parent);
    let sym = |id: &SymbolId| &symbols.node(*id).expect("sibling").symbol;
    match sort {
        SortOrder::Position => {}
        SortOrder::Name => list.sort_by(|a, b| {
            sym(a)
                .name
                .to_lowercase()
                .cmp(&sym(b).name.to_lowercase())
                .then(a.cmp(b))
        }),
        SortOrder::Type => list.sort_by(|a, b| {
            kind_name(&sym(a).kind)
                .cmp(kind_name(&sym(b).kind))
                .then(sym(a).name.to_lowercase().cmp(&sym(b).name.to_lowercase()))
        }),
    }
    list
}

/// `breadcrumbs.copyPath` 等使用的符号路径文本（`breadcrumbs.symbolPathSeparator` 连接）。
pub fn symbol_path_text(symbols: &DocumentSymbols, path: &[SymbolId], separator: &str) -> String {
    path.iter()
        .filter_map(|id| symbols.node(*id))
        .map(|n| n.symbol.name.as_ref())
        .collect::<Vec<_>>()
        .join(separator)
}
