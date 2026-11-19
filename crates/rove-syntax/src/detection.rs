//! 对有界文本前缀做保守、离线的语言识别。

use jsonc_parser::ParseOptions;
use rove_base::{ByteOffset, Error, Result, Span};
use rove_buffer::TextSource;
use serde_json::Value;

pub const MAX_BYTES: usize = 8 * 1024;
pub const MAX_LINES: usize = 100;
const MIN_YAML_ROWS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// 明确的声明或完整解析。
    High,
    /// 有结构的线索，足以选择对应的语法。
    Likely,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detection {
    pub language: &'static str,
    pub confidence: Confidence,
}

enum Decision {
    Found(Detection),
    Unsupported,
    Continue,
}

impl Decision {
    fn found(language: &'static str, confidence: Confidence) -> Self {
        Self::Found(Detection {
            language,
            confidence,
        })
    }
}

type Rule = fn(&str, bool) -> Decision;
const RULES: &[Rule] = &[shebang, first_line, json, xml, yaml];

/// 在调用方所在的后台线程读取最多 8 KiB，规则只分析其中的前 100 行。
/// 这条路径不访问行索引，也能处理行索引尚未就绪的超大内存映射文件。
pub fn detect_text(text: &dyn TextSource) -> Result<Option<Detection>> {
    let len = text.byte_len();
    let end = len.min(MAX_BYTES as u64);
    let mut prefix = String::with_capacity(end as usize);
    // 8 KiB 边界最多落在剩余三个字节的 UTF-8 字符中间。
    for trim in 0..=3 {
        let end = end.saturating_sub(trim);
        let range = Span::new(ByteOffset(0), ByteOffset(end))?;
        match text.visit_chunks(range, &mut |chunk| prefix.push_str(chunk)) {
            Ok(()) => return Ok(detect_prefix(&prefix, end == len)),
            Err(Error::OutOfBounds) if end != 0 && trim < 3 => prefix.clear(),
            Err(error) => return Err(error),
        }
    }
    unreachable!("the final prefix attempt always returns")
}

/// 关联与扩展名判断由调用方执行，手动命令可以直接使用此规则表。
/// complete 表示前缀已覆盖全文，JSON/JSONC 仅在这个条件下尝试完整解析。
pub fn detect_prefix(text: &str, complete: bool) -> Option<Detection> {
    let by_bytes = text.floor_char_boundary(MAX_BYTES);
    let bytes = &text.as_bytes()[..by_bytes];
    let by_line = bytes
        .iter()
        .enumerate()
        .filter(|&(i, &byte)| byte == b'\n' || byte == b'\r' && bytes.get(i + 1) != Some(&b'\n'))
        .nth(MAX_LINES - 1)
        .map_or(by_bytes, |(end, _)| end + 1);
    let complete = complete && by_line == text.len();
    let text = text[..by_line]
        .strip_prefix('\u{feff}')
        .unwrap_or(&text[..by_line]);
    if text.contains('\0') {
        return None;
    }
    for rule in RULES {
        match rule(text, complete) {
            Decision::Found(found) => return Some(found),
            Decision::Unsupported => return None,
            Decision::Continue => (),
        }
    }
    None
}

fn shebang(text: &str, _: bool) -> Decision {
    let Some(line) = text
        .split(['\r', '\n'])
        .next()
        .and_then(|line| line.strip_prefix("#!"))
    else {
        return Decision::Continue;
    };
    let mut words = line.split_ascii_whitespace();
    let mut interpreter = words
        .next()
        .map(|word| word.rsplit('/').next().unwrap_or_default());
    if interpreter == Some("env") {
        let name = words.next();
        interpreter = if name == Some("-S") {
            words.next()
        } else {
            name
        }
        .map(|word| word.rsplit('/').next().unwrap_or_default());
    }
    match interpreter {
        Some("lua" | "luajit" | "lua5.1" | "lua5.2" | "lua5.3" | "lua5.4" | "lua5.5") => {
            Decision::found("lua", Confidence::High)
        }
        _ => Decision::Unsupported,
    }
}

fn first_line(text: &str, _: bool) -> Decision {
    let text = text.trim_start();
    let line = text
        .split(['\r', '\n'])
        .next()
        .unwrap_or_default()
        .trim_end();
    if let Some(doctype) = line
        .get(.."<!DOCTYPE".len())
        .filter(|prefix| prefix.eq_ignore_ascii_case("<!DOCTYPE"))
    {
        let name = line[doctype.len()..]
            .trim_start()
            .split([' ', '\t', '>'])
            .next();
        if name.is_some_and(|name| name.eq_ignore_ascii_case("html")) {
            return Decision::Unsupported;
        }
    }
    if let Some(declaration) = text
        .strip_prefix("<?xml")
        .filter(|declaration| declaration.starts_with(char::is_whitespace))
    {
        return if declaration.split_once("?>").is_some() {
            Decision::found("xml", Confidence::High)
        } else {
            Decision::Unsupported
        };
    }
    if line == "---" {
        let mut since_marker = false;
        for line in text.split(['\r', '\n']).skip(1) {
            if line.trim_end() == "---" && since_marker {
                return Decision::found("yaml", Confidence::High);
            }
            since_marker |= yaml_item(line).is_some();
        }
    }
    Decision::Continue
}

const STRICT_JSONC: ParseOptions = ParseOptions {
    allow_comments: true,
    allow_loose_object_property_names: false,
    allow_trailing_commas: true,
    allow_missing_commas: false,
    allow_single_quoted_strings: false,
    allow_hexadecimal_numbers: false,
    allow_unary_plus_numbers: false,
    allow_bare_decimal_point_numbers: false,
    allow_non_finite_numbers: false,
    allow_extended_string_escapes: false,
};

fn json_head(mut text: &str) -> &str {
    loop {
        text = text.trim_start();
        if let Some(line) = text.strip_prefix("//") {
            text = line.split_once(['\r', '\n']).map_or("", |(_, next)| next);
        } else if let Some(comment) = text.strip_prefix("/*") {
            text = comment.split_once("*/").map_or("", |(_, next)| next);
        } else {
            return text;
        }
    }
}

fn json(text: &str, complete: bool) -> Decision {
    if !json_head(text).starts_with(['{', '[']) {
        return Decision::Continue;
    }
    if !complete {
        return Decision::Unsupported;
    }
    if serde_json::from_str::<Value>(text).is_ok_and(|v| v.is_object() || v.is_array()) {
        return Decision::found("json", Confidence::High);
    }
    if jsonc_parser::parse_to_serde_value::<Value>(text, &STRICT_JSONC)
        .is_ok_and(|v| v.is_object() || v.is_array())
    {
        return Decision::found("jsonc", Confidence::High);
    }
    Decision::Unsupported
}

fn xml(text: &str, _: bool) -> Decision {
    let mut text = text.trim_start();
    while let Some(comment) = text.strip_prefix("<!--") {
        let Some((_, rest)) = comment.split_once("-->") else {
            return Decision::Unsupported;
        };
        text = rest.trim_start();
    }
    if !text.starts_with('<') {
        return Decision::Continue;
    }
    match read_start_tag(text) {
        Some(name) if !name.eq_ignore_ascii_case("html") => {
            Decision::found("xml", Confidence::Likely)
        }
        _ => Decision::Unsupported,
    }
}

fn name_prefix(text: &str) -> Option<(&str, &str)> {
    let first = text.chars().next()?;
    if !(first.is_alphabetic() || matches!(first, '_' | ':')) {
        return None;
    }
    let end = text
        .find(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | ':' | '-' | '.')))
        .unwrap_or(text.len());
    Some(text.split_at(end))
}

fn read_start_tag(text: &str) -> Option<&str> {
    let (name, mut rest) = name_prefix(text.strip_prefix('<')?)?;
    loop {
        let trimmed = rest.trim_start();
        if trimmed.starts_with('>') || trimmed.starts_with("/>") {
            return Some(name);
        }
        if trimmed.len() == rest.len() {
            return None;
        }
        let (_, after_name) = name_prefix(trimmed)?;
        let value = after_name.trim_start().strip_prefix('=')?.trim_start();
        let quote = value.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
        let (value, tail) = value[1..].split_once(quote)?;
        if value.contains('<') {
            return None;
        }
        rest = tail;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum YamlItem {
    Sequence,
    Mapping,
}

fn yaml_item(line: &str) -> Option<(usize, YamlItem)> {
    let entry = line.trim_start_matches(' ');
    let indent = line.len() - entry.len();
    if entry
        .strip_prefix("- ")
        .is_some_and(|item| !item.trim().is_empty())
    {
        return Some((indent, YamlItem::Sequence));
    }
    let (key, value) = entry.split_once(':')?;
    if !key.starts_with(|ch: char| ch.is_alphabetic() || ch == '_')
        || !key
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        || !value.starts_with(' ')
        || value.trim().is_empty()
    {
        return None;
    }
    Some((indent, YamlItem::Mapping))
}

fn yaml(text: &str, _: bool) -> Decision {
    let mut previous = None;
    let mut run = 0;
    for line in text.split(['\r', '\n']) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let item = yaml_item(line);
        run = if item.is_some() && item == previous {
            run + 1
        } else {
            1
        };
        previous = item;
        if item.is_some() && run >= MIN_YAML_ROWS {
            return Decision::found("yaml", Confidence::Likely);
        }
    }
    Decision::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_supported_runtimes_are_inferred_from_shebangs() {
        for sample in [
            "#!/usr/bin/env lua\nprint(1)",
            "#!/usr/bin/env -S lua -v",
            "#!/usr/bin/lua5.4\n",
        ] {
            assert_eq!(detect_prefix(sample, true).unwrap().language, "lua");
        }
        for sample in [
            "#!/bin/sh\na: 1\nb: 2\nc: 3",
            "#!/usr/bin/env python3",
            "#!",
        ] {
            assert_eq!(detect_prefix(sample, true), None, "{sample:?}");
        }
    }

    #[test]
    fn explicit_xml_and_yaml_markers_take_priority_over_structure() {
        for (sample, language) in [
            ("<?xml version=\"1.0\"?>\n<root/>", "xml"),
            ("---\nname: x\n---\nname: y", "yaml"),
        ] {
            assert_eq!(
                detect_prefix(sample, true),
                Some(Detection {
                    language,
                    confidence: Confidence::High
                })
            );
        }
        for sample in [
            "<!doctype HTML>\n<root/>",
            "<?xml version=",
            "---\nA heading\n---",
        ] {
            assert_eq!(detect_prefix(sample, true), None, "{sample:?}");
        }
    }

    #[test]
    fn json_and_jsonc_require_full_valid_containers() {
        for (sample, language) in [
            ("  {\"a\": [1]}", "json"),
            ("[null, true]", "json"),
            ("// Comment\n{\"a\":1,}", "jsonc"),
            ("/* comment */ [1]", "jsonc"),
        ] {
            assert_eq!(detect_prefix(sample, true).unwrap().language, language);
            assert_eq!(detect_prefix(sample, false), None);
        }
        for sample in [
            "{ unquoted: 1 }",
            "[1 2]",
            "{\"a\":",
            "{\"a\":1} rest",
            "123",
            "true",
        ] {
            assert_eq!(detect_prefix(sample, true), None, "{sample:?}");
        }
    }

    #[test]
    fn xml_accepts_only_a_complete_first_tag() {
        for sample in [
            "<project key='a'>",
            "<!-- note -->\n<config/>",
            "<根 ns:attr=\"a>b\" />",
        ] {
            assert_eq!(detect_prefix(sample, false).unwrap().language, "xml");
        }
        for sample in [
            "< 10",
            "<node attr without a value>",
            "<html lang='en'>",
            "<root attr='unfinished",
            "</root>",
        ] {
            assert_eq!(detect_prefix(sample, true), None, "{sample:?}");
        }
    }

    #[test]
    fn yaml_requires_consecutive_items_at_the_same_indent() {
        for sample in ["a: one\nb: two\nc: three", "  - one\n  - two\n  - three"] {
            assert_eq!(detect_prefix(sample, true).unwrap().language, "yaml");
        }
        for sample in [
            "Title: a single line",
            "a: 1\n b: 2\nc: 3",
            "one: x\ntwo: y\nfree text\nthree: z",
            "ordinary plain text",
        ] {
            assert_eq!(detect_prefix(sample, true), None, "{sample:?}");
        }
    }

    struct TrackedSource<'a> {
        inner: &'a dyn TextSource,
        reads: std::sync::Mutex<Vec<rove_base::ByteRange>>,
    }

    impl TextSource for TrackedSource<'_> {
        fn version(&self) -> rove_base::Version {
            self.inner.version()
        }

        fn byte_len(&self) -> u64 {
            self.inner.byte_len()
        }

        fn lines(&self) -> rove_buffer::LineCount {
            panic!("detection must not require a line index");
        }

        fn line_range(&self, _: rove_base::Line) -> Result<rove_base::ByteRange> {
            panic!("detection must not require a line index");
        }

        fn visit_chunks(
            &self,
            range: rove_base::ByteRange,
            visitor: &mut dyn FnMut(&str),
        ) -> Result<()> {
            self.reads.lock().unwrap().push(range.clone());
            self.inner.visit_chunks(range, visitor)
        }

        fn offset(&self, _: rove_base::Position) -> Result<ByteOffset> {
            panic!("detection must not convert positions");
        }

        fn position(&self, _: ByteOffset) -> Result<rove_base::Position> {
            panic!("detection must not convert positions");
        }

        fn utf16_to_utf8(
            &self,
            _: rove_base::Line,
            _: rove_base::Utf16Column,
        ) -> Result<rove_base::Utf8Column> {
            panic!("detection must not convert positions");
        }

        fn utf8_to_utf16(
            &self,
            _: rove_base::Line,
            _: rove_base::Utf8Column,
        ) -> Result<rove_base::Utf16Column> {
            panic!("detection must not convert positions");
        }
    }

    #[test]
    fn mapped_document_only_exposes_the_first_eight_kibibytes() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("no-extension");
        let source = format!("<root>\n{}\n</root>", " ".repeat(2 * 1024 * 1024));
        std::fs::write(&path, source).unwrap();
        let settings = rove_config::DefaultSettings(rove_config::SettingRegistry::default());
        let opened = rove_buffer::open_file(
            &path,
            &rove_buffer::LoadOptions::new("plaintext", &settings),
            rove_buffer::SizePolicy {
                huge_bytes: 1024 * 1024,
                optimize: true,
            },
        )
        .unwrap();
        assert_eq!(opened.metadata.size, rove_buffer::SizeClass::Huge);
        let text = TrackedSource {
            inner: &**opened.text.mapped().unwrap(),
            reads: Default::default(),
        };
        assert_eq!(detect_text(&text).unwrap().unwrap().language, "xml");
        let reads = text.reads.lock().unwrap();
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].start(), ByteOffset(0));
        assert_eq!(reads[0].end(), ByteOffset(MAX_BYTES as u64));
    }

    #[test]
    fn text_snapshot_read_respects_lines_bytes_and_unicode() {
        let after_limit = format!("{}<root/>", "\n".repeat(MAX_LINES));
        let by_bytes = format!("{}😀\n<root/>", " ".repeat(MAX_BYTES - 2));
        let prefix_with_partial_unicode =
            format!("<?xml version='1.0'?>{}😀", " ".repeat(MAX_BYTES - 23));
        for sample in [after_limit, by_bytes] {
            assert_eq!(
                detect_text(&rove_buffer::RopeText::from_text(&sample)).unwrap(),
                None
            );
        }
        assert_eq!(
            detect_text(&rove_buffer::RopeText::from_text(
                &prefix_with_partial_unicode
            ))
            .unwrap()
            .unwrap()
            .language,
            "xml"
        );
    }
}
