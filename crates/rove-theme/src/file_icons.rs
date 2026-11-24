//! VS Code file icon themes. Selection does no disk I/O.

use crate::extensions::IconThemeEntry;
use crate::{parse_hex, Color, ThemeKind};
use rove_base::{Error, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

const SETI_JSON: &str = include_str!("../data/seti/vs-seti-icon-theme.json");
const SETI_FONT: &[u8] = include_bytes!("../data/seti/seti.woff");
const MAX_ASSET_BYTES: usize = 8 * 1024 * 1024;
const MAX_THEME_ASSET_BYTES: usize = 64 * 1024 * 1024;
const FONT_TYPES: &[&str] = &["woff", "woff2", "ttf", "otf"];
const IMAGE_TYPES: &[&str] = &["svg", "png", "jpg", "jpeg", "gif", "webp", "ico"];

#[derive(Clone, Debug)]
pub struct IconFont {
    pub id: Arc<str>,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub enum FileIcon {
    Glyph {
        font_id: Arc<str>,
        character: char,
        color: Option<Color>,
        scale: f32,
        bold: bool,
        italic: bool,
    },
    Image {
        extension: Arc<str>,
        bytes: Arc<[u8]>,
    },
}

#[derive(Clone, Copy)]
pub enum IconTarget<'a> {
    File {
        name: &'a str,
        language: Option<&'a str>,
    },
    Folder {
        name: &'a str,
        expanded: bool,
        root: bool,
    },
}

#[derive(Default)]
struct Associations {
    names: BTreeMap<String, String>,
    extensions: BTreeMap<String, String>,
    languages: BTreeMap<String, String>,
    folders: BTreeMap<String, String>,
    open_folders: BTreeMap<String, String>,
    roots: BTreeMap<String, String>,
    open_roots: BTreeMap<String, String>,
    file: Option<String>,
    folder: Option<String>,
    open_folder: Option<String>,
    root: Option<String>,
    open_root: Option<String>,
}

impl Associations {
    fn parse(value: &Value) -> Self {
        Self {
            names: keys(value, "fileNames"),
            extensions: keys(value, "fileExtensions"),
            languages: keys(value, "languageIds"),
            folders: keys(value, "folderNames"),
            open_folders: keys(value, "folderNamesExpanded"),
            roots: keys(value, "rootFolderNames"),
            open_roots: keys(value, "rootFolderNamesExpanded"),
            file: field(value, "file"),
            folder: field(value, "folder"),
            open_folder: field(value, "folderExpanded"),
            root: field(value, "rootFolder"),
            open_root: field(value, "rootFolderExpanded"),
        }
    }
}

pub struct IconTheme {
    definitions: BTreeMap<String, FileIcon>,
    associations: Associations,
    light: Associations,
    high_contrast: Associations,
    pub fonts: Vec<IconFont>,
    pub hides_explorer_arrows: bool,
    pub diagnostics: Vec<String>,
}

pub fn seti() -> Result<IconTheme> {
    IconTheme::parse(SETI_JSON, |path, _| match path {
        "./seti.woff" | "seti.woff" => Ok(SETI_FONT.to_vec()),
        _ => Err(Error::NotFound),
    })
}

pub fn load(entry: &IconThemeEntry) -> Result<IconTheme> {
    let text = entry.files.read_json(&entry.path)?;
    let normalized = entry.path.replace('\\', "/");
    let base = normalized.rsplit_once('/').map_or("", |(base, _)| base);
    IconTheme::parse(&text, |relative, types| {
        if relative.starts_with(['/', '\\']) || relative.contains(':') {
            return Err(Error::InvalidInput);
        }
        let path = if base.is_empty() {
            relative.to_owned()
        } else {
            format!("{base}/{relative}")
        };
        entry.files.read_asset(&path, types, MAX_ASSET_BYTES as u64)
    })
}

type FontStyle = (f32, bool, bool);

impl IconTheme {
    fn parse(
        text: &str,
        mut load_asset: impl FnMut(&str, &[&str]) -> Result<Vec<u8>>,
    ) -> Result<Self> {
        let data: Value = jsonc_parser::parse_to_serde_value(text, &Default::default())
            .map_err(|_| Error::InvalidInput)?;
        if !data.is_object() {
            return Err(Error::InvalidInput);
        }
        let mut diagnostics = Vec::new();
        let mut cache = BTreeMap::<String, Arc<[u8]>>::new();
        let mut bytes_loaded = 0;
        let mut asset = |path: &str, allowed: &[&str]| {
            if !suffix(path).is_some_and(|ext| allowed.iter().any(|x| x.eq_ignore_ascii_case(ext)))
            {
                return Err(Error::InvalidInput);
            }
            if let Some(bytes) = cache.get(path) {
                return Ok(bytes.clone());
            }
            let bytes: Arc<[u8]> = load_asset(path, allowed)?.into();
            if bytes.len() > MAX_ASSET_BYTES || bytes_loaded + bytes.len() > MAX_THEME_ASSET_BYTES {
                return Err(Error::InvalidInput);
            }
            bytes_loaded += bytes.len();
            cache.insert(path.to_owned(), bytes.clone());
            Ok(bytes)
        };

        let mut fonts = Vec::new();
        let mut styles = BTreeMap::<Arc<str>, FontStyle>::new();
        for font in array(&data, "fonts") {
            let Some(id) = string(font, "id") else {
                continue;
            };
            let bytes = array(font, "src")
                .iter()
                .filter_map(|item| string(item, "path"))
                .find_map(|path| match asset(path, FONT_TYPES) {
                    Ok(bytes) => Some(bytes),
                    Err(error) => {
                        diagnostics.push(format!("无法加载图标字体 {path:?}：{error}"));
                        None
                    }
                });
            let Some(bytes) = bytes else { continue };
            let id: Arc<str> = id.into();
            if styles.contains_key(&id) {
                continue;
            }
            let style = (
                string(font, "size").and_then(parse_scale).unwrap_or(1.0),
                string(font, "weight").is_some_and(|weight| {
                    weight == "bold" || weight.parse::<u16>().is_ok_and(|n| n >= 600)
                }),
                string(font, "style").is_some_and(|style| matches!(style, "italic" | "oblique")),
            );
            styles.insert(id.clone(), style);
            fonts.push(IconFont { id, bytes });
        }
        let default_font = fonts.first().map(|font| font.id.as_ref());
        let definitions = data.get("iconDefinitions").and_then(Value::as_object);
        let mut icons = BTreeMap::new();
        for (id, definition) in definitions.into_iter().flatten() {
            let icon = if let Some(path) = string(definition, "iconPath") {
                match asset(path, IMAGE_TYPES) {
                    Ok(bytes) => Some(FileIcon::Image {
                        extension: suffix(path).unwrap_or("").to_ascii_lowercase().into(),
                        bytes,
                    }),
                    Err(error) => {
                        diagnostics.push(format!("无法加载文件图标 {path:?}：{error}"));
                        None
                    }
                }
            } else {
                let font = string(definition, "fontId").or(default_font);
                let character = string(definition, "fontCharacter").and_then(parse_glyph);
                match (font, character) {
                    (Some(font), Some(character)) => {
                        styles
                            .get(font)
                            .map(|&(scale, bold, italic)| FileIcon::Glyph {
                                font_id: font.into(),
                                character,
                                color: string(definition, "fontColor").and_then(parse_hex),
                                scale: string(definition, "fontSize")
                                    .and_then(parse_scale)
                                    .unwrap_or(scale),
                                bold,
                                italic,
                            })
                    }
                    _ => None,
                }
            };
            if let Some(icon) = icon {
                icons.insert(id.clone(), icon);
            }
        }
        Ok(Self {
            definitions: icons,
            associations: Associations::parse(&data),
            light: Associations::parse(data.get("light").unwrap_or(&Value::Null)),
            high_contrast: Associations::parse(data.get("highContrast").unwrap_or(&Value::Null)),
            fonts,
            hides_explorer_arrows: data
                .get("hidesExplorerArrows")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            diagnostics,
        })
    }

    pub fn images(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.definitions.values().filter_map(|icon| match icon {
            FileIcon::Image { extension, bytes } => Some((extension.as_ref(), bytes.as_ref())),
            FileIcon::Glyph { .. } => None,
        })
    }

    pub fn select(&self, target: IconTarget<'_>, kind: ThemeKind) -> Option<&FileIcon> {
        let base = &self.associations;
        let overrides = match kind {
            ThemeKind::Dark => None,
            ThemeKind::Light => Some(&self.light),
            ThemeKind::HighContrastDark | ThemeKind::HighContrastLight => Some(&self.high_contrast),
        };
        let get = |key: &str, field: fn(&Associations) -> &BTreeMap<String, String>| {
            overrides
                .and_then(|a| field(a).get(key))
                .or_else(|| field(base).get(key))
        };
        let fallback = |field: fn(&Associations) -> &Option<String>| {
            overrides
                .and_then(|a| field(a).as_ref())
                .or_else(|| field(base).as_ref())
        };
        let icon = match target {
            IconTarget::File { name, language } => {
                let name = name.to_lowercase();
                let language = language.map(str::to_ascii_lowercase);
                get(&name, |a| &a.names)
                    .or_else(|| {
                        name.match_indices('.')
                            .find_map(|(position, _)| get(&name[position + 1..], |a| &a.extensions))
                    })
                    .or_else(|| {
                        language
                            .as_deref()
                            .and_then(|language| get(language, |a| &a.languages))
                    })
                    .or_else(|| fallback(|a| &a.file))
            }
            IconTarget::Folder {
                name,
                expanded,
                root,
            } => {
                let name = name.to_lowercase();
                root.then(|| {
                    expanded
                        .then(|| get(&name, |a| &a.open_roots))
                        .flatten()
                        .or_else(|| get(&name, |a| &a.roots))
                        .or_else(|| expanded.then(|| fallback(|a| &a.open_root)).flatten())
                        .or_else(|| fallback(|a| &a.root))
                })
                .flatten()
                .or_else(|| expanded.then(|| get(&name, |a| &a.open_folders)).flatten())
                .or_else(|| get(&name, |a| &a.folders))
                .or_else(|| expanded.then(|| fallback(|a| &a.open_folder)).flatten())
                .or_else(|| fallback(|a| &a.folder))
            }
        }?;
        self.definitions.get(icon)
    }
}

fn parse_glyph(value: &str) -> Option<char> {
    if let Some(hex) = value.strip_prefix('\\') {
        return u32::from_str_radix(hex.trim(), 16)
            .ok()
            .and_then(char::from_u32);
    }
    let mut chars = value.chars();
    let result = chars.next()?;
    chars.next().is_none().then_some(result)
}

fn parse_scale(value: &str) -> Option<f32> {
    let scale = if let Some(percent) = value.strip_suffix('%') {
        percent.parse::<f32>().ok()? / 100.0
    } else if let Some(em) = value.strip_suffix("em") {
        em.parse().ok()?
    } else {
        value.strip_suffix("px")?.parse::<f32>().ok()? / 16.0
    };
    (scale.is_finite() && scale > 0.0).then_some(scale.clamp(0.25, 4.0))
}

fn suffix(path: &str) -> Option<&str> {
    path.rsplit_once('.').map(|(_, extension)| extension)
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str().filter(|s| !s.is_empty())
}

fn field(value: &Value, key: &str) -> Option<String> {
    string(value, key).map(str::to_owned)
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn keys(value: &Value, key: &str) -> BTreeMap<String, String> {
    let map: Option<&Map<String, Value>> = value.get(key).and_then(Value::as_object);
    map.into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.to_lowercase(), value.as_str()?.to_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_seti_resolves_file_and_language_icons() {
        let theme = seti().unwrap();
        assert!(theme.diagnostics.is_empty());
        assert!(theme
            .fonts
            .first()
            .is_some_and(|font| font.bytes.starts_with(b"wOFF")));
        for file in ["Unit.cs", "data.json", "render.shader", "notes.md"] {
            let target = IconTarget::File {
                name: file,
                language: None,
            };
            assert!(matches!(
                theme.select(target, ThemeKind::Dark),
                Some(FileIcon::Glyph { .. })
            ));
        }
    }

    #[test]
    fn associations_choose_names_before_extensions_then_language_and_overlay_light() {
        let theme = IconTheme::parse(
            r##"{
                "iconDefinitions":{"name":{"iconPath":"name.svg"},"source":{"iconPath":"src.svg"},"language":{"iconPath":"language.svg"},"light":{"iconPath":"light.svg"}},
                "fileNames":{"config.lang":"name"},
                "fileExtensions":{"lang":"source"},
                "languageIds":{"language":"language"},
                "light":{"fileNames":{"config.lang":"light"}}
            }"##,
            |path, _| Ok(path.as_bytes().to_vec()),
        ).unwrap();
        let selected = |name, kind| match theme
            .select(
                IconTarget::File {
                    name,
                    language: Some("language"),
                },
                kind,
            )
            .unwrap()
        {
            FileIcon::Image { bytes, .. } => std::str::from_utf8(bytes).unwrap(),
            FileIcon::Glyph { .. } => unreachable!(),
        };
        assert_eq!(selected("Config.LANG", ThemeKind::Dark), "name.svg");
        assert_eq!(selected("Config.LANG", ThemeKind::Light), "light.svg");
        assert_eq!(selected("other.lang", ThemeKind::Light), "src.svg");
        assert_eq!(selected("no-extension", ThemeKind::Dark), "language.svg");
    }
}
