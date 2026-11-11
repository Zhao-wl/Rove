//! 大纲、面包屑与转到符号的窗口状态与绘制部件（GPUI）。由宿主（骨架的 `Workbench`）持有并驱动：
//! 宿主负责取活动文档、在后台执行 [`SymbolService`]、经 `OpenDocuments` 定位；本模块只保存缓存与
//! 界面状态、构建列表数据源和绘制元素。
use super::{
    model::{
        self, BreadcrumbSettings, CollapseItems, Labeler, OutlineSettings, Segment, SymbolTree,
    },
    registry,
};
use crate::{
    documents::EditorGroupId,
    list_widget::{
        matcher::FindMode,
        options::{ListOptions, OpenMode},
        quick_input_view::QuickInputView,
        tree::{Children, TreeModel, TreeSource},
        view::{highlighted_label, hsla, ListView, RowContext, RowRenderer},
        widget::{ListWidget, RowHeight},
    },
    lists::{ItemId, QuickInputId},
    shell::icons,
};
use gpui::{
    div, prelude::*, px, App, Div, Entity, FocusHandle, Hsla, Pixels, SharedString, Window,
};
use rove_base::{Cancellation, DocumentId, Position, Version};
use rove_i18n::{Message, Strings};
use rove_syntax::{
    symbols::{kinds, DocumentSymbols, SymbolId, SymbolService},
    Symbol,
};
use rove_theme::Theme;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

/// 最近使用的文档符号缓存条数（切回已打开的文件时立即显示）。
const CACHE: usize = 32;

/// 大纲小节当前显示的内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// 没有活动的文本编辑器。
    NoEditor,
    /// 语言没有符号提供方（UX_SPEC §6.3）。
    Unsupported,
    /// 超大文件不提供符号（UX_SPEC §3.1）。
    TooLarge,
    /// 第一次分析尚未完成（之后的新版本到达前继续显示旧结果）。
    Loading,
    Ready,
}

/// 一个编辑器组的光标所在符号路径（面包屑与跟随光标用），随光标增量更新。
#[derive(Clone, Debug)]
pub struct CursorPath {
    pub document: DocumentId,
    pub version: Version,
    pub position: Position,
    pub path: Vec<SymbolId>,
}

/// 面包屑选择器（点击一段弹出的同级列表）。
pub struct Picker {
    pub group: EditorGroupId,
    /// 弹出它的段下标。
    pub segment: usize,
    pub list: Entity<ListView>,
    pub items: Vec<PickerItem>,
}

#[derive(Clone, Debug)]
pub enum PickerItem {
    Symbol(SymbolId),
    /// 工作区相对路径与是否是文件。
    Path(Arc<str>, bool),
}

/// 面包屑栏的键盘状态（`breadcrumbsActive`）。
pub struct Bar {
    pub focus: FocusHandle,
    /// 被聚焦的组与段。
    pub group: Option<EditorGroupId>,
    pub focused: usize,
}

/// 转到文件中的符号：正在显示的快速输入。
pub struct Goto {
    pub view: Entity<QuickInputView>,
    pub id: Option<QuickInputId>,
    pub document: Option<DocumentId>,
    pub revision: u64,
    pub _events: rove_base::Subscription,
}

pub struct Outline {
    pub list: Entity<ListView>,
    pub settings: OutlineSettings,
    pub crumbs: BreadcrumbSettings,
    /// 大纲视图菜单里的“跟随光标”开关（不是设置项，UX_SPEC §4.2）。
    pub follow_cursor: bool,
    pub status: Status,
    /// 大纲列表正在显示的文档与版本。
    pub shown: Option<(DocumentId, Arc<DocumentSymbols>)>,
    /// 后台分析中的请求（文档、版本、取消）。
    pub pending: Option<(DocumentId, Version, Cancellation)>,
    pub paths: HashMap<EditorGroupId, CursorPath>,
    pub bar: Bar,
    pub picker: Option<Picker>,
    pub goto: Option<Goto>,
    pub service: SymbolService,
    pub label: Labeler,
    /// 与 `outline.collapseItems` 默认状态相反的节点（用户手动展开/折叠过），按文档保存。
    toggled: HashMap<DocumentId, HashSet<Arc<str>>>,
    cache: Vec<(DocumentId, Arc<DocumentSymbols>)>,
    /// 行渲染器读取当前符号（图标与颜色）。
    current: Rc<RefCell<Option<Arc<DocumentSymbols>>>>,
    /// 分析失败的版本（不再重试，直到文档出现新版本）。
    pub failed: Option<(DocumentId, Version)>,
    /// 跟随光标最近一次选中的路径（文档、符号版本、路径），相同时不重复滚动。
    pub revealed: Option<(DocumentId, Version, Vec<SymbolId>)>,
    /// 面包屑栏或其选择器持有键盘焦点（`breadcrumbsActive`），每帧由宿主更新。
    pub bar_active: bool,
    /// 定位完成后把焦点交给编辑器（大纲回车、选择器与快速输入接受）。
    pub focus_editor: bool,
    /// 选择器换了一层目录：下一帧把焦点交给新列表。
    pub focus_picker: bool,
    /// 最近一次由大纲选择触发的定位（单击同时产生选择与打开事件，只定位一次）。
    pub last_nav: Option<Arc<str>>,
}

/// 标签：YAML 多文档节点显示为“文档 N”，其余是符号名。
pub fn labeler(strings: Arc<dyn Strings>) -> Labeler {
    Arc::new(move |symbol: &Symbol| {
        if rove_syntax::symbols::kind_name(&symbol.kind) == kinds::DOCUMENT {
            let message = Message {
                key: "outline.document".into(),
                args: Arc::from([("index".into(), symbol.name.clone())]),
            };
            strings
                .text(&message)
                .map(Arc::from)
                .unwrap_or_else(|_| symbol.name.clone())
        } else {
            symbol.name.clone()
        }
    })
}

struct NoSymbols;
impl TreeSource for NoSymbols {
    fn children(&self, _: Option<&ItemId>) -> Children {
        Children::Ready(Arc::from([]))
    }
}

/// 大纲列表的选项：在树里直接输入即过滤（VS Code `outline` 的 filterOnType）；单击类名只选中、
/// 由宿主定位，展开/折叠走 twistie 或双击。
pub fn list_options(base: ListOptions) -> ListOptions {
    ListOptions {
        find_mode: FindMode::Filter,
        expand_mode: OpenMode::DoubleClick,
        ..base
    }
}

impl Outline {
    pub fn new(
        theme: Arc<dyn Theme>,
        strings: Arc<dyn Strings>,
        list_base: ListOptions,
        settings: &dyn rove_config::Settings,
        cx: &mut App,
    ) -> Self {
        let current: Rc<RefCell<Option<Arc<DocumentSymbols>>>> = Rc::default();
        let label = labeler(strings.clone());
        let options = list_options(list_base);
        let outline = OutlineSettings::read(settings);
        let renderer = symbol_row(current.clone(), theme.clone(), outline.icons);
        let list = cx.new(|cx| {
            let widget = ListWidget::tree(
                TreeModel::new(Arc::new(NoSymbols)),
                RowHeight::Fixed(22.0),
                options,
            );
            let mut view = ListView::new(widget, theme, strings, cx);
            view.set_renderer(renderer);
            view
        });
        Self {
            list,
            settings: outline,
            crumbs: BreadcrumbSettings::read(settings),
            follow_cursor: true,
            status: Status::NoEditor,
            shown: None,
            pending: None,
            paths: HashMap::new(),
            bar: Bar {
                focus: cx.focus_handle(),
                group: None,
                focused: 0,
            },
            picker: None,
            goto: None,
            service: SymbolService,
            label,
            toggled: HashMap::new(),
            cache: Vec::new(),
            current,
            failed: None,
            revealed: None,
            bar_active: false,
            focus_editor: false,
            focus_picker: false,
            last_nav: None,
        }
    }

    /// 缓存中某文档的最新符号（任意版本）。
    pub fn cached(&self, document: DocumentId) -> Option<&Arc<DocumentSymbols>> {
        self.cache
            .iter()
            .rev()
            .find(|(d, _)| *d == document)
            .map(|(_, s)| s)
    }

    pub fn store(&mut self, document: DocumentId, symbols: Arc<DocumentSymbols>) {
        self.cache.retain(|(d, _)| *d != document);
        self.cache.push((document, symbols));
        if self.cache.len() > CACHE {
            self.cache.remove(0);
        }
    }

    /// 关闭的文档不再保留缓存与展开状态。
    pub fn retain_documents(&mut self, open: &HashSet<DocumentId>) {
        self.cache.retain(|(d, _)| open.contains(d));
        self.toggled.retain(|d, _| open.contains(d));
        self.paths.retain(|_, p| open.contains(&p.document));
    }

    /// 用户展开/折叠了一个节点：记下与默认状态的差异，新版本到达后保留。
    pub fn note_expanded(&mut self, document: DocumentId, key: Arc<str>, expanded: bool) {
        let default = self.settings.collapse == CollapseItems::AlwaysExpand;
        let set = self.toggled.entry(document).or_default();
        if expanded == default {
            set.remove(&key);
        } else {
            set.insert(key);
        }
    }

    /// 把一份符号装进大纲列表；节点 ID 是稳定键，焦点、选择与滚动位置按 ID 保留。
    pub fn show(&mut self, document: DocumentId, symbols: Arc<DocumentSymbols>, cx: &mut App) {
        let same = self
            .shown
            .as_ref()
            .is_some_and(|(d, s)| *d == document && Arc::ptr_eq(s, &symbols));
        self.status = Status::Ready;
        if same {
            return;
        }
        *self.current.borrow_mut() = Some(symbols.clone());
        let tree = SymbolTree::new(
            symbols.clone(),
            self.settings.filter.clone(),
            self.label.clone(),
        );
        let expand_default = self.settings.collapse == CollapseItems::AlwaysExpand;
        let toggled = self.toggled.get(&document).cloned().unwrap_or_default();
        let expand: Vec<ItemId> = tree
            .expandable()
            .into_iter()
            .filter(|id| expand_default != toggled.contains(&id.0))
            .collect();
        let new_document = self.shown.as_ref().is_none_or(|(d, _)| *d != document);
        self.shown = Some((document, symbols));
        self.list.update(cx, |view, cx| {
            view.widget.update_tree(|t| {
                *t = TreeModel::new(Arc::new(tree));
                for id in &expand {
                    let _ = t.set_expanded(id, true);
                }
            });
            if new_document {
                view.widget.set_scroll_top(0.0);
            }
            // 刷新数据不是用户操作，不产生打开/选择事件。
            view.widget.take_events();
            cx.notify();
        });
    }

    /// 清空列表（切到无法提供大纲的编辑器）。
    pub fn clear(&mut self, status: Status, cx: &mut App) {
        self.status = status;
        if self.shown.take().is_some() {
            *self.current.borrow_mut() = None;
            self.list.update(cx, |view, cx| {
                view.widget
                    .update_tree(|t| *t = TreeModel::new(Arc::new(NoSymbols)));
                view.widget.take_events();
                cx.notify();
            });
        }
    }

    /// 跟随光标：选中并滚动到光标所在的最深可见节点（不产生打开事件）。
    pub fn reveal_path(&mut self, path: &[SymbolId], cx: &mut App) {
        let Some((_, symbols)) = &self.shown else {
            return;
        };
        let Some(target) = model::deepest_visible(symbols, &self.settings.filter, path) else {
            return;
        };
        let id = model::item_id(symbols, target);
        let ancestors: Vec<ItemId> = symbols
            .ancestors(target)
            .into_iter()
            .take_while(|a| *a != target)
            .map(|a| model::item_id(symbols, a))
            .collect();
        self.list.update(cx, |view, cx| {
            if view.widget.selected_ids().first() == Some(&id) {
                return;
            }
            view.widget.update_tree(|t| {
                for a in &ancestors {
                    let _ = t.set_expanded(a, true);
                }
            });
            if view.widget.focus_id(&id).is_ok() {
                view.widget.select_ids(std::slice::from_ref(&id));
                if let Some(i) = view.widget.focus() {
                    view.widget.reveal(i);
                }
            }
            view.widget.take_events();
            cx.notify();
        });
    }

    /// 面包屑段（文件路径 + 光标所在符号路径），按 `breadcrumbs.*` 设置。
    pub fn segments(
        &self,
        resource: &rove_base::Resource,
        symbols: Option<&DocumentSymbols>,
        path: &[SymbolId],
    ) -> Vec<Segment> {
        let mut out = model::file_segments(resource, self.crumbs.file_path);
        if let Some(symbols) = symbols {
            out.extend(model::symbol_segments(
                symbols,
                &self.crumbs.filter,
                path,
                self.crumbs.symbol_path,
            ));
        }
        out
    }

    /// 设置变化：重读大纲与面包屑设置，过滤或图标变化时重建列表。
    pub fn reload_settings(
        &mut self,
        settings: &dyn rove_config::Settings,
        theme: Arc<dyn Theme>,
        cx: &mut App,
    ) {
        let outline = OutlineSettings::read(settings);
        self.crumbs = BreadcrumbSettings::read(settings);
        if outline != self.settings {
            let icons = outline.icons;
            self.settings = outline;
            let renderer = symbol_row(self.current.clone(), theme, icons);
            self.list.update(cx, |v, _| v.set_renderer(renderer));
            if let Some((document, symbols)) = self.shown.take() {
                self.show(document, symbols, cx);
            }
        }
    }
}

/// codicon 字形。
pub fn codicon(name: &str, size: Pixels, color: Option<Hsla>) -> Div {
    let glyph = icons::codepoint(name)
        .map(|c| c.to_string())
        .unwrap_or_default();
    div()
        .flex_shrink_0()
        .w(size)
        .font_family(icons::FONT_FAMILY)
        .text_size(size)
        .when_some(color, |d, c| d.text_color(c))
        .child(SharedString::from(glyph))
}

/// 符号种类图标（`outline.icons` / `breadcrumbs.icons`）。
pub fn kind_icon(theme: &dyn Theme, symbol: &Symbol, size: Pixels) -> Div {
    let (name, color) = model::icon(&symbol.kind);
    codicon(name, size, theme.color(color).ok().flatten().map(hsla))
}

/// 大纲行：twistie + 种类图标 + 带匹配高亮的名称。
fn symbol_row(
    current: Rc<RefCell<Option<Arc<DocumentSymbols>>>>,
    theme: Arc<dyn Theme>,
    icons: bool,
) -> RowRenderer {
    Rc::new(move |ctx: &RowContext, window: &mut Window, cx: &mut App| {
        if ctx.kind != crate::list_widget::tree::RowKind::Item {
            return crate::list_widget::view::default_row(ctx, window, cx);
        }
        let twistie = if ctx.row.expandable {
            icons::codepoint(if ctx.row.expanded {
                "chevron-down"
            } else {
                "chevron-right"
            })
            .map(|c| c.to_string())
            .unwrap_or_default()
        } else {
            String::new()
        };
        let mut el = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .overflow_hidden()
            .child(
                div()
                    .w(px(16.))
                    .flex_shrink_0()
                    .font_family(icons::FONT_FAMILY)
                    .child(SharedString::from(twistie)),
            );
        if icons {
            let symbols = current.borrow();
            let node = symbols
                .as_ref()
                .and_then(|s| s.find_key(&ctx.row.id.0).and_then(|id| s.node(id)));
            if let Some(node) = node {
                el = el.child(kind_icon(theme.as_ref(), &node.symbol, px(14.)));
            }
        }
        el.child(div().flex_shrink_0().child(highlighted_label(ctx)))
            .into_any_element()
    })
}

/// 小节顶部的工具条按钮：跟随光标、全部折叠。
pub fn toolbar_commands() -> [(&'static str, &'static str); 2] {
    [
        (registry::OUTLINE_FOLLOW_CURSOR, "target"),
        (registry::OUTLINE_COLLAPSE, "collapse-all"),
    ]
}
