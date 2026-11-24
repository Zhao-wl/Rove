//! File icon rendering. Decode extension data off the UI thread and retain the snapshots.

use gpui::StyledImage;
use gpui::{
    div, img, px, rgba, App, Div, FontWeight, ObjectFit, ParentElement, RenderImage, SharedString,
    Styled,
};
use image::{Frame, ImageReader, Limits};
use resvg::usvg;
use rove_theme::file_icons::{FileIcon, IconTarget, IconTheme};
use rove_theme::ThemeKind;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Cursor;
use std::sync::Arc;

const BITMAP_EDGE: f32 = 64.0;

#[derive(Default, Clone)]
pub(crate) enum FileIcons {
    #[default]
    Loading,
    Disabled,
    Ready(Arc<RegisteredIcons>),
}

pub(crate) struct PreparedIcons {
    theme: IconTheme,
    fonts: Vec<PreparedFont>,
    images: BTreeMap<usize, Arc<RenderImage>>,
}

struct PreparedFont {
    id: Arc<str>,
    family: SharedString,
    bytes: Vec<u8>,
}

pub(crate) type FontCache = BTreeMap<(usize, u64), SharedString>;

pub(crate) struct RegisteredIcons {
    theme: IconTheme,
    fonts: BTreeMap<Arc<str>, SharedString>,
    images: BTreeMap<usize, Arc<RenderImage>>,
}

impl PreparedIcons {
    pub(crate) fn new(theme: IconTheme) -> Self {
        let mut images = BTreeMap::new();
        for (extension, bytes) in theme.images() {
            let identity = bytes.as_ptr() as usize;
            if images.contains_key(&identity) {
                continue;
            }
            let image = if extension == "svg" {
                decode_svg(bytes)
            } else {
                decode_image(bytes)
            };
            if let Some(image) = image {
                images.insert(identity, image);
            }
        }
        let mut fonts = Vec::new();
        for font in &theme.fonts {
            let prepared = font_bytes(&font.bytes).and_then(|bytes| {
                let face = ttf_parser::Face::parse(&bytes, 0).ok()?;
                let family = [
                    ttf_parser::name_id::TYPOGRAPHIC_FAMILY,
                    ttf_parser::name_id::FAMILY,
                ]
                .into_iter()
                .find_map(|name_id| {
                    face.names().into_iter().find_map(|name| {
                        if name.name_id == name_id {
                            name.to_string()
                        } else {
                            None
                        }
                    })
                })?;
                Some(PreparedFont {
                    id: font.id.clone(),
                    family: family.into(),
                    bytes,
                })
            });
            if let Some(font) = prepared {
                fonts.push(font);
            } else {
                eprintln!("rove-theme: unusable icon font {:?}", font.id);
            }
        }
        Self {
            theme,
            fonts,
            images,
        }
    }

    pub(crate) fn register(self, cx: &App, cached: &mut FontCache) -> FileIcons {
        let mut registered = BTreeMap::new();
        for font in self.fonts {
            let mut hash = DefaultHasher::new();
            font.bytes.hash(&mut hash);
            let key = (font.bytes.len(), hash.finish());
            if let Some(family) = cached.get(&key) {
                registered.insert(font.id, family.clone());
                continue;
            }
            let bytes = Cow::Owned(font.bytes);
            match cx.text_system().add_fonts(vec![bytes]) {
                Ok(()) => {
                    cached.insert(key, font.family.clone());
                    registered.insert(font.id, font.family);
                }
                Err(error) => eprintln!("rove-theme: could not register icon font: {error}"),
            }
        }
        FileIcons::Ready(Arc::new(RegisteredIcons {
            theme: self.theme,
            fonts: registered,
            images: self.images,
        }))
    }
}

impl FileIcons {
    pub(crate) fn hides_explorer_arrows(&self) -> bool {
        matches!(self, Self::Ready(icons) if icons.theme.hides_explorer_arrows)
    }

    pub(crate) fn element(
        &self,
        target: IconTarget<'_>,
        kind: ThemeKind,
        size: f32,
        fallback: impl FnOnce() -> Div,
    ) -> Option<Div> {
        let icons = match self {
            Self::Loading => return Some(fallback()),
            Self::Disabled => return None,
            Self::Ready(icons) => icons,
        };
        let icon = icons.theme.select(target, kind)?;
        let slot = div()
            .flex()
            .items_center()
            .justify_center()
            .size(px(size))
            .flex_shrink_0();
        match icon {
            FileIcon::Glyph {
                font_id,
                character,
                color,
                scale,
                bold,
                italic,
            } => {
                let family = icons.fonts.get(font_id)?;
                let mut glyph = slot.font_family(family.clone()).text_size(px(size * scale));
                if let Some(color) = color {
                    glyph = glyph.text_color(rgba(u32::from_be_bytes([
                        color.0, color.1, color.2, color.3,
                    ])));
                }
                if *bold {
                    glyph = glyph.font_weight(FontWeight::BOLD);
                }
                if *italic {
                    glyph = glyph.italic();
                }
                Some(glyph.child(SharedString::from(character.to_string())))
            }
            FileIcon::Image { bytes, .. } => {
                let image = icons.images.get(&(bytes.as_ptr() as usize))?;
                Some(
                    slot.child(
                        img(image.clone())
                            .size(px(size))
                            .object_fit(ObjectFit::Contain),
                    ),
                )
            }
        }
    }
}

fn decode_svg(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).ok()?;
    let scale = BITMAP_EDGE / tree.size().width().max(tree.size().height());
    crate::image_preview::decode::rasterize(&tree, scale).ok()
}

fn decode_image(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let mut image = reader
        .decode()
        .ok()?
        .thumbnail(BITMAP_EDGE as u32, BITMAP_EDGE as u32)
        .to_rgba8();
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![Frame::new(image)])))
}

fn font_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.starts_with(b"wOFF") || bytes.starts_with(b"wOF2") {
        return unpack_webfont(bytes);
    }
    Some(bytes.to_vec())
}

#[cfg(windows)]
fn unpack_webfont(bytes: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::DirectWrite::{
        DWriteCreateFactory, IDWriteFactory5, DWRITE_FACTORY_TYPE_SHARED,
    };

    const MAX_FONT_SIZE: u64 = 32 * 1024 * 1024;
    let input_size = u32::try_from(bytes.len()).ok()?;
    // SAFETY: the input slice remains live until DirectWrite releases the unpacked stream.
    // The returned fragment stays valid until ReleaseFileFragment and is copied before that call.
    unsafe {
        let factory: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
        let input = bytes.as_ptr().cast();
        let kind = factory.AnalyzeContainerType(input, input_size);
        let stream = factory.UnpackFontFile(kind, input, input_size).ok()?;
        let size = stream.GetFileSize().ok()?;
        if size == 0 || size > MAX_FONT_SIZE {
            return None;
        }
        let mut start = std::ptr::null_mut();
        let mut context = std::ptr::null_mut();
        stream
            .ReadFileFragment(&mut start, 0, size, &mut context)
            .ok()?;
        let result = (!start.is_null())
            .then(|| std::slice::from_raw_parts(start.cast(), size as usize).to_vec());
        stream.ReleaseFileFragment(context);
        result
    }
}

#[cfg(not(windows))]
fn unpack_webfont(_bytes: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn directwrite_unpacks_the_bundled_seti_glyph_font() {
        let theme = rove_theme::file_icons::seti().unwrap();
        let prepared = PreparedIcons::new(theme);
        assert_eq!(prepared.fonts.len(), 1);
        let raw = &prepared.fonts[0].bytes;
        let face = ttf_parser::Face::parse(raw, 0).unwrap();
        assert!(face.glyph_index('\u{e001}').is_some());
    }
}
