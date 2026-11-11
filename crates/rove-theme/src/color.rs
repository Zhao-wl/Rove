//! VS Code 颜色运算的移植（`src/vs/base/common/color.ts`，MIT）。
//!
//! 派生默认值（`darken`/`lighten`/`transparent`/`lessProminent` 等）要和 VS Code 逐位一致，
//! 所以内部沿用它的表示：RGB 为 0–255 整数，alpha 是保留三位小数的浮点数；HSLA 同样四舍五入到
//! 三位小数。只有到契约边界才转换成 [`crate::Color`]。

use crate::Color;

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// JS 的 `x | 0`：向零截断。
fn trunc_i(x: f64) -> i64 {
    x.trunc() as i64
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hsla {
    h: f64,
    s: f64,
    l: f64,
    a: f64,
}

impl Hsla {
    fn new(h: f64, s: f64, l: f64, a: f64) -> Self {
        Self {
            h: trunc_i(h.clamp(0.0, 360.0)) as f64,
            s: round3(s.clamp(0.0, 1.0)),
            l: round3(l.clamp(0.0, 1.0)),
            a: round3(a.clamp(0.0, 1.0)),
        }
    }
}

/// VS Code 的 `Color`：RGBA 加上（若由 HSLA 构造）缓存的 HSLA。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct VsColor {
    r: u8,
    g: u8,
    b: u8,
    a: f64,
    hsla: Option<Hsla>,
}

impl VsColor {
    pub(crate) fn rgba(r: f64, g: f64, b: f64, a: f64) -> Self {
        let c = |v: f64| trunc_i(v.clamp(0.0, 255.0)) as u8;
        Self {
            r: c(r),
            g: c(g),
            b: c(b),
            a: round3(a.clamp(0.0, 1.0)),
            hsla: None,
        }
    }

    pub(crate) const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0.0,
        hsla: None,
    };

    /// `Color.Format.CSS.parseHex`：`#rgb`、`#rgba`、`#rrggbb`、`#rrggbbaa`，大小写不敏感。
    pub(crate) fn from_hex(hex: &str) -> Option<Self> {
        let digits = hex.strip_prefix('#')?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let d = |i: usize| u8::from_str_radix(&digits[i..i + 1], 16).unwrap() as f64;
        let dd = |i: usize| d(i) * 16.0 + d(i + 1);
        Some(match digits.len() {
            3 => Self::rgba(d(0) * 17.0, d(1) * 17.0, d(2) * 17.0, 1.0),
            4 => Self::rgba(d(0) * 17.0, d(1) * 17.0, d(2) * 17.0, d(3) * 17.0 / 255.0),
            6 => Self::rgba(dd(0), dd(2), dd(4), 1.0),
            8 => Self::rgba(dd(0), dd(2), dd(4), dd(6) / 255.0),
            _ => return None,
        })
    }

    pub(crate) fn from_color(c: Color) -> Self {
        Self::rgba(c.0 as f64, c.1 as f64, c.2 as f64, c.3 as f64 / 255.0)
    }

    pub(crate) fn to_color(self) -> Color {
        Color(self.r, self.g, self.b, (self.a * 255.0).round() as u8)
    }

    fn from_hsla(h: Hsla) -> Self {
        let (hh, s, l) = (h.h / 360.0, h.s, h.l);
        let (r, g, b) = if s == 0.0 {
            (l, l, l)
        } else {
            let q = if l < 0.5 {
                l * (1.0 + s)
            } else {
                l + s - l * s
            };
            let p = 2.0 * l - q;
            (
                hue2rgb(p, q, hh + 1.0 / 3.0),
                hue2rgb(p, q, hh),
                hue2rgb(p, q, hh - 1.0 / 3.0),
            )
        };
        let mut c = Self::rgba(
            (r * 255.0).round(),
            (g * 255.0).round(),
            (b * 255.0).round(),
            h.a,
        );
        c.hsla = Some(h);
        c
    }

    fn hsla(&self) -> Hsla {
        if let Some(h) = self.hsla {
            return h;
        }
        let (r, g, b) = (
            self.r as f64 / 255.0,
            self.g as f64 / 255.0,
            self.b as f64 / 255.0,
        );
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (min + max) / 2.0;
        let chroma = max - min;
        let (mut h, mut s) = (0.0, 0.0);
        if chroma > 0.0 {
            s = (if l <= 0.5 {
                chroma / (2.0 * l)
            } else {
                chroma / (2.0 - 2.0 * l)
            })
            .min(1.0);
            h = if max == r {
                (g - b) / chroma + if g < b { 6.0 } else { 0.0 }
            } else if max == g {
                (b - r) / chroma + 2.0
            } else {
                (r - g) / chroma + 4.0
            };
            h = (h * 60.0).round();
        }
        Hsla::new(h, s, l, self.a)
    }

    pub(crate) fn lighten(self, factor: f64) -> Self {
        let h = self.hsla();
        Self::from_hsla(Hsla::new(h.h, h.s, h.l + h.l * factor, h.a))
    }

    pub(crate) fn darken(self, factor: f64) -> Self {
        let h = self.hsla();
        Self::from_hsla(Hsla::new(h.h, h.s, h.l - h.l * factor, h.a))
    }

    pub(crate) fn transparent(self, factor: f64) -> Self {
        Self::rgba(self.r as f64, self.g as f64, self.b as f64, self.a * factor)
    }

    pub(crate) fn with_opacity(self, opacity: f64) -> Self {
        Self::rgba(self.r as f64, self.g as f64, self.b as f64, opacity)
    }

    pub(crate) fn mix(self, other: Self, factor: f64) -> Self {
        let f = factor.clamp(0.0, 1.0);
        let m = |a: f64, b: f64| a + (b - a) * f;
        Self::rgba(
            m(self.r as f64, other.r as f64),
            m(self.g as f64, other.g as f64),
            m(self.b as f64, other.b as f64),
            m(self.a, other.a),
        )
    }

    pub(crate) fn make_opaque(self, background: Self) -> Self {
        if self.a == 1.0 || background.a != 1.0 {
            return self;
        }
        let f = |bg: u8, c: u8| bg as f64 - self.a * (bg as f64 - c as f64);
        Self::rgba(
            f(background.r, self.r),
            f(background.g, self.g),
            f(background.b, self.b),
            1.0,
        )
    }

    fn relative_luminance(&self) -> f64 {
        let comp = |v: u8| {
            let c = v as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let l = 0.2126 * comp(self.r) + 0.7152 * comp(self.g) + 0.0722 * comp(self.b);
        (l * 10000.0).round() / 10000.0
    }

    fn is_darker_than(&self, other: &Self) -> bool {
        self.relative_luminance() < other.relative_luminance()
    }

    fn is_lighter_than(&self, other: &Self) -> bool {
        self.relative_luminance() > other.relative_luminance()
    }

    fn lighter_relative_to(self, relative: Self, factor: f64) -> Self {
        if self.is_lighter_than(&relative) {
            return self;
        }
        let factor = if factor == 0.0 { 0.5 } else { factor };
        let (l1, l2) = (self.relative_luminance(), relative.relative_luminance());
        self.lighten(factor * (l2 - l1) / l2)
    }

    fn darker_relative_to(self, relative: Self, factor: f64) -> Self {
        if self.is_darker_than(&relative) {
            return self;
        }
        let factor = if factor == 0.0 { 0.5 } else { factor };
        let (l1, l2) = (self.relative_luminance(), relative.relative_luminance());
        self.darken(factor * (l1 - l2) / l1)
    }

    /// `ColorTransformType.LessProminent`（已解析背景时的分支）。
    pub(crate) fn less_prominent(
        self,
        background: Option<Self>,
        factor: f64,
        transparency: f64,
    ) -> Self {
        match background {
            None => self.transparent(factor * transparency),
            Some(bg) if self.is_darker_than(&bg) => self
                .lighter_relative_to(bg, factor)
                .transparent(transparency),
            Some(bg) => self
                .darker_relative_to(bg, factor)
                .transparent(transparency),
        }
    }
}

fn hue2rgb(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// 严格的主题颜色字符串校验（vscode-textmate `isValidHexColor`）：`#rgb`、`#rgba`、`#rrggbb`、`#rrggbbaa`。
pub(crate) fn is_valid_hex(s: &str) -> bool {
    s.strip_prefix('#').is_some_and(|d| {
        matches!(d.len(), 3 | 4 | 6 | 8) && d.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// 解析主题里的颜色字符串；非法时返回 `None`（调用方负责记录诊断并跳过该键）。
pub fn parse_hex(s: &str) -> Option<Color> {
    is_valid_hex(s)
        .then(|| VsColor::from_hex(s))
        .flatten()
        .map(VsColor::to_color)
}

/// `#rrggbb` 或带 alpha 的 `#rrggbbaa`（alpha 为 255 时省略），小写。
pub fn format_hex(c: Color) -> String {
    if c.3 == 255 {
        format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.0, c.1, c.2, c.3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> VsColor {
        VsColor::from_hex(s).unwrap()
    }

    #[test]
    fn parses_all_hex_forms_like_vscode() {
        assert_eq!(parse_hex("#fff"), Some(Color(255, 255, 255, 255)));
        assert_eq!(parse_hex("#f008"), Some(Color(255, 0, 0, 0x88)));
        assert_eq!(parse_hex("#0F4A85"), Some(Color(0x0f, 0x4a, 0x85, 255)));
        assert_eq!(parse_hex("#0F4A851A"), Some(Color(0x0f, 0x4a, 0x85, 0x1a)));
        for bad in ["", "red", "#12", "#12345", "#ggg", "#1234567", "0F4A85"] {
            assert_eq!(parse_hex(bad), None, "{bad}");
        }
        // 每个 alpha 字节都能无损往返（VS Code 的 alpha 保留三位小数）。
        for a in 0..=255u8 {
            assert_eq!(VsColor::from_color(Color(1, 2, 3, a)).to_color().3, a);
        }
    }

    // 期望值按 VS Code 1.145.2 base/common/color.ts 的公式逐步手算（见各行注释）。
    #[test]
    fn transforms_match_vscode_color_ts() {
        // a = round3(0.12) = 0.12 → 0.12 * 255 = 30.6 → 31
        assert_eq!(
            hex("#ffffff").transparent(0.12).to_color(),
            Color(255, 255, 255, 31)
        );
        assert_eq!(hex("#000000").lighten(0.2).to_color(), Color(0, 0, 0, 255));
        // #3794ff → HSLA(212, 1, 0.608)；l = round3(0.608 * 1.2) = 0.73
        // → q = 1, p = 0.46 → r = 0.46*255 = 117, g = 0.712*255 = 182, b = 255
        assert_eq!(
            hex("#3794ff").lighten(0.2).to_color(),
            Color(117, 182, 255, 255)
        );
        // l = round3(0.608 * 0.8) = 0.486 → q = 0.972, p = 0 → (0, 116, 248)
        assert_eq!(
            hex("#3794ff").darken(0.2).to_color(),
            Color(0, 116, 248, 255)
        );
        // a = round3(128/255) = 0.502 → g = 255 - 0.502 * 255 = 126.99 → 截断为 126
        assert_eq!(
            hex("#ff000080").make_opaque(hex("#ffffff")).to_color(),
            Color(255, 126, 126, 255)
        );
        assert_eq!(
            hex("#000000").mix(hex("#ffffff"), 0.33).to_color(),
            Color(84, 84, 84, 255)
        );
        assert_eq!(
            hex("#123456").with_opacity(0.5).to_color(),
            Color(0x12, 0x34, 0x56, 128)
        );
    }

    #[test]
    fn less_prominent_moves_away_from_background() {
        let fg = hex("#ea5c00");
        let dark_bg = hex("#1e1e1e");
        let out = fg.less_prominent(Some(dark_bg), 0.5, 0.4).to_color();
        assert_eq!(out.3, 102); // 0.4 * 255
        let light = fg.less_prominent(None, 0.5, 0.4).to_color();
        assert_eq!(light, Color(0xea, 0x5c, 0x00, 51));
    }

    #[test]
    fn formats_hex() {
        assert_eq!(format_hex(Color(1, 2, 3, 255)), "#010203");
        assert_eq!(format_hex(Color(1, 2, 3, 4)), "#01020304");
    }
}
