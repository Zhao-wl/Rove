//! 编码设置项的登记与读取（#7 消费 #1 的设置契约）。
//!
//! 只登记键、默认值与语言覆盖能力，不读取也不写回设置文件：设置层是 #13。
//! `DefaultSettings` 在用户没有显式设置时返回 `rove_default`，这正是 Rove 的
//! 差异默认值（`files.autoGuessEncoding` 默认开启、候选编码默认 GBK）的生效方式，
//! 绝不写回共用的 `settings.json`。

use std::borrow::Cow;
use std::sync::Arc;

use encoding_rs::{Encoding as RsEncoding, GBK, UTF_16BE, UTF_16LE, UTF_8};
use rove_base::{Result, Value};
use rove_config::{Key, Scope, Setting, SettingRegistry, Settings};

use crate::contracts::Encoding;

pub const FILES_ENCODING: &str = "files.encoding";
pub const FILES_AUTO_GUESS_ENCODING: &str = "files.autoGuessEncoding";
pub const FILES_CANDIDATE_GUESS_ENCODINGS: &str = "files.candidateGuessEncodings";

/// Rove 在用户未设置时的缺省编码。
pub const ROVE_DEFAULT_ENCODING: &str = "utf8";
/// Rove 在用户未设置时的缺省自动识别开关（VS Code 默认 `false`，见 `compat/SETTINGS.md` §3）。
pub const ROVE_DEFAULT_AUTO_GUESS: bool = true;
/// Rove 在用户未设置时的缺省候选编码（见 `compat/SETTINGS.md` §3）。
pub const ROVE_DEFAULT_CANDIDATES: [&str; 1] = ["gbk"];

/// 把本 crate 消费的三个 `files.*` 键登记进设置表。装配方在启动时调用一次。
pub fn register(registry: &mut SettingRegistry) -> Result<()> {
    registry.register(
        FILES_ENCODING,
        Setting {
            default: Value::String(ROVE_DEFAULT_ENCODING.into()),
            rove_default: None,
            language_overridable: true,
        },
    )?;
    registry.register(
        FILES_AUTO_GUESS_ENCODING,
        Setting {
            default: Value::Bool(false),
            rove_default: Some(Value::Bool(ROVE_DEFAULT_AUTO_GUESS)),
            language_overridable: true,
        },
    )?;
    registry.register(
        FILES_CANDIDATE_GUESS_ENCODINGS,
        Setting {
            default: Value::Array(Arc::from(Vec::<Value>::new())),
            rove_default: Some(Value::Array(Arc::from(
                ROVE_DEFAULT_CANDIDATES
                    .iter()
                    .map(|id| Value::String((*id).into()))
                    .collect::<Vec<_>>(),
            ))),
            language_overridable: false,
        },
    )?;
    Ok(())
}

/// 把 VS Code 的编码 id（`files.encoding` / 候选编码列表里的字符串）映射到 `encoding_rs`。
///
/// VS Code 的 id 与 WHATWG label 不完全一致（`utf16le`、`windows1252`、`iso88592`、
/// `shiftjis` …），先归一化成 WHATWG label 再交给 `Encoding::for_label`。
pub fn encoding_from_id(id: &str) -> Option<&'static RsEncoding> {
    let id = id.trim().to_ascii_lowercase();
    let label: Cow<'_, str> = if let Some(digits) = id.strip_prefix("windows") {
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            Cow::Owned(format!("windows-{digits}"))
        } else {
            Cow::Borrowed(id.as_str())
        }
    } else if let Some(suffix) = id.strip_prefix("iso8859") {
        if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
            Cow::Owned(format!("iso-8859-{suffix}"))
        } else {
            Cow::Borrowed(id.as_str())
        }
    } else {
        match id.as_str() {
            "utf8" | "utf8bom" | "utf-8" => Cow::Borrowed("utf-8"),
            "utf16le" => Cow::Borrowed("utf-16le"),
            "utf16be" => Cow::Borrowed("utf-16be"),
            "gb2312" => Cow::Borrowed("gbk"),
            "cp950" | "big5hkscs" => Cow::Borrowed("big5"),
            "shiftjis" => Cow::Borrowed("shift_jis"),
            "eucjp" => Cow::Borrowed("euc-jp"),
            "euckr" => Cow::Borrowed("euc-kr"),
            "macroman" => Cow::Borrowed("macintosh"),
            "cp866" => Cow::Borrowed("ibm866"),
            "koi8r" => Cow::Borrowed("koi8-r"),
            "koi8u" => Cow::Borrowed("koi8-u"),
            other => Cow::Borrowed(other),
        }
    };
    encoding_rs::Encoding::for_label(label.as_bytes())
}

/// 元信息里的编码 → `encoding_rs`，供写回（#48）与往返测试使用。
pub fn rs_encoding(encoding: &Encoding) -> Option<&'static RsEncoding> {
    match encoding {
        Encoding::Utf8 => Some(UTF_8),
        Encoding::Utf16Le => Some(UTF_16LE),
        Encoding::Utf16Be => Some(UTF_16BE),
        Encoding::Gbk => Some(GBK),
        Encoding::Other(name) => encoding_rs::Encoding::for_label(name.as_bytes()),
    }
}

/// `encoding_rs` 的编码 → 文档元信息里的编码。未覆盖的编码保留 `encoding_rs` 的名字。
pub fn metadata_encoding(encoding: &'static RsEncoding) -> Encoding {
    if encoding == UTF_8 {
        Encoding::Utf8
    } else if encoding == UTF_16LE {
        Encoding::Utf16Le
    } else if encoding == UTF_16BE {
        Encoding::Utf16Be
    } else if encoding == GBK {
        Encoding::Gbk
    } else {
        Encoding::Other(Arc::from(encoding.name()))
    }
}

/// 三处编码设置解析后的快照；只在一次打开操作内使用。
#[derive(Clone, Debug)]
pub(crate) struct Preferences {
    pub auto_guess: bool,
    /// 用户给出的候选编码，保持顺序；未设置时是 Rove 缺省 `["gbk"]`。
    pub candidates: Vec<&'static RsEncoding>,
    /// 猜不出来时使用的 `files.encoding`。
    pub fallback: &'static RsEncoding,
}

impl Preferences {
    /// 按语言作用域读取设置；未登记、类型错误或取值无法识别时回落到 Rove 缺省值。
    pub fn read(settings: &dyn Settings, language: &str) -> Self {
        let scope = Scope {
            language: (!language.is_empty()).then(|| Arc::<str>::from(language)),
        };
        let encoding_id = Key::<String>::new(FILES_ENCODING)
            .read(settings, &scope)
            .unwrap_or_else(|_| ROVE_DEFAULT_ENCODING.to_string());
        // `files.encoding = "utf8bom"` 读取时同样是 UTF-8；BOM 是否存在看文件本身。
        let fallback = encoding_from_id(&encoding_id).unwrap_or(UTF_8);
        let auto_guess = Key::<bool>::new(FILES_AUTO_GUESS_ENCODING)
            .read(settings, &scope)
            .unwrap_or(ROVE_DEFAULT_AUTO_GUESS);
        let candidates = Key::<Value>::new(FILES_CANDIDATE_GUESS_ENCODINGS)
            .read(settings, &scope)
            .ok()
            .and_then(|value| match value {
                Value::Array(items) => Some(
                    items
                        .iter()
                        .filter_map(|item| match item {
                            Value::String(id) => encoding_from_id(id),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_else(|| {
                ROVE_DEFAULT_CANDIDATES
                    .iter()
                    .filter_map(|id| encoding_from_id(id))
                    .collect()
            });
        Self {
            auto_guess,
            candidates,
            fallback,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_config::DefaultSettings;

    fn registered() -> DefaultSettings {
        let mut registry = SettingRegistry::default();
        register(&mut registry).unwrap();
        DefaultSettings(registry)
    }

    #[test]
    fn vscode_encoding_ids_map_to_whatwg_encodings() {
        assert_eq!(encoding_from_id("utf8"), Some(UTF_8));
        assert_eq!(encoding_from_id("UTF8BOM"), Some(UTF_8));
        assert_eq!(encoding_from_id("utf16le"), Some(UTF_16LE));
        assert_eq!(encoding_from_id("utf16be"), Some(UTF_16BE));
        assert_eq!(encoding_from_id("gbk"), Some(GBK));
        assert_eq!(encoding_from_id("gb2312"), Some(GBK));
        assert_eq!(encoding_from_id("gb18030"), Some(encoding_rs::GB18030));
        assert_eq!(
            encoding_from_id("windows1252"),
            Some(encoding_rs::WINDOWS_1252)
        );
        assert_eq!(
            encoding_from_id("iso88591"),
            Some(encoding_rs::WINDOWS_1252)
        );
        assert_eq!(encoding_from_id("shiftjis"), Some(encoding_rs::SHIFT_JIS));
        assert_eq!(encoding_from_id("cp950"), Some(encoding_rs::BIG5));
        assert!(encoding_from_id("no-such-encoding").is_none());
    }

    #[test]
    fn rove_defaults_apply_only_through_the_registry() {
        let settings = registered();
        let preferences = Preferences::read(&settings, "csharp");
        assert!(preferences.auto_guess);
        assert_eq!(preferences.candidates, vec![GBK]);
        assert_eq!(preferences.fallback, UTF_8);
    }

    #[test]
    fn explicit_values_win_over_rove_defaults() {
        struct Explicit;
        impl Settings for Explicit {
            fn read(&self, key: &str, _: &Scope) -> Result<Value> {
                match key {
                    FILES_ENCODING => Ok(Value::String("utf8bom".into())),
                    FILES_AUTO_GUESS_ENCODING => Ok(Value::Bool(false)),
                    FILES_CANDIDATE_GUESS_ENCODINGS => Ok(Value::Array(Arc::from([
                        Value::String("big5".into()),
                        Value::String("no-such-encoding".into()),
                    ]))),
                    _ => Err(rove_base::Error::NotFound),
                }
            }
            fn write(
                &self,
                _: Arc<str>,
                _: Scope,
                _: Value,
                _: rove_base::Cancellation,
            ) -> rove_base::Task<()> {
                Box::pin(async { Err(rove_base::Error::Unsupported) })
            }
            fn changes(&self) -> &dyn rove_base::Events<rove_config::SettingsChange> {
                &rove_base::NoEvents
            }
        }
        let preferences = Preferences::read(&Explicit, "");
        assert!(!preferences.auto_guess);
        assert_eq!(preferences.candidates, vec![encoding_rs::BIG5]);
        assert_eq!(preferences.fallback, UTF_8);
    }

    #[test]
    fn metadata_encoding_round_trips_through_rs_encoding() {
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
            Encoding::Gbk,
            Encoding::Other(Arc::from(encoding_rs::WINDOWS_1252.name())),
        ] {
            let rs = rs_encoding(&encoding).expect("known encoding");
            assert_eq!(metadata_encoding(rs), encoding);
        }
    }
}
