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

const COLOR: [u8; 3] = [0x2E, 0x6F, 0xE6];

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

type Point = (f32, f32);

// 两道风迹的控制点来自 docs/design/rove-icon.svg，使用 64 × 64 坐标。
const UPPER_SWEEP: [Point; 10] = [
    (6., 37.),
    (18., 17.),
    (38., 9.),
    (59., 13.),
    (47., 17.),
    (40., 23.),
    (34., 32.),
    (24., 30.),
    (14., 32.),
    (6., 37.),
];
const LOWER_SWEEP: [Point; 10] = [
    (13., 53.),
    (21., 41.),
    (35., 35.),
    (49., 36.),
    (41., 40.),
    (36., 45.),
    (34., 51.),
    (26., 49.),
    (19., 50.),
    (13., 53.),
];

fn sweep_polygon(points: &[Point; 10]) -> Vec<Point> {
    const STEPS: u32 = 32;
    let mut polygon = Vec::with_capacity((3 * STEPS) as usize);
    for start in [0, 3, 6] {
        let [a, b, c, d] = [
            points[start],
            points[start + 1],
            points[start + 2],
            points[start + 3],
        ];
        for step in 0..STEPS {
            let t = step as f32 / STEPS as f32;
            let u = 1.0 - t;
            let interpolate = |a, b, c, d| {
                u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
            };
            polygon.push((
                interpolate(a.0, b.0, c.0, d.0),
                interpolate(a.1, b.1, c.1, d.1),
            ));
        }
    }
    polygon
}

fn scanline_crossings(y: f32, polygon: &[Point], crossings: &mut Vec<f32>) {
    crossings.clear();
    for (&(x1, y1), &(x2, y2)) in polygon.iter().zip(polygon.iter().cycle().skip(1)) {
        if (y1 > y) != (y2 > y) {
            crossings.push((x2 - x1) * (y - y1) / (y2 - y1) + x1);
        }
    }
}

fn inside_at(x: f32, crossings: &[f32]) -> bool {
    !crossings
        .iter()
        .filter(|&&edge| x < edge)
        .count()
        .is_multiple_of(2)
}

/// 渲染成 RGBA 像素（行优先，从上到下）。4×4 超采样抗锯齿。
fn render(size: u32) -> Vec<Rgba> {
    const SS: u32 = 4;
    let upper = sweep_polygon(&UPPER_SWEEP);
    let lower = sweep_polygon(&LOWER_SWEEP);
    let mut upper_crossings = Vec::new();
    let mut lower_crossings = Vec::new();
    let units_per_sample = 64.0 / (size * SS) as f32;
    let mut covered = vec![0u32; size as usize];
    let mut out = Vec::with_capacity((size * size) as usize);
    for py in 0..size {
        covered.fill(0);
        for sy in 0..SS {
            let y = ((py * SS + sy) as f32 + 0.5) * units_per_sample;
            scanline_crossings(y, &upper, &mut upper_crossings);
            scanline_crossings(y, &lower, &mut lower_crossings);
            for (px, count) in covered.iter_mut().enumerate() {
                for sx in 0..SS {
                    let x = ((px as u32 * SS + sx) as f32 + 0.5) * units_per_sample;
                    *count +=
                        u32::from(inside_at(x, &upper_crossings) || inside_at(x, &lower_crossings));
                }
            }
        }
        out.extend(covered.iter().map(|count| {
            let alpha = (count * 255 + SS * SS / 2) / (SS * SS);
            [COLOR[0], COLOR[1], COLOR[2], alpha as u8]
        }));
    }
    out
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
            let bgra = if p[3] == 0 {
                [0; 4]
            } else {
                [p[2], p[1], p[0], p[3]]
            };
            out.extend_from_slice(&bgra);
        }
    }
    // AND 掩码供不使用 alpha 的图标绘制路径使用（每行按 4 字节对齐）。
    let stride = size.div_ceil(32) * 4;
    let mut mask = vec![0u8; (stride * size) as usize];
    for row in 0..size {
        for column in 0..size {
            if pixels[((size - 1 - row) * size + column) as usize][3] == 0 {
                mask[(row * stride + column / 8) as usize] |= 0x80 >> (column % 8);
            }
        }
    }
    out.extend_from_slice(&mask);
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

    #[test]
    fn both_blue_sweeps_have_a_transparent_gap_at_small_sizes() {
        for (size, upper_y, gap_y, lower_y) in [(16, 5, 8, 11), (32, 11, 16, 22)] {
            let pixels = render(size);
            let at = |x, y| pixels[(y * size + x) as usize];
            let x = size / 2;
            assert_eq!(at(x, upper_y), [0x2E, 0x6F, 0xE6, 255], "{size} × {size}");
            assert_eq!(at(x, lower_y), [0x2E, 0x6F, 0xE6, 255], "{size} × {size}");
            assert_eq!(at(x, gap_y)[3], 0, "{size} × {size}");
            assert_eq!(at(0, 0)[3], 0, "{size} × {size}");

            let bmp = encode_bmp(&pixels, size);
            let stride = size.div_ceil(32) * 4;
            let mask = |y| {
                let byte = 40 + size * size * 4 + (size - 1 - y) * stride + x / 8;
                bmp[byte as usize] & (0x80 >> (x % 8))
            };
            assert_eq!(mask(upper_y), 0);
            assert_ne!(mask(gap_y), 0);
            assert_eq!(mask(lower_y), 0);
        }
    }

    #[test]
    fn crc32_matches_the_png_specification() {
        let mut crc = Crc::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
    }
}
