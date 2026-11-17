use gpui::RenderImage;
use image::{AnimationDecoder, Frame, ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage};
use resvg::{tiny_skia, usvg};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};

const MAX_EDGE: u32 = 16_384;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_FILE: u64 = 128 * 1024 * 1024;
const MAX_SVG_FILE: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecodeError {
    Unavailable,
    Invalid,
    Rendering,
    TooLarge,
    Cancelled,
}

pub(crate) struct Decoded {
    pub dimensions: (u32, u32),
    pub frames: Arc<RenderImage>,
    pub svg: Option<Arc<usvg::Tree>>,
    pub blocked: usize,
}

impl Decoded {
    fn raster(dimensions: (u32, u32), frames: Vec<Frame>) -> Self {
        Self {
            dimensions,
            frames: Arc::new(RenderImage::new(smallvec::SmallVec::from_vec(frames))),
            svg: None,
            blocked: 0,
        }
    }
}

pub(crate) fn decode(path: &Path, cancelled: &AtomicBool) -> Result<Decoded, DecodeError> {
    let file = File::open(path).map_err(|_| DecodeError::Unavailable)?;
    let len = file.metadata().map_err(|_| DecodeError::Unavailable)?.len();
    if len > MAX_FILE {
        return Err(DecodeError::TooLarge);
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let format = match extension.to_ascii_lowercase().as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "tga" => ImageFormat::Tga,
        "svg" => return decode_svg(file.take(MAX_SVG_FILE + 1), cancelled),
        _ => return Err(DecodeError::Invalid),
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(MAX_PIXELS * 4);
    if cancelled.load(Ordering::Relaxed) {
        return Err(DecodeError::Cancelled);
    }
    if format == ImageFormat::Gif {
        // Decode one frame at a time and check cancellation between frames. The total frame
        // budget bounds both Rust memory and the eventual sprite atlas footprint.
        let mut decoder =
            image::codecs::gif::GifDecoder::new(BufReader::new(file)).map_err(image_error)?;
        let dimensions = decoder.dimensions();
        let pixels = check_size(dimensions)?;
        decoder.set_limits(limits).map_err(image_error)?;
        let mut frames = Vec::new();
        let mut reader = decoder.into_frames();
        while !cancelled.load(Ordering::Relaxed) {
            let Some(frame) = reader.next() else { break };
            if frames.len() == 1024 || (frames.len() as u64 + 1) * pixels > MAX_PIXELS {
                return Err(DecodeError::TooLarge);
            }
            frames.push(to_bgra(frame.map_err(image_error)?));
        }
        if cancelled.load(Ordering::Relaxed) {
            return Err(DecodeError::Cancelled);
        }
        if frames.is_empty() {
            return Err(DecodeError::Invalid);
        }
        return Ok(Decoded::raster(dimensions, frames));
    }

    let mut reader = ImageReader::with_format(BufReader::new(file), format);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(image_error)?;
    let dimensions = decoder.dimensions();
    check_size(dimensions)?;
    if decoder.total_bytes() > MAX_PIXELS * 4 {
        return Err(DecodeError::TooLarge);
    }
    let frame = Frame::new(
        image::DynamicImage::from_decoder(decoder)
            .map_err(image_error)?
            .into_rgba8(),
    );
    Ok(Decoded::raster(dimensions, vec![to_bgra(frame)]))
}

fn image_error(error: image::ImageError) -> DecodeError {
    match error {
        image::ImageError::Limits(_) => DecodeError::TooLarge,
        _ => DecodeError::Invalid,
    }
}

fn check_size((width, height): (u32, u32)) -> Result<u64, DecodeError> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 {
        return Err(DecodeError::Invalid);
    }
    if width > MAX_EDGE || height > MAX_EDGE || pixels > MAX_PIXELS {
        return Err(DecodeError::TooLarge);
    }
    Ok(pixels)
}

fn to_bgra(frame: Frame) -> Frame {
    let delay = frame.delay();
    let mut pixels = frame.into_buffer();
    let bytes: &mut [u8] = pixels.as_mut();
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Frame::from_parts(pixels, 0, 0, delay)
}

fn decode_svg(mut file: impl Read, cancelled: &AtomicBool) -> Result<Decoded, DecodeError> {
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|_| DecodeError::Invalid)?;
    if text.len() as u64 > MAX_SVG_FILE {
        return Err(DecodeError::TooLarge);
    }
    let xml = roxmltree::Document::parse(&text).map_err(|_| DecodeError::Invalid)?;
    if xml.root_element().tag_name().name() != "svg" {
        return Err(DecodeError::Invalid);
    }
    let blocked = xml
        .descendants()
        .filter(|n| n.is_element())
        .map(|node| {
            let refs = node
                .attributes()
                .filter(|attr| attr.name() == "href" && remote(attr.value()))
                .count();
            let css = node
                .attributes()
                .filter(|attr| attr.name() == "style")
                .map(|attr| attr.value())
                .chain((node.tag_name().name() == "style").then(|| node.text().unwrap_or_default()))
                .map(remote_css)
                .sum::<usize>();
            refs + css
        })
        .sum();
    if cancelled.load(Ordering::Relaxed) {
        return Err(DecodeError::Cancelled);
    }
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let fontdb = if xml.descendants().any(|node| node.has_tag_name("text")) {
        FONTS
            .get_or_init(|| {
                let mut fonts = usvg::fontdb::Database::new();
                fonts.load_system_fonts();
                Arc::new(fonts)
            })
            .clone()
    } else {
        Arc::default()
    };
    let options = usvg::Options {
        resources_dir: None,
        // No filesystem resolver: UNC paths and all URI schemes are denied. Inline nested
        // images are also disabled to keep the decode limit enforceable for every allocation.
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        fontdb,
        ..Default::default()
    };
    let tree = Arc::new(usvg::Tree::from_str(&text, &options).map_err(|_| DecodeError::Invalid)?);
    let dimensions = (
        tree.size().width().ceil() as u32,
        tree.size().height().ceil() as u32,
    );
    check_size(dimensions)?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(DecodeError::Cancelled);
    }
    Ok(Decoded {
        dimensions,
        frames: rasterize(&tree, 1.0)?,
        svg: Some(tree),
        blocked,
    })
}

fn remote(target: &str) -> bool {
    let lower = target.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("//")
}

fn remote_css(css: &str) -> usize {
    css.to_ascii_lowercase()
        .split("url(")
        .skip(1)
        .filter_map(|value| value.split_once(')').map(|(target, _)| target))
        .filter(|target| remote(target.trim().trim_matches(['\'', '"'])))
        .count()
}

pub(crate) fn rasterize(tree: &usvg::Tree, scale: f32) -> Result<Arc<RenderImage>, DecodeError> {
    let width = tree.size().width();
    let height = tree.size().height();
    // Keep zoom useful for large SVGs without asking for unbounded bitmap memory.
    let scale = scale
        .min(MAX_EDGE as f32 / width.max(height))
        .min((MAX_PIXELS as f32 / (width * height)).sqrt());
    let dimensions = (
        (width * scale).floor().max(1.0) as u32,
        (height * scale).floor().max(1.0) as u32,
    );
    check_size(dimensions)?;
    let mut pixmap =
        tiny_skia::Pixmap::new(dimensions.0, dimensions.1).ok_or(DecodeError::TooLarge)?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut pixels = pixmap.take();
    // Tiny-skia returns premultiplied RGBA; GPUI's image sprites hold straight BGRA.
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let image =
        RgbaImage::from_raw(dimensions.0, dimensions.1, pixels).ok_or(DecodeError::Invalid)?;
    Ok(Decoded::raster(dimensions, vec![to_bgra(Frame::new(image))]).frames)
}
