//! 打开文件的加载路径（#7）：读盘 → 编码识别与解码 → 不可变 rope 快照 → 元信息。
//!
//! 缓存与“同一个文件只打开一次”的策略属于 #17（`rove-ui` 的 `FileDocuments`），
//! 这里只负责把字节变成 [`LoadedText`]。

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use rove_base::*;
use rove_config::Settings;

use crate::contracts::{DocumentMetadata, ReadOnlyReason, SizeClass};
use crate::decode::{self, Detection};
use crate::settings::Preferences;
use crate::RopeText;

/// 一次打开的输入参数。`language` 用于 `files.encoding` 等设置的语言级覆盖，
/// 由调用方（工作区/语言配置）给出，本 crate 不自己判定语言。
pub struct LoadOptions<'a> {
    pub language: &'a str,
    pub settings: &'a dyn Settings,
    /// 用户明确指定的编码（“通过编码重新打开”）；优先于 BOM 与自动识别。
    pub explicit_encoding: Option<&'a str>,
    /// “仍然打开”：二进制/解码失败时不拒绝，按 lossy 文本显示。
    pub force_open: bool,
    /// 只读原因。默认是本次会话只读；权限/规则由 #15/#48 判定后传入。
    pub read_only: Arc<[ReadOnlyReason]>,
    /// 大小级别；本 issue 只实现 `Normal`，#8 接管分级。
    pub size: SizeClass,
}

impl<'a> LoadOptions<'a> {
    pub fn new(language: &'a str, settings: &'a dyn Settings) -> Self {
        Self {
            language,
            settings,
            explicit_encoding: None,
            force_open: false,
            read_only: Arc::from([ReadOnlyReason::Session]),
            size: SizeClass::Normal,
        }
    }
}

/// 加载结果：文本快照 + 文档元信息 + 编码结论。
///
/// `detection.guessed` 记录编码是否由自动识别得出。冻结的
/// [`DocumentMetadata`](rove_base::DocumentMetadata) 没有这个字段，
/// 需要状态栏悬停提示“自动识别”的 #27 只能通过本结构或后续契约变更拿到它。
#[derive(Debug)]
pub struct LoadedText {
    pub text: Arc<RopeText>,
    pub metadata: DocumentMetadata,
    pub detection: Detection,
}

/// 读取并加载一个文件。I/O 错误原样保留 `ErrorKind`。
pub fn open_path(path: &Path, options: &LoadOptions<'_>) -> Result<LoadedText> {
    let bytes = std::fs::read(path)?;
    open_bytes(&bytes, options)
}

/// 从已有字节加载；`open_path` 只是它的读盘包装，测试与基准可以复用它。
pub fn open_bytes(bytes: &[u8], options: &LoadOptions<'_>) -> Result<LoadedText> {
    let preferences = Preferences::read(options.settings, options.language);
    let decoded = decode::decode(
        bytes,
        &preferences,
        options.explicit_encoding,
        options.force_open,
    )?;
    let text = match decoded.text {
        Cow::Borrowed(text) => RopeText::from_text(text),
        Cow::Owned(text) => RopeText::new(text),
    };
    let metadata = DocumentMetadata {
        language: Arc::from(options.language),
        encoding: decoded.detection.encoding.clone(),
        bom: decoded.detection.bom,
        line_endings: decoded.line_endings,
        read_only: options.read_only.clone(),
        size: options.size,
    };
    Ok(LoadedText {
        text: Arc::new(text),
        metadata,
        detection: decoded.detection,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{Encoding, Eol, LineEndings, TextSource};
    use crate::settings;
    use rove_base::{ByteOffset, Line, Span};
    use rove_config::{DefaultSettings, SettingRegistry};

    fn settings() -> DefaultSettings {
        let mut registry = SettingRegistry::default();
        settings::register(&mut registry).unwrap();
        DefaultSettings(registry)
    }

    fn full_text(text: &dyn TextSource) -> String {
        let mut out = String::new();
        let range = Span::new(ByteOffset(0), ByteOffset(text.byte_len())).unwrap();
        text.visit_chunks(range, &mut |chunk| out.push_str(chunk))
            .unwrap();
        out
    }

    #[test]
    fn opens_bytes_with_metadata() {
        let settings = settings();
        let options = LoadOptions::new("csharp", &settings);
        let loaded = open_bytes(b"// a\r\n// b\n", &options).unwrap();
        assert_eq!(loaded.metadata.language.as_ref(), "csharp");
        assert_eq!(loaded.metadata.encoding, Encoding::Utf8);
        assert!(!loaded.metadata.bom);
        assert!(matches!(
            loaded.metadata.line_endings,
            LineEndings::Mixed {
                preferred: Eol::CrLf
            }
        ));
        assert_eq!(loaded.metadata.size, SizeClass::Normal);
        assert_eq!(
            loaded.metadata.read_only.as_ref(),
            [ReadOnlyReason::Session]
        );
        assert_eq!(loaded.text.lines().known, 3);
        assert_eq!(
            loaded.text.line_range(Line(0)).unwrap().end(),
            ByteOffset(4)
        );
        assert_eq!(full_text(loaded.text.as_ref()), "// a\r\n// b\n");
        assert!(!loaded.detection.guessed);
    }

    #[test]
    fn reports_io_errors_with_their_kind() {
        let settings = settings();
        let options = LoadOptions::new("csharp", &settings);
        let error = open_path(Path::new("no-such-file-rove-test.cs"), &options).unwrap_err();
        assert!(matches!(
            error,
            Error::Io {
                kind: std::io::ErrorKind::NotFound,
                ..
            }
        ));
    }

    #[test]
    fn force_open_keeps_binary_bytes_as_lossy_text() {
        let settings = settings();
        let mut options = LoadOptions::new("plaintext", &settings);
        assert_eq!(
            open_bytes(b"abc\0def", &options).err(),
            Some(Error::Unsupported)
        );
        options.force_open = true;
        let loaded = open_bytes(b"abc\0def", &options).unwrap();
        assert_eq!(full_text(loaded.text.as_ref()), "abc\0def");
    }
}
