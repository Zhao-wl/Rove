//! #30：高 DPI 下的物理像素对齐。
//!
//! GPUI 的几何量一律是**逻辑像素**，提交到渲染器时统一乘以窗口的缩放系数（`Window::scale_factor`）。
//! 文字布局（`layout_line`）在逻辑像素里完成，和缩放无关；真正受缩放影响的是**四边形的边界**：
//! 一条写死 1 逻辑像素宽的细线，在 125% 下占 1.25 个物理像素、150% 下占 1.5 个，边界落在半个物理
//! 像素上，光栅化后被抗锯齿摊到两列像素里——看起来发虚、像 2 px，极端情况下（边界正好相邻）还会
//! 显得忽粗忽细。
//!
//! 这里提供三件小工具，供绘制代码把细线与插入符对齐到物理像素网格：
//!
//! - [`snap`]：把逻辑坐标对齐到物理像素网格；
//! - [`hairline`]：1 个物理像素的逻辑厚度（100% 下就是 1.0，不随缩放变成 2 px，也不会被取整抹掉）；
//! - [`device_thickness`]：把一个逻辑厚度换算成**整数个**物理像素（至少 1 个）后的逻辑厚度，
//!   给“粗细跟随缩放、但要求边界清晰”的线（下划线、删除线、改写插入符）用。
//!
//! 约定：`scale` 取 `Window::scale_factor()`。非正或非有限的缩放（测试替身、异常状态）一律按 1.0
//! 处理，绝不产生 NaN / 无穷大的几何量。

use gpui::{point, px, size, Bounds, Pixels};

/// 规范化缩放系数：非有限或非正时按 1.0。
#[inline]
fn norm(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// 把逻辑坐标对齐到物理像素网格（`round(v * scale) / scale`）。
#[inline]
pub fn snap(v: f32, scale: f32) -> f32 {
    if !v.is_finite() {
        return v;
    }
    let s = norm(scale);
    (v * s).round() / s
}

/// 1 个物理像素的逻辑厚度。100% 下为 1.0；125% 下为 0.8；200% 下为 0.5。
#[inline]
pub fn hairline(scale: f32) -> f32 {
    1.0 / norm(scale)
}

/// 把逻辑厚度换算成整数个物理像素（至少 1 个）后的逻辑厚度。
///
/// 例：1.0 逻辑像素在 150% 下是 1.5 个物理像素，四舍五入取 2 个，折回 1.333… 逻辑像素——边界落在
/// 物理像素网格上，不发虚；0.3 逻辑像素在 100% 下仍然保留 1 个物理像素，不会消失。
#[inline]
pub fn device_thickness(logical: f32, scale: f32) -> f32 {
    let s = norm(scale);
    if !logical.is_finite() {
        return hairline(s);
    }
    ((logical * s).round().max(1.0)) / s
}

/// 把矩形对齐到物理像素网格：两边各自对齐，宽高至少 1 个物理像素。
pub fn snap_bounds(b: Bounds<Pixels>, scale: f32) -> Bounds<Pixels> {
    let s = norm(scale);
    let hair = hairline(s);
    let x0 = snap(f32::from(b.origin.x), s);
    let y0 = snap(f32::from(b.origin.y), s);
    let x1 = snap(f32::from(b.origin.x + b.size.width), s);
    let y1 = snap(f32::from(b.origin.y + b.size.height), s);
    Bounds::new(
        point(px(x0), px(y0)),
        size(px((x1 - x0).max(hair)), px((y1 - y0).max(hair))),
    )
}

/// 竖直细线（缩进参考线、分隔线）：x 对齐到物理像素网格，宽度恰好 1 个物理像素。
pub fn vline(x: f32, y: f32, height: f32, scale: f32) -> Bounds<Pixels> {
    let s = norm(scale);
    Bounds::new(
        point(px(snap(x, s)), px(snap(y, s))),
        size(px(hairline(s)), px(height)),
    )
}

/// 水平细线（下划线、删除线、行号栏分隔线）：y 对齐到物理像素网格，高度取整数个物理像素。
pub fn hline(x: f32, y: f32, width: f32, thickness: f32, scale: f32) -> Bounds<Pixels> {
    let s = norm(scale);
    Bounds::new(
        point(px(x), px(snap(y, s))),
        size(px(width), px(device_thickness(thickness, s))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

    /// 逻辑厚度乘缩放后必须是整数个物理像素（允许浮点误差）。
    fn whole_device_pixels(logical: f32, scale: f32) -> bool {
        let phys = logical * scale;
        (phys - phys.round()).abs() < 1e-3 && phys.round() >= 1.0
    }

    #[test]
    fn hairline_is_exactly_one_device_pixel() {
        for s in SCALES {
            let h = hairline(s);
            assert!(whole_device_pixels(h, s), "scale {s}: {h}");
            assert!((h * s - 1.0).abs() < 1e-6, "scale {s} 不是 1 物理像素");
        }
        assert_eq!(hairline(1.0), 1.0);
        assert_eq!(hairline(2.0), 0.5);
    }

    #[test]
    fn snap_lands_on_the_device_grid() {
        for s in SCALES {
            for v in [0.0, 0.3, 7.5, 12.4, 100.7, -3.2] {
                let got = snap(v, s);
                let phys = got * s;
                assert!(
                    (phys - phys.round()).abs() < 1e-3,
                    "scale {s}, v {v}: {got}"
                );
                // 对齐的位移不超过半个物理像素。
                assert!((got - v).abs() <= 0.5 / s + 1e-6, "scale {s}, v {v}: {got}");
            }
        }
    }

    #[test]
    fn device_thickness_is_whole_pixels_and_never_vanishes() {
        for s in SCALES {
            for t in [0.0, 0.2, 1.0, 1.4, 2.0] {
                let got = device_thickness(t, s);
                assert!(whole_device_pixels(got, s), "scale {s}, t {t}: {got}");
            }
            // 1 逻辑像素在各缩放下的物理厚度：100%→1、125%→1、150%→2、200%→2。
            let phys = (device_thickness(1.0, s) * s).round();
            assert_eq!(phys, (1.0f32 * s).round().max(1.0), "scale {s}");
        }
    }

    #[test]
    fn degenerate_scales_fall_back_to_one() {
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(hairline(bad), 1.0);
            assert_eq!(snap(2.4, bad), 2.0);
            assert_eq!(device_thickness(1.0, bad), 1.0);
        }
    }

    #[test]
    fn snap_bounds_keeps_at_least_one_device_pixel() {
        for s in SCALES {
            let b = snap_bounds(
                Bounds::new(point(px(10.3), px(4.7)), size(px(0.0), px(0.1))),
                s,
            );
            assert!(f32::from(b.size.width) * s >= 1.0 - 1e-3, "scale {s}");
            assert!(f32::from(b.size.height) * s >= 1.0 - 1e-3, "scale {s}");
        }
    }

    #[test]
    fn vline_and_hline_are_crisp() {
        for s in SCALES {
            let v = vline(10.3, 4.7, 18.0, s);
            assert!(whole_device_pixels(f32::from(v.size.width), s));
            let x = f32::from(v.origin.x) * s;
            assert!((x - x.round()).abs() < 1e-3, "scale {s}: {x}");

            let h = hline(3.0, 17.4, 40.0, 1.0, s);
            assert!(whole_device_pixels(f32::from(h.size.height), s));
            let y = f32::from(h.origin.y) * s;
            assert!((y - y.round()).abs() < 1e-3, "scale {s}: {y}");
            assert_eq!(f32::from(h.size.width), 40.0);
        }
    }
}
