//! 把一个元素摆进代码视图（#19）的“行下方嵌入区域”。
//!
//! 嵌入区域由代码视图在文档流里留出高度（`CodeView::set_view_zone`），滚动时随行移动；本元素在
//! **代码视图 prepaint 之后**读取同一帧的区域位置（`CodeView::view_zone_bounds`），再把子元素布局并
//! 摆在那里，所以滚动时没有一帧的滞后，也不另算一份“屏幕行 ↔ 文档行”坐标（折叠映射由代码视图
//! 统一处理）。子元素裁剪在代码视图的范围内；区域滚出视口时不绘制。
//!
//! 用法：作为代码视图**之后**的兄弟元素加入同一个父元素（父元素先 prepaint 代码视图）。

use crate::code_view::CodeView;
use gpui::{
    point, prelude::*, size, AnyElement, App, AvailableSpace, Bounds, ContentMask, Element,
    ElementId, Entity, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Style,
    Window,
};

pub struct ZoneOverlay {
    code: Entity<CodeView>,
    after: u64,
    child: AnyElement,
}

/// `child` 占满 `code` 在第 `after` 行（文档行）下方的嵌入区域：宽度从行号栏左边到文本区右边
/// （不盖住竖直滚动条），高度为区域高度。
pub fn zone_overlay(code: Entity<CodeView>, after: u64, child: AnyElement) -> ZoneOverlay {
    ZoneOverlay { code, after, child }
}

impl IntoElement for ZoneOverlay {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ZoneOverlay {
    type RequestLayoutState = ();
    type PrepaintState = Option<Bounds<Pixels>>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // 不占父元素的布局空间：子元素按区域单独布局。
        let mut style = Style {
            position: gpui::Position::Absolute,
            ..Style::default()
        };
        style.size.width = gpui::px(0.).into();
        style.size.height = gpui::px(0.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        let (zone, view) = {
            let code = self.code.read(cx);
            (code.view_zone_bounds(self.after)?, code.layout.bounds)
        };
        let text_right = zone.origin.x + zone.size.width;
        let origin = point(view.origin.x, zone.origin.y);
        let area = size(text_right - view.origin.x, zone.size.height);
        let mask = view;
        // 给子元素一个确定的尺寸（`size_full` 的根元素据此撑满区域）。
        let child = std::mem::replace(&mut self.child, gpui::Empty.into_any_element());
        self.child = gpui::div()
            .w(area.width)
            .h(area.height)
            .child(child)
            .into_any_element();
        window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
            self.child.layout_as_root(
                size(
                    AvailableSpace::Definite(area.width),
                    AvailableSpace::Definite(area.height),
                ),
                window,
                cx,
            );
            self.child.prepaint_at(origin, window, cx);
        });
        Some(mask)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        mask: &mut Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(mask) = *mask {
            window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
                self.child.paint(window, cx);
            });
        }
    }
}
