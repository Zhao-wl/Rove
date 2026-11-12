//! 速览窗口（#41，决策 D3：查找引用的默认展示方式）：速览定义、引用、实现。
//!
//! 在当前行下方内嵌展开（#19 代码视图的“行下方嵌入区域”，不是浮层）：左侧只读代码预览，
//! 右侧按文件分组的结果树（#18 的虚拟化树）；数据来自 #38 的查询路由，打开结果走 #17 的
//! 打开文档服务（由宿主处理 [`PeekEvent::Open`]）。
//!
//! | 模块 | 内容 | GPUI |
//! |---|---|---|
//! | [`model`] | 分组索引（文件 → 计数 + 偏移）、展开规则（≤ 100 全展开）、结果间导航、标题与近似标记 | 否 |
//! | [`source`] | 查询接口 [`PeekQueries`]（#38 `CsRouter` / 冻结契约适配）、能力开关 [`Availability`]、后台流水线 | 否 |
//! | [`snippets`] | 结果行的单行片段（只为展开的文件读取） | 否 |
//! | [`results_tree`] | 结果树数据源（#18 `TreeSource`） | 否 |
//! | [`external`] | 外部符号片段的登记点与保底实现（#42 的接缝） | 否 |
//! | [`registry`] | 命令、字符串、颜色键、上下文键、设置读取、附加默认键位、`PeekAction` | 仅 action |
//! | [`widget`] | 窗口本身：标题栏、预览、结果树、拖动调整、窗口栈（嵌套速览） | 是 |
//! | [`zone`] | 把元素摆进代码视图的嵌入区域 | 是 |

pub mod external;
pub mod model;
pub mod registry;
pub mod results_tree;
pub mod snippets;
pub mod source;
pub mod widget;
pub mod zone;

pub use external::{ExternalSnippet, ExternalSnippetProvider, ExternalSnippets};
pub use model::{Marker, PeekKind, PeekResults};
pub use source::{Availability, BatchQueries, PeekQueries};
pub use widget::{PeekEvent, PeekRequest, PeekServices, PeekWidget};

use crate::code_view::{CodeView, ScrollPosition};
use gpui::Context;

/// 打开嵌入区域后滚动代码视图，让锚定行与整个区域尽量都可见（锚定行优先）。
pub fn reveal_zone(code: &mut CodeView, anchor: u64, cx: &mut Context<CodeView>) {
    let Some(m) = code.metrics() else {
        return;
    };
    let view_line = code.view_line(anchor);
    let anchor_top = m.line_top(view_line, &code.zones);
    let zone_lines = code.zones.get(view_line) / m.line_height;
    let bottom = anchor_top + 1.0 + zone_lines;
    let top = code.scroll_position().top;
    let viewport = m.viewport_lines();
    let target = if anchor_top < top {
        anchor_top
    } else if bottom > top + viewport {
        (bottom - viewport).min(anchor_top)
    } else {
        return;
    };
    let x = code.scroll_position().x;
    code.set_scroll_position(ScrollPosition { top: target, x }, true, cx);
}
