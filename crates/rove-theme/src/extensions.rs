//! Reads theme contributions from installed VS Code extensions without loading extension code.

use crate::file::{parse_kind, Diagnostic, ThemeSource};
use crate::service::ThemeEntry;
use rove_base::{Error, Result};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_THEME_JSON: u64 = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct IconThemeEntry {
    pub id: Arc<str>,
    pub label: Arc<str>,
    pub path: Arc<str>,
    pub files: Arc<ExtensionFiles>,
}

#[derive(Default)]
pub struct ExtensionThemes {
    pub colors: Vec<ThemeEntry>,
    pub icons: Vec<IconThemeEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

/// A filesystem source restricted to ordinary local files inside one extension.
pub struct ExtensionFiles(PathBuf);

impl ExtensionFiles {
    pub(crate) fn resolve(&self, path: &str, extensions: &[&str]) -> Result<PathBuf> {
        let path = relative_path(path).ok_or(Error::InvalidInput)?;
        let extension = path.extension().and_then(|s| s.to_str());
        if !extension.is_some_and(|ext| extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))) {
            return Err(Error::InvalidInput);
        }
        let mut full_path = self.0.clone();
        for part in path.components() {
            full_path.push(part.as_os_str());
            if is_redirect(&fs::symlink_metadata(&full_path)?) {
                return Err(Error::InvalidInput);
            }
        }
        Ok(full_path)
    }

    pub(crate) fn read_asset(
        &self,
        path: &str,
        extensions: &[&str],
        max_bytes: u64,
    ) -> Result<Vec<u8>> {
        let file = File::open(self.resolve(path, extensions)?)?;
        if !file.metadata()?.is_file() {
            return Err(Error::InvalidInput);
        }
        let mut bytes = Vec::new();
        file.take(max_bytes + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max_bytes {
            return Err(Error::InvalidInput);
        }
        Ok(bytes)
    }

    pub(crate) fn read_json(&self, path: &str) -> Result<String> {
        let bytes = self.read_asset(path, &["json"], MAX_THEME_JSON)?;
        let text = String::from_utf8(bytes).map_err(|_| Error::InvalidInput)?;
        Ok(text
            .strip_prefix('\u{feff}')
            .map(str::to_owned)
            .unwrap_or(text))
    }
}

impl ThemeSource for ExtensionFiles {
    fn read(&self, path: &str) -> Result<Cow<'static, str>> {
        self.read_json(path).map(Cow::Owned)
    }
}

fn relative_path(value: &str) -> Option<PathBuf> {
    if value.is_empty() || value.starts_with(['/', '\\']) || value.contains(':') {
        return None;
    }
    let mut path = PathBuf::new();
    for segment in value.split(['/', '\\']) {
        match segment {
            "" | "." => continue,
            ".." if path.pop() => continue,
            ".." => return None,
            value
                if value.ends_with(['.', ' '])
                    || value.eq_ignore_ascii_case("node_modules")
                    || value.eq_ignore_ascii_case(".gitignore") =>
            {
                return None
            }
            value => path.push(value),
        }
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

pub fn is_local_directory(root: &Path) -> bool {
    validate_local_root(root).is_ok() && root.is_dir()
}

fn validate_local_root(root: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::path::Prefix;
        if !matches!(
            root.components().next(),
            Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        ) {
            return Err(Error::InvalidInput);
        }
    }
    if !root.is_absolute() {
        return Err(Error::InvalidInput);
    }
    let mut path = PathBuf::new();
    for component in root.components() {
        path.push(component.as_os_str());
        match component {
            Component::ParentDir => return Err(Error::InvalidInput),
            Component::Normal(_) if is_redirect(&fs::symlink_metadata(&path)?) => {
                return Err(Error::InvalidInput);
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_redirect(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum VersionPart {
    Numeric(u64),
    Name(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ExtensionVersion {
    numeric: [u64; 3],
    stable: bool,
    prerelease: Vec<VersionPart>,
}

impl ExtensionVersion {
    fn parse(version: &str) -> Option<Self> {
        let version = version.split('+').next()?;
        let (core, suffix) = version.split_once('-').unwrap_or((version, ""));
        let mut parts = core.split('.');
        let numeric = [
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        ];
        if parts.next().is_some() {
            return None;
        }
        let stable = suffix.is_empty();
        let prerelease = if stable {
            Vec::new()
        } else {
            suffix
                .split('.')
                .map(|part| match part.parse() {
                    Ok(number) => VersionPart::Numeric(number),
                    Err(_) => VersionPart::Name(part.to_owned()),
                })
                .collect()
        };
        Some(Self {
            numeric,
            stable,
            prerelease,
        })
    }
}

struct Extension {
    manifest: Value,
    files: Arc<ExtensionFiles>,
    version: Option<ExtensionVersion>,
}

impl ExtensionThemes {
    fn diagnose(&mut self, file: &Path, field: &str, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            file: file.display().to_string(),
            field: field.to_owned(),
            message: message.into(),
        });
    }

    fn add(&mut self, extension: Extension) {
        let Extension {
            manifest, files, ..
        } = extension;
        let Some(contributes) = manifest.get("contributes") else {
            return;
        };
        let entries = |key: &str| {
            contributes
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        };
        for theme in entries("themes") {
            let (Some(label), Some(path), Some(kind)) = (
                required(theme, "label"),
                required(theme, "path"),
                required(theme, "uiTheme").and_then(parse_kind),
            ) else {
                self.diagnose(
                    &files.0,
                    "contributes.themes",
                    "缺少标签、路径或有效的 uiTheme",
                );
                continue;
            };
            if let Err(error) = files.resolve(path, &["json"]) {
                self.diagnose(&files.0, path, format!("无法加载颜色主题：{error}"));
                continue;
            }
            self.colors.push(ThemeEntry {
                id: required(theme, "id").unwrap_or(label).into(),
                label: label.into(),
                kind,
                path: path.into(),
                source: files.clone(),
            });
        }
        for theme in entries("iconThemes") {
            let (Some(id), Some(path)) = (required(theme, "id"), required(theme, "path")) else {
                self.diagnose(&files.0, "contributes.iconThemes", "缺少图标主题 ID 或路径");
                continue;
            };
            if let Err(error) = files.resolve(path, &["json"]) {
                self.diagnose(&files.0, path, format!("无法加载图标主题：{error}"));
                continue;
            }
            self.icons.push(IconThemeEntry {
                id: id.into(),
                label: required(theme, "label").unwrap_or(id).into(),
                path: path.into(),
                files: files.clone(),
            });
        }
    }
}

fn required<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str().filter(|s| !s.trim().is_empty())
}

fn read_manifest(path: PathBuf) -> Result<(String, Extension)> {
    let files = Arc::new(ExtensionFiles(path));
    let text = files.read_json("package.json")?;
    let manifest: Value = jsonc_parser::parse_to_serde_value(&text, &Default::default())
        .map_err(|_| Error::InvalidInput)?;
    let publisher = required(&manifest, "publisher").ok_or(Error::InvalidInput)?;
    let name = required(&manifest, "name").ok_or(Error::InvalidInput)?;
    let key = format!("{publisher}.{name}").to_ascii_lowercase();
    let version = required(&manifest, "version").and_then(ExtensionVersion::parse);
    Ok((
        key,
        Extension {
            manifest,
            files,
            version,
        },
    ))
}

/// Scans manifests only. Callers run this on a background executor and publish the result.
pub fn scan_extensions(root: &Path) -> ExtensionThemes {
    let mut result = ExtensionThemes::default();
    if let Err(err) = validate_local_root(root) {
        if matches!(
            &err,
            Error::Io {
                kind: std::io::ErrorKind::NotFound,
                ..
            }
        ) {
            return result;
        }
        result.diagnose(root, "", format!("扩展目录不可用：{err}"));
        return result;
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) => {
            result.diagnose(root, "", format!("无法枚举扩展目录：{err}"));
            return result;
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry)
                if !entry.file_name().eq_ignore_ascii_case("node_modules")
                    && entry.file_type().is_ok_and(|kind| kind.is_dir()) =>
            {
                let path = entry.path();
                // Directory junctions may report is_dir on Windows.
                if fs::symlink_metadata(&path).is_ok_and(|meta| !is_redirect(&meta)) {
                    paths.push(path);
                }
            }
            Ok(_) => {}
            Err(err) => result.diagnose(root, "", format!("无法枚举扩展条目：{err}")),
        }
    }
    paths.sort_unstable();
    let mut latest = BTreeMap::<String, Extension>::new();
    for path in paths {
        match read_manifest(path.clone()) {
            Ok((key, extension))
                if latest
                    .get(&key)
                    .is_none_or(|current| extension.version > current.version) =>
            {
                latest.insert(key, extension);
            }
            Ok(_) => {}
            Err(err) => result.diagnose(&path, "package.json", format!("无法读取清单：{err}")),
        }
    }
    for extension in latest.into_values() {
        result.add(extension);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let tick = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("rove-extension-{}-{tick}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn write(&self, name: &str, contents: &str) {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn use_newest_manifest_and_keep_loading_other_extensions() {
        let root = Fixture::new();
        root.write(
            "old/package.json",
            r#"{"publisher":"sample","name":"themes","version":"1.2.0","contributes":{
            "themes":[{"label":"Older","path":"old.json","uiTheme":"vs-dark"}]}}"#,
        );
        root.write("old/old.json", "{}");
        root.write("new/package.json", r#"{"publisher":"sample","name":"themes","version":"1.10.0","contributes":{
            "themes":[{"id":"theme-id","label":"Newer","path":"colors/main.json","uiTheme":"vs-dark"}],
            "iconThemes":[{"id":"icons-id","label":"Icons","path":"icons.json"}]}}"#);
        root.write(
            "new/colors/main.json",
            r##"{"include":"../base.json","colors":{"editor.background":"#123456"}}"##,
        );
        root.write(
            "new/base.json",
            r##"{"colors":{"editor.foreground":"#fedcba"}}"##,
        );
        root.write("new/icons.json", "{}");
        root.write("broken/package.json", "[malformed");
        let scan = scan_extensions(&root.0);
        let theme = &scan.colors[0];
        assert_eq!(scan.colors.len(), 1);
        assert_eq!(theme.id.as_ref(), "theme-id");
        assert_eq!(theme.label.as_ref(), "Newer");
        assert_eq!(scan.icons[0].id.as_ref(), "icons-id");
        assert_eq!(scan.diagnostics.len(), 1);
        let document = crate::file::load(theme.source.as_ref(), &theme.path).unwrap();
        assert!(document.diagnostics.is_empty());
        assert_eq!(
            document.colors["editor.background"],
            crate::Color(18, 52, 86, 255)
        );
        assert_eq!(
            document.colors["editor.foreground"],
            crate::Color(254, 220, 186, 255)
        );
    }

    #[test]
    fn never_reads_script_or_package_dependencies_or_files_outside_extension() {
        let root = Fixture::new();
        root.write("theme/main.json", r#"{"include":"../outside.json"}"#);
        root.write("theme/inside.js", "{}");
        root.write("theme/node_modules/inside.json", "{}");
        root.write("outside.json", "{}");
        let files = ExtensionFiles(root.0.join("theme"));
        for path in [
            "../outside.json",
            "inside.js",
            "NODE_MODULES/inside.json",
            "https://example.com/other.json",
            r"\\host\share\other.json",
            "other.json:alternate",
            "/outside.json",
        ] {
            assert!(files.read(path).is_err(), "{path}");
        }
        let document = crate::file::load(&files, "main.json").unwrap();
        assert_eq!(document.diagnostics.len(), 1);
        assert!(document.diagnostics[0].file.contains("outside.json"));
    }

    #[test]
    fn prereleases_compare_by_parts_and_before_their_stable_version() {
        let version = |v| ExtensionVersion::parse(v).unwrap();
        assert!(version("1.2.3-rc.9") < version("1.2.3-rc.10"));
        assert!(version("1.2.3-rc.10") < version("1.2.3+build"));
        assert!(version("1.2.3") < version("1.2.4-beta"));
    }
}
