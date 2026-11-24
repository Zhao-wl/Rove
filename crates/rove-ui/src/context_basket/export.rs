use super::model::{Availability, Basket, Entry};
use std::{borrow::Cow, fmt::Write};

pub const DEFAULT_REFERENCE: &str = "@{path} (lines {start}-{end})";

pub struct Excerpt {
    pub language: String,
    pub text: String,
}

#[derive(Default, PartialEq, Eq)]
pub struct Export {
    pub text: String,
    pub entries: usize,
    pub lines: u64,
    pub characters: usize,
    pub used_default_template: bool,
}

impl Export {
    pub fn estimated_tokens(&self) -> usize {
        self.characters.div_ceil(4)
    }
}

pub fn markdown(
    basket: &Basket,
    mut current_excerpt: impl FnMut(&Entry) -> Option<Excerpt>,
) -> Export {
    let mut out = Export::default();
    if !basket.introduction.trim().is_empty() {
        out.text.push_str(&lf(&basket.introduction));
        out.text.push_str("\n\n");
    }
    for entry in basket
        .items()
        .iter()
        .filter(|entry| entry.availability == Availability::Current)
    {
        let Some(excerpt) = current_excerpt(entry) else {
            continue;
        };
        let source = lf(&excerpt.text);
        out.entries += 1;
        out.lines += entry.lines.lines();
        out.characters += source.chars().count();
        write!(
            &mut out.text,
            "## {}. {}:{}-{}",
            out.entries,
            entry.path.as_str(),
            entry.lines.first(),
            entry.lines.last(),
        )
        .unwrap();
        if let Some(symbol) = &entry.symbol {
            let label = symbol.replace(['\r', '\n'], " ");
            let fence = ticks(longest_run(&label).saturating_add(1));
            write!(&mut out.text, " · {fence}{label}{fence}").unwrap();
        }
        out.text.push_str("\n\n");
        if !entry.note.trim().is_empty() {
            out.text.push_str(&lf(&entry.note));
            out.text.push_str("\n\n");
        }
        let fence = ticks(longest_run(&source).saturating_add(1).max(3));
        let language = excerpt.language.as_str();
        let language = if language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'+' | b'#'))
        {
            language
        } else {
            ""
        };
        writeln!(&mut out.text, "{fence}{language}").unwrap();
        out.text.push_str(&source);
        if !source.ends_with('\n') {
            out.text.push('\n');
        }
        write!(&mut out.text, "{fence}\n\n").unwrap();
    }
    if out.entries == 0 {
        out.text.clear();
    } else {
        out.text.pop();
    }
    out
}

#[derive(Clone, Copy)]
enum Field {
    Literal,
    Path,
    Start,
    End,
}

fn parse_template(source: &str) -> Option<Vec<(Field, &str)>> {
    let mut tokens = Vec::new();
    let mut remaining = source;
    let mut required = 0;
    while let Some(index) = remaining.find('{') {
        let prefix = &remaining[..index];
        if prefix.contains('}') {
            return None;
        }
        let (key, rest) = remaining[index + 1..].split_once('}')?;
        let (field, flag) = match key {
            "path" => (Field::Path, 1),
            "start" => (Field::Start, 2),
            "end" => (Field::End, 4),
            _ => return None,
        };
        if !prefix.is_empty() {
            tokens.push((Field::Literal, prefix));
        }
        tokens.push((field, ""));
        required |= flag;
        remaining = rest;
    }
    if remaining.contains('}') || required != 7 || source.contains(['\n', '\r']) {
        return None;
    }
    if !remaining.is_empty() {
        tokens.push((Field::Literal, remaining));
    }
    Some(tokens)
}

pub fn references(basket: &Basket, template: &str) -> Export {
    let parsed = parse_template(template);
    let mut out = Export {
        used_default_template: parsed.is_none(),
        ..Export::default()
    };
    let tokens = parsed.unwrap_or_else(|| parse_template(DEFAULT_REFERENCE).unwrap());
    for entry in basket
        .items()
        .iter()
        .filter(|item| item.availability == Availability::Current)
    {
        let path = entry.path.as_str();
        let quote = path.chars().any(char::is_whitespace);
        for (field, literal) in &tokens {
            match field {
                Field::Literal => out.text.push_str(literal),
                Field::Path if quote => {
                    write!(&mut out.text, "\"{}\"", path.replace('"', "\\\"")).unwrap()
                }
                Field::Path => out.text.push_str(path),
                Field::Start => write!(&mut out.text, "{}", entry.lines.first()).unwrap(),
                Field::End => write!(&mut out.text, "{}", entry.lines.last()).unwrap(),
            }
        }
        out.text.push('\n');
        out.entries += 1;
        out.lines += entry.lines.lines();
    }
    out.characters = out.text.chars().count();
    out
}

fn lf(text: &str) -> Cow<'_, str> {
    if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

fn longest_run(text: &str) -> usize {
    text.split(|ch| ch != '`')
        .map(str::len)
        .max()
        .unwrap_or_default()
}

fn ticks(length: usize) -> String {
    std::iter::repeat_n('`', length).collect()
}

#[cfg(test)]
mod tests {
    use super::super::model::LineRange;
    use super::*;
    use rove_base::WorkspacePath;

    fn sample() -> Basket {
        let mut basket = Basket::default();
        basket.introduction = "目的\r\n第二行".into();
        let mut active = Entry::new(
            WorkspacePath::new("Assets/Standard Assets/Foo.cs").unwrap(),
            LineRange::new(41, 58).unwrap(),
            Some("Foo.Move(Vector3)".into()),
        );
        active.note = "备注\r保持文字".into();
        basket.insert(active).unwrap();
        let missing = WorkspacePath::new("Assets/Deleted.cs").unwrap();
        basket
            .insert(Entry::new(
                missing.clone(),
                LineRange::new(1, 2).unwrap(),
                None,
            ))
            .unwrap();
        basket.mark_removed(&missing);
        basket
    }

    #[test]
    fn clipboard_markdown_uses_current_unicode_content_and_a_longer_fence() {
        let out = markdown(&sample(), |_| {
            Some(Excerpt {
                language: "csharp".into(),
                text: "当前\r\n`````inside\r尾部".into(),
            })
        });
        assert_eq!(out.entries, 1);
        assert_eq!(out.lines, 18);
        assert!(out.text.starts_with(
            "目的\n第二行\n\n## 1. Assets/Standard Assets/Foo.cs:41-58 · `Foo.Move(Vector3)`"
        ));
        assert!(out
            .text
            .contains("\n备注\n保持文字\n\n``````csharp\n当前\n`````inside\n尾部\n``````\n"));
        assert!(!out.text.contains("Deleted"));
        assert!(!out.text.contains('\r'));
    }

    #[test]
    fn references_quote_spaces_and_fall_back_on_invalid_templates() {
        let default = references(&sample(), DEFAULT_REFERENCE);
        assert_eq!(
            default.text,
            "@\"Assets/Standard Assets/Foo.cs\" (lines 41-58)\n"
        );
        assert!(!default.used_default_template);
        let custom = references(&sample(), "{path}:{start}..{end}");
        assert_eq!(custom.text, "\"Assets/Standard Assets/Foo.cs\":41..58\n");
        for invalid in [
            "{path}:{start}",
            "{path}:{start}:{wrong}:{end}",
            "{path} {start} {end}\n",
        ] {
            let out = references(&sample(), invalid);
            assert!(out.used_default_template);
            assert_eq!(out.text, default.text);
        }
    }
}
