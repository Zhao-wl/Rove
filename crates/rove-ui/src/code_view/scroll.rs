//! f64 滚动模型。
//!
//! GPUI 的 `Pixels` 是 f32：1429 万行 × 20 px ≈ 2.86 亿 px，f32 在这个量级的分辨率约 32 px，
//! 所以滚动位置一律用 **f64 的“虚拟行”** 表示：`top = 3.25` 表示视口顶边在第 3 行往下 1/4 行处。
//! 上内边距、行下方嵌入区域（view zone）也折算成虚拟行；只有最终落到屏幕上的相对坐标才转成 f32。

use std::collections::BTreeMap;

/// 视口顶边对齐方式（滚动定位接口）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reveal {
    /// 已可见就不动，否则最少滚动（遵守 `cursorSurroundingLines`）。
    Nearest,
    /// 目标行居中。
    Center,
    /// 不在视口内时居中，否则不动（VS Code `revealLineInCenterIfOutsideViewport`）。
    CenterIfOutside,
    /// 目标行放在视口顶端。
    Top,
}

/// 行下方嵌入区域：`after` 行之后插入 `height` 像素（速览窗口、截断标记等）。
#[derive(Clone, Debug, Default)]
pub struct ViewZones {
    zones: BTreeMap<u64, f64>,
}

impl ViewZones {
    /// 设置（或以 0 删除）某行下方的嵌入高度，单位像素。
    pub fn set(&mut self, after: u64, height: f64) {
        if height > 0.0 {
            self.zones.insert(after, height);
        } else {
            self.zones.remove(&after);
        }
    }
    pub fn get(&self, after: u64) -> f64 {
        self.zones.get(&after).copied().unwrap_or(0.0)
    }
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (u64, f64)> + '_ {
        self.zones.iter().map(|(k, v)| (*k, *v))
    }
    /// 第 `line` 行之前所有嵌入区域的高度和（像素）。O(区域数)，区域只有寥寥几个。
    pub fn before(&self, line: u64) -> f64 {
        self.zones.range(..line).map(|(_, h)| h).sum()
    }
    pub fn total(&self) -> f64 {
        self.zones.values().sum()
    }
}

/// 计算滚动所需的度量；每次布局时由视图填好，纯数据。
#[derive(Clone, Debug)]
pub struct Metrics {
    pub line_height: f64,
    /// 文本区的可视高度（不含水平滚动条，除非 `ignoreHorizontalScrollbarInContentHeight`）。
    pub viewport_height: f64,
    /// 文本区的可视宽度。
    pub viewport_width: f64,
    pub line_count: u64,
    pub padding_top: f64,
    pub padding_bottom: f64,
    pub scroll_beyond_last_line: bool,
    /// 已知最宽行的像素宽度（只统计渲染过的行，VS Code 同样如此）。
    pub content_width: f64,
    /// `editor.scrollBeyondLastColumn` × 半角字符宽。
    pub beyond_last_column: f64,
}

impl Metrics {
    pub fn viewport_lines(&self) -> f64 {
        (self.viewport_height / self.line_height).max(0.0)
    }
    /// 第 `line` 行顶边的虚拟行坐标。
    pub fn line_top(&self, line: u64, zones: &ViewZones) -> f64 {
        (self.padding_top + zones.before(line)) / self.line_height + line as f64
    }
    /// 内容总高度（虚拟行）。
    pub fn total_lines(&self, zones: &ViewZones) -> f64 {
        let mut total = (self.padding_top + self.padding_bottom + zones.total()) / self.line_height
            + self.line_count as f64;
        if self.scroll_beyond_last_line {
            // VS Code：最后一行可以滚到视口顶端（额外高度 = 视口高 - 行高）。
            total += (self.viewport_lines() - 1.0).max(0.0);
        }
        total
    }
    pub fn max_top(&self, zones: &ViewZones) -> f64 {
        (self.total_lines(zones) - self.viewport_lines()).max(0.0)
    }
    pub fn max_x(&self) -> f64 {
        (self.content_width + self.beyond_last_column - self.viewport_width).max(0.0)
    }
    /// 一页的行数（VS Code `pageSize = max(1, floor(高度 / 行高) - 2)`）。
    pub fn page_lines(&self) -> u64 {
        ((self.viewport_height / self.line_height).floor() as i64 - 2).max(1) as u64
    }
}

/// 视口位置：`top` 为虚拟行（f64），`x` 为水平像素（f64）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollPosition {
    pub top: f64,
    pub x: f64,
}

impl ScrollPosition {
    pub fn clamped(self, m: &Metrics, zones: &ViewZones) -> Self {
        Self {
            top: self.top.clamp(0.0, m.max_top(zones)),
            x: self.x.clamp(0.0, m.max_x()),
        }
    }

    /// 视口内第一行（整数行号）与该行顶边相对视口顶边的像素偏移（≤ 0）。
    pub fn first_visible(&self, m: &Metrics, zones: &ViewZones) -> (u64, f64) {
        let lh = m.line_height;
        if zones.is_empty() {
            let line_coord = self.top - m.padding_top / lh;
            let first = line_coord.floor().max(0.0);
            let first = (first as u64).min(m.line_count.saturating_sub(1));
            let y = (m.line_top(first, zones) - self.top) * lh;
            return (first, y);
        }
        // 带嵌入区域时从估计值向前修正（区域数很少，这里最多走几步）。
        let mut first = ((self.top - (m.padding_top + zones.total()) / lh)
            .floor()
            .max(0.0) as u64)
            .min(m.line_count.saturating_sub(1));
        while first + 1 < m.line_count && m.line_top(first + 1, zones) + 0.0 <= self.top {
            first += 1;
        }
        while first > 0 && m.line_top(first, zones) > self.top {
            first -= 1;
        }
        (first, (m.line_top(first, zones) - self.top) * lh)
    }

    /// 让 `line` 进入视口（按 `reveal` 策略）；`surrounding` 是 `cursorSurroundingLines`。
    pub fn reveal_line(
        self,
        line: u64,
        reveal: Reveal,
        surrounding: u64,
        m: &Metrics,
        zones: &ViewZones,
    ) -> Self {
        let vis = m.viewport_lines();
        let target = m.line_top(line, zones);
        let inside = target >= self.top && target + 1.0 <= self.top + vis;
        let top = match reveal {
            Reveal::Top => target,
            Reveal::Center => target - (vis - 1.0) / 2.0,
            Reveal::CenterIfOutside if inside => self.top,
            Reveal::CenterIfOutside => target - (vis - 1.0) / 2.0,
            Reveal::Nearest => {
                // 视口太矮时 surrounding 自动缩小，保证目标行可见。
                let s = (surrounding as f64).min(((vis - 1.0) / 2.0).floor().max(0.0));
                let lo = target - s;
                let hi = target + 1.0 + s;
                if lo < self.top {
                    lo
                } else if hi > self.top + vis {
                    hi - vis
                } else {
                    self.top
                }
            }
        };
        Self { top, x: self.x }.clamped(m, zones)
    }

    /// 让水平位置 `x0..x1`（像素，相对文本起点）进入视口。
    pub fn reveal_x(self, x0: f64, x1: f64, m: &Metrics) -> Self {
        let mut x = self.x;
        if x0 < x {
            x = x0;
        } else if x1 > x + m.viewport_width {
            x = x1 - m.viewport_width;
        }
        Self {
            top: self.top,
            x: x.max(0.0),
        }
    }
}

/// 滚轮设置（`editor.mouseWheelScrollSensitivity` 等）。
#[derive(Clone, Copy, Debug)]
pub struct WheelOptions {
    pub sensitivity: f64,
    pub fast_sensitivity: f64,
    pub predominant_axis: bool,
}

/// 滚轮增量换算：返回 (水平像素, 垂直像素)，正值表示内容向下/向右移动（视口向上/向左）。
/// `alt` 时乘 `fastScrollSensitivity`；`shift` 时把垂直滚动转为水平（Windows 惯例，与 VS Code 一致）。
pub fn wheel_delta(dx: f64, dy: f64, alt: bool, shift: bool, o: &WheelOptions) -> (f64, f64) {
    let mut k = o.sensitivity;
    if alt {
        k *= o.fast_sensitivity;
    }
    let (mut dx, mut dy) = (dx * k, dy * k);
    if shift && dx == 0.0 {
        dx = dy;
        dy = 0.0;
    }
    if o.predominant_axis && dx != 0.0 && dy != 0.0 {
        if dx.abs() >= dy.abs() {
            dy = 0.0;
        } else {
            dx = 0.0;
        }
    }
    (dx, dy)
}

/// 滚动条滑块几何。全部 f64；只有最终坐标转 f32。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thumb {
    pub start: f64,
    pub len: f64,
}

/// VS Code 的滑块最小尺寸。
pub const MIN_THUMB: f64 = 20.0;

/// 由“可见量 / 总量 / 当前位置”算滑块；`track` 为轨道长度（像素）。
pub fn thumb(track: f64, visible: f64, total: f64, pos: f64) -> Option<Thumb> {
    if total <= visible || track <= 0.0 {
        return None;
    }
    let len = (track * visible / total).clamp(MIN_THUMB.min(track), track);
    let max_pos = total - visible;
    let start = (track - len) * (pos / max_pos).clamp(0.0, 1.0);
    Some(Thumb { start, len })
}

/// 滑块起点（像素）→ 位置（与 [`thumb`] 互逆）。
pub fn pos_for_thumb(track: f64, visible: f64, total: f64, start: f64) -> f64 {
    let Some(t) = thumb(track, visible, total, 0.0) else {
        return 0.0;
    };
    let free = track - t.len;
    if free <= 0.0 {
        return 0.0;
    }
    (start / free).clamp(0.0, 1.0) * (total - visible)
}

/// 概览标尺 / 滚动条轨道上的坐标换算（供 R55 标记与 #21/#25 的范围高亮使用）：
/// 行号 → 轨道上的 y（像素）。
pub fn track_y_for_line(track: f64, line: u64, m: &Metrics, zones: &ViewZones) -> f64 {
    let total = m.total_lines(zones);
    if total <= 0.0 {
        return 0.0;
    }
    track * m.line_top(line, zones) / total
}

/// 轨道 y → 行号（与 [`track_y_for_line`] 互逆，向下取整）。
pub fn line_for_track_y(track: f64, y: f64, m: &Metrics, zones: &ViewZones) -> u64 {
    let total = m.total_lines(zones);
    if track <= 0.0 {
        return 0;
    }
    let coord = (y / track).clamp(0.0, 1.0) * total;
    let line = (coord - (m.padding_top + zones.total()) / m.line_height).max(0.0);
    let mut l = (line.floor() as u64).min(m.line_count.saturating_sub(1));
    while l + 1 < m.line_count && m.line_top(l + 1, zones) <= coord {
        l += 1;
    }
    l
}

/// 平滑滚动（`editor.smoothScrolling`）：VS Code 的动画时长 125 ms，三次缓出。
pub const SMOOTH_SCROLL_MS: f64 = 125.0;

pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(lines: u64) -> Metrics {
        Metrics {
            line_height: 20.0,
            viewport_height: 400.0,
            viewport_width: 800.0,
            line_count: lines,
            padding_top: 0.0,
            padding_bottom: 0.0,
            scroll_beyond_last_line: true,
            content_width: 0.0,
            beyond_last_column: 0.0,
        }
    }

    #[test]
    fn gigantic_files_keep_exact_line_positions() {
        // 1 GB 测试文件的行数；f32 像素偏移在这里会有 ~32 px 误差。
        let m = metrics(14_290_000);
        let z = ViewZones::default();
        for line in [0, 1, 7_145_000, 14_289_998, 14_289_999] {
            let p = ScrollPosition::default().reveal_line(line, Reveal::Top, 0, &m, &z);
            let (first, y) = p.first_visible(&m, &z);
            assert_eq!(first, line);
            assert_eq!(y, 0.0);
        }
        // 半行偏移在千万行量级仍精确
        let p = ScrollPosition {
            top: 14_000_000.5,
            x: 0.0,
        };
        assert_eq!(p.first_visible(&m, &z), (14_000_000, -10.0));
        // 滑块与行号一致：拖到底就是最后一行在顶端。
        let total = m.total_lines(&z);
        let vis = m.viewport_lines();
        let t = thumb(400.0, vis, total, m.max_top(&z)).unwrap();
        assert!((t.start + t.len - 400.0).abs() < 1e-9);
        let back = pos_for_thumb(400.0, vis, total, t.start);
        assert!((back - m.max_top(&z)).abs() < 1e-6);
        let mid = pos_for_thumb(400.0, vis, total, (400.0 - t.len) / 2.0);
        assert!((mid - m.max_top(&z) / 2.0).abs() < 1e-6);
    }

    #[test]
    fn scroll_beyond_last_line_and_padding() {
        let mut m = metrics(100);
        let z = ViewZones::default();
        assert_eq!(m.max_top(&z), 99.0);
        m.scroll_beyond_last_line = false;
        assert_eq!(m.max_top(&z), 80.0);
        m.padding_top = 10.0;
        m.padding_bottom = 30.0;
        assert_eq!(m.max_top(&z), 82.0);
        let p = ScrollPosition::default();
        assert_eq!(p.first_visible(&m, &z), (0, 10.0));
    }

    #[test]
    fn reveal_strategies_and_surrounding_lines() {
        let m = metrics(1000);
        let z = ViewZones::default();
        let p = ScrollPosition { top: 100.0, x: 0.0 };
        assert_eq!(p.reveal_line(105, Reveal::Nearest, 0, &m, &z).top, 100.0);
        assert_eq!(p.reveal_line(130, Reveal::Nearest, 0, &m, &z).top, 111.0);
        assert_eq!(p.reveal_line(130, Reveal::Nearest, 3, &m, &z).top, 114.0);
        assert_eq!(p.reveal_line(101, Reveal::Nearest, 3, &m, &z).top, 98.0);
        assert_eq!(p.reveal_line(500, Reveal::Center, 0, &m, &z).top, 490.5);
        assert_eq!(
            p.reveal_line(105, Reveal::CenterIfOutside, 0, &m, &z).top,
            100.0
        );
        assert_eq!(p.reveal_line(0, Reveal::Center, 0, &m, &z).top, 0.0);
        // surrounding 超过半屏时收缩
        assert_eq!(p.reveal_line(200, Reveal::Nearest, 1000, &m, &z).top, 190.0);
    }

    #[test]
    fn view_zones_shift_following_lines() {
        let m = metrics(100);
        let mut z = ViewZones::default();
        z.set(4, 100.0); // 第 5 行下方 5 行高的嵌入区
        assert_eq!(m.line_top(5, &z), 10.0);
        let p = ScrollPosition { top: 7.0, x: 0.0 };
        assert_eq!(p.first_visible(&m, &z), (4, -60.0));
        let p = ScrollPosition { top: 10.0, x: 0.0 };
        assert_eq!(p.first_visible(&m, &z), (5, 0.0));
        assert_eq!(m.total_lines(&z), 100.0 + 5.0 + 19.0);
        assert_eq!(line_for_track_y(124.0, 10.0, &m, &z), 5);
        assert_eq!(track_y_for_line(124.0, 5, &m, &z), 10.0);
    }

    #[test]
    fn wheel_sensitivity_fast_and_predominant_axis() {
        let o = WheelOptions {
            sensitivity: 2.0,
            fast_sensitivity: 5.0,
            predominant_axis: true,
        };
        assert_eq!(wheel_delta(1.0, 10.0, false, false, &o), (0.0, 20.0));
        assert_eq!(wheel_delta(0.0, 10.0, true, false, &o), (0.0, 100.0));
        assert_eq!(wheel_delta(0.0, 10.0, false, true, &o), (20.0, 0.0));
        let o = WheelOptions {
            predominant_axis: false,
            ..o
        };
        assert_eq!(wheel_delta(1.0, 10.0, false, false, &o), (2.0, 20.0));
    }

    #[test]
    fn horizontal_reveal_and_beyond_last_column() {
        let mut m = metrics(10);
        m.content_width = 1000.0;
        m.beyond_last_column = 32.0;
        assert_eq!(m.max_x(), 232.0);
        let p = ScrollPosition::default();
        assert_eq!(p.reveal_x(900.0, 910.0, &m).x, 110.0);
        assert_eq!(p.reveal_x(10.0, 20.0, &m).x, 0.0);
    }

    #[test]
    fn smooth_scroll_easing_is_monotonic() {
        let mut last = 0.0;
        for i in 0..=10 {
            let v = ease_out_cubic(i as f64 / 10.0);
            assert!(v >= last);
            last = v;
        }
        assert_eq!(ease_out_cubic(1.0), 1.0);
    }
}
