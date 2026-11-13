//! 预览页与打包资源（#29）：全部内容编进 `rove.exe`，经 WebView2 的资源请求事件从内存返回，
//! 运行时不读安装目录、不联网。
//!
//! | 资源 | 来源 | 许可 |
//! |---|---|---|
//! | `vendor/markdown.css`、`vendor/highlight.css` | VS Code 1.145.2 `extensions/markdown-language-features/media/` | MIT |
//! | `vendor/highlight.min.js` | highlight.js 11.11.1（`@highlightjs/cdn-assets`，常用语言包） | BSD-3-Clause |
//! | `vendor/katex/*` | KaTeX 0.16.25（`katex.min.js`、`katex.min.css`、woff2 字体） | MIT |
//! | `vendor/mermaid.min.js` | Mermaid 11.17.2 | MIT |
//! | `bridge.js`、`rove.css`、`blocked-image.svg` | Rove 自写 | — |
use rove_theme::{Color, Theme, ThemeKind};
use serde_json::{Map, Value};
use std::{collections::BTreeSet, sync::OnceLock};

macro_rules! asset {
    ($path:literal, $mime:literal) => {
        (
            $path,
            $mime,
            include_bytes!(concat!("../../assets/markdown/", $path)) as &[u8],
        )
    };
}

/// (相对 `https://rove.local/` 的路径, MIME 类型, 内容)
pub const ASSETS: &[(&str, &str, &[u8])] = &[
    asset!("bridge.js", "text/javascript"),
    asset!("rove.css", "text/css"),
    asset!("blocked-image.svg", "image/svg+xml"),
    asset!("vendor/markdown.css", "text/css"),
    asset!("vendor/highlight.css", "text/css"),
    asset!("vendor/highlight.min.js", "text/javascript"),
    asset!("vendor/mermaid.min.js", "text/javascript"),
    asset!("vendor/katex/katex.min.js", "text/javascript"),
    asset!("vendor/katex/katex.min.css", "text/css"),
    asset!("vendor/katex/fonts/KaTeX_AMS-Regular.woff2", "font/woff2"),
    asset!(
        "vendor/katex/fonts/KaTeX_Caligraphic-Bold.woff2",
        "font/woff2"
    ),
    asset!(
        "vendor/katex/fonts/KaTeX_Caligraphic-Regular.woff2",
        "font/woff2"
    ),
    asset!("vendor/katex/fonts/KaTeX_Fraktur-Bold.woff2", "font/woff2"),
    asset!(
        "vendor/katex/fonts/KaTeX_Fraktur-Regular.woff2",
        "font/woff2"
    ),
    asset!("vendor/katex/fonts/KaTeX_Main-Bold.woff2", "font/woff2"),
    asset!(
        "vendor/katex/fonts/KaTeX_Main-BoldItalic.woff2",
        "font/woff2"
    ),
    asset!("vendor/katex/fonts/KaTeX_Main-Italic.woff2", "font/woff2"),
    asset!("vendor/katex/fonts/KaTeX_Main-Regular.woff2", "font/woff2"),
    asset!(
        "vendor/katex/fonts/KaTeX_Math-BoldItalic.woff2",
        "font/woff2"
    ),
    asset!("vendor/katex/fonts/KaTeX_Math-Italic.woff2", "font/woff2"),
    asset!(
        "vendor/katex/fonts/KaTeX_SansSerif-Bold.woff2",
        "font/woff2"
    ),
    asset!(
        "vendor/katex/fonts/KaTeX_SansSerif-Italic.woff2",
        "font/woff2"
    ),
    asset!(
        "vendor/katex/fonts/KaTeX_SansSerif-Regular.woff2",
        "font/woff2"
    ),
    asset!(
        "vendor/katex/fonts/KaTeX_Script-Regular.woff2",
        "font/woff2"
    ),
    asset!("vendor/katex/fonts/KaTeX_Size1-Regular.woff2", "font/woff2"),
    asset!("vendor/katex/fonts/KaTeX_Size2-Regular.woff2", "font/woff2"),
    asset!("vendor/katex/fonts/KaTeX_Size3-Regular.woff2", "font/woff2"),
    asset!("vendor/katex/fonts/KaTeX_Size4-Regular.woff2", "font/woff2"),
    asset!(
        "vendor/katex/fonts/KaTeX_Typewriter-Regular.woff2",
        "font/woff2"
    ),
];

/// 预览页的地址（WebView2 只允许导航到这里）。
pub const INDEX_URL: &str = "https://rove.local/index.html";

/// 按路径取打包资源（`index.html` 由 [`index_html`] 生成，不在表里）。
pub fn asset(path: &str) -> Option<(&'static str, &'static [u8])> {
    ASSETS
        .iter()
        .find(|(p, _, _)| *p == path)
        .map(|(_, mime, bytes)| (*mime, *bytes))
}

/// 预览页：CSP 只放行带本次随机 nonce 的打包脚本（文档里的脚本即使漏过净化也不会执行）；
/// 样式与字体只来自打包主机和本地文件主机；图片另外允许 `https:`/`http:`，但远程请求在没有放行时
/// 会被 WebView2 的资源请求拦截（返回 403）。
pub fn index_html(nonce: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-{nonce}'; style-src https://rove.local https://*.rove-file.local https: 'unsafe-inline'; img-src https://rove.local https://*.rove-file.local https: http: data:; media-src https://rove.local https://*.rove-file.local https: http: data:; font-src https://rove.local https://*.rove-file.local data:; connect-src 'none'; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'">
<link rel="stylesheet" href="https://rove.local/vendor/markdown.css">
<link rel="stylesheet" href="https://rove.local/vendor/highlight.css">
<link rel="stylesheet" href="https://rove.local/vendor/katex/katex.min.css">
<link rel="stylesheet" href="https://rove.local/rove.css">
</head>
<body class="vscode-body vscode-dark">
<div id="rove-content"></div>
<script nonce="{nonce}" src="https://rove.local/vendor/highlight.min.js"></script>
<script nonce="{nonce}" src="https://rove.local/bridge.js"></script>
</body>
</html>
"#
    )
}

/// 一个不可预测的 nonce（每个 WebView2 实例一份）。
pub fn nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for _ in 0..2 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

/// 预览样式里引用的全部 `--vscode-*` 变量名（扫描打包的 CSS，只做一次）。
pub fn css_variables() -> &'static BTreeSet<String> {
    static NAMES: OnceLock<BTreeSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = BTreeSet::new();
        for (path, _, bytes) in ASSETS {
            if !path.ends_with(".css") {
                continue;
            }
            let text = String::from_utf8_lossy(bytes);
            let mut rest = text.as_ref();
            while let Some(at) = rest.find("--vscode-") {
                let tail = &rest[at..];
                let end = tail
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                    .unwrap_or(tail.len());
                if end > "--vscode-".len() {
                    names.insert(tail[..end].to_string());
                }
                rest = &tail[end..];
            }
        }
        names
    })
}

/// `--vscode-textLink-foreground` → `textLink.foreground`（VS Code 的颜色键不含 `-`）。
pub fn color_key(variable: &str) -> Option<String> {
    let name = variable.strip_prefix("--vscode-")?;
    Some(name.replace('-', "."))
}

fn css_color(color: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        color.0, color.1, color.2, color.3
    )
}

/// VS Code 网页视图的主题类名。
pub fn theme_class(kind: ThemeKind) -> &'static str {
    match kind {
        ThemeKind::Dark => "vscode-dark",
        ThemeKind::Light => "vscode-light",
        ThemeKind::HighContrastDark => "vscode-high-contrast",
        ThemeKind::HighContrastLight => "vscode-high-contrast-light",
    }
}

/// 字体设置（`markdown.preview.fontFamily/fontSize/lineHeight` 与 `editor.fontFamily/fontSize`）。
#[derive(Clone, Debug, PartialEq)]
pub struct Fonts {
    pub family: String,
    pub size: f64,
    pub line_height: f64,
    pub editor_family: String,
    pub editor_size: f64,
}

/// 推给预览页的主题消息：颜色变量、字体变量与主题类名。主题切换时只发这条消息，不重建实例。
pub fn theme_message(theme: &dyn Theme, kind: ThemeKind, fonts: &Fonts) -> Value {
    let mut vars = Map::new();
    for name in css_variables() {
        let Some(key) = color_key(name) else { continue };
        let value = match theme.color(&key) {
            Ok(Some(color)) => Value::String(css_color(color)),
            _ => Value::Null,
        };
        vars.insert(name.clone(), value);
    }
    vars.insert(
        "--markdown-font-family".into(),
        Value::String(fonts.family.clone()),
    );
    vars.insert(
        "--markdown-font-size".into(),
        Value::String(format!("{}px", fonts.size)),
    );
    vars.insert(
        "--markdown-line-height".into(),
        Value::String(format!("{}", fonts.line_height)),
    );
    vars.insert(
        "--vscode-editor-font-family".into(),
        Value::String(fonts.editor_family.clone()),
    );
    vars.insert(
        "--vscode-editor-font-size".into(),
        Value::String(format!("{}px", fonts.editor_size)),
    );
    serde_json::json!({
        "version": 1,
        "type": "theme",
        "kind": theme_class(kind),
        "vars": vars,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_asset_is_bundled_and_the_page_references_only_bundled_scripts() {
        for (path, mime, bytes) in ASSETS {
            assert!(!bytes.is_empty(), "{path}");
            assert!(!mime.is_empty());
        }
        let page = index_html("abc");
        for url in page.split("src=\"").skip(1) {
            let url = url.split('"').next().unwrap();
            let path = url.strip_prefix("https://rove.local/").unwrap();
            assert!(asset(path).is_some(), "{path}");
        }
        assert!(page.contains("script-src 'nonce-abc'"));
        assert!(page.contains("connect-src 'none'"));
        // katex.min.css 引用的 woff2 字体都已打包（ttf/woff 回退永远不会被请求）。
        let (_, css) = asset("vendor/katex/katex.min.css").unwrap();
        let css = String::from_utf8_lossy(css);
        for font in css.split("url(fonts/").skip(1) {
            let name = font.split(')').next().unwrap();
            if name.ends_with(".woff2") {
                assert!(
                    asset(&format!("vendor/katex/fonts/{name}")).is_some(),
                    "{name}"
                );
            }
        }
        assert_ne!(nonce(), nonce());
    }

    #[test]
    fn theme_variables_map_to_registered_color_keys() {
        let names = css_variables();
        assert!(names.contains("--vscode-textLink-foreground"));
        assert!(names.contains("--vscode-markdownAlert-note-foreground"));
        assert_eq!(
            color_key("--vscode-markdownAlert-note-foreground").as_deref(),
            Some("markdownAlert.note.foreground")
        );
        let mut colors = rove_theme::service::p0_registry();
        super::super::registry::register_colors(&mut colors).unwrap();
        for name in names {
            let key = color_key(name).unwrap();
            if key.starts_with("editor.font") {
                continue;
            }
            assert!(colors.get(&key).is_some(), "未登记的颜色键 {key}");
        }
    }
}
