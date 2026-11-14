//! `cargo xtask icon`：生成应用图标（issue #76 第 4 节）。
//!
//! 图标是**代码画出来的**（不是二进制素材），所以可以复核、可以改：这里描述形状，
//! 命令把它栅格化成多尺寸 `.ico`（16 / 32 / 48 / 64 / 128 / 256，含 256×256）写到
//! `packaging/windows/rove.ico`。`rove-app` 的构建脚本把它编进 `rove.exe` 的资源（ID 1），
//! gpui 的 `load_icon()` 正是按 ID 1 取图标，所以任务栏与 Alt+Tab 也用同一张图。
//!
//! - `cargo xtask icon`：重新生成 `.ico`；
//! - `cargo xtask icon --check`：CI / 测试用，生成结果与仓库里的文件不一致就失败；
//! - `cargo xtask icon --preview <PNG>`：另存一张 256×256 的 PNG 方便肉眼看。
//!
//! 小尺寸用未压缩 BMP（兼容性最好），64 以上用 PNG（体积小）。

use std::fs;
use std::path::PathBuf;

use crate::util;

pub const ICON_PATH: &str = "packaging/windows/rove.ico";
/// `.ico` 里的尺寸；256 是 Windows 10/11 大图标与安装器要求的。
const SIZES: &[u32] = &[16, 32, 48, 64, 128, 256];
/// 这个尺寸以上用 PNG 编码（ICO 从 Vista 起支持 PNG 条目）。
const PNG_FROM: u32 = 64;

type Rgba = [u8; 4];

/// 背景的上下两端颜色（线性渐变）与字形颜色。
const TOP: Rgba = [0x3B, 0x8E, 0xFF, 0xFF];
const BOTTOM: Rgba = [0x15, 0x4C, 0xC4, 0xFF];
const GLYPH: Rgba = [0xFF, 0xFF, 0xFF, 0xFF];

pub fn run(args: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let path = util::flag(args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(|| util::repo_root().join(ICON_PATH));
    let ico = build_ico();

    if let Some(preview) = util::flag(args, "--preview") {
        let preview = PathBuf::from(preview);
        fs::write(&preview, encode_png(&render(256), 256))?;
        println!("预览：{}", preview.display());
    }

    if util::has_flag(args, "--check") {
        let current = fs::read(&path).unwrap_or_default();
        if current != ico {
            println!(
                "::error::{} 与 `cargo xtask icon` 的生成结果不一致；重新生成并提交",
                path.display()
            );
            return Ok(1);
        }
        println!(
            "OK：{} 与生成结果一致（{} 字节）。",
            path.display(),
            ico.len()
        );
        return Ok(0);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &ico)?;
    println!(
        "已写出 {}（{}，{:?} 六种尺寸）",
        path.display(),
        util::human_bytes(ico.len() as u64),
        SIZES
    );
    Ok(0)
}

// ---------------------------------------------------------------- 形状

/// 归一化坐标（0..1，y 向下）下某点属于前景（字形）、背景还是透明。
/// 返回 (背景覆盖率, 字形覆盖率)；用超采样得到抗锯齿。
fn sample(x: f32, y: f32) -> (bool, bool) {
    let background = rounded_rect(x, y, 0.015, 0.985, 0.22);
    let glyph = glyph_r(x, y);
    (background, background && glyph)
}

fn rounded_rect(x: f32, y: f32, lo: f32, hi: f32, radius: f32) -> bool {
    if x < lo || x > hi || y < lo || y > hi {
        return false;
    }
    let dx = (lo + radius - x).max(x - (hi - radius)).max(0.0);
    let dy = (lo + radius - y).max(y - (hi - radius)).max(0.0);
    dx * dx + dy * dy <= radius * radius
}

/// 字母 R：竖干 + 右上的椭圆环（碗，中间镂空）+ 右下的斜腿。
fn glyph_r(x: f32, y: f32) -> bool {
    const TOP_Y: f32 = 0.20;
    const BOTTOM_Y: f32 = 0.80;
    const STEM_L: f32 = 0.260;
    const STEM_R: f32 = 0.365;
    // 竖干
    if (STEM_L..=STEM_R).contains(&x) && (TOP_Y..=BOTTOM_Y).contains(&y) {
        return true;
    }
    // 碗：椭圆环的右半部分，左边正好接在竖干的右沿上。
    let (cx, cy) = (STEM_R, 0.375f32);
    let outside = ellipse(x, y, cx, cy, 0.245, 0.175);
    let inside = ellipse(x, y, cx, cy, 0.140, 0.070);
    if x >= cx && outside <= 1.0 && inside >= 1.0 {
        return true;
    }
    // 斜腿：从碗的右下角斜向右下，用平行四边形（上沿与碗重叠，接缝不留缺口）。
    let leg = [
        (0.405f32, 0.500f32),
        (0.515f32, 0.500f32),
        (0.715f32, BOTTOM_Y),
        (0.585f32, BOTTOM_Y),
    ];
    point_in_polygon(x, y, &leg)
}

/// 点相对椭圆的归一化距离平方：<= 1 在椭圆内。
fn ellipse(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> f32 {
    let (dx, dy) = ((x - cx) / rx, (y - cy) / ry);
    dx * dx + dy * dy
}

fn point_in_polygon(x: f32, y: f32, points: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        let (xi, yi) = points[i];
        let (xj, yj) = points[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// 渲染成 RGBA 像素（行优先，从上到下）。4×4 超采样抗锯齿。
fn render(size: u32) -> Vec<Rgba> {
    const SS: u32 = 4;
    let mut out = Vec::with_capacity((size * size) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut bg, mut fg) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    let (b, g) = sample(x, y);
                    bg += u32::from(b);
                    fg += u32::from(g);
                }
            }
            let total = (SS * SS) as f32;
            let bg = bg as f32 / total;
            let fg = fg as f32 / total;
            let t = py as f32 / (size - 1).max(1) as f32;
            let base = lerp(TOP, BOTTOM, t);
            // 字形盖在背景上，背景再按覆盖率做边缘抗锯齿。
            let mixed = lerp(base, GLYPH, fg.min(bg));
            out.push([mixed[0], mixed[1], mixed[2], (bg * 255.0).round() as u8]);
        }
    }
    out
}

fn lerp(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let mix = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]), 255]
}

// ---------------------------------------------------------------- 编码

pub fn build_ico() -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = SIZES
        .iter()
        .map(|size| {
            let pixels = render(*size);
            let data = if *size >= PNG_FROM {
                encode_png(&pixels, *size)
            } else {
                encode_bmp(&pixels, *size)
            };
            (*size, data)
        })
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]); // 保留位 + 类型 1（图标）
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        let dimension = if *size >= 256 { 0u8 } else { *size as u8 };
        out.push(dimension);
        out.push(dimension);
        out.push(0); // 调色板
        out.push(0); // 保留位
        out.extend_from_slice(&1u16.to_le_bytes()); // 色彩平面
        out.extend_from_slice(&32u16.to_le_bytes()); // 位深
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

/// ICO 里的 BMP 条目：BITMAPINFOHEADER（高度是两倍：颜色 + 掩码）+ 自下而上的 BGRA + AND 掩码。
fn encode_bmp(pixels: &[Rgba], size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(size * size * 4).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]); // 分辨率与调色板
    for row in (0..size).rev() {
        for column in 0..size {
            let p = pixels[(row * size + column) as usize];
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    // AND 掩码：32 位图标里用不到，但结构必须在（每行按 4 字节对齐）。
    let stride = size.div_ceil(32) * 4;
    out.extend(std::iter::repeat_n(0u8, (stride * size) as usize));
    out
}

fn encode_png(pixels: &[Rgba], size: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity((size * (size * 4 + 1)) as usize);
    for row in 0..size {
        raw.push(0); // 过滤器 None
        for column in 0..size {
            raw.extend_from_slice(&pixels[(row * size + column) as usize]);
        }
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    use std::io::Write;
    encoder.write_all(&raw).expect("写入内存不会失败");
    let compressed = encoder.finish().expect("压缩不会失败");

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = Vec::new();
    header.extend_from_slice(&size.to_be_bytes());
    header.extend_from_slice(&size.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 位 / RGBA / deflate / 无过滤 / 非隔行
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &compressed);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

struct Crc(u32);

impl Crc {
    fn new() -> Self {
        Self(0xFFFF_FFFF)
    }
    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            let mut value = (self.0 ^ u32::from(*byte)) & 0xFF;
            for _ in 0..8 {
                value = if value & 1 != 0 {
                    0xEDB8_8320 ^ (value >> 1)
                } else {
                    value >> 1
                };
            }
            self.0 = value ^ (self.0 >> 8);
        }
    }
    fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仓库里的 .ico 必须与代码生成的一致（改了形状就要重新生成并提交）。
    #[test]
    fn committed_icon_matches_the_generator() {
        let path = util::repo_root().join(ICON_PATH);
        let current = fs::read(&path).unwrap_or_else(|e| panic!("读 {}：{e}", path.display()));
        assert_eq!(
            current,
            build_ico(),
            "{} 与 `cargo xtask icon` 的结果不一致",
            path.display()
        );
    }

    #[test]
    fn ico_header_declares_every_size_including_256() {
        let ico = build_ico();
        assert_eq!(&ico[0..4], &[0, 0, 1, 0]);
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        assert_eq!(count, SIZES.len());
        let mut seen = Vec::new();
        for i in 0..count {
            let entry = 6 + i * 16;
            let width = ico[entry];
            // 256 在目录里写作 0。
            seen.push(if width == 0 { 256u32 } else { u32::from(width) });
            let length = u32::from_le_bytes(ico[entry + 8..entry + 12].try_into().unwrap());
            let offset = u32::from_le_bytes(ico[entry + 12..entry + 16].try_into().unwrap());
            assert!(offset as usize + length as usize <= ico.len(), "条目越界");
        }
        assert_eq!(seen, SIZES.to_vec());
        assert!(seen.contains(&256), "必须有 256×256（issue #76 第 4 节）");
    }

    #[test]
    fn png_entries_are_real_pngs_and_bmp_entries_are_dibs() {
        let ico = build_ico();
        let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
        for i in 0..count {
            let entry = 6 + i * 16;
            let size = if ico[entry] == 0 {
                256
            } else {
                u32::from(ico[entry])
            };
            let length =
                u32::from_le_bytes(ico[entry + 8..entry + 12].try_into().unwrap()) as usize;
            let offset =
                u32::from_le_bytes(ico[entry + 12..entry + 16].try_into().unwrap()) as usize;
            let data = &ico[offset..offset + length];
            if size >= PNG_FROM {
                assert_eq!(
                    &data[0..8],
                    &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
                );
            } else {
                assert_eq!(u32::from_le_bytes(data[0..4].try_into().unwrap()), 40);
            }
        }
    }

    /// 画出来的图要真的有内容：中心是字形（白），四角是透明。
    #[test]
    fn glyph_is_drawn_on_an_opaque_rounded_background() {
        let size = 64;
        let pixels = render(size);
        let at = |x: u32, y: u32| pixels[(y * size + x) as usize];
        assert_eq!(at(0, 0)[3], 0, "左上角应是透明的圆角外");
        assert!(at(size / 2, size / 2)[3] > 250, "中心不透明");
        // 竖干所在的列（x≈0.35）在上半部分应是白色字形。
        let stem = (0.35 * size as f32) as u32;
        let p = at(stem, size / 2);
        assert!(
            p[0] > 240 && p[1] > 240 && p[2] > 240,
            "竖干应是白色，实际 {p:?}"
        );
        // 碗的中间（x≈0.40, y≈0.375）是镂空，应该露出蓝色背景。
        let hole = at((0.40 * size as f32) as u32, (0.375 * size as f32) as u32);
        assert!(hole[2] > hole[0], "碗心应露出蓝色背景，实际 {hole:?}");
    }

    #[test]
    fn crc32_matches_the_png_specification() {
        let mut crc = Crc::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
    }
}
