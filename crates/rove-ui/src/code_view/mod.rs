//! 代码视图（#19）：自定义 GPUI `Element`，f64 滚动、按内容缓存的整形与预取、行号、只读选区、
//! 命中测试与滚动条。
//!
//! # 对外接口速览
//!
//! | 用途 | 入口 |
//! |---|---|
//! | 设置 / 更换文本源 | [`CodeView::set_text`] |
//! | 按行样式区间与整行样式（多提供方叠加） | [`CodeView::add_decorations`]、[`CodeView::invalidate_decorations`]、[`decorations`] |
//! | 选区读写与事件 | [`CodeView::selection`]、[`CodeView::set_selection`]、[`CodeViewEvent::SelectionChanged`] |
//! | 滚动定位（行、行列、居中策略） | [`CodeView::reveal_line`]、[`CodeView::reveal_position`]、[`Reveal`] |
//! | 视口变化事件 | [`CodeViewEvent::ViewportChanged`]、[`CodeView::viewport`] |
//! | 行内覆盖物 / 行下方嵌入区域 | [`decorations::Overlay`]、[`CodeView::set_view_zone`]、[`CodeView::view_zone_bounds`] |
//! | 单行渲染上限挂钩 | [`CodeView::set_line_render_limit`]、[`CodeView::is_line_truncated`] |
//! | 坐标换算（命中测试） | [`CodeView::position_for_point`]、[`CodeView::point_for_position`] |
//! | 滚动条 / 概览标尺坐标 | [`scroll::thumb`]、[`scroll::track_y_for_line`]、[`scroll::line_for_track_y`] |
//! | 命令（VS Code ID） | [`CodeView::execute`]、[`commands::COMMANDS`] |
//! | 语法着色（#20）：快照与变化行 | [`CodeView::set_syntax`]、[`highlight`] |
//! | 折叠（#20）：状态读写、区间、文档行 ↔ 屏幕行映射 | [`CodeView::folds`]、[`CodeView::set_folds`]、[`CodeViewEvent::FoldsChanged`]、[`folding::FoldMap`] |
//! | 括号匹配 / 同名高亮（#20） | [`CodeView::bracket_match`]、[`CodeView::has_word_highlights`] |
//! | 多光标与行操作（#52） | [`CodeView::selections`]、[`CodeView::set_selections`]、[`multicursor`]、[`lineops`] |
//! | 编辑与输入法（#47）：编辑入口、编辑事件、组合串 | [`CodeView::set_edit_host`]、[`edit::EditHost`]、[`CodeViewEvent::Edited`]、[`CodeView::apply_edits`]、[`CodeView::type_text`]、[`CodeView::composition`] |
//! | 只读与文档级别（#20） | [`CodeView::set_read_only`]、[`CodeViewEvent::EditBlocked`]、[`CodeView::set_document_level`]、[`CodeView::disabled_features`] |
//! | 粘性滚动（#55）：符号树、当前粘性行、区高度 | [`CodeView::set_symbols`]、[`CodeView::sticky_lines`]、[`CodeView::sticky_height`]、[`sticky`] |
//!
//! 事件用 GPUI 的 `cx.subscribe(&view, |_, _, e: &CodeViewEvent, _| ..)` 订阅；回调只做轻量更新。
//!
//! # 线程与帧预算
//!
//! 所有方法只在 UI 线程调用。绘制只读缓存；读文本、调用装饰提供方与整形都在帧之间的空闲切片里
//! （每片 [`view::PREPARE_SLICE`]），缓存未命中时先画等宽近似占位，下一帧替换，行高不变。

// #53：注释切换、自动缩进、自动闭合、包围与回车规则。
pub mod autoedit;
pub mod brackets;
pub mod cache;
pub mod colors;
pub mod columns;
pub mod commands;
pub mod decorations;
// #47：编辑交互与输入法。
pub mod edit;
mod element;
pub mod folding;
pub mod guides;
pub mod highlight;
// #52：多光标、列选择与行操作。
pub mod lineops;
pub mod multicursor;
pub mod occurrences;
pub mod options;
pub mod scroll;
pub mod selection;
pub mod sticky;
pub mod view;
// #58：软换行（换行点计算与视觉行映射）。
pub mod wrap;

pub use decorations::{
    priority, DecorationProvider, GlyphIcon, InlineDecoration, LayerId, LineDecorations, LineStyle,
    Overlay, TextStyle,
};
pub use edit::{Composition, EditHost, EditNotice, TextEdited};
pub use multicursor::{ColumnSelect, MultiCursorModifier};
pub use scroll::{Reveal, ScrollPosition};
pub use selection::Selection;
pub use view::{
    presented_read_only, read_only_reason_key, CodeView, CodeViewEvent, CodeViewServices, Feature,
    FrameStats, SelectionSource, Viewport,
};
pub use wrap::{VisualMap, WordWrap, WrapOptions, WrappingIndent, WrappingStrategy};

use rove_base::Result;

/// 一次性登记代码视图的设置项、颜色键、命令与界面字符串（装配时调用）。
pub fn register(
    settings: &mut rove_config::SettingRegistry,
    colors: &mut rove_theme::ColorRegistry,
    commands: &mut rove_commands::CommandRegistry,
    strings: &mut rove_i18n::StringTable,
) -> Result<()> {
    options::register(settings)?;
    colors::register(colors)?;
    commands::register(commands)?;
    commands::register_strings(strings)
}
