//! First-layer index for large JSON. It scans balanced values without allocating a full CST.
//! On-demand scans are limited to the requested container's original source range. The complete
//! grammar parse remains the source of syntax diagnostics, computed independently on the worker.
use super::{
    model::{key_path, Entry, Model},
    Location,
};
use rove_base::{Error, Result, Span};
use rove_syntax::{symbols::kinds, Symbol, SymbolKind};
use std::{ops::Range, sync::Arc};

pub(super) const LARGE_JSON_BYTES: usize = 20 * 1024 * 1024;

pub(super) fn roots(model: &Model<'_>) -> Result<Vec<Entry>> {
    let mut index = JsonIndex::new(model, 0..model.text.len());
    index.trivia()?;
    match index.peek() {
        Some(b'{' | b'[') => index.children("$"),
        Some(_) => {
            let value = index.value()?;
            let selection = value.span.clone();
            Ok(vec![index.entry(
                "$".to_owned(),
                "$".to_owned(),
                selection.clone(),
                selection,
                value,
            )?])
        }
        None => Ok(Vec::new()),
    }
}

pub(super) fn children(model: &Model<'_>, range: Range<usize>, path: &str) -> Result<Vec<Entry>> {
    JsonIndex::new(model, range).children(path)
}

struct Value {
    span: Range<usize>,
    kind: SymbolKind,
    count: Option<usize>,
}

struct JsonIndex<'a, 'b> {
    model: &'a Model<'b>,
    end: usize,
    at: usize,
    next_cancel: usize,
}

impl<'a, 'b> JsonIndex<'a, 'b> {
    fn new(model: &'a Model<'b>, range: Range<usize>) -> Self {
        Self {
            model,
            at: range.start,
            end: range.end,
            next_cancel: range.start,
        }
    }

    fn peek(&self) -> Option<u8> {
        (self.at < self.end).then(|| self.model.text.as_bytes()[self.at])
    }

    fn check(&mut self) -> Result<()> {
        if self.at >= self.next_cancel {
            self.model.cancel.check()?;
            self.next_cancel = self.at.saturating_add(64 * 1024);
        }
        Ok(())
    }

    fn trivia(&mut self) -> Result<()> {
        loop {
            self.check()?;
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.at += 1,
                Some(b'/') if self.comment() => {}
                _ => return Ok(()),
            }
        }
    }

    fn comment(&mut self) -> bool {
        if self.at + 1 >= self.end {
            return false;
        }
        let bytes = self.model.text.as_bytes();
        match bytes.get(self.at + 1).copied() {
            Some(b'/') => {
                self.at += 2;
                let body = &bytes[self.at..self.end];
                self.at += memchr::memchr2(b'\n', b'\r', body).unwrap_or(body.len());
                true
            }
            Some(b'*') => {
                self.at += 2;
                let body = &bytes[self.at..self.end];
                self.at += memchr::memmem::find(body, b"*/").map_or(body.len(), |end| end + 2);
                true
            }
            _ => false,
        }
    }

    /// Skip one quoted token. Escape handling only needs to skip the immediately escaped byte.
    #[inline]
    fn quoted(&mut self) {
        let bytes = self.model.text.as_bytes();
        self.at += 1;
        while self.at < self.end {
            let byte = bytes[self.at];
            self.at += 1;
            if byte == b'"' {
                break;
            }
            if byte == b'\\' {
                self.at = (self.at + 1).min(self.end);
            }
        }
    }

    fn value(&mut self) -> Result<Value> {
        self.check()?;
        let start = self.at;
        let Some(first) = self.peek() else {
            return Err(Error::InvalidInput);
        };
        let (kind, count) = match first {
            b'{' | b'[' => {
                let kind = if first == b'{' {
                    SymbolKind::Object
                } else {
                    SymbolKind::Array
                };
                (kind, Some(self.balanced()?))
            }
            b'"' => {
                self.quoted();
                (other(kinds::STRING), None)
            }
            other_byte => {
                while let Some(byte) = self.peek() {
                    if matches!(
                        byte,
                        b' ' | b'\t' | b'\r' | b'\n' | b',' | b':' | b'}' | b']' | b'/'
                    ) {
                        break;
                    }
                    self.at += 1;
                    self.check()?;
                }
                let kind = match other_byte {
                    b'-' | b'0'..=b'9' => kinds::NUMBER,
                    b't' | b'f' => kinds::BOOLEAN,
                    b'n' => kinds::NULL,
                    _ => kinds::STRING,
                };
                (other(kind), None)
            }
        };
        if self.at == start {
            return Err(Error::InvalidInput);
        }
        self.check()?;
        Ok(Value {
            span: start..self.at,
            kind,
            count,
        })
    }

    #[inline]
    fn balanced(&mut self) -> Result<usize> {
        let bytes = self.model.text.as_bytes();
        let mut depth = 1;
        let mut count = 0;
        let mut new_member = true;
        self.at += 1;
        while self.at < self.end {
            if self.at >= self.next_cancel {
                self.check()?;
            }
            match bytes[self.at] {
                b' ' | b'\n' | b'\r' | b'\t' => self.at += 1,
                b',' => {
                    self.at += 1;
                    new_member |= depth == 1;
                }
                b'}' | b']' => {
                    self.at += 1;
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                b'/' if self.comment() => {}
                byte => {
                    if depth == 1 && new_member {
                        count += 1;
                        new_member = false;
                    }
                    match byte {
                        b'"' => self.quoted(),
                        b'{' | b'[' => {
                            self.at += 1;
                            depth += 1;
                        }
                        _ => self.at += 1,
                    }
                }
            }
        }
        Ok(count)
    }

    fn children(&mut self, parent: &str) -> Result<Vec<Entry>> {
        let Some(first @ (b'{' | b'[')) = self.peek() else {
            return Ok(Vec::new());
        };
        let object = first == b'{';
        self.at += 1;
        let mut entries = Vec::new();
        while entries.len() < self.model.limit {
            self.trivia()?;
            if matches!(self.peek(), None | Some(b'}' | b']')) {
                break;
            }
            let start = self.at;
            let (name, name_span, path) = if object {
                if self.peek() != Some(b'"') {
                    break;
                }
                self.quoted();
                let end = self.at;
                let raw = &self.model.text[start..end];
                let name: String =
                    serde_json::from_str(raw).unwrap_or_else(|_| raw.trim_matches('"').to_string());
                let path = key_path(parent, &name);
                self.trivia()?;
                if self.peek() != Some(b':') {
                    break;
                }
                self.at += 1;
                self.trivia()?;
                (name, Some(start..end), path)
            } else {
                let name = entries.len().to_string();
                let path = format!("{parent}[{name}]");
                (name, None, path)
            };
            let value = match self.value() {
                Ok(value) => value,
                Err(Error::InvalidInput) => break,
                Err(error) => return Err(error),
            };
            let range = start..value.span.end;
            let selection = name_span.unwrap_or_else(|| value.span.clone());
            entries.push(self.entry(name, path, range, selection, value)?);
            self.trivia()?;
            if self.peek() != Some(b',') {
                break;
            }
            self.at += 1;
        }
        Ok(entries)
    }

    fn entry(
        &self,
        name: String,
        path: String,
        range: Range<usize>,
        selection: Range<usize>,
        value: Value,
    ) -> Result<Entry> {
        let preview: Arc<str> = match value.count {
            Some(count) if value.kind == SymbolKind::Array => format!("[{count}]").into(),
            Some(count) => format!("{{{count}}}").into(),
            None => super::model::preview(&self.model.text[value.span.clone()]),
        };
        let container = value.count.map(|_| Location::Json(value.span.clone()));
        let lines = self.model.lines;
        Ok(Entry {
            path: path.into(),
            preview,
            value: Some(value.span),
            container,
            symbol: Symbol {
                name: name.into(),
                kind: value.kind,
                range: Span::new(lines.at(range.start), lines.at(range.end))?,
                selection: Span::new(lines.at(selection.start), lines.at(selection.end))?,
                children: Arc::from([]),
            },
        })
    }
}

fn other(kind: &str) -> SymbolKind {
    SymbolKind::Other(kind.into())
}

/// Mask the comma token only. The bundled JSON grammar accepts comments, with JSONC
/// trailing commas handled here so source byte and line coordinates remain exact.
pub(super) fn jsonc_source<'a>(
    text: &'a str,
    cancel: &rove_base::Cancellation,
) -> Result<std::borrow::Cow<'a, str>> {
    use std::borrow::Cow;
    let bytes = text.as_bytes();
    let mut trailing = Vec::new();
    let mut comma = None;
    let mut value_before = false;
    let mut i = 0;
    let mut next_cancel = 0;
    while let Some(&b) = bytes.get(i) {
        if i >= next_cancel {
            cancel.check()?;
            next_cancel = i.saturating_add(64 * 1024);
        }
        match b {
            b' ' | b'\n' | b'\r' | b'\t' => i += 1,
            b',' => {
                comma = value_before.then_some(i);
                value_before = false;
                i += 1;
            }
            b'}' | b']' => {
                if let Some(token) = comma.take() {
                    trailing.push(token);
                }
                value_before = true;
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                i += memchr::memchr2(b'\n', b'\r', &bytes[i..]).unwrap_or(bytes.len() - i);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                i += memchr::memmem::find(&bytes[i..], b"*/").map_or(bytes.len() - i, |p| p + 2);
            }
            b'"' => {
                comma = None;
                value_before = true;
                i += 1;
                while let Some(&byte) = bytes.get(i) {
                    i += 1;
                    if byte == b'"' {
                        break;
                    }
                    if byte == b'\\' {
                        i = (i + 1).min(bytes.len());
                    }
                }
            }
            _ => {
                comma = None;
                value_before = !matches!(b, b'{' | b'[' | b':');
                i += 1;
            }
        }
    }
    cancel.check()?;
    if trailing.is_empty() {
        return Ok(Cow::Borrowed(text));
    }
    let mut masked = bytes.to_vec();
    for offset in trailing {
        masked[offset] = b' ';
    }
    Ok(Cow::Owned(
        String::from_utf8(masked).expect("only ASCII punctuation was masked"),
    ))
}
